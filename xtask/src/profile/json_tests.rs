//! Focused contracts for profile-owned JSON string escaping.

use super::*;

#[test]
fn escape_uses_serde_json_string_rules() {
    assert_eq!(escape("with \"quotes\"\n"), "with \\\"quotes\\\"\\n");
}
