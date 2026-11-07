//! The ZIP archive reader (`src/zip.rs`).
//!
//! The executable code below is the library's, token for token (CI checks
//! this with `.github/scripts/verus_drift.py`); every line marked
//! `// @ghost` is specification or proof. The theorem: for every byte
//! string of every length, `ZipArchive::parse` returns, without panicking,
//! and so do `index_of`, `raw_data`, `read` and `read_by_name` on the
//! archive it returns, for every index and name. Every header read, every
//! slice and every offset sum stays in bounds, on 32-bit targets as on
//! 64-bit ones, and every loop ends. `read` decompresses with the verified
//! decoder (`inflate`, DEFLATE or Deflate64) and checks the CRC with the
//! verified `crc32`.

use crate::crc32::crc32;
use crate::error::InflateError;
use crate::inflate::{decompress_deflate64_with_limit, decompress_with_limit};
use crate::std_specs::axiom_byte_slice_len;
use vstd::prelude::*;

verus! {

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const FLAG_ENCRYPTED: u16 = 1;
/// Compression method 0.
pub const METHOD_STORED: u16 = 0;
/// Compression method 8.
pub const METHOD_DEFLATED: u16 = 8;
/// Compression method 9, Deflate64 (read only).
pub const METHOD_DEFLATE64: u16 = 9;
const MAX_COMMENT: usize = 0xFFFF;

/// Errors from reading or writing ZIP archives.
pub enum ZipError {
    /// The data ended in the middle of a structure.
    Truncated,
    /// A header did not start with its signature.
    BadSignature,
    /// No end-of-central-directory record was found.
    NotAZip,
    /// The archive spans several disks.
    MultiDisk,
    /// The archive or an entry needs ZIP64 (4 GiB+ or 65535+ entries).
    Zip64Unsupported,
    /// The entry is encrypted.
    Encrypted,
    /// The entry uses a compression method other than stored or deflate.
    UnsupportedMethod(u16),
    /// The entry's sizes are inconsistent with its data.
    SizeMismatch,
    /// The entry's data does not match its CRC-32.
    CrcMismatch {
        /// CRC recorded in the archive.
        expected: u32,
        /// CRC of the extracted data.
        actual: u32,
    },
    /// The entry's DEFLATE data is corrupt.
    Inflate(InflateError),
    /// The entry name is empty, too long, absolute, or contains `..`,
    /// backslashes or NUL.
    InvalidName(String),
    /// An entry with this name was already added.
    DuplicateName(String),
    /// No entry with this index or name.
    NotFound,
}

impl From<InflateError> for ZipError {
    fn from(e: InflateError) -> Self {
        Self::Inflate(e)
    }
}

// @ghost-begin
/// What the conversion above returns, for `?`.
impl vstd::std_specs::convert::FromSpecImpl<InflateError> for ZipError {
    open spec fn obeys_from_spec() -> bool {
        true
    }

    open spec fn from_spec(e: InflateError) -> Self {
        ZipError::Inflate(e)
    }
}
// @ghost-end

/// The little-endian 16-bit field at `b[i..i + 2]`.
fn u16_at(b: &[u8], i: usize) -> u16
    requires // @ghost
        i + 1 < b@.len(), // @ghost
{
    u16::from(b[i]) | (u16::from(b[i + 1]) << 8)
}

/// The little-endian 32-bit field at `b[i..i + 4]`.
fn u32_at(b: &[u8], i: usize) -> u32
    requires // @ghost
        i + 3 < b@.len(), // @ghost
{
    u32::from(b[i]) | (u32::from(b[i + 1]) << 8) | (u32::from(b[i + 2]) << 16) | (u32::from(b[i + 3]) << 24)
}

/// A timestamp in MS-DOS format (two-second resolution, years 1980-2107).
pub struct DosDateTime {
    /// 1980..=2107
    pub year: u16,
    /// 1..=12
    pub month: u8,
    /// 1..=31
    pub day: u8,
    /// 0..=23
    pub hour: u8,
    /// 0..=59
    pub minute: u8,
    /// 0..=58, even
    pub second: u8,
}

impl DosDateTime {
    /// Decodes a (time, date) pair. The result may be invalid (e.g. month
    /// 0) if the archive is; check with [`is_valid`](Self::is_valid).
    pub const fn from_dos(time: u16, date: u16) -> Self {
        proof { // @ghost-begin
            assert(date >> 9 <= 127) by (bit_vector);
            assert(time & 0x1f <= 31) by (bit_vector);
        } // @ghost-end
        Self {
            year: 1980 + (date >> 9),
            month: ((date >> 5) & 0x0f) as u8,
            day: (date & 0x1f) as u8,
            hour: (time >> 11) as u8,
            minute: ((time >> 5) & 0x3f) as u8,
            second: ((time & 0x1f) * 2) as u8,
        }
    }
}

/// Fixed-size part of a local file header (APPNOTE 4.3.7).
pub struct LocalHeader {
    /// Version needed to extract.
    pub version_needed: u16,
    /// General purpose bit flags.
    pub flags: u16,
    /// Compression method.
    pub method: u16,
    /// DOS time.
    pub mod_time: u16,
    /// DOS date.
    pub mod_date: u16,
    /// CRC-32 of the uncompressed data.
    pub crc32: u32,
    /// Size of the stored data.
    pub compressed_size: u32,
    /// Size after extraction.
    pub uncompressed_size: u32,
    /// Length of the file name that follows.
    pub name_len: u16,
    /// Length of the extra field after the name.
    pub extra_len: u16,
}

impl LocalHeader {
    /// Size of the fixed part in bytes.
    pub const SIZE: usize = 30;

    /// Decodes a header from the start of `b`.
    pub fn parse(b: &[u8]) -> Result<Self, ZipError> {
        if b.len() < Self::SIZE {
            return Err(ZipError::Truncated);
        }
        if u32_at(b, 0) != LOCAL_SIG {
            return Err(ZipError::BadSignature);
        }
        Ok(Self {
            version_needed: u16_at(b, 4),
            flags: u16_at(b, 6),
            method: u16_at(b, 8),
            mod_time: u16_at(b, 10),
            mod_date: u16_at(b, 12),
            crc32: u32_at(b, 14),
            compressed_size: u32_at(b, 18),
            uncompressed_size: u32_at(b, 22),
            name_len: u16_at(b, 26),
            extra_len: u16_at(b, 28),
        })
    }
}

/// Fixed-size part of a central directory header (APPNOTE 4.3.12).
pub struct CentralHeader {
    /// Version made by (high byte: host system).
    pub version_made_by: u16,
    /// Version needed to extract.
    pub version_needed: u16,
    /// General purpose bit flags.
    pub flags: u16,
    /// Compression method.
    pub method: u16,
    /// DOS time.
    pub mod_time: u16,
    /// DOS date.
    pub mod_date: u16,
    /// CRC-32 of the uncompressed data.
    pub crc32: u32,
    /// Size of the stored data.
    pub compressed_size: u32,
    /// Size after extraction.
    pub uncompressed_size: u32,
    /// Length of the file name.
    pub name_len: u16,
    /// Length of the extra field.
    pub extra_len: u16,
    /// Length of the file comment.
    pub comment_len: u16,
    /// Disk number where the entry starts.
    pub disk_start: u16,
    /// Internal file attributes.
    pub internal_attr: u16,
    /// External file attributes (Unix mode in the high 16 bits).
    pub external_attr: u32,
    /// Offset of the local header.
    pub local_offset: u32,
}

impl CentralHeader {
    /// Size of the fixed part in bytes.
    pub const SIZE: usize = 46;

    /// Decodes a header from the start of `b`.
    pub fn parse(b: &[u8]) -> Result<Self, ZipError> {
        if b.len() < Self::SIZE {
            return Err(ZipError::Truncated);
        }
        if u32_at(b, 0) != CENTRAL_SIG {
            return Err(ZipError::BadSignature);
        }
        Ok(Self {
            version_made_by: u16_at(b, 4),
            version_needed: u16_at(b, 6),
            flags: u16_at(b, 8),
            method: u16_at(b, 10),
            mod_time: u16_at(b, 12),
            mod_date: u16_at(b, 14),
            crc32: u32_at(b, 16),
            compressed_size: u32_at(b, 20),
            uncompressed_size: u32_at(b, 24),
            name_len: u16_at(b, 28),
            extra_len: u16_at(b, 30),
            comment_len: u16_at(b, 32),
            disk_start: u16_at(b, 34),
            internal_attr: u16_at(b, 36),
            external_attr: u32_at(b, 38),
            local_offset: u32_at(b, 42),
        })
    }
}

/// End-of-central-directory record (APPNOTE 4.3.16), without the comment.
pub struct EndOfCentralDirectory {
    /// Number of this disk.
    pub disk: u16,
    /// Disk where the central directory starts.
    pub cd_disk: u16,
    /// Central directory entries on this disk.
    pub entries_on_disk: u16,
    /// Total central directory entries.
    pub entries_total: u16,
    /// Size of the central directory in bytes.
    pub cd_size: u32,
    /// Offset of the central directory.
    pub cd_offset: u32,
    /// Length of the archive comment that follows.
    pub comment_len: u16,
}

impl EndOfCentralDirectory {
    /// Size of the fixed part in bytes.
    pub const SIZE: usize = 22;

    /// Decodes a record from the start of `b`.
    pub fn parse(b: &[u8]) -> Result<Self, ZipError> {
        if b.len() < Self::SIZE {
            return Err(ZipError::Truncated);
        }
        if u32_at(b, 0) != EOCD_SIG {
            return Err(ZipError::BadSignature);
        }
        Ok(Self {
            disk: u16_at(b, 4),
            cd_disk: u16_at(b, 6),
            entries_on_disk: u16_at(b, 8),
            entries_total: u16_at(b, 10),
            cd_size: u32_at(b, 12),
            cd_offset: u32_at(b, 16),
            comment_len: u16_at(b, 20),
        })
    }
}

/// One entry of a parsed archive, from its central directory record.
pub struct ZipEntry {
    /// The name, decoded as UTF-8 (invalid sequences replaced).
    pub name: String,
    /// The name exactly as stored.
    pub raw_name: Vec<u8>,
    /// Compression method (0 = stored, 8 = deflate).
    pub method: u16,
    /// General purpose flags.
    pub flags: u16,
    /// CRC-32 of the extracted data.
    pub crc32: u32,
    /// Size of the stored data.
    pub compressed_size: u32,
    /// Size after extraction.
    pub uncompressed_size: u32,
    /// Modification time (may be invalid if the archive's is).
    pub modified: DosDateTime,
    /// Unix permission bits, if the archive was made on a Unix system.
    pub unix_mode: Option<u32>,
    /// Offset of the local header.
    pub local_offset: u32,
}

impl ZipEntry {
    /// True if the entry is a directory.
    pub fn is_dir(&self) -> bool {
        self.raw_name.last() == Some(&b'/')
    }
}

/// A ZIP archive parsed from a byte slice.
pub struct ZipArchive<'a> {
    data: &'a [u8],
    entries: Vec<ZipEntry>,
}

/// Finds the end-of-central-directory record: scanning backwards over the
/// last 65557 bytes for a signature whose comment ends exactly at the end
/// of the data.
#[verifier::loop_isolation(false)]
fn find_eocd(data: &[u8]) -> (r: Result<usize, ZipError>)
    ensures // @ghost
        r is Ok ==> r->Ok_0 + EndOfCentralDirectory::SIZE <= data@.len(), // @ghost
{
    proof { // @ghost-begin
        axiom_byte_slice_len(data);
    } // @ghost-end
    if data.len() < EndOfCentralDirectory::SIZE {
        return Err(ZipError::NotAZip);
    }
    let last = data.len() - EndOfCentralDirectory::SIZE;
    let first = last.saturating_sub(MAX_COMMENT);
    let mut i = last;
    loop
        invariant // @ghost
            first <= i <= last, // @ghost
        decreases i, // @ghost
    {
        if u32_at(data, i) == EOCD_SIG && i + EndOfCentralDirectory::SIZE + u16_at(data, i + 20) as usize == data.len()
        {
            return Ok(i);
        }
        if i == first {
            return Err(ZipError::NotAZip);
        }
        i -= 1;
    }
}

/// Central directory range `(start, end)` given the end record at `e`.
/// Guarantees `start <= end <= e` (proven by Kani for all field values).
fn directory_bounds(e: usize, eocd: &EndOfCentralDirectory) -> (r: Result<(usize, usize), ZipError>)
    ensures // @ghost
        r is Ok ==> r->Ok_0.0 <= r->Ok_0.1 <= e, // @ghost
{
    let start = eocd.cd_offset as usize;
    let end = start.checked_add(eocd.cd_size as usize).ok_or(ZipError::Truncated)?;
    if end > e {
        return Err(ZipError::Truncated);
    }
    Ok((start, end))
}

/// Byte ranges `(name_start, name_end, next)` of the central record at `p`.
/// Guarantees `p + 46 == name_start <= name_end <= next <= cd_end` (proven
/// by Kani for all positions and field values), so the parse loop's slices
/// are in bounds and it always advances.
fn record_bounds(p: usize, cd_end: usize, h: &CentralHeader) -> (r: Result<(usize, usize, usize), ZipError>)
    ensures // @ghost
        r is Ok ==> p + 46 == r->Ok_0.0 && r->Ok_0.0 <= r->Ok_0.1 <= r->Ok_0.2 <= cd_end, // @ghost
{
    let name_start = p.checked_add(CentralHeader::SIZE).ok_or(ZipError::Truncated)?;
    let name_end = name_start.checked_add(h.name_len as usize).ok_or(ZipError::Truncated)?;
    let next = name_end.checked_add(h.extra_len as usize + h.comment_len as usize).ok_or(ZipError::Truncated)?;
    if next > cd_end {
        return Err(ZipError::Truncated);
    }
    Ok((name_start, name_end, next))
}

impl<'a> ZipArchive<'a> {
    /// Parses the central directory of `data`.
    #[verifier::loop_isolation(false)]
    pub fn parse(data: &'a [u8]) -> Result<Self, ZipError> {
        let e = find_eocd(data)?;
        let eocd = EndOfCentralDirectory::parse(&data[e..])?;
        if eocd.disk != 0 || eocd.cd_disk != 0 || eocd.entries_on_disk != eocd.entries_total {
            return Err(ZipError::MultiDisk);
        }
        if eocd.entries_total == 0xFFFF || eocd.cd_size == u32::MAX || eocd.cd_offset == u32::MAX {
            return Err(ZipError::Zip64Unsupported);
        }
        let (cd_start, cd_end) = directory_bounds(e, &eocd)?;
        let total = eocd.entries_total as usize;
        let mut entries = Vec::with_capacity(total.min(eocd.cd_size as usize / CentralHeader::SIZE));
        let mut p = cd_start;
        for _ in 0..total
            invariant // @ghost
                p <= cd_end, // @ghost
        {
            // Invariant: p <= cd_end <= e < data.len() (see the bounds lemmas).
            let h = CentralHeader::parse(&data[p..cd_end])?;
            let (name_start, name_end, next) = record_bounds(p, cd_end, &h)?;
            if h.disk_start != 0 {
                return Err(ZipError::MultiDisk);
            }
            if h.compressed_size == u32::MAX || h.uncompressed_size == u32::MAX || h.local_offset == u32::MAX {
                return Err(ZipError::Zip64Unsupported);
            }
            let raw_name = data[name_start..name_end].to_vec();
            entries.push(ZipEntry {
                name: String::from_utf8_lossy(&raw_name).into_owned(),
                raw_name,
                method: h.method,
                flags: h.flags,
                crc32: h.crc32,
                compressed_size: h.compressed_size,
                uncompressed_size: h.uncompressed_size,
                modified: DosDateTime::from_dos(h.mod_time, h.mod_date),
                unix_mode: if h.version_made_by >> 8 == 3 { Some(h.external_attr >> 16) } else { None },
                local_offset: h.local_offset,
            });
            p = next;
        }
        Ok(Self { data, entries })
    }

    /// Index of the entry with this name.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        let name = name.as_bytes();
        let mut i = 0;
        while i < self.entries.len()
            invariant // @ghost
                i <= self.entries@.len(), // @ghost
            decreases self.entries@.len() - i, // @ghost
        {
            if self.entries[i].raw_name.as_slice() == name {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    /// The raw (still compressed) bytes of entry `index`.
    pub fn raw_data(&self, index: usize) -> Result<&'a [u8], ZipError> {
        proof { // @ghost-begin
            axiom_byte_slice_len(self.data);
        } // @ghost-end
        let e = self.entries.get(index).ok_or(ZipError::NotFound)?;
        let off = e.local_offset as usize;
        let lh = LocalHeader::parse(self.data.get(off..).ok_or(ZipError::Truncated)?)?;
        let start = off + LocalHeader::SIZE + lh.name_len as usize + lh.extra_len as usize;
        let end = start.checked_add(e.compressed_size as usize).ok_or(ZipError::Truncated)?;
        self.data.get(start..end).ok_or(ZipError::Truncated)
    }

    /// Extracts entry `index`, checking its size and CRC-32. Output is
    /// limited to the size recorded in the central directory.
    pub fn read(&self, index: usize) -> Result<Vec<u8>, ZipError> {
        let e = self.entries.get(index).ok_or(ZipError::NotFound)?;
        if e.flags & FLAG_ENCRYPTED != 0 {
            return Err(ZipError::Encrypted);
        }
        let raw = self.raw_data(index)?;
        let out = match e.method {
            METHOD_STORED => {
                if e.compressed_size != e.uncompressed_size {
                    return Err(ZipError::SizeMismatch);
                }
                raw.to_vec()
            }
            METHOD_DEFLATED => decompress_with_limit(raw, e.uncompressed_size as usize)?.data,
            METHOD_DEFLATE64 => decompress_deflate64_with_limit(raw, e.uncompressed_size as usize)?.data,
            m => return Err(ZipError::UnsupportedMethod(m)),
        };
        if out.len() != e.uncompressed_size as usize {
            return Err(ZipError::SizeMismatch);
        }
        let actual = crc32(&out);
        if actual != e.crc32 {
            return Err(ZipError::CrcMismatch { expected: e.crc32, actual });
        }
        Ok(out)
    }

    /// Extracts the entry with this name.
    pub fn read_by_name(&self, name: &str) -> Result<Vec<u8>, ZipError> {
        self.read(self.index_of(name).ok_or(ZipError::NotFound)?)
    }
}

} // verus!
