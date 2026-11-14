//! 7z archives: reading and writing, without encryption.
//!
//! A 7z archive is a 32-byte signature header pointing at a header at the
//! end of the file, and between them packed streams. The header (itself
//! usually LZMA-compressed) describes *folders*: chains of coders that
//! turn packed streams into one output, which holds one or more files
//! back to back (a solid block). Files without data (directories, empty
//! files) take no stream.
//!
//! Reading supports the coders 7-Zip uses by default and for executables:
//! Copy, LZMA, LZMA2, Deflate, Deflate64, BZip2, Delta, the branch filters
//! (x86, PowerPC, IA-64, ARM, ARM-Thumb, SPARC, ARM64, RISC-V) and BCJ2.
//! Encrypted archives and PPMd are reported as unsupported. Every file's
//! CRC is checked, as are the header's, and output is limited.
//!
//! Writing makes one solid LZMA2 folder (or Copy at level 0) with names,
//! modification times and attributes, and an LZMA-compressed header, as
//! 7-Zip does by default.
//!
//! ```
//! use verified_deflate::sevenz::{SevenZArchive, SevenZWriter, EntryOptions};
//!
//! let mut w = SevenZWriter::new();
//! w.add_directory("docs", &EntryOptions::default()).unwrap();
//! w.add_file("docs/hello.txt", b"hello, world", &EntryOptions::default()).unwrap();
//! let bytes = w.finish(5);
//!
//! let archive = SevenZArchive::parse(&bytes).unwrap();
//! assert_eq!(archive.entries()[1].name, "docs/hello.txt");
//! assert_eq!(archive.read(1).unwrap(), b"hello, world");
//! ```

use crate::crc32::crc32;
use crate::filters::{delta_decode, BranchFilter};
use crate::lzma::{LzmaOptions, LzmaProps};
use core::fmt;

/// The six bytes every 7z archive starts with.
pub const SIGNATURE: [u8; 6] = [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C];

const K_END: u64 = 0x00;
const K_HEADER: u64 = 0x01;
const K_ARCHIVE_PROPERTIES: u64 = 0x02;
const K_ADDITIONAL_STREAMS_INFO: u64 = 0x03;
const K_MAIN_STREAMS_INFO: u64 = 0x04;
const K_FILES_INFO: u64 = 0x05;
const K_PACK_INFO: u64 = 0x06;
const K_UNPACK_INFO: u64 = 0x07;
const K_SUBSTREAMS_INFO: u64 = 0x08;
const K_SIZE: u64 = 0x09;
const K_CRC: u64 = 0x0A;
const K_FOLDER: u64 = 0x0B;
const K_CODERS_UNPACK_SIZE: u64 = 0x0C;
const K_NUM_UNPACK_STREAM: u64 = 0x0D;
const K_EMPTY_STREAM: u64 = 0x0E;
const K_EMPTY_FILE: u64 = 0x0F;
const K_ANTI: u64 = 0x10;
const K_NAME: u64 = 0x11;
const K_CTIME: u64 = 0x12;
const K_ATIME: u64 = 0x13;
const K_MTIME: u64 = 0x14;
const K_WIN_ATTRIBUTES: u64 = 0x15;
const K_ENCODED_HEADER: u64 = 0x17;

/// Most items (files, folders, streams, coders) an archive may declare;
/// more is taken as corruption rather than allocated.
const MAX_ITEMS: u64 = 1 << 22;
const MAX_CODERS: u64 = 64;

/// The Windows attribute bits 7-Zip uses.
pub const ATTR_READONLY: u32 = 0x01;
/// Directory.
pub const ATTR_DIRECTORY: u32 = 0x10;
/// Archive (set on ordinary files).
pub const ATTR_ARCHIVE: u32 = 0x20;
/// The high 16 bits hold a Unix mode.
pub const ATTR_UNIX_EXTENSION: u32 = 0x8000;

/// Why a 7z archive could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SevenZError {
    /// The data does not start with the 7z signature.
    NotSevenZ,
    /// The data ended in the middle of a structure.
    Truncated,
    /// The archive is corrupt; the message says how.
    Corrupt(&'static str),
    /// The archive is encrypted.
    Encrypted,
    /// A coder or feature this reader does not support.
    Unsupported(String),
    /// A coder's data is corrupt.
    Codec(String),
    /// A file's or a block's data does not match its CRC.
    CrcMismatch,
    /// Decoding would exceed the output limit.
    OutputLimitExceeded,
    /// No entry with this index.
    NotFound,
    /// An entry name is empty, absolute or contains `..` (writing).
    InvalidName(String),
}

impl fmt::Display for SevenZError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSevenZ => f.write_str("not a 7z archive"),
            Self::Truncated => f.write_str("7z archive is truncated"),
            Self::Corrupt(why) => write!(f, "corrupt 7z archive: {why}"),
            Self::Encrypted => f.write_str("encrypted 7z archives are not supported"),
            Self::Unsupported(what) => write!(f, "unsupported in 7z: {what}"),
            Self::Codec(e) => write!(f, "corrupt compressed data: {e}"),
            Self::CrcMismatch => f.write_str("CRC mismatch"),
            Self::OutputLimitExceeded => f.write_str("decompressed data exceeds the output limit"),
            Self::NotFound => f.write_str("no such entry"),
            Self::InvalidName(n) => write!(f, "invalid entry name {n:?}"),
        }
    }
}

impl std::error::Error for SevenZError {}

fn codec<E: fmt::Display>(e: E) -> SevenZError {
    SevenZError::Codec(e.to_string())
}

// ---------------------------------------------------------------------------
// Header reading

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8, SevenZError> {
        let v = *self.b.get(self.pos).ok_or(SevenZError::Truncated)?;
        self.pos += 1;
        Ok(v)
    }

    fn bytes(&mut self, n: u64) -> Result<&'a [u8], SevenZError> {
        let n = usize::try_from(n).map_err(|_| SevenZError::Truncated)?;
        let end = self.pos.checked_add(n).ok_or(SevenZError::Truncated)?;
        let v = self.b.get(self.pos..end).ok_or(SevenZError::Truncated)?;
        self.pos = end;
        Ok(v)
    }

    fn u32(&mut self) -> Result<u32, SevenZError> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, SevenZError> {
        let b = self.bytes(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    /// 7z's variable-length number: the first byte's leading one bits
    /// count the bytes that follow (little-endian), its remaining bits are
    /// the top of the value.
    fn number(&mut self) -> Result<u64, SevenZError> {
        let first = self.byte()?;
        let mut mask = 0x80u8;
        let mut value = 0u64;
        for i in 0..8 {
            if first & mask == 0 {
                let high = u64::from(first & mask.wrapping_sub(1));
                return Ok(value | (high << (8 * i)));
            }
            value |= u64::from(self.byte()?) << (8 * i);
            mask >>= 1;
        }
        Ok(value)
    }

    /// A count, which may not exceed `MAX_ITEMS`.
    fn count(&mut self) -> Result<usize, SevenZError> {
        let n = self.number()?;
        if n > MAX_ITEMS {
            return Err(SevenZError::Corrupt("count too large"));
        }
        Ok(n as usize)
    }

    fn bits(&mut self, n: usize) -> Result<Vec<bool>, SevenZError> {
        let bytes = self.bytes(n.div_ceil(8) as u64)?;
        Ok((0..n).map(|i| bytes[i / 8] & (0x80 >> (i % 8)) != 0).collect())
    }

    /// An "all defined" byte, or a bit vector.
    fn defined(&mut self, n: usize) -> Result<Vec<bool>, SevenZError> {
        if self.byte()? != 0 {
            Ok(vec![true; n])
        } else {
            self.bits(n)
        }
    }

    fn digests(&mut self, n: usize) -> Result<Vec<Option<u32>>, SevenZError> {
        let defined = self.defined(n)?;
        defined.into_iter().map(|d| if d { self.u32().map(Some) } else { Ok(None) }).collect()
    }

    /// Skips a property's size-prefixed data.
    fn skip_data(&mut self) -> Result<(), SevenZError> {
        let n = self.number()?;
        self.bytes(n).map(|_| ())
    }
}

/// One coder of a folder.
#[derive(Debug, Clone)]
struct Coder {
    /// The method ID, its bytes read as a big-endian number as 7-Zip
    /// does (so an empty ID is 0, Copy).
    id: u64,
    num_in: usize,
    num_out: usize,
    props: Vec<u8>,
}

/// A folder: coders, how their streams connect, and its packed streams.
#[derive(Debug, Clone)]
struct Folder {
    coders: Vec<Coder>,
    /// (in stream, out stream): the in stream is fed by the out stream.
    bind_pairs: Vec<(usize, usize)>,
    /// The in streams that read packed streams, in pack order.
    packed: Vec<usize>,
    /// The size of every out stream.
    unpack_sizes: Vec<u64>,
    crc: Option<u32>,
    /// Index of the folder's first packed stream.
    first_pack: usize,
}

impl Folder {
    /// The out stream no bind pair consumes: the folder's output.
    fn main_out(&self) -> Option<usize> {
        let total: usize = self.coders.iter().map(|c| c.num_out).sum();
        (0..total).find(|o| !self.bind_pairs.iter().any(|&(_, out)| out == *o))
    }

    fn unpack_size(&self) -> u64 {
        self.main_out().and_then(|o| self.unpack_sizes.get(o).copied()).unwrap_or(0)
    }
}

/// An entry of a 7z archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SevenZEntry {
    /// The path, with `/` between components (7-Zip on Windows may store
    /// `\`, which is converted).
    pub name: String,
    /// Whether the entry has data (non-empty files).
    pub has_stream: bool,
    /// Whether it is a directory.
    pub is_dir: bool,
    /// An "anti" item, which deletes this name when the archive is used
    /// as an update.
    pub is_anti: bool,
    /// Uncompressed size.
    pub size: u64,
    /// CRC32 of the data, if recorded.
    pub crc: Option<u32>,
    /// Modification time, as a Windows FILETIME (100 ns since 1601).
    pub mtime: Option<u64>,
    /// Windows attributes; with [`ATTR_UNIX_EXTENSION`], a Unix mode in
    /// the high 16 bits.
    pub attributes: Option<u32>,
    /// The folder (solid block) holding the data, and the offset in it.
    pub folder: Option<(usize, u64)>,
}

impl SevenZEntry {
    /// The Unix mode, if the attributes carry one.
    pub fn unix_mode(&self) -> Option<u32> {
        self.attributes.filter(|a| a & ATTR_UNIX_EXTENSION != 0).map(|a| a >> 16)
    }

    /// Whether the entry is a symbolic link (its data is the target).
    pub fn is_symlink(&self) -> bool {
        self.unix_mode().is_some_and(|m| m & 0o170_000 == 0o120_000)
    }

    /// The modification time in seconds since the Unix epoch.
    pub fn mtime_unix(&self) -> Option<i64> {
        self.mtime.map(|t| (t / 10_000_000) as i64 - 11_644_473_600)
    }
}

/// A parsed 7z archive.
#[derive(Debug, Clone)]
pub struct SevenZArchive<'a> {
    data: &'a [u8],
    /// Offset and size of every packed stream.
    packs: Vec<(u64, u64)>,
    folders: Vec<Folder>,
    entries: Vec<SevenZEntry>,
}

/// What a streams-info block describes.
#[derive(Default)]
struct StreamsInfo {
    pack_pos: u64,
    pack_sizes: Vec<u64>,
    folders: Vec<Folder>,
    /// Every substream: folder, offset, size, CRC.
    substreams: Vec<(usize, u64, u64, Option<u32>)>,
}

fn read_folder(r: &mut Reader<'_>) -> Result<Folder, SevenZError> {
    let num_coders = r.number()?;
    if num_coders == 0 || num_coders > MAX_CODERS {
        return Err(SevenZError::Corrupt("bad number of coders"));
    }
    let mut coders = Vec::new();
    for _ in 0..num_coders {
        let flags = r.byte()?;
        if flags & 0xC0 != 0 {
            return Err(SevenZError::Unsupported("alternative coder methods".into()));
        }
        let id_bytes = r.bytes(u64::from(flags & 0x0F))?;
        if id_bytes.len() > 8 {
            return Err(SevenZError::Unsupported("method ID longer than 8 bytes".into()));
        }
        let id = id_bytes.iter().fold(0u64, |v, &b| v << 8 | u64::from(b));
        let (num_in, num_out) = if flags & 0x10 != 0 {
            let i = r.number()?;
            let o = r.number()?;
            if i > MAX_CODERS || o > MAX_CODERS || i == 0 || o == 0 {
                return Err(SevenZError::Corrupt("bad coder stream counts"));
            }
            (i as usize, o as usize)
        } else {
            (1, 1)
        };
        let props = if flags & 0x20 != 0 {
            let n = r.number()?;
            r.bytes(n)?.to_vec()
        } else {
            Vec::new()
        };
        coders.push(Coder { id, num_in, num_out, props });
    }
    let total_in: usize = coders.iter().map(|c| c.num_in).sum();
    let total_out: usize = coders.iter().map(|c| c.num_out).sum();
    let mut bind_pairs = Vec::new();
    for _ in 0..total_out - 1 {
        let i = r.number()?;
        let o = r.number()?;
        if i >= total_in as u64 || o >= total_out as u64 {
            return Err(SevenZError::Corrupt("bind pair out of range"));
        }
        bind_pairs.push((i as usize, o as usize));
    }
    if total_in < bind_pairs.len() {
        return Err(SevenZError::Corrupt("more bind pairs than streams"));
    }
    let num_packed = total_in - bind_pairs.len();
    let packed = if num_packed == 1 {
        let free = (0..total_in).find(|i| !bind_pairs.iter().any(|&(bi, _)| bi == *i));
        vec![free.ok_or(SevenZError::Corrupt("no packed stream"))?]
    } else {
        let mut v = Vec::new();
        for _ in 0..num_packed {
            let i = r.number()?;
            if i >= total_in as u64 {
                return Err(SevenZError::Corrupt("packed stream out of range"));
            }
            v.push(i as usize);
        }
        v
    };
    Ok(Folder { coders, bind_pairs, packed, unpack_sizes: Vec::new(), crc: None, first_pack: 0 })
}

fn read_streams_info(r: &mut Reader<'_>) -> Result<StreamsInfo, SevenZError> {
    let mut s = StreamsInfo::default();
    let mut id = r.number()?;
    if id == K_PACK_INFO {
        s.pack_pos = r.number()?;
        let n = r.count()?;
        loop {
            id = r.number()?;
            match id {
                K_END => break,
                K_SIZE => {
                    for _ in 0..n {
                        s.pack_sizes.push(r.number()?);
                    }
                }
                K_CRC => {
                    r.digests(n)?;
                }
                _ => r.skip_data()?,
            }
        }
        if s.pack_sizes.len() != n {
            return Err(SevenZError::Corrupt("pack sizes missing"));
        }
        id = r.number()?;
    }
    if id == K_UNPACK_INFO {
        if r.number()? != K_FOLDER {
            return Err(SevenZError::Corrupt("expected folders"));
        }
        let n = r.count()?;
        if r.byte()? != 0 {
            return Err(SevenZError::Unsupported("external folders".into()));
        }
        let mut first_pack = 0;
        for _ in 0..n {
            let mut f = read_folder(r)?;
            f.first_pack = first_pack;
            first_pack += f.packed.len();
            s.folders.push(f);
        }
        if r.number()? != K_CODERS_UNPACK_SIZE {
            return Err(SevenZError::Corrupt("expected unpack sizes"));
        }
        for f in &mut s.folders {
            let outs: usize = f.coders.iter().map(|c| c.num_out).sum();
            for _ in 0..outs {
                f.unpack_sizes.push(r.number()?);
            }
        }
        loop {
            id = r.number()?;
            match id {
                K_END => break,
                K_CRC => {
                    let crcs = r.digests(s.folders.len())?;
                    for (f, c) in s.folders.iter_mut().zip(crcs) {
                        f.crc = c;
                    }
                }
                _ => r.skip_data()?,
            }
        }
        if s.folders.iter().map(|f| f.packed.len()).sum::<usize>() > s.pack_sizes.len() {
            return Err(SevenZError::Corrupt("folders need more packed streams than there are"));
        }
        id = r.number()?;
    }
    // Substreams: by default each folder is one stream.
    let mut counts = vec![1usize; s.folders.len()];
    if id == K_SUBSTREAMS_INFO {
        loop {
            id = r.number()?;
            match id {
                K_NUM_UNPACK_STREAM => {
                    for c in counts.iter_mut() {
                        *c = r.count()?;
                    }
                }
                K_CRC | K_SIZE | K_END => break,
                _ => r.skip_data()?,
            }
        }
        if counts.iter().sum::<usize>() as u64 > MAX_ITEMS {
            return Err(SevenZError::Corrupt("too many streams"));
        }
        let mut sizes: Vec<Vec<u64>> = Vec::new();
        let has_sizes = id == K_SIZE;
        for (f, &n) in s.folders.iter().zip(&counts) {
            let total = f.unpack_size();
            let mut v = Vec::with_capacity(n);
            if n > 0 {
                let mut sum = 0u64;
                if has_sizes {
                    for _ in 1..n {
                        let size = r.number()?;
                        sum = sum.checked_add(size).ok_or(SevenZError::Corrupt("stream sizes overflow"))?;
                        v.push(size);
                    }
                } else if n > 1 {
                    return Err(SevenZError::Corrupt("stream sizes missing"));
                }
                if sum > total {
                    return Err(SevenZError::Corrupt("stream sizes exceed the folder"));
                }
                v.push(total - sum);
            }
            sizes.push(v);
        }
        if has_sizes {
            id = r.number()?;
        }
        // CRCs: those not known from a one-stream folder's CRC.
        let unknown = s.folders.iter().zip(&counts).filter(|(f, &n)| n != 1 || f.crc.is_none()).map(|(_, &n)| n).sum();
        let mut digests: Option<Vec<Option<u32>>> = None;
        loop {
            match id {
                K_END => break,
                K_CRC => digests = Some(r.digests(unknown)?),
                _ => r.skip_data()?,
            }
            id = r.number()?;
        }
        let mut next_digest = digests.into_iter().flatten();
        for (fi, (f, v)) in s.folders.iter().zip(&sizes).enumerate() {
            let known = v.len() == 1 && f.crc.is_some();
            let mut offset = 0;
            for &size in v {
                let crc = if known { f.crc } else { next_digest.next().flatten() };
                s.substreams.push((fi, offset, size, crc));
                offset += size;
            }
        }
        id = r.number()?;
    } else {
        for (fi, f) in s.folders.iter().enumerate() {
            s.substreams.push((fi, 0, f.unpack_size(), f.crc));
        }
    }
    if id != K_END {
        return Err(SevenZError::Corrupt("unexpected property in streams info"));
    }
    Ok(s)
}

fn utf16_names(b: &[u8], n: usize) -> Result<Vec<String>, SevenZError> {
    if n == 0 {
        return Ok(Vec::new());
    }
    if b.len() % 2 != 0 {
        return Err(SevenZError::Corrupt("odd names size"));
    }
    let units: Vec<u16> = b.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let names: Vec<String> =
        units.split(|&u| u == 0).take(n).map(|s| String::from_utf16_lossy(s).replace('\\', "/")).collect();
    if names.len() != n || units.last() != Some(&0) {
        return Err(SevenZError::Corrupt("names do not match the file count"));
    }
    Ok(names)
}

fn read_files_info(r: &mut Reader<'_>, streams: &StreamsInfo) -> Result<Vec<SevenZEntry>, SevenZError> {
    let n = r.count()?;
    if n as u64 > (r.b.len() as u64 + 1) * 8 && n > streams.substreams.len() {
        return Err(SevenZError::Corrupt("more files than the header can describe"));
    }
    let mut empty_stream = vec![false; n];
    let mut empty_file: Vec<bool> = Vec::new();
    let mut anti: Vec<bool> = Vec::new();
    let mut names: Option<Vec<String>> = None;
    let mut mtimes = vec![None; n];
    let mut attributes = vec![None; n];
    loop {
        let id = r.number()?;
        if id == K_END {
            break;
        }
        let size = r.number()?;
        let body = r.bytes(size)?;
        let mut p = Reader { b: body, pos: 0 };
        match id {
            K_EMPTY_STREAM => {
                empty_stream = p.bits(n)?;
                let k = empty_stream.iter().filter(|&&e| e).count();
                empty_file = vec![false; k];
                anti = vec![false; k];
            }
            K_EMPTY_FILE => empty_file = p.bits(empty_file.len())?,
            K_ANTI => anti = p.bits(anti.len())?,
            K_NAME => {
                if p.byte()? != 0 {
                    return Err(SevenZError::Unsupported("external names".into()));
                }
                names = Some(utf16_names(&body[1..], n)?);
            }
            K_MTIME | K_CTIME | K_ATIME => {
                let defined = p.defined(n)?;
                if p.byte()? != 0 {
                    return Err(SevenZError::Unsupported("external times".into()));
                }
                for (i, d) in defined.into_iter().enumerate() {
                    if d {
                        let t = p.u64()?;
                        if id == K_MTIME {
                            mtimes[i] = Some(t);
                        }
                    }
                }
            }
            K_WIN_ATTRIBUTES => {
                let defined = p.defined(n)?;
                if p.byte()? != 0 {
                    return Err(SevenZError::Unsupported("external attributes".into()));
                }
                for (i, d) in defined.into_iter().enumerate() {
                    if d {
                        attributes[i] = Some(p.u32()?);
                    }
                }
            }
            _ => {}
        }
    }
    let with_stream = empty_stream.iter().filter(|&&e| !e).count();
    if with_stream != streams.substreams.len() {
        return Err(SevenZError::Corrupt("files and streams do not match"));
    }
    let names = names.unwrap_or_else(|| (0..n).map(|i| format!("file{i}")).collect());
    let mut entries = Vec::with_capacity(n);
    let mut next_stream = streams.substreams.iter();
    let mut next_empty = 0;
    for i in 0..n {
        let (has_stream, is_dir, is_anti, size, crc, folder) = if empty_stream[i] {
            let k = next_empty;
            next_empty += 1;
            let is_file = empty_file.get(k).copied().unwrap_or(false);
            (false, !is_file, anti.get(k).copied().unwrap_or(false), 0, None, None)
        } else {
            let &(f, off, size, crc) =
                next_stream.next().ok_or(SevenZError::Corrupt("files and streams do not match"))?;
            (true, false, false, size, crc, Some((f, off)))
        };
        let is_dir = is_dir || attributes[i].is_some_and(|a| a & ATTR_DIRECTORY != 0) && !has_stream;
        entries.push(SevenZEntry {
            name: names[i].clone(),
            has_stream,
            is_dir,
            is_anti,
            size,
            crc,
            mtime: mtimes[i],
            attributes: attributes[i],
            folder,
        });
    }
    Ok(entries)
}

impl<'a> SevenZArchive<'a> {
    /// Parses an archive held in memory, decoding its header if it is
    /// compressed (at most `1 << 26` bytes of header).
    pub fn parse(data: &'a [u8]) -> Result<Self, SevenZError> {
        if data.len() < 32 {
            return Err(if SIGNATURE.starts_with(&data[..data.len().min(6)]) {
                SevenZError::Truncated
            } else {
                SevenZError::NotSevenZ
            });
        }
        if data[..6] != SIGNATURE {
            return Err(SevenZError::NotSevenZ);
        }
        if data[6] != 0 {
            return Err(SevenZError::Unsupported(format!("format version {}.{}", data[6], data[7])));
        }
        let mut r = Reader { b: data, pos: 8 };
        let start_crc = r.u32()?;
        if crc32(&data[12..32]) != start_crc {
            return Err(SevenZError::Corrupt("start header CRC mismatch"));
        }
        let next_offset = r.u64()?;
        let next_size = r.u64()?;
        let next_crc = r.u32()?;
        let mut archive = Self { data, packs: Vec::new(), folders: Vec::new(), entries: Vec::new() };
        if next_size == 0 {
            return Ok(archive);
        }
        let start = 32u64.checked_add(next_offset).ok_or(SevenZError::Truncated)?;
        let end = start.checked_add(next_size).ok_or(SevenZError::Truncated)?;
        if end > data.len() as u64 {
            return Err(SevenZError::Truncated);
        }
        let header = &data[start as usize..end as usize];
        if crc32(header) != next_crc {
            return Err(SevenZError::Corrupt("header CRC mismatch"));
        }
        let mut owned: Vec<u8>;
        let mut header: &[u8] = header;
        for _ in 0..4 {
            let mut r = Reader { b: header, pos: 0 };
            match r.number()? {
                K_HEADER => {
                    archive.read_header(&mut r)?;
                    return Ok(archive);
                }
                K_ENCODED_HEADER => {
                    let s = read_streams_info(&mut r)?;
                    let tmp = Self::with_streams(data, &s)?;
                    if tmp.folders.is_empty() {
                        return Err(SevenZError::Corrupt("encoded header has no data"));
                    }
                    owned = tmp.decode_folder(0, 1 << 26)?;
                    header = &owned;
                }
                _ => return Err(SevenZError::Corrupt("bad header")),
            }
        }
        Err(SevenZError::Corrupt("header encoded too many times"))
    }

    fn with_streams(data: &'a [u8], s: &StreamsInfo) -> Result<Self, SevenZError> {
        let mut packs = Vec::with_capacity(s.pack_sizes.len());
        let mut pos = 32u64.checked_add(s.pack_pos).ok_or(SevenZError::Truncated)?;
        for &size in &s.pack_sizes {
            packs.push((pos, size));
            pos = pos.checked_add(size).ok_or(SevenZError::Truncated)?;
        }
        if pos > data.len() as u64 {
            return Err(SevenZError::Truncated);
        }
        Ok(Self { data, packs, folders: s.folders.clone(), entries: Vec::new() })
    }

    fn read_header(&mut self, r: &mut Reader<'_>) -> Result<(), SevenZError> {
        let mut id = r.number()?;
        if id == K_ARCHIVE_PROPERTIES {
            loop {
                if r.number()? == K_END {
                    break;
                }
                r.skip_data()?;
            }
            id = r.number()?;
        }
        if id == K_ADDITIONAL_STREAMS_INFO {
            return Err(SevenZError::Unsupported("additional streams".into()));
        }
        let mut streams = StreamsInfo::default();
        if id == K_MAIN_STREAMS_INFO {
            streams = read_streams_info(r)?;
            let s = Self::with_streams(self.data, &streams)?;
            self.packs = s.packs;
            self.folders = s.folders;
            id = r.number()?;
        }
        if id == K_FILES_INFO {
            self.entries = read_files_info(r, &streams)?;
            id = r.number()?;
        } else if !streams.substreams.is_empty() {
            return Err(SevenZError::Corrupt("streams without files"));
        }
        if id != K_END {
            return Err(SevenZError::Corrupt("unexpected property in header"));
        }
        Ok(())
    }

    /// The entries, in archive order.
    pub fn entries(&self) -> &[SevenZEntry] {
        &self.entries
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the archive has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of folders (solid blocks).
    pub fn folder_count(&self) -> usize {
        self.folders.len()
    }

    /// A folder's methods, as 7-Zip names them (`"LZMA2:24"`,
    /// `"BCJ LZMA:23"`), outermost first.
    pub fn folder_methods(&self, f: usize) -> String {
        let Some(folder) = self.folders.get(f) else { return String::new() };
        folder.coders.iter().rev().map(method_name).collect::<Vec<_>>().join(" ")
    }

    /// The packed size of folder `f`'s streams.
    pub fn folder_packed_size(&self, f: usize) -> u64 {
        self.folders.get(f).map_or(0, |folder| {
            (0..folder.packed.len()).filter_map(|k| self.packs.get(folder.first_pack + k)).map(|p| p.1).sum()
        })
    }

    /// Decodes folder `f` whole, checking its CRC if it has one, producing
    /// at most `max_output` bytes.
    pub fn decode_folder(&self, f: usize, max_output: usize) -> Result<Vec<u8>, SevenZError> {
        let folder = self.folders.get(f).ok_or(SevenZError::NotFound)?;
        let mut packs: Vec<&[u8]> = Vec::with_capacity(folder.packed.len());
        for k in 0..folder.packed.len() {
            let &(pos, size) =
                self.packs.get(folder.first_pack + k).ok_or(SevenZError::Corrupt("missing packed stream"))?;
            packs.push(&self.data[pos as usize..(pos + size) as usize]);
        }
        let main = folder.main_out().ok_or(SevenZError::Corrupt("folder has no output"))?;
        let out = decode_out(folder, main, &packs, max_output, 0)?;
        if folder.crc.is_some_and(|c| c != crc32(&out)) {
            return Err(SevenZError::CrcMismatch);
        }
        Ok(out)
    }

    /// The data of entry `i`, checked against its CRC. Decodes its whole
    /// folder; to extract many entries of one solid block, decode the
    /// folder once with [`decode_folder`](Self::decode_folder) and use
    /// [`entry_data`](Self::entry_data).
    pub fn read(&self, i: usize) -> Result<Vec<u8>, SevenZError> {
        let e = self.entries.get(i).ok_or(SevenZError::NotFound)?;
        match e.folder {
            None => Ok(Vec::new()),
            Some((f, _)) => {
                let out = self.decode_folder(f, usize::MAX)?;
                self.entry_data(i, &out).map(<[u8]>::to_vec)
            }
        }
    }

    /// Entry `i`'s bytes within its folder's decoded output, checked
    /// against its CRC.
    pub fn entry_data<'o>(&self, i: usize, folder_output: &'o [u8]) -> Result<&'o [u8], SevenZError> {
        let e = self.entries.get(i).ok_or(SevenZError::NotFound)?;
        let Some((_, off)) = e.folder else { return Ok(&[]) };
        let end = off.checked_add(e.size).ok_or(SevenZError::Corrupt("entry past its folder"))?;
        let d = folder_output.get(off as usize..end as usize).ok_or(SevenZError::Corrupt("entry past its folder"))?;
        if e.crc.is_some_and(|c| c != crc32(d)) {
            return Err(SevenZError::CrcMismatch);
        }
        Ok(d)
    }
}

fn method_name(c: &Coder) -> String {
    match c.id {
        0x00 => "Copy".into(),
        0x21 => match c.props.first().and_then(|&b| crate::lzma2::dict_size_from_byte(b)) {
            Some(d) => format!("LZMA2:{}", dict_name(d)),
            None => "LZMA2".into(),
        },
        0x03_0101 => match c.props.get(1..5) {
            Some(d) => format!("LZMA:{}", dict_name(u32::from_le_bytes([d[0], d[1], d[2], d[3]]))),
            None => "LZMA".into(),
        },
        0x03 => format!("Delta:{}", c.props.first().map_or(1, |&d| u32::from(d) + 1)),
        0x04_0108 => "Deflate".into(),
        0x04_0109 => "Deflate64".into(),
        0x04_0202 => "BZip2".into(),
        0x0303_0103 => "BCJ".into(),
        0x0303_011B => "BCJ2".into(),
        0x0303_0205 => "PPC".into(),
        0x0303_0401 => "IA64".into(),
        0x0303_0501 => "ARM".into(),
        0x0303_0701 => "ARMT".into(),
        0x0303_0805 => "SPARC".into(),
        0x0A => "ARM64".into(),
        0x0B => "RISCV".into(),
        0x03_0401 => "PPMD".into(),
        0x04F7_1101 => "ZSTD".into(),
        0x06F1_0701 => "7zAES".into(),
        id => format!("{id:X}"),
    }
}

/// A dictionary size as 7-Zip prints it: the power of two, or a size.
fn dict_name(d: u32) -> String {
    if d.is_power_of_two() {
        d.trailing_zeros().to_string()
    } else if d % (1 << 20) == 0 {
        format!("{}m", d >> 20)
    } else if d % 1024 == 0 {
        format!("{}k", d >> 10)
    } else {
        format!("{d}b")
    }
}

/// Decodes out stream `out` of `folder`.
fn decode_out(
    folder: &Folder,
    out: usize,
    packs: &[&[u8]],
    max_output: usize,
    depth: usize,
) -> Result<Vec<u8>, SevenZError> {
    if depth > folder.coders.len() {
        return Err(SevenZError::Corrupt("coders bound in a cycle"));
    }
    let size = *folder.unpack_sizes.get(out).ok_or(SevenZError::Corrupt("missing unpack size"))?;
    let size = usize::try_from(size).map_err(|_| SevenZError::OutputLimitExceeded)?;
    if size > max_output {
        return Err(SevenZError::OutputLimitExceeded);
    }
    // The coder that produces this out stream, and its first in stream.
    let (mut first_out, mut first_in) = (0, 0);
    let mut owner = None;
    for (ci, c) in folder.coders.iter().enumerate() {
        if out < first_out + c.num_out {
            owner = Some(ci);
            break;
        }
        first_out += c.num_out;
        first_in += c.num_in;
    }
    let ci = owner.ok_or(SevenZError::Corrupt("out stream without a coder"))?;
    let coder = &folder.coders[ci];
    if coder.num_out != 1 {
        return Err(SevenZError::Unsupported("coder with several outputs".into()));
    }
    let mut owned: Vec<Vec<u8>> = Vec::new();
    let mut sources: Vec<Result<usize, usize>> = Vec::new(); // Ok(owned index) or Err(pack index)
    for j in 0..coder.num_in {
        let in_index = first_in + j;
        if let Some(&(_, bound)) = folder.bind_pairs.iter().find(|&&(i, _)| i == in_index) {
            owned.push(decode_out(folder, bound, packs, max_output, depth + 1)?);
            sources.push(Ok(owned.len() - 1));
        } else {
            let k = folder
                .packed
                .iter()
                .position(|&p| p == in_index)
                .ok_or(SevenZError::Corrupt("unconnected in stream"))?;
            sources.push(Err(k));
        }
    }
    let inputs: Vec<&[u8]> = sources
        .iter()
        .map(|s| match *s {
            Ok(i) => Ok(owned[i].as_slice()),
            Err(k) => packs.get(k).copied().ok_or(SevenZError::Corrupt("missing packed stream")),
        })
        .collect::<Result<_, _>>()?;
    let result = run_coder(coder, &inputs, size)?;
    if result.len() != size {
        return Err(SevenZError::Codec(format!("{} produced {} bytes, not {size}", method_name(coder), result.len())));
    }
    Ok(result)
}

/// Runs one coder's decoder, producing `size` bytes.
fn run_coder(c: &Coder, inputs: &[&[u8]], size: usize) -> Result<Vec<u8>, SevenZError> {
    let one = || -> Result<&[u8], SevenZError> {
        if inputs.len() == 1 {
            Ok(inputs[0])
        } else {
            Err(SevenZError::Corrupt("coder has the wrong number of inputs"))
        }
    };
    let branch = |f: BranchFilter| -> Result<Vec<u8>, SevenZError> {
        let mut buf = one()?.to_vec();
        let start = match c.props.len() {
            0 => 0,
            4 => u32::from_le_bytes([c.props[0], c.props[1], c.props[2], c.props[3]]),
            _ => return Err(SevenZError::Unsupported("branch filter properties".into())),
        };
        if start % f.alignment() != 0 {
            return Err(SevenZError::Unsupported("misaligned branch filter start".into()));
        }
        f.decode(&mut buf, start);
        Ok(buf)
    };
    match c.id {
        0x00 => {
            let input = one()?;
            input.get(..size).map(<[u8]>::to_vec).ok_or(SevenZError::Truncated)
        }
        0x21 => {
            let dict = c
                .props
                .first()
                .and_then(|&b| crate::lzma2::dict_size_from_byte(b))
                .ok_or_else(|| SevenZError::Unsupported("LZMA2 properties".into()))?;
            crate::lzma2::decompress_with_limit(one()?, dict, size).map(|d| d.data).map_err(codec)
        }
        0x03_0101 => {
            let p: &[u8; 5] =
                c.props.as_slice().try_into().map_err(|_| SevenZError::Unsupported("LZMA properties".into()))?;
            let props = LzmaProps::parse(p).map_err(codec)?;
            crate::lzma::decompress_raw(one()?, &props, Some(size as u64), size).map(|d| d.data).map_err(codec)
        }
        0x04_0108 => crate::inflate::decompress_with_limit(one()?, size).map(|d| d.data).map_err(codec),
        0x04_0109 => crate::inflate::decompress_deflate64_with_limit(one()?, size).map(|d| d.data).map_err(codec),
        0x04_0202 => crate::bzip2::decompress_with_limit(one()?, size).map_err(codec),
        0x03 => {
            let dist =
                usize::from(*c.props.first().ok_or_else(|| SevenZError::Unsupported("Delta properties".into()))?) + 1;
            let mut buf = one()?.to_vec();
            delta_decode(&mut buf, dist);
            Ok(buf)
        }
        0x0303_0103 => branch(BranchFilter::X86),
        0x0303_0205 => branch(BranchFilter::PowerPc),
        0x0303_0401 => branch(BranchFilter::Ia64),
        0x0303_0501 => branch(BranchFilter::Arm),
        0x0303_0701 => branch(BranchFilter::ArmThumb),
        0x0303_0805 => branch(BranchFilter::Sparc),
        0x0A => branch(BranchFilter::Arm64),
        0x0B => branch(BranchFilter::RiscV),
        0x0303_011B => {
            if inputs.len() != 4 {
                return Err(SevenZError::Corrupt("BCJ2 needs four inputs"));
            }
            bcj2_decode(inputs[0], inputs[1], inputs[2], inputs[3], size)
        }
        0x06F1_0701 => Err(SevenZError::Encrypted),
        _ => Err(SevenZError::Unsupported(format!("method {}", method_name(c)))),
    }
}

/// 7-Zip's BCJ2 decoder: x86 calls and jumps whose targets were moved to
/// two streams, with a range-coded stream saying which were.
fn bcj2_decode(main: &[u8], call: &[u8], jump: &[u8], rc: &[u8], size: usize) -> Result<Vec<u8>, SevenZError> {
    const TOP: u32 = 1 << 24;
    let mut probs = [1024u16; 258];
    let mut out = Vec::with_capacity(size);
    if rc.len() < 5 {
        return Err(SevenZError::Corrupt("BCJ2 range coder stream too short"));
    }
    let mut code = u32::from_be_bytes([rc[1], rc[2], rc[3], rc[4]]);
    let mut range = u32::MAX;
    let mut rc_pos = 5;
    let (mut call_pos, mut jump_pos, mut main_pos) = (0, 0, 0);
    let mut prev = 0u8;
    if size == 0 {
        return Ok(out);
    }
    loop {
        // Copy bytes up to and including the next E8/E9/Jcc opcode.
        let mut found = false;
        while main_pos < main.len() && out.len() < size {
            let b = main[main_pos];
            main_pos += 1;
            out.push(b);
            if (b & 0xFE) == 0xE8 || (prev == 0x0F && (b & 0xF0) == 0x80) {
                found = true;
                break;
            }
            prev = b;
        }
        if !found || out.len() == size {
            break;
        }
        let b = out[out.len() - 1];
        let p = &mut probs[if b == 0xE8 {
            usize::from(prev)
        } else if b == 0xE9 {
            256
        } else {
            257
        }];
        let bound = (range >> 11) * u32::from(*p);
        let taken = if code < bound {
            range = bound;
            *p += (2048 - *p) >> 5;
            false
        } else {
            range -= bound;
            code -= bound;
            *p -= *p >> 5;
            true
        };
        if range < TOP {
            let next = *rc.get(rc_pos).ok_or(SevenZError::Corrupt("BCJ2 range coder stream ended"))?;
            rc_pos += 1;
            range <<= 8;
            code = (code << 8) | u32::from(next);
        }
        if !taken {
            prev = b;
            continue;
        }
        let src = if b == 0xE8 { &mut call_pos } else { &mut jump_pos };
        let stream = if b == 0xE8 { call } else { jump };
        let v = stream.get(*src..*src + 4).ok_or(SevenZError::Corrupt("BCJ2 address stream ended"))?;
        *src += 4;
        let dest = u32::from_be_bytes([v[0], v[1], v[2], v[3]]).wrapping_sub(out.len() as u32 + 4);
        for (k, byte) in dest.to_le_bytes().into_iter().enumerate() {
            if out.len() == size {
                break;
            }
            out.push(byte);
            if k == 3 {
                prev = byte;
            }
        }
        if out.len() == size {
            break;
        }
    }
    if out.len() != size {
        return Err(SevenZError::Corrupt("BCJ2 output is short"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Writing

/// Per-entry settings for [`SevenZWriter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EntryOptions {
    /// Modification time, in seconds since the Unix epoch.
    pub mtime: Option<i64>,
    /// Unix permission bits (and type bits), stored as 7-Zip does on
    /// Unix: in the attributes' high 16 bits.
    pub unix_mode: Option<u32>,
}

impl EntryOptions {
    fn filetime(&self) -> Option<u64> {
        self.mtime.and_then(|t| u64::try_from((t + 11_644_473_600) * 10_000_000).ok())
    }

    fn attributes(&self, kind_bits: u32, base: u32) -> u32 {
        match self.unix_mode {
            Some(m) => {
                let mode = kind_bits | (m & 0o7777);
                base | ATTR_UNIX_EXTENSION | (mode << 16) | if m & 0o200 == 0 { ATTR_READONLY } else { 0 }
            }
            None => base,
        }
    }
}

struct WriteItem {
    name: String,
    data: Option<Vec<u8>>,
    is_dir: bool,
    mtime: Option<u64>,
    attributes: u32,
}

/// Builds a 7z archive in memory.
#[derive(Default)]
pub struct SevenZWriter {
    items: Vec<WriteItem>,
}

fn check_name(name: &str) -> Result<(), SevenZError> {
    crate::zip::validate_name(name).map_err(|_| SevenZError::InvalidName(name.to_string()))
}

impl SevenZWriter {
    /// Starts an empty archive.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file.
    pub fn add_file(&mut self, name: &str, data: &[u8], opts: &EntryOptions) -> Result<(), SevenZError> {
        check_name(name)?;
        self.items.push(WriteItem {
            name: name.to_string(),
            data: Some(data.to_vec()),
            is_dir: false,
            mtime: opts.filetime(),
            attributes: opts.attributes(0o100_000, ATTR_ARCHIVE),
        });
        Ok(())
    }

    /// Adds a directory (a trailing `/` is dropped).
    pub fn add_directory(&mut self, name: &str, opts: &EntryOptions) -> Result<(), SevenZError> {
        let name = name.trim_end_matches('/');
        check_name(name)?;
        self.items.push(WriteItem {
            name: name.to_string(),
            data: None,
            is_dir: true,
            mtime: opts.filetime(),
            attributes: opts.attributes(0o040_000, ATTR_DIRECTORY),
        });
        Ok(())
    }

    /// Adds a symbolic link, as 7-Zip stores one on Unix: a file whose
    /// data is the target, with the link type in its mode.
    pub fn add_symlink(&mut self, name: &str, target: &str, opts: &EntryOptions) -> Result<(), SevenZError> {
        check_name(name)?;
        let opts = EntryOptions { unix_mode: Some(opts.unix_mode.unwrap_or(0o777)), ..*opts };
        self.items.push(WriteItem {
            name: name.to_string(),
            data: Some(target.as_bytes().to_vec()),
            is_dir: false,
            mtime: opts.filetime(),
            attributes: opts.attributes(0o120_000, ATTR_ARCHIVE),
        });
        Ok(())
    }

    /// Adds an entry exactly as `archive` has it (name, data, time and
    /// attributes), given its data.
    pub fn add_entry(&mut self, entry: &SevenZEntry, data: &[u8]) {
        self.items.push(WriteItem {
            name: entry.name.clone(),
            data: (!entry.is_dir).then(|| data.to_vec()),
            is_dir: entry.is_dir,
            mtime: entry.mtime,
            attributes: entry.attributes.unwrap_or(if entry.is_dir { ATTR_DIRECTORY } else { ATTR_ARCHIVE }),
        });
    }

    /// Writes the archive: every file's data in one solid block,
    /// compressed with LZMA2 at `level` 1–9 (stored with Copy at 0), and
    /// the header compressed with LZMA.
    pub fn finish(self, level: u8) -> Vec<u8> {
        self.finish_with(level, true)
    }

    /// [`finish`](Self::finish), choosing whether to compress the header
    /// (7-Zip's `-mhc`).
    pub fn finish_with(self, level: u8, compress_header: bool) -> Vec<u8> {
        let mut out = vec![0u8; 32];
        out[..6].copy_from_slice(&SIGNATURE);
        out[7] = 4;
        let streams: Vec<&[u8]> =
            self.items.iter().filter_map(|i| i.data.as_deref()).filter(|d| !d.is_empty()).collect();
        let mut header = vec![K_HEADER as u8];
        if !streams.is_empty() {
            let solid: Vec<u8> = streams.concat();
            let (coder, packed) = if level == 0 {
                (vec![0x01, 0x00], solid.clone())
            } else {
                let mut opts = LzmaOptions::level(level);
                opts.props.dict_size = opts.fitted_dict(solid.len());
                let b = crate::lzma2::dict_size_byte(opts.props.dict_size);
                opts.props.dict_size = crate::lzma2::dict_size_from_byte(b).unwrap_or(u32::MAX);
                (vec![0x21, 0x21, 0x01, b], crate::lzma2::compress(&solid, &opts))
            };
            out.extend_from_slice(&packed);
            header.push(K_MAIN_STREAMS_INFO as u8);
            write_streams_info(&mut header, 0, packed.len() as u64, &coder, solid.len() as u64, None);
            // Substreams: count, sizes but the last, CRCs.
            header.pop(); // reopen the streams info before its end
            header.push(K_SUBSTREAMS_INFO as u8);
            header.push(K_NUM_UNPACK_STREAM as u8);
            write_number(&mut header, streams.len() as u64);
            if streams.len() > 1 {
                header.push(K_SIZE as u8);
                for s in &streams[..streams.len() - 1] {
                    write_number(&mut header, s.len() as u64);
                }
            }
            header.push(K_CRC as u8);
            header.push(1);
            for s in &streams {
                header.extend_from_slice(&crc32(s).to_le_bytes());
            }
            header.push(K_END as u8);
            header.push(K_END as u8);
        }
        if !self.items.is_empty() {
            write_files_info(&mut header, &self.items);
        }
        header.push(K_END as u8);

        if !compress_header {
            return finish_signature(out, &header);
        }
        // The header goes out compressed with LZMA, as 7-Zip writes it.
        let header_pos = (out.len() - 32) as u64;
        let mut lzma = LzmaOptions::level(5);
        lzma.props.dict_size = lzma.fitted_dict(header.len()).max(1 << 16);
        let packed = crate::lzma::compress_raw(&header, &lzma, false);
        let mut coder = vec![0x23, 0x03, 0x01, 0x01, 0x05];
        coder.extend_from_slice(&lzma.props.to_bytes());
        out.extend_from_slice(&packed);
        let mut encoded = vec![K_ENCODED_HEADER as u8];
        write_streams_info(
            &mut encoded,
            header_pos,
            packed.len() as u64,
            &coder,
            header.len() as u64,
            Some(crc32(&header)),
        );

        finish_signature(out, &encoded)
    }
}

/// Appends the (possibly encoded) header and fills in the signature
/// header that points at it.
fn finish_signature(mut out: Vec<u8>, header: &[u8]) -> Vec<u8> {
    let next_offset = (out.len() - 32) as u64;
    out.extend_from_slice(header);
    out[12..20].copy_from_slice(&next_offset.to_le_bytes());
    out[20..28].copy_from_slice(&(header.len() as u64).to_le_bytes());
    out[28..32].copy_from_slice(&crc32(header).to_le_bytes());
    let c = crc32(&out[12..32]);
    out[8..12].copy_from_slice(&c.to_le_bytes());
    out
}

fn write_number(out: &mut Vec<u8>, v: u64) {
    // The first byte: as many leading ones as bytes follow, then the top
    // bits of the value.
    let mut n = 0;
    while n < 8 && v >= 1u64 << (7 * (n + 1)) {
        n += 1;
    }
    if n == 8 {
        out.push(0xFF);
        out.extend_from_slice(&v.to_le_bytes());
        return;
    }
    let high = (v >> (8 * n)) as u8;
    let lead = !(0xFFu8 >> n);
    out.push(lead | high);
    out.extend_from_slice(&v.to_le_bytes()[..n]);
}

/// A streams info with one packed stream and one one-coder folder, ending
/// with `kEnd`.
fn write_streams_info(out: &mut Vec<u8>, pack_pos: u64, pack_size: u64, coder: &[u8], unpack: u64, crc: Option<u32>) {
    out.push(K_PACK_INFO as u8);
    write_number(out, pack_pos);
    out.push(1);
    out.push(K_SIZE as u8);
    write_number(out, pack_size);
    out.push(K_END as u8);
    out.push(K_UNPACK_INFO as u8);
    out.push(K_FOLDER as u8);
    out.push(1);
    out.push(0);
    out.push(1); // one coder
    out.extend_from_slice(coder);
    out.push(K_CODERS_UNPACK_SIZE as u8);
    write_number(out, unpack);
    if let Some(c) = crc {
        out.push(K_CRC as u8);
        out.push(1);
        out.extend_from_slice(&c.to_le_bytes());
    }
    out.push(K_END as u8);
    out.push(K_END as u8);
}

fn bit_vector(bits: &[bool]) -> Vec<u8> {
    let mut v = vec![0u8; bits.len().div_ceil(8)];
    for (i, &b) in bits.iter().enumerate() {
        if b {
            v[i / 8] |= 0x80 >> (i % 8);
        }
    }
    v
}

fn write_property(out: &mut Vec<u8>, id: u64, body: &[u8]) {
    write_number(out, id);
    write_number(out, body.len() as u64);
    out.extend_from_slice(body);
}

fn write_files_info(out: &mut Vec<u8>, items: &[WriteItem]) {
    out.push(K_FILES_INFO as u8);
    write_number(out, items.len() as u64);
    let empty: Vec<bool> = items.iter().map(|i| i.data.as_ref().is_none_or(Vec::is_empty)).collect();
    if empty.iter().any(|&e| e) {
        write_property(out, K_EMPTY_STREAM, &bit_vector(&empty));
        let empty_files: Vec<bool> =
            items.iter().filter(|i| i.data.as_ref().is_none_or(Vec::is_empty)).map(|i| !i.is_dir).collect();
        if empty_files.iter().any(|&f| f) {
            write_property(out, K_EMPTY_FILE, &bit_vector(&empty_files));
        }
    }
    let mut names = vec![0u8];
    for i in items {
        for u in i.name.encode_utf16() {
            names.extend_from_slice(&u.to_le_bytes());
        }
        names.extend_from_slice(&[0, 0]);
    }
    write_property(out, K_NAME, &names);
    if items.iter().any(|i| i.mtime.is_some()) {
        let defined: Vec<bool> = items.iter().map(|i| i.mtime.is_some()).collect();
        let mut body = Vec::new();
        if defined.iter().all(|&d| d) {
            body.push(1);
        } else {
            body.push(0);
            body.extend(bit_vector(&defined));
        }
        body.push(0);
        for t in items.iter().filter_map(|i| i.mtime) {
            body.extend_from_slice(&t.to_le_bytes());
        }
        write_property(out, K_MTIME, &body);
    }
    let mut body = vec![1, 0];
    for i in items {
        body.extend_from_slice(&i.attributes.to_le_bytes());
    }
    write_property(out, K_WIN_ATTRIBUTES, &body);
    out.push(K_END as u8);
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Every 64-bit number round-trips through 7z's variable-length form,
    /// using exactly the bytes written (one to nine).
    #[kani::proof]
    #[kani::unwind(10)]
    fn numbers_round_trip() {
        let v: u64 = kani::any();
        let mut b = Vec::new();
        write_number(&mut b, v);
        assert!(!b.is_empty() && b.len() <= 9);
        let mut r = Reader { b: &b, pos: 0 };
        assert_eq!(r.number(), Ok(v));
        assert_eq!(r.pos, b.len());
    }

    /// Reading a number never panics and stays within the data.
    #[kani::proof]
    #[kani::unwind(10)]
    fn numbers_are_read_safely() {
        let b: [u8; 9] = kani::any();
        let n: usize = kani::any_where(|&n: &usize| n <= 9);
        let mut r = Reader { b: &b[..n], pos: 0 };
        if r.number().is_ok() {
            assert!(r.pos <= n);
        }
    }

    /// Bit vectors round-trip, most significant bit first.
    #[kani::proof]
    #[kani::unwind(14)]
    fn bit_vectors_round_trip() {
        let bits: [bool; 12] = kani::any();
        let n: usize = kani::any_where(|&n: &usize| n <= 12);
        let v = bit_vector(&bits[..n]);
        assert_eq!(v.len(), n.div_ceil(8));
        let mut r = Reader { b: &v, pos: 0 };
        assert_eq!(r.bits(n).unwrap().as_slice(), &bits[..n]);
    }

    /// Modification times round-trip through FILETIME, from 1601 to the
    /// year 30000. (64-bit multiplication and division are slow to check
    /// symbolically; the unit test checks the ends of the range and a
    /// sample between.)
    #[cfg(any(feature = "kani-extended", kani_sample))] // too large for CI runners
    #[kani::proof]
    fn times_round_trip() {
        let t: i64 = kani::any_where(|&t: &i64| (-11_644_473_600..=883_000_000_000).contains(&t));
        let o = EntryOptions { mtime: Some(t), unix_mode: None };
        let e = SevenZEntry {
            name: String::new(),
            has_stream: false,
            is_dir: false,
            is_anti: false,
            size: 0,
            crc: None,
            mtime: o.filetime(),
            attributes: None,
            folder: None,
        };
        assert_eq!(e.mtime_unix(), Some(t));
    }

    /// BCJ2 decoding never panics on any small streams and produces exactly
    /// the size asked for, or an error.
    #[kani::proof]
    #[kani::unwind(7)]
    fn bcj2_never_panics() {
        let main: [u8; 3] = kani::any();
        let call: [u8; 4] = kani::any();
        let jump: [u8; 4] = kani::any();
        let rc: [u8; 6] = kani::any();
        let size: usize = kani::any_where(|&s: &usize| s <= 5);
        if let Ok(out) = bcj2_decode(&main, &call, &jump, &rc, size) {
            assert_eq!(out.len(), size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Modification times round-trip through FILETIME: the ends of the
    /// range (1601 to the year 30000), the seconds around them and around
    /// 1970, and a million in between.
    #[test]
    fn times_round_trip() {
        let check = |t: i64| {
            let o = EntryOptions { mtime: Some(t), unix_mode: None };
            let e = SevenZEntry {
                name: String::new(),
                has_stream: false,
                is_dir: false,
                is_anti: false,
                size: 0,
                crc: None,
                mtime: o.filetime(),
                attributes: None,
                folder: None,
            };
            assert_eq!(e.mtime_unix(), Some(t));
        };
        let (lo, hi) = (-11_644_473_600i64, 883_000_000_000i64);
        for d in 0..1000 {
            check(lo + d);
            check(hi - d);
            check(d - 500);
        }
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..1_000_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            check(lo + (x % (hi - lo + 1) as u64) as i64);
        }
    }

    #[test]
    fn numbers() {
        for v in [0u64, 1, 0x7F, 0x80, 0x3FFF, 0x4000, 0x1F_FFFF, 0x20_0000, u64::from(u32::MAX), 1 << 56, u64::MAX] {
            let mut b = Vec::new();
            write_number(&mut b, v);
            let mut r = Reader { b: &b, pos: 0 };
            assert_eq!(r.number().unwrap(), v, "{v:#x}");
            assert_eq!(r.pos, b.len(), "{v:#x}");
        }
        assert_eq!(Reader { b: &[0x81, 0x02], pos: 0 }.number().unwrap(), 0x102);
    }

    fn sample(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 % 251) as u8).collect()
    }

    #[test]
    fn round_trips() {
        for level in [0, 1, 5, 9] {
            let opts = EntryOptions { mtime: Some(1_700_000_000), unix_mode: Some(0o644) };
            let mut w = SevenZWriter::new();
            w.add_directory("dir", &opts).unwrap();
            w.add_file("dir/a.txt", b"hello hello hello", &opts).unwrap();
            w.add_file("empty", b"", &opts).unwrap();
            w.add_file("big", &sample(300_000), &EntryOptions::default()).unwrap();
            w.add_symlink("link", "dir/a.txt", &opts).unwrap();
            w.add_file("名前.txt", b"unicode", &opts).unwrap();
            let bytes = w.finish(level);
            let a = SevenZArchive::parse(&bytes).unwrap();
            let names: Vec<&str> = a.entries().iter().map(|e| e.name.as_str()).collect();
            assert_eq!(names, ["dir", "dir/a.txt", "empty", "big", "link", "名前.txt"]);
            assert!(a.entries()[0].is_dir && !a.entries()[2].is_dir && !a.entries()[2].has_stream);
            assert_eq!(a.read(1).unwrap(), b"hello hello hello");
            assert_eq!(a.read(2).unwrap(), b"");
            assert_eq!(a.read(3).unwrap(), sample(300_000));
            assert!(a.entries()[4].is_symlink());
            assert_eq!(a.read(4).unwrap(), b"dir/a.txt");
            assert_eq!(a.entries()[1].mtime_unix(), Some(1_700_000_000));
            assert_eq!(a.entries()[1].unix_mode(), Some(0o100_644));
            assert_eq!(a.folder_count(), 1);
            let solid = a.decode_folder(0, usize::MAX).unwrap();
            assert_eq!(a.entry_data(5, &solid).unwrap(), b"unicode");
        }
        // A plain header reads the same.
        let mut w = SevenZWriter::new();
        w.add_file("a", b"plain header", &EntryOptions::default()).unwrap();
        w.add_directory("d", &EntryOptions::default()).unwrap();
        let plain = w.finish_with(5, false);
        let a = SevenZArchive::parse(&plain).unwrap();
        assert_eq!((a.len(), a.read(0).unwrap()), (2, b"plain header".to_vec()));
        // Empty archive, and one with only a directory.
        let a = SevenZWriter::new().finish(5);
        assert!(SevenZArchive::parse(&a).unwrap().is_empty());
        let mut w = SevenZWriter::new();
        w.add_directory("d/", &EntryOptions::default()).unwrap();
        let a = w.finish(5);
        let a = SevenZArchive::parse(&a).unwrap();
        assert_eq!((a.len(), a.entries()[0].is_dir, a.folder_count()), (1, true, 0));
    }

    #[test]
    fn errors() {
        let mut w = SevenZWriter::new();
        w.add_file("f", &sample(10_000), &EntryOptions::default()).unwrap();
        let bytes = w.finish(5);
        assert_eq!(SevenZArchive::parse(b"PK\x03\x04").unwrap_err(), SevenZError::NotSevenZ);
        for cut in [10, 32, bytes.len() - 1] {
            assert!(SevenZArchive::parse(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        for i in [9, 20, 40, bytes.len() - 5] {
            let mut bad = bytes.clone();
            bad[i] ^= 1;
            let r = SevenZArchive::parse(&bad).and_then(|a| a.read(0));
            assert!(r.is_err(), "flip at {i}");
        }
        assert!(w_invalid("../x").is_err() && w_invalid("/abs").is_err());
    }

    fn w_invalid(name: &str) -> Result<(), SevenZError> {
        SevenZWriter::new().add_file(name, b"", &EntryOptions::default())
    }
}
