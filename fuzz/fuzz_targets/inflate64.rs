//! The Deflate64 decoder against zlib's infback9 (Mark Adler's Deflate64
//! decoder, built from `fuzz/infback9`) on arbitrary bytes: both accept the
//! same streams, with the same output and the same number of bytes consumed.

#![no_main]

use std::os::raw::c_int;

use libfuzzer_sys::fuzz_target;
use verified_deflate::decompress_deflate64_with_limit;

/// Output limit for both decoders.
const LIMIT: usize = 1 << 20;

/// Output and bytes consumed of an accepted stream.
type Accepted = Option<(Vec<u8>, usize)>;

extern "C" {
    /// `fuzz/infback9/reference.c`: 0 for a complete, valid stream, 1 if
    /// its output passes `limit`, -1 if invalid or truncated, -2 if out of
    /// memory.
    fn reference_inflate9(
        input: *const u8,
        input_len: usize,
        out: *mut u8,
        limit: usize,
        out_len: *mut usize,
        consumed: *mut usize,
    ) -> c_int;
}

fn ours(input: &[u8]) -> Accepted {
    decompress_deflate64_with_limit(input, LIMIT).ok().map(|d| (d.data, d.consumed))
}

fn infback9(input: &[u8]) -> Accepted {
    let mut out = vec![0u8; LIMIT];
    let (mut len, mut consumed) = (0usize, 0usize);
    // SAFETY: `input` and `out` are valid for the lengths passed; the C code
    // writes at most `LIMIT` bytes to `out` and one value through each
    // count pointer, and keeps none of the pointers after it returns.
    let r =
        unsafe { reference_inflate9(input.as_ptr(), input.len(), out.as_mut_ptr(), LIMIT, &mut len, &mut consumed) };
    assert!(r != -2, "infback9 could not allocate");
    (r == 0).then(|| {
        out.truncate(len);
        (out, consumed)
    })
}

fuzz_target!(|input: &[u8]| {
    // As in `inflate`: rejections may differ in reason, so only acceptance
    // and its results are compared.
    match (ours(input), infback9(input)) {
        (Some(a), Some(b)) => {
            assert!(a.0 == b.0, "different output");
            assert_eq!(a.1, b.1, "different bytes consumed");
        }
        (None, None) => {}
        (a, b) => panic!("ours accepts: {}, infback9 accepts: {}", a.is_some(), b.is_some()),
    }
});
