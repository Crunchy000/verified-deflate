//! LZMA compression and decompression, and `.lzma` files.
//!
//! LZMA codes literals and matches into a range coder with adaptive
//! binary probabilities. Its parameters are `lc`, `lp` and `pb` (how many
//! bits of the previous byte and of the position select the literal and
//! match probabilities) and the dictionary size (how far back a match may
//! reach). This module is the core that [`crate::lzma2`], [`crate::xz`],
//! ZIP method 14 and 7z archives build on, and it reads and writes the
//! `.lzma` format (a 13-byte header, then the stream).
//!
//! The decoder accepts what liblzma (XZ Utils) accepts: the first range
//! coder byte must be 0, a match may not reach before the start of the
//! data or further back than the dictionary, a stream of known size must
//! end exactly there with the range coder finished (an end marker may
//! follow in `.lzma` files), and one of unknown size must end with the
//! marker. It also takes `lc` up to 8 as 7-Zip does (liblzma stops at
//! `lc + lp` = 4). Decompression takes an output limit.
//!
//! The encoder uses a hash-chain match finder and the fast parser of
//! liblzma's presets 0–3, with a deeper search at higher levels.
//!
//! ```
//! use verified_deflate::lzma;
//!
//! let data = b"to be or not to be, that is the question".repeat(100);
//! let packed = lzma::compress(&data, 6);
//! assert_eq!(lzma::decompress_with_limit(&packed, 1 << 20).unwrap(), data);
//! ```

use core::fmt;

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
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LzmaError {
    /// The properties (`lc`, `lp`, `pb` or the dictionary size) are invalid.
    BadProperties,
    /// The data ended in the middle of the stream.
    Truncated,
    /// The stream is corrupt; the message says how.
    Corrupt(&'static str),
    /// Decompressing would exceed the output limit.
    OutputLimitExceeded,
    /// Bytes follow the end of a `.lzma` file's stream.
    TrailingData,
}

impl fmt::Display for LzmaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadProperties => f.write_str("invalid LZMA properties"),
            Self::Truncated => f.write_str("unexpected end of LZMA data"),
            Self::Corrupt(why) => write!(f, "corrupt LZMA data: {why}"),
            Self::OutputLimitExceeded => f.write_str("decompressed data exceeds the output limit"),
            Self::TrailingData => f.write_str("trailing data after the LZMA stream"),
        }
    }
}

impl std::error::Error for LzmaError {}

/// An LZMA stream's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LzmaProps {
    /// Literal context bits, 0–8.
    pub lc: u8,
    /// Literal position bits, 0–4.
    pub lp: u8,
    /// Position bits, 0–4.
    pub pb: u8,
    /// Dictionary size: how far back matches may reach.
    pub dict_size: u32,
}

impl LzmaProps {
    /// The usual parameters: `lc` 3, `lp` 0, `pb` 2.
    pub const fn new(dict_size: u32) -> Self {
        Self { lc: 3, lp: 0, pb: 2, dict_size }
    }

    /// Decodes `lc`, `lp` and `pb` from their byte, `(pb × 5 + lp) × 9 + lc`.
    pub const fn from_byte(b: u8, dict_size: u32) -> Result<Self, LzmaError> {
        if b >= 9 * 5 * 5 {
            return Err(LzmaError::BadProperties);
        }
        Ok(Self { lc: b % 9, lp: (b / 9) % 5, pb: b / 45, dict_size })
    }

    /// The byte that holds `lc`, `lp` and `pb`.
    pub const fn byte(&self) -> u8 {
        (self.pb * 5 + self.lp) * 9 + self.lc
    }

    /// The five-byte form used by `.lzma` headers, 7z and ZIP: the
    /// `lc`/`lp`/`pb` byte and the dictionary size, little-endian.
    pub fn parse(b: &[u8; 5]) -> Result<Self, LzmaError> {
        Self::from_byte(b[0], u32::from_le_bytes([b[1], b[2], b[3], b[4]]))
    }

    /// The five-byte form; see [`parse`](Self::parse).
    pub fn to_bytes(&self) -> [u8; 5] {
        let d = self.dict_size.to_le_bytes();
        [self.byte(), d[0], d[1], d[2], d[3]]
    }

    fn check(&self) -> Result<(), LzmaError> {
        if self.lc > 8 || self.lp > 4 || self.pb > 4 {
            Err(LzmaError::BadProperties)
        } else {
            Ok(())
        }
    }

    /// The dictionary the decoder keeps, as liblzma sizes it: at least
    /// 4 KiB, rounded up to 16 bytes.
    fn effective_dict(&self) -> usize {
        ((self.dict_size.max(4096) as usize).saturating_add(15)) & !15
    }
}

// ---------------------------------------------------------------------------
// The probability model, shared by decoder and encoder

#[derive(Clone)]
struct LenModel {
    choice: u16,
    choice2: u16,
    low: [[u16; 8]; POS_STATES_MAX],
    mid: [[u16; 8]; POS_STATES_MAX],
    high: [u16; 256],
}

impl LenModel {
    const fn new() -> Self {
        Self {
            choice: PROB_INIT,
            choice2: PROB_INIT,
            low: [[PROB_INIT; 8]; 16],
            mid: [[PROB_INIT; 8]; 16],
            high: [PROB_INIT; 256],
        }
    }
}

/// Probabilities, state and the four most recent distances.
#[derive(Clone)]
pub(crate) struct Model {
    lc: u32,
    lp_mask: usize,
    pb_mask: usize,
    literal: Vec<u16>,
    is_match: [[u16; POS_STATES_MAX]; STATES],
    is_rep: [u16; STATES],
    is_rep0: [u16; STATES],
    is_rep1: [u16; STATES],
    is_rep2: [u16; STATES],
    is_rep0_long: [[u16; POS_STATES_MAX]; STATES],
    dist_slot: [[u16; DIST_SLOTS]; DIST_STATES],
    dist_special: [u16; 1 + FULL_DISTANCES - END_POS_MODEL_INDEX as usize],
    dist_align: [u16; 1 << ALIGN_BITS],
    len: LenModel,
    rep_len: LenModel,
    state: usize,
    reps: [u32; 4],
}

impl Model {
    pub(crate) fn new(props: &LzmaProps) -> Self {
        Self {
            lc: u32::from(props.lc),
            lp_mask: (1 << props.lp) - 1,
            pb_mask: (1 << props.pb) - 1,
            literal: vec![PROB_INIT; 0x300 << (props.lc + props.lp)],
            is_match: [[PROB_INIT; 16]; STATES],
            is_rep: [PROB_INIT; STATES],
            is_rep0: [PROB_INIT; STATES],
            is_rep1: [PROB_INIT; STATES],
            is_rep2: [PROB_INIT; STATES],
            is_rep0_long: [[PROB_INIT; 16]; STATES],
            dist_slot: [[PROB_INIT; DIST_SLOTS]; DIST_STATES],
            dist_special: [PROB_INIT; 1 + FULL_DISTANCES - END_POS_MODEL_INDEX as usize],
            dist_align: [PROB_INIT; 16],
            len: LenModel::new(),
            rep_len: LenModel::new(),
            state: 0,
            reps: [0; 4],
        }
    }

    /// Resets probabilities, state and distances (an LZMA2 state reset).
    pub(crate) fn reset(&mut self, props: &LzmaProps) {
        *self = Self::new(props);
    }

    /// Where the literal probabilities for this position and previous
    /// byte start.
    fn literal_base(&self, pos: usize, prev: u8) -> usize {
        0x300 * (((pos & self.lp_mask) << self.lc) + (usize::from(prev) >> (8 - self.lc)))
    }
}

const fn after_literal(s: usize) -> usize {
    if s < 4 {
        0
    } else if s < 10 {
        s - 3
    } else {
        s - 6
    }
}
const fn after_match(s: usize) -> usize {
    if s < 7 {
        7
    } else {
        10
    }
}
const fn after_rep(s: usize) -> usize {
    if s < 7 {
        8
    } else {
        11
    }
}
const fn after_short_rep(s: usize) -> usize {
    if s < 7 {
        9
    } else {
        11
    }
}
const fn dist_state(len: usize) -> usize {
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
    /// Set when a byte past the end was needed.
    overrun: bool,
}

impl<'a> RangeDecoder<'a> {
    /// Starts decoding: five bytes, the first of which must be 0.
    fn new(data: &'a [u8]) -> Result<Self, LzmaError> {
        if data.len() < 5 {
            return Err(LzmaError::Truncated);
        }
        if data[0] != 0 {
            return Err(LzmaError::Corrupt("range coder does not start with 0"));
        }
        let code = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
        Ok(Self { data, pos: 5, range: u32::MAX, code, overrun: false })
    }

    #[inline(always)]
    fn normalize(&mut self) {
        if self.range < TOP {
            self.range <<= 8;
            let b = match self.data.get(self.pos) {
                Some(&b) => b,
                None => {
                    self.overrun = true;
                    0
                }
            };
            self.pos += 1;
            self.code = (self.code << 8) | u32::from(b);
        }
    }

    #[inline(always)]
    fn bit(&mut self, p: &mut u16) -> u32 {
        self.normalize();
        let bound = (self.range >> 11) * u32::from(*p);
        if self.code < bound {
            self.range = bound;
            *p += ((BIT_MODEL_TOTAL - u32::from(*p)) >> MOVE_BITS) as u16;
            0
        } else {
            self.range -= bound;
            self.code -= bound;
            *p -= *p >> MOVE_BITS;
            1
        }
    }

    fn tree(&mut self, probs: &mut [u16], bits: u32) -> u32 {
        let mut m = 1usize;
        for _ in 0..bits {
            m = (m << 1) | self.bit(&mut probs[m]) as usize;
        }
        m as u32 - (1 << bits)
    }

    fn reverse_tree(&mut self, probs: &mut [u16], base: usize, bits: u32) -> u32 {
        let mut m = 1usize;
        let mut sym = 0;
        for i in 0..bits {
            let b = self.bit(&mut probs[base + m]);
            m = (m << 1) | b as usize;
            sym |= b << i;
        }
        sym
    }

    /// Bits with probability one half, with liblzma's arithmetic exactly
    /// (it differs from a plain comparison once a corrupt stream has
    /// pushed `code` past `range`).
    fn direct(&mut self, bits: u32) -> u32 {
        let mut v = 0u32;
        for _ in 0..bits {
            self.normalize();
            self.range >>= 1;
            self.code = self.code.wrapping_sub(self.range);
            let mask = 0u32.wrapping_sub(self.code >> 31);
            self.code = self.code.wrapping_add(self.range & mask);
            v = (v << 1).wrapping_add(mask.wrapping_add(1));
        }
        v
    }

    fn len(&mut self, m: &mut LenModel, pos_state: usize) -> usize {
        if self.bit(&mut m.choice) == 0 {
            2 + self.tree(&mut m.low[pos_state], 3) as usize
        } else if self.bit(&mut m.choice2) == 0 {
            10 + self.tree(&mut m.mid[pos_state], 3) as usize
        } else {
            18 + self.tree(&mut m.high, 8) as usize
        }
    }
}

// ---------------------------------------------------------------------------
// Decoder

/// How a stream ends.
#[derive(Clone, Copy)]
pub(crate) enum End {
    /// After exactly this many bytes of output; `allow_marker` lets an end
    /// marker follow them.
    Known { size: u64, allow_marker: bool },
    /// At the end marker.
    Marker,
}

/// Decodes one LZMA stream (an LZMA2 chunk's, or a whole LZMA1 one) onto
/// `out`, whose bytes from `dict_start` on are the dictionary, producing
/// at most `max_output` bytes in `out` in all. Returns the input used.
pub(crate) fn decode_stream(
    model: &mut Model,
    props: &LzmaProps,
    input: &[u8],
    end: End,
    out: &mut Vec<u8>,
    dict_start: usize,
    max_output: usize,
) -> Result<usize, LzmaError> {
    let mut rc = RangeDecoder::new(input)?;
    let dict_size = props.effective_dict();
    let start_len = out.len();
    let (limit, mut marker_ok) = match end {
        End::Known { size, .. } => (Some(usize::try_from(size).unwrap_or(usize::MAX)), false),
        End::Marker => (None, true),
    };
    let target = limit.map(|n| start_len.saturating_add(n));
    if target.is_some_and(|t| t > max_output) {
        return Err(LzmaError::OutputLimitExceeded);
    }
    let m = model;
    loop {
        if rc.overrun {
            return Err(LzmaError::Truncated);
        }
        let pos = out.len() - dict_start;
        if Some(out.len()) == target {
            // In rare cases one more byte has to be read anyway.
            rc.normalize();
            if rc.overrun {
                return Err(LzmaError::Truncated);
            }
            if rc.code == 0 {
                return Ok(rc.pos);
            }
            match end {
                End::Known { allow_marker: true, .. } => marker_ok = true,
                _ => return Err(LzmaError::Corrupt("data after the end of the stream")),
            }
        }
        let full = pos.min(dict_size);
        let pos_state = pos & m.pb_mask;
        let state = m.state;
        if rc.bit(&mut m.is_match[state][pos_state]) == 0 {
            // Literal.
            if Some(out.len()) == target || out.len() >= max_output {
                return Err(if target.is_some() {
                    LzmaError::Corrupt("data after the end of the stream")
                } else {
                    LzmaError::OutputLimitExceeded
                });
            }
            let prev = if pos > 0 { out[out.len() - 1] } else { 0 };
            let base = m.literal_base(pos, prev);
            let probs = &mut m.literal[base..base + 0x300];
            let mut sym = 1usize;
            if state >= 7 {
                let mut match_byte = out[out.len() - m.reps[0] as usize - 1] as usize;
                loop {
                    let match_bit = (match_byte >> 7) & 1;
                    match_byte <<= 1;
                    let b = rc.bit(&mut probs[((1 + match_bit) << 8) + sym]) as usize;
                    sym = (sym << 1) | b;
                    if match_bit != b || sym >= 0x100 {
                        break;
                    }
                }
            }
            while sym < 0x100 {
                sym = (sym << 1) | rc.bit(&mut probs[sym]) as usize;
            }
            out.push(sym as u8);
            m.state = after_literal(state);
            continue;
        }
        let len;
        if rc.bit(&mut m.is_rep[state]) == 0 {
            // Match with a new distance.
            len = rc.len(&mut m.len, pos_state);
            m.state = after_match(state);
            let slot = rc.tree(&mut m.dist_slot[dist_state(len)], 6);
            let dist = if slot < 4 {
                slot
            } else {
                let footer = (slot >> 1) - 1;
                let base = (2 | (slot & 1)) << footer;
                if slot < END_POS_MODEL_INDEX {
                    base + rc.reverse_tree(&mut m.dist_special, (base - slot) as usize, footer)
                } else {
                    let high = rc.direct(footer - ALIGN_BITS) << ALIGN_BITS;
                    base + high + rc.reverse_tree(&mut m.dist_align, 0, ALIGN_BITS)
                }
            };
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
            if rc.bit(&mut m.is_rep0[state]) == 0 {
                if rc.bit(&mut m.is_rep0_long[state][pos_state]) == 0 {
                    // Short rep: one byte from the last distance.
                    if Some(out.len()) == target || out.len() >= max_output {
                        return Err(if target.is_some() {
                            LzmaError::Corrupt("data after the end of the stream")
                        } else {
                            LzmaError::OutputLimitExceeded
                        });
                    }
                    m.state = after_short_rep(state);
                    out.push(out[out.len() - m.reps[0] as usize - 1]);
                    continue;
                }
            } else {
                let dist;
                if rc.bit(&mut m.is_rep1[state]) == 0 {
                    dist = m.reps[1];
                } else {
                    if rc.bit(&mut m.is_rep2[state]) == 0 {
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
            len = rc.len(&mut m.rep_len, pos_state);
            m.state = after_rep(state);
        }
        // Copy the match, as far as the end and the limit allow.
        let room = match target {
            Some(t) => t - out.len(),
            None => max_output - out.len(),
        };
        let n = len.min(room);
        let d = m.reps[0] as usize + 1;
        let from = out.len() - d;
        if d >= n {
            out.extend_from_within(from..from + n);
        } else {
            for i in 0..n {
                out.push(out[from + i]);
            }
        }
        if n < len {
            return Err(if target.is_some() {
                LzmaError::Corrupt("match runs past the end of the stream")
            } else {
                LzmaError::OutputLimitExceeded
            });
        }
    }
}

/// Decoded data and the input it used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decompressed {
    /// The decompressed bytes.
    pub data: Vec<u8>,
    /// Bytes of input the stream used.
    pub consumed: usize,
}

/// Decodes a raw LZMA stream (as in ZIP method 14 and 7z) with these
/// properties. With `size`, the stream holds exactly that many bytes,
/// optionally followed by an end marker; without, it must end with the
/// marker. At most `max_output` bytes are produced.
pub fn decompress_raw(
    input: &[u8],
    props: &LzmaProps,
    size: Option<u64>,
    max_output: usize,
) -> Result<Decompressed, LzmaError> {
    props.check()?;
    let mut model = Model::new(props);
    let mut out = Vec::new();
    if let Some(n) = size {
        out.reserve(usize::try_from(n).unwrap_or(0).min(max_output).min(1 << 26));
    }
    let end = match size {
        Some(size) => End::Known { size, allow_marker: true },
        None => End::Marker,
    };
    let consumed = decode_stream(&mut model, props, input, end, &mut out, 0, max_output)?;
    Ok(Decompressed { data: out, consumed })
}

/// The 13-byte `.lzma` header: properties, dictionary size, and the
/// uncompressed size (`None` when unknown, stored as all ones).
pub fn parse_header(input: &[u8]) -> Result<(LzmaProps, Option<u64>), LzmaError> {
    if input.len() < 13 {
        return Err(LzmaError::Truncated);
    }
    let props = LzmaProps::parse(&[input[0], input[1], input[2], input[3], input[4]])?;
    props.check()?;
    let mut s = [0u8; 8];
    s.copy_from_slice(&input[5..13]);
    let size = u64::from_le_bytes(s);
    Ok((props, (size != u64::MAX).then_some(size)))
}

/// Decompresses a `.lzma` file with no output limit. For untrusted input
/// prefer [`decompress_with_limit`].
pub fn decompress(input: &[u8]) -> Result<Vec<u8>, LzmaError> {
    decompress_with_limit(input, usize::MAX)
}

/// Decompresses a `.lzma` file, producing at most `max_output` bytes.
/// Nothing may follow the stream.
pub fn decompress_with_limit(input: &[u8], max_output: usize) -> Result<Vec<u8>, LzmaError> {
    let d = read_file(input, max_output)?;
    if d.consumed != input.len() {
        return Err(LzmaError::TrailingData);
    }
    Ok(d.data)
}

/// Decodes the `.lzma` file at the start of `input`, reporting the bytes
/// it used (header included); bytes after it are left alone.
pub fn read_file(input: &[u8], max_output: usize) -> Result<Decompressed, LzmaError> {
    let (props, size) = parse_header(input)?;
    let d = decompress_raw(&input[13..], &props, size, max_output)?;
    Ok(Decompressed { data: d.data, consumed: 13 + d.consumed })
}

// ---------------------------------------------------------------------------
// Range encoder

pub(crate) struct RangeEncoder {
    low: u64,
    range: u32,
    cache: u8,
    cache_size: u64,
    pub(crate) out: Vec<u8>,
}

impl RangeEncoder {
    pub(crate) fn new() -> Self {
        Self { low: 0, range: u32::MAX, cache: 0, cache_size: 1, out: Vec::new() }
    }

    fn shift_low(&mut self) {
        if (self.low as u32) < 0xFF00_0000 || (self.low >> 32) != 0 {
            let carry = (self.low >> 32) as u8;
            let mut temp = self.cache;
            loop {
                self.out.push(temp.wrapping_add(carry));
                temp = 0xFF;
                self.cache_size -= 1;
                if self.cache_size == 0 {
                    break;
                }
            }
            self.cache = (self.low >> 24) as u8;
        }
        self.cache_size += 1;
        self.low = (self.low & 0x00FF_FFFF) << 8;
    }

    #[inline(always)]
    fn bit(&mut self, p: &mut u16, bit: u32) {
        let bound = (self.range >> 11) * u32::from(*p);
        if bit == 0 {
            self.range = bound;
            *p += ((BIT_MODEL_TOTAL - u32::from(*p)) >> MOVE_BITS) as u16;
        } else {
            self.low += u64::from(bound);
            self.range -= bound;
            *p -= *p >> MOVE_BITS;
        }
        while self.range < TOP {
            self.range <<= 8;
            self.shift_low();
        }
    }

    fn tree(&mut self, probs: &mut [u16], bits: u32, value: u32) {
        let mut m = 1usize;
        for i in (0..bits).rev() {
            let b = (value >> i) & 1;
            self.bit(&mut probs[m], b);
            m = (m << 1) | b as usize;
        }
    }

    fn reverse_tree(&mut self, probs: &mut [u16], base: usize, bits: u32, mut value: u32) {
        let mut m = 1usize;
        for _ in 0..bits {
            let b = value & 1;
            value >>= 1;
            self.bit(&mut probs[base + m], b);
            m = (m << 1) | b as usize;
        }
    }

    fn direct(&mut self, value: u32, bits: u32) {
        for i in (0..bits).rev() {
            self.range >>= 1;
            if (value >> i) & 1 == 1 {
                self.low += u64::from(self.range);
            }
            while self.range < TOP {
                self.range <<= 8;
                self.shift_low();
            }
        }
    }

    fn len(&mut self, m: &mut LenModel, len: usize, pos_state: usize) {
        let l = (len - MATCH_LEN_MIN) as u32;
        if l < 8 {
            self.bit(&mut m.choice, 0);
            self.tree(&mut m.low[pos_state], 3, l);
        } else if l < 16 {
            self.bit(&mut m.choice, 1);
            self.bit(&mut m.choice2, 0);
            self.tree(&mut m.mid[pos_state], 3, l - 8);
        } else {
            self.bit(&mut m.choice, 1);
            self.bit(&mut m.choice2, 1);
            self.tree(&mut m.high, 8, l - 16);
        }
    }

    /// Bytes the stream will take if finished now.
    pub(crate) fn pending(&self) -> usize {
        self.out.len() + self.cache_size as usize + 4
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        for _ in 0..5 {
            self.shift_low();
        }
        self.out
    }
}

// ---------------------------------------------------------------------------
// Encoder

/// Compression settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LzmaOptions {
    /// `lc`, `lp`, `pb` and the dictionary size.
    pub props: LzmaProps,
    /// A match this long is taken at once.
    pub nice_len: usize,
    /// How many earlier positions the match finder tries.
    pub depth: usize,
}

impl LzmaOptions {
    /// The settings for a level from 0 (fastest) to 9 (smallest output);
    /// higher values are taken as 9. Dictionaries grow from 256 KiB to 64
    /// MiB, as in xz's presets.
    pub fn level(level: u8) -> Self {
        let (dict_log, nice_len, depth) = match level {
            0 => (18, 32, 4),
            1 => (20, 48, 8),
            2 => (21, 64, 16),
            3 => (22, 96, 24),
            4 => (22, 128, 32),
            5 => (23, 192, 48),
            6 => (23, 273, 64),
            7 => (24, 273, 96),
            8 => (25, 273, 160),
            _ => (26, 273, 256),
        };
        Self { props: LzmaProps::new(1 << dict_log), nice_len, depth }
    }

    /// The dictionary size for `len` bytes of input: the setting, but no
    /// more than the input needs (at least 4 KiB).
    pub(crate) fn fitted_dict(&self, len: usize) -> u32 {
        let need = (len as u64).max(4096).next_power_of_two().min(u64::from(u32::MAX)) as u32;
        self.props.dict_size.min(need)
    }
}

/// A hash-chain match finder over the whole input.
struct MatchFinder<'a> {
    data: &'a [u8],
    /// The next position to insert.
    pos: usize,
    dict_size: usize,
    depth: usize,
    nice_len: usize,
    hash2: Vec<u32>,
    hash3: Vec<u32>,
    head: Vec<u32>,
    chain: Vec<u32>,
    hash_mask: usize,
}

const NONE: u32 = u32::MAX;
const HASH2_BITS: u32 = 10;
const HASH3_BITS: u32 = 16;

impl<'a> MatchFinder<'a> {
    fn new(data: &'a [u8], dict_size: usize, depth: usize, nice_len: usize) -> Self {
        let window = dict_size.min(data.len()).max(1);
        let hash_bits = (usize::BITS - window.leading_zeros()).clamp(16, 24);
        Self {
            data,
            pos: 0,
            dict_size,
            depth: depth.max(1),
            nice_len: nice_len.clamp(MATCH_LEN_MIN, MATCH_LEN_MAX),
            hash2: vec![NONE; 1 << HASH2_BITS],
            hash3: vec![NONE; 1 << HASH3_BITS],
            head: vec![NONE; 1 << hash_bits],
            chain: vec![NONE; window],
            hash_mask: (1 << hash_bits) - 1,
        }
    }

    fn hashes(&self, p: usize) -> (usize, usize, usize) {
        let d = self.data;
        let h2 = (usize::from(d[p]) | usize::from(d[p + 1]) << 8) & ((1 << HASH2_BITS) - 1);
        let v3 = u32::from(d[p]) | u32::from(d[p + 1]) << 8 | u32::from(d[p + 2]) << 16;
        let h3 = (v3.wrapping_mul(0x9E37_79B1) >> (32 - HASH3_BITS)) as usize;
        let v4 = v3 | u32::from(d[p + 3]) << 24;
        let h4 = (v4.wrapping_mul(0x9E37_79B1) >> 8) as usize & self.hash_mask;
        (h2, h3, h4)
    }

    /// Inserts the next position and returns its matches, longest last,
    /// each strictly longer than the one before: `(length, distance - 1)`.
    fn find(&mut self, matches: &mut Vec<(usize, u32)>) {
        matches.clear();
        let p = self.pos;
        self.pos += 1;
        let d = self.data;
        let avail = (d.len() - p).min(MATCH_LEN_MAX);
        if d.len() - p < 4 {
            return;
        }
        let (h2, h3, h4) = self.hashes(p);
        let c2 = self.hash2[h2];
        let c3 = self.hash3[h3];
        let mut c4 = self.head[h4];
        self.hash2[h2] = p as u32;
        self.hash3[h3] = p as u32;
        self.head[h4] = p as u32;
        let slot = p % self.chain.len();
        self.chain[slot] = c4;
        let reach = |c: u32| c != NONE && p - (c as usize) <= self.dict_size;
        let len_at = |c: usize, from: usize| {
            let mut l = from;
            while l < avail && d[c + l] == d[p + l] {
                l += 1;
            }
            l
        };
        let mut best = 1;
        for c in [c2, c3] {
            if reach(c) && d[c as usize] == d[p] && d[c as usize + 1] == d[p + 1] {
                let l = len_at(c as usize, 2);
                if l > best {
                    best = l;
                    matches.push((l, (p - c as usize - 1) as u32));
                    if l >= self.nice_len {
                        return;
                    }
                }
            }
        }
        for _ in 0..self.depth {
            // Nothing can beat a match that reaches the end.
            if !reach(c4) || best >= avail {
                break;
            }
            let c = c4 as usize;
            if d[c + best] == d[p + best] {
                let l = len_at(c, 0);
                if l > best {
                    best = l;
                    matches.push((l, (p - c - 1) as u32));
                    if l >= self.nice_len {
                        break;
                    }
                }
            }
            let next = self.chain[c % self.chain.len()];
            // A link older than the window is stale.
            if next == NONE || next as usize >= c {
                break;
            }
            c4 = next;
        }
    }

    /// Inserts the next `n` positions without searching.
    fn skip(&mut self, n: usize) {
        for _ in 0..n {
            let p = self.pos;
            self.pos += 1;
            if self.data.len() - p >= 4 {
                let (h2, h3, h4) = self.hashes(p);
                self.hash2[h2] = p as u32;
                self.hash3[h3] = p as u32;
                let slot = p % self.chain.len();
                self.chain[slot] = self.head[h4];
                self.head[h4] = p as u32;
            }
        }
    }
}

/// `true` when `big` is so much further than `small` that a match one
/// byte longer at `big` is not worth it (liblzma's `change_pair`).
const fn change_pair(small: u32, big: u32) -> bool {
    (big >> 7) > small
}

/// An LZMA encoder over a whole input, symbol by symbol, so that LZMA2
/// can cut it into chunks.
pub(crate) struct Encoder<'a> {
    pub(crate) props: LzmaProps,
    pub(crate) model: Model,
    mf: MatchFinder<'a>,
    data: &'a [u8],
    /// The next position to encode.
    pub(crate) pos: usize,
    /// Matches already found for `pos` by a look-ahead.
    ahead: Option<Vec<(usize, u32)>>,
    matches: Vec<(usize, u32)>,
}

impl<'a> Encoder<'a> {
    pub(crate) fn new(data: &'a [u8], opts: &LzmaOptions) -> Self {
        let props = opts.props;
        Self {
            props,
            model: Model::new(&props),
            mf: MatchFinder::new(data, props.dict_size.max(4096) as usize, opts.depth, opts.nice_len),
            data,
            pos: 0,
            ahead: None,
            matches: Vec::with_capacity(MATCH_LEN_MAX),
        }
    }

    /// The next symbol, as liblzma's fast parser chooses it:
    /// `(u32::MAX, 1)` for a literal, `(i, len)` for repeated distance
    /// `i` < 4, `(4 + distance - 1, len)` for a new match.
    fn choose(&mut self) -> (u32, usize) {
        let p = self.pos;
        let d = self.data;
        let mut matches = match self.ahead.take() {
            Some(m) => m,
            None => {
                let mut m = core::mem::take(&mut self.matches);
                self.mf.find(&mut m);
                m
            }
        };
        let avail = (d.len() - p).min(MATCH_LEN_MAX);
        let nice = self.mf.nice_len;
        if avail < 2 || p == 0 {
            self.matches = matches;
            return (u32::MAX, 1);
        }
        let rep_len_of = |rep: u32| -> usize {
            let r = rep as usize + 1;
            if r > p {
                return 0;
            }
            let mut l = 0;
            while l < avail && d[p + l] == d[p - r + l] {
                l += 1;
            }
            l
        };
        let (mut rep_len, mut rep_index) = (0, 0);
        for i in 0..4 {
            let l = rep_len_of(self.model.reps[i]);
            if l < 2 {
                continue;
            }
            if l >= nice {
                self.mf.skip(l - 1);
                self.matches = matches;
                return (i as u32, l);
            }
            if l > rep_len {
                rep_len = l;
                rep_index = i;
            }
        }
        let mut len_main = matches.last().map_or(0, |m| m.0);
        if len_main >= nice {
            let back = matches.last().map_or(0, |m| m.1);
            self.mf.skip(len_main - 1);
            self.matches = matches;
            return (back + 4, len_main);
        }
        let mut back_main = 0;
        if len_main >= 2 {
            back_main = matches[matches.len() - 1].1;
            while matches.len() > 1 && len_main == matches[matches.len() - 2].0 + 1 {
                if !change_pair(matches[matches.len() - 2].1, back_main) {
                    break;
                }
                matches.pop();
                len_main = matches[matches.len() - 1].0;
                back_main = matches[matches.len() - 1].1;
            }
            if len_main == 2 && back_main >= 0x80 {
                len_main = 1;
            }
        }
        if rep_len >= 2
            && (rep_len + 1 >= len_main
                || (rep_len + 2 >= len_main && back_main > (1 << 9))
                || (rep_len + 3 >= len_main && back_main > (1 << 15)))
        {
            self.mf.skip(rep_len - 1);
            self.matches = matches;
            return (rep_index as u32, rep_len);
        }
        if len_main < 2 || avail <= 2 {
            self.matches = matches;
            return (u32::MAX, 1);
        }
        // Look at the next position: if it has a better match, this byte
        // goes out as a literal.
        let mut next = Vec::with_capacity(MATCH_LEN_MAX);
        self.mf.find(&mut next);
        if let Some(&(next_len, next_dist)) = next.last() {
            if next_len >= 2
                && ((next_len >= len_main && next_dist < back_main)
                    || (next_len == len_main + 1 && !change_pair(back_main, next_dist))
                    || next_len > len_main + 1
                    || (next_len + 1 >= len_main && len_main >= 3 && change_pair(next_dist, back_main)))
            {
                self.ahead = Some(next);
                self.matches = matches;
                return (u32::MAX, 1);
            }
        }
        let limit = (len_main - 1).max(2);
        for i in 0..4 {
            let r = self.model.reps[i] as usize + 1;
            if r <= p + 1 && p + 1 + limit <= d.len() && d[p + 1..p + 1 + limit] == d[p + 1 - r..p + 1 - r + limit] {
                self.ahead = Some(next);
                self.matches = matches;
                return (u32::MAX, 1);
            }
        }
        self.mf.skip(len_main - 2);
        self.matches = matches;
        (back_main + 4, len_main)
    }

    /// Encodes the next symbol, returning the bytes it covers.
    pub(crate) fn encode_next(&mut self, rc: &mut RangeEncoder) -> usize {
        let (back, len) = self.choose();
        let p = self.pos;
        let m = &mut self.model;
        let pos_state = p & m.pb_mask;
        let state = m.state;
        if back == u32::MAX {
            rc.bit(&mut m.is_match[state][pos_state], 0);
            let prev = if p > 0 { self.data[p - 1] } else { 0 };
            let base = m.literal_base(p, prev);
            let probs = &mut m.literal[base..base + 0x300];
            let byte = u32::from(self.data[p]);
            if state >= 7 {
                // While the bits agree with the byte at the last distance,
                // they use the matched half of the table (0x100 + its bit
                // × 0x100); from the first difference on, the plain tree.
                let mut match_byte = u32::from(self.data[p - m.reps[0] as usize - 1]);
                let mut offs = 0x100u32;
                let mut s = byte | 0x100;
                while s < 0x10000 {
                    match_byte <<= 1;
                    let idx = offs + (match_byte & offs) + (s >> 8);
                    rc.bit(&mut probs[idx as usize], (s >> 7) & 1);
                    s <<= 1;
                    offs &= !(match_byte ^ s);
                }
            } else {
                let mut sym = 1usize;
                for i in (0..8).rev() {
                    let b = (byte >> i) & 1;
                    rc.bit(&mut probs[sym], b);
                    sym = (sym << 1) | b as usize;
                }
            }
            m.state = after_literal(state);
        } else if back < 4 {
            rc.bit(&mut m.is_match[state][pos_state], 1);
            rc.bit(&mut m.is_rep[state], 1);
            let i = back as usize;
            if i == 0 {
                rc.bit(&mut m.is_rep0[state], 0);
                rc.bit(&mut m.is_rep0_long[state][pos_state], u32::from(len != 1));
            } else {
                let dist = m.reps[i];
                rc.bit(&mut m.is_rep0[state], 1);
                if i == 1 {
                    rc.bit(&mut m.is_rep1[state], 0);
                } else {
                    rc.bit(&mut m.is_rep1[state], 1);
                    rc.bit(&mut m.is_rep2[state], u32::from(i == 3));
                    if i == 3 {
                        m.reps[3] = m.reps[2];
                    }
                    m.reps[2] = m.reps[1];
                }
                m.reps[1] = m.reps[0];
                m.reps[0] = dist;
            }
            if len == 1 {
                m.state = after_short_rep(state);
            } else {
                rc.len(&mut m.rep_len, len, pos_state);
                m.state = after_rep(state);
            }
        } else {
            let dist = back - 4;
            rc.bit(&mut m.is_match[state][pos_state], 1);
            rc.bit(&mut m.is_rep[state], 0);
            rc.len(&mut m.len, len, pos_state);
            m.state = after_match(state);
            encode_distance(rc, m, dist, len);
            m.reps = [dist, m.reps[0], m.reps[1], m.reps[2]];
        }
        self.pos += len;
        len
    }

    /// Writes the end-of-stream marker.
    pub(crate) fn encode_marker(&mut self, rc: &mut RangeEncoder) {
        let m = &mut self.model;
        let pos_state = self.pos & m.pb_mask;
        let state = m.state;
        rc.bit(&mut m.is_match[state][pos_state], 1);
        rc.bit(&mut m.is_rep[state], 0);
        rc.len(&mut m.len, MATCH_LEN_MIN, pos_state);
        m.state = after_match(state);
        encode_distance(rc, m, u32::MAX, MATCH_LEN_MIN);
    }
}

fn encode_distance(rc: &mut RangeEncoder, m: &mut Model, dist: u32, len: usize) {
    let slot = dist_slot(dist);
    rc.tree(&mut m.dist_slot[dist_state(len)], 6, slot);
    if slot >= 4 {
        let footer = (slot >> 1) - 1;
        let base = (2 | (slot & 1)) << footer;
        let rest = dist - base;
        if slot < END_POS_MODEL_INDEX {
            rc.reverse_tree(&mut m.dist_special, (base - slot) as usize, footer, rest);
        } else {
            rc.direct(rest >> ALIGN_BITS, footer - ALIGN_BITS);
            rc.reverse_tree(&mut m.dist_align, 0, ALIGN_BITS, rest & 15);
        }
    }
}

/// The distance slot of `dist` (distance − 1): its two top bits' place.
const fn dist_slot(dist: u32) -> u32 {
    if dist < 4 {
        dist
    } else {
        let n = 31 - dist.leading_zeros();
        (n << 1) | ((dist >> (n - 1)) & 1)
    }
}

/// Compresses `data` into a raw LZMA stream, ending it with the end
/// marker if `marker` is set (needed when the reader does not know the
/// size).
pub fn compress_raw(data: &[u8], opts: &LzmaOptions, marker: bool) -> Vec<u8> {
    let mut enc = Encoder::new(data, opts);
    let mut rc = RangeEncoder::new();
    while enc.pos < data.len() {
        enc.encode_next(&mut rc);
    }
    if marker {
        enc.encode_marker(&mut rc);
    }
    rc.finish()
}

/// Decodes a ZIP method-14 entry holding `size` bytes: the LZMA SDK
/// version (2 bytes), the properties' size (2 bytes, 5), the properties,
/// then the stream, which may end with a marker (general purpose flag
/// bit 1 says it does) and must fill the entry.
pub fn decompress_zip_entry(raw: &[u8], size: usize) -> Result<Vec<u8>, LzmaError> {
    if raw.len() < 9 {
        return Err(LzmaError::Truncated);
    }
    if u16::from_le_bytes([raw[2], raw[3]]) != 5 {
        return Err(LzmaError::BadProperties);
    }
    let props = LzmaProps::parse(&[raw[4], raw[5], raw[6], raw[7], raw[8]])?;
    let d = decompress_raw(&raw[9..], &props, Some(size as u64), size)?;
    if 9 + d.consumed != raw.len() {
        return Err(LzmaError::Corrupt("data after the LZMA stream"));
    }
    Ok(d.data)
}

/// Encodes `data` as a ZIP method-14 entry (no end marker: the entry
/// records the size).
pub fn compress_zip_entry(data: &[u8], level: u8) -> Vec<u8> {
    let mut opts = LzmaOptions::level(level);
    opts.props.dict_size = opts.fitted_dict(data.len());
    let mut out = vec![9, 20, 5, 0];
    out.extend_from_slice(&opts.props.to_bytes());
    out.extend(compress_raw(data, &opts, false));
    out
}

/// Compresses `data` into a `.lzma` file at `level` 0–9 (6 is xz's
/// default), recording its size, with the dictionary no larger than the
/// data needs.
pub fn compress(data: &[u8], level: u8) -> Vec<u8> {
    let mut opts = LzmaOptions::level(level);
    opts.props.dict_size = opts.fitted_dict(data.len());
    compress_file(data, &opts)
}

/// Compresses `data` into a `.lzma` file with these settings.
pub fn compress_file(data: &[u8], opts: &LzmaOptions) -> Vec<u8> {
    let mut out = opts.props.to_bytes().to_vec();
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend(compress_raw(data, opts, false));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample(len: usize, seed: u32) -> Vec<u8> {
        let mut x = seed;
        let mut v = Vec::with_capacity(len);
        while v.len() < len {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            match (x >> 16) % 5 {
                0 => v.extend_from_slice(b"the quick brown fox "),
                1 => v.resize(v.len() + ((x >> 8) % 300) as usize, (x >> 24) as u8),
                2 => v.push((x >> 24) as u8),
                3 if v.len() > 1000 => {
                    let from = ((x >> 4) as usize) % (v.len() - 500);
                    v.extend_from_within(from..from + ((x >> 20) % 400) as usize);
                }
                _ => v.extend_from_slice(&x.to_le_bytes()),
            }
        }
        v.truncate(len);
        v
    }

    #[test]
    fn dist_slots() {
        for (d, s) in [(0, 0), (3, 3), (4, 4), (5, 4), (6, 5), (7, 5), (8, 6), (12, 7), (u32::MAX, 63)] {
            assert_eq!(dist_slot(d), s, "{d}");
        }
    }

    #[test]
    fn props_bytes() {
        let p = LzmaProps::new(1 << 23);
        assert_eq!(p.byte(), 0x5D);
        assert_eq!(LzmaProps::parse(&p.to_bytes()).unwrap(), p);
        assert_eq!(LzmaProps::from_byte(225, 0), Err(LzmaError::BadProperties));
    }

    #[test]
    fn round_trips() {
        for (len, seed) in [(0, 1), (1, 1), (2, 2), (5, 3), (100, 4), (5000, 5), (200_000, 6)] {
            let data = sample(len, seed);
            for level in [0, 3, 6, 9] {
                let f = compress(&data, level);
                assert_eq!(decompress(&f).unwrap(), data, "len {len} level {level}");
                let opts = LzmaOptions::level(level);
                for marker in [false, true] {
                    let raw = compress_raw(&data, &opts, marker);
                    let size = (!marker).then_some(data.len() as u64);
                    let d = decompress_raw(&raw, &opts.props, size, usize::MAX).unwrap();
                    assert_eq!(
                        (d.data, d.consumed),
                        (data.clone(), raw.len()),
                        "len {len} level {level} marker {marker}"
                    );
                }
            }
        }
        // Other lc/lp/pb.
        let data = sample(30_000, 9);
        for (lc, lp, pb) in [(0, 0, 0), (8, 0, 0), (0, 4, 4), (4, 4, 4), (1, 2, 3)] {
            let opts = LzmaOptions { props: LzmaProps { lc, lp, pb, dict_size: 1 << 16 }, nice_len: 64, depth: 16 };
            let raw = compress_raw(&data, &opts, true);
            assert_eq!(decompress_raw(&raw, &opts.props, None, usize::MAX).unwrap().data, data, "{lc} {lp} {pb}");
        }
    }

    #[test]
    fn compresses() {
        let data = sample(200_000, 7);
        let f = compress(&data, 6);
        assert!(f.len() < data.len() / 4, "{} -> {}", data.len(), f.len());
    }

    #[test]
    fn errors() {
        let data = sample(20_000, 8);
        let f = compress(&data, 6);
        for cut in [0, 5, 13, 14, 20, f.len() / 2, f.len() - 1] {
            assert!(decompress(&f[..cut]).is_err(), "cut at {cut}");
        }
        assert_eq!(decompress(&[&f[..], b"x"].concat()), Err(LzmaError::TrailingData));
        let mut bad = f.clone();
        bad[13] = 1;
        assert!(decompress(&bad).is_err());
        assert_eq!(decompress_with_limit(&f, data.len() - 1), Err(LzmaError::OutputLimitExceeded));
        // A known size that is too small or too large.
        let mut short = f.clone();
        short[5..13].copy_from_slice(&(data.len() as u64 - 1).to_le_bytes());
        assert!(decompress(&short).is_err());
        let mut long = f.clone();
        long[5..13].copy_from_slice(&(data.len() as u64 + 1).to_le_bytes());
        assert!(decompress(&long).is_err());
        // An end marker after a known size is accepted in .lzma files.
        let opts = LzmaOptions::level(6);
        let mut with_marker = opts.props.to_bytes().to_vec();
        with_marker.extend_from_slice(&(data.len() as u64).to_le_bytes());
        with_marker.extend(compress_raw(&data, &opts, true));
        assert_eq!(decompress(&with_marker).unwrap(), data);
        // Unknown size needs the marker.
        let mut unknown = opts.props.to_bytes().to_vec();
        unknown.extend_from_slice(&u64::MAX.to_le_bytes());
        unknown.extend(compress_raw(&data, &opts, false));
        assert!(decompress(&unknown).is_err());
    }
}
