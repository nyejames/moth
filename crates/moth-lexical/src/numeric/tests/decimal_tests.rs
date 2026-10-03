//! Canonical `Dec` scale identity tests.

use crate::numeric::decimal::NumberScale;
#[test]
fn canonical_scale_names_accept_only_boundaries() {
    for (spelling, expected_scale) in [("Dec", 0), ("Dec0", 0), ("Dec1", 1), ("Dec256", 256)] {
        assert_eq!(
            NumberScale::from_name(spelling).map(NumberScale::get),
            Some(expected_scale)
        );
    }

    // Invalid spellings stay named types: leading zeroes, above the canonical capacity,
    // non-digit suffixes and trailing-letter names never parse as a scale.
    for invalid in ["Dec01", "Dec00", "Dec257", "Dec+1", "DecBox", "decimal"] {
        assert_eq!(NumberScale::from_name(invalid), None);
    }

    // The retired `Number` family spellings are ordinary named types now.
    for retired in [
        "Number",
        "Number0",
        "Number1",
        "Number2",
        "Number256",
        "NumberBox",
    ] {
        assert_eq!(NumberScale::from_name(retired), None, "{retired:?}");
    }
}
