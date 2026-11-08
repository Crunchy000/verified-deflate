//! The LZMA, LZMA2 and `.xz` encoders on any data, at any level, with any
//! check and filters: C liblzma decodes what they write back to the
//! data, and so does this crate.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use lzma_sys::*;
use verified_deflate::filters::BranchFilter;
use verified_deflate::lzma::{self, LzmaOptions};
use verified_deflate::xz::{self, Check, Filter, XzOptions};

#[derive(Arbitrary, Debug)]
struct Input {
    level: u8,
    check: u8,
    filters: Vec<(u8, u8)>,
    repeat: bool,
    data: Vec<u8>,
}

const BRANCH: [BranchFilter; 7] = [
    BranchFilter::X86,
    BranchFilter::PowerPc,
    BranchFilter::Ia64,
    BranchFilter::Arm,
    BranchFilter::ArmThumb,
    BranchFilter::Sparc,
    BranchFilter::Arm64,
];

/// liblzma's `.xz` (or, with `alone`, `.lzma`) decoding of `input`,
/// which must be exactly one stream of `len` bytes.
fn reference(input: &[u8], len: usize, alone: bool) -> Option<Vec<u8>> {
    let mut out = vec![0u8; len + 1];
    // SAFETY: as in lzma_decode.rs.
    unsafe {
        let mut s: lzma_stream = std::mem::zeroed();
        let r = if alone { lzma_alone_decoder(&mut s, u64::MAX) } else { lzma_stream_decoder(&mut s, u64::MAX, 0) };
        if r != LZMA_OK {
            return None;
        }
        s.next_in = input.as_ptr();
        s.avail_in = input.len();
        s.next_out = out.as_mut_ptr();
        s.avail_out = out.len();
        let r = lzma_code(&mut s, LZMA_FINISH);
        let produced = out.len() - s.avail_out;
        let rest = s.avail_in;
        lzma_end(&mut s);
        (r == LZMA_STREAM_END && rest == 0).then(|| {
            out.truncate(produced);
            out
        })
    }
}

fuzz_target!(|input: Input| {
    let data = if input.repeat { input.data.repeat(50) } else { input.data };
    let level = input.level % 10;
    let check = [Check::None, Check::Crc32, Check::Crc64, Check::Sha256][usize::from(input.check % 4)];
    let filters: Vec<Filter> = input
        .filters
        .iter()
        .take(3)
        .map(|&(kind, arg)| match kind % 8 {
            7 => Filter::Delta(u16::from(arg) + 1),
            k => {
                let b = BRANCH[usize::from(k)];
                Filter::Branch(b, (u32::from(arg) * 4096) & !(b.alignment() - 1))
            }
        })
        .collect();
    let packed = xz::compress_with(&data, &XzOptions { lzma: LzmaOptions::level(level), check, filters });
    assert!(reference(&packed, data.len(), false).as_deref() == Some(&data[..]), "liblzma disagrees on .xz");
    assert!(xz::decompress(&packed).as_deref() == Ok(&data[..]), ".xz round trip failed");
    let file = lzma::compress(&data, level);
    assert!(reference(&file, data.len(), true).as_deref() == Some(&data[..]), "liblzma disagrees on .lzma");
    assert!(lzma::decompress(&file).as_deref() == Ok(&data[..]), ".lzma round trip failed");
});
