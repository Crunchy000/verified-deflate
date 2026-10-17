//! End-to-end Kani harnesses: the real encoder feeding the real decoder.
//!
//! Dynamic-Huffman blocks are not covered end to end here: building a
//! 286-symbol code needs loop unrolling far beyond what a model checker can
//! handle. They are covered compositionally instead (code-length
//! generation, canonical encode/decode, the run-length coded header, bit
//! I/O; see the harnesses in `huffman`, `deflate` and `bitio`), by tests
//! against zlib, and at run time by `CompressOptions::verify`.

use crate::deflate::{compress_with, encode_with_matcher, CompressOptions, Strategy};
use crate::inflate::decompress_with_limit;
use crate::lz77::MatchConfig;
use crate::zip::{FileOptions, ZipArchive, ZipWriter};

fn roundtrip(data: &[u8], level: u8, strategy: Strategy) {
    let opts = CompressOptions { level, strategy, verify: false };
    let packed = if level == 0 {
        compress_with(data, &opts)
    } else {
        // The level's search parameters with small tables (see `lz77::proofs`).
        let cfg = MatchConfig { window_bits: 3, hash_bits: 2, ..MatchConfig::for_level(level) };
        encode_with_matcher(data, &opts, cfg)
    };
    let d = decompress_with_limit(&packed, data.len()).unwrap();
    assert_eq!(d.data.as_slice(), data);
    assert_eq!(d.consumed, packed.len());
}

/// Stored blocks: any 4 bytes, and the empty input, round-trip exactly.
/// (Lengths are fixed per harness: a symbolic length makes every buffer
/// size symbolic, which the model checker handles far less well.)
#[kani::stub(crate::inflate::Output::grow, crate::inflate::grow_is_unreachable)]
#[kani::proof]
#[kani::unwind(6)]
fn roundtrip_stored() {
    let data: [u8; 4] = kani::any();
    roundtrip(&data, 0, Strategy::Stored);
    roundtrip(&[], 0, Strategy::Stored);
}

/// Fixed Huffman with the level-6 lazy matcher: any 4 bytes round-trip
/// exactly (four bytes allow a literal followed by an overlapping match),
/// for every hash function.
#[kani::stub(crate::lz77::Matcher::hash, crate::lz77::any_bucket)]
#[kani::stub(crate::lz77::common_prefix, crate::lz77::common_prefix_ref)]
#[kani::stub(crate::lz77::copy_within_buffer, crate::lz77::copy_within_buffer_ref)]
#[kani::stub(crate::inflate::Output::grow, crate::inflate::grow_is_unreachable)]
#[cfg(any(feature = "kani-extended", kani_sample))] // too large for CI runners
#[kani::proof]
#[kani::unwind(7)]
fn roundtrip_fixed_lazy() {
    let data: [u8; 4] = kani::any();
    roundtrip(&data, 6, Strategy::Fixed);
}

/// Fixed Huffman with the greedy level-1 matcher on any 3 bytes.
#[kani::stub(crate::lz77::Matcher::hash, crate::lz77::any_bucket)]
#[kani::stub(crate::lz77::common_prefix, crate::lz77::common_prefix_ref)]
#[kani::stub(crate::lz77::copy_within_buffer, crate::lz77::copy_within_buffer_ref)]
#[kani::stub(crate::inflate::Output::grow, crate::inflate::grow_is_unreachable)]
#[cfg(any(feature = "kani-extended", kani_sample))] // too large for CI runners
#[kani::proof]
#[kani::unwind(6)]
fn roundtrip_fixed_greedy() {
    let data: [u8; 3] = kani::any();
    roundtrip(&data, 1, Strategy::Fixed);
}

/// Inputs built from two symbols are match-heavy; any 6 of them round-trip
/// with the level-9 matcher.
#[kani::stub(crate::lz77::Matcher::hash, crate::lz77::any_bucket)]
#[kani::stub(crate::lz77::common_prefix, crate::lz77::common_prefix_ref)]
#[kani::stub(crate::lz77::copy_within_buffer, crate::lz77::copy_within_buffer_ref)]
#[kani::stub(crate::inflate::Output::grow, crate::inflate::grow_is_unreachable)]
#[cfg(any(feature = "kani-extended", kani_sample))] // too large for CI runners
#[kani::proof]
#[kani::unwind(8)]
fn roundtrip_fixed_two_symbol_alphabet() {
    let bits: [bool; 6] = kani::any();
    let a: u8 = kani::any();
    let b: u8 = kani::any();
    let mut data = [0u8; 6];
    for i in 0..6 {
        data[i] = if bits[i] { a } else { b };
    }
    roundtrip(&data, 9, Strategy::Fixed);
}

/// A one-entry ZIP archive of any 3 bytes, stored, reads back exactly with
/// its CRC check passing.
#[kani::stub(crate::inflate::Output::grow, crate::inflate::grow_is_unreachable)]
#[cfg(any(feature = "kani-extended", kani_sample))] // too large for CI runners
#[kani::proof]
#[kani::unwind(8)]
fn zip_roundtrip_stored() {
    let data: [u8; 3] = kani::any();
    let opts = FileOptions { level: 0, verify: false, ..FileOptions::default() };
    let mut w = ZipWriter::new();
    w.add_file("f", &data, &opts).unwrap();
    let bytes = w.finish().unwrap();
    let a = ZipArchive::parse(&bytes).unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a.read(0).unwrap().as_slice(), &data[..]);
}
