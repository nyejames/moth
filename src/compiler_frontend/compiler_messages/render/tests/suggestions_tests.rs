//! Closest-name suggestion policy tests.
//!
//! WHAT: pins order-independent equal-distance ties and the shared-character rule.
//! WHY: renderers share one threshold owner; candidate order and short unrelated
//! names must not change the chosen suggestion.

use crate::compiler_frontend::compiler_messages::render::closest_name_suggestion_with_extra;
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn closest_name_suggestion_is_deterministic_and_requires_a_shared_character() {
    let string_table = StringTable::new();
    let first = closest_name_suggestion_with_extra("ab", &[], ["ac", "aa"], &string_table);
    let second = closest_name_suggestion_with_extra("ab", &[], ["aa", "ac"], &string_table);
    assert_eq!(first.as_deref(), Some("aa"));
    assert_eq!(second, first);

    let suggestion = closest_name_suggestion_with_extra("T", &[], ["I8", "U8"], &string_table);
    assert_eq!(suggestion, None);
}
