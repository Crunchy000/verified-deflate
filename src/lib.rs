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
//! ## What is verified
//!
//! Each module carries `#[kani::proof]` harnesses (run with `cargo kani`).
//! They check, for all inputs up to the stated sizes: bit I/O round trips;
//! CRC-32 equals its bitwise definition; the length and distance tables
//! are correct for every value; canonical Huffman codes are prefix-free;
//! generated code lengths are complete and length-limited; Huffman encode
//! then decode is the identity; LZ77 tokens are valid and replay to the
//! input; stored and fixed-Huffman streams round-trip end to end; the
//! decoder never panics and respects its output limit; ZIP headers round
//! trip and the archive parser never panics. See `VERIFICATION.md` for the
//! full list, bounds, and what is *not* proven.
//!
//! [RFC 1951]: https://www.rfc-editor.org/rfc/rfc1951
//! [Kani]: https://github.com/model-checking/kani

#![forbid(unsafe_code)]
// Proof harnesses spell out bounds as explicit comparisons, like a spec.
#![cfg_attr(kani, allow(clippy::manual_range_contains, clippy::needless_range_loop))]

pub mod bitio;
pub mod crc32;
pub mod deflate;
mod error;
pub mod huffman;
pub mod inflate;
pub mod lz77;
pub mod tables;
pub mod zip;

#[cfg(kani)]
mod e2e;

pub use crc32::{crc32, Crc32};
pub use deflate::{compress, compress_with, CompressOptions, Strategy};
pub use error::InflateError;
pub use inflate::{decompress, decompress_with_limit, Decompressed};
