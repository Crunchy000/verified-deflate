//! CRC-32 (ISO-HDLC / IEEE 802.3, reflected polynomial `0xEDB88320`), as
//! used by ZIP, gzip and PNG.
//!
//! The fast path processes eight bytes per step using eight 256-entry
//! tables ("slicing-by-8"). All tables are computed at compile time from a
//! bit-at-a-time reference ([`update_bitwise`]), and the Kani harnesses
//! prove that the table-driven code computes exactly what the reference
//! computes.

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
            let prev = t[k - 1][i];
            t[k][i] = (prev >> 8) ^ t[0][(prev & 0xff) as usize];
            i += 1;
        }
        k += 1;
    }
    t
}

/// `TABLES[k][b]` is the CRC register after processing byte `b` followed by
/// `k` zero bytes, starting from a zero register.
static TABLES: [[u32; 256]; 8] = make_tables();

/// Continues a CRC-32 computation: `update(crc32(a), b) == crc32(a ++ b)`.
pub fn update(crc: u32, data: &[u8]) -> u32 {
    let t = &TABLES;
    let mut c = !crc;
    let mut chunks = data.chunks_exact(8);
    for ch in &mut chunks {
        let lo = c ^ u32::from_le_bytes([ch[0], ch[1], ch[2], ch[3]]);
        let hi = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]);
        c = t[7][(lo & 0xff) as usize]
            ^ t[6][((lo >> 8) & 0xff) as usize]
            ^ t[5][((lo >> 16) & 0xff) as usize]
            ^ t[4][(lo >> 24) as usize]
            ^ t[3][(hi & 0xff) as usize]
            ^ t[2][((hi >> 8) & 0xff) as usize]
            ^ t[1][((hi >> 16) & 0xff) as usize]
            ^ t[0][(hi >> 24) as usize];
    }
    for &b in chunks.remainder() {
        c = t[0][((c ^ u32::from(b)) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

/// Bit-at-a-time reference implementation of [`update`]. Slow; used as the
/// specification in proofs and tests.
pub fn update_bitwise(crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    for &b in data {
        c = byte_bitwise(c, b);
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
    fn fast_matches_reference_on_many_lengths() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
        for len in 0..data.len() {
            assert_eq!(update(0x1234_5678, &data[..len]), update_bitwise(0x1234_5678, &data[..len]));
        }
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Every entry of the base table equals the bitwise definition.
    #[kani::proof]
    #[kani::unwind(9)]
    fn base_table_matches_definition() {
        let b: u8 = kani::any();
        assert_eq!(TABLES[0][b as usize], byte_bitwise(0, b));
    }

    /// Every entry of the higher tables means "byte then k zero bytes".
    #[kani::proof]
    #[kani::unwind(9)]
    fn slice_tables_match_definition() {
        let b: u8 = kani::any();
        let k: usize = kani::any();
        kani::assume(k >= 1 && k < 8);
        let mut c = byte_bitwise(0, b);
        let mut j = 0;
        while j < 7 {
            if j < k {
                c = byte_bitwise(c, 0);
            }
            j += 1;
        }
        assert_eq!(TABLES[k][b as usize], c);
    }

    /// Eight arbitrary bytes (one slicing-by-8 step) from an arbitrary
    /// starting CRC give the same result as the bitwise reference.
    #[kani::proof]
    #[kani::unwind(9)]
    fn slicing_step_matches_reference() {
        let init: u32 = kani::any();
        let data: [u8; 8] = kani::any();
        assert_eq!(update(init, &data), update_bitwise(init, &data));
    }

    /// The byte-at-a-time tail path matches the reference too.
    #[kani::proof]
    #[kani::unwind(9)]
    fn tail_matches_reference() {
        let init: u32 = kani::any();
        let data: [u8; 7] = kani::any();
        let len: usize = kani::any();
        kani::assume(len <= 7);
        assert_eq!(update(init, &data[..len]), update_bitwise(init, &data[..len]));
    }

    /// Streaming in two pieces equals hashing the concatenation.
    #[kani::proof]
    #[kani::unwind(9)]
    fn streaming_is_concatenation() {
        let data: [u8; 6] = kani::any();
        let split: usize = kani::any();
        kani::assume(split <= 6);
        let mut h = Crc32::new();
        h.update(&data[..split]);
        h.update(&data[split..]);
        assert_eq!(h.finalize(), update_bitwise(0, &data));
    }
}
