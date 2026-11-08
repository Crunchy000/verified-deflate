//! Archives the 7z writer makes, from any entries, at any level, with the
//! header compressed or not, read back exactly: names, kinds, data,
//! modification times and modes. Names the writer refuses are unsafe.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use verified_deflate::sevenz::{EntryOptions, SevenZArchive, SevenZWriter};
use verified_deflate::zip::validate_name;

#[derive(Arbitrary, Debug)]
enum Kind {
    File(Vec<u8>),
    Directory,
    Symlink(String),
}

#[derive(Arbitrary, Debug)]
struct Entry {
    name: String,
    kind: Kind,
    mtime: Option<u32>,
    mode: Option<u16>,
}

#[derive(Arbitrary, Debug)]
struct Input {
    level: u8,
    compress_header: bool,
    entries: Vec<Entry>,
}

fuzz_target!(|input: Input| {
    let mut w = SevenZWriter::new();
    let mut expected = Vec::new();
    for e in &input.entries {
        let opts = EntryOptions { mtime: e.mtime.map(i64::from), unix_mode: e.mode.map(|m| u32::from(m) & 0o7777) };
        let r = match &e.kind {
            Kind::File(d) => w.add_file(&e.name, d, &opts).map(|()| (e.name.clone(), false, d.clone())),
            Kind::Directory => w.add_directory(&e.name, &opts).map(|()| (e.name.trim_end_matches('/').to_string(), true, Vec::new())),
            Kind::Symlink(t) => w.add_symlink(&e.name, t, &opts).map(|()| (e.name.clone(), false, t.as_bytes().to_vec())),
        };
        match r {
            Ok(x) => expected.push((x, opts)),
            Err(_) => {
                let name = if matches!(e.kind, Kind::Directory) { e.name.trim_end_matches('/') } else { &e.name };
                assert!(validate_name(name).is_err(), "refused a safe name {name:?}");
            }
        }
    }
    let bytes = w.finish_with(input.level % 10, input.compress_header);
    let a = SevenZArchive::parse(&bytes).expect("own archive parses");
    assert_eq!(a.len(), expected.len());
    for (i, ((name, is_dir, data), opts)) in expected.iter().enumerate() {
        let e = &a.entries()[i];
        assert_eq!(&e.name, name);
        assert_eq!(e.is_dir, *is_dir);
        assert!(a.read(i).unwrap() == *data, "entry {i} reads back differently");
        assert_eq!(e.mtime_unix(), opts.mtime);
        if let Some(m) = opts.unix_mode {
            assert_eq!(e.unix_mode().map(|x| x & 0o7777), Some(m));
        }
    }
});
