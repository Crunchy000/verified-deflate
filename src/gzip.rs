//! Reading gzip streams (RFC 1952): a header, a DEFLATE stream, then the
//! CRC-32 and length of the data. A file may hold several such members one
//! after another, which decode to their concatenation. (Writing gzip is in
//! [`deflate_bytes_gzip`](crate::deflate_bytes_gzip) and
//! [`write::GzEncoder`](crate::write::GzEncoder).)
//!
//! ```
//! use verified_deflate::{deflate_bytes_gzip, gzip};
//!
//! let packed = deflate_bytes_gzip(b"hello, gzip");
//! assert_eq!(gzip::decompress(&packed).unwrap(), b"hello, gzip");
//! ```

use crate::crc32::{crc32, Crc32};
use crate::error::InflateError;
use crate::gzip_header::{ExtraFlags, FileSystemType};
use crate::inflate::decompress_with_limit as inflate_with_limit;
use core::fmt;

const FTEXT: u8 = 1;
const FHCRC: u8 = 1 << 1;
const FEXTRA: u8 = 1 << 2;
const FNAME: u8 = 1 << 3;
const FCOMMENT: u8 = 1 << 4;
const RESERVED: u8 = 0b1110_0000;
/// Compression method 8, DEFLATE: the only one RFC 1952 defines.
const CM_DEFLATE: u8 = 8;

/// Why a gzip stream could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GzipError {
    /// The data does not start with the gzip signature `1f 8b`.
    NotGzip,
    /// The data ended inside a header or trailer.
    Truncated,
    /// The header names a compression method other than DEFLATE (8).
    UnsupportedMethod(u8),
    /// The header sets flag bits RFC 1952 reserves.
    ReservedFlags(u8),
    /// The header's own CRC-16 (flag FHCRC) does not match it.
    HeaderCrcMismatch,
    /// The DEFLATE stream is corrupt.
    Inflate(InflateError),
    /// The data does not match the CRC-32 in the trailer.
    CrcMismatch {
        /// CRC recorded in the trailer.
        expected: u32,
        /// CRC of the decompressed data.
        actual: u32,
    },
    /// The data's length, modulo 2^32, does not match the trailer.
    SizeMismatch {
        /// Length recorded in the trailer.
        expected: u32,
        /// Length of the decompressed data, modulo 2^32.
        actual: u32,
    },
    /// After the last member come bytes that are neither another member
    /// nor zero padding.
    TrailingData,
}

impl fmt::Display for GzipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotGzip => f.write_str("not in gzip format"),
            Self::Truncated => f.write_str("unexpected end of gzip data"),
            Self::UnsupportedMethod(m) => write!(f, "unknown gzip compression method {m}"),
            Self::ReservedFlags(b) => write!(f, "gzip header sets reserved flags {b:#04x}"),
            Self::HeaderCrcMismatch => f.write_str("gzip header CRC mismatch"),
            Self::Inflate(e) => write!(f, "corrupt compressed data: {e}"),
            Self::CrcMismatch { expected, actual } => {
                write!(f, "CRC mismatch: trailer says {expected:08x}, data is {actual:08x}")
            }
            Self::SizeMismatch { expected, actual } => {
                write!(f, "length mismatch: trailer says {expected}, data is {actual} (mod 2^32)")
            }
            Self::TrailingData => f.write_str("trailing data after the last gzip member"),
        }
    }
}

impl std::error::Error for GzipError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Inflate(e) => Some(e),
            _ => None,
        }
    }
}

impl From<InflateError> for GzipError {
    fn from(e: InflateError) -> Self {
        Self::Inflate(e)
    }
}

/// A gzip member's header (RFC 1952 §2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GzipHeader {
    /// Modification time of the original file, in seconds since the Unix
    /// epoch; 0 if none was recorded.
    pub mtime: u32,
    /// Extra flags (2: maximum compression, 4: fastest).
    pub xfl: ExtraFlags,
    /// The system the stream was made on.
    pub os: FileSystemType,
    /// The raw OS byte (`os` maps unknown values to `Unknown`).
    pub os_byte: u8,
    /// The writer's guess that the data is text (flag FTEXT).
    pub text: bool,
    /// The extra field (flag FEXTRA), if present.
    pub extra: Option<Vec<u8>>,
    /// The original file name (flag FNAME), without its terminating NUL.
    /// RFC 1952 specifies ISO 8859-1; most writers store UTF-8.
    pub filename: Option<Vec<u8>>,
    /// The comment (flag FCOMMENT), without its terminating NUL.
    pub comment: Option<Vec<u8>>,
    /// Whether the header carried, and passed, a CRC-16 (flag FHCRC).
    pub header_crc: bool,
}

/// One decoded gzip member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GzipMember {
    /// The member's header.
    pub header: GzipHeader,
    /// The decompressed data, checked against the trailer.
    pub data: Vec<u8>,
    /// Bytes of input the member took, header to trailer.
    pub consumed: usize,
}

/// Reads a NUL-terminated field at `b[*p..]`, advancing past the NUL.
fn zero_terminated(b: &[u8], p: &mut usize) -> Result<Vec<u8>, GzipError> {
    let rest = b.get(*p..).ok_or(GzipError::Truncated)?;
    let n = rest.iter().position(|&c| c == 0).ok_or(GzipError::Truncated)?;
    *p += n + 1;
    Ok(rest[..n].to_vec())
}

fn le32(b: &[u8], i: usize) -> Result<u32, GzipError> {
    let w = b.get(i..i + 4).ok_or(GzipError::Truncated)?;
    Ok(u32::from(w[0]) | (u32::from(w[1]) << 8) | (u32::from(w[2]) << 16) | (u32::from(w[3]) << 24))
}

/// Parses a gzip header at the start of `b`, returning it and its length.
pub fn parse_header(b: &[u8]) -> Result<(GzipHeader, usize), GzipError> {
    if b.len() < 2 || b[0] != 0x1f || b[1] != 0x8b {
        return Err(if b.len() < 2 && b == &[0x1f, 0x8b][..b.len()] {
            GzipError::Truncated
        } else {
            GzipError::NotGzip
        });
    }
    if b.len() < 10 {
        return Err(GzipError::Truncated);
    }
    if b[2] != CM_DEFLATE {
        return Err(GzipError::UnsupportedMethod(b[2]));
    }
    let flg = b[3];
    if flg & RESERVED != 0 {
        return Err(GzipError::ReservedFlags(flg & RESERVED));
    }
    let mut p = 10;
    let extra = if flg & FEXTRA != 0 {
        let len = b.get(p..p + 2).ok_or(GzipError::Truncated)?;
        let xlen = usize::from(len[0]) | (usize::from(len[1]) << 8);
        let field = b.get(p + 2..p + 2 + xlen).ok_or(GzipError::Truncated)?.to_vec();
        p += 2 + xlen;
        Some(field)
    } else {
        None
    };
    let filename = if flg & FNAME != 0 { Some(zero_terminated(b, &mut p)?) } else { None };
    let comment = if flg & FCOMMENT != 0 { Some(zero_terminated(b, &mut p)?) } else { None };
    if flg & FHCRC != 0 {
        let c = b.get(p..p + 2).ok_or(GzipError::Truncated)?;
        let stored = u16::from(c[0]) | (u16::from(c[1]) << 8);
        if stored != crc32(&b[..p]) as u16 {
            return Err(GzipError::HeaderCrcMismatch);
        }
        p += 2;
    }
    let header = GzipHeader {
        mtime: le32(b, 4)?,
        xfl: ExtraFlags::from_u8(b[8]),
        os: FileSystemType::from_u8(b[9]),
        os_byte: b[9],
        text: flg & FTEXT != 0,
        extra,
        filename,
        comment,
        header_crc: flg & FHCRC != 0,
    };
    Ok((header, p))
}

/// Decodes the gzip member at the start of `input`, producing at most
/// `max_output` bytes (or failing with
/// [`InflateError::OutputLimitExceeded`]), and checks its CRC-32 and length.
pub fn read_member(input: &[u8], max_output: usize) -> Result<GzipMember, GzipError> {
    let (header, start) = parse_header(input)?;
    let d = inflate_with_limit(&input[start..], max_output)?;
    let trailer = start + d.consumed;
    let expected_crc = le32(input, trailer)?;
    let expected_len = le32(input, trailer + 4)?;
    let actual = crc32(&d.data);
    if actual != expected_crc {
        return Err(GzipError::CrcMismatch { expected: expected_crc, actual });
    }
    let len = d.data.len() as u32; // modulo 2^32, as RFC 1952 specifies
    if len != expected_len {
        return Err(GzipError::SizeMismatch { expected: expected_len, actual: len });
    }
    Ok(GzipMember { header, data: d.data, consumed: trailer + 8 })
}

/// Decompresses a gzip file: every member, concatenated, with no output
/// limit. Zero bytes after the last member are allowed (tape padding, as
/// gzip allows); anything else there is an error.
///
/// For untrusted input prefer [`decompress_with_limit`].
pub fn decompress(input: &[u8]) -> Result<Vec<u8>, GzipError> {
    decompress_with_limit(input, usize::MAX)
}

/// Like [`decompress`], failing with
/// [`InflateError::OutputLimitExceeded`] rather than producing more than
/// `max_output` bytes in all.
pub fn decompress_with_limit(input: &[u8], max_output: usize) -> Result<Vec<u8>, GzipError> {
    let mut out = Vec::new();
    let mut rest = input;
    loop {
        let m = read_member(rest, max_output - out.len())?;
        if out.is_empty() {
            out = m.data;
        } else {
            out.extend_from_slice(&m.data);
        }
        rest = &rest[m.consumed..];
        if rest.iter().all(|&b| b == 0) {
            return Ok(out);
        }
        if rest.len() < 2 || rest[0] != 0x1f || rest[1] != 0x8b {
            return Err(GzipError::TrailingData);
        }
    }
}

/// The CRC-32 and length (modulo 2^32) a gzip trailer records for `data`;
/// for writers that build members themselves.
pub fn trailer(data: &[u8]) -> [u8; 8] {
    let mut c = Crc32::new();
    c.update(data);
    let crc = c.finalize().to_le_bytes();
    let len = (data.len() as u32).to_le_bytes();
    [crc[0], crc[1], crc[2], crc[3], len[0], len[1], len[2], len[3]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gzip_header::GzBuilder;
    use crate::{deflate_bytes_gzip, deflate_bytes_gzip_conf, Compression};

    #[test]
    fn round_trip_through_our_writer() {
        for data in [&b""[..], b"a", &b"gzip gzip gzip gzip ".repeat(500)] {
            assert_eq!(decompress(&deflate_bytes_gzip(data)).unwrap(), data);
        }
    }

    #[test]
    fn header_fields_are_read() {
        let builder = GzBuilder::new().filename("notes.txt").comment("a comment").extra(vec![1, 2, 3, 4]).mtime(1234);
        let packed = deflate_bytes_gzip_conf(b"payload", Compression::Default, builder);
        let m = read_member(&packed, 100).unwrap();
        assert_eq!(m.data, b"payload");
        assert_eq!(m.consumed, packed.len());
        assert_eq!(m.header.filename.as_deref(), Some(&b"notes.txt"[..]));
        assert_eq!(m.header.comment.as_deref(), Some(&b"a comment"[..]));
        assert_eq!(m.header.extra.as_deref(), Some(&[1, 2, 3, 4][..]));
        assert_eq!(m.header.mtime, 1234);
    }

    #[test]
    fn members_concatenate_and_zero_padding_is_allowed() {
        let mut two = deflate_bytes_gzip(b"first ");
        two.extend_from_slice(&deflate_bytes_gzip(b"second"));
        assert_eq!(decompress(&two).unwrap(), b"first second");
        two.extend_from_slice(&[0; 512]);
        assert_eq!(decompress(&two).unwrap(), b"first second");
        two.push(7);
        assert_eq!(decompress(&two), Err(GzipError::TrailingData));
    }

    #[test]
    fn corruption_is_reported() {
        let good = deflate_bytes_gzip(b"check me");
        let n = good.len();
        let mut bad = good.clone();
        bad[n - 8] ^= 1;
        assert!(matches!(decompress(&bad), Err(GzipError::CrcMismatch { .. })));
        let mut bad = good.clone();
        bad[n - 4] ^= 1;
        assert!(matches!(decompress(&bad), Err(GzipError::SizeMismatch { .. })));
        assert_eq!(decompress(&good[..n - 1]), Err(GzipError::Truncated));
        assert_eq!(decompress(&good[..1]), Err(GzipError::Truncated));
        assert_eq!(decompress(b"PK\x03\x04"), Err(GzipError::NotGzip));
        let mut bad = good.clone();
        bad[2] = 9;
        assert_eq!(decompress(&bad), Err(GzipError::UnsupportedMethod(9)));
        let mut bad = good.clone();
        bad[3] |= 0x80;
        assert_eq!(decompress(&bad), Err(GzipError::ReservedFlags(0x80)));
        assert_eq!(
            decompress_with_limit(&deflate_bytes_gzip(&[0; 100]), 99).unwrap_err(),
            InflateError::OutputLimitExceeded.into()
        );
    }

    #[test]
    fn header_crc_is_checked() {
        // A minimal header with FHCRC, then an empty stored block and trailer.
        let mut h = vec![0x1f, 0x8b, 8, FHCRC, 0, 0, 0, 0, 0, 3];
        let c = crc32(&h) as u16;
        h.extend_from_slice(&c.to_le_bytes());
        h.extend_from_slice(&[0x03, 0x00]); // a final fixed block, end of block
        h.extend_from_slice(&trailer(b""));
        assert_eq!(decompress(&h).unwrap(), b"");
        assert!(read_member(&h, 0).unwrap().header.header_crc);
        h[10] ^= 1;
        assert_eq!(decompress(&h), Err(GzipError::HeaderCrcMismatch));
    }
}
