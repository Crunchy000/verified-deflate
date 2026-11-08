//! LZMA, LZMA2 and `.xz` against XZ Utils (liblzma, through Python's
//! `lzma`, and the `xz` program):
//!
//! - Their `.xz` files at presets 0–9 and extreme, with every check type
//!   and every filter chain they offer (delta, x86, PowerPC, IA-64, ARM,
//!   ARM-Thumb, SPARC, and with `xz` ARM64 and, where supported, RISC-V),
//!   their `.lzma` files and raw LZMA1 and LZMA2 streams with other
//!   `lc`/`lp`/`pb` and dictionaries, decode exactly here.
//! - They decode what this crate writes in each of those forms.
//! - Damaged streams: for thousands of single-byte changes and cuts, this
//!   crate accepts exactly the raw LZMA1, raw LZMA2, `.lzma` and `.xz`
//!   streams liblzma accepts, with the same output and length used.
//! - With 7-Zip installed, it decodes our `.xz` files and we decode its.
//!
//! Each part is skipped if its tool is missing, unless
//! `REQUIRE_INTEROP_TOOLS` is set, as it is in CI.

mod common;

use std::fs;
use std::path::Path;
use std::process::Command;
use verified_deflate::filters::BranchFilter;
use verified_deflate::lzma::{self, LzmaOptions, LzmaProps};
use verified_deflate::lzma2;
use verified_deflate::xz::{self, Check, Filter, XzOptions};

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

/// Inputs: the shared corpus, plus code-like bytes for the branch filters.
fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = common::corpus()
        .into_iter()
        .filter(|(n, _)| !n.starts_with("random-") || n == "random-200k" || n == "random-5")
        .collect();
    let mut rng = common::Rng::new(77);
    let mut code = Vec::new();
    while code.len() < 200_000 {
        match rng.below(6) {
            0 => code.extend_from_slice(&[0xE8, rng.below(256) as u8, rng.below(4) as u8, 0, 0]),
            1 => code.extend_from_slice(&[rng.below(256) as u8, rng.below(256) as u8, 0x00, 0xEB]),
            2 => code.extend_from_slice(&[(rng.below(64) as u8) << 2 | 1, 0, 0, 0x94]),
            3 => code.extend_from_slice(&[0x48, 0, rng.below(256) as u8, 0x01]),
            _ => code.extend(rng.bytes(4)),
        }
    }
    v.push(("code-200k".into(), code));
    v
}

const THEIRS: &str = r#"
import lzma, os, sys
d = sys.argv[1]
L2 = lzma.FILTER_LZMA2
chains = {
    'plain': None,
    'delta4': [{'id': lzma.FILTER_DELTA, 'dist': 4}, {'id': L2, 'preset': 6}],
    'x86': [{'id': lzma.FILTER_X86}, {'id': L2, 'preset': 6}],
    'x86at': [{'id': lzma.FILTER_X86, 'start_offset': 0x1000}, {'id': L2, 'preset': 6}],
    'ppc': [{'id': lzma.FILTER_POWERPC}, {'id': L2, 'preset': 6}],
    'ia64': [{'id': lzma.FILTER_IA64}, {'id': L2, 'preset': 6}],
    'arm': [{'id': lzma.FILTER_ARM}, {'id': L2, 'preset': 6}],
    'armt': [{'id': lzma.FILTER_ARMTHUMB}, {'id': L2, 'preset': 6}],
    'sparc': [{'id': lzma.FILTER_SPARC}, {'id': L2, 'preset': 6}],
    'three': [{'id': lzma.FILTER_DELTA, 'dist': 2}, {'id': lzma.FILTER_X86}, {'id': lzma.FILTER_ARM}, {'id': L2, 'preset': 1}],
}
for f in sorted(os.listdir(d)):
    if not f.endswith('.raw'):
        continue
    n, data = f[:-4], open(os.path.join(d, f), 'rb').read()
    w = lambda name, b: open(os.path.join(d, n + '.' + name), 'wb').write(b)
    for p in (0, 1, 6, 9):
        w('p%d.xz' % p, lzma.compress(data, preset=p))
    w('pe.xz', lzma.compress(data, preset=9 | lzma.PRESET_EXTREME))
    for c in (lzma.CHECK_NONE, lzma.CHECK_CRC32, lzma.CHECK_SHA256):
        w('c%d.xz' % c, lzma.compress(data, check=c, preset=1))
    for name, chain in chains.items():
        if chain:
            w(name + '.xz', lzma.compress(data, filters=chain))
    w('p6.lzma', lzma.compress(data, format=lzma.FORMAT_ALONE))
    w('lc0lp2pb0.raw1', lzma.compress(data, format=lzma.FORMAT_RAW, filters=[{'id': lzma.FILTER_LZMA1, 'lc': 0, 'lp': 2, 'pb': 0, 'dict_size': 1 << 16}]))
    w('lc4pb4.raw1', lzma.compress(data, format=lzma.FORMAT_RAW, filters=[{'id': lzma.FILTER_LZMA1, 'lc': 4, 'lp': 0, 'pb': 4, 'dict_size': 1 << 20}]))
    w('lc1lp3.raw2', lzma.compress(data, format=lzma.FORMAT_RAW, filters=[{'id': L2, 'lc': 1, 'lp': 3, 'pb': 1, 'dict_size': 1 << 16}]))
    w('p6.raw2', lzma.compress(data, format=lzma.FORMAT_RAW, filters=[{'id': L2, 'preset': 6, 'dict_size': 1 << 23}]))
    # Ours, decoded by liblzma.
    for name in os.listdir(d):
        if not name.startswith(n + '.ours.'):
            continue
        ours, kind = open(os.path.join(d, name), 'rb').read(), name[len(n) + 6:]
        if kind.endswith('.xz'):
            got = lzma.decompress(ours, format=lzma.FORMAT_XZ)
        elif kind.endswith('.lzma'):
            got = lzma.decompress(ours, format=lzma.FORMAT_ALONE)
        elif kind.endswith('.raw1'):
            got = lzma.decompress(ours, format=lzma.FORMAT_RAW, filters=[{'id': lzma.FILTER_LZMA1, 'dict_size': 1 << 23}])
        elif kind.endswith('.raw2'):
            got = lzma.decompress(ours, format=lzma.FORMAT_RAW, filters=[{'id': L2, 'dict_size': 1 << 23}])
        assert got == data, (n, kind)
print('ok')
"#;

fn ours(name: &str, data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut v = Vec::new();
    for level in [0, 6, 9] {
        v.push((format!("{name}.ours.p{level}.xz"), xz::compress(data, level)));
        v.push((format!("{name}.ours.p{level}.lzma"), lzma::compress(data, level)));
    }
    for (tag, check, filters) in [
        ("none", Check::None, vec![]),
        ("crc32", Check::Crc32, vec![]),
        ("sha256", Check::Sha256, vec![]),
        ("delta", Check::Crc64, vec![Filter::Delta(4)]),
        ("x86", Check::Crc64, vec![Filter::Branch(BranchFilter::X86, 0)]),
        ("ppc", Check::Crc64, vec![Filter::Branch(BranchFilter::PowerPc, 0)]),
        ("ia64", Check::Crc64, vec![Filter::Branch(BranchFilter::Ia64, 0)]),
        ("arm", Check::Crc64, vec![Filter::Branch(BranchFilter::Arm, 4096)]),
        ("armt", Check::Crc64, vec![Filter::Branch(BranchFilter::ArmThumb, 0)]),
        ("sparc", Check::Crc64, vec![Filter::Branch(BranchFilter::Sparc, 0)]),
        ("arm64", Check::Crc64, vec![Filter::Branch(BranchFilter::Arm64, 0)]),
        (
            "three",
            Check::Crc32,
            vec![Filter::Delta(2), Filter::Branch(BranchFilter::X86, 0), Filter::Branch(BranchFilter::Arm, 0)],
        ),
    ] {
        let opts = XzOptions { lzma: LzmaOptions::level(3), check, filters };
        v.push((format!("{name}.ours.{tag}.xz"), xz::compress_with(data, &opts)));
    }
    let opts = LzmaOptions::level(6);
    v.push((format!("{name}.ours.marker.raw1"), lzma::compress_raw(data, &opts, true)));
    v.push((format!("{name}.ours.p6.raw2"), lzma2::compress(data, &opts)));
    v
}

#[test]
fn liblzma_round_trips() {
    if !common::have("python3", "--version") {
        return missing("python3");
    }
    let dir = common::temp_dir("xz-lib");
    let corpus = corpus();
    for (name, data) in &corpus {
        fs::write(dir.join(format!("{name}.raw")), data).unwrap();
        for (f, bytes) in ours(name, data) {
            fs::write(dir.join(f), bytes).unwrap();
        }
    }
    assert_eq!(python(THEIRS, &[&dir]).trim(), "ok");
    for (name, data) in &corpus {
        let read = |f: &str| fs::read(dir.join(format!("{name}.{f}"))).unwrap();
        for f in [
            "p0.xz",
            "p1.xz",
            "p6.xz",
            "p9.xz",
            "pe.xz",
            "c0.xz",
            "c1.xz",
            "c10.xz",
            "delta4.xz",
            "x86.xz",
            "x86at.xz",
            "ppc.xz",
            "ia64.xz",
            "arm.xz",
            "armt.xz",
            "sparc.xz",
            "three.xz",
        ] {
            let packed = read(f);
            let s = xz::read_stream(&packed, usize::MAX).unwrap_or_else(|e| panic!("{name}.{f}: {e}"));
            assert!(s.data == *data, "{name}.{f} decodes differently");
            assert_eq!(s.consumed, packed.len(), "{name}.{f}");
            assert_eq!(xz::uncompressed_size(&packed), Some(data.len() as u64), "{name}.{f}");
        }
        assert!(lzma::decompress(&read("p6.lzma")).unwrap() == *data, "{name}.p6.lzma");
        for (f, props) in [
            ("lc0lp2pb0.raw1", LzmaProps { lc: 0, lp: 2, pb: 0, dict_size: 1 << 16 }),
            ("lc4pb4.raw1", LzmaProps { lc: 4, lp: 0, pb: 4, dict_size: 1 << 20 }),
        ] {
            let packed = read(f);
            let d =
                lzma::decompress_raw(&packed, &props, None, usize::MAX).unwrap_or_else(|e| panic!("{name}.{f}: {e}"));
            assert!(d.data == *data && d.consumed == packed.len(), "{name}.{f}");
        }
        for (f, dict) in [("lc1lp3.raw2", 1 << 16), ("p6.raw2", 1 << 23)] {
            let packed = read(f);
            let d =
                lzma2::decompress_with_limit(&packed, dict, usize::MAX).unwrap_or_else(|e| panic!("{name}.{f}: {e}"));
            assert!(d.data == *data && d.consumed == packed.len(), "{name}.{f}");
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn xz_program_round_trips() {
    if !common::have("xz", "--version") {
        return missing("xz");
    }
    let dir = common::temp_dir("xz-prog");
    let riscv = Command::new("xz")
        .args(["--riscv", "--lzma2", "-c"])
        .stdin(std::process::Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success());
    for (name, data) in corpus().into_iter().filter(|(n, _)| {
        ["words-300k", "code-200k", "random-200k", "zeros-1M", "sources", "random-5"].contains(&n.as_str())
    }) {
        let raw = dir.join(&name);
        fs::write(&raw, &data).unwrap();
        let mut variants: Vec<Vec<&str>> = vec![
            vec!["-0"],
            vec!["-3"],
            vec!["-6"],
            vec!["-9e"],
            vec!["--arm64", "--lzma2=preset=6"],
            vec!["--format=lzma", "-6"],
            vec!["--check=sha256", "-1"],
        ];
        if riscv {
            variants.push(vec!["--riscv", "--lzma2=preset=6"]);
        }
        for args in variants {
            let out = Command::new("xz").args(&args).arg("-c").arg(&raw).output().unwrap();
            assert!(out.status.success(), "xz {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            let got = if args.contains(&"--format=lzma") {
                lzma::decompress(&out.stdout)
            } else {
                xz::decompress(&out.stdout).map_err(|_| lzma::LzmaError::TrailingData)
            };
            assert!(got.as_deref() == Ok(&data[..]), "xz {args:?} {name}");
        }
        for (f, bytes) in ours(&name, &data) {
            let p = dir.join(&f);
            fs::write(&p, bytes).unwrap();
            if f.ends_with(".raw1") || f.ends_with(".raw2") {
                continue;
            }
            let fmt = if f.ends_with(".lzma") { "--format=lzma" } else { "--format=xz" };
            let out = Command::new("xz").args(["-dc", fmt]).arg(&p).output().unwrap();
            assert!(out.status.success(), "xz -d {f}: {}", String::from_utf8_lossy(&out.stderr));
            assert!(out.stdout == data, "xz -d {f} differs");
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

/// Decodes each case as liblzma does; prints the output's length and
/// SHA-256 and the bytes used, or the error.
const LIBLZMA_DECODE: &str = r#"
import hashlib, lzma, os, sys
d = sys.argv[1]
for f in sorted(os.listdir(d), key=lambda f: int(f.split('.')[0])):
    data = open(os.path.join(d, f), 'rb').read()
    kind = f.split('.')[1]
    if kind == 'xz':
        dec = lzma.LZMADecompressor(format=lzma.FORMAT_XZ)
    elif kind == 'lzma':
        dec = lzma.LZMADecompressor(format=lzma.FORMAT_ALONE)
    elif kind == 'raw1':
        dec = lzma.LZMADecompressor(format=lzma.FORMAT_RAW, filters=[{'id': lzma.FILTER_LZMA1, 'lc': 3, 'lp': 0, 'pb': 2, 'dict_size': 1 << 16}])
    else:
        dec = lzma.LZMADecompressor(format=lzma.FORMAT_RAW, filters=[{'id': lzma.FILTER_LZMA2, 'dict_size': 1 << 16}])
    try:
        out = dec.decompress(data, max_length=1 << 22)
    except lzma.LZMAError:
        print(f, 'error')
        continue
    if not dec.eof:
        print(f, 'unfinished')
        continue
    print(f, 'ok', len(data) - len(dec.unused_data), hashlib.sha256(out).hexdigest())
"#;

#[test]
fn damaged_streams_agree_with_liblzma() {
    if !common::have("python3", "--version") {
        return missing("python3");
    }
    let dir = common::temp_dir("xz-damaged");
    let mut rng = common::Rng::new(2027);
    let small = LzmaOptions { props: LzmaProps::new(1 << 16), nice_len: 32, depth: 8 };
    let texts =
        [b"hello, hello, hello, lzma".repeat(3), rng.bytes(500), [vec![9u8; 300], b"abcabcabd".repeat(40)].concat()];
    let mut bases: Vec<(&str, Vec<u8>)> = Vec::new();
    for t in &texts {
        bases.push(("raw1", lzma::compress_raw(t, &small, true)));
        bases.push(("raw2", lzma2::compress(t, &small)));
        bases.push(("lzma", lzma::compress_file(t, &small)));
        bases.push(("xz", xz::compress_with(t, &XzOptions { lzma: small, check: Check::Crc32, filters: vec![] })));
    }
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
    for (kind, base) in &bases {
        cases.push((kind, [&base[..], b"tail"].concat()));
        for _ in 0..150 {
            let mut c = base.clone();
            let i = rng.below(c.len() as u64) as usize;
            match rng.below(3) {
                0 => c[i] ^= 1 << rng.below(8),
                1 => c[i] = rng.below(256) as u8,
                _ => c.truncate(i),
            }
            cases.push((kind, c));
        }
    }
    for (i, (kind, c)) in cases.iter().enumerate() {
        fs::write(dir.join(format!("{i}.{kind}")), c).unwrap();
    }
    let report = python(LIBLZMA_DECODE, &[&dir]);
    let mut accepted = 0;
    for line in report.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let i: usize = f[0].split('.').next().unwrap().parse().unwrap();
        let (kind, input) = &cases[i];
        let limit = 1 << 22;
        let ours: Result<(Vec<u8>, usize), String> = match *kind {
            "raw1" => lzma::decompress_raw(input, &small.props, None, limit)
                .map(|d| (d.data, d.consumed))
                .map_err(|e| e.to_string()),
            "raw2" => lzma2::decompress_with_limit(input, 1 << 16, limit)
                .map(|d| (d.data, d.consumed))
                .map_err(|e| e.to_string()),
            "lzma" => {
                // liblzma takes lc + lp at most 4; this crate, as 7-Zip, up to 12.
                if lzma::parse_header(input).is_ok_and(|(p, _)| p.lc + p.lp > 4) {
                    continue;
                }
                lzma::read_file(input, limit).map(|d| (d.data, d.consumed)).map_err(|e| e.to_string())
            }
            _ => xz::read_stream(input, limit).map(|s| (s.data, s.consumed)).map_err(|e| e.to_string()),
        };
        match (f[1], ours) {
            ("ok", Ok((data, consumed))) => {
                accepted += 1;
                assert_eq!(
                    (consumed.to_string(), common::sha256_hex(&data)),
                    (f[2].to_string(), f[3].to_string()),
                    "case {i} ({kind})"
                );
            }
            ("ok", Err(e)) => panic!("case {i} ({kind}): liblzma accepts, we fail with {e}"),
            (_, Ok(_)) => panic!("case {i} ({kind}): liblzma fails ({}), we accept", f[1]),
            (_, Err(_)) => {}
        }
    }
    assert_eq!(report.lines().count(), cases.len());
    assert!(accepted >= 12, "{accepted} accepted");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn seven_zip_round_trips() {
    let sz = ["7z", "7zz"].into_iter().find(|c| common::have(c, "i"));
    let Some(sz) = sz else { return missing("7z") };
    let dir = common::temp_dir("xz-7z");
    for (name, data) in corpus()
        .into_iter()
        .filter(|(n, _)| ["words-300k", "code-200k", "random-200k", "sources"].contains(&n.as_str()))
    {
        fs::write(dir.join(&name), &data).unwrap();
        for args in [vec!["-txz", "-mx1"], vec!["-txz", "-mx9"], vec!["-txz", "-mf=BCJ"]] {
            let archive = format!("{name}{}.xz", args[1]);
            let out =
                Command::new(sz).arg("a").args(&args).arg(&archive).arg(&name).current_dir(&dir).output().unwrap();
            assert!(out.status.success(), "7z a {args:?}: {}", String::from_utf8_lossy(&out.stdout));
            let got = xz::decompress(&fs::read(dir.join(&archive)).unwrap());
            assert!(got.as_deref() == Ok(&data[..]), "7z {args:?} {name}");
        }
        let ours = format!("{name}.ours.xz");
        fs::write(dir.join(&ours), xz::compress(&data, 6)).unwrap();
        let out = Command::new(sz).args(["x", "-so", &ours]).current_dir(&dir).output().unwrap();
        assert!(out.status.success(), "7z x on ours: {}", String::from_utf8_lossy(&out.stderr));
        assert!(out.stdout == data, "7z x on ours ({name}) differs");
    }
    let _ = fs::remove_dir_all(&dir);
}
