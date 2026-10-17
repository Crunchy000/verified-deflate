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
use crate::lz77::copy_match;
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
    let guess = input.len().saturating_mul(3).min(max_output).min(1 << 26);
    let mut st = State { br: BitReader::new(input), out: Vec::with_capacity(guess), limit: max_output };
    loop {
        let header = st.br.read_bits(3)?;
        match header >> 1 {
            0 => st.stored()?,
            1 => st.codes(&FIXED_LIT, &FIXED_DIST)?,
            2 => {
                let (lit, dist) = read_dynamic_codes(&mut st.br)?;
                st.codes(&lit, &dist)?;
            }
            _ => return Err(InflateError::InvalidBlockType),
        }
        if header & 1 == 1 {
            break;
        }
    }
    Ok(Decompressed { consumed: st.br.bytes_consumed(), data: st.out })
}

const FIXED_LIT: Decoder = match Decoder::new(&fixed_litlen_lengths()) {
    Ok(d) => d,
    Err(_) => panic!("fixed literal/length code is invalid"),
};
const FIXED_DIST: Decoder = match Decoder::new(&FIXED_DIST_LENGTHS) {
    Ok(d) => d,
    Err(_) => panic!("fixed distance code is invalid"),
};

struct State<'a> {
    br: BitReader<'a>,
    out: Vec<u8>,
    limit: usize,
}

impl State<'_> {
    /// Fails unless `n` more output bytes fit within the limit.
    /// Invariant: `out.len() <= limit`.
    #[inline]
    fn reserve(&self, n: usize) -> Result<(), InflateError> {
        if n > self.limit - self.out.len() {
            Err(InflateError::OutputLimitExceeded)
        } else {
            Ok(())
        }
    }

    fn stored(&mut self) -> Result<(), InflateError> {
        self.br.align_to_byte();
        let len = self.br.read_bits(16)?;
        let nlen = self.br.read_bits(16)?;
        if len != (!nlen & 0xffff) {
            return Err(InflateError::StoredLengthMismatch);
        }
        self.reserve(len as usize)?;
        self.br.read_aligned_bytes(len as usize, &mut self.out)
    }

    fn codes(&mut self, lit: &Decoder, dist: &Decoder) -> Result<(), InflateError> {
        loop {
            let sym = lit.decode(&mut self.br)? as usize;
            if sym < 256 {
                self.reserve(1)?;
                self.out.push(sym as u8);
            } else if sym == END_OF_BLOCK {
                return Ok(());
            } else {
                let li = sym - 257;
                if li >= LENGTH_BASE.len() {
                    return Err(InflateError::InvalidSymbol);
                }
                let len = LENGTH_BASE[li] as usize + self.br.read_bits(u32::from(LENGTH_EXTRA[li]))? as usize;
                let ds = dist.decode(&mut self.br)? as usize;
                if ds >= DIST_BASE.len() {
                    return Err(InflateError::InvalidSymbol);
                }
                let d = DIST_BASE[ds] as usize + self.br.read_bits(u32::from(DIST_EXTRA[ds]))? as usize;
                if d > self.out.len() {
                    return Err(InflateError::DistanceTooFarBack);
                }
                self.reserve(len)?;
                copy_match(&mut self.out, d, len);
            }
        }
    }
}

/// Reads a dynamic block header (§3.2.7) and builds its two decoders.
fn read_dynamic_codes(br: &mut BitReader<'_>) -> Result<(Decoder, Decoder), InflateError> {
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
    let clcode = Decoder::new(&cl).map_err(|_| InflateError::InvalidCodeLengthCode)?;
    if !clcode.is_complete() {
        return Err(InflateError::InvalidCodeLengthCode);
    }

    let total = nlen + ndist;
    let mut lengths = [0u8; NUM_LITLEN + NUM_DIST];
    let mut idx = 0;
    while idx < total {
        let sym = clcode.decode(br)? as u8;
        let extra = br.read_bits(codelen_extra_bits(sym))?;
        expand_code_length(sym, extra, &mut lengths[..total], &mut idx)?;
    }
    if lengths[END_OF_BLOCK] == 0 {
        return Err(InflateError::MissingEndOfBlock);
    }

    let lit = Decoder::new(&lengths[..nlen]).map_err(|_| InflateError::InvalidLiteralLengthCode)?;
    if !(lit.is_complete() || lit.is_single_unit_code()) {
        return Err(InflateError::InvalidLiteralLengthCode);
    }
    let dist = Decoder::new(&lengths[nlen..total]).map_err(|_| InflateError::InvalidDistanceCode)?;
    if !(dist.is_complete() || dist.is_single_unit_code() || dist.coded_symbols() == 0) {
        return Err(InflateError::InvalidDistanceCode);
    }
    Ok((lit, dist))
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

    /// No 4-byte input can make the decoder panic (this reaches every block
    /// type, dynamic headers up to the code-length code, and matches), and
    /// successful results respect the output limit and input length.
    #[kani::proof]
    #[kani::unwind(21)]
    fn never_panics_4_bytes() {
        let data: [u8; 4] = kani::any();
        let limit: usize = kani::any();
        kani::assume(limit <= 16);
        if let Ok(d) = decompress_with_limit(&data, limit) {
            assert!(d.data.len() <= limit);
            assert!(d.consumed <= data.len());
        }
    }

    /// No 6-byte input that is a single fixed-Huffman block can make the
    /// decoder panic; this reaches long overlapping matches.
    #[kani::proof]
    #[kani::unwind(17)]
    fn never_panics_fixed_6_bytes() {
        let mut data: [u8; 6] = kani::any();
        data[0] = (data[0] & !0b111) | 0b011; // BFINAL = 1, BTYPE = 01
        if let Ok(d) = decompress_with_limit(&data, 300) {
            assert!(d.data.len() <= 300);
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
        let mut s = vec![0x01, len as u8, (len >> 8) as u8, nlen as u8, (nlen >> 8) as u8];
        s.extend_from_slice(&payload[..len as usize]);
        let r = decompress_with_limit(&s, 16);
        if nlen == !len {
            let d = r.unwrap();
            assert_eq!(d.data.as_slice(), &payload[..len as usize]);
            assert_eq!(d.consumed, s.len());
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
