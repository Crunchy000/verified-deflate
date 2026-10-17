# Verification

This crate's correctness argument has four layers, from strongest to
weakest:

1. **Kani proofs**: bounded model checking of the real code, for *every*
   input up to the stated size. Each harness is a `#[kani::proof]` function
   next to the code it checks. 46 harnesses run in CI on every push; 7
   more are too large for CI runners and are gated behind the
   `kani-extended` feature (see below).
2. **Safe Rust**: `#![forbid(unsafe_code)]`, so memory safety comes from
   the compiler. Kani adds the absence of panics inside its bounds:
   out-of-bounds indexing, arithmetic overflow, failed `unwrap`, division
   by zero.
3. **Differential testing**: the engine is checked against the reference
   zlib (through Python's `zlib` module), Info-ZIP `zip`/`unzip` and
   Python's `zipfile`. Large randomised round-trip and corruption tests
   run on top.
4. **Run-time checks**: CRC-32 on every ZIP entry, an output limit on
   every decompression, and the optional `verify` mode. `verify`
   decompresses each compressed stream and falls back to stored blocks if
   the stream does not reproduce the input. With `verify` on, a wrong
   compressed stream cannot be returned, even outside what the proofs
   cover.

## Running the proofs

```sh
cargo install --locked kani-verifier --version 0.68.0
cargo kani setup
cargo kani                                         # every CI harness
cargo kani --harness huffman::proofs::             # one module
cargo kani -Z stubbing --harness lz77::proofs::    # modules that use stubs
cargo kani -Z stubbing --features kani-extended    # include the large ones
```

CI (`.github/workflows/ci.yml`) runs each module as its own job, and each
harness as its own process. A harness gets 16 GiB of memory and 30
minutes, and the job summary reports per-harness results. On GitHub's
standard runners the whole CI suite verifies in about 45 minutes of wall
time across parallel jobs. Most harnesses take seconds; the slowest, the
Huffman length builder and encode/decode round trip, take about 8 minutes
and peak at about 12 GiB.

Without Kani, the harnesses can still be compiled and *sampled* with
random inputs. This is a smoke test, not a proof; see `tools/kani-sample`.

```sh
RUSTFLAGS="--cfg kani --cfg kani_sample" cargo test --lib --release -- proofs e2e
```

## Techniques, and what they mean for the claims

Loop bounds are checked by Kani's unwinding assertions. A bound that is
too small fails the proof; it never silently skips behaviour. To keep the
problems small enough to solve, the harnesses use these techniques. Each
one either preserves the claim or is stated in the claim.

- **Fixed input lengths.** Several harnesses take "any *N* bytes" rather
  than "any length up to *N*". A symbolic length makes every buffer size
  symbolic, which costs the model checker far more.
- **Small match-finder tables.** The LZ77 harnesses use each level's search
  parameters with an 8-byte window and 4 hash buckets. The tokenizer's
  logic does not depend on table size for inputs shorter than the window.
- **Hash stub.** `Matcher::hash` is replaced by a function that returns an
  arbitrary bucket on every call. The tokenizer checks every candidate
  byte by byte, so its correctness cannot depend on the hash. A proof
  under the stub therefore holds for *every* hash function, which is
  stronger than a proof for the real one.
- **Growth stub.** Decoder harnesses whose output fits in the initial
  buffer replace `Output::grow` with a panic, so Kani must also prove
  that growth is unreachable.
- **Exact-capacity instantiations.** The Huffman length builder is generic
  over its scratch capacity. The proofs run it with capacity equal to the
  alphabet size; `build_lengths` runs the same code with capacity 32 or
  288.
- **Memory-safety instrumentation off for some modules.** The `lz77`,
  `deflate`, `inflate` and `e2e` modules run with
  `--no-memory-safety-checks`. This only drops CBMC's pointer checks inside
  the standard library's unsafe internals. Rust-level panics (bounds,
  overflow, `unwrap`) are still checked, and this crate contains no unsafe
  code.

Several implementation choices were made to keep the code checkable. The
tokenizer and the code-length run-length coder take one step per input
position and write into caller-provided slices. The decoder writes its
output by index and rebuilds Huffman decoders in place. Huffman decoding
steps are written out rather than looped. None of these changes behaviour;
the zlib interop tests confirm the output is unchanged.

## What is proven (in CI)

### Bit I/O (`bitio`)

| Harness | Property |
|---|---|
| `write_then_read_roundtrip` | Any three writes of 0–32 bits read back exactly; the bit count is exact; the output is the minimum number of bytes; padding is zero. |
| `reader_eof_is_exact` | Over 0–4 bytes of input, three reads of 0–32 bits each succeed exactly when enough bits remain, and otherwise return `UnexpectedEof`. |
| `read_bits_matches_read_bit` | `read_bits(n)` equals `n` single-bit reads, at any bit offset 0–7, for n ≤ 16. |
| `aligned_bytes_roundtrip` | Bits, padding, a byte-aligned payload, then more bits: all round-trip; `bytes_consumed` is exact; over-long reads are errors. |
| `reverse_bits_is_reversal` | For every code of 1–15 bits, `reverse_bits` reverses exactly those bits and is an involution. |

### CRC-32 (`crc32`)

| Harness | Property |
|---|---|
| `table_matches_definition` | All 256 table entries equal the bitwise definition. |
| `table_step_equals_bitwise_step` | For **every** 32-bit register and **every** byte, the table step equals eight bitwise steps. `update` and the reference `update_bitwise` are the same fold over these steps, so they agree on inputs of every length. |
| `update_matches_reference` | Whole computations agree directly for 0–4 bytes and any starting value. |
| `streaming_is_concatenation` | Hashing in two pieces, split anywhere, equals hashing the whole. |

### DEFLATE tables (`tables`)

| Harness | Property |
|---|---|
| `length_code_covers_every_length` | Every length 3–258 maps to a code whose base and extra bits cover it; 258 uses code 285. |
| `length_decode_is_inverse` | Every (code, extra bits) pair decodes to a valid length that encodes back to the same code. The one exception is RFC-invalid 284+31, which is read as 258, as zlib does. |
| `distance_code_covers_every_distance` | Every distance 1–32768 maps to a covering code. |
| `distance_decode_is_inverse` | Every (code, extra bits) pair decodes to a valid distance that encodes back to the same code. |
| `fixed_lengths_match_rfc` | The fixed literal/length code lengths match RFC 1951 §3.2.6 for all 288 symbols. |

### Huffman codes (`huffman`)

| Harness | Property |
|---|---|
| `canonical_codes_are_prefix_free` | For any lengths (0–15) over 6 symbols, codes are assigned iff the Kraft sum is ≤ 1. Each code fits its length, and no code is a prefix of another. |
| `build_lengths_complete_and_limited` | For any frequencies 0–255 over 5 symbols and any limit 3–15, the lengths form a complete code (Kraft sum exactly 1), respect the limit, and give every used symbol a code. A limit of 3 forces the clamp-and-repair path. |
| `build_lengths_full_range_frequencies` | Same for any `u32` frequencies over 3 symbols and any limit 2–15. |
| `build_lengths_tight_limit` | Same for 4 symbols in 2 bits, where every skewed distribution needs repair. |
| `build_lengths_rejects_impossible_limit` | Five used symbols with a 2-bit limit are refused rather than mis-coded. |
| `decoder_new_never_panics` | For any lengths (0–255) over 8 symbols, the decoder builder never panics. It accepts exactly the lengths whose Kraft sum is ≤ 1 and reports completeness exactly. |
| `encode_then_decode_symbol` | For any valid lengths over 6 symbols and any coded symbol, packing its canonical code as the encoder does and decoding returns that symbol. Exactly its length is consumed. |
| `decode_arbitrary_bits` | Decoding arbitrary bits with any accepted lengths never panics, and only yields symbols that have codes. |

### LZ77 (`lz77`)

| Harness | Property |
|---|---|
| `tokens_valid_and_replay_small_tables` | For any 4 bytes, with a 2-entry chain limit, lazy matching and every hash function: every match has length 3–258 and a distance within the data produced so far, and the tokens describe the input exactly. |
| `tokens_valid_and_replay_level9` | Same with level 9's search parameters. |
| `tokens_valid_and_replay_greedy` | Same with level 1's greedy parameters. |
| `runs_are_matched` | Six equal bytes become one literal and one overlapping match. |
| `copy_within_buffer_meets_spec` | For every position, distance and length that fit a 12-byte buffer, each written byte equals the byte `dist` earlier and nothing else changes. This includes heavily overlapping copies. |
| `replay_arbitrary_tokens` | Replaying arbitrary tokens never panics; invalid matches are errors. |

### Encoder (`deflate`)

| Harness | Property |
|---|---|
| `code_length_rle_roundtrip` | The run-length coding of any 8 code lengths is undone exactly by the *decoder's own* expansion routine. |
| `code_length_rle_zero_runs` | Same for 12 lengths drawn from {0, v}, reaching the long-zero-run code 18. |
| `fixed_code_table_matches_rfc` | The compile-time fixed code tables hold the RFC's canonical codes. |
| `stored_cost_is_exact` | The stored-block size model equals the bits written, from any bit offset, for 0, 3 and 8 bytes of any content. |

### Decoder (`inflate`)

| Harness | Property |
|---|---|
| `stored_block_semantics` | A 4-byte stored block decodes to exactly its payload when NLEN complements LEN; any other NLEN is rejected. |
| `expand_code_length_is_safe` | Code-length expansion never panics or writes out of bounds, for any symbol, extra bits and position. |

### ZIP (`zip`)

| Harness | Property |
|---|---|
| `local_header_roundtrip`, `central_header_roundtrip`, `eocd_roundtrip` | For all field values, headers serialize to their fixed size and parse back unchanged. |
| `header_parsers_never_panic` | Header parsers never panic on 0–46 arbitrary bytes. |
| `find_eocd_is_sound` | Locating the end record in any 0–30 bytes never panics. A found record is a real signature whose comment ends exactly at the end of the data. |
| `directory_bounds_sound` | For **all** positions and field values, the central directory range lies before the end record. |
| `record_bounds_sound` | For **all** positions and field values, an accepted central record lies inside the directory and moves the parser forward. Together with the two harnesses above, this shows every slice the archive parser takes is in bounds, for archives of any size. |
| `dos_datetime_roundtrip` | Every valid DOS timestamp survives encoding and decoding. |
| `civil_date_conversion_roundtrip` | Every day from 1980-01-01 to 2107-12-31 converts to a valid date and back. |
| `unix_time_always_valid` | Every `i64` Unix time maps to a valid DOS timestamp. |
| `validate_name_rejects_traversal` | For every byte string of 0–8 bytes, an accepted name has no leading `/`, no `..` component, no backslash and no NUL. The spec is stated independently of the implementation. |

### End to end (`e2e`)

| Harness | Property |
|---|---|
| `roundtrip_stored` | For any 4 bytes, and for the empty input, the stored encoder followed by the decoder gives back the input exactly and consumes the whole stream. |

## Extended harnesses (`--features kani-extended`)

These exist and run under the sampling shim, but each needs more memory
or time than a CI runner allows (16 GiB, 30 minutes). They are the natural next targets for a larger
machine or further restructuring of the decoder.

| Harness | Property |
|---|---|
| `inflate::proofs::never_panics_2_bytes` | No 2-byte input can make the decoder panic, for any output limit 0–16. |
| `inflate::proofs::dynamic_header_never_panics` | Reading a dynamic block header from any 4 bytes never panics. |
| `inflate::proofs::never_panics_fixed_5_bytes` | No 5-byte fixed-Huffman block can make the decoder panic. |
| `e2e::roundtrip_fixed_lazy` | Any 4 bytes round-trip through fixed Huffman with the level-6 lazy matcher. |
| `e2e::roundtrip_fixed_greedy` | Any 3 bytes round-trip through fixed Huffman with the greedy matcher. |
| `e2e::roundtrip_fixed_two_symbol_alphabet` | Any 6 bytes over a two-symbol alphabet round-trip with level 9. |
| `e2e::zip_roundtrip_stored` | A one-entry archive of any 3 bytes, stored, reads back exactly and passes its CRC check. |

## What is not proven

The gaps, stated precisely:

- **Everything is bounded.** The proofs cover all inputs *up to the sizes
  above*. The exceptions are a few harnesses that hold for all values:
  CRC steps, the code tables, and the ZIP bounds lemmas. The code is
  written over slices of any length, so the same code paths run on large
  inputs. Large inputs are covered by tests, not proofs.
- **The decoder on arbitrary streams is proven piecewise in CI.** The
  pieces are Huffman decoding on arbitrary bits, code construction from
  arbitrary lengths, code-length expansion, the LZ77 copy, stored blocks,
  bit reading and the tables. The whole-stream "never panics" harnesses
  are in the extended set. On top of the pieces, about 140,000 corrupted
  and random streams are fuzzed in the test suite.
- **Compressor-to-decompressor round trips are proven in CI for stored
  blocks.** Fixed-Huffman and whole-archive round trips are in the
  extended set. In CI they
  are covered compositionally: tokenizer validity, code tables, Huffman
  encode and decode, bit I/O. Dynamic-Huffman blocks need more unrolling
  than a model checker handles at DEFLATE's alphabet sizes. They are covered
  by the length builder and run-length proofs on small alphabets, zlib
  interop, and `verify` at run time.
- **Huffman construction is proven on alphabets of 3–6 symbols**, not on
  DEFLATE's 19, 30 and 286. The length-limit repair's argument is
  documented in `build_lengths` and exercised by tests on full-size
  alphabets.
- **Compression ratio and speed** are not correctness properties and are
  not verified.
- **The `vzip` command-line tool** is not verified. It is a thin layer over
  the library, and extraction refuses names that fail `validate_name`.
- **The trusted base** includes the Rust compiler and standard library,
  Kani and its CBMC back end, and the harnesses themselves. A harness that
  asserts the wrong thing proves the wrong thing, so review them as you
  would a specification.
