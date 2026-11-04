//! Specifications of standard-library functions that `vstd` does not
//! specify. These are trusted: each states less than the function's
//! documented behaviour (none of them can panic).

use vstd::prelude::*;

verus! {

/// `x.div_ceil(rhs)` is x / rhs rounded up.
pub assume_specification[ u32::div_ceil ](x: u32, rhs: u32) -> (r: u32)
    requires
        rhs > 0,
    ensures
        r as int == (x as int + rhs as int - 1) / (rhs as int),
;

/// Some 16-bit value.
pub assume_specification[ u16::reverse_bits ](x: u16) -> u16;

/// `fill` keeps the slice's length.
pub assume_specification<T: Clone>[ <[T]>::fill ](s: &mut [T], value: T)
    ensures
        final(s)@.len() == old(s)@.len(),
;

} // verus!
