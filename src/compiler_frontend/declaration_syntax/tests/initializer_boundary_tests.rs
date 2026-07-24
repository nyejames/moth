//! Boundary-retention tests for declaration-shell initializer collection.
//!
//! WHAT: verifies that incomplete inline value-if tails retain their real boundary,
//! and that declarations missing an initializer are rejected with the correct
//! structured diagnostic at the real source boundary.
//! WHY: AST otherwise appends a synthetic EOF at the declaration location, erasing
//! multiline context and the source location of authored closing tokens, and the
//! shell must distinguish an authored `=` with no initializer from an omitted `=`.

use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, InvalidDeclarationReason,
    RuleDiagnosticKind,
};
use crate::compiler_frontend::declaration_syntax::declaration_shell::parse_declaration_syntax;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind, TokenizerEntryMode};

/// Returns a stable label without exposing literal payload details.
fn label(kind: &TokenKind) -> &'static str {
    match kind {
        TokenKind::If => "If",
        TokenKind::Then => "Then",
        TokenKind::Else => "Else",
        TokenKind::Newline => "Newline",
        TokenKind::Eof => "Eof",
        TokenKind::End => "End",
        TokenKind::Comma => "Comma",
        TokenKind::Add => "Add",
        TokenKind::BoolLiteral(_) => "BoolLiteral",
        TokenKind::NumericLiteral(_) => "NumericLiteral",
        other => {
            panic!("unexpected initializer token kind in boundary test: {other:?}")
        }
    }
}

/// Parses the initializer token slice for a top-level `value = ...` declaration.
fn parse_shell(source: &str) -> Vec<&'static str> {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("test.moth", &mut string_table);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut token_stream = tokenize(
        source,
        &source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        None,
    )
    .expect("tokenization should succeed");

    let name = string_table.intern("value");
    token_stream.index = 2; // skip ModuleStart and the declaration name, land on `=`

    let declaration_syntax = parse_declaration_syntax(&mut token_stream, name, &mut string_table)
        .expect("declaration shell should parse");

    declaration_syntax
        .initializer_tokens
        .iter()
        .map(|token| label(&token.kind))
        .collect()
}

#[test]
fn retains_eof_boundary_after_trailing_then() {
    let kinds = parse_shell("value = if true then");

    assert_eq!(kinds.len(), 4, "expected if, true, then and retained EOF");
    assert_eq!(kinds[0], "If");
    assert_eq!(kinds[1], "BoolLiteral");
    assert_eq!(kinds[2], "Then");
    assert_eq!(
        kinds[3], "Eof",
        "trailing then at EOF must retain the real EOF"
    );
}

#[test]
fn retains_eof_boundary_after_trailing_else() {
    let kinds = parse_shell("value = if true then 1 else");

    assert_eq!(
        kinds.len(),
        6,
        "expected if, true, then, 1, else and retained EOF",
    );
    assert_eq!(kinds[4], "Else");
    assert_eq!(
        kinds[5], "Eof",
        "trailing else at EOF must retain the real EOF"
    );
}

#[test]
fn retains_newline_boundary_after_trailing_then() {
    let kinds = parse_shell("value = if true then\n");

    assert_eq!(
        kinds.len(),
        4,
        "expected if, true, then and retained newline"
    );
    assert_eq!(kinds[2], "Then");
    assert_eq!(
        kinds[3], "Newline",
        "trailing then before a newline must retain it",
    );
}

#[test]
fn retains_newline_boundary_after_trailing_else() {
    let kinds = parse_shell("value = if true then 1 else\n");

    assert_eq!(
        kinds.len(),
        6,
        "expected if, true, then, 1, else and retained newline",
    );
    assert_eq!(kinds[4], "Else");
    assert_eq!(
        kinds[5], "Newline",
        "trailing else before a newline must retain it",
    );
}

#[test]
fn retains_eof_boundary_when_inline_value_if_is_missing_else() {
    let kinds = parse_shell("value = if true then 1");

    assert_eq!(
        kinds,
        ["If", "BoolLiteral", "Then", "NumericLiteral", "Eof"],
        "a complete then branch must retain the real boundary where else is missing",
    );
}

#[test]
fn retains_newline_boundary_when_inline_value_if_is_missing_else() {
    let kinds = parse_shell("value = if true then 1\n");

    assert_eq!(
        kinds,
        ["If", "BoolLiteral", "Then", "NumericLiteral", "Newline",],
        "a complete then branch must retain the authored newline where else is missing",
    );
}

#[test]
fn collects_authored_multiline_else_with_the_incomplete_inline_value_if() {
    let kinds = parse_shell("value = if true then 1\nelse 0\n");

    assert_eq!(
        kinds,
        [
            "If",
            "BoolLiteral",
            "Then",
            "NumericLiteral",
            "Newline",
            "Else",
            "NumericLiteral",
        ],
        "the AST must receive the newline and authored else together",
    );
}

#[test]
fn ordinary_initializer_slice_is_unchanged() {
    let kinds = parse_shell("value = if true then 1 else 0\n");

    assert_eq!(
        kinds.len(),
        6,
        "ordinary initializer must not retain a boundary token",
    );
    assert_eq!(
        kinds[5], "NumericLiteral",
        "ordinary initializer must end at the last value",
    );
}

#[test]
fn retains_end_boundary_after_trailing_then() {
    let kinds = parse_shell("value = if true then;");

    assert_eq!(
        kinds.len(),
        4,
        "expected if, true, then and retained block end"
    );
    assert_eq!(kinds[2], "Then");
    assert_eq!(kinds[3], "End");
}

#[test]
fn retains_comma_boundary_after_trailing_else() {
    let kinds = parse_shell("value = if true then 1 else,");

    assert_eq!(
        kinds.len(),
        6,
        "expected if, true, then, value, else and retained comma"
    );
    assert_eq!(kinds[4], "Else");
    assert_eq!(kinds[5], "Comma");
}

#[test]
fn non_control_flow_initializer_is_unchanged() {
    let kinds = parse_shell("value = 1 + 2\n");

    assert_eq!(kinds.len(), 3);
    assert_eq!(kinds[0], "NumericLiteral");
    assert_eq!(kinds[1], "Add");
    assert_eq!(kinds[2], "NumericLiteral");
}

// Missing declaration initializer diagnostics.
//
// An authored `=` with no initializer is structurally distinct from a declaration
// that omits `=` entirely: the first stopped after a real boundary token, the second
// never saw `=`. They use separate structured reasons and anchor at the boundary
// where the initializer is missing, not at the declaration name or target type.

/// Tokenizes `source` once and positions the stream just after the declaration name.
///
/// Returns the shared string table, the stream, the interned declaration name, and the
/// index of the first `Assign` token (when present) so callers can compute the boundary
/// location the shell must point at.
fn tokenize_for_declaration(source: &str) -> (StringTable, FileTokens, StringId, Option<usize>) {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("test.moth", &mut string_table);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let token_stream = tokenize(
        source,
        &source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        None,
    )
    .expect("tokenization should succeed");

    let name = string_table.intern("value");
    let assign_index = token_stream
        .tokens
        .iter()
        .position(|token| token.kind == TokenKind::Assign);

    (string_table, token_stream, name, assign_index)
}

/// Parses the declaration shell and returns the rejection diagnostic plus the interned
/// declaration name so callers can assert the structured payload facts.
fn parse_shell_error(source: &str) -> (CompilerDiagnostic, StringId) {
    let (mut string_table, mut token_stream, name, _) = tokenize_for_declaration(source);
    token_stream.index = 2; // skip ModuleStart and the declaration name

    let diagnostic = *parse_declaration_syntax(&mut token_stream, name, &mut string_table)
        .expect_err("declaration shell should reject the missing initializer");
    (diagnostic, name)
}

/// Location of the token immediately after the first authored `=`.
///
/// This is the real newline/end/EOF/comma boundary the diagnostic must anchor against.
fn boundary_location_after_assign(source: &str) -> SourceLocation {
    let (_string_table, token_stream, _name, assign_index) = tokenize_for_declaration(source);
    let assign_index = assign_index.expect("source must contain an authored '='");
    token_stream.tokens[assign_index + 1].location.clone()
}

/// Location of the first top-level boundary token (newline/end/EOF/comma) starting after
/// the declaration name. Used for the no-`=` path, which never sees an authored `=`.
fn first_boundary_location_after_name(source: &str) -> SourceLocation {
    let (_string_table, token_stream, _name, _) = tokenize_for_declaration(source);
    token_stream
        .tokens
        .iter()
        .skip(2)
        .find(|token| {
            matches!(
                token.kind,
                TokenKind::Newline | TokenKind::End | TokenKind::Eof | TokenKind::Comma
            )
        })
        .expect("source must contain a declaration boundary")
        .location
        .clone()
}

/// Asserts the diagnostic is the authored-`=` rejection, names the declaration, and
/// points at the given boundary rather than the declaration name or target type.
fn assert_missing_initializer_expression(
    diagnostic: &CompilerDiagnostic,
    expected_name: StringId,
    expected_boundary: &SourceLocation,
    case: &str,
) {
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::InvalidDeclaration),
        "{case}: authored `=` with no initializer must use the InvalidDeclaration kind",
    );
    assert_eq!(
        diagnostic.kind.code(),
        "MOTH-RULE-0043",
        "{case}: authored `=` with no initializer must keep the InvalidDeclaration stable code",
    );
    match &diagnostic.payload {
        DiagnosticPayload::InvalidDeclaration { reason, name } => {
            assert_eq!(
                *reason,
                InvalidDeclarationReason::MissingInitializerExpression,
                "{case}: authored `=` with no initializer must carry the MissingInitializerExpression reason",
            );
            assert_eq!(
                *name,
                Some(expected_name),
                "{case}: authored `=` with no initializer must name the declaration",
            );
        }
        other => panic!("{case}: expected InvalidDeclaration payload, got {other:?}"),
    }
    assert_eq!(
        &diagnostic.primary_location, expected_boundary,
        "{case}: authored `=` with no initializer must point at the real boundary after `=`, not the declaration name or target type",
    );
}

/// Asserts the diagnostic is the no-`=` rejection: the declaration omitted `=` entirely
/// and must use the renamed `MissingDeclarationInitializer` kind and stable code.
fn assert_missing_declaration_initializer(
    diagnostic: &CompilerDiagnostic,
    expected_name: StringId,
    expected_boundary: &SourceLocation,
    case: &str,
) {
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::MissingDeclarationInitializer),
        "{case}: a declaration that omits `=` must use the MissingDeclarationInitializer kind",
    );
    assert_eq!(
        diagnostic.kind.code(),
        "MOTH-RULE-0031",
        "{case}: a declaration that omits `=` must keep the MOTH-RULE-0031 stable code",
    );
    match &diagnostic.payload {
        DiagnosticPayload::MissingDeclarationInitializer { name } => {
            assert_eq!(
                *name, expected_name,
                "{case}: the no-`=` diagnostic must name the declaration",
            );
        }
        other => panic!("{case}: expected MissingDeclarationInitializer payload, got {other:?}"),
    }
    assert_eq!(
        &diagnostic.primary_location, expected_boundary,
        "{case}: the no-`=` diagnostic must point at the boundary where `=` was expected",
    );
}

#[test]
fn authored_assign_with_no_initializer_rejects_inferred_declarations_at_each_boundary() {
    let cases = [
        ("value =\n", "inferred declaration at newline boundary"),
        ("value =", "inferred declaration at EOF boundary"),
        ("value =;", "inferred declaration at end boundary"),
        ("value =,", "inferred declaration at comma boundary"),
    ];

    for (source, case) in cases {
        let (diagnostic, name) = parse_shell_error(source);
        let boundary = boundary_location_after_assign(source);
        assert_missing_initializer_expression(&diagnostic, name, &boundary, case);
    }
}

#[test]
fn authored_assign_with_no_initializer_rejects_explicit_type_and_binding_modes() {
    let cases = [
        ("value Int =\n", "explicit type at newline boundary"),
        ("value Int =", "explicit type at EOF boundary"),
        ("value ~=\n", "mutable binding"),
        ("value #=\n", "compile-time binding"),
        ("value $Int =\n", "reactive binding"),
    ];

    for (source, case) in cases {
        let (diagnostic, name) = parse_shell_error(source);
        let boundary = boundary_location_after_assign(source);
        assert_missing_initializer_expression(&diagnostic, name, &boundary, case);
    }
}

#[test]
fn omitted_assign_rejects_at_each_boundary_and_binding_mode() {
    let cases = [
        ("value\n", "inferred declaration at newline boundary"),
        ("value Int", "explicit type at EOF boundary"),
        ("value $Int", "reactive binding"),
    ];

    for (source, case) in cases {
        let (diagnostic, name) = parse_shell_error(source);
        let boundary = first_boundary_location_after_name(source);
        assert_missing_declaration_initializer(&diagnostic, name, &boundary, case);
    }
}
