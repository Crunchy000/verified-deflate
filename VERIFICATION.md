# Verification

This crate's correctness argument has four layers, from strongest to
weakest:

1. **Kani proofs**: bounded model checking of the real code, for *every*
   input up to the stated size. Each harness is a `#[kani::proof]` function
   next to the code it checks. 66 harnesses run in CI on every push; 11
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
cargo kani -Z stubbing                             # every CI harness
cargo kani --harness crc32::proofs::               # one module
cargo kani -Z stubbing --harness lz77::proofs::    # modules that use stubs
cargo kani -Z stubbing --features kani-extended    # include the large ones
```

CI (`.github/workflows/ci.yml`) runs each module as its own job (the
slow Huffman module as three), and each harness as its own process. A harness gets 16 GiB of memory and 30
minutes, and the job summary reports per-harness results. On GitHub's
standard runners the whole CI suite verifies in about 25 minutes of wall
time across parallel jobs. Most harnesses take seconds; the slowest, the
Huffman length builder, encode/decode round trip and table-decoder
equivalence, take 7–10 minutes and peak at about 10.7 GiB. A manual run of the workflow can select modules
and harnesses (by name pattern), change the time limit, include the
extended set, and publish each harness's log to a `ci-logs/*` branch as
soon as it finishes.

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
- **Reference versions of fast paths.** Each speed-oriented routine has a
  plain reference version, and a dedicated harness ties the two together.
  The word-at-a-time `common_prefix` is proved equal to a byte loop. The
  match copy `copy_within_buffer` is proved to meet a specification that
  determines its result uniquely, which the byte-loop reference
  `copy_within_buffer_ref` also meets. Larger harnesses replace the fast
  routine with its reference, whose model is much smaller; the claim is
  unchanged because the two compute the same function. In the same way,
  the Huffman decoder's property harnesses run on `Canonical`, the
  bit-by-bit decoder, and `fast_decode_matches_slow` proves the
  table-driven `Decoder` (which wraps a `Canonical`) equal to it.
- **Specification stub for sorting.** The Huffman length builder's
  harnesses replace its heapsort with the heapsort's specification: the
  permutation of the input sorted by (weight, index), chosen
  nondeterministically. The keys are distinct, so that permutation exists
  and is unique, and the stub is exactly the function's behaviour.
  `sort_by_weight_sorts` proves the heapsort meets it.
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
output by index and rebuilds Huffman decoders in place. The slow Huffman
decoding steps are written out rather than looped. The bit reader's
`read_bits` loads only the bytes a read needs, so concrete header bits in
a proof are never mixed with symbolic payload bytes (only the Huffman
table lookup reads eight bytes ahead). None of these changes behaviour;
the zlib interop tests confirm the output is unchanged.

## What is proven (in CI)

### Bit I/O (`bitio`)

| Harness | Property |
|---|---|
| `write_then_read_roundtrip` | Any three writes of 0–32 bits read back exactly; the bit count is exact; the output is the minimum number of bytes; padding is zero. |
| `reader_eof_is_exact` | Over 0–4 bytes of input, three reads of 0–32 bits each succeed exactly when enough bits remain, and otherwise return `UnexpectedEof`. |
| `read_bits_matches_read_bit` | `read_bits(n)` equals `n` single-bit reads, at any bit offset 0–7, for n ≤ 16. |
| `aligned_bytes_roundtrip` | Bits, padding, a byte-aligned payload, then more bits: all round-trip; `bytes_consumed` is exact; over-long reads are errors. |
| `aligned_bytes_after_lookahead_short`, `aligned_bytes_after_lookahead_long` | The same after the reader has looked ahead (as the Huffman decoder does before the next block header), so whole bytes of the payload already sit in its buffer: they come out first and in order. A 3-byte payload takes the zero-padded refill and can be wholly buffered; a 9-byte payload is split by an 8-byte refill. This is the stored block that follows a Huffman block. |
| `reverse_bits_is_reversal` | For every code of 1–15 bits, `reverse_bits` reverses exactly those bits and is an involution. |

### Adler-32 (`adler32`)

| Harness | Property |
|---|---|
| `update_matches_bytewise` | From any valid state and for any 0–6 bytes, the update that reduces once per span equals reducing after every byte. A compile-time check shows the sums cannot overflow within a span of 5552 bytes (zlib's bound), so spans of any length are safe. |
| `from_checksum_resumes` | Resuming from a finished checksum continues the same computation, for every state. |

### CRC-32 (`crc32`)

| Harness | Property |
|---|---|
| `table_matches_definition` | All 256 table entries equal the bitwise definition, `update_bitwise`. That is a bit-at-a-time loop written independently of the table code: it shares no function or constant with it. |
| `table_step_equals_bitwise_step` | For **every** 32-bit register and **every** byte, the table step equals eight steps of that definition. |
| `slicing_tables_follow_recurrence` | Each of the seven further slicing-by-8 tables is the previous one followed by a zero byte. |
| `byte_step_is_linear`, `lanes_are_linear`, `eight_byte_step_matches_on_a_basis` | For **every** register and **every** eight bytes, the eight-byte step equals eight table steps. As one query this is too hard for the solver, so it is proved by linearity. The first two harnesses prove, for all inputs, that both sides are GF(2)-linear in their 96 input bits. The third checks that they agree on all 96 basis vectors. Linear maps that agree on a basis are equal. With the previous rows, `update` (eight bytes at a time, then byte by byte) equals the reference `update_bitwise` on inputs of every length: both apply steps proved equal, between the same two inversions. |
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
| `limit_depths_keeps_code_complete` | The length-limit repair on its own, for every complete code of six symbols, weights 0–3 (they only break ties) and limits of 3 and 4 bits: the result is a complete code within the limit. Six symbols are the fewest that reach the under-subscribed half of the repair (for example depths 5,5,4,3,2,1 at limit 3). Five never do, so the builder harnesses above do not cover it. |
| `decoder_new_never_panics` | For any lengths (0–255) over 8 symbols, the decoder builder never panics. It accepts exactly the lengths whose Kraft sum is ≤ 1 and reports completeness exactly. |
| `encode_then_decode_symbol` | For any valid lengths over 6 symbols and any coded symbol, packing its canonical code as the encoder does and decoding returns that symbol. Exactly its length is consumed. |
| `decode_arbitrary_bits` | Decoding arbitrary bits with any accepted lengths never panics, and only yields symbols that have codes. |
| `fast_decode_matches_slow` | For any valid lengths over 4 symbols (up to 6 bits), any lookup-table width of 1–4 bits and any 0–1 bytes of input, the table-driven decoder returns exactly what the bit-by-bit decoder returns and consumes the same bits. This covers codes shorter than, as long as and longer than the table, and input ending mid-code. (The two harnesses above check the bit-by-bit decoder.) |
| `sort_by_weight_sorts` | For any weights and any 0–5 distinct indices, the length builder's heapsort returns them sorted by (weight, index). The length-builder harnesses above use this as a specification stub. |

### LZ77 (`lz77`)

| Harness | Property |
|---|---|
| `tokens_valid_and_replay_small_tables` | For any 4 bytes, with a 2-entry chain limit, lazy matching and every hash function: every match has length 3–258 and a distance within the data produced so far, and the tokens describe the input exactly. |
| `tokens_valid_and_replay_level9` | Same with level 9's search parameters. |
| `tokens_valid_and_replay_greedy` | Same with level 1's greedy parameters. |
| `runs_are_matched` | Six equal bytes become one literal and one overlapping match. |
| `common_prefix_matches_reference` | The word-at-a-time match comparison equals the byte-by-byte loop, for any two inputs of up to 20 bytes. The tokenizer harnesses above use the byte loop. |
| `copy_within_buffer_meets_spec` | For every position, distance and length that fit a 12-byte buffer, each written byte equals the byte `dist` earlier and nothing else changes. This includes heavily overlapping copies. |
| `rle_tokens_valid_and_replay` | Run-length mode (`CompressionOptions::rle()`): for any 6 bytes, every match has distance 1 and length 3–258, and the tokens describe the input exactly. |
| `no_chain_means_literals_only` | Huffman-only mode (`CompressionOptions::huffman_only()`): every token is the literal input byte, for any start position and output capacity, and the tokenizer's contract holds. |
| `short_copy_meets_spec` | The same for the inline path for short copies whose source lies wholly before the destination, on a 32-byte buffer that reaches every case of it (up to 16 bytes). |
| `replay_arbitrary_tokens` | Replaying arbitrary tokens never panics; invalid matches are errors. |

### Encoder (`deflate`)

| Harness | Property |
|---|---|
| `code_length_rle_roundtrip` | The run-length coding of any 8 code lengths is undone exactly by the *decoder's own* expansion routine. |
| `code_length_rle_zero_runs` | Same for 12 lengths drawn from {0, v}, reaching the long-zero-run code 18. |
| `fixed_code_table_matches_rfc` | The compile-time fixed code tables hold the RFC's canonical codes. |
| `stored_cost_is_exact` | The stored-block size model equals the bits written, from any bit offset, for 0, 3 and 8 bytes of any content. |
| `stored_cost_adds_up_across_blocks` | Across block boundaries, from any bit offset, the size model is one full block plus the rest from the byte-aligned point where that block ends. This holds for any length up to 16 MiB and block sizes of 1–4 bytes and 65,535 (production). With the previous row, the model is exact for any number of blocks. Two one-byte blocks written back to back take exactly the modelled bits. |
| `fixed_tokens_have_the_rfc_layout` | For any literal, or any match of length 3–258 at distance 1–32768, the fixed-Huffman coder writes exactly RFC 1951's layout. That is the symbol's fixed code (most significant bit first), its extra bits, the distance's five-bit code and extra bits, then end of block. The expected codes are computed from the RFC's ranges, not from the tables. |

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
| `size_limits_are_exact` | A size or offset is stored iff it is below `0xFFFFFFFF` (the ZIP64 marker). An entry is refused iff its 30-byte local header, name and data would end at or past that offset, and the archive iff its 22-byte end record would. |
| `dos_datetime_roundtrip` | Every valid DOS timestamp survives encoding and decoding. |
| `civil_date_conversion_roundtrip` | Every day from 1980-01-01 to 2107-12-31 converts to a valid date and back. |
| `is_valid_is_the_calendar` | For every value of every field, `is_valid` is exactly "in the DOS range and a real calendar date". This is stated without `is_leap` or `days_in_month`: converting the date to a day number and back gives the same date. |
| `unix_time_always_valid` | Every `i64` Unix time maps to a valid DOS timestamp. |
| `unix_time_converts_exactly` | Every `i64` Unix time in the DOS range converts to the right time, the input rounded down to an even second, and every other time clamps to the nearer end. |
| `validate_name_is_exactly_the_rule` | For every byte string of 0–8 bytes, a name is accepted exactly when it meets a rule stated position by position. It must be non-empty, have no backslash or NUL, have every `/` follow a non-`/` byte, have no `.` or `..` component, and not start with a drive letter. Accepted names cannot escape the extraction directory, and no safe name is refused. |

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
| `deflate::proofs::fixed_block_tokens_roundtrip` | A literal of any value (every 8- and 9-bit literal code), then a match of length 12 (a length code with an extra bit) at distance 1 (an overlapping copy), written by `write_fixed` and decoded by the real decoder, gives back exactly the bytes it stands for. This runs the decoder's Huffman-block loop on encoder output without the match finder. The decoder's lookup table is replaced by the canonical walk, which `fast_decode_matches_slow` proves it agrees with: the 288-symbol table is too large to check symbolically. |
| `e2e::zip_roundtrip_stored` | A one-entry archive of any 3 bytes, stored, reads back exactly and passes its CRC check. |
| `huffman::proofs::fast_decode_matches_slow_wide` | The table-driven decoder equals the bit-by-bit one for 5 symbols of up to 9 bits and 0–2 bytes of input. |
| `crc32::proofs::eight_byte_step_equals_table_steps` | The eight-byte CRC step lemma as a single query. CI proves it by linearity instead (see above). |
| `crc32::proofs::four_byte_lanes_equal_table_steps` | Its four-byte half as a single query. |

## Checking the proofs: coverage and mutation

A proof only covers the code it executes, and only checks what its
assertions say. Two measurements check both, in a manual run of the CI
workflow with `assurance` set:

- **Coverage.** Every harness runs with Kani's source coverage
  (`--coverage -Z source-coverage`), which records, for each code region,
  whether *some* input within the harness's bounds reaches it. The union
  over all harnesses shows the functions no proof enters and the branches
  no proof takes. An unreached region inside a proved function means the
  bounds, the stubs or an assumption exclude it.
- **Mutation.** cargo-mutants makes small deliberate bugs (swaps an
  operator, negates a condition, replaces a function body with a constant)
  and checks that something fails. It runs twice: against the harnesses
  run as randomised tests (`kani-sample`) and against the whole test
  suite. A mutant that the sampled harnesses catch also fails the real
  proof, since sampling only finds genuine counterexamples.

`.github/scripts/assurance_report.py` joins the two and classifies every
mutant:

| class | meaning |
|---|---|
| proved | the sampled proofs catch it |
| proof-reach | a proof executes the code, but sampling did not catch the mutant |
| tests-only | no proof executes the code; the tests catch the mutant |
| unchecked | nothing catches it |

A follow-up run with `recheck` set verifies every proof-reach mutant with
real Kani, which checks all inputs where sampling tries some. A mutant
that survives the real proofs marks a property that does not pin the
behaviour down, or a change that has no observable effect (an equivalent
mutant, such as a match-finder heuristic that only affects the
compression ratio). The reports are published to the `ci-logs/assurance`
and `ci-logs/assurance-recheck` branches.

Proof code and the reference specifications the proofs compare against
are marked `#[cfg_attr(mutants, mutants::skip)]`: they are the checker,
not the code under test.

To run it, dispatch the CI workflow:

- With `assurance` set, a run takes about an hour on public runners: 12 Kani jobs with coverage, then 2 × 16 mutation shards.
- Then dispatch again with `recheck` set (and `modules` set to `none`) for the real-Kani step.
- `mutants_passes` and `mutants_shards` re-run part of the mutation; `reuse_run` merges results from earlier runs.

Some details of the setup:

- **Memory and file-size limits.** Mutated tests run under them, so a mutant that loops while allocating or writing fails its test instead of taking down the runner.
- **Two Huffman harnesses** (`encode_then_decode_symbol` and `fast_decode_matches_slow`) exceed 14 GiB with coverage instrumentation. Their coverage is recorded in a second run without Kani's default checks, which do not affect what code is reached.

### Results (October 2026)

Of about 1,900 mutants that compile:

| caught by | first run | after closing the gaps |
|---|---:|---:|
| the proofs | 1,203 (63%) | 1,281 (67%) |
| only the tests | 403 (21%) | 486 (26%) |
| nothing | 297 (16%) | 135 (7%) |

In the second run, 42 of the mutants caught by the proofs were missed by
sampling and caught only by real Kani. One more was undetermined: its
harness timed out. By file, after:

| file | mutants | proofs | tests only | nothing |
|---|---:|---:|---:|---:|
| `adler32` | 23 | 20 | 2 | 1 |
| `bitio` | 196 | 158 | 19 | 19 |
| `crc32` | 78 | 75 | 2 | 1 |
| `tables` | 57 | 55 | 0 | 2 |
| `huffman` | 276 | 211 | 39 | 26 |
| `lz77` | 281 | 200 | 35 | 46 |
| `deflate` | 232 | 109 | 106 | 16 |
| `inflate` | 107 | 81 | 26 | 0 |
| `zip` | 442 | 372 | 55 | 15 |
| `write`, `compat`, `gzip_header`, `error` | 149 | 0 | 143 | 6 |
| `vzip` | 62 | 0 | 59 | 3 |

(Two mutants found by the second run were closed after it and are counted as caught here: the `is_complete` check and the degenerate alphabets of `build_lengths`.)

**What the 135 are.** Each was checked by hand, and none is a missing check:

- **Match-finder heuristics** (most of the 46 in `lz77`): hash function, chain limits, lazy-match thresholds, run lengths, and the boundary between the word-at-a-time and byte-at-a-time comparison and copy. They change the compression ratio and speed, never validity: every candidate match is compared byte for byte and bounded by the window.
- **Fast paths with a proven fallback** (most of `huffman` and `bitio`): the Huffman lookup table, the bit reader's refill and look-ahead thresholds. A wrong table entry or a skipped refill sends decoding down the canonical walk or the byte-at-a-time path. `fast_decode_matches_slow` and the bit-reader proofs show those give the same results.
- **Size-only choices:** ties between block types, and run-length coding of code lengths at its thresholds. The encoder still emits a valid stream that decodes to the input; only its size can change, and the size bound is tested.
- **Equivalent rewrites:**
  - `|` to `^` on bit fields that do not overlap;
  - `<` to `<=` where equality gives the same result;
  - buffer capacities;
  - date branches for years before 1970, which the DOS range never reaches.
- **Inert inputs:**
  - platform-specific `default_os` arms;
  - `byte_bitwise`'s register input, which the table builder only ever passes as zero;
  - `vzip`'s `--no-verify`, which only switches off the run-time self-check.

**How the gaps were closed.** The first run found the following; the second run, and mutant checks on each fix, confirm they are now caught:

1. **A stored block after a Huffman block.** New harnesses `aligned_bytes_after_lookahead_short` and `_long` cover a reader that has already buffered whole bytes of the stored data. A corpus entry of compressible, then random, then compressible data is decoded from every zlib level and strategy.
2. **Incomplete Huffman codes.** `tests/incomplete_codes.rs` hand-builds dynamic blocks with a single one-bit code, no distance codes, and incomplete codes of every kind. Python's zlib confirms which are valid.
3. **Length-limit repair.**
   - Proved on its own by `limit_depths_keeps_code_complete` at six symbols, the fewest that reach its second half.
   - Fibonacci-frequency tests cover alphabets of 2–40 symbols at every limit.
   - The documented tie-breaking is pinned on Huffman-shaped examples.
4. **DOS dates.**
   - `is_valid_is_the_calendar` equates `is_valid` with an independent definition.
   - `unix_time_converts_exactly` proves the conversion right, not just valid.
   - Tests cover leap years and invalid dates.
5. **Streaming writers.** Tests cover a deferred write error followed by `flush` or `finish` for all three encoders, `Interrupted` retries, and `GzEncoder` finishing on drop.
6. **CRC-32.** Its specification is now a separate bit loop that shares nothing with the implementation.
7. **Stored-block sizes.** `stored_cost_adds_up_across_blocks` covers any number of blocks.
8. **ZIP names.** `validate_name_is_exactly_the_rule` proves names are accepted exactly when they meet the rule: no safe name is refused.
9. **Encoder and decoder layers.**
   - `fixed_tokens_have_the_rfc_layout` proves the fixed-Huffman encoder's bit layout for every token.
   - A token round trip through the real decoder is in the extended set.
   - Dynamic-block writing and reading, and archive-level ZIP code, remain covered by tests and the per-component proofs.
10. **Smaller items.**
    - ZIP size limits are proved exact; multi-disk and ZIP64 rejection and directory modes are tested.
    - The `deflate`-compatible option and level mappings are pinned to `deflate`'s and zlib's tables.
    - Tests now cover the output-size bound against stored blocks, the streaming memory bound, gzip header values and messages, and `vzip`'s options and reports.

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
  blocks and for fixed-Huffman tokens.**
  - The fixed-Huffman coder's bit layout is proven for every single token.
  - A literal of any value followed by a match round-trips through the real decoder in the extended set: the fixed-Huffman decoder is too large to check symbolically on a CI runner.
  - Round trips that include the match finder, and whole-archive round trips, are in the extended set. In CI they are covered compositionally: tokenizer validity, code tables, Huffman encode and decode, bit I/O.
  - Dynamic-Huffman blocks need more unrolling than a model checker handles at DEFLATE's alphabet sizes. They are covered by the length builder and run-length proofs on small alphabets, zlib interop, and `verify` at run time.
- **Huffman construction is proven on alphabets of 3–6 symbols**, not on
  DEFLATE's 19, 30 and 286.
  - The length-limit repair is proven on its own for every complete code of up to 6 symbols, which reaches both of its loops.
  - Its argument does not depend on the alphabet size, and tests exercise it with Fibonacci frequencies on alphabets of 2–40 symbols at every limit.
- **Compression ratio and speed** are not correctness properties and are
  not verified.
- **The `vzip` command-line tool** is not verified. It is a thin layer over
  the library, and extraction refuses names that fail `validate_name`.
- **The trusted base** includes the Rust compiler and standard library,
  Kani and its CBMC back end, and the harnesses themselves. A harness that
  asserts the wrong thing proves the wrong thing, so review them as you
  would a specification.
