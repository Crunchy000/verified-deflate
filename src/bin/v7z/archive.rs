//! The formats `v7z` reads, behind one view: an opened archive is a list
//! of items, each of which can be read.

use std::borrow::Cow;
use verified_deflate::gzip;
use verified_deflate::tar::{EntryKind, TarArchive};
use verified_deflate::zip::{DosDateTime, ZipArchive, METHOD_DEFLATE64, METHOD_DEFLATED, METHOD_STORED};

/// An archive format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Zip,
    Tar,
    Gzip,
}

impl Kind {
    /// The name 7-Zip uses for the type (`-t`, and `Type =` in listings).
    pub fn name(self) -> &'static str {
        match self {
            Kind::Zip => "zip",
            Kind::Tar => "tar",
            Kind::Gzip => "gzip",
        }
    }

    /// The type named by `-t{name}`.
    pub fn from_type(t: &str) -> Result<Kind, String> {
        match t {
            "zip" => Ok(Kind::Zip),
            "tar" => Ok(Kind::Tar),
            "gzip" | "gz" => Ok(Kind::Gzip),
            "7z" | "xz" | "bzip2" | "bz2" | "lzma" | "wim" => Err(format!("archive type {t} is not supported yet")),
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
        } else {
            None
        }
    }

    /// The type of an archive, from its first bytes.
    pub fn detect(data: &[u8]) -> Option<Kind> {
        if data.starts_with(b"PK\x03\x04") || data.starts_with(b"PK\x05\x06") || data.starts_with(b"PK\x07\x08") {
            Some(Kind::Zip)
        } else if data.starts_with(&[0x1f, 0x8b]) {
            Some(Kind::Gzip)
        } else if verified_deflate::tar::is_tar(data) {
            Some(Kind::Tar)
        } else {
            None
        }
    }
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
    pub size: u64,
    /// Bytes the item takes in the archive, where the format says.
    pub packed: Option<u64>,
    pub mtime: Option<Time>,
    /// Unix permission bits, if recorded.
    pub mode: Option<u32>,
    pub crc: Option<u32>,
    pub method: Option<&'static str>,
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
}

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
    let base = archive_name.rsplit(['/', '\\']).next().unwrap_or(archive_name);
    let lower = base.to_ascii_lowercase();
    for (ext, replacement) in [(".tgz", ".tar"), (".tpz", ".tar"), (".gzip", ""), (".gz", "")] {
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
                            size: u64::from(e.uncompressed_size),
                            packed: Some(u64::from(e.compressed_size)),
                            mtime: Some(Time::Dos(e.modified)),
                            mode: unix.map(|m| m & 0o7777),
                            crc: Some(e.crc32),
                            method: Some(match e.method {
                                METHOD_STORED => "Store",
                                METHOD_DEFLATED => "Deflate",
                                METHOD_DEFLATE64 => "Deflate64",
                                _ => "Unknown",
                            }),
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
                            size: e.size,
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
                    size: u64::from(isize),
                    packed: Some(data.len() as u64),
                    mtime: (header.mtime != 0).then_some(Time::Unix(i64::from(header.mtime))),
                    mode: None,
                    crc: Some(crc),
                    method: Some("Deflate"),
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
        }
    }

    /// The data of item `i`, checked against its CRC where the format has
    /// one. Directories and links read as empty.
    pub fn read(&self, i: usize) -> Result<Cow<'a, [u8]>, String> {
        match &self.source {
            Source::Zip(zip) => zip.read(i).map(Cow::Owned).map_err(|e| e.to_string()),
            Source::Tar(tar) => Ok(Cow::Borrowed(tar.data(i))),
            Source::Gzip(data) => gzip::decompress(data).map(Cow::Owned).map_err(|e| e.to_string()),
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
    }
}
