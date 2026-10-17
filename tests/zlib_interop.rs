//! Cross-checks against the reference zlib, through Python's `zlib`
//! module: zlib must decode everything we produce (all levels and block
//! strategies), and we must decode everything zlib produces (all levels,
//! strategies and window sizes). Skipped if `python3` is unavailable.

mod common;

use std::fs;
use std::process::Command;
use verified_deflate::{compress_with, decompress_with_limit, CompressOptions, Strategy};

const ZLIB_CHECK: &str = r#"
import sys, os, zlib
d = sys.argv[1]
names = sorted(n[:-4] for n in os.listdir(d) if n.endswith('.raw'))
bad = 0
for n in names:
    data = open(os.path.join(d, n + '.raw'), 'rb').read()
    # 1. zlib decodes each of our streams exactly, consuming all of it.
    for f in sorted(os.listdir(d)):
        if f.startswith(n + '.ours.'):
            s = open(os.path.join(d, f), 'rb').read()
            o = zlib.decompressobj(-15)
            out = o.decompress(s) + o.flush()
            if out != data or not o.eof or o.unused_data:
                print('MISMATCH', f); bad += 1
    # 2. Produce zlib streams for every level / strategy / window size.
    strategies = [zlib.Z_DEFAULT_STRATEGY, zlib.Z_FILTERED, zlib.Z_HUFFMAN_ONLY, zlib.Z_RLE, zlib.Z_FIXED]
    for level in range(10):
        for si, st in enumerate(strategies):
            for wbits in (9, 12, 15):
                if level not in (0, 1, 6, 9) and (si or wbits != 15):
                    continue
                c = zlib.compressobj(level, zlib.DEFLATED, -wbits, 9 if wbits == 15 else 1, st)
                s = c.compress(data) + c.flush()
                open(os.path.join(d, '%s.zlib.%d.%d.%d' % (n, level, si, wbits)), 'wb').write(s)
print('zlib', zlib.ZLIB_RUNTIME_VERSION, 'bad', bad)
sys.exit(1 if bad else 0)
"#;

#[test]
fn interop_with_reference_zlib() {
    if !common::have("python3", "--version") {
        eprintln!("python3 not found; skipping zlib interop");
        return;
    }
    let dir = common::temp_dir("zlib");
    let corpus = common::corpus();
    for (name, data) in &corpus {
        fs::write(dir.join(format!("{name}.raw")), data).unwrap();
        for (si, strategy) in [Strategy::Auto, Strategy::Stored, Strategy::Fixed, Strategy::Dynamic].iter().enumerate()
        {
            for level in 0..=9u8 {
                if *strategy != Strategy::Auto && level != 6 {
                    continue;
                }
                let c = compress_with(data, &CompressOptions { level, strategy: *strategy, verify: false });
                fs::write(dir.join(format!("{name}.ours.{si}.{level}")), c).unwrap();
            }
        }
    }
    let script = dir.join("check.py");
    fs::write(&script, ZLIB_CHECK).unwrap();
    let out = Command::new("python3").arg("-I").arg(&script).arg(&dir).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!("{stdout}{}", String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "zlib rejected some of our streams");

    let mut checked = 0;
    for (name, data) in &corpus {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let fname = path.file_name().unwrap().to_string_lossy().into_owned();
            if !fname.starts_with(&format!("{name}.zlib.")) {
                continue;
            }
            let s = fs::read(&path).unwrap();
            let d = decompress_with_limit(&s, data.len()).unwrap_or_else(|e| panic!("{fname}: {e}"));
            assert_eq!(&d.data, data, "{fname}");
            assert_eq!(d.consumed, s.len(), "{fname}");
            checked += 1;
        }
    }
    eprintln!("decoded {checked} zlib streams");
    assert!(checked > 300);
    let _ = fs::remove_dir_all(&dir);
}

/// Our compression ratio should be in zlib's neighbourhood. This is not a
/// correctness property; it guards against silent regressions such as the
/// matcher finding no matches.
#[test]
fn compression_ratio_is_reasonable() {
    if !common::have("python3", "--version") {
        return;
    }
    let corpus = common::corpus();
    let (_, text) = corpus.iter().find(|(n, _)| n == "words-300k").unwrap();
    let dir = common::temp_dir("ratio");
    let raw = dir.join("in.raw");
    fs::write(&raw, text).unwrap();
    let out = Command::new("python3")
        .args(["-I", "-c", "import sys,zlib; c=zlib.compressobj(6,8,-15); d=open(sys.argv[1],'rb').read(); print(len(c.compress(d)+c.flush()))"])
        .arg(&raw)
        .output()
        .unwrap();
    let zlib_len: usize = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    let ours = verified_deflate::compress(text, 6).len();
    eprintln!("words-300k level 6: ours {ours}, zlib {zlib_len}");
    assert!(ours as f64 <= zlib_len as f64 * 1.10, "ours {ours} vs zlib {zlib_len}");
    let _ = fs::remove_dir_all(&dir);
}
