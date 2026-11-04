//! The ZIP reader on arbitrary bytes: parsing and reading any entry return
//! (Verus proves that much), and what they return is consistent: an entry
//! that reads successfully has its recorded size and CRC-32, and looking an
//! entry up by name finds the first entry of that name.

#![no_main]

use libfuzzer_sys::fuzz_target;
use verified_deflate::crc32;
use verified_deflate::zip::{ZipArchive, ZipError};

fuzz_target!(|data: &[u8]| {
    let Ok(archive) = ZipArchive::parse(data) else {
        return;
    };
    assert_eq!(archive.len(), archive.entries().len());
    assert_eq!(archive.is_empty(), archive.entries().is_empty());
    for (i, e) in archive.entries().iter().enumerate() {
        let _ = e.is_dir();
        let raw = archive.raw_data(i);
        let read = archive.read(i);
        if let Ok(out) = &read {
            assert_eq!(out.len(), e.uncompressed_size as usize);
            assert_eq!(crc32(out), e.crc32);
            assert!(raw.is_ok());
        }
        if let Ok(name) = std::str::from_utf8(&e.raw_name) {
            assert_eq!(e.name, name);
            let first = archive.entries().iter().position(|f| f.raw_name == e.raw_name);
            assert_eq!(archive.index_of(name), first);
            assert!(archive.read_by_name(name) == archive.read(first.unwrap()));
        }
    }
    assert!(archive.read(archive.len()) == Err(ZipError::NotFound));
});
