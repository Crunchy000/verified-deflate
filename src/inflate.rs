//! DEFLATE decompression (RFC 1951 §3.2), structured like Mark Adler's
//! reference decoder `puff.c`, and Deflate64 decompression (ZIP method 9),
//! which differs only in its length and distance tables (see
//! [`LENGTH_BASE_64`]).
//!
//! The decoder is strict in the same places zlib is: over-subscribed codes
//! are always rejected, and incomplete codes are only accepted for a single
//! one-bit code (or an empty distance code). Every malformed input yields an
//! [`InflateError`]. Verus proves (`verus/src/inflate.rs`) that for every
//! input of every length the decoder returns, never panics, and keeps to
//! the output limit; Kani checks its parts in more detail on small inputs.

use crate::bitio::BitReader;
use crate::error::InflateError;
use crate::huffman::Decoder;
use crate::lz77::copy_within_buffer;
use crate::tables::{
    codelen_extra_bits, fixed_litlen_lengths, CODELEN_ORDER, DIST_BASE_64, DIST_EXTRA_64, END_OF_BLOCK,
    FIXED_DIST_LENGTHS, LENGTH_BASE, LENGTH_BASE_64, LENGTH_EXTRA, LENGTH_EXTRA_64, NUM_CODELEN, NUM_DIST, NUM_DIST_64,
    NUM_LITLEN,
};

/// Result of [`decompress_with_limit`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decompressed {
    /// The decompressed bytes.
    pub data: Vec<u8>,
    /// Bytes of input used, up to and including the byte holding the last
    /// bit of the final block. Anything after that is not part of the stream.
    pub consumed: usize,
}

/// Decompresses a raw DEFLATE stream with no output limit.
///
/// For untrusted input prefer [`decompress_with_limit`]: a few kilobytes of
/// DEFLATE can expand to gigabytes.
pub fn decompress(input: &[u8]) -> Result<Vec<u8>, InflateError> {
    decompress_with_limit(input, usize::MAX).map(|d| d.data)
}

/// Decompresses a raw DEFLATE stream, failing with
/// [`InflateError::OutputLimitExceeded`] rather than producing more than
/// `max_output` bytes.
pub fn decompress_with_limit(input: &[u8], max_output: usize) -> Result<Decompressed, InflateError> {
    inflate::<false>(input, max_output)
}

/// Decompresses a raw Deflate64 stream (ZIP compression method 9) with no
/// output limit.
///
/// For untrusted input prefer [`decompress_deflate64_with_limit`].
pub fn decompress_deflate64(input: &[u8]) -> Result<Vec<u8>, InflateError> {
    decompress_deflate64_with_limit(input, usize::MAX).map(|d| d.data)
}

/// Decompresses a raw Deflate64 stream (ZIP compression method 9), failing
/// with [`InflateError::OutputLimitExceeded`] rather than producing more
/// than `max_output` bytes.
///
/// Deflate64 is DEFLATE with a 64 KiB window: length code 285 takes 16
/// extra bits (lengths up to 65538) and distance codes 30 and 31 reach back
/// 65536 bytes. Windows and 7-Zip use it for large ZIP entries.
pub fn decompress_deflate64_with_limit(input: &[u8], max_output: usize) -> Result<Decompressed, InflateError> {
    inflate::<true>(input, max_output)
}

/// Decompresses a DEFLATE stream, or a Deflate64 stream if `DEFLATE64`.
/// (A constant parameter, so each format's decoder is compiled with its own
/// tables, as fast as a decoder for that format alone.)
fn inflate<const DEFLATE64: bool>(input: &[u8], max_output: usize) -> Result<Decompressed, InflateError> {
    let mut st = State {
        br: BitReader::new(input),
        out: Output::new(input.len(), max_output),
        lit: Decoder::EMPTY,
        dist: Decoder::EMPTY,
        lengths: [0; NUM_LITLEN + NUM_DIST_64],
    };
    loop {
        let header = st.br.read_bits(3)?;
        match header >> 1 {
            0 => st.stored()?,
            1 => codes::<DEFLATE64>(&mut st.br, &mut st.out, &FIXED_LIT, &FIXED_DIST)?,
            2 => {
                read_dynamic_codes::<DEFLATE64>(&mut st.br, &mut st.lit, &mut st.dist, &mut st.lengths)?;
                codes::<DEFLATE64>(&mut st.br, &mut st.out, &st.lit, &st.dist)?;
            }
            _ => return Err(InflateError::InvalidBlockType),
        }
        if header & 1 == 1 {
            break;
        }
    }
    Ok(Decompressed { consumed: st.br.bytes_consumed(), data: st.out.finish() })
}

/// Initial output buffer size bounds.
const INITIAL_CAPACITY: usize = 4096;
const MAX_INITIAL_CAPACITY: usize = 1 << 26;

const FIXED_LIT: Decoder = match Decoder::new(&fixed_litlen_lengths()) {
    Ok(d) => d,
    Err(_) => Decoder::EMPTY,
};
const FIXED_DIST: Decoder = match Decoder::new(&FIXED_DIST_LENGTHS) {
    Ok(d) => d,
    Err(_) => Decoder::EMPTY,
};
// The fixed codes are valid, so the fallbacks above are never taken:
// checked when the crate is compiled.
const _: () = assert!(Decoder::new(&fixed_litlen_lengths()).is_ok() && Decoder::new(&FIXED_DIST_LENGTHS).is_ok());

/// The decoder's output: a zero-filled buffer written by index and grown
/// by doubling when needed. Writing by index (rather than `Vec::push`)
/// keeps the model checker's view of the output small.
pub(crate) struct Output {
    buf: Vec<u8>,
    len: usize,
    limit: usize,
}

impl Output {
    /// An empty output for a stream of `input_len` bytes, with room for as
    /// many bytes as the input has (more is added as needed, by doubling),
    /// but never more than `limit`, so a caller that knows the exact size,
    /// as a ZIP reader does, gets an exact buffer.
    fn new(input_len: usize, limit: usize) -> Self {
        let cap = input_len.clamp(INITIAL_CAPACITY, MAX_INITIAL_CAPACITY);
        let cap = if cap < limit { cap } else { limit };
        Self { buf: vec![0; cap], len: 0, limit }
    }

    /// Makes room for `n` more bytes, failing if that would exceed the
    /// limit. Invariants: `len <= limit` and `len <= buf.len()`.
    #[inline]
    fn reserve(&mut self, n: usize) -> Result<(), InflateError> {
        if n > self.limit - self.len {
            return Err(InflateError::OutputLimitExceeded);
        }
        if n > self.buf.len() - self.len {
            self.grow(n);
        }
        Ok(())
    }

    #[cold]
    pub(crate) fn grow(&mut self, n: usize) {
        // At least double, but never past the limit (`reserve` has checked
        // that `len + n` is within it). Resizing in place lets the
        // allocator extend the block, so only the new part is zeroed.
        let want = (self.len + n).max(self.buf.len().saturating_mul(2)).min(self.limit);
        self.buf.resize(want, 0);
    }

    fn finish(mut self) -> Vec<u8> {
        self.buf.truncate(self.len);
        self.buf
    }
}

struct State<'a> {
    br: BitReader<'a>,
    out: Output,
    /// Decoders for the current dynamic block, rebuilt in place.
    lit: Decoder,
    dist: Decoder,
    /// Scratch space for a dynamic block's code lengths.
    lengths: [u8; NUM_LITLEN + NUM_DIST_64],
}

impl State<'_> {
    fn stored(&mut self) -> Result<(), InflateError> {
        self.br.align_to_byte();
        let len = self.br.read_bits(16)?;
        let nlen = self.br.read_bits(16)?;
        if len != (!nlen & 0xffff) {
            return Err(InflateError::StoredLengthMismatch);
        }
        let len = len as usize;
        self.out.reserve(len)?;
        let start = self.out.len;
        self.br.read_aligned_bytes(&mut self.out.buf.as_mut_slice()[start..start + len])?;
        self.out.len += len;
        Ok(())
    }
}

/// Decodes one block's symbols with the given codes, as Deflate64 if
/// `DEFLATE64`.
fn codes<const DEFLATE64: bool>(
    br: &mut BitReader<'_>,
    out: &mut Output,
    lit: &Decoder,
    dist: &Decoder,
) -> Result<(), InflateError> {
    // Deflate64 decodes length code 285 and distance codes 30 and 31
    // differently; its distance tables extend DEFLATE's.
    let (length_base, length_extra, num_dist) = if DEFLATE64 {
        (&LENGTH_BASE_64, &LENGTH_EXTRA_64, NUM_DIST_64)
    } else {
        (&LENGTH_BASE, &LENGTH_EXTRA, NUM_DIST)
    };
    loop {
        let sym = lit.decode(br)? as usize;
        if sym < 256 {
            out.reserve(1)?;
            out.buf[out.len] = sym as u8;
            out.len += 1;
        } else if sym == END_OF_BLOCK {
            return Ok(());
        } else {
            let li = sym - 257;
            if li >= length_base.len() {
                return Err(InflateError::InvalidSymbol);
            }
            let len = length_base[li] as usize + br.read_bits(u32::from(length_extra[li]))? as usize;
            let ds = dist.decode(br)? as usize;
            if ds >= num_dist {
                return Err(InflateError::InvalidSymbol);
            }
            let d = DIST_BASE_64[ds] as usize + br.read_bits(u32::from(DIST_EXTRA_64[ds]))? as usize;
            if d > out.len {
                return Err(InflateError::DistanceTooFarBack);
            }
            out.reserve(len)?;
            copy_within_buffer(&mut out.buf, out.len, d, len);
            out.len += len;
        }
    }
}

/// Reads a dynamic block header (§3.2.7) and builds its two decoders.
/// Deflate64 allows all 32 distance codes.
#[allow(clippy::needless_range_loop)] // an index loop, as the Verus copy needs
fn read_dynamic_codes<const DEFLATE64: bool>(
    br: &mut BitReader<'_>,
    lit: &mut Decoder,
    dist: &mut Decoder,
    lengths: &mut [u8; NUM_LITLEN + NUM_DIST_64],
) -> Result<(), InflateError> {
    let nlen = br.read_bits(5)? as usize + 257;
    let ndist = br.read_bits(5)? as usize + 1;
    let ncode = br.read_bits(4)? as usize + 4;
    if nlen > NUM_LITLEN || ndist > if DEFLATE64 { NUM_DIST_64 } else { NUM_DIST } {
        return Err(InflateError::TooManyCodes);
    }
    let mut cl = [0u8; NUM_CODELEN];
    for i in 0..ncode {
        cl[CODELEN_ORDER[i]] = br.read_bits(3)? as u8;
    }
    let mut clcode = Decoder::EMPTY;
    clcode.rebuild(&cl).map_err(|_| InflateError::InvalidCodeLengthCode)?;
    if !clcode.is_complete() {
        return Err(InflateError::InvalidCodeLengthCode);
    }

    // Every entry of lengths[..total] is written below before it is read.
    let total = nlen + ndist;
    let mut idx = 0;
    while idx < total {
        let sym = clcode.decode(br)? as u8;
        let extra = br.read_bits(codelen_extra_bits(sym))?;
        expand_code_length(sym, extra, &mut lengths[..total], &mut idx)?;
    }
    if lengths[END_OF_BLOCK] == 0 {
        return Err(InflateError::MissingEndOfBlock);
    }

    lit.rebuild(&lengths[..nlen]).map_err(|_| InflateError::InvalidLiteralLengthCode)?;
    if !(lit.is_complete() || lit.is_single_unit_code()) {
        return Err(InflateError::InvalidLiteralLengthCode);
    }
    dist.rebuild(&lengths[nlen..total]).map_err(|_| InflateError::InvalidDistanceCode)?;
    if !(dist.is_complete() || dist.is_single_unit_code() || dist.coded_symbols() == 0) {
        return Err(InflateError::InvalidDistanceCode);
    }
    Ok(())
}

/// Applies one code-length symbol (with its extra-bits value) to
/// `lengths`, advancing `idx`. Shared by the decoder and the encoder's
/// proofs, so the encoder's run-length coding is checked against exactly
/// the logic that will decode it. Requires `*idx <= lengths.len()`.
pub(crate) fn expand_code_length(sym: u8, extra: u32, lengths: &mut [u8], idx: &mut usize) -> Result<(), InflateError> {
    let (value, repeat) = match sym {
        0..=15 => (sym, 1),
        16 => {
            if *idx == 0 {
                return Err(InflateError::RepeatWithoutPrevious);
            }
            (lengths[*idx - 1], 3 + extra as usize)
        }
        17 => (0, 3 + extra as usize),
        18 => (0, 11 + extra as usize),
        _ => return Err(InflateError::InvalidSymbol),
    };
    if repeat > lengths.len() - *idx {
        return Err(InflateError::CodeLengthOverflow);
    }
    lengths[*idx..*idx + repeat].fill(value);
    *idx += repeat;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_block_hello() {
        // "hello" compressed by zlib (raw deflate, level 6).
        let c = [0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x07, 0x00];
        let d = decompress_with_limit(&c, 100).unwrap();
        assert_eq!(d.data, b"hello");
        assert_eq!(d.consumed, c.len());
    }

    #[test]
    fn rejects_reserved_block_type() {
        assert_eq!(decompress(&[0x07]), Err(InflateError::InvalidBlockType));
    }

    #[test]
    fn stored_length_check() {
        assert_eq!(decompress(&[0x01, 0x01, 0x00, 0x00, 0x00]), Err(InflateError::StoredLengthMismatch));
        assert_eq!(decompress(&[0x01, 0x01, 0x00, 0xfe, 0xff, b'x']), Ok(b"x".to_vec()));
    }

    #[test]
    fn limit_is_enforced() {
        let c = crate::deflate::compress(&[0u8; 10_000], 9);
        assert_eq!(decompress_with_limit(&c, 9_999), Err(InflateError::OutputLimitExceeded));
        assert_eq!(decompress_with_limit(&c, 10_000).unwrap().data.len(), 10_000);
    }

    /// Writes a block header (`btype` 0 stored, 1 fixed).
    fn header(w: &mut crate::bitio::BitWriter, last: bool, btype: u32) {
        w.write_bits(u32::from(last) | (btype << 1), 3);
    }

    /// A stored block of `data` (at most 65535 bytes), starting at a byte
    /// boundary.
    fn stored_block(w: &mut crate::bitio::BitWriter, data: &[u8]) {
        header(w, false, 0);
        w.write_bits(0, 5);
        w.write_bits(data.len() as u32, 16);
        w.write_bits(!(data.len() as u32), 16);
        for &b in data {
            w.write_bits(u32::from(b), 8);
        }
    }

    /// A fixed-Huffman match: length symbol `ls` with `le` (`lb` bits), then
    /// distance symbol `ds` with `de` (`db` bits).
    fn fixed_match(w: &mut crate::bitio::BitWriter, ls: usize, le: u32, lb: u32, ds: usize, de: u32, db: u32) {
        crate::deflate::FIXED_LIT.put_with_extra(w, ls, le, lb);
        crate::deflate::FIXED_DIST.put_with_extra(w, ds, de, db);
    }

    #[test]
    fn deflate64_length_285_takes_16_extra_bits() {
        // 'a', then length code 285 with extra 999 (length 1002) at
        // distance 1, then end of block.
        let mut w = crate::bitio::BitWriter::new();
        header(&mut w, true, 1);
        crate::deflate::FIXED_LIT.put(&mut w, usize::from(b'a'));
        fixed_match(&mut w, 285, 999, 16, 0, 0, 0);
        crate::deflate::FIXED_LIT.put(&mut w, END_OF_BLOCK);
        let s = w.finish();
        let d = decompress_deflate64_with_limit(&s, 2000).unwrap();
        assert_eq!(d.data, vec![b'a'; 1003]);
        assert_eq!(d.consumed, s.len());
        // As DEFLATE, code 285 is length 258 with no extra bits, so the
        // same bits mean something else.
        assert_ne!(decompress(&s), Ok(vec![b'a'; 1003]));
        // The longest Deflate64 match: 3 + 65535.
        let mut w = crate::bitio::BitWriter::new();
        header(&mut w, true, 1);
        crate::deflate::FIXED_LIT.put(&mut w, usize::from(b'z'));
        fixed_match(&mut w, 285, 0xffff, 16, 0, 0, 0);
        crate::deflate::FIXED_LIT.put(&mut w, END_OF_BLOCK);
        assert_eq!(decompress_deflate64(&w.finish()), Ok(vec![b'z'; 65539]));
    }

    #[test]
    fn deflate64_distance_codes_30_and_31() {
        // 65536 bytes of history in two stored blocks, then matches at
        // distance 40000 (code 30) and 65536 (code 31, the farthest).
        let history: Vec<u8> = (0..65536u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
        let stream = |dists: &[(usize, u32, u32)]| {
            let mut w = crate::bitio::BitWriter::new();
            stored_block(&mut w, &history[..65535]);
            stored_block(&mut w, &history[65535..]);
            header(&mut w, true, 1);
            for &(ds, de, db) in dists {
                fixed_match(&mut w, 257, 0, 0, ds, de, db); // length 3
            }
            crate::deflate::FIXED_LIT.put(&mut w, END_OF_BLOCK);
            w.finish()
        };
        let s = stream(&[(30, 40000 - 32769, 14), (31, 65536 - 49153, 14)]);
        let out = decompress_deflate64(&s).unwrap();
        let mut expected = history.clone();
        for d in [40000, 65536] {
            let from = expected.len() - d;
            expected.extend_from_within(from..from + 3);
        }
        assert_eq!(out, expected);
        // DEFLATE has no distance codes 30 and 31.
        assert_eq!(decompress(&s), Err(InflateError::InvalidSymbol));
        // A distance past everything written so far is refused: after
        // 65535 bytes, distance 65536 is one too far.
        let mut w = crate::bitio::BitWriter::new();
        stored_block(&mut w, &history[..65535]);
        header(&mut w, true, 1);
        fixed_match(&mut w, 257, 0, 0, 31, 65536 - 49153, 14);
        crate::deflate::FIXED_LIT.put(&mut w, END_OF_BLOCK);
        assert_eq!(decompress_deflate64(&w.finish()), Err(InflateError::DistanceTooFarBack));
    }

    #[test]
    fn deflate64_reads_deflate_streams_without_long_codes() {
        // A DEFLATE stream that uses neither length 258 nor distances past
        // 32768 means the same in Deflate64.
        let data = b"Deflate64 extends DEFLATE; Deflate64 extends DEFLATE's window.".repeat(3);
        let c = crate::deflate::compress(&data, 6);
        assert_eq!(decompress_deflate64(&c), Ok(data));
    }

    #[test]
    fn trailing_data_is_not_consumed() {
        let mut c = crate::deflate::compress(b"abcabcabc", 6);
        let n = c.len();
        c.extend_from_slice(b"TRAILER");
        let d = decompress_with_limit(&c, 100).unwrap();
        assert_eq!(d.consumed, n);
        assert_eq!(d.data, b"abcabcabc");
    }
}

/// Proof stub for `Output::grow`: harnesses whose outputs fit in the
/// initial buffer replace growth with a panic, so Kani must prove growth is
/// unreachable instead of modelling it. (If it were reachable, the harness
/// would fail, so this keeps the proof sound.)
#[cfg_attr(mutants, mutants::skip)] // proof code: the checker, not code under test
#[cfg(kani)]
#[cfg_attr(kani_sample, allow(dead_code))] // stubs are not applied when sampling
pub(crate) fn grow_is_unreachable(_out: &mut Output, _n: usize) {
    panic!("output growth is unreachable in this harness");
}

#[cfg_attr(mutants, mutants::skip)] // proof code: the checker, not code under test
#[cfg(kani)]
mod proofs {
    use super::*;

    /// No 2-byte input can make the decoder panic, with any output limit
    /// 0-16; successful results respect the limit and the input length.
    /// Two bytes reach every block type's header handling, a fixed-block
    /// literal and end-of-block, and every early-EOF path.
    #[kani::stub(Output::grow, grow_is_unreachable)]
    #[kani::stub(crate::lz77::copy_within_buffer, crate::lz77::copy_within_buffer_ref)]
    #[cfg(any(feature = "kani-extended", kani_sample))] // too large for CI runners
    #[kani::proof]
    #[kani::unwind(5)]
    fn never_panics_2_bytes() {
        let data: [u8; 2] = kani::any();
        let limit: usize = kani::any();
        kani::assume(limit <= 16);
        if let Ok(d) = decompress_with_limit(&data, limit) {
            assert!(d.data.len() <= limit);
            assert!(d.consumed <= data.len());
        }
    }

    /// Reading a dynamic block header from any 4 bytes never panics: this
    /// covers the header fields, the code-length code (built from as many
    /// 3-bit lengths as fit) and the start of the run-length-coded lengths.
    #[cfg(any(feature = "kani-extended", kani_sample))] // too large for CI runners
    #[kani::proof]
    #[kani::unwind(20)]
    fn dynamic_header_never_panics() {
        let data: [u8; 4] = kani::any();
        let mut br = BitReader::new(&data);
        let mut lit = Decoder::EMPTY;
        let mut dist = Decoder::EMPTY;
        let mut lengths = [0u8; NUM_LITLEN + NUM_DIST_64];
        let _ = read_dynamic_codes::<false>(&mut br, &mut lit, &mut dist, &mut lengths);
    }

    /// No 5-byte input that is a single fixed-Huffman block can make the
    /// decoder panic, with an 8-byte output limit. Five bytes reach a
    /// literal followed by a length-258 match code; the limit keeps copies
    /// short while every decoding path is still taken.
    #[kani::stub(Output::grow, grow_is_unreachable)]
    #[kani::stub(crate::lz77::copy_within_buffer, crate::lz77::copy_within_buffer_ref)]
    #[cfg(any(feature = "kani-extended", kani_sample))] // too large for CI runners
    #[kani::proof]
    #[kani::unwind(9)]
    fn never_panics_fixed_5_bytes() {
        let mut data: [u8; 5] = kani::any();
        data[0] = (data[0] & !0b111) | 0b011; // BFINAL = 1, BTYPE = 01
        if let Ok(d) = decompress_with_limit(&data, 8) {
            assert!(d.data.len() <= 8);
        }
    }

    /// A 4-byte stored block decodes to exactly its payload when NLEN is
    /// the complement of LEN, and any other NLEN is rejected.
    #[kani::proof]
    #[kani::stub(Output::grow, grow_is_unreachable)]
    #[kani::unwind(6)]
    fn stored_block_semantics() {
        let payload: [u8; 4] = kani::any();
        let nlen: u16 = kani::any();
        let nl = nlen.to_le_bytes();
        let s = [0x01, 4, 0, nl[0], nl[1], payload[0], payload[1], payload[2], payload[3]];
        let r = decompress_with_limit(&s, 16);
        if nlen == !4u16 {
            let d = r.unwrap();
            assert_eq!(d.data.len(), 4);
            for i in 0..4 {
                assert_eq!(d.data[i], payload[i]);
            }
            assert_eq!(d.consumed, 9);
        } else {
            assert_eq!(r, Err(InflateError::StoredLengthMismatch));
        }
    }

    /// Code-length expansion never panics or writes out of bounds, and on
    /// success advances by exactly the repeat count.
    #[kani::proof]
    #[kani::unwind(12)]
    fn expand_code_length_is_safe() {
        let mut lengths: [u8; 10] = kani::any();
        let sym: u8 = kani::any();
        let extra: u32 = kani::any();
        let mut idx: usize = kani::any();
        kani::assume(idx <= 10);
        let before = idx;
        if expand_code_length(sym, extra, &mut lengths, &mut idx).is_ok() {
            assert!(sym <= 18);
            assert!(idx > before && idx <= 10);
            if sym <= 15 {
                assert_eq!(idx, before + 1);
                assert_eq!(lengths[before], sym);
            }
        } else {
            assert_eq!(idx, before);
        }
    }
}
