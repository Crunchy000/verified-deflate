//! The bzip2 encoder, on any data at any level, plain and with
//! "randomised" blocks: C libbz2 decodes what it writes back to the data,
//! and so does this crate.

#![no_main]

use bzip2_sys::{bz_stream, BZ2_bzDecompress, BZ2_bzDecompressEnd, BZ2_bzDecompressInit, BZ_OK, BZ_STREAM_END};
use libfuzzer_sys::fuzz_target;
use verified_deflate::bzip2;

/// libbz2's decoding of one whole stream into exactly `len` bytes.
fn reference(input: &[u8], len: usize) -> Option<Vec<u8>> {
    // One byte more than expected, so too much output is noticed.
    let mut out = vec![0u8; len + 1];
    // SAFETY: as in bzip2_decode.rs.
    unsafe {
        let mut s: bz_stream = std::mem::zeroed();
        if BZ2_bzDecompressInit(&mut s, 0, 0) != BZ_OK {
            return None;
        }
        s.next_in = input.as_ptr() as *mut _;
        s.avail_in = input.len() as u32;
        s.next_out = out.as_mut_ptr() as *mut _;
        s.avail_out = out.len() as u32;
        let r = BZ2_bzDecompress(&mut s);
        let produced = out.len() - s.avail_out as usize;
        let rest = s.avail_in;
        BZ2_bzDecompressEnd(&mut s);
        (r == BZ_STREAM_END && rest == 0).then(|| {
            out.truncate(produced);
            out
        })
    }
}

fuzz_target!(|input: &[u8]| {
    let Some((&control, data)) = input.split_first() else { return };
    // Levels 1-9 from the low bits; occasionally repeat the data so runs
    // and blocks get long.
    let level = control % 9 + 1;
    let data = if control & 0x80 != 0 { data.repeat(64) } else { data.to_vec() };
    let randomised = control & 0x40 != 0;
    let packed = if randomised { bzip2::compress_randomised(&data, level) } else { bzip2::compress(&data, level) };
    assert_eq!(packed[3], b'0' + level);
    assert!(reference(&packed, data.len()).as_deref() == Some(&data[..]), "libbz2 disagrees (level {level}, randomised {randomised})");
    assert!(bzip2::decompress(&packed).as_deref() == Ok(&data[..]), "round trip failed");
});
