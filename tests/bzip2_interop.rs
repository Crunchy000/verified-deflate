//! bzip2 against tools we did not write:
//!
//! - Streams from the `bzip2` program and from libbz2 (Python's `bz2`) at
//!   levels 1, 5 and 9, and concatenated streams, decode exactly; both
//!   decode what we write, including "randomised" blocks.
//! - Damaged streams: for hundreds of single-byte changes and cuts to our
//!   streams, as they are and with their CRCs rewritten to match, we
//!   accept exactly the streams libbz2 accepts, with the same output and
//!   length used.
//! - With 7-Zip installed, it decodes our streams and we decode its.
//!
//! Each part is skipped if its tool is missing, unless
//! `REQUIRE_INTEROP_TOOLS` is set, as it is in CI.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;
use verified_deflate::bzip2;

fn missing(tool: &str) {
    assert!(std::env::var_os("REQUIRE_INTEROP_TOOLS").is_none(), "{tool} not found, and REQUIRE_INTEROP_TOOLS is set");
    eprintln!("{tool} not found; skipping");
}

fn python(script: &str, args: &[&Path]) -> String {
    let out = Command::new("python3").arg("-I").arg("-c").arg(script).args(args).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "python failed: {stdout}{}", String::from_utf8_lossy(&out.stderr));
    stdout
}

fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut v = common::corpus();
    // Runs at the run-length step's limits, and a block that is one run.
    let mut runs = Vec::new();
    for n in [1usize, 3, 4, 5, 6, 250, 251, 252, 255, 256, 259, 260, 1000] {
        runs.extend(std::iter::repeat_n(b'r', n));
        runs.push(b'.');
    }
    v.push(("runs".into(), runs));
    v.push(("run-ends-block".into(), vec![b'z'; 4]));
    v
}

#[test]
fn bzip2_program_round_trips() {
    if !common::have("bzip2", "--help") {
        return missing("bzip2");
    }
    let dir = common::temp_dir("bz2-prog");
    for (name, data) in corpus() {
        let raw = dir.join(format!("{name}.raw"));
        fs::write(&raw, &data).unwrap();
        for level in [1u8, 5, 9] {
            // Theirs, decoded by us.
            let out = Command::new("bzip2").arg(format!("-{level}")).arg("-c").arg(&raw).output().unwrap();
            assert!(out.status.success());
            assert_eq!(bzip2::decompress(&out.stdout).unwrap(), data, "bzip2 -{level} {name}");
            // Ours, decoded by them.
            for randomised in [false, true] {
                let ours =
                    if randomised { bzip2::compress_randomised(&data, level) } else { bzip2::compress(&data, level) };
                let packed = dir.join(format!("{name}.{level}.{randomised}.bz2"));
                fs::write(&packed, &ours).unwrap();
                let out = Command::new("bzip2").arg("-dc").arg(&packed).output().unwrap();
                assert!(
                    out.status.success(),
                    "bzip2 -d on ours ({name}, level {level}, randomised {randomised}): {}",
                    String::from_utf8_lossy(&out.stderr)
                );
                assert!(
                    out.stdout == data,
                    "bzip2 -d on ours ({name}, level {level}, randomised {randomised}) differs"
                );
            }
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

const LIBBZ2_STREAMS: &str = r#"
import bz2, os, sys
d = sys.argv[1]
for f in sorted(os.listdir(d)):
    if not f.endswith('.raw'):
        continue
    n, data = f[:-4], open(os.path.join(d, f), 'rb').read()
    for level in (1, 5, 9):
        open(os.path.join(d, '%s.%d.lib.bz2' % (n, level)), 'wb').write(bz2.compress(data, level))
    # Two streams, concatenated.
    h = len(data) // 2
    open(os.path.join(d, '%s.multi.bz2' % n), 'wb').write(bz2.compress(data[:h], 1) + bz2.compress(data[h:], 9))
    # Ours, decoded by libbz2.
    for level in (1, 9):
        ours = open(os.path.join(d, '%s.%d.ours.bz2' % (n, level)), 'rb').read()
        assert bz2.decompress(ours) == data, (n, level)
print('ok')
"#;

#[test]
fn libbz2_round_trips() {
    if !common::have("python3", "--version") {
        return missing("python3");
    }
    let dir = common::temp_dir("bz2-lib");
    let corpus = corpus();
    for (name, data) in &corpus {
        fs::write(dir.join(format!("{name}.raw")), data).unwrap();
        for level in [1u8, 9] {
            fs::write(dir.join(format!("{name}.{level}.ours.bz2")), bzip2::compress(data, level)).unwrap();
        }
    }
    assert_eq!(python(LIBBZ2_STREAMS, &[&dir]).trim(), "ok");
    for (name, data) in &corpus {
        for level in [1, 5, 9] {
            let s = fs::read(dir.join(format!("{name}.{level}.lib.bz2"))).unwrap();
            let got = bzip2::read_stream(&s, usize::MAX).unwrap();
            assert!(got.data == *data, "libbz2 level {level} {name}");
            assert_eq!((got.consumed, got.level), (s.len(), level), "{name}");
        }
        let multi = fs::read(dir.join(format!("{name}.multi.bz2"))).unwrap();
        assert!(bzip2::decompress(&multi).unwrap() == *data, "two streams, {name}");
    }
    let _ = fs::remove_dir_all(&dir);
}

/// Decodes one stream from each file, as libbz2 does: the output and the
/// bytes used, or the error.
const LIBBZ2_DECODE: &str = r#"
import bz2, hashlib, os, sys
d = sys.argv[1]
for f in sorted(os.listdir(d), key=lambda f: int(f.split('.')[0])):
    data = open(os.path.join(d, f), 'rb').read()
    dec = bz2.BZ2Decompressor()
    try:
        out = dec.decompress(data)
    except (OSError, ValueError) as e:
        print(f, 'error')
        continue
    if not dec.eof:
        print(f, 'truncated')
        continue
    print(f, 'ok', len(data) - len(dec.unused_data), hashlib.sha256(out).hexdigest())
"#;

#[test]
fn damaged_streams_agree_with_libbz2() {
    if !common::have("python3", "--version") {
        return missing("python3");
    }
    let dir = common::temp_dir("bz2-damaged");
    let mut rng = common::Rng::new(2026);
    let bases: Vec<Vec<u8>> = vec![
        bzip2::compress(b"hello, hello, hello, bzip2", 9),
        bzip2::compress(&b"abcd".repeat(3000), 1),
        bzip2::compress(&rng.bytes(3000), 9),
        bzip2::compress_randomised(&b"aaaaaaaab".repeat(200), 9),
        bzip2::compress(&[vec![7u8; 600], rng.bytes(200)].concat(), 2),
    ];
    let mut cases: Vec<Vec<u8>> = Vec::new();
    for base in &bases {
        for _ in 0..120 {
            let mut c = base.clone();
            let i = rng.below(c.len() as u64) as usize;
            match rng.below(3) {
                0 => c[i] ^= 1 << rng.below(8),
                1 => c[i] = rng.below(256) as u8,
                _ => c.truncate(i),
            }
            // Also with the CRCs made to match, so the change is judged by
            // the structural checks rather than caught by a CRC.
            let mut fixed = c.clone();
            bzip2::fix_crcs(&mut fixed, usize::MAX);
            cases.push(c);
            cases.push(fixed);
        }
        // Trailing bytes are not part of the stream.
        cases.push([&base[..], b"tail"].concat());
    }
    for (i, c) in cases.iter().enumerate() {
        fs::write(dir.join(format!("{i}.bz2")), c).unwrap();
    }
    let report = python(LIBBZ2_DECODE, &[&dir]);
    let mut accepted = 0;
    for line in report.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let i: usize = f[0].split('.').next().unwrap().parse().unwrap();
        let ours = bzip2::read_stream(&cases[i], usize::MAX);
        match (f[1], &ours) {
            ("ok", Ok(s)) => {
                accepted += 1;
                assert_eq!(
                    (s.consumed.to_string(), common::sha256_hex(&s.data)),
                    (f[2].to_string(), f[3].to_string()),
                    "case {i}"
                );
            }
            ("ok", Err(e)) => panic!("case {i}: libbz2 accepts, we fail with {e}"),
            (_, Ok(_)) => panic!("case {i}: libbz2 fails ({}), we accept", f[1]),
            (_, Err(_)) => {}
        }
    }
    assert_eq!(report.lines().count(), cases.len());
    // Many changes leave a valid stream once the CRCs match.
    assert!(accepted >= 100, "{accepted} accepted");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn seven_zip_round_trips() {
    let sz = ["7z", "7zz"].into_iter().find(|c| common::have(c, "i"));
    let Some(sz) = sz else { return missing("7z") };
    let dir = common::temp_dir("bz2-7z");
    for (name, data) in corpus()
        .into_iter()
        .filter(|(n, _)| ["words-300k", "random-200k", "zeros-1M", "runs", "sources"].contains(&n.as_str()))
    {
        fs::write(dir.join(&name), &data).unwrap();
        for mx in ["1", "9"] {
            let archive = format!("{name}.{mx}.7z.bz2");
            let out = Command::new(sz)
                .args(["a", "-tbzip2", &format!("-mx{mx}"), &archive, &name])
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(out.status.success(), "7z a: {}", String::from_utf8_lossy(&out.stdout));
            assert!(bzip2::decompress(&fs::read(dir.join(&archive)).unwrap()).unwrap() == data, "7z -mx{mx} {name}");
        }
        let ours = format!("{name}.ours.bz2");
        fs::write(dir.join(&ours), bzip2::compress(&data, 9)).unwrap();
        let out = Command::new(sz).args(["x", "-so", &ours]).current_dir(&dir).output().unwrap();
        assert!(out.status.success(), "7z x on ours: {}", String::from_utf8_lossy(&out.stderr));
        assert!(out.stdout == data, "7z x on ours ({name}) differs");
    }
    let _ = fs::remove_dir_all(&dir);
}
