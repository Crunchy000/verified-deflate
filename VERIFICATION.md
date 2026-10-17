# Verification

This crate's correctness argument has four layers, from strongest to
weakest:

1. **Kani proofs** — bounded model checking of the real code, for *every*
   input up to the stated size. Each harness is a `#[kani::proof]` function
   next to the code it checks.
2. **Safe Rust** — `#![forbid(unsafe_code)]`, so memory safety comes from
   the compiler. Kani adds the absence of panics (out-of-bounds indexing,
   arithmetic overflow, failed `unwrap`, division by zero) inside its bounds.
3. **Differential testing** against the reference zlib (via Python's `zlib`
   module), Info-ZIP `zip`/`unzip` and Python's `zipfile`, plus large
   randomised round-trip and corruption tests.
4. **Run-time checks** — CRC-32 on every ZIP entry, an output limit on every
   decompression, and the optional `verify` mode, which decompresses each
   compressed stream and falls back to stored blocks if it does not
   reproduce the input. With `verify` on, a wrong compressed stream cannot be
   returned even outside what the proofs cover.

## Running the proofs

```sh
cargo install --locked kani-verifier --version 0.68.0
cargo kani setup
cargo kani                                   # every harness
cargo kani --harness huffman::proofs::       # one module
```

CI runs each module as a separate job (`.github/workflows/ci.yml`) and
reports per-harness results in the job summary.

Without Kani, the harnesses can still be compiled and *sampled* with random
inputs (this is a smoke test, not a proof; see `tools/kani-sample`):

```sh
RUSTFLAGS="--cfg kani --cfg kani_sample" cargo test --lib --release -- proofs e2e
```

## What is proven

Bounds are "for all inputs" up to the size given. Loop unwinding bounds are
checked by Kani's unwinding assertions, so a bound that is too small fails
the proof instead of silently skipping behaviour.

### Bit I/O (`bitio`)

| Harness | Property |
|---|---|
| `write_then_read_roundtrip` | Any three writes of 0–32 bits read back exactly; bit count exact; output is the minimum number of bytes; padding is zero. |
| `reader_eof_is_exact` | Over 0–4 bytes of input, three reads of 0–32 bits each succeed exactly when enough bits remain, and otherwise return `UnexpectedEof`. Never panics. |
| `read_bits_matches_read_bit` | `read_bits(n)` equals `n` single-bit reads, at any bit offset 0–7, for n ≤ 16. |
| `aligned_bytes_roundtrip` | Bits, padding, byte-aligned payload, more bits: all round-trip; `bytes_consumed` is exact; over-long reads are errors. |
| `reverse_bits_is_reversal` | For every code of 1–15 bits, `reverse_bits` reverses exactly those bits and is an involution. |

### CRC-32 (`crc32`)

| Harness | Property |
|---|---|
| `base_table_matches_definition` | All 256 base table entries equal the bitwise definition. |
| `slice_tables_match_definition` | All 7 × 256 slicing-by-8 entries mean "byte followed by k zero bytes". |
| `slicing_step_matches_reference` | For any initial CRC and any 8 bytes, the table-driven step equals the bit-at-a-time reference. |
| `tail_matches_reference` | Same for 0–7 trailing bytes. |
| `streaming_is_concatenation` | Hashing in two pieces, split anywhere, equals hashing the whole. |

### DEFLATE tables (`tables`)

| Harness | Property |
|---|---|
| `length_code_covers_every_length` | Every length 3–258 maps to a code whose base and extra bits cover it; 258 uses code 285. |
| `length_decode_is_inverse` | Every (code, extra bits) pair decodes to a valid length that encodes back to the same code (except RFC-invalid 284+31, read as 258 like zlib). |
| `distance_code_covers_every_distance` | Every distance 1–32768 maps to a covering code. |
| `distance_decode_is_inverse` | Every (code, extra bits) pair decodes to a valid distance that encodes back to the same code. |
| `fixed_lengths_match_rfc` | The fixed literal/length code lengths match RFC 1951 §3.2.6 for all 288 symbols. |

### Huffman codes (`huffman`)

| Harness | Property |
|---|---|
| `canonical_codes_are_prefix_free` | For any lengths (0–15) over 6 symbols: codes are assigned iff the Kraft sum is ≤ 1, each fits its length, and no code is a prefix of another. |
| `build_lengths_complete_and_limited` | For any frequencies (full `u32` range) over 5 symbols and any limit 3–15: the lengths form a complete code (Kraft sum exactly 1), respect the limit, and give every used symbol a code. A limit of 3 forces the frequency-halving path. |
| `build_lengths_tight_limit` | Same for 4 symbols with limit 2, which reaches the flatten-to-equal-weights fallback. |
| `build_lengths_rejects_impossible_limit` | Five used symbols with a 2-bit limit are refused, not mis-coded. |
| `decoder_new_never_panics` | For any lengths (0–255) over 8 symbols the decoder builder never panics, accepts exactly the lengths with Kraft sum ≤ 1, and reports completeness exactly. |
| `encode_then_decode_symbol` | For any valid lengths over 6 symbols and any coded symbol: writing its canonical code as the encoder does and decoding returns the symbol and consumes exactly its length. |
| `decode_arbitrary_bits` | Decoding arbitrary bits never panics and only yields symbols that have codes. |

### LZ77 (`lz77`)

| Harness | Property |
|---|---|
| `tokens_valid_and_replay_small_tables` | For any input of 0–8 bytes, with a 4-entry hash table and an 8-byte window (so collisions, chain reuse and eviction all occur) and lazy matching: every match has length 3–258 and a distance within the window and the data produced so far, and replaying the tokens yields the input exactly. |
| `tokens_valid_and_replay_level9` | Same for any 7 bytes with the production level-9 configuration. |
| `tokens_valid_and_replay_greedy` | Same for any 7 bytes with the greedy level-1 configuration. |
| `runs_are_matched` | A run of 10 equal bytes becomes one literal and one overlapping match. |
| `copy_match_equals_naive` | The doubling overlap copy equals a byte-at-a-time copy for distances 1–4 and lengths 0–12. |
| `replay_arbitrary_tokens` | Replaying arbitrary tokens never panics; invalid matches are errors. |

### Encoder (`deflate`)

| Harness | Property |
|---|---|
| `code_length_rle_roundtrip` | The run-length coding of any 8 code lengths, expanded by the *decoder's own* routine, gives back the same lengths. |
| `code_length_rle_zero_runs` | Same for 16 lengths drawn from {0, v}, reaching the long-zero-run code 18. |
| `fixed_code_table_matches_rfc` | The compile-time fixed code tables hold the RFC's canonical codes. |
| `stored_cost_is_exact` | The stored-block size model equals the bits actually written. |
| `fixed_cost_is_exact` | The fixed-block size model equals the bits written, for any three tokens. |

### Decoder (`inflate`)

| Harness | Property |
|---|---|
| `never_panics_4_bytes` | No 4-byte input can make the decoder panic, with any output limit 0–16; results respect the limit. Reaches all block types, dynamic headers up to the code-length code, and matches. |
| `never_panics_fixed_6_bytes` | No 6-byte single fixed-Huffman block can make the decoder panic (reaches 258-byte overlapping matches). |
| `stored_block_semantics` | A stored block decodes to exactly its payload; any LEN/NLEN mismatch is rejected. |
| `expand_code_length_is_safe` | Code-length expansion never panics or writes out of bounds, for any symbol, extra bits and position. |

### ZIP (`zip`)

| Harness | Property |
|---|---|
| `local_header_roundtrip`, `central_header_roundtrip`, `eocd_roundtrip` | For all field values, headers serialize to their fixed size and parse back unchanged. |
| `header_parsers_never_panic` | Header parsers never panic on 0–46 arbitrary bytes. |
| `find_eocd_is_sound` | Locating the end record in any buffer of 0–30 bytes never panics; a found record is a real signature whose comment ends exactly at the end of the data. |
| `archive_parse_never_panics` | With an end record at the end of a 70-byte buffer and every other byte arbitrary (directory offset, size, count, the central header and name), parsing never panics and entries lie inside the directory. |
| `dos_datetime_roundtrip` | Every valid DOS timestamp survives encoding and decoding. |
| `civil_date_conversion_roundtrip` | Every day from 1980-01-01 to 2107-12-31 converts to a valid date and back. |
| `unix_time_always_valid` | Every `i64` Unix time maps to a valid DOS timestamp. |
| `validate_name_rejects_traversal` | For every 6-byte UTF-8 name, accepted names have no leading `/`, no `..` component and no backslash. |

### End to end (`e2e`)

| Harness | Property |
|---|---|
| `roundtrip_stored` | Any input of 0–8 bytes, stored: decompress(compress(x)) = x, consuming the whole stream. |
| `roundtrip_fixed_lazy` | Any input of 0–5 bytes, fixed Huffman, level-6 lazy matcher. |
| `roundtrip_fixed_greedy` | Any 6 bytes, fixed Huffman, level-1 greedy matcher. |
| `roundtrip_fixed_two_symbol_alphabet` | Any 8 bytes over any two-symbol alphabet (match-heavy), level 9. |
| `zip_roundtrip_stored` | A one-entry archive of any 0–4 bytes reads back exactly with its CRC check passing. |

## What is not proven

Be precise about the gaps:

- **Everything is bounded.** The proofs cover all inputs *up to the sizes
  above*. The code is written over slices of any length, so the same code
  paths run on large inputs, but nothing here proves the properties for
  larger sizes. Large inputs are covered by the tests, not by proofs.
- **Dynamic-Huffman blocks are not verified end to end.** Building and
  writing a 286-symbol code needs more loop unrolling than a model checker
  can handle. They are covered compositionally (code-length generation,
  canonical encode/decode, the run-length-coded header through the
  decoder's own expansion routine, bit I/O, the length/distance tables), by
  zlib interop tests, and at run time by `verify`.
- **Huffman construction is proven on alphabets of 4–6 symbols**, not on
  DEFLATE's 19-, 30- and 286-symbol alphabets. That the length-limiting
  fallback always succeeds for those sizes rests on a documented argument
  (equal weights give lengths within one of log₂ n), checked by tests.
- **The decoder's no-panic proofs cover 4- and 6-byte inputs**; the
  corruption tests extend this by sampling to large inputs.
- **Compression ratio and speed** are not correctness properties and are
  not verified.
- **The `vzip` command-line tool** is not verified (it is a thin layer over
  the library; extraction refuses names that fail `validate_name`).
- **The trusted base** includes the Rust compiler and standard library,
  Kani and its CBMC back end, and the harnesses themselves: a harness that
  asserts the wrong thing proves the wrong thing. Review them as you would
  a specification.
