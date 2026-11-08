//! `.xz` files: LZMA2 data in blocks, with integrity checks and an index.
//!
//! A stream is a 12-byte header (magic, check type, CRC32), blocks, an
//! index listing every block's sizes, and a 12-byte footer. Each block has
//! a header naming its filter chain (LZMA2, after up to three branch or
//! delta filters) and optionally its sizes, then the compressed data,
//! padding to four bytes and the check (none, CRC32, CRC64 or SHA-256) of
//! its uncompressed data. Streams may be concatenated, with zero padding
//! in multiples of four bytes between them.
//!
//! The decoder checks everything liblzma checks: every CRC32, the block
//! checks, sizes against the block headers and the index, the footer
//! against the header and index, zero padding, and canonical integers.
//! Check types it does not implement (the reserved IDs) are skipped, as
//! liblzma skips them. Decompression takes an output limit.
//!
//! ```
//! use verified_deflate::xz;
//!
//! let data = b"to be or not to be, that is the question".repeat(100);
//! let packed = xz::compress(&data, 6);
//! assert_eq!(xz::decompress_with_limit(&packed, 1 << 20).unwrap(), data);
//! ```

use crate::crc32::crc32;
use crate::filters::{delta_decode, delta_encode, BranchFilter};
use crate::lzma::{LzmaError, LzmaOptions};
use crate::lzma2;
use core::fmt;

const HEADER_MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0x00];
const FOOTER_MAGIC: [u8; 2] = *b"YZ";
const FILTER_LZMA2: u64 = 0x21;
const FILTER_DELTA: u64 = 0x03;
/// IDs from here up are reserved; liblzma rejects them as corrupt.
const RESERVED_ID_START: u64 = 1 << 62;
const VLI_MAX: u64 = u64::MAX / 2;
const UNPADDED_SIZE_MAX: u64 = VLI_MAX & !3;

/// Why an `.xz` file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum XzError {
    /// The data does not start with the `.xz` magic bytes.
    NotXz,
    /// The data ended in the middle of a stream.
    Truncated,
    /// The stream is corrupt; the message says how.
    Corrupt(&'static str),
    /// The stream uses a feature this decoder (like liblzma) does not
    /// know: reserved flags or an unknown filter.
    Unsupported(&'static str),
    /// A block's data does not match its check.
    CheckMismatch,
    /// The LZMA2 data is corrupt.
    Lzma(LzmaError),
    /// Decompressing would exceed the output limit.
    OutputLimitExceeded,
    /// After the last stream come bytes that are neither padding nor
    /// another stream.
    TrailingData,
}

impl fmt::Display for XzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotXz => f.write_str("not in .xz format"),
            Self::Truncated => f.write_str("unexpected end of .xz data"),
            Self::Corrupt(why) => write!(f, "corrupt .xz data: {why}"),
            Self::Unsupported(what) => write!(f, "unsupported .xz feature: {what}"),
            Self::CheckMismatch => f.write_str("block data does not match its check"),
            Self::Lzma(e) => write!(f, "{e}"),
            Self::OutputLimitExceeded => f.write_str("decompressed data exceeds the output limit"),
            Self::TrailingData => f.write_str("trailing data after the last .xz stream"),
        }
    }
}

impl std::error::Error for XzError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lzma(e) => Some(e),
            _ => None,
        }
    }
}

impl From<LzmaError> for XzError {
    fn from(e: LzmaError) -> Self {
        match e {
            LzmaError::OutputLimitExceeded => Self::OutputLimitExceeded,
            LzmaError::Truncated => Self::Truncated,
            e => Self::Lzma(e),
        }
    }
}

/// A block's integrity check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Check {
    /// No check.
    None,
    /// CRC32 (as in zip and gzip).
    Crc32,
    /// CRC64 (ECMA-182), xz's default.
    Crc64,
    /// SHA-256.
    Sha256,
    /// A reserved check type (2, 3, 5–9, 11–15): its field is skipped.
    Reserved(u8),
}

impl Check {
    /// The check with this ID (0–15).
    pub const fn from_id(id: u8) -> Self {
        match id {
            0 => Self::None,
            1 => Self::Crc32,
            4 => Self::Crc64,
            10 => Self::Sha256,
            n => Self::Reserved(n & 15),
        }
    }

    /// The check's ID in the stream flags.
    pub const fn id(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Crc32 => 1,
            Self::Crc64 => 4,
            Self::Sha256 => 10,
            Self::Reserved(n) => n,
        }
    }

    /// Bytes the check takes in each block.
    pub const fn size(self) -> usize {
        match self.id() {
            0 => 0,
            1..=3 => 4,
            4..=6 => 8,
            7..=9 => 16,
            10..=12 => 32,
            _ => 64,
        }
    }

    /// The check of `data`, or `None` for a reserved type.
    fn compute(self, data: &[u8]) -> Option<Vec<u8>> {
        match self {
            Self::None => Some(Vec::new()),
            Self::Crc32 => Some(crc32(data).to_le_bytes().to_vec()),
            Self::Crc64 => Some(crc64(data).to_le_bytes().to_vec()),
            Self::Sha256 => Some(sha256(data).to_vec()),
            Self::Reserved(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------
// CRC64 and SHA-256

const CRC64_TABLE: [u64; 256] = {
    let mut t = [0u64; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u64;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xC96C_5795_D787_0F42 } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

/// The CRC64 of `data` (ECMA-182, reflected, as xz uses it).
pub fn crc64(data: &[u8]) -> u64 {
    let mut c = !0u64;
    for &b in data {
        c = CRC64_TABLE[((c as u8) ^ b) as usize] ^ (c >> 8);
    }
    !c
}

/// SHA-256 (FIPS 180-4).
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut tail = data[data.len() / 64 * 64..].to_vec();
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in data[..data.len() / 64 * 64].chunks(64).chain(tail.chunks(64)) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            v = [t1.wrapping_add(s0.wrapping_add(maj)), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for (a, b) in h.iter_mut().zip(v) {
            *a = a.wrapping_add(b);
        }
    }
    let mut out = [0u8; 32];
    for (i, x) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// Integers

/// Reads a variable-length integer from `b` at `*pos`, within `end`: at
/// most nine bytes, seven bits each, least significant first, with no
/// needless trailing zero byte.
fn read_vli(b: &[u8], pos: &mut usize, end: usize) -> Result<u64, XzError> {
    let mut v = 0u64;
    for i in 0..9 {
        if *pos >= end {
            return Err(XzError::Corrupt("integer runs past its field"));
        }
        let byte = b[*pos];
        *pos += 1;
        v |= u64::from(byte & 0x7F) << (7 * i);
        if byte & 0x80 == 0 {
            if byte == 0 && i > 0 {
                return Err(XzError::Corrupt("integer is not in its shortest form"));
            }
            return Ok(v);
        }
    }
    Err(XzError::Corrupt("integer longer than nine bytes"))
}

fn write_vli(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

// ---------------------------------------------------------------------------
// Decoding

/// A filter in a block's chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Filter {
    /// LZMA2 with this dictionary size (always last).
    Lzma2 {
        /// The dictionary size.
        dict_size: u32,
    },
    /// The delta filter with this distance (1–256).
    Delta(u16),
    /// A branch filter, starting at this offset.
    Branch(BranchFilter, u32),
}

/// A decoded `.xz` stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XzStream {
    /// The decompressed data of every block.
    pub data: Vec<u8>,
    /// Bytes of input the stream used, through its footer.
    pub consumed: usize,
    /// The stream's check type.
    pub check: Check,
}

/// Whether `data` starts with the `.xz` magic bytes.
pub fn is_xz(data: &[u8]) -> bool {
    data.starts_with(&HEADER_MAGIC)
}

/// Parses the stream flags (two bytes after the magic, or in the footer).
fn stream_flags(b: &[u8]) -> Result<Check, XzError> {
    if b[0] != 0 || b[1] & 0xF0 != 0 {
        return Err(XzError::Unsupported("reserved stream flags"));
    }
    Ok(Check::from_id(b[1]))
}

/// Parses a block header at `input[at..]`: its size, the sizes it
/// declares and its filters, in order.
#[allow(clippy::type_complexity)]
fn block_header(
    input: &[u8],
    at: usize,
    check: Check,
) -> Result<(usize, Option<u64>, Option<u64>, Vec<Filter>), XzError> {
    let size = (usize::from(input[at]) + 1) * 4;
    let h = input.get(at..at + size).ok_or(XzError::Truncated)?;
    let body = size - 4;
    if crc32(&h[..body]) != le32(h, body) {
        return Err(XzError::Corrupt("block header CRC mismatch"));
    }
    let flags = h[1];
    if flags & 0x3C != 0 {
        return Err(XzError::Unsupported("reserved block flags"));
    }
    let mut pos = 2;
    let compressed = if flags & 0x40 != 0 {
        let c = read_vli(h, &mut pos, body)?;
        let unpadded = c.checked_add(size as u64 + check.size() as u64);
        if c == 0 || !unpadded.is_some_and(|u| u <= UNPADDED_SIZE_MAX) {
            return Err(XzError::Corrupt("invalid block compressed size"));
        }
        Some(c)
    } else {
        None
    };
    let uncompressed = if flags & 0x80 != 0 { Some(read_vli(h, &mut pos, body)?) } else { None };
    let mut filters = Vec::new();
    for _ in 0..(flags & 3) + 1 {
        let id = read_vli(h, &mut pos, body)?;
        if id >= RESERVED_ID_START {
            return Err(XzError::Corrupt("reserved filter ID"));
        }
        let props_size = read_vli(h, &mut pos, body)?;
        if props_size > (body - pos) as u64 {
            return Err(XzError::Corrupt("filter properties run past the header"));
        }
        let props = &h[pos..pos + props_size as usize];
        pos += props_size as usize;
        let f = match id {
            FILTER_LZMA2 => {
                if props.len() != 1 || props[0] & 0xC0 != 0 {
                    return Err(XzError::Unsupported("LZMA2 properties"));
                }
                let dict_size =
                    lzma2::dict_size_from_byte(props[0]).ok_or(XzError::Unsupported("LZMA2 dictionary size"))?;
                Filter::Lzma2 { dict_size }
            }
            FILTER_DELTA => {
                if props.len() != 1 {
                    return Err(XzError::Unsupported("delta properties"));
                }
                Filter::Delta(u16::from(props[0]) + 1)
            }
            _ => {
                let b = BranchFilter::from_xz_id(id).ok_or(XzError::Unsupported("unknown filter"))?;
                let start = match props.len() {
                    0 => 0,
                    4 => le32(props, 0),
                    _ => return Err(XzError::Unsupported("branch filter properties")),
                };
                Filter::Branch(b, start)
            }
        };
        filters.push(f);
    }
    if h[pos..body].iter().any(|&b| b != 0) {
        return Err(XzError::Unsupported("block header padding is not zero"));
    }
    // Only LZMA2 may be last, and only last.
    for (i, f) in filters.iter().enumerate() {
        let last = i + 1 == filters.len();
        match f {
            Filter::Lzma2 { .. } if !last => return Err(XzError::Unsupported("LZMA2 before another filter")),
            Filter::Lzma2 { .. } => {}
            _ if last => return Err(XzError::Unsupported("filter chain does not end with LZMA2")),
            Filter::Branch(b, start) if start % b.alignment() != 0 => {
                return Err(XzError::Unsupported("misaligned branch filter start"))
            }
            _ => {}
        }
    }
    Ok((size, compressed, uncompressed, filters))
}

/// Decodes the `.xz` stream at the start of `input`, producing at most
/// `max_output` bytes, and checks it all.
pub fn read_stream(input: &[u8], max_output: usize) -> Result<XzStream, XzError> {
    let mut out = Vec::new();
    let (consumed, check) = read_stream_into(input, &mut out, max_output)?;
    Ok(XzStream { data: out, consumed, check })
}

fn read_stream_into(input: &[u8], out: &mut Vec<u8>, max_output: usize) -> Result<(usize, Check), XzError> {
    if input.len() < 12 {
        return Err(if HEADER_MAGIC.starts_with(input) || input.starts_with(&HEADER_MAGIC) {
            XzError::Truncated
        } else {
            XzError::NotXz
        });
    }
    if input[..6] != HEADER_MAGIC {
        return Err(XzError::NotXz);
    }
    if crc32(&input[6..8]) != le32(input, 8) {
        return Err(XzError::Corrupt("stream header CRC mismatch"));
    }
    let check = stream_flags(&input[6..8])?;
    let mut pos = 12;
    // (unpadded size, uncompressed size) of each block.
    let mut records: Vec<(u64, u64)> = Vec::new();
    loop {
        let b = *input.get(pos).ok_or(XzError::Truncated)?;
        if b == 0 {
            break;
        }
        let (header_size, declared_c, declared_u, filters) = block_header(input, pos, check)?;
        let data_start = pos + header_size;
        let start = out.len();
        let dict_size = match filters.last() {
            Some(Filter::Lzma2 { dict_size }) => *dict_size,
            _ => return Err(XzError::Unsupported("filter chain does not end with LZMA2")),
        };
        let packed = input.get(data_start..).ok_or(XzError::Truncated)?;
        let used = lzma2::decode_into(packed, dict_size, out, max_output)?;
        if declared_c.is_some_and(|c| c != used as u64) {
            return Err(XzError::Corrupt("block compressed size does not match its header"));
        }
        let produced = (out.len() - start) as u64;
        if declared_u.is_some_and(|u| u != produced) {
            return Err(XzError::Corrupt("block uncompressed size does not match its header"));
        }
        // The other filters, last applied first.
        for f in filters[..filters.len() - 1].iter().rev() {
            match *f {
                Filter::Delta(d) => delta_decode(&mut out[start..], usize::from(d)),
                Filter::Branch(b, s) => b.decode(&mut out[start..], s),
                Filter::Lzma2 { .. } => {}
            }
        }
        let mut p = data_start + used;
        let pad = (4 - used % 4) % 4;
        let padding = input.get(p..p + pad).ok_or(XzError::Truncated)?;
        if padding.iter().any(|&b| b != 0) {
            return Err(XzError::Corrupt("block padding is not zero"));
        }
        p += pad;
        let stored = input.get(p..p + check.size()).ok_or(XzError::Truncated)?;
        if let Some(c) = check.compute(&out[start..]) {
            if c != stored {
                return Err(XzError::CheckMismatch);
            }
        }
        p += check.size();
        records.push(((header_size + used + check.size()) as u64, produced));
        pos = p;
    }

    // The index: it must list exactly the blocks read.
    let index_start = pos;
    pos += 1;
    let end = input.len();
    let count = read_vli(input, &mut pos, end).map_err(truncated_if_at_end(input))?;
    if count != records.len() as u64 {
        return Err(XzError::Corrupt("index lists a different number of blocks"));
    }
    for &(unpadded, uncompressed) in &records {
        let u = read_vli(input, &mut pos, end).map_err(truncated_if_at_end(input))?;
        let n = read_vli(input, &mut pos, end).map_err(truncated_if_at_end(input))?;
        if !(5..=UNPADDED_SIZE_MAX).contains(&u) || u != unpadded || n != uncompressed {
            return Err(XzError::Corrupt("index does not match the blocks"));
        }
    }
    while (pos - index_start) % 4 != 0 {
        if *input.get(pos).ok_or(XzError::Truncated)? != 0 {
            return Err(XzError::Corrupt("index padding is not zero"));
        }
        pos += 1;
    }
    let crc_at = pos;
    if input.len() < crc_at + 4 {
        return Err(XzError::Truncated);
    }
    if crc32(&input[index_start..crc_at]) != le32(input, crc_at) {
        return Err(XzError::Corrupt("index CRC mismatch"));
    }
    let index_size = crc_at + 4 - index_start;

    // The footer.
    let f = input.get(crc_at + 4..crc_at + 16).ok_or(XzError::Truncated)?;
    if f[10..12] != FOOTER_MAGIC {
        return Err(XzError::Corrupt("bad stream footer magic"));
    }
    if crc32(&f[4..10]) != le32(f, 0) {
        return Err(XzError::Corrupt("stream footer CRC mismatch"));
    }
    let footer_check = stream_flags(&f[8..10])?;
    if (u64::from(le32(f, 4)) + 1) * 4 != index_size as u64 {
        return Err(XzError::Corrupt("footer's index size is wrong"));
    }
    if footer_check != check {
        return Err(XzError::Corrupt("footer flags differ from the header's"));
    }
    Ok((crc_at + 16, check))
}

/// Maps an integer running past the end of the data to `Truncated`.
fn truncated_if_at_end(input: &[u8]) -> impl Fn(XzError) -> XzError + '_ {
    let _ = input;
    |e| match e {
        XzError::Corrupt("integer runs past its field") => XzError::Truncated,
        e => e,
    }
}

/// Decompresses an `.xz` file, every stream, with no output limit. For
/// untrusted input prefer [`decompress_with_limit`].
pub fn decompress(input: &[u8]) -> Result<Vec<u8>, XzError> {
    decompress_with_limit(input, usize::MAX)
}

/// Decompresses an `.xz` file: every stream, concatenated, with the zero
/// padding allowed between and after them (in multiples of four bytes),
/// producing at most `max_output` bytes.
pub fn decompress_with_limit(input: &[u8], max_output: usize) -> Result<Vec<u8>, XzError> {
    let mut out = Vec::new();
    let mut pos = 0;
    loop {
        let (used, _) = read_stream_into(&input[pos..], &mut out, max_output)?;
        pos += used;
        let zeros = input[pos..].iter().take_while(|&&b| b == 0).count();
        if zeros % 4 != 0 {
            return Err(XzError::Corrupt("stream padding is not a multiple of four bytes"));
        }
        pos += zeros;
        if pos == input.len() {
            return Ok(out);
        }
        if !input[pos..].starts_with(&HEADER_MAGIC) {
            return Err(XzError::TrailingData);
        }
    }
}

/// The total uncompressed size the indexes of the stream(s) record,
/// read from the end of `input` without decompressing; `None` if they
/// cannot be read.
pub fn uncompressed_size(input: &[u8]) -> Option<u64> {
    let mut end = input.len();
    let mut total = 0u64;
    loop {
        while end >= 4 && input[end - 4..end] == [0; 4] {
            end -= 4;
        }
        if end == 0 {
            return Some(total);
        }
        if end < 24 || input[end - 2..end] != FOOTER_MAGIC {
            return None;
        }
        let f = &input[end - 12..end];
        let index_size = (u64::from(le32(f, 4)) + 1) * 4;
        let index_start = (end - 12).checked_sub(usize::try_from(index_size).ok()?)?;
        let index = &input[index_start..end - 12];
        if index.first() != Some(&0) || crc32(&index[..index.len() - 4]) != le32(index, index.len() - 4) {
            return None;
        }
        let mut pos = 1;
        let lim = index.len() - 4;
        let count = read_vli(index, &mut pos, lim).ok()?;
        let mut blocks = 0u64;
        for _ in 0..count {
            blocks = blocks.checked_add(read_vli(index, &mut pos, lim).ok()?.next_multiple_of(4))?;
            total = total.checked_add(read_vli(index, &mut pos, lim).ok()?)?;
        }
        end = index_start.checked_sub(usize::try_from(blocks).ok()?)?.checked_sub(12)?;
        if !input[end..].starts_with(&HEADER_MAGIC) {
            return None;
        }
    }
}

// ---------------------------------------------------------------------------
// Encoding

/// How [`compress_with`] writes a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XzOptions {
    /// LZMA2 settings.
    pub lzma: LzmaOptions,
    /// The block check.
    pub check: Check,
    /// Filters applied before LZMA2 (at most three; delta or branch).
    pub filters: Vec<Filter>,
}

impl XzOptions {
    /// xz's settings for a level 0–9: CRC64, no other filters.
    pub fn level(level: u8) -> Self {
        Self { lzma: LzmaOptions::level(level), check: Check::Crc64, filters: Vec::new() }
    }
}

/// Compresses `data` into an `.xz` file at `level` 0–9 (6 is xz's
/// default), with a CRC64 check.
pub fn compress(data: &[u8], level: u8) -> Vec<u8> {
    compress_with(data, &XzOptions::level(level))
}

/// Compresses `data` into a one-block `.xz` stream.
///
/// # Panics
///
/// If `opts.filters` has more than three filters, includes LZMA2, or
/// a RISC-V filter (whose encoder is not implemented), or `opts.check` is
/// reserved.
pub fn compress_with(data: &[u8], opts: &XzOptions) -> Vec<u8> {
    assert!(opts.filters.len() <= 3, "at most three filters before LZMA2");
    assert!(!matches!(opts.check, Check::Reserved(_)), "reserved check type");
    let flags = [0, opts.check.id()];
    let mut out = HEADER_MAGIC.to_vec();
    out.extend_from_slice(&flags);
    out.extend_from_slice(&crc32(&flags).to_le_bytes());
    let mut records = Vec::new();
    if !data.is_empty() {
        let mut filtered = data.to_vec();
        for f in &opts.filters {
            match *f {
                Filter::Delta(d) => {
                    assert!((1..=256).contains(&d), "delta distance 1-256");
                    delta_encode(&mut filtered, usize::from(d));
                }
                Filter::Branch(b, s) => assert!(b.encode(&mut filtered, s), "no encoder for {b:?}"),
                Filter::Lzma2 { .. } => panic!("LZMA2 is added last by itself"),
            }
        }
        let mut lzma = opts.lzma;
        lzma.props.dict_size = lzma.fitted_dict(data.len());
        let dict_byte = lzma2::dict_size_byte(lzma.props.dict_size);
        lzma.props.dict_size = lzma2::dict_size_from_byte(dict_byte).unwrap_or(u32::MAX);
        let packed = lzma2::compress(&filtered, &lzma);
        // Block header: flags, both sizes, the filters, padding, CRC32.
        let mut h = vec![0, (opts.filters.len() as u8) | 0x40 | 0x80];
        write_vli(&mut h, packed.len() as u64);
        write_vli(&mut h, data.len() as u64);
        for f in &opts.filters {
            match *f {
                Filter::Delta(d) => h.extend_from_slice(&[FILTER_DELTA as u8, 1, (d - 1) as u8]),
                Filter::Branch(b, s) => {
                    write_vli(&mut h, b.xz_id());
                    if s == 0 {
                        h.push(0);
                    } else {
                        h.push(4);
                        h.extend_from_slice(&s.to_le_bytes());
                    }
                }
                Filter::Lzma2 { .. } => {}
            }
        }
        h.extend_from_slice(&[FILTER_LZMA2 as u8, 1, dict_byte]);
        while (h.len() + 4) % 4 != 0 {
            h.push(0);
        }
        h[0] = ((h.len() + 4) / 4 - 1) as u8;
        let c = crc32(&h);
        h.extend_from_slice(&c.to_le_bytes());
        let header_size = h.len();
        out.extend_from_slice(&h);
        out.extend_from_slice(&packed);
        out.resize(out.len() + (4 - packed.len() % 4) % 4, 0);
        out.extend(opts.check.compute(data).unwrap_or_default());
        records.push(((header_size + packed.len() + opts.check.size()) as u64, data.len() as u64));
    }
    let index_start = out.len();
    out.push(0);
    write_vli(&mut out, records.len() as u64);
    for &(u, n) in &records {
        write_vli(&mut out, u);
        write_vli(&mut out, n);
    }
    while (out.len() - index_start) % 4 != 0 {
        out.push(0);
    }
    let c = crc32(&out[index_start..]);
    out.extend_from_slice(&c.to_le_bytes());
    let backward = ((out.len() - index_start) / 4 - 1) as u32;
    let mut footer = backward.to_le_bytes().to_vec();
    footer.extend_from_slice(&flags);
    let c = crc32(&footer);
    out.extend_from_slice(&c.to_le_bytes());
    out.extend_from_slice(&footer);
    out.extend_from_slice(&FOOTER_MAGIC);
    out
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Every CRC64 table entry is eight steps of the reflected definition,
    /// and one table step equals eight bitwise steps from any register.
    #[kani::proof]
    #[kani::unwind(9)]
    fn crc64_matches_definition() {
        let b: u8 = kani::any();
        let mut c = u64::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xC96C_5795_D787_0F42 } else { c >> 1 };
        }
        assert_eq!(CRC64_TABLE[b as usize], c);
        let r: u64 = kani::any();
        let mut bitwise = r ^ u64::from(b);
        for _ in 0..8 {
            bitwise = if bitwise & 1 != 0 { (bitwise >> 1) ^ 0xC96C_5795_D787_0F42 } else { bitwise >> 1 };
        }
        assert_eq!(CRC64_TABLE[((r as u8) ^ b) as usize] ^ (r >> 8), bitwise);
    }

    /// Every integer up to 2^63 − 1 is written in its shortest form and
    /// read back, using exactly the bytes written.
    #[kani::proof]
    #[kani::unwind(11)]
    fn integers_round_trip() {
        let v: u64 = kani::any_where(|&v: &u64| v <= VLI_MAX);
        let mut b = Vec::new();
        write_vli(&mut b, v);
        assert!(!b.is_empty() && b.len() <= 9);
        assert!(b.len() == 1 || *b.last().unwrap() != 0);
        let mut pos = 0;
        assert_eq!(read_vli(&b, &mut pos, b.len()), Ok(v));
        assert_eq!(pos, b.len());
    }

    /// Reading integers never panics, stays within its field, and refuses
    /// a needless trailing zero byte.
    #[kani::proof]
    #[kani::unwind(11)]
    fn integers_are_read_safely() {
        let b: [u8; 10] = kani::any();
        let end: usize = kani::any_where(|&e: &usize| e <= 10);
        let mut pos = 0;
        if let Ok(v) = read_vli(&b, &mut pos, end) {
            assert!(pos <= end && pos >= 1 && v <= VLI_MAX);
            assert!(pos == 1 || b[pos - 1] != 0);
        }
    }

    /// Check types: their IDs round-trip and their sizes are as specified.
    #[kani::proof]
    fn checks_are_consistent() {
        let id: u8 = kani::any_where(|&i: &u8| i < 16);
        let c = Check::from_id(id);
        assert_eq!(c.id(), id);
        assert!([0, 4, 8, 16, 32, 64].contains(&c.size()));
        assert_eq!(Check::Crc32.size(), 4);
        assert_eq!(Check::Crc64.size(), 8);
        assert_eq!(Check::Sha256.size(), 32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(len: usize) -> Vec<u8> {
        let mut x = 99u32;
        let mut v = Vec::new();
        while v.len() < len {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            if x >> 30 == 0 {
                v.extend_from_slice(&x.to_le_bytes());
            } else {
                v.extend_from_slice(b"sample text, sample text ");
            }
        }
        v.truncate(len);
        v
    }

    #[test]
    fn digests() {
        assert_eq!(crc64(b"123456789"), 0x995D_C9BB_DF19_39FA);
        let h = sha256(b"abc");
        assert_eq!(h[..4], [0xba, 0x78, 0x16, 0xbf]);
        assert_eq!(h[28..], [0xf2, 0x00, 0x15, 0xad]);
        let h = sha256(&[b'a'; 1_000_000]);
        assert_eq!(h[..4], [0xcd, 0xc7, 0x6e, 0x5c]);
    }

    #[test]
    fn vli() {
        for v in [0u64, 1, 127, 128, 16383, 16384, VLI_MAX] {
            let mut b = Vec::new();
            write_vli(&mut b, v);
            let mut pos = 0;
            assert_eq!(read_vli(&b, &mut pos, b.len()).unwrap(), v);
            assert_eq!(pos, b.len());
        }
        assert!(read_vli(&[0x80, 0x00], &mut 0, 2).is_err());
        assert!(read_vli(&[0xFF; 10], &mut 0, 10).is_err());
    }

    #[test]
    fn round_trips() {
        for (data, check, filters) in [
            (Vec::new(), Check::Crc64, vec![]),
            (b"x".to_vec(), Check::None, vec![]),
            (sample(100_000), Check::Crc32, vec![]),
            (sample(100_000), Check::Sha256, vec![Filter::Delta(4)]),
            (sample(300_000), Check::Crc64, vec![Filter::Branch(BranchFilter::X86, 0), Filter::Delta(1)]),
        ] {
            let opts = XzOptions { lzma: LzmaOptions::level(6), check, filters };
            let packed = compress_with(&data, &opts);
            let s = read_stream(&packed, usize::MAX).unwrap();
            assert!(s.data == data);
            assert_eq!((s.consumed, s.check), (packed.len(), check));
            assert_eq!(uncompressed_size(&packed), Some(data.len() as u64));
        }
    }

    #[test]
    fn concatenation_and_padding() {
        let a = compress(b"first ", 1);
        let b = compress(b"second", 9);
        let both = [&a[..], &[0; 8], &b[..], &[0; 4]].concat();
        assert_eq!(decompress(&both).unwrap(), b"first second");
        assert_eq!(uncompressed_size(&both), Some(12));
        assert!(decompress(&[&a[..], &[0; 3]].concat()).is_err());
        assert_eq!(decompress(&[&a[..], b"junk"].concat()), Err(XzError::TrailingData));
    }

    #[test]
    fn errors() {
        let data = sample(50_000);
        let packed = compress(&data, 6);
        assert_eq!(decompress(b"not xz at all"), Err(XzError::NotXz));
        for cut in [0, 6, 12, 13, 40, packed.len() / 2, packed.len() - 1] {
            assert!(decompress(&packed[..cut]).is_err(), "cut at {cut}");
        }
        for i in [7, 12, 14, 30, packed.len() / 2, packed.len() - 20, packed.len() - 10, packed.len() - 3] {
            let mut bad = packed.clone();
            bad[i] ^= 0x01;
            assert!(decompress(&bad).is_err(), "flip at {i}");
        }
        assert_eq!(decompress_with_limit(&packed, data.len() - 1), Err(XzError::OutputLimitExceeded));
    }
}
