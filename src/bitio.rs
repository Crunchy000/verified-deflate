//! Bit-level reading and writing in DEFLATE bit order (RFC 1951 §3.1.1).
//!
//! DEFLATE packs data elements starting at the least-significant bit of each
//! byte. Multi-bit fields (extra bits, header fields) are written LSB first.
//! Huffman codes are defined MSB first, so the encoder stores them already
//! reversed (see [`reverse_bits`]) and every write goes through
//! [`BitWriter::write_bits`].

use crate::error::InflateError;

/// Accumulates bits LSB-first into a byte vector.
///
/// Invariant (checked by the Kani harnesses below): between calls fewer than
/// eight bits are pending, and no pending bit above `nbits` is set.
#[derive(Debug, Default, Clone)]
pub struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    nbits: u32,
}

impl BitWriter {
    /// Creates an empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty writer whose buffer can hold `bytes` bytes without reallocating.
    pub fn with_capacity(bytes: usize) -> Self {
        Self { out: Vec::with_capacity(bytes), acc: 0, nbits: 0 }
    }

    /// Appends the low `n` bits of `value`, least-significant bit first.
    ///
    /// `n` must be at most 32; bits of `value` at or above `n` are ignored.
    #[inline]
    pub fn write_bits(&mut self, value: u32, n: u32) {
        debug_assert!(n <= 32, "write_bits: n = {n} exceeds 32");
        let n = if n > 32 { 32 } else { n };
        let masked = u64::from(value) & ((1u64 << n) - 1);
        self.acc |= masked << self.nbits;
        self.nbits += n;
        // Fewer than 8 bits were pending and n <= 32, so 1..=4 whole bytes
        // are ready here. Flushing them without a loop keeps proofs small.
        if self.nbits >= 8 {
            let k = self.nbits / 8;
            self.out.extend_from_slice(&self.acc.to_le_bytes()[..k as usize]);
            self.acc >>= 8 * k;
            self.nbits -= 8 * k;
        }
    }

    /// Total number of bits written so far.
    pub fn bit_len(&self) -> u64 {
        (self.out.len() as u64) * 8 + u64::from(self.nbits)
    }

    /// True when the next bit would start a new byte.
    pub fn is_byte_aligned(&self) -> bool {
        self.nbits == 0
    }

    /// Fills the rest of the current byte with zero bits.
    pub fn pad_to_byte(&mut self) {
        if self.nbits > 0 {
            self.out.push(self.acc as u8);
            self.acc = 0;
            self.nbits = 0;
        }
    }

    /// Pads to a byte boundary, then appends `bytes` verbatim.
    pub fn write_aligned_bytes(&mut self, bytes: &[u8]) {
        self.pad_to_byte();
        self.out.extend_from_slice(bytes);
    }

    /// Pads to a byte boundary and returns the bytes written.
    pub fn finish(mut self) -> Vec<u8> {
        self.pad_to_byte();
        self.out
    }
}

/// Reads bits LSB-first from a byte slice.
#[derive(Debug, Clone)]
pub struct BitReader<'a> {
    data: &'a [u8],
    /// Index of the next byte not yet loaded into `acc`.
    pos: usize,
    acc: u64,
    nbits: u32,
}

impl<'a> BitReader<'a> {
    /// Creates a reader positioned at the first bit of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, acc: 0, nbits: 0 }
    }

    /// Loads the whole bytes needed to have at least `n` (≤ 32) bits
    /// buffered (fewer are). On failure nothing is consumed.
    ///
    /// Loading only what is needed keeps the bits of bytes not yet reached
    /// out of the accumulator, so header fields that are concrete in a
    /// proof stay concrete for the model checker.
    #[inline(never)]
    fn fill(&mut self, n: u32) -> Result<(), InflateError> {
        // 1..=4 bytes; the loop below has a constant bound so that Kani
        // unrolls it exactly four times.
        let need = (n - self.nbits).div_ceil(8) as usize;
        let end = self.pos + need;
        if end > self.data.len() {
            return Err(InflateError::UnexpectedEof);
        }
        let mut j = 0;
        while j < 4 {
            if j < need {
                self.acc |= u64::from(self.data[self.pos + j]) << self.nbits;
                self.nbits += 8;
            }
            j += 1;
        }
        self.pos = end;
        Ok(())
    }

    /// Tops up the accumulator for [`peek`](Self::peek): to at least 57
    /// bits with one unaligned 8-byte load when 8 or more input bytes
    /// remain, otherwise with all whole bytes left that fit. Bits above
    /// `nbits` stay zero.
    #[inline]
    fn refill(&mut self) {
        if self.nbits > 56 {
            return;
        }
        // Whole bytes that fit: 1..=8.
        let room = ((64 - self.nbits) / 8) as usize;
        if let Some(c) = self.data.get(self.pos..self.pos + 8) {
            let w = u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
            let w = if room == 8 { w } else { w & ((1u64 << (room * 8)) - 1) };
            self.acc |= w << self.nbits;
            self.pos += room;
            self.nbits += 8 * room as u32;
        } else {
            // Fewer than 8 bytes left: load them zero-padded (no loop, which
            // keeps this cheap for the model checker).
            let rest = &self.data[self.pos..];
            let k = if rest.len() < room { rest.len() } else { room };
            let mut b = [0u8; 8];
            b[..k].copy_from_slice(&rest[..k]);
            self.acc |= u64::from_le_bytes(b) << self.nbits;
            self.pos += k;
            self.nbits += 8 * k as u32;
        }
    }

    /// Reads `n` bits (at most 32) as an unsigned integer, first bit in bit 0.
    /// On failure nothing is consumed.
    #[inline(always)]
    pub fn read_bits(&mut self, n: u32) -> Result<u32, InflateError> {
        debug_assert!(n <= 32, "read_bits: n = {n} exceeds 32");
        let n = if n > 32 { 32 } else { n };
        if self.nbits < n {
            self.fill(n)?;
        }
        let v = (self.acc & ((1u64 << n) - 1)) as u32;
        self.acc >>= n;
        self.nbits -= n;
        Ok(v)
    }

    /// Reads a single bit.
    #[inline]
    pub fn read_bit(&mut self) -> Result<u32, InflateError> {
        if self.nbits == 0 {
            // The accumulator is empty, so load exactly one byte.
            let b = *self.data.get(self.pos).ok_or(InflateError::UnexpectedEof)?;
            self.acc = u64::from(b);
            self.pos += 1;
            self.nbits = 8;
        }
        let v = (self.acc & 1) as u32;
        self.acc >>= 1;
        self.nbits -= 1;
        Ok(v)
    }

    /// The next `n` (at most 32) bits without consuming them, with zeros in
    /// place of any bits past the end of the input. Check
    /// [`available`](Self::available) before relying on them.
    #[inline]
    pub fn peek(&mut self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        if self.nbits < n {
            self.refill();
        }
        (self.acc & ((1u64 << n) - 1)) as u32
    }

    /// Bits currently buffered, all of which are real input. After
    /// [`peek`](Self::peek)`(n)` this is at least `n` unless the input ends
    /// sooner.
    #[inline]
    pub fn available(&self) -> u32 {
        self.nbits
    }

    /// Discards `n` buffered bits. Requires `n <= available()`.
    #[inline]
    pub fn consume(&mut self, n: u32) {
        debug_assert!(n <= self.nbits);
        self.acc >>= n;
        self.nbits -= n;
    }

    /// Discards bits up to the next byte boundary.
    pub fn align_to_byte(&mut self) {
        let drop = self.nbits % 8;
        self.acc >>= drop;
        self.nbits -= drop;
    }

    /// Fills `out` with the next `out.len()` whole bytes. The reader must
    /// be byte-aligned (call [`align_to_byte`](Self::align_to_byte) first).
    /// On failure the reader's position is unspecified.
    pub fn read_aligned_bytes(&mut self, out: &mut [u8]) -> Result<(), InflateError> {
        debug_assert!(self.nbits % 8 == 0, "read_aligned_bytes on unaligned reader");
        self.align_to_byte();
        let len = out.len();
        // Whole bytes already sitting in the accumulator (at most 8) come first.
        let buffered = (self.nbits / 8) as usize;
        let k = if len < buffered { len } else { buffered };
        let acc = self.acc.to_le_bytes();
        out[..k].copy_from_slice(&acc[..k]);
        if k == 8 {
            self.acc = 0;
        } else {
            self.acc >>= 8 * k as u32;
        }
        self.nbits -= 8 * k as u32;
        let remaining = len - k;
        let end = match self.pos.checked_add(remaining) {
            Some(end) if end <= self.data.len() => end,
            _ => return Err(InflateError::UnexpectedEof),
        };
        out[k..].copy_from_slice(&self.data[self.pos..end]);
        self.pos = end;
        Ok(())
    }

    /// Number of input bytes consumed, counting a partially read byte as consumed.
    pub fn bytes_consumed(&self) -> usize {
        self.pos - (self.nbits / 8) as usize
    }

    /// Number of bits not yet read.
    pub fn bits_remaining(&self) -> u64 {
        ((self.data.len() - self.pos) as u64) * 8 + u64::from(self.nbits)
    }
}

/// Reverses the low `len` bits of `code` (`len` ≤ 16); higher bits must be zero.
///
/// Huffman codes are specified MSB-first but packed LSB-first, so the
/// encoder stores each code reversed and writes it with
/// [`BitWriter::write_bits`].
#[inline]
pub const fn reverse_bits(code: u16, len: u32) -> u16 {
    if len == 0 {
        0
    } else if len >= 16 {
        code.reverse_bits()
    } else {
        code.reverse_bits() >> (16 - len)
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    fn mask(n: u32) -> u32 {
        if n >= 32 {
            u32::MAX
        } else {
            (1u32 << n) - 1
        }
    }

    /// Three writes of arbitrary widths read back exactly, the bit count is
    /// exact, and the output is the minimum number of bytes.
    #[kani::proof]
    #[kani::unwind(6)]
    fn write_then_read_roundtrip() {
        let vals: [u32; 3] = kani::any();
        let lens: [u32; 3] = kani::any();
        kani::assume(lens[0] <= 32 && lens[1] <= 32 && lens[2] <= 32);
        let mut w = BitWriter::new();
        w.write_bits(vals[0], lens[0]);
        w.write_bits(vals[1], lens[1]);
        w.write_bits(vals[2], lens[2]);
        let total = u64::from(lens[0] + lens[1] + lens[2]);
        assert_eq!(w.bit_len(), total);
        let bytes = w.finish();
        assert_eq!(bytes.len() as u64, total.div_ceil(8));
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read_bits(lens[0]), Ok(vals[0] & mask(lens[0])));
        assert_eq!(r.read_bits(lens[1]), Ok(vals[1] & mask(lens[1])));
        assert_eq!(r.read_bits(lens[2]), Ok(vals[2] & mask(lens[2])));
        // Padding bits are zero.
        let pad = (8 - (total % 8) as u32) % 8;
        assert_eq!(r.read_bits(pad), Ok(0));
        assert_eq!(r.bits_remaining(), 0);
    }

    /// Reads never panic, succeed exactly while enough bits remain, and
    /// report `UnexpectedEof` otherwise.
    #[kani::proof]
    #[kani::unwind(6)]
    fn reader_eof_is_exact() {
        let data: [u8; 4] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= 4);
        let mut r = BitReader::new(&data[..len]);
        let mut consumed = 0u64;
        for _ in 0..3 {
            let n: u32 = kani::any();
            kani::assume(n <= 32);
            let before = r.bits_remaining();
            assert_eq!(before, 8 * len as u64 - consumed);
            match r.read_bits(n) {
                Ok(v) => {
                    assert!(u64::from(n) <= before);
                    assert_eq!(v & !mask(n), 0);
                    consumed += u64::from(n);
                }
                Err(e) => {
                    assert_eq!(e, InflateError::UnexpectedEof);
                    assert!(u64::from(n) > before);
                    return;
                }
            }
        }
    }

    /// `read_bits(n)` returns the same bits as `n` calls to `read_bit`.
    #[kani::proof]
    #[kani::unwind(17)]
    fn read_bits_matches_read_bit() {
        let data: [u8; 3] = kani::any();
        let skip: u32 = kani::any();
        let n: u32 = kani::any();
        kani::assume(skip <= 7 && n <= 16);
        let mut a = BitReader::new(&data);
        let mut b = BitReader::new(&data);
        let _ = a.read_bits(skip);
        let _ = b.read_bits(skip);
        let whole = a.read_bits(n).unwrap();
        let mut assembled = 0u32;
        let mut i = 0;
        while i < n {
            assembled |= b.read_bit().unwrap() << i;
            i += 1;
        }
        assert_eq!(whole, assembled);
        assert_eq!(a.bits_remaining(), b.bits_remaining());
    }

    /// Stored-block framing: bits, pad, aligned bytes, then more bits all
    /// survive a write/read round trip, and `bytes_consumed` is exact.
    #[kani::proof]
    #[kani::unwind(6)]
    fn aligned_bytes_roundtrip() {
        let head: u32 = kani::any();
        let head_len: u32 = kani::any();
        kani::assume(head_len <= 20);
        let payload: [u8; 3] = kani::any();
        let tail: u32 = kani::any();
        let mut w = BitWriter::new();
        w.write_bits(head, head_len);
        w.write_aligned_bytes(&payload);
        w.write_bits(tail, 5);
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read_bits(head_len), Ok(head & mask(head_len)));
        r.align_to_byte();
        let mut out = [0u8; 3];
        assert!(r.read_aligned_bytes(&mut out).is_ok());
        assert_eq!(out, payload);
        assert_eq!(r.read_bits(5), Ok(tail & 31));
        assert_eq!(r.bytes_consumed(), bytes.len());
        // Asking for more bytes than exist is an error, not a panic.
        let mut r2 = BitReader::new(&bytes);
        let mut big = [0u8; 16];
        assert_eq!(r2.read_aligned_bytes(&mut big[..bytes.len() + 1]), Err(InflateError::UnexpectedEof));
    }

    /// `reverse_bits` reverses exactly the low `len` bits and is an involution.
    #[kani::proof]
    #[kani::unwind(17)]
    fn reverse_bits_is_reversal() {
        let len: u32 = kani::any();
        kani::assume(len >= 1 && len <= 15);
        let code: u16 = kani::any();
        kani::assume(u32::from(code) < (1u32 << len));
        let r = reverse_bits(code, len);
        assert!(u32::from(r) < (1u32 << len));
        let mut i = 0;
        while i < len {
            assert_eq!((r >> i) & 1, (code >> (len - 1 - i)) & 1);
            i += 1;
        }
        assert_eq!(reverse_bits(r, len), code);
    }
}
