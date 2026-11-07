//! The `v7z` command line: its commands and switches on zip, tar and
//! gzip, its refusal to write outside the output directory, and (when
//! 7-Zip is installed, as in CI) agreement with `7z` itself: each reads
//! the other's archives, and both list the same items.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn v7z(args: &[&str], cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_v7z")).args(args).current_dir(cwd).stdin(Stdio::null()).output().unwrap()
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap_or(-1)
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// A tree of files and directories (and a symlink), returning each file's
/// relative path and contents.
fn make_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut rng = common::Rng::new(7);
    let mut files = BTreeMap::new();
    for (name, len) in
        [("a.txt", 12), ("sub/b.bin", 70_000), ("sub/deeper/c.txt", 3000), ("space name", 1), ("empty", 0)]
    {
        let data = if name.ends_with(".txt") { b"text line\n".repeat(len / 10 + 1) } else { rng.bytes(len) };
        let p = root.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, &data).unwrap();
        files.insert(name.to_string(), data);
    }
    fs::create_dir_all(root.join("emptydir")).unwrap();
    files
}

/// Every regular file under `root`, by relative path.
fn read_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let m = fs::symlink_metadata(&p).unwrap();
            if m.is_dir() {
                walk(base, &p, out);
            } else if m.is_file() {
                out.insert(p.strip_prefix(base).unwrap().to_string_lossy().into_owned(), fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn fresh(tag: &str) -> PathBuf {
    let d = common::temp_dir(tag);
    fs::create_dir_all(d.join("src")).unwrap();
    d
}

#[test]
fn zip_and_tar_round_trip_with_every_command() {
    for archive in ["t.zip", "t.tar"] {
        let dir = fresh(&format!("v7z-rt-{archive}"));
        let files = make_tree(&dir.join("src"));
        // a: from inside src, so names are relative to it.
        let o = v7z(&["a", &format!("../{archive}"), "."], &dir.join("src"));
        assert_eq!(code(&o), 0, "{}", text(&o));
        assert!(text(&o).contains("Everything is Ok"));
        // t
        let o = v7z(&["t", archive], &dir);
        assert_eq!(code(&o), 0, "{}", text(&o));
        assert!(text(&o).contains(&format!("Files: {}", files.len())));
        // x reproduces the tree; e flattens it.
        let o = v7z(&["x", archive, "-oout"], &dir);
        assert_eq!(code(&o), 0, "{}", text(&o));
        assert_eq!(read_tree(&dir.join("out")), files);
        assert!(dir.join("out/emptydir").is_dir());
        let o = v7z(&["e", archive, "-oflat", "*.txt", "-r"], &dir);
        assert_eq!(code(&o), 0, "{}", text(&o));
        let flat: Vec<String> = read_tree(&dir.join("flat")).into_keys().collect();
        assert_eq!(flat, ["a.txt", "c.txt"]);
        // l lists every item; names select (a directory selects its contents).
        let o = v7z(&["l", archive], &dir);
        assert!(text(&o).contains("sub/deeper/c.txt") && text(&o).contains("5 files, 3 folders"), "{}", text(&o));
        let o = v7z(&["l", archive, "sub"], &dir);
        assert!(text(&o).contains("3 files") || text(&o).contains("2 files, 2 folders"), "{}", text(&o));
        // -slt technical listing.
        let o = v7z(&["l", "-slt", archive, "a.txt"], &dir);
        let t = text(&o);
        assert!(t.contains("Path = a.txt\nFolder = -\nSize = 20\n"), "{t}");
        // a on an existing archive updates it: a new file is added, a
        // changed one replaced, the rest kept.
        fs::write(dir.join("src/a.txt"), b"changed").unwrap();
        fs::write(dir.join("src/new.txt"), b"new").unwrap();
        let o = v7z(&["a", &format!("../{archive}"), "a.txt", "new.txt"], &dir.join("src"));
        assert_eq!(code(&o), 0, "{}", text(&o));
        assert!(text(&o).contains("Updating archive"));
        let o = v7z(&["x", archive, "-oupd", "-y"], &dir);
        assert_eq!(code(&o), 0);
        let mut expected = files.clone();
        expected.insert("a.txt".into(), b"changed".to_vec());
        expected.insert("new.txt".into(), b"new".to_vec());
        assert_eq!(read_tree(&dir.join("upd")), expected);
        // d removes a directory with everything in it.
        let o = v7z(&["d", archive, "sub"], &dir);
        assert_eq!(code(&o), 0, "{}", text(&o));
        let o = v7z(&["x", archive, "-odel"], &dir);
        assert_eq!(code(&o), 0);
        let left: Vec<String> = read_tree(&dir.join("del")).into_keys().collect();
        assert_eq!(left, ["a.txt", "empty", "new.txt", "space name"]);
        // -x! excludes.
        let o = v7z(&["x", archive, "-oexcl", "-x!*.txt"], &dir);
        assert_eq!(code(&o), 0);
        let left: Vec<String> = read_tree(&dir.join("excl")).into_keys().collect();
        assert_eq!(left, ["empty", "space name"]);
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn gzip_holds_one_file_and_pipes_work() {
    let dir = fresh("v7z-gz");
    let data = b"gzip me ".repeat(1000);
    fs::write(dir.join("src/data.txt"), &data).unwrap();
    let o = v7z(&["a", "d.txt.gz", "src/data.txt", "-mx9"], &dir);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let packed = fs::read(dir.join("d.txt.gz")).unwrap();
    assert_eq!(verified_deflate::gzip::decompress(&packed).unwrap(), data);
    // The item takes the stored name.
    let o = v7z(&["l", "-slt", "d.txt.gz"], &dir);
    assert!(text(&o).contains("Path = data.txt\n"), "{}", text(&o));
    let o = v7z(&["e", "d.txt.gz", "-oout"], &dir);
    assert_eq!(code(&o), 0);
    assert_eq!(fs::read(dir.join("out/data.txt")).unwrap(), data);
    // Two files do not fit in a gzip archive.
    fs::write(dir.join("src/other"), b"x").unwrap();
    let o = v7z(&["a", "two.gz", "src/data.txt", "src/other"], &dir);
    assert_eq!(code(&o), 7, "{}", text(&o));
    // -si and -so: a pipe through gzip.
    let mut child = Command::new(env!("CARGO_BIN_EXE_v7z"))
        .args(["a", "-tgzip", "-si", "-so", "x.gz"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(&data).unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(code(&o), 0, "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(verified_deflate::gzip::decompress(&o.stdout).unwrap(), data);
    assert!(!dir.join("x.gz").exists(), "-so must not write the archive file");
    let mut child = Command::new(env!("CARGO_BIN_EXE_v7z"))
        .args(["x", "-si", "-so", "-tgzip"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&packed).unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(code(&o), 0);
    assert_eq!(o.stdout, data);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn overwrite_modes() {
    let dir = fresh("v7z-ow");
    fs::write(dir.join("src/f.txt"), b"archived").unwrap();
    assert_eq!(code(&v7z(&["a", "a.zip", "src/f.txt"], &dir)), 0);
    fs::create_dir_all(dir.join("out/src")).unwrap();
    let existing = dir.join("out/src/f.txt");
    fs::write(&existing, b"old").unwrap();
    // No answer to the question (stdin closed): stop, and leave the file.
    let o = v7z(&["x", "a.zip", "-oout"], &dir);
    assert_eq!(code(&o), 255, "{}", text(&o));
    assert_eq!(fs::read(&existing).unwrap(), b"old");
    assert_eq!(code(&v7z(&["x", "a.zip", "-oout", "-aos"], &dir)), 0);
    assert_eq!(fs::read(&existing).unwrap(), b"old");
    assert_eq!(code(&v7z(&["x", "a.zip", "-oout", "-aou"], &dir)), 0);
    assert_eq!(fs::read(dir.join("out/src/f_1.txt")).unwrap(), b"archived");
    assert_eq!(code(&v7z(&["x", "a.zip", "-oout", "-aot"], &dir)), 0);
    assert_eq!(fs::read(&existing).unwrap(), b"archived");
    assert_eq!(fs::read(dir.join("out/src/f_2.txt")).unwrap(), b"old");
    fs::write(&existing, b"old again").unwrap();
    assert_eq!(code(&v7z(&["x", "a.zip", "-oout", "-y"], &dir)), 0);
    assert_eq!(fs::read(&existing).unwrap(), b"archived");
    let _ = fs::remove_dir_all(&dir);
}

/// A tar archive with the given (name, type, link target, data) entries,
/// written raw so that unsafe names can be made.
fn raw_tar(entries: &[(&str, u8, &str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, kind, link, data) in entries {
        let mut h = [0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..107].copy_from_slice(b"0000644");
        h[108..115].copy_from_slice(b"0000000");
        h[116..123].copy_from_slice(b"0000000");
        h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
        h[136..147].copy_from_slice(b"00000000000");
        h[156] = *kind;
        h[157..157 + link.len()].copy_from_slice(link.as_bytes());
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
        h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(512), 0);
    }
    out.extend_from_slice(&[0; 1024]);
    out
}

#[test]
fn extraction_stays_inside_the_output_directory() {
    let dir = fresh("v7z-evil");
    let evil = raw_tar(&[
        ("../escape.txt", b'0', "", b"no"),
        ("/abs.txt", b'0', "", b"abs"),
        ("ok/../../up.txt", b'0', "", b"no"),
        ("abslink", b'2', "/etc", b""),
        ("uplink", b'2', "../..", b""),
        ("dot", b'2', ".", b""),
        ("chain", b'2', "dot/..", b""),
        ("chain/through.txt", b'0', "", b"through"),
        ("good/f.txt", b'0', "", b"fine"),
        ("good/rel", b'2', "f.txt", b""),
    ]);
    fs::write(dir.join("evil.tar"), evil).unwrap();
    let o = v7z(&["x", "evil.tar", "-oout"], &dir);
    let t = text(&o);
    assert_eq!(code(&o), 2, "{t}");
    // Nothing lands outside out/; the absolute name is made relative.
    assert!(!dir.join("escape.txt").exists() && !dir.join("up.txt").exists());
    assert_eq!(fs::read(dir.join("out/abs.txt")).unwrap(), b"abs");
    assert_eq!(fs::read(dir.join("out/good/f.txt")).unwrap(), b"fine");
    assert!(t.contains("Dangerous path"), "{t}");
    #[cfg(unix)]
    {
        // Links out of the directory are not made. Links are made last, so
        // `chain/through.txt` was written inside out/, as a file in a
        // directory, before `chain` could become a link (and a link cannot
        // then replace that directory).
        assert!(fs::symlink_metadata(dir.join("out/abslink")).is_err());
        assert!(fs::symlink_metadata(dir.join("out/uplink")).is_err());
        assert_eq!(fs::read_link(dir.join("out/good/rel")).unwrap().to_str(), Some("f.txt"));
        assert!(t.contains("Dangerous symbolic link"), "{t}");
        assert!(!dir.join("through.txt").exists());
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn errors_have_7zip_exit_codes() {
    let dir = fresh("v7z-errors");
    fs::write(dir.join("src/f"), b"data data data data").unwrap();
    assert_eq!(code(&v7z(&["a", "a.zip", "src/f"], &dir)), 0);
    // A CRC failure is a fatal error (2), and the bad file is not written.
    let mut bytes = fs::read(dir.join("a.zip")).unwrap();
    let pos = bytes.windows(4).position(|w| w == b"data").unwrap_or(40);
    bytes[pos] ^= 0x20;
    fs::write(dir.join("bad.zip"), &bytes).unwrap();
    let o = v7z(&["t", "bad.zip"], &dir);
    assert_eq!(code(&o), 2, "{}", text(&o));
    assert!(text(&o).contains("CRC Failed") || text(&o).contains("Data Error"), "{}", text(&o));
    // Not an archive; a missing one; a bad command line; a missing input.
    fs::write(dir.join("junk"), b"not an archive at all").unwrap();
    assert_eq!(code(&v7z(&["l", "junk"], &dir)), 2);
    assert_eq!(code(&v7z(&["l", "missing.zip"], &dir)), 2);
    assert_eq!(code(&v7z(&["z", "a.zip"], &dir)), 7);
    assert_eq!(code(&v7z(&["l", "a.zip", "-qq"], &dir)), 7);
    assert_eq!(code(&v7z(&["a", "-pSecret", "c.zip", "src"], &dir)), 7);
    let o = v7z(&["a", "w.zip", "src/f", "src/nope"], &dir);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(dir.join("w.zip").exists());
    // No archive type for a name without an extension (7-Zip would make .7z).
    assert_eq!(code(&v7z(&["a", "noext", "src/f"], &dir)), 7);
    let _ = fs::remove_dir_all(&dir);
}

/// The 7-Zip command, if installed.
fn seven_zip() -> Option<&'static str> {
    ["7z", "7zz", "7za"].into_iter().find(|c| common::have(c, "i"))
}

fn missing(tool: &str) {
    assert!(std::env::var_os("REQUIRE_INTEROP_TOOLS").is_none(), "{tool} not found, and REQUIRE_INTEROP_TOOLS is set");
    eprintln!("{tool} not found; skipping");
}

fn run(cmd: &str, args: &[&str], cwd: &Path) -> Output {
    Command::new(cmd).args(args).current_dir(cwd).stdin(Stdio::null()).output().unwrap()
}

/// (path, is folder, size) of every item, from `l -slt` output.
fn slt_items(listing: &str) -> Vec<(String, bool, u64)> {
    let body = listing.split("----------\n").nth(1).unwrap_or("");
    let mut items = Vec::new();
    for block in body.split("\n\n") {
        let field = |k: &str| block.lines().find_map(|l| l.strip_prefix(&format!("{k} = "))).map(String::from);
        if let Some(path) = field("Path") {
            let folder = field("Folder").as_deref() == Some("+");
            let size = field("Size").and_then(|s| s.parse().ok()).unwrap_or(0);
            items.push((path.trim_end_matches('/').to_string(), folder, if folder { 0 } else { size }));
        }
    }
    items.sort();
    items
}

#[test]
fn agrees_with_7zip() {
    let Some(sz) = seven_zip() else {
        return missing("7-Zip");
    };
    let dir = fresh("v7z-7zip");
    let files = make_tree(&dir.join("src"));
    // Ours, read by 7-Zip.
    for (archive, extra) in [("ours.zip", ""), ("ours.tar", ""), ("ours.gz", "")] {
        let args: Vec<&str> =
            if archive.ends_with(".gz") { vec!["a", archive, "src/sub/b.bin"] } else { vec!["a", archive, "src"] };
        let _ = extra;
        assert_eq!(code(&v7z(&args, &dir)), 0, "{archive}");
        let o = run(sz, &["t", archive], &dir);
        assert_eq!(code(&o), 0, "7z t {archive}: {}", text(&o));
        let out = format!("x7-{archive}");
        let o = run(sz, &["x", archive, &format!("-o{out}"), "-y"], &dir);
        assert_eq!(code(&o), 0, "7z x {archive}: {}", text(&o));
        if archive.ends_with(".gz") {
            assert_eq!(fs::read(dir.join(&out).join("b.bin")).unwrap(), files["sub/b.bin"]);
        } else {
            let got: BTreeMap<String, Vec<u8>> = read_tree(&dir.join(&out))
                .into_iter()
                .map(|(k, v)| (k.strip_prefix("src/").unwrap().to_string(), v))
                .collect();
            assert_eq!(got, files, "7z x {archive}");
        }
        // Both list the same items.
        let theirs = slt_items(&String::from_utf8_lossy(&run(sz, &["l", "-slt", archive], &dir).stdout));
        let ours = slt_items(&String::from_utf8_lossy(&v7z(&["l", "-slt", archive], &dir).stdout));
        assert_eq!(ours, theirs, "l -slt {archive}");
    }
    // 7-Zip's, read by us: zip at three levels and with Deflate64, tar, gzip.
    for (archive, args) in [
        ("7z1.zip", vec!["-tzip", "-mx1"]),
        ("7z9.zip", vec!["-tzip", "-mx9"]),
        ("7z64.zip", vec!["-tzip", "-mm=Deflate64"]),
        ("7z.tar", vec!["-ttar"]),
        ("7z.gz", vec!["-tgzip"]),
    ] {
        let mut a = vec!["a"];
        a.extend(&args);
        a.push(archive);
        a.push(if archive.ends_with(".gz") { "src/sub/b.bin" } else { "src" });
        let o = run(sz, &a, &dir);
        assert_eq!(code(&o), 0, "7z a {archive}: {}", text(&o));
        let o = v7z(&["t", archive], &dir);
        assert_eq!(code(&o), 0, "v7z t {archive}: {}", text(&o));
        let out = format!("xv-{archive}");
        let o = v7z(&["x", archive, &format!("-o{out}")], &dir);
        assert_eq!(code(&o), 0, "v7z x {archive}: {}", text(&o));
        if archive.ends_with(".gz") {
            assert_eq!(fs::read(dir.join(&out).join("b.bin")).unwrap(), files["sub/b.bin"]);
        } else {
            let got: BTreeMap<String, Vec<u8>> = read_tree(&dir.join(&out))
                .into_iter()
                .map(|(k, v)| (k.strip_prefix("src/").unwrap().to_string(), v))
                .collect();
            assert_eq!(got, files, "v7z x {archive}");
        }
        let theirs = slt_items(&String::from_utf8_lossy(&run(sz, &["l", "-slt", archive], &dir).stdout));
        let ours = slt_items(&String::from_utf8_lossy(&v7z(&["l", "-slt", archive], &dir).stdout));
        assert_eq!(ours, theirs, "l -slt {archive}");
    }
    // A damaged archive is a fatal error (2) for both.
    let mut bytes = fs::read(dir.join("7z9.zip")).unwrap();
    let n = bytes.len();
    bytes[n / 3] ^= 0xff;
    fs::write(dir.join("damaged.zip"), &bytes).unwrap();
    assert_eq!(code(&run(sz, &["t", "damaged.zip"], &dir)), 2);
    assert_eq!(code(&v7z(&["t", "damaged.zip"], &dir)), 2);
    let _ = fs::remove_dir_all(&dir);
}
