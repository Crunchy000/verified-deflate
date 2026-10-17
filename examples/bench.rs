//! Rough throughput numbers: `cargo run --release --example bench [FILE]`.
//! Without a file, uses ~10 MB of generated text.

use std::time::Instant;
use verified_deflate::{compress, decompress};

fn main() {
    let data = match std::env::args().nth(1) {
        Some(p) => std::fs::read(p).expect("read input"),
        None => {
            let words = ["deflate", "huffman", "verified", "kani", "proof", "zip", "the", "a", "of", "match"];
            let mut x = 1u64;
            let mut v = Vec::new();
            while v.len() < 10 << 20 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                v.extend_from_slice(words[(x % 10) as usize].as_bytes());
                v.push(if x % 11 == 0 { b'\n' } else { b' ' });
            }
            v
        }
    };
    let mb = data.len() as f64 / 1e6;
    println!("input: {:.1} MB", mb);
    for level in [1u8, 6, 9] {
        let t = Instant::now();
        let c = compress(&data, level);
        let ct = t.elapsed().as_secs_f64();
        let t = Instant::now();
        let d = decompress(&c).unwrap();
        let dt = t.elapsed().as_secs_f64();
        assert_eq!(d, data);
        println!(
            "level {level}: ratio {:5.1}%  compress {:6.1} MB/s  decompress {:6.1} MB/s",
            c.len() as f64 * 100.0 / data.len() as f64,
            mb / ct,
            mb / dt
        );
    }
}
