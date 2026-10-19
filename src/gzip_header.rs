//! gzip headers (RFC 1952 §2.3), with the header-building API of the
//! [`gzip-header`](https://docs.rs/gzip-header/1.1.0) crate (`GzBuilder`,
//! `FileSystemType`, `ExtraFlags`), which the `deflate` crate's gzip
//! functions take. Headers are built byte for byte as `gzip-header`
//! builds them.

use crate::crc32::crc32;
use std::fmt;

const FHCRC: u8 = 1 << 1;
const FEXTRA: u8 = 1 << 2;
const FNAME: u8 = 1 << 3;
const FCOMMENT: u8 = 1 << 4;

/// The operating system / file system field of a gzip header. Value 19
/// (`Apple`) is defined by zlib rather than the gzip specification.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum FileSystemType {
    /// MS-DOS / old FAT file system.
    Fat = 0,
    Amiga = 1,
    Vms = 2,
    Unix = 3,
    Vcms = 4,
    AtariTos = 5,
    Hpfs = 6,
    /// Apple platforms (newer encoders may use `Apple`).
    Macintosh = 7,
    Zsystem = 8,
    Cpm = 9,
    /// Windows/NTFS for zlib newer than 1.2.11, or TOPS-20.
    Tops20OrNTFS = 10,
    /// Windows for older zlib versions and other encoders.
    NTFS = 11,
    SmsQdos = 12,
    Riscos = 13,
    /// Newer FAT file systems (FAT32).
    Vfat = 14,
    Mvs = 15,
    Beos = 16,
    TandemNsk = 17,
    Theos = 18,
    /// Modern Apple platforms (defined by zlib).
    Apple = 19,
    /// Unknown or unset.
    Unknown = 255,
}

impl FileSystemType {
    /// The raw byte value.
    pub const fn as_u8(&self) -> u8 {
        *self as u8
    }

    /// The variant for a raw byte; values not listed are `Unknown`.
    pub fn from_u8(value: u8) -> FileSystemType {
        use FileSystemType::*;
        match value {
            0 => Fat,
            1 => Amiga,
            2 => Vms,
            3 => Unix,
            4 => Vcms,
            5 => AtariTos,
            6 => Hpfs,
            7 => Macintosh,
            8 => Zsystem,
            9 => Cpm,
            10 => Tops20OrNTFS,
            11 => NTFS,
            12 => SmsQdos,
            13 => Riscos,
            14 => Vfat,
            15 => Mvs,
            16 => Beos,
            17 => TandemNsk,
            18 => Theos,
            19 => Apple,
            _ => Unknown,
        }
    }
}

impl fmt::Display for FileSystemType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use FileSystemType::*;
        match *self {
            Fat => "FAT filesystem (MS-DOS, OS/2, NT/Win32)",
            Amiga => "Amiga",
            Vms => "VMS or OpenVMS",
            Unix => "Unix type system/Linux",
            Vcms => "VM/CMS",
            AtariTos => "Atari TOS",
            Hpfs => "HPFS filesystem (OS/2, NT)",
            Macintosh => "Macintosh operating system (Classic Mac OS, OS/X, macOS, iOS etc.)",
            Zsystem => "Z-System",
            Cpm => "CP/M",
            Tops20OrNTFS => "NTFS (New zlib versions) or TOPS-20",
            NTFS => "NTFS",
            SmsQdos => "SMS/QDOS",
            Riscos => "Acorn RISC OS",
            Vfat => "VFAT file system (Win95, NT)",
            Mvs => "MVS or PRIMOS",
            Beos => "BeOS",
            TandemNsk => "Tandem/NSK",
            Theos => "THEOS",
            Apple => "macOS, OS/X, iOS or watchOS",
            Unknown => "Unknown or unset",
        }
        .fmt(f)
    }
}

/// The extra-flags (`XFL`) field: a hint about how hard the encoder tried.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ExtraFlags {
    /// No hint (0).
    #[default]
    Default = 0,
    /// Maximum compression (2).
    MaximumCompression = 2,
    /// Fastest compression (4).
    FastestCompression = 4,
}

impl ExtraFlags {
    /// The variant for a raw byte; values other than 2 and 4 are `Default`.
    pub fn from_u8(value: u8) -> ExtraFlags {
        match value {
            2 => ExtraFlags::MaximumCompression,
            4 => ExtraFlags::FastestCompression,
            _ => ExtraFlags::Default,
        }
    }

    /// The raw byte value.
    pub const fn as_u8(&self) -> u8 {
        *self as u8
    }
}

impl fmt::Display for ExtraFlags {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ExtraFlags::Default => "No extra flags (Default) or unknown.",
            ExtraFlags::MaximumCompression => "Maximum compression algorithm (DEFLATE).",
            ExtraFlags::FastestCompression => "Fastest compression algorithm (DEFLATE)",
        }
        .fmt(f)
    }
}

/// Builds a gzip header.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct GzBuilder {
    extra: Option<Vec<u8>>,
    filename: Option<Vec<u8>>,
    comment: Option<Vec<u8>>,
    mtime: u32,
    os: Option<FileSystemType>,
    xfl: ExtraFlags,
}

impl GzBuilder {
    /// A builder with no optional fields, modification time 0 and the
    /// operating system of the target (see [`os`](Self::os)).
    pub fn new() -> GzBuilder {
        GzBuilder::default()
    }

    /// Sets the modification time (Unix time, seconds).
    pub fn mtime(mut self, mtime: u32) -> GzBuilder {
        self.mtime = mtime;
        self
    }

    /// Sets the extra field.
    ///
    /// # Panics
    /// When the header is built, if the field is longer than 65535 bytes
    /// (its length is a 16-bit field).
    pub fn extra<T: Into<Vec<u8>>>(mut self, extra: T) -> GzBuilder {
        self.extra = Some(extra.into());
        self
    }

    /// Sets the file name field.
    ///
    /// # Panics
    /// If the name contains a zero byte.
    pub fn filename<T: Into<Vec<u8>>>(mut self, filename: T) -> GzBuilder {
        self.filename = Some(without_nul(filename.into()));
        self
    }

    /// Sets the comment field.
    ///
    /// # Panics
    /// If the comment contains a zero byte.
    pub fn comment<T: Into<Vec<u8>>>(mut self, comment: T) -> GzBuilder {
        self.comment = Some(without_nul(comment.into()));
        self
    }

    /// Sets the operating system field. If not set, it is taken from the
    /// target: `Unix` for Linux and the BSDs, `Apple` for macOS, and
    /// `Unknown` otherwise (as `gzip-header` does).
    pub fn os(mut self, os: FileSystemType) -> GzBuilder {
        self.os = Some(os);
        self
    }

    /// Sets the extra-flags field (default [`ExtraFlags::Default`]).
    pub fn xfl(mut self, xfl: ExtraFlags) -> GzBuilder {
        self.xfl = xfl;
        self
    }

    /// The header bytes, with the extra-flags field set to `lvl`.
    pub fn into_header_xfl(mut self, lvl: ExtraFlags) -> Vec<u8> {
        self.xfl = lvl;
        self.into_header()
    }

    /// The header bytes.
    pub fn into_header(self) -> Vec<u8> {
        self.into_header_inner(false)
    }

    /// The header bytes, ending with a header CRC (`FHCRC`).
    pub fn into_header_with_checksum(self) -> Vec<u8> {
        self.into_header_inner(true)
    }

    fn into_header_inner(self, use_crc: bool) -> Vec<u8> {
        let GzBuilder { extra, filename, comment, mtime, os, xfl } = self;
        let os = os.unwrap_or_else(default_os);
        let mut flg = 0;
        if use_crc {
            flg |= FHCRC;
        }
        let mut header = vec![0u8; 10];
        if let Some(v) = extra {
            let len = u16::try_from(v.len()).expect("gzip extra field longer than 65535 bytes");
            flg |= FEXTRA;
            header.extend_from_slice(&len.to_le_bytes());
            header.extend_from_slice(&v);
        }
        if let Some(name) = filename {
            flg |= FNAME;
            header.extend_from_slice(&name);
            header.push(0);
        }
        if let Some(c) = comment {
            flg |= FCOMMENT;
            header.extend_from_slice(&c);
            header.push(0);
        }
        header[..4].copy_from_slice(&[0x1f, 0x8b, 8, flg]);
        header[4..8].copy_from_slice(&mtime.to_le_bytes());
        header[8] = xfl.as_u8();
        header[9] = os.as_u8();
        if use_crc {
            let crc = crc32(&header) as u16;
            header.extend_from_slice(&crc.to_le_bytes());
        }
        header
    }
}

/// The value as a zero-terminated string's contents; panics on a zero byte
/// (as building a `CString` does).
fn without_nul(v: Vec<u8>) -> Vec<u8> {
    assert!(!v.contains(&0), "gzip header string contains a zero byte");
    v
}

/// The operating system field used when none is set, chosen as
/// `gzip-header` chooses it (from `std::env::consts::OS`).
fn default_os() -> FileSystemType {
    match std::env::consts::OS {
        "linux" | "freebsd" | "dragonfly" | "netbsd" | "openbsd" | "solaris" | "bitrig" => FileSystemType::Unix,
        "macos" => FileSystemType::Apple,
        "win32" => FileSystemType::Tops20OrNTFS,
        _ => FileSystemType::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_header() {
        let h = GzBuilder::new().os(FileSystemType::Unix).into_header();
        assert_eq!(h, [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3]);
    }

    #[test]
    fn every_field() {
        let h = GzBuilder::new()
            .mtime(0x0403_0201)
            .extra(vec![9, 9])
            .filename("a.txt")
            .comment("hi")
            .os(FileSystemType::Apple)
            .into_header_xfl(ExtraFlags::MaximumCompression);
        let mut want = vec![0x1f, 0x8b, 8, FEXTRA | FNAME | FCOMMENT, 1, 2, 3, 4, 2, 19, 2, 0, 9, 9];
        want.extend_from_slice(b"a.txt\0hi\0");
        assert_eq!(h, want);
        let c = GzBuilder::new().os(FileSystemType::Fat).into_header_with_checksum();
        assert_eq!(c[3], FHCRC);
        assert_eq!(&c[10..], &(crc32(&c[..10]) as u16).to_le_bytes());
    }

    #[test]
    #[should_panic]
    fn nul_in_name_panics() {
        let _ = GzBuilder::new().filename(&b"a\0b"[..]);
    }

    #[test]
    fn enum_values_round_trip() {
        // RFC 1952 defines operating systems 0-13; gzip-header adds 14-19.
        for v in 0..=255u8 {
            let t = FileSystemType::from_u8(v);
            if v <= 19 || v == 255 {
                assert_eq!(t.as_u8(), v);
            } else {
                assert_eq!(t, FileSystemType::Unknown);
            }
            let x = ExtraFlags::from_u8(v);
            if v == 2 || v == 4 {
                assert_eq!(x.as_u8(), v);
            } else {
                assert_eq!(x, ExtraFlags::Default);
            }
        }
        assert_eq!(FileSystemType::from_u8(3), FileSystemType::Unix);
        assert_eq!(FileSystemType::from_u8(11), FileSystemType::NTFS);
        assert_eq!(ExtraFlags::from_u8(2), ExtraFlags::MaximumCompression);
        assert_eq!(ExtraFlags::from_u8(4), ExtraFlags::FastestCompression);
    }

    #[test]
    fn descriptions_are_distinct() {
        let mut seen = std::collections::HashSet::new();
        for v in (0..=19u8).chain([255]) {
            let d = FileSystemType::from_u8(v).to_string();
            assert!(!d.is_empty() && seen.insert(d), "{v}");
        }
        for x in [ExtraFlags::Default, ExtraFlags::MaximumCompression, ExtraFlags::FastestCompression] {
            let d = x.to_string();
            assert!(!d.is_empty() && seen.insert(d));
        }
        assert_eq!(FileSystemType::Unix.to_string(), "Unix type system/Linux");
    }

    #[test]
    fn extra_flags_and_default_os() {
        let h = GzBuilder::new().xfl(ExtraFlags::FastestCompression).os(FileSystemType::Unix).into_header();
        assert_eq!(h[8], 4);
        let h = GzBuilder::new().into_header();
        if cfg!(target_os = "linux") {
            assert_eq!(h[9], FileSystemType::Unix.as_u8());
        } else if cfg!(target_os = "macos") {
            assert_eq!(h[9], FileSystemType::Apple.as_u8());
        }
    }
}
