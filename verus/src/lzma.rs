//! The LZMA decoder (`src/lzma.rs`): `decompress_zip_entry`,
//! `decompress_raw` and `decode_stream`, with the range decoder and the
//! probability model.
//!
//! The executable code below is the library's, token for token (CI checks
//! this with `.github/scripts/verus_drift.py`); every line marked
//! `// @ghost` is specification or proof. The theorem: on every input of
//! every length, every set of properties and every output limit, the
//! decoder returns, without panicking (every index is in bounds, no
//! arithmetic overflows, every loop ends), and its output stays within
//! the limit. `decode_stream`, which LZMA2 runs chunk after chunk on one
//! model, also leaves the model fit for the next chunk. (That the output
//! is the stream's contents rests on the tests against liblzma and the
//! fuzzing; see VERIFICATION.md.)

use crate::huffman_decode::lemma_shl_usize_u32;
use crate::limit_depths::{lemma_p2_mono, lemma_p2_pos, lemma_shl, p2};
use vstd::prelude::*;

verus! {

// @ghost-begin
/// Every probability is at most 2048 (it starts at 1024, and both updates
/// keep it in 31..=2017), so the range arithmetic cannot overflow.
pub(crate) open spec fn probs_ok(s: Seq<u16>) -> bool {
    forall|i: int| 0 <= i < s.len() ==> #[trigger] s[i] <= 2048
}

/// The constants, worked out.
proof fn lemma_consts()
    ensures
        BIT_MODEL_TOTAL == 2048,
        PROB_INIT == 1024,
        MOVE_BITS == 5,
        ALIGN_BITS == 4,
        DIST_SLOT == 432,
        DIST_SPECIAL == 688,
        DIST_ALIGN == 803,
        LEN == 819,
        LEN_PROBS == 514,
        LEN_LOW == 2,
        LEN_MID == 130,
        LEN_HIGH == 258,
        REP_LEN == 1333,
        LITERAL == 1847,
        END_POS_MODEL_INDEX == 14,
{
    assert(BIT_MODEL_TOTAL == 2048) by (compute_only);
    assert(PROB_INIT == 1024) by (compute_only);
    assert(LEN == 819) by (compute_only);
    assert(DIST_ALIGN == 803) by (compute_only);
    assert(LEN_PROBS == 514) by (compute_only);
    assert(LITERAL == 1847) by (compute_only);
}

proof fn lemma_p2_small()
    ensures
        p2(0) == 1,
        p2(1) == 2,
        p2(4) == 16,
        p2(5) == 32,
        p2(6) == 64,
        p2(8) == 256,
        p2(12) == 4096,
        p2(26) == 67108864,
{
    reveal_with_fuel(p2, 27);
}

/// `1u32 << s` is 2^s.
proof fn lemma_shl_u32(s: u32)
    requires
        s < 32,
    ensures
        (1u32 << s) as nat == p2(s as nat),
{
    lemma_shl(s);
    assert((1u32 << s) as u64 == 1u64 << s) by (bit_vector)
        requires
            s < 32,
    ;
}

/// Appending a bit: `(m << 1) | b` is `2m + b`.
proof fn lemma_append_bit(m: usize, b: usize)
    requires
        m < 0x1000_0000,
        b <= 1,
    ensures
        (m << 1) | b == 2 * m + b,
{
    assert((m << 1) | b == 2 * m + b) by (bit_vector)
        requires
            m < 0x1000_0000,
            b <= 1,
    ;
}
// @ghost-end

/// The longest match.
pub const MATCH_LEN_MAX: usize = 273;
const MATCH_LEN_MIN: usize = 2;
const STATES: usize = 12;
const POS_STATES_MAX: usize = 16;
const DIST_STATES: usize = 4;
const DIST_SLOTS: usize = 64;
const END_POS_MODEL_INDEX: u32 = 14;
const FULL_DISTANCES: usize = 128;
const ALIGN_BITS: u32 = 4;
const BIT_MODEL_TOTAL: u32 = 1 << 11;
const MOVE_BITS: u32 = 5;
const PROB_INIT: u16 = (BIT_MODEL_TOTAL / 2) as u16;
const TOP: u32 = 1 << 24;

/// Why an LZMA stream could not be read.
pub enum LzmaError {
    /// The properties (`lc`, `lp`, `pb` or the dictionary size) are invalid.
    BadProperties,
    /// The data ended in the middle of the stream.
    Truncated,
    /// The stream is corrupt; the message says how.
    Corrupt(&'static str),
    /// Decompressing would exceed the output limit.
    OutputLimitExceeded,
    /// Bytes follow the end of the stream.
    TrailingData,
}

pub struct LzmaProps {
    pub lc: u8,
    pub lp: u8,
    pub pb: u8,
    pub dict_size: u32,
}

impl LzmaProps {
    // @ghost-begin
    pub open spec fn valid(&self) -> bool {
        self.lc <= 8 && self.lp <= 4 && self.pb <= 4
    }
    // @ghost-end

    pub const fn new(dict_size: u32) -> (r: Self)
        ensures // @ghost
            r.valid(), // @ghost
            r.lc + r.lp <= 4, // @ghost
    {
        Self { lc: 3, lp: 0, pb: 2, dict_size }
    }

    pub const fn from_byte(b: u8, dict_size: u32) -> (r: Result<Self, LzmaError>)
        ensures // @ghost
            r is Ok ==> r->Ok_0.valid(), // @ghost
    {
        if b >= 9 * 5 * 5 {
            return Err(LzmaError::BadProperties);
        }
        proof { // @ghost-begin
            assert(b / 45 <= 4) by (nonlinear_arith)
                requires
                    b < 225,
            ;
        } // @ghost-end
        Ok(Self { lc: b % 9, lp: (b / 9) % 5, pb: b / 45, dict_size })
    }

    pub fn parse(b: &[u8; 5]) -> (r: Result<Self, LzmaError>)
        ensures // @ghost
            r is Ok ==> r->Ok_0.valid(), // @ghost
    {
        Self::from_byte(b[0], b[1] as u32 | (b[2] as u32) << 8 | (b[3] as u32) << 16 | (b[4] as u32) << 24)
    }

    fn check(&self) -> (r: Result<(), LzmaError>)
        ensures // @ghost
            r is Ok ==> self.valid(), // @ghost
    {
        if self.lc > 8 || self.lp > 4 || self.pb > 4 {
            Err(LzmaError::BadProperties)
        } else {
            Ok(())
        }
    }

    fn effective_dict(&self) -> usize {
        let d = if self.dict_size < 4096 { 4096 } else { self.dict_size as usize };
        d.saturating_add(15) & !15
    }
}

// ---------------------------------------------------------------------------
// The probability model, shared by decoder and encoder

const IS_MATCH: usize = 0;
const IS_REP: usize = IS_MATCH + STATES * POS_STATES_MAX;
const IS_REP0: usize = IS_REP + STATES;
const IS_REP1: usize = IS_REP0 + STATES;
const IS_REP2: usize = IS_REP1 + STATES;
const IS_REP0_LONG: usize = IS_REP2 + STATES;
const DIST_SLOT: usize = IS_REP0_LONG + STATES * POS_STATES_MAX;
const DIST_SPECIAL: usize = DIST_SLOT + DIST_STATES * DIST_SLOTS;
const DIST_ALIGN: usize = DIST_SPECIAL + 1 + FULL_DISTANCES - END_POS_MODEL_INDEX as usize;
const LEN: usize = DIST_ALIGN + (1 << ALIGN_BITS);
const REP_LEN: usize = LEN + LEN_PROBS;
const LITERAL: usize = REP_LEN + LEN_PROBS;

const LEN_CHOICE: usize = 0;
const LEN_CHOICE2: usize = 1;
const LEN_LOW: usize = 2;
const LEN_MID: usize = LEN_LOW + POS_STATES_MAX * 8;
const LEN_HIGH: usize = LEN_MID + POS_STATES_MAX * 8;
const LEN_PROBS: usize = LEN_HIGH + 256;

pub(crate) struct Model {
    lc: u32,
    lp: u32,
    pb: u32,
    probs: Vec<u16>,
    state: usize,
    reps: [u32; 4],
}

impl Model {
    // @ghost-begin
    /// The table holds every group and 2^(lc + lp) literal contexts, every
    /// probability is in range, and the state is one of the twelve.
    pub(crate) closed spec fn wf(&self) -> bool {
        &&& self.lc <= 8
        &&& self.lp <= 4
        &&& self.pb <= 4
        &&& self.probs@.len() == LITERAL + 0x300 * p2((self.lc + self.lp) as nat)
        &&& probs_ok(self.probs@)
        &&& self.state < 12
    }

    /// The distances it remembers reach back no further than `pos`, the
    /// bytes decoded since the dictionary began (a distance of 0 is the
    /// fresh value, and needs one byte); after a match the last one is
    /// within `pos`.
    pub(crate) closed spec fn reps_ok(&self, pos: int) -> bool {
        &&& forall|i: int| 0 <= i < 4 ==> (#[trigger] self.reps@[i] as int) < pos || self.reps@[i] == 0
        &&& self.state >= 7 ==> (self.reps@[0] as int) < pos
    }

    /// The parts of the model decoding a distance leaves alone.
    closed spec fn same_but_probs(&self, other: &Model) -> bool {
        &&& self.lc == other.lc
        &&& self.lp == other.lp
        &&& self.pb == other.pb
        &&& self.state == other.state
        &&& self.reps == other.reps
        &&& self.probs@.len() == other.probs@.len()
    }
    // @ghost-end

    pub(crate) fn new(props: &LzmaProps) -> (r: Self)
        requires // @ghost
            props.valid(), // @ghost
        ensures // @ghost
            r.wf(), // @ghost
            r.reps_ok(0), // @ghost
    {
        proof { // @ghost-begin
            lemma_consts();
            let s: u8 = (props.lc + props.lp) as u8;
            assert(0x300usize << s == 0x300usize * (1usize << (s as u32))) by (bit_vector)
                requires
                    s <= 12,
            ;
            lemma_shl_usize_u32(s as u32);
        } // @ghost-end
        Self {
            lc: props.lc as u32,
            lp: props.lp as u32,
            pb: props.pb as u32,
            probs: vec![PROB_INIT; LITERAL + (0x300 << (props.lc + props.lp))],
            state: 0,
            reps: [0; 4],
        }
    }

    pub(crate) fn reset(&mut self, props: &LzmaProps)
        requires // @ghost
            props.valid(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).reps_ok(0), // @ghost
    {
        *self = Self::new(props);
    }

    fn literal_base(&self, pos: usize, prev: u8) -> (r: usize)
        requires // @ghost
            self.wf(), // @ghost
        ensures // @ghost
            LITERAL <= r, // @ghost
            r + 0x300 <= self.probs@.len(), // @ghost
    {
        proof { // @ghost-begin
            lemma_consts();
            let lc = self.lc;
            let lp = self.lp;
            let pv = prev as usize;
            let sh: u32 = (8 - lc) as u32;
            let s: u32 = (lc + lp) as u32;
            assert(((pos & ((1usize << lp) - 1)) << lc) + (pv >> sh) < (1usize << s)) by (bit_vector)
                requires
                    lc <= 8,
                    lp <= 4,
                    pv < 256,
                    sh == 8u32 - lc,
                    s == lc + lp,
            ;
            assert((1usize << lp) >= 1) by (bit_vector)
                requires
                    lp <= 4,
            ;
            lemma_shl_usize_u32(s);
        } // @ghost-end
        LITERAL + 0x300 * (((pos & ((1 << self.lp) - 1)) << self.lc) + ((prev as usize) >> (8 - self.lc)))
    }

    fn pos_state(&self, pos: usize) -> (r: usize)
        requires // @ghost
            self.wf(), // @ghost
        ensures // @ghost
            r < 16, // @ghost
    {
        proof { // @ghost-begin
            let pb = self.pb;
            assert((pos & ((1usize << pb) - 1)) < 16) by (bit_vector)
                requires
                    pb <= 4,
            ;
            assert((1usize << pb) >= 1) by (bit_vector)
                requires
                    pb <= 4,
            ;
        } // @ghost-end
        pos & ((1 << self.pb) - 1)
    }
}

// @ghost-begin
/// More bytes decoded keep the distances in reach.
pub(crate) proof fn lemma_reps_ok_grow(m: &Model, a: int, b: int)
    requires
        m.reps_ok(a),
        a <= b,
    ensures
        m.reps_ok(b),
{
}
// @ghost-end

const fn after_literal(s: usize) -> (r: usize)
    requires // @ghost
        s < 12, // @ghost
    ensures // @ghost
        r < 7, // @ghost
{
    if s < 4 {
        0
    } else if s < 10 {
        s - 3
    } else {
        s - 6
    }
}
const fn after_match(s: usize) -> (r: usize)
    ensures // @ghost
        7 <= r < 12, // @ghost
{
    if s < 7 {
        7
    } else {
        10
    }
}
const fn after_rep(s: usize) -> (r: usize)
    ensures // @ghost
        7 <= r < 12, // @ghost
{
    if s < 7 {
        8
    } else {
        11
    }
}
const fn after_short_rep(s: usize) -> (r: usize)
    ensures // @ghost
        7 <= r < 12, // @ghost
{
    if s < 7 {
        9
    } else {
        11
    }
}
const fn dist_state(len: usize) -> (r: usize)
    requires // @ghost
        len >= 2, // @ghost
    ensures // @ghost
        r < 4, // @ghost
{
    if len < DIST_STATES + MATCH_LEN_MIN {
        len - MATCH_LEN_MIN
    } else {
        DIST_STATES - 1
    }
}

// ---------------------------------------------------------------------------
// Range decoder

struct RangeDecoder<'a> {
    data: &'a [u8],
    pos: usize,
    range: u32,
    code: u32,
    overrun: bool,
}

impl<'a> RangeDecoder<'a> {
    // @ghost-begin
    spec fn wf(&self) -> bool {
        self.pos <= self.data@.len()
    }
    // @ghost-end

    fn new(data: &'a [u8]) -> (r: Result<Self, LzmaError>)
        ensures // @ghost
            r is Ok ==> r->Ok_0.wf() && r->Ok_0.data == data && !r->Ok_0.overrun, // @ghost
    {
        if data.len() < 5 {
            return Err(LzmaError::Truncated);
        }
        if data[0] != 0 {
            return Err(LzmaError::Corrupt("range coder does not start with 0"));
        }
        let code = (data[1] as u32) << 24 | (data[2] as u32) << 16 | (data[3] as u32) << 8 | data[4] as u32;
        Ok(Self { data, pos: 5, range: u32::MAX, code, overrun: false })
    }

    fn normalize(&mut self)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).data == old(self).data, // @ghost
            old(self).overrun ==> final(self).overrun, // @ghost
    {
        if self.range < TOP {
            self.range <<= 8;
            let mut b = 0;
            if self.pos < self.data.len() {
                b = self.data[self.pos];
                self.pos += 1;
            } else {
                self.overrun = true;
            }
            self.code = (self.code << 8) | b as u32;
        }
    }

    fn bit(&mut self, probs: &mut Vec<u16>, i: usize) -> (r: u32)
        requires // @ghost
            old(self).wf(), // @ghost
            i < old(probs)@.len(), // @ghost
            probs_ok(old(probs)@), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).data == old(self).data, // @ghost
            old(self).overrun ==> final(self).overrun, // @ghost
            final(probs)@.len() == old(probs)@.len(), // @ghost
            probs_ok(final(probs)@), // @ghost
            r <= 1, // @ghost
    {
        self.normalize();
        let p = probs[i];
        proof { // @ghost-begin
            lemma_consts();
            let range = self.range;
            assert(range >> 11 == range / 2048) by (bit_vector);
            assert((range / 2048) * (p as u32 as int) <= (range / 2048) * 2048) by (nonlinear_arith)
                requires
                    p <= 2048,
            ;
            assert((range / 2048) * 2048 <= range) by (nonlinear_arith);
            let q: u32 = (2048 - p) as u32;
            assert(q >> 5u32 <= q) by (bit_vector);
            assert(p >> 5u32 <= p) by (bit_vector);
        } // @ghost-end
        let bound = (self.range >> 11) * p as u32;
        if self.code < bound {
            self.range = bound;
            probs[i] = p + ((BIT_MODEL_TOTAL - p as u32) >> MOVE_BITS) as u16;
            0
        } else {
            self.range -= bound;
            self.code -= bound;
            probs[i] = p - (p >> MOVE_BITS);
            1
        }
    }

    fn tree(&mut self, probs: &mut Vec<u16>, base: usize, bits: u32) -> (r: u32)
        requires // @ghost
            old(self).wf(), // @ghost
            1 <= bits <= 8, // @ghost
            base + p2(bits as nat) <= old(probs)@.len(), // @ghost
            probs_ok(old(probs)@), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).data == old(self).data, // @ghost
            old(self).overrun ==> final(self).overrun, // @ghost
            final(probs)@.len() == old(probs)@.len(), // @ghost
            probs_ok(final(probs)@), // @ghost
            (r as nat) < p2(bits as nat), // @ghost
    {
        let mut m = 1usize;
        let mut k = 0;
        proof { // @ghost-begin
            lemma_p2_small();
        } // @ghost-end
        while k < bits
            invariant // @ghost
                k <= bits <= 8, // @ghost
                p2(k as nat) <= m < p2((k + 1) as nat), // @ghost
                self.wf(), // @ghost
                self.data == old(self).data, // @ghost
                old(self).overrun ==> self.overrun, // @ghost
                probs@.len() == old(probs)@.len(), // @ghost
                base + p2(bits as nat) <= probs@.len(), // @ghost
                probs_ok(probs@), // @ghost
            decreases bits - k, // @ghost
        {
            proof { // @ghost-begin
                lemma_p2_mono((k + 1) as nat, bits as nat);
                lemma_p2_mono((k + 1) as nat, 9);
                reveal_with_fuel(p2, 10);
            } // @ghost-end
            let b = self.bit(probs, base + m);
            proof { // @ghost-begin
                lemma_append_bit(m, b as usize);
                assert(p2((k + 2) as nat) == 2 * p2((k + 1) as nat));
                assert(p2((k + 1) as nat) == 2 * p2(k as nat));
            } // @ghost-end
            m = (m << 1) | b as usize;
            k += 1;
        }
        proof { // @ghost-begin
            lemma_shl_u32(bits);
            lemma_p2_mono((bits + 1) as nat, 9);
            reveal_with_fuel(p2, 10);
        } // @ghost-end
        m as u32 - (1 << bits)
    }

    fn reverse_tree(&mut self, probs: &mut Vec<u16>, base: usize, bits: u32) -> (r: u32)
        requires // @ghost
            old(self).wf(), // @ghost
            bits <= 8, // @ghost
            base + p2(bits as nat) <= old(probs)@.len(), // @ghost
            probs_ok(old(probs)@), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).data == old(self).data, // @ghost
            old(self).overrun ==> final(self).overrun, // @ghost
            final(probs)@.len() == old(probs)@.len(), // @ghost
            probs_ok(final(probs)@), // @ghost
            (r as nat) < p2(bits as nat), // @ghost
    {
        let mut m = 1usize;
        let mut sym = 0;
        let mut i = 0;
        proof { // @ghost-begin
            lemma_p2_small();
        } // @ghost-end
        while i < bits
            invariant // @ghost
                i <= bits <= 8, // @ghost
                p2(i as nat) <= m < p2((i + 1) as nat), // @ghost
                (sym as nat) < p2(i as nat), // @ghost
                self.wf(), // @ghost
                self.data == old(self).data, // @ghost
                old(self).overrun ==> self.overrun, // @ghost
                probs@.len() == old(probs)@.len(), // @ghost
                base + p2(bits as nat) <= probs@.len(), // @ghost
                probs_ok(probs@), // @ghost
            decreases bits - i, // @ghost
        {
            proof { // @ghost-begin
                lemma_p2_mono((i + 1) as nat, bits as nat);
                lemma_p2_mono((i + 1) as nat, 9);
                reveal_with_fuel(p2, 10);
            } // @ghost-end
            let b = self.bit(probs, base + m);
            proof { // @ghost-begin
                lemma_append_bit(m, b as usize);
                assert(p2((i + 2) as nat) == 2 * p2((i + 1) as nat));
                assert(p2((i + 1) as nat) == 2 * p2(i as nat));
                lemma_shl_u32(i);
                let s = sym;
                let bb = b;
                let ii = i;
                assert(s | (bb << ii) == s + bb * (1u32 << ii)) by (bit_vector)
                    requires
                        ii < 8,
                        bb <= 1,
                        s < (1u32 << ii),
                ;
            } // @ghost-end
            m = (m << 1) | b as usize;
            sym |= b << i;
            i += 1;
        }
        sym
    }

    fn direct(&mut self, bits: u32) -> (r: u32)
        requires // @ghost
            old(self).wf(), // @ghost
            bits <= 26, // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).data == old(self).data, // @ghost
            old(self).overrun ==> final(self).overrun, // @ghost
            (r as nat) < p2(bits as nat), // @ghost
    {
        let mut v = 0u32;
        let mut k = 0;
        proof { // @ghost-begin
            lemma_p2_small();
        } // @ghost-end
        while k < bits
            invariant // @ghost
                k <= bits <= 26, // @ghost
                (v as nat) < p2(k as nat), // @ghost
                self.wf(), // @ghost
                self.data == old(self).data, // @ghost
                old(self).overrun ==> self.overrun, // @ghost
            decreases bits - k, // @ghost
        {
            self.normalize();
            self.range >>= 1;
            self.code = self.code.wrapping_sub(self.range);
            proof { // @ghost-begin
                let c = self.code;
                assert(c >> 31 <= 1) by (bit_vector);
            } // @ghost-end
            let mask = 0u32.wrapping_sub(self.code >> 31);
            self.code = self.code.wrapping_add(self.range & mask);
            proof { // @ghost-begin
                lemma_p2_mono(k as nat, 26);
                lemma_p2_small();
                assert(p2((k + 1) as nat) == 2 * p2(k as nat));
                assert(mask == 0 || mask == 0xffff_ffffu32);
                let vv = v;
                assert(vv << 1 == 2 * vv) by (bit_vector)
                    requires
                        vv < 0x0400_0000,
                ;
            } // @ghost-end
            v = (v << 1).wrapping_add(mask.wrapping_add(1));
            k += 1;
        }
        v
    }

    fn len(&mut self, probs: &mut Vec<u16>, base: usize, pos_state: usize) -> (r: usize)
        requires // @ghost
            old(self).wf(), // @ghost
            base + LEN_PROBS <= old(probs)@.len(), // @ghost
            pos_state < 16, // @ghost
            probs_ok(old(probs)@), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).data == old(self).data, // @ghost
            old(self).overrun ==> final(self).overrun, // @ghost
            final(probs)@.len() == old(probs)@.len(), // @ghost
            probs_ok(final(probs)@), // @ghost
            2 <= r <= 273, // @ghost
    {
        proof { // @ghost-begin
            lemma_consts();
            lemma_p2_small();
            reveal_with_fuel(p2, 9);
        } // @ghost-end
        if self.bit(probs, base + LEN_CHOICE) == 0 {
            2 + self.tree(probs, base + LEN_LOW + pos_state * 8, 3) as usize
        } else if self.bit(probs, base + LEN_CHOICE2) == 0 {
            10 + self.tree(probs, base + LEN_MID + pos_state * 8, 3) as usize
        } else {
            18 + self.tree(probs, base + LEN_HIGH, 8) as usize
        }
    }
}

// ---------------------------------------------------------------------------
// Decoder

#[derive(Clone, Copy)]
pub(crate) enum End {
    Known { size: u64, allow_marker: bool },
    Marker,
}

// @ghost-begin
/// The stream may not end with a marker, as in LZMA2.
pub(crate) open spec fn no_marker(end: End) -> bool {
    match end {
        End::Known { allow_marker, .. } => !allow_marker,
        End::Marker => false,
    }
}
// @ghost-end

#[verifier::loop_isolation(false)] // @ghost
#[verifier::allow_complex_invariants] // @ghost
#[verifier::spinoff_prover] // @ghost
#[verifier::rlimit(100)] // @ghost
pub(crate) fn decode_stream(
    model: &mut Model,
    props: &LzmaProps,
    input: &[u8],
    end: End,
    out: &mut Vec<u8>,
    dict_start: usize,
    max_output: usize,
) -> (r: Result<usize, LzmaError>)
    requires // @ghost
        old(model).wf(), // @ghost
        dict_start <= old(out)@.len() <= max_output, // @ghost
        old(model).reps_ok(old(out)@.len() - dict_start), // @ghost
    ensures // @ghost
        final(model).wf(), // @ghost
        old(out)@.len() <= final(out)@.len() <= max_output, // @ghost
        r is Ok ==> r->Ok_0 <= input@.len(), // @ghost
        r is Ok && no_marker(end) ==> final(model).reps_ok(final(out)@.len() - dict_start), // @ghost
{
    let mut rc = RangeDecoder::new(input)?;
    let dict_size = props.effective_dict();
    let start_len = out.len();
    let (known, allow_marker, size) = match end {
        End::Known { size, allow_marker } => (true, allow_marker, size),
        End::Marker => (false, true, 0),
    };
    let mut marker_ok = !known;
    if known && size > (max_output - start_len) as u64 {
        return Err(LzmaError::OutputLimitExceeded);
    }
    // Where the output stops: at the known size, or else at the limit.
    let target = if known { start_len + size as usize } else { max_output };
    let m = model;
    loop
        invariant // @ghost
            m.wf(), // @ghost
            rc.wf(), // @ghost
            rc.data == input, // @ghost
            start_len <= out@.len() <= target <= max_output, // @ghost
            m.reps_ok(out@.len() - dict_start), // @ghost
            no_marker(end) ==> !marker_ok, // @ghost
            no_marker(end) ==> known && !allow_marker, // @ghost
        decreases target - out@.len(), // @ghost
    {
        if rc.overrun {
            return Err(LzmaError::Truncated);
        }
        let pos = out.len() - dict_start;
        if known && out.len() == target {
            // In rare cases one more byte has to be read anyway.
            rc.normalize();
            if rc.overrun {
                return Err(LzmaError::Truncated);
            }
            if rc.code == 0 {
                return Ok(rc.pos);
            }
            if allow_marker {
                marker_ok = true;
            } else {
                return Err(LzmaError::Corrupt("data after the end of the stream"));
            }
        }
        let full = if pos < dict_size { pos } else { dict_size };
        let pos_state = m.pos_state(pos);
        let state = m.state;
        proof { // @ghost-begin
            lemma_consts();
            lemma_p2_small();
            lemma_p2_pos((m.lc + m.lp) as nat);
        } // @ghost-end
        if rc.bit(&mut m.probs, IS_MATCH + state * POS_STATES_MAX + pos_state) == 0 {
            // Literal.
            if out.len() >= target {
                return Err(if known {
                    LzmaError::Corrupt("data after the end of the stream")
                } else {
                    LzmaError::OutputLimitExceeded
                });
            }
            let prev = if pos > 0 { out[out.len() - 1] } else { 0 };
            let base = m.literal_base(pos, prev);
            let mut sym = 1usize;
            if state >= 7 {
                let mut match_byte = out[out.len() - m.reps[0] as usize - 1] as usize;
                loop
                    invariant_except_break // @ghost
                        sym < 0x100, // @ghost
                    invariant // @ghost
                        1 <= sym < 0x200, // @ghost
                        m.wf(), // @ghost
                        rc.wf(), // @ghost
                        rc.data == input, // @ghost
                        base + 0x300 <= m.probs@.len(), // @ghost
                    ensures // @ghost
                        1 <= sym < 0x200, // @ghost
                    decreases 0x200 - sym, // @ghost
                {
                    proof { // @ghost-begin
                        let mb = match_byte;
                        assert(((mb >> 7) & 1) <= 1) by (bit_vector);
                    } // @ghost-end
                    let match_bit = (match_byte >> 7) & 1;
                    match_byte <<= 1;
                    proof { // @ghost-begin
                        assert((1 + match_bit) << 8 <= 512) by (bit_vector)
                            requires
                                match_bit <= 1,
                        ;
                    } // @ghost-end
                    let b = rc.bit(&mut m.probs, base + ((1 + match_bit) << 8) + sym) as usize;
                    proof { // @ghost-begin
                        lemma_append_bit(sym, b);
                    } // @ghost-end
                    sym = (sym << 1) | b;
                    if match_bit != b || sym >= 0x100 {
                        break;
                    }
                }
            }
            while sym < 0x100
                invariant // @ghost
                    1 <= sym < 0x200, // @ghost
                    m.wf(), // @ghost
                    rc.wf(), // @ghost
                    rc.data == input, // @ghost
                    base + 0x300 <= m.probs@.len(), // @ghost
                decreases 0x200 - sym, // @ghost
            {
                let b = rc.bit(&mut m.probs, base + sym);
                proof { // @ghost-begin
                    lemma_append_bit(sym, b as usize);
                } // @ghost-end
                sym = (sym << 1) | b as usize;
            }
            out.push(sym as u8);
            m.state = after_literal(state);
            continue;
        }
        let len;
        if rc.bit(&mut m.probs, IS_REP + state) == 0 {
            // Match with a new distance.
            len = rc.len(&mut m.probs, LEN, pos_state);
            m.state = after_match(state);
            let dist = decode_distance(&mut rc, m, len);
            if dist == u32::MAX {
                if !marker_ok {
                    return Err(LzmaError::Corrupt("unexpected end marker"));
                }
                rc.normalize();
                if rc.overrun {
                    return Err(LzmaError::Truncated);
                }
                return if rc.code == 0 { Ok(rc.pos) } else { Err(LzmaError::Corrupt("data after the end marker")) };
            }
            if dist as usize >= full {
                return Err(LzmaError::Corrupt("match distance beyond the data"));
            }
            m.reps = [dist, m.reps[0], m.reps[1], m.reps[2]];
        } else {
            if full == 0 {
                return Err(LzmaError::Corrupt("repeated match before any data"));
            }
            if rc.bit(&mut m.probs, IS_REP0 + state) == 0 {
                if rc.bit(&mut m.probs, IS_REP0_LONG + state * POS_STATES_MAX + pos_state) == 0 {
                    // Short rep: one byte from the last distance.
                    if out.len() >= target {
                        return Err(if known {
                            LzmaError::Corrupt("data after the end of the stream")
                        } else {
                            LzmaError::OutputLimitExceeded
                        });
                    }
                    m.state = after_short_rep(state);
                    let b = out[out.len() - m.reps[0] as usize - 1];
                    out.push(b);
                    continue;
                }
            } else {
                let mut dist = m.reps[1];
                if rc.bit(&mut m.probs, IS_REP1 + state) != 0 {
                    if rc.bit(&mut m.probs, IS_REP2 + state) == 0 {
                        dist = m.reps[2];
                    } else {
                        dist = m.reps[3];
                        m.reps[3] = m.reps[2];
                    }
                    m.reps[2] = m.reps[1];
                }
                m.reps[1] = m.reps[0];
                m.reps[0] = dist;
            }
            len = rc.len(&mut m.probs, REP_LEN, pos_state);
            m.state = after_rep(state);
        }
        // Copy the match, as far as the end and the limit allow.
        let room = target - out.len();
        let n = if len < room { len } else { room };
        let d = m.reps[0] as usize + 1;
        let from = out.len() - d;
        if d >= n {
            let at = out.len();
            out.resize(at + n, 0);
            out.as_mut_slice().copy_within(from..from + n, at);
        } else {
            let mut i = 0;
            while i < n
                invariant // @ghost
                    i <= n, // @ghost
                    out@.len() == from + d + i, // @ghost
                    from + d + n <= target, // @ghost
                decreases n - i, // @ghost
            {
                let b = out[from + i];
                out.push(b);
                i += 1;
            }
        }
        if n < len {
            return Err(if known {
                LzmaError::Corrupt("match runs past the end of the stream")
            } else {
                LzmaError::OutputLimitExceeded
            });
        }
    }
}

fn decode_distance(rc: &mut RangeDecoder<'_>, m: &mut Model, len: usize) -> u32
    requires // @ghost
        old(rc).wf(), // @ghost
        old(m).wf(), // @ghost
        2 <= len, // @ghost
    ensures // @ghost
        final(rc).wf(), // @ghost
        final(rc).data == old(rc).data, // @ghost
        old(rc).overrun ==> final(rc).overrun, // @ghost
        final(m).wf(), // @ghost
        final(m).same_but_probs(old(m)), // @ghost
{
    proof { // @ghost-begin
        lemma_consts();
        lemma_p2_small();
        lemma_p2_pos((m.lc + m.lp) as nat);
    } // @ghost-end
    let slot = rc.tree(&mut m.probs, DIST_SLOT + dist_state(len) * DIST_SLOTS, 6);
    if slot < 4 {
        return slot;
    }
    proof { // @ghost-begin
        let s = slot;
        assert(1 <= (s >> 1) - 1 <= 30) by (bit_vector)
            requires
                4 <= s < 64,
        ;
    } // @ghost-end
    let footer = (slot >> 1) - 1;
    let base = (2 | (slot & 1)) << footer;
    if slot < END_POS_MODEL_INDEX {
        proof { // @ghost-begin
            let s = slot;
            let f = footer;
            assert(s <= ((2 | (s & 1)) << f)) by (bit_vector)
                requires
                    4 <= s < 14,
                    f == (s >> 1) - 1,
            ;
            assert(((2 | (s & 1)) << f) - s + (1u32 << f) <= 115) by (bit_vector)
                requires
                    4 <= s < 14,
                    f == (s >> 1) - 1,
            ;
            assert(((2 | (s & 1)) << f) <= 96) by (bit_vector)
                requires
                    4 <= s < 14,
                    f == (s >> 1) - 1,
            ;
            assert(f <= 5) by (bit_vector)
                requires
                    4 <= s < 14,
                    f == (s >> 1) - 1,
            ;
            lemma_shl_u32(f);
            lemma_p2_mono(f as nat, 5);
        } // @ghost-end
        base + rc.reverse_tree(&mut m.probs, DIST_SPECIAL + (base - slot) as usize, footer)
    } else {
        proof { // @ghost-begin
            let s = slot;
            let f = footer;
            assert(6 <= f <= 30) by (bit_vector)
                requires
                    14 <= s < 64,
                    f == (s >> 1) - 1,
            ;
            lemma_p2_mono((f - 4) as nat, 26);
        } // @ghost-end
        let h = rc.direct(footer - ALIGN_BITS);
        proof { // @ghost-begin
            let s = slot;
            let f = footer;
            let hh = h;
            let f4: u32 = (f - 4) as u32;
            lemma_shl_u32(f4);
            assert((((2 | (s & 1)) << f) as u64) + ((hh << 4u32) as u64) + 15 <= 0xffff_ffff) by (bit_vector)
                requires
                    14 <= s < 64,
                    f == (s >> 1) - 1,
                    f4 == f - 4u32,
                    hh < (1u32 << f4),
            ;
            assert((hh << 4u32) as u64 == (hh as u64) * 16) by (bit_vector)
                requires
                    6 <= f <= 30,
                    f4 == f - 4u32,
                    hh < (1u32 << f4),
            ;
        } // @ghost-end
        let high = h << ALIGN_BITS;
        base + high + rc.reverse_tree(&mut m.probs, DIST_ALIGN, ALIGN_BITS)
    }
}

pub struct Decompressed {
    pub data: Vec<u8>,
    pub consumed: usize,
}

pub fn decompress_raw(
    input: &[u8],
    props: &LzmaProps,
    size: Option<u64>,
    max_output: usize,
) -> (r: Result<Decompressed, LzmaError>)
    ensures // @ghost
        r is Ok ==> r->Ok_0.data@.len() <= max_output, // @ghost
        r is Ok ==> r->Ok_0.consumed <= input@.len(), // @ghost
{
    props.check()?;
    let mut model = Model::new(props);
    // Room for the output if its size is known, up to 64 MiB.
    let mut cap = 0;
    if let Some(n) = size {
        cap = if n < 1 << 26 { n as usize } else { 1 << 26 };
        if cap > max_output {
            cap = max_output;
        }
    }
    let mut out = Vec::with_capacity(cap);
    let end = match size {
        Some(size) => End::Known { size, allow_marker: true },
        None => End::Marker,
    };
    let consumed = decode_stream(&mut model, props, input, end, &mut out, 0, max_output)?;
    Ok(Decompressed { data: out, consumed })
}

pub fn decompress_zip_entry(raw: &[u8], size: usize) -> (r: Result<Vec<u8>, LzmaError>)
    ensures // @ghost
        r is Ok ==> r->Ok_0@.len() <= size, // @ghost
{
    if raw.len() < 9 {
        return Err(LzmaError::Truncated);
    }
    if raw[2] != 5 || raw[3] != 0 {
        return Err(LzmaError::BadProperties);
    }
    let props = LzmaProps::parse(&[raw[4], raw[5], raw[6], raw[7], raw[8]])?;
    let d = decompress_raw(&raw[9..], &props, Some(size as u64), size)?;
    if 9 + d.consumed != raw.len() {
        return Err(LzmaError::Corrupt("data after the LZMA stream"));
    }
    Ok(d.data)
}

} // verus!
