#!/usr/bin/env python3
"""Writes starting inputs for the fuzz targets that read streams.

For `inflate`: raw DEFLATE streams from zlib at every level and strategy,
of text, binary, repetitive and random data. For `zip_read`: archives from
Python's zipfile, stored and deflated, with directories, a non-ASCII name,
an archive comment and an empty archive. For `gzip_zlib`: gzip and zlib
streams, with header fields and two members. For `tar_read`: archives in
each of tarfile's formats. For `bzip2_decode`: bzip2 streams at levels 1,
5 and 9, and two streams concatenated, and for `zip_read` an archive with
bzip2 (method 12) entries. For `lzma_decode`: raw LZMA1 and LZMA2 streams,
`.lzma` files and `.xz` files with each check and with filters, each
behind the byte that selects its form, and for `zip_read` an archive with
LZMA (method 14) entries. If the `inflate64` package is
installed, also Deflate64 streams for `inflate64`, and archives with
Deflate64 (method 9) entries for `zip_read`. The other targets build their
own inputs. Files go to fuzz/corpus/<target>/, named by content, so running
this again adds nothing new.

usage: seeds.py [fuzz directory]
"""
import bz2
import hashlib
import io
import lzma
import os
import random
import gzip
import struct
import sys
import tarfile
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

    # gzip (with header fields, and two members) and zlib streams.
    for data in samples[:6]:
        put(root, "gzip_zlib", gzip.compress(data, 6, mtime=0))
        put(root, "gzip_zlib", zlib.compress(data, 6))
    buf = io.BytesIO()
    with gzip.GzipFile(filename="name.txt", mode="wb", fileobj=buf, mtime=1) as g:
        g.write(samples[4])
    buf.write(gzip.compress(b"second member", mtime=0))
    put(root, "gzip_zlib", buf.getvalue())

    # bzip2 streams, and bzip2 in a zip.
    for data in samples:
        for level in (1, 5, 9):
            put(root, "bzip2_decode", bz2.compress(data, level))
    put(root, "bzip2_decode", bz2.compress(samples[2], 1) + bz2.compress(samples[4], 9))
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_BZIP2) as z:
        z.writestr("fox.txt", samples[4])
        z.writestr("random", samples[5])
    put(root, "zip_read", buf.getvalue())

    # LZMA in each form, behind the byte lzma_decode reads to pick it.
    lzma1 = [{"id": lzma.FILTER_LZMA1, "lc": 3, "lp": 0, "pb": 2, "dict_size": 1 << 16}]
    lzma2 = [{"id": lzma.FILTER_LZMA2, "dict_size": 1 << 16}]
    for data in samples:
        put(root, "lzma_decode", b"\x00\x5d" + lzma.compress(data, format=lzma.FORMAT_RAW, filters=lzma1))
        put(root, "lzma_decode", b"\x01\x00" + lzma.compress(data, format=lzma.FORMAT_RAW, filters=lzma2))
        put(root, "lzma_decode", b"\x02" + lzma.compress(data, format=lzma.FORMAT_ALONE))
        put(root, "lzma_decode", b"\x03" + lzma.compress(data, check=lzma.CHECK_CRC32, preset=1))
    for check in (lzma.CHECK_NONE, lzma.CHECK_CRC64, lzma.CHECK_SHA256):
        put(root, "lzma_decode", b"\x03" + lzma.compress(samples[4], check=check))
    for f in (lzma.FILTER_X86, lzma.FILTER_ARM, lzma.FILTER_DELTA):
        put(root, "lzma_decode", b"\x03" + lzma.compress(samples[5], filters=[{"id": f}, {"id": lzma.FILTER_LZMA2, "preset": 1}]))
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_LZMA) as z:
        z.writestr("fox.txt", samples[4])
        z.writestr("random", samples[5])
    put(root, "zip_read", buf.getvalue())

    # Tar in each of tarfile's formats: a directory, files, links, and a
    # name long enough for the format's extension.
    for fmt in (tarfile.USTAR_FORMAT, tarfile.GNU_FORMAT, tarfile.PAX_FORMAT):
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode="w", format=fmt) as t:
            for name, kind, data, link in [
                ("dir", tarfile.DIRTYPE, b"", ""),
                ("dir/a.txt", tarfile.REGTYPE, samples[2], ""),
                ("dir/link", tarfile.SYMTYPE, b"", "a.txt"),
                ("hard", tarfile.LNKTYPE, b"", "dir/a.txt"),
                ("p" * 120 + "/" + "n" * 60, tarfile.REGTYPE, b"x", ""),
            ]:
                ti = tarfile.TarInfo(name)
                ti.type, ti.linkname, ti.size, ti.mtime = kind, link, len(data), 1700000000
                t.addfile(ti, io.BytesIO(data))
        put(root, "tar_read", buf.getvalue())

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
