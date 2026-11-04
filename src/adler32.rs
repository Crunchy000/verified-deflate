//! Adler-32 (RFC 1950 §8.2), the checksum of the zlib format.
//!
//! With `a = 1 + the sum of the bytes` and `b = the sum of the values of
//! a after each byte`, both modulo 65521, the checksum is `b << 16 | a`.
//! [`Adler32::update`] defers the modulo for up to 5552 bytes at a time,
//! as zlib does; a compile-time check below shows the sums cannot overflow
//! in that span, and Kani proves the result equal to the byte-by-byte
//! definition.

/// The modulus: the largest prime below 2^16.
const BASE: u32 = 65_521;

/// Bytes processed between reductions (zlib's `NMAX`): the largest `n`
/// for which `b` stays below 2^32 even if every byte is 255.
const NMAX: usize = 5552;

// Starting a span with a, b < BASE, after n bytes of value at most 255:
// a <= BASE - 1 + 255 n, and b <= (n + 1)(BASE - 1) + 255 n (n + 1) / 2.
const _: () = {
    let n = NMAX as u64;
    let base = BASE as u64;
    assert!((base - 1) + 255 * n <= u32::MAX as u64);
    assert!((n + 1) * (base - 1) + 255 * n * (n + 1) / 2 <= u32::MAX as u64);
};

/// Adler-32 of `data`.
pub fn adler32(data: &[u8]) -> u32 {
    let mut h = Adler32::new();
    h.update(data);
    h.finalize()
}

/// Streaming Adler-32: `update` may be called any number of times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Adler32 {
    a: u32,
    b: u32,
}

impl Default for Adler32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Adler32 {
    /// The checksum of no data (1).
    pub const fn new() -> Self {
        Self { a: 1, b: 0 }
    }

    /// Resumes from a checksum returned by [`finalize`](Self::finalize).
    pub const fn from_checksum(sum: u32) -> Self {
        Self { a: (sum & 0xffff) % BASE, b: (sum >> 16) % BASE }
    }

    /// Feeds more data.
    pub fn update(&mut self, data: &[u8]) {
        let (mut a, mut b) = (self.a, self.b);
        for chunk in data.chunks(NMAX) {
            for &x in chunk {
                a += u32::from(x);
                b += a;
            }
            a %= BASE;
            b %= BASE;
        }
        self.a = a;
        self.b = b;
    }

    /// The checksum of everything fed so far.
    pub const fn finalize(&self) -> u32 {
        (self.b << 16) | self.a
    }
}

/// Byte-by-byte reference version of [`Adler32::update`], reducing after
/// every byte; `a` and `b` must be below 65521. Used as the specification
/// in proofs and tests.
#[cfg_attr(not(any(kani, test)), allow(dead_code))]
pub(crate) fn adler32_bytewise(mut a: u32, mut b: u32, data: &[u8]) -> u32 {
    for &x in data {
        a = (a + u32::from(x)) % BASE;
        b = (b + a) % BASE;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_values() {
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(adler32(b"abc"), 0x024D_0127);
    }

    #[test]
    fn long_inputs_match_reference() {
        // Long runs of 0xff stress the deferred reduction across spans.
        let mut data = vec![0xffu8; 3 * NMAX + 17];
        for (i, x) in data.iter_mut().enumerate().skip(NMAX) {
            *x = (i.wrapping_mul(2_654_435_761) >> 13) as u8;
        }
        for len in [0, 1, NMAX - 1, NMAX, NMAX + 1, 2 * NMAX, data.len()] {
            assert_eq!(adler32(&data[..len]), adler32_bytewise(1, 0, &data[..len]), "len {len}");
        }
    }

    #[test]
    fn streaming_and_resuming() {
        let data: Vec<u8> = (0..20_000u32).map(|i| (i * 7 + i / 300) as u8).collect();
        for split in [0, 1, 5551, 5552, 5553, 12_345, 20_000] {
            let mut h = Adler32::new();
            h.update(&data[..split]);
            let mut r = Adler32::from_checksum(h.finalize());
            r.update(&data[split..]);
            assert_eq!(r.finalize(), adler32(&data), "split {split}");
        }
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// From any valid state, the deferred-reduction update equals the
    /// byte-by-byte definition on any 0-6 bytes. (Within a span the sums
    /// cannot overflow, by the compile-time bound above, and reducing once
    /// at the end of the span equals reducing after every byte.)
    #[kani::proof]
    #[kani::unwind(8)]
    fn update_matches_bytewise() {
        let a: u32 = kani::any_where(|&a: &u32| a < BASE);
        let b: u32 = kani::any_where(|&b: &u32| b < BASE);
        let data: [u8; 6] = kani::any();
        let len: usize = kani::any_where(|&l: &usize| l <= 6);
        let mut h = Adler32 { a, b };
        h.update(&data[..len]);
        assert_eq!(h.finalize(), adler32_bytewise(a, b, &data[..len]));
    }

    /// Resuming from a finalized checksum continues the same computation.
    #[kani::proof]
    fn from_checksum_resumes() {
        let a: u32 = kani::any_where(|&a: &u32| a < BASE);
        let b: u32 = kani::any_where(|&b: &u32| b < BASE);
        let h = Adler32 { a, b };
        assert_eq!(Adler32::from_checksum(h.finalize()), h);
    }
}
