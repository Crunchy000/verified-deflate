//! The Burrows–Wheeler transform used by bzip2: sorting a block's cyclic
//! rotations.
//!
//! A block that repeats a shorter string `u` (`s = u^k`) sorts as `u`
//! does, each row `k` times. Otherwise its rotations are all different,
//! and one of them, `w`, is smaller than the rest: a Lyndon word. The
//! rotations of `w` sort in the same order as its suffixes (with the end
//! of the string smaller than every byte). Where neither of two suffixes
//! is a prefix of the other, the first difference decides both orders.
//! Where a suffix `x` is a proper prefix of a longer one `x·y…`, the
//! rotation starting with `x` continues with `w` itself and the other with
//! a different rotation of `w`, which is larger, so the shorter suffix's
//! rotation is the smaller, as in suffix order. The suffixes are sorted
//! with SA-IS (Nong, Zhang and Chan, 2009), which takes linear time
//! whatever the input, so no block can make compression slow.

/// Marks an empty suffix-array slot.
const EMPTY: u32 = u32::MAX;

/// The last column of the sorted rotation matrix of `s`, and the row at
/// which `s` itself (rotation 0) sorts.
///
/// # Panics
///
/// If `s` is empty, or longer than `u32::MAX / 2 - 1` bytes.
pub(crate) fn bwt(s: &[u8]) -> (Vec<u8>, usize) {
    let n = s.len();
    assert!(n > 0 && n < u32::MAX as usize, "block size out of range");
    let p = period(s);
    if p < n {
        // s = u^k: each of u's rows k times, and rotation 0 of s is in
        // the group of u's rotation 0.
        let k = n / p;
        let (last, origin) = bwt(&s[..p]);
        return (last.iter().flat_map(|&c| core::iter::repeat_n(c, k)).collect(), origin * k);
    }
    // w = s rotated by r is a Lyndon word. Shift each byte up by one and
    // end with a unique smallest sentinel (0), as SA-IS needs.
    let r = least_rotation(s);
    let mut t: Vec<u32> = Vec::with_capacity(n + 1);
    t.extend(s[r..].iter().chain(&s[..r]).map(|&b| u32::from(b) + 1));
    t.push(0);
    let sa = suffix_array(&t, 257);
    let mut last = Vec::with_capacity(n);
    let mut origin = 0;
    // sa[0] is the sentinel; rotation q of w is rotation (q + r) % n of s.
    for (row, &q) in sa[1..].iter().enumerate() {
        let start = (q as usize + r) % n;
        if start == 0 {
            origin = row;
        }
        last.push(s[(start + n - 1) % n]);
    }
    (last, origin)
}

/// The length of the shortest `u` with `s = u^k`.
fn period(s: &[u8]) -> usize {
    // The prefix function: pi[i] is the length of the longest proper
    // prefix of s[..=i] that is also its suffix.
    let n = s.len();
    let mut pi = vec![0u32; n];
    for i in 1..n {
        let mut k = pi[i - 1] as usize;
        while k > 0 && s[i] != s[k] {
            k = pi[k - 1] as usize;
        }
        if s[i] == s[k] {
            k += 1;
        }
        pi[i] = k as u32;
    }
    let p = n - pi[n - 1] as usize;
    if n % p == 0 {
        p
    } else {
        n
    }
}

/// The start of the smallest rotation of `s` (the first, if several are
/// equal), in linear time.
fn least_rotation(s: &[u8]) -> usize {
    let n = s.len();
    // Candidates i and j; their rotations agree on the first k bytes.
    let (mut i, mut j, mut k) = (0, 1, 0);
    while i < n && j < n && k < n {
        let (a, b) = (s[(i + k) % n], s[(j + k) % n]);
        if a == b {
            k += 1;
            continue;
        }
        // The larger rotation, and every one starting within its first
        // k + 1 bytes, cannot be the smallest.
        if a > b {
            i += k + 1;
        } else {
            j += k + 1;
        }
        if i == j {
            j += 1;
        }
        k = 0;
    }
    i.min(j)
}

/// The suffix array of `t`, whose values are below `k` and whose last
/// value is a 0 that occurs nowhere else.
pub(crate) fn suffix_array(t: &[u32], k: usize) -> Vec<u32> {
    let mut sa = vec![EMPTY; t.len()];
    sais(t, &mut sa, k);
    sa
}

/// Whether position `i` starts a leftmost-S-type suffix.
fn is_lms(stype: &[bool], i: usize) -> bool {
    i > 0 && stype[i] && !stype[i - 1]
}

/// The first slot of each character's bucket.
fn bucket_starts(counts: &[u32]) -> Vec<u32> {
    let mut sum = 0;
    counts
        .iter()
        .map(|&c| {
            let start = sum;
            sum += c;
            start
        })
        .collect()
}

/// One past the last slot of each character's bucket.
fn bucket_ends(counts: &[u32]) -> Vec<u32> {
    let mut sum = 0;
    counts
        .iter()
        .map(|&c| {
            sum += c;
            sum
        })
        .collect()
}

/// Induces the order of the L-type suffixes from the S-type ones placed in
/// `sa`, then of the S-type suffixes from the L-type ones.
fn induce(t: &[u32], sa: &mut [u32], stype: &[bool], counts: &[u32]) {
    let n = t.len();
    let mut starts = bucket_starts(counts);
    for i in 0..n {
        let j = sa[i];
        if j != EMPTY && j > 0 && !stype[j as usize - 1] {
            let c = t[j as usize - 1] as usize;
            sa[starts[c] as usize] = j - 1;
            starts[c] += 1;
        }
    }
    let mut ends = bucket_ends(counts);
    for i in (0..n).rev() {
        let j = sa[i];
        if j != EMPTY && j > 0 && stype[j as usize - 1] {
            let c = t[j as usize - 1] as usize;
            ends[c] -= 1;
            sa[ends[c] as usize] = j - 1;
        }
    }
}

/// Whether the LMS substrings at `a` and `b` are equal: the same values
/// and types up to and including the next LMS position of each.
fn lms_substrings_equal(t: &[u32], stype: &[bool], a: usize, b: usize) -> bool {
    let mut d = 0;
    loop {
        let (x, y) = (a + d, b + d);
        let (end_x, end_y) = (d > 0 && is_lms(stype, x), d > 0 && is_lms(stype, y));
        if t[x] != t[y] || stype[x] != stype[y] || end_x != end_y {
            return false;
        }
        if end_x {
            return true;
        }
        // The sentinel at the end is an LMS position that is reached
        // before either index could run past it, or the values differ.
        d += 1;
    }
}

fn sais(t: &[u32], sa: &mut [u32], k: usize) {
    let n = t.len();
    if n == 1 {
        sa[0] = 0;
        return;
    }
    let mut stype = vec![false; n];
    stype[n - 1] = true;
    for i in (0..n - 1).rev() {
        stype[i] = t[i] < t[i + 1] || (t[i] == t[i + 1] && stype[i + 1]);
    }
    let mut counts = vec![0u32; k];
    for &c in t {
        counts[c as usize] += 1;
    }

    // 1. Sort the LMS substrings: place the LMS positions at their
    //    buckets' ends and induce.
    sa.fill(EMPTY);
    let mut ends = bucket_ends(&counts);
    for (i, &c) in t.iter().enumerate().skip(1) {
        if is_lms(&stype, i) {
            ends[c as usize] -= 1;
            sa[ends[c as usize] as usize] = i as u32;
        }
    }
    induce(t, sa, &stype, &counts);

    // 2. Name the sorted LMS substrings, equal ones alike, giving the
    //    reduced string in text order.
    let mut sorted_lms = Vec::new();
    for &p in sa.iter() {
        if p != EMPTY && is_lms(&stype, p as usize) {
            sorted_lms.push(p as usize);
        }
    }
    let m = sorted_lms.len();
    let mut names = vec![EMPTY; n];
    let mut name = 0u32;
    let mut prev: Option<usize> = None;
    for &p in &sorted_lms {
        if prev.is_none_or(|q| !lms_substrings_equal(t, &stype, q, p)) {
            name += 1;
            prev = Some(p);
        }
        names[p] = name - 1;
    }
    let lms_positions: Vec<usize> = (1..n).filter(|&i| is_lms(&stype, i)).collect();
    let reduced: Vec<u32> = lms_positions.iter().map(|&p| names[p]).collect();
    drop(names);

    // 3. Sort the LMS suffixes: directly if every name is distinct,
    //    otherwise by recursing on the reduced string.
    let mut reduced_sa = vec![EMPTY; m];
    if (name as usize) < m {
        sais(&reduced, &mut reduced_sa, name as usize);
    } else {
        for (i, &c) in reduced.iter().enumerate() {
            reduced_sa[c as usize] = i as u32;
        }
    }

    // 4. Place the sorted LMS suffixes at their buckets' ends, last first
    //    so each keeps its order, and induce the rest.
    sa.fill(EMPTY);
    let mut ends = bucket_ends(&counts);
    for &r in reduced_sa.iter().rev() {
        let p = lms_positions[r as usize];
        let c = t[p] as usize;
        ends[c] -= 1;
        sa[ends[c] as usize] = p as u32;
    }
    induce(t, sa, &stype, &counts);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive_bwt(s: &[u8]) -> (Vec<u8>, Vec<usize>) {
        let n = s.len();
        let rot = |i: usize| -> Vec<u8> { s[i..].iter().chain(&s[..i]).copied().collect() };
        let mut rows: Vec<usize> = (0..n).collect();
        rows.sort_by_key(|&i| rot(i));
        let last = rows.iter().map(|&i| s[(i + n - 1) % n]).collect();
        // Every row equal to rotation 0 is a valid origin.
        let origins = rows.iter().enumerate().filter(|&(_, &i)| rot(i) == rot(0)).map(|(r, _)| r).collect();
        (last, origins)
    }

    fn check(s: &[u8]) {
        let (last, origin) = bwt(s);
        let (want, origins) = naive_bwt(s);
        assert_eq!(last, want, "{s:?}");
        assert!(origins.contains(&origin), "{s:?}: origin {origin} not in {origins:?}");
    }

    #[test]
    fn matches_a_naive_sort() {
        for s in [&b"a"[..], b"ab", b"ba", b"banana", b"mississippi", b"aaaa", b"abab", b"abcabcabc", b"\0\xff\0\xff\0"]
        {
            check(s);
        }
        let mut x = 12345u32;
        for len in 1..200 {
            for alphabet in [2u32, 3, 256] {
                let s: Vec<u8> = (0..len)
                    .map(|_| {
                        x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                        ((x >> 16) % alphabet) as u8
                    })
                    .collect();
                check(&s);
            }
        }
    }

    #[test]
    fn suffix_array_matches_a_naive_sort() {
        let mut x = 7u32;
        for len in 1..300 {
            let mut t: Vec<u32> = (0..len)
                .map(|_| {
                    x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    1 + (x >> 16) % 4
                })
                .collect();
            t.push(0);
            let sa = suffix_array(&t, 5);
            let mut want: Vec<u32> = (0..t.len() as u32).collect();
            want.sort_by(|&a, &b| t[a as usize..].cmp(&t[b as usize..]));
            assert_eq!(sa, want);
        }
    }

    #[test]
    fn helpers() {
        assert_eq!(period(b"abab"), 2);
        assert_eq!(period(b"aaa"), 1);
        assert_eq!(period(b"aba"), 3);
        assert_eq!(period(b"abcabcab"), 8);
        assert_eq!(least_rotation(b"cab"), 1);
        assert_eq!(least_rotation(b"bca"), 2);
        assert_eq!(least_rotation(b"baab"), 1);
        let mut x = 99u32;
        for len in 1..100 {
            let s: Vec<u8> = (0..len)
                .map(|_| {
                    x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    b'a' + ((x >> 16) % 3) as u8
                })
                .collect();
            let rot = |i: usize| -> Vec<u8> { s[i..].iter().chain(&s[..i]).copied().collect() };
            let min = (0..len).map(rot).min().unwrap();
            assert_eq!(rot(least_rotation(&s)), min);
        }
    }

    #[test]
    fn periodic_blocks() {
        check(&b"xy".repeat(500));
        check(&[7u8; 1000]);
        check(&b"abcab".repeat(301));
    }
}
