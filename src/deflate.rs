//! DEFLATE compression (RFC 1951 §3.2).
//!
//! The input is tokenized by the LZ77 [`Matcher`] in chunks of up to
//! [`BLOCK_TOKENS`] tokens. For each chunk the encoder computes the exact
//! size of a stored, a fixed-Huffman and a dynamic-Huffman block and writes
//! the smallest (or the one forced by [`Strategy`]).

use crate::bitio::{reverse_bits, BitWriter};
use crate::huffman::{build_lengths, canonical_codes, HuffmanError, MAX_CODE_LEN};
use crate::inflate::decompress_with_limit;
use crate::lz77::{MatchConfig, Matcher, Token};
use crate::tables::{
    codelen_extra_bits, distance_code, fixed_litlen_lengths, length_code, CODELEN_ORDER, DIST_BASE, DIST_EXTRA,
    END_OF_BLOCK, FIXED_DIST_LENGTHS, LENGTH_BASE, LENGTH_EXTRA, NUM_CODELEN, NUM_DIST, NUM_LITLEN,
};

/// Largest payload of a single stored block.
pub const MAX_STORED_BLOCK: usize = 65_535;

/// Maximum LZ77 tokens per Huffman block.
pub const BLOCK_TOKENS: usize = 16_384;

/// Longest code allowed in the code-length code (§3.2.7: 3-bit lengths).
const MAX_CODELEN_LEN: usize = 7;

/// Which block types the encoder may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Strategy {
    /// Pick the smallest of stored, fixed and dynamic for each block.
    #[default]
    Auto,
    /// Only stored (uncompressed) blocks.
    Stored,
    /// Only fixed-Huffman blocks.
    Fixed,
    /// Dynamic-Huffman blocks (falling back to fixed only if a code
    /// cannot be built, which does not happen for DEFLATE's alphabets).
    Dynamic,
}

/// Compression settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressOptions {
    /// 0 = store only, 1 = fastest … 9 = smallest. Values above 9 act as 9.
    pub level: u8,
    /// Block-type policy.
    pub strategy: Strategy,
    /// Decompress the result and compare it with the input before
    /// returning; on any mismatch return a stored-only stream instead.
    /// This is translation validation: the output is then correct by
    /// construction even outside what the bounded proofs cover.
    pub verify: bool,
}

impl Default for CompressOptions {
    fn default() -> Self {
        Self { level: 6, strategy: Strategy::Auto, verify: false }
    }
}

/// Compresses `data` into a raw DEFLATE stream at the given level (0-9).
pub fn compress(data: &[u8], level: u8) -> Vec<u8> {
    compress_with(data, &CompressOptions { level, ..CompressOptions::default() })
}

/// Compresses `data` into a raw DEFLATE stream with explicit options.
pub fn compress_with(data: &[u8], opts: &CompressOptions) -> Vec<u8> {
    let out = encode(data, opts);
    if opts.verify {
        let ok = matches!(decompress_with_limit(&out, data.len()), Ok(ref d) if d.data == data);
        if !ok {
            let mut w = BitWriter::with_capacity(data.len() + data.len() / MAX_STORED_BLOCK * 5 + 5);
            write_stored(&mut w, data, true);
            return w.finish();
        }
    }
    out
}

fn encode(data: &[u8], opts: &CompressOptions) -> Vec<u8> {
    let strategy = if opts.level == 0 { Strategy::Stored } else { opts.strategy };
    let mut w = BitWriter::with_capacity(data.len() / 2 + 16);
    if strategy == Strategy::Stored {
        write_stored(&mut w, data, true);
        return w.finish();
    }
    let mut matcher = Matcher::new(MatchConfig::for_level(opts.level));
    let mut tokens = Vec::with_capacity(BLOCK_TOKENS.min(data.len() + 1));
    let mut pos = 0;
    loop {
        tokens.clear();
        let end = matcher.tokenize(data, pos, BLOCK_TOKENS, &mut tokens);
        let last = end >= data.len();
        write_block(&mut w, &tokens, &data[pos..end], last, strategy);
        pos = end;
        if last {
            break;
        }
    }
    w.finish()
}

/// A Huffman code ready for writing: codes are stored bit-reversed so they
/// can be emitted LSB-first with [`BitWriter::write_bits`].
#[derive(Debug, Clone)]
pub(crate) struct CodeTable<const N: usize> {
    pub(crate) code: [u16; N],
    pub(crate) len: [u8; N],
}

impl<const N: usize> CodeTable<N> {
    pub(crate) const fn from_lengths(len: [u8; N]) -> Result<Self, HuffmanError> {
        let mut code = [0u16; N];
        if let Err(e) = canonical_codes(&len, &mut code) {
            return Err(e);
        }
        let mut i = 0;
        while i < N {
            code[i] = reverse_bits(code[i], len[i] as u32);
            i += 1;
        }
        Ok(Self { code, len })
    }

    #[inline]
    pub(crate) fn put(&self, w: &mut BitWriter, sym: usize) {
        w.write_bits(u32::from(self.code[sym]), u32::from(self.len[sym]));
    }
}

pub(crate) const FIXED_LIT: CodeTable<288> = match CodeTable::from_lengths(fixed_litlen_lengths()) {
    Ok(t) => t,
    Err(_) => panic!("fixed literal/length code is invalid"),
};
pub(crate) const FIXED_DIST: CodeTable<32> = match CodeTable::from_lengths(FIXED_DIST_LENGTHS) {
    Ok(t) => t,
    Err(_) => panic!("fixed distance code is invalid"),
};

fn write_block(w: &mut BitWriter, tokens: &[Token], span: &[u8], last: bool, strategy: Strategy) {
    match strategy {
        Strategy::Stored => write_stored(w, span, last),
        Strategy::Fixed => write_fixed(w, tokens, last),
        Strategy::Dynamic | Strategy::Auto => {
            let stats = BlockStats::new(tokens);
            let plan = plan_dynamic(&stats);
            if strategy == Strategy::Dynamic {
                match plan {
                    Some(p) => write_dynamic(w, tokens, &p, last),
                    None => write_fixed(w, tokens, last),
                }
                return;
            }
            let fixed = stats.fixed_bits();
            let stored = stored_bits(w.bit_len(), span.len());
            let dynamic = plan.as_ref().map_or(u64::MAX, |p| p.bits);
            match plan {
                Some(p) if dynamic < fixed && dynamic < stored => write_dynamic(w, tokens, &p, last),
                _ if fixed <= stored => write_fixed(w, tokens, last),
                _ => write_stored(w, span, last),
            }
        }
    }
}

/// Writes `data` as one or more stored blocks (an empty input still gets
/// one empty block so the stream has a final block).
pub(crate) fn write_stored(w: &mut BitWriter, data: &[u8], last: bool) {
    if data.is_empty() {
        write_stored_chunk(w, data, last);
        return;
    }
    let mut chunks = data.chunks(MAX_STORED_BLOCK).peekable();
    while let Some(chunk) = chunks.next() {
        let is_last = last && chunks.peek().is_none();
        write_stored_chunk(w, chunk, is_last);
    }
}

fn write_stored_chunk(w: &mut BitWriter, chunk: &[u8], last: bool) {
    debug_assert!(chunk.len() <= MAX_STORED_BLOCK);
    w.write_bits(u32::from(last), 3); // BFINAL, BTYPE = 00
    w.pad_to_byte();
    let len = chunk.len() as u16;
    w.write_bits(u32::from(len), 16);
    w.write_bits(u32::from(!len), 16);
    w.write_aligned_bytes(chunk);
}

/// Exact size in bits of [`write_stored`] for `len` bytes when the writer
/// is at bit position `bit_pos`.
pub(crate) fn stored_bits(bit_pos: u64, len: usize) -> u64 {
    let chunks = if len == 0 { 1 } else { len.div_ceil(MAX_STORED_BLOCK) } as u64;
    let first_pad = (8 - (bit_pos + 3) % 8) % 8;
    3 + first_pad + 32 + (chunks - 1) * (3 + 5 + 32) + 8 * len as u64
}

fn write_fixed(w: &mut BitWriter, tokens: &[Token], last: bool) {
    w.write_bits(u32::from(last) | (1 << 1), 3);
    write_tokens(w, tokens, &FIXED_LIT, &FIXED_DIST);
}

/// Writes the tokens followed by end-of-block.
fn write_tokens<const L: usize, const D: usize>(
    w: &mut BitWriter,
    tokens: &[Token],
    lit: &CodeTable<L>,
    dist: &CodeTable<D>,
) {
    for t in tokens {
        match *t {
            Token::Literal(b) => lit.put(w, b as usize),
            Token::Match { len, dist: d } => {
                let (len, d) = (len as usize, d as usize);
                let lc = length_code(len);
                lit.put(w, 257 + lc);
                w.write_bits((len - LENGTH_BASE[lc] as usize) as u32, u32::from(LENGTH_EXTRA[lc]));
                let dc = distance_code(d);
                dist.put(w, dc);
                w.write_bits((d - DIST_BASE[dc] as usize) as u32, u32::from(DIST_EXTRA[dc]));
            }
        }
    }
    lit.put(w, END_OF_BLOCK);
}

/// Symbol frequencies of one block.
struct BlockStats {
    lit: [u32; NUM_LITLEN],
    dist: [u32; NUM_DIST],
    extra_bits: u64,
}

impl BlockStats {
    fn new(tokens: &[Token]) -> Self {
        let mut s = Self { lit: [0; NUM_LITLEN], dist: [0; NUM_DIST], extra_bits: 0 };
        for t in tokens {
            match *t {
                Token::Literal(b) => s.lit[b as usize] += 1,
                Token::Match { len, dist } => {
                    let lc = length_code(len as usize);
                    s.lit[257 + lc] += 1;
                    s.extra_bits += u64::from(LENGTH_EXTRA[lc]);
                    let dc = distance_code(dist as usize);
                    s.dist[dc] += 1;
                    s.extra_bits += u64::from(DIST_EXTRA[dc]);
                }
            }
        }
        s.lit[END_OF_BLOCK] += 1;
        s
    }

    /// Bits for the block body (symbols, extra bits, end-of-block).
    fn body_bits(&self, lit_len: &[u8], dist_len: &[u8]) -> u64 {
        let lit: u64 = self.lit.iter().zip(lit_len).map(|(&f, &l)| u64::from(f) * u64::from(l)).sum();
        let dist: u64 = self.dist.iter().zip(dist_len).map(|(&f, &l)| u64::from(f) * u64::from(l)).sum();
        lit + dist + self.extra_bits
    }

    fn fixed_bits(&self) -> u64 {
        3 + self.body_bits(&FIXED_LIT.len, &FIXED_DIST.len)
    }
}

/// One code-length symbol (0..=18) with its extra-bits value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClToken {
    pub(crate) sym: u8,
    pub(crate) extra: u8,
}

/// Run-length encodes a code-length sequence with symbols 16 (repeat the
/// previous length 3-6 times), 17 (3-10 zeros) and 18 (11-138 zeros).
///
/// Runs are found in a single pass over the input; each completed run is
/// then emitted by [`emit_run`].
pub(crate) fn rle_code_lengths(lengths: &[u8], out: &mut Vec<ClToken>) {
    let mut value = 0u8;
    let mut run = 0usize;
    for &v in lengths {
        if run > 0 && v == value {
            run += 1;
            continue;
        }
        if run > 0 {
            emit_run(value, run, out);
        }
        value = v;
        run = 1;
    }
    if run > 0 {
        emit_run(value, run, out);
    }
}

/// Emits a run of `run` (at least one) copies of the code length `v`.
fn emit_run(v: u8, run: usize, out: &mut Vec<ClToken>) {
    if v == 0 {
        let mut r = run;
        while r >= 11 {
            let k = if r < 138 { r } else { 138 };
            out.push(ClToken { sym: 18, extra: (k - 11) as u8 });
            r -= k;
        }
        if r >= 3 {
            out.push(ClToken { sym: 17, extra: (r - 3) as u8 });
            r = 0;
        }
        while r > 0 {
            out.push(ClToken { sym: 0, extra: 0 });
            r -= 1;
        }
    } else {
        out.push(ClToken { sym: v, extra: 0 });
        let mut r = run - 1;
        while r >= 3 {
            let k = if r < 6 { r } else { 6 };
            out.push(ClToken { sym: 16, extra: (k - 3) as u8 });
            r -= k;
        }
        while r > 0 {
            out.push(ClToken { sym: v, extra: 0 });
            r -= 1;
        }
    }
}

/// Everything needed to write one dynamic block, plus its exact size.
struct DynamicPlan {
    lit: CodeTable<NUM_LITLEN>,
    dist: CodeTable<NUM_DIST>,
    codelen: CodeTable<NUM_CODELEN>,
    hlit: usize,
    hdist: usize,
    hclen: usize,
    cl_tokens: Vec<ClToken>,
    bits: u64,
}

fn last_nonzero(l: &[u8]) -> usize {
    l.iter().rposition(|&x| x != 0).unwrap_or(0)
}

fn plan_dynamic(stats: &BlockStats) -> Option<DynamicPlan> {
    let mut lit_len = [0u8; NUM_LITLEN];
    build_lengths(&stats.lit, MAX_CODE_LEN, &mut lit_len).ok()?;
    let mut dist_len = [0u8; NUM_DIST];
    build_lengths(&stats.dist, MAX_CODE_LEN, &mut dist_len).ok()?;
    let hlit = (last_nonzero(&lit_len) + 1).max(257);
    let hdist = last_nonzero(&dist_len) + 1;
    let mut all = [0u8; NUM_LITLEN + NUM_DIST];
    all[..hlit].copy_from_slice(&lit_len[..hlit]);
    all[hlit..hlit + hdist].copy_from_slice(&dist_len[..hdist]);
    let mut cl_tokens = Vec::new();
    rle_code_lengths(&all[..hlit + hdist], &mut cl_tokens);
    let mut cl_freq = [0u32; NUM_CODELEN];
    for t in &cl_tokens {
        cl_freq[t.sym as usize] += 1;
    }
    let mut cl_len = [0u8; NUM_CODELEN];
    build_lengths(&cl_freq, MAX_CODELEN_LEN, &mut cl_len).ok()?;
    let hclen = CODELEN_ORDER.iter().rposition(|&s| cl_len[s] != 0).map_or(0, |i| i + 1).max(4);
    let cl_bits: u64 =
        cl_tokens.iter().map(|t| u64::from(cl_len[t.sym as usize]) + u64::from(codelen_extra_bits(t.sym))).sum();
    let bits = 3 + 5 + 5 + 4 + 3 * hclen as u64 + cl_bits + stats.body_bits(&lit_len, &dist_len);
    Some(DynamicPlan {
        lit: CodeTable::from_lengths(lit_len).ok()?,
        dist: CodeTable::from_lengths(dist_len).ok()?,
        codelen: CodeTable::from_lengths(cl_len).ok()?,
        hlit,
        hdist,
        hclen,
        cl_tokens,
        bits,
    })
}

fn write_dynamic(w: &mut BitWriter, tokens: &[Token], p: &DynamicPlan, last: bool) {
    w.write_bits(u32::from(last) | (2 << 1), 3);
    w.write_bits((p.hlit - 257) as u32, 5);
    w.write_bits((p.hdist - 1) as u32, 5);
    w.write_bits((p.hclen - 4) as u32, 4);
    for &sym in &CODELEN_ORDER[..p.hclen] {
        w.write_bits(u32::from(p.codelen.len[sym]), 3);
    }
    for t in &p.cl_tokens {
        p.codelen.put(w, t.sym as usize);
        w.write_bits(u32::from(t.extra), codelen_extra_bits(t.sym));
    }
    write_tokens(w, tokens, &p.lit, &p.dist);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inflate::decompress;

    fn sample_data() -> Vec<u8> {
        let mut v = Vec::new();
        let mut x = 12345u32;
        for i in 0..50_000u32 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            v.push(if i % 3 == 0 { b'a' + (x % 7) as u8 } else { (x >> 24) as u8 & 0x3f });
        }
        v
    }

    /// The cost model must equal the bits actually written, or block-type
    /// choices are made on wrong numbers.
    #[test]
    fn cost_model_is_exact() {
        let data = sample_data();
        let mut m = Matcher::new(MatchConfig::for_level(6));
        let mut toks = Vec::new();
        m.tokenize(&data, 0, BLOCK_TOKENS, &mut toks);
        let stats = BlockStats::new(&toks);
        for start_bits in 0..8u32 {
            let mut w = BitWriter::new();
            w.write_bits(0, start_bits);
            write_fixed(&mut w, &toks, false);
            assert_eq!(w.bit_len() - u64::from(start_bits), stats.fixed_bits());

            let plan = plan_dynamic(&stats).unwrap();
            let mut w = BitWriter::new();
            w.write_bits(0, start_bits);
            write_dynamic(&mut w, &toks, &plan, false);
            assert_eq!(w.bit_len() - u64::from(start_bits), plan.bits);

            for len in [0usize, 1, 1000, 65_535, 65_536, 200_000] {
                let span = vec![0u8; len];
                let mut w = BitWriter::new();
                w.write_bits(0, start_bits);
                write_stored(&mut w, &span, false);
                assert_eq!(w.bit_len() - u64::from(start_bits), stored_bits(u64::from(start_bits), len));
            }
        }
    }

    #[test]
    fn empty_input_is_two_bytes() {
        assert_eq!(compress(b"", 6), vec![0x03, 0x00]);
        assert_eq!(compress(b"", 0), vec![0x01, 0x00, 0x00, 0xff, 0xff]);
    }

    #[test]
    fn every_strategy_and_level_roundtrips() {
        let data = sample_data();
        for strategy in [Strategy::Auto, Strategy::Stored, Strategy::Fixed, Strategy::Dynamic] {
            for level in 0..=9 {
                let opts = CompressOptions { level, strategy, verify: false };
                let c = compress_with(&data, &opts);
                assert_eq!(decompress(&c).unwrap(), data, "{strategy:?} level {level}");
            }
        }
    }

    #[test]
    fn rle_handles_long_runs() {
        let mut lengths = vec![0u8; 300];
        lengths[5] = 8;
        lengths[6..40].fill(9);
        let mut toks = Vec::new();
        rle_code_lengths(&lengths, &mut toks);
        let mut out = vec![0u8; 300];
        let mut idx = 0;
        for t in toks {
            crate::inflate::expand_code_length(t.sym, u32::from(t.extra), &mut out, &mut idx).unwrap();
        }
        assert_eq!(idx, 300);
        assert_eq!(out, lengths);
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;
    use crate::inflate::expand_code_length;

    fn check_rle(lengths: &[u8]) {
        // Every token covers at least one length, so this never reallocates.
        let mut toks = Vec::with_capacity(lengths.len());
        rle_code_lengths(lengths, &mut toks);
        let mut out = vec![0u8; lengths.len()];
        let mut idx = 0;
        for t in toks.iter() {
            assert!(t.sym <= 18);
            assert!(u32::from(t.extra) < (1u32 << codelen_extra_bits(t.sym)));
            assert!(expand_code_length(t.sym, u32::from(t.extra), &mut out, &mut idx).is_ok());
        }
        assert_eq!(idx, lengths.len());
        assert_eq!(out.as_slice(), lengths);
    }

    /// RLE of arbitrary code lengths decodes, through the decoder's own
    /// expansion routine, back to the same lengths (covers symbols 0-17).
    #[kani::proof]
    #[kani::unwind(9)]
    fn code_length_rle_roundtrip() {
        let lengths: [u8; 8] = core::array::from_fn(|_| kani::any_where(|&l: &u8| l <= 15));
        check_rle(&lengths);
    }

    /// Long zero runs (symbol 18) and mixed runs also round-trip: 12
    /// lengths drawn from {0, v}.
    #[kani::proof]
    #[kani::unwind(13)]
    fn code_length_rle_zero_runs() {
        let v: u8 = kani::any_where(|&v: &u8| v <= 15);
        let mask: [bool; 12] = kani::any();
        let mut lengths = [0u8; 12];
        for i in 0..12 {
            if mask[i] {
                lengths[i] = v;
            }
        }
        check_rle(&lengths);
    }

    /// The fixed tables built at compile time are the RFC's canonical codes:
    /// symbol 0 is 00110000, 144 is 110010000, 256 is 0000000 and 280 is
    /// 11000000 (all MSB-first, stored reversed).
    #[kani::proof]
    fn fixed_code_table_matches_rfc() {
        assert_eq!(FIXED_LIT.code[0], reverse_bits(0b0011_0000, 8));
        assert_eq!(FIXED_LIT.code[143], reverse_bits(0b1011_1111, 8));
        assert_eq!(FIXED_LIT.code[144], reverse_bits(0b1_1001_0000, 9));
        assert_eq!(FIXED_LIT.code[255], reverse_bits(0b1_1111_1111, 9));
        assert_eq!(FIXED_LIT.code[256], reverse_bits(0, 7));
        assert_eq!(FIXED_LIT.code[279], reverse_bits(0b001_0111, 7));
        assert_eq!(FIXED_LIT.code[280], reverse_bits(0b1100_0000, 8));
        assert_eq!(FIXED_LIT.code[287], reverse_bits(0b1100_0111, 8));
        let d: usize = kani::any();
        kani::assume(d < 32);
        assert_eq!(FIXED_DIST.code[d], reverse_bits(d as u16, 5));
    }

    /// The stored-block size model equals the bits actually written.
    #[kani::proof]
    #[kani::unwind(10)]
    fn stored_cost_is_exact() {
        let pre: u32 = kani::any();
        kani::assume(pre < 8);
        let data: [u8; 8] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= 8);
        let mut w = BitWriter::with_capacity(16);
        w.write_bits(0, pre);
        write_stored(&mut w, &data[..len], true);
        assert_eq!(w.bit_len() - u64::from(pre), stored_bits(u64::from(pre), len));
    }
}
