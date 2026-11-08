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

/// SHA-256, to compare outputs with Python's `hashlib`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(chunk[4 * i..4 * i + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for (a, b) in h.iter_mut().zip(v) {
            *a = a.wrapping_add(b);
        }
    }
    let mut out = [0u8; 32];
    for (i, x) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// SHA-256 in lowercase hex, as `hashlib`'s `hexdigest`.
pub fn sha256_hex(data: &[u8]) -> String {
    sha256(data).iter().map(|b| format!("{b:02x}")).collect()
}
