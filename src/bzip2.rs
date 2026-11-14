//! bzip2 compression and decompression.
//!
//! A bzip2 stream is `BZh` and a block-size digit (`1`–`9`, for blocks of
//! 100–900 kB), then blocks, then an end-of-stream marker with a CRC of
//! every block's CRC. Each block undoes, in reverse, the steps that made
//! it: Huffman coding with up to six tables chosen per 50 symbols, a
//! move-to-front transform with runs of zeros coded in base 2, the
//! Burrows–Wheeler transform, and a run-length step that turns runs of
//! 4–255 equal bytes into four bytes and a count.
//!
//! The decoder accepts exactly what the reference implementation, libbz2
//! 1.0.8, accepts, including its handling of incomplete or
//! over-subscribed Huffman codes and the deprecated "randomised" blocks
//! that bzip2 0.9.0 could write; fuzzing compares the two. Decompression
//! takes an output limit, so a small file cannot exhaust memory. The
//! encoder writes standard streams that bzip2, libbz2 and 7-Zip read.
//!
//! ```
//! use verified_deflate::bzip2;
//!
//! let data = b"to be or not to be, that is the question".repeat(100);
//! let packed = bzip2::compress(&data, 9);
//! assert_eq!(bzip2::decompress_with_limit(&packed, 1 << 20).unwrap(), data);
//! ```

// The decoder is written as its Verus copy in `verus/` needs: plain
// assignments, shifts and comparisons rather than their shorthands.
#![allow(clippy::assign_op_pattern, clippy::manual_rotate, clippy::manual_range_contains)]

use core::fmt;

/// Every stream starts with these bytes and a block-size digit.
pub const MAGIC: &[u8; 3] = b"BZh";
/// The 48-bit marker that starts a block (the digits of pi).
const BLOCK_MAGIC: u64 = 0x3141_5926_5359;
/// The 48-bit marker that ends a stream (the digits of the square root of pi).
const END_MAGIC: u64 = 0x1772_4538_5090;

const RUNA: u16 = 0;
const RUNB: u16 = 1;
/// Symbols per selector group.
const GROUP_SIZE: usize = 50;
/// Most Huffman tables a block may have.
const MAX_GROUPS: usize = 6;
/// The most symbols: 256 move-to-front values, RUNA/RUNB, end of block.
const MAX_ALPHA: usize = 258;
/// The longest code the format allows.
const MAX_CODE_LEN: usize = 20;
/// libbz2 reads, but ignores, selectors past this many.
const MAX_SELECTORS: usize = 2 + 900_000 / GROUP_SIZE;

/// Why a bzip2 stream could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Bzip2Error {
    /// The data does not start with `BZh` and a digit 1–9.
    NotBzip2,
    /// The data ended in the middle of the stream.
    Truncated,
    /// The stream is corrupt; the message says how.
    Corrupt(&'static str),
    /// A block's data does not match its CRC.
    BlockCrcMismatch {
        /// CRC recorded in the block header.
        expected: u32,
        /// CRC of the decompressed block.
        actual: u32,
    },
    /// The stream's combined CRC does not match its blocks.
    StreamCrcMismatch {
        /// CRC recorded at the end of the stream.
        expected: u32,
        /// CRC computed from the blocks.
        actual: u32,
    },
    /// Decompressing would exceed the output limit.
    OutputLimitExceeded,
    /// After the last stream come bytes that do not start another.
    TrailingData,
}

impl fmt::Display for Bzip2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotBzip2 => f.write_str("not in bzip2 format"),
            Self::Truncated => f.write_str("unexpected end of bzip2 data"),
            Self::Corrupt(why) => write!(f, "corrupt bzip2 data: {why}"),
            Self::BlockCrcMismatch { expected, actual } => {
                write!(f, "block CRC mismatch: header says {expected:08x}, data is {actual:08x}")
            }
            Self::StreamCrcMismatch { expected, actual } => {
                write!(f, "stream CRC mismatch: trailer says {expected:08x}, blocks give {actual:08x}")
            }
            Self::OutputLimitExceeded => f.write_str("decompressed data exceeds the output limit"),
            Self::TrailingData => f.write_str("trailing data after the last bzip2 stream"),
        }
    }
}

impl std::error::Error for Bzip2Error {}

// ---------------------------------------------------------------------------
// CRC

/// bzip2's CRC-32 table: the polynomial of zlib's CRC-32, but most
/// significant bit first (not reflected).
const fn make_crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u32) << 24;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04C1_1DB7 } else { c << 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

const CRC_TABLE: [u32; 256] = make_crc_table();

/// A running bzip2 CRC, as libbz2 keeps it (inverted).
#[derive(Clone, Copy)]
struct Crc(u32);

impl Crc {
    const fn new() -> Self {
        Self(0xFFFF_FFFF)
    }

    fn byte(&mut self, b: u8) {
        self.0 = (self.0 << 8) ^ CRC_TABLE[((self.0 >> 24) as u8 ^ b) as usize];
    }

    const fn finish(self) -> u32 {
        !self.0
    }
}

/// The bzip2 CRC of `data`, as a block header records it.
pub fn block_crc(data: &[u8]) -> u32 {
    let mut c = Crc::new();
    let mut i = 0;
    while i < data.len() {
        c.byte(data[i]);
        i += 1;
    }
    c.finish()
}

/// The stream CRC after a block with CRC `block`: the old one rotated
/// left by one bit, then combined.
const fn combine(stream: u32, block: u32) -> u32 {
    ((stream << 1) | (stream >> 31)) ^ block
}

// ---------------------------------------------------------------------------
// The randomisation table

/// The table that drives "randomised" blocks: bzip2 0.9.0 flipped the low
/// bit of selected bytes of a block that sorted slowly. Later versions
/// never write such blocks but still read them, as this decoder does.
const RAND_NUMS: [u16; 512] = [
    619, 720, 127, 481, 931, 816, 813, 233, 566, 247, 985, 724, 205, 454, 863, 491, 741, 242, 949, 214, 733, 859, 335,
    708, 621, 574, 73, 654, 730, 472, 419, 436, 278, 496, 867, 210, 399, 680, 480, 51, 878, 465, 811, 169, 869, 675,
    611, 697, 867, 561, 862, 687, 507, 283, 482, 129, 807, 591, 733, 623, 150, 238, 59, 379, 684, 877, 625, 169, 643,
    105, 170, 607, 520, 932, 727, 476, 693, 425, 174, 647, 73, 122, 335, 530, 442, 853, 695, 249, 445, 515, 909, 545,
    703, 919, 874, 474, 882, 500, 594, 612, 641, 801, 220, 162, 819, 984, 589, 513, 495, 799, 161, 604, 958, 533, 221,
    400, 386, 867, 600, 782, 382, 596, 414, 171, 516, 375, 682, 485, 911, 276, 98, 553, 163, 354, 666, 933, 424, 341,
    533, 870, 227, 730, 475, 186, 263, 647, 537, 686, 600, 224, 469, 68, 770, 919, 190, 373, 294, 822, 808, 206, 184,
    943, 795, 384, 383, 461, 404, 758, 839, 887, 715, 67, 618, 276, 204, 918, 873, 777, 604, 560, 951, 160, 578, 722,
    79, 804, 96, 409, 713, 940, 652, 934, 970, 447, 318, 353, 859, 672, 112, 785, 645, 863, 803, 350, 139, 93, 354, 99,
    820, 908, 609, 772, 154, 274, 580, 184, 79, 626, 630, 742, 653, 282, 762, 623, 680, 81, 927, 626, 789, 125, 411,
    521, 938, 300, 821, 78, 343, 175, 128, 250, 170, 774, 972, 275, 999, 639, 495, 78, 352, 126, 857, 956, 358, 619,
    580, 124, 737, 594, 701, 612, 669, 112, 134, 694, 363, 992, 809, 743, 168, 974, 944, 375, 748, 52, 600, 747, 642,
    182, 862, 81, 344, 805, 988, 739, 511, 655, 814, 334, 249, 515, 897, 955, 664, 981, 649, 113, 974, 459, 893, 228,
    433, 837, 553, 268, 926, 240, 102, 654, 459, 51, 686, 754, 806, 760, 493, 403, 415, 394, 687, 700, 946, 670, 656,
    610, 738, 392, 760, 799, 887, 653, 978, 321, 576, 617, 626, 502, 894, 679, 243, 440, 680, 879, 194, 572, 640, 724,
    926, 56, 204, 700, 707, 151, 457, 449, 797, 195, 791, 558, 945, 679, 297, 59, 87, 824, 713, 663, 412, 693, 342,
    606, 134, 108, 571, 364, 631, 212, 174, 643, 304, 329, 343, 97, 430, 751, 497, 314, 983, 374, 822, 928, 140, 206,
    73, 263, 980, 736, 876, 478, 430, 305, 170, 514, 364, 692, 829, 82, 855, 953, 676, 246, 369, 970, 294, 750, 807,
    827, 150, 790, 288, 923, 804, 378, 215, 828, 592, 281, 565, 555, 710, 82, 896, 831, 547, 261, 524, 462, 293, 465,
    502, 56, 661, 821, 976, 991, 658, 869, 905, 758, 745, 193, 768, 550, 608, 933, 378, 286, 215, 979, 792, 961, 61,
    688, 793, 644, 986, 403, 106, 366, 905, 644, 372, 567, 466, 434, 645, 210, 389, 550, 919, 135, 780, 773, 635, 389,
    707, 100, 626, 958, 165, 504, 920, 176, 193, 713, 857, 265, 203, 50, 668, 108, 645, 990, 626, 197, 510, 357, 358,
    850, 858, 364, 936, 638,
];

/// The low-bit flips of a randomised block, one call per byte.
struct Randomiser {
    to_go: u16,
    index: usize,
}

impl Randomiser {
    const fn new() -> Self {
        Self { to_go: 0, index: 0 }
    }

    /// The mask (0 or 1) for the next byte, as libbz2's `BZ_RAND_UPD_MASK`
    /// then `BZ_RAND_MASK`. (Every table entry is at least 50, so the
    /// count never wraps.)
    fn next(&mut self) -> u8 {
        if self.to_go == 0 {
            self.to_go = RAND_NUMS[self.index % 512];
            self.index = (self.index + 1) % 512;
        }
        self.to_go = self.to_go.wrapping_sub(1);
        if self.to_go == 1 {
            1
        } else {
            0
        }
    }
}

// ---------------------------------------------------------------------------
// Bit input, most significant bit first

struct Bits<'a> {
    data: &'a [u8],
    /// The next byte to load into `buf`.
    pos: usize,
    /// Unread bits, at the top of the word.
    buf: u64,
    /// How many bits of `buf` are unread.
    n: u32,
}

impl<'a> Bits<'a> {
    const fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, buf: 0, n: 0 }
    }

    fn refill(&mut self) {
        while self.n <= 56 && self.pos < self.data.len() {
            self.buf = self.buf | ((self.data[self.pos] as u64) << (56 - self.n));
            self.pos += 1;
            self.n += 8;
        }
    }

    /// Reads `k` (1–32) bits.
    fn read(&mut self, k: u32) -> Result<u32, Bzip2Error> {
        if self.n < k {
            self.refill();
            if self.n < k {
                return Err(Bzip2Error::Truncated);
            }
        }
        let v = (self.buf >> (64 - k)) as u32;
        self.buf = self.buf << k;
        self.n -= k;
        Ok(v)
    }

    fn bit(&mut self) -> Result<bool, Bzip2Error> {
        let b = self.read(1)?;
        Ok(b == 1)
    }

    fn byte(&mut self) -> Result<u8, Bzip2Error> {
        let b = self.read(8)?;
        Ok(b as u8)
    }

    /// Bytes used so far, counting a partly read byte.
    const fn consumed(&self) -> usize {
        self.pos - (self.n / 8) as usize
    }
}

// ---------------------------------------------------------------------------
// Huffman decoding, as libbz2 does it

/// Bits looked up in one step; longer codes are decoded a bit at a time.
const FAST_BITS: u32 = 10;

/// One table's decoding state: libbz2's `limit`, `base` and `perm`, and a
/// lookup table over the next [`FAST_BITS`] bits.
struct DecodeTable {
    limit: [i32; MAX_CODE_LEN + 2],
    base: [i32; MAX_CODE_LEN + 2],
    perm: [u16; MAX_ALPHA],
    min_len: u32,
    /// `len << 9 | symbol`, or 0 where the code is longer than
    /// [`FAST_BITS`] (or invalid).
    fast: [u16; 1 << FAST_BITS],
}

impl DecodeTable {
    /// libbz2's `BZ2_hbCreateDecodeTables`, for 3–258 lengths of 1–20.
    /// The lengths need not form a complete, or even a valid, prefix code;
    /// decoding then does what libbz2's does.
    fn new(lengths: &[u8]) -> Self {
        let mut min_len = 32u8;
        let mut max_len = 0u8;
        let mut i = 0;
        while i < lengths.len() {
            if lengths[i] < min_len {
                min_len = lengths[i];
            }
            if lengths[i] > max_len {
                max_len = lengths[i];
            }
            i += 1;
        }
        let mut t = Self {
            limit: [0; MAX_CODE_LEN + 2],
            base: [0; MAX_CODE_LEN + 2],
            perm: [0; MAX_ALPHA],
            min_len: min_len as u32,
            fast: [0; 1 << FAST_BITS],
        };
        // The symbols, by length then by value.
        let mut pp = 0;
        let mut len = min_len;
        while len <= max_len {
            let mut sym = 0;
            while sym < lengths.len() {
                if lengths[sym] == len && pp < MAX_ALPHA {
                    t.perm[pp] = sym as u16;
                    pp += 1;
                }
                sym += 1;
            }
            len += 1;
        }
        // base[l + 1] counts the codes of length l, then accumulates.
        let mut i = 0;
        while i < lengths.len() {
            let l = lengths[i] as usize;
            t.base[l + 1] += 1;
            i += 1;
        }
        let mut i = 1;
        while i < MAX_CODE_LEN + 2 {
            t.base[i] += t.base[i - 1];
            i += 1;
        }
        let mut vec = 0i32;
        let mut i = min_len as usize;
        while i <= max_len as usize {
            vec += t.base[i + 1] - t.base[i];
            t.limit[i] = vec - 1;
            vec <<= 1;
            i += 1;
        }
        let mut i = min_len as usize + 1;
        while i <= max_len as usize {
            t.base[i] = ((t.limit[i - 1] + 1) << 1) - t.base[i];
            i += 1;
        }
        let mut code = 0u32;
        while code < 1 << FAST_BITS {
            let mut zn = t.min_len;
            while zn <= FAST_BITS {
                let zvec = (code >> (FAST_BITS - zn)) as i32;
                if zvec <= t.limit[zn as usize] {
                    if let Some(sym) = t.symbol(zn, zvec) {
                        t.fast[code as usize] = ((zn << 9) as u16) | sym;
                    }
                    break;
                }
                zn += 1;
            }
            code += 1;
        }
        t
    }

    /// The symbol for the `zn`-bit code `zvec`, if libbz2 accepts it.
    fn symbol(&self, zn: u32, zvec: i32) -> Option<u16> {
        let i = zvec - self.base[zn as usize];
        if 0 <= i && i < MAX_ALPHA as i32 {
            Some(self.perm[i as usize])
        } else {
            None
        }
    }

    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, Bzip2Error> {
        if bits.n < FAST_BITS {
            bits.refill();
        }
        if bits.n >= FAST_BITS {
            let e = self.fast[(bits.buf >> (64 - FAST_BITS)) as usize];
            if e != 0 {
                let len = (e >> 9) as u32;
                bits.buf = bits.buf << len;
                bits.n -= len;
                return Ok(e & 0x1ff);
            }
        }
        self.decode_slow(bits)
    }

    /// libbz2's GET_MTF_VAL, a bit at a time.
    fn decode_slow(&self, bits: &mut Bits<'_>) -> Result<u16, Bzip2Error> {
        let mut zn = self.min_len;
        let mut zvec = bits.read(zn)? as i32;
        loop {
            if zn > MAX_CODE_LEN as u32 {
                return Err(Bzip2Error::Corrupt("invalid Huffman code"));
            }
            if zvec <= self.limit[zn as usize] {
                break;
            }
            zn += 1;
            let b = bits.read(1)?;
            zvec = (zvec << 1) | b as i32;
        }
        match self.symbol(zn, zvec) {
            Some(sym) => Ok(sym),
            None => Err(Bzip2Error::Corrupt("invalid Huffman code")),
        }
    }
}

/// The next symbol of a block: each group of 50 is coded with the table
/// its selector names.
fn next_symbol(
    bits: &mut Bits<'_>,
    tables: &[DecodeTable],
    selectors: &[u8],
    group: &mut usize,
    left: &mut usize,
) -> Result<u16, Bzip2Error> {
    if *left == 0 {
        if *group >= selectors.len() {
            return Err(Bzip2Error::Corrupt("ran out of selectors"));
        }
        *group += 1;
        *left = GROUP_SIZE;
    }
    *left -= 1;
    tables[selectors[*group - 1] as usize].decode(bits)
}

// ---------------------------------------------------------------------------
// Decompression

/// A decoded bzip2 stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bzip2Stream {
    /// The decompressed data.
    pub data: Vec<u8>,
    /// Bytes of input the stream used, through its final padded byte.
    pub consumed: usize,
    /// The block size digit from the header, 1–9 (blocks of up to
    /// 100,000 × this many bytes).
    pub level: u8,
}

/// Whether `data` starts like a bzip2 stream: `BZh`, a digit 1–9, and the
/// start of a block or of the end-of-stream marker.
pub fn is_bzip2(data: &[u8]) -> bool {
    data.len() >= 10
        && data.starts_with(MAGIC)
        && (b'1'..=b'9').contains(&data[3])
        && (data[4..10] == BLOCK_MAGIC.to_be_bytes()[2..] || data[4..10] == END_MAGIC.to_be_bytes()[2..])
}

/// Reads a stream's header: `BZh` and the block-size digit.
fn stream_level(input: &[u8]) -> Result<u8, Bzip2Error> {
    if input.len() < 4 {
        return Err(if input.starts_with(&MAGIC[..input.len()]) { Bzip2Error::Truncated } else { Bzip2Error::NotBzip2 });
    }
    if input[0] != b'B' || input[1] != b'Z' || input[2] != b'h' || input[3] < b'1' || input[3] > b'9' {
        return Err(Bzip2Error::NotBzip2);
    }
    Ok(input[3] - b'0')
}

/// Reads a block's or the end marker's six magic bytes: `true` for the
/// end of the stream.
fn block_magic(bits: &mut Bits<'_>) -> Result<bool, Bzip2Error> {
    let first = bits.byte()?;
    let end = if first == 0x17 {
        true
    } else if first == 0x31 {
        false
    } else {
        return Err(Bzip2Error::Corrupt("bad block header"));
    };
    let magic = if end { END_MAGIC } else { BLOCK_MAGIC };
    let mut shift = 5u32;
    while shift > 0 {
        shift -= 1;
        let b = bits.byte()?;
        if b != (magic >> (8 * shift)) as u8 {
            return Err(Bzip2Error::Corrupt("bad block header"));
        }
    }
    Ok(end)
}

/// Decodes the bzip2 stream at the start of `input`, producing at most
/// `max_output` bytes, and checks every CRC.
pub fn read_stream(input: &[u8], max_output: usize) -> Result<Bzip2Stream, Bzip2Error> {
    let level = stream_level(input)?;
    let mut bits = Bits::new(&input[4..]);
    let mut out = Vec::new();
    let mut stream_crc = 0u32;
    let mut tt = Vec::new();
    loop {
        let end = block_magic(&mut bits)?;
        let crc = bits.read(32)?;
        if end {
            if crc != stream_crc {
                return Err(Bzip2Error::StreamCrcMismatch { expected: crc, actual: stream_crc });
            }
            return Ok(Bzip2Stream { data: out, consumed: 4 + bits.consumed(), level });
        }
        let actual = read_block(&mut bits, level, &mut tt, &mut out, max_output)?;
        if actual != crc {
            return Err(Bzip2Error::BlockCrcMismatch { expected: crc, actual });
        }
        stream_crc = combine(stream_crc, actual);
    }
}

/// Rewrites the CRCs in the bzip2 stream at the start of `stream` (each
/// block's, and the stream's) to match what the blocks decode to, for as
/// many blocks as decode. Fuzzing uses this to reach past the CRC checks
/// and compare decoders on what the blocks hold. Decoding stops at
/// `max_output` bytes. Returns how many CRCs it wrote.
#[doc(hidden)]
pub fn fix_crcs(stream: &mut [u8], max_output: usize) -> usize {
    let Ok(level) = stream_level(stream) else { return 0 };
    // (bit offset after the header, CRC) of each CRC field.
    let mut slots = Vec::new();
    {
        let mut bits = Bits::new(&stream[4..]);
        let mut out = Vec::new();
        let mut tt = Vec::new();
        let mut stream_crc = 0u32;
        while let Ok(end) = block_magic(&mut bits) {
            let at = bits.pos * 8 - bits.n as usize;
            if bits.read(32).is_err() {
                break;
            }
            if end {
                slots.push((at, stream_crc));
                break;
            }
            let Ok(actual) = read_block(&mut bits, level, &mut tt, &mut out, max_output) else { break };
            slots.push((at, actual));
            stream_crc = combine(stream_crc, actual);
        }
    }
    for &(bit, crc) in &slots {
        for k in 0..32 {
            let (byte, shift) = ((32 + bit + k) / 8, 7 - (32 + bit + k) % 8);
            let v = (crc >> (31 - k)) as u8 & 1;
            stream[byte] = (stream[byte] & !(1 << shift)) | (v << shift);
        }
    }
    slots.len()
}

/// Decodes one block (after its magic and CRC) onto `out`, returning the
/// CRC of what it added. `tt` is scratch space kept between blocks.
fn read_block(
    bits: &mut Bits<'_>,
    level: u8,
    tt: &mut Vec<u32>,
    out: &mut Vec<u8>,
    max_output: usize,
) -> Result<u32, Bzip2Error> {
    let randomised = bits.bit()?;
    let orig_ptr = bits.read(24)? as usize;
    let block_max = 100_000 * level as usize;
    if orig_ptr > 10 + block_max {
        return Err(Bzip2Error::Corrupt("origin pointer out of range"));
    }

    // Which byte values occur: 16 bits for groups of 16, then 16 per
    // group that is used.
    let used16 = bits.read(16)?;
    let mut seq_to_unseq = [0u8; 256];
    let mut n_in_use = 0usize;
    let mut i = 0usize;
    while i < 16 {
        if used16 & (0x8000 >> i) != 0 {
            let used = bits.read(16)?;
            let mut j = 0usize;
            while j < 16 {
                if used & (0x8000 >> j) != 0 {
                    seq_to_unseq[n_in_use] = (i * 16 + j) as u8;
                    n_in_use += 1;
                }
                j += 1;
            }
        }
        i += 1;
    }
    if n_in_use == 0 {
        return Err(Bzip2Error::Corrupt("no byte values in use"));
    }
    let alpha_size = n_in_use + 2;

    // The selectors, move-to-front coded in unary.
    let n_groups = bits.read(3)? as usize;
    if n_groups < 2 || n_groups > MAX_GROUPS {
        return Err(Bzip2Error::Corrupt("bad number of Huffman tables"));
    }
    let n_selectors = bits.read(15)? as usize;
    if n_selectors == 0 {
        return Err(Bzip2Error::Corrupt("no selectors"));
    }
    let mut selectors: Vec<u8> = Vec::new();
    let mut pos: [u8; MAX_GROUPS] = [0, 1, 2, 3, 4, 5];
    let mut i = 0usize;
    while i < n_selectors {
        let mut j = 0usize;
        while bits.bit()? {
            j += 1;
            if j >= n_groups {
                return Err(Bzip2Error::Corrupt("bad selector"));
            }
        }
        if i < MAX_SELECTORS {
            let v = pos[j];
            let mut k = j;
            while k > 0 {
                pos[k] = pos[k - 1];
                k -= 1;
            }
            pos[0] = v;
            selectors.push(v);
        }
        i += 1;
    }

    // The code lengths, delta coded.
    let mut tables: Vec<DecodeTable> = Vec::new();
    let mut lengths = [0u8; MAX_ALPHA];
    let mut t = 0usize;
    while t < n_groups {
        let mut curr = bits.read(5)? as i32;
        let mut s = 0usize;
        while s < alpha_size {
            loop {
                if curr < 1 || curr > MAX_CODE_LEN as i32 {
                    return Err(Bzip2Error::Corrupt("bad code length"));
                }
                if !bits.bit()? {
                    break;
                }
                if bits.bit()? {
                    curr -= 1;
                } else {
                    curr += 1;
                }
            }
            lengths[s] = curr as u8;
            s += 1;
        }
        tables.push(DecodeTable::new(&lengths[..alpha_size]));
        t += 1;
    }

    // The move-to-front values, with runs of the front value in RUNA and
    // RUNB, into `tt` as bytes.
    let eob = n_in_use as u16 + 1;
    let mut mtf = [0u8; 256];
    let mut i = 0usize;
    while i < 256 {
        mtf[i] = i as u8;
        i += 1;
    }
    let mut counts = [0u32; 256];
    tt.clear();
    let mut group = 0usize;
    let mut left = 0usize;
    let mut sym = next_symbol(bits, &tables, &selectors, &mut group, &mut left)?;
    loop {
        if sym == eob {
            break;
        }
        if sym == RUNA || sym == RUNB {
            let mut run = 0usize;
            let mut weight = 1usize;
            while sym == RUNA || sym == RUNB {
                if weight >= 2 * 1024 * 1024 {
                    return Err(Bzip2Error::Corrupt("run too long"));
                }
                run += weight << sym;
                weight <<= 1;
                sym = next_symbol(bits, &tables, &selectors, &mut group, &mut left)?;
            }
            if run > block_max - tt.len() {
                return Err(Bzip2Error::Corrupt("block too long"));
            }
            let b = seq_to_unseq[mtf[0] as usize];
            counts[b as usize] += run as u32;
            tt.resize(tt.len() + run, b as u32);
        } else {
            if tt.len() >= block_max {
                return Err(Bzip2Error::Corrupt("block too long"));
            }
            let nn = (sym - 1) as usize;
            let v = mtf[nn];
            let mut k = nn;
            while k > 0 {
                mtf[k] = mtf[k - 1];
                k -= 1;
            }
            mtf[0] = v;
            let b = seq_to_unseq[v as usize];
            counts[b as usize] += 1;
            tt.push(b as u32);
            sym = next_symbol(bits, &tables, &selectors, &mut group, &mut left)?;
        }
    }
    if orig_ptr >= tt.len() {
        return Err(Bzip2Error::Corrupt("origin pointer out of range"));
    }
    let block = inverse_bwt(tt, &counts, orig_ptr, randomised)?;
    let start = out.len();
    rle1_decode(&block, out, max_output)?;
    Ok(block_crc(&out[start..]))
}

/// Undoes the Burrows–Wheeler transform: `tt` holds the last column (one
/// byte per entry) and `counts` how often each byte occurs in it, and the
/// original block is the rotation at row `orig_ptr`. Each entry gets, in
/// its upper bits, the row that follows it; walking those links from the
/// origin spells the block. Randomised blocks have their bits flipped
/// back.
fn inverse_bwt(tt: &mut [u32], counts: &[u32; 256], orig_ptr: usize, randomised: bool) -> Result<Vec<u8>, Bzip2Error> {
    let n = tt.len();
    // Where each byte's rows start in the sorted first column.
    let mut start = [0u32; 256];
    let mut sum = 0u32;
    let mut c = 0usize;
    while c < 256 {
        start[c] = sum;
        sum = sum.wrapping_add(counts[c]);
        c += 1;
    }
    let mut i = 0usize;
    while i < n {
        let b = (tt[i] & 0xff) as usize;
        let row = start[b] as usize;
        // The counts are exact, so each byte's rows stay in range.
        if row >= n {
            return Err(Bzip2Error::Corrupt("byte counts do not match the block"));
        }
        tt[row] = tt[row] | ((i as u32) << 8);
        start[b] = start[b].wrapping_add(1);
        i += 1;
    }
    let mut block = Vec::with_capacity(n);
    let mut rand = Randomiser::new();
    let mut pos = orig_ptr;
    let mut k = 0usize;
    while k < n {
        if pos >= n {
            return Err(Bzip2Error::Corrupt("broken row link"));
        }
        pos = (tt[pos] >> 8) as usize;
        if pos >= n {
            return Err(Bzip2Error::Corrupt("broken row link"));
        }
        let b = tt[pos] as u8;
        block.push(if randomised { b ^ rand.next() } else { b });
        k += 1;
    }
    Ok(block)
}

/// Undoes the initial run-length step onto `out`: after four equal bytes
/// comes a count (0–255) of further copies. At most `max_output` bytes
/// end up in `out`. A block may not end between a run of four and its
/// count.
fn rle1_decode(block: &[u8], out: &mut Vec<u8>, max_output: usize) -> Result<(), Bzip2Error> {
    let mut last = 0u8;
    let mut run = 0u32;
    let mut i = 0usize;
    while i < block.len() {
        let b = block[i];
        i += 1;
        if run == 4 {
            let extra = b as usize;
            if extra > max_output.saturating_sub(out.len()) {
                return Err(Bzip2Error::OutputLimitExceeded);
            }
            out.resize(out.len() + extra, last);
            run = 0;
        } else {
            if run > 0 && b == last {
                run += 1;
            } else {
                last = b;
                run = 1;
            }
            if out.len() >= max_output {
                return Err(Bzip2Error::OutputLimitExceeded);
            }
            out.push(b);
        }
    }
    if run == 4 {
        return Err(Bzip2Error::Corrupt("block ends inside a run"));
    }
    Ok(())
}

/// The initial run-length step, on `data` from its start: runs of 4–255
/// equal bytes become four bytes and a count of the rest; shorter runs
/// stay. Stops once `block` holds `limit` bytes or more (each run adds at
/// most 5), and returns how much of `data` it used.
fn rle1_encode(data: &[u8], block: &mut Vec<u8>, limit: usize) -> usize {
    let mut i = 0;
    while i < data.len() && block.len() < limit {
        let b = data[i];
        let mut run = 1;
        while run < 255 && i + run < data.len() && data[i + run] == b {
            run += 1;
        }
        if run >= 4 {
            block.extend_from_slice(&[b, b, b, b, (run - 4) as u8]);
        } else {
            block.resize(block.len() + run, b);
        }
        i += run;
    }
    i
}

/// Appends a run of `zeros` (at least 1) move-to-front zeros in bzip2's
/// bijective base 2: RUNA for digit 1, RUNB for digit 2, least
/// significant first, so the run is the sum of digit × 2^position.
fn push_zero_run(syms: &mut Vec<u16>, zeros: usize) {
    let mut z = zeros - 1;
    loop {
        syms.push(if z & 1 == 1 { RUNB } else { RUNA });
        if z < 2 {
            break;
        }
        z = (z - 2) / 2;
    }
}

/// Decompresses a bzip2 file: every stream, concatenated, as `bzip2 -d`
/// does, with no output limit. For untrusted input prefer
/// [`decompress_with_limit`].
pub fn decompress(input: &[u8]) -> Result<Vec<u8>, Bzip2Error> {
    decompress_with_limit(input, usize::MAX)
}

/// Like [`decompress`], failing with [`Bzip2Error::OutputLimitExceeded`]
/// rather than producing more than `max_output` bytes in all. Anything
/// after the last stream that does not start another one is an error
/// ([`Bzip2Error::TrailingData`]).
pub fn decompress_with_limit(input: &[u8], max_output: usize) -> Result<Vec<u8>, Bzip2Error> {
    let mut out: Vec<u8> = Vec::new();
    let mut rest = input;
    loop {
        let s = read_stream(rest, max_output - out.len())?;
        out.extend_from_slice(&s.data);
        rest = &rest[s.consumed..];
        if rest.is_empty() {
            return Ok(out);
        }
        if !rest.starts_with(MAGIC) {
            return Err(Bzip2Error::TrailingData);
        }
    }
}

// ---------------------------------------------------------------------------
// Compression

/// Bit output, most significant bit first.
struct BitOut {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitOut {
    fn write(&mut self, v: u32, k: u32) {
        debug_assert!(k <= 32 && (k == 32 || v >> k == 0));
        self.acc = (self.acc << k) | u64::from(v);
        self.n += k;
        while self.n >= 8 {
            self.n -= 8;
            self.out.push((self.acc >> self.n) as u8);
        }
    }

    fn write48(&mut self, v: u64) {
        self.write((v >> 24) as u32, 24);
        self.write((v & 0xff_ffff) as u32, 24);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push((self.acc << (8 - self.n)) as u8);
        }
        self.out
    }
}

/// Compresses `data` into a bzip2 stream with blocks of up to 100,000 ×
/// `level` bytes (`level` is clamped to 1–9; 9, the most compression, is
/// what `bzip2` uses by default).
pub fn compress(data: &[u8], level: u8) -> Vec<u8> {
    compress_inner(data, level, false)
}

/// Like [`compress`], but writes "randomised" blocks, which no bzip2 since
/// 0.9.5 writes. For testing decoders only.
#[doc(hidden)]
pub fn compress_randomised(data: &[u8], level: u8) -> Vec<u8> {
    compress_inner(data, level, true)
}

fn compress_inner(data: &[u8], level: u8, randomise: bool) -> Vec<u8> {
    let level = level.clamp(1, 9);
    // libbz2's block limit, which leaves room for the run that crosses it.
    let block_limit = 100_000 * usize::from(level) - 19;
    let mut w = BitOut { out: Vec::with_capacity(data.len() / 3 + 64), acc: 0, n: 0 };
    for &b in MAGIC {
        w.write(u32::from(b), 8);
    }
    w.write(u32::from(b'0' + level), 8);
    let mut stream_crc = 0u32;
    let mut block = Vec::with_capacity(block_limit + 5);
    let mut i = 0;
    while i < data.len() {
        block.clear();
        let start = i;
        i += rle1_encode(&data[i..], &mut block, block_limit);
        let crc = block_crc(&data[start..i]);
        stream_crc = combine(stream_crc, crc);
        if randomise {
            let mut r = Randomiser::new();
            for b in &mut block {
                *b ^= r.next();
            }
        }
        write_block(&mut w, &block, crc, randomise);
    }
    w.write48(END_MAGIC);
    w.write(stream_crc, 32);
    w.finish()
}

/// Writes one block of run-length-coded bytes.
fn write_block(w: &mut BitOut, block: &[u8], crc: u32, randomised: bool) {
    let (last, orig_ptr) = crate::bwt::bwt(block);

    let mut in_use = [false; 256];
    for &b in block {
        in_use[b as usize] = true;
    }
    let mut unseq_to_seq = [0u8; 256];
    let mut n_in_use = 0;
    for (b, _) in in_use.iter().enumerate().filter(|(_, &u)| u) {
        unseq_to_seq[b] = n_in_use as u8;
        n_in_use += 1;
    }
    let alpha_size = n_in_use + 2;
    let eob = (n_in_use + 1) as u16;

    // Move-to-front, with runs of zeros in bijective base 2 (RUNA = 1,
    // RUNB = 2, least significant digit first).
    let mut mtf: [u8; 256] = core::array::from_fn(|i| i as u8);
    let mut syms: Vec<u16> = Vec::with_capacity(last.len() + 1);
    let mut zeros = 0usize;
    let flush = |syms: &mut Vec<u16>, zeros: &mut usize| {
        if *zeros > 0 {
            push_zero_run(syms, *zeros);
            *zeros = 0;
        }
    };
    for &b in &last {
        let s = unseq_to_seq[b as usize];
        if mtf[0] == s {
            zeros += 1;
            continue;
        }
        flush(&mut syms, &mut zeros);
        let p = mtf.iter().position(|&x| x == s).unwrap_or(0);
        mtf.copy_within(0..p, 1);
        mtf[0] = s;
        syms.push(p as u16 + 1);
    }
    flush(&mut syms, &mut zeros);
    syms.push(eob);

    let (n_groups, selectors, lengths) = choose_tables(&syms, alpha_size);

    w.write48(BLOCK_MAGIC);
    w.write(crc, 32);
    w.write(u32::from(randomised), 1);
    w.write(orig_ptr as u32, 24);
    let mut used16 = 0u32;
    for i in 0..16 {
        if in_use[i * 16..i * 16 + 16].iter().any(|&u| u) {
            used16 |= 0x8000 >> i;
        }
    }
    w.write(used16, 16);
    for i in 0..16 {
        if used16 & (0x8000 >> i) != 0 {
            let mut bits16 = 0;
            for j in 0..16 {
                if in_use[i * 16 + j] {
                    bits16 |= 0x8000 >> j;
                }
            }
            w.write(bits16, 16);
        }
    }
    w.write(n_groups as u32, 3);
    w.write(selectors.len() as u32, 15);
    let mut pos: [u8; MAX_GROUPS] = [0, 1, 2, 3, 4, 5];
    for &s in &selectors {
        let j = pos.iter().position(|&x| x == s).unwrap_or(0);
        pos.copy_within(0..j, 1);
        pos[0] = s;
        for _ in 0..j {
            w.write(1, 1);
        }
        w.write(0, 1);
    }
    for t in lengths.iter().take(n_groups) {
        let mut curr = t[0];
        w.write(u32::from(curr), 5);
        for &len in &t[..alpha_size] {
            while curr < len {
                w.write(0b10, 2);
                curr += 1;
            }
            while curr > len {
                w.write(0b11, 2);
                curr -= 1;
            }
            w.write(0, 1);
        }
    }
    let codes: Vec<Vec<u32>> = lengths[..n_groups].iter().map(|l| canonical(&l[..alpha_size])).collect();
    for (g, chunk) in syms.chunks(GROUP_SIZE).enumerate() {
        let t = selectors[g] as usize;
        for &s in chunk {
            w.write(codes[t][s as usize], u32::from(lengths[t][s as usize]));
        }
    }
}

/// Canonical codes for `lengths`: by length, then symbol, as libbz2's
/// `BZ2_hbAssignCodes`.
fn canonical(lengths: &[u8]) -> Vec<u32> {
    let mut codes = vec![0u32; lengths.len()];
    let mut code = 0u32;
    for len in 1..=MAX_CODE_LEN as u8 {
        for (sym, &l) in lengths.iter().enumerate() {
            if l == len {
                codes[sym] = code;
                code += 1;
            }
        }
        code <<= 1;
    }
    codes
}

/// Longest code the encoder makes ([`crate::huffman::build_lengths`]'s limit).
const ENCODE_MAX_LEN: usize = crate::huffman::MAX_CODE_LEN;

/// Picks the Huffman tables and, for each group of 50 symbols, the table
/// that codes it: libbz2's method, starting from tables that each favour
/// a band of symbol frequencies, then four rounds of choosing the
/// cheapest table per group and rebuilding the tables from the groups
/// that chose them.
#[allow(clippy::type_complexity)]
fn choose_tables(syms: &[u16], alpha_size: usize) -> (usize, Vec<u8>, Vec<[u8; MAX_ALPHA]>) {
    let n = syms.len();
    let n_groups = match n {
        0..200 => 2,
        200..600 => 3,
        600..1200 => 4,
        1200..2400 => 5,
        _ => 6,
    };
    let mut freq = [0u32; MAX_ALPHA];
    for &s in syms {
        freq[s as usize] += 1;
    }
    // Initial tables: a band of symbols each, cheap inside it.
    let mut lengths = vec![[0u8; MAX_ALPHA]; n_groups];
    let mut remaining = n as u32;
    let mut gs = 0usize;
    for parts_left in (1..=n_groups).rev() {
        let target = remaining / parts_left as u32;
        let mut ge = gs as isize - 1;
        let mut acc = 0u32;
        while acc < target && ge < alpha_size as isize - 1 {
            ge += 1;
            acc += freq[ge as usize];
        }
        if ge > gs as isize && parts_left != n_groups && parts_left != 1 && (n_groups - parts_left) % 2 == 1 {
            acc -= freq[ge as usize];
            ge -= 1;
        }
        for (v, len) in lengths[parts_left - 1].iter_mut().enumerate().take(alpha_size) {
            *len = if (v as isize) >= gs as isize && (v as isize) <= ge { 0 } else { 15 };
        }
        gs = (ge + 1) as usize;
        remaining -= acc;
    }

    let n_selectors = n.div_ceil(GROUP_SIZE);
    let mut selectors = vec![0u8; n_selectors];
    for _ in 0..4 {
        let mut group_freq = vec![[0u32; MAX_ALPHA]; n_groups];
        for (g, chunk) in syms.chunks(GROUP_SIZE).enumerate() {
            let mut best = 0;
            let mut best_cost = u32::MAX;
            for (t, l) in lengths.iter().enumerate() {
                let cost: u32 = chunk.iter().map(|&s| u32::from(l[s as usize])).sum();
                if cost < best_cost {
                    best_cost = cost;
                    best = t;
                }
            }
            selectors[g] = best as u8;
            for &s in chunk {
                group_freq[best][s as usize] += 1;
            }
        }
        for (t, f) in group_freq.iter().enumerate() {
            // Every symbol needs a code, used or not.
            let weights: Vec<u32> = f[..alpha_size].iter().map(|&x| x.max(1)).collect();
            let mut l = [0u8; MAX_ALPHA];
            if crate::huffman::build_lengths(&weights, ENCODE_MAX_LEN, &mut l[..alpha_size]).is_err() {
                // Cannot happen (at most 258 symbols, all used, fit in
                // 15-bit codes); a flat code is still valid.
                let flat = (usize::BITS - (alpha_size - 1).leading_zeros()) as u8;
                l[..alpha_size].fill(flat.max(1));
            }
            lengths[t] = l;
        }
    }
    (n_groups, selectors, lengths)
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// The CRC of `data` by its definition: the polynomial 0x04C11DB7
    /// applied bit by bit, most significant first, from all ones, then
    /// inverted.
    fn crc_bitwise(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= u32::from(b) << 24;
            for _ in 0..8 {
                c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04C1_1DB7 } else { c << 1 };
            }
        }
        !c
    }

    /// Every CRC table entry is eight steps of the definition.
    #[kani::proof]
    #[kani::unwind(9)]
    fn crc_table_matches_definition() {
        let b: u8 = kani::any();
        let mut c = u32::from(b) << 24;
        for _ in 0..8 {
            c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04C1_1DB7 } else { c << 1 };
        }
        assert_eq!(CRC_TABLE[b as usize], c);
    }

    /// One table step equals eight bitwise steps from any register, so the
    /// CRC of every input equals the definition's.
    #[kani::proof]
    #[kani::unwind(9)]
    fn crc_step_matches_definition() {
        let mut c = Crc(kani::any());
        let b: u8 = kani::any();
        let mut r = c.0 ^ (u32::from(b) << 24);
        for _ in 0..8 {
            r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04C1_1DB7 } else { r << 1 };
        }
        c.byte(b);
        assert_eq!(c.0, r);
    }

    /// Whole CRCs of up to three bytes equal the definition.
    #[kani::proof]
    #[kani::unwind(9)]
    fn crc_matches_definition() {
        let data: [u8; 3] = kani::any();
        let n: usize = kani::any_where(|&n: &usize| n <= 3);
        assert_eq!(block_crc(&data[..n]), crc_bitwise(&data[..n]));
    }

    /// Every run of 1–4096 zeros is coded in bijective base 2 (RUNA = 1,
    /// RUNB = 2, least significant digit first) in at most 12 symbols, and
    /// summing digit × 2^position, as the decoder does, gives the run back.
    #[kani::proof]
    #[kani::unwind(14)]
    fn zero_runs_round_trip() {
        let zeros: usize = kani::any_where(|&z: &usize| (1..=4096).contains(&z));
        let mut syms = Vec::new();
        push_zero_run(&mut syms, zeros);
        assert!(!syms.is_empty() && syms.len() <= 12);
        let mut run = 0usize;
        let mut weight = 1usize;
        for &sym in &syms {
            assert!(sym == RUNA || sym == RUNB);
            run += weight << sym;
            weight <<= 1;
        }
        assert_eq!(run, zeros);
    }

    /// Bits written most significant first read back the same, for any two
    /// fields of 1–32 bits.
    #[kani::proof]
    #[kani::unwind(10)]
    fn bits_round_trip() {
        let k1: u32 = kani::any_where(|&k: &u32| (1..=32).contains(&k));
        let k2: u32 = kani::any_where(|&k: &u32| (1..=32).contains(&k));
        let v1: u32 = kani::any_where(|&v: &u32| k1 == 32 || v >> k1 == 0);
        let v2: u32 = kani::any_where(|&v: &u32| k2 == 32 || v >> k2 == 0);
        let mut w = BitOut { out: Vec::new(), acc: 0, n: 0 };
        w.write(v1, k1);
        w.write(v2, k2);
        let bytes = w.finish();
        assert_eq!(bytes.len() as u32, (k1 + k2).div_ceil(8));
        let mut r = Bits::new(&bytes);
        assert_eq!(r.read(k1), Ok(v1));
        assert_eq!(r.read(k2), Ok(v2));
        assert_eq!(r.consumed(), bytes.len());
    }

    /// The lookup table never disagrees with libbz2's bit-by-bit decoding:
    /// for any code lengths (1–20, even incomplete or over-subscribed) of a
    /// four-symbol alphabet and any input, the two give the same symbol, or
    /// both an error, using the same bits.
    #[cfg(any(feature = "kani-extended", kani_sample))] // too large for CI runners
    #[kani::proof]
    #[kani::unwind(1030)]
    #[kani::solver(kissat)]
    fn decode_table_matches_bit_by_bit_decoding() {
        let lengths: [u8; 4] = kani::any();
        let lengths = lengths.map(|l| l % 20 + 1);
        let t = DecodeTable::new(&lengths);
        let input: [u8; 3] = kani::any();
        let mut fast = Bits::new(&input);
        let mut slow = Bits::new(&input);
        let a = t.decode(&mut fast);
        let b = t.decode_slow(&mut slow);
        assert_eq!(a, b);
        if a.is_ok() {
            assert_eq!(fast.n, slow.n);
            assert_eq!(fast.pos, slow.pos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Checks over finite domains, run on every input of the domain: each
    // is a proof for that domain, and far cheaper than a model checker.

    /// The run-length step round-trips for every input of up to twelve
    /// bytes over a two-byte alphabet (every run shape up to length 12),
    /// and its output never ends between a run of four and its count.
    #[test]
    fn rle1_round_trips_for_every_short_input() {
        for n in 0..=12usize {
            for bits in 0..1u32 << n {
                let data: Vec<u8> = (0..n).map(|i| (bits >> i) as u8 & 1).collect();
                let mut block = Vec::new();
                assert_eq!(rle1_encode(&data, &mut block, usize::MAX), n);
                let mut out = Vec::new();
                assert_eq!(rle1_decode(&block, &mut out, usize::MAX), Ok(()), "{data:?}");
                assert_eq!(out, data);
            }
        }
    }

    /// Runs of every length up to 3,000 split at 255 and come back whole,
    /// with the block holding five bytes per 255 or fewer.
    #[test]
    fn every_long_run_round_trips() {
        for len in 1..=3000usize {
            let data = vec![7u8; len];
            let mut block = Vec::new();
            assert_eq!(rle1_encode(&data, &mut block, usize::MAX), len);
            assert!(block.len() <= 5 * len.div_ceil(255));
            let mut out = Vec::new();
            assert_eq!(rle1_decode(&block, &mut out, usize::MAX), Ok(()));
            assert_eq!(out, data);
        }
    }

    /// The decoder's inverse transform undoes the encoder's block sort for
    /// every block of up to eight bytes over a three-byte alphabet, and
    /// every block of one or two bytes.
    #[test]
    fn inverse_bwt_undoes_bwt_for_every_short_block() {
        let check = |s: &[u8]| {
            let (last, orig) = crate::bwt::bwt(s);
            let mut counts = [0u32; 256];
            let mut tt: Vec<u32> = Vec::new();
            for &b in &last {
                counts[b as usize] += 1;
                tt.push(u32::from(b));
            }
            assert_eq!(inverse_bwt(&mut tt, &counts, orig, false).unwrap(), s);
        };
        for n in 1..=8u32 {
            for mut k in 0..3u32.pow(n) {
                let s: Vec<u8> = (0..n)
                    .map(|_| {
                        let d = (k % 3) as u8;
                        k /= 3;
                        d
                    })
                    .collect();
                check(&s);
            }
        }
        for v in 0..=0xFFFFu32 {
            check(&[v as u8]);
            check(&[v as u8, (v >> 8) as u8]);
        }
    }

    /// Compressing then decompressing gives the input back, for every
    /// input of up to two bytes.
    #[test]
    fn compress_round_trips_for_every_input_up_to_two_bytes() {
        let check = |d: &[u8]| assert_eq!(decompress(&compress(d, 1)).unwrap(), d);
        check(b"");
        for v in 0..=0xFFFFu32 {
            if v < 256 {
                check(&[v as u8]);
            }
            check(&[v as u8, (v >> 8) as u8]);
        }
    }

    #[test]
    fn crc_matches_bzip2() {
        // `printf 123456789 | bzip2 | xxd`: the block CRC is fc891918.
        assert_eq!(block_crc(b"123456789"), 0xfc89_1918);
        assert_eq!(block_crc(b""), 0);
    }

    #[test]
    fn empty_input_is_the_minimal_stream() {
        let s = compress(b"", 9);
        assert_eq!(s, b"BZh9\x17\x72\x45\x38\x50\x90\0\0\0\0");
        assert_eq!(decompress(&s).unwrap(), b"");
        assert!(is_bzip2(&s));
    }

    fn sample(len: usize, seed: u32) -> Vec<u8> {
        let mut x = seed;
        let mut v = Vec::with_capacity(len);
        while v.len() < len {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            match (x >> 16) % 4 {
                0 => v.extend_from_slice(b"the quick brown fox "),
                1 => v.resize(v.len() + ((x >> 8) % 300) as usize, (x >> 24) as u8),
                2 => v.push((x >> 24) as u8),
                _ => v.extend_from_slice(&x.to_le_bytes()),
            }
        }
        v.truncate(len);
        v
    }

    #[test]
    fn round_trips() {
        for (len, level) in [(1, 1), (2, 9), (3, 9), (4, 9), (5, 9), (255, 9), (1000, 1), (250_000, 1), (300_000, 9)] {
            for seed in [1, 2] {
                let data = sample(len, seed);
                for randomise in [false, true] {
                    let packed = compress_inner(&data, level, randomise);
                    assert_eq!(decompress(&packed).unwrap(), data, "len {len} level {level} randomise {randomise}");
                }
            }
        }
        // Runs of every length around the run-length step's limits.
        for n in [3, 4, 5, 254, 255, 256, 259, 260, 510, 1000] {
            let data = [vec![b'x'], vec![b'a'; n], vec![b'y']].concat();
            assert_eq!(decompress(&compress(&data, 9)).unwrap(), data, "run {n}");
            let data = vec![b'a'; n];
            assert_eq!(decompress(&compress(&data, 9)).unwrap(), data, "bare run {n}");
        }
    }

    #[test]
    fn blocks_split_at_the_level() {
        // 250 kB of noise at level 1 takes three blocks.
        let mut x = 9u32;
        let data: Vec<u8> = (0..250_000)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (x >> 23) as u8
            })
            .collect();
        let packed = compress(&data, 1);
        // Blocks are not byte-aligned: look for the marker at every bit.
        let bit = |i: usize| u64::from(packed[i / 8] >> (7 - i % 8) & 1);
        let mut window = 0u64;
        let mut blocks = 0;
        for i in 0..packed.len() * 8 {
            window = (window << 1 | bit(i)) & 0xffff_ffff_ffff;
            blocks += usize::from(i >= 47 && window == BLOCK_MAGIC);
        }
        assert_eq!(blocks, 3);
        assert_eq!(decompress(&packed).unwrap(), data);
    }

    #[test]
    fn errors() {
        let data = sample(5000, 3);
        let packed = compress(&data, 9);
        assert_eq!(decompress(b"BZh0"), Err(Bzip2Error::NotBzip2));
        assert_eq!(decompress(b"BZ"), Err(Bzip2Error::Truncated));
        assert_eq!(decompress(b"PK\x03\x04"), Err(Bzip2Error::NotBzip2));
        for cut in [4, 10, 20, packed.len() / 2, packed.len() - 1] {
            assert!(decompress(&packed[..cut]).is_err(), "cut at {cut}");
        }
        // Concatenated streams decode to both; anything else after one fails.
        let two = [packed.clone(), compress(b"more", 1)].concat();
        assert_eq!(decompress(&two).unwrap(), [&data[..], b"more"].concat());
        assert_eq!(decompress(&[&packed[..], b"junk"].concat()), Err(Bzip2Error::TrailingData));
        // A flipped bit in the middle is caught, by a check or a CRC.
        for i in [12, packed.len() / 3, packed.len() / 2] {
            let mut bad = packed.clone();
            bad[i] ^= 0x10;
            assert!(decompress(&bad).is_err(), "flip at {i}");
        }
        // The output limit.
        assert_eq!(decompress_with_limit(&packed, 4999), Err(Bzip2Error::OutputLimitExceeded));
        assert_eq!(decompress_with_limit(&packed, 5000).unwrap(), data);
        let zeros = compress(&vec![0u8; 1_000_000], 9);
        assert_eq!(decompress_with_limit(&zeros, 999_999), Err(Bzip2Error::OutputLimitExceeded));
    }

    #[test]
    fn fix_crcs_repairs_every_crc() {
        let data = sample(250_000, 4);
        let good = compress(&data, 1);
        let mut bad = good.clone();
        // Zero the first block's CRC and the stream's (the last 4 bytes
        // hold most of it; the rest is in the byte before).
        bad[10..14].fill(0);
        let n = bad.len();
        bad[n - 4..].fill(0);
        assert!(decompress(&bad).is_err());
        assert_eq!(fix_crcs(&mut bad, usize::MAX), 2);
        assert_eq!(bad, good);
    }

    #[test]
    fn stream_reports_what_it_used() {
        let packed = compress(b"hello", 3);
        let s = read_stream(&[&packed[..], b"rest"].concat(), usize::MAX).unwrap();
        assert_eq!((s.data.as_slice(), s.consumed, s.level), (&b"hello"[..], packed.len(), 3));
    }
}
