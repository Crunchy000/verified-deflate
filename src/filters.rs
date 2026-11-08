//! Filters that make executables and tables compress better: the branch
//! (BCJ) filters, which turn relative call and jump targets into absolute
//! ones, and the delta filter. `.xz` and 7z archives chain them before
//! LZMA or LZMA2.
//!
//! Each branch filter is the one in XZ Utils (liblzma) and 7-Zip, bit for
//! bit: x86, PowerPC, IA-64, ARM, ARM-Thumb, SPARC, ARM64 and RISC-V.
//! Applied to a whole buffer they give what the streaming versions give;
//! a tail too short to hold an instruction is left as it is.

/// A branch-conversion filter, by the instruction set it understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BranchFilter {
    /// x86 and x86-64 (`E8` calls and `E9` jumps).
    X86,
    /// Big-endian PowerPC.
    PowerPc,
    /// Itanium.
    Ia64,
    /// 32-bit little-endian ARM.
    Arm,
    /// ARM Thumb.
    ArmThumb,
    /// SPARC.
    Sparc,
    /// 64-bit ARM.
    Arm64,
    /// RISC-V (decoding only).
    RiscV,
}

impl BranchFilter {
    /// The filter's ID in `.xz` files.
    pub const fn xz_id(self) -> u64 {
        match self {
            Self::X86 => 0x04,
            Self::PowerPc => 0x05,
            Self::Ia64 => 0x06,
            Self::Arm => 0x07,
            Self::ArmThumb => 0x08,
            Self::Sparc => 0x09,
            Self::Arm64 => 0x0A,
            Self::RiscV => 0x0B,
        }
    }

    /// The filter with this `.xz` ID.
    pub const fn from_xz_id(id: u64) -> Option<Self> {
        Some(match id {
            0x04 => Self::X86,
            0x05 => Self::PowerPc,
            0x06 => Self::Ia64,
            0x07 => Self::Arm,
            0x08 => Self::ArmThumb,
            0x09 => Self::Sparc,
            0x0A => Self::Arm64,
            0x0B => Self::RiscV,
            _ => return None,
        })
    }

    /// What a start offset must be a multiple of (liblzma refuses others):
    /// the instruction size, 16 for IA-64's bundles.
    pub const fn alignment(self) -> u32 {
        match self {
            Self::X86 => 1,
            Self::ArmThumb | Self::RiscV => 2,
            Self::PowerPc | Self::Arm | Self::Sparc | Self::Arm64 => 4,
            Self::Ia64 => 16,
        }
    }

    /// Undoes the filter on `buf`, which starts at offset `start` of the
    /// filtered data.
    pub fn decode(self, buf: &mut [u8], start: u32) {
        self.code(buf, start, false);
    }

    /// Applies the filter to `buf`, which starts at offset `start`.
    /// Returns `false`, leaving `buf` alone, for RISC-V, whose encoder is
    /// not implemented.
    pub fn encode(self, buf: &mut [u8], start: u32) -> bool {
        if self == Self::RiscV {
            return false;
        }
        self.code(buf, start, true);
        true
    }

    fn code(self, buf: &mut [u8], start: u32, enc: bool) {
        match self {
            Self::X86 => x86(buf, start, enc),
            Self::PowerPc => powerpc(buf, start, enc),
            Self::Ia64 => ia64(buf, start, enc),
            Self::Arm => arm(buf, start, enc),
            Self::ArmThumb => armthumb(buf, start, enc),
            Self::Sparc => sparc(buf, start, enc),
            Self::Arm64 => arm64(buf, start, enc),
            Self::RiscV => riscv_decode(buf, start),
        }
    }
}

const fn test86(b: u8) -> bool {
    b == 0 || b == 0xFF
}

fn x86(buf: &mut [u8], now: u32, enc: bool) {
    const MASK_TO_BIT_NUMBER: [u32; 5] = [0, 1, 2, 2, 3];
    if buf.len() < 5 {
        return;
    }
    let mut prev_mask = 0u32;
    let mut prev_pos = now.wrapping_sub(5);
    let limit = buf.len() - 5;
    let mut i = 0;
    while i <= limit {
        let b = buf[i];
        if b != 0xE8 && b != 0xE9 {
            i += 1;
            continue;
        }
        let pos = now.wrapping_add(i as u32);
        let offset = pos.wrapping_sub(prev_pos);
        prev_pos = pos;
        if offset > 5 {
            prev_mask = 0;
        } else {
            for _ in 0..offset {
                prev_mask &= 0x77;
                prev_mask <<= 1;
            }
        }
        let b4 = buf[i + 4];
        if test86(b4) && (prev_mask >> 1) <= 4 && (prev_mask >> 1) != 3 {
            let mut src = u32::from_le_bytes([buf[i + 1], buf[i + 2], buf[i + 3], b4]);
            let mut dest;
            loop {
                let here = pos.wrapping_add(5);
                dest = if enc { src.wrapping_add(here) } else { src.wrapping_sub(here) };
                if prev_mask == 0 {
                    break;
                }
                let j = MASK_TO_BIT_NUMBER[(prev_mask >> 1) as usize];
                if !test86((dest >> (24 - j * 8)) as u8) {
                    break;
                }
                src = dest ^ ((1u32 << (32 - j * 8)) - 1);
            }
            buf[i + 4] = !(((dest >> 24) & 1).wrapping_sub(1)) as u8;
            buf[i + 3] = (dest >> 16) as u8;
            buf[i + 2] = (dest >> 8) as u8;
            buf[i + 1] = dest as u8;
            i += 5;
            prev_mask = 0;
        } else {
            i += 1;
            prev_mask |= 1;
            if test86(b4) {
                prev_mask |= 0x10;
            }
        }
    }
}

fn arm(buf: &mut [u8], now: u32, enc: bool) {
    let n = buf.len() & !3;
    let mut i = 0;
    while i < n {
        if buf[i + 3] == 0xEB {
            let src = (u32::from(buf[i + 2]) << 16 | u32::from(buf[i + 1]) << 8 | u32::from(buf[i])) << 2;
            let here = now.wrapping_add(i as u32).wrapping_add(8);
            let dest = if enc { here.wrapping_add(src) } else { src.wrapping_sub(here) } >> 2;
            buf[i + 2] = (dest >> 16) as u8;
            buf[i + 1] = (dest >> 8) as u8;
            buf[i] = dest as u8;
        }
        i += 4;
    }
}

fn armthumb(buf: &mut [u8], now: u32, enc: bool) {
    if buf.len() < 4 {
        return;
    }
    let size = buf.len() - 4;
    let mut i = 0;
    while i <= size {
        if (buf[i + 1] & 0xF8) == 0xF0 && (buf[i + 3] & 0xF8) == 0xF8 {
            let src = ((u32::from(buf[i + 1]) & 7) << 19
                | u32::from(buf[i]) << 11
                | (u32::from(buf[i + 3]) & 7) << 8
                | u32::from(buf[i + 2]))
                << 1;
            let here = now.wrapping_add(i as u32).wrapping_add(4);
            let dest = if enc { here.wrapping_add(src) } else { src.wrapping_sub(here) } >> 1;
            buf[i + 1] = 0xF0 | ((dest >> 19) & 7) as u8;
            buf[i] = (dest >> 11) as u8;
            buf[i + 3] = 0xF8 | ((dest >> 8) & 7) as u8;
            buf[i + 2] = dest as u8;
            i += 2;
        }
        i += 2;
    }
}

fn powerpc(buf: &mut [u8], now: u32, enc: bool) {
    let n = buf.len() & !3;
    let mut i = 0;
    while i < n {
        if (buf[i] >> 2) == 0x12 && (buf[i + 3] & 3) == 1 {
            let src = (u32::from(buf[i]) & 3) << 24
                | u32::from(buf[i + 1]) << 16
                | u32::from(buf[i + 2]) << 8
                | (u32::from(buf[i + 3]) & !3);
            let here = now.wrapping_add(i as u32);
            let dest = if enc { here.wrapping_add(src) } else { src.wrapping_sub(here) };
            buf[i] = 0x48 | ((dest >> 24) & 3) as u8;
            buf[i + 1] = (dest >> 16) as u8;
            buf[i + 2] = (dest >> 8) as u8;
            buf[i + 3] = (buf[i + 3] & 3) | dest as u8;
        }
        i += 4;
    }
}

fn sparc(buf: &mut [u8], now: u32, enc: bool) {
    let n = buf.len() & !3;
    let mut i = 0;
    while i < n {
        if (buf[i] == 0x40 && (buf[i + 1] & 0xC0) == 0) || (buf[i] == 0x7F && (buf[i + 1] & 0xC0) == 0xC0) {
            let src = u32::from_be_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]) << 2;
            let here = now.wrapping_add(i as u32);
            let mut dest = if enc { here.wrapping_add(src) } else { src.wrapping_sub(here) } >> 2;
            dest = ((0u32.wrapping_sub((dest >> 22) & 1) << 22) & 0x3FFF_FFFF) | (dest & 0x3F_FFFF) | 0x4000_0000;
            buf[i..i + 4].copy_from_slice(&dest.to_be_bytes());
        }
        i += 4;
    }
}

fn ia64(buf: &mut [u8], now: u32, enc: bool) {
    const BRANCH_TABLE: [u32; 32] =
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 6, 6, 0, 0, 7, 7, 4, 4, 0, 0, 4, 4, 0, 0];
    let n = buf.len() & !15;
    let mut i = 0;
    while i < n {
        let mask = BRANCH_TABLE[usize::from(buf[i] & 0x1F)];
        let mut bit_pos = 5u32;
        for slot in 0..3 {
            if (mask >> slot) & 1 != 0 {
                let byte_pos = (bit_pos >> 3) as usize;
                let bit_res = bit_pos & 7;
                let mut instruction = 0u64;
                for j in 0..6 {
                    instruction |= u64::from(buf[i + j + byte_pos]) << (8 * j);
                }
                let mut norm = instruction >> bit_res;
                if ((norm >> 37) & 0xF) == 0x5 && ((norm >> 9) & 0x7) == 0 {
                    let mut src = ((norm >> 13) & 0xF_FFFF) as u32;
                    src |= (((norm >> 36) & 1) as u32) << 20;
                    src <<= 4;
                    let here = now.wrapping_add(i as u32);
                    let dest = if enc { here.wrapping_add(src) } else { src.wrapping_sub(here) } >> 4;
                    norm &= !(0x8F_FFFFu64 << 13);
                    norm |= u64::from(dest & 0xF_FFFF) << 13;
                    norm |= u64::from(dest & 0x10_0000) << (36 - 20);
                    instruction &= (1u64 << bit_res) - 1;
                    instruction |= norm << bit_res;
                    for j in 0..6 {
                        buf[i + j + byte_pos] = (instruction >> (8 * j)) as u8;
                    }
                }
            }
            bit_pos += 41;
        }
        i += 16;
    }
}

fn arm64(buf: &mut [u8], now: u32, enc: bool) {
    let n = buf.len() & !3;
    let mut i = 0;
    while i < n {
        let pc = now.wrapping_add(i as u32);
        let mut instr = u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
        if (instr >> 26) == 0x25 {
            let src = instr;
            let pc = if enc { pc >> 2 } else { 0u32.wrapping_sub(pc >> 2) };
            instr = 0x9400_0000 | (src.wrapping_add(pc) & 0x03FF_FFFF);
            buf[i..i + 4].copy_from_slice(&instr.to_le_bytes());
        } else if (instr & 0x9F00_0000) == 0x9000_0000 {
            let src = ((instr >> 29) & 3) | ((instr >> 3) & 0x001F_FFFC);
            if (src.wrapping_add(0x0002_0000) & 0x001C_0000) == 0 {
                instr &= 0x9000_001F;
                let pc = if enc { pc >> 12 } else { 0u32.wrapping_sub(pc >> 12) };
                let dest = src.wrapping_add(pc);
                instr |= (dest & 3) << 29;
                instr |= (dest & 0x0003_FFFC) << 3;
                instr |= 0u32.wrapping_sub(dest & 0x0002_0000) & 0x00E0_0000;
                buf[i..i + 4].copy_from_slice(&instr.to_le_bytes());
            }
        }
        i += 4;
    }
}

fn riscv_decode(buf: &mut [u8], now: u32) {
    if buf.len() < 8 {
        return;
    }
    let size = buf.len() - 8;
    let read32le = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let mut i = 0;
    while i <= size {
        let mut inst = u32::from(buf[i]);
        if inst == 0xEF {
            // JAL
            let b1 = u32::from(buf[i + 1]);
            if (b1 & 0x0D) != 0 {
                i += 2;
                continue;
            }
            let b2 = u32::from(buf[i + 2]);
            let b3 = u32::from(buf[i + 3]);
            let pc = now.wrapping_add(i as u32);
            let addr = (((b1 & 0xF0) << 13) | (b2 << 9) | (b3 << 1)).wrapping_sub(pc);
            buf[i + 1] = ((b1 & 0x0F) | ((addr >> 8) & 0xF0)) as u8;
            buf[i + 2] = (((addr >> 16) & 0x0F) | ((addr >> 7) & 0x10) | ((addr << 4) & 0xE0)) as u8;
            buf[i + 3] = (((addr >> 4) & 0x7F) | ((addr >> 13) & 0x80)) as u8;
            i += 4;
        } else if (inst & 0x7F) == 0x17 {
            // AUIPC
            inst |= u32::from(buf[i + 1]) << 8 | u32::from(buf[i + 2]) << 16 | u32::from(buf[i + 3]) << 24;
            let mut inst2;
            if inst & 0xE80 != 0 {
                inst2 = read32le(buf, i + 4);
                if ((inst << 8) ^ inst2.wrapping_sub(3)) & 0xF8003 != 0 {
                    i += 6;
                    continue;
                }
                let addr = (inst & 0xFFFF_F000).wrapping_add(inst2 >> 20);
                inst = 0x17 | (2 << 7) | (inst2 << 12);
                inst2 = addr;
            } else {
                let inst2_rs1 = inst >> 27;
                if (inst.wrapping_sub(0x3117) << 18) >= (inst2_rs1 & 0x1D) {
                    i += 4;
                    continue;
                }
                let addr = u32::from_be_bytes([buf[i + 4], buf[i + 5], buf[i + 6], buf[i + 7]])
                    .wrapping_sub(now.wrapping_add(i as u32));
                inst2 = (inst >> 12) | (addr << 20);
                inst = 0x17 | (inst2_rs1 << 7) | (addr.wrapping_add(0x800) & 0xFFFF_F000);
            }
            buf[i..i + 4].copy_from_slice(&inst.to_le_bytes());
            buf[i + 4..i + 8].copy_from_slice(&inst2.to_le_bytes());
            i += 8;
        } else {
            i += 2;
        }
    }
}

/// Undoes the delta filter: each byte had the byte `distance` (1–256)
/// before it subtracted.
pub fn delta_decode(buf: &mut [u8], distance: usize) {
    for i in distance..buf.len() {
        buf[i] = buf[i].wrapping_add(buf[i - distance]);
    }
}

/// Applies the delta filter.
pub fn delta_encode(buf: &mut [u8], distance: usize) {
    for i in (distance..buf.len()).rev() {
        buf[i] = buf[i].wrapping_sub(buf[i - distance]);
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Decoding undoes encoding, for every buffer of up to `N` bytes and
    /// every start offset the filter allows.
    fn round_trip<const N: usize>(f: BranchFilter) {
        let data: [u8; N] = kani::any();
        let n: usize = kani::any_where(|&n: &usize| n <= N);
        let start: u32 = kani::any::<u32>() & !(f.alignment() - 1);
        let mut buf = data;
        assert!(f.encode(&mut buf[..n], start));
        f.decode(&mut buf[..n], start);
        assert_eq!(buf, data);
    }

    #[kani::proof]
    #[kani::unwind(16)]
    fn x86_round_trips() {
        round_trip::<12>(BranchFilter::X86);
    }

    #[kani::proof]
    #[kani::unwind(16)]
    fn powerpc_round_trips() {
        round_trip::<12>(BranchFilter::PowerPc);
    }

    #[kani::proof]
    #[kani::unwind(36)]
    fn ia64_round_trips() {
        round_trip::<32>(BranchFilter::Ia64);
    }

    #[kani::proof]
    #[kani::unwind(16)]
    fn arm_round_trips() {
        round_trip::<12>(BranchFilter::Arm);
    }

    #[kani::proof]
    #[kani::unwind(16)]
    fn armthumb_round_trips() {
        round_trip::<12>(BranchFilter::ArmThumb);
    }

    #[kani::proof]
    #[kani::unwind(16)]
    fn sparc_round_trips() {
        round_trip::<12>(BranchFilter::Sparc);
    }

    #[kani::proof]
    #[kani::unwind(16)]
    fn arm64_round_trips() {
        round_trip::<12>(BranchFilter::Arm64);
    }

    /// The RISC-V decoder (which has no encoder here) never panics on any
    /// buffer of up to 20 bytes at any start.
    #[kani::proof]
    #[kani::unwind(22)]
    fn riscv_decoding_never_panics() {
        let mut data: [u8; 20] = kani::any();
        let n: usize = kani::any_where(|&n: &usize| n <= 20);
        BranchFilter::RiscV.decode(&mut data[..n], kani::any());
    }

    /// Delta decoding undoes delta encoding, for every distance.
    #[kani::proof]
    #[kani::unwind(10)]
    fn delta_round_trips() {
        let data: [u8; 8] = kani::any();
        let dist: usize = kani::any_where(|&d: &usize| (1..=256).contains(&d));
        let mut buf = data;
        delta_encode(&mut buf, dist);
        delta_decode(&mut buf, dist);
        assert_eq!(buf, data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_like(len: usize, seed: u32) -> Vec<u8> {
        // Bytes rich in the patterns every filter looks for.
        let mut x = seed;
        let pats: [&[u8]; 8] = [
            &[0xE8, 1, 2, 0, 0],
            &[0xE9, 0xF0, 0xFF, 0xFF, 0xFF],
            &[0x12, 0x34, 0x56, 0xEB],
            &[0x00, 0xF0, 0x00, 0xF8],
            &[0x48, 0, 1, 0x01],
            &[0x40, 0x01, 0x02, 0x03],
            &[0x01, 0x00, 0x00, 0x94],
            &[0x97, 0x05, 0, 0, 0x93, 0x85, 0x05, 0],
        ];
        let mut v = Vec::new();
        while v.len() < len {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            if x >> 31 == 0 {
                v.extend_from_slice(pats[(x >> 16) as usize % 8]);
            } else {
                v.push((x >> 16) as u8);
            }
        }
        v.truncate(len);
        v
    }

    #[test]
    fn encode_then_decode_is_identity() {
        for f in [
            BranchFilter::X86,
            BranchFilter::PowerPc,
            BranchFilter::Ia64,
            BranchFilter::Arm,
            BranchFilter::ArmThumb,
            BranchFilter::Sparc,
            BranchFilter::Arm64,
        ] {
            for (len, start) in [(0, 0), (3, 0), (17, 5), (4096, 0), (100_000, 0x1234_5678)] {
                let start = start & !(f.alignment() - 1);
                let data = code_like(len, len as u32 + 1);
                let mut buf = data.clone();
                assert!(f.encode(&mut buf, start));
                if len >= 4096 {
                    assert_ne!(buf, data, "{f:?} changed nothing");
                }
                f.decode(&mut buf, start);
                assert_eq!(buf, data, "{f:?} len {len}");
            }
        }
        let data = code_like(10_000, 3);
        for dist in [1, 2, 4, 256] {
            let mut buf = data.clone();
            delta_encode(&mut buf, dist);
            delta_decode(&mut buf, dist);
            assert_eq!(buf, data);
        }
    }

    #[test]
    fn ids() {
        for id in 0x04..=0x0B {
            assert_eq!(BranchFilter::from_xz_id(id).unwrap().xz_id(), id);
        }
        assert_eq!(BranchFilter::from_xz_id(0x21), None);
        assert!(!BranchFilter::RiscV.encode(&mut [0; 8], 0));
    }
}
