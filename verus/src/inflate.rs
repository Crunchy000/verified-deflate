//! The DEFLATE decoder (`src/inflate.rs`).
//!
//! The executable code below is the library's, token for token (CI checks
//! this with `.github/scripts/verus_drift.py`); every line marked
//! `// @ghost` is specification or proof. The theorem: for every input of
//! every length and every output limit, `decompress_with_limit` returns
//! (it never panics and never loops forever), and when it succeeds the
//! output is within the limit and the bytes consumed are within the input.
//! The loops terminate because each block and each symbol reads at least
//! one bit, and the bits left to read never grow.

use crate::bitio::BitReader;
use crate::error::InflateError;
use crate::huffman_decode::Decoder;
use crate::limit_depths::{lemma_p2_mono, p2};
use crate::lz77::copy_within_buffer;
use crate::tables::{
    codelen_extra_bits, fixed_litlen_lengths, lemma_tables, CODELEN_ORDER, DIST_BASE, DIST_EXTRA, END_OF_BLOCK,
    FIXED_DIST_LENGTHS, LENGTH_BASE, LENGTH_EXTRA, NUM_CODELEN, NUM_DIST, NUM_LITLEN,
};
use vstd::prelude::*;

verus! {

/// 2^3 .. 2^7 and 2^13.
proof fn lemma_p2_small()
    ensures
        p2(3) == 8,
        p2(4) == 16,
        p2(5) == 32,
        p2(7) == 128,
        p2(13) == 8192,
{
    reveal_with_fuel(p2, 14);
}

/// Result of [`decompress_with_limit`].
pub struct Decompressed {
    /// The decompressed bytes.
    pub data: Vec<u8>,
    /// Bytes of input used, up to and including the byte holding the last
    /// bit of the final block.
    pub consumed: usize,
}

/// Decompresses a raw DEFLATE stream with no output limit.
pub fn decompress(input: &[u8]) -> Result<Vec<u8>, InflateError> {
    decompress_with_limit(input, usize::MAX).map(|d| d.data)
}

/// Decompresses a raw DEFLATE stream, failing with
/// [`InflateError::OutputLimitExceeded`] rather than producing more than
/// `max_output` bytes.
#[verifier::loop_isolation(false)]
pub fn decompress_with_limit(input: &[u8], max_output: usize) -> (r: Result<Decompressed, InflateError>)
    ensures // @ghost
        r is Ok ==> r->Ok_0.data@.len() <= max_output, // @ghost
        r is Ok ==> r->Ok_0.consumed <= input@.len(), // @ghost
{
    let mut st = State {
        br: BitReader::new(input),
        out: Output::new(input.len(), max_output),
        lit: Decoder::EMPTY,
        dist: Decoder::EMPTY,
        lengths: [0; NUM_LITLEN + NUM_DIST],
    };
    loop
        invariant // @ghost
            st.wf(), // @ghost
            st.out.limit() == max_output, // @ghost
            st.br.input_len() == input@.len(), // @ghost
        decreases st.br.remaining(), // @ghost
    {
        let header = st.br.read_bits(3)?;
        proof { // @ghost-begin
            lemma_p2_small();
        } // @ghost-end
        match header >> 1 {
            0 => st.stored()?,
            1 => codes(&mut st.br, &mut st.out, &FIXED_LIT, &FIXED_DIST)?,
            2 => {
                read_dynamic_codes(&mut st.br, &mut st.lit, &mut st.dist, &mut st.lengths)?;
                codes(&mut st.br, &mut st.out, &st.lit, &st.dist)?;
            }
            _ => return Err(InflateError::InvalidBlockType),
        }
        if header & 1 == 1 {
            break;
        }
    }
    Ok(Decompressed { consumed: st.br.bytes_consumed(), data: st.out.finish() })
}

/// Initial output buffer size bounds.
const INITIAL_CAPACITY: usize = 4096;

const MAX_INITIAL_CAPACITY: usize = 1 << 26;

exec const FIXED_LIT: Decoder
    ensures // @ghost
        FIXED_LIT.wf(), // @ghost
{
    match Decoder::new(&fixed_litlen_lengths()) {
        Ok(d) => d,
        Err(_) => Decoder::EMPTY,
    }
}

exec const FIXED_DIST: Decoder
    ensures // @ghost
        FIXED_DIST.wf(), // @ghost
{
    match Decoder::new(&FIXED_DIST_LENGTHS) {
        Ok(d) => d,
        Err(_) => Decoder::EMPTY,
    }
}

/// The decoder's output: a zero-filled buffer written by index and grown
/// by doubling when needed.
pub(crate) struct Output {
    buf: Vec<u8>,
    len: usize,
    limit: usize,
}

impl Output {
    /// What has been written lies within the buffer, which lies within the
    /// limit.
    pub(crate) closed spec fn wf(&self) -> bool {
        &&& self.len <= self.buf.len()
        &&& self.buf.len() <= self.limit
    }

    pub(crate) closed spec fn limit(&self) -> usize {
        self.limit
    }

    /// An empty output for a stream of `input_len` bytes.
    fn new(input_len: usize, limit: usize) -> (r: Self)
        ensures // @ghost
            r.wf(), // @ghost
            r.limit == limit, // @ghost
    {
        proof { // @ghost-begin
            assert(MAX_INITIAL_CAPACITY == 67108864) by (compute_only);
        } // @ghost-end
        let cap = input_len.clamp(INITIAL_CAPACITY, MAX_INITIAL_CAPACITY);
        let cap = if cap < limit { cap } else { limit };
        Self { buf: vec![0; cap], len: 0, limit }
    }

    /// Makes room for `n` more bytes, failing if that would exceed the
    /// limit. Invariants: `len <= limit` and `len <= buf.len()`.
    #[inline]
    fn reserve(&mut self, n: usize) -> (r: Result<(), InflateError>)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).len == old(self).len, // @ghost
            final(self).limit == old(self).limit, // @ghost
            r is Ok ==> final(self).len + n <= final(self).buf.len(), // @ghost
    {
        if n > self.limit - self.len {
            return Err(InflateError::OutputLimitExceeded);
        }
        if n > self.buf.len() - self.len {
            self.grow(n);
        }
        Ok(())
    }

    #[cold]
    pub(crate) fn grow(&mut self, n: usize)
        requires // @ghost
            old(self).wf(), // @ghost
            n <= old(self).limit() - old(self).len_spec(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).len_spec() == old(self).len_spec(), // @ghost
            final(self).limit() == old(self).limit(), // @ghost
            final(self).len_spec() + n <= final(self).cap_spec(), // @ghost
    {
        // At least double, but never past the limit (`reserve` has checked
        // that `len + n` is within it). Resizing in place lets the
        // allocator extend the block, so only the new part is zeroed.
        let want = (self.len + n).max(self.buf.len().saturating_mul(2)).min(self.limit);
        self.buf.resize(want, 0);
    }

    pub(crate) closed spec fn len_spec(&self) -> usize {
        self.len
    }

    pub(crate) closed spec fn cap_spec(&self) -> usize {
        self.buf.len()
    }

    fn finish(mut self) -> (r: Vec<u8>)
        requires // @ghost
            self.wf(), // @ghost
        ensures // @ghost
            r@.len() <= self.limit, // @ghost
    {
        self.buf.truncate(self.len);
        self.buf
    }
}

struct State<'a> {
    br: BitReader<'a>,
    out: Output,
    /// Decoders for the current dynamic block, rebuilt in place.
    lit: Decoder,
    dist: Decoder,
    /// Scratch space for a dynamic block's code lengths.
    lengths: [u8; NUM_LITLEN + NUM_DIST],
}

impl State<'_> {
    spec fn wf(&self) -> bool {
        &&& self.br.wf()
        &&& self.out.wf()
        &&& self.lit.wf()
        &&& self.dist.wf()
    }

    fn stored(&mut self) -> (r: Result<(), InflateError>)
        requires // @ghost
            old(self).wf(), // @ghost
        ensures // @ghost
            final(self).wf(), // @ghost
            final(self).out.limit == old(self).out.limit, // @ghost
            final(self).br.input_len() == old(self).br.input_len(), // @ghost
            final(self).br.remaining() <= old(self).br.remaining(), // @ghost
    {
        self.br.align_to_byte();
        let len = self.br.read_bits(16)?;
        let nlen = self.br.read_bits(16)?;
        if len != (!nlen & 0xffff) {
            return Err(InflateError::StoredLengthMismatch);
        }
        let len = len as usize;
        self.out.reserve(len)?;
        let start = self.out.len;
        self.br.read_aligned_bytes(&mut self.out.buf.as_mut_slice()[start..start + len])?;
        self.out.len += len;
        Ok(())
    }
}

/// Decodes one block's symbols with the given codes.
fn codes(br: &mut BitReader<'_>, out: &mut Output, lit: &Decoder, dist: &Decoder) -> (r: Result<(), InflateError>)
    requires // @ghost
        old(br).wf(), // @ghost
        old(out).wf(), // @ghost
        lit.wf(), // @ghost
        dist.wf(), // @ghost
    ensures // @ghost
        final(br).wf(), // @ghost
        final(out).wf(), // @ghost
        final(out).limit == old(out).limit, // @ghost
        final(br).input_len() == old(br).input_len(), // @ghost
        final(br).remaining() <= old(br).remaining(), // @ghost
{
    proof { // @ghost-begin
        lemma_tables();
        lemma_p2_small();
    } // @ghost-end
    loop
        invariant // @ghost
            br.wf(), // @ghost
            out.wf(), // @ghost
            lit.wf(), // @ghost
            dist.wf(), // @ghost
            out.limit == old(out).limit, // @ghost
            br.input_len() == old(br).input_len(), // @ghost
            br.remaining() <= old(br).remaining(), // @ghost
            forall|i: int| 0 <= i < 29 ==> #[trigger] LENGTH_BASE@[i] <= 258, // @ghost
            forall|i: int| 0 <= i < 29 ==> #[trigger] LENGTH_EXTRA@[i] <= 5, // @ghost
            forall|i: int| 0 <= i < 30 ==> 1 <= #[trigger] DIST_BASE@[i] <= 24577, // @ghost
            forall|i: int| 0 <= i < 30 ==> #[trigger] DIST_EXTRA@[i] <= 13, // @ghost
            p2(5) == 32, // @ghost
            p2(13) == 8192, // @ghost
        decreases br.remaining(), // @ghost
    {
        let sym = lit.decode(br)? as usize;
        if sym < 256 {
            out.reserve(1)?;
            out.buf[out.len] = sym as u8;
            out.len += 1;
        } else if sym == END_OF_BLOCK {
            return Ok(());
        } else {
            let li = sym - 257;
            if li >= LENGTH_BASE.len() {
                return Err(InflateError::InvalidSymbol);
            }
            proof { // @ghost-begin
                lemma_p2_mono(LENGTH_EXTRA@[li as int] as nat, 5);
            } // @ghost-end
            let len = LENGTH_BASE[li] as usize + br.read_bits(u32::from(LENGTH_EXTRA[li]))? as usize;
            let ds = dist.decode(br)? as usize;
            if ds >= DIST_BASE.len() {
                return Err(InflateError::InvalidSymbol);
            }
            proof { // @ghost-begin
                lemma_p2_mono(DIST_EXTRA@[ds as int] as nat, 13);
            } // @ghost-end
            let d = DIST_BASE[ds] as usize + br.read_bits(u32::from(DIST_EXTRA[ds]))? as usize;
            if d > out.len {
                return Err(InflateError::DistanceTooFarBack);
            }
            out.reserve(len)?;
            copy_within_buffer(&mut out.buf, out.len, d, len);
            out.len += len;
        }
    }
}

/// Reads a dynamic block header (§3.2.7) and builds its two decoders.
#[allow(clippy::needless_range_loop)] // an index loop, as the Verus copy needs
fn read_dynamic_codes(
    br: &mut BitReader<'_>,
    lit: &mut Decoder,
    dist: &mut Decoder,
    lengths: &mut [u8; NUM_LITLEN + NUM_DIST],
) -> (r: Result<(), InflateError>)
    requires // @ghost
        old(br).wf(), // @ghost
        old(lit).wf(), // @ghost
        old(dist).wf(), // @ghost
    ensures // @ghost
        final(br).wf(), // @ghost
        final(lit).wf(), // @ghost
        final(dist).wf(), // @ghost
        final(br).input_len() == old(br).input_len(), // @ghost
        final(br).remaining() <= old(br).remaining(), // @ghost
{
    proof { // @ghost-begin
        lemma_tables();
        lemma_p2_small();
    } // @ghost-end
    let nlen = br.read_bits(5)? as usize + 257;
    let ndist = br.read_bits(5)? as usize + 1;
    let ncode = br.read_bits(4)? as usize + 4;
    if nlen > NUM_LITLEN || ndist > NUM_DIST {
        return Err(InflateError::TooManyCodes);
    }
    let mut cl = [0u8; NUM_CODELEN];
    for i in 0..ncode
        invariant // @ghost
            br.wf(), // @ghost
            br.input_len() == old(br).input_len(), // @ghost
            br.remaining() <= old(br).remaining(), // @ghost
            ncode <= 19, // @ghost
            lit.wf(), // @ghost
            dist.wf(), // @ghost
            forall|j: int| 0 <= j < 19 ==> #[trigger] CODELEN_ORDER@[j] < 19, // @ghost
    {
        cl[CODELEN_ORDER[i]] = br.read_bits(3)? as u8;
    }
    let mut clcode = Decoder::EMPTY;
    clcode.rebuild(&cl).map_err(|_| InflateError::InvalidCodeLengthCode)?;
    if !clcode.is_complete() {
        return Err(InflateError::InvalidCodeLengthCode);
    }

    // Every entry of lengths[..total] is written below before it is read.
    let total = nlen + ndist;
    let mut idx = 0;
    while idx < total
        invariant // @ghost
            br.wf(), // @ghost
            br.input_len() == old(br).input_len(), // @ghost
            br.remaining() <= old(br).remaining(), // @ghost
            clcode.wf(), // @ghost
            lit.wf(), // @ghost
            dist.wf(), // @ghost
            idx <= total <= 316, // @ghost
            p2(7) == 128, // @ghost
        decreases total - idx, // @ghost
    {
        let sym = clcode.decode(br)? as u8;
        proof { // @ghost-begin
            lemma_p2_mono(codelen_extra_bits(sym) as nat, 7);
        } // @ghost-end
        let extra = br.read_bits(codelen_extra_bits(sym))?;
        expand_code_length(sym, extra, &mut lengths[..total], &mut idx)?;
    }
    if lengths[END_OF_BLOCK] == 0 {
        return Err(InflateError::MissingEndOfBlock);
    }

    lit.rebuild(&lengths[..nlen]).map_err(|_| InflateError::InvalidLiteralLengthCode)?;
    if !(lit.is_complete() || lit.is_single_unit_code()) {
        return Err(InflateError::InvalidLiteralLengthCode);
    }
    dist.rebuild(&lengths[nlen..total]).map_err(|_| InflateError::InvalidDistanceCode)?;
    if !(dist.is_complete() || dist.is_single_unit_code() || dist.coded_symbols() == 0) {
        return Err(InflateError::InvalidDistanceCode);
    }
    Ok(())
}

/// Applies one code-length symbol (with its extra-bits value) to
/// `lengths`, advancing `idx`. Requires `*idx <= lengths.len()`.
pub(crate) fn expand_code_length(sym: u8, extra: u32, lengths: &mut [u8], idx: &mut usize) -> (r: Result<(), InflateError>)
    requires // @ghost
        *old(idx) <= old(lengths)@.len(), // @ghost
        extra < 128, // @ghost
    ensures // @ghost
        final(lengths)@.len() == old(lengths)@.len(), // @ghost
        *final(idx) <= final(lengths)@.len(), // @ghost
        r is Ok ==> *final(idx) > *old(idx), // @ghost
{
    let (value, repeat) = match sym {
        0..=15 => (sym, 1),
        16 => {
            if *idx == 0 {
                return Err(InflateError::RepeatWithoutPrevious);
            }
            (lengths[*idx - 1], 3 + extra as usize)
        }
        17 => (0, 3 + extra as usize),
        18 => (0, 11 + extra as usize),
        _ => return Err(InflateError::InvalidSymbol),
    };
    if repeat > lengths.len() - *idx {
        return Err(InflateError::CodeLengthOverflow);
    }
    lengths[*idx..*idx + repeat].fill(value);
    *idx += repeat;
    Ok(())
}

} // verus!
