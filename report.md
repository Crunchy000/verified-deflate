# Assurance report

1905 viable mutants (160 more did not compile). 67 proof harnesses with coverage data (66 recorded).

| class | mutants | share | meaning |
|---|---:|---:|---|
| proved | 1239 | 65% | caught by the proofs (sampled, so the real proofs catch them too) |
| proof-reach | 216 | 11% | a proof executes the code but sampling missed it: re-check with real Kani |
| tests-only | 414 | 22% | no proof executes the code; the tests catch it |
| unchecked | 36 | 2% | nothing catches it: a gap, or a change with no observable effect |

Proof-reach mutants caught by the tests: 104; caught by nothing: 112.

## By file

| file | mutants | proved | proof-reach | tests-only | unchecked | proof coverage (regions) |
|---|---:|---:|---:|---:|---:|---:|
| src/adler32.rs | 23 | 20 | 1 | 2 | 0 | 10/10 100% |
| src/bin/vzip.rs | 62 | 0 | 0 | 58 | 4 | 0/0 - |
| src/bitio.rs | 196 | 158 | 23 | 15 | 0 | 91/98 93% |
| src/compat.rs | 46 | 0 | 0 | 42 | 4 | 0/0 - |
| src/crc32.rs | 78 | 75 | 0 | 2 | 1 | 18/19 95% |
| src/deflate.rs | 232 | 109 | 15 | 100 | 8 | 66/142 46% |
| src/error.rs | 1 | 0 | 0 | 1 | 0 | 0/0 - |
| src/gzip_header.rs | 61 | 0 | 0 | 58 | 3 | 0/0 - |
| src/huffman.rs | 278 | 194 | 65 | 17 | 2 | 227/250 91% |
| src/inflate.rs | 107 | 78 | 4 | 25 | 0 | 38/120 32% |
| src/lz77.rs | 281 | 197 | 75 | 5 | 4 | 150/166 90% |
| src/tables.rs | 57 | 55 | 0 | 0 | 2 | 11/11 100% |
| src/write.rs | 41 | 0 | 0 | 41 | 0 | 0/0 - |
| src/zip.rs | 442 | 353 | 33 | 48 | 8 | 134/138 97% |

## Shards

- proofs: 16 shards reported
- tests: 16 shards reported
- harnesses without coverage data (timed out or failed to run): deflate::proofs::fixed_block_tokens_roundtrip


## Priority 1: caught by nothing (148)

Neither the proofs nor the tests notice these changes. Each is either a missing check or a change with no observable effect (an equivalent mutant).

- **src/bin/vzip.rs: create** (3)
  - `src/bin/vzip.rs:76:41: replace == with != in create`
  - `src/bin/vzip.rs:76:18: delete ! in create`
  - `src/bin/vzip.rs:83:37: delete field verify from struct FileOptions expression in create`
- **src/compat.rs: deflate_bytes_gzip_conf** (3)
  - `src/compat.rs:299:29: replace / with % in deflate_bytes_gzip_conf`
  - `src/compat.rs:299:33: replace + with * in deflate_bytes_gzip_conf`
  - `src/compat.rs:299:29: replace / with * in deflate_bytes_gzip_conf`
- **src/deflate.rs: encode_with_matcher** (2)
  - `src/deflate.rs:96:19: replace < with == in encode_with_matcher`
  - `src/deflate.rs:96:19: replace < with <= in encode_with_matcher`
- **src/deflate.rs: write_block** (2)
  - `src/deflate.rs:293:36: replace < with <= in write_block`
  - `src/deflate.rs:293:55: replace < with <= in write_block`
- **src/gzip_header.rs: default_os** (2)
  - `src/gzip_header.rs:286:9: delete match arm "macos" in default_os`
  - `src/gzip_header.rs:287:9: delete match arm "win32" in default_os`
- **src/huffman.rs: decode_without_table** (2)
  - `src/huffman.rs:687:5: replace decode_without_table -> Result<u16, InflateError> with Ok(1)`
  - `src/huffman.rs:687:5: replace decode_without_table -> Result<u16, InflateError> with Ok(0)`
- **src/lz77.rs: Matcher::tokenize** (2)
  - `src/lz77.rs:325:32: replace + with * in Matcher::tokenize`
  - `src/lz77.rs:325:32: replace + with - in Matcher::tokenize`
- **src/tables.rs: len_code_by_search** (2)
  - `src/tables.rs:65:13: replace + with * in len_code_by_search`
  - `src/tables.rs:65:17: replace < with <= in len_code_by_search`
- **src/zip.rs: ZipWriter::add_directory** (2)
  - `src/zip.rs:688:56: replace | with ^ in ZipWriter::add_directory`
  - `src/zip.rs:688:41: replace | with ^ in ZipWriter::add_directory`
- **src/zip.rs: ZipWriter::add_file** (2)
  - `src/zip.rs:658:24: replace < with <= in ZipWriter::add_file`
  - `src/zip.rs:669:39: replace | with ^ in ZipWriter::add_file`
- **src/zip.rs: civil_from_days** (2)
  - `src/zip.rs:196:40: replace - with / in civil_from_days`
  - `src/zip.rs:196:40: replace - with + in civil_from_days`
- **src/zip.rs: days_from_civil** (2)
  - `src/zip.rs:185:40: replace - with / in days_from_civil`
  - `src/zip.rs:185:40: replace - with + in days_from_civil`
- **src/bin/vzip.rs: test** (1)
  - `src/bin/vzip.rs:242:21: replace += with -= in test`
- **src/compat.rs: CompressionOptions::engine** (1)
  - `src/compat.rs:171:82: delete field max_lazy from struct MatchConfig expression in CompressionOptions::engine`
- **src/crc32.rs: byte_bitwise** (1)
  - `src/crc32.rs:18:21: replace ^ with | in byte_bitwise`
- **src/deflate.rs: StreamEncoder::compress_pending** (1)
  - `src/deflate.rs:204:16: replace > with >= in StreamEncoder::compress_pending`
- **src/deflate.rs: plan_dynamic** (1)
  - `src/deflate.rs:508:36: replace + with * in plan_dynamic`
- **src/deflate.rs: write_dynamic** (1)
  - `src/deflate.rs:539:34: replace | with ^ in write_dynamic`
- **src/deflate.rs: write_fixed** (1)
  - `src/deflate.rs:345:34: replace | with ^ in write_fixed`
- **src/gzip_header.rs: (outside functions)** (1)
  - `src/gzip_header.rs:10:21: replace << with >>`
- **src/lz77.rs: Matcher::prime** (1)
  - `src/lz77.rs:125:26: replace < with <= in Matcher::prime`
- **src/lz77.rs: Matcher::reset** (1)
  - `src/lz77.rs:114:9: replace Matcher::reset with ()`
- **src/lz77.rs: Matcher::tokenize** (21)
  - `src/lz77.rs:310:65: replace - with / in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:310:65: replace - with + in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:319:68: replace < with > in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:319:37: replace < with == in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:319:64: replace + with * in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:319:30: replace && with || in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:319:64: replace + with - in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:319:68: replace < with == in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:319:57: replace && with || in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:319:37: replace < with <= in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:319:68: replace < with <= in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:319:37: replace < with > in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:320:78: replace / with % in Matcher::tokenize` — reached by tokens_valid_and_replay_small_tables
  - `src/lz77.rs:320:36: replace >= with < in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:320:78: replace / with * in Matcher::tokenize` — reached by tokens_valid_and_replay_small_tables
  - `src/lz77.rs:320:82: replace + with * in Matcher::tokenize` — reached by tokens_valid_and_replay_small_tables
  - `src/lz77.rs:321:87: replace + with * in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:321:87: replace + with - in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:321:60: replace + with * in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:333:43: replace || with && in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:333:50: replace <= with > in Matcher::tokenize` — reached by tokens_valid_and_replay_greedy
- **src/huffman.rs: Decoder::rebuild_with_fast_bits** (12)
  - `src/huffman.rs:605:31: replace > with >= in Decoder::rebuild_with_fast_bits` — reached by decoder_new_never_panics, fast_decode_matches_slow
  - `src/huffman.rs:605:31: replace > with < in Decoder::rebuild_with_fast_bits` — reached by decoder_new_never_panics, fast_decode_matches_slow
  - `src/huffman.rs:605:31: replace > with == in Decoder::rebuild_with_fast_bits` — reached by decoder_new_never_panics, fast_decode_matches_slow
  - `src/huffman.rs:623:17: replace < with > in Decoder::rebuild_with_fast_bits` — reached by decoder_new_never_panics, fast_decode_matches_slow
  - `src/huffman.rs:623:17: replace < with == in Decoder::rebuild_with_fast_bits` — reached by decoder_new_never_panics, fast_decode_matches_slow
  - `src/huffman.rs:625:18: replace != with == in Decoder::rebuild_with_fast_bits` — reached by decoder_new_never_panics, fast_decode_matches_slow
  - `src/huffman.rs:630:31: replace < with == in Decoder::rebuild_with_fast_bits` — reached by decoder_new_never_panics, fast_decode_matches_slow
  - `src/huffman.rs:630:36: replace << with >> in Decoder::rebuild_with_fast_bits` — reached by decoder_new_never_panics, fast_decode_matches_slow
  - `src/huffman.rs:630:31: replace < with > in Decoder::rebuild_with_fast_bits` — reached by decoder_new_never_panics, fast_decode_matches_slow
  - `src/huffman.rs:631:61: replace | with & in Decoder::rebuild_with_fast_bits` — reached by decoder_new_never_panics, fast_decode_matches_slow
  - `src/huffman.rs:631:54: replace << with >> in Decoder::rebuild_with_fast_bits` — reached by decoder_new_never_panics, fast_decode_matches_slow
  - `src/huffman.rs:631:61: replace | with ^ in Decoder::rebuild_with_fast_bits` — reached by decoder_new_never_panics, fast_decode_matches_slow
- **src/huffman.rs: limit_depths** (7)
  - `src/huffman.rs:220:15: replace > with >= in limit_depths` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit, limit_depths_keeps_code_complete
  - `src/huffman.rs:233:41: replace > with < in limit_depths` — reached by build_lengths_complete_and_limited, limit_depths_keeps_code_complete
  - `src/huffman.rs:233:90: replace < with <= in limit_depths` — reached by limit_depths_keeps_code_complete
  - `src/huffman.rs:250:25: replace > with >= in limit_depths` — reached by limit_depths_keeps_code_complete
  - `src/huffman.rs:253:90: replace > with >= in limit_depths` — reached by limit_depths_keeps_code_complete
  - `src/huffman.rs:253:90: replace > with == in limit_depths` — reached by limit_depths_keeps_code_complete
  - `src/huffman.rs:261:23: replace << with >> in limit_depths` — reached by limit_depths_keeps_code_complete
- **src/lz77.rs: Matcher::hash** (6)
  - `src/lz77.rs:133:66: replace << with >> in Matcher::hash` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:133:72: replace | with & in Matcher::hash` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:133:72: replace | with ^ in Matcher::hash` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:133:38: replace | with ^ in Matcher::hash` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:133:100: replace << with >> in Matcher::hash` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:133:60: replace + with * in Matcher::hash` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
- **src/lz77.rs: Matcher::longest_match** (6)
  - `src/lz77.rs:209:32: replace < with <= in Matcher::longest_match` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:244:24: replace > with >= in Matcher::longest_match` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:254:19: replace -= with /= in Matcher::longest_match` — reached by tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:254:19: replace -= with += in Matcher::longest_match` — reached by tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:256:39: replace < with == in Matcher::longest_match` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:256:39: replace < with <= in Matcher::longest_match` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
- **src/bitio.rs: BitReader<'a>::refill** (5)
  - `src/bitio.rs:143:9: replace BitReader<'a>::refill with ()` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, fast_decode_matches_slow
  - `src/bitio.rs:143:23: replace > with >= in BitReader<'a>::refill` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, fast_decode_matches_slow
  - `src/bitio.rs:143:23: replace > with < in BitReader<'a>::refill` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, fast_decode_matches_slow
  - `src/bitio.rs:143:23: replace > with == in BitReader<'a>::refill` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, fast_decode_matches_slow
  - `src/bitio.rs:157:35: replace < with <= in BitReader<'a>::refill` — reached by aligned_bytes_after_lookahead_short, fast_decode_matches_slow
- **src/deflate.rs: rle_code_lengths** (5)
  - `src/deflate.rs:431:16: replace > with < in rle_code_lengths` — reached by code_length_rle_roundtrip, code_length_rle_zero_runs
  - `src/deflate.rs:431:16: replace > with == in rle_code_lengths` — reached by code_length_rle_roundtrip, code_length_rle_zero_runs
  - `src/deflate.rs:431:16: replace > with >= in rle_code_lengths` — reached by code_length_rle_roundtrip, code_length_rle_zero_runs
  - `src/deflate.rs:435:16: replace > with >= in rle_code_lengths` — reached by code_length_rle_roundtrip, code_length_rle_zero_runs
  - `src/deflate.rs:441:12: replace > with >= in rle_code_lengths` — reached by code_length_rle_roundtrip, code_length_rle_zero_runs
- **src/zip.rs: DosDateTime::to_dos** (4)
  - `src/zip.rs:239:47: replace | with ^ in DosDateTime::to_dos` — reached by dos_datetime_roundtrip
  - `src/zip.rs:239:77: replace | with ^ in DosDateTime::to_dos` — reached by dos_datetime_roundtrip
  - `src/zip.rs:240:65: replace | with ^ in DosDateTime::to_dos` — reached by dos_datetime_roundtrip
  - `src/zip.rs:240:94: replace | with ^ in DosDateTime::to_dos` — reached by dos_datetime_roundtrip
- **src/bitio.rs: BitReader<'a>::peek** (3)
  - `src/bitio.rs:201:23: replace < with <= in BitReader<'a>::peek` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, fast_decode_matches_slow
  - `src/bitio.rs:201:23: replace < with > in BitReader<'a>::peek` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, fast_decode_matches_slow
  - `src/bitio.rs:201:23: replace < with == in BitReader<'a>::peek` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, fast_decode_matches_slow
- **src/bitio.rs: BitReader<'a>::read_bits** (3)
  - `src/bitio.rs:169:22: replace > with >= in BitReader<'a>::read_bits` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, aligned_bytes_roundtrip, read_bits_matches_read_bit …
  - `src/bitio.rs:169:22: replace > with == in BitReader<'a>::read_bits` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, aligned_bytes_roundtrip, read_bits_matches_read_bit …
  - `src/bitio.rs:170:23: replace < with <= in BitReader<'a>::read_bits` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, aligned_bytes_roundtrip, read_bits_matches_read_bit …
- **src/huffman.rs: huffman_depths** (3)
  - `src/huffman.rs:292:40: replace > with >= in huffman_depths` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
  - `src/huffman.rs:301:23: replace - with / in huffman_depths` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
  - `src/huffman.rs:301:23: replace - with + in huffman_depths` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
- **src/lz77.rs: common_prefix** (3)
  - `src/lz77.rs:382:26: replace < with <= in common_prefix` — reached by common_prefix_matches_reference, rle_tokens_valid_and_replay, runs_are_matched, tokens_valid_and_replay_greedy …
  - `src/lz77.rs:384:13: replace + with * in common_prefix` — reached by common_prefix_matches_reference, rle_tokens_valid_and_replay, runs_are_matched, tokens_valid_and_replay_greedy …
  - `src/lz77.rs:384:17: replace <= with > in common_prefix` — reached by common_prefix_matches_reference, rle_tokens_valid_and_replay, runs_are_matched, tokens_valid_and_replay_greedy …
- **src/lz77.rs: tokenize_runs** (3)
  - `src/lz77.rs:357:45: replace - with / in tokenize_runs` — reached by rle_tokens_valid_and_replay
  - `src/lz77.rs:358:38: replace < with <= in tokenize_runs` — reached by rle_tokens_valid_and_replay
  - `src/lz77.rs:360:52: replace - with / in tokenize_runs` — reached by rle_tokens_valid_and_replay
- **src/bitio.rs: BitWriter::write_bits** (2)
  - `src/bitio.rs:39:22: replace > with == in BitWriter::write_bits` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, aligned_bytes_roundtrip, write_then_read_roundtrip …
  - `src/bitio.rs:39:22: replace > with >= in BitWriter::write_bits` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, aligned_bytes_roundtrip, write_then_read_roundtrip …
- **src/deflate.rs: emit_run** (2)
  - `src/deflate.rs:457:26: replace < with <= in emit_run` — reached by code_length_rle_zero_runs
  - `src/deflate.rs:473:26: replace < with <= in emit_run` — reached by code_length_rle_roundtrip, code_length_rle_zero_runs
- **src/huffman.rs: Canonical::rebuild** (2)
  - `src/huffman.rs:446:23: replace << with >> in Canonical::rebuild` — reached by decode_arbitrary_bits, decoder_new_never_panics, encode_then_decode_symbol, fast_decode_matches_slow
  - `src/huffman.rs:474:44: replace + with * in Canonical::rebuild` — reached by decode_arbitrary_bits, decoder_new_never_panics, encode_then_decode_symbol, fast_decode_matches_slow
- **src/huffman.rs: sort_by_weight** (2)
  - `src/huffman.rs:333:23: replace / with * in sort_by_weight` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit, sort_by_weight_sorts
  - `src/huffman.rs:339:15: replace > with >= in sort_by_weight` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit, sort_by_weight_sorts
- **src/zip.rs: DosDateTime::from_unix_seconds** (2)
  - `src/zip.rs:262:25: replace < with <= in DosDateTime::from_unix_seconds` — reached by unix_time_always_valid, unix_time_converts_exactly
  - `src/zip.rs:264:24: replace > with >= in DosDateTime::from_unix_seconds` — reached by unix_time_always_valid, unix_time_converts_exactly
- **src/adler32.rs: Adler32::finalize** (1)
  - `src/adler32.rs:74:24: replace | with ^ in Adler32::finalize` — reached by from_checksum_resumes, update_matches_bytewise
- **src/bitio.rs: BitReader<'a>::available** (1)
  - `src/bitio.rs:212:9: replace BitReader<'a>::available -> u32 with 0` — reached by fast_decode_matches_slow
- **src/bitio.rs: BitReader<'a>::fill** (1)
  - `src/bitio.rs:126:17: replace < with <= in BitReader<'a>::fill` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, aligned_bytes_roundtrip, read_bits_matches_read_bit …
- **src/bitio.rs: BitReader<'a>::read_aligned_bytes** (1)
  - `src/bitio.rs:239:24: replace < with <= in BitReader<'a>::read_aligned_bytes` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, aligned_bytes_roundtrip, roundtrip_stored …
- **src/bitio.rs: BitWriter::pad_to_byte** (1)
  - `src/bitio.rs:65:23: replace > with >= in BitWriter::pad_to_byte` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, aligned_bytes_roundtrip, write_then_read_roundtrip …
- **src/bitio.rs: BitWriter::with_capacity** (1)
  - `src/bitio.rs:30:9: replace BitWriter::with_capacity -> Self with Default::default()` — reached by stored_cost_adds_up_across_blocks, stored_cost_is_exact, roundtrip_stored
- **src/bitio.rs: padded_word** (1)
  - `src/bitio.rs:279:13: replace < with <= in padded_word` — reached by aligned_bytes_after_lookahead_short, fast_decode_matches_slow
- **src/deflate.rs: CodeTable<N>::put_with_extra** (1)
  - `src/deflate.rs:262:48: replace | with ^ in CodeTable<N>::put_with_extra` — reached by fixed_tokens_have_the_rfc_layout
- **src/huffman.rs: (outside functions)** (1)
  - `src/huffman.rs:421:110: replace << with >>` — reached by sort_by_weight_sorts
- **src/huffman.rs: Decoder::decode** (1)
  - `src/huffman.rs:667:31: replace >> with << in Decoder::decode` — reached by fast_decode_matches_slow
- **src/huffman.rs: build_lengths_in** (1)
  - `src/huffman.rs:167:14: replace || with && in build_lengths_in` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_rejects_impossible_limit, build_lengths_tight_limit
- **src/huffman.rs: heavier** (1)
  - `src/huffman.rs:377:24: replace > with >= in heavier` — reached by sort_by_weight_sorts
- **src/huffman.rs: is_complete** (1)
  - `src/huffman.rs:58:5: replace is_complete -> bool with true` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
- **src/huffman.rs: take_lightest** (1)
  - `src/huffman.rs:318:29: replace < with > in take_lightest` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
- **src/inflate.rs: Output::reserve** (1)
  - `src/inflate.rs:107:14: replace > with >= in Output::reserve` — reached by roundtrip_stored, stored_block_semantics
- **src/lz77.rs: Matcher::bucket** (1)
  - `src/lz77.rs:140:28: replace > with >= in Matcher::bucket` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
- **src/lz77.rs: Matcher::max_dist** (1)
  - `src/lz77.rs:159:16: replace < with <= in Matcher::max_dist` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
- **src/lz77.rs: copy_within_buffer** (1)
  - `src/lz77.rs:461:37: replace < with <= in copy_within_buffer` — reached by copy_within_buffer_meets_spec
- **src/zip.rs: (outside functions)** (1)
  - `src/zip.rs:23:39: replace | with ^` — reached by central_header_roundtrip, eocd_roundtrip, find_eocd_is_sound, header_parsers_never_panic …
- **src/zip.rs: civil_from_days** (1)
  - `src/zip.rs:198:48: replace - with + in civil_from_days` — reached by civil_date_conversion_roundtrip, is_valid_is_the_calendar, unix_time_always_valid, unix_time_converts_exactly
- **src/zip.rs: find_eocd** (1)
  - `src/zip.rs:767:40: replace && with || in find_eocd` — reached by find_eocd_is_sound

## Priority 2: executed by proofs, caught only by tests (104)

A proof runs this code but its property did not fail under sampling. Real Kani may still catch these; any it does not mark a property that does not pin the behaviour down.

- **src/huffman.rs: limit_depths** (13)
  - `src/huffman.rs:233:90: replace < with > in limit_depths` — reached by limit_depths_keeps_code_complete
  - `src/huffman.rs:233:41: replace > with >= in limit_depths` — reached by build_lengths_complete_and_limited, limit_depths_keeps_code_complete
  - `src/huffman.rs:233:52: replace || with && in limit_depths` — reached by build_lengths_complete_and_limited, build_lengths_tight_limit, limit_depths_keeps_code_complete
  - `src/huffman.rs:233:90: replace < with == in limit_depths` — reached by limit_depths_keeps_code_complete
  - `src/huffman.rs:233:41: replace > with == in limit_depths` — reached by build_lengths_complete_and_limited, limit_depths_keeps_code_complete
  - `src/huffman.rs:233:65: replace == with != in limit_depths` — reached by build_lengths_complete_and_limited, limit_depths_keeps_code_complete
  - `src/huffman.rs:233:77: replace && with || in limit_depths` — reached by build_lengths_complete_and_limited, build_lengths_tight_limit, limit_depths_keeps_code_complete
  - `src/huffman.rs:253:77: replace && with || in limit_depths` — reached by limit_depths_keeps_code_complete
  - `src/huffman.rs:253:41: replace > with >= in limit_depths` — reached by limit_depths_keeps_code_complete
  - `src/huffman.rs:253:90: replace > with < in limit_depths` — reached by limit_depths_keeps_code_complete
  - `src/huffman.rs:253:65: replace == with != in limit_depths` — reached by limit_depths_keeps_code_complete
  - `src/huffman.rs:261:35: replace - with / in limit_depths` — reached by limit_depths_keeps_code_complete
  - `src/huffman.rs:261:35: replace - with + in limit_depths` — reached by limit_depths_keeps_code_complete
- **src/huffman.rs: huffman_depths** (9)
  - `src/huffman.rs:276:5: replace huffman_depths with ()` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
  - `src/huffman.rs:292:40: replace > with < in huffman_depths` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
  - `src/huffman.rs:292:40: replace > with == in huffman_depths` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
  - `src/huffman.rs:292:30: replace - with / in huffman_depths` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
  - `src/huffman.rs:292:22: replace + with * in huffman_depths` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
  - `src/huffman.rs:298:14: replace += with *= in huffman_depths` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
  - `src/huffman.rs:302:14: replace > with < in huffman_depths` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
  - `src/huffman.rs:302:14: replace > with == in huffman_depths` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
  - `src/huffman.rs:304:48: replace + with * in huffman_depths` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
- **src/lz77.rs: MatchConfig::for_level** (8)
  - `src/lz77.rs:71:13: delete match arm 0 | 1 in MatchConfig::for_level` — reached by roundtrip_stored, no_chain_means_literals_only, rle_tokens_valid_and_replay, runs_are_matched …
  - `src/lz77.rs:72:13: delete match arm 2 in MatchConfig::for_level` — reached by roundtrip_stored, no_chain_means_literals_only, rle_tokens_valid_and_replay, runs_are_matched …
  - `src/lz77.rs:73:13: delete match arm 3 in MatchConfig::for_level` — reached by roundtrip_stored, no_chain_means_literals_only, rle_tokens_valid_and_replay, runs_are_matched …
  - `src/lz77.rs:74:13: delete match arm 4 in MatchConfig::for_level` — reached by roundtrip_stored, no_chain_means_literals_only, rle_tokens_valid_and_replay, runs_are_matched …
  - `src/lz77.rs:75:13: delete match arm 5 in MatchConfig::for_level` — reached by roundtrip_stored, no_chain_means_literals_only, rle_tokens_valid_and_replay, runs_are_matched …
  - `src/lz77.rs:76:13: delete match arm 6 in MatchConfig::for_level` — reached by roundtrip_stored, no_chain_means_literals_only, rle_tokens_valid_and_replay, runs_are_matched …
  - `src/lz77.rs:77:13: delete match arm 7 in MatchConfig::for_level` — reached by roundtrip_stored, no_chain_means_literals_only, rle_tokens_valid_and_replay, runs_are_matched …
  - `src/lz77.rs:78:13: delete match arm 8 in MatchConfig::for_level` — reached by roundtrip_stored, no_chain_means_literals_only, rle_tokens_valid_and_replay, runs_are_matched …
- **src/lz77.rs: Matcher::longest_match** (7)
  - `src/lz77.rs:229:32: replace - with + in Matcher::longest_match` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:229:32: replace - with / in Matcher::longest_match` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:230:21: replace > with >= in Matcher::longest_match` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:242:26: replace + with * in Matcher::longest_match` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:247:28: replace >= with < in Matcher::longest_match` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:253:30: replace & with | in Matcher::longest_match` — reached by tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:253:30: replace & with ^ in Matcher::longest_match` — reached by tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
- **src/zip.rs: is_leap** (7)
  - `src/zip.rs:168:5: replace is_leap -> bool with true` — reached by civil_date_conversion_roundtrip, dos_datetime_roundtrip, is_valid_is_the_calendar, unix_time_always_valid
  - `src/zip.rs:168:39: replace % with + in is_leap` — reached by civil_date_conversion_roundtrip, dos_datetime_roundtrip, is_valid_is_the_calendar, unix_time_always_valid
  - `src/zip.rs:168:17: replace && with || in is_leap` — reached by civil_date_conversion_roundtrip, dos_datetime_roundtrip, is_valid_is_the_calendar, unix_time_always_valid
  - `src/zip.rs:168:22: replace % with + in is_leap` — reached by civil_date_conversion_roundtrip, dos_datetime_roundtrip, is_valid_is_the_calendar, unix_time_always_valid
  - `src/zip.rs:168:39: replace % with / in is_leap` — reached by civil_date_conversion_roundtrip, dos_datetime_roundtrip, is_valid_is_the_calendar, unix_time_always_valid
  - `src/zip.rs:168:22: replace % with / in is_leap` — reached by civil_date_conversion_roundtrip, dos_datetime_roundtrip, is_valid_is_the_calendar, unix_time_always_valid
  - `src/zip.rs:168:45: replace == with != in is_leap` — reached by civil_date_conversion_roundtrip, dos_datetime_roundtrip, is_valid_is_the_calendar, unix_time_always_valid
- **src/zip.rs: name_is_safe** (7)
  - `src/zip.rs:510:32: replace > with == in name_is_safe` — reached by validate_name_is_exactly_the_rule
  - `src/zip.rs:510:32: replace > with >= in name_is_safe` — reached by validate_name_is_exactly_the_rule
  - `src/zip.rs:525:45: replace + with * in name_is_safe` — reached by validate_name_is_exactly_the_rule
  - `src/zip.rs:525:45: replace + with - in name_is_safe` — reached by validate_name_is_exactly_the_rule
  - `src/zip.rs:526:33: replace == with != in name_is_safe` — reached by validate_name_is_exactly_the_rule
  - `src/zip.rs:526:57: replace == with != in name_is_safe` — reached by validate_name_is_exactly_the_rule
  - `src/zip.rs:526:49: replace || with && in name_is_safe` — reached by validate_name_is_exactly_the_rule
- **src/huffman.rs: take_lightest** (5)
  - `src/huffman.rs:318:29: replace < with == in take_lightest` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
  - `src/huffman.rs:318:29: replace < with <= in take_lightest` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
  - `src/huffman.rs:319:59: replace <= with > in take_lightest` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
  - `src/huffman.rs:324:17: replace - with / in take_lightest` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
  - `src/huffman.rs:324:17: replace - with + in take_lightest` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_tight_limit
- **src/zip.rs: find_eocd** (5)
  - `src/zip.rs:760:19: replace < with <= in find_eocd` — reached by find_eocd_is_sound
  - `src/zip.rs:767:75: replace + with - in find_eocd` — reached by find_eocd_is_sound
  - `src/zip.rs:771:14: replace == with != in find_eocd` — reached by find_eocd_is_sound
  - `src/zip.rs:774:11: replace -= with /= in find_eocd` — reached by find_eocd_is_sound
  - `src/zip.rs:774:11: replace -= with += in find_eocd` — reached by find_eocd_is_sound
- **src/deflate.rs: (outside functions)** (4)
  - `src/deflate.rs:120:26: replace * with +` — reached by roundtrip_stored
  - `src/deflate.rs:120:26: replace * with /` — reached by roundtrip_stored
  - `src/deflate.rs:123:44: replace * with /` — reached by roundtrip_stored
  - `src/deflate.rs:123:44: replace * with +` — reached by roundtrip_stored
- **src/lz77.rs: Matcher::insert_at** (4)
  - `src/lz77.rs:151:27: replace & with ^ in Matcher::insert_at` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:151:27: replace & with | in Matcher::insert_at` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:151:42: replace - with / in Matcher::insert_at` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:151:42: replace - with + in Matcher::insert_at` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
- **src/lz77.rs: tokenize_runs** (4)
  - `src/lz77.rs:357:33: replace == with != in tokenize_runs` — reached by rle_tokens_valid_and_replay
  - `src/lz77.rs:357:16: replace > with < in tokenize_runs` — reached by rle_tokens_valid_and_replay
  - `src/lz77.rs:358:32: replace - with + in tokenize_runs` — reached by rle_tokens_valid_and_replay
  - `src/lz77.rs:358:32: replace - with / in tokenize_runs` — reached by rle_tokens_valid_and_replay
- **src/bitio.rs: BitReader<'a>::refill** (3)
  - `src/bitio.rs:147:39: replace / with % in BitReader<'a>::refill` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, fast_decode_matches_slow
  - `src/bitio.rs:147:25: replace - with + in BitReader<'a>::refill` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, fast_decode_matches_slow
  - `src/bitio.rs:151:27: replace << with >> in BitReader<'a>::refill` — reached by aligned_bytes_after_lookahead_long
- **src/huffman.rs: build_lengths_in** (3)
  - `src/huffman.rs:161:10: replace > with < in build_lengths_in` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_rejects_impossible_limit, build_lengths_tight_limit
  - `src/huffman.rs:167:10: replace < with == in build_lengths_in` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_rejects_impossible_limit, build_lengths_tight_limit
  - `src/huffman.rs:167:10: replace < with <= in build_lengths_in` — reached by build_lengths_complete_and_limited, build_lengths_full_range_frequencies, build_lengths_rejects_impossible_limit, build_lengths_tight_limit
- **src/inflate.rs: Output::reserve** (3)
  - `src/inflate.rs:107:14: replace > with == in Output::reserve` — reached by roundtrip_stored, stored_block_semantics
  - `src/inflate.rs:107:14: replace > with < in Output::reserve` — reached by roundtrip_stored, stored_block_semantics
  - `src/inflate.rs:107:31: replace - with + in Output::reserve` — reached by roundtrip_stored, stored_block_semantics
- **src/lz77.rs: Matcher::hash** (3)
  - `src/lz77.rs:133:9: replace Matcher::hash -> usize with 0` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:133:38: replace | with & in Matcher::hash` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:133:9: replace Matcher::hash -> usize with 1` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
- **src/zip.rs: (outside functions)** (3)
  - `src/zip.rs:23:33: replace << with >>` — reached by central_header_roundtrip, eocd_roundtrip, find_eocd_is_sound, header_parsers_never_panic …
  - `src/zip.rs:23:39: replace | with &` — reached by central_header_roundtrip, eocd_roundtrip, find_eocd_is_sound, header_parsers_never_panic …
  - `src/zip.rs:25:26: replace << with >>` — reached by central_header_roundtrip, eocd_roundtrip, find_eocd_is_sound, header_parsers_never_panic …
- **src/deflate.rs: encode_with_matcher** (2)
  - `src/deflate.rs:88:34: replace == with != in encode_with_matcher` — reached by roundtrip_stored
  - `src/deflate.rs:90:17: replace == with != in encode_with_matcher` — reached by roundtrip_stored
- **src/lz77.rs: Matcher::bucket** (2)
  - `src/lz77.rs:140:9: replace Matcher::bucket -> Option<usize> with Some(0)` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
  - `src/lz77.rs:140:9: replace Matcher::bucket -> Option<usize> with Some(1)` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
- **src/lz77.rs: common_prefix** (2)
  - `src/lz77.rs:392:11: replace += with *= in common_prefix` — reached by common_prefix_matches_reference
  - `src/lz77.rs:392:11: replace += with -= in common_prefix` — reached by common_prefix_matches_reference
- **src/bitio.rs: BitReader<'a>::bytes_consumed** (1)
  - `src/bitio.rs:260:18: replace - with + in BitReader<'a>::bytes_consumed` — reached by aligned_bytes_after_lookahead_long, aligned_bytes_after_lookahead_short, aligned_bytes_roundtrip, roundtrip_stored …
- **src/deflate.rs: write_stored_in** (1)
  - `src/deflate.rs:316:28: replace && with || in write_stored_in` — reached by stored_cost_adds_up_across_blocks, stored_cost_is_exact, roundtrip_stored
- **src/huffman.rs: Canonical::is_complete** (1)
  - `src/huffman.rs:497:9: replace Canonical::is_complete -> bool with false` — reached by decoder_new_never_panics
- **src/huffman.rs: Canonical::rebuild** (1)
  - `src/huffman.rs:460:30: replace - with + in Canonical::rebuild` — reached by decode_arbitrary_bits, decoder_new_never_panics, encode_then_decode_symbol, fast_decode_matches_slow
- **src/huffman.rs: Decoder::is_complete** (1)
  - `src/huffman.rs:643:9: replace Decoder::is_complete -> bool with false` — reached by decoder_new_never_panics
- **src/lz77.rs: Matcher::dist_to** (1)
  - `src/lz77.rs:173:9: replace Matcher::dist_to -> usize with 1` — reached by runs_are_matched, tokens_valid_and_replay_greedy, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
- **src/lz77.rs: Matcher::tokenize** (1)
  - `src/lz77.rs:321:60: replace + with - in Matcher::tokenize` — reached by runs_are_matched, tokens_valid_and_replay_level9, tokens_valid_and_replay_small_tables
- **src/lz77.rs: copy_within_buffer** (1)
  - `src/lz77.rs:437:12: replace <= with > in copy_within_buffer` — reached by copy_within_buffer_meets_spec, short_copy_meets_spec
- **src/zip.rs: days_in_month** (1)
  - `src/zip.rs:175:14: replace match guard is_leap(year) with true in days_in_month` — reached by civil_date_conversion_roundtrip, dos_datetime_roundtrip, is_valid_is_the_calendar, unix_time_always_valid
- **src/zip.rs: record_bounds** (1)
  - `src/zip.rs:796:58: replace + with * in record_bounds` — reached by record_bounds_sound

## Priority 3: no proof executes this code (414)

Only the tests protect these.

- **src/deflate.rs: plan_dynamic** (30)
  - `src/deflate.rs:502:5: replace plan_dynamic -> Option<DynamicPlan> with None`
  - `src/deflate.rs:506:40: replace + with * in plan_dynamic`
  - `src/deflate.rs:506:40: replace + with - in plan_dynamic`
  - `src/deflate.rs:507:41: replace + with - in plan_dynamic`
  - `src/deflate.rs:507:41: replace + with * in plan_dynamic`
  - `src/deflate.rs:508:36: replace + with - in plan_dynamic`
  - `src/deflate.rs:510:20: replace + with * in plan_dynamic`
  - `src/deflate.rs:510:20: replace + with - in plan_dynamic`
  - `src/deflate.rs:512:49: replace + with * in plan_dynamic`
  - `src/deflate.rs:512:49: replace + with - in plan_dynamic`
  - `src/deflate.rs:515:33: replace += with *= in plan_dynamic`
  - `src/deflate.rs:515:33: replace += with -= in plan_dynamic`
  - `src/deflate.rs:519:63: replace != with == in plan_dynamic`
  - `src/deflate.rs:519:85: replace + with * in plan_dynamic`
  - `src/deflate.rs:519:85: replace + with - in plan_dynamic`
  - `src/deflate.rs:522:52: replace + with * in plan_dynamic`
  - `src/deflate.rs:522:52: replace + with - in plan_dynamic`
  - `src/deflate.rs:524:18: replace + with * in plan_dynamic`
  - `src/deflate.rs:524:49: replace + with - in plan_dynamic`
  - `src/deflate.rs:524:30: replace + with - in plan_dynamic`
  - `src/deflate.rs:524:22: replace + with - in plan_dynamic`
  - `src/deflate.rs:524:59: replace + with * in plan_dynamic`
  - `src/deflate.rs:524:26: replace + with - in plan_dynamic`
  - `src/deflate.rs:524:49: replace + with * in plan_dynamic`
  - `src/deflate.rs:524:26: replace + with * in plan_dynamic`
  - `src/deflate.rs:524:22: replace + with * in plan_dynamic`
  - `src/deflate.rs:524:34: replace * with / in plan_dynamic`
  - `src/deflate.rs:524:30: replace + with * in plan_dynamic`
  - `src/deflate.rs:524:59: replace + with - in plan_dynamic`
  - `src/deflate.rs:524:34: replace * with + in plan_dynamic`
- **src/gzip_header.rs: FileSystemType::from_u8** (20)
  - `src/gzip_header.rs:61:13: delete match arm 0 in FileSystemType::from_u8`
  - `src/gzip_header.rs:62:13: delete match arm 1 in FileSystemType::from_u8`
  - `src/gzip_header.rs:63:13: delete match arm 2 in FileSystemType::from_u8`
  - `src/gzip_header.rs:64:13: delete match arm 3 in FileSystemType::from_u8`
  - `src/gzip_header.rs:65:13: delete match arm 4 in FileSystemType::from_u8`
  - `src/gzip_header.rs:66:13: delete match arm 5 in FileSystemType::from_u8`
  - `src/gzip_header.rs:67:13: delete match arm 6 in FileSystemType::from_u8`
  - `src/gzip_header.rs:68:13: delete match arm 7 in FileSystemType::from_u8`
  - `src/gzip_header.rs:69:13: delete match arm 8 in FileSystemType::from_u8`
  - `src/gzip_header.rs:70:13: delete match arm 9 in FileSystemType::from_u8`
  - `src/gzip_header.rs:71:13: delete match arm 10 in FileSystemType::from_u8`
  - `src/gzip_header.rs:72:13: delete match arm 11 in FileSystemType::from_u8`
  - `src/gzip_header.rs:73:13: delete match arm 12 in FileSystemType::from_u8`
  - `src/gzip_header.rs:74:13: delete match arm 13 in FileSystemType::from_u8`
  - `src/gzip_header.rs:75:13: delete match arm 14 in FileSystemType::from_u8`
  - `src/gzip_header.rs:76:13: delete match arm 15 in FileSystemType::from_u8`
  - `src/gzip_header.rs:77:13: delete match arm 16 in FileSystemType::from_u8`
  - `src/gzip_header.rs:78:13: delete match arm 17 in FileSystemType::from_u8`
  - `src/gzip_header.rs:79:13: delete match arm 18 in FileSystemType::from_u8`
  - `src/gzip_header.rs:80:13: delete match arm 19 in FileSystemType::from_u8`
- **src/compat.rs: CompressionOptions::engine** (18)
  - `src/compat.rs:168:43: replace == with != in CompressionOptions::engine`
  - `src/compat.rs:170:53: delete field rle from struct MatchConfig expression in CompressionOptions::engine`
  - `src/compat.rs:171:69: delete field lazy from struct MatchConfig expression in CompressionOptions::engine`
  - `src/compat.rs:171:55: delete field max_chain from struct MatchConfig expression in CompressionOptions::engine`
  - `src/compat.rs:175:43: replace == with != in CompressionOptions::engine`
  - `src/compat.rs:175:91: replace > with < in CompressionOptions::engine`
  - `src/compat.rs:175:65: replace && with || in CompressionOptions::engine`
  - `src/compat.rs:175:91: replace > with >= in CompressionOptions::engine`
  - `src/compat.rs:175:91: replace > with == in CompressionOptions::engine`
  - `src/compat.rs:177:17: delete field max_chain from struct MatchConfig expression in CompressionOptions::engine`
  - `src/compat.rs:178:17: delete field nice_len from struct MatchConfig expression in CompressionOptions::engine`
  - `src/compat.rs:180:17: delete field good_len from struct MatchConfig expression in CompressionOptions::engine`
  - `src/compat.rs:180:37: replace + with * in CompressionOptions::engine`
  - `src/compat.rs:180:37: replace + with - in CompressionOptions::engine`
  - `src/compat.rs:183:87: replace + with - in CompressionOptions::engine`
  - `src/compat.rs:183:17: delete field max_lazy from struct MatchConfig expression in CompressionOptions::engine`
  - `src/compat.rs:183:87: replace + with * in CompressionOptions::engine`
  - `src/compat.rs:184:17: delete field lazy from struct MatchConfig expression in CompressionOptions::engine`
- **src/inflate.rs: read_dynamic_codes** (17)
  - `src/inflate.rs:193:42: replace + with * in read_dynamic_codes`
  - `src/inflate.rs:193:5: replace read_dynamic_codes -> Result<(), InflateError> with Ok(())`
  - `src/inflate.rs:194:43: replace + with * in read_dynamic_codes`
  - `src/inflate.rs:196:35: replace > with >= in read_dynamic_codes`
  - `src/inflate.rs:196:13: replace > with >= in read_dynamic_codes`
  - `src/inflate.rs:205:8: delete ! in read_dynamic_codes`
  - `src/inflate.rs:210:22: replace + with - in read_dynamic_codes`
  - `src/inflate.rs:212:15: replace < with > in read_dynamic_codes`
  - `src/inflate.rs:212:15: replace < with == in read_dynamic_codes`
  - `src/inflate.rs:212:15: replace < with <= in read_dynamic_codes`
  - `src/inflate.rs:217:30: replace == with != in read_dynamic_codes`
  - `src/inflate.rs:222:28: replace || with && in read_dynamic_codes`
  - `src/inflate.rs:222:8: delete ! in read_dynamic_codes`
  - `src/inflate.rs:226:29: replace || with && in read_dynamic_codes`
  - `src/inflate.rs:226:59: replace || with && in read_dynamic_codes`
  - `src/inflate.rs:226:8: delete ! in read_dynamic_codes`
  - `src/inflate.rs:226:83: replace == with != in read_dynamic_codes`
- **src/bin/vzip.rs: take_level** (12)
  - `src/bin/vzip.rs:61:5: replace take_level -> (u8, Vec<String>) with (1, vec![])`
  - `src/bin/vzip.rs:61:5: replace take_level -> (u8, Vec<String>) with (1, vec!["xyzzy".into()])`
  - `src/bin/vzip.rs:61:5: replace take_level -> (u8, Vec<String>) with (0, vec!["xyzzy".into()])`
  - `src/bin/vzip.rs:61:5: replace take_level -> (u8, Vec<String>) with (0, vec![String::new()])`
  - `src/bin/vzip.rs:61:5: replace take_level -> (u8, Vec<String>) with (1, vec![String::new()])`
  - `src/bin/vzip.rs:61:5: replace take_level -> (u8, Vec<String>) with (0, vec![])`
  - `src/bin/vzip.rs:65:41: replace && with || in take_level`
  - `src/bin/vzip.rs:65:20: replace == with != in take_level`
  - `src/bin/vzip.rs:65:25: replace && with || in take_level`
  - `src/bin/vzip.rs:65:33: replace == with != in take_level`
  - `src/bin/vzip.rs:66:26: replace - with + in take_level`
  - `src/bin/vzip.rs:66:26: replace - with / in take_level`
- **src/deflate.rs: StreamEncoder::compress_pending** (11)
  - `src/deflate.rs:180:9: replace StreamEncoder::compress_pending with ()`
  - `src/deflate.rs:181:22: replace == with != in StreamEncoder::compress_pending`
  - `src/deflate.rs:181:29: replace && with || in StreamEncoder::compress_pending`
  - `src/deflate.rs:181:32: delete ! in StreamEncoder::compress_pending`
  - `src/deflate.rs:184:26: replace == with != in StreamEncoder::compress_pending`
  - `src/deflate.rs:192:32: replace >= with < in StreamEncoder::compress_pending`
  - `src/deflate.rs:193:94: replace && with || in StreamEncoder::compress_pending`
  - `src/deflate.rs:204:16: replace > with < in StreamEncoder::compress_pending`
  - `src/deflate.rs:204:16: replace > with == in StreamEncoder::compress_pending`
  - `src/deflate.rs:205:34: replace - with / in StreamEncoder::compress_pending`
  - `src/deflate.rs:205:34: replace - with + in StreamEncoder::compress_pending`
- **src/deflate.rs: write_block** (11)
  - `src/deflate.rs:282:25: replace == with != in write_block`
  - `src/deflate.rs:293:44: replace && with || in write_block`
  - `src/deflate.rs:293:28: replace match guard dynamic < fixed && dynamic < stored with true in write_block`
  - `src/deflate.rs:293:55: replace < with > in write_block`
  - `src/deflate.rs:293:36: replace < with > in write_block`
  - `src/deflate.rs:293:55: replace < with == in write_block`
  - `src/deflate.rs:293:36: replace < with == in write_block`
  - `src/deflate.rs:293:28: replace match guard dynamic < fixed && dynamic < stored with false in write_block`
  - `src/deflate.rs:294:28: replace <= with > in write_block`
  - `src/deflate.rs:294:22: replace match guard fixed <= stored with false in write_block`
  - `src/deflate.rs:294:22: replace match guard fixed <= stored with true in write_block`
- **src/zip.rs: ZipWriter::add_directory** (11)
  - `src/zip.rs:683:9: replace ZipWriter::add_directory -> Result<(), ZipError> with Ok(())`
  - `src/zip.rs:685:35: replace & with ^ in ZipWriter::add_directory`
  - `src/zip.rs:685:35: replace & with | in ZipWriter::add_directory`
  - `src/zip.rs:686:43: replace >> with << in ZipWriter::add_directory`
  - `src/zip.rs:686:34: replace & with ^ in ZipWriter::add_directory`
  - `src/zip.rs:686:25: replace | with ^ in ZipWriter::add_directory`
  - `src/zip.rs:686:34: replace & with | in ZipWriter::add_directory`
  - `src/zip.rs:686:25: replace | with & in ZipWriter::add_directory`
  - `src/zip.rs:688:41: replace | with & in ZipWriter::add_directory`
  - `src/zip.rs:688:49: replace << with >> in ZipWriter::add_directory`
  - `src/zip.rs:688:56: replace | with & in ZipWriter::add_directory`
- **src/deflate.rs: BlockStats::body_bits** (10)
  - `src/deflate.rs:402:81: replace * with + in BlockStats::body_bits`
  - `src/deflate.rs:402:9: replace BlockStats::body_bits -> u64 with 0`
  - `src/deflate.rs:402:9: replace BlockStats::body_bits -> u64 with 1`
  - `src/deflate.rs:402:81: replace * with / in BlockStats::body_bits`
  - `src/deflate.rs:403:84: replace * with / in BlockStats::body_bits`
  - `src/deflate.rs:403:84: replace * with + in BlockStats::body_bits`
  - `src/deflate.rs:404:13: replace + with * in BlockStats::body_bits`
  - `src/deflate.rs:404:13: replace + with - in BlockStats::body_bits`
  - `src/deflate.rs:404:20: replace + with * in BlockStats::body_bits`
  - `src/deflate.rs:404:20: replace + with - in BlockStats::body_bits`
- **src/deflate.rs: StreamEncoder::push** (9)
  - `src/deflate.rs:167:33: replace - with + in StreamEncoder::push`
  - `src/deflate.rs:167:51: replace - with / in StreamEncoder::push`
  - `src/deflate.rs:167:51: replace - with + in StreamEncoder::push`
  - `src/deflate.rs:167:9: replace StreamEncoder::push -> usize with 1`
  - `src/deflate.rs:167:9: replace StreamEncoder::push -> usize with 0`
  - `src/deflate.rs:167:33: replace - with / in StreamEncoder::push`
  - `src/deflate.rs:170:27: replace - with + in StreamEncoder::push`
  - `src/deflate.rs:170:39: replace == with != in StreamEncoder::push`
  - `src/deflate.rs:170:27: replace - with / in StreamEncoder::push`
- **src/deflate.rs: write_dynamic** (9)
  - `src/deflate.rs:539:34: replace | with & in write_dynamic`
  - `src/deflate.rs:539:39: replace << with >> in write_dynamic`
  - `src/deflate.rs:539:5: replace write_dynamic with ()`
  - `src/deflate.rs:540:26: replace - with + in write_dynamic`
  - `src/deflate.rs:540:26: replace - with / in write_dynamic`
  - `src/deflate.rs:541:27: replace - with + in write_dynamic`
  - `src/deflate.rs:541:27: replace - with / in write_dynamic`
  - `src/deflate.rs:542:27: replace - with + in write_dynamic`
  - `src/deflate.rs:542:27: replace - with / in write_dynamic`
- **src/write.rs: Core<W>::send** (9)
  - `src/write.rs:57:9: replace Core<W>::send -> io::Result<()> with Ok(())`
  - `src/write.rs:58:25: replace < with <= in Core<W>::send`
  - `src/write.rs:58:25: replace < with == in Core<W>::send`
  - `src/write.rs:58:25: replace < with > in Core<W>::send`
  - `src/write.rs:61:36: replace += with -= in Core<W>::send`
  - `src/write.rs:61:36: replace += with *= in Core<W>::send`
  - `src/write.rs:62:27: replace match guard e.kind() == io::ErrorKind::Interrupted with false in Core<W>::send`
  - `src/write.rs:62:27: replace match guard e.kind() == io::ErrorKind::Interrupted with true in Core<W>::send`
  - `src/write.rs:62:36: replace == with != in Core<W>::send`
- **src/zip.rs: ZipArchive<'a>::parse** (9)
  - `src/zip.rs:808:27: replace || with && in ZipArchive<'a>::parse`
  - `src/zip.rs:808:48: replace || with && in ZipArchive<'a>::parse`
  - `src/zip.rs:811:69: replace || with && in ZipArchive<'a>::parse`
  - `src/zip.rs:811:41: replace || with && in ZipArchive<'a>::parse`
  - `src/zip.rs:825:46: replace || with && in ZipArchive<'a>::parse`
  - `src/zip.rs:825:81: replace || with && in ZipArchive<'a>::parse`
  - `src/zip.rs:838:49: replace >> with << in ZipArchive<'a>::parse`
  - `src/zip.rs:838:54: replace == with != in ZipArchive<'a>::parse`
  - `src/zip.rs:838:82: replace >> with << in ZipArchive<'a>::parse`
- **src/bin/vzip.rs: create** (8)
  - `src/bin/vzip.rs:75:5: replace create -> Result<()> with Ok(())`
  - `src/bin/vzip.rs:77:56: replace != with == in create`
  - `src/bin/vzip.rs:83:30: delete field level from struct FileOptions expression in create`
  - `src/bin/vzip.rs:90:74: replace / with % in create`
  - `src/bin/vzip.rs:90:66: replace * with / in create`
  - `src/bin/vzip.rs:90:66: replace * with + in create`
  - `src/bin/vzip.rs:90:27: replace == with != in create`
  - `src/bin/vzip.rs:90:74: replace / with * in create`
- **src/zip.rs: ZipWriter::add_file** (8)
  - `src/zip.rs:651:40: replace == with != in ZipWriter::add_file`
  - `src/zip.rs:658:24: replace < with == in ZipWriter::add_file`
  - `src/zip.rs:658:24: replace < with > in ZipWriter::add_file`
  - `src/zip.rs:669:57: replace & with ^ in ZipWriter::add_file`
  - `src/zip.rs:669:39: replace | with & in ZipWriter::add_file`
  - `src/zip.rs:669:57: replace & with | in ZipWriter::add_file`
  - `src/zip.rs:669:68: replace << with >> in ZipWriter::add_file`
  - `src/zip.rs:672:61: delete field method from struct EntryMeta expression in ZipWriter::add_file`
- **src/bin/vzip.rs: extract** (7)
  - `src/bin/vzip.rs:161:5: replace extract -> Result<()> with Ok(())`
  - `src/bin/vzip.rs:167:13: delete match arm "-d" in extract`
  - `src/bin/vzip.rs:168:13: delete match arm "-o" in extract`
  - `src/bin/vzip.rs:185:23: replace && with || in extract`
  - `src/bin/vzip.rs:185:12: delete ! in extract`
  - `src/bin/vzip.rs:193:82: replace & with | in extract`
  - `src/bin/vzip.rs:193:82: replace & with ^ in extract`
- **src/bitio.rs: BitReader<'a>::refill** (7)
  - `src/bitio.rs:150:67: replace * with + in BitReader<'a>::refill`
  - `src/bitio.rs:150:49: replace & with ^ in BitReader<'a>::refill`
  - `src/bitio.rs:150:49: replace & with | in BitReader<'a>::refill`
  - `src/bitio.rs:150:58: replace << with >> in BitReader<'a>::refill`
  - `src/bitio.rs:150:73: replace - with / in BitReader<'a>::refill`
  - `src/bitio.rs:150:67: replace * with / in BitReader<'a>::refill`
  - `src/bitio.rs:150:73: replace - with + in BitReader<'a>::refill`
- **src/gzip_header.rs: GzBuilder::into_header_inner** (7)
  - `src/gzip_header.rs:239:9: replace GzBuilder::into_header_inner -> Vec<u8> with vec![0]`
  - `src/gzip_header.rs:239:9: replace GzBuilder::into_header_inner -> Vec<u8> with vec![]`
  - `src/gzip_header.rs:239:9: replace GzBuilder::into_header_inner -> Vec<u8> with vec![1]`
  - `src/gzip_header.rs:243:17: replace |= with &= in GzBuilder::into_header_inner`
  - `src/gzip_header.rs:248:17: replace |= with &= in GzBuilder::into_header_inner`
  - `src/gzip_header.rs:253:17: replace |= with &= in GzBuilder::into_header_inner`
  - `src/gzip_header.rs:258:17: replace |= with &= in GzBuilder::into_header_inner`
- **src/huffman.rs: build_lengths** (6)
  - `src/huffman.rs:141:20: replace <= with > in build_lengths`
  - `src/huffman.rs:141:5: replace build_lengths -> Result<(), HuffmanError> with Ok(())`
  - `src/huffman.rs:142:48: replace * with + in build_lengths`
  - `src/huffman.rs:142:48: replace * with / in build_lengths`
  - `src/huffman.rs:144:45: replace * with / in build_lengths`
  - `src/huffman.rs:144:45: replace * with + in build_lengths`
- **src/bin/vzip.rs: add_path** (5)
  - `src/bin/vzip.rs:130:5: replace add_path -> Result<()> with Ok(())`
  - `src/bin/vzip.rs:149:16: replace += with *= in add_path`
  - `src/bin/vzip.rs:149:16: replace += with -= in add_path`
  - `src/bin/vzip.rs:150:16: replace += with *= in add_path`
  - `src/bin/vzip.rs:150:16: replace += with -= in add_path`
- **src/bin/vzip.rs: list** (5)
  - `src/bin/vzip.rs:201:5: replace list -> Result<()> with Ok(())`
  - `src/bin/vzip.rs:226:15: replace += with *= in list`
  - `src/bin/vzip.rs:226:15: replace += with -= in list`
  - `src/bin/vzip.rs:227:16: replace += with -= in list`
  - `src/bin/vzip.rs:227:16: replace += with *= in list`
- **src/bitio.rs: BitWriter::is_byte_aligned** (5)
  - `src/bitio.rs:59:24: replace == with != in BitWriter::is_byte_aligned`
  - `src/bitio.rs:59:9: replace BitWriter::is_byte_aligned -> bool with false`
  - `src/bitio.rs:59:9: replace BitWriter::is_byte_aligned -> bool with true`
  - `src/bitio.rs:59:20: replace % with + in BitWriter::is_byte_aligned`
  - `src/bitio.rs:59:20: replace % with / in BitWriter::is_byte_aligned`
- **src/huffman.rs: Canonical::is_single_unit_code** (5)
  - `src/huffman.rs:508:25: replace && with || in Canonical::is_single_unit_code`
  - `src/huffman.rs:508:42: replace == with != in Canonical::is_single_unit_code`
  - `src/huffman.rs:508:9: replace Canonical::is_single_unit_code -> bool with true`
  - `src/huffman.rs:508:9: replace Canonical::is_single_unit_code -> bool with false`
  - `src/huffman.rs:508:20: replace == with != in Canonical::is_single_unit_code`
- **src/bin/vzip.rs: options_for** (4)
  - `src/bin/vzip.rs:113:5: replace options_for -> FileOptions with Default::default()`
  - `src/bin/vzip.rs:117:23: delete - in options_for`
  - `src/bin/vzip.rs:124:49: replace & with ^ in options_for`
  - `src/bin/vzip.rs:124:49: replace & with | in options_for`
- **src/compat.rs: deflate_bytes_gzip_conf** (4)
  - `src/compat.rs:298:5: replace deflate_bytes_gzip_conf -> Vec<u8> with vec![]`
  - `src/compat.rs:298:5: replace deflate_bytes_gzip_conf -> Vec<u8> with vec![1]`
  - `src/compat.rs:298:5: replace deflate_bytes_gzip_conf -> Vec<u8> with vec![0]`
  - `src/compat.rs:299:33: replace + with - in deflate_bytes_gzip_conf`
- **src/deflate.rs: BlockStats::fixed_bits** (4)
  - `src/deflate.rs:408:11: replace + with - in BlockStats::fixed_bits`
  - `src/deflate.rs:408:9: replace BlockStats::fixed_bits -> u64 with 1`
  - `src/deflate.rs:408:11: replace + with * in BlockStats::fixed_bits`
  - `src/deflate.rs:408:9: replace BlockStats::fixed_bits -> u64 with 0`
- **src/deflate.rs: compress** (4)
  - `src/deflate.rs:63:5: replace compress -> Vec<u8> with vec![]`
  - `src/deflate.rs:63:5: replace compress -> Vec<u8> with vec![1]`
  - `src/deflate.rs:63:44: delete field level from struct CompressOptions expression in compress`
  - `src/deflate.rs:63:5: replace compress -> Vec<u8> with vec![0]`
- **src/zip.rs: ZipArchive<'a>::index_of** (4)
  - `src/zip.rs:863:9: replace ZipArchive<'a>::index_of -> Option<usize> with Some(0)`
  - `src/zip.rs:863:53: replace == with != in ZipArchive<'a>::index_of`
  - `src/zip.rs:863:9: replace ZipArchive<'a>::index_of -> Option<usize> with None`
  - `src/zip.rs:863:9: replace ZipArchive<'a>::index_of -> Option<usize> with Some(1)`
- **src/bin/vzip.rs: open** (3)
  - `src/bin/vzip.rs:156:5: replace open -> Result<Vec<u8>> with Ok(vec![0])`
  - `src/bin/vzip.rs:156:5: replace open -> Result<Vec<u8>> with Ok(vec![])`
  - `src/bin/vzip.rs:156:5: replace open -> Result<Vec<u8>> with Ok(vec![1])`
- **src/bin/vzip.rs: raw_deflate** (3)
  - `src/bin/vzip.rs:261:5: replace raw_deflate -> Result<()> with Ok(())`
  - `src/bin/vzip.rs:262:8: delete ! in raw_deflate`
  - `src/bin/vzip.rs:266:56: delete field level from struct CompressOptions expression in raw_deflate`
- **src/bin/vzip.rs: read_stdin** (3)
  - `src/bin/vzip.rs:255:5: replace read_stdin -> Result<Vec<u8>> with Ok(vec![0])`
  - `src/bin/vzip.rs:255:5: replace read_stdin -> Result<Vec<u8>> with Ok(vec![])`
  - `src/bin/vzip.rs:255:5: replace read_stdin -> Result<Vec<u8>> with Ok(vec![1])`
- **src/bin/vzip.rs: test** (3)
  - `src/bin/vzip.rs:234:5: replace test -> Result<()> with Ok(())`
  - `src/bin/vzip.rs:242:21: replace += with *= in test`
  - `src/bin/vzip.rs:246:12: replace == with != in test`
- **src/bitio.rs: BitWriter::take_bytes** (3)
  - `src/bitio.rs:84:9: replace BitWriter::take_bytes -> Vec<u8> with vec![0]`
  - `src/bitio.rs:84:9: replace BitWriter::take_bytes -> Vec<u8> with vec![]`
  - `src/bitio.rs:84:9: replace BitWriter::take_bytes -> Vec<u8> with vec![1]`
- **src/compat.rs: deflate_bytes** (3)
  - `src/compat.rs:246:5: replace deflate_bytes -> Vec<u8> with vec![1]`
  - `src/compat.rs:246:5: replace deflate_bytes -> Vec<u8> with vec![]`
  - `src/compat.rs:246:5: replace deflate_bytes -> Vec<u8> with vec![0]`
- **src/compat.rs: deflate_bytes_conf** (3)
  - `src/compat.rs:231:5: replace deflate_bytes_conf -> Vec<u8> with vec![0]`
  - `src/compat.rs:231:5: replace deflate_bytes_conf -> Vec<u8> with vec![]`
  - `src/compat.rs:231:5: replace deflate_bytes_conf -> Vec<u8> with vec![1]`
- **src/compat.rs: deflate_bytes_gzip** (3)
  - `src/compat.rs:317:5: replace deflate_bytes_gzip -> Vec<u8> with vec![1]`
  - `src/compat.rs:317:5: replace deflate_bytes_gzip -> Vec<u8> with vec![]`
  - `src/compat.rs:317:5: replace deflate_bytes_gzip -> Vec<u8> with vec![0]`
- **src/compat.rs: deflate_bytes_zlib** (3)
  - `src/compat.rs:279:5: replace deflate_bytes_zlib -> Vec<u8> with vec![1]`
  - `src/compat.rs:279:5: replace deflate_bytes_zlib -> Vec<u8> with vec![0]`
  - `src/compat.rs:279:5: replace deflate_bytes_zlib -> Vec<u8> with vec![]`
- **src/compat.rs: deflate_bytes_zlib_conf** (3)
  - `src/compat.rs:261:5: replace deflate_bytes_zlib_conf -> Vec<u8> with vec![1]`
  - `src/compat.rs:261:5: replace deflate_bytes_zlib_conf -> Vec<u8> with vec![0]`
  - `src/compat.rs:261:5: replace deflate_bytes_zlib_conf -> Vec<u8> with vec![]`
- **src/deflate.rs: StreamEncoder::take_output** (3)
  - `src/deflate.rs:225:9: replace StreamEncoder::take_output -> Vec<u8> with vec![]`
  - `src/deflate.rs:225:9: replace StreamEncoder::take_output -> Vec<u8> with vec![1]`
  - `src/deflate.rs:225:9: replace StreamEncoder::take_output -> Vec<u8> with vec![0]`
- **src/deflate.rs: last_nonzero** (3)
  - `src/deflate.rs:498:5: replace last_nonzero -> usize with 0`
  - `src/deflate.rs:498:31: replace != with == in last_nonzero`
  - `src/deflate.rs:498:5: replace last_nonzero -> usize with 1`
- **src/gzip_header.rs: (outside functions)** (3)
  - `src/gzip_header.rs:11:22: replace << with >>`
  - `src/gzip_header.rs:12:21: replace << with >>`
  - `src/gzip_header.rs:13:24: replace << with >>`
- **src/gzip_header.rs: ExtraFlags::from_u8** (3)
  - `src/gzip_header.rs:132:9: replace ExtraFlags::from_u8 -> ExtraFlags with Default::default()`
  - `src/gzip_header.rs:133:13: delete match arm 2 in ExtraFlags::from_u8`
  - `src/gzip_header.rs:134:13: delete match arm 4 in ExtraFlags::from_u8`
- **src/gzip_header.rs: GzBuilder::into_header** (3)
  - `src/gzip_header.rs:230:9: replace GzBuilder::into_header -> Vec<u8> with vec![]`
  - `src/gzip_header.rs:230:9: replace GzBuilder::into_header -> Vec<u8> with vec![1]`
  - `src/gzip_header.rs:230:9: replace GzBuilder::into_header -> Vec<u8> with vec![0]`
- **src/gzip_header.rs: GzBuilder::into_header_with_checksum** (3)
  - `src/gzip_header.rs:235:9: replace GzBuilder::into_header_with_checksum -> Vec<u8> with vec![0]`
  - `src/gzip_header.rs:235:9: replace GzBuilder::into_header_with_checksum -> Vec<u8> with vec![]`
  - `src/gzip_header.rs:235:9: replace GzBuilder::into_header_with_checksum -> Vec<u8> with vec![1]`
- **src/gzip_header.rs: GzBuilder::into_header_xfl** (3)
  - `src/gzip_header.rs:224:9: replace GzBuilder::into_header_xfl -> Vec<u8> with vec![0]`
  - `src/gzip_header.rs:224:9: replace GzBuilder::into_header_xfl -> Vec<u8> with vec![]`
  - `src/gzip_header.rs:224:9: replace GzBuilder::into_header_xfl -> Vec<u8> with vec![1]`
- **src/gzip_header.rs: without_nul** (3)
  - `src/gzip_header.rs:277:5: replace without_nul -> Vec<u8> with vec![0]`
  - `src/gzip_header.rs:277:5: replace without_nul -> Vec<u8> with vec![]`
  - `src/gzip_header.rs:277:5: replace without_nul -> Vec<u8> with vec![1]`
- **src/inflate.rs: Output::grow** (3)
  - `src/inflate.rs:118:9: replace Output::grow with ()`
  - `src/inflate.rs:118:30: replace + with * in Output::grow`
  - `src/inflate.rs:118:30: replace + with - in Output::grow`
- **src/inflate.rs: decompress** (3)
  - `src/inflate.rs:34:5: replace decompress -> Result<Vec<u8>, InflateError> with Ok(vec![])`
  - `src/inflate.rs:34:5: replace decompress -> Result<Vec<u8>, InflateError> with Ok(vec![1])`
  - `src/inflate.rs:34:5: replace decompress -> Result<Vec<u8>, InflateError> with Ok(vec![0])`
- **src/lz77.rs: Matcher::prime** (3)
  - `src/lz77.rs:122:9: replace Matcher::prime with ()`
  - `src/lz77.rs:125:26: replace < with > in Matcher::prime`
  - `src/lz77.rs:125:26: replace < with == in Matcher::prime`
- **src/write.rs: <impl Drop for DeflateEncoder<W>>::drop** (3)
  - `src/write.rs:173:38: replace && with || in <impl Drop for DeflateEncoder<W>>::drop`
  - `src/write.rs:173:9: replace <impl Drop for DeflateEncoder<W>>::drop with ()`
  - `src/write.rs:173:41: delete ! in <impl Drop for DeflateEncoder<W>>::drop`
- **src/write.rs: <impl Drop for GzEncoder<W>>::drop** (3)
  - `src/write.rs:343:41: delete ! in <impl Drop for GzEncoder<W>>::drop`
  - `src/write.rs:343:9: replace <impl Drop for GzEncoder<W>>::drop with ()`
  - `src/write.rs:343:38: replace && with || in <impl Drop for GzEncoder<W>>::drop`
- **src/write.rs: <impl Drop for ZlibEncoder<W>>::drop** (3)
  - `src/write.rs:246:38: replace && with || in <impl Drop for ZlibEncoder<W>>::drop`
  - `src/write.rs:246:41: delete ! in <impl Drop for ZlibEncoder<W>>::drop`
  - `src/write.rs:246:9: replace <impl Drop for ZlibEncoder<W>>::drop with ()`
- **src/zip.rs: ZipArchive<'a>::read_by_name** (3)
  - `src/zip.rs:906:9: replace ZipArchive<'a>::read_by_name -> Result<Vec<u8>, ZipError> with Ok(vec![1])`
  - `src/zip.rs:906:9: replace ZipArchive<'a>::read_by_name -> Result<Vec<u8>, ZipError> with Ok(vec![0])`
  - `src/zip.rs:906:9: replace ZipArchive<'a>::read_by_name -> Result<Vec<u8>, ZipError> with Ok(vec![])`
- **src/zip.rs: ZipEntry::is_dir** (3)
  - `src/zip.rs:745:9: replace ZipEntry::is_dir -> bool with true`
  - `src/zip.rs:745:30: replace == with != in ZipEntry::is_dir`
  - `src/zip.rs:745:9: replace ZipEntry::is_dir -> bool with false`
- **src/adler32.rs: adler32** (2)
  - `src/adler32.rs:28:5: replace adler32 -> u32 with 1`
  - `src/adler32.rs:28:5: replace adler32 -> u32 with 0`
- **src/bin/vzip.rs: archive_name** (2)
  - `src/bin/vzip.rs:98:5: replace archive_name -> Result<String> with Ok("xyzzy".into())`
  - `src/bin/vzip.rs:98:5: replace archive_name -> Result<String> with Ok(String::new())`
- **src/bin/vzip.rs: raw_inflate** (2)
  - `src/bin/vzip.rs:271:8: delete ! in raw_inflate`
  - `src/bin/vzip.rs:271:5: replace raw_inflate -> Result<()> with Ok(())`
- **src/compat.rs: (outside functions)** (2)
  - `src/compat.rs:31:33: replace * with +`
  - `src/compat.rs:31:33: replace * with /`
- **src/crc32.rs: crc32** (2)
  - `src/crc32.rs:124:5: replace crc32 -> u32 with 0`
  - `src/crc32.rs:124:5: replace crc32 -> u32 with 1`
- **src/deflate.rs: encode_with_matcher** (2)
  - `src/deflate.rs:96:19: replace < with > in encode_with_matcher`
  - `src/deflate.rs:108:24: replace >= with < in encode_with_matcher`
- **src/gzip_header.rs: ExtraFlags::as_u8** (2)
  - `src/gzip_header.rs:141:9: replace ExtraFlags::as_u8 -> u8 with 0`
  - `src/gzip_header.rs:141:9: replace ExtraFlags::as_u8 -> u8 with 1`
- **src/gzip_header.rs: FileSystemType::as_u8** (2)
  - `src/gzip_header.rs:54:9: replace FileSystemType::as_u8 -> u8 with 1`
  - `src/gzip_header.rs:54:9: replace FileSystemType::as_u8 -> u8 with 0`
- **src/huffman.rs: Canonical::coded_symbols** (2)
  - `src/huffman.rs:502:9: replace Canonical::coded_symbols -> usize with 0`
  - `src/huffman.rs:502:9: replace Canonical::coded_symbols -> usize with 1`
- **src/huffman.rs: Decoder::coded_symbols** (2)
  - `src/huffman.rs:648:9: replace Decoder::coded_symbols -> usize with 0`
  - `src/huffman.rs:648:9: replace Decoder::coded_symbols -> usize with 1`
- **src/huffman.rs: Decoder::is_single_unit_code** (2)
  - `src/huffman.rs:654:9: replace Decoder::is_single_unit_code -> bool with true`
  - `src/huffman.rs:654:9: replace Decoder::is_single_unit_code -> bool with false`
- **src/lz77.rs: Matcher::tokenize** (2)
  - `src/lz77.rs:324:29: replace += with *= in Matcher::tokenize`
  - `src/lz77.rs:324:29: replace += with -= in Matcher::tokenize`
- **src/write.rs: <impl Write for DeflateEncoder<W>>::write** (2)
  - `src/write.rs:160:9: replace <impl Write for DeflateEncoder<W>>::write -> io::Result<usize> with Ok(1)`
  - `src/write.rs:160:9: replace <impl Write for DeflateEncoder<W>>::write -> io::Result<usize> with Ok(0)`
- **src/write.rs: <impl Write for GzEncoder<W>>::write** (2)
  - `src/write.rs:327:9: replace <impl Write for GzEncoder<W>>::write -> io::Result<usize> with Ok(1)`
  - `src/write.rs:327:9: replace <impl Write for GzEncoder<W>>::write -> io::Result<usize> with Ok(0)`
- **src/write.rs: <impl Write for ZlibEncoder<W>>::write** (2)
  - `src/write.rs:231:9: replace <impl Write for ZlibEncoder<W>>::write -> io::Result<usize> with Ok(0)`
  - `src/write.rs:231:9: replace <impl Write for ZlibEncoder<W>>::write -> io::Result<usize> with Ok(1)`
- **src/write.rs: Core<W>::collect** (2)
  - `src/write.rs:46:9: replace Core<W>::collect with ()`
  - `src/write.rs:47:22: replace == with != in Core<W>::collect`
- **src/write.rs: Core<W>::finish_with** (2)
  - `src/write.rs:101:9: replace Core<W>::finish_with -> io::Result<()> with Ok(())`
  - `src/write.rs:101:12: delete ! in Core<W>::finish_with`
- **src/write.rs: Core<W>::sync_flush** (2)
  - `src/write.rs:90:12: delete ! in Core<W>::sync_flush`
  - `src/write.rs:90:9: replace Core<W>::sync_flush -> io::Result<()> with Ok(())`
- **src/write.rs: Core<W>::write** (2)
  - `src/write.rs:74:9: replace Core<W>::write -> io::Result<usize> with Ok(1)`
  - `src/write.rs:74:9: replace Core<W>::write -> io::Result<usize> with Ok(0)`
- **src/write.rs: GzEncoder<W>::checksum** (2)
  - `src/write.rs:321:9: replace GzEncoder<W>::checksum -> u32 with 1`
  - `src/write.rs:321:9: replace GzEncoder<W>::checksum -> u32 with 0`
- **src/write.rs: ZlibEncoder<W>::checksum** (2)
  - `src/write.rs:225:9: replace ZlibEncoder<W>::checksum -> u32 with 1`
  - `src/write.rs:225:9: replace ZlibEncoder<W>::checksum -> u32 with 0`
- **src/zip.rs: <impl std::error::Error for ZipError>::source** (2)
  - `src/zip.rs:94:9: replace <impl std::error::Error for ZipError>::source -> Option<&(dyn std::error::Error +'static)> with None`
  - `src/zip.rs:95:13: delete match arm Self::Inflate(e) in <impl std::error::Error for ZipError>::source`
- **src/zip.rs: ZipArchive<'a>::is_empty** (2)
  - `src/zip.rs:858:9: replace ZipArchive<'a>::is_empty -> bool with false`
  - `src/zip.rs:858:9: replace ZipArchive<'a>::is_empty -> bool with true`
- **src/bin/vzip.rs: main** (1)
  - `src/bin/vzip.rs:32:5: replace main -> ExitCode with Default::default()`
- **src/compat.rs: <impl From<Compression> for CompressionOptions>::from** (1)
  - `src/compat.rs:206:9: replace <impl From<Compression> for CompressionOptions>::from -> CompressionOptions with Default::default()`
- **src/compat.rs: <impl fmt::Display for MatchingType>::fmt** (1)
  - `src/compat.rs:62:9: replace <impl fmt::Display for MatchingType>::fmt -> fmt::Result with Ok(Default::default())`
- **src/compat.rs: deflate_into** (1)
  - `src/compat.rs:216:5: replace deflate_into with ()`
- **src/deflate.rs: StreamEncoder::finish** (1)
  - `src/deflate.rs:220:9: replace StreamEncoder::finish with ()`
- **src/deflate.rs: StreamEncoder::reset** (1)
  - `src/deflate.rs:158:9: replace StreamEncoder::reset with ()`
- **src/deflate.rs: StreamEncoder::sync_flush** (1)
  - `src/deflate.rs:214:9: replace StreamEncoder::sync_flush with ()`
- **src/deflate.rs: compress_with** (1)
  - `src/deflate.rs:71:12: delete ! in compress_with`
- **src/error.rs: <impl fmt::Display for InflateError>::fmt** (1)
  - `src/error.rs:42:9: replace <impl fmt::Display for InflateError>::fmt -> fmt::Result with Ok(Default::default())`
- **src/gzip_header.rs: <impl fmt::Display for ExtraFlags>::fmt** (1)
  - `src/gzip_header.rs:147:9: replace <impl fmt::Display for ExtraFlags>::fmt -> fmt::Result with Ok(Default::default())`
- **src/gzip_header.rs: <impl fmt::Display for FileSystemType>::fmt** (1)
  - `src/gzip_header.rs:88:9: replace <impl fmt::Display for FileSystemType>::fmt -> fmt::Result with Ok(Default::default())`
- **src/gzip_header.rs: GzBuilder::comment** (1)
  - `src/gzip_header.rs:204:9: replace GzBuilder::comment -> GzBuilder with Default::default()`
- **src/gzip_header.rs: GzBuilder::extra** (1)
  - `src/gzip_header.rs:186:9: replace GzBuilder::extra -> GzBuilder with Default::default()`
- **src/gzip_header.rs: GzBuilder::filename** (1)
  - `src/gzip_header.rs:195:9: replace GzBuilder::filename -> GzBuilder with Default::default()`
- **src/gzip_header.rs: GzBuilder::mtime** (1)
  - `src/gzip_header.rs:176:9: replace GzBuilder::mtime -> GzBuilder with Default::default()`
- **src/gzip_header.rs: GzBuilder::os** (1)
  - `src/gzip_header.rs:212:9: replace GzBuilder::os -> GzBuilder with Default::default()`
- **src/gzip_header.rs: GzBuilder::xfl** (1)
  - `src/gzip_header.rs:218:9: replace GzBuilder::xfl -> GzBuilder with Default::default()`
- **src/gzip_header.rs: default_os** (1)
  - `src/gzip_header.rs:285:9: delete match arm "linux" | "freebsd" | "dragonfly" | "netbsd" | "openbsd" | "solaris" | "bitrig" in default_os`
- **src/inflate.rs: codes** (1)
  - `src/inflate.rs:175:44: replace + with - in codes`
- **src/inflate.rs: decompress_with_limit** (1)
  - `src/inflate.rs:53:13: delete match arm 2 in decompress_with_limit`
- **src/write.rs: <impl Write for DeflateEncoder<W>>::flush** (1)
  - `src/write.rs:166:9: replace <impl Write for DeflateEncoder<W>>::flush -> io::Result<()> with Ok(())`
- **src/write.rs: <impl Write for GzEncoder<W>>::flush** (1)
  - `src/write.rs:336:9: replace <impl Write for GzEncoder<W>>::flush -> io::Result<()> with Ok(())`
- **src/write.rs: <impl Write for ZlibEncoder<W>>::flush** (1)
  - `src/write.rs:239:9: replace <impl Write for ZlibEncoder<W>>::flush -> io::Result<()> with Ok(())`
- **src/write.rs: GzEncoder<W>::output_all** (1)
  - `src/write.rs:285:9: replace GzEncoder<W>::output_all -> io::Result<()> with Ok(())`
- **src/write.rs: ZlibEncoder<W>::output_all** (1)
  - `src/write.rs:205:9: replace ZlibEncoder<W>::output_all -> io::Result<()> with Ok(())`
- **src/zip.rs: <impl fmt::Display for ZipError>::fmt** (1)
  - `src/zip.rs:72:9: replace <impl fmt::Display for ZipError>::fmt -> fmt::Result with Ok(Default::default())`
- **src/zip.rs: ZipArchive<'a>::entries** (1)
  - `src/zip.rs:848:9: replace ZipArchive<'a>::entries -> &[ZipEntry] with Vec::leak(Vec::new())`
- **src/zip.rs: ZipArchive<'a>::len** (1)
  - `src/zip.rs:853:9: replace ZipArchive<'a>::len -> usize with 1`
- **src/zip.rs: ZipArchive<'a>::raw_data** (1)
  - `src/zip.rs:871:68: replace + with - in ZipArchive<'a>::raw_data`
- **src/zip.rs: ZipWriter::check_new_name** (1)
  - `src/zip.rs:585:9: replace ZipWriter::check_new_name -> Result<(), ZipError> with Ok(())`
- **src/zip.rs: validate_name** (1)
  - `src/zip.rs:500:5: replace validate_name -> Result<(), ZipError> with Ok(())`

## Functions no proof enters

- src/adler32.rs: adler32 (2 mutants)
- src/bin/vzip.rs: add_path (5 mutants)
- src/bin/vzip.rs: archive_name (2 mutants)
- src/bin/vzip.rs: create (11 mutants)
- src/bin/vzip.rs: extract (7 mutants)
- src/bin/vzip.rs: list (5 mutants)
- src/bin/vzip.rs: main (1 mutants)
- src/bin/vzip.rs: open (3 mutants)
- src/bin/vzip.rs: options_for (4 mutants)
- src/bin/vzip.rs: raw_deflate (3 mutants)
- src/bin/vzip.rs: raw_inflate (2 mutants)
- src/bin/vzip.rs: read_stdin (3 mutants)
- src/bin/vzip.rs: take_level (12 mutants)
- src/bin/vzip.rs: test (4 mutants)
- src/bitio.rs: BitWriter::is_byte_aligned (5 mutants)
- src/bitio.rs: BitWriter::take_bytes (3 mutants)
- src/compat.rs: <impl From<Compression> for CompressionOptions>::from (1 mutants)
- src/compat.rs: <impl fmt::Display for MatchingType>::fmt (1 mutants)
- src/compat.rs: CompressionOptions::engine (20 mutants)
- src/compat.rs: CompressionOptions::fast (1 mutants)
- src/compat.rs: CompressionOptions::high (1 mutants)
- src/compat.rs: CompressionOptions::huffman_only (1 mutants)
- src/compat.rs: CompressionOptions::rle (1 mutants)
- src/compat.rs: deflate_bytes (3 mutants)
- src/compat.rs: deflate_bytes_conf (3 mutants)
- src/compat.rs: deflate_bytes_gzip (3 mutants)
- src/compat.rs: deflate_bytes_gzip_conf (7 mutants)
- src/compat.rs: deflate_bytes_zlib (3 mutants)
- src/compat.rs: deflate_bytes_zlib_conf (3 mutants)
- src/compat.rs: deflate_into (1 mutants)
- src/crc32.rs: byte_bitwise (16 mutants)
- src/crc32.rs: crc32 (2 mutants)
- src/crc32.rs: make_tables (24 mutants)
- src/deflate.rs: BlockStats::body_bits (10 mutants)
- src/deflate.rs: BlockStats::fixed_bits (4 mutants)
- src/deflate.rs: CodeTable<N>::from_lengths (6 mutants)
- src/deflate.rs: StreamEncoder::compress_pending (12 mutants)
- src/deflate.rs: StreamEncoder::finish (1 mutants)
- src/deflate.rs: StreamEncoder::push (9 mutants)
- src/deflate.rs: StreamEncoder::reset (1 mutants)
- src/deflate.rs: StreamEncoder::sync_flush (1 mutants)
- src/deflate.rs: StreamEncoder::take_output (3 mutants)
- src/deflate.rs: compress (4 mutants)
- src/deflate.rs: last_nonzero (3 mutants)
- src/deflate.rs: plan_dynamic (35 mutants)
- src/deflate.rs: write_block (14 mutants)
- src/deflate.rs: write_dynamic (10 mutants)
- src/deflate.rs: write_fixed (4 mutants)
- src/error.rs: <impl fmt::Display for InflateError>::fmt (1 mutants)
- src/gzip_header.rs: <impl fmt::Display for ExtraFlags>::fmt (1 mutants)
- src/gzip_header.rs: <impl fmt::Display for FileSystemType>::fmt (1 mutants)
- src/gzip_header.rs: ExtraFlags::as_u8 (2 mutants)
- src/gzip_header.rs: ExtraFlags::from_u8 (3 mutants)
- src/gzip_header.rs: FileSystemType::as_u8 (2 mutants)
- src/gzip_header.rs: FileSystemType::from_u8 (21 mutants)
- src/gzip_header.rs: GzBuilder::comment (1 mutants)
- src/gzip_header.rs: GzBuilder::extra (1 mutants)
- src/gzip_header.rs: GzBuilder::filename (1 mutants)
- src/gzip_header.rs: GzBuilder::into_header (3 mutants)
- src/gzip_header.rs: GzBuilder::into_header_inner (7 mutants)
- src/gzip_header.rs: GzBuilder::into_header_with_checksum (3 mutants)
- src/gzip_header.rs: GzBuilder::into_header_xfl (3 mutants)
- src/gzip_header.rs: GzBuilder::mtime (1 mutants)
- src/gzip_header.rs: GzBuilder::os (1 mutants)
- src/gzip_header.rs: GzBuilder::xfl (1 mutants)
- src/gzip_header.rs: default_os (4 mutants)
- src/gzip_header.rs: without_nul (3 mutants)
- src/huffman.rs: Canonical::coded_symbols (2 mutants)
- src/huffman.rs: Canonical::is_single_unit_code (5 mutants)
- src/huffman.rs: Decoder::coded_symbols (2 mutants)
- src/huffman.rs: Decoder::is_single_unit_code (2 mutants)
- src/huffman.rs: Decoder::rebuild (1 mutants)
- src/huffman.rs: build_lengths (6 mutants)
- src/huffman.rs: decode_without_table (2 mutants)
- src/inflate.rs: Output::grow (3 mutants)
- src/inflate.rs: codes (20 mutants)
- src/inflate.rs: decompress (3 mutants)
- src/inflate.rs: read_dynamic_codes (29 mutants)
- src/lz77.rs: Matcher::prime (4 mutants)
- src/lz77.rs: Matcher::reset (1 mutants)
- src/tables.rs: build_dist_code (14 mutants)
- src/tables.rs: build_len_code (9 mutants)
- src/tables.rs: dist_code_by_search (13 mutants)
- src/tables.rs: fixed_litlen_lengths (16 mutants)
- src/tables.rs: len_code_by_search (14 mutants)
- src/write.rs: <impl Drop for DeflateEncoder<W>>::drop (3 mutants)
- src/write.rs: <impl Drop for GzEncoder<W>>::drop (3 mutants)
- src/write.rs: <impl Drop for ZlibEncoder<W>>::drop (3 mutants)
- src/write.rs: <impl Write for DeflateEncoder<W>>::flush (1 mutants)
- src/write.rs: <impl Write for DeflateEncoder<W>>::write (2 mutants)
- src/write.rs: <impl Write for GzEncoder<W>>::flush (1 mutants)
- src/write.rs: <impl Write for GzEncoder<W>>::write (2 mutants)
- src/write.rs: <impl Write for ZlibEncoder<W>>::flush (1 mutants)
- src/write.rs: <impl Write for ZlibEncoder<W>>::write (2 mutants)
- src/write.rs: Core<W>::collect (2 mutants)
- src/write.rs: Core<W>::finish_with (2 mutants)
- src/write.rs: Core<W>::reset (1 mutants)
- src/write.rs: Core<W>::send (9 mutants)
- src/write.rs: Core<W>::sync_flush (2 mutants)
- src/write.rs: Core<W>::write (2 mutants)
- src/write.rs: DeflateEncoder<W>::finish (1 mutants)
- src/write.rs: DeflateEncoder<W>::reset (1 mutants)
- src/write.rs: GzEncoder<W>::checksum (2 mutants)
- src/write.rs: GzEncoder<W>::finish (1 mutants)
- src/write.rs: GzEncoder<W>::from_builder (4 mutants)
- src/write.rs: GzEncoder<W>::output_all (1 mutants)
- src/write.rs: GzEncoder<W>::reset (1 mutants)
- src/write.rs: GzEncoder<W>::reset_inner (1 mutants)
- src/write.rs: GzEncoder<W>::reset_with_builder (1 mutants)
- src/write.rs: ZlibEncoder<W>::checksum (2 mutants)
- src/write.rs: ZlibEncoder<W>::finish (1 mutants)
- src/write.rs: ZlibEncoder<W>::output_all (1 mutants)
- src/write.rs: ZlibEncoder<W>::reset (1 mutants)
- src/zip.rs: <impl From<InflateError> for ZipError>::from (1 mutants)
- src/zip.rs: <impl fmt::Display for ZipError>::fmt (1 mutants)
- src/zip.rs: <impl std::error::Error for ZipError>::source (3 mutants)
- src/zip.rs: ZipArchive<'a>::entries (2 mutants)
- src/zip.rs: ZipArchive<'a>::index_of (4 mutants)
- src/zip.rs: ZipArchive<'a>::is_empty (2 mutants)
- src/zip.rs: ZipArchive<'a>::len (2 mutants)
- src/zip.rs: ZipArchive<'a>::parse (20 mutants)
- src/zip.rs: ZipArchive<'a>::raw_data (9 mutants)
- src/zip.rs: ZipArchive<'a>::read (9 mutants)
- src/zip.rs: ZipArchive<'a>::read_by_name (3 mutants)
- src/zip.rs: ZipEntry::is_dir (3 mutants)
- src/zip.rs: ZipWriter::add_directory (13 mutants)
- src/zip.rs: ZipWriter::add_entry (1 mutants)
- src/zip.rs: ZipWriter::add_file (11 mutants)
- src/zip.rs: ZipWriter::check_new_name (2 mutants)
- src/zip.rs: ZipWriter::finish (5 mutants)
- src/zip.rs: validate_name (1 mutants)

## Lines no proof executes, inside functions that proofs enter

- src/bitio.rs: BitReader<'a>::read_bits: lines 169
- src/bitio.rs: BitReader<'a>::refill: lines 144, 150, 157
- src/bitio.rs: BitWriter::write_bits: lines 39
- src/bitio.rs: reverse_bits: lines 296, 298
- src/crc32.rs: update: lines 92-95
- src/deflate.rs: compress_with: lines 70-71, 70, 70, 70, 72-74, 75
- src/deflate.rs: emit_run: lines 457
- src/deflate.rs: encode_with_matcher: lines 88, 93-96, 100-101, 101, 102-103, 104-105, 107-111, 112-115, 113
- src/deflate.rs: rle_code_lengths: lines 443
- src/huffman.rs: Canonical::rebuild: lines 448
- src/huffman.rs: Decoder::rebuild_with_fast_bits: lines 605
- src/huffman.rs: build_lengths_in: lines 159, 162, 165, 168, 195
- src/huffman.rs: canonical_codes: lines 67, 74
- src/huffman.rs: limit_depths: lines 240, 258, 260
- src/inflate.rs: Output::reserve: lines 105, 107-109
- src/inflate.rs: State<'_>::stored: lines 141, 142, 147, 149
- src/inflate.rs: decompress_with_limit: lines 49, 52, 52, 54, 54, 55-56, 56, 58, 62
- src/lz77.rs: MatchConfig::for_level: lines 72, 73, 74, 75, 77, 78
- src/lz77.rs: Matcher::longest_match: lines 209, 225, 249-250
- src/lz77.rs: Matcher::max_dist: lines 162
- src/lz77.rs: Matcher::tokenize: lines 301, 306, 309, 321-327
- src/lz77.rs: tokenize_runs: lines 353, 358
- src/zip.rs: civil_from_days: lines 196
- src/zip.rs: days_from_civil: lines 185
- src/zip.rs: days_in_month: lines 177
- src/zip.rs: directory_bounds: lines 782

## Harnesses

| harness | result | regions covered / instrumented (code under test) | coverage from |
|---|---|---:|---|
| adler32::proofs::from_checksum_resumes | OK | 2/2 | kaniraw |
| adler32::proofs::update_matches_bytewise | OK | 9/9 | kaniraw |
| bitio::proofs::aligned_bytes_after_lookahead_long | OK | 58/78 | kaniraw |
| bitio::proofs::aligned_bytes_after_lookahead_short | OK | 65/78 | kaniraw |
| bitio::proofs::aligned_bytes_roundtrip | OK | 46/53 | kaniraw |
| bitio::proofs::read_bits_matches_read_bit | OK | 26/30 | kaniraw |
| bitio::proofs::reader_eof_is_exact | OK | 22/23 | kaniraw |
| bitio::proofs::reverse_bits_is_reversal | OK | 4/6 | kaniraw |
| bitio::proofs::write_then_read_roundtrip | OK | 35/39 | kaniraw |
| crc32::proofs::byte_step_is_linear | OK | 1/1 | kaniraw |
| crc32::proofs::eight_byte_step_matches_on_a_basis | OK | 3/3 | kaniraw |
| crc32::proofs::lanes_are_linear | OK | 1/1 | kaniraw |
| crc32::proofs::slicing_tables_follow_recurrence | OK | 1/1 | kaniraw |
| crc32::proofs::streaming_is_concatenation | OK | 10/13 | kaniraw |
| crc32::proofs::table_matches_definition | OK | 6/6 | kaniraw |
| crc32::proofs::table_step_equals_bitwise_step | OK | 7/7 | kaniraw |
| crc32::proofs::update_matches_reference | OK | 13/16 | kaniraw |
| deflate::proofs::code_length_rle_roundtrip | OK | 43/53 | kaniraw |
| deflate::proofs::code_length_rle_zero_runs | OK | 48/53 | kaniraw |
| deflate::proofs::fixed_block_tokens_roundtrip | INCOMPLETE | 0/0 | none |
| deflate::proofs::fixed_code_table_matches_rfc | OK | 4/6 | kaniraw |
| deflate::proofs::fixed_tokens_have_the_rfc_layout | OK | 54/59 | kaniraw |
| deflate::proofs::stored_cost_adds_up_across_blocks | OK | 28/31 | kaniraw |
| deflate::proofs::stored_cost_is_exact | OK | 32/33 | kaniraw |
| e2e::roundtrip_stored | OK | 93/807 | kaniraw |
| huffman::proofs::build_lengths_complete_and_limited | OK | 85/108 | kaniraw |
| huffman::proofs::build_lengths_full_range_frequencies | OK | 72/108 | kaniraw |
| huffman::proofs::build_lengths_rejects_impossible_limit | OK | 12/94 | kaniraw |
| huffman::proofs::build_lengths_tight_limit | OK | 82/108 | kaniraw |
| huffman::proofs::canonical_codes_are_prefix_free | OK | 33/36 | kaniraw |
| huffman::proofs::decode_arbitrary_bits | OK | 64/67 | kaniraw |
| huffman::proofs::decoder_new_never_panics | OK | 61/65 | kaniraw |
| huffman::proofs::encode_then_decode_symbol | INCOMPLETE | 106/120 | kaniraw (coverage-only run) |
| huffman::proofs::fast_decode_matches_slow | INCOMPLETE | 103/129 | kaniraw (coverage-only run) |
| huffman::proofs::limit_depths_keeps_code_complete | OK | 34/37 | kaniraw |
| huffman::proofs::sort_by_weight_sorts | OK | 19/19 | kaniraw |
| inflate::proofs::expand_code_length_is_safe | OK | 13/13 | kaniraw |
| inflate::proofs::stored_block_semantics | OK | 55/314 | kaniraw |
| lz77::proofs::common_prefix_matches_reference | OK | 20/21 | kaniraw |
| lz77::proofs::copy_within_buffer_meets_spec | OK | 18/20 | kaniraw |
| lz77::proofs::no_chain_means_literals_only | OK | 10/123 | kaniraw |
| lz77::proofs::replay_arbitrary_tokens | OK | 20/20 | kaniraw |
| lz77::proofs::rle_tokens_valid_and_replay | OK | 34/120 | kaniraw |
| lz77::proofs::runs_are_matched | OK | 67/116 | kaniraw |
| lz77::proofs::short_copy_meets_spec | OK | 14/20 | kaniraw |
| lz77::proofs::tokens_valid_and_replay_greedy | OK | 73/120 | kaniraw |
| lz77::proofs::tokens_valid_and_replay_level9 | OK | 77/120 | kaniraw |
| lz77::proofs::tokens_valid_and_replay_small_tables | OK | 74/109 | kaniraw |
| tables::proofs::distance_code_covers_every_distance | OK | 4/4 | kaniraw |
| tables::proofs::distance_decode_is_inverse | OK | 4/4 | kaniraw |
| tables::proofs::fixed_lengths_match_rfc | OK | 0/0 | kaniraw |
| tables::proofs::length_code_covers_every_length | OK | 1/1 | kaniraw |
| tables::proofs::length_decode_is_inverse | OK | 1/1 | kaniraw |
| zip::proofs::central_header_roundtrip | OK | 9/11 | kaniraw |
| zip::proofs::civil_date_conversion_roundtrip | OK | 26/29 | kaniraw |
| zip::proofs::directory_bounds_sound | OK | 5/6 | kaniraw |
| zip::proofs::dos_datetime_roundtrip | OK | 24/25 | kaniraw |
| zip::proofs::eocd_roundtrip | OK | 9/11 | kaniraw |
| zip::proofs::find_eocd_is_sound | OK | 12/12 | kaniraw |
| zip::proofs::header_parsers_never_panic | OK | 20/20 | kaniraw |
| zip::proofs::is_valid_is_the_calendar | OK | 37/40 | kaniraw |
| zip::proofs::local_header_roundtrip | OK | 9/11 | kaniraw |
| zip::proofs::record_bounds_sound | OK | 10/10 | kaniraw |
| zip::proofs::size_limits_are_exact | OK | 7/7 | kaniraw |
| zip::proofs::unix_time_always_valid | OK | 37/39 | kaniraw |
| zip::proofs::unix_time_converts_exactly | OK | 21/23 | kaniraw |
| zip::proofs::validate_name_is_exactly_the_rule | OK | 32/32 | kaniraw |
