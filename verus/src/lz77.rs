//! The decoder's match copy (`copy_within_buffer` in `src/lz77.rs`, with
//! `word8` and `word4`): never out of bounds for any distance of at least 1
//! that reaches back no further than the data written, and any length that
//! fits the buffer.

use vstd::prelude::*;

verus! {

#[inline]
pub fn copy_within_buffer(buf: &mut [u8], pos: usize, dist: usize, len: usize)
    requires // @ghost
        1 <= dist <= pos, // @ghost
        pos + len <= old(buf)@.len(), // @ghost
    ensures // @ghost
        final(buf)@.len() == old(buf)@.len(), // @ghost
{
    let src = pos - dist;
    if dist == 1 {
        let b = buf[src];
        buf[pos..pos + len].fill(b);
        return;
    }
    if len <= 16 && dist >= len {
        // A short copy whose source lies wholly before its destination:
        // two (possibly overlapping) words, both loaded before either is
        // stored, cover it exactly without a call to memmove.
        if len >= 8 {
            let a = word8(buf, src);
            let b = word8(buf, src + len - 8);
            buf[pos..pos + 8].copy_from_slice(&a);
            buf[pos + len - 8..pos + len].copy_from_slice(&b);
        } else if len >= 4 {
            let a = word4(buf, src);
            let b = word4(buf, src + len - 4);
            buf[pos..pos + 4].copy_from_slice(&a);
            buf[pos + len - 4..pos + len].copy_from_slice(&b);
        } else {
            for i in 0..len
                invariant // @ghost
                    buf@.len() == old(buf)@.len(), // @ghost
                    src + len <= pos, // @ghost
                    pos + len <= buf@.len(), // @ghost
            {
                buf[pos + i] = buf[src + i];
            }
        }
        return;
    }
    let mut copied = 0;
    while copied < len
        invariant // @ghost
            buf@.len() == old(buf)@.len(), // @ghost
            copied <= len, // @ghost
            src + dist == pos, // @ghost
            1 <= dist, // @ghost
            pos + len <= buf@.len(), // @ghost
        decreases len - copied, // @ghost
    {
        let avail = dist + copied;
        let chunk = if len - copied < avail { len - copied } else { avail };
        buf.copy_within(src..src + chunk, pos + copied);
        copied += chunk;
    }
}

/// The 8 bytes at `buf[i..]`.
#[inline(always)]
fn word8(buf: &[u8], i: usize) -> [u8; 8]
    requires // @ghost
        i + 8 <= buf@.len(), // @ghost
{
    let w = &buf[i..i + 8];
    [w[0], w[1], w[2], w[3], w[4], w[5], w[6], w[7]]
}

/// The 4 bytes at `buf[i..]`.
#[inline(always)]
fn word4(buf: &[u8], i: usize) -> [u8; 4]
    requires // @ghost
        i + 4 <= buf@.len(), // @ghost
{
    let w = &buf[i..i + 4];
    [w[0], w[1], w[2], w[3]]
}

} // verus!
