//! Error type shared by the decoder and the bit reader.

use core::fmt;

/// Why a DEFLATE stream could not be decoded.
///
/// Every malformed input maps to one of these variants; the decoder never
/// panics on bad data (this is one of the properties checked with Kani).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InflateError {
    /// The input ended before the final block was complete.
    UnexpectedEof,
    /// A block header used the reserved block type 3.
    InvalidBlockType,
    /// A stored block's LEN and NLEN fields were not one's complements.
    StoredLengthMismatch,
    /// A dynamic block declared more than 286 literal/length or 30 distance codes.
    TooManyCodes,
    /// The code-length code of a dynamic block was over-subscribed or incomplete.
    InvalidCodeLengthCode,
    /// A repeat-previous code (16) appeared before any code length.
    RepeatWithoutPrevious,
    /// Code-length repeats ran past the number of declared codes.
    CodeLengthOverflow,
    /// A dynamic block gave the end-of-block symbol (256) no code.
    MissingEndOfBlock,
    /// The literal/length code lengths were over-subscribed or incomplete.
    InvalidLiteralLengthCode,
    /// The distance code lengths were over-subscribed or incomplete.
    InvalidDistanceCode,
    /// A bit sequence did not decode to a valid symbol.
    InvalidSymbol,
    /// A match referred to data before the start of the output.
    DistanceTooFarBack,
    /// The output would have exceeded the caller's limit.
    OutputLimitExceeded,
}

impl fmt::Display for InflateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            Self::UnexpectedEof => "unexpected end of compressed data",
            Self::InvalidBlockType => "invalid block type",
            Self::StoredLengthMismatch => "stored block length check failed",
            Self::TooManyCodes => "too many literal/length or distance codes",
            Self::InvalidCodeLengthCode => "invalid code-length code",
            Self::RepeatWithoutPrevious => "code-length repeat with no previous length",
            Self::CodeLengthOverflow => "code-length repeat past the end of the code list",
            Self::MissingEndOfBlock => "no code for the end-of-block symbol",
            Self::InvalidLiteralLengthCode => "invalid literal/length code lengths",
            Self::InvalidDistanceCode => "invalid distance code lengths",
            Self::InvalidSymbol => "invalid Huffman code in compressed data",
            Self::DistanceTooFarBack => "match distance reaches before start of output",
            Self::OutputLimitExceeded => "decompressed size exceeds the limit",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for InflateError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_distinct() {
        use InflateError::*;
        let all = [
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
        ];
        let mut seen = std::collections::HashSet::new();
        for e in all {
            let m = e.to_string();
            assert!(!m.is_empty() && seen.insert(m), "{e:?}");
        }
        assert_eq!(UnexpectedEof.to_string(), "unexpected end of compressed data");
    }
}
