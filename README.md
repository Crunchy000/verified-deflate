# verified-deflate

A DEFLATE ([RFC 1951]) compressor and decompressor with ZIP archive
support, in safe Rust with no dependencies. Its key properties are proven
with the [Kani] model checker, and it is cross-checked against the
reference zlib and Info-ZIP.

- **Complete DEFLATE**: stored, fixed-Huffman and dynamic-Huffman blocks,
  hash-chain LZ77 with lazy matching, levels 0–9. Each block uses whichever
  of the three encodings is smallest, computed exactly.
- **Strict, panic-free decoder** that rejects malformed streams the way
  zlib does, with an output limit to stop decompression bombs.
- **ZIP archives**: writer and reader for stored and deflated entries,
  UTF-8 names, Unix permissions, timestamps and data descriptors. Every
  entry is checked against its CRC-32.
- **`vzip`**, a small command-line tool to create, extract, list and test
  archives.
- `#![forbid(unsafe_code)]`, zero dependencies, MSRV 1.83.

## Verification

47 Kani harnesses sit next to the code they check and run in CI on every
push. For every input up to the stated sizes, they prove these
properties:

- Bit-level I/O round-trips, and reads fail cleanly at end of input.
- CRC-32's table step equals eight bitwise steps for every register and
  byte, so the CRC is correct for inputs of any length.
- The length and distance code tables are correct for **every** length
  (3–258) and distance (1–32768).
- Canonical Huffman codes are prefix-free. Generated code lengths always
  form a complete code within the length limit. Huffman encode-then-decode
  is the identity, and decoding arbitrary bits never panics.
- LZ77 tokens are always valid and describe exactly the input, for every
  hash function. The overlapping match copy meets its specification.
- The encoder's code-length run-length coding is undone by the decoder's
  own routine.
- Stored streams round-trip end to end, including inside a ZIP archive.
- ZIP headers round-trip. Every slice the archive parser takes is in
  bounds, for archives of any size. Name validation blocks path traversal.

Six larger harnesses are gated behind the `kani-extended` feature
because they need more memory than a CI runner has: whole-stream decoder
no-panic and fixed-Huffman round trips. Dynamic-Huffman blocks are covered
compositionally rather than end to end.
[VERIFICATION.md](VERIFICATION.md) lists every harness, its bounds, the
modelling choices behind it, and what is **not** proven.

Two further checks run on top of the proofs. The test suite compares the
engine against real zlib in both directions: zlib decodes every stream
this crate produces, and this crate decodes about 1,400 zlib streams across
all levels, strategies and window sizes. Archives are validated with
`unzip -t` and Python's `zipfile`. Separately, `CompressOptions::verify`
decompresses every stream before returning it, so a wrong stream can never
be returned.

## Usage

```rust
use verified_deflate::{compress, decompress_with_limit};

let data = b"to be or not to be, that is the question";
let packed = compress(data, 6);                       // raw DEFLATE, level 0-9
let unpacked = decompress_with_limit(&packed, 1 << 20).unwrap();
assert_eq!(unpacked.data, data);
```

```rust
use verified_deflate::zip::{FileOptions, ZipArchive, ZipWriter};

let mut w = ZipWriter::new();
w.add_directory("docs", &FileOptions::default()).unwrap();
w.add_file("docs/hello.txt", b"hello, world", &FileOptions::default()).unwrap();
let bytes = w.finish().unwrap();

let archive = ZipArchive::parse(&bytes).unwrap();
assert_eq!(archive.read_by_name("docs/hello.txt").unwrap(), b"hello, world");
```

```sh
cargo install --path .
vzip create -9 out.zip src/ README.md
vzip list out.zip
vzip test out.zip
vzip extract -d /tmp/out out.zip
```

## Performance

These are rough numbers from `cargo run --release --example bench` on 10 MB
of generated text, against zlib 1.3 on the same machine:

| level | ratio | compress | decompress | zlib ratio | zlib compress | zlib decompress |
|---|---|---|---|---|---|---|
| 1 | 20.7% | 85 MB/s | 95 MB/s | 21.7% | 93 MB/s | 228 MB/s |
| 6 | 15.1% | 19 MB/s | 180 MB/s | 15.1% | 29 MB/s | 401 MB/s |
| 9 | 13.7% | 2.4 MB/s | 180 MB/s | 13.7% | 3.8 MB/s | 369 MB/s |

Compression ratios match zlib. Speed was traded for checkability in a few
places: Huffman codes are decoded bit by bit, like zlib's `puff.c`;
matches are compared and copied byte by byte; and the CRC uses one table
rather than eight. Each of these keeps the model-checking problems small.

## Limitations

- No ZIP64: archives and entries must be under 4 GiB, with fewer than
  65,535 entries. Larger archives are rejected, never misread.
- No encryption or multi-disk archives. Only methods 0 (stored) and 8
  (deflate) are supported.
- Whole buffers in memory; there is no streaming API yet.
- Raw DEFLATE only; no zlib or gzip wrappers.

## Layout

| Path | Contents |
|---|---|
| `src/bitio.rs` | LSB-first bit reader and writer |
| `src/crc32.rs` | CRC-32 |
| `src/tables.rs` | RFC 1951 constant tables |
| `src/huffman.rs` | Code-length generation, canonical codes, decoder |
| `src/lz77.rs` | Match finder, token replay |
| `src/deflate.rs` | Encoder |
| `src/inflate.rs` | Decoder |
| `src/zip.rs` | ZIP writer and reader |
| `src/e2e.rs` | End-to-end Kani harnesses |
| `.github/scripts/` | Per-harness Kani runner and report used by CI |
| `src/bin/vzip.rs` | Command-line tool |
| `tests/` | zlib, Info-ZIP and robustness tests |
| `tools/kani-sample/` | Runs harnesses as randomised tests where Kani isn't installed |

## License

MIT

[RFC 1951]: https://www.rfc-editor.org/rfc/rfc1951
[Kani]: https://github.com/model-checking/kani
