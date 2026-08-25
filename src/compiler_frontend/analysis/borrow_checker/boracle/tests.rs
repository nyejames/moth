//! Smoke tests for the feature-gated boracle seam.

#[test]
fn boracle_feature_marker_is_present() {
    assert_eq!(super::BORACLE_FEATURE_MARKER, "boracle");
}
