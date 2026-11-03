//! The DEFLATE encoder (`src/deflate.rs`): one-shot `compress` and
//! `compress_with`, the streaming `StreamEncoder`, and the block writers
//! under them.
//!
//! The executable code below is the library's, token for token (CI checks
//! this with `.github/scripts/verus_drift.py`); every line marked
//! `// @ghost` is specification or proof. The theorem: for every input of
//! every length, every level and every strategy, compression returns
//! without panicking and every loop terminates; the same holds for every
//! sequence of calls to the streaming encoder.

use crate::bitio::{reverse_bits, BitWriter};
use crate::build_lengths::{build_lengths, canonical_codes, MAX_CODE_LEN};
use crate::inflate::decompress_with_limit;
use crate::limit_depths::HuffmanError;
use crate::lz77::{tokenized, valid, MatchConfig, Matcher, Token};
use crate::std_specs::axiom_byte_slice_len;
use crate::tables::{
    codelen_extra_bits, distance_code, fixed_litlen_lengths, lemma_tables, length_code, CODELEN_ORDER, DIST_BASE,
    DIST_EXTRA, END_OF_BLOCK, FIXED_DIST_LENGTHS, LENGTH_BASE, LENGTH_EXTRA, NUM_CODELEN, NUM_DIST, NUM_LITLEN,
};
use vstd::prelude::*;

verus! {

/// Largest payload of a single stored block.
pub const MAX_STORED_BLOCK: usize = 65_535;

/// Maximum LZ77 tokens per Huffman block.
pub const BLOCK_TOKENS: usize = 16_384;

/// Longest code allowed in the code-length code (§3.2.7: 3-bit lengths).
const MAX_CODELEN_LEN: usize = 7;

/// Which block types the encoder may use.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Strategy {
    /// Pick the smallest of stored, fixed and dynamic for each block.
    #[default]
    Auto,
    /// Only stored (uncompressed) blocks.
    Stored,
    /// Only fixed-Huffman blocks.
    Fixed,
    /// Dynamic-Huffman blocks.
    Dynamic,
}

/// Compression settings.
#[derive(Clone, Copy)]
pub struct CompressOptions {
    /// 0 = store only, 1 = fastest … 9 = smallest.
    pub level: u8,
    /// Block-type policy.
    pub strategy: Strategy,
    /// Decompress the result and compare it with the input before
    /// returning; on any mismatch return a stored-only stream instead.
    pub verify: bool,
}

impl Default for CompressOptions {
    fn default() -> Self {
        Self { level: 6, strategy: Strategy::Auto, verify: false }
    }
}

/// All tokens are ones DEFLATE can express.
pub open spec fn all_valid(t: Seq<Token>) -> bool {
    forall|j: int| 0 <= j < t.len() ==> valid(#[trigger] t[j])
}

/// Compresses `data` into a raw DEFLATE stream at the given level (0-9).
pub fn compress(data: &[u8], level: u8) -> Vec<u8> {
    compress_with(data, &CompressOptions { level, ..CompressOptions::default() })
}

/// Compresses `data` into a raw DEFLATE stream with explicit options.
pub fn compress_with(data: &[u8], opts: &CompressOptions) -> Vec<u8> {
    proof { // @ghost-begin
        axiom_byte_slice_len(data);
    } // @ghost-end
    let out = encode(data, opts);
    if opts.verify {
        let ok = matches!(decompress_with_limit(&out, data.len()), Ok(ref d) if d.data.as_slice() == data);
        if !ok {
            let mut w = BitWriter::with_capacity(data.len() + data.len() / MAX_STORED_BLOCK * 5 + 5);
            write_stored(&mut w, data, true);
            return w.finish();
        }
    }
    out
}

fn encode(data: &[u8], opts: &CompressOptions) -> Vec<u8> {
    encode_with_matcher(data, opts, MatchConfig::for_level(opts.level))
}

/// The encoder with an explicit match-finder configuration.
#[verifier::loop_isolation(false)]
pub(crate) fn encode_with_matcher(data: &[u8], opts: &CompressOptions, cfg: MatchConfig) -> Vec<u8> {
    proof { // @ghost-begin
        axiom_byte_slice_len(data);
    } // @ghost-end
    let strategy = if opts.level == 0 { Strategy::Stored } else { opts.strategy };
    let mut w = BitWriter::with_capacity(data.len() / 2 + 16);
    if strategy == Strategy::Stored {
        write_stored(&mut w, data, true);
        return w.finish();
    }
    let mut matcher = Matcher::new(cfg);
    let mut tokens = vec![Token::Literal(0); BLOCK_TOKENS.min(data.len() + 1)];
    if data.len() < tokens.len() {
        // Everything fits in one block: one tokenize call covers the whole
        // input (each token covers at least one byte).
        let (end, count) = matcher.tokenize(data, 0, &mut tokens);
        debug_assert_eq!(end, data.len());
        write_block(&mut w, &tokens[..count], &data[..end], true, strategy);
        return w.finish();
    }
    let mut pos = 0;
    loop
        invariant // @ghost
            matcher.wf(), // @ghost
            w.wf(), // @ghost
            pos <= data@.len(), // @ghost
            1 <= tokens@.len() <= BLOCK_TOKENS, // @ghost
        decreases data@.len() - pos, // @ghost
    {
        let (end, count) = matcher.tokenize(data, pos, &mut tokens);
        let last = end >= data.len();
        write_block(&mut w, &tokens[..count], &data[pos..end], last, strategy);
        pos = end;
        if last {
            break;
        }
    }
    w.finish()
}

/// Bytes of earlier input a streaming encoder keeps for matches.
const WINDOW: usize = 32 * 1024;

/// Input a streaming encoder buffers before compressing it.
pub(crate) const STREAM_CHUNK: usize = 256 * 1024;

/// Incremental raw-DEFLATE encoder, the core of the `write` encoders.
pub(crate) struct StreamEncoder {
    strategy: Strategy,
    matcher: Matcher,
    /// Already-compressed history (`buf[..hist]`, at most the window)
    /// followed by input not yet compressed.
    buf: Vec<u8>,
    hist: usize,
    bits: BitWriter,
    tokens: Vec<Token>,
}

impl StreamEncoder {
    /// The history is at most the window and the pending input at most a
    /// chunk.
    pub(crate) closed spec fn wf(&self) -> bool {
        &&& self.matcher.wf()
        &&& self.bits.wf()
        &&& self.hist <= WINDOW
        &&& self.hist <= self.buf@.len()
        &&& self.buf@.len() - self.hist <= STREAM_CHUNK
        &&& self.tokens@.len() == BLOCK_TOKENS
    }

    pub(crate) fn new(cfg: MatchConfig, strategy: Strategy) -> (r: Self)
        ensures // @ghost
            r.wf(), // @ghost
    {
        Self {
            strategy,
            matcher: Matcher::new(cfg),
            buf: Vec::new(),
            hist: 0,
            bits: BitWriter::new(),
            tokens: vec![Token::Literal(0); BLOCK_TOKENS],
        }
    }

    /// Starts a new, independent stream with the same settings.
    pub(crate) fn reset(&mut self)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
    {
        self.buf.clear();
        self.hist = 0;
        self.bits = BitWriter::new();
    }

    /// Buffers as much of `data` as fits in the current chunk, compressing
    /// the chunk once it is full.
    pub(crate) fn push(&mut self, data: &[u8]) -> usize
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
    {
        let room = STREAM_CHUNK - (self.buf.len() - self.hist);
        let n = data.len().min(room);
        self.buf.extend_from_slice(&data[..n]);
        if self.buf.len() - self.hist == STREAM_CHUNK {
            self.compress_pending(false);
        }
        n
    }

    /// Compresses everything buffered.
    #[verifier::loop_isolation(false)]
    fn compress_pending(&mut self, last: bool)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
    {
        let len = self.buf.len();
        if self.hist == len && !last {
            return;
        }
        if self.strategy == Strategy::Stored {
            write_stored(&mut self.bits, &self.buf[self.hist..], last);
        } else {
            self.matcher.reset();
            self.matcher.prime(&self.buf, self.hist);
            let mut pos = self.hist;
            loop
                invariant // @ghost
                    self.matcher.wf(), // @ghost
                    self.bits.wf(), // @ghost
                    self.buf@.len() == len, // @ghost
                    self.hist <= pos <= len, // @ghost
                    self.tokens@.len() == BLOCK_TOKENS, // @ghost
                decreases len - pos, // @ghost
            {
                let (end, count) = self.matcher.tokenize(&self.buf, pos, &mut self.tokens);
                let done = end >= len;
                write_block(&mut self.bits, &self.tokens[..count], &self.buf[pos..end], last && done, self.strategy);
                pos = end;
                if done {
                    break;
                }
            }
        }
        if last {
            self.bits.pad_to_byte();
        }
        // Keep the last window of input as history.
        if len > WINDOW {
            self.buf.copy_within(len - WINDOW.., 0);
            self.buf.truncate(WINDOW);
        }
        self.hist = self.buf.len();
    }

    /// zlib's sync flush.
    pub(crate) fn sync_flush(&mut self)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
    {
        self.compress_pending(false);
        write_stored(&mut self.bits, &[], false);
    }

    /// Compresses everything buffered and ends the stream.
    pub(crate) fn finish(&mut self)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
    {
        self.compress_pending(true);
    }

    /// The whole bytes of output produced so far.
    pub(crate) fn take_output(&mut self) -> Vec<u8>
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
    {
        self.bits.take_bytes()
    }
}

/// A Huffman code ready for writing.
pub(crate) struct CodeTable<const N: usize> {
    pub(crate) code: [u16; N],
    pub(crate) len: [u8; N],
}

impl<const N: usize> CodeTable<N> {
    /// Every code is at most 15 bits.
    pub(crate) open spec fn wf(&self) -> bool {
        forall|i: int| 0 <= i < N ==> #[trigger] self.len@[i] <= 15
    }

    pub(crate) const fn from_lengths(len: [u8; N]) -> (r: Result<Self, HuffmanError>)
        requires // @ghost
            N <= 288, // @ghost
        ensures // @ghost
            r matches Ok(t) ==> t.wf(), // @ghost
    {
        let mut code = [0u16; N];
        if let Err(e) = canonical_codes(&len, &mut code) {
            return Err(e);
        }
        let mut i = 0;
        while i < N
            invariant // @ghost
                i <= N, // @ghost
            decreases N - i, // @ghost
        {
            code[i] = reverse_bits(code[i], len[i] as u32);
            i += 1;
        }
        Ok(Self { code, len })
    }

    #[inline]
    pub(crate) fn put(&self, w: &mut BitWriter, sym: usize)
        requires // @ghost
            self.wf(), // @ghost
            sym < N, // @ghost
            old(w).wf(), // @ghost
        ensures // @ghost
            final(w).wf(), // @ghost
    {
        w.write_bits(u32::from(self.code[sym]), u32::from(self.len[sym]));
    }

    /// Writes `sym`'s code followed by the low `n` bits of `extra`, in one
    /// write.
    #[inline]
    pub(crate) fn put_with_extra(&self, w: &mut BitWriter, sym: usize, extra: u32, n: u32)
        requires // @ghost
            self.wf(), // @ghost
            sym < N, // @ghost
            n <= 17, // @ghost
            old(w).wf(), // @ghost
        ensures // @ghost
            final(w).wf(), // @ghost
    {
        let l = u32::from(self.len[sym]);
        proof { // @ghost-begin
            assert((1u32 << n) >= 1) by (bit_vector)
                requires
                    n <= 17,
            ;
        } // @ghost-end
        let extra = extra & ((1u32 << n) - 1);
        w.write_bits(u32::from(self.code[sym]) | (extra << l), l + n);
    }
}

pub(crate) exec const FIXED_LIT: CodeTable<288>
    ensures // @ghost
        FIXED_LIT.wf(), // @ghost
{
    match CodeTable::from_lengths(fixed_litlen_lengths()) {
        Ok(t) => t,
        Err(_) => CodeTable { code: [0; 288], len: [0; 288] },
    }
}

pub(crate) exec const FIXED_DIST: CodeTable<32>
    ensures // @ghost
        FIXED_DIST.wf(), // @ghost
{
    match CodeTable::from_lengths(FIXED_DIST_LENGTHS) {
        Ok(t) => t,
        Err(_) => CodeTable { code: [0; 32], len: [0; 32] },
    }
}

fn write_block(w: &mut BitWriter, tokens: &[Token], span: &[u8], last: bool, strategy: Strategy)
    requires // @ghost
        old(w).wf(), // @ghost
        tokens@.len() <= BLOCK_TOKENS, // @ghost
        all_valid(tokens@), // @ghost
        span@.len() <= 258 * BLOCK_TOKENS, // @ghost
    ensures // @ghost
        final(w).wf(), // @ghost
{
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
            let dynamic = match &plan {
                Some(p) => p.bits,
                None => u64::MAX,
            };
            match plan {
                Some(p) if dynamic < fixed && dynamic < stored => write_dynamic(w, tokens, &p, last),
                _ if fixed <= stored => write_fixed(w, tokens, last),
                _ => write_stored(w, span, last),
            }
        }
    }
}

/// Writes `data` as one or more stored blocks.
pub(crate) fn write_stored(w: &mut BitWriter, data: &[u8], last: bool)
    requires // @ghost
        old(w).wf(), // @ghost
    ensures // @ghost
        final(w).wf(), // @ghost
{
    write_stored_in(w, data, last, MAX_STORED_BLOCK);
}

/// [`write_stored`] with blocks of at most `max` bytes.
fn write_stored_in(w: &mut BitWriter, data: &[u8], last: bool, max: usize)
    requires // @ghost
        old(w).wf(), // @ghost
        1 <= max <= MAX_STORED_BLOCK, // @ghost
    ensures // @ghost
        final(w).wf(), // @ghost
{
    if data.is_empty() {
        write_stored_chunk(w, data, last);
        return;
    }
    let mut start = 0;
    while start < data.len()
        invariant // @ghost
            w.wf(), // @ghost
            start <= data@.len(), // @ghost
            1 <= max <= MAX_STORED_BLOCK, // @ghost
        decreases data@.len() - start, // @ghost
    {
        let end = if data.len() - start > max { start + max } else { data.len() };
        write_stored_chunk(w, &data[start..end], last && end == data.len());
        start = end;
    }
}

fn write_stored_chunk(w: &mut BitWriter, chunk: &[u8], last: bool)
    requires // @ghost
        old(w).wf(), // @ghost
        chunk@.len() <= MAX_STORED_BLOCK, // @ghost
    ensures // @ghost
        final(w).wf(), // @ghost
{
    debug_assert!(chunk.len() <= MAX_STORED_BLOCK);
    w.write_bits(u32::from(last), 3); // BFINAL, BTYPE = 00
    w.pad_to_byte();
    let len = chunk.len() as u16;
    w.write_bits(u32::from(len), 16);
    w.write_bits(u32::from(!len), 16);
    w.write_aligned_bytes(chunk);
}

/// Exact size in bits of [`write_stored`] for `len` bytes.
pub(crate) fn stored_bits(bit_pos: u64, len: usize) -> u64
    requires // @ghost
        len <= 0xffff_ffff, // @ghost
{
    stored_bits_in(bit_pos, len, MAX_STORED_BLOCK)
}

/// [`stored_bits`] for blocks of at most `max` bytes.
fn stored_bits_in(bit_pos: u64, len: usize, max: usize) -> u64
    requires // @ghost
        len <= 0xffff_ffff, // @ghost
        max >= 1, // @ghost
{
    let chunks = if len == 0 { 1 } else { len.div_ceil(max) } as u64;
    let first_pad = (8 - (bit_pos % 8 + 3) % 8) % 8;
    3 + first_pad + 32 + (chunks - 1) * (3 + 5 + 32) + 8 * len as u64
}

fn write_fixed(w: &mut BitWriter, tokens: &[Token], last: bool)
    requires // @ghost
        old(w).wf(), // @ghost
        all_valid(tokens@), // @ghost
    ensures // @ghost
        final(w).wf(), // @ghost
{
    w.write_bits(u32::from(last) | (1 << 1), 3);
    write_tokens(w, tokens, &FIXED_LIT, &FIXED_DIST);
}

/// Writes the tokens followed by end-of-block.
fn write_tokens<const L: usize, const D: usize>(
    w: &mut BitWriter,
    tokens: &[Token],
    lit: &CodeTable<L>,
    dist: &CodeTable<D>,
)
    requires // @ghost
        old(w).wf(), // @ghost
        all_valid(tokens@), // @ghost
        lit.wf(), // @ghost
        dist.wf(), // @ghost
        L >= 286, // @ghost
        D >= 30, // @ghost
    ensures // @ghost
        final(w).wf(), // @ghost
{
    proof { // @ghost-begin
        lemma_tables();
    } // @ghost-end
    for i in 0..tokens.len()
        invariant // @ghost
            w.wf(), // @ghost
            all_valid(tokens@), // @ghost
            forall|k: int| 0 <= k < 29 ==> #[trigger] LENGTH_EXTRA@[k] <= 5, // @ghost
            forall|k: int| 0 <= k < 30 ==> #[trigger] DIST_EXTRA@[k] <= 13, // @ghost
    {
        match tokens[i] {
            Token::Literal(b) => lit.put(w, b as usize),
            Token::Match { len, dist: d } => {
                let (len, d) = (len as usize, d as usize);
                // Each code goes out together with its extra bits: at most
                // 15 + 5 and 15 + 13 bits.
                let lc = length_code(len);
                lit.put_with_extra(w, 257 + lc, (len - LENGTH_BASE[lc] as usize) as u32, u32::from(LENGTH_EXTRA[lc]));
                let dc = distance_code(d);
                dist.put_with_extra(w, dc, (d - DIST_BASE[dc] as usize) as u32, u32::from(DIST_EXTRA[dc]));
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
    /// The counts of a block of at most `BLOCK_TOKENS` tokens.
    spec fn wf(&self) -> bool {
        &&& forall|k: int| 0 <= k < 286 ==> #[trigger] self.lit@[k] <= 16385
        &&& forall|k: int| 0 <= k < 30 ==> #[trigger] self.dist@[k] <= 16384
        &&& self.extra_bits <= 18 * 16384
    }

    fn new(tokens: &[Token]) -> (r: Self)
        requires // @ghost
            tokens@.len() <= BLOCK_TOKENS, // @ghost
            all_valid(tokens@), // @ghost
        ensures // @ghost
            r.wf(), // @ghost
    {
        let mut s = Self { lit: [0; NUM_LITLEN], dist: [0; NUM_DIST], extra_bits: 0 };
        proof { // @ghost-begin
            lemma_tables();
        } // @ghost-end
        for i in 0..tokens.len()
            invariant // @ghost
                tokens@.len() <= BLOCK_TOKENS, // @ghost
                all_valid(tokens@), // @ghost
                forall|k: int| 0 <= k < 286 ==> #[trigger] s.lit@[k] <= i, // @ghost
                forall|k: int| 0 <= k < 30 ==> #[trigger] s.dist@[k] <= i, // @ghost
                s.extra_bits <= 18 * i, // @ghost
                forall|k: int| 0 <= k < 29 ==> #[trigger] LENGTH_EXTRA@[k] <= 5, // @ghost
                forall|k: int| 0 <= k < 30 ==> #[trigger] DIST_EXTRA@[k] <= 13, // @ghost
        {
            match tokens[i] {
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
    fn body_bits(&self, lit_len: &[u8; NUM_LITLEN], dist_len: &[u8; NUM_DIST]) -> (r: u64)
        requires // @ghost
            self.wf(), // @ghost
            forall|k: int| 0 <= k < 286 ==> #[trigger] lit_len@[k] <= 15, // @ghost
            forall|k: int| 0 <= k < 30 ==> #[trigger] dist_len@[k] <= 15, // @ghost
        ensures // @ghost
            r <= 0x1000_0000, // @ghost
    {
        let mut bits = self.extra_bits;
        for i in 0..NUM_LITLEN
            invariant // @ghost
                self.wf(), // @ghost
                forall|k: int| 0 <= k < 286 ==> #[trigger] lit_len@[k] <= 15, // @ghost
                bits <= 18 * 16384 + i * (16385 * 15), // @ghost
        {
            proof { // @ghost-begin
                let a = self.lit@[i as int] as int;
                let b = lit_len@[i as int] as int;
                assert(a * b <= 16385 * 15) by (nonlinear_arith)
                    requires
                        0 <= a <= 16385,
                        0 <= b <= 15,
                ;
            } // @ghost-end
            bits += u64::from(self.lit[i]) * u64::from(lit_len[i]);
        }
        for i in 0..NUM_DIST
            invariant // @ghost
                self.wf(), // @ghost
                forall|k: int| 0 <= k < 30 ==> #[trigger] dist_len@[k] <= 15, // @ghost
                bits <= 18 * 16384 + 286 * (16385 * 15) + i * (16384 * 15), // @ghost
        {
            proof { // @ghost-begin
                let a = self.dist@[i as int] as int;
                let b = dist_len@[i as int] as int;
                assert(a * b <= 16384 * 15) by (nonlinear_arith)
                    requires
                        0 <= a <= 16384,
                        0 <= b <= 15,
                ;
            } // @ghost-end
            bits += u64::from(self.dist[i]) * u64::from(dist_len[i]);
        }
        bits
    }

    fn fixed_bits(&self) -> (r: u64)
        requires // @ghost
            self.wf(), // @ghost
        ensures // @ghost
            r <= 0x1000_0003, // @ghost
    {
        let mut dist_len = [0u8; NUM_DIST];
        dist_len.copy_from_slice(&FIXED_DIST.len[..NUM_DIST]);
        let mut lit_len = [0u8; NUM_LITLEN];
        lit_len.copy_from_slice(&FIXED_LIT.len[..NUM_LITLEN]);
        3 + self.body_bits(&lit_len, &dist_len)
    }
}

/// One code-length symbol (0..=18) with its extra-bits value.
#[derive(Clone, Copy)]
pub(crate) struct ClToken {
    pub(crate) sym: u8,
    pub(crate) extra: u8,
}

/// Code-length tokens: every symbol is 0-18.
pub(crate) open spec fn cl_ok(t: Seq<ClToken>, n: int) -> bool {
    forall|j: int| 0 <= j < n ==> #[trigger] t[j].sym <= 18
}

/// Run-length encodes a code-length sequence.
pub(crate) fn rle_code_lengths(lengths: &[u8], out: &mut [ClToken]) -> (r: usize)
    requires // @ghost
        old(out)@.len() >= lengths@.len(), // @ghost
        forall|i: int| 0 <= i < lengths@.len() ==> #[trigger] lengths@[i] <= 15, // @ghost
    ensures // @ghost
        final(out)@.len() == old(out)@.len(), // @ghost
        r <= lengths@.len(), // @ghost
        cl_ok(final(out)@, r as int), // @ghost
{
    let mut n = 0;
    let mut value = 0u8;
    let mut run = 0usize;
    for i in 0..lengths.len()
        invariant // @ghost
            out@.len() == old(out)@.len() >= lengths@.len(), // @ghost
            forall|k: int| 0 <= k < lengths@.len() ==> #[trigger] lengths@[k] <= 15, // @ghost
            value <= 15, // @ghost
            run + n <= i, // @ghost
            i > 0 ==> run >= 1, // @ghost
            cl_ok(out@, n as int), // @ghost
    {
        let v = lengths[i];
        if run > 0 && v == value {
            run += 1;
            continue;
        }
        if run > 0 {
            emit_run(value, run, out, &mut n);
        }
        value = v;
        run = 1;
    }
    if run > 0 {
        emit_run(value, run, out, &mut n);
    }
    n
}

/// Emits a run of `run` (at least one) copies of the code length `v`.
fn emit_run(v: u8, run: usize, out: &mut [ClToken], n: &mut usize)
    requires // @ghost
        run >= 1, // @ghost
        v <= 15, // @ghost
        *old(n) + run <= old(out)@.len(), // @ghost
        cl_ok(old(out)@, *old(n) as int), // @ghost
    ensures // @ghost
        final(out)@.len() == old(out)@.len(), // @ghost
        *old(n) < *final(n) <= *old(n) + run, // @ghost
        cl_ok(final(out)@, *final(n) as int), // @ghost
{
    if v == 0 {
        let mut r = run;
        while r >= 11
            invariant // @ghost
                out@.len() == old(out)@.len(), // @ghost
                *old(n) <= *n, // @ghost
                *n + r <= *old(n) + run, // @ghost
                r == run && *n == *old(n) || *n > *old(n), // @ghost
                cl_ok(out@, *n as int), // @ghost
            decreases r, // @ghost
        {
            let k = if r < 138 { r } else { 138 };
            put_cl(out, n, 18, (k - 11) as u8);
            r -= k;
        }
        if r >= 3 {
            put_cl(out, n, 17, (r - 3) as u8);
            r = 0;
        }
        while r > 0
            invariant // @ghost
                out@.len() == old(out)@.len(), // @ghost
                *old(n) <= *n, // @ghost
                *n + r <= *old(n) + run, // @ghost
                r == run && *n == *old(n) || *n > *old(n), // @ghost
                cl_ok(out@, *n as int), // @ghost
            decreases r, // @ghost
        {
            put_cl(out, n, 0, 0);
            r -= 1;
        }
    } else {
        put_cl(out, n, v, 0);
        let mut r = run - 1;
        while r >= 3
            invariant // @ghost
                out@.len() == old(out)@.len(), // @ghost
                *old(n) < *n, // @ghost
                *n + r <= *old(n) + run, // @ghost
                cl_ok(out@, *n as int), // @ghost
            decreases r, // @ghost
        {
            let k = if r < 6 { r } else { 6 };
            put_cl(out, n, 16, (k - 3) as u8);
            r -= k;
        }
        while r > 0
            invariant // @ghost
                out@.len() == old(out)@.len(), // @ghost
                *old(n) < *n, // @ghost
                *n + r <= *old(n) + run, // @ghost
                v <= 15, // @ghost
                cl_ok(out@, *n as int), // @ghost
            decreases r, // @ghost
        {
            put_cl(out, n, v, 0);
            r -= 1;
        }
    }
}

/// Writes one code-length token at `out[*n]`, advancing `*n`.
#[inline]
fn put_cl(out: &mut [ClToken], n: &mut usize, sym: u8, extra: u8)
    requires // @ghost
        *old(n) < old(out)@.len(), // @ghost
        sym <= 18, // @ghost
        cl_ok(old(out)@, *old(n) as int), // @ghost
    ensures // @ghost
        final(out)@.len() == old(out)@.len(), // @ghost
        *final(n) == *old(n) + 1, // @ghost
        cl_ok(final(out)@, *final(n) as int), // @ghost
{
    out[*n] = ClToken { sym, extra };
    *n += 1;
}

/// Everything needed to write one dynamic block, plus its exact size.
struct DynamicPlan {
    lit: CodeTable<NUM_LITLEN>,
    dist: CodeTable<NUM_DIST>,
    codelen: CodeTable<NUM_CODELEN>,
    hlit: usize,
    hdist: usize,
    hclen: usize,
    cl_tokens: [ClToken; NUM_LITLEN + NUM_DIST],
    cl_count: usize,
    bits: u64,
}

impl DynamicPlan {
    spec fn wf(&self) -> bool {
        &&& self.lit.wf()
        &&& self.dist.wf()
        &&& self.codelen.wf()
        &&& 257 <= self.hlit <= 286
        &&& 1 <= self.hdist <= 30
        &&& 4 <= self.hclen <= 19
        &&& self.cl_count <= 316
        &&& cl_ok(self.cl_tokens@, self.cl_count as int)
    }
}

/// Index of the last nonzero entry of `l`, or 0 if there is none.
fn last_nonzero(l: &[u8]) -> (r: usize)
    ensures // @ghost
        l@.len() > 0 ==> r < l@.len(), // @ghost
        l@.len() == 0 ==> r == 0, // @ghost
{
    let mut i = l.len();
    while i > 0
        invariant // @ghost
            i <= l@.len(), // @ghost
        decreases i, // @ghost
    {
        i -= 1;
        if l[i] != 0 {
            return i;
        }
    }
    0
}

#[verifier::loop_isolation(false)]
fn plan_dynamic(stats: &BlockStats) -> (r: Option<DynamicPlan>)
    requires // @ghost
        stats.wf(), // @ghost
    ensures // @ghost
        r matches Some(p) ==> p.wf(), // @ghost
{
    let mut lit_len = [0u8; NUM_LITLEN];
    build_lengths(&stats.lit, MAX_CODE_LEN, &mut lit_len).ok()?;
    let mut dist_len = [0u8; NUM_DIST];
    build_lengths(&stats.dist, MAX_CODE_LEN, &mut dist_len).ok()?;
    let hlit = (last_nonzero(&lit_len) + 1).max(257);
    let hdist = last_nonzero(&dist_len) + 1;
    let mut all = [0u8; NUM_LITLEN + NUM_DIST];
    all[..hlit].copy_from_slice(&lit_len[..hlit]);
    all[hlit..hlit + hdist].copy_from_slice(&dist_len[..hdist]);
    proof { // @ghost-begin
        assert forall|i: int| 0 <= i < hlit + hdist implies #[trigger] all@[i] <= 15 by {
            if i < hlit {
                assert(all@.subrange(0, hlit as int)[i] == all@[i]);
                assert(lit_len@.subrange(0, hlit as int)[i] == lit_len@[i]);
            } else {
                assert(all@.subrange(hlit as int, hlit + hdist)[i - hlit] == all@[i]);
                assert(dist_len@.subrange(0, hdist as int)[i - hlit] == dist_len@[i - hlit]);
            }
        }
        assert forall|i: int| 0 <= i < hlit + hdist implies #[trigger] all@.subrange(0, hlit + hdist)[i] <= 15 by {
            assert(all@.subrange(0, hlit + hdist)[i] == all@[i]);
        }
    } // @ghost-end
    let mut cl_tokens = [ClToken { sym: 0, extra: 0 }; NUM_LITLEN + NUM_DIST];
    let cl_count = rle_code_lengths(&all[..hlit + hdist], &mut cl_tokens);
    let mut cl_freq = [0u32; NUM_CODELEN];
    for i in 0..cl_count
        invariant // @ghost
            cl_count <= 316, // @ghost
            cl_ok(cl_tokens@, cl_count as int), // @ghost
            forall|k: int| 0 <= k < 19 ==> #[trigger] cl_freq@[k] <= i, // @ghost
    {
        cl_freq[cl_tokens[i].sym as usize] += 1;
    }
    let mut cl_len = [0u8; NUM_CODELEN];
    build_lengths(&cl_freq, MAX_CODELEN_LEN, &mut cl_len).ok()?;
    proof { // @ghost-begin
        lemma_tables();
    } // @ghost-end
    // Code lengths sent: up to the last nonzero one in transmission order,
    // and at least four.
    let mut hclen = NUM_CODELEN;
    while hclen > 4 && cl_len[CODELEN_ORDER[hclen - 1]] == 0
        invariant // @ghost
            4 <= hclen <= 19, // @ghost
            forall|k: int| 0 <= k < 19 ==> #[trigger] CODELEN_ORDER@[k] < 19, // @ghost
        decreases hclen, // @ghost
    {
        hclen -= 1;
    }
    let mut cl_bits = 0u64;
    for i in 0..cl_count
        invariant // @ghost
            cl_count <= 316, // @ghost
            cl_ok(cl_tokens@, cl_count as int), // @ghost
            forall|k: int| 0 <= k < 19 ==> #[trigger] cl_len@[k] <= 7, // @ghost
            cl_bits <= 14 * i, // @ghost
    {
        let t = cl_tokens[i];
        cl_bits += u64::from(cl_len[t.sym as usize]) + u64::from(codelen_extra_bits(t.sym));
    }
    let bits = 3 + 5 + 5 + 4 + 3 * hclen as u64 + cl_bits + stats.body_bits(&lit_len, &dist_len);
    Some(DynamicPlan {
        lit: CodeTable::from_lengths(lit_len).ok()?,
        dist: CodeTable::from_lengths(dist_len).ok()?,
        codelen: CodeTable::from_lengths(cl_len).ok()?,
        hlit,
        hdist,
        hclen,
        cl_tokens,
        cl_count,
        bits,
    })
}

fn write_dynamic(w: &mut BitWriter, tokens: &[Token], p: &DynamicPlan, last: bool)
    requires // @ghost
        old(w).wf(), // @ghost
        p.wf(), // @ghost
        all_valid(tokens@), // @ghost
    ensures // @ghost
        final(w).wf(), // @ghost
{
    proof { // @ghost-begin
        lemma_tables();
    } // @ghost-end
    w.write_bits(u32::from(last) | (2 << 1), 3);
    w.write_bits((p.hlit - 257) as u32, 5);
    w.write_bits((p.hdist - 1) as u32, 5);
    w.write_bits((p.hclen - 4) as u32, 4);
    for i in 0..p.hclen
        invariant // @ghost
            w.wf(), // @ghost
            p.wf(), // @ghost
            forall|k: int| 0 <= k < 19 ==> #[trigger] CODELEN_ORDER@[k] < 19, // @ghost
    {
        w.write_bits(u32::from(p.codelen.len[CODELEN_ORDER[i]]), 3);
    }
    for i in 0..p.cl_count
        invariant // @ghost
            w.wf(), // @ghost
            p.wf(), // @ghost
    {
        let t = p.cl_tokens[i];
        p.codelen.put(w, t.sym as usize);
        w.write_bits(u32::from(t.extra), codelen_extra_bits(t.sym));
    }
    write_tokens(w, tokens, &p.lit, &p.dist);
}

} // verus!
