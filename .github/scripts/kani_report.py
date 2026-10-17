#!/usr/bin/env python3
"""Summarise `cargo kani --output-format=terse` output.

Writes a per-harness table to the job summary and attaches it to the run
as annotations (one notice with the table, one error per failed harness),
so results can be read through the checks API."""
import os
import re
import sys


def esc(s: str) -> str:
    return s.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def main(path: str) -> int:
    try:
        text = open(path, encoding="utf-8", errors="replace").read()
    except FileNotFoundError:
        print("::error title=Kani::no log produced")
        return 1
    blocks = re.split(r"(?m)^(?=Checking harness )", text)
    rows, failures = [], []
    for b in blocks:
        m = re.match(r"Checking harness (\S+?)\.\.\.", b)
        if not m:
            continue
        name = m.group(1)
        status = "FAILED" if "VERIFICATION:- FAILED" in b else (
            "SUCCESSFUL" if "VERIFICATION:- SUCCESSFUL" in b else "UNKNOWN")
        t = re.search(r"Verification Time: ([0-9.]+)s", b)
        secs = t.group(1) if t else "?"
        checks = re.search(r"\*\* (\d+) of (\d+) failed", b)
        rows.append(f"{status:10} {secs:>9}s  {name}")
        if status != "SUCCESSFUL":
            failures.append((name, b.strip()[:6000]))
    summary = "\n".join(rows) if rows else "no harness results found"
    tail = re.findall(r"(?m)^(Complete - .*|Summary:.*)$", text)
    if tail:
        summary += "\n" + "\n".join(tail)
    if not rows:
        # Probably a build failure: show the first errors.
        errs = [l for l in text.splitlines() if l.strip()]
        first = next((i for i, l in enumerate(errs) if l.startswith("error")), max(0, len(errs) - 60))
        summary += "\n" + "\n".join(errs[first:first + 60])
    print(summary)
    print(f"::notice title=Kani results::{esc(summary)}")
    for name, block in failures[:9]:
        print(f"::error title=Kani {name}::{esc(block)}")
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as f:
            f.write("```\n" + summary + "\n```\n")
    return 1 if failures or not rows else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
