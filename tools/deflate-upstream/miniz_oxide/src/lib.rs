//! Stand-in for the miniz_oxide functions the `deflate` crate's tests use,
//! implemented with verified-deflate. Decompression checks the zlib header
//! and Adler-32 trailer; compression (used only for a ratio comparison) is
//! verified-deflate at the given level.
pub mod inflate {
    #[derive(Debug)]
    pub struct Error(pub String);
    pub fn decompress_to_vec(input: &[u8]) -> Result<Vec<u8>, Error> {
        verified_deflate::decompress(input).map_err(|e| Error(format!("{e:?}")))
    }
    pub fn decompress_to_vec_zlib(input: &[u8]) -> Result<Vec<u8>, Error> {
        if input.len() < 6 || (u16::from(input[0]) << 8 | u16::from(input[1])) % 31 != 0 || input[0] & 0x0f != 8 {
            return Err(Error("bad zlib header".into()));
        }
        let d = verified_deflate::decompress_with_limit(&input[2..], usize::MAX).map_err(|e| Error(format!("{e:?}")))?;
        let t = &input[2 + d.consumed..];
        if t.len() != 4 || u32::from_be_bytes([t[0], t[1], t[2], t[3]]) != verified_deflate::adler32(&d.data) {
            return Err(Error("bad zlib trailer".into()));
        }
        Ok(d.data)
    }
}
pub mod deflate {
    pub fn compress_to_vec_zlib(input: &[u8], level: u8) -> Vec<u8> {
        let mut out = vec![0x78, 0xda];
        out.extend(verified_deflate::compress(input, level.min(9)));
        out.extend(verified_deflate::adler32(input).to_be_bytes());
        out
    }
}
