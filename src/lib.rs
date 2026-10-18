//! # verified-deflate
//!
//! A DEFLATE ([RFC 1951]) compressor and decompressor plus ZIP archive
//! support, written in safe Rust with no dependencies, with bounded proofs
//! of its key properties checked by the [Kani] model checker.
//!
//! ```
//! use verified_deflate::{compress, decompress};
//!
//! let data = b"to be or not to be, that is the question";
//! let packed = compress(data, 6);
//! assert_eq!(decompress(&packed).unwrap(), data);
//! ```
//!
//! ```
//! use verified_deflate::zip::{FileOptions, ZipArchive, ZipWriter};
//!
//! let mut w = ZipWriter::new();
//! w.add_file("hello.txt", b"hello, world", &FileOptions::default()).unwrap();
//! let bytes = w.finish().unwrap();
//!
//! let archive = ZipArchive::parse(&bytes).unwrap();
//! assert_eq!(archive.read_by_name("hello.txt").unwrap(), b"hello, world");
//! ```
//!
//! ## The `deflate` crate's API
//!
//! The whole API of the [`deflate`](https://docs.rs/deflate/1.0.0) crate
//! (version 1.0) is available at this crate's root: [`deflate_bytes`] and
//! its `_conf`, `_zlib` and `_gzip` variants, [`Compression`],
//! [`CompressionOptions`], [`MatchingType`], [`SpecialOptions`] and the
//! streaming encoders in [`write`](mod@write). Code written for `deflate` compiles
//! unchanged with this crate renamed to `deflate` in `Cargo.toml`; gzip
//! headers use [`gzip_header::GzBuilder`] in place of the `gzip-header`
//! crate's.
//!
//! ```
//! use std::io::Write;
//! use verified_deflate::write::GzEncoder;
//! use verified_deflate::{deflate_bytes_zlib_conf, Compression};
//!
//! let zlib = deflate_bytes_zlib_conf(b"some data", Compression::Best);
//! let mut gz = GzEncoder::new(Vec::new(), Compression::Default);
//! gz.write_all(b"some data")?;
//! let gzip = gz.finish()?;
//! # let _ = (zlib, gzip);
//! # Ok::<(), std::io::Error>(())
//! ```
//!
//! ## What is verified
//!
//! Each module carries `#[kani::proof]` harnesses (run with `cargo kani`).
//! For all inputs up to the stated sizes, they check: bit I/O round trips;
//! Adler-32 equals its definition; the slicing-by-8 CRC-32 equals its
//! bitwise definition; the length and
//! distance tables are correct for every value; canonical Huffman codes
//! are prefix-free; generated code lengths are complete and
//! length-limited; Huffman encode then decode is the identity, and the
//! table-driven decoder equals the bit-by-bit one; LZ77 tokens are valid
//! and describe the input; stored streams round-trip end to end; ZIP
//! headers round-trip and the archive parser's slices are always in
//! bounds. See `VERIFICATION.md` for the full list, bounds, and what is
//! *not* proven.
//!
//! [RFC 1951]: https://www.rfc-editor.org/rfc/rfc1951
//! [Kani]: https://github.com/model-checking/kani

#![forbid(unsafe_code)]
// Proof harnesses spell out bounds as explicit comparisons, like a spec.
#![cfg_attr(kani, allow(clippy::manual_range_contains, clippy::needless_range_loop))]

pub mod adler32;
pub mod bitio;
mod compat;
pub mod crc32;
pub mod deflate;
mod error;
pub mod gzip_header;
pub mod huffman;
pub mod inflate;
pub mod lz77;
pub mod tables;
pub mod write;
pub mod zip;

#[cfg(kani)]
mod e2e;

pub use adler32::{adler32, Adler32};
pub use compat::{
    deflate_bytes, deflate_bytes_conf, deflate_bytes_gzip, deflate_bytes_gzip_conf, deflate_bytes_zlib,
    deflate_bytes_zlib_conf, Compression, CompressionOptions, MatchingType, SpecialOptions,
};
pub use crc32::{crc32, Crc32};
pub use deflate::{compress, compress_with, CompressOptions, Strategy};
pub use error::InflateError;
pub use inflate::{decompress, decompress_with_limit, Decompressed};
