//! Behavior-level parser tests for the JS `@moth.*` annotation scanner.
//!
//! WHAT: asserts parser output (opaque types, functions, diagnostics) for valid and
//!       invalid JS source snippets.
//! WHY: these tests exercise the full parser stack, not private helper internals.

mod parser_tests;
