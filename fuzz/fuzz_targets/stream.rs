//! The streaming encoders (raw DEFLATE, zlib and gzip), with any options,
//! fed any sequence of writes and flushes: what they produce decodes to
//! what was written, with zlib, and for raw DEFLATE with this crate too.

#![no_main]

use std::io::{Read, Write};

use arbitrary::Arbitrary;
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use libfuzzer_sys::fuzz_target;
use verified_deflate::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use verified_deflate::{decompress_with_limit, CompressionOptions, MatchingType, SpecialOptions};

#[derive(Arbitrary, Debug)]
enum Op {
    Write(Vec<u8>),
    Flush,
}

#[derive(Arbitrary, Debug)]
struct Input {
    container: u8,
    max_hash_checks: u16,
    lazy_if_less_than: u16,
    lazy: bool,
    special: u8,
    ops: Vec<Op>,
}

/// Applies the operations, returning everything written.
fn feed<W: Write>(w: &mut W, ops: &[Op]) -> Vec<u8> {
    let mut written = Vec::new();
    for op in ops {
        match op {
            Op::Write(b) => {
                w.write_all(b).unwrap();
                written.extend_from_slice(b);
            }
            Op::Flush => w.flush().unwrap(),
        }
    }
    written
}

fn read_all<R: Read>(mut r: R) -> Vec<u8> {
    let mut out = Vec::new();
    r.read_to_end(&mut out).expect("zlib rejects the stream");
    out
}

fuzz_target!(|input: Input| {
    let options = CompressionOptions {
        max_hash_checks: input.max_hash_checks,
        lazy_if_less_than: input.lazy_if_less_than,
        matching_type: if input.lazy { MatchingType::Lazy } else { MatchingType::Greedy },
        special: match input.special % 3 {
            0 => SpecialOptions::Normal,
            1 => SpecialOptions::_ForceFixed,
            _ => SpecialOptions::_ForceStored,
        },
    };
    match input.container % 3 {
        0 => {
            let mut e = DeflateEncoder::new(Vec::new(), options);
            let written = feed(&mut e, &input.ops);
            let stream = e.finish().unwrap();
            assert!(read_all(DeflateDecoder::new(&stream[..])) == written, "zlib decodes it differently");
            let d = decompress_with_limit(&stream, written.len()).expect("our decoder rejects the stream");
            assert!(d.data == written, "we decode it differently");
            assert_eq!(d.consumed, stream.len(), "the stream does not end where it should");
        }
        1 => {
            let mut e = ZlibEncoder::new(Vec::new(), options);
            let written = feed(&mut e, &input.ops);
            let stream = e.finish().unwrap();
            assert!(read_all(ZlibDecoder::new(&stream[..])) == written, "zlib decodes it differently");
        }
        _ => {
            let mut e = GzEncoder::new(Vec::new(), options);
            let written = feed(&mut e, &input.ops);
            let stream = e.finish().unwrap();
            assert!(read_all(GzDecoder::new(&stream[..])) == written, "zlib decodes it differently");
        }
    }
});
