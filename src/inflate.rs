//! DEFLATE decompression (RFC 1951 §3.2), structured like Mark Adler's
//! reference decoder `puff.c`.
//!
//! The decoder is strict in the same places zlib is: over-subscribed codes
//! are always rejected, and incomplete codes are only accepted for a single
//! one-bit code (or an empty distance code). Every malformed input yields an
//! [`InflateError`]; the Kani harnesses check that no input of the
//! verified sizes can make it panic, and that the output limit holds.

use crate::bitio::BitReader;
use crate::error::InflateError;
use crate::huffman::Decoder;
use crate::lz77::copy_within_buffer;
use crate::tables::{
    codelen_extra_bits, fixed_litlen_lengths, CODELEN_ORDER, DIST_BASE, DIST_EXTRA, END_OF_BLOCK, FIXED_DIST_LENGTHS,
    LENGTH_BASE, LENGTH_EXTRA, NUM_CODELEN, NUM_DIST, NUM_LITLEN,
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
    let mut st = State {
        br: BitReader::new(input),
        out: Output::new(max_output),
        lit: Decoder::EMPTY,
        dist: Decoder::EMPTY,
        lengths: [0; NUM_LITLEN + NUM_DIST],
    };
    loop {
        let header = st.br.read_bits(3)?;
        match header >> 1 {
            0 => st.stored()?,
            1 => codes(&mut st.br, &mut st.out, &FIXED_LIT, &FIXED_DIST)?,
            2 => {
                read_dynamic_codes(&mut st.br, &mut st.lit, &mut st.dist, &mut st.lengths)?;
                let State { br, out, lit, dist, .. } = &mut st;
                codes(br, out, lit, dist)?;
            }
            _ => return Err(InflateError::InvalidBlockType),
        }
        if header & 1 == 1 {
            break;
        }
    }
    Ok(Decompressed { consumed: st.br.bytes_consumed(), data: st.out.finish() })
}

/// Initial output buffer size.
const INITIAL_CAPACITY: usize = 4096;

const FIXED_LIT: Decoder = match Decoder::new(&fixed_litlen_lengths()) {
    Ok(d) => d,
    Err(_) => panic!("fixed literal/length code is invalid"),
};
const FIXED_DIST: Decoder = match Decoder::new(&FIXED_DIST_LENGTHS) {
    Ok(d) => d,
    Err(_) => panic!("fixed distance code is invalid"),
};

/// The decoder's output: a zero-filled buffer written by index and grown
/// by doubling when needed. Writing by index (rather than `Vec::push`)
/// keeps the model checker's view of the output small.
struct Output {
    buf: Vec<u8>,
    len: usize,
    limit: usize,
}

impl Output {
    fn new(limit: usize) -> Self {
        Self { buf: vec![0; INITIAL_CAPACITY], len: 0, limit }
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
    fn grow(&mut self, n: usize) {
        let want = (self.len + n).max(self.buf.len().saturating_mul(2));
        let mut bigger = vec![0; want];
        bigger[..self.len].copy_from_slice(&self.buf[..self.len]);
        self.buf = bigger;
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
    lengths: [u8; NUM_LITLEN + NUM_DIST],
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
        self.br.read_aligned_bytes(&mut self.out.buf[start..start + len])?;
        self.out.len += len;
        Ok(())
    }
}

/// Decodes one block's symbols with the given codes.
fn codes(br: &mut BitReader<'_>, out: &mut Output, lit: &Decoder, dist: &Decoder) -> Result<(), InflateError> {
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
            if li >= LENGTH_BASE.len() {
                return Err(InflateError::InvalidSymbol);
            }
            let len = LENGTH_BASE[li] as usize + br.read_bits(u32::from(LENGTH_EXTRA[li]))? as usize;
            let ds = dist.decode(br)? as usize;
            if ds >= DIST_BASE.len() {
                return Err(InflateError::InvalidSymbol);
            }
            let d = DIST_BASE[ds] as usize + br.read_bits(u32::from(DIST_EXTRA[ds]))? as usize;
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
fn read_dynamic_codes(
    br: &mut BitReader<'_>,
    lit: &mut Decoder,
    dist: &mut Decoder,
    lengths: &mut [u8; NUM_LITLEN + NUM_DIST],
) -> Result<(), InflateError> {
    let nlen = br.read_bits(5)? as usize + 257;
    let ndist = br.read_bits(5)? as usize + 1;
    let ncode = br.read_bits(4)? as usize + 4;
    if nlen > NUM_LITLEN || ndist > NUM_DIST {
        return Err(InflateError::TooManyCodes);
    }
    let mut cl = [0u8; NUM_CODELEN];
    for &sym in &CODELEN_ORDER[..ncode] {
        cl[sym] = br.read_bits(3)? as u8;
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

#[cfg(kani)]
mod proofs {
    use super::*;

    /// No 2-byte input can make the decoder panic, with any output limit
    /// 0-16; successful results respect the limit and the input length.
    /// Two bytes reach every block type's header handling, a fixed-block
    /// literal and end-of-block, and every early-EOF path.
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
    #[kani::proof]
    #[kani::unwind(20)]
    fn dynamic_header_never_panics() {
        let data: [u8; 4] = kani::any();
        let mut br = BitReader::new(&data);
        let mut lit = Decoder::EMPTY;
        let mut dist = Decoder::EMPTY;
        let mut lengths = [0u8; NUM_LITLEN + NUM_DIST];
        let _ = read_dynamic_codes(&mut br, &mut lit, &mut dist, &mut lengths);
    }

    /// No 5-byte input that is a single fixed-Huffman block can make the
    /// decoder panic, with an 8-byte output limit. Five bytes reach a
    /// literal followed by a length-258 match code; the limit keeps copies
    /// short while every decoding path is still taken.
    #[kani::proof]
    #[kani::unwind(9)]
    fn never_panics_fixed_5_bytes() {
        let mut data: [u8; 5] = kani::any();
        data[0] = (data[0] & !0b111) | 0b011; // BFINAL = 1, BTYPE = 01
        if let Ok(d) = decompress_with_limit(&data, 8) {
            assert!(d.data.len() <= 8);
        }
    }

    /// A stored block decodes to exactly its payload, and any LEN/NLEN
    /// mismatch is rejected.
    #[kani::proof]
    #[kani::unwind(8)]
    fn stored_block_semantics() {
        let payload: [u8; 4] = kani::any();
        let len: u16 = kani::any();
        let nlen: u16 = kani::any();
        kani::assume(len <= 4);
        let l = len.to_le_bytes();
        let nl = nlen.to_le_bytes();
        let s = [0x01, l[0], l[1], nl[0], nl[1], payload[0], payload[1], payload[2], payload[3]];
        let n = 5 + len as usize;
        let r = decompress_with_limit(&s[..n], 16);
        if nlen == !len {
            let d = r.unwrap();
            assert_eq!(d.data.len(), len as usize);
            for i in 0..len as usize {
                assert_eq!(d.data[i], payload[i]);
            }
            assert_eq!(d.consumed, n);
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
