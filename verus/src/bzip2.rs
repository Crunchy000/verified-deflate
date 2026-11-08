//! The bzip2 decoder, as the ZIP reader's proof takes it: given, not yet
//! proven.
//!
//! `ZipArchive::read` decodes method-12 entries with the library's
//! `bzip2::decompress_with_limit`. Its own proof is still to come; until
//! then the ZIP proof assumes, through the `external_body` below, that it
//! returns on every input without panicking. It is tested against libbz2
//! and the bzip2 program, and fuzzed against C libbz2 (see
//! VERIFICATION.md). The error type stands in for the library's, whose
//! contents the ZIP proof never looks at.

use vstd::prelude::*;

verus! {

/// Stands for the library's `bzip2::Bzip2Error`.
pub enum Bzip2Error {
    /// Any reason a bzip2 stream could not be decoded.
    Corrupt,
}

/// Assumed to return on every input, without panicking (see above).
#[verifier::external_body]
pub fn decompress_with_limit(input: &[u8], max_output: usize) -> Result<Vec<u8>, Bzip2Error> {
    let _ = (input, max_output);
    unimplemented!("trusted: the library's bzip2 decoder")
}

} // verus!
