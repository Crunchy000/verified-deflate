#!/usr/bin/env bash
# Runs the `deflate` crate's own tests (version 1.0.0) against this crate,
# imported as `deflate` the way a user migrating from `deflate` would
# import it. The tests are fetched from deflate's repository at the 1.0.0
# release, not copied into this one.
#
# usage: tools/deflate-upstream/run.sh [work-dir]
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
work="${1:-$(mktemp -d)}"
commit=3262c2526f7efebea7d3da73f5ec547860f82d46   # deflate 1.0.0

if [ ! -d "$work/deflate-rs/.git" ]; then
  git clone -q https://github.com/image-rs/deflate-rs "$work/deflate-rs"
fi
git -C "$work/deflate-rs" checkout -q "$commit"

pkg="$work/upstream-tests"
rm -rf "$pkg"
mkdir -p "$pkg/src" "$pkg/tests"
: > "$pkg/src/lib.rs"
cat > "$pkg/Cargo.toml" <<TOML
[package]
name = "deflate-upstream-tests"
version = "0.0.0"
edition = "2018"
publish = false

[dev-dependencies]
# The migration a deflate user makes: this crate, renamed to \`deflate\`.
deflate = { package = "verified-deflate", path = "$root", features = ["gzip"] }
miniz_oxide = { path = "$here/miniz_oxide" }

[features]
gzip = []

[workspace]
TOML
cp -r "$work/deflate-rs/tests/." "$pkg/tests/"
python3 "$here/port.py" "$work/deflate-rs" "$pkg/tests/ported_unit.rs"
cd "$pkg"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$work/target}" cargo test --release --features gzip -- --include-ignored
