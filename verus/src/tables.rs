//! The decoder's tables (`src/tables.rs`), and the facts about them that
//! the decoder proof uses.

use vstd::prelude::*;

verus! {

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

/// Deflate64's base length for length codes 257..=285: DEFLATE's, except
/// code 285, which is base 3 with 16 extra bits.
pub const LENGTH_BASE_64: [u16; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 3];

/// Deflate64's extra bits for length codes 257..=285.
pub const LENGTH_EXTRA_64: [u8; 29] =
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 16];

/// Number of Deflate64 distance symbols (0..=31).
pub const NUM_DIST_64: usize = 32;

/// Base distance for Deflate64's distance codes 0..=31.
pub const DIST_BASE_64: [u16; NUM_DIST_64] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577, 32769, 49153,
];

/// Extra bits for Deflate64's distance codes 0..=31.
pub const DIST_EXTRA_64: [u8; NUM_DIST_64] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14];

/// The bounds of the Deflate64 table entries.
pub proof fn lemma_tables_64()
    ensures
        forall|i: int| 0 <= i < 29 ==> #[trigger] LENGTH_BASE_64@[i] <= 227,
        forall|i: int| 0 <= i < 29 ==> #[trigger] LENGTH_EXTRA_64@[i] <= 16,
        forall|i: int| 0 <= i < 32 ==> 1 <= #[trigger] DIST_BASE_64@[i] <= 49153,
        forall|i: int| 0 <= i < 32 ==> #[trigger] DIST_EXTRA_64@[i] <= 14,
{
    assert forall|i: int| 0 <= i < 29 implies #[trigger] LENGTH_BASE_64@[i] <= 227 && LENGTH_EXTRA_64@[i] <= 16 by {
        assert(LENGTH_BASE_64@ =~= seq![3u16, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 3]);
        assert(LENGTH_EXTRA_64@ =~= seq![0u8, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 16]);
    }
    assert forall|i: int| 0 <= i < 32 implies 1 <= #[trigger] DIST_BASE_64@[i] <= 49153 && DIST_EXTRA_64@[i] <= 14 by {
        assert(DIST_BASE_64@ =~= seq![1u16, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577, 32769, 49153]);
        assert(DIST_EXTRA_64@ =~= seq![0u8, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14]);
    }
}

/// Order in which code-length code lengths are transmitted (§3.2.7).
pub const CODELEN_ORDER: [usize; NUM_CODELEN] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// Code lengths of the fixed distance code.
pub exec const FIXED_DIST_LENGTHS: [u8; 32] = [5; 32];

/// The bounds of the table entries.
pub proof fn lemma_tables()
    ensures
        forall|i: int| 0 <= i < 29 ==> #[trigger] LENGTH_BASE@[i] <= 258,
        forall|i: int| 0 <= i < 29 ==> #[trigger] LENGTH_EXTRA@[i] <= 5,
        forall|i: int| 0 <= i < 30 ==> 1 <= #[trigger] DIST_BASE@[i] <= 24577,
        forall|i: int| 0 <= i < 30 ==> #[trigger] DIST_EXTRA@[i] <= 13,
        forall|i: int| 0 <= i < 19 ==> #[trigger] CODELEN_ORDER@[i] < 19,
        LENGTH_BASE@[0] == 3,
        LENGTH_BASE@[28] == 258,
        DIST_BASE@[0] == 1,
{
    assert(LENGTH_BASE@ =~= seq![3u16, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258]);
    assert(DIST_BASE@ =~= seq![1u16, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577]);
    assert forall|i: int| 0 <= i < 29 implies #[trigger] LENGTH_BASE@[i] <= 258 && LENGTH_EXTRA@[i] <= 5 by {
        assert(LENGTH_BASE@ =~= seq![3u16, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258]);
        assert(LENGTH_EXTRA@ =~= seq![0u8, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0]);
    }
    assert forall|i: int| 0 <= i < 30 implies 1 <= #[trigger] DIST_BASE@[i] <= 24577 && DIST_EXTRA@[i] <= 13 by {
        assert(DIST_BASE@ =~= seq![1u16, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577]);
        assert(DIST_EXTRA@ =~= seq![0u8, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13]);
    }
    assert forall|i: int| 0 <= i < 19 implies #[trigger] CODELEN_ORDER@[i] < 19 by {
        assert(CODELEN_ORDER@ =~= seq![16usize, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15]);
    }
}

/// Extra bits following code-length symbol `sym` (§3.2.7): 2 for 16,
/// 3 for 17, 7 for 18, none otherwise.
pub const fn codelen_extra_bits(sym: u8) -> (r: u32)
    ensures // @ghost
        r <= 7, // @ghost
{
    match sym {
        16 => 2,
        17 => 3,
        18 => 7,
        _ => 0,
    }
}

/// Code lengths of the fixed literal/length code (§3.2.6).
pub const fn fixed_litlen_lengths() -> [u8; 288] {
    let mut l = [0u8; 288];
    let mut i = 0;
    while i < 288
        invariant // @ghost
            i <= 288, // @ghost
        decreases 288 - i, // @ghost
    {
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

/// `LEN_CODE[len - 3]` is the length-code index (0..=28) for `len`.
exec static LEN_CODE: [u8; 256]
    ensures // @ghost
        forall|i: int| // @ghost
            0 <= i < 256 ==> #[trigger] LEN_CODE@[i] < 29 && LENGTH_BASE@[LEN_CODE@[i] as int] <= i + 3, // @ghost
{
    build_len_code()
}

/// zlib-style two-level distance lookup.
exec static DIST_CODE: [u8; 512]
    ensures // @ghost
        forall|i: int| 0 <= i < 512 ==> #[trigger] DIST_CODE@[i] < 30, // @ghost
        forall|i: int| 0 <= i < 256 ==> #[trigger] DIST_BASE@[DIST_CODE@[i] as int] <= i + 1, // @ghost
        forall|i: int| // @ghost
            0 <= i < 256 ==> #[trigger] DIST_BASE@[DIST_CODE@[256 + i] as int] <= i * 128 + 1, // @ghost
{
    build_dist_code()
}

const fn len_code_by_search(len: usize) -> (r: u8)
    requires // @ghost
        len >= 3, // @ghost
    ensures // @ghost
        r < 29, // @ghost
        LENGTH_BASE@[r as int] <= len, // @ghost
{
    proof { // @ghost-begin
        lemma_tables();
    } // @ghost-end
    // 258 has its own code (285); every other length uses the last base ≤ len.
    if len == MAX_MATCH {
        return 28;
    }
    let mut c = 0;
    while c + 1 < 28 && LENGTH_BASE[c + 1] as usize <= len
        invariant // @ghost
            c < 28, // @ghost
            LENGTH_BASE@[c as int] <= len, // @ghost
        decreases 28 - c, // @ghost
    {
        c += 1;
    }
    c as u8
}

const fn dist_code_by_search(dist: usize) -> (r: u8)
    requires // @ghost
        dist >= 1, // @ghost
    ensures // @ghost
        r < 30, // @ghost
        DIST_BASE@[r as int] <= dist, // @ghost
{
    proof { // @ghost-begin
        lemma_tables();
    } // @ghost-end
    let mut c = 0;
    while c + 1 < 30 && DIST_BASE[c + 1] as usize <= dist
        invariant // @ghost
            c < 30, // @ghost
            DIST_BASE@[c as int] <= dist, // @ghost
        decreases 30 - c, // @ghost
    {
        c += 1;
    }
    c as u8
}

const fn build_len_code() -> (r: [u8; 256])
    ensures // @ghost
        forall|i: int| 0 <= i < 256 ==> #[trigger] r@[i] < 29 && LENGTH_BASE@[r@[i] as int] <= i + 3, // @ghost
{
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256
        invariant // @ghost
            i <= 256, // @ghost
            forall|j: int| 0 <= j < 256 ==> #[trigger] t@[j] < 29, // @ghost
            forall|j: int| 0 <= j < i ==> #[trigger] LENGTH_BASE@[t@[j] as int] <= j + 3, // @ghost
        decreases 256 - i, // @ghost
    {
        t[i] = len_code_by_search(i + MIN_MATCH);
        i += 1;
    }
    t
}

const fn build_dist_code() -> (r: [u8; 512])
    ensures // @ghost
        forall|i: int| 0 <= i < 512 ==> #[trigger] r@[i] < 30, // @ghost
        forall|i: int| 0 <= i < 256 ==> #[trigger] DIST_BASE@[r@[i] as int] <= i + 1, // @ghost
        forall|i: int| 0 <= i < 256 ==> #[trigger] DIST_BASE@[r@[256 + i] as int] <= i * 128 + 1, // @ghost
{
    let mut t = [0u8; 512];
    let mut i = 0;
    while i < 256
        invariant // @ghost
            i <= 256, // @ghost
            forall|j: int| 0 <= j < 512 ==> #[trigger] t@[j] < 30, // @ghost
            forall|j: int| 0 <= j < i ==> #[trigger] DIST_BASE@[t@[j] as int] <= j + 1, // @ghost
            forall|j: int| 0 <= j < i ==> #[trigger] DIST_BASE@[t@[256 + j] as int] <= j * 128 + 1, // @ghost
        decreases 256 - i, // @ghost
    {
        proof { // @ghost-begin
            let k = i;
            assert(k << 7 == k * 128) by (bit_vector)
                requires
                    k < 256,
            ;
        } // @ghost-end
        t[i] = dist_code_by_search(i + 1);
        t[256 + i] = dist_code_by_search((i << 7) + 1);
        i += 1;
    }
    t
}

/// Length-code index (0..=28) for a match length in `3..=258`.
#[inline]
pub fn length_code(len: usize) -> (r: usize)
    requires // @ghost
        3 <= len <= 258, // @ghost
    ensures // @ghost
        r < 29, // @ghost
        LENGTH_BASE@[r as int] <= len, // @ghost
{
    LEN_CODE[len - MIN_MATCH] as usize
}

/// Distance code (0..=29) for a distance in `1..=32768`.
#[inline]
pub fn distance_code(dist: usize) -> (r: usize)
    requires // @ghost
        1 <= dist <= 32768, // @ghost
    ensures // @ghost
        r < 30, // @ghost
        DIST_BASE@[r as int] <= dist, // @ghost
{
    let d = dist - 1;
    proof { // @ghost-begin
        assert(d >> 7 <= 255) by (bit_vector)
            requires
                d <= 32767,
        ;
        assert((d >> 7) * 128 <= d) by (bit_vector)
            requires
                d <= 32767,
        ;
    } // @ghost-end
    if d < 256 {
        DIST_CODE[d] as usize
    } else {
        DIST_CODE[256 + (d >> 7)] as usize
    }
}

} // verus!
