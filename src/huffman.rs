//! Canonical Huffman codes (RFC 1951 §3.2.2).
//!
//! * [`build_lengths`] turns symbol frequencies into code lengths that are
//!   at most a given limit and always form a *complete* prefix code (Kraft
//!   sum exactly 1), which strict decoders such as zlib require.
//! * [`canonical_codes`] assigns the RFC's canonical codes to a set of
//!   lengths.
//! * [`Decoder`] decodes canonical codes with the count/symbol scheme of
//!   Mark Adler's `puff.c`: no lookup tables, every index provably in
//!   bounds.
//!
//! All functions work on slices of any alphabet size, which is what lets the
//! Kani harnesses check them exhaustively on small alphabets.

use crate::bitio::BitReader;
use crate::error::InflateError;

/// Longest code length DEFLATE allows.
pub const MAX_CODE_LEN: usize = 15;
/// Largest alphabet the decoder accepts (the fixed literal/length code).
pub const MAX_SYMBOLS: usize = 288;

/// Why a set of code lengths or frequencies was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuffmanError {
    /// A code length exceeded 15.
    CodeTooLong,
    /// The lengths describe more codes than fit (Kraft sum above 1).
    OverSubscribed,
    /// More symbols than the decoder supports.
    TooManySymbols,
    /// Input and output slices had different lengths.
    LengthMismatch,
    /// The alphabet cannot be coded within the requested length limit.
    LimitTooSmall,
}

/// Sum over coded symbols of `2^(15 - length)`: the Kraft sum scaled so
/// that a complete code sums to exactly `1 << 15`.
pub const fn kraft_sum(lengths: &[u8]) -> Result<u64, HuffmanError> {
    let mut sum = 0u64;
    let mut i = 0;
    while i < lengths.len() {
        let l = lengths[i] as usize;
        if l > MAX_CODE_LEN {
            return Err(HuffmanError::CodeTooLong);
        }
        if l > 0 {
            sum += 1u64 << (MAX_CODE_LEN - l);
        }
        i += 1;
    }
    Ok(sum)
}

/// True when `lengths` form a complete prefix code.
pub const fn is_complete(lengths: &[u8]) -> bool {
    matches!(kraft_sum(lengths), Ok(s) if s == 1u64 << MAX_CODE_LEN)
}

/// Assigns canonical Huffman codes (MSB-first values) to `lengths`
/// following RFC 1951 §3.2.2. Symbols with length 0 get code 0.
///
/// Fails if a length exceeds 15 or the lengths are over-subscribed.
pub const fn canonical_codes(lengths: &[u8], codes: &mut [u16]) -> Result<(), HuffmanError> {
    if lengths.len() != codes.len() {
        return Err(HuffmanError::LengthMismatch);
    }
    let mut bl_count = [0u32; MAX_CODE_LEN + 1];
    let mut i = 0;
    while i < lengths.len() {
        let l = lengths[i] as usize;
        if l > MAX_CODE_LEN {
            return Err(HuffmanError::CodeTooLong);
        }
        if l > 0 {
            bl_count[l] += 1;
        }
        i += 1;
    }
    // Kraft check: `left` is the unused code space at each depth.
    let mut left: i64 = 1;
    let mut bits = 1;
    while bits <= MAX_CODE_LEN {
        left = left * 2 - bl_count[bits] as i64;
        if left < 0 {
            return Err(HuffmanError::OverSubscribed);
        }
        bits += 1;
    }
    // Step 2 of the RFC algorithm: smallest code of each length.
    let mut next = [0u32; MAX_CODE_LEN + 1];
    let mut code = 0u32;
    bits = 1;
    while bits <= MAX_CODE_LEN {
        code = (code + bl_count[bits - 1]) << 1;
        next[bits] = code;
        bits += 1;
    }
    // Step 3: consecutive codes within each length, in symbol order.
    i = 0;
    while i < lengths.len() {
        let l = lengths[i] as usize;
        if l != 0 {
            codes[i] = next[l] as u16;
            next[l] += 1;
        } else {
            codes[i] = 0;
        }
        i += 1;
    }
    Ok(())
}

/// Computes code lengths for `freqs`, each at most `limit` bits, writing
/// them to `lengths` (same length as `freqs`, at least 2).
///
/// Guarantees, checked by Kani on small alphabets:
/// * the lengths form a complete prefix code (Kraft sum exactly 1);
/// * every symbol with a nonzero frequency gets a nonzero length;
/// * no length exceeds `limit`.
///
/// When fewer than two symbols are used, symbols 0 and/or 1 are given
/// length-1 codes so the code stays complete.
///
/// The lengths come from a Huffman tree over the frequencies. If the tree
/// is deeper than `limit`, long codes are clamped to `limit` and the Kraft
/// sum is repaired, as in Zstandard's `HUF_setMaxHeight`: while the code is
/// over-subscribed, the least frequent of the longest codes below the limit
/// is lengthened (each step removes code space); then, while code space is
/// unused, the most frequent of the longest codes is shortened. Shortening
/// a longest code never overshoots, because the unused space is a multiple
/// of that code's share, so the result is exactly complete. A suitable code
/// to lengthen always exists when at most `2^limit` symbols are used, which
/// is checked up front.
///
/// No heap allocation is used: scratch space lives in fixed-size arrays,
/// sized for small alphabets (up to 32 symbols) or for the largest DEFLATE
/// alphabet. Both sizes run the same generic code.
pub fn build_lengths(freqs: &[u32], limit: usize, lengths: &mut [u8]) -> Result<(), HuffmanError> {
    if freqs.len() <= SMALL_ALPHABET {
        build_lengths_in::<SMALL_ALPHABET, { 2 * SMALL_ALPHABET }>(freqs, limit, lengths)
    } else {
        build_lengths_in::<MAX_SYMBOLS, { 2 * MAX_SYMBOLS }>(freqs, limit, lengths)
    }
}

/// Scratch capacity used for alphabets of up to this many symbols.
const SMALL_ALPHABET: usize = 32;

/// [`build_lengths`] with scratch arrays for `N` symbols and `M` = 2`N` tree nodes.
fn build_lengths_in<const N: usize, const M: usize>(
    freqs: &[u32],
    limit: usize,
    lengths: &mut [u8],
) -> Result<(), HuffmanError> {
    let n = freqs.len();
    if lengths.len() != n {
        return Err(HuffmanError::LengthMismatch);
    }
    if n > N {
        return Err(HuffmanError::TooManySymbols);
    }
    if limit > MAX_CODE_LEN {
        return Err(HuffmanError::CodeTooLong);
    }
    if n < 2 || limit == 0 {
        return Err(HuffmanError::LimitTooSmall);
    }
    let mut used = [0u16; N];
    let mut m = 0;
    for i in 0..n {
        lengths[i] = 0;
        if freqs[i] != 0 {
            used[m] = i as u16;
            m += 1;
        }
    }
    if m < 2 {
        let s = if m == 1 { used[0] as usize } else { 0 };
        lengths[s] = 1;
        lengths[if s == 0 { 1 } else { 0 }] = 1;
        return Ok(());
    }
    if m > (1usize << limit) {
        return Err(HuffmanError::LimitTooSmall);
    }
    let mut weight = [0u64; N];
    for k in 0..m {
        weight[k] = u64::from(freqs[used[k] as usize]);
    }
    let mut depth = [0u32; N];
    huffman_depths::<N, M>(&weight, m, &mut depth);

    limit_depths(&mut depth[..m], &weight[..m], limit)?;
    for k in 0..m {
        lengths[used[k] as usize] = depth[k] as u8;
    }
    Ok(())
}

/// Limits the depths of a complete code over `depth.len()` symbols to
/// `limit` bits, keeping it complete (the repair step of [`build_lengths`]).
/// Depths over the limit are clamped; then, while the code is
/// over-subscribed, the least frequent of the longest codes below the limit
/// is lengthened; then, while code space is unused, the most frequent of
/// the longest codes is shortened. `weight` only breaks ties.
///
/// Requires 2 ≤ `depth.len()` ≤ 2^`limit` and `weight.len()` =
/// `depth.len()`; then the result is a complete code within the limit
/// (proved for every complete code of up to six symbols by
/// `limit_depths_keeps_code_complete`).
fn limit_depths(depth: &mut [u32], weight: &[u64], limit: usize) -> Result<(), HuffmanError> {
    let m = depth.len();
    // Kraft sum in units of 2^-limit; a complete code sums to `full`.
    let limit32 = limit as u32;
    let full = 1u64 << limit;
    let mut kraft = 0u64;
    for d in depth.iter_mut() {
        if *d > limit32 {
            *d = limit32;
        }
        kraft += 1u64 << (limit32 - *d);
    }
    // Over-subscribed: lengthen the least frequent of the longest codes
    // that are still below the limit. Each step frees 2^-(len+1).
    while kraft > full {
        let mut pick = None;
        for k in 0..m {
            if depth[k] < limit32 {
                let better = match pick {
                    None => true,
                    Some(p) => depth[k] > depth[p] || (depth[k] == depth[p] && weight[k] < weight[p]),
                };
                if better {
                    pick = Some(k);
                }
            }
        }
        let Some(p) = pick else { return Err(HuffmanError::LimitTooSmall) };
        kraft -= 1u64 << (limit32 - depth[p] - 1);
        depth[p] += 1;
    }
    // Under-subscribed: shorten the most frequent of the longest codes.
    // The deficit is a multiple of that code's share, so this lands
    // exactly on a complete code.
    while kraft < full {
        let mut pick = None;
        for k in 0..m {
            if depth[k] > 1 {
                let better = match pick {
                    None => true,
                    Some(p) => depth[k] > depth[p] || (depth[k] == depth[p] && weight[k] > weight[p]),
                };
                if better {
                    pick = Some(k);
                }
            }
        }
        let Some(p) = pick else { return Err(HuffmanError::LimitTooSmall) };
        kraft += 1u64 << (limit32 - depth[p]);
        depth[p] -= 1;
    }
    Ok(())
}

/// Depth of each leaf in a Huffman tree over `weights[..m]` (`2 <= m <= N`).
///
/// The two-queue method: leaves sorted by weight form one queue, merged
/// nodes (created in non-decreasing weight order) form the other, and each
/// step merges the two lightest fronts. O(m log m) for the sort, then
/// linear. Nodes `0..m` are the leaves; each merge creates the next node
/// id, larger than both children's, so the root is the last node and
/// depths can be filled in by walking ids downwards.
fn huffman_depths<const N: usize, const M: usize>(weights: &[u64; N], m: usize, depths: &mut [u32; N]) {
    debug_assert!((2..=N).contains(&m) && M >= 2 * N);
    let mut order = [0u16; N];
    let mut w = [0u64; M];
    let mut parent = [0u16; M];
    // Explicit loops rather than `copy_from_slice`: a copy of symbolic
    // length is far more expensive for the model checker.
    #[allow(clippy::manual_memcpy)]
    for i in 0..m {
        order[i] = i as u16;
        w[i] = weights[i];
    }
    sort_by_weight(&mut order[..m], weights);
    // Leaf queue: order[leaf..m]. Merged queue: node ids merged..next.
    let mut leaf = 0;
    let mut merged = m;
    let mut next = m;
    while (m - leaf) + (next - merged) > 1 {
        let a = take_lightest(&order[..m], &mut leaf, &mut merged, next, &w);
        let b = take_lightest(&order[..m], &mut leaf, &mut merged, next, &w);
        w[next] = w[a].saturating_add(w[b]);
        parent[a] = next as u16;
        parent[b] = next as u16;
        next += 1;
    }
    let mut depth = [0u32; M];
    let mut id = next - 1;
    while id > 0 {
        id -= 1;
        depth[id] = depth[parent[id] as usize] + 1;
    }
    // An explicit loop rather than `copy_from_slice`: a copy of symbolic
    // length is far more expensive for the model checker.
    #[allow(clippy::manual_memcpy)]
    for i in 0..m {
        depths[i] = depth[i];
    }
}

/// Takes the lighter of the two queue fronts (a leaf on ties).
#[inline]
fn take_lightest(order: &[u16], leaf: &mut usize, merged: &mut usize, next: usize, w: &[u64]) -> usize {
    let leaf_ok = *leaf < order.len();
    let merged_ok = *merged < next;
    if leaf_ok && (!merged_ok || w[order[*leaf] as usize] <= w[*merged]) {
        *leaf += 1;
        order[*leaf - 1] as usize
    } else {
        *merged += 1;
        *merged - 1
    }
}

/// Sorts symbol indices by (weight, index) with heapsort: O(n log n) and
/// no allocation. `sort_by_weight_sorts` proves it; the length-builder
/// proofs use [`sort_by_weight_spec`] in its place.
fn sort_by_weight(idx: &mut [u16], w: &[u64]) {
    let n = idx.len();
    let mut start = n / 2;
    while start > 0 {
        start -= 1;
        sift_down(idx, start, n, w);
    }
    let mut end = n;
    while end > 1 {
        end -= 1;
        idx.swap(0, end);
        sift_down(idx, 0, end, w);
    }
}

/// The specification of [`sort_by_weight`] for distinct indices (as
/// `huffman_depths` passes): the permutation of `idx` sorted by
/// (weight, index). Keys are distinct, so that permutation exists and is
/// unique; choosing it nondeterministically under that assumption is
/// exactly the function's behaviour, without its loops.
#[cfg_attr(mutants, mutants::skip)] // proof code: the checker, not code under test
#[cfg(kani)]
#[cfg_attr(kani_sample, allow(dead_code))] // stubs are not applied when sampling
fn sort_by_weight_spec(idx: &mut [u16], w: &[u64]) {
    let n = idx.len();
    assert!(n <= 8, "sort_by_weight_spec models at most 8 entries");
    let mut orig = [0u16; 8];
    let mut taken = [false; 8];
    #[allow(clippy::manual_memcpy)] // a symbolic-length copy costs the model checker more
    for i in 0..n {
        orig[i] = idx[i];
    }
    for i in 0..n {
        let p: usize = kani::any_where(|&p: &usize| p < n);
        kani::assume(!taken[p]);
        taken[p] = true;
        idx[i] = orig[p];
    }
    for i in 1..n {
        let (a, b) = (idx[i - 1], idx[i]);
        kani::assume((w[a as usize], a) < (w[b as usize], b));
    }
}

#[inline]
fn heavier(a: u16, b: u16, w: &[u64]) -> bool {
    (w[a as usize], a) > (w[b as usize], b)
}

fn sift_down(idx: &mut [u16], mut root: usize, end: usize, w: &[u64]) {
    loop {
        let mut child = 2 * root + 1;
        if child >= end {
            return;
        }
        if child + 1 < end && heavier(idx[child + 1], idx[child], w) {
            child += 1;
        }
        if heavier(idx[child], idx[root], w) {
            idx.swap(root, child);
            root = child;
        } else {
            return;
        }
    }
}

/// Bits looked up at once by [`Decoder::decode`]'s table. Codes up to
/// this long (all fixed-Huffman codes, and nearly all dynamic ones) decode
/// with one lookup; longer ones fall back to the canonical bit-by-bit walk.
pub const FAST_BITS: u32 = 10;
const FAST_SIZE: usize = 1 << FAST_BITS;

/// Canonical Huffman decoder, bit by bit: per-length counts plus a symbol
/// list sorted by code, as in zlib's `puff.c`. This is the reference
/// decoder; [`Decoder`] adds a lookup table on top of it.
#[derive(Clone, Debug)]
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
    /// A decoder with no codes (every decode fails).
    pub const EMPTY: Self = Self { count: [0; MAX_CODE_LEN + 1], symbol: [0; MAX_SYMBOLS], coded: 0, left: 1 << 15 };

    /// Builds a decoder for the given code lengths.
    ///
    /// Over-subscribed lengths are rejected. Incomplete codes are accepted
    /// here; [`is_complete`](Self::is_complete) and
    /// [`is_single_unit_code`](Self::is_single_unit_code) let the caller
    /// apply DEFLATE's rules.
    pub const fn new(lengths: &[u8]) -> Result<Self, HuffmanError> {
        let mut d = Self::EMPTY;
        match d.rebuild(lengths) {
            Ok(()) => Ok(d),
            Err(e) => Err(e),
        }
    }

    /// Rebuilds this decoder in place for new code lengths (as [`new`],
    /// without moving a decoder around). On error the decoder decodes
    /// nothing.
    ///
    /// [`new`]: Self::new
    pub const fn rebuild(&mut self, lengths: &[u8]) -> Result<(), HuffmanError> {
        let n = lengths.len();
        self.count = [0; MAX_CODE_LEN + 1];
        self.coded = 0;
        self.left = 1 << 15;
        if n > MAX_SYMBOLS {
            return Err(HuffmanError::TooManySymbols);
        }
        let mut count = [0u16; MAX_CODE_LEN + 1];
        let mut s = 0;
        while s < n {
            let l = lengths[s] as usize;
            if l > MAX_CODE_LEN {
                return Err(HuffmanError::CodeTooLong);
            }
            count[l] += 1;
            s += 1;
        }
        let coded = n as u16 - count[0];
        count[0] = 0;
        let mut left: i32 = 1;
        let mut len = 1;
        while len <= MAX_CODE_LEN {
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
        len = 1;
        while len <= MAX_CODE_LEN {
            offs[len + 1] = offs[len] + count[len];
            len += 1;
        }
        s = 0;
        while s < n {
            let l = lengths[s] as usize;
            if l != 0 {
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

    /// True for a code with exactly one symbol, of length 1: the only
    /// incomplete code DEFLATE decoders (zlib included) accept.
    pub const fn is_single_unit_code(&self) -> bool {
        self.coded == 1 && self.count[1] == 1
    }

    /// Reads one symbol bit by bit. Fails with `UnexpectedEof` if the input
    /// runs out and `InvalidSymbol` if the bits match no code.
    ///
    /// The fifteen per-length steps are written out rather than looped, so
    /// that the model checker's loop bound for the callers does not have to
    /// cover them.
    #[inline(never)]
    #[allow(unused_assignments)]
    pub fn decode(&self, br: &mut BitReader<'_>) -> Result<u16, InflateError> {
        // Invariant at each step: first <= code, first is the smallest
        // canonical code of the current length, and index is the position
        // of that code in `symbol`.
        let mut code: u32 = 0;
        let mut first: u32 = 0;
        let mut index: u32 = 0;
        macro_rules! step {
            ($len:expr) => {
                code |= br.read_bit()?;
                let count = u32::from(self.count[$len]);
                if code < first + count {
                    return Ok(self.symbol[(index + (code - first)) as usize]);
                }
                index += count;
                first = (first + count) << 1;
                code <<= 1;
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

/// Huffman decoder: the [`Canonical`] decoder plus a lookup table indexed
/// by the next `fast_bits` input bits. [`decode`](Self::decode) uses the
/// table and falls back to the canonical walk. Kani proves the two decode
/// identically, so everything proven about the canonical decoder carries
/// over.
#[derive(Clone, Debug)]
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
    /// A decoder with no codes (every decode fails).
    pub const EMPTY: Self = Self { canon: Canonical::EMPTY, fast: [0; FAST_SIZE], fast_bits: FAST_BITS };

    /// Like [`new`](Self::new) with a lookup table of `fast_bits` bits
    /// (clamped to [`FAST_BITS`]). Proofs use small tables.
    pub const fn with_fast_bits(lengths: &[u8], fast_bits: u32) -> Result<Self, HuffmanError> {
        let mut d = Self::EMPTY;
        match d.rebuild_with_fast_bits(lengths, fast_bits) {
            Ok(()) => Ok(d),
            Err(e) => Err(e),
        }
    }

    /// Builds a decoder for the given code lengths (see [`Canonical::new`]).
    pub const fn new(lengths: &[u8]) -> Result<Self, HuffmanError> {
        let mut d = Self::EMPTY;
        match d.rebuild(lengths) {
            Ok(()) => Ok(d),
            Err(e) => Err(e),
        }
    }

    /// Rebuilds this decoder in place for new code lengths. On error the
    /// decoder decodes nothing.
    pub const fn rebuild(&mut self, lengths: &[u8]) -> Result<(), HuffmanError> {
        self.rebuild_with_fast_bits(lengths, FAST_BITS)
    }

    /// [`rebuild`](Self::rebuild) with a lookup table of `fast_bits` bits.
    pub const fn rebuild_with_fast_bits(&mut self, lengths: &[u8], fast_bits: u32) -> Result<(), HuffmanError> {
        self.fast = [0; FAST_SIZE];
        let fb = if fast_bits > FAST_BITS { FAST_BITS } else { fast_bits };
        self.fast_bits = fb;
        if let Err(e) = self.canon.rebuild(lengths) {
            return Err(e);
        }
        // Each code of at most fb bits fills every entry whose low bits
        // spell it (codes are packed bit-reversed). The codes are the
        // canonical ones, assigned in symbol order within each length.
        let count = &self.canon.count;
        let mut next = [0u32; MAX_CODE_LEN + 1];
        let mut code = 0u32;
        let mut len = 1;
        while len <= MAX_CODE_LEN {
            code = (code + count[len - 1] as u32) << 1;
            next[len] = code;
            len += 1;
        }
        let mut s = 0;
        while s < lengths.len() {
            let l = lengths[s] as usize;
            if l != 0 {
                let c = next[l];
                next[l] += 1;
                if l as u32 <= fb {
                    let mut idx = crate::bitio::reverse_bits(c as u16, l as u32) as usize;
                    while idx < (1 << fb) {
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

    /// True for a code with exactly one symbol, of length 1: the only
    /// incomplete code DEFLATE decoders (zlib included) accept.
    pub const fn is_single_unit_code(&self) -> bool {
        self.canon.is_single_unit_code()
    }

    /// Reads one symbol. Fails with `UnexpectedEof` if the input runs out
    /// and `InvalidSymbol` if the bits match no code.
    ///
    /// One table lookup decodes any code of at most `fast_bits` bits that
    /// is fully present in the input; everything else (longer codes, input
    /// running out, invalid bits) goes to the canonical walk, which decides
    /// it exactly as if the table were not there.
    #[inline(always)]
    pub fn decode(&self, br: &mut BitReader<'_>) -> Result<u16, InflateError> {
        let e = self.fast[br.peek(self.fast_bits) as usize];
        let len = u32::from(e >> 12);
        if len != 0 && len <= br.available() {
            br.consume(len);
            return Ok(e & 0x0fff);
        }
        self.canon.decode(br)
    }

    /// Reads one symbol with the canonical walk only (the reference).
    pub fn decode_slow(&self, br: &mut BitReader<'_>) -> Result<u16, InflateError> {
        self.canon.decode(br)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_example_codes() {
        // RFC 1951 §3.2.2 example: lengths (3,3,3,3,3,2,4,4) for A..H.
        let lengths = [3, 3, 3, 3, 3, 2, 4, 4];
        let mut codes = [0u16; 8];
        canonical_codes(&lengths, &mut codes).unwrap();
        assert_eq!(codes, [0b010, 0b011, 0b100, 0b101, 0b110, 0b00, 0b1110, 0b1111]);
    }

    #[test]
    fn build_lengths_respects_limit_on_fibonacci_weights() {
        // Fibonacci frequencies produce maximally deep Huffman trees.
        let mut freqs = [0u32; 40];
        let (mut a, mut b) = (1u32, 1u32);
        for f in freqs.iter_mut() {
            *f = a;
            let c = a.saturating_add(b);
            a = b;
            b = c;
        }
        for limit in [6usize, 7, 9, 15] {
            let mut lengths = [0u8; 40];
            build_lengths(&freqs, limit, &mut lengths).unwrap();
            assert!(lengths.iter().all(|&l| l >= 1 && l as usize <= limit));
            assert!(is_complete(&lengths), "limit {limit}");
        }
    }

    #[test]
    fn build_lengths_on_fibonacci_weights_every_size_and_limit() {
        // Every alphabet size 2-40 and every feasible limit, with Fibonacci
        // weights in increasing, decreasing and interleaved order. Small
        // limits on these weights make the clamp overshoot, so the
        // under-subscribed half of the repair runs (from 6 symbols at
        // limit 3).
        for n in 2..=40usize {
            let mut fib = vec![0u32; n];
            let (mut a, mut b) = (1u32, 1u32);
            for f in fib.iter_mut() {
                *f = a;
                let c = a.saturating_add(b);
                a = b;
                b = c;
            }
            let rev: Vec<u32> = fib.iter().rev().copied().collect();
            let mixed: Vec<u32> = (0..n).map(|i| fib[(i * 7) % n]).collect();
            for freqs in [&fib, &rev, &mixed] {
                for limit in 1..=MAX_CODE_LEN {
                    if n > 1 << limit {
                        continue;
                    }
                    let mut lengths = vec![0u8; n];
                    build_lengths(freqs, limit, &mut lengths).unwrap();
                    assert!(lengths.iter().all(|&l| l >= 1 && l as usize <= limit), "n {n} limit {limit}");
                    assert!(is_complete(&lengths), "n {n} limit {limit}: {lengths:?}");
                }
            }
        }
    }

    #[test]
    fn limit_depths_under_subscribed_example() {
        // Depths of a Huffman tree over weights 1,1,2,3,5,8, limited to 3
        // bits: clamping gives 3,3,3,3,2,1 (over-subscribed); lengthening
        // the 1 overshoots; shortening one 3 completes the code.
        let mut depth = [5u32, 5, 4, 3, 2, 1];
        limit_depths(&mut depth, &[1, 1, 2, 3, 5, 8], 3).unwrap();
        let lengths = depth.map(|d| d as u8);
        assert!(is_complete(&lengths), "{depth:?}");
        assert!(depth.iter().all(|&d| (1..=3).contains(&d)));
    }

    #[test]
    fn limit_depths_picks_by_frequency() {
        // Ties among the longest codes go by frequency: the least frequent
        // is lengthened and the most frequent shortened, as documented.
        // Clamping [1,2,3,4,5,5] to 3 bits gives [1,2,3,3,3,3]; lengthening
        // the 2 and then the 1 overshoots, so one 3 is shortened: the most
        // frequent (weight 5), not merely the last.
        let mut d = [1u32, 2, 3, 4, 5, 5];
        limit_depths(&mut d, &[8, 5, 3, 2, 1, 1], 3).unwrap();
        assert_eq!(d, [2, 2, 3, 3, 3, 3]);
        // Over-subscribed with a tie below the limit: [2,2,2,3,4,4] clamped
        // to 3 bits is over-subscribed by one unit; of the three 2s, the
        // least frequent (weight 1, in the middle) is lengthened.
        let mut d = [2u32, 2, 2, 3, 4, 4];
        limit_depths(&mut d, &[9, 1, 7, 3, 2, 2], 3).unwrap();
        assert_eq!(d, [2, 3, 2, 3, 3, 3]);
    }

    #[test]
    fn build_lengths_full_litlen_alphabet() {
        let freqs: Vec<u32> = (0..286u32).map(|i| (i * 7919) % 1000 + 1).collect();
        let mut lengths = vec![0u8; 286];
        build_lengths(&freqs, 15, &mut lengths).unwrap();
        assert!(is_complete(&lengths));
        let mut cl = [0u8; 19];
        build_lengths(&[1u32; 19], 7, &mut cl).unwrap();
        assert!(is_complete(&cl));
    }
}

#[cfg_attr(mutants, mutants::skip)] // proof code: the checker, not code under test
#[cfg(kani)]
mod proofs {
    use super::*;
    use crate::bitio::reverse_bits;

    const N: usize = 6;

    fn any_lengths<const K: usize>(max: u8) -> [u8; K] {
        core::array::from_fn(|_| kani::any_where(|&x: &u8| x <= max))
    }

    /// Canonical codes of any lengths with Kraft sum ≤ 1 fit in their
    /// lengths and are prefix-free: no code is a prefix of another.
    #[kani::proof]
    #[kani::unwind(17)]
    fn canonical_codes_are_prefix_free() {
        let lengths: [u8; N] = any_lengths(MAX_CODE_LEN as u8);
        let mut codes = [0u16; N];
        let ok = canonical_codes(&lengths, &mut codes).is_ok();
        assert_eq!(ok, kraft_sum(&lengths).unwrap() <= 1 << MAX_CODE_LEN);
        if !ok {
            return;
        }
        for i in 0..N {
            if lengths[i] == 0 {
                continue;
            }
            assert!(u32::from(codes[i]) < (1u32 << lengths[i]));
            for j in 0..N {
                if i == j || lengths[j] == 0 || lengths[i] > lengths[j] {
                    continue;
                }
                let shift = lengths[j] - lengths[i];
                assert_ne!(codes[j] >> shift, codes[i]);
            }
        }
    }

    fn check_lengths(freqs: &[u32], lengths: &[u8], limit: usize) {
        assert!(is_complete(lengths));
        for i in 0..freqs.len() {
            assert!(lengths[i] as usize <= limit);
            if freqs[i] != 0 {
                assert!(lengths[i] != 0);
            }
        }
    }

    /// The length builder always yields a complete code within the limit
    /// that codes every used symbol, for five symbols with frequencies
    /// 0-255 and any limit 3-15. A limit of 3 forces the clamp-and-repair
    /// path; its under-subscribed half needs six symbols and is proved
    /// separately (`limit_depths_keeps_code_complete`). (`build_lengths`
    /// runs this same generic function with larger scratch arrays.)
    #[kani::proof]
    #[kani::stub(sort_by_weight, sort_by_weight_spec)]
    #[kani::unwind(11)]
    fn build_lengths_complete_and_limited() {
        let f: [u8; 5] = kani::any();
        let freqs = f.map(u32::from);
        let limit: usize = kani::any();
        kani::assume(limit >= 3 && limit <= MAX_CODE_LEN);
        let mut lengths = [0u8; 5];
        assert!(build_lengths_in::<5, 10>(&freqs, limit, &mut lengths).is_ok());
        check_lengths(&freqs, &lengths, limit);
    }

    /// The same over the full `u32` frequency range, for three symbols.
    #[kani::proof]
    #[kani::stub(sort_by_weight, sort_by_weight_spec)]
    #[kani::unwind(8)]
    fn build_lengths_full_range_frequencies() {
        let freqs: [u32; 3] = kani::any();
        let limit: usize = kani::any();
        kani::assume(limit >= 2 && limit <= MAX_CODE_LEN);
        let mut lengths = [0u8; 3];
        assert!(build_lengths_in::<3, 6>(&freqs, limit, &mut lengths).is_ok());
        check_lengths(&freqs, &lengths, limit);
    }

    /// The tightest feasible case: four symbols in two bits, where every
    /// skewed distribution needs repair.
    #[kani::proof]
    #[kani::stub(sort_by_weight, sort_by_weight_spec)]
    #[kani::unwind(11)]
    fn build_lengths_tight_limit() {
        let f: [u8; 4] = kani::any();
        let freqs = f.map(u32::from);
        let mut lengths = [0u8; 4];
        assert!(build_lengths_in::<4, 8>(&freqs, 2, &mut lengths).is_ok());
        check_lengths(&freqs, &lengths, 2);
    }

    /// Requests that cannot be met are refused rather than producing a bad
    /// code: five used symbols cannot fit in two bits.
    #[kani::proof]
    #[kani::stub(sort_by_weight, sort_by_weight_spec)]
    #[kani::unwind(11)]
    fn build_lengths_rejects_impossible_limit() {
        let f: [u8; 5] = kani::any();
        let freqs = f.map(u32::from);
        for &x in freqs.iter() {
            kani::assume(x != 0);
        }
        let mut lengths = [0u8; 5];
        assert_eq!(build_lengths_in::<5, 10>(&freqs, 2, &mut lengths), Err(HuffmanError::LimitTooSmall));
    }

    /// The length-limit repair on its own, for every complete code of six
    /// symbols, weights 0-3 (they only break ties) and the limits of 3 and
    /// 4 bits that six symbols allow below their natural depth: the result
    /// is a complete code within the limit. Six symbols are the fewest
    /// that reach the under-subscribed half of the repair (for example
    /// depths 5,5,4,3,2,1 at limit 3), which the builder harnesses above,
    /// at five symbols, never do. The repair does not depend on how the
    /// depths were built, so this covers it for the builder too.
    #[kani::proof]
    #[kani::unwind(14)]
    fn limit_depths_keeps_code_complete() {
        // Any complete code of six symbols: start from one leaf at depth 0
        // and split a chosen leaf in two, five times. Every code tree
        // arises this way.
        let mut depth = [0u32; 6];
        let mut leaves = 1;
        while leaves < 6 {
            let i: usize = kani::any_where(|&i: &usize| i < leaves);
            depth[i] += 1;
            depth[leaves] = depth[i];
            leaves += 1;
        }
        let mut weight = [0u64; 6];
        for w in weight.iter_mut() {
            *w = kani::any_where(|&x: &u64| x < 4);
        }
        let limit: usize = kani::any_where(|&l: &usize| l == 3 || l == 4);
        assert!(limit_depths(&mut depth, &weight, limit).is_ok());
        let mut sum = 0u32;
        for k in 0..6 {
            assert!(depth[k] >= 1 && depth[k] as usize <= limit);
            sum += 1 << (limit as u32 - depth[k]);
        }
        assert_eq!(sum, 1 << limit);
    }

    /// The heapsort behind the length builder sorts: for any weights and
    /// any 0-5 distinct indices, the result is a permutation of them
    /// sorted by (weight, index), which is what `sort_by_weight_spec`
    /// (used by the builder proofs) assumes.
    #[kani::proof]
    #[kani::unwind(7)]
    fn sort_by_weight_sorts() {
        let w: [u64; 5] = kani::any();
        let n: usize = kani::any_where(|&n: &usize| n <= 5);
        let mut idx = [0u16, 1, 2, 3, 4];
        sort_by_weight(&mut idx[..n], &w);
        for i in 0..n {
            assert!((idx[i] as usize) < n);
            for j in 0..i {
                assert_ne!(idx[i], idx[j]);
            }
            if i > 0 {
                let (a, b) = (idx[i - 1], idx[i]);
                assert!((w[a as usize], a) < (w[b as usize], b));
            }
        }
    }

    /// The decoder builder never panics, and accepts exactly the lengths
    /// whose Kraft sum is at most 1.
    #[kani::proof]
    #[kani::unwind(17)]
    fn decoder_new_never_panics() {
        let lengths: [u8; 8] = kani::any();
        // A 4-bit table keeps the fill loop small; `new` runs the same code
        // with a 10-bit table.
        let res = Decoder::with_fast_bits(&lengths, 4);
        match kraft_sum(&lengths) {
            Err(_) => assert!(res.is_err()),
            Ok(k) => {
                assert_eq!(res.is_ok(), k <= 1 << MAX_CODE_LEN);
                if let Ok(d) = res {
                    assert_eq!(d.is_complete(), k == 1 << MAX_CODE_LEN);
                }
            }
        }
    }

    /// Core round trip: for any valid lengths and any coded symbol, packing
    /// the symbol's canonical code bit-reversed (as the encoder does) and
    /// decoding it returns the symbol and consumes exactly its length.
    #[kani::proof]
    #[kani::unwind(17)]
    fn encode_then_decode_symbol() {
        let lengths: [u8; N] = any_lengths(MAX_CODE_LEN as u8);
        let mut codes = [0u16; N];
        kani::assume(canonical_codes(&lengths, &mut codes).is_ok());
        // The canonical (reference) decoder; `fast_decode_matches_slow`
        // shows the table-driven `Decoder` agrees with it.
        let dec = Canonical::new(&lengths).unwrap();
        let s: usize = kani::any_where(|&s: &usize| s < N);
        kani::assume(lengths[s] != 0);
        // Pack the code as `BitWriter::write_bits` does (proven in `bitio`),
        // followed by 16 arbitrary bits, and decode it.
        let len = u32::from(lengths[s]);
        let trailing: u16 = kani::any();
        let word = u32::from(reverse_bits(codes[s], len)) | (u32::from(trailing) << len);
        let bytes = word.to_le_bytes();
        let mut r = BitReader::new(&bytes);
        assert_eq!(dec.decode(&mut r), Ok(s as u16));
        assert_eq!(r.read_bits(16), Ok(u32::from(trailing)));
    }

    /// Decoding arbitrary bits with arbitrary accepted lengths never panics
    /// and only ever yields symbols that have a code.
    #[kani::proof]
    #[kani::unwind(17)]
    fn decode_arbitrary_bits() {
        let lengths: [u8; N] = any_lengths(MAX_CODE_LEN as u8);
        // The canonical decoder, as in `encode_then_decode_symbol`.
        let Ok(dec) = Canonical::new(&lengths) else { return };
        let data: [u8; 2] = kani::any();
        let mut r = BitReader::new(&data);
        if let Ok(s) = dec.decode(&mut r) {
            assert!((s as usize) < N);
            assert!(lengths[s as usize] != 0);
        }
    }

    /// The table-driven decoder is the bit-by-bit reference decoder: for
    /// any valid lengths over 4 symbols (up to 6 bits), any table width of
    /// 1-4 bits, and any 0-1 bytes of input, `decode` and `decode_slow`
    /// return the same result and, on success, consume the same number of
    /// bits. Codes shorter than, as long as and longer than the table, and
    /// input ending mid-code, are all covered.
    #[kani::proof]
    #[kani::unwind(17)]
    fn fast_decode_matches_slow() {
        let lengths: [u8; 4] = any_lengths(6);
        let fast_bits: u32 = kani::any_where(|&b: &u32| (1..=4).contains(&b));
        let Ok(dec) = Decoder::with_fast_bits(&lengths, fast_bits) else { return };
        let data: [u8; 1] = kani::any();
        let len: usize = kani::any_where(|&l: &usize| l <= 1);
        check_fast_matches_slow(&dec, &data[..len]);
    }

    /// The same for 5 symbols of up to 9 bits and 0-2 bytes of input.
    #[cfg(any(feature = "kani-extended", kani_sample))] // too large for CI runners
    #[kani::proof]
    #[kani::unwind(17)]
    fn fast_decode_matches_slow_wide() {
        let lengths: [u8; 5] = any_lengths(9);
        let fast_bits: u32 = kani::any_where(|&b: &u32| (1..=4).contains(&b));
        let Ok(dec) = Decoder::with_fast_bits(&lengths, fast_bits) else { return };
        let data: [u8; 2] = kani::any();
        let len: usize = kani::any_where(|&l: &usize| l <= 2);
        check_fast_matches_slow(&dec, &data[..len]);
    }

    fn check_fast_matches_slow(dec: &Decoder, data: &[u8]) {
        let mut fast = BitReader::new(data);
        let mut slow = BitReader::new(data);
        let a = dec.decode(&mut fast);
        let b = dec.decode_slow(&mut slow);
        assert_eq!(a, b);
        if a.is_ok() {
            assert_eq!(fast.bits_remaining(), slow.bits_remaining());
        }
    }
}
