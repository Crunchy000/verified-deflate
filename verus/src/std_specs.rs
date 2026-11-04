//! Specifications of standard-library functions that `vstd` does not
//! specify, and one fact about Rust slices. These are trusted: each states
//! no more than the documented behaviour (none of the functions can panic).

use vstd::prelude::*;

verus! {

/// `x.div_ceil(rhs)` is x / rhs rounded up.
pub assume_specification[ u32::div_ceil ](x: u32, rhs: u32) -> (r: u32)
    requires
        rhs > 0,
    ensures
        r as int == (x as int + rhs as int - 1) / (rhs as int),
;

/// `x.div_ceil(rhs)` is x / rhs rounded up.
pub assume_specification[ usize::div_ceil ](x: usize, rhs: usize) -> (r: usize)
    requires
        rhs > 0,
    ensures
        r as int == (x as int + rhs as int - 1) / (rhs as int),
;

/// A byte slice has at most `isize::MAX` elements: Rust never makes a
/// slice larger than `isize::MAX` bytes (see `std::slice::from_raw_parts`).
pub proof fn axiom_byte_slice_len(s: &[u8])
    ensures
        s@.len() <= isize::MAX,
{
    admit();
}

/// `u32::from(b)` is 0 or 1.
pub assume_specification[ <u32 as core::convert::From<bool>>::from ](b: bool) -> (r: u32)
    ensures
        r <= 1,
;

/// Some 16-bit value.
pub assume_specification[ u16::reverse_bits ](x: u16) -> u16;

/// `fill` keeps the slice's length.
pub assume_specification<T: Clone>[ <[T]>::fill ](s: &mut [T], value: T)
    ensures
        final(s)@.len() == old(s)@.len(),
;

} // verus!
