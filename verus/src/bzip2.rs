//! The bzip2 decoder (`src/bzip2.rs`): `decompress_with_limit` and
//! `read_stream`, with the bit reader, the Huffman tables, the inverse
//! Burrows–Wheeler transform and the run-length step.
//!
//! The executable code below is the library's, token for token (CI checks
//! this with `.github/scripts/verus_drift.py`); every line marked
//! `// @ghost` is specification or proof. The theorem: on every byte
//! string of every length, and every output limit, `decompress_with_limit`
//! and `read_stream` return, without panicking (every index is in bounds,
//! no arithmetic overflows, every loop ends), and what they return is
//! within the limit. (That the output is the stream's contents rests on
//! the tests against libbz2 and the fuzzing; see VERIFICATION.md.)

use crate::limit_depths::{lemma_p2_mono, lemma_shl, p2};
use vstd::prelude::*;

verus! {

// @ghost-begin
proof fn lemma_p2_small()
    ensures
        p2(1) == 2,
        p2(20) == 1048576,
{
    reveal_with_fuel(p2, 21);
}

/// A lookup-table entry as `DecodeTable::new` writes it: its code length
/// (1–10) above its symbol.
proof fn lemma_fast_entry(zn: u32, sym: u16)
    requires
        1 <= zn <= 10,
        sym < 512,
    ensures
        ((zn << 9) as u16) | sym != 0,
        1 <= (((zn << 9) as u16) | sym) >> 9 <= 10,
        (((zn << 9) as u16) | sym) & 0x1ff == sym,
{
    assert(((((zn << 9) as u16) | sym) >> 9) == zn as u16) by (bit_vector)
        requires
            1 <= zn <= 10,
            sym < 512,
    ;
    assert(((((zn << 9) as u16) | sym) & 0x1ff) == sym) by (bit_vector)
        requires
            1 <= zn <= 10,
            sym < 512,
    ;
}

/// An entry of the lookup table: empty, or a code length of 1–10.
spec fn fast_entry_ok(e: u16) -> bool {
    e == 0 || (1 <= (e >> 9) && (e >> 9) <= 10)
}

/// Every table is well formed.
spec fn all_shaped(ts: Seq<DecodeTable>) -> bool {
    forall|k: int| 0 <= k < ts.len() ==> #[trigger] ts[k].wf_shape()
}

/// Every table's symbols are below `m`.
spec fn all_below(ts: Seq<DecodeTable>, m: nat) -> bool {
    forall|k: int| 0 <= k < ts.len() ==> #[trigger] ts[k].syms_below(m)
}

/// `s` is a symbol one of the tables can produce.
spec fn sym_from(ts: Seq<DecodeTable>, s: u16) -> bool {
    exists|k: int| 0 <= k < ts.len() && #[trigger] ts[k].yields(s)
}

proof fn lemma_yield_below(t: DecodeTable, s: u16, m: nat)
    requires
        t.syms_below(m),
        t.yields(s),
    ensures
        (s as nat) < m,
{
    if exists|k: int| 0 <= k < 258 && (#[trigger] t.perm@[k]) == s {
        let k = choose|k: int| 0 <= k < 258 && (#[trigger] t.perm@[k]) == s;
        assert((t.perm@[k] as nat) < m);
    } else {
        let c = choose|c: int| 0 <= c < 1024 && (#[trigger] t.fast@[c]) != 0 && (t.fast@[c] & 0x1ff) == s;
        assert(t.fast@[c] == 0 || ((t.fast@[c] & 0x1ff) as nat) < m);
    }
}

proof fn lemma_below(ts: Seq<DecodeTable>, s: u16, m: nat)
    requires
        sym_from(ts, s),
        all_below(ts, m),
    ensures
        (s as nat) < m,
{
    let k = choose|k: int| 0 <= k < ts.len() && #[trigger] ts[k].yields(s);
    assert(ts[k].syms_below(m));
    lemma_yield_below(ts[k], s, m);
}
// @ghost-end

/// The 48-bit marker that starts a block (the digits of pi).
const BLOCK_MAGIC: u64 = 0x3141_5926_5359;
/// The 48-bit marker that ends a stream (the digits of the square root of pi).
const END_MAGIC: u64 = 0x1772_4538_5090;

const RUNA: u16 = 0;
const RUNB: u16 = 1;
/// Symbols per selector group.
const GROUP_SIZE: usize = 50;
/// Most Huffman tables a block may have.
const MAX_GROUPS: usize = 6;
/// The most symbols: 256 move-to-front values, RUNA/RUNB, end of block.
const MAX_ALPHA: usize = 258;
/// The longest code the format allows.
const MAX_CODE_LEN: usize = 20;
/// libbz2 reads, but ignores, selectors past this many.
const MAX_SELECTORS: usize = 2 + 900_000 / GROUP_SIZE;

/// Why a bzip2 stream could not be read.
pub enum Bzip2Error {
    /// The data does not start with `BZh` and a digit 1–9.
    NotBzip2,
    /// The data ended in the middle of the stream.
    Truncated,
    /// The stream is corrupt; the message says how.
    Corrupt(&'static str),
    /// A block's data does not match its CRC.
    BlockCrcMismatch {
        /// CRC recorded in the block header.
        expected: u32,
        /// CRC of the decompressed block.
        actual: u32,
    },
    /// The stream's combined CRC does not match its blocks.
    StreamCrcMismatch {
        /// CRC recorded at the end of the stream.
        expected: u32,
        /// CRC computed from the blocks.
        actual: u32,
    },
    /// Decompressing would exceed the output limit.
    OutputLimitExceeded,
    /// After the last stream come bytes that do not start another.
    TrailingData,
}

// ---------------------------------------------------------------------------
// CRC

const fn make_crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256
        invariant // @ghost
            i <= 256, // @ghost
        decreases 256 - i, // @ghost
    {
        let mut c = (i as u32) << 24;
        let mut k = 0;
        while k < 8
            invariant // @ghost
                k <= 8, // @ghost
                i < 256, // @ghost
            decreases 8 - k, // @ghost
        {
            c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04C1_1DB7 } else { c << 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

exec const CRC_TABLE: [u32; 256] = make_crc_table();

#[derive(Clone, Copy)]
struct Crc(u32);

impl Crc {
    const fn new() -> Self {
        Self(0xFFFF_FFFF)
    }

    fn byte(&mut self, b: u8) {
        self.0 = (self.0 << 8) ^ CRC_TABLE[((self.0 >> 24) as u8 ^ b) as usize];
    }

    const fn finish(self) -> u32 {
        !self.0
    }
}

pub fn block_crc(data: &[u8]) -> u32 {
    let mut c = Crc::new();
    let mut i = 0;
    while i < data.len()
        invariant // @ghost
            i <= data@.len(), // @ghost
        decreases data@.len() - i, // @ghost
    {
        c.byte(data[i]);
        i += 1;
    }
    c.finish()
}

const fn combine(stream: u32, block: u32) -> u32 {
    ((stream << 1) | (stream >> 31)) ^ block
}

/// The table that drives "randomised" blocks: bzip2 0.9.0 flipped the low
/// bit of selected bytes of a block that sorted slowly. Later versions
/// never write such blocks but still read them, as this decoder does.
const RAND_NUMS: [u16; 512] = [
    619, 720, 127, 481, 931, 816, 813, 233, 566, 247, 985, 724, 205, 454, 863, 491, 741, 242, 949, 214, 733, 859, 335,
    708, 621, 574, 73, 654, 730, 472, 419, 436, 278, 496, 867, 210, 399, 680, 480, 51, 878, 465, 811, 169, 869, 675,
    611, 697, 867, 561, 862, 687, 507, 283, 482, 129, 807, 591, 733, 623, 150, 238, 59, 379, 684, 877, 625, 169, 643,
    105, 170, 607, 520, 932, 727, 476, 693, 425, 174, 647, 73, 122, 335, 530, 442, 853, 695, 249, 445, 515, 909, 545,
    703, 919, 874, 474, 882, 500, 594, 612, 641, 801, 220, 162, 819, 984, 589, 513, 495, 799, 161, 604, 958, 533, 221,
    400, 386, 867, 600, 782, 382, 596, 414, 171, 516, 375, 682, 485, 911, 276, 98, 553, 163, 354, 666, 933, 424, 341,
    533, 870, 227, 730, 475, 186, 263, 647, 537, 686, 600, 224, 469, 68, 770, 919, 190, 373, 294, 822, 808, 206, 184,
    943, 795, 384, 383, 461, 404, 758, 839, 887, 715, 67, 618, 276, 204, 918, 873, 777, 604, 560, 951, 160, 578, 722,
    79, 804, 96, 409, 713, 940, 652, 934, 970, 447, 318, 353, 859, 672, 112, 785, 645, 863, 803, 350, 139, 93, 354, 99,
    820, 908, 609, 772, 154, 274, 580, 184, 79, 626, 630, 742, 653, 282, 762, 623, 680, 81, 927, 626, 789, 125, 411,
    521, 938, 300, 821, 78, 343, 175, 128, 250, 170, 774, 972, 275, 999, 639, 495, 78, 352, 126, 857, 956, 358, 619,
    580, 124, 737, 594, 701, 612, 669, 112, 134, 694, 363, 992, 809, 743, 168, 974, 944, 375, 748, 52, 600, 747, 642,
    182, 862, 81, 344, 805, 988, 739, 511, 655, 814, 334, 249, 515, 897, 955, 664, 981, 649, 113, 974, 459, 893, 228,
    433, 837, 553, 268, 926, 240, 102, 654, 459, 51, 686, 754, 806, 760, 493, 403, 415, 394, 687, 700, 946, 670, 656,
    610, 738, 392, 760, 799, 887, 653, 978, 321, 576, 617, 626, 502, 894, 679, 243, 440, 680, 879, 194, 572, 640, 724,
    926, 56, 204, 700, 707, 151, 457, 449, 797, 195, 791, 558, 945, 679, 297, 59, 87, 824, 713, 663, 412, 693, 342,
    606, 134, 108, 571, 364, 631, 212, 174, 643, 304, 329, 343, 97, 430, 751, 497, 314, 983, 374, 822, 928, 140, 206,
    73, 263, 980, 736, 876, 478, 430, 305, 170, 514, 364, 692, 829, 82, 855, 953, 676, 246, 369, 970, 294, 750, 807,
    827, 150, 790, 288, 923, 804, 378, 215, 828, 592, 281, 565, 555, 710, 82, 896, 831, 547, 261, 524, 462, 293, 465,
    502, 56, 661, 821, 976, 991, 658, 869, 905, 758, 745, 193, 768, 550, 608, 933, 378, 286, 215, 979, 792, 961, 61,
    688, 793, 644, 986, 403, 106, 366, 905, 644, 372, 567, 466, 434, 645, 210, 389, 550, 919, 135, 780, 773, 635, 389,
    707, 100, 626, 958, 165, 504, 920, 176, 193, 713, 857, 265, 203, 50, 668, 108, 645, 990, 626, 197, 510, 357, 358,
    850, 858, 364, 936, 638,
];

struct Randomiser {
    to_go: u16,
    index: usize,
}

impl Randomiser {
    spec fn wf(&self) -> bool { // @ghost-begin
        self.index < 512
    } // @ghost-end

    const fn new() -> (r: Self)
        ensures // @ghost
            r.wf(), // @ghost
    {
        Self { to_go: 0, index: 0 }
    }

    fn next(&mut self) -> u8
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
    {
        if self.to_go == 0 {
            self.to_go = RAND_NUMS[self.index % 512];
            self.index = (self.index + 1) % 512;
        }
        self.to_go = self.to_go.wrapping_sub(1);
        if self.to_go == 1 {
            1
        } else {
            0
        }
    }
}

// ---------------------------------------------------------------------------
// Bit input, most significant bit first

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u64,
    n: u32,
}

impl<'a> Bits<'a> {
    // @ghost-begin
    /// At most 64 bits are buffered, and they came from bytes already
    /// loaded.
    spec fn wf(&self) -> bool {
        &&& self.pos <= self.data@.len()
        &&& self.n <= 64
        &&& self.n <= 8 * self.pos
    }

    /// The bits not yet read: what makes the decoder's loops end.
    spec fn remaining(&self) -> nat {
        (8 * (self.data@.len() - self.pos) + self.n) as nat
    }
    // @ghost-end

    const fn new(data: &'a [u8]) -> (r: Self)
        ensures // @ghost
            r.wf(), // @ghost
            r.data == data, // @ghost
    {
        Self { data, pos: 0, buf: 0, n: 0 }
    }

    fn refill(&mut self)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).data == old(self).data, // @ghost
            final(self).remaining() == old(self).remaining(), // @ghost
    {
        while self.n <= 56 && self.pos < self.data.len()
            invariant // @ghost
                self.wf(), // @ghost
                self.data == old(self).data, // @ghost
                self.remaining() == old(self).remaining(), // @ghost
            decreases self.data@.len() - self.pos, // @ghost
        {
            self.buf = self.buf | ((self.data[self.pos] as u64) << (56 - self.n));
            self.pos += 1;
            self.n += 8;
        }
    }

    fn read(&mut self, k: u32) -> (r: Result<u32, Bzip2Error>)
        requires // @ghost
            old(self).wf(), // @ghost
            1 <= k <= 32, // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).data == old(self).data, // @ghost
            final(self).remaining() <= old(self).remaining(), // @ghost
            r is Ok ==> final(self).remaining() + k == old(self).remaining(), // @ghost
            r is Ok ==> (r->Ok_0 as nat) < p2(k as nat), // @ghost
    {
        if self.n < k {
            self.refill();
            if self.n < k {
                return Err(Bzip2Error::Truncated);
            }
        }
        proof { // @ghost-begin
            let b = self.buf;
            assert(b >> (64 - k) < (1u64 << k)) by (bit_vector)
                requires
                    1 <= k <= 32,
            ;
            assert(b >> (64 - k) <= 0xffff_ffffu64) by (bit_vector)
                requires
                    1 <= k <= 32,
            ;
            lemma_shl(k);
        } // @ghost-end
        let v = (self.buf >> (64 - k)) as u32;
        self.buf = self.buf << k;
        self.n -= k;
        Ok(v)
    }

    fn bit(&mut self) -> (r: Result<bool, Bzip2Error>)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).data == old(self).data, // @ghost
            final(self).remaining() <= old(self).remaining(), // @ghost
            r is Ok ==> final(self).remaining() + 1 == old(self).remaining(), // @ghost
    {
        let b = self.read(1)?;
        Ok(b == 1)
    }

    fn byte(&mut self) -> (r: Result<u8, Bzip2Error>)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).data == old(self).data, // @ghost
            final(self).remaining() <= old(self).remaining(), // @ghost
            r is Ok ==> final(self).remaining() + 8 == old(self).remaining(), // @ghost
    {
        let b = self.read(8)?;
        Ok(b as u8)
    }

    const fn consumed(&self) -> (r: usize)
        requires // @ghost
            self.wf(), // @ghost
        ensures // @ghost
            r <= self.data@.len(), // @ghost
    {
        proof { // @ghost-begin
            assert(self.n / 8 <= self.pos) by (nonlinear_arith)
                requires
                    self.n <= 8 * self.pos,
            ;
        } // @ghost-end
        self.pos - (self.n / 8) as usize
    }
}

// ---------------------------------------------------------------------------
// Huffman decoding, as libbz2 does it

const FAST_BITS: u32 = 10;

struct DecodeTable {
    limit: [i32; MAX_CODE_LEN + 2],
    base: [i32; MAX_CODE_LEN + 2],
    perm: [u16; MAX_ALPHA],
    min_len: u32,
    fast: [u16; 1 << FAST_BITS],
}

impl DecodeTable {
    // @ghost-begin
    /// The bounds that keep decoding in range: code lengths of 1–20,
    /// limits and bases small enough that no subtraction overflows, and
    /// lookup entries that consume at most `FAST_BITS` bits.
    spec fn wf_shape(&self) -> bool {
        &&& 1 <= self.min_len <= 20
        &&& forall|k: int| 0 <= k < 22 ==> -1 <= #[trigger] self.limit@[k] <= 600_000_000
        &&& forall|k: int| 0 <= k < 22 ==> -6000 <= #[trigger] self.base@[k] <= 1_300_000_000
        &&& forall|c: int| 0 <= c < 1024 ==> fast_entry_ok(#[trigger] self.fast@[c])
    }

    spec fn perm_below(&self, m: nat) -> bool {
        forall|k: int| 0 <= k < 258 ==> ((#[trigger] self.perm@[k]) as nat) < m
    }

    spec fn fast_below(&self, m: nat) -> bool {
        forall|c: int| 0 <= c < 1024 ==> (#[trigger] self.fast@[c]) == 0 || ((self.fast@[c] & 0x1ff) as nat) < m
    }

    /// Every symbol the table can produce is below `m`.
    spec fn syms_below(&self, m: nat) -> bool {
        self.perm_below(m) && self.fast_below(m)
    }

    /// `s` is a symbol the table can produce.
    spec fn yields(&self, s: u16) -> bool {
        ||| exists|k: int| 0 <= k < 258 && (#[trigger] self.perm@[k]) == s
        ||| exists|c: int| 0 <= c < 1024 && (#[trigger] self.fast@[c]) != 0 && (self.fast@[c] & 0x1ff) == s
    }
    // @ghost-end

    #[verifier::loop_isolation(false)] // @ghost
    #[verifier::spinoff_prover] // @ghost
    fn new(lengths: &[u8]) -> (r: Self)
        requires // @ghost
            1 <= lengths@.len() <= 258, // @ghost
            forall|k: int| 0 <= k < lengths@.len() ==> 1 <= #[trigger] lengths@[k] <= 20, // @ghost
        ensures // @ghost
            r.wf_shape(), // @ghost
            r.syms_below(lengths@.len()), // @ghost
    {
        let mut min_len = 32u8;
        let mut max_len = 0u8;
        let mut i = 0;
        while i < lengths.len()
            invariant // @ghost
                i <= lengths@.len(), // @ghost
                i == 0 ==> min_len == 32 && max_len == 0, // @ghost
                i > 0 ==> 1 <= min_len <= max_len <= 20, // @ghost
            decreases lengths@.len() - i, // @ghost
        {
            if lengths[i] < min_len {
                min_len = lengths[i];
            }
            if lengths[i] > max_len {
                max_len = lengths[i];
            }
            i += 1;
        }
        // The symbols, by length then by value.
        let mut perm = [0u16; MAX_ALPHA];
        let mut pp = 0;
        let mut len = min_len;
        while len <= max_len
            invariant // @ghost
                1 <= min_len <= max_len <= 20, // @ghost
                min_len <= len <= max_len + 1, // @ghost
                pp <= 258, // @ghost
                forall|k: int| 0 <= k < 258 ==> ((#[trigger] perm@[k]) as int) < lengths@.len(), // @ghost
            decreases max_len + 1 - len, // @ghost
        {
            let mut sym = 0;
            while sym < lengths.len()
                invariant // @ghost
                    len <= max_len <= 20, // @ghost
                    sym <= lengths@.len(), // @ghost
                    pp <= 258, // @ghost
                    forall|k: int| 0 <= k < 258 ==> ((#[trigger] perm@[k]) as int) < lengths@.len(), // @ghost
                decreases lengths@.len() - sym, // @ghost
            {
                if lengths[sym] == len && pp < MAX_ALPHA {
                    perm[pp] = sym as u16;
                    pp += 1;
                }
                sym += 1;
            }
            len += 1;
        }
        // base[l + 1] counts the codes of length l, then accumulates.
        let mut base = [0i32; MAX_CODE_LEN + 2];
        let mut i = 0;
        while i < lengths.len()
            invariant // @ghost
                i <= lengths@.len(), // @ghost
                forall|k: int| 0 <= k < 22 ==> 0 <= #[trigger] base@[k] <= i, // @ghost
            decreases lengths@.len() - i, // @ghost
        {
            let l = lengths[i] as usize;
            base[l + 1] += 1;
            i += 1;
        }
        let mut i = 1;
        while i < MAX_CODE_LEN + 2
            invariant // @ghost
                1 <= i <= 22, // @ghost
                forall|j: int| 1 <= j < i ==> 0 <= (#[trigger] base@[j]) - base@[j - 1] <= 258, // @ghost
                forall|j: int| 0 <= j < i ==> 0 <= #[trigger] base@[j] <= 258 * (j + 1), // @ghost
                forall|j: int| i <= j < 22 ==> 0 <= #[trigger] base@[j] <= 258, // @ghost
            decreases 22 - i, // @ghost
        {
            base[i] += base[i - 1];
            i += 1;
        }
        let mut limit = [0i32; MAX_CODE_LEN + 2];
        let mut vec = 0i32;
        let mut i = min_len as usize;
        proof { // @ghost-begin
            lemma_p2_small();
            assert(p2(0) == 1);
        } // @ghost-end
        while i <= max_len as usize
            invariant // @ghost
                1 <= min_len <= max_len <= 20, // @ghost
                min_len <= i <= max_len + 1, // @ghost
                0 <= vec <= 516 * p2((i - min_len) as nat) - 516, // @ghost
                forall|k: int| 0 <= k < 22 ==> -1 <= #[trigger] limit@[k] <= 600_000_000, // @ghost
            decreases max_len + 1 - i, // @ghost
        {
            proof { // @ghost-begin
                lemma_p2_mono((i - min_len) as nat, 20);
                lemma_p2_small();
                assert(p2((i + 1 - min_len) as nat) == 2 * p2((i - min_len) as nat));
                assert(0 <= base@[i as int + 1] - base@[i as int] <= 258);
            } // @ghost-end
            vec += base[i + 1] - base[i];
            limit[i] = vec - 1;
            vec = 2 * vec;
            i += 1;
        }
        let mut i = min_len as usize + 1;
        while i <= max_len as usize
            invariant // @ghost
                min_len + 1 <= i <= max_len + 1, // @ghost
                max_len <= 20, // @ghost
                forall|j: int| i <= j < 22 ==> 0 <= #[trigger] base@[j] <= 5676, // @ghost
                forall|j: int| 0 <= j < 22 ==> -6000 <= #[trigger] base@[j] <= 1_300_000_000, // @ghost
            decreases max_len + 1 - i, // @ghost
        {
            proof { // @ghost-begin
                assert(-1 <= limit@[i as int - 1] <= 600_000_000);
                assert(0 <= base@[i as int] <= 5676);
            } // @ghost-end
            base[i] = 2 * (limit[i - 1] + 1) - base[i];
            i += 1;
        }
        let mut t = Self { limit, base, perm, min_len: min_len as u32, fast: [0; 1 << FAST_BITS] };
        let mut code = 0u32;
        proof { // @ghost-begin
            assert((1u32 << 10u32) == 1024u32) by (bit_vector);
            assert(forall|c: int| 0 <= c < 1024 ==> (#[trigger] t.fast@[c]) == 0);
        } // @ghost-end
        while code < 1 << FAST_BITS
            invariant // @ghost
                code <= 1024, // @ghost
                t.limit == limit, // @ghost
                t.base == base, // @ghost
                t.perm == perm, // @ghost
                t.min_len == min_len as u32, // @ghost
                forall|c: int| 0 <= c < 1024 ==> fast_entry_ok(#[trigger] t.fast@[c]), // @ghost
                t.fast_below(lengths@.len()), // @ghost
            decreases 1024 - code, // @ghost
        {
            let mut zn = t.min_len;
            while zn <= FAST_BITS
                invariant // @ghost
                    code < 1024, // @ghost
                    1 <= zn <= 21, // @ghost
                    t.limit == limit, // @ghost
                    t.base == base, // @ghost
                    t.perm == perm, // @ghost
                    t.min_len == min_len as u32, // @ghost
                    forall|c: int| 0 <= c < 1024 ==> fast_entry_ok(#[trigger] t.fast@[c]), // @ghost
                    t.fast_below(lengths@.len()), // @ghost
                decreases 21 - zn, // @ghost
            {
                proof { // @ghost-begin
                    let sh: u32 = (FAST_BITS - zn) as u32;
                    assert(code >> sh <= code) by (bit_vector);
                } // @ghost-end
                let zvec = (code >> (FAST_BITS - zn)) as i32;
                if zvec <= t.limit[zn as usize] {
                    if let Some(sym) = t.symbol(zn, zvec) {
                        proof { // @ghost-begin
                            assert(t.perm_below(lengths@.len()));
                            assert(t.syms_below(lengths@.len()));
                            lemma_yield_below(t, sym, lengths@.len());
                            lemma_fast_entry(zn, sym);
                        } // @ghost-end
                        t.fast[code as usize] = ((zn << 9) as u16) | sym;
                    }
                    break;
                }
                zn += 1;
            }
            code += 1;
        }
        proof { // @ghost-begin
            assert(t.perm_below(lengths@.len()));
            assert(t.wf_shape());
        } // @ghost-end
        t
    }

    fn symbol(&self, zn: u32, zvec: i32) -> (r: Option<u16>)
        requires // @ghost
            forall|k: int| 0 <= k < 22 ==> -6000 <= #[trigger] self.base@[k] <= 1_300_000_000, // @ghost
            zn < 22, // @ghost
            0 <= zvec < 2_097_152, // @ghost
        ensures // @ghost
            r is Some ==> self.yields(r->Some_0), // @ghost
    {
        let i = zvec - self.base[zn as usize];
        if 0 <= i && i < MAX_ALPHA as i32 {
            Some(self.perm[i as usize])
        } else {
            None
        }
    }

    fn decode(&self, bits: &mut Bits<'_>) -> (r: Result<u16, Bzip2Error>)
        requires // @ghost
            self.wf_shape(), // @ghost
            old(bits).wf(), // @ghost
        ensures // @ghost
            final(bits).wf(), // @ghost
            final(bits).data == old(bits).data, // @ghost
            final(bits).remaining() <= old(bits).remaining(), // @ghost
            r is Ok ==> self.yields(r->Ok_0), // @ghost
    {
        if bits.n < FAST_BITS {
            bits.refill();
        }
        if bits.n >= FAST_BITS {
            let ghost idx = (bits.buf >> 54u32) as int; // @ghost
            proof { // @ghost-begin
                let b = bits.buf;
                assert(b >> 54u32 < 1024u64) by (bit_vector);
                assert(64 - FAST_BITS == 54u32);
            } // @ghost-end
            let e = self.fast[(bits.buf >> (64 - FAST_BITS)) as usize];
            if e != 0 {
                proof { // @ghost-begin
                    assert(e == self.fast@[idx]);
                    assert(fast_entry_ok(self.fast@[idx]));
                } // @ghost-end
                let len = (e >> 9) as u32;
                bits.buf = bits.buf << len;
                bits.n -= len;
                return Ok(e & 0x1ff);
            }
        }
        self.decode_slow(bits)
    }

    fn decode_slow(&self, bits: &mut Bits<'_>) -> (r: Result<u16, Bzip2Error>)
        requires // @ghost
            self.wf_shape(), // @ghost
            old(bits).wf(), // @ghost
        ensures // @ghost
            final(bits).wf(), // @ghost
            final(bits).data == old(bits).data, // @ghost
            final(bits).remaining() <= old(bits).remaining(), // @ghost
            r is Ok ==> self.yields(r->Ok_0), // @ghost
    {
        let mut zn = self.min_len;
        let mut zvec = bits.read(zn)? as i32;
        proof { // @ghost-begin
            lemma_p2_small();
            lemma_p2_mono(zn as nat, 20);
        } // @ghost-end
        loop
            invariant // @ghost
                self.wf_shape(), // @ghost
                1 <= zn <= 21, // @ghost
                0 <= zvec, // @ghost
                (zvec as nat) < p2(zn as nat), // @ghost
                bits.wf(), // @ghost
                bits.data == old(bits).data, // @ghost
                bits.remaining() <= old(bits).remaining(), // @ghost
            ensures // @ghost
                zn <= 20, // @ghost
            decreases 22 - zn, // @ghost
        {
            if zn > MAX_CODE_LEN as u32 {
                return Err(Bzip2Error::Corrupt("invalid Huffman code"));
            }
            if zvec <= self.limit[zn as usize] {
                break;
            }
            proof { // @ghost-begin
                lemma_p2_small();
                lemma_p2_mono(zn as nat, 20);
                assert(p2((zn + 1) as nat) == 2 * p2(zn as nat));
            } // @ghost-end
            zn += 1;
            let b = bits.read(1)?;
            zvec = 2 * zvec + b as i32;
        }
        proof { // @ghost-begin
            lemma_p2_small();
            lemma_p2_mono(zn as nat, 20);
        } // @ghost-end
        match self.symbol(zn, zvec) {
            Some(sym) => Ok(sym),
            None => Err(Bzip2Error::Corrupt("invalid Huffman code")),
        }
    }
}

fn next_symbol(
    bits: &mut Bits<'_>,
    tables: &[DecodeTable],
    selectors: &[u8],
    group: &mut usize,
    left: &mut usize,
) -> (r: Result<u16, Bzip2Error>)
    requires // @ghost
        old(bits).wf(), // @ghost
        all_shaped(tables@), // @ghost
        forall|k: int| 0 <= k < selectors@.len() ==> ((#[trigger] selectors@[k]) as int) < tables@.len(), // @ghost
        *old(group) <= selectors@.len(), // @ghost
        *old(left) > 0 ==> *old(group) >= 1, // @ghost
    ensures // @ghost
        final(bits).wf(), // @ghost
        final(bits).data == old(bits).data, // @ghost
        final(bits).remaining() <= old(bits).remaining(), // @ghost
        *final(group) <= selectors@.len(), // @ghost
        *final(left) > 0 ==> *final(group) >= 1, // @ghost
        r is Ok ==> sym_from(tables@, r->Ok_0), // @ghost
{
    if *left == 0 {
        if *group >= selectors.len() {
            return Err(Bzip2Error::Corrupt("ran out of selectors"));
        }
        *group += 1;
        *left = GROUP_SIZE;
    }
    *left -= 1;
    proof { // @ghost-begin
        let s = selectors@[*group - 1] as int;
        assert(tables@[s].wf_shape());
    } // @ghost-end
    tables[selectors[*group - 1] as usize].decode(bits)
}

// ---------------------------------------------------------------------------
// Decompression

/// A decoded bzip2 stream.
pub struct Bzip2Stream {
    /// The decompressed data.
    pub data: Vec<u8>,
    /// Bytes of input the stream used, through its final padded byte.
    pub consumed: usize,
    /// The block size digit from the header, 1–9 (blocks of up to
    /// 100,000 × this many bytes).
    pub level: u8,
}

fn magic_prefix(data: &[u8]) -> bool {
    (data.len() < 1 || data[0] == b'B') && (data.len() < 2 || data[1] == b'Z') && (data.len() < 3 || data[2] == b'h')
}

fn stream_level(input: &[u8]) -> (r: Result<u8, Bzip2Error>)
    ensures // @ghost
        r is Ok ==> 1 <= r->Ok_0 <= 9 && input@.len() >= 4, // @ghost
{
    if !magic_prefix(input) {
        return Err(Bzip2Error::NotBzip2);
    }
    if input.len() < 4 {
        return Err(Bzip2Error::Truncated);
    }
    if input[3] < b'1' || input[3] > b'9' {
        return Err(Bzip2Error::NotBzip2);
    }
    Ok(input[3] - b'0')
}

fn block_magic(bits: &mut Bits<'_>) -> (r: Result<bool, Bzip2Error>)
    requires // @ghost
        old(bits).wf(), // @ghost
    ensures // @ghost
        final(bits).wf(), // @ghost
        final(bits).data == old(bits).data, // @ghost
        final(bits).remaining() <= old(bits).remaining(), // @ghost
        r is Ok ==> final(bits).remaining() < old(bits).remaining(), // @ghost
{
    let first = bits.byte()?;
    let end = if first == 0x17 {
        true
    } else if first == 0x31 {
        false
    } else {
        return Err(Bzip2Error::Corrupt("bad block header"));
    };
    let magic = if end { END_MAGIC } else { BLOCK_MAGIC };
    let mut shift = 5u32;
    while shift > 0
        invariant // @ghost
            shift <= 5, // @ghost
            bits.wf(), // @ghost
            bits.data == old(bits).data, // @ghost
            bits.remaining() < old(bits).remaining(), // @ghost
        decreases shift, // @ghost
    {
        shift -= 1;
        let b = bits.byte()?;
        if b != (magic >> (8 * shift)) as u8 {
            return Err(Bzip2Error::Corrupt("bad block header"));
        }
    }
    Ok(end)
}

pub fn read_stream(input: &[u8], max_output: usize) -> (r: Result<Bzip2Stream, Bzip2Error>)
    ensures // @ghost
        r is Ok ==> r->Ok_0.data@.len() <= max_output, // @ghost
        r is Ok ==> 4 <= r->Ok_0.consumed <= input@.len(), // @ghost
{
    let level = stream_level(input)?;
    let mut bits = Bits::new(&input[4..]);
    let mut out = Vec::new();
    let mut stream_crc = 0u32;
    let mut tt = Vec::new();
    loop
        invariant // @ghost
            1 <= level <= 9, // @ghost
            input@.len() >= 4, // @ghost
            bits.wf(), // @ghost
            bits.data@.len() == input@.len() - 4, // @ghost
            out@.len() <= max_output, // @ghost
        decreases bits.remaining(), // @ghost
    {
        let end = block_magic(&mut bits)?;
        let crc = bits.read(32)?;
        if end {
            if crc != stream_crc {
                return Err(Bzip2Error::StreamCrcMismatch { expected: crc, actual: stream_crc });
            }
            return Ok(Bzip2Stream { data: out, consumed: 4 + bits.consumed(), level });
        }
        let actual = read_block(&mut bits, level, &mut tt, &mut out, max_output)?;
        if actual != crc {
            return Err(Bzip2Error::BlockCrcMismatch { expected: crc, actual });
        }
        stream_crc = combine(stream_crc, actual);
    }
}

#[verifier::loop_isolation(false)] // @ghost
#[verifier::allow_complex_invariants] // @ghost
#[verifier::spinoff_prover] // @ghost
fn read_block(
    bits: &mut Bits<'_>,
    level: u8,
    tt: &mut Vec<u32>,
    out: &mut Vec<u8>,
    max_output: usize,
) -> (r: Result<u32, Bzip2Error>)
    requires // @ghost
        old(bits).wf(), // @ghost
        1 <= level <= 9, // @ghost
        old(out)@.len() <= max_output, // @ghost
    ensures // @ghost
        final(bits).wf(), // @ghost
        final(bits).data == old(bits).data, // @ghost
        final(bits).remaining() <= old(bits).remaining(), // @ghost
        final(out)@.len() <= max_output, // @ghost
{
    let randomised = bits.bit()?;
    let orig_ptr = bits.read(24)? as usize;
    let block_max = 100_000 * level as usize;
    if orig_ptr > 10 + block_max {
        return Err(Bzip2Error::Corrupt("origin pointer out of range"));
    }

    // Which byte values occur: 16 bits for groups of 16, then 16 per
    // group that is used.
    let used16 = bits.read(16)?;
    let mut seq_to_unseq = [0u8; 256];
    let mut n_in_use = 0usize;
    let mut i = 0usize;
    while i < 16
        invariant // @ghost
            i <= 16, // @ghost
            n_in_use <= 16 * i, // @ghost
            bits.wf(), // @ghost
            bits.data == old(bits).data, // @ghost
            bits.remaining() <= old(bits).remaining(), // @ghost
        decreases 16 - i, // @ghost
    {
        if used16 & (0x8000 >> i) != 0 {
            let used = bits.read(16)?;
            let mut j = 0usize;
            while j < 16
                invariant // @ghost
                    i < 16, // @ghost
                    j <= 16, // @ghost
                    n_in_use <= 16 * i + j, // @ghost
                    bits.wf(), // @ghost
                    bits.data == old(bits).data, // @ghost
                    bits.remaining() <= old(bits).remaining(), // @ghost
                decreases 16 - j, // @ghost
            {
                if used & (0x8000 >> j) != 0 {
                    seq_to_unseq[n_in_use] = (i * 16 + j) as u8;
                    n_in_use += 1;
                }
                j += 1;
            }
        }
        i += 1;
    }
    if n_in_use == 0 {
        return Err(Bzip2Error::Corrupt("no byte values in use"));
    }
    let alpha_size = n_in_use + 2;

    // The selectors, move-to-front coded in unary.
    let n_groups = bits.read(3)? as usize;
    if n_groups < 2 || n_groups > MAX_GROUPS {
        return Err(Bzip2Error::Corrupt("bad number of Huffman tables"));
    }
    let n_selectors = bits.read(15)? as usize;
    if n_selectors == 0 {
        return Err(Bzip2Error::Corrupt("no selectors"));
    }
    let mut selectors: Vec<u8> = Vec::new();
    let mut pos: [u8; MAX_GROUPS] = [0, 1, 2, 3, 4, 5];
    proof { // @ghost-begin
        assert(pos@[0] == 0 && pos@[1] == 1 && pos@[2] == 2 && pos@[3] == 3 && pos@[4] == 4 && pos@[5] == 5);
    } // @ghost-end
    let mut i = 0usize;
    while i < n_selectors
        invariant // @ghost
            i <= n_selectors, // @ghost
            forall|k: int| 0 <= k < n_groups ==> ((#[trigger] pos@[k]) as int) < n_groups, // @ghost
            forall|k: int| 0 <= k < selectors@.len() ==> ((#[trigger] selectors@[k]) as int) < n_groups, // @ghost
            bits.wf(), // @ghost
            bits.data == old(bits).data, // @ghost
            bits.remaining() <= old(bits).remaining(), // @ghost
        decreases n_selectors - i, // @ghost
    {
        let mut j = 0usize;
        loop
            invariant // @ghost
                j < n_groups, // @ghost
                bits.wf(), // @ghost
                bits.data == old(bits).data, // @ghost
                bits.remaining() <= old(bits).remaining(), // @ghost
            decreases n_groups - j, // @ghost
        {
            if !bits.bit()? {
                break;
            }
            j += 1;
            if j >= n_groups {
                return Err(Bzip2Error::Corrupt("bad selector"));
            }
        }
        if i < MAX_SELECTORS {
            let v = pos[j];
            let mut k = j;
            while k > 0
                invariant // @ghost
                    k <= j, // @ghost
                    (v as int) < n_groups, // @ghost
                    forall|q: int| 0 <= q < n_groups ==> ((#[trigger] pos@[q]) as int) < n_groups, // @ghost
                decreases k, // @ghost
            {
                pos[k] = pos[k - 1];
                k -= 1;
            }
            pos[0] = v;
            selectors.push(v);
        }
        i += 1;
    }

    // The code lengths, delta coded.
    let mut tables: Vec<DecodeTable> = Vec::new();
    let mut lengths = [0u8; MAX_ALPHA];
    let mut t = 0usize;
    while t < n_groups
        invariant // @ghost
            t <= n_groups, // @ghost
            tables@.len() == t, // @ghost
            all_shaped(tables@), // @ghost
            all_below(tables@, alpha_size as nat), // @ghost
            bits.wf(), // @ghost
            bits.data == old(bits).data, // @ghost
            bits.remaining() <= old(bits).remaining(), // @ghost
        decreases n_groups - t, // @ghost
    {
        let mut curr = bits.read(5)? as i32;
        let mut s = 0usize;
        while s < alpha_size
            invariant // @ghost
                s <= alpha_size, // @ghost
                forall|k: int| 0 <= k < s ==> 1 <= #[trigger] lengths@[k] <= 20, // @ghost
                bits.wf(), // @ghost
                bits.data == old(bits).data, // @ghost
                bits.remaining() <= old(bits).remaining(), // @ghost
            decreases alpha_size - s, // @ghost
        {
            loop
                invariant // @ghost
                    bits.wf(), // @ghost
                    bits.data == old(bits).data, // @ghost
                    bits.remaining() <= old(bits).remaining(), // @ghost
                ensures // @ghost
                    1 <= curr <= 20, // @ghost
                decreases bits.remaining(), // @ghost
            {
                if curr < 1 || curr > MAX_CODE_LEN as i32 {
                    return Err(Bzip2Error::Corrupt("bad code length"));
                }
                if !bits.bit()? {
                    break;
                }
                if bits.bit()? {
                    curr -= 1;
                } else {
                    curr += 1;
                }
            }
            lengths[s] = curr as u8;
            s += 1;
        }
        let ghost before = tables@; // @ghost
        tables.push(DecodeTable::new(&lengths[..alpha_size]));
        proof { // @ghost-begin
            assert(forall|k: int| 0 <= k < before.len() ==> tables@[k] == before[k]);
            assert(all_shaped(tables@));
            assert(all_below(tables@, alpha_size as nat));
        } // @ghost-end
        t += 1;
    }

    // The move-to-front values, with runs of the front value in RUNA and
    // RUNB, into `tt` as bytes.
    let eob = n_in_use as u16 + 1;
    let mut mtf = [0u8; 256];
    let mut i = 0usize;
    while i < 256
        invariant // @ghost
            i <= 256, // @ghost
        decreases 256 - i, // @ghost
    {
        mtf[i] = i as u8;
        i += 1;
    }
    let mut counts = [0u32; 256];
    tt.clear();
    let mut group = 0usize;
    let mut left = 0usize;
    proof { // @ghost-begin
        assert(forall|c: int| 0 <= c < 256 ==> (#[trigger] counts@[c]) == 0);
    } // @ghost-end
    let mut sym = next_symbol(bits, tables.as_slice(), selectors.as_slice(), &mut group, &mut left)?;
    proof { // @ghost-begin
        lemma_below(tables@, sym, alpha_size as nat);
    } // @ghost-end
    loop
        invariant // @ghost
            (sym as int) < alpha_size, // @ghost
            tt@.len() <= block_max, // @ghost
            forall|c: int| 0 <= c < 256 ==> #[trigger] counts@[c] <= tt@.len(), // @ghost
            group <= selectors@.len(), // @ghost
            left > 0 ==> group >= 1, // @ghost
            bits.wf(), // @ghost
            bits.data == old(bits).data, // @ghost
            bits.remaining() <= old(bits).remaining(), // @ghost
        decreases block_max - tt@.len(), // @ghost
    {
        if sym == eob {
            break;
        }
        if sym == RUNA || sym == RUNB {
            let mut run = 0usize;
            let mut weight = 1usize;
            while sym == RUNA || sym == RUNB
                invariant // @ghost
                    1 <= weight <= 4 * 1024 * 1024, // @ghost
                    weight - 1 <= run <= 2 * weight - 2, // @ghost
                    weight == 1 ==> (sym == RUNA || sym == RUNB), // @ghost
                    (sym as int) < alpha_size, // @ghost
                    group <= selectors@.len(), // @ghost
                    left > 0 ==> group >= 1, // @ghost
                    bits.wf(), // @ghost
                    bits.data == old(bits).data, // @ghost
                    bits.remaining() <= old(bits).remaining(), // @ghost
                decreases 4 * 1024 * 1024 - weight, // @ghost
            {
                if weight >= 2 * 1024 * 1024 {
                    return Err(Bzip2Error::Corrupt("run too long"));
                }
                run += if sym == RUNB { 2 * weight } else { weight };
                weight = 2 * weight;
                sym = next_symbol(bits, tables.as_slice(), selectors.as_slice(), &mut group, &mut left)?;
                proof { // @ghost-begin
                    lemma_below(tables@, sym, alpha_size as nat);
                } // @ghost-end
            }
            if run > block_max - tt.len() {
                return Err(Bzip2Error::Corrupt("block too long"));
            }
            let b = seq_to_unseq[mtf[0] as usize];
            counts[b as usize] += run as u32;
            tt.resize(tt.len() + run, b as u32);
        } else {
            if tt.len() >= block_max {
                return Err(Bzip2Error::Corrupt("block too long"));
            }
            let nn = (sym - 1) as usize;
            let v = mtf[nn];
            let mut k = nn;
            while k > 0
                invariant // @ghost
                    k <= nn < 256, // @ghost
                decreases k, // @ghost
            {
                mtf[k] = mtf[k - 1];
                k -= 1;
            }
            mtf[0] = v;
            let b = seq_to_unseq[v as usize];
            counts[b as usize] += 1;
            tt.push(b as u32);
            sym = next_symbol(bits, tables.as_slice(), selectors.as_slice(), &mut group, &mut left)?;
            proof { // @ghost-begin
                lemma_below(tables@, sym, alpha_size as nat);
            } // @ghost-end
        }
    }
    if orig_ptr >= tt.len() {
        return Err(Bzip2Error::Corrupt("origin pointer out of range"));
    }
    let block = inverse_bwt(tt, &counts, orig_ptr, randomised)?;
    let start = out.len();
    rle1_decode(block.as_slice(), out, max_output)?;
    Ok(block_crc(&out.as_slice()[start..]))
}

fn inverse_bwt(tt: &mut Vec<u32>, counts: &[u32; 256], orig_ptr: usize, randomised: bool) -> Result<Vec<u8>, Bzip2Error> {
    let n = tt.len();
    // Where each byte's rows start in the sorted first column.
    let mut start = [0u32; 256];
    let mut sum = 0u32;
    let mut c = 0usize;
    while c < 256
        invariant // @ghost
            c <= 256, // @ghost
        decreases 256 - c, // @ghost
    {
        start[c] = sum;
        sum = sum.wrapping_add(counts[c]);
        c += 1;
    }
    let mut i = 0usize;
    while i < n
        invariant // @ghost
            i <= n, // @ghost
            tt@.len() == n, // @ghost
        decreases n - i, // @ghost
    {
        proof { // @ghost-begin
            let x = tt@[i as int];
            assert(x & 0xff < 256) by (bit_vector);
        } // @ghost-end
        let b = (tt[i] & 0xff) as usize;
        let row = start[b] as usize;
        // The counts are exact, so each byte's rows stay in range.
        if row >= n {
            return Err(Bzip2Error::Corrupt("byte counts do not match the block"));
        }
        tt[row] = tt[row] | ((i as u32) << 8);
        start[b] = start[b].wrapping_add(1);
        i += 1;
    }
    let mut block = Vec::with_capacity(n);
    let mut rand = Randomiser::new();
    let mut pos = orig_ptr;
    let mut k = 0usize;
    while k < n
        invariant // @ghost
            k <= n, // @ghost
            tt@.len() == n, // @ghost
            rand.wf(), // @ghost
        decreases n - k, // @ghost
    {
        if pos >= n {
            return Err(Bzip2Error::Corrupt("broken row link"));
        }
        pos = (tt[pos] >> 8) as usize;
        if pos >= n {
            return Err(Bzip2Error::Corrupt("broken row link"));
        }
        let b = tt[pos] as u8;
        block.push(if randomised { b ^ rand.next() } else { b });
        k += 1;
    }
    Ok(block)
}

fn rle1_decode(block: &[u8], out: &mut Vec<u8>, max_output: usize) -> (r: Result<(), Bzip2Error>)
    requires // @ghost
        old(out)@.len() <= max_output, // @ghost
    ensures // @ghost
        old(out)@.len() <= final(out)@.len() <= max_output, // @ghost
{
    let mut last = 0u8;
    let mut run = 0u32;
    let mut i = 0usize;
    while i < block.len()
        invariant // @ghost
            i <= block@.len(), // @ghost
            run <= 4, // @ghost
            old(out)@.len() <= out@.len() <= max_output, // @ghost
        decreases block@.len() - i, // @ghost
    {
        let b = block[i];
        i += 1;
        if run == 4 {
            let extra = b as usize;
            if extra > max_output.saturating_sub(out.len()) {
                return Err(Bzip2Error::OutputLimitExceeded);
            }
            out.resize(out.len() + extra, last);
            run = 0;
        } else {
            if run > 0 && b == last {
                run += 1;
            } else {
                last = b;
                run = 1;
            }
            if out.len() >= max_output {
                return Err(Bzip2Error::OutputLimitExceeded);
            }
            out.push(b);
        }
    }
    if run == 4 {
        return Err(Bzip2Error::Corrupt("block ends inside a run"));
    }
    Ok(())
}

pub fn decompress_with_limit(input: &[u8], max_output: usize) -> (r: Result<Vec<u8>, Bzip2Error>)
    ensures // @ghost
        r is Ok ==> r->Ok_0@.len() <= max_output, // @ghost
{
    let mut out: Vec<u8> = Vec::new();
    let mut rest = input;
    loop
        invariant // @ghost
            out@.len() <= max_output, // @ghost
        decreases rest@.len(), // @ghost
    {
        let s = read_stream(rest, max_output - out.len())?;
        out.extend_from_slice(s.data.as_slice());
        rest = &rest[s.consumed..];
        if rest.is_empty() {
            return Ok(out);
        }
        if rest.len() < 3 || !magic_prefix(rest) {
            return Err(Bzip2Error::TrailingData);
        }
    }
}

} // verus!
