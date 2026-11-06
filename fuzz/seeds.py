#!/usr/bin/env python3
"""Writes starting inputs for the fuzz targets that read streams.

For `inflate`: raw DEFLATE streams from zlib at every level and strategy,
of text, binary, repetitive and random data. For `zip_read`: archives from
Python's zipfile, stored and deflated, with directories, a non-ASCII name,
an archive comment and an empty archive. The other targets build their own
inputs. Files go to fuzz/corpus/<target>/, named by content, so running
this again adds nothing new.

usage: seeds.py [fuzz directory]
"""
import hashlib
import io
import os
import random
import sys
import zipfile
import zlib


def put(root, target, data):
    d = os.path.join(root, "corpus", target)
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, hashlib.sha1(data).hexdigest()), "wb") as f:
        f.write(data)


def main(root):
    rng = random.Random(1951)
    samples = [
        b"",
        b"a",
        b"hello hello hello hello hello",
        bytes(range(256)) * 3,
        b"The quick brown fox jumps over the lazy dog. " * 40,
        bytes(rng.randrange(256) for _ in range(3000)),
        bytes(rng.choice(b"ab") for _ in range(3000)),
        b"\0" * 5000,
    ]
    strategies = [zlib.Z_DEFAULT_STRATEGY, zlib.Z_FILTERED, zlib.Z_HUFFMAN_ONLY, zlib.Z_RLE, zlib.Z_FIXED]
    for data in samples:
        for level in range(10):
            for strategy in strategies:
                c = zlib.compressobj(level, zlib.DEFLATED, -15, 8, strategy)
                put(root, "inflate", c.compress(data) + c.flush())

    for method in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED):
        buf = io.BytesIO()
        with zipfile.ZipFile(buf, "w", method) as z:
            z.writestr("a.txt", samples[2])
            z.writestr("dir/", b"")
            z.writestr("dir/fox.txt", samples[4])
            z.writestr("dir/zeros", samples[7])
            z.writestr("café.bin", samples[5][:200])
            z.comment = b"archive comment"
        put(root, "zip_read", buf.getvalue())
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w"):
        pass
    put(root, "zip_read", buf.getvalue())


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.abspath(__file__)))
