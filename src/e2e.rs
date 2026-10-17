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

/// Stored blocks: any input of up to 4 bytes round-trips exactly.
#[kani::proof]
#[kani::unwind(6)]
fn roundtrip_stored() {
    let data: [u8; 4] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= 4);
    roundtrip(&data[..len], 0, Strategy::Stored);
}

/// Fixed Huffman with the level-6 lazy matcher: any input of up to 4 bytes
/// round-trips exactly (four bytes allow a literal followed by a match).
/// Fixed codes are at most 9 bits, which bounds the decoder's bit loop.
#[kani::proof]
#[kani::unwind(10)]
fn roundtrip_fixed_lazy() {
    let data: [u8; 4] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= 4);
    roundtrip(&data[..len], 6, Strategy::Fixed);
}

/// Fixed Huffman with the greedy level-1 matcher on any 4 bytes.
#[kani::proof]
#[kani::unwind(10)]
fn roundtrip_fixed_greedy() {
    let data: [u8; 4] = kani::any();
    roundtrip(&data, 1, Strategy::Fixed);
}

/// Inputs built from two symbols are match-heavy; any 6 of them round-trip
/// with the level-9 matcher.
#[kani::proof]
#[kani::unwind(10)]
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

/// A one-entry ZIP archive of any stored content up to 3 bytes reads back
/// exactly, with the CRC check passing.
#[kani::proof]
#[kani::unwind(8)]
fn zip_roundtrip_stored() {
    let data: [u8; 3] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= 3);
    let opts = FileOptions { level: 0, verify: false, ..FileOptions::default() };
    let mut w = ZipWriter::new();
    w.add_file("f", &data[..len], &opts).unwrap();
    let bytes = w.finish().unwrap();
    let a = ZipArchive::parse(&bytes).unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a.read(0).unwrap().as_slice(), &data[..len]);
}
