//! ZIP archives (PKWARE APPNOTE 6.3): writing and reading archives whose
//! entries are stored or DEFLATE-compressed.
//!
//! Supported: method 0 (stored) and 8 (deflate), UTF-8 names (flag bit 11),
//! Unix permissions, DOS timestamps, data descriptors when reading.
//! Not supported: ZIP64 (archives or entries of 4 GiB and more, or 65535+
//! entries), encryption, multi-disk archives. These are reported as errors,
//! never misread.

use crate::crc32::crc32;
use crate::deflate::{compress_with, CompressOptions, Strategy};
use crate::error::InflateError;
use crate::inflate::decompress_with_limit;
use core::fmt;
use std::collections::HashSet;

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
/// "Version needed to extract": 2.0 (deflate, directories).
const VERSION_NEEDED: u16 = 20;
/// "Version made by": host system 3 (UNIX), spec version 2.0.
const VERSION_MADE_BY: u16 = (3 << 8) | 20;
const FLAG_ENCRYPTED: u16 = 1;
const FLAG_UTF8: u16 = 1 << 11;
/// Compression method 0.
pub const METHOD_STORED: u16 = 0;
/// Compression method 8.
pub const METHOD_DEFLATED: u16 = 8;
const MAX_COMMENT: usize = 0xFFFF;

/// Errors from reading or writing ZIP archives.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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

impl fmt::Display for ZipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("archive is truncated"),
            Self::BadSignature => f.write_str("bad header signature"),
            Self::NotAZip => f.write_str("no end-of-central-directory record (not a zip file?)"),
            Self::MultiDisk => f.write_str("multi-disk archives are not supported"),
            Self::Zip64Unsupported => f.write_str("ZIP64 is not supported"),
            Self::Encrypted => f.write_str("encrypted entries are not supported"),
            Self::UnsupportedMethod(m) => write!(f, "unsupported compression method {m}"),
            Self::SizeMismatch => f.write_str("entry size does not match its data"),
            Self::CrcMismatch { expected, actual } => {
                write!(f, "CRC mismatch: archive says {expected:08x}, data is {actual:08x}")
            }
            Self::Inflate(e) => write!(f, "corrupt compressed data: {e}"),
            Self::InvalidName(n) => write!(f, "invalid entry name {n:?}"),
            Self::DuplicateName(n) => write!(f, "duplicate entry name {n:?}"),
            Self::NotFound => f.write_str("no such entry"),
        }
    }
}

impl std::error::Error for ZipError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Inflate(e) => Some(e),
            _ => None,
        }
    }
}

impl From<InflateError> for ZipError {
    fn from(e: InflateError) -> Self {
        Self::Inflate(e)
    }
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn put16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Converts a size or offset to the 32-bit field it is stored in.
/// `0xFFFFFFFF` is reserved as the ZIP64 marker, so it is refused too.
fn field32(n: usize) -> Result<u32, ZipError> {
    match u32::try_from(n) {
        Ok(v) if v != u32::MAX => Ok(v),
        _ => Err(ZipError::Zip64Unsupported),
    }
}

/// A timestamp in MS-DOS format (two-second resolution, years 1980-2107).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

impl Default for DosDateTime {
    fn default() -> Self {
        Self::EPOCH
    }
}

const fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

const fn days_in_month(year: i64, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (H. Hinnant's
/// `days_from_civil`).
const fn days_from_civil(y: i64, m: u8, d: u8) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`] (H. Hinnant's `civil_from_days`).
const fn civil_from_days(z: i64) -> (i64, u8, u8) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

impl DosDateTime {
    /// 1980-01-01 00:00:00, the earliest representable time.
    pub const EPOCH: Self = Self { year: 1980, month: 1, day: 1, hour: 0, minute: 0, second: 0 };
    /// 2107-12-31 23:59:58, the latest representable time.
    pub const MAX: Self = Self { year: 2107, month: 12, day: 31, hour: 23, minute: 59, second: 58 };

    /// Builds a timestamp, rounding odd seconds down; `None` if out of range.
    pub const fn new(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Option<Self> {
        let t = Self { year, month, day, hour, minute, second: second & !1 };
        if t.is_valid() {
            Some(t)
        } else {
            None
        }
    }

    /// True if every field is in range (including the day of the month).
    pub const fn is_valid(&self) -> bool {
        self.year >= 1980
            && self.year <= 2107
            && self.month >= 1
            && self.month <= 12
            && self.day >= 1
            && self.day <= days_in_month(self.year as i64, self.month)
            && self.hour <= 23
            && self.minute <= 59
            && self.second <= 58
            && self.second % 2 == 0
    }

    /// Encodes as the (time, date) pair stored in ZIP headers.
    pub const fn to_dos(&self) -> (u16, u16) {
        let time = ((self.hour as u16) << 11) | ((self.minute as u16) << 5) | (self.second as u16 / 2);
        let date = ((self.year.wrapping_sub(1980) & 0x7f) << 9) | ((self.month as u16) << 5) | self.day as u16;
        (time, date)
    }

    /// Decodes a (time, date) pair. The result may be invalid (e.g. month
    /// 0) if the archive is; check with [`is_valid`](Self::is_valid).
    pub const fn from_dos(time: u16, date: u16) -> Self {
        Self {
            year: 1980 + (date >> 9),
            month: ((date >> 5) & 0x0f) as u8,
            day: (date & 0x1f) as u8,
            hour: (time >> 11) as u8,
            minute: ((time >> 5) & 0x3f) as u8,
            second: ((time & 0x1f) * 2) as u8,
        }
    }

    /// Converts seconds since the Unix epoch (UTC), clamping to the
    /// representable range.
    pub const fn from_unix_seconds(secs: i64) -> Self {
        const LO: i64 = days_from_civil(1980, 1, 1) * 86_400;
        const HI: i64 = days_from_civil(2107, 12, 31) * 86_400 + 86_399;
        let s = if secs < LO {
            LO
        } else if secs > HI {
            HI
        } else {
            secs
        };
        let days = s.div_euclid(86_400);
        let rem = s.rem_euclid(86_400);
        let (y, m, d) = civil_from_days(days);
        Self {
            year: y as u16,
            month: m,
            day: d,
            hour: (rem / 3600) as u8,
            minute: (rem / 60 % 60) as u8,
            second: (rem % 60) as u8 & !1,
        }
    }
}

/// Fixed-size part of a local file header (APPNOTE 4.3.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// Appends the encoded header.
    pub fn write(&self, out: &mut Vec<u8>) {
        put32(out, LOCAL_SIG);
        put16(out, self.version_needed);
        put16(out, self.flags);
        put16(out, self.method);
        put16(out, self.mod_time);
        put16(out, self.mod_date);
        put32(out, self.crc32);
        put32(out, self.compressed_size);
        put32(out, self.uncompressed_size);
        put16(out, self.name_len);
        put16(out, self.extra_len);
    }

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// Appends the encoded header.
    pub fn write(&self, out: &mut Vec<u8>) {
        put32(out, CENTRAL_SIG);
        put16(out, self.version_made_by);
        put16(out, self.version_needed);
        put16(out, self.flags);
        put16(out, self.method);
        put16(out, self.mod_time);
        put16(out, self.mod_date);
        put32(out, self.crc32);
        put32(out, self.compressed_size);
        put32(out, self.uncompressed_size);
        put16(out, self.name_len);
        put16(out, self.extra_len);
        put16(out, self.comment_len);
        put16(out, self.disk_start);
        put16(out, self.internal_attr);
        put32(out, self.external_attr);
        put32(out, self.local_offset);
    }

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// Appends the encoded record.
    pub fn write(&self, out: &mut Vec<u8>) {
        put32(out, EOCD_SIG);
        put16(out, self.disk);
        put16(out, self.cd_disk);
        put16(out, self.entries_on_disk);
        put16(out, self.entries_total);
        put32(out, self.cd_size);
        put32(out, self.cd_offset);
        put16(out, self.comment_len);
    }

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

/// Checks an entry name: non-empty, at most 65535 bytes, relative, `/`
/// separated, with no empty, `.` or `..` components, no backslashes, NUL
/// or drive prefix. A single trailing `/` (directory) is allowed.
pub fn validate_name(name: &str) -> Result<(), ZipError> {
    let bad = || Err(ZipError::InvalidName(name.to_string()));
    if name.is_empty() || name.len() > 0xFFFF || name.contains('\0') || name.contains('\\') {
        return bad();
    }
    let b = name.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        return bad();
    }
    let body = name.strip_suffix('/').unwrap_or(name);
    if body.is_empty() || body.split('/').any(|c| c.is_empty() || c == "." || c == "..") {
        return bad();
    }
    Ok(())
}

/// Per-entry settings for [`ZipWriter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileOptions {
    /// Compression level 0-9 (0 stores).
    pub level: u8,
    /// DEFLATE block strategy.
    pub strategy: Strategy,
    /// Modification time.
    pub modified: DosDateTime,
    /// Unix permission bits (e.g. `0o644`).
    pub unix_mode: u32,
    /// Decompress each entry after compressing it and fall back to storing
    /// it if the round trip does not reproduce the input exactly.
    pub verify: bool,
}

impl Default for FileOptions {
    fn default() -> Self {
        Self { level: 6, strategy: Strategy::Auto, modified: DosDateTime::EPOCH, unix_mode: 0o644, verify: true }
    }
}

/// Header fields of an entry being added.
#[derive(Clone, Copy)]
struct EntryMeta {
    method: u16,
    crc: u32,
    uncompressed: usize,
    modified: DosDateTime,
    external_attr: u32,
}

/// Builds a ZIP archive in memory.
#[derive(Debug, Default)]
pub struct ZipWriter {
    out: Vec<u8>,
    central: Vec<(CentralHeader, Vec<u8>)>,
    names: HashSet<String>,
}

impl ZipWriter {
    /// Starts an empty archive.
    pub fn new() -> Self {
        Self::default()
    }

    fn check_new_name(&self, name: &str) -> Result<(), ZipError> {
        validate_name(name)?;
        if self.names.contains(name) {
            return Err(ZipError::DuplicateName(name.to_string()));
        }
        if self.central.len() >= 0xFFFF {
            return Err(ZipError::Zip64Unsupported);
        }
        Ok(())
    }

    fn add_entry(&mut self, name: &str, payload: &[u8], meta: EntryMeta) -> Result<(), ZipError> {
        let EntryMeta { method, crc, uncompressed, modified, external_attr } = meta;
        let local_offset = field32(self.out.len())?;
        let compressed_size = field32(payload.len())?;
        let uncompressed_size = field32(uncompressed)?;
        field32(self.out.len() + LocalHeader::SIZE + name.len() + payload.len())?;
        let flags = if name.is_ascii() { 0 } else { FLAG_UTF8 };
        let (mod_time, mod_date) = modified.to_dos();
        let name_len = name.len() as u16;
        LocalHeader {
            version_needed: VERSION_NEEDED,
            flags,
            method,
            mod_time,
            mod_date,
            crc32: crc,
            compressed_size,
            uncompressed_size,
            name_len,
            extra_len: 0,
        }
        .write(&mut self.out);
        self.out.extend_from_slice(name.as_bytes());
        self.out.extend_from_slice(payload);
        let central = CentralHeader {
            version_made_by: VERSION_MADE_BY,
            version_needed: VERSION_NEEDED,
            flags,
            method,
            mod_time,
            mod_date,
            crc32: crc,
            compressed_size,
            uncompressed_size,
            name_len,
            extra_len: 0,
            comment_len: 0,
            disk_start: 0,
            internal_attr: 0,
            external_attr,
            local_offset,
        };
        self.central.push((central, name.as_bytes().to_vec()));
        self.names.insert(name.to_string());
        Ok(())
    }

    /// Adds a file. It is DEFLATE-compressed unless that would not make it
    /// smaller (or `level` is 0), in which case it is stored.
    pub fn add_file(&mut self, name: &str, data: &[u8], opts: &FileOptions) -> Result<(), ZipError> {
        self.check_new_name(name)?;
        if name.ends_with('/') {
            return Err(ZipError::InvalidName(name.to_string()));
        }
        field32(data.len())?;
        let crc = crc32(data);
        let compressed = if opts.level == 0 {
            None
        } else {
            let c = compress_with(
                data,
                &CompressOptions { level: opts.level, strategy: opts.strategy, verify: opts.verify },
            );
            if c.len() < data.len() {
                Some(c)
            } else {
                None
            }
        };
        let meta = EntryMeta {
            method: METHOD_STORED,
            crc,
            uncompressed: data.len(),
            modified: opts.modified,
            external_attr: (0o100_000 | (opts.unix_mode & 0o7777)) << 16,
        };
        match compressed {
            Some(c) => self.add_entry(name, &c, EntryMeta { method: METHOD_DEFLATED, ..meta }),
            None => self.add_entry(name, data, meta),
        }
    }

    /// Adds a directory entry (a trailing `/` is added if missing).
    ///
    /// Directories get search permission wherever they have read
    /// permission, so the default file mode 0o644 becomes 0o755; without
    /// it, extracted directories could not be entered.
    pub fn add_directory(&mut self, name: &str, opts: &FileOptions) -> Result<(), ZipError> {
        let name = if name.ends_with('/') { name.to_string() } else { format!("{name}/") };
        self.check_new_name(&name)?;
        let mode = opts.unix_mode & 0o7777;
        let mode = mode | ((mode & 0o444) >> 2);
        // Unix directory mode in the high half, MS-DOS directory bit (0x10) in the low.
        let external_attr = ((0o040_000 | mode) << 16) | 0x10;
        let meta = EntryMeta { method: METHOD_STORED, crc: 0, uncompressed: 0, modified: opts.modified, external_attr };
        self.add_entry(&name, &[], meta)
    }

    /// Writes the central directory and returns the archive bytes.
    pub fn finish(mut self) -> Result<Vec<u8>, ZipError> {
        let cd_offset = field32(self.out.len())?;
        for (h, name) in &self.central {
            h.write(&mut self.out);
            self.out.extend_from_slice(name);
        }
        let cd_size = field32(self.out.len() - cd_offset as usize)?;
        field32(self.out.len() + EndOfCentralDirectory::SIZE)?;
        let n = self.central.len() as u16;
        EndOfCentralDirectory {
            disk: 0,
            cd_disk: 0,
            entries_on_disk: n,
            entries_total: n,
            cd_size,
            cd_offset,
            comment_len: 0,
        }
        .write(&mut self.out);
        Ok(self.out)
    }
}

/// One entry of a parsed archive, from its central directory record.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone)]
pub struct ZipArchive<'a> {
    data: &'a [u8],
    entries: Vec<ZipEntry>,
}

/// Finds the end-of-central-directory record: scanning backwards over the
/// last 65557 bytes for a signature whose comment ends exactly at the end
/// of the data.
fn find_eocd(data: &[u8]) -> Result<usize, ZipError> {
    if data.len() < EndOfCentralDirectory::SIZE {
        return Err(ZipError::NotAZip);
    }
    let last = data.len() - EndOfCentralDirectory::SIZE;
    let first = last.saturating_sub(MAX_COMMENT);
    let mut i = last;
    loop {
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

impl<'a> ZipArchive<'a> {
    /// Parses the central directory of `data`.
    pub fn parse(data: &'a [u8]) -> Result<Self, ZipError> {
        let e = find_eocd(data)?;
        let eocd = EndOfCentralDirectory::parse(&data[e..])?;
        if eocd.disk != 0 || eocd.cd_disk != 0 || eocd.entries_on_disk != eocd.entries_total {
            return Err(ZipError::MultiDisk);
        }
        if eocd.entries_total == 0xFFFF || eocd.cd_size == u32::MAX || eocd.cd_offset == u32::MAX {
            return Err(ZipError::Zip64Unsupported);
        }
        let cd_start = eocd.cd_offset as usize;
        let cd_end = cd_start.checked_add(eocd.cd_size as usize).ok_or(ZipError::Truncated)?;
        if cd_end > e {
            return Err(ZipError::Truncated);
        }
        let total = eocd.entries_total as usize;
        let mut entries = Vec::with_capacity(total.min(eocd.cd_size as usize / CentralHeader::SIZE));
        let mut p = cd_start;
        for _ in 0..total {
            let h = CentralHeader::parse(&data[p..cd_end])?;
            let name_start = p + CentralHeader::SIZE;
            let name_end = name_start + h.name_len as usize;
            let next = name_end + h.extra_len as usize + h.comment_len as usize;
            if next > cd_end {
                return Err(ZipError::Truncated);
            }
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

    /// The entries in central-directory order.
    pub fn entries(&self) -> &[ZipEntry] {
        &self.entries
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if the archive has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Index of the entry with this name.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.raw_name == name.as_bytes())
    }

    /// The raw (still compressed) bytes of entry `index`.
    pub fn raw_data(&self, index: usize) -> Result<&'a [u8], ZipError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_read_back() {
        let mut w = ZipWriter::new();
        let opts = FileOptions::default();
        let text = b"hello hello hello hello hello hello".repeat(50);
        w.add_directory("docs", &opts).unwrap();
        w.add_file("docs/a.txt", &text, &opts).unwrap();
        w.add_file("empty", b"", &opts).unwrap();
        w.add_file("caf\u{e9}.bin", &[1, 2, 3], &opts).unwrap();
        assert!(matches!(w.add_file("docs/a.txt", b"x", &opts), Err(ZipError::DuplicateName(_))));
        let zip = w.finish().unwrap();
        let a = ZipArchive::parse(&zip).unwrap();
        assert_eq!(a.len(), 4);
        assert!(a.entries()[0].is_dir());
        assert_eq!(a.entries()[1].method, METHOD_DEFLATED);
        assert_eq!(a.read_by_name("docs/a.txt").unwrap(), text);
        assert_eq!(a.read_by_name("empty").unwrap(), b"");
        assert_eq!(a.read_by_name("caf\u{e9}.bin").unwrap(), [1, 2, 3]);
        assert_eq!(a.entries()[3].flags & FLAG_UTF8, FLAG_UTF8);
        assert_eq!(a.entries()[1].unix_mode, Some(0o100_644));
        assert_eq!(a.entries()[0].unix_mode, Some(0o040_755));
    }

    #[test]
    fn names_are_validated() {
        for bad in ["", "/abs", "a/../b", "..", "a//b", "a\\b", "C:/x", "./a", "a/./b", "/"] {
            assert!(validate_name(bad).is_err(), "{bad:?} accepted");
        }
        for good in ["a", "a/b", "dir/", "a.b/c..d", "...", "ab:c"] {
            assert!(validate_name(good).is_ok(), "{good:?} rejected");
        }
    }

    #[test]
    fn corruption_is_detected() {
        let mut w = ZipWriter::new();
        w.add_file("f", &b"some data some data".repeat(10), &FileOptions::default()).unwrap();
        let zip = w.finish().unwrap();
        let start = LocalHeader::SIZE + 1;
        for i in start..start + 10 {
            let mut bad = zip.clone();
            bad[i] ^= 0x55;
            let a = ZipArchive::parse(&bad).unwrap();
            assert!(a.read(0).is_err(), "flip at {i} undetected");
        }
    }

    #[test]
    fn unix_time_conversion() {
        assert_eq!(DosDateTime::from_unix_seconds(0), DosDateTime::EPOCH);
        assert_eq!(DosDateTime::from_unix_seconds(i64::MAX), DosDateTime::MAX);
        // 2024-02-29 12:34:57 UTC
        let t = DosDateTime::from_unix_seconds(1_709_210_097);
        assert_eq!(t, DosDateTime::new(2024, 2, 29, 12, 34, 56).unwrap());
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    fn any_local() -> LocalHeader {
        LocalHeader {
            version_needed: kani::any(),
            flags: kani::any(),
            method: kani::any(),
            mod_time: kani::any(),
            mod_date: kani::any(),
            crc32: kani::any(),
            compressed_size: kani::any(),
            uncompressed_size: kani::any(),
            name_len: kani::any(),
            extra_len: kani::any(),
        }
    }

    fn any_central() -> CentralHeader {
        CentralHeader {
            version_made_by: kani::any(),
            version_needed: kani::any(),
            flags: kani::any(),
            method: kani::any(),
            mod_time: kani::any(),
            mod_date: kani::any(),
            crc32: kani::any(),
            compressed_size: kani::any(),
            uncompressed_size: kani::any(),
            name_len: kani::any(),
            extra_len: kani::any(),
            comment_len: kani::any(),
            disk_start: kani::any(),
            internal_attr: kani::any(),
            external_attr: kani::any(),
            local_offset: kani::any(),
        }
    }

    /// Local headers serialize to 30 bytes and parse back unchanged.
    #[kani::proof]
    fn local_header_roundtrip() {
        let h = any_local();
        let mut v = Vec::new();
        h.write(&mut v);
        assert_eq!(v.len(), LocalHeader::SIZE);
        assert_eq!(LocalHeader::parse(&v), Ok(h));
    }

    /// Central headers serialize to 46 bytes and parse back unchanged.
    #[kani::proof]
    fn central_header_roundtrip() {
        let h = any_central();
        let mut v = Vec::new();
        h.write(&mut v);
        assert_eq!(v.len(), CentralHeader::SIZE);
        assert_eq!(CentralHeader::parse(&v), Ok(h));
    }

    /// End records serialize to 22 bytes and parse back unchanged.
    #[kani::proof]
    fn eocd_roundtrip() {
        let h = EndOfCentralDirectory {
            disk: kani::any(),
            cd_disk: kani::any(),
            entries_on_disk: kani::any(),
            entries_total: kani::any(),
            cd_size: kani::any(),
            cd_offset: kani::any(),
            comment_len: kani::any(),
        };
        let mut v = Vec::new();
        h.write(&mut v);
        assert_eq!(v.len(), EndOfCentralDirectory::SIZE);
        assert_eq!(EndOfCentralDirectory::parse(&v), Ok(h));
    }

    /// Header parsers never panic on short or arbitrary input.
    #[kani::proof]
    fn header_parsers_never_panic() {
        let b: [u8; 46] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= 46);
        let _ = LocalHeader::parse(&b[..len]);
        let _ = CentralHeader::parse(&b[..len]);
        let _ = EndOfCentralDirectory::parse(&b[..len]);
    }

    /// Locating the end record in any buffer of up to 30 bytes never
    /// panics, and a found record really is a signature whose comment ends
    /// exactly at the end of the buffer.
    #[kani::proof]
    #[kani::unwind(11)]
    fn find_eocd_is_sound() {
        let data: [u8; 30] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= 30);
        let d = &data[..len];
        if let Ok(i) = find_eocd(d) {
            assert_eq!(u32_at(d, i), EOCD_SIG);
            assert_eq!(i + EndOfCentralDirectory::SIZE + u16_at(d, i + 20) as usize, len);
        }
    }

    /// With an end record at the end of a 70-byte buffer and every other
    /// byte arbitrary (directory offset, size and count, the central header
    /// and its name), parsing never panics and any entry lies inside the
    /// central directory.
    #[kani::proof]
    #[kani::unwind(4)]
    fn archive_parse_never_panics() {
        let mut data: [u8; 70] = kani::any();
        data[48..52].copy_from_slice(&EOCD_SIG.to_le_bytes());
        data[68] = 0;
        data[69] = 0;
        if let Ok(a) = ZipArchive::parse(&data) {
            assert!(a.len() <= 1);
            for e in a.entries() {
                assert!(e.raw_name.len() <= 48 - CentralHeader::SIZE);
            }
        }
    }

    /// Valid DOS timestamps survive encoding and decoding.
    #[kani::proof]
    fn dos_datetime_roundtrip() {
        let t = DosDateTime {
            year: 1980 + u16::from(kani::any_where(|&y: &u8| y <= 127)),
            month: kani::any_where(|&m: &u8| (1..=12).contains(&m)),
            day: kani::any_where(|&d: &u8| (1..=31).contains(&d)),
            hour: kani::any_where(|&h: &u8| h <= 23),
            minute: kani::any_where(|&m: &u8| m <= 59),
            second: kani::any_where(|&s: &u8| s <= 58 && s % 2 == 0),
        };
        kani::assume(t.is_valid());
        let (time, date) = t.to_dos();
        assert_eq!(DosDateTime::from_dos(time, date), t);
    }

    /// Every day in the DOS range converts to a valid calendar date and
    /// back to the same day number.
    #[kani::proof]
    fn civil_date_conversion_roundtrip() {
        let lo = days_from_civil(1980, 1, 1);
        let hi = days_from_civil(2107, 12, 31);
        // The DOS range spans fewer than 65536 days.
        let off: u16 = kani::any();
        kani::assume(i64::from(off) <= hi - lo);
        let z = lo + i64::from(off);
        let (y, m, d) = civil_from_days(z);
        assert!(y >= 1980 && y <= 2107);
        assert!(m >= 1 && m <= 12);
        assert!(d >= 1 && d <= days_in_month(y, m));
        assert_eq!(days_from_civil(y, m, d), z);
    }

    /// Any Unix time maps to a valid DOS timestamp.
    #[kani::proof]
    fn unix_time_always_valid() {
        let s: i64 = kani::any();
        assert!(DosDateTime::from_unix_seconds(s).is_valid());
    }

    /// Name validation never panics and rejects every name containing a
    /// `..` component or a leading `/`.
    #[kani::proof]
    #[kani::unwind(8)]
    fn validate_name_rejects_traversal() {
        let b: [u8; 6] = kani::any();
        let Ok(name) = core::str::from_utf8(&b) else { return };
        if validate_name(name).is_ok() {
            assert!(!name.starts_with('/'));
            assert!(!name.split('/').any(|c| c == ".."));
            assert!(!name.contains('\\'));
        }
    }
}
