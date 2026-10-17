//! Archives we write must pass Info-ZIP's `unzip -t` and Python's
//! `zipfile`; archives Info-ZIP's `zip` writes (every level) must read back
//! correctly with our reader. Each tool is skipped if not installed.

mod common;

use std::fs;
use std::process::Command;
use verified_deflate::zip::{FileOptions, ZipArchive, ZipWriter};

fn build_archive() -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
    let mut files = Vec::new();
    for (name, data) in common::corpus() {
        files.push((format!("corpus/{name}.bin"), data));
    }
    files.push((
        "unicod\u{e9}/\u{65e5}\u{672c}.txt".to_string(),
        "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}".repeat(100).into_bytes(),
    ));
    let mut w = ZipWriter::new();
    w.add_directory("corpus", &FileOptions::default()).unwrap();
    for (i, (name, data)) in files.iter().enumerate() {
        let opts = FileOptions { level: (i % 10) as u8, ..FileOptions::default() };
        w.add_file(name, data, &opts).unwrap();
    }
    (w.finish().unwrap(), files)
}

#[test]
fn our_archives_read_back() {
    let (zip, files) = build_archive();
    let a = ZipArchive::parse(&zip).unwrap();
    assert_eq!(a.len(), files.len() + 1);
    for (name, data) in &files {
        assert_eq!(&a.read_by_name(name).unwrap(), data, "{name}");
    }
}

#[test]
fn unzip_accepts_our_archives() {
    if !common::have("unzip", "-v") {
        eprintln!("unzip not found; skipping");
        return;
    }
    let dir = common::temp_dir("unzip");
    let (zip, files) = build_archive();
    let path = dir.join("ours.zip");
    fs::write(&path, &zip).unwrap();
    let out = Command::new("unzip").arg("-t").arg(&path).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success() && text.contains("No errors detected"), "{text}");
    // Extract and compare byte for byte.
    let x = dir.join("x");
    let out = Command::new("unzip").arg("-q").arg(&path).arg("-d").arg(&x).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    for (name, data) in files.iter().filter(|(n, _)| n.is_ascii()) {
        assert_eq!(&fs::read(x.join(name)).unwrap(), data, "{name}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn python_zipfile_accepts_our_archives() {
    if !common::have("python3", "--version") {
        eprintln!("python3 not found; skipping");
        return;
    }
    let dir = common::temp_dir("pyzip");
    let (zip, files) = build_archive();
    let path = dir.join("ours.zip");
    fs::write(&path, &zip).unwrap();
    let script = "import sys, zipfile, hashlib\n\
                  z = zipfile.ZipFile(sys.argv[1])\n\
                  assert z.testzip() is None\n\
                  for i in z.infolist():\n    \
                      print(i.filename, hashlib.sha256(z.read(i)).hexdigest())\n";
    let out = Command::new("python3").args(["-I", "-c", script]).arg(&path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let listing = String::from_utf8(out.stdout).unwrap();
    for (name, _) in &files {
        assert!(listing.lines().any(|l| l.starts_with(name.as_str())), "zipfile did not list {name}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn we_read_info_zip_archives() {
    if !common::have("zip", "-v") {
        eprintln!("zip not found; skipping");
        return;
    }
    let dir = common::temp_dir("infozip");
    let src = dir.join("src");
    fs::create_dir_all(&src).unwrap();
    let corpus = common::corpus();
    for (name, data) in &corpus {
        fs::write(src.join(name), data).unwrap();
    }
    for level in 0..=9 {
        let path = dir.join(format!("l{level}.zip"));
        let out = Command::new("zip")
            .current_dir(&dir)
            .args(["-q", "-r", &format!("-{level}")])
            .arg(&path)
            .arg("src")
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let bytes = fs::read(&path).unwrap();
        let a = ZipArchive::parse(&bytes).unwrap();
        for (name, data) in &corpus {
            let got = a.read_by_name(&format!("src/{name}")).unwrap_or_else(|e| panic!("level {level} {name}: {e}"));
            assert_eq!(&got, data, "level {level} {name}");
        }
    }
    // Streamed archives use data descriptors (flag bit 3) with zero sizes in
    // the local headers; the reader must rely on the central directory.
    let out = Command::new("sh")
        .current_dir(&dir)
        .args(["-c", "cat src/words-300k | zip -q - - | cat > stream.zip"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let bytes = fs::read(dir.join("stream.zip")).unwrap();
    let a = ZipArchive::parse(&bytes).unwrap();
    assert_ne!(a.entries()[0].flags & 8, 0, "expected a data descriptor");
    let (_, words) = corpus.iter().find(|(n, _)| n == "words-300k").unwrap();
    assert_eq!(&a.read(0).unwrap(), words);
    let _ = fs::remove_dir_all(&dir);
}
