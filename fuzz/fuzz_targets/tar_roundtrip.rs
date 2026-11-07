//! Archives the tar writer makes, from any entries and metadata, read back
//! exactly: names (long ones through pax headers), kinds, data, link
//! targets, modes, owners and times. Names the writer refuses are unsafe.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use verified_deflate::tar::{EntryKind, TarArchive, TarError, TarOptions, TarWriter};
use verified_deflate::zip::validate_name;

#[derive(Arbitrary, Debug)]
enum Kind {
    File(Vec<u8>),
    Directory,
    Symlink(String),
    HardLink(String),
}

#[derive(Arbitrary, Debug)]
struct Entry {
    name: String,
    kind: Kind,
    mode: u16,
    mtime: i64,
    uid: u64,
    gid: u64,
    uname: String,
    gname: String,
}

fuzz_target!(|entries: Vec<Entry>| {
    let mut w = TarWriter::new();
    let mut written = Vec::new();
    for e in &entries {
        // Names that fit their fields: no NUL (they end the field).
        if e.uname.contains('\0') || e.gname.contains('\0') {
            continue;
        }
        let opts = TarOptions {
            mode: u32::from(e.mode) & 0o7777,
            mtime: e.mtime,
            uid: e.uid,
            gid: e.gid,
            uname: e.uname.clone(),
            gname: e.gname.clone(),
        };
        let r = match &e.kind {
            Kind::File(data) => w.add_file(&e.name, data, &opts),
            Kind::Directory => w.add_directory(&e.name, &opts),
            Kind::Symlink(t) => w.add_symlink(&e.name, t, &opts),
            Kind::HardLink(t) => w.add_hard_link(&e.name, t, &opts),
        };
        match r {
            Ok(()) => written.push((e, opts)),
            Err(TarError::InvalidName(n)) => assert!(validate_name(&n).is_err() || n.ends_with('/')),
            Err(TarError::InvalidLinkTarget(_)) => assert!(matches!(e.kind, Kind::Symlink(_) | Kind::HardLink(_))),
            Err(other) => panic!("unexpected error {other:?}"),
        }
    }
    let bytes = w.finish();
    let tar = TarArchive::parse(&bytes).unwrap();
    assert_eq!(tar.len(), written.len());
    for (i, (e, opts)) in written.iter().enumerate() {
        let got = &tar.entries()[i];
        let (kind, name, data, link): (EntryKind, String, &[u8], &str) = match &e.kind {
            Kind::File(d) => (EntryKind::File, e.name.clone(), d, ""),
            Kind::Directory => {
                let n = if e.name.ends_with('/') { e.name.clone() } else { format!("{}/", e.name) };
                (EntryKind::Directory, n, &[], "")
            }
            Kind::Symlink(t) => (EntryKind::Symlink, e.name.clone(), &[], t),
            Kind::HardLink(t) => (EntryKind::HardLink, e.name.clone(), &[], t),
        };
        assert_eq!(got.kind, kind);
        assert_eq!(got.path, name.as_bytes());
        assert!(tar.data(i) == data);
        assert_eq!(got.link_target, link.as_bytes());
        if opts.mode != 0 {
            assert_eq!(got.mode, opts.mode);
        }
        assert_eq!((got.mtime, got.uid, got.gid), (opts.mtime, opts.uid, opts.gid));
        assert_eq!((got.uname.as_slice(), got.gname.as_slice()), (opts.uname.as_bytes(), opts.gname.as_bytes()));
    }
});
