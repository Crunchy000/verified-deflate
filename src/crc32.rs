//! CRC-32 (ISO-HDLC / IEEE 802.3, reflected polynomial `0xEDB88320`), as
//! used by ZIP, gzip and PNG.
//!
//! The implementation is table-driven "slicing-by-8": eight bytes per step
//! from eight tables, then the classic one-table step for the last few
//! bytes. The tables are computed at compile time, bit by bit. Kani
//! proves, for every register value and every input, that one table step
//! equals eight steps of [`update_bitwise`], an independent bit-at-a-time
//! definition, and (by linearity) that one eight-byte step equals eight
//! table steps; so the fast CRC agrees with the bitwise definition on
//! inputs of every length.

/// The reflected CRC-32 polynomial.
pub const POLY: u32 = 0xEDB8_8320;

/// Processes one byte bit by bit; `crc` is the raw (non-inverted) register.
const fn byte_bitwise(crc: u32, byte: u8) -> u32 {
    let mut c = crc ^ byte as u32;
    let mut k = 0;
    while k < 8 {
        c = if c & 1 != 0 { (c >> 1) ^ POLY } else { c >> 1 };
        k += 1;
    }
    c
}

const fn make_tables() -> [[u32; 256]; 8] {
    let mut t = [[0u32; 256]; 8];
    let mut i = 0;
    while i < 256 {
        t[0][i] = byte_bitwise(0, i as u8);
        i += 1;
    }
    let mut k = 1;
    while k < 8 {
        let mut i = 0;
        while i < 256 {
            // One more zero byte after the entry of the previous table.
            let p = t[k - 1][i];
            t[k][i] = t[0][(p & 0xff) as usize] ^ (p >> 8);
            i += 1;
        }
        k += 1;
    }
    t
}

/// `TABLES[k][b]` is the register after processing byte `b` and then `k`
/// zero bytes, from a zero register. `TABLES[0]` is the classic table.
static TABLES: [[u32; 256]; 8] = make_tables();

/// Processes one byte with the table.
#[inline]
fn byte_table(crc: u32, byte: u8) -> u32 {
    TABLES[0][((crc ^ u32::from(byte)) & 0xff) as usize] ^ (crc >> 8)
}

/// The four bytes of `x` (lowest first) looked up in `TABLES[k + 3]` down
/// to `TABLES[k]`, combined.
#[inline(always)]
fn lanes(x: u32, k: usize) -> u32 {
    TABLES[k + 3][(x & 0xff) as usize]
        ^ TABLES[k + 2][((x >> 8) & 0xff) as usize]
        ^ TABLES[k + 1][((x >> 16) & 0xff) as usize]
        ^ TABLES[k][(x >> 24) as usize]
}

/// Processes eight bytes at once: equal to eight [`byte_table`] steps. The
/// register meets the first four bytes; each byte's contribution is then
/// looked up in the table for the number of bytes that follow it.
#[inline(always)]
fn step8(crc: u32, b: &[u8; 8]) -> u32 {
    let x = crc ^ u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let y = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    lanes(x, 4) ^ lanes(y, 0)
}

/// The shared loop: pre- and post-inversion around a per-byte step.
#[inline]
fn fold(crc: u32, data: &[u8], step: impl Fn(u32, u8) -> u32) -> u32 {
    let mut c = !crc;
    for &b in data {
        c = step(c, b);
    }
    !c
}

/// Continues a CRC-32 computation: `update(crc32(a), b) == crc32(a ++ b)`.
pub fn update(crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    let mut rest = data;
    while let Some((chunk, tail)) = rest.split_first_chunk::<8>() {
        c = step8(c, chunk);
        rest = tail;
    }
    // `fold` inverts on entry and exit; undo the entry inversion.
    fold(!c, rest, byte_table)
}

/// Bit-at-a-time definition of [`update`]: the register starts inverted,
/// each byte is XORed in and shifted out bit by bit against the reflected
/// polynomial, and the result is inverted. Written independently of the
/// table-driven code (it shares no function or constant with it), so that
/// a mistake there cannot also hide here. Slow; the specification in
/// proofs and tests.
#[cfg_attr(mutants, mutants::skip)] // specification used by proofs
pub fn update_bitwise(crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            let low = c & 1;
            c >>= 1;
            if low == 1 {
                c ^= 0xEDB8_8320;
            }
        }
    }
    !c
}

/// CRC-32 of `data`.
pub fn crc32(data: &[u8]) -> u32 {
    update(0, data)
}

/// Streaming CRC-32 hasher.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_value() {
        // The standard CRC-32 check value.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"The quick brown fox jumps over the lazy dog"), 0x414F_A339);
    }

    #[test]
    fn eight_byte_step_matches_byte_steps() {
        let mut x = 0x9E37_79B9u32;
        for _ in 0..100_000 {
            x = x.wrapping_mul(0x0101_0101).wrapping_add(0x7F4A_7C15).rotate_left(7);
            let c = x;
            let b = x.wrapping_mul(31).to_le_bytes();
            let b8 = [b[0], b[1], b[2], b[3], b[3], b[2], b[1], b[0] ^ 0x5a];
            let mut r = c;
            for &v in &b8 {
                r = byte_table(r, v);
            }
            assert_eq!(step8(c, &b8), r);
        }
    }

    #[test]
    fn fast_matches_reference_on_many_lengths() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
        for len in 0..data.len() {
            assert_eq!(update(0x1234_5678, &data[..len]), update_bitwise(0x1234_5678, &data[..len]));
        }
    }
}

#[cfg_attr(mutants, mutants::skip)] // proof code: the checker, not code under test
#[cfg(kani)]
mod proofs {
    use super::*;

    /// One raw-register step of the independent definition.
    fn spec_step(c: u32, b: u8) -> u32 {
        !update_bitwise(!c, &[b])
    }

    /// Every table entry equals the bitwise definition.
    #[kani::proof]
    #[kani::unwind(9)]
    fn table_matches_definition() {
        let b: u8 = kani::any();
        assert_eq!(TABLES[0][b as usize], spec_step(0, b));
    }

    /// Each slicing table is the previous one followed by a zero byte.
    #[kani::proof]
    fn slicing_tables_follow_recurrence() {
        let k: usize = kani::any_where(|&k: &usize| k >= 1 && k < 8);
        let b: u8 = kani::any();
        assert_eq!(TABLES[k][b as usize], byte_table(TABLES[k - 1][b as usize], 0));
    }

    // The eight-byte step equals eight table steps, for every register and
    // every eight bytes. Checking that directly is too hard for the SAT
    // solver (eight dependent table lookups against eight independent
    // ones), so it is proved by linearity. Both sides are GF(2)-linear
    // maps of the 96 input bits (register and bytes): eight table steps by
    // `byte_step_is_linear` (a fold of linear steps is linear), and the
    // eight-byte step by `lanes_are_linear` (it is two lane lookups of
    // XORs of its inputs). Linear maps that agree on a basis are equal, and
    // `eight_byte_step_matches_on_a_basis` checks all 96 basis vectors.
    // With `table_step_equals_bitwise_step`, `update` therefore equals
    // `update_bitwise` on inputs of every length.

    /// One table step is linear in the register and the byte together.
    #[kani::proof]
    fn byte_step_is_linear() {
        let (r, s): (u32, u32) = (kani::any(), kani::any());
        let (a, b): (u8, u8) = (kani::any(), kani::any());
        assert_eq!(byte_table(r ^ s, a ^ b), byte_table(r, a) ^ byte_table(s, b));
    }

    /// Each four-lane lookup is linear.
    #[kani::proof]
    fn lanes_are_linear() {
        let k: usize = kani::any_where(|&k: &usize| k == 0 || k == 4);
        let (x, y): (u32, u32) = (kani::any(), kani::any());
        assert_eq!(lanes(x ^ y, k), lanes(x, k) ^ lanes(y, k));
    }

    /// The eight-byte step and eight table steps agree on every basis
    /// vector: each single set bit of the register or of the eight bytes.
    #[kani::proof]
    #[kani::unwind(9)]
    fn eight_byte_step_matches_on_a_basis() {
        let bit: usize = kani::any_where(|&i: &usize| i < 96);
        let mut c = 0u32;
        let mut b = [0u8; 8];
        if bit < 32 {
            c = 1 << bit;
        } else {
            b[(bit - 32) / 8] = 1 << ((bit - 32) % 8);
        }
        let mut r = c;
        for &x in &b {
            r = byte_table(r, x);
        }
        assert_eq!(step8(c, &b), r);
    }

    /// The direct statement, for every register and every eight bytes
    /// (proved above by linearity; too slow for CI as a single query).
    #[cfg(any(feature = "kani-extended", kani_sample))]
    #[kani::proof]
    #[kani::unwind(9)]
    fn eight_byte_step_equals_table_steps() {
        let c: u32 = kani::any();
        let b: [u8; 8] = kani::any();
        let mut r = c;
        for &x in &b {
            r = byte_table(r, x);
        }
        assert_eq!(step8(c, &b), r);
    }

    /// Four table steps from any register over any four bytes equal the
    /// four-lane lookup of their XOR (also too slow for CI as one query).
    #[cfg(any(feature = "kani-extended", kani_sample))]
    #[kani::proof]
    #[kani::unwind(5)]
    fn four_byte_lanes_equal_table_steps() {
        let c: u32 = kani::any();
        let b: [u8; 4] = kani::any();
        let mut r = c;
        for &x in &b {
            r = byte_table(r, x);
        }
        assert_eq!(lanes(c ^ u32::from_le_bytes(b), 0), r);
    }

    /// The key lemma: for every register value and every byte, the table
    /// step equals eight bitwise steps of the independent definition.
    /// `update` applies such steps between the same two inversions as
    /// `update_bitwise`, so they agree on all inputs of every length.
    #[kani::proof]
    #[kani::unwind(9)]
    fn table_step_equals_bitwise_step() {
        let c: u32 = kani::any();
        let b: u8 = kani::any();
        assert_eq!(byte_table(c, b), spec_step(c, b));
    }

    /// Direct check of whole computations on up to 4 bytes from any
    /// starting value.
    #[kani::proof]
    #[kani::unwind(9)]
    fn update_matches_reference() {
        let init: u32 = kani::any();
        let data: [u8; 4] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= 4);
        assert_eq!(update(init, &data[..len]), update_bitwise(init, &data[..len]));
    }

    /// Streaming in two pieces equals hashing the concatenation.
    #[kani::proof]
    #[kani::unwind(9)]
    fn streaming_is_concatenation() {
        let data: [u8; 4] = kani::any();
        let split: usize = kani::any();
        kani::assume(split <= 4);
        let mut h = Crc32::new();
        h.update(&data[..split]);
        h.update(&data[split..]);
        assert_eq!(h.finalize(), update(0, &data));
    }
}
