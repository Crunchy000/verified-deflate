//! The Huffman decoders (`Canonical` and `Decoder` in `src/huffman.rs`).
//!
//! The executable code below is the library's, token for token (CI checks
//! this with `.github/scripts/verus_drift.py`); every line marked
//! `// @ghost` is specification or proof. Building a decoder from any code
//! lengths whatsoever leaves it well-formed (`wf`): at most 288 symbols
//! have codes, so every index the bit-by-bit walk computes is in bounds,
//! and the lookup table is at most 2^10 entries. Decoding with a
//! well-formed decoder never panics, and a successful decode reads at
//! least one bit.

use crate::bitio::BitReader;
use crate::build_lengths::{lemma_shl_usize, MAX_CODE_LEN, MAX_SYMBOLS};
use crate::error::InflateError;
use crate::limit_depths::{lemma_p2_mono, lemma_p2_pos, p2, HuffmanError};
use vstd::prelude::*;

verus! {

/// Bits looked up at once by [`Decoder::decode`]'s table.
pub const FAST_BITS: u32 = 10;

const FAST_SIZE: usize = 1 << FAST_BITS;

/// The number of codes of lengths 1-15.
pub open spec fn total(c: Seq<u16>) -> int {
    c[1] + c[2] + c[3] + c[4] + c[5] + c[6] + c[7] + c[8] + c[9] + c[10] + c[11] + c[12] + c[13] + c[14]
        + c[15]
}

/// All sixteen counts.
pub open spec fn sum16(c: Seq<u16>) -> int {
    c[0] + total(c)
}

/// The codes of lengths 1..k-1.
pub open spec fn psum(c: Seq<u16>, k: int) -> int
    decreases k,
{
    if k <= 1 {
        0
    } else {
        psum(c, k - 1) + c[k - 1]
    }
}

/// How many of `s` equal `l`.
pub open spec fn hist(s: Seq<u8>, l: int) -> nat
    decreases s.len(),
{
    if s.len() == 0 {
        0
    } else {
        hist(s.drop_last(), l) + (if s.last() == l {
            1nat
        } else {
            0nat
        })
    }
}

proof fn lemma_hist_take(s: Seq<u8>, i: int)
    requires
        0 <= i < s.len(),
    ensures
        forall|l: int|
            #[trigger] hist(s.take(i + 1), l) == hist(s.take(i), l) + (if s[i] == l {
                1nat
            } else {
                0nat
            }),
{
    assert(s.take(i + 1).drop_last() =~= s.take(i));
    assert(s.take(i + 1).last() == s[i]);
}

proof fn lemma_hist_take_le(s: Seq<u8>, i: int, l: int)
    requires
        0 <= i <= s.len(),
    ensures
        hist(s.take(i), l) <= hist(s, l),
    decreases s.len() - i,
{
    if i == s.len() {
        assert(s.take(i) =~= s);
    } else {
        lemma_hist_take(s, i);
        lemma_hist_take_le(s, i + 1, l);
    }
}

proof fn lemma_sum16_update(c: Seq<u16>, l: int, v: u16)
    requires
        c.len() == 16,
        0 <= l < 16,
    ensures
        sum16(c.update(l, v)) == sum16(c) - c[l] + v,
        total(c.update(l, v)) == total(c) - (if l == 0 {
            0
        } else {
            c[l] as int
        }) + (if l == 0 {
            0
        } else {
            v as int
        }),
{
}

proof fn lemma_psum(c: Seq<u16>)
    ensures
        psum(c, 16) == total(c),
        forall|a: int, b: int| #![trigger psum(c, a), psum(c, b)] 1 <= a <= b ==> psum(c, a) <= psum(c, b),
{
    reveal_with_fuel(psum, 16);
    assert forall|a: int, b: int| #![trigger psum(c, a), psum(c, b)] 1 <= a <= b implies psum(c, a) <= psum(c, b) by {
        lemma_psum_mono(c, a, b);
    }
}

proof fn lemma_psum_mono(c: Seq<u16>, a: int, b: int)
    requires
        1 <= a <= b,
    ensures
        psum(c, a) <= psum(c, b),
    decreases b - a,
{
    if a < b {
        lemma_psum_mono(c, a, b - 1);
    }
}

pub proof fn lemma_p2_10()
    ensures
        p2(10) == 1024,
        p2(15) == 32768,
{
    reveal_with_fuel(p2, 16);
}

/// `1 << s` (a `u32` shift amount) is 2^s.
proof fn lemma_shl_usize_u32(s: u32)
    requires
        s <= 15,
    ensures
        (1usize << s) as nat == p2(s as nat),
    decreases s,
{
    if s == 0 {
        assert(1usize << 0u32 == 1usize) by (bit_vector);
    } else {
        let t = (s - 1) as u32;
        lemma_shl_usize_u32(t);
        assert(1usize << s == 2 * (1usize << t)) by (bit_vector)
            requires
                t < 15,
                s == t + 1,
        ;
    }
}

pub struct Canonical {
    /// `count[l]` = number of symbols with code length `l` (`count[0]` unused).
    count: [u16; MAX_CODE_LEN + 1],
    /// Coded symbols ordered by (length, symbol), i.e. by canonical code.
    symbol: [u16; MAX_SYMBOLS],
    /// Number of symbols with a nonzero length.
    coded: u16,
    /// Unused code space in units of `2^-15`; zero for a complete code.
    left: u16,
}

impl Canonical {
    /// At most 288 symbols have codes of lengths 1-15.
    pub closed spec fn wf(&self) -> bool {
        &&& self.count@[0] == 0
        &&& total(self.count@) <= 288
    }

    /// A decoder with no codes (every decode fails).
    pub exec const EMPTY: Self
        ensures // @ghost
            Self::EMPTY.wf(), // @ghost
    {
        Self { count: [0; MAX_CODE_LEN + 1], symbol: [0; MAX_SYMBOLS], coded: 0, left: 1 << 15 }
    }

    /// Rebuilds this decoder in place for new code lengths. On error the
    /// decoder decodes nothing.
    pub const fn rebuild(&mut self, lengths: &[u8]) -> (r: Result<(), HuffmanError>)
        ensures // @ghost
            final(self).wf(), // @ghost
            r is Ok ==> lengths@.len() <= 288, // @ghost
            r is Ok ==> forall|i: int| 0 <= i < lengths@.len() ==> lengths@[i] <= 15, // @ghost
    {
        let n = lengths.len();
        self.count = [0; MAX_CODE_LEN + 1];
        self.coded = 0;
        self.left = 1 << 15;
        if n > MAX_SYMBOLS {
            return Err(HuffmanError::TooManySymbols);
        }
        let mut count = [0u16; MAX_CODE_LEN + 1];
        let mut s = 0;
        while s < n
            invariant // @ghost
                s <= n <= 288, // @ghost
                n == lengths@.len(), // @ghost
                self.wf(), // @ghost
                sum16(count@) == s, // @ghost
                forall|l: int| 0 <= l < 16 ==> #[trigger] count@[l] <= s, // @ghost
                forall|l: int| 0 <= l < 16 ==> #[trigger] count@[l] == hist(lengths@.take(s as int), l), // @ghost
                forall|i: int| 0 <= i < s ==> lengths@[i] <= 15, // @ghost
            decreases n - s, // @ghost
        {
            let l = lengths[s] as usize;
            if l > MAX_CODE_LEN {
                return Err(HuffmanError::CodeTooLong);
            }
            proof { // @ghost-begin
                lemma_hist_take(lengths@, s as int);
                lemma_sum16_update(count@, l as int, (count@[l as int] + 1) as u16);
            } // @ghost-end
            count[l] += 1;
            s += 1;
        }
        proof { // @ghost-begin
            assert(lengths@.take(n as int) =~= lengths@);
        } // @ghost-end
        let coded = n as u16 - count[0];
        count[0] = 0;
        proof { // @ghost-begin
            lemma_p2_10();
        } // @ghost-end
        let mut left: i32 = 1;
        let mut len = 1;
        while len <= MAX_CODE_LEN
            invariant // @ghost
                1 <= len <= 16, // @ghost
                0 <= left <= p2((len - 1) as nat), // @ghost
                self.wf(), // @ghost
                p2(15) == 32768, // @ghost
            decreases 16 - len, // @ghost
        {
            proof { // @ghost-begin
                lemma_p2_mono((len - 1) as nat, 15);
            } // @ghost-end
            left = left * 2 - count[len] as i32;
            if left < 0 {
                return Err(HuffmanError::OverSubscribed);
            }
            len += 1;
        }
        // offs[l] = index in `symbol` of the first code of length l. Entries
        // of `symbol` at or beyond `coded` are never read, so stale values
        // from an earlier build need no clearing.
        let mut offs = [0u16; MAX_CODE_LEN + 2];
        proof { // @ghost-begin
            lemma_psum(count@);
        } // @ghost-end
        len = 1;
        while len <= MAX_CODE_LEN
            invariant // @ghost
                1 <= len <= 16, // @ghost
                forall|l: int| 1 <= l <= len ==> #[trigger] offs@[l] == psum(count@, l), // @ghost
                psum(count@, 16) == total(count@) <= 288, // @ghost
                forall|a: int, b: int| #![trigger psum(count@, a), psum(count@, b)] 1 <= a <= b ==> psum(count@, a) <= psum(count@, b), // @ghost
            decreases 16 - len, // @ghost
        {
            proof { // @ghost-begin
                assert(psum(count@, len + 1) == psum(count@, len as int) + count@[len as int]);
            } // @ghost-end
            offs[len + 1] = offs[len] + count[len];
            len += 1;
        }
        s = 0;
        while s < n
            invariant // @ghost
                s <= n == lengths@.len(), // @ghost
                self.wf(), // @ghost
                forall|i: int| 0 <= i < n ==> lengths@[i] <= 15, // @ghost
                forall|l: int| 1 <= l < 16 ==> #[trigger] count@[l] == hist(lengths@, l), // @ghost
                forall|l: int| // @ghost
                    1 <= l < 16 ==> #[trigger] offs@[l] == psum(count@, l) + hist(lengths@.take(s as int), l), // @ghost
                psum(count@, 16) == total(count@) <= 288, // @ghost
                forall|a: int, b: int| #![trigger psum(count@, a), psum(count@, b)] 1 <= a <= b ==> psum(count@, a) <= psum(count@, b), // @ghost
            decreases n - s, // @ghost
        {
            let l = lengths[s] as usize;
            proof { // @ghost-begin
                lemma_hist_take(lengths@, s as int);
            } // @ghost-end
            if l != 0 {
                proof { // @ghost-begin
                    lemma_hist_take_le(lengths@, s + 1, l as int);
                    assert(psum(count@, l + 1) == psum(count@, l as int) + count@[l as int]);
                } // @ghost-end
                self.symbol[offs[l] as usize] = s as u16;
                offs[l] += 1;
            }
            s += 1;
        }
        self.count = count;
        self.coded = coded;
        self.left = left as u16;
        Ok(())
    }

    /// True when the code is complete (Kraft sum exactly 1).
    pub const fn is_complete(&self) -> bool {
        self.left == 0
    }

    /// Number of symbols with a code.
    pub const fn coded_symbols(&self) -> usize {
        self.coded as usize
    }

    /// True for a code with exactly one symbol, of length 1.
    pub const fn is_single_unit_code(&self) -> bool {
        self.coded == 1 && self.count[1] == 1
    }

    /// Reads one symbol bit by bit.
    #[inline(never)]
    #[allow(unused_assignments)]
    #[verifier::rlimit(40)]
    pub fn decode(&self, br: &mut BitReader<'_>) -> (r: Result<u16, InflateError>)
        requires // @ghost
            self.wf(), // @ghost
            old(br).wf(), // @ghost
        ensures // @ghost
            final(br).wf(), // @ghost
            final(br).input_len() == old(br).input_len(), // @ghost
            final(br).remaining() <= old(br).remaining(), // @ghost
            r is Ok ==> final(br).remaining() < old(br).remaining(), // @ghost
    {
        // Invariant at each step: first <= code, first is the smallest
        // canonical code of the current length, and index is the position
        // of that code in `symbol`. (Plain arithmetic rather than shifts
        // and ors, which compile the same, so that Verus can bound it.)
        let mut code: u32 = 0;
        let mut first: u32 = 0;
        let mut index: u32 = 0;
        macro_rules! step {
            ($len:expr) => {
                code += br.read_bit()?;
                let count = u32::from(self.count[$len]);
                if code < first + count {
                    return Ok(self.symbol[(index + (code - first)) as usize]);
                }
                index += count;
                first = (first + count) * 2;
                code *= 2;
            };
        }
        step!(1);
        step!(2);
        step!(3);
        step!(4);
        step!(5);
        step!(6);
        step!(7);
        step!(8);
        step!(9);
        step!(10);
        step!(11);
        step!(12);
        step!(13);
        step!(14);
        step!(15);
        Err(InflateError::InvalidSymbol)
    }
}

pub struct Decoder {
    canon: Canonical,
    /// `fast[bits]` = `(len << 12) | symbol` for the code that the low
    /// `len` of `bits` spell (bit-reversed), or 0 if no code of at most
    /// `fast_bits` bits matches.
    fast: [u16; FAST_SIZE],
    /// Table width in bits, at most [`FAST_BITS`].
    fast_bits: u32,
}

impl Decoder {
    pub closed spec fn wf(&self) -> bool {
        &&& self.canon.wf()
        &&& self.fast_bits <= FAST_BITS
    }

    /// A decoder with no codes (every decode fails).
    pub exec const EMPTY: Self
        ensures // @ghost
            Self::EMPTY.wf(), // @ghost
    {
        Self { canon: Canonical::EMPTY, fast: [0; FAST_SIZE], fast_bits: FAST_BITS }
    }

    /// Builds a decoder for the given code lengths (see [`Canonical::new`]).
    pub const fn new(lengths: &[u8]) -> (r: Result<Self, HuffmanError>)
        ensures // @ghost
            r is Ok ==> r->Ok_0.wf(), // @ghost
    {
        let mut d = Self::EMPTY;
        match d.rebuild(lengths) {
            Ok(()) => Ok(d),
            Err(e) => Err(e),
        }
    }

    /// Rebuilds this decoder in place for new code lengths. On error the
    /// decoder decodes nothing.
    pub const fn rebuild(&mut self, lengths: &[u8]) -> (r: Result<(), HuffmanError>)
        ensures // @ghost
            final(self).wf(), // @ghost
    {
        self.rebuild_with_fast_bits(lengths, FAST_BITS)
    }

    /// [`rebuild`](Self::rebuild) with a lookup table of `fast_bits` bits.
    pub const fn rebuild_with_fast_bits(&mut self, lengths: &[u8], fast_bits: u32) -> (r: Result<(), HuffmanError>)
        ensures // @ghost
            final(self).wf(), // @ghost
    {
        self.fast = [0; FAST_SIZE];
        let fb = if fast_bits > FAST_BITS { FAST_BITS } else { fast_bits };
        self.fast_bits = fb;
        if let Err(e) = self.canon.rebuild(lengths) {
            return Err(e);
        }
        proof { // @ghost-begin
            lemma_p2_10();
            assert(FAST_SIZE == 1024) by (compute_only);
            lemma_shl_usize_u32(fb);
            lemma_p2_mono(fb as nat, 10);
        } // @ghost-end
        // Each code of at most fb bits fills every entry whose low bits
        // spell it (codes are packed bit-reversed). The codes are the
        // canonical ones, assigned in symbol order within each length.
        let count = &self.canon.count;
        let mut next = [0u32; MAX_CODE_LEN + 1];
        let mut code = 0u32;
        let mut len = 1;
        while len <= MAX_CODE_LEN
            invariant // @ghost
                1 <= len <= 16, // @ghost
                count@[0] == 0, // @ghost
                total(count@) <= 288, // @ghost
                p2(15) == 32768, // @ghost
                code <= 576 * (p2((len - 1) as nat) - 1), // @ghost
                forall|l: int| 0 <= l < 16 ==> #[trigger] next@[l] <= 576 * 32768, // @ghost
            decreases 16 - len, // @ghost
        {
            proof { // @ghost-begin
                lemma_p2_mono(len as nat, 15);
                lemma_p2_pos((len - 1) as nat);
            } // @ghost-end
            code = (code + count[len - 1] as u32) * 2;
            next[len] = code;
            len += 1;
        }
        let mut s = 0;
        while s < lengths.len()
            invariant // @ghost
                self.wf(), // @ghost
                fb == self.fast_bits <= 10, // @ghost
                (1usize << fb) as nat == p2(fb as nat) <= 1024, // @ghost
                FAST_SIZE == 1024, // @ghost
                lengths@.len() <= 288, // @ghost
                forall|i: int| 0 <= i < lengths@.len() ==> lengths@[i] <= 15, // @ghost
                forall|l: int| 0 <= l < 16 ==> #[trigger] next@[l] <= 576 * 32768 + s, // @ghost
            decreases lengths.len() - s, // @ghost
        {
            let l = lengths[s] as usize;
            if l != 0 {
                let c = next[l];
                next[l] += 1;
                if l as u32 <= fb {
                    proof { // @ghost-begin
                        lemma_shl_usize(l);
                        lemma_p2_mono(l as nat, fb as nat);
                        lemma_p2_pos((l - 1) as nat);
                    } // @ghost-end
                    let mut idx = crate::bitio::reverse_bits(c as u16, l as u32) as usize;
                    while idx < (1 << fb)
                        invariant // @ghost
                            self.wf(), // @ghost
                            fb == self.fast_bits, // @ghost
                            (1usize << fb) as nat <= 1024, // @ghost
                            FAST_SIZE == 1024, // @ghost
                            1 <= l <= fb, // @ghost
                            2 <= (1usize << l) as nat <= 1024, // @ghost
                        decreases (1usize << fb) - idx, // @ghost
                    {
                        self.fast[idx] = ((l as u16) << 12) | s as u16;
                        idx += 1 << l;
                    }
                }
            }
            s += 1;
        }
        Ok(())
    }

    /// True when the code is complete (Kraft sum exactly 1).
    pub const fn is_complete(&self) -> bool {
        self.canon.is_complete()
    }

    /// Number of symbols with a code.
    pub const fn coded_symbols(&self) -> usize {
        self.canon.coded_symbols()
    }

    /// True for a code with exactly one symbol, of length 1.
    pub const fn is_single_unit_code(&self) -> bool {
        self.canon.is_single_unit_code()
    }

    /// Reads one symbol.
    #[inline(always)]
    pub fn decode(&self, br: &mut BitReader<'_>) -> (r: Result<u16, InflateError>)
        requires // @ghost
            self.wf(), // @ghost
            old(br).wf(), // @ghost
        ensures // @ghost
            final(br).wf(), // @ghost
            final(br).input_len() == old(br).input_len(), // @ghost
            final(br).remaining() <= old(br).remaining(), // @ghost
            r is Ok ==> final(br).remaining() < old(br).remaining(), // @ghost
    {
        proof { // @ghost-begin
            lemma_p2_10();
            lemma_p2_mono(self.fast_bits as nat, 10);
            assert(FAST_SIZE == 1024) by (compute_only);
        } // @ghost-end
        let e = self.fast[br.peek(self.fast_bits) as usize];
        proof { // @ghost-begin
            assert(e >> 12 < 16) by (bit_vector);
        } // @ghost-end
        let len = u32::from(e >> 12);
        if len != 0 && len <= br.available() {
            br.consume(len);
            return Ok(e & 0x0fff);
        }
        self.canon.decode(br)
    }
}

} // verus!
