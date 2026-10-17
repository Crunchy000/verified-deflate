#!/usr/bin/env bash
# Runs a command, keeping its output in a log. On failure, the tail of the
# log is attached to the run as an error annotation, so failures can be
# read from the API without downloading raw logs.
set -uo pipefail
title="$1"; shift
log="$(mktemp)"
"$@" 2>&1 | tee "$log"
status=${PIPESTATUS[0]}
if [ "$status" -ne 0 ]; then
  body="$(grep -v '^\s*$' "$log" | tail -n 80)"
  body="${body//'%'/'%25'}"; body="${body//$'\r'/'%0D'}"; body="${body//$'\n'/'%0A'}"
  echo "::error title=${title} failed::${body}"
fi
exit "$status"
