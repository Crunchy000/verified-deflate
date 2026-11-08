//! The bzip2 decoder against C libbz2 1.0.8, on arbitrary bytes: a stream
//! is accepted by both or by neither, with the same output and the same
//! number of bytes consumed. Each input is tried as it is, and again with
//! its CRCs rewritten to match what its blocks decode to, so that changes
//! inside blocks are judged by the decoders' structural checks rather
//! than all caught by a CRC.

#![no_main]

use bzip2_sys::{bz_stream, BZ2_bzDecompress, BZ2_bzDecompressEnd, BZ2_bzDecompressInit, BZ_OK, BZ_STREAM_END};
use libfuzzer_sys::fuzz_target;
use verified_deflate::bzip2;

/// Output limit for both decoders.
const LIMIT: usize = 1 << 20;

type Accepted = Option<(Vec<u8>, usize)>;

/// libbz2's decoder, all input at once: it stops at the end of the first
/// stream, or when it needs more input or more room for output.
fn reference(input: &[u8]) -> Accepted {
    let mut out = vec![0u8; LIMIT + 1];
    // SAFETY: the stream is zero-initialised as libbz2 requires (null
    // allocators select malloc and free), points only into `input` and
    // `out` while they live, and is ended before they are dropped.
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
        let consumed = input.len() - s.avail_in as usize;
        BZ2_bzDecompressEnd(&mut s);
        if r == BZ_STREAM_END && produced <= LIMIT {
            out.truncate(produced);
            Some((out, consumed))
        } else {
            None
        }
    }
}

fn compare(what: &str, input: &[u8]) {
    let ours = bzip2::read_stream(input, LIMIT).ok().map(|s| (s.data, s.consumed));
    match (ours, reference(input)) {
        (Some(a), Some(b)) => {
            assert!(a.0 == b.0, "{what}: different output");
            assert_eq!(a.1, b.1, "{what}: different bytes consumed");
        }
        (None, None) => {}
        (a, b) => panic!("{what}: ours accepts {}, libbz2 accepts {}", a.is_some(), b.is_some()),
    }
}

fuzz_target!(|input: &[u8]| {
    compare("as is", input);
    let mut fixed = input.to_vec();
    if bzip2::fix_crcs(&mut fixed, LIMIT) > 0 && fixed != input {
        compare("with CRCs fixed", &fixed);
    }
    // Whole files: concatenated streams decode to the streams' data.
    if let Ok(all) = bzip2::decompress_with_limit(input, LIMIT) {
        let first = bzip2::read_stream(input, LIMIT).unwrap();
        assert!(all.starts_with(&first.data));
    }
});
