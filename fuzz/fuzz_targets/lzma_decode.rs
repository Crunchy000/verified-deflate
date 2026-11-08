//! The LZMA decoders against C liblzma (XZ Utils), on arbitrary bytes.
//! The first byte picks the form: a raw LZMA1 stream (with `lc`, `lp`,
//! `pb` from the second byte), a raw LZMA2 stream, a `.lzma` file or an
//! `.xz` stream. Each is accepted by both or by neither, with the same
//! output and the same number of bytes consumed. Where liblzma refuses
//! for its own reasons (`lc + lp` over 4, a dictionary over its memory
//! limit) the input is skipped.

#![no_main]

use libfuzzer_sys::fuzz_target;
use lzma_sys::*;
use verified_deflate::lzma::{self, LzmaProps};
use verified_deflate::{lzma2, xz};

/// Output limit for both decoders.
const LIMIT: usize = 1 << 20;
const DICT: u32 = 1 << 16;
const MEMLIMIT: u64 = 1 << 28;

enum Theirs {
    Accepted(Vec<u8>, usize),
    Rejected,
    /// liblzma would not try (memory limit, unsupported options).
    Skipped,
}

/// Runs an initialised liblzma decoder over all of `input`.
///
/// # Safety
///
/// `s` must be a decoder initialised by liblzma.
unsafe fn run(mut s: lzma_stream, input: &[u8]) -> Theirs {
    let mut out = vec![0u8; LIMIT + 1];
    s.next_in = input.as_ptr();
    s.avail_in = input.len();
    s.next_out = out.as_mut_ptr();
    s.avail_out = out.len();
    let r = lzma_code(&mut s, LZMA_FINISH);
    let produced = out.len() - s.avail_out;
    let consumed = input.len() - s.avail_in;
    lzma_end(&mut s);
    if r == LZMA_MEMLIMIT_ERROR || r == LZMA_MEM_ERROR {
        return Theirs::Skipped;
    }
    if r == LZMA_STREAM_END && produced <= LIMIT {
        out.truncate(produced);
        Theirs::Accepted(out, consumed)
    } else {
        Theirs::Rejected
    }
}

fn reference(kind: u8, props: LzmaProps, input: &[u8]) -> Theirs {
    // SAFETY: the stream is zero-initialised (LZMA_STREAM_INIT), the
    // options and filter array outlive the init call that copies them,
    // and `run` ends the decoder.
    unsafe {
        let mut s: lzma_stream = std::mem::zeroed();
        let r = match kind {
            0 | 1 => {
                let mut opts: lzma_options_lzma = std::mem::zeroed();
                lzma_lzma_preset(&mut opts, 6);
                opts.dict_size = DICT;
                opts.lc = u32::from(props.lc);
                opts.lp = u32::from(props.lp);
                opts.pb = u32::from(props.pb);
                let filters = [
                    lzma_filter {
                        id: if kind == 0 { LZMA_FILTER_LZMA1 } else { LZMA_FILTER_LZMA2 },
                        options: &mut opts as *mut _ as *mut _,
                    },
                    lzma_filter { id: LZMA_VLI_UNKNOWN, options: std::ptr::null_mut() },
                ];
                lzma_raw_decoder(&mut s, filters.as_ptr())
            }
            2 => lzma_alone_decoder(&mut s, MEMLIMIT),
            _ => lzma_stream_decoder(&mut s, MEMLIMIT, 0),
        };
        if r != LZMA_OK {
            return Theirs::Skipped;
        }
        run(s, input)
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&kind, rest)) = data.split_first() else { return };
    let kind = kind % 4;
    let (props, input) = match kind {
        0 | 1 => {
            let Some((&b, input)) = rest.split_first() else { return };
            let Ok(p) = LzmaProps::from_byte(b % 225, DICT) else { return };
            (p, input)
        }
        _ => (LzmaProps::new(DICT), rest),
    };
    // liblzma refuses lc + lp > 4; this crate takes up to 12, as 7-Zip.
    let lclp_ok = |p: &LzmaProps| p.lc + p.lp <= 4;
    if kind == 0 && !lclp_ok(&props) {
        return;
    }
    if kind == 2 && lzma::parse_header(input).is_ok_and(|(p, _)| !lclp_ok(&p)) {
        return;
    }
    let ours = match kind {
        0 => lzma::decompress_raw(input, &props, None, LIMIT).ok().map(|d| (d.data, d.consumed)),
        1 => {
            // The properties come from the stream; only the dictionary is given.
            lzma2::decompress_with_limit(input, DICT, LIMIT).ok().map(|d| (d.data, d.consumed))
        }
        2 => lzma::read_file(input, LIMIT).ok().map(|d| (d.data, d.consumed)),
        _ => xz::read_stream(input, LIMIT).ok().map(|s| (s.data, s.consumed)),
    };
    match (ours, reference(kind, props, input)) {
        (_, Theirs::Skipped) => {}
        (Some(a), Theirs::Accepted(data, consumed)) => {
            assert!(a.0 == data, "kind {kind}: different output");
            assert_eq!(a.1, consumed, "kind {kind}: different bytes consumed");
        }
        (None, Theirs::Rejected) => {}
        (a, b) => panic!("kind {kind}: ours accepts {}, liblzma accepts {}", a.is_some(), matches!(b, Theirs::Accepted(..))),
    }
});
