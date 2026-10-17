//! Throughput benchmark: `cargo run --release --example bench [FILE...]`.
//!
//! Without arguments it uses a fixed set of generated corpora. Reports the
//! best of several runs. Set `BENCH_DUMP=dir` to write the corpora to files
//! (to compare against other implementations on identical inputs).

use std::time::{Duration, Instant};
use verified_deflate::{compress, crc32, decompress};

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn corpora() -> Vec<(String, Vec<u8>)> {
    let mut x = 0x1234_5678_9abc_def0u64;
    let words = ["deflate", "huffman", "verified", "kani", "proof", "zip", "the", "a", "of", "match", "window", "and"];
    let mut text = Vec::new();
    while text.len() < 8 << 20 {
        let r = xorshift(&mut x);
        text.extend_from_slice(words[(r % words.len() as u64) as usize].as_bytes());
        text.push(if r % 13 == 0 { b'\n' } else { b' ' });
    }
    let mut src = Vec::new();
    while src.len() < 8 << 20 {
        for f in ["src/deflate.rs", "src/inflate.rs", "src/huffman.rs", "src/lz77.rs", "src/zip.rs", "src/bitio.rs"] {
            src.extend(std::fs::read(f).expect("run from the crate root"));
        }
    }
    src.truncate(8 << 20);
    let mut records = Vec::new();
    let mut i = 0u32;
    while records.len() < 8 << 20 {
        let r = xorshift(&mut x);
        records.extend_from_slice(&i.to_le_bytes());
        records.extend_from_slice(&((r % 1000) as u32).to_le_bytes());
        records.extend_from_slice(b"\x01\x00\x00\x00status=ok;");
        records.extend_from_slice(&(r as u16).to_le_bytes());
        i += 1;
    }
    let random: Vec<u8> = (0..(4 << 20)).map(|_| xorshift(&mut x) as u8).collect();
    let zeros = vec![0u8; 16 << 20];
    vec![
        ("text".into(), text),
        ("source".into(), src),
        ("records".into(), records),
        ("random".into(), random),
        ("zeros".into(), zeros),
    ]
}

fn best<T>(runs: usize, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut best = Duration::MAX;
    let mut out = None;
    for _ in 0..runs {
        let t = Instant::now();
        let r = f();
        best = best.min(t.elapsed());
        out = Some(r);
    }
    (best, out.unwrap())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let inputs = if args.is_empty() {
        corpora()
    } else {
        args.iter().map(|p| (p.clone(), std::fs::read(p).expect("read input"))).collect()
    };
    if let Ok(dir) = std::env::var("BENCH_DUMP") {
        for (name, data) in &inputs {
            std::fs::write(format!("{dir}/{name}"), data).unwrap();
        }
    }
    let levels: Vec<u8> = std::env::var("BENCH_LEVELS")
        .map(|s| s.split(',').map(|x| x.parse().unwrap()).collect())
        .unwrap_or_else(|_| vec![1, 6, 9]);
    println!("{:<9} {:>5} {:>7} {:>10} {:>10}", "input", "level", "ratio", "comp MB/s", "dec MB/s");
    for (name, data) in &inputs {
        let mb = data.len() as f64 / 1e6;
        for &level in &levels {
            let runs = if level >= 9 { 1 } else { 3 };
            let (ct, c) = best(runs, || compress(std::hint::black_box(data), level));
            let (dt, d) = best(3, || decompress(std::hint::black_box(&c)).unwrap());
            assert!(d == *data, "round trip failed for {name} at level {level}");
            println!(
                "{:<9} {:>5} {:>6.1}% {:>10.1} {:>10.1}",
                name,
                level,
                c.len() as f64 * 100.0 / data.len() as f64,
                mb / ct.as_secs_f64(),
                mb / dt.as_secs_f64()
            );
        }
        let (t, _) = best(3, || std::hint::black_box(crc32(std::hint::black_box(data))));
        println!("{:<9} {:>5} {:>7} {:>10.1}", name, "crc", "", mb / t.as_secs_f64());
    }
}
