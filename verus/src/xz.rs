//! The xz decoder, as the ZIP reader's proof takes it: given, not yet
//! proven.
//!
//! `ZipArchive::read` decodes method-95 entries with the library's
//! `xz::decompress_with_limit`. Its own proof is still to come; until then the ZIP proof
//! assumes, through the `external_body` below, that it returns on every
//! input without panicking. It is tested against liblzma, the xz program and 7-Zip and fuzzed
//! against liblzma (see VERIFICATION.md). The error type stands in for
//! the library's, whose contents the ZIP proof never looks at.

use vstd::prelude::*;

verus! {

/// Stands for the library's `xz::XzError`.
pub enum XzError {
    /// Any reason the data could not be decoded.
    Corrupt,
}

/// Assumed to return on every input, without panicking (see above).
#[verifier::external_body]
pub fn decompress_with_limit(input: &[u8], max_output: usize) -> Result<Vec<u8>, XzError> {
    let _ = (input, max_output);
    unimplemented!("trusted: the library's xz decoder")
}

} // verus!
