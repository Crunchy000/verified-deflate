//! Reading zlib streams (RFC 1950): a two-byte header, a DEFLATE stream,
//! then the Adler-32 of the data. (Writing zlib is in
//! [`deflate_bytes_zlib`](crate::deflate_bytes_zlib) and
//! [`write::ZlibEncoder`](crate::write::ZlibEncoder).)
//!
//! ```
//! use verified_deflate::{deflate_bytes_zlib, zlib};
//!
//! let packed = deflate_bytes_zlib(b"hello, zlib");
//! assert_eq!(zlib::decompress(&packed).unwrap(), b"hello, zlib");
//! ```

use crate::adler32::adler32;
use crate::error::InflateError;
use crate::inflate::{decompress_with_limit as inflate_with_limit, Decompressed};
use core::fmt;

/// Why a zlib stream could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ZlibError {
    /// The first two bytes are not a zlib header (their check fails, or the
    /// window size is over 32 KiB).
    NotZlib,
    /// The data ended inside the header or the checksum.
    Truncated,
    /// The header names a compression method other than DEFLATE (8).
    UnsupportedMethod(u8),
    /// The stream needs a preset dictionary, which is not supported.
    PresetDictionary,
    /// The DEFLATE stream is corrupt.
    Inflate(InflateError),
    /// The data does not match the Adler-32 checksum.
    ChecksumMismatch {
        /// Checksum recorded in the stream.
        expected: u32,
        /// Checksum of the decompressed data.
        actual: u32,
    },
}

impl fmt::Display for ZlibError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotZlib => f.write_str("not a zlib stream"),
            Self::Truncated => f.write_str("unexpected end of zlib data"),
            Self::UnsupportedMethod(m) => write!(f, "unknown zlib compression method {m}"),
            Self::PresetDictionary => f.write_str("zlib preset dictionaries are not supported"),
            Self::Inflate(e) => write!(f, "corrupt compressed data: {e}"),
            Self::ChecksumMismatch { expected, actual } => {
                write!(f, "Adler-32 mismatch: stream says {expected:08x}, data is {actual:08x}")
            }
        }
    }
}

impl std::error::Error for ZlibError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Inflate(e) => Some(e),
            _ => None,
        }
    }
}

impl From<InflateError> for ZlibError {
    fn from(e: InflateError) -> Self {
        Self::Inflate(e)
    }
}

/// Decompresses a zlib stream with no output limit.
///
/// For untrusted input prefer [`decompress_with_limit`].
pub fn decompress(input: &[u8]) -> Result<Vec<u8>, ZlibError> {
    decompress_with_limit(input, usize::MAX).map(|d| d.data)
}

/// Decompresses a zlib stream, failing with
/// [`InflateError::OutputLimitExceeded`] rather than producing more than
/// `max_output` bytes. `consumed` counts the header, the DEFLATE stream and
/// the checksum; anything after them is left alone.
pub fn decompress_with_limit(input: &[u8], max_output: usize) -> Result<Decompressed, ZlibError> {
    let (cmf, flg) = match input {
        [cmf, flg, ..] => (*cmf, *flg),
        _ => return Err(ZlibError::Truncated),
    };
    if (u16::from(cmf) << 8 | u16::from(flg)) % 31 != 0 || cmf >> 4 > 7 {
        return Err(ZlibError::NotZlib);
    }
    if cmf & 0x0f != 8 {
        return Err(ZlibError::UnsupportedMethod(cmf & 0x0f));
    }
    if flg & 0x20 != 0 {
        return Err(ZlibError::PresetDictionary);
    }
    let d = inflate_with_limit(&input[2..], max_output)?;
    let end = 2 + d.consumed;
    let sum = input.get(end..end + 4).ok_or(ZlibError::Truncated)?;
    let expected = u32::from_be_bytes([sum[0], sum[1], sum[2], sum[3]]);
    let actual = adler32(&d.data);
    if expected != actual {
        return Err(ZlibError::ChecksumMismatch { expected, actual });
    }
    Ok(Decompressed { data: d.data, consumed: end + 4 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{deflate_bytes_zlib, deflate_bytes_zlib_conf, Compression};

    #[test]
    fn round_trip_through_our_writer() {
        for data in [&b""[..], b"z", &b"zlib zlib zlib ".repeat(700)] {
            for level in [Compression::Fast, Compression::Default, Compression::Best] {
                let packed = deflate_bytes_zlib_conf(data, level);
                let d = decompress_with_limit(&packed, data.len()).unwrap();
                assert_eq!(d.data, data);
                assert_eq!(d.consumed, packed.len());
            }
        }
    }

    #[test]
    fn corruption_is_reported() {
        let good = deflate_bytes_zlib(b"check me");
        let n = good.len();
        let mut bad = good.clone();
        bad[n - 1] ^= 1;
        assert!(matches!(decompress(&bad), Err(ZlibError::ChecksumMismatch { .. })));
        assert_eq!(decompress(&good[..n - 1]), Err(ZlibError::Truncated));
        assert_eq!(decompress(&good[..1]), Err(ZlibError::Truncated));
        assert_eq!(decompress(&[0x78, 0x9d]), Err(ZlibError::NotZlib));
        assert_eq!(decompress(&[0x88, 0x98]), Err(ZlibError::NotZlib)); // window over 32 KiB
        assert_eq!(decompress(&[0x79, 0x18]), Err(ZlibError::UnsupportedMethod(9)));
        assert_eq!(decompress(&[0x78, 0xbb, 0, 0, 0, 1]), Err(ZlibError::PresetDictionary));
    }
}
