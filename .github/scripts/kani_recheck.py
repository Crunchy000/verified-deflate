#!/usr/bin/env python3
"""Re-checks mutants with real Kani (the second step of the assurance mode).

The assurance report lists, in kani_candidates.json, the mutants that a
proof executes but that the sampled proofs did not catch. Sampling tries
random inputs; Kani checks all of them, so it may still catch these. This
applies each mutant in turn and verifies the harnesses that reach it.

  kani_recheck.py run <assurance-dir> <shard> <shards> <out.jsonl>
      Applies each of this shard's mutants to the source tree, verifies up
      to MAX_HARNESSES of the harnesses that reach it (those in the
      mutant's own module first, then the fastest), stops at the first
      failure, and restores the file.

  kani_recheck.py summarize <assurance-dir> <results-dir> <out-dir>
      Merges the shards' results into recheck.md and recheck.json.
"""
import json
import os
import re
import subprocess
import sys
import time

MAX_HARNESSES = 3
MEM_GB = 14

# Kani flags per module, as in the CI matrix.
FLAGS = {
    "huffman": ["-Z", "stubbing"],
    "lz77": ["-Z", "stubbing", "--no-memory-safety-checks"],
    "deflate": ["-Z", "stubbing", "--no-memory-safety-checks"],
    "inflate": ["-Z", "stubbing", "--no-memory-safety-checks"],
    "e2e": ["-Z", "stubbing", "--no-memory-safety-checks"],
}


def harness_times(adir):
    """{harness: seconds} for the harnesses that verified in the assurance run."""
    times = {}
    # A harness that ran out of memory only with coverage instrumentation
    # verifies without it (its coverage-only run succeeded); use it too.
    for dirpath, _, files in os.walk(os.path.join(adir, "inputs")):
        for name in ("status.tsv", "coverage-fallback.tsv"):
            if name in files:
                for line in open(os.path.join(dirpath, name)):
                    parts = line.rstrip("\n").split("\t")
                    if len(parts) == 3 and parts[1] == "0":
                        times.setdefault(parts[0], int(parts[2]))
    return times


def apply(text, span, replacement):
    """Replaces a cargo-mutants span (1-based lines, 1-based char columns,
    end exclusive) in `text`."""
    lines = text.split("\n")

    def offset(lc):
        return sum(len(l) + 1 for l in lines[:lc["line"] - 1]) + lc["column"] - 1

    a, b = offset(span["start"]), offset(span["end"])
    return text[:a] + replacement + text[b:]


def capped(cmd):
    """Runs under a memory cap, as kani_module.sh does."""
    if subprocess.run(["sudo", "-n", "true"], capture_output=True).returncode == 0 and \
            subprocess.run(["which", "systemd-run"], capture_output=True).returncode == 0:
        return ["sudo", "-E", "systemd-run", "--quiet", "--scope", f"-pMemoryMax={MEM_GB}G",
                "-pMemorySwapMax=0", f"--uid={os.getuid()}", f"--gid={os.getgid()}",
                "env", f"PATH={os.environ['PATH']}", f"HOME={os.environ['HOME']}"] + cmd
    return cmd


def verify(harness, timeout):
    module = harness.split("::", 1)[0]
    cmd = ["timeout", str(timeout), "cargo", "kani", "--harness", harness, "--exact"] + FLAGS.get(module, [])
    t0 = time.time()
    p = subprocess.run(capped(cmd), capture_output=True, text=True, errors="replace")
    out = p.stdout + p.stderr
    secs = round(time.time() - t0)
    if "VERIFICATION:- FAILED" in out:
        failed = re.findall(r"Failed Checks: (.*)", out)
        return "FAILED", secs, failed[:3]
    if "VERIFICATION:- SUCCESSFUL" in out:
        return "SUCCESSFUL", secs, []
    if p.returncode == 124:
        return "TIMEOUT", secs, []
    if re.search(r"^error(\[E\d+\])?:", out, re.M):
        return "COMPILE-ERROR", secs, [l for l in out.splitlines() if l.startswith("error")][:3]
    return f"ERROR({p.returncode})", secs, out.splitlines()[-3:]


def run(adir, shard, shards, out_path):
    cands = json.load(open(os.path.join(adir, "kani_candidates.json")))
    times = harness_times(adir)
    mine = [c for i, c in enumerate(cands) if i % shards == shard]
    print(f"{len(mine)} of {len(cands)} candidates in shard {shard}/{shards}", flush=True)
    with open(out_path, "w") as out:
        for c in mine:
            path = c["file"]
            original = open(path).read()
            module = os.path.splitext(os.path.basename(path))[0]
            hs = [h for h in c["harnesses"] if h in times]
            hs.sort(key=lambda h: (not h.startswith(module + "::"), times[h]))
            result = {"name": c["name"], "harnesses": [], "verdict": "no-harness"}
            try:
                open(path, "w").write(apply(original, c["span"], c["replacement"]))
                verdict = "survives"
                for h in hs[:MAX_HARNESSES]:
                    timeout = min(1800, max(300, 4 * times[h]))
                    status, secs, detail = verify(h, timeout)
                    result["harnesses"].append({"harness": h, "status": status, "secs": secs, "detail": detail})
                    print(f"{status:14} {secs:5}s  {h}  <- {c['name']}", flush=True)
                    if status == "FAILED":
                        verdict = "caught"
                        break
                    if status != "SUCCESSFUL":
                        verdict = "undetermined" if verdict == "survives" else verdict
                    if status == "COMPILE-ERROR":
                        verdict = "unviable"
                        break
                result["verdict"] = verdict if hs else "no-harness"
            finally:
                open(path, "w").write(original)
            out.write(json.dumps(result) + "\n")
            out.flush()


def summarize(adir, rdir, outdir):
    os.makedirs(outdir, exist_ok=True)
    cands = {c["name"]: c for c in json.load(open(os.path.join(adir, "kani_candidates.json")))}
    results = {}
    for dirpath, _, files in os.walk(rdir):
        for f in files:
            if f.endswith(".jsonl"):
                for line in open(os.path.join(dirpath, f)):
                    if line.strip():
                        r = json.loads(line)
                        results[r["name"]] = r
    counts = {}
    for r in results.values():
        counts[r["verdict"]] = counts.get(r["verdict"], 0) + 1
    missing = sorted(set(cands) - set(results))
    lines = ["# Kani re-check of mutants the sampled proofs missed\n",
             f"{len(results)} of {len(cands)} candidates checked.\n",
             "| verdict | mutants | meaning |", "|---|---:|---|"]
    meaning = {
        "caught": "the real proof fails on the mutant: the proof does pin this down",
        "survives": "every harness that reaches it still verifies: the properties do not pin this down",
        "undetermined": "a harness timed out or errored before any failed",
        "unviable": "does not compile under Kani",
        "no-harness": "no verified harness reaches it",
    }
    for v in ("caught", "survives", "undetermined", "unviable", "no-harness"):
        lines.append(f"| {v} | {counts.get(v, 0)} | {meaning[v]} |")
    for v, title in (("survives", "Survive the real proofs"), ("undetermined", "Undetermined")):
        rs = sorted((r for r in results.values() if r["verdict"] == v), key=lambda r: r["name"])
        lines.append(f"\n## {title} ({len(rs)})\n")
        for r in rs:
            tc = " (tests catch it)" if cands.get(r["name"], {}).get("tests_caught") else " (tests miss it too)"
            hs = ", ".join(f"{h['harness'].rsplit('::', 1)[-1]} {h['status']}" for h in r["harnesses"])
            lines.append(f"- `{r['name']}`{tc} — {hs}")
    if missing:
        lines.append(f"\n## Not checked ({len(missing)})\n")
        lines.extend(f"- `{m}`" for m in missing)
    open(os.path.join(outdir, "recheck.md"), "w").write("\n".join(lines) + "\n")
    json.dump(sorted(results.values(), key=lambda r: r["name"]),
              open(os.path.join(outdir, "recheck.json"), "w"), indent=1)
    print("\n".join(lines[:12]))


if __name__ == "__main__":
    if sys.argv[1] == "run":
        run(sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), sys.argv[5])
    elif sys.argv[1] == "summarize":
        summarize(sys.argv[2], sys.argv[3], sys.argv[4])
    else:
        sys.exit(__doc__)
