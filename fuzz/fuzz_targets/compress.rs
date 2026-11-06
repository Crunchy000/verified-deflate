//! Round trips through the one-shot encoder at every level, strategy and
//! `verify` setting: the stream decodes to the input, with this crate's
//! decoder and with zlib's. And zlib's streams, at every level, decode to
//! the input with this crate's decoder.

#![no_main]

use std::io::{Read, Write};

use arbitrary::Arbitrary;
use flate2::{read::DeflateDecoder, write::DeflateEncoder, Compression};
use libfuzzer_sys::fuzz_target;
use verified_deflate::{compress_with, decompress_with_limit, CompressOptions, Strategy};

const STRATEGIES: [Strategy; 4] = [Strategy::Auto, Strategy::Stored, Strategy::Fixed, Strategy::Dynamic];

#[derive(Arbitrary, Debug)]
struct Input {
    /// Any value: above 9 acts as 9.
    level: u8,
    strategy: u8,
    verify: bool,
    zlib_level: u8,
    data: Vec<u8>,
}

/// Decodes `stream` with this crate, checking it is exactly one stream.
fn ours(stream: &[u8], len: usize) -> Vec<u8> {
    let d = decompress_with_limit(stream, len).expect("our decoder rejects the stream");
    assert_eq!(d.consumed, stream.len(), "the stream does not end where it should");
    d.data
}

fn zlib_decode(stream: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    DeflateDecoder::new(stream).read_to_end(&mut out).expect("zlib rejects the stream");
    out
}

fn zlib_encode(data: &[u8], level: u32) -> Vec<u8> {
    let mut e = DeflateEncoder::new(Vec::new(), Compression::new(level));
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fuzz_target!(|input: Input| {
    let data = &input.data;
    let options = CompressOptions {
        level: input.level,
        strategy: STRATEGIES[usize::from(input.strategy) % 4],
        verify: input.verify,
    };
    let stream = compress_with(data, &options);
    assert!(ours(&stream, data.len()) == *data, "our round trip differs");
    assert!(zlib_decode(&stream) == *data, "zlib decodes our stream differently");

    let theirs = zlib_encode(data, u32::from(input.zlib_level % 10));
    assert!(ours(&theirs, data.len()) == *data, "we decode zlib's stream differently");
});
