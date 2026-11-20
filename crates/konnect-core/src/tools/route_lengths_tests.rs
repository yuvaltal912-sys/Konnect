//! Tests of the length plan of a fastroute call (`route_lengths.rs`). Stage 1 holds the
//! skew function; Stage 2 adds the rest.

use super::*;

#[test]
fn fastroute_is_given_half_of_a_declared_skew() {
    // L5 and L13: fastroute leaves alone a pair already inside its own window, so it
    // gets half and keeps the other half as the margin for the pad-edge gap.
    assert_eq!(
        FASTROUTE_WINDOW_SHARE, 0.5,
        "the share is pinned by this test"
    );
    assert_eq!(fastroute_skew_mm(0.3), 0.15);
    assert_eq!(fastroute_skew_mm(0.05), 0.025);
    assert_eq!(fastroute_skew_mm(1.0), 0.5);
    // The result is strictly inside the declared maximum, whatever it is.
    for max in [0.001, 0.05, 0.1, 2.5, 100.0] {
        let given = fastroute_skew_mm(max);
        assert!(given > 0.0 && given < max, "{max}: {given}");
    }
}
