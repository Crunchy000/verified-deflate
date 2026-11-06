//! Verus proofs for verified-deflate.
//!
//! Kani checks the library's code for every input up to a size bound.
//! Verus, a deductive verifier, proves properties for inputs of every
//! size. Each module here holds a copy of library code with its
//! specification and proof; `.github/scripts/verus_drift.py` checks in CI
//! that the copy's executable code is the library's, token for token
//! (`copies.txt` lists the pairs).
//!
//! - `limit_depths`, `build_lengths`: the Huffman code-length builder
//!   always produces a complete code within the limit, for any alphabet.
//! - `name_is_safe`: ZIP entry names are accepted exactly when safe.
//! - `inflate`, with `bitio`, `huffman_decode`, `lz77`, `tables` and
//!   `error`: the decoder returns on every input, without panicking, and
//!   its output stays within the caller's limit.
//! - `deflate`, with `lz77`, `bitio`, `build_lengths` and `tables`: the
//!   encoder returns on every input, level and strategy, without
//!   panicking, and so does the streaming encoder.
//! - `zip`, with `crc32`, `inflate` and their modules: the ZIP reader
//!   returns on every byte string, without panicking, and so does reading
//!   any entry of the archive it returns.
//! - `std_specs`: the few standard-library functions taken as given.
//!
//! Run with the Verus release named in `.github/workflows/verus.yml`:
//!
//! ```sh
//! verus --crate-type=lib verus/src/lib.rs
//! ```

pub mod bitio;
pub mod build_lengths;
pub mod crc32;
pub mod deflate;
pub mod error;
pub mod huffman_decode;
pub mod inflate;
pub mod limit_depths;
pub mod lz77;
pub mod name_is_safe;
pub mod std_specs;
pub mod tables;
pub mod zip;
