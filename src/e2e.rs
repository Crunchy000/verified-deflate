//! End-to-end Kani harnesses: the real encoder feeding the real decoder.
//!
//! Dynamic-Huffman blocks are not covered end to end here: building a
//! 286-symbol code needs loop unrolling far beyond what a model checker can
//! handle. They are covered compositionally instead (code-length
//! generation, canonical encode/decode, the run-length coded header, bit
//! I/O; see the harnesses in `huffman`, `deflate` and `bitio`), by tests
//! against zlib, and at run time by `CompressOptions::verify`.

use crate::deflate::{compress_with, CompressOptions, Strategy};
use crate::inflate::decompress_with_limit;
use crate::zip::{FileOptions, ZipArchive, ZipWriter};

fn roundtrip(data: &[u8], level: u8, strategy: Strategy) {
    let packed = compress_with(data, &CompressOptions { level, strategy, verify: false });
    let d = decompress_with_limit(&packed, data.len()).unwrap();
    assert_eq!(d.data.as_slice(), data);
    assert_eq!(d.consumed, packed.len());
}

/// Stored blocks: any input of up to 8 bytes round-trips exactly.
#[kani::proof]
#[kani::unwind(10)]
fn roundtrip_stored() {
    let data: [u8; 8] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= 8);
    roundtrip(&data[..len], 0, Strategy::Stored);
}

/// Fixed Huffman with the level-6 lazy matcher: any input of up to 5 bytes
/// round-trips exactly (literals, matches and overlapping matches).
#[kani::proof]
#[kani::unwind(17)]
fn roundtrip_fixed_lazy() {
    let data: [u8; 5] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= 5);
    roundtrip(&data[..len], 6, Strategy::Fixed);
}

/// Fixed Huffman with the greedy level-1 matcher on 6 bytes.
#[kani::proof]
#[kani::unwind(17)]
fn roundtrip_fixed_greedy() {
    let data: [u8; 6] = kani::any();
    roundtrip(&data, 1, Strategy::Fixed);
}

/// Inputs built from two symbols are match-heavy; 8 of them round-trip.
#[kani::proof]
#[kani::unwind(17)]
fn roundtrip_fixed_two_symbol_alphabet() {
    let bits: [bool; 8] = kani::any();
    let a: u8 = kani::any();
    let b: u8 = kani::any();
    let mut data = [0u8; 8];
    for i in 0..8 {
        data[i] = if bits[i] { a } else { b };
    }
    roundtrip(&data, 9, Strategy::Fixed);
}

/// A one-entry ZIP archive of any stored content up to 4 bytes reads back
/// exactly, with the CRC check passing.
#[kani::proof]
#[kani::unwind(12)]
fn zip_roundtrip_stored() {
    let data: [u8; 4] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= 4);
    let opts = FileOptions { level: 0, verify: false, ..FileOptions::default() };
    let mut w = ZipWriter::new();
    w.add_file("f", &data[..len], &opts).unwrap();
    let bytes = w.finish().unwrap();
    let a = ZipArchive::parse(&bytes).unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a.read(0).unwrap().as_slice(), &data[..len]);
}
