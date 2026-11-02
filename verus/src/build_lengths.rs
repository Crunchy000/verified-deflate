//! The Huffman code-length builder (`huffman::build_lengths` in the
//! library, with `build_lengths_in`, `huffman_depths`, `take_lightest`,
//! `sort_by_weight`, `sift_down` and `heavier`), proved for alphabets of
//! every size, including DEFLATE's 19, 30 and 286 symbols.
//!
//! The executable code below is the library's, token for token (CI checks
//! this with `.github/scripts/verus_drift.py`); every line marked
//! `// @ghost` is specification or proof. The theorem (`good`): for any
//! frequencies and any limit, `build_lengths` succeeds exactly when the
//! request can be met, and then the lengths form a complete prefix code
//! (Kraft sum exactly 1, the library's `is_complete`), no length exceeds
//! the limit, and the symbols with codes are exactly the symbols used (or
//! symbols 0 and 1 when fewer than two are used). No call can panic.
//!
//! Which tree the builder makes, and so how short the code is, is not part
//! of the theorem: any complete code within the limit is a valid DEFLATE
//! code.

use crate::limit_depths::{
    kraft_sum, lemma_kraft_scale, lemma_kraft_take, lemma_p2_add, lemma_p2_pos, limit_depths, p2,
    within, HuffmanError,
};
use vstd::prelude::*;

verus! {

pub const MAX_CODE_LEN: usize = 15;

pub const MAX_SYMBOLS: usize = 288;

const SMALL_ALPHABET: usize = 32;

/// The number of used symbols (nonzero frequencies).
pub open spec fn used_count(f: Seq<u32>) -> nat
    decreases f.len(),
{
    if f.len() == 0 {
        0
    } else {
        used_count(f.drop_last()) + (if f.last() != 0 {
            1nat
        } else {
            0nat
        })
    }
}

/// The Kraft sum of code lengths in units of 2^-15, over the symbols with
/// a code (length 0 means none): what the library's `kraft_sum` computes.
/// A complete code sums to 2^15.
pub open spec fn kraft15(l: Seq<u8>) -> nat
    decreases l.len(),
{
    if l.len() == 0 {
        0
    } else {
        kraft15(l.drop_last()) + (if l.last() != 0 {
            p2((15 - l.last()) as nat)
        } else {
            0nat
        })
    }
}

/// The theorem about a successful build: lengths `l` for frequencies `f`
/// under `limit`.
pub open spec fn good(f: Seq<u32>, l: Seq<u8>, limit: nat) -> bool {
    &&& l.len() == f.len()
    &&& forall|i: int| 0 <= i < l.len() ==> l[i] <= limit
    &&& forall|i: int| 0 <= i < l.len() && f[i] != 0 ==> l[i] != 0
    &&& used_count(f) >= 2 ==> forall|i: int| 0 <= i < l.len() && f[i] == 0 ==> l[i] == 0
    &&& kraft15(l) == p2(15)
}

/// When a build succeeds: one length per frequency, at least two symbols
/// and at most the scratch capacity `n_max`, a limit of 1-15 bits, and no
/// more used symbols than codes of that length.
pub open spec fn can_build(f: Seq<u32>, lengths_len: nat, limit: nat, n_max: nat) -> bool {
    &&& f.len() == lengths_len
    &&& 2 <= f.len() <= n_max
    &&& 1 <= limit <= 15
    &&& used_count(f) <= p2(limit)
}

proof fn lemma_used_take(f: Seq<u32>, i: int)
    requires
        0 <= i < f.len(),
    ensures
        used_count(f.take(i + 1)) == used_count(f.take(i)) + (if f[i] != 0 {
            1nat
        } else {
            0nat
        }),
{
    assert(f.take(i + 1).drop_last() =~= f.take(i));
    assert(f.take(i + 1).last() == f[i]);
}

proof fn lemma_kraft15_zero(l: Seq<u8>)
    requires
        forall|i: int| 0 <= i < l.len() ==> l[i] == 0,
    ensures
        kraft15(l) == 0,
    decreases l.len(),
{
    if l.len() > 0 {
        let s = l.drop_last();
        assert forall|i: int| 0 <= i < s.len() implies s[i] == 0 by {
            assert(s[i] == l[i]);
        }
        lemma_kraft15_zero(s);
        assert(l.last() == l[l.len() - 1]);
    }
}

/// Giving a symbol without a code a length of 1-15 adds its share.
proof fn lemma_kraft15_set(l: Seq<u8>, i: int, v: u8)
    requires
        0 <= i < l.len(),
        l[i] == 0,
        1 <= v <= 15,
    ensures
        kraft15(l.update(i, v)) == kraft15(l) + p2((15 - v) as nat),
    decreases l.len(),
{
    let u = l.update(i, v);
    if i == l.len() - 1 {
        assert(u.drop_last() =~= l.drop_last());
        assert(u.last() == v);
        assert(l.last() == l[i]);
    } else {
        lemma_kraft15_set(l.drop_last(), i, v);
        assert(u.drop_last() =~= l.drop_last().update(i, v));
        assert(u.last() == l.last());
    }
}

/// `1 << s` is 2^s.
proof fn lemma_shl_usize(s: usize)
    requires
        s <= 15,
    ensures
        (1usize << s) as nat == p2(s as nat),
    decreases s,
{
    if s == 0 {
        assert(1usize << 0usize == 1usize) by (bit_vector);
    } else {
        let t = (s - 1) as usize;
        lemma_shl_usize(t);
        assert(1usize << s == 2 * (1usize << t)) by (bit_vector)
            requires
                t < 15,
                s == t + 1,
        ;
    }
}

#[allow(dead_code)]
pub fn build_lengths(freqs: &[u32], limit: usize, lengths: &mut [u8]) -> (r: Result<(), HuffmanError>)
    ensures // @ghost
        r is Ok <==> can_build(freqs@, old(lengths)@.len(), limit as nat, MAX_SYMBOLS as nat), // @ghost
        r is Ok ==> good(freqs@, final(lengths)@, limit as nat), // @ghost
{
    if freqs.len() <= SMALL_ALPHABET {
        build_lengths_in::<SMALL_ALPHABET, { 2 * SMALL_ALPHABET }>(freqs, limit, lengths)
    } else {
        build_lengths_in::<MAX_SYMBOLS, { 2 * MAX_SYMBOLS }>(freqs, limit, lengths)
    }
}

#[verifier::loop_isolation(false)]
fn build_lengths_in<const N: usize, const M: usize>(
    freqs: &[u32],
    limit: usize,
    lengths: &mut [u8],
) -> (r: Result<(), HuffmanError>)
    requires // @ghost
        2 * N <= M <= 65536, // @ghost
    ensures // @ghost
        r is Ok <==> can_build(freqs@, old(lengths)@.len(), limit as nat, N as nat), // @ghost
        r is Ok ==> good(freqs@, final(lengths)@, limit as nat), // @ghost
{
    let n = freqs.len();
    if lengths.len() != n {
        return Err(HuffmanError::LengthMismatch);
    }
    if n > N {
        return Err(HuffmanError::TooManySymbols);
    }
    if limit > MAX_CODE_LEN {
        return Err(HuffmanError::CodeTooLong);
    }
    if n < 2 || limit == 0 {
        return Err(HuffmanError::LimitTooSmall);
    }
    let ghost f = freqs@; // @ghost
    let mut used = [0u16; N];
    let mut m = 0;
    for i in 0..n
        invariant // @ghost
            lengths@.len() == n, // @ghost
            m <= i, // @ghost
            m as nat == used_count(f.take(i as int)), // @ghost
            forall|j: int| 0 <= j < i ==> lengths@[j] == 0, // @ghost
            forall|k: int| 0 <= k < m ==> (used@[k] as int) < i && f[used@[k] as int] != 0, // @ghost
            forall|a: int, b: int| 0 <= a < b < m ==> used@[a] < used@[b], // @ghost
            forall|j: int| // @ghost
                0 <= j < i && f[j] != 0 ==> used_count(f.take(j)) < m && used@[used_count(f.take(j)) as int] == j, // @ghost
    {
        proof { // @ghost-begin
            lemma_used_take(f, i as int);
        } // @ghost-end
        lengths[i] = 0;
        if freqs[i] != 0 {
            used[m] = i as u16;
            m += 1;
        }
    }
    proof { // @ghost-begin
        assert(f.take(n as int) =~= f);
        lemma_kraft15_zero(lengths@);
        lemma_p2_pos((limit - 1) as nat);
        assert(p2(limit as nat) == 2 * p2((limit - 1) as nat));
    } // @ghost-end
    if m < 2 {
        let s = if m == 1 { used[0] as usize } else { 0 };
        proof { // @ghost-begin
            lemma_kraft15_set(lengths@, s as int, 1);
        } // @ghost-end
        lengths[s] = 1;
        proof { // @ghost-begin
            lemma_kraft15_set(lengths@, (if s == 0 { 1int } else { 0int }), 1);
        } // @ghost-end
        lengths[if s == 0 { 1 } else { 0 }] = 1;
        proof { // @ghost-begin
            assert(p2(15) == 2 * p2(14));
            assert forall|i: int| 0 <= i < n && f[i] != 0 implies lengths@[i] != 0 by {
                assert(used@[used_count(f.take(i)) as int] == i);
            }
        } // @ghost-end
        return Ok(());
    }
    proof { // @ghost-begin
        lemma_shl_usize(limit);
    } // @ghost-end
    if m > (1usize << limit) {
        return Err(HuffmanError::LimitTooSmall);
    }
    let mut weight = [0u64; N];
    for k in 0..m
        invariant // @ghost
            weight@.len() == N, // @ghost
    {
        weight[k] = u64::from(freqs[used[k] as usize]);
    }
    let mut depth = [0u32; N];
    huffman_depths::<N, M>(&weight, m, &mut depth);
    proof { // @ghost-begin
        assert forall|i: int| 0 <= i < m implies depth@.subrange(0, m as int)[i] >= 1 by {
            assert(depth@.subrange(0, m as int)[i] == depth@[i]);
        }
    } // @ghost-end

    limit_depths(&mut depth[..m], &weight[..m], limit)?;
    let ghost d = depth@.take(m as int); // @ghost
    proof { // @ghost-begin
        assert(within(d, 1, limit as nat));
        assert(kraft_sum(d, limit as nat) == p2(limit as nat));
    } // @ghost-end
    for k in 0..m
        invariant // @ghost
            lengths@.len() == n, // @ghost
            depth@.take(m as int) == d, // @ghost
            forall|j: int| 0 <= j < k ==> lengths@[used@[j] as int] == depth@[j] as u8, // @ghost
            forall|j: int| k <= j < m ==> lengths@[used@[j] as int] == 0, // @ghost
            forall|i: int| 0 <= i < n && f[i] == 0 ==> lengths@[i] == 0, // @ghost
            forall|i: int| 0 <= i < n ==> lengths@[i] <= limit, // @ghost
            kraft15(lengths@) == kraft_sum(d.take(k as int), 15), // @ghost
    {
        proof { // @ghost-begin
            assert(d[k as int] == depth@[k as int]);
            assert(d.take(k as int + 1).drop_last() =~= d.take(k as int));
            lemma_kraft_take(d, k as int, 15);
            lemma_kraft15_set(lengths@, used@[k as int] as int, depth@[k as int] as u8);
        } // @ghost-end
        lengths[used[k] as usize] = depth[k] as u8;
    }
    proof { // @ghost-begin
        assert(d.take(m as int) =~= d);
        lemma_kraft_scale(d, limit as nat, 15);
        lemma_p2_add((15 - limit) as nat, limit as nat);
        assert((15 - limit) as nat + limit as nat == 15);
        assert forall|i: int| 0 <= i < n && f[i] != 0 implies lengths@[i] != 0 by {
            let k = used_count(f.take(i)) as int;
            assert(d[k] == depth@[k]);
        }
    } // @ghost-end
    Ok(())
}

#[verifier::loop_isolation(false)]
fn huffman_depths<const N: usize, const M: usize>(weights: &[u64; N], m: usize, depths: &mut [u32; N])
    requires // @ghost
        2 <= m <= N, // @ghost
        2 * N <= M <= 65536, // @ghost
    ensures // @ghost
        forall|i: int| 0 <= i < m ==> final(depths)@[i] >= 1, // @ghost
{
    let mut order = [0u16; N];
    let mut w = [0u64; M];
    let mut parent = [0u16; M];
    for i in 0..m
        invariant // @ghost
            forall|j: int| 0 <= j < i ==> (order@[j] as int) < N, // @ghost
    {
        order[i] = i as u16;
        w[i] = weights[i];
    }
    proof { // @ghost-begin
        assert forall|j: int| 0 <= j < m implies (order@.subrange(0, m as int)[j] as int) < N by {
            assert(order@.subrange(0, m as int)[j] == order@[j]);
        }
    } // @ghost-end
    sort_by_weight(&mut order[..m], weights);
    // Leaf queue: order[leaf..m]. Merged queue: node ids merged..next.
    let mut leaf = 0;
    let mut merged = m;
    let mut next = m;
    proof { // @ghost-begin
        assert forall|j: int| 0 <= j < m implies (order@[j] as int) < N by {
            assert(order@.subrange(0, m as int)[j] == order@[j]);
        }
    } // @ghost-end
    while (m - leaf) + (next - merged) > 1
        invariant // @ghost
            leaf <= m <= merged <= next, // @ghost
            (m - leaf) + (next - merged) >= 1, // @ghost
            (m - leaf) + (next - merged) + next == 2 * m, // @ghost
            forall|j: int| 0 <= j < m ==> (order@[j] as int) < N, // @ghost
            forall|j: int| 0 <= j < M ==> (parent@[j] as int) < M, // @ghost
        decreases (m - leaf) + (next - merged), // @ghost
    {
        proof { // @ghost-begin
            assert forall|j: int| 0 <= j < m implies (order@.subrange(0, m as int)[j] as int) < M by {
                assert(order@.subrange(0, m as int)[j] == order@[j]);
            }
        } // @ghost-end
        let a = take_lightest(&order[..m], &mut leaf, &mut merged, next, &w);
        let b = take_lightest(&order[..m], &mut leaf, &mut merged, next, &w);
        w[next] = w[a].saturating_add(w[b]);
        parent[a] = next as u16;
        parent[b] = next as u16;
        next += 1;
    }
    let mut depth = [0u32; M];
    let mut id = next - 1;
    while id > 0
        invariant // @ghost
            m < next <= M, // @ghost
            id <= next - 1, // @ghost
            forall|j: int| 0 <= j < M ==> (parent@[j] as int) < M, // @ghost
            forall|j: int| 0 <= j < M ==> depth@[j] <= next - 1 - id, // @ghost
            forall|j: int| id <= j < next - 1 ==> depth@[j] >= 1, // @ghost
        decreases id, // @ghost
    {
        id -= 1;
        depth[id] = depth[parent[id] as usize] + 1;
    }
    for i in 0..m
        invariant // @ghost
            forall|j: int| 0 <= j < i ==> depths@[j] >= 1, // @ghost
    {
        depths[i] = depth[i];
    }
}

fn take_lightest(order: &[u16], leaf: &mut usize, merged: &mut usize, next: usize, w: &[u64]) -> (r: usize)
    requires // @ghost
        *old(leaf) <= order@.len(), // @ghost
        *old(merged) <= next <= w@.len(), // @ghost
        (order@.len() - *old(leaf)) + (next - *old(merged)) >= 1, // @ghost
        forall|j: int| 0 <= j < order@.len() ==> (order@[j] as int) < w@.len(), // @ghost
    ensures // @ghost
        (r as int) < w@.len(), // @ghost
        *final(leaf) <= order@.len(), // @ghost
        *final(merged) <= next, // @ghost
        (*final(leaf) == *old(leaf) + 1 && *final(merged) == *old(merged)) // @ghost
            || (*final(leaf) == *old(leaf) && *final(merged) == *old(merged) + 1), // @ghost
{
    let leaf_ok = *leaf < order.len();
    let merged_ok = *merged < next;
    if leaf_ok && (!merged_ok || w[order[*leaf] as usize] <= w[*merged]) {
        *leaf += 1;
        order[*leaf - 1] as usize
    } else {
        *merged += 1;
        *merged - 1
    }
}

fn sort_by_weight(idx: &mut [u16], w: &[u64])
    requires // @ghost
        old(idx)@.len() <= 65536, // @ghost
        forall|i: int| 0 <= i < old(idx)@.len() ==> (old(idx)@[i] as int) < w@.len(), // @ghost
    ensures // @ghost
        final(idx)@.len() == old(idx)@.len(), // @ghost
        forall|i: int| 0 <= i < final(idx)@.len() ==> (final(idx)@[i] as int) < w@.len(), // @ghost
{
    let n = idx.len();
    let mut start = n / 2;
    while start > 0
        invariant // @ghost
            idx@.len() == n <= 65536, // @ghost
            start <= n / 2, // @ghost
            forall|i: int| 0 <= i < n ==> (idx@[i] as int) < w@.len(), // @ghost
        decreases start, // @ghost
    {
        start -= 1;
        sift_down(idx, start, n, w);
    }
    let mut end = n;
    while end > 1
        invariant // @ghost
            idx@.len() == n <= 65536, // @ghost
            end <= n, // @ghost
            forall|i: int| 0 <= i < n ==> (idx@[i] as int) < w@.len(), // @ghost
        decreases end, // @ghost
    {
        end -= 1;
        idx.swap(0, end);
        sift_down(idx, 0, end, w);
    }
}

fn heavier(a: u16, b: u16, w: &[u64]) -> (r: bool)
    requires // @ghost
        (a as int) < w@.len(), // @ghost
        (b as int) < w@.len(), // @ghost
{
    (w[a as usize], a) > (w[b as usize], b)
}

fn sift_down(idx: &mut [u16], mut root: usize, end: usize, w: &[u64])
    requires // @ghost
        root < end <= old(idx)@.len() <= 65536, // @ghost
        forall|i: int| 0 <= i < old(idx)@.len() ==> (old(idx)@[i] as int) < w@.len(), // @ghost
    ensures // @ghost
        final(idx)@.len() == old(idx)@.len(), // @ghost
        forall|i: int| 0 <= i < final(idx)@.len() ==> (final(idx)@[i] as int) < w@.len(), // @ghost
{
    loop
        invariant // @ghost
            idx@.len() == old(idx)@.len(), // @ghost
            root < end <= idx@.len() <= 65536, // @ghost
            forall|i: int| 0 <= i < idx@.len() ==> (idx@[i] as int) < w@.len(), // @ghost
        decreases end - root, // @ghost
    {
        let mut child = 2 * root + 1;
        if child >= end {
            return;
        }
        if child + 1 < end && heavier(idx[child + 1], idx[child], w) {
            child += 1;
        }
        if heavier(idx[child], idx[root], w) {
            idx.swap(root, child);
            root = child;
        } else {
            return;
        }
    }
}

} // verus!
