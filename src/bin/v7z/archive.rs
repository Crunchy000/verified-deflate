//! The formats `v7z` reads, behind one view: an opened archive is a list
//! of items, each of which can be read.

use std::borrow::Cow;
use std::cell::RefCell;
use verified_deflate::sevenz::SevenZArchive;
use verified_deflate::tar::{EntryKind, TarArchive};
use verified_deflate::zip::{
    DosDateTime, ZipArchive, METHOD_BZIP2, METHOD_DEFLATE64, METHOD_DEFLATED, METHOD_LZMA, METHOD_STORED, METHOD_XZ,
};
use verified_deflate::{bzip2, gzip, lzma, xz};

/// An archive format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Zip,
    Tar,
    Gzip,
    Bzip2,
    Xz,
    Lzma,
    SevenZ,
}

impl Kind {
    /// The name 7-Zip uses for the type (`-t`, and `Type =` in listings).
    pub fn name(self) -> &'static str {
        match self {
            Kind::Zip => "zip",
            Kind::Tar => "tar",
            Kind::Gzip => "gzip",
            Kind::Bzip2 => "bzip2",
            Kind::Xz => "xz",
            Kind::Lzma => "lzma",
            Kind::SevenZ => "7z",
        }
    }

    /// The type named by `-t{name}`.
    pub fn from_type(t: &str) -> Result<Kind, String> {
        match t {
            "zip" => Ok(Kind::Zip),
            "tar" => Ok(Kind::Tar),
            "gzip" | "gz" => Ok(Kind::Gzip),
            "bzip2" | "bz2" => Ok(Kind::Bzip2),
            "xz" => Ok(Kind::Xz),
            "lzma" => Ok(Kind::Lzma),
            "7z" => Ok(Kind::SevenZ),
            other => Err(format!("unsupported archive type: {other}")),
        }
    }

    /// The type a file name's extension suggests, for creating archives.
    pub fn from_extension(name: &str) -> Option<Kind> {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".zip") || lower.ends_with(".jar") {
            Some(Kind::Zip)
        } else if lower.ends_with(".tar") {
            Some(Kind::Tar)
        } else if lower.ends_with(".gz")
            || lower.ends_with(".gzip")
            || lower.ends_with(".tgz")
            || lower.ends_with(".tpz")
        {
            Some(Kind::Gzip)
        } else if [".bz2", ".bzip2", ".tbz2", ".tbz"].iter().any(|e| lower.ends_with(e)) {
            Some(Kind::Bzip2)
        } else if lower.ends_with(".xz") || lower.ends_with(".txz") {
            Some(Kind::Xz)
        } else if lower.ends_with(".lzma") {
            Some(Kind::Lzma)
        } else if lower.ends_with(".7z") {
            Some(Kind::SevenZ)
        } else {
            None
        }
    }

    /// The type of an archive, from its first bytes.
    pub fn detect(data: &[u8]) -> Option<Kind> {
        if data.starts_with(&verified_deflate::sevenz::SIGNATURE) {
            Some(Kind::SevenZ)
        } else if data.starts_with(b"PK\x03\x04") || data.starts_with(b"PK\x05\x06") || data.starts_with(b"PK\x07\x08")
        {
            Some(Kind::Zip)
        } else if data.starts_with(&[0x1f, 0x8b]) {
            Some(Kind::Gzip)
        } else if bzip2::is_bzip2(data) {
            Some(Kind::Bzip2)
        } else if xz::is_xz(data) {
            Some(Kind::Xz)
        } else if verified_deflate::tar::is_tar(data) {
            Some(Kind::Tar)
        } else if looks_like_lzma(data) {
            Some(Kind::Lzma)
        } else {
            None
        }
    }
}

/// Whether `data` looks like a `.lzma` file, which has no magic number:
/// valid properties, a dictionary of 2^n or 2^n + 2^(n−1) bytes, a size
/// that is unknown or under 256 GiB (liblzma's tests), and a range coder
/// that starts with 0.
fn looks_like_lzma(data: &[u8]) -> bool {
    let Ok((props, size)) = lzma::parse_header(data) else { return false };
    let d = props.dict_size;
    let tidy = d == u32::MAX || (d >= 4096 && (d.is_power_of_two() || (d / 3).is_power_of_two() && d % 3 == 0));
    props.lc + props.lp <= 4 && tidy && size.is_none_or(|n| n < 1 << 38) && data.get(13) == Some(&0)
}

/// When an item was last modified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Time {
    /// An MS-DOS time (ZIP), with no time zone.
    Dos(DosDateTime),
    /// Seconds since the Unix epoch (tar, gzip).
    Unix(i64),
}

impl Time {
    /// Seconds since the Unix epoch (a DOS time is read as UTC).
    pub fn unix_seconds(self) -> i64 {
        match self {
            Time::Dos(d) => d.to_unix_seconds(),
            Time::Unix(s) => s,
        }
    }

    /// `YYYY-MM-DD HH:MM:SS`, as 7-Zip lists times (Unix times in UTC).
    pub fn display(self) -> String {
        let (y, mo, d, h, mi, s) = match self {
            Time::Dos(t) => (i64::from(t.year), u32::from(t.month), u32::from(t.day), t.hour, t.minute, t.second),
            Time::Unix(secs) => {
                let days = secs.div_euclid(86_400);
                let rem = secs.rem_euclid(86_400);
                let (y, m, d) = civil_from_days(days);
                (y, m, d, (rem / 3600) as u8, (rem / 60 % 60) as u8, (rem % 60) as u8)
            }
        };
        format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
    }
}

/// H. Hinnant's `civil_from_days`: the date of a day number (0 = 1970-01-01).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// What an item is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    File,
    Dir,
    /// A symbolic link to the target.
    Symlink(String),
    /// A hard link to an earlier item.
    HardLink(String),
    /// A device, FIFO or other special file, extracted as nothing.
    Special,
}

/// One item of an opened archive.
#[derive(Debug, Clone)]
pub struct Item {
    /// The path as stored, as text.
    pub path: String,
    pub kind: ItemKind,
    /// Unknown for bzip2 until it is decompressed, as in 7-Zip.
    pub size: Option<u64>,
    /// Bytes the item takes in the archive, where the format says.
    pub packed: Option<u64>,
    pub mtime: Option<Time>,
    /// Unix permission bits, if recorded.
    pub mode: Option<u32>,
    pub crc: Option<u32>,
    pub method: Option<String>,
    pub host_os: Option<&'static str>,
    pub user: Option<String>,
    pub group: Option<String>,
    /// MS-DOS attributes (read-only 0x01, hidden 0x02, system 0x04,
    /// directory 0x10, archive 0x20), where recorded.
    pub dos_attrib: Option<u8>,
}

impl Item {
    pub fn is_dir(&self) -> bool {
        self.kind == ItemKind::Dir
    }
}

enum Source<'a> {
    Zip(ZipArchive<'a>),
    Tar(TarArchive<'a>),
    Gzip(&'a [u8]),
    Bzip2(&'a [u8]),
    Xz(&'a [u8]),
    Lzma(&'a [u8]),
    /// A 7z archive, with its folders' outputs once decoded.
    SevenZ(SevenZArchive<'a>, RefCell<Vec<Option<Decoded>>>),
}

/// A 7z folder's output, or why it could not be decoded.
type Decoded = Result<Vec<u8>, String>;

/// An archive opened for reading.
pub struct Opened<'a> {
    pub kind: Kind,
    pub items: Vec<Item>,
    source: Source<'a>,
}

fn host_os(version_made_by: u16) -> &'static str {
    match version_made_by >> 8 {
        0 => "FAT",
        3 => "Unix",
        7 => "Macintosh",
        10 => "TOPS-20",
        11 => "NTFS",
        19 => "OS X",
        _ => "Unknown",
    }
}

/// The name gzip's item gets when its header has none: the archive's name
/// without `.gz` (`.tgz` becomes `.tar`), as 7-Zip names it.
fn gzip_item_name(archive_name: &str) -> String {
    item_name(archive_name, &[(".tgz", ".tar"), (".tpz", ".tar"), (".gzip", ""), (".gz", "")])
}

/// The name of a bzip2 file's item: the archive's name without `.bz2`
/// (`.tbz2` becomes `.tar`).
fn bzip2_item_name(archive_name: &str) -> String {
    item_name(archive_name, &[(".tbz2", ".tar"), (".tbz", ".tar"), (".bzip2", ""), (".bz2", "")])
}

/// The name of an xz or lzma file's item (`.txz` becomes `.tar`).
fn xz_item_name(archive_name: &str) -> String {
    item_name(archive_name, &[(".txz", ".tar"), (".xz", ""), (".lzma", "")])
}

fn item_name(archive_name: &str, extensions: &[(&str, &str)]) -> String {
    let base = archive_name.rsplit(['/', '\\']).next().unwrap_or(archive_name);
    let lower = base.to_ascii_lowercase();
    for &(ext, replacement) in extensions {
        if lower.ends_with(ext) && lower.len() > ext.len() {
            return format!("{}{replacement}", &base[..base.len() - ext.len()]);
        }
    }
    base.to_string()
}

impl<'a> Opened<'a> {
    /// Opens `data` as an archive of the given kind; `archive_name` names
    /// a gzip file's item when its header does not.
    pub fn open(data: &'a [u8], kind: Kind, archive_name: &str) -> Result<Self, String> {
        match kind {
            Kind::Zip => {
                let zip = ZipArchive::parse(data).map_err(|e| e.to_string())?;
                let items = zip
                    .entries()
                    .iter()
                    .map(|e| {
                        let unix = e.unix_mode;
                        let is_link = unix.is_some_and(|m| m & 0o170_000 == 0o120_000);
                        let kind = if e.is_dir() {
                            ItemKind::Dir
                        } else if is_link {
                            ItemKind::Symlink(String::new()) // the target is the data
                        } else {
                            ItemKind::File
                        };
                        Item {
                            path: e.name.trim_end_matches('/').to_string(),
                            kind,
                            size: Some(u64::from(e.uncompressed_size)),
                            packed: Some(u64::from(e.compressed_size)),
                            mtime: Some(Time::Dos(e.modified)),
                            mode: unix.map(|m| m & 0o7777),
                            crc: Some(e.crc32),
                            method: Some(
                                match e.method {
                                    METHOD_STORED => "Store",
                                    METHOD_DEFLATED => "Deflate",
                                    METHOD_DEFLATE64 => "Deflate64",
                                    METHOD_BZIP2 => "BZip2",
                                    METHOD_LZMA => "LZMA",
                                    METHOD_XZ => "XZ",
                                    _ => "Unknown",
                                }
                                .to_string(),
                            ),
                            host_os: Some(host_os(e.version_made_by)),
                            user: None,
                            group: None,
                            dos_attrib: Some(e.external_attr as u8),
                        }
                    })
                    .collect();
                let mut opened = Self { kind, items, source: Source::Zip(zip) };
                // A ZIP symlink's target is its (small) data.
                for i in 0..opened.items.len() {
                    if opened.items[i].kind == ItemKind::Symlink(String::new()) {
                        let target =
                            opened.read(i).map(|d| String::from_utf8_lossy(&d).into_owned()).unwrap_or_default();
                        opened.items[i].kind = ItemKind::Symlink(target);
                    }
                }
                Ok(opened)
            }
            Kind::Tar => {
                let tar = TarArchive::parse(data).map_err(|e| e.to_string())?;
                let items = tar
                    .entries()
                    .iter()
                    .map(|e| {
                        let target = String::from_utf8_lossy(&e.link_target).into_owned();
                        Item {
                            path: e.path_lossy().trim_end_matches('/').to_string(),
                            kind: match e.kind {
                                EntryKind::File => ItemKind::File,
                                EntryKind::Directory => ItemKind::Dir,
                                EntryKind::Symlink => ItemKind::Symlink(target),
                                EntryKind::HardLink => ItemKind::HardLink(target),
                                _ => ItemKind::Special,
                            },
                            size: Some(e.size),
                            packed: Some(e.size.next_multiple_of(512)),
                            mtime: Some(Time::Unix(e.mtime)),
                            mode: Some(e.mode & 0o7777),
                            crc: None,
                            method: None,
                            host_os: None,
                            user: (!e.uname.is_empty()).then(|| String::from_utf8_lossy(&e.uname).into_owned()),
                            group: (!e.gname.is_empty()).then(|| String::from_utf8_lossy(&e.gname).into_owned()),
                            dos_attrib: None,
                        }
                    })
                    .collect();
                Ok(Self { kind, items, source: Source::Tar(tar) })
            }
            Kind::Gzip => {
                let (header, _) = gzip::parse_header(data).map_err(|e| e.to_string())?;
                let n = data.len();
                let isize =
                    if n >= 4 { u32::from_le_bytes([data[n - 4], data[n - 3], data[n - 2], data[n - 1]]) } else { 0 };
                let crc =
                    if n >= 8 { u32::from_le_bytes([data[n - 8], data[n - 7], data[n - 6], data[n - 5]]) } else { 0 };
                let path = match &header.filename {
                    Some(f) => String::from_utf8_lossy(f).into_owned(),
                    None => gzip_item_name(archive_name),
                };
                let item = Item {
                    path,
                    kind: ItemKind::File,
                    size: Some(u64::from(isize)),
                    packed: Some(data.len() as u64),
                    mtime: (header.mtime != 0).then_some(Time::Unix(i64::from(header.mtime))),
                    mode: None,
                    crc: Some(crc),
                    method: Some("Deflate".into()),
                    host_os: Some(match header.os_byte {
                        0 => "FAT",
                        3 => "Unix",
                        7 | 19 => "Macintosh",
                        11 => "NTFS",
                        _ => "Unknown",
                    }),
                    user: None,
                    group: None,
                    dos_attrib: None,
                };
                Ok(Self { kind, items: vec![item], source: Source::Gzip(data) })
            }
            Kind::Bzip2 => {
                if !bzip2::is_bzip2(data) {
                    return Err(bzip2::Bzip2Error::NotBzip2.to_string());
                }
                let item = Item {
                    path: bzip2_item_name(archive_name),
                    kind: ItemKind::File,
                    size: None,
                    packed: Some(data.len() as u64),
                    mtime: None,
                    mode: None,
                    crc: None,
                    method: None,
                    host_os: None,
                    user: None,
                    group: None,
                    dos_attrib: None,
                };
                Ok(Self { kind, items: vec![item], source: Source::Bzip2(data) })
            }
            Kind::Xz | Kind::Lzma => {
                let (size, method) = if kind == Kind::Xz {
                    if !xz::is_xz(data) {
                        return Err(xz::XzError::NotXz.to_string());
                    }
                    (xz::uncompressed_size(data), "LZMA2")
                } else {
                    (lzma::parse_header(data).map_err(|e| e.to_string())?.1, "LZMA")
                };
                let item = Item {
                    path: xz_item_name(archive_name),
                    kind: ItemKind::File,
                    size,
                    packed: Some(data.len() as u64),
                    mtime: None,
                    mode: None,
                    crc: None,
                    method: Some(method.into()),
                    host_os: None,
                    user: None,
                    group: None,
                    dos_attrib: None,
                };
                let source = if kind == Kind::Xz { Source::Xz(data) } else { Source::Lzma(data) };
                Ok(Self { kind, items: vec![item], source })
            }
            Kind::SevenZ => {
                let a = SevenZArchive::parse(data).map_err(|e| e.to_string())?;
                // As 7-Zip lists them: a block's packed size on its first file.
                let mut seen = vec![false; a.folder_count()];
                let items = a
                    .entries()
                    .iter()
                    .map(|e| {
                        let packed = e.folder.and_then(|(f, _)| {
                            let first = !seen[f];
                            seen[f] = true;
                            first.then(|| a.folder_packed_size(f))
                        });
                        Item {
                            path: e.name.trim_end_matches('/').to_string(),
                            kind: if e.is_dir {
                                ItemKind::Dir
                            } else if e.is_symlink() {
                                ItemKind::Symlink(String::new())
                            } else {
                                ItemKind::File
                            },
                            size: Some(e.size),
                            packed,
                            mtime: e.mtime_unix().map(Time::Unix),
                            mode: e.unix_mode().map(|m| m & 0o7777),
                            crc: e.crc,
                            method: e.folder.map(|(f, _)| a.folder_methods(f)),
                            host_os: None,
                            user: None,
                            group: None,
                            dos_attrib: e.attributes.map(|x| x as u8),
                        }
                    })
                    .collect();
                let cache = RefCell::new(vec![None; a.folder_count()]);
                let mut opened = Self { kind, items, source: Source::SevenZ(a, cache) };
                // A symbolic link's target is its data.
                for i in 0..opened.items.len() {
                    if opened.items[i].kind == ItemKind::Symlink(String::new()) {
                        let target =
                            opened.read(i).map(|d| String::from_utf8_lossy(&d).into_owned()).unwrap_or_default();
                        opened.items[i].kind = ItemKind::Symlink(target);
                    }
                }
                Ok(opened)
            }
        }
    }

    /// The data of item `i`, checked against its CRC where the format has
    /// one. Directories and links read as empty.
    pub fn read(&self, i: usize) -> Result<Cow<'a, [u8]>, String> {
        match &self.source {
            Source::Zip(zip) => zip.read(i).map(Cow::Owned).map_err(|e| e.to_string()),
            Source::Tar(tar) => Ok(Cow::Borrowed(tar.data(i))),
            Source::Gzip(data) => gzip::decompress(data).map(Cow::Owned).map_err(|e| e.to_string()),
            Source::Bzip2(data) => bzip2::decompress(data).map(Cow::Owned).map_err(|e| e.to_string()),
            Source::Xz(data) => xz::decompress(data).map(Cow::Owned).map_err(|e| e.to_string()),
            Source::Lzma(data) => lzma::decompress(data).map(Cow::Owned).map_err(|e| e.to_string()),
            Source::SevenZ(a, cache) => {
                let e = a.entries().get(i).ok_or("no such entry")?;
                let Some((f, _)) = e.folder else { return Ok(Cow::Borrowed(&[])) };
                let mut cache = cache.borrow_mut();
                let out = cache[f].get_or_insert_with(|| a.decode_folder(f, usize::MAX).map_err(|e| e.to_string()));
                match out {
                    Ok(out) => a.entry_data(i, out).map(|d| Cow::Owned(d.to_vec())).map_err(|e| e.to_string()),
                    Err(e) => Err(e.clone()),
                }
            }
        }
    }

    pub fn zip(&self) -> Option<&ZipArchive<'a>> {
        match &self.source {
            Source::Zip(z) => Some(z),
            _ => None,
        }
    }

    pub fn tar(&self) -> Option<&TarArchive<'a>> {
        match &self.source {
            Source::Tar(t) => Some(t),
            _ => None,
        }
    }

    pub fn sevenz(&self) -> Option<&SevenZArchive<'a>> {
        match &self.source {
            Source::SevenZ(a, _) => Some(a),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_times_display_in_utc() {
        assert_eq!(Time::Unix(0).display(), "1970-01-01 00:00:00");
        assert_eq!(Time::Unix(1_700_000_000).display(), "2023-11-14 22:13:20");
        assert_eq!(Time::Unix(-1).display(), "1969-12-31 23:59:59");
        assert_eq!(Time::Unix(951_782_400).display(), "2000-02-29 00:00:00");
    }

    #[test]
    fn gzip_items_are_named_like_7zip_names_them() {
        assert_eq!(gzip_item_name("dir/a.tar.gz"), "a.tar");
        assert_eq!(gzip_item_name("b.tgz"), "b.tar");
        assert_eq!(gzip_item_name("c.GZ"), "c");
        assert_eq!(gzip_item_name("plain"), "plain");
        assert_eq!(bzip2_item_name("x/a.tar.bz2"), "a.tar");
        assert_eq!(bzip2_item_name("b.tbz2"), "b.tar");
        assert_eq!(bzip2_item_name("c.bzip2"), "c");
        assert_eq!(xz_item_name("d.tar.xz"), "d.tar");
        assert_eq!(xz_item_name("e.txz"), "e.tar");
        assert_eq!(xz_item_name("f.lzma"), "f");
        assert!(looks_like_lzma(&lzma::compress(b"hello", 6)));
        assert!(!looks_like_lzma(b"hello, this is plain text"));
    }
}
