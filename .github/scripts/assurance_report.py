#!/usr/bin/env python3
"""Joins Kani source coverage with cargo-mutants outcomes (CI assurance mode).

usage: assurance_report.py <input-dir> <output-dir>

<input-dir>/coverage/*/  one artifact per Kani job: status.tsv, <harness>.log
                         and coverage/<harness>/*_kaniraw.json
<input-dir>/mutants/mutants-<pass>-<shard>/outcomes.json
                         pass is `proofs` (the harnesses run as randomised
                         tests) or `tests` (the whole test suite)

Each mutant (a small deliberate bug) is classified by whether the sampled
proofs caught it, whether the tests caught it, and whether any proof
executes the code it changes:

  proved        the sampled proofs caught it, so the real proofs do too
  proof-reach   a proof executes the code but sampling did not catch it;
                the real proof (every input) may, so these are the
                candidates to re-check with Kani
  tests-only    no proof executes the code; only the tests caught it
  unchecked     nothing caught it (a real gap, or a change that does not
                alter behaviour, such as a heuristic that only affects speed
                or compression ratio)

Writes summary.md (headline numbers), report.md (everything, grouped by
priority), assurance.json (every mutant and region) and kani_candidates.json.
"""
import json
import os
import re
import sys
from collections import defaultdict

CAUGHT = ("CaughtMutant", "Timeout")


def norm_file(path, root):
    """Path relative to the repository for files of this crate, else None."""
    p = path.replace("\\", "/")
    if os.path.isabs(p):
        if not p.startswith(root + "/"):
            # Same checkout path on every runner; fall back to the last
            # /src/ for anything else that is clearly not a dependency.
            if re.search(r"/(rustc|library|\.cargo|\.rustup|\.kani)/", p):
                return None
            i = p.rfind("/src/")
            if i < 0:
                return None
            p = p[i + 1:]
        else:
            p = p[len(root) + 1:]
    while p.startswith("./"):
        p = p[2:]
    return p if p.startswith("src/") else None


# ---------------------------------------------------------------- coverage

def harness_status(log):
    if "VERIFICATION:- SUCCESSFUL" in log:
        return "OK"
    if "VERIFICATION:- FAILED" in log:
        return "FAILED"
    return "INCOMPLETE"


def parse_log_coverage(log):
    """The coverage listing Kani prints after the verification result."""
    out = []
    i = log.find("Source-based code coverage results:")
    if i < 0:
        return out
    cur = None
    for line in log[i:].splitlines()[1:]:
        m = re.match(r"^(\S.*?) \((.*)\)$", line)
        if m:
            cur = (m.group(1), m.group(2))
            continue
        m = re.match(r"^ \* (\d+):(\d+) - (\d+):(\d+) (\w+)", line)
        if m and cur:
            out.append({
                "function": cur[1],
                "region": {"file": cur[0], "start": [int(m.group(1)), int(m.group(2))],
                           "end": [int(m.group(3)), int(m.group(4))]},
                "status": m.group(5),
            })
        elif line.startswith("Summary:") or line.startswith("Complete -"):
            break
    return out


def load_coverage(cov_root, root):
    """Returns (regions, harnesses).

    regions: {(file, sl, sc, el, ec): {"functions", "covered_by", "seen_by"}}
    harnesses: {name: {"status", "covered", "seen", "source"}}"""
    regions = {}
    harnesses = {}
    if not os.path.isdir(cov_root):
        return regions, harnesses
    for art in sorted(os.listdir(cov_root)):
        adir = os.path.join(cov_root, art)
        if not os.path.isdir(adir):
            continue
        full = {}
        st = os.path.join(adir, "status.tsv")
        if os.path.exists(st):
            for line in open(st):
                parts = line.rstrip("\n").split("\t")
                if len(parts) == 3:
                    full[parts[0].rsplit("::", 1)[-1]] = parts[0]
        short_names = set(full)
        cdir = os.path.join(adir, "coverage")
        if os.path.isdir(cdir):
            short_names |= set(os.listdir(cdir))
        for short in sorted(short_names):
            name = full.get(short, short)
            try:
                log = open(os.path.join(adir, short + ".log"), errors="replace").read()
            except FileNotFoundError:
                log = ""
            checks, source = [], None
            hdir = os.path.join(cdir, short)
            if os.path.isdir(hdir):
                for f in sorted(os.listdir(hdir)):
                    if f.endswith("_kaniraw.json"):
                        try:
                            data = json.load(open(os.path.join(hdir, f)))
                        except (OSError, ValueError):
                            continue
                        data = data.get("data", data)
                        for fkey, lst in data.items():
                            for c in lst:
                                c.setdefault("region", {}).setdefault("file", fkey)
                                checks.append(c)
                        source = "kaniraw"
            if not checks:
                checks = parse_log_coverage(log)
                source = "log" if checks else None
            h = harnesses.setdefault(name, {"status": harness_status(log), "covered": 0, "seen": 0,
                                            "source": source})
            for c in checks:
                r = c.get("region", {})
                f = norm_file(str(r.get("file", "")), root)
                if not f:
                    continue
                fn = c.get("function", "")
                # The proofs themselves are the checker, not code under test.
                if re.search(r"(^|::)(proofs|e2e)::", fn) or f == "src/e2e.rs":
                    continue
                try:
                    key = (f, int(r["start"][0]), int(r["start"][1]), int(r["end"][0]), int(r["end"][1]))
                except (KeyError, IndexError, TypeError, ValueError):
                    continue
                reg = regions.setdefault(key, {"functions": set(), "covered_by": set(), "seen_by": set()})
                reg["functions"].add(fn)
                reg["seen_by"].add(name)
                h["seen"] += 1
                if str(c.get("status", "")).upper() == "COVERED":
                    reg["covered_by"].add(name)
                    h["covered"] += 1
    return regions, harnesses


# ----------------------------------------------------------------- mutants

def load_mutants(mut_root):
    """Returns (mutants, shards): mutants keyed by name with per-pass outcomes."""
    mutants = {}
    shards = defaultdict(dict)  # pass -> shard -> baseline summary
    for dirpath, _, files in os.walk(mut_root):
        if "outcomes.json" not in files:
            continue
        m = re.search(r"mutants-(proofs|tests)-(\d+)", dirpath)
        if not m:
            continue
        pas, shard = m.group(1), int(m.group(2))
        try:
            data = json.load(open(os.path.join(dirpath, "outcomes.json")))
        except (OSError, ValueError):
            shards[pas][shard] = "unreadable"
            continue
        shards[pas][shard] = "no baseline"
        for o in data.get("outcomes", []):
            sc = o.get("scenario")
            if sc == "Baseline" or (isinstance(sc, dict) and "Baseline" in sc):
                shards[pas][shard] = o.get("summary", "?")
                continue
            if not (isinstance(sc, dict) and "Mutant" in sc):
                continue
            mu = sc["Mutant"]
            name = mu.get("name") or json.dumps(mu, sort_keys=True)
            e = mutants.setdefault(name, {"name": name, "file": mu.get("file"),
                                          "function": (mu.get("function") or {}).get("function_name"),
                                          "function_span": (mu.get("function") or {}).get("span"),
                                          "span": mu.get("span"), "replacement": mu.get("replacement"),
                                          "genre": mu.get("genre"), "outcomes": {}})
            e["outcomes"][pas] = o.get("summary")
    return mutants, shards


# ------------------------------------------------------------------- join

def pos(lc):
    return (int(lc["line"]), int(lc["column"]))


def index_regions(regions):
    by_file = defaultdict(list)
    for key, reg in regions.items():
        f, sl, sc, el, ec = key
        by_file[f].append(((sl, sc), (el, ec), reg, key))
    for lst in by_file.values():
        lst.sort(key=lambda t: (t[0], t[1]))
    return by_file


def in_span(start, end, fspan):
    if not fspan:
        return True
    return pos(fspan["start"]) <= start and end <= pos(fspan["end"])


def mutant_coverage(mu, by_file):
    """(state, harnesses) where state is covered, not-executed or no-proof."""
    regs = [t for t in by_file.get(mu["file"], []) if in_span(t[0], t[1], mu["function_span"])]
    if not regs:
        return "no-proof", []
    p = pos(mu["span"]["start"]) if mu.get("span") else regs[0][0]
    containing = [t for t in regs if t[0] <= p <= t[1]]
    if containing:
        def size(t):
            return (t[1][0] - t[0][0], t[1][1] - t[0][1] if t[1][0] == t[0][0] else t[1][1])
        best = min(size(t) for t in containing)
        chosen = [t for t in containing if size(t) == best]
    else:
        before = [t for t in regs if t[0] <= p]
        chosen = [max(before, key=lambda t: t[0])] if before else [regs[0]]
    by = set()
    for t in chosen:
        by |= t[2]["covered_by"]
    return ("covered" if by else "not-executed"), sorted(by)


def classify(mu):
    po, to = mu["outcomes"].get("proofs"), mu["outcomes"].get("tests")
    if "Unviable" in (po, to) and not (po in CAUGHT or to in CAUGHT):
        return "unviable"
    if po is None or to is None:
        return "incomplete"
    if po in CAUGHT:
        return "proved"
    if mu["coverage"] == "covered":
        return "proof-reach"
    return "tests-only" if to in CAUGHT else "unchecked"


def short_fn(mu):
    return f'{mu["file"]}: {mu["function"] or "(outside functions)"}'


def line_of(mu):
    return pos(mu["span"]["start"])[0] if mu.get("span") else 0


def main(inp, outdir):
    root = os.getcwd()
    os.makedirs(outdir, exist_ok=True)
    regions, harnesses = load_coverage(os.path.join(inp, "coverage"), root)
    mutants, shards = load_mutants(os.path.join(inp, "mutants"))
    by_file = index_regions(regions)

    for mu in mutants.values():
        mu["coverage"], mu["covered_by"] = mutant_coverage(mu, by_file)
        mu["class"] = classify(mu)
        mu["tests_caught"] = mu["outcomes"].get("tests") in CAUGHT

    viable = [m for m in mutants.values() if m["class"] not in ("unviable",)]
    counts = defaultdict(int)
    for m in viable:
        counts[m["class"]] += 1

    # Functions: reached by a proof if any of their regions is covered.
    functions = {}
    for m in mutants.values():
        if not m["function"]:
            continue
        fs = m["function_span"]
        k = (m["file"], m["function"], pos(fs["start"]) if fs else (0, 0))
        f = functions.setdefault(k, {"file": m["file"], "function": m["function"], "span": fs,
                                     "mutants": [], "regions": 0, "covered": 0, "uncovered": []})
        f["mutants"].append(m)
    for f in functions.values():
        for (s, e, reg, _) in by_file.get(f["file"], []):
            if in_span(s, e, f["span"]):
                f["regions"] += 1
                if reg["covered_by"]:
                    f["covered"] += 1
                else:
                    f["uncovered"].append((s, e))

    # Per file.
    files = defaultdict(lambda: defaultdict(int))
    for m in viable:
        files[m["file"]][m["class"]] += 1
        files[m["file"]]["total"] += 1
    for f, lst in by_file.items():
        files[f]["regions"] = len(lst)
        files[f]["covered_regions"] = sum(1 for t in lst if t[2]["covered_by"])

    classes = ["proved", "proof-reach", "tests-only", "unchecked", "incomplete"]
    total = len(viable)

    def pct(a, b):
        return f"{100 * a / b:.0f}%" if b else "-"

    s = []
    s.append("# Assurance report\n")
    s.append(f"{total} viable mutants ({sum(1 for m in mutants.values() if m['class'] == 'unviable')} "
             f"more did not compile). {len(harnesses)} proof harnesses with coverage data "
             f"({sum(1 for h in harnesses.values() if h['source'])} recorded).\n")
    s.append("| class | mutants | share | meaning |")
    s.append("|---|---:|---:|---|")
    meaning = {
        "proved": "caught by the proofs (sampled, so the real proofs catch them too)",
        "proof-reach": "a proof executes the code but sampling missed it: re-check with real Kani",
        "tests-only": "no proof executes the code; the tests catch it",
        "unchecked": "nothing catches it: a gap, or a change with no observable effect",
        "incomplete": "a shard did not finish",
    }
    for c in classes:
        if counts[c] or c != "incomplete":
            s.append(f"| {c} | {counts[c]} | {pct(counts[c], total)} | {meaning[c]} |")
    pr_t = sum(1 for m in viable if m["class"] == "proof-reach" and m["tests_caught"])
    s.append(f"\nProof-reach mutants caught by the tests: {pr_t}; "
             f"caught by nothing: {counts['proof-reach'] - pr_t}.\n")
    s.append("## By file\n")
    s.append("| file | mutants | proved | proof-reach | tests-only | unchecked | proof coverage (regions) |")
    s.append("|---|---:|---:|---:|---:|---:|---:|")
    for f in sorted(files):
        d = files[f]
        s.append(f"| {f} | {d['total']} | {d['proved']} | {d['proof-reach']} | {d['tests-only']} | "
                 f"{d['unchecked']} | {d['covered_regions']}/{d['regions']} {pct(d['covered_regions'], d['regions'])} |")
    s.append("\n## Shards\n")
    for pas in ("proofs", "tests"):
        got = shards.get(pas, {})
        bad = {k: v for k, v in got.items() if v != "Success"}
        missing = sorted(set(range(16)) - set(got))
        s.append(f"- {pas}: {len(got)} shards reported"
                 + (f"; baseline not successful in {bad}" if bad else "")
                 + (f"; missing {missing}" if missing else ""))
    no_cov = sorted(n for n, h in harnesses.items() if not h["source"])
    if no_cov:
        s.append(f"- harnesses without coverage data (timed out or failed to run): {', '.join(no_cov)}")
    summary = "\n".join(s) + "\n"

    r = [summary]

    def group(cls, extra=None):
        by = defaultdict(list)
        for m in viable:
            if m["class"] == cls and (extra is None or extra(m)):
                by[short_fn(m)].append(m)
        return sorted(by.items(), key=lambda kv: (-len(kv[1]), kv[0]))

    def emit(title, intro, groups, show_harness=False):
        n = sum(len(v) for _, v in groups)
        r.append(f"\n## {title} ({n})\n")
        r.append(intro + "\n")
        for fn, lst in groups:
            r.append(f"- **{fn}** ({len(lst)})")
            for m in sorted(lst, key=line_of):
                h = ""
                if show_harness and m["covered_by"]:
                    hs = [x.rsplit("::", 1)[-1] for x in m["covered_by"]]
                    h = f" — reached by {', '.join(hs[:4])}{' …' if len(hs) > 4 else ''}"
                r.append(f"  - `{m['name']}`{h}")

    emit("Priority 1: caught by nothing", "Neither the proofs nor the tests notice these changes. "
         "Each is either a missing check or a change with no observable effect (an equivalent mutant).",
         group("unchecked") + group("proof-reach", lambda m: not m["tests_caught"]), show_harness=True)
    emit("Priority 2: executed by proofs, caught only by tests",
         "A proof runs this code but its property did not fail under sampling. Real Kani may still "
         "catch these; any it does not mark a property that does not pin the behaviour down.",
         group("proof-reach", lambda m: m["tests_caught"]), show_harness=True)
    emit("Priority 3: no proof executes this code", "Only the tests protect these.", group("tests-only"))

    r.append("\n## Functions no proof enters\n")
    for f in sorted(functions.values(), key=lambda f: (f["file"], f["function"])):
        if f["covered"] == 0:
            r.append(f"- {f['file']}: {f['function']} ({len(f['mutants'])} mutants)")
    r.append("\n## Lines no proof executes, inside functions that proofs enter\n")
    for f in sorted(functions.values(), key=lambda f: (f["file"], f["function"])):
        if f["covered"] and f["uncovered"]:
            spans = ", ".join(f"{s[0]}" + (f"-{e[0]}" if e[0] != s[0] else "") for s, e in f["uncovered"])
            r.append(f"- {f['file']}: {f['function']}: lines {spans}")
    r.append("\n## Harnesses\n")
    r.append("| harness | result | regions covered / instrumented (code under test) |")
    r.append("|---|---|---:|")
    for n in sorted(harnesses):
        h = harnesses[n]
        r.append(f"| {n} | {h['status']} | {h['covered']}/{h['seen']} |")

    open(os.path.join(outdir, "summary.md"), "w").write(summary)
    open(os.path.join(outdir, "report.md"), "w").write("\n".join(r) + "\n")
    js = {
        "counts": dict(counts),
        "mutants": [{k: v for k, v in m.items()} for m in sorted(mutants.values(), key=lambda m: m["name"])],
        "harnesses": harnesses,
        "regions": [{"file": k[0], "start": [k[1], k[2]], "end": [k[3], k[4]],
                     "functions": sorted(v["functions"]), "covered_by": sorted(v["covered_by"]),
                     "seen_by": sorted(v["seen_by"])} for k, v in sorted(regions.items())],
        "shards": shards,
    }
    json.dump(js, open(os.path.join(outdir, "assurance.json"), "w"), indent=1, default=sorted)
    cands = [{"name": m["name"], "file": m["file"], "span": m["span"], "replacement": m["replacement"],
              "harnesses": m["covered_by"], "tests_caught": m["tests_caught"]}
             for m in sorted(viable, key=lambda m: m["name"]) if m["class"] == "proof-reach"]
    json.dump(cands, open(os.path.join(outdir, "kani_candidates.json"), "w"), indent=1)
    print(summary)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1], sys.argv[2]))
