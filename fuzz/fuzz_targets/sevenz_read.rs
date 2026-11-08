//! The 7z reader on damaged and arbitrary archives. The first byte picks
//! the input: the bytes themselves, or an archive the writer makes from
//! the rest (with a plain or compressed header) with up to eight bytes
//! changed. Either way the two header CRCs are then made to match, so
//! changes reach the header parser instead of stopping at a CRC.
//! Parsing and reading never panic, and what reads is consistent: each
//! entry's data lies in its folder's output, has its recorded size, and
//! matches its CRC when it has one.

#![no_main]

use libfuzzer_sys::fuzz_target;
use verified_deflate::crc32::crc32;
use verified_deflate::sevenz::{EntryOptions, SevenZArchive, SevenZWriter};

const LIMIT: usize = 1 << 22;

/// Rewrites the signature header's CRCs to match.
fn fix_crcs(a: &mut [u8]) {
    if a.len() < 32 {
        return;
    }
    let off = u64::from_le_bytes(a[12..20].try_into().unwrap());
    let size = u64::from_le_bytes(a[20..28].try_into().unwrap());
    let start = 32u64.saturating_add(off);
    if let Some(end) = start.checked_add(size).filter(|&e| e <= a.len() as u64) {
        let c = crc32(&a[start as usize..end as usize]);
        a[28..32].copy_from_slice(&c.to_le_bytes());
    }
    let c = crc32(&a[12..32]);
    a[8..12].copy_from_slice(&c.to_le_bytes());
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else { return };
    let mut archive = if mode % 2 == 0 {
        rest.to_vec()
    } else {
        // Up to four files and a directory from the input, then changes.
        let (spec, changes) = rest.split_at(rest.len().saturating_sub(16));
        let mut w = SevenZWriter::new();
        let mut pieces = spec.chunks(spec.len() / 4 + 1);
        for i in 0..4 {
            let piece = pieces.next().unwrap_or_default();
            let _ = w.add_file(&format!("f{i}"), piece, &EntryOptions { mtime: Some(i64::from(mode) << 20), unix_mode: Some(0o644) });
        }
        let _ = w.add_directory("dir", &EntryOptions::default());
        let mut a = w.finish_with(if mode & 2 != 0 { 1 } else { 0 }, mode & 4 != 0);
        for pair in changes.chunks_exact(2) {
            let i = usize::from(pair[0]) * a.len() / 256;
            a[i] ^= pair[1];
        }
        a
    };
    fix_crcs(&mut archive);
    let Ok(a) = SevenZArchive::parse(&archive) else { return };
    let _ = (a.len(), a.is_empty());
    for f in 0..a.folder_count() {
        let _ = (a.folder_methods(f), a.folder_packed_size(f));
        let Ok(out) = a.decode_folder(f, LIMIT) else { continue };
        for (i, e) in a.entries().iter().enumerate() {
            if e.folder.map(|x| x.0) != Some(f) {
                continue;
            }
            if let Ok(d) = a.entry_data(i, &out) {
                assert_eq!(d.len() as u64, e.size);
                if let Some(c) = e.crc {
                    assert_eq!(crc32(d), c);
                }
            }
        }
    }
});
