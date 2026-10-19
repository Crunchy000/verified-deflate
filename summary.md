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
