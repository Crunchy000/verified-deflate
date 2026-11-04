//! LZ77 (`src/lz77.rs`): the match finder and tokenizer the encoder uses,
//! and the decoder's match copy.
//!
//! The executable code below is the library's, token for token (CI checks
//! this with `.github/scripts/verus_drift.py`); every line marked
//! `// @ghost` is specification or proof. The tokenizer never panics on any
//! input, emits only matches DEFLATE can express (length 3-258, distance
//! 1-32768), always makes progress, and covers at most 258 bytes per
//! token. The match copy is never out of bounds for any distance of at
//! least 1 that reaches back no further than the data written.

use crate::bitio::lemma_slice_len;
use crate::limit_depths::p2;
use crate::std_specs::axiom_byte_slice_len;
use crate::tables::{MAX_DISTANCE, MAX_MATCH, MIN_MATCH};
use vstd::prelude::*;
use vstd::std_specs::bits::axiom_u64_trailing_zeros;

verus! {

/// One LZ77 output element.
#[derive(Clone, Copy)]
pub enum Token {
    /// A byte copied to the output as-is.
    Literal(u8),
    /// Copy `len` bytes (3..=258) starting `dist` bytes back (1..=32768).
    Match {
        /// Match length.
        len: u16,
        /// Match distance.
        dist: u16,
    },
}

/// A token DEFLATE can express.
pub open spec fn valid(t: Token) -> bool {
    match t {
        Token::Literal(_) => true,
        Token::Match { len, dist } => 3 <= len <= 258 && 1 <= dist <= 32768,
    }
}

/// Match-finder tuning.
#[derive(Clone, Copy)]
pub struct MatchConfig {
    /// Maximum hash-chain entries examined per search.
    pub max_chain: usize,
    /// Stop searching once a match this long is found.
    pub nice_len: usize,
    /// When the current match is at least this long, the lazy look-ahead
    /// search examines only a quarter of `max_chain`.
    pub good_len: usize,
    /// With `lazy`, only try the look-ahead for matches shorter than this.
    pub max_lazy: usize,
    /// Defer a match by one byte if the next position has a longer one.
    pub lazy: bool,
    /// log2 of the window (at most 15, i.e. 32 KiB).
    pub window_bits: u32,
    /// log2 of the hash-head table size (1..=16).
    pub hash_bits: u32,
    /// Run-length mode.
    pub rle: bool,
}

impl MatchConfig {
    /// Settings for compression levels 1 (fastest) to 9 (smallest).
    pub const fn for_level(level: u8) -> Self {
        // (max_chain, nice_len, good_len, max_lazy, lazy), after zlib's table.
        let (max_chain, nice_len, good_len, max_lazy, lazy) = match level {
            0 | 1 => (4, 8, 4, 4, false),
            2 => (8, 16, 4, 5, false),
            3 => (32, 32, 4, 6, false),
            4 => (16, 16, 4, 4, true),
            5 => (32, 32, 8, 16, true),
            6 => (128, 128, 8, 16, true),
            7 => (256, 128, 8, 32, true),
            8 => (1024, MAX_MATCH, 32, 128, true),
            _ => (4096, MAX_MATCH, 32, MAX_MATCH, true),
        };
        Self { max_chain, nice_len, good_len, max_lazy, lazy, window_bits: 15, hash_bits: 15, rle: false }
    }
}

/// `1 << s` is 2^s, for a `usize` shifted by a `u32`.
proof fn lemma_shl_usize32(s: u32)
    requires
        s <= 16,
    ensures
        (1usize << s) as nat == p2(s as nat),
    decreases s,
{
    if s == 0 {
        assert(1usize << 0u32 == 1usize) by (bit_vector);
    } else {
        let t = (s - 1) as u32;
        lemma_shl_usize32(t);
        assert(1usize << s == 2 * (1usize << t)) by (bit_vector)
            requires
                t < 16,
                s == t + 1,
        ;
    }
}

/// `1 << s` is 2^s, for a `u32`.
proof fn lemma_shl_u32(s: u32)
    requires
        s <= 16,
    ensures
        (1u32 << s) as nat == p2(s as nat),
    decreases s,
{
    if s == 0 {
        assert(1u32 << 0u32 == 1u32) by (bit_vector);
    } else {
        let t = (s - 1) as u32;
        lemma_shl_u32(t);
        assert(1u32 << s == 2 * (1u32 << t)) by (bit_vector)
            requires
                t < 16,
                s == t + 1,
        ;
    }
}

proof fn lemma_p2_le_32768(a: nat)
    requires
        a <= 15,
    ensures
        p2(a) <= 32768,
{
    crate::limit_depths::lemma_p2_mono(a, 15);
    crate::limit_depths::lemma_p2_15();
}

/// A position masked by `m` is at most `m`.
proof fn lemma_mask_le(x: usize, m: usize)
    ensures
        x & m <= m,
{
    assert(x & m <= m) by (bit_vector);
}

/// Hash-chain match finder over a whole input buffer.
pub struct Matcher {
    cfg: MatchConfig,
    /// `head[h]` = 1 + most recent position with hash `h` (0 = none).
    head: Vec<u32>,
    /// `prev[p mod window]` = 1 + previous position with the same hash as `p`.
    prev: Vec<u32>,
    window: usize,
    hash_shift: u32,
}

impl Matcher {
    /// The tables have the sizes the configuration says, and the window is
    /// at most DEFLATE's 32 KiB.
    pub closed spec fn wf(&self) -> bool {
        &&& 1 <= self.cfg.window_bits <= 15
        &&& 1 <= self.cfg.hash_bits <= 16
        &&& self.window as nat == p2(self.cfg.window_bits as nat)
        &&& 2 <= self.window <= 32768
        &&& self.prev@.len() == self.window
        &&& self.head@.len() == p2(self.cfg.hash_bits as nat)
        &&& self.hash_shift == 32 - self.cfg.hash_bits
    }

    /// Creates a matcher; out-of-range window or hash sizes are clamped.
    pub fn new(cfg: MatchConfig) -> (r: Self)
        ensures // @ghost
            r.wf(), // @ghost
    {
        let window_bits = cfg.window_bits.clamp(1, 15);
        let hash_bits = cfg.hash_bits.clamp(1, 16);
        proof { // @ghost-begin
            lemma_shl_usize32(window_bits);
            lemma_shl_usize32(hash_bits);
            lemma_p2_le_32768(window_bits as nat);
            crate::limit_depths::lemma_p2_mono(1, window_bits as nat);
            assert(p2(0) == 1);
            assert(p2(1) == 2);
        } // @ghost-end
        let window = 1usize << window_bits;
        Self {
            cfg: MatchConfig { window_bits, hash_bits, ..cfg },
            head: vec![0; 1 << hash_bits],
            prev: vec![0; window],
            window,
            hash_shift: 32 - hash_bits,
        }
    }

    /// Forgets every recorded position, as if newly created.
    pub fn reset(&mut self)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).rle() == old(self).rle(), // @ghost
    {
        self.head.fill(0);
        self.prev.fill(0);
    }

    pub closed spec fn rle(&self) -> bool {
        self.cfg.rle
    }

    /// Records positions `0..end` of `data` in the hash chains.
    pub fn prime(&mut self, data: &[u8], end: usize)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
    {
        if self.cfg.rle {
            return;
        }
        let end = if end < data.len() { end } else { data.len() };
        for pos in 0..end
            invariant // @ghost
                self.wf(), // @ghost
                end <= data@.len(), // @ghost
        {
            self.insert_at(pos, self.bucket(data, pos));
        }
    }

    #[inline]
    fn hash(&self, data: &[u8], pos: usize) -> (r: usize)
        requires // @ghost
            self.wf(), // @ghost
            pos + 2 < data@.len(), // @ghost
        ensures // @ghost
            r < self.head@.len(), // @ghost
    {
        let v = u32::from(data[pos]) | (u32::from(data[pos + 1]) << 8) | (u32::from(data[pos + 2]) << 16);
        proof { // @ghost-begin
            let x = v.wrapping_mul(0x9E37_79B1);
            let s = self.hash_shift;
            let t = (32 - s) as u32;
            assert(x >> s < (1u32 << t)) by (bit_vector)
                requires
                    16 <= s <= 31,
                    t == 32 - s,
            ;
            lemma_shl_u32(t);
        } // @ghost-end
        (v.wrapping_mul(0x9E37_79B1) >> self.hash_shift) as usize
    }

    /// Hash bucket of `pos`, or `None` if fewer than three bytes remain.
    #[inline]
    fn bucket(&self, data: &[u8], pos: usize) -> (r: Option<usize>)
        requires // @ghost
            self.wf(), // @ghost
            pos < data@.len(), // @ghost
        ensures // @ghost
            r matches Some(h) ==> h < self.head@.len(), // @ghost
    {
        proof { // @ghost-begin
            axiom_byte_slice_len(data);
        } // @ghost-end
        if pos + MIN_MATCH > data.len() {
            None
        } else {
            Some(self.hash(data, pos))
        }
    }

    /// Records `pos` (whose bucket is `h`) in the hash chains.
    #[inline]
    fn insert_at(&mut self, pos: usize, h: Option<usize>)
        requires // @ghost
            old(self).wf(), // @ghost
            h matches Some(k) ==> k < old(self).head@.len(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).cfg == old(self).cfg, // @ghost
    {
        if let Some(h) = h {
            proof { // @ghost-begin
                lemma_mask_le(pos, (self.window - 1) as usize);
            } // @ghost-end
            self.prev[pos & (self.window - 1)] = self.head[h];
            self.head[h] = (pos as u32).wrapping_add(1);
        }
    }

    /// Distances a match at `pos` may use: `1..=max_dist(pos)`.
    #[inline]
    fn max_dist(&self, pos: usize) -> (r: usize)
        requires // @ghost
            self.wf(), // @ghost
        ensures // @ghost
            r <= pos, // @ghost
            r <= 32768, // @ghost
    {
        if pos < self.window {
            pos
        } else {
            self.window
        }
    }

    /// Distance from `pos` back to chain entry `cand`.
    #[inline]
    fn dist_to(pos: usize, cand: u32) -> usize {
        (pos as u32).wrapping_sub(cand.wrapping_sub(1)) as usize
    }

    /// Longest match at `pos` (whose bucket is `h`) strictly longer than
    /// `longer_than`, as `(len, dist)`. Requires `pos < data.len()`.
    #[inline(always)]
    fn find_match(
        &self,
        data: &[u8],
        pos: usize,
        h: Option<usize>,
        longer_than: usize,
        max_chain: usize,
    ) -> (r: Option<(usize, usize)>)
        requires // @ghost
            self.wf(), // @ghost
            pos < data@.len(), // @ghost
            h matches Some(k) ==> k < self.head@.len(), // @ghost
        ensures // @ghost
            r matches Some((len, dist)) ==> 3 <= len <= 258 && len > longer_than && pos + len // @ghost
                <= data@.len() && 1 <= dist <= 32768, // @ghost
    {
        let cand = self.head[h?];
        if Self::dist_to(pos, cand).wrapping_sub(1) >= self.max_dist(pos) {
            return None;
        }
        self.longest_match(data, pos, cand, longer_than, max_chain)
    }

    /// The chain search behind [`Self::find_match`].
    #[inline(never)]
    fn longest_match(
        &self,
        data: &[u8],
        pos: usize,
        cand: u32,
        longer_than: usize,
        max_chain: usize,
    ) -> (r: Option<(usize, usize)>)
        requires // @ghost
            self.wf(), // @ghost
            pos < data@.len(), // @ghost
        ensures // @ghost
            r matches Some((len, dist)) ==> 3 <= len <= 258 && len > longer_than && pos + len // @ghost
                <= data@.len() && 1 <= dist <= 32768, // @ghost
    {
        proof { // @ghost-begin
            axiom_byte_slice_len(data);
        } // @ghost-end
        let avail = data.len() - pos;
        let max_len = if avail < MAX_MATCH { avail } else { MAX_MATCH };
        if longer_than >= max_len {
            return None;
        }
        // The bytes a match must equal.
        let cur = &data[pos..pos + max_len];
        let mut best_len = longer_than;
        let mut best_dist = 0;
        // The byte a candidate needs to beat the best match so far.
        let mut scan_end = cur[best_len];
        let mut cand = cand;
        let mut chain = max_chain;
        let max_dist = self.max_dist(pos);
        let prev = &self.prev[..];
        if prev.is_empty() {
            return None;
        }
        let wmask = prev.len() - 1;
        while chain > 0
            invariant // @ghost
                longer_than <= best_len <= max_len <= 258, // @ghost
                longer_than < max_len, // @ghost
                data@.len() <= isize::MAX, // @ghost
                pos + max_len <= data@.len(), // @ghost
                cur@.len() == max_len, // @ghost
                max_dist <= pos, // @ghost
                max_dist <= 32768, // @ghost
                prev@.len() >= 1, // @ghost
                wmask == prev@.len() - 1, // @ghost
                best_dist != 0 ==> 1 <= best_dist <= max_dist && best_len > longer_than, // @ghost
            decreases chain, // @ghost
        {
            let dist = Self::dist_to(pos, cand);
            if dist.wrapping_sub(1) >= max_dist {
                break;
            }
            let cpos = pos - dist;
            if data[cpos + best_len] == scan_end {
                let len = common_prefix(&data[cpos..cpos + max_len], cur);
                if len > best_len {
                    best_len = len;
                    best_dist = dist;
                    if len >= self.cfg.nice_len || len == max_len {
                        break;
                    }
                    scan_end = cur[best_len];
                }
            }
            proof { // @ghost-begin
                lemma_mask_le(cpos, wmask);
            } // @ghost-end
            cand = prev[cpos & wmask];
            chain -= 1;
        }
        if best_dist == 0 || best_len < MIN_MATCH {
            None
        } else {
            Some((best_len, best_dist))
        }
    }

    /// Tokenizes `data[start..]` into `out`.
    pub fn tokenize(&mut self, data: &[u8], start: usize, out: &mut [Token]) -> (r: (usize, usize))
        requires // @ghost
            old(self).wf(), // @ghost
            start <= data@.len(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            tokenized(data@.len(), start as nat, old(out)@.len(), final(out)@, r), // @ghost
    {
        proof { // @ghost-begin
            axiom_byte_slice_len(data);
        } // @ghost-end
        if self.cfg.rle {
            return tokenize_runs(data, start, out);
        }
        if self.cfg.max_chain == 0 {
            // No chain entries may be examined, so every byte is a literal.
            let n = (data.len() - start.min(data.len())).min(out.len());
            for i in 0..n
                invariant // @ghost
                    out@.len() == old(out)@.len(), // @ghost
                    n <= out@.len(), // @ghost
                    start + n <= data@.len(), // @ghost
                    data@.len() <= isize::MAX, // @ghost
                    forall|j: int| 0 <= j < i ==> valid(#[trigger] out@[j]), // @ghost
            {
                out[i] = Token::Literal(data[start + i]);
            }
            return (start + n, n);
        }
        let max_tokens = out.len();
        let n = data.len();
        // First position not yet covered by an emitted token.
        let mut next = start;
        let mut emitted = 0;
        // A match already found for `next` by the lazy look-ahead.
        let mut pending: Option<(usize, usize)> = None;
        // Whether positions inside the last emitted match go in the chains.
        let mut insert_inside = true;
        for pos in start..n
            invariant // @ghost
                self.wf(), // @ghost
                !self.cfg.rle, // @ghost
                out@.len() == max_tokens == old(out)@.len(), // @ghost
                n == data@.len(), // @ghost
                n <= isize::MAX, // @ghost
                start <= pos <= next <= n, // @ghost
                emitted <= max_tokens, // @ghost
                emitted <= next - start, // @ghost
                next - start <= 258 * emitted, // @ghost
                pending matches Some((l, d)) ==> pos == next && 3 <= l <= 258 && next + l <= n && 1 // @ghost
                    <= d <= 32768, // @ghost
                forall|j: int| 0 <= j < emitted ==> valid(#[trigger] out@[j]), // @ghost
        {
            if pos < next {
                if insert_inside {
                    self.insert_at(pos, self.bucket(data, pos));
                }
                continue;
            }
            let h = self.bucket(data, pos);
            if emitted == max_tokens {
                return (pos, emitted);
            }
            let found = match pending.take() {
                Some(m) => Some(m),
                None => self.find_match(data, pos, h, MIN_MATCH - 1, self.cfg.max_chain),
            };
            self.insert_at(pos, h);
            let Some((len, dist)) = found else {
                out[emitted] = Token::Literal(data[pos]);
                emitted += 1;
                next = pos + 1;
                continue;
            };
            if self.cfg.lazy && len < self.cfg.max_lazy && pos + 1 < n {
                let chain = if len >= self.cfg.good_len { self.cfg.max_chain / 4 + 1 } else { self.cfg.max_chain };
                if let Some(m) = self.find_match(data, pos + 1, self.bucket(data, pos + 1), len, chain) {
                    // Better match one byte later: emit a literal instead.
                    out[emitted] = Token::Literal(data[pos]);
                    emitted += 1;
                    next = pos + 1;
                    pending = Some(m);
                    continue;
                }
            }
            out[emitted] = Token::Match { len: len as u16, dist: dist as u16 };
            emitted += 1;
            next = pos + len;
            insert_inside = self.cfg.lazy || len <= self.cfg.max_lazy;
        }
        (next, emitted)
    }
}

/// What a tokenizer returns for `data[start..]` with room for `room`
/// tokens: `(end, count)` with `count` valid tokens written, covering
/// `start..end` (at least one byte and at most 258 per token), stopping
/// only at the end of the data or when out of room.
pub open spec fn tokenized(n: nat, start: nat, room: nat, out: Seq<Token>, r: (usize, usize)) -> bool {
    &&& out.len() == room
    &&& r.1 <= room
    &&& start <= r.0 <= n
    &&& r.1 <= r.0 - start
    &&& r.0 - start <= 258 * r.1
    &&& (r.0 == n || r.1 == room)
    &&& forall|j: int| 0 <= j < r.1 ==> valid(#[trigger] out[j])
}

/// The run-length tokenizer behind [`MatchConfig::rle`].
fn tokenize_runs(data: &[u8], start: usize, out: &mut [Token]) -> (r: (usize, usize))
    requires // @ghost
        start <= data@.len(), // @ghost
    ensures // @ghost
        tokenized(data@.len(), start as nat, old(out)@.len(), final(out)@, r), // @ghost
{
    let max_tokens = out.len();
    let n = data.len();
    let mut next = start;
    let mut emitted = 0;
    // One step per position, as in `Matcher::tokenize`.
    for pos in start..n
        invariant // @ghost
            out@.len() == max_tokens == old(out)@.len(), // @ghost
            n == data@.len(), // @ghost
            start <= pos <= next <= n, // @ghost
            emitted <= max_tokens, // @ghost
            emitted <= next - start, // @ghost
            next - start <= 258 * emitted, // @ghost
            forall|j: int| 0 <= j < emitted ==> valid(#[trigger] out@[j]), // @ghost
    {
        if pos < next {
            continue;
        }
        if emitted == max_tokens {
            return (pos, emitted);
        }
        let mut len = 0;
        // A run needs at least the next byte to repeat the previous one.
        if pos > 0 && data[pos] == data[pos - 1] {
            let max_len = if n - pos < MAX_MATCH { n - pos } else { MAX_MATCH };
            // Bytes equal to the one before them: a distance-1 match.
            len = common_prefix(&data[pos - 1..pos - 1 + max_len], &data[pos..pos + max_len]);
        }
        if len >= MIN_MATCH {
            out[emitted] = Token::Match { len: len as u16, dist: 1 };
            next = pos + len;
        } else {
            out[emitted] = Token::Literal(data[pos]);
            next = pos + 1;
        }
        emitted += 1;
    }
    (next, emitted)
}

/// Length of the common prefix of `x` and `y` (at most the shorter length).
#[inline]
pub(crate) fn common_prefix(x: &[u8], y: &[u8]) -> (r: usize)
    ensures // @ghost
        r <= x@.len(), // @ghost
        r <= y@.len(), // @ghost
{
    proof { // @ghost-begin
        axiom_byte_slice_len(x);
    } // @ghost-end
    let max = if x.len() < y.len() { x.len() } else { y.len() };
    let mut n = 0;
    while n + 8 <= max
        invariant // @ghost
            n <= max, // @ghost
            max <= x@.len(), // @ghost
            max <= y@.len(), // @ghost
            x@.len() <= isize::MAX, // @ghost
        decreases max - n, // @ghost
    {
        let (Some(a), Some(b)) = (word_at(x, n), word_at(y, n)) else {
            break;
        };
        let diff = a ^ b;
        if diff != 0 {
            proof { // @ghost-begin
                axiom_u64_trailing_zeros(diff);
            } // @ghost-end
            return n + (diff.trailing_zeros() / 8) as usize;
        }
        n += 8;
    }
    while n < max && x[n] == y[n]
        invariant // @ghost
            n <= max, // @ghost
            max <= x@.len(), // @ghost
            max <= y@.len(), // @ghost
        decreases max - n, // @ghost
    {
        n += 1;
    }
    n
}

/// The little-endian word at `s[i..i + 8]`, if there are eight bytes there
/// (`i <= s.len()`), taken as given: Verus has no specification for
/// `first_chunk` or `from_le_bytes`, neither of which can panic.
#[verifier::external_body]
#[inline(always)]
fn word_at(s: &[u8], i: usize) -> Option<u64>
    requires // @ghost
        i <= s@.len(), // @ghost
{
    s[i..].first_chunk::<8>().map(|a| u64::from_le_bytes(*a))
}

#[inline]
pub fn copy_within_buffer(buf: &mut [u8], pos: usize, dist: usize, len: usize)
    requires // @ghost
        1 <= dist <= pos, // @ghost
        pos + len <= old(buf)@.len(), // @ghost
    ensures // @ghost
        final(buf)@.len() == old(buf)@.len(), // @ghost
{
    proof { // @ghost-begin
        lemma_slice_len(buf);
    } // @ghost-end
    let src = pos - dist;
    if dist == 1 {
        let b = buf[src];
        buf[pos..pos + len].fill(b);
        return;
    }
    if len <= 16 && dist >= len {
        // A short copy whose source lies wholly before its destination:
        // two (possibly overlapping) words, both loaded before either is
        // stored, cover it exactly without a call to memmove.
        if len >= 8 {
            let a = word8(buf, src);
            let b = word8(buf, src + len - 8);
            buf[pos..pos + 8].copy_from_slice(&a);
            buf[pos + len - 8..pos + len].copy_from_slice(&b);
        } else if len >= 4 {
            let a = word4(buf, src);
            let b = word4(buf, src + len - 4);
            buf[pos..pos + 4].copy_from_slice(&a);
            buf[pos + len - 4..pos + len].copy_from_slice(&b);
        } else {
            for i in 0..len
                invariant // @ghost
                    buf@.len() == old(buf)@.len(), // @ghost
                    src + len <= pos, // @ghost
                    pos + len <= buf@.len(), // @ghost
            {
                buf[pos + i] = buf[src + i];
            }
        }
        return;
    }
    let mut copied = 0;
    while copied < len
        invariant // @ghost
            buf@.len() == old(buf)@.len(), // @ghost
            copied <= len, // @ghost
            src + dist == pos, // @ghost
            1 <= dist, // @ghost
            pos + len <= buf@.len(), // @ghost
            buf@.len() <= usize::MAX, // @ghost
        decreases len - copied, // @ghost
    {
        let avail = dist + copied;
        let chunk = if len - copied < avail { len - copied } else { avail };
        buf.copy_within(src..src + chunk, pos + copied);
        copied += chunk;
    }
}

/// The 8 bytes at `buf[i..]`.
#[inline(always)]
fn word8(buf: &[u8], i: usize) -> [u8; 8]
    requires // @ghost
        i + 8 <= buf@.len(), // @ghost
{
    proof { // @ghost-begin
        lemma_slice_len(buf);
    } // @ghost-end
    let w = &buf[i..i + 8];
    [w[0], w[1], w[2], w[3], w[4], w[5], w[6], w[7]]
}

/// The 4 bytes at `buf[i..]`.
#[inline(always)]
fn word4(buf: &[u8], i: usize) -> [u8; 4]
    requires // @ghost
        i + 4 <= buf@.len(), // @ghost
{
    proof { // @ghost-begin
        lemma_slice_len(buf);
    } // @ghost-end
    let w = &buf[i..i + 4];
    [w[0], w[1], w[2], w[3]]
}

} // verus!
