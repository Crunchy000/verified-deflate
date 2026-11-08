//! The LZMA decoder, as the ZIP reader's proof takes it: given, not yet
//! proven.
//!
//! `ZipArchive::read` decodes method-14 entries with the library's
//! `lzma::decompress_zip_entry`. Its own proof is still to come; until then the ZIP proof
//! assumes, through the `external_body` below, that it returns on every
//! input without panicking. It is tested against liblzma and the xz program and fuzzed
//! against liblzma (see VERIFICATION.md). The error type stands in for
//! the library's, whose contents the ZIP proof never looks at.

use vstd::prelude::*;

verus! {

/// Stands for the library's `lzma::LzmaError`.
pub enum LzmaError {
    /// Any reason the data could not be decoded.
    Corrupt,
}

/// Assumed to return on every input, without panicking (see above).
#[verifier::external_body]
pub fn decompress_zip_entry(input: &[u8], size: usize) -> Result<Vec<u8>, LzmaError> {
    let _ = (input, size);
    unimplemented!("trusted: the library's LZMA decoder")
}

} // verus!
