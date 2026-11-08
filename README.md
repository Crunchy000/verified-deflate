# verified-deflate

A DEFLATE ([RFC 1951]) compressor and decompressor with ZIP archive
support, in safe Rust with no dependencies. Its key properties are proven
with the [Kani] model checker, the most important for inputs of every
size with [Verus], and it is cross-checked against the reference zlib and
Info-ZIP.

- **Complete DEFLATE**: stored, fixed-Huffman and dynamic-Huffman blocks,
  hash-chain LZ77 with lazy matching, levels 0–9. Each block uses whichever
  of the three encodings is smallest, computed exactly.
- **Strict, panic-free decoder** that rejects malformed streams the way
  zlib does, with an output limit to stop decompression bombs. It also
  reads **Deflate64** (the 64 KiB-window variant that Windows and 7-Zip
  write as ZIP method 9).
- **zlib and gzip** (RFC 1950, RFC 1952): encoding, one-shot or through
  streaming `io::Write` encoders, and decoding, with every checksum
  checked and multi-member gzip files.
- **bzip2**: a compressor and a decompressor that accepts exactly what
  libbz2 accepts (fuzzed against it), with an output limit, multiple
  streams and the old "randomised" blocks; also as ZIP method 12.
- **LZMA, LZMA2 and xz**: `.xz` files (every check, the delta and all
  eight branch filters, concatenated streams), `.lzma` files and raw
  LZMA/LZMA2 streams, with a decoder that accepts exactly what liblzma
  accepts (fuzzed against it); also as ZIP methods 14 and 95.
- **tar archives**: a reader for ustar, pax and GNU archives (long names,
  base-256 numbers, links, devices), and a writer that falls back to pax
  headers for names, link targets and numbers that do not fit.
- **A drop-in replacement for the [`deflate`] crate**: its whole API, checked
  by running `deflate`'s own test suite against this crate.
- **ZIP archives**: writer and reader for stored, deflated, bzip2, LZMA and xz entries,
  and reader for Deflate64 ones, with UTF-8 names, Unix permissions,
  timestamps and data descriptors. Every entry is checked against its
  CRC-32.
- **`v7z`**, a command-line tool that takes 7-Zip's commands and switches
  (`a`, `x`, `e`, `l`, `t`, `d`; `-t`, `-mx`, `-o`, `-r`, `-y`, `-ao`,
  `-si`/`-so`, `-i`/`-x`) and exit codes, for zip, tar, gzip, bzip2, xz
  and lzma. A
  simpler tool, **`vzip`**, creates, extracts, lists and tests ZIP
  archives.
- `#![forbid(unsafe_code)]`, zero dependencies, MSRV 1.83.

## Verification

68 Kani harnesses sit next to the code they check and run in CI on every
push. For every input up to the stated sizes, they prove these
properties:

- Bit-level I/O round-trips, and reads fail cleanly at end of input.
- Adler-32 (zlib's checksum) equals its byte-by-byte definition, and its
  deferred reduction cannot overflow.
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
  hash function, and in the run-length and Huffman-only modes. The word-at-a-time match comparison equals a byte loop,
  and the match copy meets its specification.
- The encoder's code-length run-length coding is undone by the decoder's
  own routine.
- Stored streams round-trip end to end.
- ZIP headers round-trip. Every slice the archive parser takes is in
  bounds, for archives of any size. Name validation blocks path traversal.

Ten larger harnesses are gated behind the `kani-extended` feature
because they need more memory or time than a CI runner has: whole-stream
decoder no-panic (which Verus proves for every input, below), fixed-Huffman round trips, a whole-archive round trip,
single-query forms of the CRC slicing lemma (which CI proves by
linearity instead), and the Huffman table-decoder equivalence at larger
bounds. Dynamic-Huffman blocks are covered
compositionally rather than end to end.
[VERIFICATION.md](VERIFICATION.md) lists every harness, its bounds, the
modelling choices behind it, and what is **not** proven.

Five results are also proved for inputs of every size, not just up to a
bound, with the deductive verifier [Verus] (in `verus/`, run in CI on
every push):

- **The decoder** returns on every input of every length, for DEFLATE and
  Deflate64: it never panics, never loops forever, and never produces more
  than the caller's output limit.
- **The encoder** returns on every input, at every level and strategy,
  without panicking, and so does the streaming encoder behind the `write`
  encoders.
- **The Huffman code-length builder** always makes a complete code within
  the length limit, for any alphabet, including DEFLATE's 286 symbols.
- **The ZIP reader** returns on every byte string of every length without
  panicking, and so does reading any entry of the archive it returns: its
  header reads, slices and offset sums stay in bounds, on 32-bit targets
  too. CRC-32 never panics either.
- **ZIP name validation** accepts exactly the names that meet its rule,
  for names of any length.

CI checks that each verified copy is the library's code token for token,
and that 70 planted bugs each make verification fail.

The proofs are themselves checked. Kani's source coverage shows which code
each proof reaches. Mutation testing plants about 1,900 small bugs; the
proofs catch 67% of them and the tests a further 26%. Each of the rest
was checked by hand, and none is a missing check: they are heuristics,
fast paths with a proven fallback, or changes with no observable effect
(see VERIFICATION.md).

Two further checks run on top of the proofs. The test suite compares the
engine against real zlib in both directions: zlib decodes every stream
this crate produces, and this crate decodes about 1,400 zlib streams across
all levels, strategies and window sizes. Archives are validated with
`unzip -t` and Python's `zipfile`. For Deflate64, the tests decode the
streams of Python's `inflate64` encoder and the archives 7-Zip writes, and
on corrupted streams accept exactly what `inflate64`'s decoder accepts. Coverage-guided fuzzing (in `fuzz/`)
runs the same comparison on inputs libFuzzer evolves:
- the decoder against zlib on arbitrary bytes;
- the Deflate64 decoder against zlib's `infback9`, Mark Adler's Deflate64
  decoder;
- every encoder, one-shot and streaming, at any options, decoded by both;
- the ZIP reader on arbitrary bytes, and on archives the writer makes.

It runs for a minute per target on every change and for 20 minutes each
night. The test suite also runs on 32-bit x86 and on big-endian s390x.
Separately, `CompressOptions::verify` decompresses every stream before
returning it, so a wrong stream can never be returned.

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

### `v7z`: the 7-Zip command line

`v7z` follows 7-Zip's syntax, `v7z <command> [<switches>...] <archive>
[<names>...]`, so scripts written for `7z` work with zip, tar, gzip,
bzip2, xz and lzma archives:

```sh
v7z a -tzip -mx9 out.zip src/ README.md   # create, or update in place
v7z a out.tar src -snl                    # store symbolic links as links
v7z a out.tar.gz out.tar                  # gzip holds exactly one file
v7z a out.tar.bz2 out.tar                 # and so do bzip2,
v7z a -mx9 out.tar.xz out.tar             # xz and lzma
v7z a -mm=BZip2 out.zip src               # bzip2 inside a zip (or LZMA, XZ)
v7z l -slt out.zip                        # technical listing
v7z t out.zip                             # test every entry's CRC
v7z x out.zip -o/tmp/out -aos             # extract, keeping existing files
v7z e out.zip '*.txt' -r -y               # extract without paths
v7z d out.zip 'src/*.rs'                  # delete entries
tar cf - src | v7z a -si -tgzip src.tar.gz   # -si / -so pipes
v7z x -so out.tar.gz | tar tf -
```

| Command | | Switches |
|---|---|---|
| `a` | add (create or update) | `-t{zip,tar,gzip,bzip2,xz,lzma}`, `-mx[N]`, `-mm={Deflate,Copy,BZip2,LZMA,XZ}`, `-snl`, `-si[name]`, `-so` |
| `x`, `e` | extract with / without paths | `-o{dir}`, `-ao{a,s,u,t}`, `-y`, `-so`, `-si` |
| `l`, `t` | list, test | `-slt` |
| `d` | delete entries | |
| `i` | show formats | |

Names take wildcards (`*`, `?`), match from the archive's root, and with
`-r` at any depth (`-r0`: wildcards only); `-i!`/`-x!` (and `@listfile`)
add and remove names. An archive's type comes from `-t`, or else from its
content when reading and its extension (`.zip`, `.tar`, `.gz`, `.tgz`,
`.bz2`, `.tbz2`, `.xz`, `.txz`, `.lzma`)
when creating. Exit codes are 7-Zip's: 0 success, 1 warning, 2 fatal error
(including any CRC or data error), 7 command-line error, 255 stopped.
Switches that only affect 7-Zip's console or threads (`-bb`, `-bso`,
`-scs`, `-mmt` and similar) are accepted and ignored. Listings carry the
same fields as 7-Zip's, but are not byte-identical.

Extraction never writes outside the output directory: absolute names are
made relative, names with `..` are refused, nothing is written through a
symbolic link, links whose target leaves the directory are not made, and
links are created after every file.

## Replacing the `deflate` crate

This crate provides the whole API of [`deflate`] 1.0 at its root:
`deflate_bytes`, `deflate_bytes_conf`, their zlib and gzip variants,
`Compression`, `CompressionOptions` (with `fast`, `high`, `rle` and
`huffman_only`), `MatchingType`, `SpecialOptions`, and the streaming
encoders `write::{DeflateEncoder, ZlibEncoder, GzEncoder}`. To switch,
rename the dependency; code written for `deflate` then compiles unchanged:

```toml
[dependencies]
deflate = { package = "verified-deflate", git = "https://github.com/Crunchy000/verified-deflate", features = ["gzip"] }
```

The one source change is for gzip headers. `deflate`'s gzip functions take
a `GzBuilder` from the separate `gzip-header` crate; here the same builder
is `deflate::gzip_header::GzBuilder`, so `use gzip_header::GzBuilder;`
becomes `use deflate::gzip_header::GzBuilder;`. It builds headers byte for
byte as `gzip-header` does.

```rust
use std::io::Write;
use verified_deflate::write::ZlibEncoder;
use verified_deflate::{deflate_bytes_zlib, Compression};

let data = b"This is some test data";
let once = deflate_bytes_zlib(data);

let mut encoder = ZlibEncoder::new(Vec::new(), Compression::Default);
encoder.write_all(data).unwrap();
let streamed = encoder.finish().unwrap();
```

CI runs `deflate` 1.0.0's own test suite (30 tests, fetched from its
repository) against this crate imported as `deflate`; see
`tools/deflate-upstream`. Python's zlib and gzip modules also decode
every option preset in every format, including streams cut at each sync
flush.

Differences from `deflate`:

- The compressed bytes differ, as they do between any two encoders. They
  are the same format and decode to the same data. On the benchmark inputs
  they are at most 0.3% larger than `deflate`'s, and up to 5% smaller.
- `SpecialOptions::_ForceFixed` and `_ForceStored`, which `deflate` left
  unimplemented, force fixed-Huffman and stored blocks.
- A gzip extra field longer than 65535 bytes panics, where `gzip-header`
  writes a corrupt header.

As in `deflate`, `flush` is a zlib sync flush, and dropping an encoder
finishes the stream. Writers compress in 256 KiB chunks, each matching
up to 32 KiB into earlier input, so memory stays bounded.

Against `deflate` 1.0.0 itself (MB/s, 4 MB of each input, same machine):

| preset | text | `deflate` | records | `deflate` | random | `deflate` |
|---|---|---|---|---|---|---|
| `fast` | 140 | 141 | 159 | 149 | 64 | 82 |
| `default` | 14.2 | 16.1 | 35.4 | 25.7 | 46 | 43 |
| `high` | 3.3 | 4.1 | 7.3 | 6.6 | 45 | 38 |
| `rle` | 190 | 197 | 147 | 182 | 257 | 368 |
| `huffman_only` | 247 | 111 | 181 | 106 | 337 | 143 |

The native `compress` levels are faster than these presets at similar
ratios (level 6 compresses the text at about 17.5 MB/s), because the presets
follow `deflate`'s search limits exactly.

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

bzip2 at level 9, against the bzip2 1.0.8 program on the same machine
(MB/s, one thread): on 1.9 MB of source code and prose, compression 8.9
(bzip2: 10.9) and decompression 39 (26), to within 0.02% of bzip2's
size; on 5 MB of random bytes, 5.6 (7.0) and 21 (12). The block sort is
SA-IS, so no input is slow to compress.

xz against XZ Utils 5.4.5 (`xz -T1`), same files: on the 1.9 MB of text,
levels 1 / 6 / 9 give 158,548 / 150,168 / 148,176 bytes (xz: 158,352 /
138,020 / 138,020) at 28 / 19 / 11 MB/s (xz: 37 / 3.9 / 3.9), and
decompression runs at 155–181 MB/s (xz: 172–195). Random data is stored
in LZMA2's uncompressed chunks by both.

## Limitations

- No ZIP64: archives and entries must be under 4 GiB, with fewer than
  65,535 entries. Larger archives are rejected, never misread.
- No encryption or multi-disk archives. Methods 0 (stored), 8
  (deflate), 12 (bzip2), 14 (LZMA) and 95 (xz) are read and written,
  and method 9 (Deflate64) is read; no others.
- Decompression works on whole buffers; streaming is for compression only.
- Deflate64 is read, not written. zlib preset dictionaries are not
  supported.
- tar: GNU sparse files and multi-volume archives are listed but their
  data is not reassembled.
- LZMA and xz compression use liblzma's fast parser (as `xz -0` to `-3`
  do) at every level, with a deeper match search at higher levels. At
  `-6` and `-9` the output is about 9% larger than xz's on text, which
  uses an optimal parser there, and is made about five times faster. The
  RISC-V branch filter is decoded but not encoded.
- `v7z`: no `.7z` format yet,
  no encryption (`-p`), and no `u`, `rn` or `h` commands. Without `-y` or
  `-ao`, an existing file prompts on the terminal; with no terminal input
  that stops extraction (exit 255).

## Layout

| Path | Contents |
|---|---|
| `src/adler32.rs` | Adler-32 |
| `src/bitio.rs` | LSB-first bit reader and writer |
| `src/crc32.rs` | CRC-32 |
| `src/tables.rs` | RFC 1951 constant tables |
| `src/huffman.rs` | Code-length generation, canonical codes, decoder |
| `src/lz77.rs` | Match finder, token replay |
| `src/deflate.rs` | Encoder |
| `src/inflate.rs` | Decoder |
| `src/zip.rs` | ZIP writer and reader |
| `src/gzip.rs`, `src/zlib.rs` | gzip and zlib readers |
| `src/tar.rs` | tar reader and writer |
| `src/bzip2.rs`, `src/bwt.rs` | bzip2 compressor and decompressor; the block sort (SA-IS) |
| `src/lzma.rs`, `src/lzma2.rs`, `src/xz.rs` | LZMA (and `.lzma` files), LZMA2, and `.xz` files with CRC64 and SHA-256 |
| `src/filters.rs` | Branch (BCJ) and delta filters |
| `src/compat.rs`, `src/write.rs`, `src/gzip_header.rs` | The `deflate` crate's API: one-shot functions, streaming encoders, gzip headers |
| `src/e2e.rs` | End-to-end Kani harnesses |
| `verus/` | Verus proofs, over copies of library functions |
| `.github/scripts/` | Per-harness Kani runner, coverage and mutation reports, Verus copy and mutant checks, used by CI |
| `src/bin/v7z/` | 7-Zip-compatible command-line tool: argument parsing, archive access, name matching, commands |
| `src/bin/vzip.rs` | Simple ZIP command-line tool |
| `tests/` | zlib, gzip, tar, bzip2, xz, Deflate64, Info-ZIP, 7-Zip, CLI and robustness tests |
| `fuzz/` | Differential fuzz targets against zlib and its Deflate64 decoder (cargo-fuzz), with their own dependencies |
| `tools/deflate-upstream/` | Runs the `deflate` crate's own tests against this crate |
| `tools/kani-sample/` | Runs harnesses as randomised tests where Kani isn't installed |

## License

MIT

[RFC 1951]: https://www.rfc-editor.org/rfc/rfc1951
[Kani]: https://github.com/model-checking/kani
[Verus]: https://github.com/verus-lang/verus
[`deflate`]: https://docs.rs/deflate/1.0.0
