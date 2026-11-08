# Fuzzing

Coverage-guided differential fuzzing with [cargo-fuzz] (libFuzzer). The
references are C zlib, through `flate2`; for Deflate64, zlib's
`infback9`, vendored in `infback9/` (see its README); for bzip2, C
libbz2 1.0.8, built from the source the `bzip2-sys` crate bundles; and
for LZMA and xz, the system's liblzma (install `liblzma-dev`; without it
`lzma-sys` builds its bundled, older XZ Utils). This crate is separate
from the library, which keeps no dependencies.

| Target | Checks |
|---|---|
| `inflate` | On arbitrary bytes, the decoder and zlib accept the same streams, with the same output and the same number of bytes consumed. |
| `inflate64` | The same for the Deflate64 decoder, against zlib's `infback9` (Mark Adler's Deflate64 decoder, built from `infback9/`). |
| `compress` | `compress_with` at every level, strategy and `verify` setting: the stream decodes to the input with this crate and with zlib. zlib's streams, at every level, decode to the input with this crate. |
| `stream` | The streaming raw, zlib and gzip encoders, with any `CompressionOptions` and any sequence of writes and flushes: zlib decodes what they produce to what was written, and for raw streams so does this crate. |
| `zip_read` | The ZIP reader on arbitrary bytes. An entry that reads successfully has its recorded size and CRC-32, and lookup by name finds the first entry of that name. (That it never panics, Verus proves, taking bzip2 decoding as given.) Entries declaring over 16 MiB are not read. |
| `zip_roundtrip` | Archives the writer makes from any entries and options, deflated, bzip2-compressed or stored, read back exactly. Names it refuses are invalid or repeated. |
| `gzip_zlib` | A gzip member and a zlib stream are accepted exactly when C zlib accepts them (through its own wrappers), with the same output and bytes consumed. |
| `tar_read` | Arbitrary bytes as a tar archive: entries' data lies inside it at the recorded size, in order, and paths made safe stay relative. |
| `tar_roundtrip` | Archives the tar writer makes from any entries and metadata read back exactly, including what needs pax headers. |
| `bzip2_decode` | On arbitrary bytes, the bzip2 decoder and libbz2 accept the same streams, with the same output and bytes consumed. Each input is tried again with its CRCs rewritten to match its blocks, so changes inside blocks meet the structural checks instead of a CRC. |
| `bzip2_roundtrip` | The bzip2 encoder at every level, plain and with "randomised" blocks: libbz2 and this crate decode what it writes to the input. |
| `lzma_decode` | On arbitrary bytes, as a raw LZMA1 stream, a raw LZMA2 stream, a `.lzma` file or an `.xz` stream (the first byte picks), the decoders and liblzma accept the same streams, with the same output and bytes consumed. |
| `xz_roundtrip` | The `.xz` encoder at every level, with every check and any delta and branch filters, and the `.lzma` encoder: liblzma and this crate decode what they write to the input. |

To run one locally (nightly Rust):

```sh
cargo install cargo-fuzz
python3 fuzz/seeds.py                      # starting inputs (Deflate64 ones need `pip install inflate64`)
cd fuzz
cargo fuzz run --sanitizer none --debug-assertions inflate -- -max_total_time=60
```

No sanitizer is needed, as the library forbids unsafe code. Debug
assertions and overflow checks are on. CI (`.github/workflows/fuzz.yml`)
fuzzes each target for a minute on every push and pull request that
touches the code. It also fuzzes each for 20 minutes every night, keeping
the corpus in the Actions cache. A failing input is uploaded as a
workflow artifact. Reproduce it with:

```sh
cargo fuzz run --sanitizer none --debug-assertions <target> <file>
```

[cargo-fuzz]: https://github.com/rust-fuzz/cargo-fuzz
