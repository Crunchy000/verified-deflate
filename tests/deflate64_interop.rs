//! Deflate64 (ZIP method 9) against references we did not write:
//!
//! - Python's `inflate64` package (a zlib fork with a 64 KiB window): we
//!   decode every stream its encoder produces, and on corrupted streams we
//!   accept exactly the ones its decoder accepts, with the same output.
//! - 7-Zip: we read the Deflate64 ZIP archives it writes.
//!
//! Each part is skipped if its tool is not installed, unless
//! `REQUIRE_INTEROP_TOOLS` is set, as it is in CI.

mod common;

use std::fs;
use std::process::Command;
use verified_deflate::zip::{ZipArchive, METHOD_DEFLATE64};
use verified_deflate::{decompress_deflate64_with_limit, decompress_with_limit};

/// Skips a test whose tool is missing, or fails it in CI.
fn missing(tool: &str) {
    assert!(std::env::var_os("REQUIRE_INTEROP_TOOLS").is_none(), "{tool} not found, and REQUIRE_INTEROP_TOOLS is set");
    eprintln!("{tool} not found; skipping");
}

/// Inputs that need Deflate64's extensions: matches longer than 258 bytes
/// and distances beyond 32 KiB.
fn deflate64_samples() -> Vec<(String, Vec<u8>)> {
    let mut rng = common::Rng::new(64);
    let block = rng.bytes(50_000);
    let mut far = block.clone();
    far.extend_from_slice(&block); // one match of 50000 at distance 50000
                                   // Compressible text whose second half repeats the first, 45000 back.
    let text: Vec<u8> = (0..45_000).map(|_| b"abcdefgh \n"[rng.below(10) as usize]).collect();
    let text_repeat = [text.as_slice(), text.as_slice()].concat();
    vec![("far-repeat".into(), far), ("zeros-300k".into(), vec![0u8; 300_000]), ("text-repeat".into(), text_repeat)]
}

/// For each `<name>.raw`, writes `<name>.d64` (the stream inflate64's
/// encoder makes). For each stream of at most 64 KiB, it also writes
/// corrupted copies `<name>.bad<i>` with the reference decoder's verdict:
/// `<name>.bad<i>.ok` holding its output when it decodes the whole stream
/// within the limit, nothing when it does not.
const REFERENCE: &str = r#"
import os, random, sys, inflate64
d, limit = sys.argv[1], int(sys.argv[2])
rng = random.Random(1951)
made = bad = 0
for f in sorted(os.listdir(d)):
    if not f.endswith('.raw'):
        continue
    n = f[:-4]
    data = open(os.path.join(d, f), 'rb').read()
    c = inflate64.Deflater()
    s = c.deflate(data) + c.flush()
    open(os.path.join(d, n + '.d64'), 'wb').write(s)
    made += 1
    if len(s) > 65536:
        continue
    for i in range(40):
        t = bytearray(s)
        k = rng.randrange(4)
        if k == 0 and t:
            for _ in range(rng.randrange(1, 4)):
                p = rng.randrange(len(t)); t[p] ^= 1 << rng.randrange(8)
        elif k == 1 and t:
            t = t[:rng.randrange(len(t))]
        elif k == 2 and t:
            p = rng.randrange(len(t)); t[p] = rng.randrange(256)
        else:
            t[rng.randrange(len(t) + 1):0] = bytes(rng.randrange(256) for _ in range(rng.randrange(1, 9)))
        t = bytes(t)
        open(os.path.join(d, '%s.bad%d' % (n, i)), 'wb').write(t)
        try:
            o = inflate64.Inflater()
            out = o.inflate(t, limit + 1)
            ok = o.eof and len(out) <= limit
        except ValueError:
            ok = False
        if ok:
            open(os.path.join(d, '%s.bad%d.ok' % (n, i)), 'wb').write(out)
        bad += 1
print('inflate64: %d streams, %d corrupted' % (made, bad))
"#;

/// Output limit for corrupted streams, for both decoders.
const LIMIT: usize = 4 << 20;

#[test]
fn decodes_and_rejects_like_inflate64() {
    if !common::have("python3", "--version")
        || !Command::new("python3").args(["-I", "-c", "import inflate64"]).output().is_ok_and(|o| o.status.success())
    {
        return missing("python3 with inflate64");
    }
    let dir = common::temp_dir("deflate64");
    let mut inputs = common::corpus();
    inputs.extend(deflate64_samples());
    for (name, data) in &inputs {
        fs::write(dir.join(format!("{name}.raw")), data).unwrap();
    }
    let script = dir.join("reference.py");
    fs::write(&script, REFERENCE).unwrap();
    let out = Command::new("python3").arg("-I").arg(&script).arg(&dir).arg(LIMIT.to_string()).output().unwrap();
    eprintln!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "the reference script failed");

    // Every stream the reference encoder makes decodes to its input,
    // consuming all of it.
    let mut needs_deflate64 = 0;
    for (name, data) in &inputs {
        let s = fs::read(dir.join(format!("{name}.d64"))).unwrap();
        let d = decompress_deflate64_with_limit(&s, data.len()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(d.data == *data, "{name}: wrong output");
        assert_eq!(d.consumed, s.len(), "{name}");
        // Read as DEFLATE, a stream using the extensions means something else.
        if decompress_with_limit(&s, data.len()).map(|d| d.data) != Ok(data.clone()) {
            needs_deflate64 += 1;
        }
    }
    // The samples built to need the extensions do (and some of the corpus).
    eprintln!("{needs_deflate64} of {} streams use Deflate64's extensions", inputs.len());
    assert!(needs_deflate64 >= deflate64_samples().len());

    // On corrupted streams, we accept exactly what the reference accepts,
    // with the same output. (One known difference never arises here:
    // inflate64's code tables come from zlib's infback9, which rejects a
    // block with no distance codes, a case RFC 1951 allows and we accept;
    // see fuzz/infback9/README.md. These corruptions, which are fixed by
    // the seed, make no such block.)
    let (mut accepted, mut rejected) = (0, 0);
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let fname = path.file_name().unwrap().to_string_lossy().into_owned();
        let Some((_, i)) = fname.rsplit_once(".bad") else { continue };
        if i.ends_with(".ok") {
            continue;
        }
        let s = fs::read(&path).unwrap();
        let reference = fs::read(dir.join(format!("{fname}.ok"))).ok();
        let ours = decompress_deflate64_with_limit(&s, LIMIT).ok().map(|d| d.data);
        assert!(ours == reference, "{fname}: ours accepts {}, inflate64 {}", ours.is_some(), reference.is_some());
        if ours.is_some() {
            accepted += 1;
        } else {
            rejected += 1;
        }
    }
    eprintln!("corrupted streams: {accepted} accepted by both, {rejected} rejected by both");
    assert!(accepted + rejected > 300);
    let _ = fs::remove_dir_all(&dir);
}

/// The 7-Zip command, if installed.
fn seven_zip() -> Option<&'static str> {
    ["7z", "7zz", "7za"].into_iter().find(|c| common::have(c, "i"))
}

#[test]
fn reads_7zip_deflate64_archives() {
    let Some(sz) = seven_zip() else {
        return missing("7-Zip");
    };
    let dir = common::temp_dir("7z-deflate64");
    let src = dir.join("in");
    fs::create_dir_all(&src).unwrap();
    let mut inputs = common::corpus();
    inputs.extend(deflate64_samples());
    for (name, data) in &inputs {
        fs::write(src.join(name), data).unwrap();
    }
    for level in ["-mx=1", "-mx=5", "-mx=9"] {
        let zip = dir.join(format!("a{level}.zip"));
        let out = Command::new(sz)
            .args(["a", "-tzip", "-mm=Deflate64", level, "-bd", "-y"])
            .arg(&zip)
            .arg(".")
            .current_dir(&src)
            .output()
            .unwrap();
        assert!(out.status.success(), "7-Zip failed: {}", String::from_utf8_lossy(&out.stderr));
        let bytes = fs::read(&zip).unwrap();
        let archive = ZipArchive::parse(&bytes).unwrap();
        let mut deflate64 = 0;
        for (name, data) in &inputs {
            let i = archive.index_of(name).unwrap_or_else(|| panic!("{name} missing"));
            if archive.entries()[i].method == METHOD_DEFLATE64 {
                deflate64 += 1;
            }
            assert!(archive.read(i).unwrap_or_else(|e| panic!("{level} {name}: {e}")) == *data, "{level} {name}");
        }
        eprintln!("7-Zip {level}: {deflate64} of {} entries Deflate64 (the rest stored), all read back", inputs.len());
        // 7-Zip stores what does not compress, but not the inputs built
        // to need Deflate64.
        for (name, _) in deflate64_samples() {
            let i = archive.index_of(&name).unwrap();
            assert_eq!(archive.entries()[i].method, METHOD_DEFLATE64, "{level} {name}");
        }
    }
    let _ = fs::remove_dir_all(&dir);
}
