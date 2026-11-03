//! The bit reader (`src/bitio.rs`): `BitReader` and its helpers.
//!
//! The executable code below is the library's, token for token (CI checks
//! this with `.github/scripts/verus_drift.py`); every line marked
//! `// @ghost` is specification or proof. Every method keeps the reader
//! well-formed (`wf`), and `remaining` (the bits not yet read) never grows:
//! each successful read of n bits takes exactly n. That measure is what
//! makes the decoder's loops terminate.

use crate::error::InflateError;
use crate::limit_depths::{lemma_shl, p2};
use vstd::prelude::*;

verus! {

pub struct BitReader<'a> {
    data: &'a [u8],
    /// Index of the next byte not yet loaded into `acc`.
    pos: usize,
    acc: u64,
    nbits: u32,
}

/// Masking to the low n bits leaves a value below 2^n.
proof fn lemma_mask(x: u64, n: u32)
    requires
        n <= 32,
    ensures
        (1u64 << n) >= 1,
        (x & (((1u64 << n) - 1) as u64)) <= 0xffff_ffff,
        ((x & (((1u64 << n) - 1) as u64)) as nat) < p2(n as nat),
{
    assert((1u64 << n) >= 1) by (bit_vector)
        requires
            n <= 32,
    ;
    assert((x & (((1u64 << n) - 1) as u64)) < (1u64 << n)) by (bit_vector)
        requires
            n <= 32,
    ;
    assert((x & (((1u64 << n) - 1) as u64)) <= 0xffff_ffff) by (bit_vector)
        requires
            n <= 32,
    ;
    lemma_shl(n);
}

/// A slice's length fits a `usize`.
pub proof fn lemma_slice_len<T>(s: &[T])
    ensures
        s@.len() <= usize::MAX,
{
    assert(s@.len() == s.len());
}

impl<'a> BitReader<'a> {
    /// At most 64 bits are buffered, and they came from bytes already
    /// loaded.
    pub closed spec fn wf(&self) -> bool {
        &&& self.data@.len() <= usize::MAX
        &&& self.pos <= self.data@.len()
        &&& self.nbits <= 64
        &&& self.nbits <= 8 * self.pos
    }

    /// The bits not yet read.
    pub closed spec fn remaining(&self) -> nat {
        (8 * (self.data@.len() - self.pos) + self.nbits) as nat
    }

    pub closed spec fn input_len(&self) -> nat {
        self.data@.len()
    }

    /// The bits buffered (what `available` returns).
    pub closed spec fn buffered(&self) -> nat {
        self.nbits as nat
    }

    /// At a byte boundary.
    pub closed spec fn aligned(&self) -> bool {
        self.nbits % 8 == 0
    }

    /// Creates a reader positioned at the first bit of `data`.
    pub fn new(data: &'a [u8]) -> (r: Self)
        ensures // @ghost
            r.wf(), // @ghost
            r.input_len() == data@.len(), // @ghost
    {
        proof { // @ghost-begin
            lemma_slice_len(data);
        } // @ghost-end
        Self { data, pos: 0, acc: 0, nbits: 0 }
    }

    /// Loads the whole bytes needed to have at least `n` (≤ 32) bits
    /// buffered (fewer are). On failure nothing is consumed.
    #[inline(never)]
    fn fill(&mut self, n: u32) -> (r: Result<(), InflateError>)
        requires // @ghost
            old(self).wf(), // @ghost
            old(self).nbits < n <= 32, // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).data == old(self).data, // @ghost
            final(self).remaining() == old(self).remaining(), // @ghost
            final(self).nbits % 8 == old(self).nbits % 8, // @ghost
            r is Ok ==> final(self).nbits >= n, // @ghost
    {
        // 1..=4 bytes; the loop below has a constant bound so that Kani
        // unrolls it exactly four times.
        let need = (n - self.nbits).div_ceil(8) as usize;
        if need > self.data.len() - self.pos {
            return Err(InflateError::UnexpectedEof);
        }
        let ghost nbits0 = self.nbits; // @ghost
        let mut j = 0;
        while j < 4
            invariant // @ghost
                j <= 4, // @ghost
                self.data == old(self).data, // @ghost
                self.pos == old(self).pos, // @ghost
                nbits0 == old(self).nbits, // @ghost
                nbits0 < n <= 32, // @ghost
                1 <= need <= 4, // @ghost
                8 * (need - 1) < n - nbits0 <= 8 * need, // @ghost
                need <= self.data@.len() - self.pos, // @ghost
                self.data@.len() <= usize::MAX, // @ghost
                self.nbits == nbits0 + 8 * (if j < need { j } else { need }), // @ghost
            decreases 4 - j, // @ghost
        {
            if j < need {
                self.acc |= u64::from(self.data[self.pos + j]) << self.nbits;
                self.nbits += 8;
            }
            j += 1;
        }
        self.pos += need;
        Ok(())
    }

    /// Tops up the accumulator for [`peek`](Self::peek).
    #[inline]
    fn refill(&mut self)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).data == old(self).data, // @ghost
            final(self).remaining() == old(self).remaining(), // @ghost
    {
        if self.nbits > 56 {
            return;
        }
        // Whole bytes that fit: 1..=8.
        let room = ((64 - self.nbits) / 8) as usize;
        // `wrapping_add`: a range that wraps is empty, so `get` fails.
        if let Some(c) = self.data.get(self.pos..self.pos.wrapping_add(8)) {
            let w = le_word([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
            proof { // @ghost-begin
                if room < 8 {
                    let s = (room * 8) as u64;
                    assert((1u64 << s) >= 1) by (bit_vector)
                        requires
                            s < 64,
                    ;
                }
            } // @ghost-end
            let w = if room == 8 { w } else { w & ((1u64 << (room * 8)) - 1) };
            self.acc |= w << self.nbits;
            self.pos += room;
            self.nbits += 8 * room as u32;
        } else {
            // Fewer than 8 bytes left: load them zero-padded.
            let rest = &self.data[self.pos..];
            let k = if rest.len() < room { rest.len() } else { room };
            self.acc |= padded_word(rest, k) << self.nbits;
            self.pos += k;
            self.nbits += 8 * k as u32;
        }
    }

    /// Reads `n` bits (at most 32) as an unsigned integer, first bit in bit 0.
    /// On failure nothing is consumed.
    #[inline(always)]
    pub fn read_bits(&mut self, n: u32) -> (r: Result<u32, InflateError>)
        requires // @ghost
            old(self).wf(), // @ghost
            n <= 32, // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).input_len() == old(self).input_len(), // @ghost
            final(self).remaining() <= old(self).remaining(), // @ghost
            r is Ok ==> final(self).remaining() + n == old(self).remaining(), // @ghost
            r is Ok ==> (r->Ok_0 as nat) < p2(n as nat), // @ghost
            old(self).aligned() && n % 8 == 0 ==> final(self).aligned(), // @ghost
    {
        debug_assert!(n <= 32, "read_bits: n = {n} exceeds 32");
        let n = if n > 32 { 32 } else { n };
        if self.nbits < n {
            self.fill(n)?;
        }
        proof { // @ghost-begin
            lemma_mask(self.acc, n);
        } // @ghost-end
        let v = (self.acc & ((1u64 << n) - 1)) as u32;
        self.acc >>= n;
        self.nbits -= n;
        Ok(v)
    }

    /// Reads a single bit.
    #[inline]
    pub fn read_bit(&mut self) -> (r: Result<u32, InflateError>)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).input_len() == old(self).input_len(), // @ghost
            final(self).remaining() <= old(self).remaining(), // @ghost
            r is Ok ==> r->Ok_0 <= 1 && final(self).remaining() + 1 == old(self).remaining(), // @ghost
    {
        if self.nbits == 0 {
            // The accumulator is empty, so load exactly one byte.
            let b = *self.data.get(self.pos).ok_or(InflateError::UnexpectedEof)?;
            self.acc = u64::from(b);
            self.pos += 1;
            self.nbits = 8;
        }
        proof { // @ghost-begin
            let a = self.acc;
            assert(a & 1 <= 1) by (bit_vector);
        } // @ghost-end
        let v = (self.acc & 1) as u32;
        self.acc >>= 1;
        self.nbits -= 1;
        Ok(v)
    }

    /// The next `n` (at most 32) bits without consuming them, with zeros in
    /// place of any bits past the end of the input.
    #[inline]
    pub fn peek(&mut self, n: u32) -> (r: u32)
        requires // @ghost
            old(self).wf(), // @ghost
            n <= 32, // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).input_len() == old(self).input_len(), // @ghost
            final(self).remaining() == old(self).remaining(), // @ghost
            (r as nat) < p2(n as nat), // @ghost
    {
        debug_assert!(n <= 32);
        if self.nbits < n {
            self.refill();
        }
        proof { // @ghost-begin
            lemma_mask(self.acc, n);
        } // @ghost-end
        (self.acc & ((1u64 << n) - 1)) as u32
    }

    /// Bits currently buffered.
    #[inline]
    pub fn available(&self) -> (r: u32)
        ensures // @ghost
            r as nat == self.buffered(), // @ghost
    {
        self.nbits
    }

    /// Discards `n` buffered bits. Requires `n <= available()`.
    #[inline]
    pub fn consume(&mut self, n: u32)
        requires // @ghost
            old(self).wf(), // @ghost
            n as nat <= old(self).buffered(), // @ghost
            n < 64, // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).input_len() == old(self).input_len(), // @ghost
            final(self).remaining() + n == old(self).remaining(), // @ghost
    {
        debug_assert!(n <= self.nbits);
        self.acc >>= n;
        self.nbits -= n;
    }

    /// Discards bits up to the next byte boundary.
    pub fn align_to_byte(&mut self)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).aligned(), // @ghost
            final(self).input_len() == old(self).input_len(), // @ghost
            final(self).remaining() <= old(self).remaining(), // @ghost
    {
        let drop = self.nbits % 8;
        self.acc >>= drop;
        self.nbits -= drop;
    }

    /// Fills `out` with the next `out.len()` whole bytes. The reader must
    /// be byte-aligned (call [`align_to_byte`](Self::align_to_byte) first).
    /// On failure the reader's position is unspecified.
    pub fn read_aligned_bytes(&mut self, out: &mut [u8]) -> (r: Result<(), InflateError>)
        requires // @ghost
            old(self).wf(), // @ghost
            old(self).aligned(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).input_len() == old(self).input_len(), // @ghost
            final(self).remaining() <= old(self).remaining(), // @ghost
            final(out)@.len() == old(out)@.len(), // @ghost
    {
        debug_assert!(self.nbits % 8 == 0, "read_aligned_bytes on unaligned reader");
        self.align_to_byte();
        let len = out.len();
        // Whole bytes already sitting in the accumulator (at most 8) come first.
        let buffered = (self.nbits / 8) as usize;
        let k = if len < buffered { len } else { buffered };
        let acc = le_bytes(self.acc);
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
    pub fn bytes_consumed(&self) -> (r: usize)
        requires // @ghost
            self.wf(), // @ghost
        ensures // @ghost
            r as nat <= self.input_len(), // @ghost
    {
        self.pos - (self.nbits / 8) as usize
    }
}

/// Accumulates bits LSB-first into a byte vector.
pub struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    nbits: u32,
}

impl BitWriter {
    /// Fewer than 32 bits are pending between calls.
    pub closed spec fn wf(&self) -> bool {
        self.nbits < 32
    }

    /// Creates an empty writer.
    pub fn new() -> (r: Self)
        ensures // @ghost
            r.wf(), // @ghost
    {
        Self { out: Vec::new(), acc: 0, nbits: 0 }
    }

    /// Creates an empty writer whose buffer can hold `bytes` bytes without reallocating.
    pub fn with_capacity(bytes: usize) -> (r: Self)
        ensures // @ghost
            r.wf(), // @ghost
    {
        Self { out: Vec::with_capacity(bytes), acc: 0, nbits: 0 }
    }

    /// Appends the low `n` bits of `value`, least-significant bit first.
    #[inline]
    pub fn write_bits(&mut self, value: u32, n: u32)
        requires // @ghost
            old(self).wf(), // @ghost
            n <= 32, // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
    {
        debug_assert!(n <= 32, "write_bits: n = {n} exceeds 32");
        let n = if n > 32 { 32 } else { n };
        proof { // @ghost-begin
            lemma_mask(0, n);
        } // @ghost-end
        let masked = u64::from(value) & ((1u64 << n) - 1);
        // Fewer than 32 bits are pending, so this fits in the accumulator.
        self.acc |= masked << self.nbits;
        self.nbits += n;
        // Flush in whole 4-byte words: a fixed-size copy, and no loop.
        if self.nbits >= 32 {
            self.out.extend_from_slice(&le_bytes32(self.acc as u32));
            self.acc >>= 32;
            self.nbits -= 32;
        }
    }

    /// Total number of bits written so far (modulo 2^64).
    pub fn bit_len(&self) -> u64 {
        (self.out.len() as u64).wrapping_mul(8).wrapping_add(u64::from(self.nbits))
    }

    /// True when the next bit would start a new byte.
    pub fn is_byte_aligned(&self) -> bool {
        self.nbits % 8 == 0
    }

    /// Fills the rest of the current byte with zero bits (and writes out
    /// all pending bits).
    pub fn pad_to_byte(&mut self)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
    {
        if self.nbits > 0 {
            let k = self.nbits.div_ceil(8) as usize;
            self.out.extend_from_slice(&le_bytes(self.acc)[..k]);
            self.acc = 0;
            self.nbits = 0;
        }
    }

    /// Pads to a byte boundary, then appends `bytes` verbatim.
    pub fn write_aligned_bytes(&mut self, bytes: &[u8])
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
    {
        self.pad_to_byte();
        self.out.extend_from_slice(bytes);
    }

    /// Takes the whole bytes written so far.
    pub fn take_bytes(&mut self) -> Vec<u8>
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
    {
        let mut out = Vec::new();
        core::mem::swap(&mut out, &mut self.out);
        out
    }

    /// Pads to a byte boundary and returns the bytes written.
    pub fn finish(mut self) -> Vec<u8>
        requires // @ghost
            self.wf(), // @ghost
    {
        self.pad_to_byte();
        self.out
    }
}

/// `u32::to_le_bytes`, taken as given (see `le_word`).
#[verifier::external_body]
#[inline(always)]
fn le_bytes32(w: u32) -> [u8; 4] {
    w.to_le_bytes()
}

/// The first `k` (at most 7) bytes of `rest` as a little-endian word,
/// zero-padded.
#[cold]
#[inline(never)]
fn padded_word(rest: &[u8], k: usize) -> u64
    requires // @ghost
        k <= rest@.len(), // @ghost
{
    let mut b = [0u8; 8];
    let mut i = 0;
    while i < 7
        invariant // @ghost
            i <= 7, // @ghost
            k <= rest@.len(), // @ghost
        decreases 7 - i, // @ghost
    {
        if i < k {
            b[i] = rest[i];
        }
        i += 1;
    }
    le_word(b)
}

/// `u64::from_le_bytes`, taken as given: Verus has no specification for
/// it, and it cannot panic.
#[verifier::external_body]
#[inline(always)]
fn le_word(b: [u8; 8]) -> u64 {
    u64::from_le_bytes(b)
}

/// `u64::to_le_bytes`, likewise.
#[verifier::external_body]
#[inline(always)]
fn le_bytes(w: u64) -> [u8; 8] {
    w.to_le_bytes()
}

/// Reverses the low `len` bits of `code` (`len` ≤ 16); higher bits must be zero.
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

} // verus!
