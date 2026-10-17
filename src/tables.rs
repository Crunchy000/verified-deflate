//! Constant tables from RFC 1951 §3.2.5 and §3.2.6, plus the
//! value→code lookups the encoder uses.
//!
//! The lookups are generated at compile time by searching the RFC tables,
//! and the Kani harnesses check them against the tables for every possible
//! length (3..=258) and distance (1..=32768).

/// Shortest match DEFLATE can express.
pub const MIN_MATCH: usize = 3;
/// Longest match DEFLATE can express.
pub const MAX_MATCH: usize = 258;
/// Largest match distance DEFLATE can express.
pub const MAX_DISTANCE: usize = 32768;

/// Literal/length symbol that ends a block.
pub const END_OF_BLOCK: usize = 256;
/// Number of literal/length symbols that can appear in a stream (0..=285).
pub const NUM_LITLEN: usize = 286;
/// Number of distance symbols that can appear in a stream (0..=29).
pub const NUM_DIST: usize = 30;
/// Number of code-length symbols (0..=18).
pub const NUM_CODELEN: usize = 19;

/// Base length for length codes 257..=285 (index = code - 257).
pub const LENGTH_BASE: [u16; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
/// Extra bits for length codes 257..=285.
pub const LENGTH_EXTRA: [u8; 29] =
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
/// Base distance for distance codes 0..=29.
pub const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
/// Extra bits for distance codes 0..=29.
pub const DIST_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// Order in which code-length code lengths are transmitted (§3.2.7).
pub const CODELEN_ORDER: [usize; NUM_CODELEN] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// Extra bits following code-length symbol `sym` (§3.2.7): 2 for 16,
/// 3 for 17, 7 for 18, none otherwise.
pub const fn codelen_extra_bits(sym: u8) -> u32 {
    match sym {
        16 => 2,
        17 => 3,
        18 => 7,
        _ => 0,
    }
}

/// `LEN_CODE[len - 3]` is the length-code index (0..=28) for `len`.
static LEN_CODE: [u8; 256] = build_len_code();
/// zlib-style two-level distance lookup: `dist - 1 < 256` indexes directly,
/// larger distances use `256 + ((dist - 1) >> 7)`.
static DIST_CODE: [u8; 512] = build_dist_code();

const fn len_code_by_search(len: usize) -> u8 {
    // 258 has its own code (285); every other length uses the last base ≤ len.
    if len == MAX_MATCH {
        return 28;
    }
    let mut c = 0;
    while c + 1 < 28 && LENGTH_BASE[c + 1] as usize <= len {
        c += 1;
    }
    c as u8
}

const fn dist_code_by_search(dist: usize) -> u8 {
    let mut c = 0;
    while c + 1 < 30 && DIST_BASE[c + 1] as usize <= dist {
        c += 1;
    }
    c as u8
}

const fn build_len_code() -> [u8; 256] {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = len_code_by_search(i + MIN_MATCH);
        i += 1;
    }
    t
}

const fn build_dist_code() -> [u8; 512] {
    let mut t = [0u8; 512];
    let mut i = 0;
    while i < 256 {
        t[i] = dist_code_by_search(i + 1);
        t[256 + i] = dist_code_by_search((i << 7) + 1);
        i += 1;
    }
    t
}

/// Length-code index (0..=28) for a match length in `3..=258`.
///
/// The literal/length symbol is `257 + length_code(len)`.
#[inline]
pub fn length_code(len: usize) -> usize {
    LEN_CODE[len - MIN_MATCH] as usize
}

/// Distance code (0..=29) for a distance in `1..=32768`.
#[inline]
pub fn distance_code(dist: usize) -> usize {
    let d = dist - 1;
    if d < 256 {
        DIST_CODE[d] as usize
    } else {
        DIST_CODE[256 + (d >> 7)] as usize
    }
}

/// Code lengths of the fixed literal/length code (§3.2.6). Symbols 286 and
/// 287 have codes but never occur in valid data.
pub const fn fixed_litlen_lengths() -> [u8; 288] {
    let mut l = [0u8; 288];
    let mut i = 0;
    while i < 288 {
        l[i] = if i < 144 {
            8
        } else if i < 256 {
            9
        } else if i < 280 {
            7
        } else {
            8
        };
        i += 1;
    }
    l
}

/// Code lengths of the fixed distance code: 32 codes of five bits
/// (codes 30 and 31 never occur in valid data).
pub const FIXED_DIST_LENGTHS: [u8; 32] = [5; 32];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_length_maps_into_its_range() {
        for len in MIN_MATCH..=MAX_MATCH {
            let c = length_code(len);
            let base = LENGTH_BASE[c] as usize;
            assert!(base <= len && len - base < (1 << LENGTH_EXTRA[c]), "len {len}");
        }
        assert_eq!(length_code(258), 28);
        assert_eq!(length_code(257), 27);
    }

    #[test]
    fn every_distance_maps_into_its_range() {
        for dist in 1..=MAX_DISTANCE {
            let c = distance_code(dist);
            let base = DIST_BASE[c] as usize;
            assert!(base <= dist && dist - base < (1 << DIST_EXTRA[c]), "dist {dist}");
        }
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// For every length 3..=258 the chosen code covers it, with the extra
    /// bits fitting, and 258 always uses its dedicated code 285.
    #[kani::proof]
    fn length_code_covers_every_length() {
        let len: usize = kani::any();
        kani::assume(len >= MIN_MATCH && len <= MAX_MATCH);
        let c = length_code(len);
        assert!(c < 29);
        let base = LENGTH_BASE[c] as usize;
        assert!(base <= len);
        assert!(len - base < (1usize << LENGTH_EXTRA[c]));
        assert!(len != MAX_MATCH || c == 28);
    }

    /// Decoding any (length code, extra bits) pair gives a length in range
    /// that encodes back to the same code; the only exception is the
    /// RFC-invalid pair (284, 31) which decoders conventionally read as 258.
    #[kani::proof]
    fn length_decode_is_inverse() {
        let c: usize = kani::any();
        let e: usize = kani::any();
        kani::assume(c < 29);
        kani::assume(e < (1usize << LENGTH_EXTRA[c]));
        let len = LENGTH_BASE[c] as usize + e;
        assert!(len >= MIN_MATCH && len <= MAX_MATCH);
        if !(c == 27 && e == 31) {
            assert_eq!(length_code(len), c);
        }
    }

    /// For every distance 1..=32768 the chosen code covers it.
    #[kani::proof]
    fn distance_code_covers_every_distance() {
        let dist: usize = kani::any();
        kani::assume(dist >= 1 && dist <= MAX_DISTANCE);
        let c = distance_code(dist);
        assert!(c < 30);
        let base = DIST_BASE[c] as usize;
        assert!(base <= dist);
        assert!(dist - base < (1usize << DIST_EXTRA[c]));
    }

    /// Decoding any (distance code, extra bits) pair gives a distance in
    /// range that encodes back to the same code.
    #[kani::proof]
    fn distance_decode_is_inverse() {
        let c: usize = kani::any();
        let e: usize = kani::any();
        kani::assume(c < 30);
        kani::assume(e < (1usize << DIST_EXTRA[c]));
        let dist = DIST_BASE[c] as usize + e;
        assert!(dist >= 1 && dist <= MAX_DISTANCE);
        assert_eq!(distance_code(dist), c);
    }

    /// The fixed code lengths match the ranges listed in RFC 1951 §3.2.6.
    #[kani::proof]
    fn fixed_lengths_match_rfc() {
        const L: [u8; 288] = fixed_litlen_lengths();
        let s: usize = kani::any();
        kani::assume(s < 288);
        let expected = match s {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
        assert_eq!(L[s], expected);
    }
}
