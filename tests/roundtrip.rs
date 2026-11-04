//! Round trips over many generated inputs, every level and strategy, with
//! and without the run-time verification option.

mod common;

use common::Rng;
use verified_deflate::{compress_with, decompress, decompress_with_limit, CompressOptions, Strategy};

fn generated(rng: &mut Rng) -> Vec<u8> {
    let len = match rng.below(4) {
        0 => rng.below(16),
        1 => rng.below(1_000),
        2 => rng.below(70_000),
        _ => rng.below(300_000),
    } as usize;
    let alphabet = 1 + rng.below(256) as u8 as u64;
    let repeat_bias = rng.below(100);
    let mut v = Vec::with_capacity(len);
    while v.len() < len {
        if !v.is_empty() && rng.below(100) < repeat_bias {
            // copy an earlier run, possibly overlapping
            let dist = 1 + rng.below(v.len().min(40_000) as u64) as usize;
            let n = (3 + rng.below(300) as usize).min(len - v.len());
            for _ in 0..n {
                let b = v[v.len() - dist];
                v.push(b);
            }
        } else {
            v.push(rng.below(alphabet) as u8);
        }
    }
    v
}

#[test]
fn generated_inputs_roundtrip() {
    let mut rng = Rng::new(2024);
    for case in 0..120 {
        let data = generated(&mut rng);
        let level = (case % 10) as u8;
        let strategy = [Strategy::Auto, Strategy::Fixed, Strategy::Dynamic, Strategy::Stored][case % 4];
        let opts = CompressOptions { level, strategy, verify: false };
        let c = compress_with(&data, &opts);
        let d = decompress_with_limit(&c, data.len()).unwrap();
        assert_eq!(d.data, data, "case {case}");
        assert_eq!(d.consumed, c.len());
        // The verifier never needs to fall back: output is identical.
        let checked = compress_with(&data, &CompressOptions { verify: true, ..opts });
        assert_eq!(checked, c, "case {case}: verify changed the output");
    }
}

#[test]
fn concatenated_streams_decode_one_at_a_time() {
    let a = verified_deflate::compress(b"first stream first stream", 9);
    let b = verified_deflate::compress(b"second", 1);
    let mut both = a.clone();
    both.extend_from_slice(&b);
    let d1 = decompress_with_limit(&both, 1000).unwrap();
    assert_eq!(d1.data, b"first stream first stream");
    assert_eq!(decompress(&both[d1.consumed..]).unwrap(), b"second");
}

/// Each block is the smallest of stored, fixed and dynamic, so output is
/// never much larger than storing the input: five bytes per stored block
/// and at most one padding byte per Huffman block on top of the data.
#[test]
fn never_much_larger_than_stored() {
    let corpus = common::corpus();
    let mut rng = Rng::new(9);
    let mut inputs: Vec<Vec<u8>> = corpus.into_iter().map(|(_, d)| d).collect();
    for _ in 0..20 {
        inputs.push(generated(&mut rng));
    }
    for data in &inputs {
        let len = data.len();
        let bound = len
            + 6 * (len / verified_deflate::deflate::BLOCK_TOKENS + 1)
            + 5 * (len / verified_deflate::deflate::MAX_STORED_BLOCK + 1);
        for level in 1..=9u8 {
            let out = compress_with(data, &CompressOptions { level, strategy: Strategy::Auto, verify: false });
            assert!(out.len() <= bound, "level {level}, {len} bytes: {} > {bound}", out.len());
        }
    }
}
