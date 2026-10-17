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
    /// Only try the lazy look-ahead for matches shorter than this.
    pub max_lazy: usize,
    /// Defer a match by one byte if the next position has a longer one.
    pub lazy: bool,
    /// log2 of the window (at most 15, i.e. 32 KiB).
    pub window_bits: u32,
    /// log2 of the hash-head table size (1..=16).
    pub hash_bits: u32,
}

impl MatchConfig {
    /// Settings for compression levels 1 (fastest) to 9 (smallest).
    /// Level 0 is handled by the encoder as "store only".
    pub const fn for_level(level: u8) -> Self {
        // (max_chain, nice_len, good_len, max_lazy, lazy), after zlib's table.
        let (max_chain, nice_len, good_len, max_lazy, lazy) = match level {
            0 | 1 => (4, 8, 4, 0, false),
            2 => (8, 16, 4, 0, false),
            3 => (32, 32, 4, 0, false),
            4 => (16, 16, 4, 4, true),
            5 => (32, 32, 8, 16, true),
            6 => (128, 128, 8, 16, true),
            7 => (256, 128, 8, 32, true),
            8 => (1024, MAX_MATCH, 32, 128, true),
            _ => (4096, MAX_MATCH, 32, MAX_MATCH, true),
        };
        Self { max_chain, nice_len, good_len, max_lazy, lazy, window_bits: 15, hash_bits: 15 }
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

    #[inline]
    fn hash(&self, data: &[u8], pos: usize) -> usize {
        let v = u32::from(data[pos]) | (u32::from(data[pos + 1]) << 8) | (u32::from(data[pos + 2]) << 16);
        (v.wrapping_mul(0x9E37_79B1) >> self.hash_shift) as usize
    }

    /// Records `pos` in the hash chains (if three bytes are available there).
    #[inline]
    fn insert(&mut self, data: &[u8], pos: usize) {
        if pos + MIN_MATCH > data.len() {
            return;
        }
        let h = self.hash(data, pos);
        self.prev[pos & (self.window - 1)] = self.head[h];
        self.head[h] = (pos as u32).wrapping_add(1);
    }

    /// Longest match at `pos` strictly longer than `longer_than`, as
    /// `(len, dist)`. Requires `pos < data.len()`.
    fn longest_match(&self, data: &[u8], pos: usize, longer_than: usize, max_chain: usize) -> Option<(usize, usize)> {
        let avail = data.len() - pos;
        if avail < MIN_MATCH {
            return None;
        }
        let max_len = if avail < MAX_MATCH { avail } else { MAX_MATCH };
        if longer_than >= max_len {
            return None;
        }
        let mut best_len = longer_than;
        let mut best_dist = 0;
        let mut cand = self.head[self.hash(data, pos)];
        let mut chain = max_chain;
        let mut last_dist = 0;
        while cand != 0 && chain > 0 {
            // Positions are stored as pos + 1 modulo 2^32; any candidate that
            // is not strictly older than the last, inside the window and
            // inside the data is discarded (this also rules out cycles).
            let dist = (pos as u32).wrapping_sub(cand - 1) as usize;
            if dist <= last_dist || dist > self.window || dist > pos {
                break;
            }
            last_dist = dist;
            let cpos = pos - dist;
            // best_len < max_len, so both indices are in bounds.
            if data[cpos + best_len] == data[pos + best_len] {
                let len = common_prefix(data, cpos, pos, max_len);
                if len > best_len {
                    best_len = len;
                    best_dist = dist;
                    if len >= self.cfg.nice_len || len == max_len {
                        break;
                    }
                }
            }
            cand = self.prev[cpos & (self.window - 1)];
            chain -= 1;
        }
        if best_dist == 0 || best_len < MIN_MATCH {
            None
        } else {
            Some((best_len, best_dist))
        }
    }

    /// Tokenizes `data[start..]`, appending at most `max_tokens` tokens to
    /// `out` (at least one if `start < data.len()` and `max_tokens > 0`).
    /// Returns the position just after the last byte covered.
    ///
    /// Matches may refer back to bytes before `start` (from earlier calls
    /// on the same buffer), but never past `data.len()`.
    pub fn tokenize(&mut self, data: &[u8], start: usize, max_tokens: usize, out: &mut Vec<Token>) -> usize {
        let n = data.len();
        let mut pos = start;
        let mut emitted = 0;
        // A match already found for `pos` by the lazy look-ahead.
        let mut pending: Option<(usize, usize)> = None;
        while pos < n && emitted < max_tokens {
            let found = match pending.take() {
                Some(m) => Some(m),
                None => self.longest_match(data, pos, MIN_MATCH - 1, self.cfg.max_chain),
            };
            let Some((len, dist)) = found else {
                self.insert(data, pos);
                out.push(Token::Literal(data[pos]));
                emitted += 1;
                pos += 1;
                continue;
            };
            self.insert(data, pos);
            if self.cfg.lazy && len < self.cfg.max_lazy && pos + 1 < n {
                let chain = if len >= self.cfg.good_len { self.cfg.max_chain / 4 + 1 } else { self.cfg.max_chain };
                if let Some(next) = self.longest_match(data, pos + 1, len, chain) {
                    // Better match one byte later: emit a literal instead.
                    out.push(Token::Literal(data[pos]));
                    emitted += 1;
                    pos += 1;
                    pending = Some(next);
                    continue;
                }
            }
            out.push(Token::Match { len: len as u16, dist: dist as u16 });
            emitted += 1;
            for p in pos + 1..pos + len {
                self.insert(data, p);
            }
            pos += len;
        }
        pos
    }
}

/// Number of equal bytes at `a` and `b` (with `a < b`), up to `max`.
/// Requires `b + max <= data.len()`.
#[inline]
fn common_prefix(data: &[u8], a: usize, b: usize, max: usize) -> usize {
    let mut n = 0;
    while n + 8 <= max {
        let x = read_u64(data, a + n);
        let y = read_u64(data, b + n);
        let diff = x ^ y;
        if diff != 0 {
            return n + (diff.trailing_zeros() / 8) as usize;
        }
        n += 8;
    }
    while n < max && data[a + n] == data[b + n] {
        n += 1;
    }
    n
}

#[inline]
fn read_u64(d: &[u8], i: usize) -> u64 {
    u64::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3], d[i + 4], d[i + 5], d[i + 6], d[i + 7]])
}

/// True if a match token is within DEFLATE's limits and refers only to
/// bytes already produced (`produced` bytes so far).
pub fn match_is_valid(len: usize, dist: usize, produced: usize) -> bool {
    (MIN_MATCH..=MAX_MATCH).contains(&len) && (1..=MAX_DISTANCE).contains(&dist) && dist <= produced
}

/// Appends a match to `out`: copies `len` bytes starting `dist` bytes from
/// the end, where the source may overlap the bytes being written (that is
/// how DEFLATE expresses runs).
///
/// Copies in doubling chunks: after copying a multiple of `dist` bytes the
/// output is periodic with period `dist`, so the next chunk can be taken
/// from the start of the match source in one `extend_from_within`.
/// Requires `1 <= dist <= out.len()`.
#[inline]
pub fn copy_match(out: &mut Vec<u8>, dist: usize, len: usize) {
    let start = out.len() - dist;
    let mut copied = 0;
    while copied < len {
        let avail = dist + copied;
        let chunk = if len - copied < avail { len - copied } else { avail };
        out.extend_from_within(start..start + chunk);
        copied += chunk;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens_for(data: &[u8], level: u8) -> Vec<Token> {
        let mut m = Matcher::new(MatchConfig::for_level(level));
        let mut out = Vec::new();
        let end = m.tokenize(data, 0, usize::MAX, &mut out);
        assert_eq!(end, data.len());
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

#[cfg(kani)]
mod proofs {
    use super::*;

    fn check_tokens(data: &[u8], cfg: MatchConfig) {
        let mut m = Matcher::new(cfg);
        let mut toks = Vec::with_capacity(data.len());
        let end = m.tokenize(data, 0, usize::MAX, &mut toks);
        assert_eq!(end, data.len());
        let mut produced = 0usize;
        for t in toks.iter() {
            if let Token::Match { len, dist } = *t {
                assert!(match_is_valid(len as usize, dist as usize, produced));
            }
            produced += t.output_len();
        }
        assert_eq!(produced, data.len());
        let mut out = Vec::with_capacity(data.len());
        assert!(replay(&toks, &mut out).is_ok());
        assert_eq!(out.as_slice(), data);
    }

    /// Small tables (4-bit hash, 8-byte window) make collisions, chain
    /// reuse and window eviction all reachable; with lazy matching on,
    /// every token is valid and the tokens replay to the input.
    #[kani::proof]
    #[kani::unwind(9)]
    fn tokens_valid_and_replay_small_tables() {
        let data: [u8; 8] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= 8);
        let cfg = MatchConfig {
            max_chain: 4,
            nice_len: 5,
            good_len: 4,
            max_lazy: 6,
            lazy: true,
            window_bits: 3,
            hash_bits: 2,
        };
        check_tokens(&data[..len], cfg);
    }

    /// Same property with the production configuration of level 9.
    #[kani::proof]
    #[kani::unwind(8)]
    fn tokens_valid_and_replay_level9() {
        let data: [u8; 7] = kani::any();
        check_tokens(&data, MatchConfig::for_level(9));
    }

    /// Same property without lazy matching (levels 1-3).
    #[kani::proof]
    #[kani::unwind(8)]
    fn tokens_valid_and_replay_greedy() {
        let data: [u8; 7] = kani::any();
        check_tokens(&data, MatchConfig::for_level(1));
    }

    /// Runs of a repeated byte become one literal plus an overlapping match.
    #[kani::proof]
    #[kani::unwind(12)]
    fn runs_are_matched() {
        let b: u8 = kani::any();
        let data = [b; 10];
        let mut m = Matcher::new(MatchConfig::for_level(6));
        let mut toks = Vec::new();
        m.tokenize(&data, 0, usize::MAX, &mut toks);
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0], Token::Literal(b));
        assert_eq!(toks[1], Token::Match { len: 9, dist: 1 });
    }

    /// The doubling copy produces exactly what a byte-at-a-time copy does,
    /// including heavily overlapping matches.
    #[kani::proof]
    #[kani::unwind(20)]
    fn copy_match_equals_naive() {
        let prefix: [u8; 4] = kani::any();
        let dist: usize = kani::any();
        let len: usize = kani::any();
        kani::assume(dist >= 1 && dist <= 4 && len <= 12);
        let mut fast = prefix.to_vec();
        copy_match(&mut fast, dist, len);
        let mut slow = prefix.to_vec();
        let mut i = 0;
        while i < len {
            let x = slow[slow.len() - dist];
            slow.push(x);
            i += 1;
        }
        assert_eq!(fast, slow);
    }

    /// Replaying arbitrary tokens never panics: invalid matches are errors.
    #[kani::proof]
    #[kani::unwind(10)]
    fn replay_arbitrary_tokens() {
        let kinds: [bool; 3] = kani::any();
        let bytes: [u8; 3] = kani::any();
        let lens: [u16; 3] = kani::any();
        let dists: [u16; 3] = kani::any();
        let mut toks = [Token::Literal(0); 3];
        for i in 0..3 {
            kani::assume(lens[i] <= 8);
            toks[i] = if kinds[i] { Token::Literal(bytes[i]) } else { Token::Match { len: lens[i], dist: dists[i] } };
        }
        let mut out = Vec::new();
        match replay(&toks, &mut out) {
            Ok(()) => assert_eq!(out.len(), toks.iter().map(|t| t.output_len()).sum::<usize>()),
            Err(e) => assert_eq!(e, InflateError::DistanceTooFarBack),
        }
    }
}
