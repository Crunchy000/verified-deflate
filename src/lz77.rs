//! LZ77 match finding (RFC 1951 §4) with hash chains and optional lazy
//! matching.
//!
//! Correctness does not depend on the hash tables at all: every candidate
//! position is checked for distance bounds and compared byte-for-byte
//! before it is used, so stale, colliding or wrapped-around hash entries
//! can only cost compression, never produce a wrong match. The Kani
//! harnesses check that every token stream the matcher emits is valid and
//! replays to exactly the input.

use crate::error::InflateError;
use crate::tables::{MAX_DISTANCE, MAX_MATCH, MIN_MATCH};

/// One LZ77 output element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl Token {
    /// Number of output bytes this token produces.
    pub fn output_len(&self) -> usize {
        match *self {
            Token::Literal(_) => 1,
            Token::Match { len, .. } => len as usize,
        }
    }
}

/// Match-finder tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchConfig {
    /// Maximum hash-chain entries examined per search.
    pub max_chain: usize,
    /// Stop searching once a match this long is found.
    pub nice_len: usize,
    /// When the current match is at least this long, the lazy look-ahead
    /// search examines only a quarter of `max_chain`.
    pub good_len: usize,
    /// With `lazy`, only try the look-ahead for matches shorter than this.
    /// Without it, positions inside a match longer than this are not added
    /// to the hash chains, which saves time on long matches (zlib uses the
    /// same field both ways).
    pub max_lazy: usize,
    /// Defer a match by one byte if the next position has a longer one.
    pub lazy: bool,
    /// log2 of the window (at most 15, i.e. 32 KiB).
    pub window_bits: u32,
    /// log2 of the hash-head table size (1..=16).
    pub hash_bits: u32,
    /// Run-length mode: only look for matches one byte back (runs of a
    /// repeated byte), without hash chains; the other search settings are
    /// ignored.
    pub rle: bool,
}

impl MatchConfig {
    /// Settings for compression levels 1 (fastest) to 9 (smallest).
    /// Level 0 is handled by the encoder as "store only".
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

/// Hash-chain match finder over a whole input buffer.
#[derive(Debug, Clone)]
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
    /// Creates a matcher; out-of-range window or hash sizes are clamped.
    pub fn new(cfg: MatchConfig) -> Self {
        let window_bits = cfg.window_bits.clamp(1, 15);
        let hash_bits = cfg.hash_bits.clamp(1, 16);
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
    pub fn reset(&mut self) {
        self.head.fill(0);
        self.prev.fill(0);
    }

    /// Records positions `0..end` of `data` in the hash chains without
    /// tokenizing them, so that a following [`tokenize`](Self::tokenize)
    /// from `end` can match against them (a preset history).
    pub fn prime(&mut self, data: &[u8], end: usize) {
        if self.cfg.rle {
            return;
        }
        let end = if end < data.len() { end } else { data.len() };
        for pos in 0..end {
            self.insert_at(pos, self.bucket(data, pos));
        }
    }

    #[inline]
    fn hash(&self, data: &[u8], pos: usize) -> usize {
        let v = u32::from(data[pos]) | (u32::from(data[pos + 1]) << 8) | (u32::from(data[pos + 2]) << 16);
        (v.wrapping_mul(0x9E37_79B1) >> self.hash_shift) as usize
    }

    /// Hash bucket of `pos`, or `None` if fewer than three bytes remain.
    #[inline]
    fn bucket(&self, data: &[u8], pos: usize) -> Option<usize> {
        if pos + MIN_MATCH > data.len() {
            None
        } else {
            Some(self.hash(data, pos))
        }
    }

    /// Records `pos` (whose bucket is `h`) in the hash chains.
    #[inline]
    fn insert_at(&mut self, pos: usize, h: Option<usize>) {
        if let Some(h) = h {
            self.prev[pos & (self.window - 1)] = self.head[h];
            self.head[h] = (pos as u32).wrapping_add(1);
        }
    }

    /// Distances a match at `pos` may use: `1..=max_dist(pos)`.
    #[inline]
    fn max_dist(&self, pos: usize) -> usize {
        if pos < self.window {
            pos
        } else {
            self.window
        }
    }

    /// Distance from `pos` back to chain entry `cand` (a position plus one,
    /// modulo 2^32). An empty entry (0) or one at or after `pos` gives a
    /// distance of 0 or more than `pos`, so a single comparison
    /// `dist.wrapping_sub(1) < max_dist(pos)` accepts exactly the usable
    /// entries.
    #[inline]
    fn dist_to(pos: usize, cand: u32) -> usize {
        (pos as u32).wrapping_sub(cand.wrapping_sub(1)) as usize
    }

    /// Longest match at `pos` (whose bucket is `h`) strictly longer than
    /// `longer_than`, as `(len, dist)`. Requires `pos < data.len()`.
    ///
    /// The common case of no usable candidate is decided here, inline; the
    /// chain search itself is out of line.
    #[inline(always)]
    fn find_match(
        &self,
        data: &[u8],
        pos: usize,
        h: Option<usize>,
        longer_than: usize,
        max_chain: usize,
    ) -> Option<(usize, usize)> {
        let cand = self.head[h?];
        if Self::dist_to(pos, cand).wrapping_sub(1) >= self.max_dist(pos) {
            return None;
        }
        self.longest_match(data, pos, cand, longer_than, max_chain)
    }

    /// The chain search behind [`Self::find_match`], starting from chain
    /// entry `cand`.
    #[inline(never)]
    fn longest_match(
        &self,
        data: &[u8],
        pos: usize,
        cand: u32,
        longer_than: usize,
        max_chain: usize,
    ) -> Option<(usize, usize)> {
        let avail = data.len() - pos;
        let max_len = if avail < MAX_MATCH { avail } else { MAX_MATCH };
        if longer_than >= max_len {
            return None;
        }
        // The bytes a match must equal. Every candidate is compared against
        // a slice of the same length, so most bounds checks disappear.
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
        // prev.len() is the window size, a power of two; masking by
        // prev.len() - 1 also needs no bounds check.
        let wmask = prev.len() - 1;
        while chain > 0 {
            // An empty entry, one at or after `pos`, or one outside the
            // window ends the search. `chain` bounds the loop even if the
            // chains were to contain a cycle.
            let dist = Self::dist_to(pos, cand);
            if dist.wrapping_sub(1) >= max_dist {
                break;
            }
            let cpos = pos - dist;
            // best_len < max_len, so cpos + best_len < pos + max_len <= n.
            // Checking the byte that would extend the best match first
            // rejects most candidates with one comparison.
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
            cand = prev[cpos & wmask];
            chain -= 1;
        }
        if best_dist == 0 || best_len < MIN_MATCH {
            None
        } else {
            Some((best_len, best_dist))
        }
    }

    /// Tokenizes `data[start..]` into `out`, writing at most `out.len()`
    /// tokens (at least one if `start < data.len()` and `out` is not empty).
    /// Returns the position just after the last byte covered and the number
    /// of tokens written.
    ///
    /// Matches may refer back to bytes before `start` (from earlier calls
    /// on the same buffer), but never past `data.len()`.
    pub fn tokenize(&mut self, data: &[u8], start: usize, out: &mut [Token]) -> (usize, usize) {
        if self.cfg.rle {
            return tokenize_runs(data, start, out);
        }
        if self.cfg.max_chain == 0 {
            // No chain entries may be examined, so no match can be found:
            // every byte is a literal, and the chains need no upkeep.
            let n = (data.len() - start.min(data.len())).min(out.len());
            for i in 0..n {
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
        // Every position is visited once, in order; positions inside an
        // emitted match are only recorded in the hash chains. Stepping one
        // position at a time (rather than jumping by match lengths) keeps
        // each data index a plain loop counter, which keeps the model
        // checker's problem small.
        // Whether positions inside the last emitted match go in the chains.
        let mut insert_inside = true;
        for pos in start..n {
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

/// The run-length tokenizer behind [`MatchConfig::rle`], with the same
/// contract as [`Matcher::tokenize`]: a match is emitted wherever the next
/// three or more bytes repeat the byte before them (distance 1).
fn tokenize_runs(data: &[u8], start: usize, out: &mut [Token]) -> (usize, usize) {
    let max_tokens = out.len();
    let n = data.len();
    let mut next = start;
    let mut emitted = 0;
    // One step per position, as in `Matcher::tokenize`.
    for pos in start..n {
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
///
/// Compares eight bytes at a time; the first differing byte is found from
/// the XOR's trailing zeros. Kani proves this equal to the byte-by-byte
/// [`common_prefix_ref`] (and the tokenizer harnesses use the reference,
/// which keeps their models small).
#[inline]
pub(crate) fn common_prefix(x: &[u8], y: &[u8]) -> usize {
    let max = if x.len() < y.len() { x.len() } else { y.len() };
    let mut n = 0;
    while n + 8 <= max {
        let (Some(a), Some(b)) = (x[n..].first_chunk::<8>(), y[n..].first_chunk::<8>()) else {
            break;
        };
        let diff = u64::from_le_bytes(*a) ^ u64::from_le_bytes(*b);
        if diff != 0 {
            return n + (diff.trailing_zeros() / 8) as usize;
        }
        n += 8;
    }
    while n < max && x[n] == y[n] {
        n += 1;
    }
    n
}

/// Reference version of [`common_prefix`]: a plain byte loop.
#[cfg_attr(mutants, mutants::skip)] // specification used by proofs
#[cfg_attr(not(any(kani, test)), allow(dead_code))]
pub(crate) fn common_prefix_ref(x: &[u8], y: &[u8]) -> usize {
    let mut n = 0;
    while n < x.len() && n < y.len() && x[n] == y[n] {
        n += 1;
    }
    n
}

/// True if a match token is within DEFLATE's limits and refers only to
/// bytes already produced (`produced` bytes so far).
pub fn match_is_valid(len: usize, dist: usize, produced: usize) -> bool {
    (MIN_MATCH..=MAX_MATCH).contains(&len) && (1..=MAX_DISTANCE).contains(&dist) && dist <= produced
}

/// The LZ77 copy on a buffer: writes `buf[pos..pos + len]`, each byte
/// equal to the byte `dist` positions earlier. Source and destination may
/// overlap (that is how DEFLATE expresses runs). Requires
/// `1 <= dist <= pos` and `pos + len <= buf.len()`.
///
/// Short copies (at most 16 bytes) whose source lies wholly before the
/// destination are done inline with two word moves. Runs of a single byte
/// are a fill. Otherwise the copy goes in chunks with `copy_within`: once a
/// multiple of `dist` bytes has been written, the output is periodic with
/// period `dist`, so the next chunk (up to everything written so far plus
/// `dist`) can be copied from the start of the source in one
/// non-overlapping move.
#[inline]
pub fn copy_within_buffer(buf: &mut [u8], pos: usize, dist: usize, len: usize) {
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
            for i in 0..len {
                buf[pos + i] = buf[src + i];
            }
        }
        return;
    }
    let mut copied = 0;
    while copied < len {
        let avail = dist + copied;
        let chunk = if len - copied < avail { len - copied } else { avail };
        buf.copy_within(src..src + chunk, pos + copied);
        copied += chunk;
    }
}

/// The 8 bytes at `buf[i..]`.
#[inline(always)]
fn word8(buf: &[u8], i: usize) -> [u8; 8] {
    let w = &buf[i..i + 8];
    [w[0], w[1], w[2], w[3], w[4], w[5], w[6], w[7]]
}

/// The 4 bytes at `buf[i..]`.
#[inline(always)]
fn word4(buf: &[u8], i: usize) -> [u8; 4] {
    let w = &buf[i..i + 4];
    [w[0], w[1], w[2], w[3]]
}

/// Reference version of [`copy_within_buffer`]: the byte-by-byte loop that
/// is the specification. `copy_within_buffer_meets_spec` proves the fast
/// version produces the same (unique) result; harnesses that only need the
/// copy's effect use this one, whose model is much smaller.
#[cfg_attr(mutants, mutants::skip)] // specification used by proofs
#[cfg_attr(not(any(kani, test)), allow(dead_code))]
pub(crate) fn copy_within_buffer_ref(buf: &mut [u8], pos: usize, dist: usize, len: usize) {
    for i in pos..pos + len {
        buf[i] = buf[i - dist];
    }
}

/// Appends a match to `out` (see [`copy_within_buffer`]).
/// Requires `1 <= dist <= out.len()`.
pub fn copy_match(out: &mut Vec<u8>, dist: usize, len: usize) {
    let pos = out.len();
    out.resize(pos + len, 0);
    copy_within_buffer(out, pos, dist, len);
}

/// Reconstructs the bytes described by `tokens`, appending to `out`
/// (which may already hold earlier output that matches refer back to).
pub fn replay(tokens: &[Token], out: &mut Vec<u8>) -> Result<(), InflateError> {
    for t in tokens {
        match *t {
            Token::Literal(b) => out.push(b),
            Token::Match { len, dist } => {
                if !match_is_valid(len as usize, dist as usize, out.len()) {
                    return Err(InflateError::DistanceTooFarBack);
                }
                copy_match(out, dist as usize, len as usize);
            }
        }
    }
    Ok(())
}

/// Stand-in for `Matcher::hash` in proofs: an arbitrary bucket on every
/// call. The tokenizer checks every candidate byte by byte, so its
/// correctness cannot depend on the hash; proving a harness under this stub
/// proves it for every hash function (and avoids a 32-bit multiplication
/// per position in the model).
#[cfg_attr(mutants, mutants::skip)] // proof code: the checker, not code under test
#[cfg(kani)]
#[cfg_attr(kani_sample, allow(dead_code))] // stubs are not applied when sampling
pub(crate) fn any_bucket(m: &Matcher, _data: &[u8], _pos: usize) -> usize {
    kani::any::<usize>() & (m.head.len() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens_for(data: &[u8], level: u8) -> Vec<Token> {
        let mut m = Matcher::new(MatchConfig::for_level(level));
        let mut out = vec![Token::Literal(0); data.len()];
        let (end, n) = m.tokenize(data, 0, &mut out);
        assert_eq!(end, data.len());
        out.truncate(n);
        out
    }

    #[test]
    fn replays_exactly() {
        let mut data = Vec::new();
        for i in 0..20_000u32 {
            data.extend_from_slice(format!("line {} of {}\n", i % 97, i % 13).as_bytes());
        }
        for level in 1..=9 {
            let toks = tokens_for(&data, level);
            assert!(toks.len() < data.len() / 4, "level {level} found few matches");
            let mut out = Vec::new();
            replay(&toks, &mut out).unwrap();
            assert_eq!(out, data, "level {level}");
        }
    }

    #[test]
    fn long_runs_use_overlapping_matches() {
        let data = vec![7u8; 1000];
        let toks = tokens_for(&data, 6);
        assert_eq!(toks[0], Token::Literal(7));
        assert!(matches!(toks[1], Token::Match { dist: 1, len: 258 }));
    }

    #[test]
    fn common_prefix_matches_reference_exhaustively() {
        let mut data = [7u8; 64];
        data[40] = 1;
        data[57] = 2;
        for a in 0..64 {
            for b in a + 1..64 {
                for max in 0..=64 - b {
                    let (x, y) = (&data[a..a + max], &data[b..b + max]);
                    assert_eq!(common_prefix(x, y), common_prefix_ref(x, y));
                }
            }
        }
    }

    #[test]
    fn copy_within_buffer_matches_reference_exhaustively() {
        let before: Vec<u8> = (0..40u8).map(|i| i.wrapping_mul(37)).collect();
        for pos in 1..=40 {
            for dist in 1..=pos {
                for len in 0..=40 - pos {
                    let (mut a, mut b) = (before.clone(), before.clone());
                    copy_within_buffer(&mut a, pos, dist, len);
                    copy_within_buffer_ref(&mut b, pos, dist, len);
                    assert_eq!(a, b, "pos {pos} dist {dist} len {len}");
                }
            }
        }
    }

    #[test]
    fn copy_match_matches_naive() {
        for dist in 1..20 {
            for len in 3..80 {
                let mut a: Vec<u8> = (0..30u8).collect();
                let mut b = a.clone();
                copy_match(&mut a, dist, len);
                for _ in 0..len {
                    let x = b[b.len() - dist];
                    b.push(x);
                }
                assert_eq!(a, b, "dist {dist} len {len}");
            }
        }
    }
}

#[cfg_attr(mutants, mutants::skip)] // proof code: the checker, not code under test
#[cfg(kani)]
mod proofs {
    use super::*;

    /// Tokenizes `data` and checks every token is valid and that the
    /// tokens describe `data` exactly. The reference replay into a fixed
    /// array is the specification of what a token sequence means
    /// (`copy_match` is checked against the same reference separately).
    fn check_tokens<const N: usize>(data: &[u8; N], cfg: MatchConfig) {
        let mut m = Matcher::new(cfg);
        let mut toks = [Token::Literal(0); N];
        let (end, count) = m.tokenize(data, 0, &mut toks);
        assert_eq!(end, N);
        let mut out = [0u8; N];
        let mut produced = 0usize;
        for t in toks.iter().take(count) {
            match *t {
                Token::Literal(b) => {
                    out[produced] = b;
                    produced += 1;
                }
                Token::Match { len, dist } => {
                    let (len, dist) = (len as usize, dist as usize);
                    assert!(match_is_valid(len, dist, produced));
                    for _ in 0..len {
                        out[produced] = out[produced - dist];
                        produced += 1;
                    }
                }
            }
        }
        assert_eq!(produced, N);
        assert_eq!(&out, data);
    }

    /// Small tables (8-byte window, 4 hash buckets) with the given level's
    /// search parameters. Table sizes do not change the tokenizer's logic
    /// for inputs shorter than the window, and under the hash stub every
    /// bucket assignment is covered; small tables keep the model small.
    fn small(level: u8) -> MatchConfig {
        MatchConfig { window_bits: 3, hash_bits: 2, ..MatchConfig::for_level(level) }
    }

    /// For any 4 bytes, with a 2-entry chain limit, lazy matching and
    /// every possible hash function: every token is valid and the tokens
    /// replay to the input.
    #[kani::stub(common_prefix, common_prefix_ref)]
    #[kani::proof]
    #[kani::stub(Matcher::hash, any_bucket)]
    #[kani::unwind(5)]
    fn tokens_valid_and_replay_small_tables() {
        let data: [u8; 4] = kani::any();
        let cfg = MatchConfig {
            max_chain: 2,
            nice_len: 4,
            good_len: 3,
            max_lazy: 4,
            lazy: true,
            window_bits: 3,
            hash_bits: 2,
            rle: false,
        };
        check_tokens(&data, cfg);
    }

    /// Same property for any 4 bytes with level 9's search parameters.
    #[kani::stub(common_prefix, common_prefix_ref)]
    #[kani::proof]
    #[kani::stub(Matcher::hash, any_bucket)]
    #[kani::unwind(5)]
    fn tokens_valid_and_replay_level9() {
        let data: [u8; 4] = kani::any();
        check_tokens(&data, small(9));
    }

    /// Same property for any 4 bytes with level 1's greedy parameters.
    #[kani::stub(common_prefix, common_prefix_ref)]
    #[kani::proof]
    #[kani::stub(Matcher::hash, any_bucket)]
    #[kani::unwind(5)]
    fn tokens_valid_and_replay_greedy() {
        let data: [u8; 4] = kani::any();
        check_tokens(&data, small(1));
    }

    /// Runs of a repeated byte become one literal plus an overlapping match.
    #[kani::stub(common_prefix, common_prefix_ref)]
    #[kani::proof]
    #[kani::unwind(8)]
    fn runs_are_matched() {
        let b: u8 = kani::any();
        let data = [b; 6];
        let mut m = Matcher::new(small(6));
        let mut toks = [Token::Literal(0); 6];
        let (_, count) = m.tokenize(&data, 0, &mut toks);
        assert_eq!(count, 2);
        assert_eq!(toks[0], Token::Literal(b));
        assert_eq!(toks[1], Token::Match { len: 5, dist: 1 });
    }

    /// Run-length mode: for any 6 bytes, every token is a literal or a
    /// distance-1 match of length 3-258, and the tokens describe the input
    /// exactly.
    #[kani::proof]
    #[kani::stub(common_prefix, common_prefix_ref)]
    #[kani::unwind(8)]
    fn rle_tokens_valid_and_replay() {
        let data: [u8; 6] = kani::any();
        let cfg = MatchConfig { rle: true, ..small(1) };
        check_tokens(&data, cfg);
        let mut m = Matcher::new(cfg);
        let mut toks = [Token::Literal(0); 6];
        let (_, count) = m.tokenize(&data, 0, &mut toks);
        for t in toks.iter().take(count) {
            if let Token::Match { dist, .. } = *t {
                assert_eq!(dist, 1);
            }
        }
    }

    /// With no chain entries to examine (Huffman-only mode), every token
    /// is a literal, for any start position and output capacity, and the
    /// tokenizer's contract holds (as many tokens as fit, each covering one
    /// byte).
    #[kani::proof]
    #[kani::unwind(6)]
    fn no_chain_means_literals_only() {
        let data: [u8; 4] = kani::any();
        let start: usize = kani::any_where(|&s: &usize| s <= 4);
        let cap: usize = kani::any_where(|&k: &usize| k <= 4);
        let mut m = Matcher::new(MatchConfig { max_chain: 0, ..small(6) });
        let mut toks = [Token::Match { len: 3, dist: 1 }; 4];
        let (end, count) = m.tokenize(&data, start, &mut toks[..cap]);
        let n = if 4 - start < cap { 4 - start } else { cap };
        assert_eq!((end, count), (start + n, n));
        for i in 0..n {
            assert_eq!(toks[i], Token::Literal(data[start + i]));
        }
    }

    /// The word-at-a-time prefix comparison equals the byte loop for any
    /// two slices of up to 20 bytes (so the 8-byte path runs up to twice,
    /// followed by every tail length).
    #[kani::proof]
    #[kani::unwind(21)]
    fn common_prefix_matches_reference() {
        let x: [u8; 20] = kani::any();
        let y: [u8; 20] = kani::any();
        let lx: usize = kani::any_where(|&l: &usize| l <= 20);
        let ly: usize = kani::any_where(|&l: &usize| l <= 20);
        assert_eq!(common_prefix(&x[..lx], &y[..ly]), common_prefix_ref(&x[..lx], &y[..ly]));
    }

    /// The buffer copy meets the LZ77 specification for every distance and
    /// length that fit, including heavily overlapping copies: each written
    /// byte equals the byte `dist` earlier in the result, and nothing
    /// outside the written range changes.
    #[kani::proof]
    #[kani::unwind(13)]
    fn copy_within_buffer_meets_spec() {
        let before: [u8; 12] = kani::any();
        let pos: usize = kani::any_where(|&p: &usize| p <= 12);
        let len: usize = kani::any_where(|&l: &usize| l <= 12 - pos);
        let dist: usize = kani::any_where(|&d: &usize| d >= 1 && d <= pos);
        let mut buf = before;
        copy_within_buffer(&mut buf, pos, dist, len);
        for i in 0..12 {
            if i >= pos && i < pos + len {
                assert_eq!(buf[i], buf[i - dist]);
            } else {
                assert_eq!(buf[i], before[i]);
            }
        }
    }

    /// The inline path for short non-overlapping copies (up to 16 bytes)
    /// meets the same specification, on a buffer large enough to reach
    /// every case of it.
    #[kani::proof]
    #[kani::unwind(33)]
    fn short_copy_meets_spec() {
        let before: [u8; 32] = kani::any();
        let pos: usize = kani::any_where(|&p: &usize| p <= 32);
        let len: usize = kani::any_where(|&l: &usize| l <= 16 && l <= 32 - pos);
        let dist: usize = kani::any_where(|&d: &usize| d >= 1 && d >= len && d <= pos);
        let mut buf = before;
        copy_within_buffer(&mut buf, pos, dist, len);
        for i in 0..32 {
            if i >= pos && i < pos + len {
                assert_eq!(buf[i], before[i - dist]);
            } else {
                assert_eq!(buf[i], before[i]);
            }
        }
    }

    /// Replaying arbitrary tokens never panics: invalid matches are errors.
    /// (The copy itself is checked by `copy_within_buffer_meets_spec`.)
    #[kani::proof]
    #[kani::stub(copy_within_buffer, copy_within_buffer_ref)]
    #[kani::unwind(6)]
    fn replay_arbitrary_tokens() {
        let kinds: [bool; 2] = kani::any();
        let bytes: [u8; 2] = kani::any();
        let lens: [u16; 2] = kani::any();
        let dists: [u16; 2] = kani::any();
        let mut toks = [Token::Literal(0); 2];
        for i in 0..2 {
            kani::assume(lens[i] <= 4);
            toks[i] = if kinds[i] { Token::Literal(bytes[i]) } else { Token::Match { len: lens[i], dist: dists[i] } };
        }
        let mut out = Vec::with_capacity(8);
        match replay(&toks, &mut out) {
            Ok(()) => assert_eq!(out.len(), toks[0].output_len() + toks[1].output_len()),
            Err(e) => assert_eq!(e, InflateError::DistanceTooFarBack),
        }
    }
}
