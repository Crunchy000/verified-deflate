//! Tar archives: reading POSIX ustar, pax (extended and global headers)
//! and GNU (long names, base-256 numbers) archives, and writing ustar with
//! pax extended headers when a name or number does not fit.
//!
//! The reader works on an in-memory archive, like [`ZipArchive`]: entries
//! are listed up front and their data is borrowed from the input. Names
//! are returned as stored; [`safe_path`] turns one into a relative path
//! that cannot leave the directory it is extracted into.
//!
//! ```
//! use verified_deflate::tar::{TarArchive, TarOptions, TarWriter};
//!
//! let mut w = TarWriter::new();
//! w.add_directory("docs/", &TarOptions::default()).unwrap();
//! w.add_file("docs/a.txt", b"hello", &TarOptions::default()).unwrap();
//! let bytes = w.finish();
//!
//! let tar = TarArchive::parse(&bytes).unwrap();
//! assert_eq!(tar.len(), 2);
//! assert_eq!(tar.data(1), b"hello");
//! ```
//!
//! [`ZipArchive`]: crate::zip::ZipArchive

use crate::zip::validate_name;
use core::fmt;

const BLOCK: usize = 512;

/// Why a tar archive could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TarError {
    /// The data ended inside a header or an entry's data.
    Truncated,
    /// A header's checksum does not match it (`offset` is the header's
    /// position): the data is corrupt or not a tar archive.
    BadChecksum {
        /// Offset of the header in the archive.
        offset: usize,
    },
    /// A numeric header field is not a valid octal or base-256 number.
    BadNumber {
        /// Offset of the header in the archive.
        offset: usize,
    },
    /// A pax extended header is malformed.
    BadPaxHeader {
        /// Offset of the header in the archive.
        offset: usize,
    },
    /// The entry name is empty, absolute, or contains `..`, backslashes or
    /// NUL (writing only; see [`validate_name`]).
    InvalidName(String),
    /// A symbolic link target is empty or contains NUL (writing only).
    InvalidLinkTarget(String),
}

impl fmt::Display for TarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("tar archive is truncated"),
            Self::BadChecksum { offset } => write!(f, "tar header checksum mismatch at offset {offset}"),
            Self::BadNumber { offset } => write!(f, "invalid number in tar header at offset {offset}"),
            Self::BadPaxHeader { offset } => write!(f, "malformed pax header at offset {offset}"),
            Self::InvalidName(n) => write!(f, "invalid entry name {n:?}"),
            Self::InvalidLinkTarget(t) => write!(f, "invalid link target {t:?}"),
        }
    }
}

impl std::error::Error for TarError {}

/// What an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EntryKind {
    /// A regular file (type `0`, NUL or `7`).
    File,
    /// A hard link to an earlier entry (type `1`).
    HardLink,
    /// A symbolic link (type `2`).
    Symlink,
    /// A character device (type `3`).
    CharDevice,
    /// A block device (type `4`).
    BlockDevice,
    /// A directory (type `5`).
    Directory,
    /// A FIFO (type `6`).
    Fifo,
    /// Any other type, such as a GNU sparse file (`S`); its data, if any,
    /// is available but its meaning is not interpreted.
    Other(u8),
}

/// One entry of a tar archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TarEntry {
    /// The path as stored (from a pax `path` record, a GNU long name, or
    /// the ustar prefix and name).
    pub path: Vec<u8>,
    /// What the entry is.
    pub kind: EntryKind,
    /// Permission bits (and set-ID/sticky bits).
    pub mode: u32,
    /// Owner's numeric user ID.
    pub uid: u64,
    /// Owner's numeric group ID.
    pub gid: u64,
    /// Owner's user name, if recorded.
    pub uname: Vec<u8>,
    /// Owner's group name, if recorded.
    pub gname: Vec<u8>,
    /// Modification time, in whole seconds since the Unix epoch.
    pub mtime: i64,
    /// Size of the entry's data.
    pub size: u64,
    /// Target of a symbolic or hard link.
    pub link_target: Vec<u8>,
    /// Device numbers of a device entry.
    pub device: (u32, u32),
    /// Offset of the entry's data in the archive.
    pub data_offset: usize,
    /// Offset of the entry's first header (counting pax and GNU long-name
    /// headers) in the archive.
    pub header_offset: usize,
}

impl TarEntry {
    /// The path as text (invalid UTF-8 replaced).
    pub fn path_lossy(&self) -> String {
        String::from_utf8_lossy(&self.path).into_owned()
    }

    /// True for directories, including old-style entries whose name ends
    /// in `/`.
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Directory
    }
}

/// A tar archive parsed from a byte slice.
#[derive(Debug, Clone)]
pub struct TarArchive<'a> {
    data: &'a [u8],
    entries: Vec<TarEntry>,
    end: usize,
}

/// Parses an octal or GNU base-256 number field.
fn number(field: &[u8], offset: usize) -> Result<u64, TarError> {
    if let Some((&first, rest)) = field.split_first() {
        if first & 0x80 != 0 {
            // GNU base-256: big-endian, the first byte's top bit a marker
            // (0x80 positive; 0xff negative, which no field here allows).
            if first == 0xff {
                return Err(TarError::BadNumber { offset });
            }
            let mut v = u64::from(first & 0x7f);
            for &b in rest {
                v = v
                    .checked_mul(256)
                    .and_then(|v| v.checked_add(u64::from(b)))
                    .ok_or(TarError::BadNumber { offset })?;
            }
            return Ok(v);
        }
    }
    let mut v: u64 = 0;
    let mut i = 0;
    while i < field.len() && field[i] == b' ' {
        i += 1;
    }
    while i < field.len() && (b'0'..=b'7').contains(&field[i]) {
        v = v
            .checked_mul(8)
            .and_then(|v| v.checked_add(u64::from(field[i] - b'0')))
            .ok_or(TarError::BadNumber { offset })?;
        i += 1;
    }
    // The number ends at NUL or space (or the end of the field); an empty
    // field reads as 0, as GNU tar and 7-Zip read it.
    if field[i..].iter().any(|&b| b != 0 && b != b' ') {
        return Err(TarError::BadNumber { offset });
    }
    Ok(v)
}

/// The bytes of `field` up to its first NUL.
fn text(field: &[u8]) -> &[u8] {
    match field.iter().position(|&b| b == 0) {
        Some(n) => &field[..n],
        None => field,
    }
}

/// True if the header's checksum matches: the sum of its bytes with the
/// checksum field read as spaces, unsigned (POSIX) or signed (some old
/// writers).
fn checksum_ok(h: &[u8], offset: usize) -> Result<bool, TarError> {
    let stored = number(&h[148..156], offset)?;
    let (mut unsigned, mut signed) = (0u64, 0i64);
    for (i, &b) in h.iter().enumerate() {
        let b = if (148..156).contains(&i) { b' ' } else { b };
        unsigned += u64::from(b);
        signed += i64::from(b as i8);
    }
    Ok(stored == unsigned || i64::try_from(stored).is_ok_and(|s| s == signed))
}

/// Overrides from pax records.
#[derive(Default, Clone)]
struct Pax {
    path: Option<Vec<u8>>,
    linkpath: Option<Vec<u8>>,
    size: Option<u64>,
    mtime: Option<i64>,
    uid: Option<u64>,
    gid: Option<u64>,
    uname: Option<Vec<u8>>,
    gname: Option<Vec<u8>>,
}

impl Pax {
    /// Applies the records in `body` (`"%d %s=%s\n"` each).
    fn apply(&mut self, body: &[u8], offset: usize) -> Result<(), TarError> {
        let bad = TarError::BadPaxHeader { offset };
        let mut rest = body;
        while !rest.is_empty() {
            if rest.iter().all(|&b| b == 0) {
                break; // padding
            }
            let sp = rest.iter().position(|&b| b == b' ').ok_or(bad.clone())?;
            let len: usize = std::str::from_utf8(&rest[..sp]).ok().and_then(|s| s.parse().ok()).ok_or(bad.clone())?;
            if len <= sp + 1 || len > rest.len() || rest[len - 1] != b'\n' {
                return Err(bad);
            }
            let record = &rest[sp + 1..len - 1];
            let eq = record.iter().position(|&b| b == b'=').ok_or(bad.clone())?;
            let (key, value) = (&record[..eq], &record[eq + 1..]);
            let num = |v: &[u8]| std::str::from_utf8(v).ok().and_then(|s| s.parse::<u64>().ok()).ok_or(bad.clone());
            match key {
                b"path" => self.path = Some(value.to_vec()),
                b"linkpath" => self.linkpath = Some(value.to_vec()),
                b"size" => self.size = Some(num(value)?),
                b"uid" => self.uid = Some(num(value)?),
                b"gid" => self.gid = Some(num(value)?),
                b"uname" => self.uname = Some(value.to_vec()),
                b"gname" => self.gname = Some(value.to_vec()),
                b"mtime" => {
                    // Seconds, possibly negative and with a fraction.
                    let s = std::str::from_utf8(value).map_err(|_| bad.clone())?;
                    let whole = s.split('.').next().unwrap_or("");
                    self.mtime = Some(whole.parse::<i64>().map_err(|_| bad.clone())?);
                }
                _ => {} // atime, ctime, charset, comment, vendor records
            }
            rest = &rest[len..];
        }
        Ok(())
    }

    /// `self` with `local`'s records taking precedence.
    fn overlay(&self, local: &Pax) -> Pax {
        Pax {
            path: local.path.clone().or_else(|| self.path.clone()),
            linkpath: local.linkpath.clone().or_else(|| self.linkpath.clone()),
            size: local.size.or(self.size),
            mtime: local.mtime.or(self.mtime),
            uid: local.uid.or(self.uid),
            gid: local.gid.or(self.gid),
            uname: local.uname.clone().or_else(|| self.uname.clone()),
            gname: local.gname.clone().or_else(|| self.gname.clone()),
        }
    }
}

/// `offset + len` rounded up to a whole block, within `limit`.
fn skip_blocks(offset: usize, len: u64, limit: usize) -> Result<usize, TarError> {
    let len = usize::try_from(len).map_err(|_| TarError::Truncated)?;
    let end = offset.checked_add(len).ok_or(TarError::Truncated)?;
    if end > limit {
        return Err(TarError::Truncated);
    }
    let padded = end.checked_add(BLOCK - 1).ok_or(TarError::Truncated)? / BLOCK * BLOCK;
    Ok(padded.min(limit))
}

impl<'a> TarArchive<'a> {
    /// Parses every header of `data`. Reading stops at the end-of-archive
    /// marker (a zero block) or at the end of the data.
    pub fn parse(data: &'a [u8]) -> Result<Self, TarError> {
        let mut entries = Vec::new();
        let mut global = Pax::default();
        let mut local = Pax::default();
        let mut long_name: Option<Vec<u8>> = None;
        let mut long_link: Option<Vec<u8>> = None;
        let mut header_start: Option<usize> = None;
        let mut p = 0;
        while p < data.len() {
            let h = data.get(p..p + BLOCK).ok_or(TarError::Truncated)?;
            if h.iter().all(|&b| b == 0) {
                break;
            }
            if !checksum_ok(h, p)? {
                return Err(TarError::BadChecksum { offset: p });
            }
            let first = *header_start.get_or_insert(p);
            let typeflag = h[156];
            let header_size = number(&h[124..136], p)?;
            let body = p + BLOCK;
            match typeflag {
                b'x' | b'g' | b'L' | b'K' => {
                    let len = usize::try_from(header_size).map_err(|_| TarError::Truncated)?;
                    let end = body.checked_add(len).ok_or(TarError::Truncated)?;
                    let content = data.get(body..end).ok_or(TarError::Truncated)?;
                    match typeflag {
                        b'x' => local.apply(content, p)?,
                        b'g' => global.apply(content, p)?,
                        b'L' => long_name = Some(text(content).to_vec()),
                        _ => long_link = Some(text(content).to_vec()),
                    }
                    p = skip_blocks(body, header_size, data.len())?;
                    continue;
                }
                _ => {}
            }
            let pax = global.overlay(&local);
            let ustar = &h[257..263] == b"ustar\0";
            let path = if let Some(path) = pax.path {
                path
            } else if let Some(name) = long_name.take() {
                name
            } else {
                let name = text(&h[0..100]);
                let prefix = if ustar { text(&h[345..500]) } else { &[][..] };
                if prefix.is_empty() {
                    name.to_vec()
                } else {
                    [prefix, b"/", name].concat()
                }
            };
            let link_target = pax.linkpath.or(long_link.take()).unwrap_or_else(|| text(&h[157..257]).to_vec());
            let kind = match typeflag {
                b'0' | 0 | b'7' if path.ends_with(b"/") => EntryKind::Directory,
                b'0' | 0 | b'7' => EntryKind::File,
                b'1' => EntryKind::HardLink,
                b'2' => EntryKind::Symlink,
                b'3' => EntryKind::CharDevice,
                b'4' => EntryKind::BlockDevice,
                b'5' => EntryKind::Directory,
                b'6' => EntryKind::Fifo,
                t => EntryKind::Other(t),
            };
            let size = pax.size.unwrap_or(header_size);
            // Links and directories have no data, whatever the size field
            // says (POSIX); everything else is followed by `size` bytes.
            let has_data = !matches!(
                kind,
                EntryKind::HardLink
                    | EntryKind::Symlink
                    | EntryKind::Directory
                    | EntryKind::CharDevice
                    | EntryKind::BlockDevice
                    | EntryKind::Fifo
            );
            let size = if has_data { size } else { 0 };
            let mtime = match pax.mtime {
                Some(t) => t,
                None => i64::try_from(number(&h[136..148], p)?).map_err(|_| TarError::BadNumber { offset: p })?,
            };
            let entry = TarEntry {
                path,
                kind,
                mode: (number(&h[100..108], p)? & 0o7777_7777) as u32,
                uid: pax.uid.map_or_else(|| number(&h[108..116], p), Ok)?,
                gid: pax.gid.map_or_else(|| number(&h[116..124], p), Ok)?,
                uname: pax.uname.unwrap_or_else(|| text(&h[265..297]).to_vec()),
                gname: pax.gname.unwrap_or_else(|| text(&h[297..329]).to_vec()),
                mtime,
                size,
                link_target,
                device: if ustar || &h[257..263] == b"ustar " {
                    (number(&h[329..337], p)? as u32, number(&h[337..345], p)? as u32)
                } else {
                    (0, 0)
                },
                data_offset: body,
                header_offset: first,
            };
            p = skip_blocks(body, size, data.len())?;
            entries.push(entry);
            local = Pax::default();
            header_start = None;
        }
        Ok(Self { data, entries, end: p })
    }

    /// The entries, in archive order.
    pub fn entries(&self) -> &[TarEntry] {
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

    /// Bytes of the archive up to the end-of-archive marker (or its end).
    pub fn archive_len(&self) -> usize {
        self.end
    }

    /// The data of entry `index` (empty for directories and links).
    ///
    /// Panics if `index` is out of range.
    pub fn data(&self, index: usize) -> &'a [u8] {
        let e = &self.entries[index];
        // `parse` checked that the data lies within the archive.
        &self.data[e.data_offset..e.data_offset + e.size as usize]
    }

    /// Index of the last entry with this path (a later entry replaces an
    /// earlier one of the same name when extracting, as in tar itself).
    pub fn index_of(&self, path: &str) -> Option<usize> {
        self.entries.iter().rposition(|e| e.path == path.as_bytes())
    }
}

/// True if `data` starts with a tar header: a non-zero 512-byte block
/// whose checksum matches. (How `file` and 7-Zip recognise tar, which has
/// no signature: old archives lack the `ustar` magic.)
pub fn is_tar(data: &[u8]) -> bool {
    match data.get(..BLOCK) {
        Some(h) => !h.iter().all(|&b| b == 0) && checksum_ok(h, 0).unwrap_or(false),
        None => false,
    }
}

/// Turns a stored path into a safe relative one, as tar and 7-Zip do when
/// extracting: drops leading `/` and `./` components and empty ones, and
/// refuses (`None`) a path with a `..` component, a NUL, or nothing left.
pub fn safe_path(path: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(path).ok()?;
    let mut parts = Vec::new();
    for c in s.split('/') {
        match c {
            "" | "." => {}
            ".." => return None,
            c if c.contains('\0') => return None,
            c => parts.push(c),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Metadata for entries written by [`TarWriter`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TarOptions {
    /// Permission bits (default `0o644` for files, `0o755` for
    /// directories and `0o777` for links when left at 0).
    pub mode: u32,
    /// Modification time, seconds since the Unix epoch.
    pub mtime: i64,
    /// Owner's numeric user ID.
    pub uid: u64,
    /// Owner's numeric group ID.
    pub gid: u64,
    /// Owner's user name (may be empty).
    pub uname: String,
    /// Owner's group name (may be empty).
    pub gname: String,
}

/// Builds a tar archive in memory (POSIX ustar; pax extended headers for
/// names, link targets and numbers that do not fit).
#[derive(Debug, Default)]
pub struct TarWriter {
    out: Vec<u8>,
}

/// Writes `v` as zero-padded octal filling `field` but its last byte (NUL),
/// returning false if it does not fit.
fn put_octal(field: &mut [u8], v: u64) -> bool {
    let digits = field.len() - 1;
    if digits < 22 && v >= 1u64 << (3 * digits) {
        return false;
    }
    let mut x = v;
    for d in field[..digits].iter_mut().rev() {
        *d = b'0' + (x & 7) as u8;
        x >>= 3;
    }
    field[digits] = 0;
    true
}

/// A pax record `"<len> <key>=<value>\n"`, where `<len>` counts itself.
fn pax_record(key: &str, value: &[u8]) -> Vec<u8> {
    let body = key.len() + value.len() + 3; // space, '=', newline
    let mut len = body + 1;
    while len != body + len.to_string().len() {
        len = body + len.to_string().len();
    }
    let mut r = format!("{len} {key}=").into_bytes();
    r.extend_from_slice(value);
    r.push(b'\n');
    r
}

/// Splits `path` into a ustar (prefix, name) pair, if it fits.
fn split_ustar(path: &[u8]) -> Option<(&[u8], &[u8])> {
    if path.len() <= 100 {
        return Some((&[], path));
    }
    // The prefix must end at a '/', which is not stored.
    let lo = path.len().saturating_sub(101);
    (lo..path.len().min(156)).rev().find(|&i| path[i] == b'/' && i <= 155 && path.len() - i - 1 <= 100).and_then(|i| {
        let name = &path[i + 1..];
        (!name.is_empty()).then_some((&path[..i], name))
    })
}

impl TarWriter {
    /// An empty archive.
    pub fn new() -> Self {
        Self::default()
    }

    fn entry(&mut self, path: &str, typeflag: u8, link: &[u8], data: &[u8], opts: &TarOptions, default_mode: u32) {
        let path = path.as_bytes();
        let mut pax = Vec::new();
        let (prefix, name) = match split_ustar(path) {
            Some(pn) => pn,
            None => {
                pax.extend(pax_record("path", path));
                (&[][..], &path[..path.len().min(100)])
            }
        };
        if link.len() > 100 {
            pax.extend(pax_record("linkpath", link));
        }
        let mut h = [0u8; BLOCK];
        h[..name.len()].copy_from_slice(name);
        put_octal(&mut h[100..108], u64::from(if opts.mode == 0 { default_mode } else { opts.mode & 0o7777 }));
        if !put_octal(&mut h[108..116], opts.uid) {
            pax.extend(pax_record("uid", opts.uid.to_string().as_bytes()));
            put_octal(&mut h[108..116], 0);
        }
        if !put_octal(&mut h[116..124], opts.gid) {
            pax.extend(pax_record("gid", opts.gid.to_string().as_bytes()));
            put_octal(&mut h[116..124], 0);
        }
        let size = data.len() as u64;
        if !put_octal(&mut h[124..136], size) {
            pax.extend(pax_record("size", size.to_string().as_bytes()));
            put_octal(&mut h[124..136], 0);
        }
        match u64::try_from(opts.mtime).ok().filter(|&t| put_octal(&mut h[136..148], t)) {
            Some(_) => {}
            None => {
                pax.extend(pax_record("mtime", opts.mtime.to_string().as_bytes()));
                put_octal(&mut h[136..148], 0);
            }
        }
        h[156] = typeflag;
        let short_link = &link[..link.len().min(100)];
        h[157..157 + short_link.len()].copy_from_slice(short_link);
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        for (field, value) in [(265..297, &opts.uname), (297..329, &opts.gname)] {
            let v = value.as_bytes();
            if v.len() < 32 {
                h[field.start..field.start + v.len()].copy_from_slice(v);
            } else {
                pax.extend(pax_record(if field.start == 265 { "uname" } else { "gname" }, v));
            }
        }
        h[345..345 + prefix.len()].copy_from_slice(prefix);
        if !pax.is_empty() {
            // The extended header's own name, as GNU tar forms it.
            let base = path.rsplit(|&b| b == b'/').find(|c| !c.is_empty()).unwrap_or(b"entry");
            let pax_name = [&b"PaxHeaders/"[..], &base[..base.len().min(89)]].concat();
            let pax_opts = TarOptions { mode: 0o644, mtime: opts.mtime.max(0), ..TarOptions::default() };
            self.entry_raw(&pax_name, b'x', &pax, &pax_opts);
        }
        self.finish_header(&mut h);
        self.out.extend_from_slice(&h);
        self.out.extend_from_slice(data);
        self.pad();
    }

    /// A plain header for a pax extended header's body.
    fn entry_raw(&mut self, name: &[u8], typeflag: u8, body: &[u8], opts: &TarOptions) {
        let mut h = [0u8; BLOCK];
        h[..name.len()].copy_from_slice(name);
        put_octal(&mut h[100..108], u64::from(opts.mode));
        put_octal(&mut h[108..116], 0);
        put_octal(&mut h[116..124], 0);
        put_octal(&mut h[124..136], body.len() as u64);
        put_octal(&mut h[136..148], u64::try_from(opts.mtime).unwrap_or(0) & 0o777_7777_7777);
        h[156] = typeflag;
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        self.finish_header(&mut h);
        self.out.extend_from_slice(&h);
        self.out.extend_from_slice(body);
        self.pad();
    }

    fn finish_header(&self, h: &mut [u8; BLOCK]) {
        h[148..156].copy_from_slice(b"        ");
        let sum: u64 = h.iter().map(|&b| u64::from(b)).sum();
        // Six digits and a NUL, then the space; the sum of 512 bytes fits.
        put_octal(&mut h[148..155], sum);
        h[155] = b' ';
    }

    fn pad(&mut self) {
        let n = self.out.len().next_multiple_of(BLOCK);
        self.out.resize(n, 0);
    }

    /// Adds a regular file.
    pub fn add_file(&mut self, path: &str, data: &[u8], opts: &TarOptions) -> Result<(), TarError> {
        if validate_name(path).is_err() || path.ends_with('/') {
            return Err(TarError::InvalidName(path.to_string()));
        }
        self.entry(path, b'0', &[], data, opts, 0o644);
        Ok(())
    }

    /// Adds a directory; a `/` is appended to the name if missing.
    pub fn add_directory(&mut self, path: &str, opts: &TarOptions) -> Result<(), TarError> {
        let path = if path.ends_with('/') { path.to_string() } else { format!("{path}/") };
        if validate_name(&path).is_err() {
            return Err(TarError::InvalidName(path));
        }
        self.entry(&path, b'5', &[], &[], opts, 0o755);
        Ok(())
    }

    /// Adds a symbolic link to `target` (stored as given).
    pub fn add_symlink(&mut self, path: &str, target: &str, opts: &TarOptions) -> Result<(), TarError> {
        if validate_name(path).is_err() || path.ends_with('/') {
            return Err(TarError::InvalidName(path.to_string()));
        }
        if target.is_empty() || target.contains('\0') {
            return Err(TarError::InvalidLinkTarget(target.to_string()));
        }
        self.entry(path, b'2', target.as_bytes(), &[], opts, 0o777);
        Ok(())
    }

    /// Adds a hard link to an earlier entry `target`.
    pub fn add_hard_link(&mut self, path: &str, target: &str, opts: &TarOptions) -> Result<(), TarError> {
        if validate_name(path).is_err() || path.ends_with('/') {
            return Err(TarError::InvalidName(path.to_string()));
        }
        if validate_name(target).is_err() {
            return Err(TarError::InvalidLinkTarget(target.to_string()));
        }
        self.entry(path, b'1', target.as_bytes(), &[], opts, 0o644);
        Ok(())
    }

    /// Copies entry `index` of `archive` as it is: its headers (pax and
    /// GNU long-name ones too) and data. For updating an archive, or
    /// deleting entries from one.
    pub fn copy_entry(&mut self, archive: &TarArchive<'_>, index: usize) {
        let e = &archive.entries[index];
        let end = (e.data_offset + e.size as usize).next_multiple_of(BLOCK).min(archive.data.len());
        self.out.extend_from_slice(&archive.data[e.header_offset..end]);
        self.pad();
    }

    /// Writes the end-of-archive marker (two zero blocks) and returns the
    /// archive.
    pub fn finish(mut self) -> Vec<u8> {
        self.out.extend_from_slice(&[0; 2 * BLOCK]);
        self.out
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Octal fields hold exactly the numbers that fit, and read back.
    #[kani::proof]
    #[kani::unwind(13)]
    fn octal_numbers_round_trip() {
        let v: u64 = kani::any();
        let mut f8 = [0u8; 8];
        assert_eq!(put_octal(&mut f8, v), v < 1 << 21);
        if v < 1 << 21 {
            assert_eq!(number(&f8, 0), Ok(v));
        }
        let mut f12 = [0u8; 12];
        assert_eq!(put_octal(&mut f12, v), v < 1 << 33);
        if v < 1 << 33 {
            assert_eq!(number(&f12, 0), Ok(v));
        }
    }

    /// Base-256 fields read as their big-endian value, whatever it is;
    /// numbers past 64 bits are errors, not wrapped.
    #[kani::proof]
    #[kani::unwind(13)]
    fn base256_numbers_read_exactly() {
        let v: u64 = kani::any();
        let mut f = [0u8; 12];
        f[0] = 0x80;
        f[4..].copy_from_slice(&v.to_be_bytes());
        assert_eq!(number(&f, 0), Ok(v));
        let high: u8 = kani::any_where(|&b: &u8| b != 0);
        f[3] = high;
        assert!(number(&f, 0).is_err());
    }

    /// Number fields never panic, whatever they hold.
    #[kani::proof]
    #[kani::unwind(13)]
    fn numbers_never_panic() {
        let f: [u8; 12] = kani::any();
        let n: usize = kani::any_where(|&n: &usize| n <= 12);
        let _ = number(&f[..n], 0);
    }

    /// The writer's header checksum is one the reader accepts, for any
    /// header contents.
    #[kani::proof]
    #[kani::unwind(520)]
    fn written_checksums_check() {
        let mut h: [u8; BLOCK] = kani::any();
        TarWriter::new().finish_header(&mut h);
        assert_eq!(checksum_ok(&h, 0), Ok(true));
    }

    /// Paths made safe stay relative and never climb: for every path of
    /// up to six bytes from `a`, `/`, `.` and NUL, the result has no empty,
    /// `.` or `..` component and no NUL, and only paths with `..` or a NUL
    /// or nothing but separators are refused.
    #[kani::proof]
    #[kani::unwind(8)]
    fn safe_paths_stay_inside() {
        let raw: [u8; 6] = kani::any();
        let raw = raw.map(|b| [b'a', b'/', b'.', 0][usize::from(b % 4)]);
        let n: usize = kani::any_where(|&n: &usize| n <= 6);
        let path = &raw[..n];
        let parts: Vec<&[u8]> = path.split(|&b| b == b'/').collect();
        let climbs = parts.iter().any(|p| *p == b"..") || path.contains(&0);
        let empty = parts.iter().all(|p| p.is_empty() || *p == b".");
        match safe_path(path) {
            Some(s) => {
                assert!(!climbs && !empty);
                for c in s.split('/') {
                    assert!(!c.is_empty() && c != "." && c != ".." && !c.contains('\0'));
                }
            }
            None => assert!(climbs || empty),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> TarOptions {
        TarOptions { mode: 0o640, mtime: 1_700_000_000, uid: 1000, gid: 100, uname: "u".into(), gname: "g".into() }
    }

    #[test]
    fn write_and_read_back() {
        let mut w = TarWriter::new();
        w.add_directory("d", &opts()).unwrap();
        w.add_file("d/a.txt", b"hello", &opts()).unwrap();
        w.add_file("empty", b"", &TarOptions::default()).unwrap();
        w.add_symlink("d/link", "../empty", &opts()).unwrap();
        w.add_hard_link("hard", "d/a.txt", &opts()).unwrap();
        let bytes = w.finish();
        assert_eq!(bytes.len() % BLOCK, 0);
        let t = TarArchive::parse(&bytes).unwrap();
        assert_eq!(t.len(), 5);
        let e = &t.entries()[1];
        assert_eq!((e.path.as_slice(), e.kind, e.mode, e.size), (&b"d/a.txt"[..], EntryKind::File, 0o640, 5));
        assert_eq!((e.uid, e.gid, e.mtime), (1000, 100, 1_700_000_000));
        assert_eq!((e.uname.as_slice(), e.gname.as_slice()), (&b"u"[..], &b"g"[..]));
        assert_eq!(t.data(1), b"hello");
        assert_eq!(t.entries()[0].path, b"d/");
        assert!(t.entries()[0].is_dir());
        assert_eq!(t.entries()[2].mode, 0o644);
        assert_eq!(
            (t.entries()[3].kind, t.entries()[3].link_target.as_slice()),
            (EntryKind::Symlink, &b"../empty"[..])
        );
        assert_eq!(t.entries()[4].kind, EntryKind::HardLink);
        assert_eq!(t.index_of("d/a.txt"), Some(1));
        assert_eq!(t.archive_len(), bytes.len() - 2 * BLOCK);
    }

    #[test]
    fn long_names_and_big_numbers_use_pax() {
        let long = format!("{}/{}", "x".repeat(150), "y".repeat(120));
        let target = "t".repeat(300);
        let o = TarOptions { uid: 1 << 40, mtime: -5, uname: "n".repeat(40), ..opts() };
        let mut w = TarWriter::new();
        w.add_file(&long, b"data", &o).unwrap();
        w.add_symlink("s", &target, &o).unwrap();
        // A name that fits as prefix + name needs no pax header.
        let split = format!("{}/{}", "p".repeat(150), "n".repeat(90));
        w.add_file(&split, b"", &opts()).unwrap();
        let bytes = w.finish();
        let t = TarArchive::parse(&bytes).unwrap();
        assert_eq!(t.len(), 3);
        let e = &t.entries()[0];
        assert_eq!(e.path, long.as_bytes());
        assert_eq!((e.uid, e.mtime), (1 << 40, -5));
        assert_eq!(e.uname, "n".repeat(40).as_bytes());
        assert_eq!(t.data(0), b"data");
        assert_eq!(t.entries()[1].link_target, target.as_bytes());
        assert_eq!(t.entries()[2].path, split.as_bytes());
        assert_eq!(t.entries()[2].header_offset, t.entries()[1].data_offset);
    }

    #[test]
    fn pax_records_count_their_own_length() {
        for n in 0..300 {
            let r = pax_record("path", &vec![b'a'; n]);
            let len: usize =
                std::str::from_utf8(&r[..r.iter().position(|&b| b == b' ').unwrap()]).unwrap().parse().unwrap();
            assert_eq!(len, r.len());
        }
    }

    #[test]
    fn gnu_long_names_and_base256_sizes_are_read() {
        // A GNU 'L' header carrying a long name, then the file.
        let name = "g".repeat(200);
        let mut w = TarWriter::new();
        w.entry_raw(b"././@LongLink", b'L', &[name.as_bytes(), b"\0"].concat(), &TarOptions::default());
        w.add_file("short", b"xyz", &opts()).unwrap();
        let mut bytes = w.finish();
        // Rewrite the file's size field in base-256.
        let h = 1024; // after the 'L' header and its one block of name
        bytes[h + 124..h + 136].copy_from_slice(&[0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3]);
        bytes[h + 148..h + 156].copy_from_slice(b"        ");
        let sum: u32 = bytes[h..h + BLOCK].iter().map(|&b| u32::from(b)).sum();
        bytes[h + 148..h + 155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        let t = TarArchive::parse(&bytes).unwrap();
        assert_eq!(t.entries()[0].path, name.as_bytes());
        assert_eq!(t.data(0), b"xyz");
    }

    #[test]
    fn copy_entry_keeps_entries_exactly() {
        let mut w = TarWriter::new();
        w.add_file("a", b"first", &opts()).unwrap();
        w.add_file(&"long/".repeat(40), b"pax-named", &opts()).unwrap_err();
        w.add_file(&format!("{}/f", "l".repeat(200)), b"pax-named", &opts()).unwrap();
        w.add_symlink("s", "a", &opts()).unwrap();
        let first = w.finish();
        let a = TarArchive::parse(&first).unwrap();
        let mut w = TarWriter::new();
        w.copy_entry(&a, 1);
        w.copy_entry(&a, 2);
        let second = w.finish();
        let b = TarArchive::parse(&second).unwrap();
        assert_eq!(b.len(), 2);
        for (j, i) in [1, 2].into_iter().enumerate() {
            let (x, y) = (&a.entries()[i], &b.entries()[j]);
            assert_eq!(
                (&x.path, x.kind, x.mode, x.mtime, &x.link_target),
                (&y.path, y.kind, y.mode, y.mtime, &y.link_target)
            );
            assert_eq!(a.data(i), b.data(j));
        }
    }

    #[test]
    fn corruption_is_reported() {
        let mut w = TarWriter::new();
        w.add_file("f", &[7; 1000], &opts()).unwrap();
        let bytes = w.finish();
        let mut bad = bytes.clone();
        bad[0] ^= 1;
        assert_eq!(TarArchive::parse(&bad).unwrap_err(), TarError::BadChecksum { offset: 0 });
        assert_eq!(TarArchive::parse(&bytes[..1000]).unwrap_err(), TarError::Truncated);
        assert_eq!(TarArchive::parse(&bytes[..100]).unwrap_err(), TarError::Truncated);
        // Without the end-of-archive marker the entries still read.
        assert_eq!(TarArchive::parse(&bytes[..1536]).unwrap().len(), 1);
        assert!(TarArchive::parse(&[]).unwrap().is_empty());
        assert!(is_tar(&bytes) && !is_tar(&bad) && !is_tar(&[0; 1024]) && !is_tar(&bytes[..511]));
    }

    #[test]
    fn unsafe_names_are_refused_or_cleaned() {
        let mut w = TarWriter::new();
        for bad in ["/abs", "a/../b", "", "a\\b"] {
            assert!(w.add_file(bad, b"", &opts()).is_err(), "{bad}");
        }
        assert_eq!(safe_path(b"/etc/passwd").as_deref(), Some("etc/passwd"));
        assert_eq!(safe_path(b"./a//b/").as_deref(), Some("a/b"));
        assert_eq!(safe_path(b"a/../../b"), None);
        assert_eq!(safe_path(b"./"), None);
    }
}
