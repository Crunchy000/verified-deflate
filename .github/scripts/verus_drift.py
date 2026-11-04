#!/usr/bin/env python3
"""Checks that each function verified by Verus is the production function.

The Verus proofs in verus/ verify copies of production functions, with
specifications and proof code added. This script removes the proof code
from each copy and checks that what remains is, token for token, the
production function, so a proof cannot silently drift away from the code
it is about.

Proof code in a copy is marked line by line: a line ending in `// @ghost`
is proof code, and so is every line from one containing `// @ghost-begin`
to one containing `// @ghost-end`. A named return value (`-> (r: T)`,
which Verus needs for postconditions) counts as `-> T`. Comments and
layout do not matter.

The pairs to check are listed in verus/copies.txt, one per line:
    <production file> <item> <verus file>
where <item> is a function (`name`), a method (`Type::name`, looked up in
the `impl` blocks for `Type`), or a type or constant (`struct:Name`,
`enum:Name`, `const:NAME`, `const:Type::NAME`, `static:NAME`). A method of a
trait impl is looked up as `Type::name` too. Attributes and visibility
before an item are not compared. A constant that Verus needs as
`exec const NAME: T ensures ... { body }` counts as `const NAME: T = body;`,
and likewise for `exec static`.

usage: verus_drift.py [repository root]
"""
import os
import re
import sys

TOKEN = re.compile(
    r"""
    (?P<ws>\s+)
  | (?P<comment>//[^\n]*|/\*.*?\*/)
  | (?P<str>b?"(?:\\.|[^"\\])*")
  | (?P<char>b?'(?:\\.|[^'\\])')
  | (?P<lifetime>'[A-Za-z_][A-Za-z0-9_]*)
  | (?P<num>0[xob][0-9A-Fa-f_]+[a-z0-9]*|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?[a-z0-9]*)
  | (?P<ident>[A-Za-z_][A-Za-z0-9_]*)
  | (?P<punct><<=|>>=|\.\.=|\.\.\.|::|->|=>|==|!=|<=|>=|&&|\|\||<<|>>|\+=|-=|\*=|/=|%=|\^=|&=|\|=|\.\.|[{}()\[\];:,.<>=!+\-*/%^&|@#?$~])
    """,
    re.S | re.X,
)


def tokens(src):
    out, pos = [], 0
    while pos < len(src):
        m = TOKEN.match(src, pos)
        if not m:
            raise ValueError(f"cannot tokenize at: {src[pos:pos + 40]!r}")
        pos = m.end()
        if m.lastgroup not in ("ws", "comment"):
            out.append(m.group())
    return out


def strip_ghost(src):
    keep, ghost = [], False
    for line in src.split("\n"):
        if "// @ghost-begin" in line:
            ghost = True
            continue
        if "// @ghost-end" in line:
            ghost = False
            continue
        if ghost or line.rstrip().endswith("// @ghost"):
            continue
        keep.append(line)
    if ghost:
        raise ValueError("// @ghost-begin without // @ghost-end")
    return "\n".join(keep)


def unname_return(toks):
    """`-> (r: T)` becomes `-> T`."""
    out, i = [], 0
    while i < len(toks):
        if toks[i] == "->" and toks[i + 1:i + 2] == ["("] and i + 3 < len(toks) and toks[i + 3] == ":":
            depth, j = 1, i + 4
            while depth:
                depth += {"(": 1, ")": -1}.get(toks[j], 0)
                j += 1
            out.append("->")
            out.extend(toks[i + 4:j - 1])
            i = j
        else:
            out.append(toks[i])
            i += 1
    return out


OPEN = {"{": 1, "(": 1, "[": 1, "}": -1, ")": -1, "]": -1}


def block_end(toks, j):
    """Index of the brace closing the block that opens at toks[j]."""
    depth = 0
    for k in range(j, len(toks)):
        depth += {"{": 1, "}": -1}.get(toks[k], 0)
        if depth == 0:
            return k
    raise ValueError("unbalanced braces")


def impl_ranges(toks, ty):
    """(start, end) token ranges of the `impl` blocks for `ty`, inherent or
    trait impls, in order."""
    out = []
    for i, t in enumerate(toks):
        if t != "impl":
            continue
        j = i + 1
        if toks[j] == "<":
            depth = 0
            while True:
                depth += {"<": 1, ">": -1}.get(toks[j], 0)
                j += 1
                if depth == 0:
                    break
        k = j
        while toks[k] not in ("{", "for"):
            k += 1
        self_ty = toks[k + 1] if toks[k] == "for" else toks[j]
        while toks[k] != "{":
            k += 1
        if self_ty == ty:
            out.append((k, block_end(toks, k)))
    if not out:
        raise ValueError(f"impl {ty} not found")
    return out


def scopes(toks, name):
    """The token ranges to look for `name` in: the whole file, or for
    `Type::name` each `impl` block for `Type`."""
    if "::" in name:
        ty, name = name.split("::")
        return [toks[a:b + 1] for a, b in impl_ranges(toks, ty)], name
    return [toks], name


def function(toks, name):
    """Tokens of `fn name` from `fn` to the brace closing its body."""
    blocks, name = scopes(toks, name)
    for toks in blocks:
        for i in range(len(toks) - 1):
            if toks[i] == "fn" and toks[i + 1] == name:
                j = i
                while toks[j] != "{":
                    j += 1
                return toks[i:block_end(toks, j) + 1]
    raise ValueError(f"fn {name} not found")


def type_item(toks, kind, name):
    """Tokens of `struct Name ...` or `enum Name ...` to its closing brace."""
    for i in range(len(toks) - 1):
        if toks[i] == kind and toks[i + 1] == name:
            j = i
            while toks[j] not in ("{", ";"):
                j += 1
            return toks[i:(block_end(toks, j) if toks[j] == "{" else j) + 1]
    raise ValueError(f"{kind} {name} not found")


def const_item(toks, name, keyword="const"):
    """Tokens of `const NAME: T = value;` (or `static`), reading the Verus
    form `exec const NAME: T { value }` as the same thing."""
    blocks, name = scopes(toks, name)
    for toks in blocks:
        for i in range(len(toks) - 1):
            if toks[i] == keyword and toks[i + 1] == name and (i == 0 or toks[i - 1] != "fn"):
                j, depth = i, 0
                while depth or toks[j] not in ("=", "{"):
                    depth += OPEN.get(toks[j], 0)
                    j += 1
                if toks[j] == "=":
                    k, depth = j, 0
                    while depth or toks[k] != ";":
                        depth += OPEN.get(toks[k], 0)
                        k += 1
                    return toks[i:k + 1]
                k = block_end(toks, j)
                return toks[i:j] + ["="] + toks[j + 1:k] + [";"]
    raise ValueError(f"{keyword} {name} not found")


def item(toks, spec):
    kind, sep, name = spec.partition(":")
    if sep and name.startswith(":"):  # `Type::name`, a method
        kind = ""
    if kind in ("struct", "enum"):
        return type_item(toks, kind, name)
    if kind in ("const", "static"):
        return const_item(toks, name, kind)
    return function(toks, spec)


def main(root):
    pairs = []
    for line in open(os.path.join(root, "verus/copies.txt")):
        line = line.split("#", 1)[0].strip()
        if line:
            pairs.append(line.split())
    bad = 0
    for prod_file, name, verus_file in pairs:
        prod = item(tokens(open(os.path.join(root, prod_file)).read()), name)
        copy_toks = unname_return(tokens(strip_ghost(open(os.path.join(root, verus_file)).read())))
        copy = item([t for i, t in enumerate(copy_toks) if not (t == "exec" and copy_toks[i + 1:i + 2] in (["const"], ["static"]))], name)
        if prod == copy:
            print(f"ok      {name}: {verus_file} matches {prod_file} ({len(prod)} tokens)")
            continue
        bad += 1
        i = next((k for k in range(min(len(prod), len(copy))) if prod[k] != copy[k]), min(len(prod), len(copy)))
        print(f"DRIFT   {name}: {verus_file} differs from {prod_file} at token {i}")
        print("  production: " + " ".join(prod[max(0, i - 12):i + 12]))
        print("  verus copy: " + " ".join(copy[max(0, i - 12):i + 12]))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else "."))
