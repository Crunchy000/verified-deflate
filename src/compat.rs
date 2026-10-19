//! The API of the [`deflate`](https://docs.rs/deflate/1.0.0) crate
//! (version 1.0), re-exported at this crate's root: the `deflate_bytes*`
//! functions, [`Compression`], [`CompressionOptions`], [`MatchingType`],
//! [`SpecialOptions`] and the [`write`](crate::write) encoders. Code
//! written against `deflate` compiles unchanged against this crate
//! renamed to `deflate` in `Cargo.toml`; only gzip users also switch
//! `gzip_header::GzBuilder` to [`crate::gzip_header::GzBuilder`].
//!
//! The output is not byte-for-byte that of `deflate`'s encoder (no two
//! DEFLATE encoders agree byte for byte), but it is a valid stream of the
//! same format that decodes to the same data, from the same engine as
//! [`compress`](crate::compress).

use crate::adler32::adler32;
use crate::crc32::crc32;
use crate::deflate::{encode_with_matcher, CompressOptions, Strategy};
use crate::gzip_header::GzBuilder;
use crate::lz77::MatchConfig;
use crate::tables::MAX_MATCH;
use std::fmt;

/// The zlib header this crate writes: deflate with a 32 KiB window, no
/// preset dictionary, "default" level (the same header as `deflate`).
pub(crate) const ZLIB_HEADER: [u8; 2] = [0x78, 0x9c];

// RFC 1950: CMF * 256 + FLG must be a multiple of 31.
const _: () = assert!((ZLIB_HEADER[0] as u32 * 256 + ZLIB_HEADER[1] as u32) % 31 == 0);

/// The largest useful [`CompressionOptions::max_hash_checks`]: the length
/// of the hash chain (32 KiB); larger values are treated as this.
const MAX_HASH_CHECKS: u16 = 32 * 1024;

/// A simplified way to choose a compression level.
///
/// See [`CompressionOptions`] for finer control.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub enum Compression {
    /// Fast, light compression ([`CompressionOptions::fast`]).
    Fast,
    /// The default level ([`CompressionOptions::default`]).
    #[default]
    Default,
    /// Stronger, slower compression ([`CompressionOptions::high`]).
    Best,
}

/// Whether the match finder uses lazy or greedy matching.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum MatchingType {
    /// Use a match as soon as one is found.
    Greedy,
    /// After finding a match, check whether the next byte starts a longer
    /// one, and if so emit a literal instead.
    ///
    /// With `max_hash_checks` set to 0, only run-length matching (distance
    /// 1) is done instead.
    Lazy,
}

impl fmt::Display for MatchingType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            MatchingType::Greedy => write!(f, "Greedy matching"),
            MatchingType::Lazy => write!(f, "Lazy matching"),
        }
    }
}

/// Special block-type options.
///
/// In `deflate` 1.0 the two forcing options were not implemented and had no
/// effect; here they force fixed-Huffman or stored blocks.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Default)]
pub enum SpecialOptions {
    /// Compress normally: each block uses the smallest block type.
    #[default]
    Normal,
    /// Use only fixed-Huffman blocks.
    _ForceFixed,
    /// Use only stored (uncompressed) blocks.
    _ForceStored,
}

/// Options for a compressor or compression function.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct CompressionOptions {
    /// The most hash-chain entries to examine when looking for a match.
    ///
    /// Higher values compress better and more slowly. Values above 32768
    /// (the chain length) act as 32768. 0 disables match searching: see
    /// [`matching_type`](Self::matching_type).
    ///
    /// Default: `128`.
    pub max_hash_checks: u16,
    /// Only look for a better match at the next byte (lazy matching) if
    /// the current match is shorter than this.
    ///
    /// * `0`: never (like greedy matching).
    /// * `1..=257`: only for matches shorter than this.
    /// * `258` and above: always.
    ///
    /// Default: `32`.
    pub lazy_if_less_than: u16,
    /// Lazy or greedy matching.
    ///
    /// With `max_hash_checks` set to 0, `Lazy` means run-length encoding
    /// only (matches one byte back), and `Greedy` means Huffman coding
    /// only (no matches).
    ///
    /// Default: [`MatchingType::Lazy`].
    pub matching_type: MatchingType,
    /// Force fixed-Huffman or stored blocks.
    ///
    /// Default: [`SpecialOptions::Normal`].
    pub special: SpecialOptions,
}

impl CompressionOptions {
    /// Settings close to the strongest level of miniz (9).
    pub const fn high() -> CompressionOptions {
        CompressionOptions {
            max_hash_checks: 1768,
            lazy_if_less_than: 128,
            matching_type: MatchingType::Lazy,
            special: SpecialOptions::Normal,
        }
    }

    /// Fast settings: one hash-chain check, greedy matching.
    pub const fn fast() -> CompressionOptions {
        CompressionOptions {
            max_hash_checks: 1,
            lazy_if_less_than: 0,
            matching_type: MatchingType::Greedy,
            special: SpecialOptions::Normal,
        }
    }

    /// Huffman coding only, with no length/distance matches.
    pub const fn huffman_only() -> CompressionOptions {
        CompressionOptions {
            max_hash_checks: 0,
            lazy_if_less_than: 0,
            matching_type: MatchingType::Greedy,
            special: SpecialOptions::Normal,
        }
    }

    /// Run-length encoding only: matches only one byte back. Very fast,
    /// and suited to data with runs of repeated bytes.
    pub const fn rle() -> CompressionOptions {
        CompressionOptions {
            max_hash_checks: 0,
            lazy_if_less_than: 0,
            matching_type: MatchingType::Lazy,
            special: SpecialOptions::Normal,
        }
    }

    /// The match-finder configuration and block policy for these options.
    pub(crate) fn engine(&self) -> (MatchConfig, Strategy) {
        let strategy = match self.special {
            SpecialOptions::Normal => Strategy::Auto,
            SpecialOptions::_ForceFixed => Strategy::Fixed,
            SpecialOptions::_ForceStored => Strategy::Stored,
        };
        let base = MatchConfig::for_level(6);
        let cfg = if self.max_hash_checks == 0 {
            match self.matching_type {
                MatchingType::Lazy => MatchConfig { rle: true, ..base },
                MatchingType::Greedy => MatchConfig { max_chain: 0, lazy: false, max_lazy: MAX_MATCH, ..base },
            }
        } else {
            let checks = self.max_hash_checks.min(MAX_HASH_CHECKS);
            let lazy = self.matching_type == MatchingType::Lazy && self.lazy_if_less_than > 0;
            MatchConfig {
                max_chain: usize::from(checks),
                nice_len: MAX_MATCH,
                // Never shorten the chain: `max_hash_checks` is the limit.
                good_len: MAX_MATCH + 1,
                // Lazy: look ahead for matches shorter than the threshold.
                // Greedy: record every position inside matches.
                max_lazy: if lazy { usize::from(self.lazy_if_less_than).min(MAX_MATCH + 1) } else { MAX_MATCH },
                lazy,
                ..base
            }
        };
        (cfg, strategy)
    }
}

impl Default for CompressionOptions {
    /// The default level: 128 hash checks, lazy matching below length 32.
    fn default() -> CompressionOptions {
        CompressionOptions {
            max_hash_checks: 128,
            lazy_if_less_than: 32,
            matching_type: MatchingType::Lazy,
            special: SpecialOptions::Normal,
        }
    }
}

impl From<Compression> for CompressionOptions {
    fn from(compression: Compression) -> CompressionOptions {
        match compression {
            Compression::Fast => CompressionOptions::fast(),
            Compression::Default => CompressionOptions::default(),
            Compression::Best => CompressionOptions::high(),
        }
    }
}

/// Raw DEFLATE stream of `input` with the given options, appended to `out`.
fn deflate_into(out: &mut Vec<u8>, input: &[u8], options: CompressionOptions) {
    let (cfg, strategy) = options.engine();
    let opts = CompressOptions { level: 6, strategy, verify: false };
    out.extend_from_slice(&encode_with_matcher(input, &opts, cfg));
}

/// Compresses `input` with DEFLATE using the given options.
///
/// ```
/// use verified_deflate::{deflate_bytes_conf, Compression};
///
/// let data = b"This is some test data";
/// let compressed_data = deflate_bytes_conf(data, Compression::Best);
/// # assert_eq!(verified_deflate::decompress(&compressed_data).unwrap(), data);
/// ```
pub fn deflate_bytes_conf<O: Into<CompressionOptions>>(input: &[u8], options: O) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() / 3 + 16);
    deflate_into(&mut out, input, options.into());
    out
}

/// Compresses `input` with DEFLATE at the default level.
///
/// ```
/// use verified_deflate::deflate_bytes;
///
/// let data = b"This is some test data";
/// let compressed_data = deflate_bytes(data);
/// # assert_eq!(verified_deflate::decompress(&compressed_data).unwrap(), data);
/// ```
pub fn deflate_bytes(input: &[u8]) -> Vec<u8> {
    deflate_bytes_conf(input, Compression::Default)
}

/// Compresses `input` with DEFLATE inside a zlib header and trailer
/// (RFC 1950), using the given options. Preset dictionaries are not
/// supported.
///
/// ```
/// use verified_deflate::{deflate_bytes_zlib_conf, Compression};
///
/// let data = b"This is some test data";
/// let compressed_data = deflate_bytes_zlib_conf(data, Compression::Best);
/// # let _ = compressed_data;
/// ```
pub fn deflate_bytes_zlib_conf<O: Into<CompressionOptions>>(input: &[u8], options: O) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() / 3 + 16);
    out.extend_from_slice(&ZLIB_HEADER);
    deflate_into(&mut out, input, options.into());
    out.extend_from_slice(&adler32(input).to_be_bytes());
    out
}

/// Compresses `input` with DEFLATE inside a zlib header and trailer, at
/// the default level.
///
/// ```
/// use verified_deflate::deflate_bytes_zlib;
///
/// let data = b"This is some test data";
/// let compressed_data = deflate_bytes_zlib(data);
/// # let _ = compressed_data;
/// ```
pub fn deflate_bytes_zlib(input: &[u8]) -> Vec<u8> {
    deflate_bytes_zlib_conf(input, Compression::Default)
}

/// Compresses `input` with DEFLATE inside a gzip header and trailer
/// (RFC 1952), using the given header and options.
///
/// ```
/// use verified_deflate::{deflate_bytes_gzip_conf, Compression};
/// use verified_deflate::gzip_header::GzBuilder;
///
/// let data = b"This is some test data";
/// let compressed_data = deflate_bytes_gzip_conf(data, Compression::Best, GzBuilder::new());
/// # let _ = compressed_data;
/// ```
pub fn deflate_bytes_gzip_conf<O: Into<CompressionOptions>>(
    input: &[u8],
    options: O,
    gzip_header: GzBuilder,
) -> Vec<u8> {
    let mut out = gzip_header.into_header();
    out.reserve(input.len() / 3 + 16);
    deflate_into(&mut out, input, options.into());
    out.extend_from_slice(&crc32(input).to_le_bytes());
    out.extend_from_slice(&(input.len() as u32).to_le_bytes());
    out
}

/// Compresses `input` with DEFLATE inside a gzip header (with default
/// fields) and trailer, at the default level.
///
/// ```
/// use verified_deflate::deflate_bytes_gzip;
///
/// let data = b"This is some test data";
/// let compressed_data = deflate_bytes_gzip(data);
/// # let _ = compressed_data;
/// ```
pub fn deflate_bytes_gzip(input: &[u8]) -> Vec<u8> {
    deflate_bytes_gzip_conf(input, Compression::Default, GzBuilder::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(checks: u16, lazy_if_less_than: u16, matching_type: MatchingType) -> CompressionOptions {
        CompressionOptions {
            max_hash_checks: checks,
            lazy_if_less_than,
            matching_type,
            special: SpecialOptions::Normal,
        }
    }

    /// The presets mean what `deflate` documents.
    #[test]
    fn presets_map_to_the_documented_search() {
        let (rle, _) = CompressionOptions::rle().engine();
        assert!(rle.rle, "rle(): run-length matching only");
        let (h, _) = CompressionOptions::huffman_only().engine();
        assert!(!h.rle && h.max_chain == 0, "huffman_only(): no matches at all");
        let (f, _) = CompressionOptions::fast().engine();
        assert!(!f.rle && f.max_chain == 1 && !f.lazy && f.max_lazy == MAX_MATCH, "fast(): one check, greedy");
        let (d, _) = CompressionOptions::default().engine();
        assert!(!d.rle && d.max_chain == 128 && d.lazy && d.max_lazy == 32, "default");
        let (b, _) = CompressionOptions::high().engine();
        assert!(!b.rle && b.max_chain == 1768 && b.lazy && b.max_lazy == 128, "high()");
        for c in [d, b] {
            // `max_hash_checks` alone bounds the search.
            assert_eq!((c.nice_len, c.good_len), (MAX_MATCH, MAX_MATCH + 1));
        }
        assert_eq!(CompressionOptions::from(Compression::Fast), CompressionOptions::fast());
        assert_eq!(CompressionOptions::from(Compression::Default), CompressionOptions::default());
        assert_eq!(CompressionOptions::from(Compression::Best), CompressionOptions::high());
    }

    #[test]
    fn options_map_to_the_search() {
        // Checks above the chain length act as the chain length.
        assert_eq!(opts(u16::MAX, 32, MatchingType::Lazy).engine().0.max_chain, 32 * 1024);
        assert_eq!(opts(32 * 1024, 32, MatchingType::Lazy).engine().0.max_chain, 32 * 1024);
        assert_eq!(opts(7, 32, MatchingType::Lazy).engine().0.max_chain, 7);
        // Lazy matching below the threshold; 0 means never; 258 and above
        // means always.
        let c = opts(10, 5, MatchingType::Lazy).engine().0;
        assert!(c.lazy && c.max_lazy == 5);
        let c = opts(10, 0, MatchingType::Lazy).engine().0;
        assert!(!c.lazy && c.max_lazy == MAX_MATCH);
        let c = opts(10, 1000, MatchingType::Lazy).engine().0;
        assert!(c.lazy && c.max_lazy == MAX_MATCH + 1);
        let c = opts(10, 5, MatchingType::Greedy).engine().0;
        assert!(!c.lazy && c.max_lazy == MAX_MATCH);
        // With no hash checks: Lazy means RLE, Greedy means Huffman only.
        let c = opts(0, 5, MatchingType::Lazy).engine().0;
        assert!(c.rle);
        let c = opts(0, 5, MatchingType::Greedy).engine().0;
        assert!(!c.rle && c.max_chain == 0 && !c.lazy);
        // Special options pick the block type.
        for (special, strategy) in [
            (SpecialOptions::Normal, Strategy::Auto),
            (SpecialOptions::_ForceFixed, Strategy::Fixed),
            (SpecialOptions::_ForceStored, Strategy::Stored),
        ] {
            assert_eq!(CompressionOptions { special, ..CompressionOptions::default() }.engine().1, strategy);
        }
    }

    #[test]
    fn matching_type_display() {
        assert_eq!(MatchingType::Greedy.to_string(), "Greedy matching");
        assert_eq!(MatchingType::Lazy.to_string(), "Lazy matching");
    }
}
