//! Archives made by the ZIP writer, from any entries and options, read back
//! exactly: the same names in the same order, the same contents, times
//! and file modes. Names the writer refuses must be invalid or repeated.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use verified_deflate::zip::{validate_name, DosDateTime, FileOptions, ZipArchive, ZipError, ZipWriter};
use verified_deflate::Strategy;

const STRATEGIES: [Strategy; 4] = [Strategy::Auto, Strategy::Stored, Strategy::Fixed, Strategy::Dynamic];

#[derive(Arbitrary, Debug)]
struct Entry {
    name: String,
    directory: bool,
    level: u8,
    strategy: u8,
    verify: bool,
    unix_mode: u32,
    unix_time: i64,
    data: Vec<u8>,
}

/// An entry the writer accepted, as the reader should see it.
struct Expected {
    name: String,
    directory: bool,
    data: Vec<u8>,
    options: FileOptions,
}

fuzz_target!(|entries: Vec<Entry>| {
    let mut w = ZipWriter::new();
    let mut expected: Vec<Expected> = Vec::new();
    for e in &entries {
        let options = FileOptions {
            level: e.level % 10,
            strategy: STRATEGIES[usize::from(e.strategy) % 4],
            modified: DosDateTime::from_unix_seconds(e.unix_time),
            unix_mode: e.unix_mode,
            verify: e.verify,
        };
        let (name, data, result) = if e.directory {
            let name = if e.name.ends_with('/') { e.name.clone() } else { format!("{}/", e.name) };
            (name, Vec::new(), w.add_directory(&e.name, &options))
        } else {
            (e.name.clone(), e.data.clone(), w.add_file(&e.name, &e.data, &options))
        };
        match result {
            Ok(()) => expected.push(Expected { name, directory: e.directory, data, options }),
            Err(ZipError::DuplicateName(_)) => assert!(expected.iter().any(|x| x.name == name)),
            Err(ZipError::InvalidName(_)) => {
                assert!(validate_name(&name).is_err() || (!e.directory && name.ends_with('/')))
            }
            Err(other) => panic!("unexpected error {other:?}"),
        }
    }
    let bytes = w.finish().unwrap();
    let archive = ZipArchive::parse(&bytes).unwrap();
    assert_eq!(archive.len(), expected.len());
    for (i, (x, e)) in expected.iter().zip(archive.entries()).enumerate() {
        assert_eq!(e.name, x.name);
        assert_eq!(e.raw_name, x.name.as_bytes());
        assert_eq!(e.is_dir(), x.directory);
        assert_eq!(e.modified, x.options.modified);
        if !x.directory {
            assert_eq!(e.unix_mode, Some(0o100_000 | (x.options.unix_mode & 0o7777)));
        }
        assert!(archive.read(i).unwrap() == x.data, "entry {i} reads back differently");
        assert_eq!(archive.index_of(&x.name), Some(i));
    }
});
