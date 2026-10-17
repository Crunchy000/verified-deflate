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

/// After this many halvings of the frequencies, [`build_lengths`] falls
/// back to equal weights, which always fit (see its documentation).
const MAX_HALVINGS: usize = 8;

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
/// The lengths come from a Huffman tree built over the frequencies. If the
/// tree is deeper than `limit`, the frequencies are halved (keeping them
/// nonzero) and the tree rebuilt. After eight halvings every
/// weight is set to 1: a Huffman code for equal weights has lengths in
/// `{k, k+1}` with `k = floor(log2 n)`, so it fits whenever
/// `n <= 2^limit`, which is checked up front.
pub fn build_lengths(freqs: &[u32], limit: usize, lengths: &mut [u8]) -> Result<(), HuffmanError> {
    let n = freqs.len();
    if lengths.len() != n {
        return Err(HuffmanError::LengthMismatch);
    }
    if limit > MAX_CODE_LEN {
        return Err(HuffmanError::CodeTooLong);
    }
    if n < 2 || limit == 0 {
        return Err(HuffmanError::LimitTooSmall);
    }
    for l in lengths.iter_mut() {
        *l = 0;
    }
    let mut used: Vec<usize> = Vec::new();
    for (i, &f) in freqs.iter().enumerate() {
        if f != 0 {
            used.push(i);
        }
    }
    if used.len() < 2 {
        let s = used.first().copied().unwrap_or(0);
        lengths[s] = 1;
        lengths[if s == 0 { 1 } else { 0 }] = 1;
        return Ok(());
    }
    if used.len() > (1usize << limit) {
        return Err(HuffmanError::LimitTooSmall);
    }
    let mut weights: Vec<u64> = used.iter().map(|&i| u64::from(freqs[i])).collect();
    let mut depths = vec![0u32; used.len()];
    let mut attempt = 0;
    while attempt <= MAX_HALVINGS {
        huffman_depths(&weights, &mut depths);
        if depths.iter().all(|&d| d as usize <= limit) {
            for (k, &s) in used.iter().enumerate() {
                lengths[s] = depths[k] as u8;
            }
            return Ok(());
        }
        let flatten = attempt + 1 == MAX_HALVINGS;
        for w in weights.iter_mut() {
            *w = if flatten { 1 } else { (*w >> 1) | 1 };
        }
        attempt += 1;
    }
    Err(HuffmanError::LimitTooSmall)
}

/// Depth of each leaf in a Huffman tree over `weights` (at least two).
///
/// Nodes `0..m` are the leaves; each merge appends a node whose id is
/// larger than both children's, so the root is the last node and depths
/// can be filled in by walking ids downwards.
fn huffman_depths(weights: &[u64], depths: &mut [u32]) {
    let m = weights.len();
    debug_assert!(m >= 2 && depths.len() == m);
    let total = 2 * m - 1;
    let mut w: Vec<u64> = Vec::with_capacity(total);
    w.extend_from_slice(weights);
    let mut parent = vec![0usize; total];
    let mut active: Vec<usize> = (0..m).collect();
    while active.len() > 1 {
        let a = take_min(&mut active, &w);
        let b = take_min(&mut active, &w);
        let id = w.len();
        w.push(w[a].saturating_add(w[b]));
        parent[a] = id;
        parent[b] = id;
        active.push(id);
    }
    let mut depth = vec![0u32; total];
    let mut id = total - 1;
    while id > 0 {
        id -= 1;
        depth[id] = depth[parent[id]] + 1;
    }
    depths.copy_from_slice(&depth[..m]);
}

/// Removes and returns the active node with the smallest weight.
fn take_min(active: &mut Vec<usize>, w: &[u64]) -> usize {
    let mut best = 0;
    for i in 1..active.len() {
        if w[active[i]] < w[active[best]] {
            best = i;
        }
    }
    active.swap_remove(best)
}

/// Canonical Huffman decoder using per-length counts and a symbol list
/// sorted by code (the representation used by zlib's `puff.c`).
#[derive(Clone, Debug)]
pub struct Decoder {
    /// `count[l]` = number of symbols with code length `l` (`count[0]` unused).
    count: [u16; MAX_CODE_LEN + 1],
    /// Coded symbols ordered by (length, symbol), i.e. by canonical code.
    symbol: [u16; MAX_SYMBOLS],
    /// Number of symbols with a nonzero length.
    coded: u16,
    /// Unused code space in units of `2^-15`; zero for a complete code.
    left: u16,
}

impl Decoder {
    /// Builds a decoder for the given code lengths.
    ///
    /// Over-subscribed lengths are rejected. Incomplete codes are accepted
    /// here; [`is_complete`](Self::is_complete) and
    /// [`is_single_unit_code`](Self::is_single_unit_code) let the caller
    /// apply DEFLATE's rules.
    pub const fn new(lengths: &[u8]) -> Result<Self, HuffmanError> {
        let n = lengths.len();
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
        // offs[l] = index in `symbol` of the first code of length l.
        let mut offs = [0u16; MAX_CODE_LEN + 2];
        len = 1;
        while len <= MAX_CODE_LEN {
            offs[len + 1] = offs[len] + count[len];
            len += 1;
        }
        let mut symbol = [0u16; MAX_SYMBOLS];
        s = 0;
        while s < n {
            let l = lengths[s] as usize;
            if l != 0 {
                symbol[offs[l] as usize] = s as u16;
                offs[l] += 1;
            }
            s += 1;
        }
        Ok(Self { count, symbol, coded, left: left as u16 })
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

    /// Reads one symbol. Fails with `UnexpectedEof` if the input runs out
    /// and `InvalidSymbol` if the bits match no code.
    #[inline]
    pub fn decode(&self, br: &mut BitReader<'_>) -> Result<u16, InflateError> {
        // Invariant at each check: first <= code, and first is the smallest
        // canonical code of length `len`; index is the position of that code
        // in `symbol`.
        let mut code: u32 = 0;
        let mut first: u32 = 0;
        let mut index: u32 = 0;
        let mut len = 1;
        while len <= MAX_CODE_LEN {
            code |= br.read_bit()?;
            let count = u32::from(self.count[len]);
            if code < first + count {
                return Ok(self.symbol[(index + (code - first)) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
            len += 1;
        }
        Err(InflateError::InvalidSymbol)
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

#[cfg(kani)]
mod proofs {
    use super::*;
    use crate::bitio::{reverse_bits, BitWriter};

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

    /// `build_lengths` always yields a complete code within the limit that
    /// codes every used symbol. A limit of 3 on five symbols forces the
    /// frequency-halving path.
    #[kani::proof]
    #[kani::unwind(11)]
    fn build_lengths_complete_and_limited() {
        let freqs: [u32; 5] = kani::any();
        let limit: usize = kani::any();
        kani::assume(limit >= 3 && limit <= MAX_CODE_LEN);
        let mut lengths = [0u8; 5];
        assert!(build_lengths(&freqs, limit, &mut lengths).is_ok());
        assert!(is_complete(&lengths));
        for i in 0..5 {
            assert!(lengths[i] as usize <= limit);
            if freqs[i] != 0 {
                assert!(lengths[i] != 0);
            }
        }
    }

    /// Small-weight version that can actually reach deep trees and the
    /// flattening fallback: limit 2 with four symbols.
    #[kani::proof]
    #[kani::unwind(11)]
    fn build_lengths_tight_limit() {
        let freqs: [u32; 4] = kani::any();
        let mut lengths = [0u8; 4];
        assert!(build_lengths(&freqs, 2, &mut lengths).is_ok());
        assert!(is_complete(&lengths));
        for i in 0..4 {
            assert!(lengths[i] <= 2);
            if freqs[i] != 0 {
                assert!(lengths[i] != 0);
            }
        }
    }

    /// Requests that cannot be met are refused rather than producing a bad
    /// code: five used symbols cannot fit in two bits.
    #[kani::proof]
    #[kani::unwind(11)]
    fn build_lengths_rejects_impossible_limit() {
        let freqs: [u32; 5] = kani::any();
        for &f in freqs.iter() {
            kani::assume(f != 0);
        }
        let mut lengths = [0u8; 5];
        assert_eq!(build_lengths(&freqs, 2, &mut lengths), Err(HuffmanError::LimitTooSmall));
    }

    /// The decoder builder never panics, and accepts exactly the lengths
    /// whose Kraft sum is at most 1.
    #[kani::proof]
    #[kani::unwind(17)]
    fn decoder_new_never_panics() {
        let lengths: [u8; 8] = kani::any();
        let res = Decoder::new(&lengths);
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

    /// Core round trip: for any valid lengths and any coded symbol, writing
    /// the symbol's canonical code (bit-reversed, as the encoder does) and
    /// decoding it returns the symbol and consumes exactly its length.
    #[kani::proof]
    #[kani::unwind(17)]
    fn encode_then_decode_symbol() {
        let lengths: [u8; N] = any_lengths(MAX_CODE_LEN as u8);
        let mut codes = [0u16; N];
        kani::assume(canonical_codes(&lengths, &mut codes).is_ok());
        let dec = Decoder::new(&lengths).unwrap();
        let s: usize = kani::any_where(|&s: &usize| s < N);
        kani::assume(lengths[s] != 0);
        let trailing: u32 = kani::any();
        let mut w = BitWriter::new();
        let len = u32::from(lengths[s]);
        w.write_bits(u32::from(reverse_bits(codes[s], len)), len);
        w.write_bits(trailing, 16);
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        assert_eq!(dec.decode(&mut r), Ok(s as u16));
        assert_eq!(r.read_bits(16), Ok(trailing & 0xffff));
    }

    /// Decoding arbitrary bits with arbitrary accepted lengths never panics
    /// and only ever yields symbols that have a code.
    #[kani::proof]
    #[kani::unwind(17)]
    fn decode_arbitrary_bits() {
        let lengths: [u8; N] = any_lengths(MAX_CODE_LEN as u8);
        let Ok(dec) = Decoder::new(&lengths) else { return };
        let data: [u8; 2] = kani::any();
        let mut r = BitReader::new(&data);
        if let Ok(s) = dec.decode(&mut r) {
            assert!((s as usize) < N);
            assert!(lengths[s as usize] != 0);
        }
    }
}
