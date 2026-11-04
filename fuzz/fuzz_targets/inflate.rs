//! The decoder against zlib on arbitrary bytes: both accept the same
//! streams, with the same output and the same number of bytes consumed.

#![no_main]

use flate2::{Decompress, FlushDecompress, Status};
use libfuzzer_sys::fuzz_target;
use verified_deflate::decompress_with_limit;

/// Output limit for both decoders.
const LIMIT: usize = 1 << 20;

/// Output and bytes consumed of an accepted stream.
type Accepted = Option<(Vec<u8>, usize)>;

fn ours(input: &[u8]) -> Accepted {
    decompress_with_limit(input, LIMIT).ok().map(|d| (d.data, d.consumed))
}

fn zlib(input: &[u8]) -> Accepted {
    let mut z = Decompress::new(false);
    // Room for one byte past the limit, so that "too long" shows.
    let mut out = Vec::with_capacity(LIMIT + 1);
    match z.decompress_vec(input, &mut out, FlushDecompress::Finish) {
        Ok(Status::StreamEnd) if out.len() <= LIMIT => Some((out, z.total_in() as usize)),
        _ => None,
    }
}

fuzz_target!(|input: &[u8]| {
    // A rejected stream may be rejected for different reasons (zlib, for
    // example, reports a truncated stream that is also too long as
    // truncated), so only acceptance and its results are compared.
    match (ours(input), zlib(input)) {
        (Some(a), Some(b)) => {
            assert!(a.0 == b.0, "different output");
            assert_eq!(a.1, b.1, "different bytes consumed");
        }
        (None, None) => {}
        (a, b) => panic!("ours accepts: {}, zlib accepts: {}", a.is_some(), b.is_some()),
    }
});
