//! Shared helpers for integration tests: a deterministic PRNG and a corpus
//! of inputs that exercise different encoder paths.
#![allow(dead_code)]

use std::path::PathBuf;

/// xorshift64* — deterministic, dependency-free.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

/// Named inputs covering literals-only, long runs, match-heavy text,
/// incompressible data, and sizes around the 64 KiB stored-block limit.
pub fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut rng = Rng::new(42);
    let mut v: Vec<(String, Vec<u8>)> = Vec::new();
    for n in [0usize, 1, 2, 3, 4, 5, 8, 100, 257, 258, 259] {
        v.push((format!("random-{n}"), rng.bytes(n)));
    }
    v.push(("zeros-1M".into(), vec![0u8; 1 << 20]));
    v.push(("one-byte-run-65536".into(), vec![b'a'; 65_536]));
    v.push(("random-65535".into(), rng.bytes(65_535)));
    v.push(("random-65536".into(), rng.bytes(65_536)));
    v.push(("random-200k".into(), rng.bytes(200_000)));
    let mut text = Vec::new();
    let words = ["the", "quick", "brown", "fox", "jumps", "over", "lazy", "dog", "and", "a", "verified", "deflate"];
    while text.len() < 300_000 {
        text.extend_from_slice(words[rng.below(words.len() as u64) as usize].as_bytes());
        text.push(if rng.below(10) == 0 { b'\n' } else { b' ' });
    }
    v.push(("words-300k".into(), text));
    // Small alphabet with skewed frequencies: deep Huffman trees.
    let skew: Vec<u8> = (0..100_000)
        .map(|_| {
            let r = rng.next();
            (r.leading_zeros() as u8).min(30)
        })
        .collect();
    v.push(("skewed-100k".into(), skew));
    // Fibonacci-like symbol counts force the code-length limit.
    let mut fib = Vec::new();
    let (mut a, mut b) = (1usize, 1usize);
    for sym in 0..25u8 {
        fib.extend(std::iter::repeat_n(sym, a));
        let c = a + b;
        a = b;
        b = c;
    }
    let mut fib_shuffled = fib.clone();
    for i in (1..fib_shuffled.len()).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        fib_shuffled.swap(i, j);
    }
    v.push(("fibonacci-counts".into(), fib_shuffled));
    // Structured binary: repeated records with a changing counter.
    let mut rec = Vec::new();
    for i in 0..20_000u32 {
        rec.extend_from_slice(&i.to_le_bytes());
        rec.extend_from_slice(b"\x00\x00record\x01\x02");
        rec.extend_from_slice(&(i * 7 % 13).to_be_bytes());
    }
    v.push(("records-400k".into(), rec));
    // Compressible, then incompressible, then compressible again: encoders
    // emit Huffman blocks around stored blocks, so a stored block starts
    // while the decoder's bit buffer still holds bytes it looked ahead at.
    let mut mixed = Vec::new();
    for i in 0..30_000u32 {
        mixed.push(b"abcab "[(i % 6) as usize]);
    }
    mixed.extend(rng.bytes(70_000));
    for i in 0..30_000u32 {
        mixed.push(b"xyzzy\n"[(i % 6) as usize]);
    }
    v.push(("text-random-text".into(), mixed));
    // This crate's own sources.
    let mut src = Vec::new();
    for f in ["src/lib.rs", "src/deflate.rs", "src/inflate.rs", "src/huffman.rs", "src/zip.rs"] {
        src.extend(std::fs::read(f).unwrap());
    }
    v.push(("sources".into(), src));
    v
}

/// A fresh, empty directory under the system temp dir.
pub fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("verified-deflate-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// True if `cmd --version` (or `-v`) runs.
pub fn have(cmd: &str, arg: &str) -> bool {
    std::process::Command::new(cmd).arg(arg).output().map(|o| o.status.success()).unwrap_or(false)
}
