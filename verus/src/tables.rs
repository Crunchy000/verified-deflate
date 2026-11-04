//! The decoder's tables (`src/tables.rs`), and the facts about them that
//! the decoder proof uses.

use vstd::prelude::*;

verus! {

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
{
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

} // verus!
