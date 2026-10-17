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
mem_gb="${KANI_MEMORY_GB:-14}"   # public runners have 16 GiB

mkdir -p kani-logs
: > kani-logs/status.tsv

# Harness names, skipping those gated behind the `kani-extended` feature
# unless KANI_EXTENDED=1 (which also enables the feature).
names=$(KANI_EXTENDED="${KANI_EXTENDED:-0}" python3 - "$file" <<'PY'
import os, re, sys
src = open(sys.argv[1]).read()
extended = os.environ.get("KANI_EXTENDED") == "1"
pat = r"((?:[ \t]*#\[[^\n]*\n)*)[ \t]*fn\s+(\w+)"
for m in re.finditer(pat, src):
    attrs, name = m.group(1), m.group(2)
    if "kani::proof" not in attrs:
        continue
    if "kani-extended" in attrs and not extended:
        continue
    print(name)
PY
)
# Optionally only the harnesses whose names match an extended regex: one
# from a manual run (KANI_HARNESS_FILTER), and one per CI job when a module
# is split across several jobs (KANI_SHARD_ONLY / KANI_SHARD_SKIP). A split
# module's last job uses SKIP, so a new harness always runs somewhere.
for f in "${KANI_HARNESS_FILTER:-}" "${KANI_SHARD_ONLY:-}"; do
  if [ -n "$f" ]; then
    names=$(printf '%s\n' "$names" | grep -E -- "$f" || true)
  fi
done
if [ -n "${KANI_SHARD_SKIP:-}" ]; then
  names=$(printf '%s\n' "$names" | grep -Ev -- "$KANI_SHARD_SKIP" || true)
fi
if [ "${KANI_EXTENDED:-0}" = "1" ]; then
  KANI_EXTRA_FLAGS="${KANI_EXTRA_FLAGS:-} --features kani-extended"
fi
echo "harnesses in $file:"; echo "$names"

# Compile once without the memory cap; later runs reuse the build.
# shellcheck disable=SC2086
cargo kani --harness "${prefix}__compile_only__" --exact ${KANI_EXTRA_FLAGS:-} > kani-logs/_build.log 2>&1 || true
if grep -q "^error" kani-logs/_build.log; then
  cat kani-logs/_build.log
fi

# Optionally publish logs to a branch after every harness, so partial
# results can be read while the job is still running.
publish() {
  [ -n "${PUBLISH_BRANCH:-}" ] || return 0
  local tmp
  tmp=$(mktemp -d)
  cp -r kani-logs/. "$tmp"/
  ( cd "$tmp" && git init -q && git checkout -q -b logs && git add . \
      && git -c user.name=ci -c user.email=ci@localhost commit -qm "Kani logs, run ${GITHUB_RUN_ID:-local}" \
      && git push -qf "https://x-access-token:${GITHUB_TOKEN}@github.com/${GITHUB_REPOSITORY}" "logs:refs/heads/${PUBLISH_BRANCH}" ) \
    || echo "log publish failed"
  rm -rf "$tmp"
}

# Runs a command under a memory cap. A cgroup limit (systemd-run) caps
# resident memory, so the verifier is stopped cleanly instead of the whole
# runner being OOM-killed; `ulimit -v` is the fallback.
run_capped() {
  if command -v systemd-run >/dev/null && sudo -n true 2>/dev/null; then
    sudo -E systemd-run --quiet --scope -p MemoryMax="${mem_gb}G" -p MemorySwapMax=0 \
      --uid="$(id -u)" --gid="$(id -g)" \
      env PATH="$PATH" HOME="$HOME" "$@"
  else
    ( ulimit -v $((mem_gb * 1024 * 1024)); exec "$@" )
  fi
}

for n in $names; do
  h="${prefix}${n}"
  echo "::group::$h"
  start=$SECONDS
  # Regular output names failed checks; concrete playback prints a unit
  # test reproducing any counterexample.
  # shellcheck disable=SC2086
  run_capped /usr/bin/time -v timeout "$timeout_s" cargo kani --harness "$h" --exact \
      -Z concrete-playback --concrete-playback=print ${KANI_EXTRA_FLAGS:-} \
    > "kani-logs/$n.log" 2>&1
  status=$?
  secs=$((SECONDS - start))
  grep -E "VERIFICATION|Failed Checks|^ File:|Verification Time" "kani-logs/$n.log" | tail -n 30
  echo "::endgroup::"
  printf '%s\t%s\t%s\n' "$h" "$status" "$secs" >> kani-logs/status.tsv
  echo "$h: exit $status after ${secs}s"
  publish
done
