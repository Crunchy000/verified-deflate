//! The library's `InflateError` (`src/error.rs`).

use vstd::prelude::*;

verus! {

pub enum InflateError {
    UnexpectedEof,
    InvalidBlockType,
    StoredLengthMismatch,
    TooManyCodes,
    InvalidCodeLengthCode,
    RepeatWithoutPrevious,
    CodeLengthOverflow,
    MissingEndOfBlock,
    InvalidLiteralLengthCode,
    InvalidDistanceCode,
    InvalidSymbol,
    DistanceTooFarBack,
    OutputLimitExceeded,
}

} // verus!
