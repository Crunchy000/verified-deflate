//! The ZIP entry-name check (`zip::name_is_safe` in the library, behind
//! `validate_name`), proved for names of every length.
//!
//! The executable code below is the library's, token for token (CI checks
//! this with `.github/scripts/verus_drift.py`); every line marked
//! `// @ghost` is specification or proof. The theorem: the check accepts a
//! byte string exactly when it meets the rule `safe`, which is stated
//! position by position and independently of the implementation (it is the
//! rule the Kani harness `validate_name_is_exactly_the_rule` checks for
//! names of up to 8 bytes).

use vstd::prelude::*;

verus! {

/// An ASCII letter, as in a drive prefix `C:`.
pub open spec fn is_letter(c: u8) -> bool {
    (b'a' <= c && c <= b'z') || (b'A' <= c && c <= b'Z')
}

/// Position `i` starts a component.
pub open spec fn starts(b: Seq<u8>, i: int) -> bool {
    i == 0 || b[i - 1] == b'/'
}

/// A component that reaches position `j` ends there.
pub open spec fn ends_at(b: Seq<u8>, j: int) -> bool {
    j == b.len() || b[j] == b'/'
}

/// Position `i` makes the name unsafe: a backslash or NUL; a component
/// `.` or `..` starting there; or a `/` that starts a component, which is
/// a leading `/` or an empty component (`//`, or a second trailing `/`).
pub open spec fn bad_at(b: Seq<u8>, i: int) -> bool {
    b[i] == b'\\' || b[i] == 0 || (starts(b, i) && b[i] == b'.' && ends_at(b, i + 1)) || (starts(b, i)
        && b[i] == b'.' && i + 1 < b.len() && b[i + 1] == b'.' && ends_at(b, i + 2)) || (b[i] == b'/'
        && starts(b, i))
}

/// The rule: non-empty, fits the 16-bit length field, no drive prefix, and
/// no unsafe position. A name meeting it is relative and cannot climb out
/// of the directory it is extracted into.
pub open spec fn safe(b: Seq<u8>) -> bool {
    &&& 0 < b.len() <= 0xFFFF
    &&& !(b.len() >= 2 && b[1] == b':' && is_letter(b[0]))
    &&& forall|i: int| 0 <= i < b.len() ==> !bad_at(b, i)
}

/// A finished component b[s..e]: non-empty, not `.` or `..`, free of `/`,
/// NUL and backslash, starting the name or after a `/`, and ending at the
/// end of the name or at a `/`. None of its positions, nor the `/` that
/// ends it, is unsafe.
proof fn lemma_component(b: Seq<u8>, s: int, e: int)
    requires
        0 <= s < e <= b.len(),
        starts(b, s),
        ends_at(b, e),
        forall|j: int| s <= j < e ==> b[j] != b'/' && b[j] != 0 && b[j] != b'\\',
        !(e - s == 1 && b[s] == b'.'),
        !(e - s == 2 && b[s] == b'.' && b[s + 1] == b'.'),
    ensures
        forall|j: int| s <= j <= e && j < b.len() ==> !bad_at(b, j),
{
    assert forall|j: int| s <= j <= e && j < b.len() implies !bad_at(b, j) by {
        if j == e {
            assert(b[e - 1] != b'/');
        } else if j > s {
            assert(b[j - 1] != b'/');
        } else {
            if e - s >= 2 {
                assert(b[s + 1] != b'/');
            }
            if e - s >= 3 {
                assert(b[s + 2] != b'/');
            }
        }
    }
}

#[allow(dead_code)]
#[allow(clippy::manual_range_contains)]
#[verifier::loop_isolation(false)]
fn name_is_safe(b: &[u8]) -> (r: bool)
    ensures // @ghost
        r == safe(b@), // @ghost
{
    let n = b.len();
    if n == 0 || n > 0xFFFF {
        return false;
    }
    // A drive prefix such as `C:`.
    if n >= 2 && b[1] == b':' && ((b'a' <= b[0] && b[0] <= b'z') || (b'A' <= b[0] && b[0] <= b'Z')) {
        return false;
    }
    // The components run up to `end`; a single trailing `/` marks a
    // directory.
    let end = if b[n - 1] == b'/' { n - 1 } else { n };
    if end == 0 {
        proof { // @ghost-begin
            assert(bad_at(b@, 0));
        } // @ghost-end
        return false;
    }
    let mut start = 0;
    let mut i = 0;
    while i <= end
        invariant // @ghost
            start <= i <= end + 1, // @ghost
            0 < start <= n ==> b@[start - 1] == b'/', // @ghost
            i == end + 1 ==> start == end + 1, // @ghost
            forall|j: int| start <= j < i ==> b@[j] != b'/' && b@[j] != 0 && b@[j] != b'\\', // @ghost
            forall|j: int| 0 <= j < start && j < n ==> !bad_at(b@, j), // @ghost
        decreases end + 1 - i, // @ghost
    {
        if i == end || b[i] == b'/' {
            // The component b[start..i] is complete.
            let len = i - start;
            proof { // @ghost-begin
                assert(ends_at(b@, i as int));
                assert(starts(b@, start as int));
            } // @ghost-end
            if len == 0 || (len == 1 && b[start] == b'.') || (len == 2 && b[start] == b'.' && b[start + 1] == b'.') {
                proof { // @ghost-begin
                    // An empty component (its `/` is unsafe; the name
                    // cannot end in one, as `end` excludes a final `/`),
                    // or `.` or `..` at `start`.
                    if len == 0 {
                        assert(bad_at(b@, i as int));
                    } else {
                        assert(bad_at(b@, start as int));
                    }
                } // @ghost-end
                return false;
            }
            proof { // @ghost-begin
                lemma_component(b@, start as int, i as int);
            } // @ghost-end
            start = i + 1;
        } else if b[i] == 0 || b[i] == b'\\' {
            proof { // @ghost-begin
                assert(bad_at(b@, i as int));
            } // @ghost-end
            return false;
        }
        i += 1;
    }
    true
}

} // verus!
