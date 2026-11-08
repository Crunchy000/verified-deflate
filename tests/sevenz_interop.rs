//! 7z archives:
//!
//! - Archives this crate writes, at every level and with the header
//!   compressed or not, read back exactly, file by file.
//! - Damaged archives (single-byte changes and cuts) are refused or read
//!   with every CRC matching, never misread.
//! - With 7-Zip installed: archives it writes with each method this crate
//!   reads (LZMA2 at several levels, LZMA, Copy, BZip2, Deflate,
//!   Deflate64, BCJ, BCJ2, Delta, ARM64, non-solid, plain header) read
//!   exactly; it tests and extracts the archives this crate writes.
//!
//! The 7-Zip part is skipped if 7-Zip is missing, unless
//! `REQUIRE_INTEROP_TOOLS` is set, as it is in CI.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::process::Command;
use verified_deflate::sevenz::{EntryOptions, SevenZArchive, SevenZWriter};

fn missing(tool: &str) {
    assert!(std::env::var_os("REQUIRE_INTEROP_TOOLS").is_none(), "{tool} not found, and REQUIRE_INTEROP_TOOLS is set");
    eprintln!("{tool} not found; skipping");
}

/// Files to archive: the corpus, x86-like code, a directory and an empty file.
fn files() -> BTreeMap<String, Vec<u8>> {
    let mut v: BTreeMap<String, Vec<u8>> = common::corpus()
        .into_iter()
        .filter(|(n, _)| {
            ["random-0", "random-5", "random-200k", "words-300k", "zeros-1M", "sources", "records-400k"]
                .contains(&n.as_str())
        })
        .map(|(n, d)| (format!("dir/{n}"), d))
        .collect();
    let mut rng = common::Rng::new(5);
    let mut code = Vec::new();
    while code.len() < 100_000 {
        if rng.below(4) == 0 {
            code.extend_from_slice(&[0xE8, rng.below(256) as u8, rng.below(8) as u8, 0, 0]);
        } else {
            code.extend(rng.bytes(3));
        }
    }
    v.insert("code.exe".into(), code);
    v
}

/// Every file an archive holds, by name, read through whole folders.
fn contents(a: &SevenZArchive<'_>) -> BTreeMap<String, Vec<u8>> {
    let outs: Vec<Vec<u8>> = (0..a.folder_count()).map(|f| a.decode_folder(f, usize::MAX).unwrap()).collect();
    a.entries()
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.is_dir)
        .map(|(i, e)| {
            let data = match e.folder {
                Some((f, _)) => a.entry_data(i, &outs[f]).unwrap().to_vec(),
                None => Vec::new(),
            };
            (e.name.clone(), data)
        })
        .collect()
}

#[test]
fn round_trips() {
    let files = files();
    for level in [0, 1, 6, 9] {
        for compress_header in [true, false] {
            let mut w = SevenZWriter::new();
            w.add_directory("dir", &EntryOptions::default()).unwrap();
            for (name, data) in &files {
                w.add_file(name, data, &EntryOptions { mtime: Some(1_600_000_000), unix_mode: Some(0o640) }).unwrap();
            }
            let bytes = w.finish_with(level, compress_header);
            let a = SevenZArchive::parse(&bytes).unwrap();
            assert_eq!(contents(&a), files, "level {level}, header compressed {compress_header}");
            assert!(a.entries()[0].is_dir);
            assert!(a.entries()[1..]
                .iter()
                .all(|e| e.mtime_unix() == Some(1_600_000_000) && e.unix_mode() == Some(0o100_640)));
        }
    }
}

fn small_archive(compress_header: bool) -> Vec<u8> {
    let mut rng = common::Rng::new(78);
    let mut w = SevenZWriter::new();
    w.add_file("a", &b"some text, some text ".repeat(200), &EntryOptions::default()).unwrap();
    w.add_file("b", &rng.bytes(3000), &EntryOptions::default()).unwrap();
    w.add_directory("d", &EntryOptions::default()).unwrap();
    w.finish_with(5, compress_header)
}

#[test]
fn damage_is_detected() {
    let mut rng = common::Rng::new(77);
    for compress_header in [true, false] {
        let bytes = small_archive(compress_header);
        let good = contents(&SevenZArchive::parse(&bytes).unwrap());
        for _ in 0..2000 {
            let mut bad = bytes.clone();
            let i = rng.below(bad.len() as u64) as usize;
            if rng.below(4) == 0 {
                bad.truncate(i);
            } else {
                bad[i] ^= 1 << rng.below(8);
            }
            // Either an error, or the same data under each name that is
            // still there (a change the format cannot see, such as in an
            // unused attribute bit).
            let Ok(a) = SevenZArchive::parse(&bad) else { continue };
            for f in 0..a.folder_count() {
                let Ok(out) = a.decode_folder(f, 1 << 24) else { continue };
                for (k, e) in a.entries().iter().enumerate().filter(|(_, e)| e.folder.map(|x| x.0) == Some(f)) {
                    if let Ok(d) = a.entry_data(k, &out) {
                        assert!(good.get(&e.name).is_none_or(|g| g == d), "byte {i}: {} misread", e.name);
                    }
                }
            }
        }
    }
}

#[test]
fn seven_zip_round_trips() {
    let Some(sz) = ["7z", "7zz", "7za"].into_iter().find(|c| common::have(c, "i")) else { return missing("7-Zip") };
    let dir = common::temp_dir("7z-interop");
    let files = files();
    for (name, data) in &files {
        let p = dir.join("in").join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    }
    // Theirs, read here.
    for args in [
        vec!["-mx1"],
        vec!["-mx5"],
        vec!["-mx9"],
        vec!["-ms=off"],
        vec!["-mhc=off"],
        vec!["-m0=LZMA"],
        vec!["-m0=Copy"],
        vec!["-m0=BZip2"],
        vec!["-m0=Deflate"],
        vec!["-m0=Deflate64"],
        vec!["-mf=BCJ"],
        vec!["-mf=BCJ2"],
        vec!["-mf=Delta:2"],
        vec!["-mf=ARM64"],
    ] {
        let archive = dir.join(format!("theirs{}.7z", args.join("").replace(['=', ':'], "_")));
        let out = Command::new(sz)
            .arg("a")
            .arg("-t7z")
            .args(&args)
            .arg(&archive)
            .arg(".")
            .current_dir(dir.join("in"))
            .output()
            .unwrap();
        assert!(out.status.success(), "7z a {args:?}: {}", String::from_utf8_lossy(&out.stdout));
        let bytes = fs::read(&archive).unwrap();
        let a = SevenZArchive::parse(&bytes).unwrap_or_else(|e| panic!("{args:?}: {e}"));
        assert_eq!(contents(&a), files, "7z a {args:?} reads differently");
    }
    // Ours, read by 7-Zip.
    for level in [0, 5, 9] {
        let mut w = SevenZWriter::new();
        w.add_directory("dir", &EntryOptions::default()).unwrap();
        for (name, data) in &files {
            w.add_file(name, data, &EntryOptions { mtime: Some(1_600_000_000), unix_mode: Some(0o644) }).unwrap();
        }
        let archive = dir.join(format!("ours{level}.7z"));
        fs::write(&archive, w.finish(level)).unwrap();
        let out = Command::new(sz).arg("t").arg(&archive).output().unwrap();
        assert!(out.status.success(), "7z t ours{level}: {}", String::from_utf8_lossy(&out.stdout));
        let x = dir.join(format!("x{level}"));
        let out = Command::new(sz).arg("x").arg(&archive).arg(format!("-o{}", x.display())).output().unwrap();
        assert!(out.status.success(), "7z x ours{level}");
        for (name, data) in &files {
            assert!(fs::read(x.join(name)).unwrap() == *data, "7z x ours{level}: {name}");
        }
    }
    let _ = fs::remove_dir_all(&dir);
}
