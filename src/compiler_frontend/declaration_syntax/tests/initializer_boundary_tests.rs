//! Boundary-retention tests for declaration-shell initializer collection.
//!
//! WHAT: verifies that incomplete inline value-if tails retain their real boundary,
//! and that declarations missing an initializer are rejected with the correct
//! structured diagnostic at the real source boundary.
//! WHY: AST otherwise appends a synthetic EOF at the declaration span, erasing
//! multiline context and the source span of authored closing tokens, and the
//! shell must distinguish an authored `=` with no initializer from an omitted `=`.

use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::{
    InvalidDeclarationCode, InvalidDeclarationView, MissingDeclarationInitializerView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::{
    CompactDiagnosticBagForTest, compact_bag_for_test,
};
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, DiagnosticCategory};
use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::declaration_syntax::declaration_shell::parse_declaration_syntax;
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceSpan};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::lexer::{LexedSource, tokenize};
use crate::compiler_frontend::tokenizer::tokens::{TokenIndex, TokenTag, TokenizerEntryMode};

/// Returns a stable label without exposing literal payload details.
fn label(tag: TokenTag) -> &'static str {
    match tag {
        TokenTag::IF => "If",
        TokenTag::THEN => "Then",
        TokenTag::ELSE => "Else",
        TokenTag::NEWLINE => "Newline",
        TokenTag::EOF => "Eof",
        TokenTag::END => "End",
        TokenTag::COMMA => "Comma",
        TokenTag::ADD => "Add",
        TokenTag::BOOL_LITERAL => "BoolLiteral",
        TokenTag::NUMERIC_LITERAL => "NumericLiteral",
        other => panic!("unexpected initializer token tag in boundary test: {other:?}"),
    }
}

/// Parses the initializer token slice for a top-level `value = ...` declaration.
fn parse_shell(source: &str) -> Vec<&'static str> {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut span_builder = ExtendedSpanBuilder::new();
    let token_stream = tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        &mut path_fork,
        crate::compiler_frontend::source::SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("tokenization should succeed");
    let name = string_table.intern("value");
    let full_range = token_stream
        .tokens
        .full_range()
        .expect("tokenized test stream must expose a checked range");
    let mut canonical_cursor = token_stream
        .tokens
        .cursor(full_range)
        .expect("tokenized test stream must expose canonical tokens");
    canonical_cursor
        .set_position(TokenIndex::from_raw(2))
        .expect("tokenized test stream must seek to the initializer");
    let mut declaration_cursor = DeclarationCursor::new(canonical_cursor);
    let declaration_syntax = parse_declaration_syntax(
        &mut declaration_cursor,
        name,
        &mut string_table,
        &mut span_builder,
    )
    .expect("declaration shell should parse");
    let initializer_range = declaration_syntax
        .initializer_range
        .expect("test declaration should retain initializer range");
    let mut initializer_cursor = token_stream
        .tokens
        .cursor(initializer_range)
        .expect("initializer range should resolve through canonical tokens");
    let mut labels = Vec::new();
    while let Some(token) = initializer_cursor.advance() {
        labels.push(label(token.tag()));
        if token.is_eof() {
            break;
        }
    }
    labels
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
/// span the shell must point at. The caller owns `span_builder` and keeps it alive wherever the returned
/// token spans are still resolved.
fn tokenize_for_declaration(
    source: &str,
    span_builder: &mut ExtendedSpanBuilder,
) -> (StringTable, LexedSource, StringId, Option<usize>) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let token_stream = tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        &mut path_fork,
        crate::compiler_frontend::source::SourceId::COMPILATION_ROOT,
        span_builder,
    )
    .expect("tokenization should succeed");

    let name = string_table.intern("value");
    let assign_index = (0..token_stream.tokens.len()).find_map(|index| {
        let index = TokenIndex::try_from_index(index)?;
        let token = token_stream.tokens.token(index).ok()?;
        (token.tag() == TokenTag::ASSIGN).then_some(index.index())
    });

    (string_table, token_stream, name, assign_index)
}

/// Parses the declaration shell and returns the rejection diagnostic plus the interned
/// declaration name so callers can assert the structured payload facts.
fn parse_shell_error(source: &str) -> (CompactDiagnosticBagForTest, StringId) {
    let mut span_builder = ExtendedSpanBuilder::new();
    let (mut string_table, token_stream, name, _) =
        tokenize_for_declaration(source, &mut span_builder);
    let full_range = token_stream
        .tokens
        .full_range()
        .expect("tokenized test stream must expose a checked range");
    let mut canonical_cursor = token_stream
        .tokens
        .cursor(full_range)
        .expect("tokenized test stream must expose canonical tokens");
    canonical_cursor
        .set_position(TokenIndex::from_raw(2))
        .expect("tokenized test stream must seek to the declaration initializer");
    let mut declaration_cursor = DeclarationCursor::new(canonical_cursor);
    let failure = parse_declaration_syntax(
        &mut declaration_cursor,
        name,
        &mut string_table,
        &mut span_builder,
    )
    .expect_err("declaration shell should reject the missing initializer");
    let diagnostic = match failure {
        HeaderParseFailure::Diagnostic(diagnostic) => diagnostic,
        HeaderParseFailure::Infrastructure(error) => {
            panic!("declaration shell infrastructure failure: {error:?}")
        }
    };
    let diagnostics = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    (diagnostics, name)
}

/// Span of the token immediately after the first authored `=`.
///
/// This is the real newline/end/EOF/comma boundary the diagnostic must anchor against.
fn boundary_span_after_assign(source: &str) -> SourceSpan {
    let mut span_builder = ExtendedSpanBuilder::new();
    let (_string_table, token_stream, _name, assign_index) =
        tokenize_for_declaration(source, &mut span_builder);
    let assign_index = assign_index.expect("source must contain an authored '='");
    SourceSpan::new(
        token_stream.file_id,
        token_stream
            .tokens
            .token(
                TokenIndex::try_from_index(assign_index + 1)
                    .expect("boundary index should fit the canonical domain"),
            )
            .expect("source must contain the boundary token")
            .span(),
    )
}
/// Span of the first top-level boundary token (newline/end/EOF/comma) starting after
fn first_boundary_span_after_name(source: &str) -> SourceSpan {
    let mut span_builder = ExtendedSpanBuilder::new();
    let (_string_table, token_stream, _name, _) =
        tokenize_for_declaration(source, &mut span_builder);
    let token = (2..token_stream.tokens.len())
        .find_map(|index| {
            let index = TokenIndex::try_from_index(index)?;
            let token = token_stream.tokens.token(index).ok()?;
            matches!(
                token.tag(),
                TokenTag::NEWLINE | TokenTag::END | TokenTag::EOF | TokenTag::COMMA
            )
            .then_some(token)
        })
        .expect("source must contain a declaration boundary");
    SourceSpan::new(token_stream.file_id, token.span())
}

/// Asserts the diagnostic is the authored-`=` rejection, names the declaration, and
/// points at the given boundary rather than the declaration name or target type.
fn assert_missing_initializer_expression(
    diagnostic: &DiagnosticView<'_>,
    expected_name: StringId,
    expected_boundary: &SourceSpan,
    case: &str,
) {
    assert_eq!(
        diagnostic.category(),
        DiagnosticCategory::Rule,
        "{case}: authored `=` with no initializer must use the InvalidDeclaration category",
    );
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-RULE-0043",
        "{case}: authored `=` with no initializer must keep the InvalidDeclaration stable code",
    );
    let facts = diagnostic
        .family_view::<InvalidDeclarationView>()
        .expect("authored `=` with no initializer must have InvalidDeclaration facts");
    assert_eq!(
        facts.code,
        InvalidDeclarationCode::MissingInitializerExpression,
        "{case}: authored `=` with no initializer must carry the MissingInitializerExpression reason",
    );
    assert_eq!(
        facts.name,
        Some(expected_name),
        "{case}: authored `=` with no initializer must name the declaration",
    );
    assert!(
        matches!(diagnostic.primary_site(), SiteRef::Local(span) if span == *expected_boundary),
        "{case}: authored `=` with no initializer must point at the real boundary after `=`, not the declaration name or target type",
    );
}

/// Asserts the diagnostic is the no-`=` rejection: the declaration omitted `=` entirely
/// and must use the `MissingDeclarationInitializer` kind and stable code.
fn assert_missing_declaration_initializer(
    diagnostic: &DiagnosticView<'_>,
    expected_name: StringId,
    expected_boundary: &SourceSpan,
    case: &str,
) {
    assert_eq!(
        diagnostic.category(),
        DiagnosticCategory::Rule,
        "{case}: a declaration that omits `=` must use the MissingDeclarationInitializer category",
    );
    assert_eq!(
        diagnostic.descriptor().code,
        "MOTH-RULE-0031",
        "{case}: a declaration that omits `=` must keep the MOTH-RULE-0031 stable code",
    );
    let facts = diagnostic
        .family_view::<MissingDeclarationInitializerView>()
        .expect("a declaration that omits `=` must have MissingDeclarationInitializer facts");
    assert_eq!(
        facts.name, expected_name,
        "{case}: the no-`=` diagnostic must name the declaration",
    );
    assert!(
        matches!(diagnostic.primary_site(), SiteRef::Local(span) if span == *expected_boundary),
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
        let (diagnostics, name) = parse_shell_error(source);
        let diagnostic = diagnostics.first().expect("diagnostic should be retained");
        let boundary = boundary_span_after_assign(source);
        assert_missing_initializer_expression(&diagnostic, name, &boundary, case);
    }
}

#[test]
fn authored_assign_with_no_initializer_rejects_explicit_type_and_binding_modes() {
    let cases = [
        ("value I32 =\n", "explicit type at newline boundary"),
        ("value I32 =", "explicit type at EOF boundary"),
        ("value ~=\n", "mutable binding"),
        ("value #=\n", "compile-time binding"),
    ];

    for (source, case) in cases {
        let (diagnostics, name) = parse_shell_error(source);
        let diagnostic = diagnostics.first().expect("diagnostic should be retained");
        let boundary = boundary_span_after_assign(source);
        assert_missing_initializer_expression(&diagnostic, name, &boundary, case);
    }
}

#[test]
fn omitted_assign_rejects_at_each_boundary_and_binding_mode() {
    let cases = [
        ("value\n", "inferred declaration at newline boundary"),
        ("value I32", "explicit type at EOF boundary"),
    ];

    for (source, case) in cases {
        let (diagnostics, name) = parse_shell_error(source);
        let diagnostic = diagnostics.first().expect("diagnostic should be retained");
        let boundary = first_boundary_span_after_name(source);
        assert_missing_declaration_initializer(&diagnostic, name, &boundary, case);
    }
}
