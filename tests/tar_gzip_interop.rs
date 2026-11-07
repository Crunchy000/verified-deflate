//! gzip, zlib and tar against tools we did not write:
//!
//! - gzip and zlib streams from Python's `gzip` and `zlib` modules (every
//!   level, every window size, header fields, several members) and from GNU
//!   gzip decode exactly.
//! - Tar archives from Python's `tarfile` (ustar, GNU and pax formats, with
//!   long names and link targets, links, devices, big IDs, old and
//!   fractional times) and from GNU tar (gnu, oldgnu, posix, ustar, v7)
//!   read with every field right.
//! - Tar archives we write are read by `tarfile` and extracted by GNU tar
//!   with the same names, contents and metadata.
//!
//! Each part is skipped if its tool is missing, unless
//! `REQUIRE_INTEROP_TOOLS` is set, as it is in CI.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use verified_deflate::tar::{safe_path, EntryKind, TarArchive, TarOptions, TarWriter};
use verified_deflate::{crc32, gzip, zlib};

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

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

const MAKE_STREAMS: &str = r#"
import gzip, io, os, sys, zlib
d = sys.argv[1]
for f in sorted(os.listdir(d)):
    if not f.endswith('.raw'):
        continue
    n, data = f[:-4], open(os.path.join(d, f), 'rb').read()
    for level in (0, 1, 6, 9):
        open(os.path.join(d, '%s.%d.gz' % (n, level)), 'wb').write(gzip.compress(data, level, mtime=level))
        for wbits in (9, 12, 15):
            c = zlib.compressobj(level, zlib.DEFLATED, wbits)
            open(os.path.join(d, '%s.%d.%d.zlib' % (n, level, wbits)), 'wb').write(c.compress(data) + c.flush())
    # Header fields and two members.
    b = io.BytesIO()
    with gzip.GzipFile(filename='name.txt', mode='wb', fileobj=b, mtime=1234567890) as g:
        g.write(data[:len(data) // 2])
    b.write(gzip.compress(data[len(data) // 2:]))
    open(os.path.join(d, '%s.multi.gz' % n), 'wb').write(b.getvalue())
"#;

#[test]
fn gzip_and_zlib_from_reference_tools() {
    if !common::have("python3", "--version") {
        return missing("python3");
    }
    let dir = common::temp_dir("gz-interop");
    let corpus = common::corpus();
    for (name, data) in &corpus {
        fs::write(dir.join(format!("{name}.raw")), data).unwrap();
    }
    python(MAKE_STREAMS, &[&dir]);
    let gnu_gzip = common::have("gzip", "--version");
    if gnu_gzip {
        for (name, _) in &corpus {
            for level in ["-1", "-9"] {
                let out = Command::new("gzip")
                    .args(["-c", "-N", level])
                    .arg(dir.join(format!("{name}.raw")))
                    .output()
                    .unwrap();
                assert!(out.status.success());
                fs::write(dir.join(format!("{name}.gnu{level}.gz")), out.stdout).unwrap();
            }
        }
    } else {
        missing("gzip");
    }
    let mut checked = 0;
    for (name, data) in &corpus {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let f = path.file_name().unwrap().to_string_lossy().into_owned();
            if !f.starts_with(&format!("{name}.")) || f.ends_with(".raw") {
                continue;
            }
            let s = fs::read(&path).unwrap();
            let out = if f.ends_with(".gz") {
                gzip::decompress_with_limit(&s, data.len()).unwrap_or_else(|e| panic!("{f}: {e}"))
            } else {
                let d = zlib::decompress_with_limit(&s, data.len()).unwrap_or_else(|e| panic!("{f}: {e}"));
                assert_eq!(d.consumed, s.len(), "{f}");
                d.data
            };
            assert!(out == *data, "{f}: wrong output");
            if f.ends_with(".multi.gz") {
                let m = gzip::read_member(&s, data.len()).unwrap();
                assert_eq!(m.header.filename.as_deref(), Some(&b"name.txt"[..]), "{f}");
                assert_eq!(m.header.mtime, 1234567890);
            }
            if f.contains(".gnu-") {
                let m = gzip::read_member(&s, data.len()).unwrap();
                assert_eq!(m.header.filename.as_deref(), Some(format!("{name}.raw").as_bytes()), "{f}");
            }
            checked += 1;
        }
    }
    eprintln!("decoded {checked} gzip and zlib streams");
    assert!(checked > 200);
    let _ = fs::remove_dir_all(&dir);
}

/// Python's description of each entry of every `*.tar` in a directory,
/// one per line: archive, name, kind, size, mode, uid, gid, mtime,
/// link target, uname, gname, CRC-32 of the data (names hex-encoded).
const DESCRIBE: &str = r#"
import os, sys, tarfile, zlib
d = sys.argv[1]
kinds = [('isreg', 'file'), ('isdir', 'dir'), ('issym', 'symlink'), ('islnk', 'hardlink'),
         ('ischr', 'chr'), ('isblk', 'blk'), ('isfifo', 'fifo')]
for f in sorted(os.listdir(d)):
    if not f.endswith('.tar'):
        continue
    with tarfile.open(os.path.join(d, f), 'r:') as t:
        for m in t.getmembers():
            kind = next(k for a, k in kinds if getattr(m, a)())
            crc = zlib.crc32(t.extractfile(m).read()) if m.isreg() else 0
            print('\t'.join([f, m.name.encode('utf-8', 'surrogateescape').hex(), kind, str(m.size if m.isreg() else 0),
                             '%o' % m.mode, str(m.uid), str(m.gid), str(int(m.mtime)),
                             m.linkname.encode('utf-8', 'surrogateescape').hex(),
                             m.uname.encode().hex(), m.gname.encode().hex(), str(crc)]))
"#;

/// Archives made by Python's tarfile in its three formats.
const MAKE_TARS: &str = r#"
import io, os, sys, tarfile
d = sys.argv[1]
def add(t, name, data=None, **kw):
    ti = tarfile.TarInfo(name)
    for k, v in kw.items():
        setattr(ti, k, v)
    if data is not None:
        ti.size = len(data)
        t.addfile(ti, io.BytesIO(data))
    else:
        t.addfile(ti)
for fmt, tag in ((tarfile.USTAR_FORMAT, 'ustar'), (tarfile.GNU_FORMAT, 'gnu'), (tarfile.PAX_FORMAT, 'pax')):
    with tarfile.open(os.path.join(d, 'py-%s.tar' % tag), 'w', format=fmt) as t:
        add(t, 'dir', type=tarfile.DIRTYPE, mode=0o750, mtime=1000)
        add(t, 'dir/file.txt', b'hello tar\n' * 100, mode=0o640, uid=1000, gid=100, uname='alice', gname='staff', mtime=1700000000)
        add(t, 'dir/empty', b'', mtime=5)
        add(t, 'dir/link', type=tarfile.SYMTYPE, linkname='file.txt', mode=0o777)
        add(t, 'hard', type=tarfile.LNKTYPE, linkname='dir/file.txt')
        add(t, 'fifo', type=tarfile.FIFOTYPE, mode=0o600)
        add(t, 'chr', type=tarfile.CHRTYPE, devmajor=1, devminor=3)
        add(t, 'p' * 150 + '/' + 'n' * 90, b'split name')
        if fmt != tarfile.USTAR_FORMAT:
            add(t, 'long/' + 'x' * 300, b'long name', mtime=7)
            add(t, 'longlink', type=tarfile.SYMTYPE, linkname='t' * 200)
            add(t, 'bigid', b'id', uid=2 ** 40, gid=3 ** 25)
        if fmt == tarfile.PAX_FORMAT:
            add(t, 'café/日本', b'unicode', mtime=-86400)
            add(t, 'frac', b'f', mtime=1700000000.75)
"#;

/// Our reading of every `*.tar` in `dir`, in the format of `DESCRIBE`.
fn describe_ours(dir: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    let mut names: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    for f in names.iter().filter(|f| f.ends_with(".tar")) {
        let bytes = fs::read(dir.join(f)).unwrap();
        let t = TarArchive::parse(&bytes).unwrap_or_else(|e| panic!("{f}: {e}"));
        for (i, e) in t.entries().iter().enumerate() {
            let kind = match e.kind {
                EntryKind::File => "file",
                EntryKind::Directory => "dir",
                EntryKind::Symlink => "symlink",
                EntryKind::HardLink => "hardlink",
                EntryKind::CharDevice => "chr",
                EntryKind::BlockDevice => "blk",
                EntryKind::Fifo => "fifo",
                _ => "other",
            };
            // tarfile drops a directory's trailing slash.
            let name = if e.is_dir() { e.path.strip_suffix(b"/").unwrap_or(&e.path) } else { &e.path };
            let crc = if e.kind == EntryKind::File { crc32(t.data(i)) } else { 0 };
            lines.push(
                [
                    f.clone(),
                    hex(name),
                    kind.into(),
                    e.size.to_string(),
                    format!("{:o}", e.mode),
                    e.uid.to_string(),
                    e.gid.to_string(),
                    e.mtime.to_string(),
                    hex(&e.link_target),
                    hex(&e.uname),
                    hex(&e.gname),
                    crc.to_string(),
                ]
                .join("\t"),
            );
        }
    }
    lines
}

#[test]
fn tar_from_python_tarfile() {
    if !common::have("python3", "--version") {
        return missing("python3");
    }
    let dir = common::temp_dir("tar-python");
    python(MAKE_TARS, &[&dir]);
    let theirs: Vec<String> = python(DESCRIBE, &[&dir]).lines().map(String::from).collect();
    let ours = describe_ours(&dir);
    for (a, b) in theirs.iter().zip(&ours) {
        assert_eq!(a, b, "tarfile vs ours");
    }
    assert_eq!(theirs.len(), ours.len());
    eprintln!("{} entries in 3 tarfile formats read alike", ours.len());
    assert!(ours.len() >= 30);
    let _ = fs::remove_dir_all(&dir);
}

/// A tree of files, directories and links to archive.
fn make_tree(root: &Path, long_names: bool) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut rng = common::Rng::new(77);
    let mut names = vec!["a.txt".to_string(), "sub/b.bin".into(), "sub/deeper/c".into(), "sp ace".into()];
    if long_names {
        names.push(format!("{}/{}", "d".repeat(120), "f".repeat(80)));
        names.push(format!("x/{}", "y".repeat(150)));
    }
    for (i, n) in names.iter().enumerate() {
        let data = rng.bytes(i * 700 + 3);
        let p = root.join(n);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, &data).unwrap();
        files.insert(n.clone(), data);
    }
    fs::create_dir_all(root.join("emptydir")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("a.txt", root.join("link")).unwrap();
    files
}

#[test]
fn tar_from_gnu_tar() {
    if !common::have("tar", "--version") {
        return missing("GNU tar");
    }
    let dir = common::temp_dir("tar-gnu");
    for (format, long) in [("gnu", true), ("oldgnu", true), ("posix", true), ("ustar", false), ("v7", false)] {
        let src = dir.join(format!("src-{format}"));
        let files = make_tree(&src, long);
        let archive = dir.join(format!("{format}.tar"));
        let out = Command::new("tar")
            .arg(format!("--format={format}"))
            .arg("-cf")
            .arg(&archive)
            .arg("-C")
            .arg(&src)
            .arg(".")
            .output()
            .unwrap();
        assert!(out.status.success(), "{format}: {}", String::from_utf8_lossy(&out.stderr));
        let bytes = fs::read(&archive).unwrap();
        let t = TarArchive::parse(&bytes).unwrap_or_else(|e| panic!("{format}: {e}"));
        let mut seen = 0;
        for (i, e) in t.entries().iter().enumerate() {
            let Some(name) = safe_path(&e.path) else { continue };
            if let Some(data) = files.get(&name) {
                assert_eq!(e.kind, EntryKind::File, "{format} {name}");
                assert!(t.data(i) == data.as_slice(), "{format} {name}");
                seen += 1;
            } else if name == "link" && format != "v7" {
                assert_eq!((e.kind, e.link_target.as_slice()), (EntryKind::Symlink, &b"a.txt"[..]), "{format}");
            } else if name == "emptydir" || name.starts_with("sub") || name.starts_with('d') || name == "x" {
                assert!(e.is_dir(), "{format} {name}");
            }
        }
        assert_eq!(seen, files.len(), "{format}: files found");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn our_tar_read_by_tarfile_and_gnu_tar() {
    let have_python = common::have("python3", "--version");
    let have_tar = common::have("tar", "--version");
    if !have_python {
        missing("python3");
    }
    if !have_tar {
        missing("GNU tar");
    }
    let dir = common::temp_dir("tar-ours");
    let o = |mode: u32, mtime: i64| TarOptions {
        mode,
        mtime,
        uid: 1000,
        gid: 100,
        uname: "alice".into(),
        gname: "staff".into(),
    };
    let mut w = TarWriter::new();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    w.add_directory("dir", &o(0o755, 1000)).unwrap();
    for (i, name) in ["dir/a.txt".to_string(), "b".into(), format!("long/{}", "z".repeat(200)), "caf\u{e9}.txt".into()]
        .into_iter()
        .enumerate()
    {
        let data = common::Rng::new(i as u64).bytes(i * 3000 + 1);
        w.add_file(&name, &data, &o(0o640, 1_700_000_000 + i as i64)).unwrap();
        files.push((name, data));
    }
    w.add_symlink("dir/link", "a.txt", &o(0o777, 5)).unwrap();
    w.add_symlink("longlink", &"t".repeat(150), &o(0o777, 5)).unwrap();
    fs::write(dir.join("ours.tar"), w.finish()).unwrap();
    // IDs and times beyond ustar's fields (GNU tar reads these, but will
    // not extract a uid over 2^32).
    let mut big = TarWriter::new();
    big.add_file("bigid", b"id", &TarOptions { uid: 1 << 40, mtime: -1, ..o(0o600, 0) }).unwrap();
    fs::write(dir.join("ours-big.tar"), big.finish()).unwrap();
    if have_python {
        // tarfile reads our archives exactly as we do.
        let theirs: Vec<String> = python(DESCRIBE, &[&dir]).lines().map(String::from).collect();
        assert_eq!(theirs, describe_ours(&dir));
        assert_eq!(theirs.len(), 8);
    }
    if have_tar {
        // GNU tar extracts it to the same files.
        let x = dir.join("x");
        fs::create_dir_all(&x).unwrap();
        let out = Command::new("tar").arg("-xf").arg(dir.join("ours.tar")).arg("-C").arg(&x).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        for (name, data) in &files {
            assert_eq!(&fs::read(x.join(name)).unwrap(), data, "{name}");
        }
        #[cfg(unix)]
        assert_eq!(fs::read_link(x.join("dir/link")).unwrap().to_str(), Some("a.txt"));
        let out = Command::new("tar").arg("-tvf").arg(dir.join("ours.tar")).output().unwrap();
        let listing = String::from_utf8_lossy(&out.stdout);
        assert!(listing.contains("alice/staff"), "{listing}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn hex_helpers_round_trip() {
    assert_eq!(unhex(&hex(b"\x00\xffab")), b"\x00\xffab");
}
