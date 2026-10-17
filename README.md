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

54 Kani harnesses sit next to the code they check and run in CI on every
push. For every input up to the stated sizes, they prove these
properties:

- Bit-level I/O round-trips, and reads fail cleanly at end of input.
- CRC-32 is correct for inputs of any length: its eight-byte
  (slicing-by-8) step equals eight table steps, and the table step equals
  eight bitwise steps, for every register value and input.
- The length and distance code tables are correct for **every** length
  (3–258) and distance (1–32768).
- Canonical Huffman codes are prefix-free. Generated code lengths always
  form a complete code within the length limit. Huffman encode-then-decode
  is the identity, decoding arbitrary bits never panics, and the
  table-driven decoder returns exactly what the bit-by-bit decoder does.
- LZ77 tokens are always valid and describe exactly the input, for every
  hash function. The word-at-a-time match comparison equals a byte loop,
  and the match copy meets its specification.
- The encoder's code-length run-length coding is undone by the decoder's
  own routine.
- Stored streams round-trip end to end.
- ZIP headers round-trip. Every slice the archive parser takes is in
  bounds, for archives of any size. Name validation blocks path traversal.

Ten larger harnesses are gated behind the `kani-extended` feature
because they need more memory or time than a CI runner has: whole-stream
decoder no-panic, fixed-Huffman round trips, a whole-archive round trip,
single-query forms of the CRC slicing lemma (which CI proves by
linearity instead), and the Huffman table-decoder equivalence at larger
bounds. Dynamic-Huffman blocks are covered
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

Throughput in MB/s from `cargo run --release --example bench` (best of
three runs), against zlib 1.3 on the same inputs and machine, a 2-core
2.8 GHz Xeon VM. The inputs are 8 MB each of generated text, this crate's
own source code and binary log records, and 4 MB of random bytes.
Compressed sizes are within 0.2 percentage points of zlib's at every level.

| input | level | compress | zlib | decompress | zlib |
|---|---|---|---|---|---|
| text | 1 | 105 | 111 | 221 | 253 |
| text | 6 | 17.5 | 26.0 | 402 | 402 |
| text | 9 | 2.7 | 3.6 | 573 | 434 |
| source | 1 | 92 | 85 | 238 | 253 |
| source | 6 | 25.3 | 27.4 | 289 | 284 |
| source | 9 | 6.5 | 9.1 | 260 | 281 |
| records | 1 | 139 | 111 | 367 | 430 |
| records | 6 | 33.6 | 37.8 | 410 | 453 |
| random | 6 | 38.5 | 33.7 | 2675 | 1967 |

CRC-32 runs at about 1.7 GB/s (zlib: 3.0 GB/s).

The fast paths are verified like the rest of the code. Each is either
proved directly or proved equal to a simple reference version, which the
larger proofs then use in its place:

- Huffman codes decode through a 10-bit lookup table, proved equal to the
  bit-by-bit canonical decoder (zlib's `puff.c` method).
- Match candidates are compared eight bytes at a time, proved equal to a
  byte loop. Short match copies are word moves, proved to meet the LZ77
  copy specification.
- CRC-32 uses slicing-by-8, proved equal to the one-table method, which is
  proved equal to the bitwise definition.
- The Huffman length builder sorts with a heapsort, proved to sort.

The main remaining gap is the hash-chain walk at levels 6–9: about 17
instructions per candidate against zlib's 10, mostly bounds checks and a
distance check on every chain entry, which makes it impossible for a
corrupted chain to produce an invalid match.

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
