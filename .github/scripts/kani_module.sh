#!/usr/bin/env bash
# Verifies each Kani harness of one module in its own process, under a
# memory cap and a time limit, so one expensive harness cannot take the
# whole job down. Logs go to kani-logs/<harness>.log and exit codes to
# kani-logs/status.tsv for kani_report.py.
#
# usage: kani_module.sh <harness-prefix> <source-file>
set -uo pipefail
prefix="$1"
file="$2"
timeout_s="${KANI_HARNESS_TIMEOUT:-1800}"
mem_kb="${KANI_MEMORY_KB:-5767168}"   # 5.5 GiB

mkdir -p kani-logs
: > kani-logs/status.tsv

names=$(python3 - "$file" <<'PY'
import re, sys
src = open(sys.argv[1]).read()
pat = r"#\[kani::proof\][^\n]*\n(?:\s*#\[[^\n]*\n)*\s*fn\s+(\w+)"
print("\n".join(m.group(1) for m in re.finditer(pat, src)))
PY
)
echo "harnesses in $file:"; echo "$names"

# Compile once without the memory cap; later runs reuse the build.
cargo kani --harness "${prefix}__compile_only__" --exact --output-format=terse > kani-logs/_build.log 2>&1 || true
if grep -q "^error" kani-logs/_build.log; then
  cat kani-logs/_build.log
fi

for n in $names; do
  h="${prefix}${n}"
  echo "::group::$h"
  start=$SECONDS
  ( ulimit -v "$mem_kb"; exec timeout "$timeout_s" cargo kani --harness "$h" --exact --output-format=terse ) \
    > "kani-logs/$n.log" 2>&1
  status=$?
  secs=$((SECONDS - start))
  tail -n 30 "kani-logs/$n.log"
  echo "::endgroup::"
  printf '%s\t%s\t%s\n' "$h" "$status" "$secs" >> kani-logs/status.tsv
  echo "$h: exit $status after ${secs}s"
done
