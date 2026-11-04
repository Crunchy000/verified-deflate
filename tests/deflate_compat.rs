//! The `deflate` crate's API: code written against `deflate` 1.0 compiles
//! against this crate imported as `deflate`, and everything it produces is
//! accepted by the reference zlib and gzip (through Python), including
//! streams cut at every sync flush.

mod common;

extern crate verified_deflate as deflate;

use std::fs;
use std::io::{self, Write};
use std::process::Command;

use deflate::gzip_header::{ExtraFlags, FileSystemType, GzBuilder};
use deflate::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
use deflate::{
    deflate_bytes, deflate_bytes_conf, deflate_bytes_gzip, deflate_bytes_gzip_conf, deflate_bytes_zlib,
    deflate_bytes_zlib_conf, Compression, CompressionOptions, MatchingType, SpecialOptions,
};

/// Every item and signature of `deflate` 1.0, spelled with explicit types
/// so that any difference fails to compile.
#[test]
fn api_matches_deflate_1_0() {
    let f: fn(&[u8]) -> Vec<u8> = deflate_bytes;
    let _ = f(b"x");
    let _: fn(&[u8], Compression) -> Vec<u8> = deflate_bytes_conf::<Compression>;
    let _: fn(&[u8], CompressionOptions) -> Vec<u8> = deflate_bytes_conf::<CompressionOptions>;
    let _: fn(&[u8]) -> Vec<u8> = deflate_bytes_zlib;
    let _: fn(&[u8], Compression) -> Vec<u8> = deflate_bytes_zlib_conf::<Compression>;
    let _: fn(&[u8]) -> Vec<u8> = deflate_bytes_gzip;
    let _: fn(&[u8], Compression, GzBuilder) -> Vec<u8> = deflate_bytes_gzip_conf::<Compression>;

    // Options: public fields, const constructors, conversions, derives.
    const HIGH: CompressionOptions = CompressionOptions::high();
    const FAST: CompressionOptions = CompressionOptions::fast();
    const HUFF: CompressionOptions = CompressionOptions::huffman_only();
    const RLE: CompressionOptions = CompressionOptions::rle();
    let custom = CompressionOptions {
        max_hash_checks: 300,
        lazy_if_less_than: 64,
        matching_type: MatchingType::Lazy,
        special: SpecialOptions::Normal,
    };
    assert_eq!(CompressionOptions::from(Compression::Best), HIGH);
    assert_eq!(CompressionOptions::from(Compression::Fast), FAST);
    assert_eq!(CompressionOptions::from(Compression::default()), CompressionOptions::default());
    assert_eq!((HIGH.max_hash_checks, HIGH.lazy_if_less_than), (1768, 128));
    assert_eq!((FAST.max_hash_checks, FAST.matching_type), (1, MatchingType::Greedy));
    assert_eq!((HUFF.max_hash_checks, HUFF.matching_type), (0, MatchingType::Greedy));
    assert_eq!((RLE.max_hash_checks, RLE.matching_type), (0, MatchingType::Lazy));
    let d = CompressionOptions::default();
    assert_eq!(
        (d.max_hash_checks, d.lazy_if_less_than, d.matching_type, d.special),
        (128, 32, MatchingType::Lazy, SpecialOptions::Normal)
    );
    assert!(Compression::Fast < Compression::Best && MatchingType::Greedy < MatchingType::Lazy);
    assert_eq!(SpecialOptions::default(), SpecialOptions::Normal);
    let _ = (SpecialOptions::_ForceFixed, SpecialOptions::_ForceStored, custom);
    assert_eq!(MatchingType::Greedy.to_string(), "Greedy matching");
    assert_eq!(MatchingType::Lazy.to_string(), "Lazy matching");
    let mut set = std::collections::HashSet::new();
    set.insert(custom);
    set.insert(Compression::Default.into());

    // Writers.
    let mut e: DeflateEncoder<Vec<u8>> = DeflateEncoder::new(Vec::new(), Compression::Default);
    e.write_all(b"abc").unwrap();
    e.flush().unwrap();
    let first: Vec<u8> = e.reset(Vec::new()).unwrap();
    let _: io::Result<Vec<u8>> = e.finish();
    assert_eq!(deflate::decompress(&first).unwrap(), b"abc");

    let mut z = ZlibEncoder::new(Vec::new(), CompressionOptions::rle());
    z.write_all(b"abc").unwrap();
    let _: u32 = z.checksum();
    let _: io::Result<Vec<u8>> = z.reset(Vec::new());
    let _: io::Result<Vec<u8>> = z.finish();

    let b: GzBuilder = GzBuilder::new().filename("f").comment("c").extra(vec![1]).mtime(1).os(FileSystemType::Unix);
    let mut g = GzEncoder::from_builder(b, Vec::new(), Compression::Fast);
    g.write_all(b"abc").unwrap();
    let _: u32 = g.checksum();
    let _: io::Result<Vec<u8>> = g.reset_with_builder(Vec::new(), GzBuilder::new().xfl(ExtraFlags::FastestCompression));
    let _: io::Result<Vec<u8>> = g.reset(Vec::new());
    let g2: GzEncoder<Vec<u8>> = GzEncoder::new(Vec::new(), CompressionOptions::default());
    let _ = g2.finish().unwrap();
}

/// The option presets exercised everywhere below.
fn presets() -> Vec<(&'static str, CompressionOptions)> {
    let co = CompressionOptions::default();
    vec![
        ("default", co),
        ("fast", CompressionOptions::fast()),
        ("high", CompressionOptions::high()),
        ("huffman", CompressionOptions::huffman_only()),
        ("rle", CompressionOptions::rle()),
        ("fixed", CompressionOptions { special: SpecialOptions::_ForceFixed, ..co }),
        ("stored", CompressionOptions { special: SpecialOptions::_ForceStored, ..co }),
        ("greedy300", CompressionOptions { max_hash_checks: 300, matching_type: MatchingType::Greedy, ..co }),
        ("alwayslazy", CompressionOptions { lazy_if_less_than: 258, ..co }),
        ("max", CompressionOptions { max_hash_checks: u16::MAX, lazy_if_less_than: u16::MAX, ..co }),
        ("lazy0", CompressionOptions { lazy_if_less_than: 0, ..co }),
    ]
}

/// Writes `data` to `w` in pseudo-random pieces, with a sync flush now and
/// then; returns (`out_len`, input length) at every flush, where `out_len`
/// reports the output written so far.
fn chunked<W: Write>(w: &mut W, data: &[u8], seed: u64, out_len: impl Fn(&W) -> usize) -> Vec<(usize, usize)> {
    let mut rng = common::Rng::new(seed);
    let mut flushes = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let n = (1 + rng.below(60_000) as usize).min(data.len() - pos);
        w.write_all(&data[pos..pos + n]).unwrap();
        pos += n;
        if flushes.is_empty() || rng.below(3) == 0 {
            w.flush().unwrap();
            flushes.push((out_len(w), pos));
        }
    }
    flushes
}

const CHECK: &str = r#"
import sys, os, zlib, gzip
d = sys.argv[1]
bad = 0
names = sorted(n[:-4] for n in os.listdir(d) if n.endswith('.raw'))
for n in names:
    data = open(os.path.join(d, n + '.raw'), 'rb').read()
    for f in sorted(os.listdir(d)):
        if not f.startswith(n + '.') or f.endswith('.raw') or f.endswith('.flushes'):
            continue
        s = open(os.path.join(d, f), 'rb').read()
        kind = f.split('.')[-1]
        try:
            if kind == 'deflate':
                o = zlib.decompressobj(-15); out = o.decompress(s) + o.flush(); ok = o.eof and not o.unused_data
            elif kind == 'zlib':
                o = zlib.decompressobj(15); out = o.decompress(s) + o.flush(); ok = o.eof and not o.unused_data
            else:
                out = gzip.decompress(s); ok = True
        except Exception as e:
            print('ERROR', f, e); bad += 1; continue
        if out != data or not ok:
            print('MISMATCH', f); bad += 1
        # Every sync flush point: the output so far decodes to the input so far.
        fl = os.path.join(d, f + '.flushes')
        if os.path.exists(fl):
            wbits = {'deflate': -15, 'zlib': 15, 'gz': 31}[kind]
            for line in open(fl):
                o_len, i_len = map(int, line.split())
                o = zlib.decompressobj(wbits)
                got = o.decompress(s[:o_len])
                if got != data[:i_len]:
                    print('FLUSH MISMATCH', f, o_len, i_len, len(got)); bad += 1
print('checked', len(names), 'inputs; bad', bad)
sys.exit(1 if bad else 0)
"#;

#[test]
fn reference_zlib_and_gzip_accept_everything() {
    if !common::have("python3", "--version") {
        eprintln!("python3 not found; skipping");
        return;
    }
    let dir = common::temp_dir("deflate-compat");
    let mut corpus = common::corpus();
    // Long enough to cross the writers' 256 KiB chunks several times.
    let mut rng = common::Rng::new(7);
    let mut big = Vec::new();
    while big.len() < 900_000 {
        let n = rng.below(4000) as usize;
        if rng.below(3) == 0 && big.len() > 40_000 {
            let start = big.len() - 1 - rng.below(32_000) as usize;
            for i in 0..n {
                big.push(big[start + i % 300]);
            }
        } else {
            big.extend(rng.bytes(n / 8));
        }
    }
    corpus.push(("mixed-900k".into(), big));
    let builder = || GzBuilder::new().filename("name.txt").comment("a comment").extra(&b"xx"[..]).mtime(1_700_000_000);
    for (name, data) in &corpus {
        fs::write(dir.join(format!("{name}.raw")), data).unwrap();
        for (p, opts) in presets() {
            fs::write(dir.join(format!("{name}.{p}.deflate")), deflate_bytes_conf(data, opts)).unwrap();
            fs::write(dir.join(format!("{name}.{p}.zlib")), deflate_bytes_zlib_conf(data, opts)).unwrap();
            fs::write(dir.join(format!("{name}.{p}.gz")), deflate_bytes_gzip_conf(data, opts, builder())).unwrap();
            assert_eq!(deflate::decompress(&deflate_bytes_conf(data, opts)).unwrap(), *data, "{name} {p}");

            // The same through the writers, in pieces with sync flushes.
            let seed = data.len() as u64 ^ p.len() as u64;
            let mut e = DeflateEncoder::new(Vec::new(), opts);
            chunked(&mut e, data, seed, |_| 0);
            let out = e.finish().unwrap();
            assert_eq!(deflate::decompress(&out).unwrap(), *data, "writer {name} {p}");
            fs::write(dir.join(format!("{name}.{p}.w.deflate")), &out).unwrap();
        }
    }
    // Sync-flush points, recorded with a writer that tracks its length.
    for (name, data) in corpus.iter().filter(|(n, _)| n == "mixed-900k" || n == "words-300k") {
        for (kind, idx) in [("deflate", 0), ("zlib", 1), ("gz", 2)] {
            let (out, flushes) = flushed_stream(idx, data);
            let file = format!("{name}.f.{kind}");
            fs::write(dir.join(&file), &out).unwrap();
            let lines: String = flushes.iter().map(|(o, i)| format!("{o} {i}\n")).collect();
            fs::write(dir.join(format!("{file}.flushes")), lines).unwrap();
            assert!(!flushes.is_empty());
        }
    }
    let script = dir.join("check.py");
    fs::write(&script, CHECK).unwrap();
    let out = Command::new("python3").arg("-I").arg(&script).arg(&dir).output().unwrap();
    eprintln!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "the reference zlib/gzip rejected some output");
    let _ = fs::remove_dir_all(&dir);
}

/// A writer whose output so far can be read while an encoder owns it.
#[derive(Clone, Default)]
struct Shared(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Encodes `data` in pieces with sync flushes; returns the stream and the
/// (output length, input length) pairs at each flush.
fn flushed_stream(kind: usize, data: &[u8]) -> (Vec<u8>, Vec<(usize, usize)>) {
    let s = Shared::default();
    let flushes = match kind {
        0 => {
            let mut e = DeflateEncoder::new(s.clone(), Compression::Default);
            let f = chunked(&mut e, data, 11, |_| s.0.borrow().len());
            e.finish().unwrap();
            f
        }
        1 => {
            let mut e = ZlibEncoder::new(s.clone(), Compression::Default);
            let f = chunked(&mut e, data, 12, |_| s.0.borrow().len());
            e.finish().unwrap();
            f
        }
        _ => {
            let mut e = GzEncoder::new(s.clone(), Compression::Best);
            let f = chunked(&mut e, data, 13, |_| s.0.borrow().len());
            e.finish().unwrap();
            f
        }
    };
    let out = s.0.borrow().clone();
    (out, flushes)
}

/// Splits a zlib stream into its DEFLATE body, checking header and trailer.
fn unzlib(s: &[u8], data: &[u8]) -> Vec<u8> {
    assert_eq!(&s[..2], &[0x78, 0x9c]);
    let body = &s[2..s.len() - 4];
    assert_eq!(&s[s.len() - 4..], &deflate::adler32(data).to_be_bytes());
    deflate::decompress(body).unwrap()
}

#[test]
fn zlib_and_gzip_framing() {
    let data = b"This is some test data, this is some test data.".repeat(50);
    assert_eq!(unzlib(&deflate_bytes_zlib(&data), &data), data);
    let g = deflate_bytes_gzip(&data);
    assert_eq!(&g[..4], &[0x1f, 0x8b, 8, 0]);
    let n = g.len();
    assert_eq!(&g[n - 8..n - 4], &deflate::crc32(&data).to_le_bytes());
    assert_eq!(&g[n - 4..], &(data.len() as u32).to_le_bytes());
    assert_eq!(deflate::decompress(&g[10..n - 8]).unwrap(), data);

    // Writers produce the same framing, and their checksums match.
    let mut z = ZlibEncoder::new(Vec::new(), Compression::Default);
    z.write_all(&data).unwrap();
    assert_eq!(z.checksum(), deflate::adler32(&data));
    assert_eq!(unzlib(&z.finish().unwrap(), &data), data);
    let mut ge = GzEncoder::new(Vec::new(), Compression::Default);
    ge.write_all(&data).unwrap();
    assert_eq!(ge.checksum(), deflate::crc32(&data));
    let out = ge.finish().unwrap();
    assert_eq!(&out[..10], &GzBuilder::new().into_header()[..]);
    assert_eq!(&out[out.len() - 8..], &g[n - 8..]);

    // Empty streams in every format.
    for o in [Compression::Fast, Compression::Default, Compression::Best] {
        assert_eq!(deflate::decompress(&deflate_bytes_conf(b"", o)).unwrap(), b"");
        assert_eq!(unzlib(&deflate_bytes_zlib_conf(b"", o), b""), b"");
        let out = GzEncoder::new(Vec::new(), o).finish().unwrap();
        assert_eq!(&out[out.len() - 8..], &[0, 0, 0, 0, 0, 0, 0, 0]);
    }
}

#[test]
fn dropping_an_encoder_finishes_the_stream() {
    let data = b"dropped, not finished ".repeat(1000);
    let mut out = Vec::new();
    {
        let mut e = DeflateEncoder::new(&mut out, Compression::Default);
        e.write_all(&data).unwrap();
    }
    assert_eq!(deflate::decompress(&out).unwrap(), data);
    let mut out = Vec::new();
    {
        let mut e = ZlibEncoder::new(&mut out, Compression::Default);
        e.write_all(&data).unwrap();
    }
    assert_eq!(unzlib(&out, &data), data);
    let mut out = Vec::new();
    {
        let mut e = GzEncoder::new(&mut out, Compression::Default);
        e.write_all(&data).unwrap();
    }
    assert_eq!(ungzip(&out, &data), data);
}

/// Checks the default 10-byte gzip header and the trailer, and returns the
/// decoded body.
fn ungzip(s: &[u8], data: &[u8]) -> Vec<u8> {
    assert_eq!(&s[..4], &[0x1f, 0x8b, 8, 0]);
    let n = s.len();
    assert_eq!(&s[n - 8..n - 4], &deflate::crc32(data).to_le_bytes());
    assert_eq!(&s[n - 4..], &(data.len() as u32).to_le_bytes());
    deflate::decompress(&s[10..n - 8]).unwrap()
}

#[test]
fn reset_starts_an_independent_stream() {
    let a = b"first stream ".repeat(100);
    let b = b"second stream, different ".repeat(100);
    let mut e = ZlibEncoder::new(Vec::new(), Compression::Best);
    e.write_all(&a).unwrap();
    let first = e.reset(Vec::new()).unwrap();
    e.write_all(&b).unwrap();
    assert_eq!(e.checksum(), deflate::adler32(&b));
    let second = e.finish().unwrap();
    assert_eq!(unzlib(&first, &a), a);
    assert_eq!(unzlib(&second, &b), b);
}

/// A writer that fails every `fail_every`-th call and otherwise accepts at
/// most `max` bytes.
struct Flaky {
    out: Vec<u8>,
    calls: usize,
    fail_every: usize,
    max: usize,
}

impl Write for Flaky {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.fail_every > 0 && self.calls % self.fail_every == 0 {
            return Err(io::Error::other("flaky"));
        }
        let n = buf.len().min(self.max);
        self.out.extend_from_slice(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn write_errors_and_short_writes_lose_nothing() {
    let mut rng = common::Rng::new(5);
    let data: Vec<u8> =
        (0..700_000).map(|i| if i % 1000 < 600 { b'a' + (i % 7) as u8 } else { rng.next() as u8 }).collect();
    for (fail_every, max) in [(0, 1), (0, 7), (3, 1000), (2, usize::MAX)] {
        let mut e = DeflateEncoder::new(Flaky { out: Vec::new(), calls: 0, fail_every, max }, Compression::Default);
        let mut pos = 0;
        let mut errors = 0;
        while pos < data.len() {
            let end = (pos + 50_000).min(data.len());
            match e.write(&data[pos..end]) {
                Ok(n) => pos += n,
                Err(_) => errors += 1,
            }
        }
        // `finish` consumes the encoder, so retry with `reset` (which ends
        // the stream once and resends whatever the writer has not taken).
        let w = loop {
            match e.reset(Flaky { out: Vec::new(), calls: 0, fail_every: 0, max: usize::MAX }) {
                Ok(w) => break w,
                Err(_) => errors += 1,
            }
        };
        assert_eq!(deflate::decompress(&w.out).unwrap(), data, "fail_every {fail_every} max {max}");
        if fail_every > 0 {
            assert!(errors > 0);
        }
    }
}

#[test]
fn streamed_output_is_as_small_as_one_shot() {
    let data = common::corpus().into_iter().find(|(n, _)| n == "words-300k").unwrap().1.repeat(3);
    for (p, opts) in presets() {
        let one = deflate_bytes_conf(&data, opts).len();
        let mut e = DeflateEncoder::new(Vec::new(), opts);
        e.write_all(&data).unwrap();
        let streamed = e.finish().unwrap().len();
        // Chunks restart the match finder, but keep 32 KiB of history.
        assert!(streamed as f64 <= one as f64 * 1.01 + 64.0, "{p}: streamed {streamed} vs one-shot {one}");
    }
}

#[test]
fn rle_and_huffman_only_behave_as_named() {
    let runs: Vec<u8> = (0..100_000u32).map(|i| (i / 500) as u8).collect();
    let rle = deflate_bytes_conf(&runs, CompressionOptions::rle()).len();
    let huff = deflate_bytes_conf(&runs, CompressionOptions::huffman_only()).len();
    let stored =
        deflate_bytes_conf(&runs, CompressionOptions { special: SpecialOptions::_ForceStored, ..Default::default() });
    assert!(rle < 2_000, "rle {rle}");
    assert!(huff > 10_000, "huffman-only {huff}");
    assert!(stored.len() > runs.len());
    assert_eq!(deflate::decompress(&stored).unwrap(), runs);
}

/// A writer that fails on chosen calls (with a chosen error kind) and
/// otherwise accepts at most `max` bytes per call.
struct Faulty {
    out: Vec<u8>,
    calls: usize,
    fails: fn(usize) -> Option<io::ErrorKind>,
    max: usize,
}

impl Faulty {
    fn new(fails: fn(usize) -> Option<io::ErrorKind>) -> Self {
        Faulty { out: Vec::new(), calls: 0, fails, max: 1000 }
    }
}

impl Write for Faulty {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if let Some(kind) = (self.fails)(self.calls) {
            return Err(io::Error::new(kind, "injected"));
        }
        let n = buf.len().min(self.max);
        self.out.extend_from_slice(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// One compressor input chunk: writing this much produces output at once.
const CHUNK: usize = 256 * 1024;

fn chunk_data() -> Vec<u8> {
    (0..CHUNK as u32).map(|i| (i % 251) as u8 ^ (i / 997) as u8).collect()
}

/// A `write` that fills a chunk produces output; when sending it fails,
/// the error is deferred and the output stays queued. The `flush` or
/// `finish` that follows must send it, not drop it.
#[test]
fn deferred_write_error_then_flush_or_finish_loses_nothing() {
    let data = chunk_data();
    // The raw encoder's first writer call sends compressed data; the zlib
    // and gzip encoders send their header first, so fail their second call.
    let first_fails: fn(usize) -> Option<io::ErrorKind> = |c| (c == 1).then_some(io::ErrorKind::Other);
    let second_fails: fn(usize) -> Option<io::ErrorKind> = |c| (c == 2).then_some(io::ErrorKind::Other);
    for flush_first in [true, false] {
        let mut e = DeflateEncoder::new(Faulty::new(first_fails), Compression::Default);
        assert_eq!(e.write(&data).unwrap(), CHUNK);
        if flush_first {
            e.flush().unwrap();
        }
        let w = e.finish().unwrap();
        assert!(w.calls > 1);
        assert_eq!(deflate::decompress(&w.out).unwrap(), data, "deflate, flush first: {flush_first}");

        let mut e = ZlibEncoder::new(Faulty::new(second_fails), Compression::Default);
        assert_eq!(e.write(&data).unwrap(), CHUNK);
        if flush_first {
            e.flush().unwrap();
        }
        let w = e.finish().unwrap();
        assert_eq!(unzlib(&w.out, &data), data, "zlib, flush first: {flush_first}");

        let mut e = GzEncoder::new(Faulty::new(second_fails), Compression::Default);
        assert_eq!(e.write(&data).unwrap(), CHUNK);
        if flush_first {
            e.flush().unwrap();
        }
        let w = e.finish().unwrap();
        assert_eq!(ungzip(&w.out, &data), data, "gzip, flush first: {flush_first}");
    }
}

/// `Interrupted` from the writer is retried, never reported: `flush` and
/// `finish` succeed even when every other write is interrupted.
#[test]
fn interrupted_writes_are_retried() {
    let data = chunk_data().repeat(2);
    let every_other: fn(usize) -> Option<io::ErrorKind> = |c| (c % 2 == 1).then_some(io::ErrorKind::Interrupted);
    let mut e = DeflateEncoder::new(Faulty::new(every_other), Compression::Fast);
    e.write_all(&data).unwrap();
    e.flush().unwrap();
    let w = e.finish().unwrap();
    assert!(w.calls > 10);
    assert_eq!(deflate::decompress(&w.out).unwrap(), data);
}
