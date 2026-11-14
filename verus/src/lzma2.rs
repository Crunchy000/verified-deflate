//! The LZMA2 decoder (`src/lzma2.rs`): `decompress_with_limit` and
//! `decode_into`, which run the LZMA decoder (`lzma.rs`) chunk by chunk.
//!
//! The executable code below is the library's, token for token (CI checks
//! this with `.github/scripts/verus_drift.py`); every line marked
//! `// @ghost` is specification or proof. The theorem: on every input of
//! every length, every dictionary size and every output limit, the
//! decoder returns, without panicking, and its output stays within the
//! limit. Between chunks the model stays fit for decoding: it is replaced
//! whenever the dictionary is reset, and otherwise the bytes it may refer
//! back to are still there.

use crate::lzma::{decode_stream, lemma_reps_ok_grow, no_marker, Decompressed, End, LzmaError, LzmaProps, Model};
use vstd::prelude::*;

verus! {

pub fn decompress_with_limit(input: &[u8], dict_size: u32, max_output: usize) -> (r: Result<Decompressed, LzmaError>)
    ensures // @ghost
        r is Ok ==> r->Ok_0.data@.len() <= max_output, // @ghost
        r is Ok ==> r->Ok_0.consumed <= input@.len(), // @ghost
{
    let mut out = Vec::new();
    let consumed = decode_into(input, dict_size, &mut out, max_output)?;
    Ok(Decompressed { data: out, consumed })
}

#[verifier::loop_isolation(false)] // @ghost
#[verifier::spinoff_prover] // @ghost
pub(crate) fn decode_into(
    input: &[u8],
    dict_size: u32,
    out: &mut Vec<u8>,
    max_output: usize,
) -> (r: Result<usize, LzmaError>)
    requires // @ghost
        old(out)@.len() <= max_output, // @ghost
    ensures // @ghost
        old(out)@.len() <= final(out)@.len() <= max_output, // @ghost
        r is Ok ==> r->Ok_0 <= input@.len(), // @ghost
{
    let mut pos = 0;
    let mut props = LzmaProps::new(dict_size);
    // Valid once a chunk has given properties (`need_props` is false).
    let mut model = Model::new(&props);
    let mut need_dict_reset = true;
    let mut need_props = true;
    let mut dict_start = out.len();
    loop
        invariant // @ghost
            pos <= input@.len(), // @ghost
            props.valid(), // @ghost
            model.wf(), // @ghost
            old(out)@.len() <= out@.len() <= max_output, // @ghost
            dict_start <= out@.len(), // @ghost
            !need_props ==> model.reps_ok(out@.len() - dict_start), // @ghost
        decreases input@.len() - pos, // @ghost
    {
        if pos >= input.len() {
            return Err(LzmaError::Truncated);
        }
        let control = input[pos];
        pos += 1;
        if control == 0x00 {
            return Ok(pos);
        }
        if control >= 0xE0 || control == 0x01 {
            need_props = true;
            need_dict_reset = false;
            dict_start = out.len();
        } else if need_dict_reset {
            return Err(LzmaError::Corrupt("LZMA2 stream does not start with a dictionary reset"));
        }
        if control >= 0x80 {
            if input.len() - pos < 4 {
                return Err(LzmaError::Truncated);
            }
            let unpacked =
                (((control & 0x1F) as usize) << 16 | (input[pos] as usize) << 8 | input[pos + 1] as usize) + 1;
            let packed = ((input[pos + 2] as usize) << 8 | input[pos + 3] as usize) + 1;
            pos += 4;
            if control >= 0xC0 {
                if pos >= input.len() {
                    return Err(LzmaError::Truncated);
                }
                let b = input[pos];
                pos += 1;
                let p = LzmaProps::from_byte(b, dict_size)?;
                if p.lc + p.lp > 4 {
                    return Err(LzmaError::BadProperties);
                }
                props = p;
                need_props = false;
                model = Model::new(&props);
            } else if need_props {
                return Err(LzmaError::Corrupt("LZMA2 chunk needs new properties"));
            } else if control >= 0xA0 {
                model.reset(&props);
            }
            if input.len() - pos < packed {
                return Err(LzmaError::Truncated);
            }
            proof { // @ghost-begin
                if control >= 0xA0 {
                    lemma_reps_ok_grow(&model, 0, out@.len() - dict_start);
                }
            } // @ghost-end
            let used = decode_stream(
                &mut model,
                &props,
                &input[pos..pos + packed],
                End::Known { size: unpacked as u64, allow_marker: false },
                out,
                dict_start,
                max_output,
            )?;
            proof { // @ghost-begin
                assert(no_marker(End::Known { size: unpacked as u64, allow_marker: false }));
            } // @ghost-end
            if used != packed {
                return Err(LzmaError::Corrupt("LZMA2 chunk size does not match its data"));
            }
            pos += packed;
        } else {
            if control > 0x02 {
                return Err(LzmaError::Corrupt("invalid LZMA2 control byte"));
            }
            if input.len() - pos < 2 {
                return Err(LzmaError::Truncated);
            }
            let n = ((input[pos] as usize) << 8 | input[pos + 1] as usize) + 1;
            pos += 2;
            if input.len() - pos < n {
                return Err(LzmaError::Truncated);
            }
            if n > max_output - out.len() {
                return Err(LzmaError::OutputLimitExceeded);
            }
            let ghost before = out@.len() - dict_start; // @ghost
            out.extend_from_slice(&input[pos..pos + n]);
            proof { // @ghost-begin
                if !need_props {
                    lemma_reps_ok_grow(&model, before, out@.len() - dict_start);
                }
            } // @ghost-end
            pos += n;
        }
    }
}

} // verus!
