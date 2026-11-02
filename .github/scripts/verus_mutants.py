#!/usr/bin/env python3
"""Checks that the Verus proofs reject broken code.

Each mutant is a small change to the executable code of a verified copy
under verus/, the kind of bug the theorem should rule out. For each one,
this copies verus/ to a scratch directory, applies the change and runs
Verus; the mutant is killed when verification fails. A surviving mutant
means the specification does not pin that behaviour down (or the proof is
vacuous), so the script exits 1.

Usage: verus_mutants.py <verus binary> <repo root>
"""

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# (file under verus/src, what the bug is, exact old text, replacement)
MUTANTS = [
    ("limit_depths.rs", "clamps to one past the limit",
     "if depth[i] > limit32 {", "if depth[i] > limit32 + 1 {"),
    ("limit_depths.rs", "leaves the code one unit over-subscribed",
     "    while kraft > full\n", "    while kraft > full + 1\n"),
    ("limit_depths.rs", "lengthens a code already at the limit",
     "if depth[k] < limit32 {", "if depth[k] <= limit32 {"),
    ("limit_depths.rs", "lengthens by the wrong share",
     "kraft -= 1u64 << (limit32 - depth[p] - 1);", "kraft -= 1u64 << (limit32 - depth[p]);"),
    ("limit_depths.rs", "leaves the code one unit short of full",
     "    while kraft < full\n", "    while kraft + 1 < full\n"),
    ("limit_depths.rs", "shortens a one-bit code",
     "if depth[k] > 1 {", "if depth[k] > 0 {"),
    ("limit_depths.rs", "shortens a shorter code, which can overshoot",
     "Some(p) => depth[k] > depth[p] || (depth[k] == depth[p] && weight[k] > weight[p]),",
     "Some(p) => depth[k] < depth[p] || (depth[k] == depth[p] && weight[k] > weight[p]),"),
    ("build_lengths.rs", "accepts more used symbols than codes of the limit",
     "if m > (1usize << limit) {", "if m > (2usize << limit) {"),
    ("build_lengths.rs", "accepts a limit of 0",
     "if n < 2 || limit == 0 {", "if n < 2 {"),
    ("build_lengths.rs", "leaves a symbol with frequency 1 uncoded",
     "if freqs[i] != 0 {", "if freqs[i] > 1 {"),
    ("build_lengths.rs", "codes only one symbol when fewer than two are used",
     "lengths[if s == 0 { 1 } else { 0 }] = 1;", "lengths[if s == 0 { 1 } else { 0 }] = 0;"),
    ("build_lengths.rs", "writes each length one too long",
     "lengths[used[k] as usize] = depth[k] as u8;", "lengths[used[k] as usize] = depth[k] as u8 + 1;"),
    ("build_lengths.rs", "merges until the queues are empty",
     "while (m - leaf) + (next - merged) > 1 {", "while (m - leaf) + (next - merged) > 0 {"),
    ("build_lengths.rs", "gives a child its parent's depth",
     "depth[id] = depth[parent[id] as usize] + 1;", "depth[id] = depth[parent[id] as usize];"),
    ("build_lengths.rs", "compares a child past the end of the heap",
     "if child + 1 < end && heavier(", "if child + 1 <= end && heavier("),
    ("name_is_safe.rs", "accepts names up to 65536 bytes",
     "if n == 0 || n > 0xFFFF {", "if n == 0 || n > 0x10000 {"),
    ("name_is_safe.rs", "accepts a lower-case drive prefix",
     "((b'a' <= b[0] && b[0] <= b'z') || (b'A' <= b[0] && b[0] <= b'Z'))",
     "(b'A' <= b[0] && b[0] <= b'Z')"),
    ("name_is_safe.rs", "refuses directory names (trailing /)",
     "let end = if b[n - 1] == b'/' { n - 1 } else { n };", "let end = n;"),
    ("name_is_safe.rs", "accepts `..`",
     " || (len == 2 && b[start] == b'.' && b[start + 1] == b'.') {", " {"),
    ("name_is_safe.rs", "refuses safe names such as `.a`",
     "(len == 2 && b[start] == b'.' && b[start + 1] == b'.')", "(len == 2 && b[start] == b'.')"),
    ("name_is_safe.rs", "accepts NUL",
     "} else if b[i] == 0 || b[i] == b'\\\\' {", "} else if b[i] == b'\\\\' {"),
]


def main() -> int:
    verus, root = sys.argv[1], Path(sys.argv[2])
    survived = 0
    for name, bug, old, new in MUTANTS:
        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp) / "verus"
            shutil.copytree(root / "verus", work)
            path = work / "src" / name
            text = path.read_text()
            if text.count(old) != 1:
                print(f"STALE     {name}: {bug}: the text to change occurs {text.count(old)} times")
                survived += 1
                continue
            path.write_text(text.replace(old, new))
            run = subprocess.run([verus, "--crate-type=lib", str(work / "src" / "lib.rs")],
                                 capture_output=True, text=True)
            out = run.stdout + run.stderr
            if run.returncode == 0:
                print(f"SURVIVED  {name}: {bug}")
                survived += 1
            else:
                first = next((line.strip() for line in out.splitlines() if line.startswith("error")), "")
                print(f"killed    {name}: {bug}  ({first})")
    print(f"{len(MUTANTS) - survived} of {len(MUTANTS)} mutants rejected by Verus")
    return 1 if survived else 0


if __name__ == "__main__":
    sys.exit(main())
