//! Corrupted, truncated and random inputs: the decoder and the ZIP reader
//! must return errors, never panic, and never exceed the output limit.
//! (Kani proves this exhaustively for small inputs; this extends the check
//! to large ones by sampling.)

mod common;

use common::Rng;
use verified_deflate::zip::{FileOptions, ZipArchive, ZipWriter};
use verified_deflate::{compress_with, decompress_with_limit, CompressOptions, Strategy};

fn mutate(rng: &mut Rng, s: &[u8]) -> Vec<u8> {
    let mut v = s.to_vec();
    match rng.below(4) {
        0 if !v.is_empty() => {
            // flip a few bits
            for _ in 0..=rng.below(4) {
                let i = rng.below(v.len() as u64) as usize;
                v[i] ^= 1 << rng.below(8);
            }
        }
        1 => v.truncate(rng.below(v.len() as u64 + 1) as usize),
        2 if !v.is_empty() => {
            // overwrite a byte with a random value
            let i = rng.below(v.len() as u64) as usize;
            v[i] = rng.next() as u8;
        }
        _ => {
            let i = rng.below(v.len() as u64 + 1) as usize;
            v.insert(i, rng.next() as u8);
        }
    }
    v
}

#[test]
fn corrupted_streams_never_panic() {
    let mut rng = Rng::new(7);
    let corpus = common::corpus();
    let mut seeds = Vec::new();
    for (name, data) in corpus.iter().filter(|(_, d)| d.len() <= 70_000) {
        for strategy in [Strategy::Auto, Strategy::Fixed, Strategy::Dynamic, Strategy::Stored] {
            let c = compress_with(data, &CompressOptions { level: 6, strategy, verify: false });
            seeds.push((name.clone(), c));
        }
    }
    let mut errors = 0usize;
    for round in 0..40_000 {
        let (_, s) = &seeds[round % seeds.len()];
        let m = mutate(&mut rng, s);
        let limit = rng.below(200_000) as usize;
        match decompress_with_limit(&m, limit) {
            Ok(d) => {
                assert!(d.data.len() <= limit);
                assert!(d.consumed <= m.len());
            }
            Err(_) => errors += 1,
        }
    }
    assert!(errors > 10_000, "only {errors} corruptions detected");
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng::new(99);
    for _ in 0..100_000 {
        let n = rng.below(64) as usize;
        let data = rng.bytes(n);
        if let Ok(d) = decompress_with_limit(&data, 1 << 16) {
            assert!(d.data.len() <= 1 << 16);
        }
    }
}

#[test]
fn zip_bombs_are_bounded() {
    // 50 MB of zeros compresses to ~50 KB; the reader must refuse to produce
    // more than the declared size even if the header lies.
    let zeros = vec![0u8; 50 << 20];
    let mut w = ZipWriter::new();
    w.add_file("z", &zeros, &FileOptions { verify: false, ..FileOptions::default() }).unwrap();
    let mut zip = w.finish().unwrap();
    // Patch the declared uncompressed size in the central directory down to 1 MiB.
    let a = ZipArchive::parse(&zip).unwrap();
    let cd = zip.len() - 22 - 46 - 1;
    assert_eq!(u32::from_le_bytes(zip[cd + 24..cd + 28].try_into().unwrap()), a.entries()[0].uncompressed_size);
    zip[cd + 24..cd + 28].copy_from_slice(&(1u32 << 20).to_le_bytes());
    let a = ZipArchive::parse(&zip).unwrap();
    assert!(a.read(0).is_err());
}

#[test]
fn corrupted_archives_never_panic() {
    let mut rng = Rng::new(3);
    let mut w = ZipWriter::new();
    for (name, data) in common::corpus().iter().filter(|(_, d)| d.len() < 5_000) {
        w.add_file(name, data, &FileOptions::default()).unwrap();
    }
    let zip = w.finish().unwrap();
    for _ in 0..20_000 {
        let m = mutate(&mut rng, &zip);
        if let Ok(a) = ZipArchive::parse(&m) {
            for i in 0..a.len() {
                let _ = a.read(i);
            }
        }
    }
}
