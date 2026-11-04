# Fuzzing

Coverage-guided differential fuzzing with [cargo-fuzz] (libFuzzer). The
reference is C zlib, through `flate2`. This crate is separate from the
library, which keeps no dependencies.

| Target | Checks |
|---|---|
| `inflate` | On arbitrary bytes, the decoder and zlib accept the same streams, with the same output and the same number of bytes consumed. |
| `compress` | `compress_with` at every level, strategy and `verify` setting: the stream decodes to the input with this crate and with zlib. zlib's streams, at every level, decode to the input with this crate. |
| `stream` | The streaming raw, zlib and gzip encoders, with any `CompressionOptions` and any sequence of writes and flushes: zlib decodes what they produce to what was written, and for raw streams so does this crate. |
| `zip_read` | The ZIP reader on arbitrary bytes. An entry that reads successfully has its recorded size and CRC-32, and lookup by name finds the first entry of that name. (That it never panics, Verus proves.) |
| `zip_roundtrip` | Archives the writer makes from any entries and options read back exactly. Names it refuses are invalid or repeated. |

To run one locally (nightly Rust):

```sh
cargo install cargo-fuzz
python3 fuzz/seeds.py                      # starting inputs for inflate and zip_read
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
