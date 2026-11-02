//! The length-limit repair of the Huffman length builder
//! (`huffman::limit_depths` in the library), proved for any number of
//! symbols.
//!
//! The executable code below is the library's, token for token (CI checks
//! this with `.github/scripts/verus_drift.py`); every line marked
//! `// @ghost` is specification or proof. The theorem: from any code
//! lengths of at least 1 over 2 to 2^limit symbols, with any weights and a
//! limit of 1-15 bits, the repair returns `Ok` and leaves a complete
//! prefix code (Kraft sum exactly 1) whose lengths are all 1..=limit.

use vstd::prelude::*;

verus! {

/// The production `HuffmanError`.
pub enum HuffmanError {
    CodeTooLong,
    OverSubscribed,
    TooManySymbols,
    LengthMismatch,
    LimitTooSmall,
}

/// 2^n.
pub open spec fn p2(n: nat) -> nat
    decreases n,
{
    if n == 0 {
        1
    } else {
        2 * p2((n - 1) as nat)
    }
}

/// The Kraft sum of code lengths `d` in units of 2^-limit: the sum of
/// 2^(limit - d[i]). A prefix code is complete when this is 2^limit.
pub open spec fn kraft_sum(d: Seq<u32>, limit: nat) -> nat
    decreases d.len(),
{
    if d.len() == 0 {
        0
    } else {
        kraft_sum(d.drop_last(), limit) + p2((limit - d.last()) as nat)
    }
}

/// Every length lies in `lo..=hi`.
pub open spec fn within(d: Seq<u32>, lo: nat, hi: nat) -> bool {
    forall|i: int| 0 <= i < d.len() ==> lo <= d[i] && d[i] <= hi
}

/// What the scan for a code to lengthen has established after `k`
/// symbols.
pub open spec fn over_pick(d: Seq<u32>, k: int, limit: u32, pick: Option<usize>) -> bool {
    match pick {
        Some(p) => p < k && d[p as int] < limit,
        None => forall|j: int| 0 <= j < k ==> d[j] >= limit,
    }
}

/// What the scan for a code to shorten has established after `k`
/// symbols: the pick is a longest code of more than one bit.
pub open spec fn under_pick(d: Seq<u32>, k: int, pick: Option<usize>) -> bool {
    match pick {
        Some(p) => p < k && d[p as int] > 1 && forall|j: int|
            0 <= j < k && d[j] > 1 ==> d[j] <= d[p as int],
        None => forall|j: int| 0 <= j < k ==> d[j] <= 1,
    }
}

pub(crate) proof fn lemma_p2_pos(n: nat)
    ensures
        p2(n) >= 1,
    decreases n,
{
    if n > 0 {
        lemma_p2_pos((n - 1) as nat);
    }
}

pub(crate) proof fn lemma_p2_mono(a: nat, b: nat)
    requires
        a <= b,
    ensures
        p2(a) <= p2(b),
    decreases b,
{
    if a < b {
        lemma_p2_mono(a, (b - 1) as nat);
    }
}

pub(crate) proof fn lemma_p2_15()
    ensures
        p2(15) == 32768,
{
    reveal_with_fuel(p2, 16);
}

pub(crate) proof fn lemma_p2_add(a: nat, b: nat)
    ensures
        p2(a + b) == p2(a) * p2(b),
    decreases b,
{
    if b > 0 {
        lemma_p2_add(a, (b - 1) as nat);
        let x = p2(a);
        let y = p2((b - 1) as nat);
        assert(p2(a + b) == 2 * p2((a + b - 1) as nat));
        assert((a + b - 1) as nat == a + (b - 1) as nat);
        assert(2 * (x * y) == x * (2 * y)) by (nonlinear_arith);
    } else {
        assert(p2(a) * 1 == p2(a)) by (nonlinear_arith);
    }
}

/// `1 << s` is 2^s.
pub(crate) proof fn lemma_shl(s: u32)
    requires
        s < 63,
    ensures
        (1u64 << s) as nat == p2(s as nat),
    decreases s,
{
    if s == 0 {
        assert(1u64 << 0u32 == 1u64) by (bit_vector);
    } else {
        let t = (s - 1) as u32;
        lemma_shl(t);
        assert(1u64 << s == 2 * (1u64 << t)) by (bit_vector)
            requires
                t < 62,
                s == t + 1,
        ;
    }
}

pub(crate) proof fn lemma_kraft_take(d: Seq<u32>, i: int, limit: nat)
    requires
        0 <= i < d.len(),
    ensures
        kraft_sum(d.take(i + 1), limit) == kraft_sum(d.take(i), limit) + p2((limit - d[i]) as nat),
{
    assert(d.take(i + 1).drop_last() =~= d.take(i));
    assert(d.take(i + 1).last() == d[i]);
}

/// Changing one length changes the sum by the difference of the shares.
proof fn lemma_kraft_update(d: Seq<u32>, i: int, v: u32, limit: nat)
    requires
        0 <= i < d.len(),
    ensures
        kraft_sum(d.update(i, v), limit) + p2((limit - d[i]) as nat) == kraft_sum(d, limit) + p2(
            (limit - v) as nat,
        ),
    decreases d.len(),
{
    let u = d.update(i, v);
    if i == d.len() - 1 {
        assert(u.drop_last() =~= d.drop_last());
        assert(u.last() == v);
    } else {
        lemma_kraft_update(d.drop_last(), i, v, limit);
        assert(u.drop_last() =~= d.drop_last().update(i, v));
        assert(u.last() == d.last());
    }
}

/// The sum is at least any one share.
proof fn lemma_kraft_ge_term(d: Seq<u32>, i: int, limit: nat)
    requires
        0 <= i < d.len(),
    ensures
        kraft_sum(d, limit) >= p2((limit - d[i]) as nat),
    decreases d.len(),
{
    if i < d.len() - 1 {
        lemma_kraft_ge_term(d.drop_last(), i, limit);
        assert(d.drop_last()[i] == d[i]);
    }
}

/// With every length at the limit, the sum is the number of symbols.
proof fn lemma_kraft_all_at_limit(d: Seq<u32>, limit: nat)
    requires
        forall|i: int| 0 <= i < d.len() ==> d[i] == limit,
    ensures
        kraft_sum(d, limit) == d.len(),
    decreases d.len(),
{
    if d.len() > 0 {
        let s = d.drop_last();
        assert forall|i: int| 0 <= i < s.len() implies s[i] == limit by {
            assert(s[i] == d[i]);
        }
        lemma_kraft_all_at_limit(s, limit);
        assert(d.last() == d[d.len() - 1]);
        assert(p2(0) == 1);
    }
}

/// With every length 1 and at least two symbols, the code is full.
proof fn lemma_kraft_all_one(d: Seq<u32>, limit: nat)
    requires
        limit >= 1,
        d.len() >= 2,
        forall|i: int| 0 <= i < d.len() ==> d[i] == 1,
    ensures
        kraft_sum(d, limit) >= p2(limit),
    decreases d.len(),
{
    let h = p2((limit - 1) as nat);
    assert(p2(limit) == 2 * h);
    let s = d.drop_last();
    assert forall|i: int| 0 <= i < s.len() implies s[i] == 1 by {
        assert(s[i] == d[i]);
    }
    assert(d.last() == d[d.len() - 1]);
    if d.len() == 2 {
        assert(s.drop_last().len() == 0);
        assert(s.last() == s[0]);
        assert(kraft_sum(s, limit) == kraft_sum(s.drop_last(), limit) + h);
        assert(kraft_sum(d, limit) == 2 * h);
    } else {
        lemma_kraft_all_one(s, limit);
    }
}

/// With every length at most `dm`, every share is a multiple of the share
/// of a length-`dm` code, and so is the sum.
pub(crate) proof fn lemma_kraft_scale(d: Seq<u32>, dm: nat, limit: nat)
    requires
        dm <= limit,
        within(d, 0, dm),
    ensures
        kraft_sum(d, limit) == p2((limit - dm) as nat) * kraft_sum(d, dm),
    decreases d.len(),
{
    if d.len() > 0 {
        let s = d.drop_last();
        let x = d.last();
        assert(x == d[d.len() - 1]);
        assert forall|i: int| 0 <= i < s.len() implies 0 <= s[i] && s[i] <= dm by {
            assert(s[i] == d[i]);
        }
        lemma_kraft_scale(s, dm, limit);
        lemma_p2_add((limit - dm) as nat, (dm - x) as nat);
        assert((limit - dm) as nat + (dm - x) as nat == (limit - x) as nat);
        let u = p2((limit - dm) as nat);
        let a = kraft_sum(s, dm);
        let b = p2((dm - x) as nat);
        assert(u * a + u * b == u * (a + b)) by (nonlinear_arith);
        assert(u * (a + b) >= 0) by (nonlinear_arith)
            requires
                u >= 0,
                a + b >= 0,
        ;
    } else {
        assert(p2((limit - dm) as nat) * 0 == 0) by (nonlinear_arith);
    }
}

/// Each share is at most 2^15, so the sum is at most 2^15 per symbol.
proof fn lemma_kraft_bound(d: Seq<u32>, limit: nat)
    requires
        limit <= 15,
        within(d, 0, limit),
    ensures
        kraft_sum(d, limit) <= 32768 * d.len(),
    decreases d.len(),
{
    if d.len() > 0 {
        let s = d.drop_last();
        assert forall|i: int| 0 <= i < s.len() implies 0 <= s[i] && s[i] <= limit by {
            assert(s[i] == d[i]);
        }
        lemma_kraft_bound(s, limit);
        assert(d.last() == d[d.len() - 1]);
        lemma_p2_mono((limit - d.last()) as nat, 15);
        lemma_p2_15();
    }
}

/// With no code below the limit, every length is the limit and the sum is
/// the symbol count.
proof fn lemma_over_none(d: Seq<u32>, lim: nat)
    requires
        within(d, 1, lim),
        forall|j: int| 0 <= j < d.len() ==> d[j] >= lim,
    ensures
        kraft_sum(d, lim) == d.len(),
{
    assert forall|j: int| 0 <= j < d.len() implies d[j] == lim by {}
    lemma_kraft_all_at_limit(d, lim);
}

/// Lengthening a code below the limit by one bit frees half its share.
proof fn lemma_over_step(d: Seq<u32>, p: int, lim: nat)
    requires
        lim <= 15,
        0 <= p < d.len(),
        within(d, 1, lim),
        d[p] < lim,
    ensures
        within(d.update(p, (d[p] + 1) as u32), 1, lim),
        p2((lim - d[p] - 1) as nat) >= 1,
        kraft_sum(d, lim) >= p2((lim - d[p] - 1) as nat),
        kraft_sum(d.update(p, (d[p] + 1) as u32), lim) + p2((lim - d[p] - 1) as nat) == kraft_sum(
            d,
            lim,
        ),
{
    let h = (lim - d[p] - 1) as nat;
    assert((lim - d[p]) as nat == h + 1);
    assert(p2(h + 1) == 2 * p2(h));
    lemma_p2_pos(h);
    lemma_kraft_ge_term(d, p, lim);
    lemma_kraft_update(d, p, (d[p] + 1) as u32, lim);
    assert((lim - (d[p] + 1) as u32) as nat == h);
    let e = d.update(p, (d[p] + 1) as u32);
    assert forall|j: int| 0 <= j < e.len() implies 1 <= e[j] && e[j] <= lim by {
        if j != p {
            assert(e[j] == d[j]);
        }
    }
}

/// With no code longer than one bit and two or more symbols, the code is
/// already full.
proof fn lemma_under_none(d: Seq<u32>, lim: nat)
    requires
        lim >= 1,
        d.len() >= 2,
        within(d, 1, lim),
        forall|j: int| 0 <= j < d.len() ==> d[j] <= 1,
    ensures
        kraft_sum(d, lim) >= p2(lim),
{
    assert forall|j: int| 0 <= j < d.len() implies d[j] == 1 by {}
    lemma_kraft_all_one(d, lim);
}

/// Shortening a longest code by one bit adds its share u = 2^(limit-dm).
/// The sum and the full code are both multiples of u, so while the code is
/// short of full it stays short by at least u, and the step never
/// overshoots.
proof fn lemma_under_step(d: Seq<u32>, p: int, lim: nat)
    requires
        lim <= 15,
        0 <= p < d.len(),
        within(d, 1, lim),
        d[p] > 1,
        forall|j: int| 0 <= j < d.len() && d[j] > 1 ==> d[j] <= d[p],
        kraft_sum(d, lim) < p2(lim),
    ensures
        within(d.update(p, (d[p] - 1) as u32), 1, lim),
        p2((lim - d[p]) as nat) >= 1,
        kraft_sum(d.update(p, (d[p] - 1) as u32), lim) == kraft_sum(d, lim) + p2(
            (lim - d[p]) as nat,
        ),
        kraft_sum(d, lim) + p2((lim - d[p]) as nat) <= p2(lim),
{
    let dm = d[p] as nat;
    let h = (lim - dm) as nat;
    assert forall|j: int| 0 <= j < d.len() implies 0 <= d[j] && d[j] <= dm by {
        if d[j] > 1 {
        } else {
        }
    }
    lemma_kraft_scale(d, dm, lim);
    lemma_p2_add(h, dm);
    assert(h + dm == lim);
    let u = p2(h);
    let a = kraft_sum(d, dm);
    let b = p2(dm);
    lemma_p2_pos(h);
    assert(kraft_sum(d, lim) == u * a);
    assert(p2(lim) == u * b);
    assert(u * a + u <= u * b) by (nonlinear_arith)
        requires
            u >= 1,
            u * a < u * b,
    ;
    lemma_kraft_update(d, p, (d[p] - 1) as u32, lim);
    assert((lim - (d[p] - 1) as u32) as nat == h + 1);
    assert(p2(h + 1) == 2 * p2(h));
    let e = d.update(p, (d[p] - 1) as u32);
    assert forall|j: int| 0 <= j < e.len() implies 1 <= e[j] && e[j] <= lim by {
        if j != p {
            assert(e[j] == d[j]);
        }
    }
}

#[allow(dead_code)]
#[verifier::loop_isolation(false)]
#[verifier::spinoff_prover]
pub(crate) fn limit_depths(depth: &mut [u32], weight: &[u64], limit: usize) -> (result: Result<(), HuffmanError>)
    requires // @ghost
        1 <= limit <= 15, // @ghost
        2 <= old(depth)@.len() <= p2(limit as nat), // @ghost
        weight@.len() == old(depth)@.len(), // @ghost
        forall|i: int| 0 <= i < old(depth)@.len() ==> old(depth)@[i] >= 1, // @ghost
    ensures // @ghost
        result is Ok, // @ghost
        final(depth)@.len() == old(depth)@.len(), // @ghost
        within(final(depth)@, 1, limit as nat), // @ghost
        kraft_sum(final(depth)@, limit as nat) == p2(limit as nat), // @ghost
{
    let m = depth.len();
    // Kraft sum in units of 2^-limit; a complete code sums to `full`.
    let limit32 = limit as u32;
    let full = 1u64 << limit32;
    let ghost lim = limit as nat; // @ghost
    proof { // @ghost-begin
        lemma_shl(limit32);
        lemma_p2_mono(lim, 15);
        lemma_p2_15();
    } // @ghost-end
    let mut kraft = 0u64;
    for i in 0..m
        invariant // @ghost
            depth@.len() == m, // @ghost
            forall|j: int| 0 <= j < i ==> 1 <= depth@[j] && depth@[j] <= lim, // @ghost
            forall|j: int| i <= j < m ==> depth@[j] == old(depth)@[j], // @ghost
            kraft as nat == kraft_sum(depth@.take(i as int), lim), // @ghost
            kraft <= 32768 * i, // @ghost
    {
        let ghost before = depth@; // @ghost
        if depth[i] > limit32 {
            depth[i] = limit32;
        }
        proof { // @ghost-begin
            assert(depth@.take(i as int) =~= before.take(i as int));
            lemma_shl((limit32 - depth@[i as int]) as u32);
            lemma_p2_mono((lim - depth@[i as int]) as nat, 15);
            lemma_kraft_take(depth@, i as int, lim);
        } // @ghost-end
        kraft += 1u64 << (limit32 - depth[i]);
    }
    proof { // @ghost-begin
        assert(depth@.take(m as int) =~= depth@);
        assert(within(depth@, 1, lim));
    } // @ghost-end
    // Over-subscribed: lengthen the least frequent of the longest codes
    // that are still below the limit. Each step frees 2^-(len+1).
    while kraft > full
        invariant // @ghost
            depth@.len() == m, // @ghost
            within(depth@, 1, lim), // @ghost
            kraft as nat == kraft_sum(depth@, lim), // @ghost
            kraft <= 32768 * m, // @ghost
        decreases kraft, // @ghost
    {
        let mut pick: Option<usize> = None;
        for k in 0..m
            invariant // @ghost
                over_pick(depth@, k as int, limit32, pick), // @ghost
        {
            if depth[k] < limit32 {
                let better = match pick {
                    None => true,
                    Some(p) => depth[k] > depth[p] || (depth[k] == depth[p] && weight[k] < weight[p]),
                };
                if better {
                    pick = Some(k);
                }
            }
        }
        proof { // @ghost-begin
            // No code below the limit: every length is the limit, and the
            // sum is the symbol count, at most 2^limit. Not over-subscribed.
            if pick is None {
                lemma_over_none(depth@, lim);
            }
        } // @ghost-end
        let p = match pick {
            Some(p) => p,
            None => return Err(HuffmanError::LimitTooSmall),
        };
        let ghost before = depth@; // @ghost
        proof { // @ghost-begin
            lemma_shl((limit32 - depth@[p as int] - 1) as u32);
            lemma_over_step(depth@, p as int, lim);
        } // @ghost-end
        kraft -= 1u64 << (limit32 - depth[p] - 1);
        depth[p] += 1;
        proof { // @ghost-begin
            assert(depth@ =~= before.update(p as int, (before[p as int] + 1) as u32));
        } // @ghost-end
    }
    // Under-subscribed: shorten the most frequent of the longest codes.
    // The deficit is a multiple of that code's share, so this lands
    // exactly on a complete code.
    while kraft < full
        invariant // @ghost
            depth@.len() == m, // @ghost
            within(depth@, 1, lim), // @ghost
            kraft as nat == kraft_sum(depth@, lim), // @ghost
            kraft <= full, // @ghost
        decreases full - kraft, // @ghost
    {
        let mut pick: Option<usize> = None;
        for k in 0..m
            invariant // @ghost
                under_pick(depth@, k as int, pick), // @ghost
        {
            if depth[k] > 1 {
                let better = match pick {
                    None => true,
                    Some(p) => depth[k] > depth[p] || (depth[k] == depth[p] && weight[k] > weight[p]),
                };
                if better {
                    pick = Some(k);
                }
            }
        }
        proof { // @ghost-begin
            // Every length 1 with two or more symbols is already full.
            if pick is None {
                lemma_under_none(depth@, lim);
            }
        } // @ghost-end
        let p = match pick {
            Some(p) => p,
            None => return Err(HuffmanError::LimitTooSmall),
        };
        let ghost before = depth@; // @ghost
        proof { // @ghost-begin
            lemma_shl((limit32 - depth@[p as int]) as u32);
            lemma_under_step(depth@, p as int, lim);
        } // @ghost-end
        kraft += 1u64 << (limit32 - depth[p]);
        depth[p] -= 1;
        proof { // @ghost-begin
            assert(depth@ =~= before.update(p as int, (before[p as int] - 1) as u32));
        } // @ghost-end
    }
    Ok(())
}

} // verus!
