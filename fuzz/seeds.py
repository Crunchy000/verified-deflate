#!/usr/bin/env python3
"""Writes starting inputs for the fuzz targets that read streams.

For `inflate`: raw DEFLATE streams from zlib at every level and strategy,
of text, binary, repetitive and random data. For `zip_read`: archives from
Python's zipfile, stored and deflated, with directories, a non-ASCII name,
an archive comment and an empty archive. If the `inflate64` package is
installed, also Deflate64 streams for `inflate64`, and archives with
Deflate64 (method 9) entries for `zip_read`. The other targets build their
own inputs. Files go to fuzz/corpus/<target>/, named by content, so running
this again adds nothing new.

usage: seeds.py [fuzz directory]
"""
import hashlib
import io
import os
import random
import struct
import sys
import zipfile
import zlib

try:
    import inflate64
except ImportError:
    inflate64 = None


def put(root, target, data):
    d = os.path.join(root, "corpus", target)
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, hashlib.sha1(data).hexdigest()), "wb") as f:
        f.write(data)


def handmade_zip(entries):
    """A ZIP archive of (name, method, compressed, raw) entries."""
    out, central = bytearray(), bytearray()
    for name, method, comp, raw in entries:
        offset, crc = len(out), zlib.crc32(raw)
        out += struct.pack("<IHHHHHIIIHH", 0x04034B50, 20, 0, method, 0, 0x21, crc, len(comp), len(raw), len(name), 0)
        out += name + comp
        central += struct.pack(
            "<IHHHHHHIIIHHHHHII", 0x02014B50, 20, 20, 0, method, 0, 0x21, crc, len(comp), len(raw), len(name),
            0, 0, 0, 0, 0, offset,
        )
        central += name
    cd = len(out)
    out += central
    out += struct.pack("<IHHHHIIH", 0x06054B50, 0, 0, len(entries), len(entries), len(central), cd, 0)
    return bytes(out)


def deflate64(data):
    c = inflate64.Deflater()
    return c.deflate(data) + c.flush()


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

    if inflate64 is None:
        print("inflate64 not installed: no Deflate64 seeds")
        return
    # A block that recurs 41000 and 60000 bytes later (distance codes 30
    # and 31), and a run long enough for length code 285's extra bits.
    block = bytes(rng.randrange(256) for _ in range(1000))
    far = block + b"\0" * 40000 + block + b"\1" * 18000 + block
    for data in samples + [far, b"\0" * 70000]:
        put(root, "inflate64", deflate64(data))
    entries = [(b"fox.txt", 9, deflate64(samples[4]), samples[4]), (b"far", 9, deflate64(far), far)]
    put(root, "zip_read", handmade_zip(entries))


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.abspath(__file__)))
