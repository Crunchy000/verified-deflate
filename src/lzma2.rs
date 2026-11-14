//! LZMA2: LZMA in chunks, as `.xz` files and 7z archives use it.
//!
//! An LZMA2 stream is a series of chunks, each starting with a control
//! byte: `0x00` ends the stream; `0x01` and `0x02` start a stored chunk
//! (with and without resetting the dictionary); `0x80`–`0xFF` start an
//! LZMA chunk of up to 2 MiB of output and 64 KiB of input, which may
//! reset the LZMA state, set new `lc`/`lp`/`pb`, or reset the dictionary.
//! The decoder enforces what liblzma enforces: the first chunk resets the
//! dictionary, a dictionary reset is followed by new properties before
//! the next LZMA chunk, `lc + lp` is at most 4, and each LZMA chunk uses
//! exactly its stated input and output.
//!
//! The encoder emits LZMA chunks, falling back to stored chunks where
//! LZMA would not save space.

use crate::lzma::{decode_stream, Decompressed, Encoder, End, LzmaError, LzmaOptions, LzmaProps, Model, RangeEncoder};

/// The most output one LZMA chunk holds.
const CHUNK_UNCOMPRESSED_MAX: usize = 1 << 21;
/// The most input one chunk holds.
const CHUNK_COMPRESSED_MAX: usize = 1 << 16;

/// The dictionary size an LZMA2 property byte (0–40) stands for:
/// `2^(n/2 + 12)` or `3 × 2^(n/2 + 11)`, and 40 for 4 GiB − 1.
pub const fn dict_size_from_byte(b: u8) -> Option<u32> {
    match b {
        0..=39 => Some((2 | (b as u32 & 1)) << (b / 2 + 11)),
        40 => Some(u32::MAX),
        _ => None,
    }
}

/// The smallest property byte whose dictionary is at least `dict_size`.
pub const fn dict_size_byte(dict_size: u32) -> u8 {
    let mut b = 0;
    while b < 40 {
        if let Some(d) = dict_size_from_byte(b) {
            if d >= dict_size {
                return b;
            }
        }
        b += 1;
    }
    40
}

/// Decodes an LZMA2 stream with a dictionary of `dict_size` bytes,
/// producing at most `max_output` bytes, through its end marker.
pub fn decompress_with_limit(input: &[u8], dict_size: u32, max_output: usize) -> Result<Decompressed, LzmaError> {
    let mut out = Vec::new();
    let consumed = decode_into(input, dict_size, &mut out, max_output)?;
    Ok(Decompressed { data: out, consumed })
}

/// [`decompress_with_limit`] onto the end of `out` (whose existing bytes
/// are not part of the dictionary); returns the input used.
pub(crate) fn decode_into(
    input: &[u8],
    dict_size: u32,
    out: &mut Vec<u8>,
    max_output: usize,
) -> Result<usize, LzmaError> {
    let mut pos = 0;
    let mut props = LzmaProps::new(dict_size);
    let mut model: Option<Model> = None;
    let mut need_dict_reset = true;
    let mut need_props = true;
    let mut dict_start = out.len();
    loop {
        let control = *input.get(pos).ok_or(LzmaError::Truncated)?;
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
            let h = input.get(pos..pos + 4).ok_or(LzmaError::Truncated)?;
            let unpacked = (usize::from(control & 0x1F) << 16 | usize::from(h[0]) << 8 | usize::from(h[1])) + 1;
            let packed = (usize::from(h[2]) << 8 | usize::from(h[3])) + 1;
            pos += 4;
            if control >= 0xC0 {
                let b = *input.get(pos).ok_or(LzmaError::Truncated)?;
                pos += 1;
                let p = LzmaProps::from_byte(b, dict_size)?;
                if p.lc + p.lp > 4 {
                    return Err(LzmaError::BadProperties);
                }
                props = p;
                need_props = false;
                model = Some(Model::new(&props));
            } else if need_props {
                return Err(LzmaError::Corrupt("LZMA2 chunk needs new properties"));
            } else if control >= 0xA0 {
                if let Some(m) = model.as_mut() {
                    m.reset(&props);
                }
            }
            let chunk = input.get(pos..pos + packed).ok_or(LzmaError::Truncated)?;
            let m = model.as_mut().ok_or(LzmaError::Corrupt("LZMA2 chunk needs new properties"))?;
            let used = decode_stream(
                m,
                &props,
                chunk,
                End::Known { size: unpacked as u64, allow_marker: false },
                out,
                dict_start,
                max_output,
            )?;
            if used != packed {
                return Err(LzmaError::Corrupt("LZMA2 chunk size does not match its data"));
            }
            pos += packed;
        } else {
            if control > 0x02 {
                return Err(LzmaError::Corrupt("invalid LZMA2 control byte"));
            }
            let h = input.get(pos..pos + 2).ok_or(LzmaError::Truncated)?;
            let n = (usize::from(h[0]) << 8 | usize::from(h[1])) + 1;
            pos += 2;
            let data = input.get(pos..pos + n).ok_or(LzmaError::Truncated)?;
            if n > max_output - out.len() {
                return Err(LzmaError::OutputLimitExceeded);
            }
            out.extend_from_slice(data);
            pos += n;
        }
    }
}

/// Compresses `data` into an LZMA2 stream (end marker included). The
/// dictionary size to record with it is `opts.props.dict_size`.
pub fn compress(data: &[u8], opts: &LzmaOptions) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 3 + 16);
    let mut enc = Encoder::new(data, opts);
    let props = enc.props;
    let mut need_dict_reset = true;
    let mut need_props = true;
    let mut need_state_reset = true;
    while enc.pos < data.len() {
        let start = enc.pos;
        if need_state_reset {
            enc.model.reset(&props);
        }
        let mut rc = RangeEncoder::new();
        // Room for the longest symbol, in output and in input.
        while enc.pos < data.len()
            && enc.pos - start <= CHUNK_UNCOMPRESSED_MAX - crate::lzma::MATCH_LEN_MAX
            && rc.pending() <= CHUNK_COMPRESSED_MAX - 64
        {
            enc.encode_next(&mut rc);
        }
        let packed = rc.finish();
        let unpacked = enc.pos - start;
        if packed.len() >= unpacked {
            // Stored chunks, of up to 64 KiB each. The encoder's state
            // moved on while the decoder's did not, so the next LZMA chunk
            // resets it.
            for (i, piece) in data[start..enc.pos].chunks(CHUNK_COMPRESSED_MAX).enumerate() {
                out.push(if need_dict_reset && i == 0 { 0x01 } else { 0x02 });
                out.extend_from_slice(&((piece.len() - 1) as u16).to_be_bytes());
                out.extend_from_slice(piece);
            }
            need_dict_reset = false;
            need_state_reset = true;
            continue;
        }
        let reset = if need_dict_reset {
            0x60
        } else if need_props {
            0x40
        } else if need_state_reset {
            0x20
        } else {
            0
        };
        out.push(0x80 | reset | ((unpacked - 1) >> 16) as u8);
        out.extend_from_slice(&((unpacked - 1) as u16).to_be_bytes());
        out.extend_from_slice(&((packed.len() - 1) as u16).to_be_bytes());
        if reset >= 0x40 {
            out.push(props.byte());
        }
        out.extend_from_slice(&packed);
        need_dict_reset = false;
        need_props = false;
        need_state_reset = false;
    }
    out.push(0x00);
    out
}

#[cfg(kani)]
mod proofs {
    use super::*;
    use crate::lzma::LzmaProps;

    /// The property byte for a dictionary size is the smallest that covers
    /// it, for every size.
    #[kani::proof]
    #[kani::unwind(42)]
    fn dictionary_byte_is_the_smallest_cover() {
        let d: u32 = kani::any();
        let b = dict_size_byte(d);
        assert!(b <= 40);
        assert!(dict_size_from_byte(b).unwrap() >= d);
        if b > 0 {
            assert!(dict_size_from_byte(b - 1).unwrap() < d);
        }
    }

    /// Dictionary sizes grow with their byte, from 4 KiB, and bytes past
    /// 40 are invalid.
    #[kani::proof]
    fn dictionary_sizes_increase() {
        let b: u8 = kani::any();
        match dict_size_from_byte(b) {
            None => assert!(b > 40),
            Some(d) => {
                assert!(d >= 4096);
                if b < 40 {
                    assert!(dict_size_from_byte(b + 1).unwrap() > d);
                }
            }
        }
    }

    /// On any ten bytes, decoding returns (no panic) within the limit.
    // Every input of every length: verus/src/lzma2.rs.
    #[cfg(any(feature = "kani-extended", kani_sample))] // too large for CI runners
    #[kani::proof]
    #[kani::unwind(28)]
    fn decoding_never_panics() {
        let input: [u8; 10] = kani::any();
        let limit: usize = kani::any_where(|&l: &usize| l <= 6);
        if let Ok(d) = decompress_with_limit(&input, 4096, limit) {
            assert!(d.data.len() <= limit && d.consumed <= input.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LZMA2 round-trips every input of up to two bytes (checked on every
    /// one).
    #[test]
    fn compress_round_trips_every_input_up_to_two_bytes() {
        let opts = LzmaOptions { props: LzmaProps { lc: 0, lp: 0, pb: 0, dict_size: 4096 }, nice_len: 8, depth: 2 };
        let check = |data: &[u8]| {
            let packed = compress(data, &opts);
            let d = decompress_with_limit(&packed, 4096, 16).unwrap();
            assert_eq!(d.data, data);
            assert_eq!(d.consumed, packed.len());
        };
        check(b"");
        for v in 0..=0xFFFFu32 {
            if v < 256 {
                check(&[v as u8]);
            }
            check(&[v as u8, (v >> 8) as u8]);
        }
    }

    fn sample(len: usize, seed: u32) -> Vec<u8> {
        let mut x = seed;
        let mut v = Vec::with_capacity(len);
        while v.len() < len {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            match (x >> 16) % 4 {
                0 => v.extend_from_slice(b"lorem ipsum dolor sit amet "),
                1 => v.resize(v.len() + ((x >> 8) % 100) as usize, b'.'),
                _ => v.extend_from_slice(&x.to_le_bytes()),
            }
        }
        v.truncate(len);
        v
    }

    #[test]
    fn dict_sizes() {
        assert_eq!(dict_size_from_byte(0), Some(4096));
        assert_eq!(dict_size_from_byte(1), Some(6144));
        assert_eq!(dict_size_from_byte(18), Some(2 << 20));
        assert_eq!(dict_size_from_byte(40), Some(u32::MAX));
        assert_eq!(dict_size_from_byte(41), None);
        assert_eq!(dict_size_byte(1 << 23), 22);
        assert_eq!(dict_size_byte((1 << 23) + 1), 23);
        assert_eq!(dict_size_byte(0), 0);
    }

    #[test]
    fn round_trips() {
        let mut x = 5u32;
        let noise: Vec<u8> = (0..300_000)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (x >> 23) as u8
            })
            .collect();
        for data in [
            Vec::new(),
            b"a".to_vec(),
            sample(100_000, 1),
            sample(3_000_000, 2),
            noise.clone(),
            [sample(70_000, 3), noise, sample(70_000, 4)].concat(),
        ] {
            let opts = LzmaOptions::level(6);
            let packed = compress(&data, &opts);
            let d = decompress_with_limit(&packed, opts.props.dict_size, usize::MAX).unwrap();
            assert!(d.data == data, "{} bytes", data.len());
            assert_eq!(d.consumed, packed.len());
        }
    }

    #[test]
    fn chunk_rules() {
        // Empty stream; a stream that does not reset the dictionary first.
        assert_eq!(decompress_with_limit(&[0], 4096, 10).unwrap().data, b"");
        assert!(decompress_with_limit(&[0x02, 0, 0, b'x', 0], 4096, 10).is_err());
        assert_eq!(decompress_with_limit(&[0x01, 0, 1, b'x', b'y', 0], 4096, 10).unwrap().data, b"xy");
        // After a reset by a stored chunk, LZMA needs properties.
        let packed = compress(&sample(1000, 7), &LzmaOptions::level(1));
        assert_eq!(packed[0] & 0xE0, 0xE0);
        let mut bad = vec![0x01, 0, 0, b'x'];
        bad.push(0xA0);
        bad.extend_from_slice(&packed[1..]);
        assert!(decompress_with_limit(&bad, 1 << 20, usize::MAX).is_err());
        assert!(decompress_with_limit(&[0x03, 0], 4096, 10).is_err());
        assert!(decompress_with_limit(&packed[..packed.len() - 1], 1 << 20, usize::MAX).is_err());
        assert_eq!(decompress_with_limit(&packed, 1 << 20, 999), Err(LzmaError::OutputLimitExceeded));
    }
}
