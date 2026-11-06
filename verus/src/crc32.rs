//! CRC-32 (`src/crc32.rs`).
//!
//! The executable code below is the library's, token for token (CI checks
//! this with `.github/scripts/verus_drift.py`); every line marked
//! `// @ghost` is specification or proof. The theorem: `crc32`, `update`
//! and the streaming `Crc32` return for every input of every length,
//! without panicking: every table index is in bounds and every loop ends.
//! (That the value is the CRC is Kani's result, in `src/crc32.rs`.)

use vstd::prelude::*;

verus! {

/// Each byte of a 32-bit word indexes a 256-entry table.
proof fn lemma_byte(x: u32)
    ensures
        x & 0xff < 256,
        (x >> 8) & 0xff < 256,
        (x >> 16) & 0xff < 256,
        x >> 24 < 256,
{
    assert(x & 0xff < 256) by (bit_vector);
    assert((x >> 8) & 0xff < 256) by (bit_vector);
    assert((x >> 16) & 0xff < 256) by (bit_vector);
    assert(x >> 24 < 256) by (bit_vector);
}

/// The reflected CRC-32 polynomial.
pub const POLY: u32 = 0xEDB8_8320;

/// Processes one byte bit by bit; `crc` is the raw (non-inverted) register.
const fn byte_bitwise(crc: u32, byte: u8) -> u32 {
    let mut c = crc ^ byte as u32;
    let mut k = 0;
    while k < 8
        invariant // @ghost
            0 <= k <= 8, // @ghost
        decreases 8 - k, // @ghost
    {
        c = if c & 1 != 0 { (c >> 1) ^ POLY } else { c >> 1 };
        k += 1;
    }
    c
}

const fn make_tables() -> [[u32; 256]; 8] {
    let mut t = [[0u32; 256]; 8];
    let mut i = 0;
    while i < 256
        invariant // @ghost
            i <= 256, // @ghost
        decreases 256 - i, // @ghost
    {
        t[0][i] = byte_bitwise(0, i as u8);
        i += 1;
    }
    let mut k = 1;
    while k < 8
        invariant // @ghost
            1 <= k <= 8, // @ghost
        decreases 8 - k, // @ghost
    {
        let mut i = 0;
        while i < 256
            invariant // @ghost
                1 <= k < 8, // @ghost
                i <= 256, // @ghost
            decreases 256 - i, // @ghost
        {
            // One more zero byte after the entry of the previous table.
            let p = t[k - 1][i];
            proof { // @ghost-begin
                lemma_byte(p);
            } // @ghost-end
            t[k][i] = t[0][(p & 0xff) as usize] ^ (p >> 8);
            i += 1;
        }
        k += 1;
    }
    t
}

/// `TABLES[k][b]` is the register after processing byte `b` and then `k`
/// zero bytes, from a zero register. `TABLES[0]` is the classic table.
exec static TABLES: [[u32; 256]; 8] = make_tables();

/// Processes one byte with the table.
#[inline]
fn byte_table(crc: u32, byte: u8) -> u32 {
    proof { // @ghost-begin
        lemma_byte(crc ^ (byte as u32));
    } // @ghost-end
    TABLES[0][((crc ^ u32::from(byte)) & 0xff) as usize] ^ (crc >> 8)
}

/// The four bytes of `x` (lowest first) looked up in `TABLES[k + 3]` down
/// to `TABLES[k]`, combined.
#[inline(always)]
fn lanes(x: u32, k: usize) -> u32
    requires // @ghost
        k <= 4, // @ghost
{
    proof { // @ghost-begin
        lemma_byte(x);
    } // @ghost-end
    TABLES[k + 3][(x & 0xff) as usize]
        ^ TABLES[k + 2][((x >> 8) & 0xff) as usize]
        ^ TABLES[k + 1][((x >> 16) & 0xff) as usize]
        ^ TABLES[k][(x >> 24) as usize]
}

/// The little-endian word in `b[i..i + 4]`.
#[inline(always)]
fn word(b: &[u8; 8], i: usize) -> u32
    requires // @ghost
        i <= 4, // @ghost
{
    u32::from(b[i]) | (u32::from(b[i + 1]) << 8) | (u32::from(b[i + 2]) << 16) | (u32::from(b[i + 3]) << 24)
}

/// Processes eight bytes at once: equal to eight [`byte_table`] steps. The
/// register meets the first four bytes; each byte's contribution is then
/// looked up in the table for the number of bytes that follow it.
#[inline(always)]
fn step8(crc: u32, b: &[u8; 8]) -> u32 {
    let x = crc ^ word(b, 0);
    let y = word(b, 4);
    lanes(x, 4) ^ lanes(y, 0)
}

/// Continues a CRC-32 computation: `update(crc32(a), b) == crc32(a ++ b)`.
pub fn update(crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    let mut rest = data;
    while let Some((chunk, tail)) = rest.split_first_chunk::<8>()
        decreases rest@.len(), // @ghost
    {
        c = step8(c, chunk);
        rest = tail;
    }
    for &b in rest {
        c = byte_table(c, b);
    }
    !c
}

/// CRC-32 of `data`.
pub fn crc32(data: &[u8]) -> u32 {
    update(0, data)
}

/// Streaming CRC-32 hasher.
pub struct Crc32 {
    value: u32,
}

impl Crc32 {
    /// Starts a new checksum.
    pub const fn new() -> Self {
        Self { value: 0 }
    }

    /// Feeds more data.
    pub fn update(&mut self, data: &[u8]) {
        self.value = update(self.value, data);
    }

    /// The checksum of everything fed so far.
    pub const fn finalize(&self) -> u32 {
        self.value
    }
}

} // verus!
