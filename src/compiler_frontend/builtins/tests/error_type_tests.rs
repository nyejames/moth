//! Builtin error type manifest unit tests.
//!
//! WHAT: validates reserved-symbol enforcement for the canonical builtin error surface.
//! WHY: solidified canonical registration facts (declaration shape, field order, `code`
//! default and type identity) are consumer-visible through the genuine parse boundary in
//! `ast/statements/tests/struct_parsing_tests.rs::parses_builtin_error_with_default_code_field`,
//! so only the reservation policy is pinned here instead of raw manifest copies.

use crate::compiler_frontend::builtins::error_type::{ERROR_TYPE_NAME, is_reserved_builtin_symbol};

#[test]
fn reserves_builtin_error_symbol_names() {
    assert!(is_reserved_builtin_symbol(ERROR_TYPE_NAME));
    assert!(!is_reserved_builtin_symbol("ErrorKind"));
    assert!(!is_reserved_builtin_symbol("ErrorLocation"));
    assert!(!is_reserved_builtin_symbol("StackFrame"));
    assert!(!is_reserved_builtin_symbol("UserError"));
}
