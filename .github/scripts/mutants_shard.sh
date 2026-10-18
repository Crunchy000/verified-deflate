#!/usr/bin/env bash
# Runs one shard of the mutation run for the CI assurance mode.
#
#   PASS=proofs  the proof harnesses, sampled with random inputs (a mutant
#                they catch makes the real proof fail too, since sampling
#                only ever finds genuine counterexamples)
#   PASS=tests   the whole test suite, including the interop tests
#
# usage: PASS=proofs|tests SHARD=k SHARDS=n mutants_shard.sh
set -uo pipefail
shards="${SHARDS:-16}"
args=()
if [ "$PASS" = "proofs" ]; then
  export RUSTFLAGS="--cfg kani --cfg kani_sample"
  export KANI_SAMPLE_ITERS="${KANI_SAMPLE_ITERS:-300000}"
  # `--lib` for the build too: the sampling stand-in for `kani` is a
  # dev-dependency, so only the library's test build has it.
  args=(--cargo-arg=--lib -- -- proofs e2e)
fi
# Round-robin sharding spreads each module's mutants over all shards, so
# no shard gets all the slow ones.
cargo mutants --shard "$SHARD/$shards" --sharding round-robin -j 2 --timeout-multiplier 4 \
  --output "mutants-$PASS-$SHARD" "${args[@]}"
code=$?
# cargo-mutants exits 2 when some mutants were missed and 3 when some timed
# out; both are results, not errors.
case "$code" in
  0 | 2 | 3) exit 0 ;;
  *) exit "$code" ;;
esac
