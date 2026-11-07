//! The tar reader on arbitrary bytes: parsing returns, and what it returns
//! is consistent: every entry's data lies in the archive with its recorded
//! size, entries do not overlap and come in order, and paths made safe
//! stay relative.

#![no_main]

use libfuzzer_sys::fuzz_target;
use verified_deflate::tar::{safe_path, TarArchive};

fuzz_target!(|data: &[u8]| {
    let Ok(tar) = TarArchive::parse(data) else {
        return;
    };
    assert!(tar.archive_len() <= data.len());
    let mut end = 0;
    for (i, e) in tar.entries().iter().enumerate() {
        assert!(e.header_offset >= end && e.data_offset > e.header_offset);
        assert_eq!(tar.data(i).len() as u64, e.size);
        end = e.data_offset + e.size as usize;
        assert!(end <= tar.archive_len());
        if let Some(p) = safe_path(&e.path) {
            assert!(!p.is_empty() && !p.starts_with('/'));
            assert!(p.split('/').all(|c| !c.is_empty() && c != "." && c != ".."));
        }
        let _ = (e.path_lossy(), e.is_dir());
    }
    for e in tar.entries() {
        if let Ok(name) = std::str::from_utf8(&e.path) {
            let last = tar.index_of(name).unwrap();
            assert_eq!(tar.entries()[last].path, e.path);
        }
    }
});
