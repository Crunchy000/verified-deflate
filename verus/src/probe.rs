//! Temporary: checks which Rust features this Verus release accepts.
#![allow(dead_code)]

use vstd::prelude::*;

verus! {

pub enum E {
    A,
    B,
}

pub assume_specification[ u32::div_ceil ](x: u32, rhs: u32) -> (r: u32)
    requires
        rhs > 0,
    ensures
        r as int == (x as int + rhs as int - 1) / (rhs as int),
;

pub assume_specification[ u16::reverse_bits ](x: u16) -> u16;

pub assume_specification<T: Clone>[ <[T]>::fill ](s: &mut [T], value: T)
    ensures
        final(s)@.len() == old(s)@.len(),
;

fn p1(n: u32) -> u32
    requires
        n <= 32,
{
    debug_assert!(n <= 32, "n = {n} exceeds 32");
    n
}

fn bit(x: u32) -> (r: Result<u32, E>)
    ensures
        r is Ok ==> r->Ok_0 <= 1,
{
    if x > 5 {
        Err(E::A)
    } else {
        Ok(x & 1)
    }
}

fn p2(x: u32) -> Result<u32, E> {
    let mut code: u32 = 0;
    macro_rules! step {
        ($k:expr) => {
            code += bit(x)?;
            if code > $k {
                return Ok(code);
            }
            code *= 2;
        };
    }
    step!(1);
    step!(2);
    Ok(code)
}

fn p3(pos: usize, rem: usize, len: usize) -> Result<usize, E> {
    let end = match pos.checked_add(rem) {
        Some(end) if end <= len => end,
        _ => return Err(E::B),
    };
    Ok(end)
}

fn p4(x: u32, y: u64, z: u16, s: &mut [u8]) -> u64 {
    let a = x.div_ceil(8);
    let b = y;
    let c = [1u8, 2];
    let d = z.reverse_bits();
    s.fill(7);
    b ^ (c[0] as u64) ^ (d as u64) ^ (a as u64)
}

pub struct D {
    v: [u16; 4],
    n: u32,
}

impl D {
    pub const EMPTY: Self = Self { v: [0; 4], n: 1 };

    pub open spec fn wf(&self) -> bool {
        self.n <= 10
    }

    pub const fn make(k: u32) -> (r: Result<Self, E>)
        ensures
            r is Ok ==> r->Ok_0.wf(),
    {
        if k > 10 {
            return Err(E::A);
        }
        Ok(Self { v: [0; 4], n: k })
    }
}

pub exec const FIXED: D
    ensures
        FIXED.wf(),
{
    match D::make(3) {
        Ok(d) => d,
        Err(_) => D::EMPTY,
    }
}

pub struct R<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> R<'a> {
    pub fn read(&mut self, out: &mut [u8]) -> (r: Result<(), E>)
        ensures
            final(self).data == old(self).data,
    {
        let b = *self.data.get(self.pos).ok_or(E::A)?;
        if out.len() > 0 {
            out[0] = b;
        }
        Ok(())
    }
}

pub struct O {
    buf: Vec<u8>,
    len: usize,
}

pub struct S<'a> {
    r: R<'a>,
    o: O,
}

impl S<'_> {
    fn stored(&mut self, len: usize) -> Result<(), E>
        requires
            old(self).o.len + len <= old(self).o.buf@.len(),
    {
        let start = self.o.len;
        self.r.read(&mut self.o.buf.as_mut_slice()[start..start + len])?;
        Ok(())
    }
}

fn p5(sym: u8, x: Result<u32, E>) -> Result<(u8, usize), E> {
    let (value, repeat) = match sym {
        0..=15 => (sym, 1),
        16 => (0, 3),
        _ => return Err(E::B),
    };
    let y = x.map_err(|_| E::B)?;
    Ok((value, repeat + y as usize))
}

fn p6(input_len: usize, limit: usize) -> usize {
    let cap = input_len.clamp(4096, 1 << 26);
    let w = (cap + 0).max(limit.saturating_mul(2)).min(limit);
    w
}

pub struct Big {
    pub data: Vec<u8>,
    pub n: usize,
}

fn p7(r: Result<Big, E>) -> Result<Vec<u8>, E> {
    r.map(|d| d.data)
}

} // verus!
