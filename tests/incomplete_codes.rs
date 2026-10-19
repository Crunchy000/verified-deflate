//! Incomplete Huffman codes in dynamic blocks. RFC 1951 leaves them
//! loosely specified; zlib accepts exactly two kinds (a single code of one
//! bit, and a distance code with no codes at all) and rejects the rest.
//! These hand-built blocks pin that behaviour, and Python's `zlib` (when
//! available) confirms it is zlib's.
//!
//! The streams are built here with an independent bit writer and
//! canonical-code builder, not with the crate's encoder.

mod common;

use std::fs;
use std::process::Command;
use verified_deflate::{decompress, InflateError};

/// LSB-first bit writer, as DEFLATE packs bits.
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn new() -> Self {
        Bits { out: Vec::new(), acc: 0, n: 0 }
    }
    fn put(&mut self, v: u32, len: u32) {
        self.acc |= u64::from(v) << self.n;
        self.n += len;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }
    /// Huffman codes are sent most significant bit first.
    fn put_code(&mut self, code: u32, len: u32) {
        let mut r = 0;
        for i in 0..len {
            r |= ((code >> i) & 1) << (len - 1 - i);
        }
        self.put(r, len);
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// Canonical codes for `lengths` (RFC 1951 §3.2.2).
fn canonical(lengths: &[u8]) -> Vec<u32> {
    let mut count = [0u32; 16];
    for &l in lengths {
        if l > 0 {
            count[l as usize] += 1;
        }
    }
    let mut next = [0u32; 16];
    let mut code = 0;
    for bits in 1..16 {
        code = (code + count[bits - 1]) << 1;
        next[bits] = code;
    }
    lengths
        .iter()
        .map(|&l| {
            if l == 0 {
                0
            } else {
                let c = next[l as usize];
                next[l as usize] += 1;
                c
            }
        })
        .collect()
}

/// Order in which code-length code lengths are sent.
const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// A complete code-length code: 13 codes of 4 bits and 6 of 5 bits.
const CL_COMPLETE: [u8; 19] = [4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5];

enum Item {
    /// A literal/length symbol.
    Lit(u32),
    /// A distance symbol.
    Dist(u32),
}

/// One final dynamic block with these code lengths and symbols. Every
/// code length is sent as its own code-length symbol (no repeat codes).
fn dynamic_block(cl: &[u8; 19], lit: &[u8], dist: &[u8], body: &[Item]) -> Vec<u8> {
    assert!(lit.len() >= 257 && !dist.is_empty());
    let mut b = Bits::new();
    b.put(1, 1); // BFINAL
    b.put(2, 2); // BTYPE = dynamic
    b.put((lit.len() - 257) as u32, 5);
    b.put((dist.len() - 1) as u32, 5);
    b.put(15, 4); // all 19 code-length code lengths follow
    for &sym in &ORDER {
        b.put(u32::from(cl[sym]), 3);
    }
    let cl_codes = canonical(cl);
    for &l in lit.iter().chain(dist) {
        b.put_code(cl_codes[l as usize], u32::from(cl[l as usize]));
    }
    let lit_codes = canonical(lit);
    let dist_codes = canonical(dist);
    for item in body {
        match *item {
            Item::Lit(s) => b.put_code(lit_codes[s as usize], u32::from(lit[s as usize])),
            Item::Dist(s) => b.put_code(dist_codes[s as usize], u32::from(dist[s as usize])),
        }
    }
    b.finish()
}

/// Literal/length code lengths with the given (symbol, length) pairs.
fn lit_lengths(n: usize, set: &[(usize, u8)]) -> Vec<u8> {
    let mut l = vec![0u8; n];
    for &(s, len) in set {
        l[s] = len;
    }
    l
}

struct Case {
    name: &'static str,
    stream: Vec<u8>,
    expect: Result<Vec<u8>, InflateError>,
}

fn cases() -> Vec<Case> {
    use Item::{Dist, Lit};
    // 'A', end of block and length 3 (symbol 257): a complete code.
    let lit3 = lit_lengths(258, &[(65, 2), (256, 2), (257, 1)]);
    // 'A' and end of block, one bit each: a complete code.
    let lit2 = lit_lengths(257, &[(65, 1), (256, 1)]);
    vec![
        Case {
            name: "single one-bit distance code",
            stream: dynamic_block(&CL_COMPLETE, &lit3, &[1], &[Lit(65), Lit(257), Dist(0), Lit(256)]),
            expect: Ok(b"AAAA".to_vec()),
        },
        Case {
            name: "no distance codes",
            stream: dynamic_block(&CL_COMPLETE, &lit2, &[0], &[Lit(65), Lit(65), Lit(256)]),
            expect: Ok(b"AA".to_vec()),
        },
        Case {
            name: "single one-bit literal/length code",
            stream: dynamic_block(&CL_COMPLETE, &lit_lengths(257, &[(256, 1)]), &[0], &[Lit(256)]),
            expect: Ok(Vec::new()),
        },
        Case {
            name: "incomplete literal/length code",
            stream: dynamic_block(&CL_COMPLETE, &lit_lengths(257, &[(65, 2), (256, 2)]), &[0], &[Lit(65), Lit(256)]),
            expect: Err(InflateError::InvalidLiteralLengthCode),
        },
        Case {
            name: "incomplete distance code of two codes",
            stream: dynamic_block(&CL_COMPLETE, &lit3, &[2, 2], &[Lit(65), Lit(257), Dist(0), Lit(256)]),
            expect: Err(InflateError::InvalidDistanceCode),
        },
        Case {
            name: "single two-bit distance code",
            stream: dynamic_block(&CL_COMPLETE, &lit3, &[2], &[Lit(65), Lit(257), Dist(0), Lit(256)]),
            expect: Err(InflateError::InvalidDistanceCode),
        },
        Case {
            name: "incomplete code-length code",
            stream: dynamic_block(&[5; 19], &lit2, &[0], &[Lit(65), Lit(256)]),
            expect: Err(InflateError::InvalidCodeLengthCode),
        },
    ]
}

#[test]
fn incomplete_codes_are_accepted_only_as_zlib_accepts_them() {
    for c in cases() {
        assert_eq!(decompress(&c.stream), c.expect, "{}", c.name);
    }
}

const ZLIB_SCRIPT: &str = r#"
import sys, zlib
for path in sys.argv[1:]:
    data = open(path, 'rb').read()
    try:
        print('ok', zlib.decompress(data, -15).hex())
    except zlib.error:
        print('error')
"#;

/// The expectations above are zlib's behaviour.
#[test]
fn zlib_agrees() {
    if !common::have("python3", "--version") {
        eprintln!("python3 not found; skipping");
        return;
    }
    let dir = common::temp_dir("incomplete");
    let cases = cases();
    let mut paths = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        let p = dir.join(format!("{i}.raw"));
        fs::write(&p, &c.stream).unwrap();
        paths.push(p);
    }
    let out = Command::new("python3").args(["-I", "-c", ZLIB_SCRIPT]).args(&paths).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), cases.len(), "{stdout}{}", String::from_utf8_lossy(&out.stderr));
    for (c, line) in cases.iter().zip(lines) {
        let zlib = line.strip_prefix("ok ").map(|hex| {
            (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect::<Vec<u8>>()
        });
        assert_eq!(zlib, c.expect.clone().ok(), "{}: zlib says {line}", c.name);
    }
    let _ = fs::remove_dir_all(&dir);
}
