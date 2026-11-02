//! Verus proofs for verified-deflate.
//!
//! Kani checks the library's code for every input up to a size bound.
//! Verus, a deductive verifier, proves properties for inputs of every
//! size. Each module here holds a copy of one library function with its
//! specification and proof; `.github/scripts/verus_drift.py` checks in CI
//! that the copy's executable code is the library's, token for token.
//!
//! Run with the Verus release named in `.github/workflows/verus.yml`:
//!
//! ```sh
//! verus --crate-type=lib verus/src/lib.rs
//! ```

pub mod limit_depths;
pub mod name_is_safe;
