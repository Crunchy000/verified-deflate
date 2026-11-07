//! The gzip and zlib readers against C zlib's own handling of those
//! wrappers, on arbitrary bytes: a single gzip member, and a zlib stream,
//! are accepted by both or by neither, with the same output and the same
//! number of bytes consumed.

#![no_main]

use flate2::{Decompress, FlushDecompress, Status};
use libfuzzer_sys::fuzz_target;
use verified_deflate::{gzip, zlib};

/// Output limit for both decoders.
const LIMIT: usize = 1 << 20;

type Accepted = Option<(Vec<u8>, usize)>;

/// zlib's inflate with the given wrapper, all input at once.
fn reference(mut z: Decompress, input: &[u8]) -> Accepted {
    let mut out = Vec::with_capacity(LIMIT + 1);
    match z.decompress_vec(input, &mut out, FlushDecompress::Finish) {
        Ok(Status::StreamEnd) if out.len() <= LIMIT => Some((out, z.total_in() as usize)),
        _ => None,
    }
}

fn compare(what: &str, ours: Accepted, theirs: Accepted) {
    match (ours, theirs) {
        (Some(a), Some(b)) => {
            assert!(a.0 == b.0, "{what}: different output");
            assert_eq!(a.1, b.1, "{what}: different bytes consumed");
        }
        (None, None) => {}
        (a, b) => panic!("{what}: ours accepts {}, zlib accepts {}", a.is_some(), b.is_some()),
    }
}

fuzz_target!(|input: &[u8]| {
    let ours = gzip::read_member(input, LIMIT).ok().map(|m| (m.data, m.consumed));
    compare("gzip", ours, reference(Decompress::new_gzip(15), input));
    let ours = zlib::decompress_with_limit(input, LIMIT).ok().map(|d| (d.data, d.consumed));
    compare("zlib", ours, reference(Decompress::new(true), input));
    // Whole files: concatenated members decode to the members' data.
    if let Ok(all) = gzip::decompress_with_limit(input, LIMIT) {
        let first = gzip::read_member(input, LIMIT).unwrap();
        assert!(all.starts_with(&first.data));
    }
});
