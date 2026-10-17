#!/usr/bin/env python3
"""Summarise per-harness Kani logs written by kani_module.sh.

Writes a table to the job summary and attaches it to the run as
annotations (one notice with the table, one error per failing harness),
so results can be read through the checks API without raw logs."""
import os
import re
import sys


def esc(s: str) -> str:
    return s.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def main(logdir: str) -> int:
    status_file = os.path.join(logdir, "status.tsv")
    if not os.path.exists(status_file):
        print("::error title=Kani::no status file; did verification start?")
        return 1
    rows, failures = [], []
    for line in open(status_file):
        parts = line.rstrip("\n").split("\t")
        if len(parts) != 3:
            continue
        harness, code, secs = parts
        short = harness.rsplit("::", 1)[-1]
        try:
            log = open(os.path.join(logdir, short + ".log"), errors="replace").read()
        except FileNotFoundError:
            log = ""
        if "VERIFICATION:- SUCCESSFUL" in log and code == "0":
            status = "OK"
        elif "VERIFICATION:- FAILED" in log:
            status = "FAILED"
        elif code == "124":
            status = "TIMEOUT"
        elif code == "137" or re.search(r"bad_alloc|out of memory|Out of memory|memory allocation", log):
            status = "OUT-OF-MEMORY"
        elif "No proof harnesses" in log or "no harnesses" in log.lower():
            status = "NOT-FOUND"
        else:
            status = f"ERROR({code})"
        t = re.search(r"Verification Time: ([0-9.]+)s", log)
        vt = f"{float(t.group(1)):.1f}s" if t else "-"
        rows.append(f"{status:14} {vt:>9} {secs:>6}s wall  {harness}")
        if status != "OK":
            # Only look at verifier output, not compiler warnings before it.
            start = log.rfind("Checking harness")
            tail = log[start:] if start >= 0 else log
            lines = [l for l in tail.splitlines() if l.strip()]
            picked = []
            for i, l in enumerate(lines):
                if re.match(r"\s*(\*\* \d+ of|Failed Checks)", l):
                    picked.extend(lines[i:i + 2])
            # Concrete playback: the counterexample as a unit test.
            m = re.search(r"(?s)(#\[test\]\s*fn kani_concrete_playback.*?\n\})", tail)
            if m:
                picked.append(m.group(1)[:3000])
            body = "\n".join(dict.fromkeys(picked)) if picked else "\n".join(lines[-60:])
            failures.append((harness, status, body[:6000]))
    summary = "\n".join(rows) or "no harnesses ran"
    ok = sum(r.startswith("OK") for r in rows)
    summary += f"\n{ok}/{len(rows)} harnesses verified"
    print(summary)
    print(f"::notice title=Kani results::{esc(summary)}")
    for harness, status, body in failures[:9]:
        print(f"::error title=Kani {status} {harness}::{esc(body)}")
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as f:
            f.write("```\n" + summary + "\n```\n")
    return 1 if failures or not rows else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else "kani-logs"))
