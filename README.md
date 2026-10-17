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

52 Kani harnesses sit next to the code they check. For every input up to
the stated sizes, they prove these properties:

- Bit-level I/O round-trips, and reads fail cleanly at end of input.
- CRC-32 (slicing-by-8) equals its bit-by-bit definition.
- The length and distance code tables are correct for **every** length
  (3–258) and distance (1–32768).
- Canonical Huffman codes are prefix-free.
- Generated code lengths always form a complete code within the length
  limit.
- Huffman encode-then-decode is the identity.
- LZ77 tokens are always valid and replay to exactly the input, including
  with tiny hash tables that force collisions.
- The encoder's header run-length coding is undone by the decoder's own
  routine.
- Stored and fixed-Huffman streams round-trip end to end.
- The decoder never panics on any input of the checked sizes, and respects
  its output limit.
- ZIP headers round-trip, the archive parser never panics, and name
  validation blocks path traversal.

The proofs are bounded, and dynamic-Huffman blocks are covered
compositionally rather than end to end. [VERIFICATION.md](VERIFICATION.md)
lists every harness, its bounds, and what is **not** proven.

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
| 1 | 20.7% | 81 MB/s | 134 MB/s | 21.7% | 93 MB/s | 228 MB/s |
| 6 | 15.1% | 18 MB/s | 245 MB/s | 15.1% | 29 MB/s | 401 MB/s |
| 9 | 13.7% | 2.4 MB/s | 367 MB/s | 13.7% | 3.8 MB/s | 369 MB/s |

Compression ratios match zlib. Speed was not the priority: the decoder
reads Huffman codes bit by bit, like zlib's `puff.c`, because that keeps
every index provably in bounds.

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
| `src/bin/vzip.rs` | Command-line tool |
| `tests/` | zlib, Info-ZIP and robustness tests |
| `tools/kani-sample/` | Runs harnesses as randomised tests where Kani isn't installed |

## License

MIT

[RFC 1951]: https://www.rfc-editor.org/rfc/rfc1951
[Kani]: https://github.com/model-checking/kani
