//! End-to-end checks of the `vzip` binary.

mod common;

use std::fs;
use std::process::Command;

fn vzip(args: &[&str], cwd: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vzip")).args(args).current_dir(cwd).output().unwrap()
}

#[test]
fn create_list_test_extract() {
    let dir = common::temp_dir("cli");
    let src = dir.join("tree");
    fs::create_dir_all(src.join("sub/deeper")).unwrap();
    fs::write(src.join("a.txt"), b"alpha ".repeat(1000)).unwrap();
    fs::write(src.join("sub/b.bin"), common::Rng::new(5).bytes(5000)).unwrap();
    fs::write(src.join("sub/deeper/empty"), b"").unwrap();

    let out = vzip(&["create", "-9", "t.zip", "tree"], &dir);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let out = vzip(&["list", "t.zip"], &dir);
    let listing = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    for name in ["tree/", "tree/a.txt", "tree/sub/b.bin", "tree/sub/deeper/empty"] {
        assert!(listing.contains(name), "missing {name} in\n{listing}");
    }

    let out = vzip(&["test", "t.zip"], &dir);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));

    let out = vzip(&["extract", "-d", "x", "t.zip"], &dir);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    for f in ["a.txt", "sub/b.bin", "sub/deeper/empty"] {
        assert_eq!(fs::read(dir.join("x/tree").join(f)).unwrap(), fs::read(src.join(f)).unwrap(), "{f}");
    }

    // Extracting again without -o refuses to overwrite.
    let out = vzip(&["extract", "-d", "x", "t.zip"], &dir);
    assert!(!out.status.success());
    let out = vzip(&["extract", "-o", "-d", "x", "t.zip"], &dir);
    assert!(out.status.success());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn raw_deflate_pipe_roundtrip() {
    let dir = common::temp_dir("cli-raw");
    let data = b"pipe pipe pipe pipe pipe ".repeat(200);
    fs::write(dir.join("in"), &data).unwrap();
    let sh = |cmd: &str| {
        Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(&dir)
            .env("VZIP", env!("CARGO_BIN_EXE_vzip"))
            .output()
            .unwrap()
    };
    assert!(sh("\"$VZIP\" deflate -9 < in > c && \"$VZIP\" inflate < c > out").status.success());
    assert_eq!(fs::read(dir.join("out")).unwrap(), data);
    assert!(fs::metadata(dir.join("c")).unwrap().len() < 100);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn refuses_path_traversal() {
    let dir = common::temp_dir("cli-evil");
    // Hand-build an archive whose entry name escapes the target directory.
    let mut zip = Vec::new();
    let name = b"../evil.txt";
    let data = b"gotcha";
    let crc = verified_deflate::crc32(data);
    let local = verified_deflate::zip::LocalHeader {
        version_needed: 20,
        flags: 0,
        method: 0,
        mod_time: 0,
        mod_date: 0x21,
        crc32: crc,
        compressed_size: data.len() as u32,
        uncompressed_size: data.len() as u32,
        name_len: name.len() as u16,
        extra_len: 0,
    };
    local.write(&mut zip);
    zip.extend_from_slice(name);
    zip.extend_from_slice(data);
    let cd_offset = zip.len() as u32;
    verified_deflate::zip::CentralHeader {
        version_made_by: 20,
        version_needed: 20,
        flags: 0,
        method: 0,
        mod_time: 0,
        mod_date: 0x21,
        crc32: crc,
        compressed_size: data.len() as u32,
        uncompressed_size: data.len() as u32,
        name_len: name.len() as u16,
        extra_len: 0,
        comment_len: 0,
        disk_start: 0,
        internal_attr: 0,
        external_attr: 0,
        local_offset: 0,
    }
    .write(&mut zip);
    zip.extend_from_slice(name);
    let cd_size = zip.len() as u32 - cd_offset;
    verified_deflate::zip::EndOfCentralDirectory {
        disk: 0,
        cd_disk: 0,
        entries_on_disk: 1,
        entries_total: 1,
        cd_size,
        cd_offset,
        comment_len: 0,
    }
    .write(&mut zip);
    fs::write(dir.join("evil.zip"), &zip).unwrap();
    fs::create_dir_all(dir.join("out")).unwrap();
    let out = vzip(&["extract", "-d", "out", "evil.zip"], &dir);
    assert!(!out.status.success());
    assert!(!dir.join("evil.txt").exists());
    let _ = fs::remove_dir_all(&dir);
}

/// Levels, file times and modes, the summary and listing totals, `test`
/// on a damaged archive, and raw `deflate -0`.
#[test]
fn options_times_modes_and_reports() {
    use std::time::{Duration, UNIX_EPOCH};
    let dir = common::temp_dir("cli-opts");
    let src = dir.join("tree");
    fs::create_dir_all(&src).unwrap();
    let text = b"compressible ".repeat(500);
    // Two-character names must not be taken for a `-N` level flag.
    fs::write(src.join("ab"), &text).unwrap();
    fs::write(src.join("a1"), b"1").unwrap();
    fs::write(src.join("old"), b"from before 1970").unwrap();
    let set_time = |name: &str, t| fs::File::options().write(true).open(src.join(name)).unwrap().set_modified(t);
    set_time("ab", UNIX_EPOCH + Duration::from_secs(1_623_760_496)).unwrap(); // 2021-06-15 12:34:56 UTC
                                                                              // Twenty years before 1970 (if the filesystem allows): clamps to 1980.
    let old_set = set_time("old", UNIX_EPOCH - Duration::from_secs(631_152_000)).is_ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(src.join("ab"), fs::Permissions::from_mode(0o640)).unwrap();
    }

    // Level 0 stores; the summary line reports files, bytes and ratio.
    let out = vzip(&["create", "-0", "../s.zip", "ab", "old", "a1"], &src);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let zip = fs::read(dir.join("s.zip")).unwrap();
    let total = text.len() + 16 + 1;
    let pct = zip.len() as f64 * 100.0 / total as f64;
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("../s.zip: 3 files, {total} -> {} bytes ({pct:.1}%)", zip.len())
    );
    let a = verified_deflate::zip::ZipArchive::parse(&zip).unwrap();
    let ab = &a.entries()[0];
    assert_eq!((ab.name.as_str(), ab.method), ("ab", 0));
    assert_eq!(ab.modified, verified_deflate::zip::DosDateTime::new(2021, 6, 15, 12, 34, 56).unwrap());
    if old_set {
        assert_eq!(a.entries()[1].modified, verified_deflate::zip::DosDateTime::EPOCH);
    }
    #[cfg(unix)]
    assert_eq!(ab.unix_mode, Some(0o100_640));

    // The listing's totals line.
    let out = vzip(&["list", "s.zip"], &dir);
    let listing = String::from_utf8_lossy(&out.stdout);
    let packed: u32 = a.entries().iter().map(|e| e.compressed_size).sum();
    assert!(listing.trim_end().ends_with(&format!("{total:>10} {packed:>10}  3 entries")), "{listing}");

    // Extraction restores the mode.
    let out = vzip(&["extract", "-d", "x", "s.zip"], &dir);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(dir.join("x/ab")).unwrap().permissions().mode() & 0o777, 0o640);
    }

    // Level 9 deflates; `test` passes it, and fails it once damaged.
    let out = vzip(&["create", "-9", "../d.zip", "ab"], &src);
    assert!(out.status.success());
    let d = fs::read(dir.join("d.zip")).unwrap();
    assert_eq!(verified_deflate::zip::ZipArchive::parse(&d).unwrap().entries()[0].method, 8);
    let out = vzip(&["test", "d.zip"], &dir);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("all 1 entries OK"));
    let mut bad = d.clone();
    bad[verified_deflate::zip::LocalHeader::SIZE + 2 + 3] ^= 0x55;
    fs::write(dir.join("bad.zip"), &bad).unwrap();
    let out = vzip(&["test", "bad.zip"], &dir);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("1 of 1 entries failed"));

    // Raw `deflate -0`: one stored block, five bytes of framing.
    let sh = |cmd: &str| {
        Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(&src)
            .env("VZIP", env!("CARGO_BIN_EXE_vzip"))
            .output()
            .unwrap()
    };
    assert!(sh("\"$VZIP\" deflate -0 < ab > ab.raw").status.success());
    assert_eq!(fs::metadata(src.join("ab.raw")).unwrap().len(), text.len() as u64 + 5);
    let _ = fs::remove_dir_all(&dir);
}
