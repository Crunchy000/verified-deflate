//! CRC-32 (ISO-HDLC / IEEE 802.3, reflected polynomial `0xEDB88320`), as
//! used by ZIP, gzip and PNG.
//!
//! The implementation is the classic byte-at-a-time table method. Its
//! table is computed at compile time from a bit-at-a-time reference
//! ([`update_bitwise`]), and both functions are the same fold over the
//! input with different step functions. Kani proves the two step functions
//! equal for every register value and every byte, so the two CRCs agree on
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

const fn make_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = byte_bitwise(0, i as u8);
        i += 1;
    }
    t
}

/// `TABLE[b]` is the register after processing byte `b` from a zero register.
static TABLE: [u32; 256] = make_table();

/// Processes one byte with the table.
#[inline]
fn byte_table(crc: u32, byte: u8) -> u32 {
    TABLE[((crc ^ u32::from(byte)) & 0xff) as usize] ^ (crc >> 8)
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
    fold(crc, data, byte_table)
}

/// Bit-at-a-time reference implementation of [`update`]. Slow; used as the
/// specification in proofs and tests.
pub fn update_bitwise(crc: u32, data: &[u8]) -> u32 {
    fold(crc, data, byte_bitwise)
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

    /// Every table entry equals the bitwise definition.
    #[kani::proof]
    #[kani::unwind(9)]
    fn table_matches_definition() {
        let b: u8 = kani::any();
        assert_eq!(TABLE[b as usize], byte_bitwise(0, b));
    }

    /// The key lemma: for every register value and every byte, the table
    /// step equals eight bitwise steps. `update` and `update_bitwise` are
    /// the same fold over these steps, so they agree on all inputs of every
    /// length.
    #[kani::proof]
    #[kani::unwind(9)]
    fn table_step_equals_bitwise_step() {
        let c: u32 = kani::any();
        let b: u8 = kani::any();
        assert_eq!(byte_table(c, b), byte_bitwise(c, b));
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
