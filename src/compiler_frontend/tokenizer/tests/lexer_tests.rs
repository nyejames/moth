use super::*;
use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, terminal::format_payload_guidance,
};
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, CompilerDiagnostic, DiagnosticCompoundAssignmentOperator,
    DiagnosticKind, DiagnosticOperator, DiagnosticPayload, InvalidStringEscapeReason,
    MissingWhitespace, NumberLiteralErrorReason, SourceSpanCapacityResource,
    SymbolicSpacingConstruct, SymbolicSpacingError, SyntaxDiagnosticKind,
};
use crate::compiler_frontend::numeric_text::token::{NumericLiteralSign, NumericLiteralToken};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::source::line_index::{LineIndex, line_start_offsets};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceDatabaseBuilder, SourceId,
};
use crate::compiler_frontend::style_directives::{
    StyleDirectiveHandlerSpec, StyleDirectiveRegistry, StyleDirectiveSpec,
    TemplateHeadCompatibility,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokenBuildError, SourceTokens, SourceTokensBuilder, TokenIndex, TokenRange, TokenRef,
    TokenTag, token_store_append_fits, token_store_length_fits,
};
use crate::compiler_tests::test_support::frontend_test_style_directives;

fn full_token_range(lexed: &LexedSource) -> TokenRange {
    lexed
        .tokens
        .full_range()
        .expect("lexer output must expose a checked full token range")
}

fn ast_cursor(lexed: &LexedSource) -> AstCursor<'_> {
    AstCursor::from_source_tokens(&lexed.tokens, full_token_range(lexed))
        .expect("lexer output must expose a canonical AST cursor")
}

fn token_string_is(token: TokenRef<'_>, string_table: &StringTable, expected: &str) -> bool {
    token
        .string_id()
        .is_some_and(|id| string_table.resolve(id) == expected)
}

fn numeric_payload(token: TokenRef<'_>) -> Option<&'_ NumericLiteralToken> {
    if token.tag() != TokenTag::NUMERIC_LITERAL {
        return None;
    }
    token
        .numeric_literal()
        .expect("numeric token payload must resolve")
}

fn token_has_path_payload(token: TokenRef<'_>) -> bool {
    token.tag() == TokenTag::PATH && token.path_syntax_id().is_some()
}

fn tokenize_source(source: &str) -> (LexedSource, StringTable) {
    let style_directives = frontend_test_style_directives();
    tokenize_source_with_registry(source, &style_directives)
}

fn tokenize_html_source(source: &str) -> (LexedSource, StringTable) {
    let style_directives = frontend_test_style_directives();
    tokenize_source_with_registry(source, &style_directives)
}

fn expect_lexical_diagnostic(failure: TokenizeFailure) -> CompilerDiagnostic {
    match failure {
        TokenizeFailure::Diagnosed(diagnostic) => diagnostic,
        TokenizeFailure::Infrastructure(error) => {
            panic!("lexical diagnosis fixture encountered infrastructure failure: {error:?}")
        }
    }
}

fn tokenize_source_error(source: &str) -> (CompilerDiagnostic, StringTable) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let source_path = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let mut span_builder = ExtendedSpanBuilder::new();
    let Err(diagnostic) = tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    ) else {
        panic!("tokenization should fail");
    };
    (expect_lexical_diagnostic(diagnostic), string_table)
}

fn tokenize_source_with_registry(
    source: &str,
    style_directives: &StyleDirectiveRegistry,
) -> (LexedSource, StringTable) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let mut span_builder = ExtendedSpanBuilder::new();
    let lexed = tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        style_directives,
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("tokenization should succeed");
    (lexed, string_table)
}

fn tokenize_source_with_directives(
    source: &str,
    directives: &[StyleDirectiveSpec],
) -> (LexedSource, StringTable) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let registry = StyleDirectiveRegistry::merged(directives)
        .expect("test style directives should merge with core directives");
    let mut span_builder = ExtendedSpanBuilder::new();
    let lexed = tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &registry,
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("tokenization should succeed");
    (lexed, string_table)
}

fn tokenize_moth_template_source(source: &str) -> (LexedSource, StringTable) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let style_directives = frontend_test_style_directives();
    let source_path = path_fork
        .try_intern_portable_path("test.mtf", &mut string_table)
        .expect("test path fits");
    let mut span_builder = ExtendedSpanBuilder::new();
    let lexed = tokenize(
        source,
        source_path,
        TokenizerEntryMode::for_source_file_kind(SourceFileKind::MothTemplate)
            .expect("Moth template should tokenize"),
        &style_directives,
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("Moth template tokenization should succeed");
    (lexed, string_table)
}

fn tokenize_moth_template_error(source: &str) -> (CompilerDiagnostic, StringTable) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let style_directives = frontend_test_style_directives();
    let source_path = path_fork
        .try_intern_portable_path("test.mtf", &mut string_table)
        .expect("test path fits");
    let mut span_builder = ExtendedSpanBuilder::new();
    let Err(diagnostic) = tokenize(
        source,
        source_path,
        TokenizerEntryMode::for_source_file_kind(SourceFileKind::MothTemplate)
            .expect("Moth template should tokenize"),
        &style_directives,
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    ) else {
        panic!("Moth template tokenization should fail");
    };
    (expect_lexical_diagnostic(diagnostic), string_table)
}

fn token_refs(tokens: &SourceTokens) -> impl Iterator<Item = TokenRef<'_>> {
    (0..tokens.len()).map(|index| {
        let index = TokenIndex::try_from_index(index).expect("source token range fits");
        tokens
            .token(index)
            .expect("source token index must resolve")
    })
}

fn token_at(tokens: &SourceTokens, index: usize) -> TokenRef<'_> {
    let index = TokenIndex::try_from_index(index).expect("source token range fits");
    tokens
        .token(index)
        .expect("source token index must resolve")
}

fn find_token_index(tokens: &SourceTokens, predicate: impl Fn(TokenRef<'_>) -> bool) -> usize {
    token_refs(tokens)
        .position(predicate)
        .expect("expected token to be present")
}

fn span_byte_range(span: LocalSpan) -> (u32, u32) {
    let resolver = ExtendedSpanBuilder::new();
    let resolved = span.resolve_with(resolver.resolver());
    (resolved.start(), resolved.end())
}

fn token_byte_range(token: TokenRef<'_>) -> (u32, u32) {
    span_byte_range(token.span())
}

fn diagnostic_byte_range(diagnostic: &CompilerDiagnostic) -> (u32, u32) {
    let span = diagnostic
        .primary_span
        .expect("tokenizer diagnostics should retain an authored source span");
    span_byte_range(span.local())
}

fn assert_invalid_number_literal(
    diagnostic: &CompilerDiagnostic,
    string_table: &StringTable,
    expected_literal: &str,
    expected_reason: NumberLiteralErrorReason,
) {
    match &diagnostic.payload {
        DiagnosticPayload::InvalidNumberLiteral {
            literal_text,
            reason,
        } => {
            assert_eq!(string_table.resolve(*literal_text), expected_literal);
            assert_eq!(*reason, expected_reason);
        }
        payload => panic!("expected invalid number literal payload, found {payload:?}"),
    }
}

fn assert_common_syntax_mistake(
    diagnostic: &CompilerDiagnostic,
    expected_reason: CommonSyntaxMistakeReason,
) {
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::CommonSyntaxMistake)
    );

    match &diagnostic.payload {
        DiagnosticPayload::CommonSyntaxMistake { reason } => {
            assert_eq!(*reason, expected_reason);
        }
        payload => panic!("expected common syntax mistake payload, found {payload:?}"),
    }
}

fn assert_symbolic_spacing(
    diagnostic: &CompilerDiagnostic,
    construct: SymbolicSpacingConstruct,
    missing: MissingWhitespace,
) {
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::CommonSyntaxMistake)
    );

    match &diagnostic.payload {
        DiagnosticPayload::CommonSyntaxMistake { reason } => match reason {
            CommonSyntaxMistakeReason::InvalidSymbolicSpacing { error } => {
                assert_eq!(
                    error,
                    &SymbolicSpacingError { construct, missing },
                    "symbolic spacing construct or missing side mismatch"
                );
            }
            other => panic!("expected InvalidSymbolicSpacing, found {other:?}"),
        },
        payload => panic!("expected common syntax mistake payload, found {payload:?}"),
    }
}

fn numeric_literal_signs(lexed: &LexedSource) -> Vec<NumericLiteralSign> {
    token_refs(lexed.tokens.as_ref())
        .filter(|&token| token.tag() == TokenTag::NUMERIC_LITERAL)
        .map(|token| {
            token
                .numeric_literal()
                .expect("numeric token payload must resolve")
                .expect("numeric token must carry a literal")
                .sign
        })
        .collect()
}

fn collect_literal_texts(lexed: &LexedSource, string_table: &StringTable) -> Vec<String> {
    token_refs(lexed.tokens.as_ref())
        .filter(|&token| {
            matches!(
                token.tag(),
                TokenTag::STRING_SLICE_LITERAL | TokenTag::RAW_STRING_LITERAL
            )
        })
        .map(|token| {
            string_table
                .resolve(token.string_id().expect("string token must carry a handle"))
                .to_owned()
        })
        .collect()
}

#[test]
fn normalizes_regular_string_newlines_from_crlf_and_bare_cr() {
    let (file_tokens, string_table) = tokenize_source("value = \"line1\r\nline2\rline3\"\n");
    let texts = collect_literal_texts(&file_tokens, &string_table);
    let string_literal = texts
        .first()
        .expect("expected one regular string literal to be tokenized");

    assert_eq!(string_literal, "line1\nline2\nline3");
    assert!(
        !string_literal.contains('\r'),
        "regular string literals should not retain carriage returns"
    );
}

#[test]
fn normalizes_raw_string_newlines_from_crlf_and_bare_cr() {
    let (file_tokens, string_table) = tokenize_source("`line1\r\nline2\rline3`");
    let texts = collect_literal_texts(&file_tokens, &string_table);
    let raw_literal = texts
        .first()
        .expect("expected one raw string literal to be tokenized");

    assert_eq!(raw_literal, "line1\nline2\nline3");
    assert!(
        !raw_literal.contains('\r'),
        "raw string literals should not retain carriage returns"
    );
}

#[test]
fn normalizes_template_body_newlines_from_crlf_and_bare_cr() {
    let (file_tokens, string_table) = tokenize_source("[:line1\r\nline2\rline3]");
    let texts = collect_literal_texts(&file_tokens, &string_table);
    let body_literal = texts
        .first()
        .expect("expected one template-body string literal to be tokenized");

    assert_eq!(body_literal, "line1\nline2\nline3");
    assert!(
        !body_literal.contains('\r'),
        "template body literals should not retain carriage returns"
    );
}

#[test]
fn normal_template_body_preserves_backslash_as_literal_text() {
    let (file_tokens, string_table) = tokenize_source(r#"[: \ ]"#);
    let texts = collect_literal_texts(&file_tokens, &string_table);

    assert_eq!(texts, vec![" \\ "]);
}

#[test]
fn normal_template_body_preserves_backslash_followed_by_n_as_literal_text() {
    let (file_tokens, string_table) = tokenize_source("[:\\n]");
    let texts = collect_literal_texts(&file_tokens, &string_table);
    assert_eq!(texts, vec!["\\n"]);
}

#[test]
fn normal_template_body_does_not_escape_opening_square_bracket() {
    let (file_tokens, string_table) = tokenize_source("[:\\[]");
    let texts = collect_literal_texts(&file_tokens, &string_table);

    assert_eq!(texts, vec!["\\"]);
}

#[test]
fn normal_template_body_preserves_backtick_as_literal_text() {
    let (file_tokens, string_table) = tokenize_source("[:` ]");
    let texts = collect_literal_texts(&file_tokens, &string_table);

    assert_eq!(texts, vec!["` "]);
}

fn assert_invalid_string_escape(source: &str, expected_reason: InvalidStringEscapeReason) {
    let (diagnostic, _string_table) = tokenize_source_error(source);
    let expected_span_width = match expected_reason {
        InvalidStringEscapeReason::UnsupportedEscape { .. } => 2,
        InvalidStringEscapeReason::PhysicalNewline
        | InvalidStringEscapeReason::TrailingBackslash => 1,
    };

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidStringEscape)
    );
    assert_eq!(diagnostic.kind.code(), "MOTH-SYNTAX-0034");
    let (start, end) = diagnostic_byte_range(&diagnostic);
    assert_eq!(end - start, expected_span_width);

    match &diagnostic.payload {
        DiagnosticPayload::InvalidStringEscape { reason } => {
            assert_eq!(*reason, expected_reason);
        }
        payload => panic!("expected invalid string escape payload, found {payload:?}"),
    }
}

#[test]
fn quoted_string_decodes_carriage_return_escape() {
    // The integration case `print_special_chars` owns the other accepted escapes.
    let (file_tokens, string_table) = tokenize_source(r#"value = "a\rb""#);
    let texts = collect_literal_texts(&file_tokens, &string_table);

    assert_eq!(texts, vec!["a\rb"]);
}

#[test]
fn quoted_string_rejects_unsupported_letter_escape() {
    assert_invalid_string_escape(
        r#"value = "a\qb""#,
        InvalidStringEscapeReason::UnsupportedEscape { escaped: 'q' },
    );
}

#[test]
fn quoted_string_rejects_unsupported_digit_escape() {
    assert_invalid_string_escape(
        r#"value = "a\0b""#,
        InvalidStringEscapeReason::UnsupportedEscape { escaped: '0' },
    );
}

#[test]
fn quoted_string_rejects_trailing_backslash() {
    // A backslash at end of source never receives an escaped character.
    assert_invalid_string_escape(
        r###"value = "ab\"###,
        InvalidStringEscapeReason::TrailingBackslash,
    );
}

#[test]
fn quoted_string_without_a_trailing_backslash_stays_unterminated() {
    let (diagnostic, _string_table) = tokenize_source_error("value = \"ab");

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnterminatedStringLiteral)
    );
}

#[test]
fn quoted_string_rejects_lf_physical_newline_continuation() {
    // A backslash before a physical line feed is a line-continuation attempt.
    assert_invalid_string_escape(
        "value = \"ab\\\ncd\"",
        InvalidStringEscapeReason::PhysicalNewline,
    );
}

#[test]
fn quoted_string_rejects_crlf_physical_newline_continuation() {
    // LF and CRLF continuation are the same typed source mistake.
    assert_invalid_string_escape(
        "value = \"ab\\\r\ncd\"",
        InvalidStringEscapeReason::PhysicalNewline,
    );
}

#[test]
fn raw_string_preserves_backslashes_and_newlines_without_escape_decoding() {
    // Raw backtick strings keep backslashes literal and physical newlines normalized to LF,
    // without any escape decoding or invalid-escape diagnostics.
    let (file_tokens, string_table) = tokenize_source("`a\\nb\\qc\nd`");
    let texts = collect_literal_texts(&file_tokens, &string_table);

    assert_eq!(texts, vec!["a\\nb\\qc\nd"]);
}

#[test]
fn moth_template_entry_body_rejects_unescaped_outer_template_close() {
    let source = "]";
    let (diagnostic, string_table) = tokenize_moth_template_error(source);

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnescapedImplicitTemplateClose)
    );
    assert!(matches!(
        &diagnostic.payload,
        DiagnosticPayload::UnescapedImplicitTemplateClose {
            source_kind: SourceFileKind::MothTemplate
        }
    ));
    let (start, end) = diagnostic_byte_range(&diagnostic);
    assert_eq!(
        source.get(start as usize..end as usize),
        Some("]"),
        "unescaped template close should retain its authored byte",
    );

    let guidance = format_payload_guidance(
        &diagnostic.payload,
        DiagnosticRenderContext::new(&string_table),
    )
    .join("\n");
    assert!(guidance.contains("Moth template `.mtf` source"));
    assert!(guidance.contains(r#"["]"]"#));
}

#[test]
fn moth_template_entry_body_preserves_backslash_as_literal_text() {
    let (file_tokens, string_table) = tokenize_moth_template_source("\\n");
    let texts = collect_literal_texts(&file_tokens, &string_table);

    assert_eq!(texts, vec!["\\n"]);
}

#[test]
fn moth_template_entry_body_allows_nested_template_close() {
    let (file_tokens, string_table) = tokenize_moth_template_source("before [:inner] after");
    let template_closes = token_refs(file_tokens.tokens.as_ref())
        .filter(|token| token.tag() == TokenTag::TEMPLATE_CLOSE)
        .count();
    let texts = collect_literal_texts(&file_tokens, &string_table);

    assert_eq!(template_closes, 1);
    assert_eq!(texts, vec!["before ", "inner", " after"]);
}

#[test]
fn moth_template_entry_body_keeps_double_dash_as_text() {
    let (file_tokens, string_table) = tokenize_moth_template_source("alpha -- still text\nbeta");
    let texts = collect_literal_texts(&file_tokens, &string_table);

    assert_eq!(texts, vec!["alpha -- still text\nbeta"]);
}

#[test]
fn normalizes_code_template_body_newlines_from_crlf_and_bare_cr() {
    let (file_tokens, string_table) =
        tokenize_source("[$code:\r\nalpha\nline\rbravo\r\ncharlie\r\ndelta\r]");
    let texts = collect_literal_texts(&file_tokens, &string_table);
    let body_literal = texts
        .iter()
        .find(|text| text.contains("alpha"))
        .expect("expected code template body literal");

    assert!(
        body_literal.contains("alpha\nline\nbravo\ncharlie\ndelta\n"),
        "code template body should normalize mixed newline sequences to LF"
    );
    assert!(
        !body_literal.contains('\r'),
        "code template body literals should not retain carriage returns"
    );
}

#[test]
fn tokenizes_double_slash_as_integer_division_operator() {
    let (file_tokens, _string_table) = tokenize_source("value = 5 // 2\n");
    let tokens = file_tokens.tokens.as_ref();

    assert!(
        token_refs(tokens).any(|token| token.tag() == TokenTag::INT_DIVIDE),
        "expected '//' to tokenize as IntDivide"
    );
    assert!(
        !token_refs(tokens).any(|token| token.tag() == TokenTag::DIVIDE_ASSIGN),
        "integer division token should not be confused with '/='"
    );
}

#[test]
fn tokenizes_double_slash_equals_as_integer_division_assignment_operator() {
    let (file_tokens, _string_table) = tokenize_source("value ~= 10\nvalue //= 3\n");
    assert!(
        token_refs(file_tokens.tokens.as_ref())
            .any(|token| token.tag() == TokenTag::INT_DIVIDE_ASSIGN),
        "expected '//=' to tokenize as IntDivideAssign"
    );
}

#[test]
fn rejects_uppercase_exponent_marker() {
    let (error, string_table) = tokenize_source_error("value = 1E6\n");

    assert_invalid_number_literal(
        &error,
        &string_table,
        "1E6",
        NumberLiteralErrorReason::UppercaseExponentMarker,
    );
}

#[test]
fn rejects_missing_exponent_digits() {
    for (source, expected_literal) in [
        ("value = 1e\n", "1e"),
        ("value = 1e+\n", "1e+"),
        ("value = 1e-\n", "1e-"),
    ] {
        let (error, string_table) = tokenize_source_error(source);

        assert_invalid_number_literal(
            &error,
            &string_table,
            expected_literal,
            NumberLiteralErrorReason::MissingExponentDigits,
        );
    }
}

#[test]
fn rejects_multiple_decimal_points_in_numeric_literal() {
    let (error, string_table) = tokenize_source_error("value = 1.2.3\n");

    assert_invalid_number_literal(
        &error,
        &string_table,
        "1.2",
        NumberLiteralErrorReason::MultipleDecimalPoints,
    );
}

#[test]
fn rejects_non_ascii_numeric_leads_as_invalid_characters() {
    for (source, expected_character) in [
        ("٣ = 2\n", '٣'),
        ("b = ½\n", '½'),
        ("value = -٣\n", '٣'),
        ("value = -½\n", '½'),
    ] {
        let (diagnostic, _string_table) = tokenize_source_error(source);
        assert_eq!(
            diagnostic.kind,
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidCharacter)
        );
        assert_eq!(diagnostic.kind.code(), "MOTH-SYNTAX-0007");
        match diagnostic.payload {
            DiagnosticPayload::InvalidCharacter { character } => {
                assert_eq!(character, expected_character);
            }
            payload => panic!("expected invalid character payload, found {payload:?}"),
        }
    }
}

#[test]
fn tokenizes_lowercase_exponent_literals() {
    let (file_tokens, string_table) = tokenize_source("value = 1e6 1e-6 1e+6 1.0e+21\n");
    let numeric_texts: Vec<String> = token_refs(file_tokens.tokens.as_ref())
        .filter_map(numeric_payload)
        .map(|token| string_table.resolve(token.normalized_text).to_owned())
        .collect();

    assert_eq!(
        numeric_texts,
        vec!["1e6", "1e-6", "1e+6", "1.0e+21"],
        "lowercase exponent literals should keep their normalized text"
    );
}

#[test]
fn tokenizes_signed_numeric_literals() {
    let (file_tokens, string_table) = tokenize_source("value = {-1, -1.5, -1e6}\n");
    let numeric_texts: Vec<String> = token_refs(file_tokens.tokens.as_ref())
        .filter_map(numeric_payload)
        .map(|token| string_table.resolve(token.normalized_text).to_owned())
        .collect();

    assert_eq!(
        numeric_literal_signs(&file_tokens),
        vec![
            NumericLiteralSign::Negative,
            NumericLiteralSign::Negative,
            NumericLiteralSign::Negative
        ]
    );
    assert_eq!(numeric_texts, vec!["1", "1.5", "1e6"]);
}

#[test]
fn preserves_signed_numeric_literal_after_binary_operator() {
    let (file_tokens, _string_table) = tokenize_source("value = count * -1\n");
    let tokens = file_tokens.tokens.as_ref();

    assert!(
        token_refs(tokens).any(|token| {
            numeric_payload(token)
                .is_some_and(|numeric| numeric.sign == NumericLiteralSign::Negative)
        }),
        "`-1` after a spaced binary operator should remain one signed numeric token"
    );
    assert!(
        !token_refs(tokens).any(|token| token.tag() == TokenTag::NEGATIVE),
        "signed numeric literals should not also emit a unary Negative token"
    );
}

#[test]
fn tokenizes_line_initial_negative_match_pattern_after_expression_body() {
    let (file_tokens, string_table) = tokenize_source(
        "value = 1\n\
         if value is:\n\
             1 => value = 1\n\
             -42 => value = -42\n\
             else => value = 0\n\
         ;\n",
    );
    let tokens = file_tokens.tokens.as_ref();
    let pattern_index = token_refs(tokens)
        .position(|token| {
            numeric_payload(token)
                .is_some_and(|numeric| numeric.sign == NumericLiteralSign::Negative)
                && token_refs(tokens)
                    .nth(token.index().index().saturating_add(1))
                    .is_some_and(|next| next.tag() == TokenTag::FAT_ARROW)
        })
        .expect("expected a numeric literal immediately before the negative arm arrow");
    let pattern_token = token_at(tokens, pattern_index);

    let numeric = numeric_payload(pattern_token).expect("expected a numeric pattern token");
    assert_eq!(numeric.sign, NumericLiteralSign::Negative);
    assert_eq!(
        string_table.resolve(numeric.source_text),
        "-42",
        "the line-initial match pattern should retain its authored sign"
    );
}

#[test]
fn tokenizes_line_initial_negative_match_pattern_with_guard() {
    let (file_tokens, _string_table) = tokenize_source(
        "value = 1\n\
         if value is:\n\
             1 => value = 1\n\
             -42 if enabled => value = -42\n\
             else => value = 0\n\
         ;\n",
    );
    let tokens = file_tokens.tokens.as_ref();
    let pattern_index = token_refs(tokens)
        .position(|token| {
            numeric_payload(token)
                .is_some_and(|numeric| numeric.sign == NumericLiteralSign::Negative)
        })
        .expect("expected a negative numeric literal in the guarded match arm");

    let mut cursor = ast_cursor(&file_tokens);
    cursor
        .set_position(pattern_index)
        .expect("pattern index must fit the canonical cursor");
    assert_eq!(
        cursor.peek_next_ref().map(|token| token.tag()),
        Some(TokenTag::IF)
    );
    assert!(
        token_refs(tokens)
            .enumerate()
            .skip(pattern_index)
            .any(|(_, token)| token.tag() == TokenTag::FAT_ARROW),
        "guarded match arm should eventually reach its arrow"
    );
}

#[test]
fn tokenizes_line_initial_negative_match_pattern_with_multiline_guard() {
    let (file_tokens, _string_table) = tokenize_source(
        "value = 1\n\
         if value is:\n\
             1 => value = 1\n\
             -42 if\n\
                 true => value = -42\n\
             else => value = 0\n\
         ;\n",
    );
    let tokens = file_tokens.tokens.as_ref();
    let pattern_index = token_refs(tokens)
        .position(|token| {
            numeric_payload(token)
                .is_some_and(|numeric| numeric.sign == NumericLiteralSign::Negative)
        })
        .expect("expected a negative numeric literal in the multiline guarded match arm");

    let mut cursor = ast_cursor(&file_tokens);
    cursor
        .set_position(pattern_index)
        .expect("pattern index must fit the canonical cursor");
    assert_eq!(
        cursor.peek_next_ref().map(|token| token.tag()),
        Some(TokenTag::IF)
    );
    let mut found_true_arrow = false;
    while let Some(token) = cursor.current() {
        if token.tag() == TokenTag::BOOL_LITERAL
            && token.bool_value() == Some(true)
            && cursor
                .peek_next_ref()
                .is_some_and(|next| next.tag() == TokenTag::FAT_ARROW)
        {
            found_true_arrow = true;
            break;
        }
        cursor.advance();
    }
    assert!(found_true_arrow);
}

#[test]
fn tokenizes_line_initial_negative_match_pattern_with_named_argument_guard() {
    let (file_tokens, _string_table) = tokenize_source(
        "value = 1\n\
         if value is:\n\
             1 => value = 1\n\
             -42 if allowed(value = candidate) => value = -42\n\
             else => value = 0\n\
         ;\n",
    );
    let tokens = file_tokens.tokens.as_ref();
    let pattern_index = token_refs(tokens)
        .position(|token| {
            numeric_payload(token)
                .is_some_and(|numeric| numeric.sign == NumericLiteralSign::Negative)
        })
        .expect("expected a negative numeric literal in the named-argument guarded match arm");

    let mut cursor = ast_cursor(&file_tokens);
    cursor
        .set_position(pattern_index)
        .expect("pattern index must fit the canonical cursor");
    assert_eq!(
        cursor.peek_next_ref().map(|token| token.tag()),
        Some(TokenTag::IF)
    );
    let arm_arrow_index = token_refs(tokens)
        .enumerate()
        .skip(pattern_index)
        .find_map(|(index, token)| (token.tag() == TokenTag::FAT_ARROW).then_some(index))
        .expect("expected the named-argument guarded match arm arrow");
    let preceding = arm_arrow_index
        .checked_sub(1)
        .map(|index| token_at(tokens, index))
        .expect("arrow must have a preceding token");
    assert_eq!(preceding.tag(), TokenTag::CLOSE_PARENTHESIS);
}

#[test]
fn tokenizes_attached_unary_negation_for_non_numeric_operands() {
    let (file_tokens, _string_table) = tokenize_source("value = -count\nother = total * -count\n");

    let negative_count = token_refs(file_tokens.tokens.as_ref())
        .filter(|token| token.tag() == TokenTag::NEGATIVE)
        .count();
    assert_eq!(negative_count, 2);
}

#[test]
fn rejects_unary_plus() {
    for source in ["value = +1\n", "value = +count\n"] {
        let (diagnostic, _string_table) = tokenize_source_error(source);
        assert_common_syntax_mistake(&diagnostic, CommonSyntaxMistakeReason::UnsupportedUnaryPlus);
    }
}

#[test]
fn rejects_unary_negation_with_whitespace() {
    for source in ["value = - 1\n", "value = - count\n"] {
        let (diagnostic, _string_table) = tokenize_source_error(source);
        assert_common_syntax_mistake(
            &diagnostic,
            CommonSyntaxMistakeReason::InvalidUnaryNegationSpacing,
        );
    }
}

#[test]
fn rejects_false_match_arm_arrows_inside_comments_and_strings() {
    for source in [
        "value = a\n-1 -- fake =>\n",
        "value = a\n-1 \"fake =>\"\n",
        "value = a\n-1 if \"fake =>\"\n",
        "value = a\n-1 if allowed(fake => candidate)\n",
    ] {
        let (diagnostic, _string_table) = tokenize_source_error(source);
        assert_symbolic_spacing(
            &diagnostic,
            SymbolicSpacingConstruct::BinaryOperator {
                operator: DiagnosticOperator::Subtract,
            },
            MissingWhitespace::After,
        );
    }
}

#[test]
fn rejects_binary_operator_spacing() {
    for (source, operator, missing) in [
        (
            "value = a+b\n",
            DiagnosticOperator::Add,
            MissingWhitespace::Both,
        ),
        (
            "value = a-1\n",
            DiagnosticOperator::Subtract,
            MissingWhitespace::Both,
        ),
        (
            "value = a -1\n",
            DiagnosticOperator::Subtract,
            MissingWhitespace::After,
        ),
        (
            "value = a- 1\n",
            DiagnosticOperator::Subtract,
            MissingWhitespace::Before,
        ),
        (
            "value = a\n-1\n",
            DiagnosticOperator::Subtract,
            MissingWhitespace::After,
        ),
        (
            "value = a*-1\n",
            DiagnosticOperator::Multiply,
            MissingWhitespace::Both,
        ),
        (
            "value = a //b\n",
            DiagnosticOperator::IntDivide,
            MissingWhitespace::After,
        ),
        (
            "value = a<b\n",
            DiagnosticOperator::LessThan,
            MissingWhitespace::Both,
        ),
        (
            "value = a<B\n",
            DiagnosticOperator::LessThan,
            MissingWhitespace::Both,
        ),
        (
            "value = a>(b)\n",
            DiagnosticOperator::GreaterThan,
            MissingWhitespace::Both,
        ),
        (
            "value = a<(b)\n",
            DiagnosticOperator::LessThan,
            MissingWhitespace::Both,
        ),
        (
            "value = identity<Int>(42)\n",
            DiagnosticOperator::LessThan,
            MissingWhitespace::Both,
        ),
        (
            "value = a>=b\n",
            DiagnosticOperator::GreaterThanOrEqual,
            MissingWhitespace::Both,
        ),
    ] {
        let (diagnostic, _string_table) = tokenize_source_error(source);
        assert_symbolic_spacing(
            &diagnostic,
            SymbolicSpacingConstruct::BinaryOperator { operator },
            missing,
        );
    }
}

/// Compound symbolic assignments (`+=`, `-=`, `*=`, `/=`, `//=`, `%=`, `^=`)
/// must enforce the same spacing rule as ordinary symbolic binary operators.
#[test]
fn accepts_valid_compound_assignment_spacing() {
    for source in [
        "count += 1\n",
        "count -= 1\n",
        "count *= 2\n",
        "count //= 3\n",
        "count /= 4\n",
        "count %= 5\n",
        "count ^= 2\n",
        "count ~= 1\n",
    ] {
        let (file_tokens, _string_table) = tokenize_source(source);
        assert!(
            token_refs(file_tokens.tokens.as_ref()).any(|token| {
                matches!(
                    token.tag(),
                    TokenTag::ADD_ASSIGN
                        | TokenTag::SUBTRACT_ASSIGN
                        | TokenTag::MULTIPLY_ASSIGN
                        | TokenTag::DIVIDE_ASSIGN
                        | TokenTag::INT_DIVIDE_ASSIGN
                        | TokenTag::MODULUS_ASSIGN
                        | TokenTag::EXPONENT_ASSIGN
                        | TokenTag::MUTABLE
                )
            }),
            "expected a compound assignment or mutable token in: {source}"
        );
    }
}

/// Every compound assignment token is covered at least once across the three
/// missing-side branches: both, after and before.
#[test]
fn rejects_compound_assignment_missing_all_spacing() {
    for (source, operator) in [
        ("count+=1\n", DiagnosticCompoundAssignmentOperator::Add),
        ("count-=1\n", DiagnosticCompoundAssignmentOperator::Subtract),
        ("count*=2\n", DiagnosticCompoundAssignmentOperator::Multiply),
        ("count/=4\n", DiagnosticCompoundAssignmentOperator::Divide),
        (
            "count//=3\n",
            DiagnosticCompoundAssignmentOperator::IntDivide,
        ),
        ("count%=5\n", DiagnosticCompoundAssignmentOperator::Modulus),
        ("count^=2\n", DiagnosticCompoundAssignmentOperator::Exponent),
    ] {
        let (diagnostic, _string_table) = tokenize_source_error(source);
        assert_symbolic_spacing(
            &diagnostic,
            SymbolicSpacingConstruct::CompoundAssignment { operator },
            MissingWhitespace::Both,
        );
    }
}

/// Compound assignments with left spacing but missing right spacing remain covered for
/// operators not owned by the `compound_assignment_spacing_rejected` case.
#[test]
fn rejects_compound_assignment_missing_right_spacing_for_other_operators() {
    for (source, operator) in [
        (
            "count -=1\n",
            DiagnosticCompoundAssignmentOperator::Subtract,
        ),
        (
            "count *=2\n",
            DiagnosticCompoundAssignmentOperator::Multiply,
        ),
        ("count /=4\n", DiagnosticCompoundAssignmentOperator::Divide),
        (
            "count //=3\n",
            DiagnosticCompoundAssignmentOperator::IntDivide,
        ),
        ("count %=5\n", DiagnosticCompoundAssignmentOperator::Modulus),
        (
            "count ^=2\n",
            DiagnosticCompoundAssignmentOperator::Exponent,
        ),
    ] {
        let (diagnostic, _string_table) = tokenize_source_error(source);
        assert_symbolic_spacing(
            &diagnostic,
            SymbolicSpacingConstruct::CompoundAssignment { operator },
            MissingWhitespace::After,
        );
    }
}

/// Compound assignments with right spacing but missing left spacing (`count+= 1`) must fail.
#[test]
fn rejects_compound_assignment_missing_left_spacing() {
    for (source, operator) in [
        ("count+= 1\n", DiagnosticCompoundAssignmentOperator::Add),
        (
            "count-= 1\n",
            DiagnosticCompoundAssignmentOperator::Subtract,
        ),
        (
            "count*= 2\n",
            DiagnosticCompoundAssignmentOperator::Multiply,
        ),
        ("count/= 4\n", DiagnosticCompoundAssignmentOperator::Divide),
        (
            "count//= 3\n",
            DiagnosticCompoundAssignmentOperator::IntDivide,
        ),
        ("count%= 5\n", DiagnosticCompoundAssignmentOperator::Modulus),
        (
            "count^= 2\n",
            DiagnosticCompoundAssignmentOperator::Exponent,
        ),
    ] {
        let (diagnostic, _string_table) = tokenize_source_error(source);
        assert_symbolic_spacing(
            &diagnostic,
            SymbolicSpacingConstruct::CompoundAssignment { operator },
            MissingWhitespace::Before,
        );
    }
}

/// Plain assignment `=` requires whitespace on both sides.
#[test]
fn rejects_assignment_spacing() {
    for (source, missing) in [
        ("count=1\n", MissingWhitespace::Both),
        ("count =1\n", MissingWhitespace::After),
        ("count= 1\n", MissingWhitespace::Before),
    ] {
        let (diagnostic, _string_table) = tokenize_source_error(source);
        assert_symbolic_spacing(&diagnostic, SymbolicSpacingConstruct::Assignment, missing);
    }
}

/// `~=` is tokenized as `Mutable` + `Assign` and must enforce outer whitespace.
/// The tokenizer inspects the complete adjacent marker before reporting either outer side.
#[test]
fn rejects_mutable_declaration_spacing() {
    for (source, missing) in [
        ("count~= 1\n", MissingWhitespace::Before),
        ("count ~=1\n", MissingWhitespace::After),
        ("count~=1\n", MissingWhitespace::Both),
    ] {
        let (diagnostic, _string_table) = tokenize_source_error(source);
        assert_symbolic_spacing(
            &diagnostic,
            SymbolicSpacingConstruct::MutableDeclaration,
            missing,
        );
    }
}

/// Internal whitespace inside the mutable marker pair (`name ~ = value`) is not a
/// tokenizer spacing error. The tokenizer accepts it and the declaration parser
/// owns the `InvalidMutableBindingSpacing` rejection.
#[test]
fn internal_mutable_marker_whitespace_does_not_trigger_symbolic_spacing() {
    for source in ["value ~ = 42\n", "value ~ =42\n"] {
        let (file_tokens, _string_table) = tokenize_source(source);
        assert!(
            token_refs(file_tokens.tokens.as_ref()).any(|token| token.tag() == TokenTag::MUTABLE),
            "expected a Mutable token in `{source}`"
        );
    }
}

/// Numeric tokens preserve both `source_text` and `normalized_text`.
///
/// WHAT: verifies that authored source text (with separators and attached sign)
///       is stored alongside normalized text for diagnostics and materialization.
/// WHY: diagnostics should report what the author typed; materialization should
///      use separator-free, lowercase text.
#[test]
fn numeric_token_preserves_source_and_normalized_text() {
    for (source, expected_source, expected_normalized) in [
        ("value = 1_000.50e-10\n", "1_000.50e-10", "1000.50e-10"),
        ("value = -1_000\n", "-1_000", "1000"),
        ("value = -1.0e+21\n", "-1.0e+21", "1.0e+21"),
    ] {
        let (file_tokens, string_table) = tokenize_source(source);
        let numeric_token = token_refs(file_tokens.tokens.as_ref())
            .find_map(numeric_payload)
            .expect("expected a numeric literal token");

        let resolved_source = string_table.resolve(numeric_token.source_text);
        let resolved_normalized = string_table.resolve(numeric_token.normalized_text);

        assert_eq!(
            resolved_source, expected_source,
            "source_text mismatch for: {source}"
        );
        assert_eq!(
            resolved_normalized, expected_normalized,
            "normalized_text mismatch for: {source}"
        );
    }
}

/// Signed numeric tokens store the attached sign in `source_text` but
/// `normalized_text` remains unsigned.
#[test]
fn signed_numeric_token_source_text_includes_sign() {
    let (file_tokens, string_table) = tokenize_source("value = -42\n");
    let numeric_token = token_refs(file_tokens.tokens.as_ref())
        .find_map(numeric_payload)
        .expect("expected a numeric literal token");

    assert_eq!(numeric_token.sign, NumericLiteralSign::Negative);
    assert_eq!(string_table.resolve(numeric_token.source_text), "-42");
    assert_eq!(string_table.resolve(numeric_token.normalized_text), "42");
}

/// Out-of-range literal with separators is accepted by the tokenizer
/// and the source_text preserves the authored form for the materialization
/// diagnostic.
#[test]
fn out_of_range_literal_with_separators_preserves_authored_text() {
    let (file_tokens, string_table) = tokenize_source("value = 9_999_999_999\n");

    let numeric_token = token_refs(file_tokens.tokens.as_ref())
        .find_map(numeric_payload)
        .expect("expected a numeric literal token");

    // The tokenizer accepts it; source_text preserves separators for later
    // materialization diagnostics.
    assert_eq!(
        string_table.resolve(numeric_token.source_text),
        "9_999_999_999"
    );
    assert_eq!(
        string_table.resolve(numeric_token.normalized_text),
        "9999999999"
    );
}

/// Diagnostic for an uppercase exponent on a negative signed literal
/// preserves the full authored text including the sign.
#[test]
fn uppercase_exponent_on_signed_literal_preserves_authored_text() {
    let (error, string_table) = tokenize_source_error("value = -1E6\n");

    assert_invalid_number_literal(
        &error,
        &string_table,
        "-1E6",
        NumberLiteralErrorReason::UppercaseExponentMarker,
    );
}

#[test]
fn tokenizer_does_not_steal_parser_owned_punctuation_diagnostics() {
    tokenize_source(r#"scores = {"Priya" =}"#);
}

#[test]
fn tokenizes_reserved_trait_keywords_as_reserved_tokens() {
    let (file_tokens, _string_table) = tokenize_source("must This\n");
    let tokens = file_tokens.tokens.as_ref();

    assert_eq!(token_at(tokens, 0).tag(), TokenTag::MODULE_START);
    assert_eq!(token_at(tokens, 1).tag(), TokenTag::MUST);
    assert_eq!(token_at(tokens, 2).tag(), TokenTag::TRAIT_THIS);
    assert_ne!(token_at(tokens, 1).tag(), TokenTag::SYMBOL);
    assert_ne!(token_at(tokens, 2).tag(), TokenTag::SYMBOL);
}

#[test]
fn tokenizes_generic_keywords_as_reserved_tokens() {
    let (file_tokens, _string_table) = tokenize_source("type of\n");
    let tokens = file_tokens.tokens.as_ref();

    assert_eq!(token_at(tokens, 1).tag(), TokenTag::TYPE);
    assert_eq!(token_at(tokens, 2).tag(), TokenTag::OF);
    assert_ne!(token_at(tokens, 1).tag(), TokenTag::SYMBOL);
    assert_ne!(token_at(tokens, 2).tag(), TokenTag::SYMBOL);
}

#[test]
fn tokenizes_lowercase_this_as_reserved_receiver_keyword() {
    let (file_tokens, string_table) = tokenize_source("this this_value This _this\n");
    let tokens = file_tokens.tokens.as_ref();

    assert_eq!(token_at(tokens, 0).tag(), TokenTag::MODULE_START);
    assert_eq!(token_at(tokens, 1).tag(), TokenTag::THIS);
    assert_eq!(token_at(tokens, 2).tag(), TokenTag::SYMBOL);
    assert!(token_string_is(
        token_at(tokens, 2),
        &string_table,
        "this_value"
    ));
    assert_eq!(token_at(tokens, 3).tag(), TokenTag::TRAIT_THIS);
    assert_eq!(token_at(tokens, 4).tag(), TokenTag::SYMBOL);
    assert!(token_string_is(token_at(tokens, 4), &string_table, "_this"));
    assert_ne!(token_at(tokens, 1).tag(), TokenTag::SYMBOL);
}

#[test]
fn tokenizes_declared_region_names_as_symbols_and_semantic_scope_words_as_keywords() {
    let (file_tokens, string_table) = tokenize_source("block group region checked async\n");
    let tokens = file_tokens.tokens.as_ref();

    for (index, word) in [(1, "block"), (2, "group"), (3, "region")] {
        assert_eq!(token_at(tokens, index).tag(), TokenTag::SYMBOL);
        assert!(
            token_string_is(token_at(tokens, index), &string_table, word),
            "expected '{word}' to lex as an ordinary symbol"
        );
    }
    assert_eq!(token_at(tokens, 4).tag(), TokenTag::CHECKED);
    assert_eq!(token_at(tokens, 5).tag(), TokenTag::ASYNC);
}

#[test]
fn tokenizes_assert_as_reserved_keyword() {
    let (file_tokens, _string_table) = tokenize_source("assert\n");
    let token = token_at(file_tokens.tokens.as_ref(), 1);
    assert_eq!(token.tag(), TokenTag::ASSERT);
    assert_ne!(token.tag(), TokenTag::SYMBOL);
}

#[test]
fn tokenizes_attached_bang_keyword_forms_as_compound_tokens() {
    let (file_tokens, _string_table) = tokenize_source("return! err\ncast! text\n");
    let tokens = file_tokens.tokens.as_ref();

    assert!(token_refs(tokens).any(|token| token.tag() == TokenTag::RETURN_BANG));
    assert!(token_refs(tokens).any(|token| token.tag() == TokenTag::CAST_BANG));
}

#[test]
fn tokenizes_spaced_bang_keyword_forms_as_separate_tokens() {
    let (file_tokens, _string_table) = tokenize_source("return ! err\ncast ! text\n");
    let tokens = file_tokens.tokens.as_ref();

    assert!(
        !token_refs(tokens)
            .any(|token| { matches!(token.tag(), TokenTag::RETURN_BANG | TokenTag::CAST_BANG) }),
        "spaced keyword/bang pairs must not become compound tokens"
    );
    assert!(
        token_refs(tokens)
            .filter(|token| token.tag() == TokenTag::BANG)
            .count()
            >= 2,
        "expected spaced keyword/bang pairs to keep standalone bang tokens"
    );
}

#[test]
fn tokenizes_export_as_reserved_keyword() {
    let (file_tokens, _string_table) = tokenize_source("export\n");
    let token = token_at(file_tokens.tokens.as_ref(), 1);
    assert_eq!(token.tag(), TokenTag::EXPORT);
    assert_ne!(token.tag(), TokenTag::SYMBOL);
}

#[test]
fn template_body_preserves_export_as_literal_text() {
    let (file_tokens, string_table) = tokenize_source("[: this contains export keyword]");
    let tokens = file_tokens.tokens.as_ref();
    let body_literal = token_refs(tokens)
        .find_map(|token| {
            (token.tag() == TokenTag::STRING_SLICE_LITERAL)
                .then(|| string_table.resolve(token.string_id().expect("literal string handle")))
        })
        .filter(|value| value.contains("export"))
        .expect("expected template body text to preserve 'export' as literal text");

    assert!(
        !token_refs(tokens).any(|token| token.tag() == TokenTag::EXPORT),
        "export inside a template body should not tokenize as a keyword"
    );
    assert!(body_literal.contains("export"));
}

#[test]
fn tokenizes_panic_as_normal_symbol() {
    let (file_tokens, string_table) = tokenize_source("panic\n");
    let token = token_at(file_tokens.tokens.as_ref(), 1);
    assert_eq!(token.tag(), TokenTag::SYMBOL);
    assert!(token_string_is(token, &string_table, "panic"));
}

#[test]
fn tokenizes_standalone_underscore_as_wildcard_but_prefixed_names_as_symbols() {
    let (file_tokens, string_table) = tokenize_source("_ _true __value\n");
    let tokens = file_tokens.tokens.as_ref();

    assert_eq!(token_at(tokens, 1).tag(), TokenTag::WILDCARD);
    assert_eq!(token_at(tokens, 2).tag(), TokenTag::SYMBOL);
    assert!(token_string_is(token_at(tokens, 2), &string_table, "_true"));
    assert_eq!(token_at(tokens, 3).tag(), TokenTag::SYMBOL);
    assert!(token_string_is(
        token_at(tokens, 3),
        &string_table,
        "__value"
    ));
}

#[test]
fn tokenizes_in_as_symbol_after_loop_syntax_removal() {
    let (file_tokens, string_table) = tokenize_source("in\n");
    let token = token_at(file_tokens.tokens.as_ref(), 1);
    assert_eq!(token.tag(), TokenTag::SYMBOL);
    assert!(token_string_is(token, &string_table, "in"));
}

#[test]
fn tokenizes_pipe_bindings_in_loop_headers() {
    let (file_tokens, string_table) = tokenize_source("loop items |item, index|:\n;\n");
    let tokens = file_tokens.tokens.as_ref();

    let loop_index = find_token_index(tokens, |token| token.tag() == TokenTag::LOOP);
    let items_index = find_token_index(tokens, |token| {
        token.tag() == TokenTag::SYMBOL && token_string_is(token, &string_table, "items")
    });
    let item_index = find_token_index(tokens, |token| {
        token.tag() == TokenTag::SYMBOL && token_string_is(token, &string_table, "item")
    });
    let index_index = find_token_index(tokens, |token| {
        token.tag() == TokenTag::SYMBOL && token_string_is(token, &string_table, "index")
    });
    let first_pipe = find_token_index(tokens, |token| {
        token.tag() == TokenTag::TYPE_PARAMETER_BRACKET
    });
    let second_pipe = token_refs(tokens)
        .enumerate()
        .skip(first_pipe + 1)
        .find_map(|(idx, token)| (token.tag() == TokenTag::TYPE_PARAMETER_BRACKET).then_some(idx))
        .expect("expected closing pipe token");

    assert!(loop_index < items_index);
    assert!(items_index < first_pipe);
    assert!(first_pipe < item_index);
    assert!(item_index < index_index);
    assert!(index_index < second_pipe);
}

#[test]
fn tokenizes_bare_loop_bindings_without_special_keyword_support() {
    let (file_tokens, string_table) = tokenize_source("loop items item, index:\n;\n");
    let tokens = file_tokens.tokens.as_ref();

    let items_index = find_token_index(tokens, |token| {
        token.tag() == TokenTag::SYMBOL && token_string_is(token, &string_table, "items")
    });
    let item_index = find_token_index(tokens, |token| {
        token.tag() == TokenTag::SYMBOL && token_string_is(token, &string_table, "item")
    });
    let comma_index = find_token_index(tokens, |token| token.tag() == TokenTag::COMMA);
    let index_index = find_token_index(tokens, |token| {
        token.tag() == TokenTag::SYMBOL && token_string_is(token, &string_table, "index")
    });

    assert!(items_index < item_index);
    assert!(item_index < comma_index);
    assert!(comma_index < index_index);
}

#[test]
fn tokenizes_none_question_mark_bang_and_catch_markers() {
    let (file_tokens, _string_table) = tokenize_source(
        "value String? = none\npersist()!\nrecover = may_fail() catch:\n    then \"\"\n;\n",
    );
    let tokens = file_tokens.tokens.as_ref();

    assert!(
        token_refs(tokens).any(|token| token.tag() == TokenTag::QUESTION_MARK),
        "expected '?' optional-type marker token"
    );
    assert!(
        token_refs(tokens).any(|token| token.tag() == TokenTag::NONE_LITERAL),
        "expected lowercase 'none' literal token"
    );
    assert!(
        token_refs(tokens)
            .filter(|token| token.tag() == TokenTag::BANG)
            .count()
            >= 1,
        "expected bang token for propagation call handling"
    );
    assert!(
        token_refs(tokens).any(|token| token.tag() == TokenTag::CATCH),
        "expected catch token for fallback call handling"
    );
    assert!(
        token_refs(tokens).any(|token| token.tag() == TokenTag::THEN),
        "expected then token for catch fallback handling"
    );
}

#[test]
fn tokenizes_style_directives_inside_template_heads() {
    let (file_tokens, string_table) = tokenize_source("[$md, $fresh: body]");
    let tokens = file_tokens.tokens.as_ref();

    let outer_head = find_token_index(tokens, |token| token.tag() == TokenTag::TEMPLATE_HEAD);
    let markdown = find_token_index(tokens, |token| {
        token.tag() == TokenTag::STYLE_DIRECTIVE && token_string_is(token, &string_table, "md")
    });
    let fresh = find_token_index(tokens, |token| {
        token.tag() == TokenTag::STYLE_DIRECTIVE && token_string_is(token, &string_table, "fresh")
    });

    assert!(outer_head < markdown);
    assert!(markdown < fresh);
    assert_eq!(token_at(tokens, markdown).tag(), TokenTag::STYLE_DIRECTIVE);
    assert_eq!(token_at(tokens, fresh).tag(), TokenTag::STYLE_DIRECTIVE);
}

#[test]
fn tokenizes_qualified_choice_inside_nested_template_head_before_body_delimiter() {
    let (file_tokens, _string_table) =
        tokenize_source("[: [handle_status(Status::Running): body]]");
    let tokens = file_tokens.tokens.as_ref();

    let template_head_count = token_refs(tokens)
        .filter(|token| token.tag() == TokenTag::TEMPLATE_HEAD)
        .count();
    let body_start_indices: Vec<usize> = token_refs(tokens)
        .enumerate()
        .filter_map(|(index, token)| {
            (token.tag() == TokenTag::START_TEMPLATE_BODY).then_some(index)
        })
        .collect();
    let double_colon_index =
        find_token_index(tokens, |token| token.tag() == TokenTag::DOUBLE_COLON);

    assert_eq!(template_head_count, 2);
    assert_eq!(body_start_indices.len(), 2);
    assert!(body_start_indices[0] < double_colon_index);
    assert!(double_colon_index < body_start_indices[1]);
    assert_eq!(
        token_refs(tokens)
            .filter(|token| token.tag() == TokenTag::DOUBLE_COLON)
            .count(),
        1
    );
}

#[test]
fn rejects_legacy_reset_style_directive_name() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let source_path = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let mut span_builder = ExtendedSpanBuilder::new();
    let source = "[$reset: body]";
    let Err(diagnostic) = tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    ) else {
        panic!("legacy reset directive should be rejected");
    };
    let error = expect_lexical_diagnostic(diagnostic);

    match &error.payload {
        DiagnosticPayload::InvalidStyleDirective { directive_name, .. } => {
            assert_eq!(string_table.resolve(*directive_name), "reset");
        }
        payload => panic!("expected invalid style directive payload, found {payload:?}"),
    }
    let (start, end) = diagnostic_byte_range(&error);
    assert_eq!(
        source.get(start as usize..end as usize),
        Some("$reset"),
        "legacy directive diagnostic should cover the authored directive name",
    );
}

#[test]
fn rejects_unknown_style_directive_name() {
    let (error, string_table) = tokenize_source_error("[$unknown_formatter: body]");

    match &error.payload {
        DiagnosticPayload::InvalidStyleDirective { directive_name, .. } => {
            assert_eq!(string_table.resolve(*directive_name), "unknown_formatter");
        }
        payload => panic!("expected invalid style directive payload, found {payload:?}"),
    }
}

#[test]
fn tokenizes_children_directive_with_template_argument() {
    let (file_tokens, string_table) = tokenize_source("[$children([:prefix]), $md:\nhello\n]");
    let tokens = file_tokens.tokens.as_ref();

    let outer_head = find_token_index(tokens, |token| token.tag() == TokenTag::TEMPLATE_HEAD);
    let children = find_token_index(tokens, |token| {
        token.tag() == TokenTag::STYLE_DIRECTIVE
            && token_string_is(token, &string_table, "children")
    });
    let open_paren = find_token_index(tokens, |token| token.tag() == TokenTag::OPEN_PARENTHESIS);
    let child_template = token_refs(tokens)
        .enumerate()
        .skip(open_paren + 1)
        .find_map(|(index, token)| (token.tag() == TokenTag::TEMPLATE_HEAD).then_some(index))
        .expect("expected child template opener");
    let close = token_refs(tokens)
        .enumerate()
        .skip(child_template + 1)
        .find_map(|(index, token)| (token.tag() == TokenTag::TEMPLATE_CLOSE).then_some(index))
        .expect("expected the child template to close");
    let close_paren = token_refs(tokens)
        .enumerate()
        .skip(close + 1)
        .find_map(|(index, token)| (token.tag() == TokenTag::CLOSE_PARENTHESIS).then_some(index))
        .expect("expected ')' after the child template");
    let comma = token_refs(tokens)
        .enumerate()
        .skip(close_paren + 1)
        .find_map(|(index, token)| (token.tag() == TokenTag::COMMA).then_some(index))
        .expect("expected a comma after the child template");
    let markdown = token_refs(tokens)
        .enumerate()
        .skip(comma + 1)
        .find_map(|(index, token)| {
            (token.tag() == TokenTag::STYLE_DIRECTIVE
                && token_string_is(token, &string_table, "md"))
            .then_some(index)
        })
        .expect("expected the outer head to continue with '$md'");

    assert!(outer_head < children);
    assert!(children < open_paren);
    assert!(open_paren < child_template);
    assert!(child_template < close);
    assert!(close < close_paren);
    assert!(close_paren < comma);
    assert!(comma < markdown);
}

#[test]
fn rejects_legacy_style_child_template_prefix_syntax() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let source_path = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");

    let mut span_builder = ExtendedSpanBuilder::new();
    let result = tokenize(
        "[$[:prefix], $md:\nhello\n]",
        source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    );
    assert!(
        result.is_err(),
        "legacy '$[' child-template syntax should fail"
    );
}

#[test]
fn tokenizes_reactive_marker_outside_template_heads() {
    let (file_tokens, _string_table) = tokenize_source("$String\n");
    let tokens = file_tokens.tokens.as_ref();

    assert_eq!(token_at(tokens, 1).tag(), TokenTag::REACTIVE);
    assert_eq!(token_at(tokens, 2).tag(), TokenTag::DATATYPE_STRING);
    assert!(
        !token_refs(tokens).any(|token| token.tag() == TokenTag::STYLE_DIRECTIVE),
        "ordinary code should not produce style directive tokens"
    );
}

#[test]
fn tokenizes_template_reactive_subscription_marker() {
    let (file_tokens, _string_table) = tokenize_source("[:[$(count)]]");
    let tokens = file_tokens.tokens.as_ref();

    assert!(
        token_refs(tokens).any(|token| token.tag() == TokenTag::REACTIVE),
        "`$(` in a template head should produce the reactive marker"
    );
    assert!(
        !token_refs(tokens).any(|token| token.tag() == TokenTag::STYLE_DIRECTIVE),
        "`$(` is subscription syntax, not a style directive"
    );
}

#[test]
fn unknown_style_directives_fail_under_strict_registry() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let source_path = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");

    let mut span_builder = ExtendedSpanBuilder::new();
    let source = "[$unknown: value]";
    let Err(diagnostic) = tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    ) else {
        panic!("unknown directive should fail during tokenization");
    };
    let error = expect_lexical_diagnostic(diagnostic);

    match &error.payload {
        DiagnosticPayload::InvalidStyleDirective { directive_name, .. } => {
            assert_eq!(string_table.resolve(*directive_name), "unknown");
        }
        payload => panic!("expected invalid style directive payload, found {payload:?}"),
    }
    let (start, end) = diagnostic_byte_range(&error);
    assert_eq!(
        source.get(start as usize..end as usize),
        Some("$unknown"),
        "unknown directive diagnostic should cover the authored directive name",
    );
}

#[test]
fn tokenizes_slot_and_insert_directives_inside_template_heads() {
    let (file_tokens, string_table) =
        tokenize_source("[wrapper: [$slot][$slot(\"style\")][$insert(\"style\"): blue]]");
    let tokens = file_tokens.tokens.as_ref();

    let slot_directive_count = token_refs(tokens)
        .filter(|token| {
            token.tag() == TokenTag::STYLE_DIRECTIVE
                && token_string_is(*token, &string_table, "slot")
        })
        .count();
    let has_insert_directive = token_refs(tokens).any(|token| {
        token.tag() == TokenTag::STYLE_DIRECTIVE && token_string_is(token, &string_table, "insert")
    });

    assert_eq!(slot_directive_count, 2);
    assert!(has_insert_directive);
    assert!(token_refs(tokens).any(|token| token.tag() == TokenTag::STRING_SLICE_LITERAL));
}

#[test]
fn rejects_numeric_slot_directive_prefixes() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let source_path = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");

    let mut span_builder = ExtendedSpanBuilder::new();
    let result = tokenize(
        "[wrapper: [$1: first]]",
        source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    );
    assert!(
        result.is_err(),
        "legacy numeric '$1' slot directives should fail"
    );
}

#[test]
fn code_template_body_keeps_nested_square_brackets_as_literal_text() {
    let (file_tokens, string_table) =
        tokenize_source("[$code(\"bst\"):\nconcatenated = [string_slice, a_mutable_string]\n]");
    let tokens = file_tokens.tokens.as_ref();

    let template_heads = token_refs(tokens)
        .filter(|token| token.tag() == TokenTag::TEMPLATE_HEAD)
        .count();
    let template_closes = token_refs(tokens)
        .filter(|token| token.tag() == TokenTag::TEMPLATE_CLOSE)
        .count();

    assert_eq!(
        template_heads, 1,
        "code template bodies should not tokenize nested '[' as template opens"
    );
    assert_eq!(template_closes, 1);

    let body_literal = token_refs(tokens)
        .filter(|&token| token.tag() == TokenTag::STRING_SLICE_LITERAL)
        .map(|token| string_table.resolve(token.string_id().expect("literal string handle")))
        .find(|value| value.contains("[string_slice, a_mutable_string]"))
        .expect("expected code template body text to include literal square brackets");

    assert!(body_literal.contains("concatenated"));
}

#[test]
fn css_template_body_keeps_selector_brackets_as_literal_text() {
    let (file_tokens, string_table) =
        tokenize_html_source("[$css:\n.button[data-kind=\"cta\"] { color: red; }\n]");
    let tokens = file_tokens.tokens.as_ref();

    let template_heads = token_refs(tokens)
        .filter(|token| token.tag() == TokenTag::TEMPLATE_HEAD)
        .count();
    let template_closes = token_refs(tokens)
        .filter(|token| token.tag() == TokenTag::TEMPLATE_CLOSE)
        .count();

    assert_eq!(
        template_heads, 1,
        "css template bodies should not tokenize selector brackets as nested templates"
    );
    assert_eq!(template_closes, 1);

    let body_literal = token_refs(tokens)
        .find_map(|token| {
            (token.tag() == TokenTag::STRING_SLICE_LITERAL)
                .then(|| string_table.resolve(token.string_id().expect("literal string handle")))
        })
        .filter(|value| value.contains("[data-kind=\"cta\"]"))
        .expect("expected css template body text to include selector brackets");

    assert!(body_literal.contains(".button"));
}

#[test]
fn html_template_body_tokenizes_attribute_brackets_using_normal_rules() {
    let (file_tokens, string_table) =
        tokenize_html_source("[$html:\n<div data-tags=\"[one,two]\">Hello</div>\n]");
    let tokens = file_tokens.tokens.as_ref();

    let template_heads = token_refs(tokens)
        .filter(|token| token.tag() == TokenTag::TEMPLATE_HEAD)
        .count();
    let template_closes = token_refs(tokens)
        .filter(|token| token.tag() == TokenTag::TEMPLATE_CLOSE)
        .count();

    assert_eq!(
        template_heads, 2,
        "normal template-body parsing should tokenize '[one,two]' as a nested template in $html"
    );
    assert_eq!(template_closes, 2);

    assert!(
        token_refs(tokens).any(|token| {
            token.tag() == TokenTag::SYMBOL && token_string_is(token, &string_table, "one")
        }),
        "expected nested template symbol 'one' from bracket content"
    );
    assert!(
        token_refs(tokens).any(|token| {
            token.tag() == TokenTag::SYMBOL && token_string_is(token, &string_table, "two")
        }),
        "expected nested template symbol 'two' from bracket content"
    );

    let preserves_literal_attribute_brackets = token_refs(tokens).any(|token| {
        token.tag() == TokenTag::STRING_SLICE_LITERAL
            && token
                .string_id()
                .is_some_and(|id| string_table.resolve(id).contains("data-tags=\"[one,two]\""))
    });
    assert!(
        !preserves_literal_attribute_brackets,
        "normal $html tokenization should not preserve attribute bracket lists as one literal slice"
    );
}

#[test]
fn html_template_body_tokenizes_slot_templates_inside_quoted_attributes_with_normal_rules() {
    let (file_tokens, string_table) = tokenize_html_source(
        "[$html:\n<h1 style=\"font-size: 2em;[$slot(\"style\")]\">[$slot]</h1>\n]",
    );
    let tokens = file_tokens.tokens.as_ref();

    let template_heads = token_refs(tokens)
        .filter(|token| token.tag() == TokenTag::TEMPLATE_HEAD)
        .count();
    let template_closes = token_refs(tokens)
        .filter(|token| token.tag() == TokenTag::TEMPLATE_CLOSE)
        .count();

    assert_eq!(
        template_heads, 3,
        "normal template-body parsing should still tokenize slot templates inside quoted attributes"
    );
    assert_eq!(template_closes, 3);

    let slot_directives = token_refs(tokens)
        .filter(|token| {
            token.tag() == TokenTag::STYLE_DIRECTIVE
                && token_string_is(*token, &string_table, "slot")
        })
        .count();
    assert_eq!(slot_directives, 2);
}

#[test]
fn html_template_body_tokenizes_symbol_wrappers_with_general_template_rules() {
    let (file_tokens, string_table) =
        tokenize_html_source("[$html:\n[title, center: LANGUAGE BASICS]\n]");
    let tokens = file_tokens.tokens.as_ref();

    let template_heads = token_refs(tokens)
        .filter(|token| token.tag() == TokenTag::TEMPLATE_HEAD)
        .count();
    let template_closes = token_refs(tokens)
        .filter(|token| token.tag() == TokenTag::TEMPLATE_CLOSE)
        .count();

    assert_eq!(
        template_heads, 2,
        "normal template-body parsing should tokenize wrapper syntax in $html bodies"
    );
    assert_eq!(template_closes, 2);

    assert!(token_refs(tokens).any(|token| {
        token.tag() == TokenTag::SYMBOL && token_string_is(token, &string_table, "title")
    }));
    assert!(token_refs(tokens).any(|token| {
        token.tag() == TokenTag::SYMBOL && token_string_is(token, &string_table, "center")
    }));
}

#[test]
fn custom_balanced_directive_uses_general_balanced_mode() {
    let directives = vec![StyleDirectiveSpec::handler(
        "highlight",
        TemplateBodyMode::Balanced,
        TemplateHeadCompatibility::fully_compatible_meaningful(),
        StyleDirectiveHandlerSpec::no_op(),
    )];
    let (file_tokens, string_table) =
        tokenize_source_with_directives("[$highlight:\n[data-kind=\"cta\"]\n]", &directives);
    let tokens = file_tokens.tokens.as_ref();

    let template_heads = token_refs(tokens)
        .filter(|token| token.tag() == TokenTag::TEMPLATE_HEAD)
        .count();
    let template_closes = token_refs(tokens)
        .filter(|token| token.tag() == TokenTag::TEMPLATE_CLOSE)
        .count();

    assert_eq!(template_heads, 1);
    assert_eq!(template_closes, 1);
    let body_literal = token_refs(tokens)
        .find_map(|token| {
            (token.tag() == TokenTag::STRING_SLICE_LITERAL)
                .then(|| string_table.resolve(token.string_id().expect("literal string handle")))
        })
        .filter(|value| value.contains("[data-kind=\"cta\"]"))
        .expect("expected balanced directive body to keep brackets as literal text");
    assert!(body_literal.contains("data-kind"));
}

#[test]
fn note_and_todo_template_bodies_are_discarded_until_balanced_close() {
    for directive in ["note", "todo"] {
        let source = format!(
            "[${directive}:\n[this [body] has [nested [brackets]] and should be discarded]\n]"
        );
        let (file_tokens, string_table) = tokenize_source(&source);
        let tokens = file_tokens.tokens.as_ref();

        let template_heads = token_refs(tokens)
            .filter(|token| token.tag() == TokenTag::TEMPLATE_HEAD)
            .count();
        let template_closes = token_refs(tokens)
            .filter(|token| token.tag() == TokenTag::TEMPLATE_CLOSE)
            .count();

        assert_eq!(template_heads, 1);
        assert_eq!(template_closes, 1);
        assert!(token_refs(tokens).any(|token| {
            token.tag() == TokenTag::STYLE_DIRECTIVE
                && token_string_is(token, &string_table, directive)
        }));
        assert!(
            !token_refs(tokens).any(|token| {
                token.tag() == TokenTag::STRING_SLICE_LITERAL
                    && token
                        .string_id()
                        .is_some_and(|id| string_table.resolve(id).contains("discarded"))
            }),
            "expected ${directive} body text to be discarded during tokenization"
        );
    }
}

#[test]
fn doc_template_body_keeps_nested_templates_as_template_tokens() {
    let (file_tokens, string_table) = tokenize_source("[$doc:\n[: child]\n]");
    let tokens = file_tokens.tokens.as_ref();

    let template_heads = token_refs(tokens)
        .filter(|token| token.tag() == TokenTag::TEMPLATE_HEAD)
        .count();
    let template_closes = token_refs(tokens)
        .filter(|token| token.tag() == TokenTag::TEMPLATE_CLOSE)
        .count();

    assert_eq!(
        template_heads, 2,
        "expected doc body nested template to tokenize as a child template"
    );
    assert_eq!(template_closes, 2);
    assert!(token_refs(tokens).any(|token| {
        token.tag() == TokenTag::STYLE_DIRECTIVE && token_string_is(token, &string_table, "doc")
    }));
}

#[test]
fn parent_relative_path_does_not_receive_missing_at_prefix_correction() {
    // `import` is an ordinary identifier and `../` is not supported with `@`, so the tokenizer
    // must not fabricate a dependency path or suggest `@../`.
    let (file_tokens, _string_table) = tokenize_source("import ../utils\n");
    assert!(
        !token_refs(file_tokens.tokens.as_ref()).any(token_has_path_payload),
        "parent-relative bare import should not tokenize as a path"
    );
}

#[test]
fn valid_at_prefixed_paths_and_operators_remain_unaffected() {
    for source in [
        "@core/math\n",
        "@./utils\n",
        "value = a / b\n",
        "value = a // b\n",
        "-- comment\n",
    ] {
        let (file_tokens, _string_table) = tokenize_source(source);
        assert!(
            token_refs(file_tokens.tokens.as_ref()).any(|token| token.tag() == TokenTag::EOF),
            "expected valid source to tokenize: {source}"
        );
    }

    let (at_core, _string_table) = tokenize_source("@core/math\n");
    assert!(
        token_refs(at_core.tokens.as_ref()).any(token_has_path_payload),
        "valid @-prefixed dependency should produce a path token"
    );
}

#[test]
fn ordinary_import_identifier_does_not_receive_path_correction() {
    let (file_tokens, _string_table) = tokenize_source("import as drawing\n");
    assert!(
        token_refs(file_tokens.tokens.as_ref()).any(|token| token.tag() == TokenTag::AS),
        "ordinary `import` followed by `as` should tokenize both words independently"
    );
}

/// Token start lines must use the same LF, CRLF and bare-CR boundaries as the lazy line index.
///
/// The fixture drives every path that consumes a carriage return: a whitespace tail, a CRLF
/// statement break, a physical CR inside a quoted string and one inside a template body. Those
/// paths reach the stream through different helpers, so a table that disagreed with only one of
/// them would still pass a single-shape fixture.
///
/// Exact token spans are resolved to bytes before line-index checks; columns are intentionally
/// derived by the line index rather than reconstructed from tokenizer state.
#[test]
fn token_start_lines_match_line_index_across_newline_shapes() {
    let source =
        "name = \"café😀\"  \r  next\nvalue = 'q'\r\ntext = \"a\rb\"\nbody = [:one\rtwo]\nlast = 2";
    let (file_tokens, _string_table) = tokenize_source(source);
    let line_starts = line_start_offsets(source);
    let line_index = LineIndex::new(source, &line_starts);

    let mut lines_with_token_starts: Vec<u32> = Vec::new();
    let mut columns_checked = 0;

    for token in token_refs(file_tokens.tokens.as_ref()) {
        let (start_byte, end_byte) = token_byte_range(token);
        let line = line_index
            .line_of_offset(start_byte)
            .expect("every token start, including EOF, must be addressable");
        if lines_with_token_starts.last() != Some(&line) {
            lines_with_token_starts.push(line);
        }

        if start_byte == end_byte {
            continue;
        }

        let authored = source
            .get(start_byte as usize..end_byte as usize)
            .expect("token byte ranges must stay on UTF-8 boundaries");
        let first_authored_scalar = authored
            .chars()
            .next()
            .expect("non-empty token ranges must contain a scalar");
        if matches!(first_authored_scalar, '\r' | '\n') {
            continue;
        }

        // A column names an offset inside one line's visible text, so the reported column must
        // round-trip back to the token's own byte start.
        let position = line_index
            .position(start_byte)
            .expect("the token start position must resolve");
        let line_text = line_index
            .line_text(line)
            .expect("a token on a non-empty line must have visible text");
        let line_start = line_index
            .line_byte_range(line)
            .expect("a line carrying a token must have a byte range")
            .start;
        let (offset_in_line, scalar_at_column) = line_text
            .char_indices()
            .nth(position.column as usize)
            .expect("a reported column must name a scalar in that line's visible text");
        assert_eq!(
            scalar_at_column,
            first_authored_scalar,
            "{:?}: line-index column must point at the token's first authored scalar",
            token.tag()
        );
        assert_eq!(
            line_start + offset_in_line as u32,
            start_byte,
            "{:?}: line-index column must round-trip to the token's byte start",
            token.tag()
        );
        columns_checked += 1;
    }

    // Every authored line carries a token start, including the two that a multi-line string and
    // a multi-line template body end on, so no CR shape in the fixture goes unmeasured.
    assert_eq!(
        lines_with_token_starts,
        (0..=7).collect::<Vec<u32>>(),
        "every authored line must carry a token start"
    );
    // The column check skips newline and multi-line tokens, so a future tokenizer change that
    // skipped everything would leave it vacuous.
    assert!(
        columns_checked >= 10,
        "the column check ran on only {columns_checked} tokens"
    );
}

/// A discarded template body still anchors its closing bracket at the authored byte.
///
/// The lexer skips a discarded body's text, so the close span is checked directly against its
/// authored byte range and then mapped through the lazy line index.
#[test]
fn discarded_template_body_close_resolves_to_its_authored_line() {
    let source = "[$note:one\rtwo]x = 1";
    let (file_tokens, _string_table) = tokenize_source(source);
    let line_starts = line_start_offsets(source);
    let line_index = LineIndex::new(source, &line_starts);

    let close = token_refs(file_tokens.tokens.as_ref())
        .find(|token| token.tag() == TokenTag::TEMPLATE_CLOSE)
        .expect("a discarded body must still emit its closing bracket");

    let (start_byte, end_byte) = token_byte_range(close);
    assert_eq!(
        source.get(start_byte as usize..end_byte as usize),
        Some("]")
    );
    assert_eq!(
        line_index.line_of_offset(start_byte),
        Some(1),
        "the closing bracket is authored on the line after the discarded body's carriage return"
    );
}

/// Every token's exact span must recover exactly the text its author wrote.
///
/// The expectations are authored lexemes sliced from the byte range carried by each token.
#[test]
fn token_byte_ranges_recover_the_authored_text() {
    let source = "name = \"café\"\n[outer: a\r\nb[inner: nested]c\rd]\n";
    let (file_tokens, _string_table) = tokenize_source(source);

    let authored: Vec<&str> = token_refs(file_tokens.tokens.as_ref())
        .map(|token| {
            let (start, end) = token_byte_range(token);
            &source[start as usize..end as usize]
        })
        .collect();

    assert_eq!(
        authored,
        vec![
            "", // ModuleStart, before any authored byte
            "name",
            "=",
            "\"café\"", // multi-byte scalar inside the quoted span
            "\n",
            "[",
            "outer",
            ":",
            " a\r\nb", // template body text spanning a CRLF
            "[",
            "inner",
            ":",
            " nested",
            "]",
            // The body's token value normalizes `\r` to `\n`; the byte range still covers the
            // carriage return the author actually wrote.
            "c\rd",
            "]",
            "\n",
            "", // Eof
        ],
        "token byte ranges must slice the authored lexemes"
    );

    let source_len = source.len() as u32;
    let mut previous_end = 0u32;
    for token in token_refs(file_tokens.tokens.as_ref()) {
        let (start_byte, end_byte) = token_byte_range(token);

        assert!(
            start_byte <= end_byte,
            "token byte range must be half-open and well-ordered: {start_byte}..{end_byte}"
        );
        assert!(
            end_byte <= source_len,
            "token end_byte {end_byte} exceeds source length {source_len}"
        );
        assert!(
            start_byte >= previous_end,
            "token ranges must not overlap: {start_byte} precedes the previous end {previous_end}"
        );
        previous_end = end_byte;
    }
}

/// Tokens returned while skipping trivia must span the token, never the trivia before it.
///
/// Whitespace runs, comments and discarded template bodies are consumed without producing a
/// token, so each of these sources previously left the following token anchored at the start of
/// the discarded text.
#[test]
fn skipped_trivia_is_excluded_from_the_following_token_range() {
    let cases: &[(&str, &[&str])] = &[
        ("name   ", &["", "name", ""]),
        ("a  \nb", &["", "a", "\n", "b", ""]),
        // A run of blank lines is one boundary token spanning exactly that run.
        ("\n\n  \nz", &["", "\n\n  \n", "z", ""]),
        ("-- hi\n", &["", "\n", ""]),
        ("-- hi", &["", ""]),
        // The discarded body is skipped; only its closing bracket is emitted.
        ("[$note: abc]", &["", "[", "$note", ":", "]", ""]),
        ("y!", &["", "y", "!", ""]),
        ("`raw`", &["", "`raw`", ""]),
    ];

    for (source, expected) in cases {
        let (file_tokens, _string_table) = tokenize_source(source);
        let authored: Vec<&str> = token_refs(file_tokens.tokens.as_ref())
            .map(|token| {
                let (start, end) = token_byte_range(token);
                &source[start as usize..end as usize]
            })
            .collect();
        assert_eq!(authored, *expected, "{source:?}: token byte ranges");

        let end_of_source = source.len() as u32;
        let final_token = token_refs(file_tokens.tokens.as_ref())
            .last()
            .expect("tokenization always emits Eof");
        assert_eq!(
            token_byte_range(final_token),
            (end_of_source, end_of_source),
            "{source:?}: Eof is a zero-width insertion point at the end of the source"
        );
    }
}

#[test]
fn malformed_and_unclosed_diagnostics_keep_recorded_byte_ranges() {
    let string_source = "prefix = 1\r\nvalue = \"authored text";
    let (string_diagnostic, _string_table) = tokenize_source_error(string_source);

    assert_eq!(
        string_diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnterminatedStringLiteral)
    );
    let (string_start, string_end) = diagnostic_byte_range(&string_diagnostic);
    assert_eq!(
        string_source.get(string_start as usize..string_end as usize),
        Some("\"authored text"),
        "unterminated-string diagnostics must retain the authored byte range"
    );
    let string_line_starts = line_start_offsets(string_source);
    let string_line_index = LineIndex::new(string_source, &string_line_starts);

    assert_eq!(string_line_index.line_of_offset(string_start), Some(1));
    assert_eq!(string_line_index.line_of_offset(string_end), Some(1));

    // The lexer's own end-of-source path: a style directive name that never arrives.
    let template_source = "prefix = 1\r\nvalue = [$";
    let (template_diagnostic, _template_string_table) = tokenize_source_error(template_source);

    assert_eq!(
        template_diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedEndOfFile)
    );
    let (template_start, template_end) = diagnostic_byte_range(&template_diagnostic);
    assert_eq!(
        template_source.get(template_start as usize..template_end as usize),
        Some("$"),
        "an end-of-source diagnostic names the authored character it could not complete"
    );

    let template_line_starts = line_start_offsets(template_source);
    let template_line_index = LineIndex::new(template_source, &template_line_starts);

    assert_eq!(
        template_line_index.line_of_offset(template_start),
        Some(1),
        "the reported range belongs to the line the author left unfinished"
    );
}

/// The token span is resolved against the producer's exact byte table, and the recovered slice is
/// compared against the lexeme the author wrote.
///
/// Token starts must not run backwards, but they may leave gaps, which is where the tokenizer
/// skipped trivia or discarded a template body.
#[test]
fn every_token_span_matches_authored_bytes() {
    let source = "name = \"café😀\"\r\nvalue = \"quoted\"\rbody = [$md:\nbody\r]\n[$note:\ndiscarded\r]\nlast = 2";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("span-bridge.moth", &mut string_table)
        .expect("test path fits");
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = tokenize(
        source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &frontend_test_style_directives(),
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("span bridge fixture should tokenize");
    let resolver = span_builder.resolver();

    assert!(
        token_refs(file_tokens.tokens.as_ref()).any(|token| token.tag() == TokenTag::TEMPLATE_HEAD)
    );
    assert!(
        token_refs(file_tokens.tokens.as_ref())
            .any(|token| token.tag() == TokenTag::STRING_SLICE_LITERAL)
    );

    let mut previous_end = 0u32;
    let authored: Vec<&str> = token_refs(file_tokens.tokens.as_ref())
        .map(|token| {
            let resolved = token.span().resolve_with(resolver);
            assert!(
                resolved.start() >= previous_end,
                "{:?} must not start before the previous token ended",
                token.tag()
            );
            previous_end = resolved.end();

            source
                .get(resolved.start() as usize..resolved.end() as usize)
                .expect("a span must name a character-boundary range of its own source")
        })
        .collect();

    assert_eq!(
        authored,
        vec![
            "", // ModuleStart, before any authored byte
            "name",
            "=",
            "\"café😀\"", // an astral scalar inside the quoted span
            "\r\n",
            "value",
            "=",
            "\"quoted\"",
            "\r", // a bare carriage return ends the line on its own
            "body",
            "=",
            "[",
            "$md",
            ":",
            "\nbody\r", // template body text spanning a bare carriage return
            "]",
            "\n",
            "[",
            "$note",
            ":",
            "]", // the discarded body's closing bracket, anchored past the skipped run
            "\n",
            "last",
            "=",
            "2",
            "", // Eof
        ],
        "resolved spans must slice the authored lexemes"
    );
}

#[test]
fn extended_token_span_resolves_exactly_through_live_builder() {
    let quoted = "x".repeat(1500);
    let source = format!("value = \"{quoted}\"");
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("long-token.moth", &mut string_table)
        .expect("test path fits");
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = tokenize(
        &source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &frontend_test_style_directives(),
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("long token should tokenize");
    let resolver = span_builder.resolver();
    let token = token_refs(file_tokens.tokens.as_ref())
        .find(|token| token.tag() == TokenTag::STRING_SLICE_LITERAL)
        .expect("long string token");
    let resolved = token.span().resolve_with(resolver);

    assert_eq!(
        span_builder.len(),
        1,
        "the long string is the only token past the inline length limit, so it owns the one row"
    );
    let expected_start = source.find('"').expect("quoted token start") as u32;
    assert_eq!(resolved.start(), expected_start);
    assert_eq!(resolved.end(), expected_start + quoted.len() as u32 + 2);
    assert_eq!(
        resolved.end() - resolved.start(),
        u32::try_from(quoted.len() + 2).expect("fixture length fits in u32")
    );
}

#[test]
fn lexical_failure_retains_extended_token_span_builder_rows() {
    let quoted = "x".repeat(1500);
    let source = format!("value = \"{quoted}\"'");
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("failed-long-token.moth", &mut string_table)
        .expect("test path fits");
    let canonical_path = path_fork.render_native(source_path, &string_table, &mut Vec::new());
    let sources =
        SourceDatabase::build([&canonical_path], &canonical_path, None, &mut string_table)
            .expect("the physical fixture should register");
    let expected_file_id = sources
        .get_by_canonical_path(&canonical_path)
        .expect("the fixture should have a source identity")
        .id;

    let mut span_builder = ExtendedSpanBuilder::new();
    let Err(diagnostic) = tokenize(
        &source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &frontend_test_style_directives(),
        &mut string_table,
        &mut path_fork,
        expected_file_id,
        &mut span_builder,
    ) else {
        panic!("the malformed trailing character should abort tokenization");
    };
    let diagnostic = expect_lexical_diagnostic(diagnostic);

    let (diagnostic_start, _diagnostic_end) = diagnostic_byte_range(&diagnostic);
    assert_eq!(diagnostic_start, (source.len() - 1) as u32);
    assert_eq!(
        span_builder.len(),
        1,
        "the already-emitted long string token must retain its extended row on failure"
    );

    // Encode the expected long range at index zero in an independent builder, then resolve that
    // handle through the failure's retained builder. This checks the row's exact range rather than
    // only proving that some capacity was retained.
    let mut probe_builder = ExtendedSpanBuilder::new();
    let expected_span = LocalSpan::exact(8, (quoted.len() + 2) as u32, &mut probe_builder)
        .expect("the expected long string range should be representable");
    let resolved = expected_span.resolve_with(span_builder.resolver());
    assert_eq!(resolved.start(), 8);
    assert_eq!(resolved.end(), (8 + quoted.len() + 2) as u32);
}

#[test]
fn diagnostic_span_allocates_exactly_when_needed() {
    let source = "x".repeat(3000);
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut stream = TokenStream::new(
        &source,
        SourceId::COMPILATION_ROOT,
        TokenizerEntryMode::SourceFile,
        &mut span_builder,
    );

    for _ in 0..1500 {
        stream.next();
    }
    stream.start_byte_offset = 0;
    stream
        .emit_string_literal(StringId::from_index(0))
        .expect("the long authored string token should encode");
    assert_eq!(
        stream.extended_span_builder.len(),
        1,
        "the long authored token should require one extended row"
    );

    for _ in 0..1500 {
        stream.next();
    }
    let diagnostic_span = stream
        .source_span_for_bytes(1500, 3000)
        .expect("the diagnostic range should be representable");
    let diagnostic_range = diagnostic_span
        .local()
        .resolve_with(stream.extended_span_builder.resolver());
    assert_eq!(
        (diagnostic_range.start(), diagnostic_range.end()),
        (1500, 3000)
    );
    assert_eq!(
        stream.extended_span_builder.len(),
        2,
        "the authored token and exact diagnostic range each retain their extended row"
    );
}

/// The returned diagnostic keeps exact UTF-8 bounds after a long token has used the original table.
#[test]
fn preparation_diagnostics_carry_exact_spans_through_the_original_builder() {
    let quoted = "😀".repeat(600);
    let source = format!("value = \"{quoted}\"\nx = \"\\🦋");
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork
        .try_intern_portable_path("diagnosed-long-token.moth", &mut string_table)
        .expect("test path fits");
    let canonical_path = path_fork.render_native(source_path, &string_table, &mut Vec::new());
    let sources =
        SourceDatabase::build([&canonical_path], &canonical_path, None, &mut string_table)
            .expect("the physical fixture should register");
    let file_id = sources
        .get_by_canonical_path(&canonical_path)
        .expect("the fixture should have a source identity")
        .id;

    let mut span_builder = ExtendedSpanBuilder::new();
    let diagnostic = tokenize(
        &source,
        source_path,
        TokenizerEntryMode::SourceFile,
        &frontend_test_style_directives(),
        &mut string_table,
        &mut path_fork,
        file_id,
        &mut span_builder,
    )
    .expect_err("the unsupported escape should abort tokenization");
    let diagnostic = expect_lexical_diagnostic(diagnostic);
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidStringEscape {
            reason: InvalidStringEscapeReason::UnsupportedEscape { escaped: '🦋' }
        }
    ));
    assert_eq!(
        span_builder.len(),
        1,
        "the preceding long token uses an extended row"
    );

    let expected_start = source
        .find("\\🦋")
        .expect("fixture should contain the escape") as u32;
    let expected_end = expected_start + "\\🦋".len() as u32;
    let span = diagnostic
        .primary_span
        .expect("tokenize must capture its diagnosed range");
    assert_eq!(span.source(), file_id);
    let resolved = span.local().resolve_with(span_builder.resolver());
    assert_eq!(
        (resolved.start(), resolved.end()),
        (expected_start, expected_end),
    );

    let mut database_builder = SourceDatabaseBuilder::new(sources);
    database_builder
        .sources_mut()
        .retain_text(file_id, source.clone())
        .expect("the diagnosed source should retain its snapshot");
    database_builder.retain_span_builder(file_id, span_builder);
    let installed = database_builder
        .finish()
        .expect("the original builder should install once");
    let resolved = span.byte_range(&installed);
    assert_eq!(
        (resolved.start(), resolved.end()),
        (expected_start, expected_end)
    );
    assert_eq!(
        &source[resolved.start() as usize..resolved.end() as usize],
        "\\🦋"
    );
}

/// Canonical token payloads retain their typed values and authored spans.
///
/// WHAT: lexes numeric literals (signed, separator, exponent), an authored path, two string
/// shapes, a char and a bool through `tokenize`, then inspects each source-owned token directly.
/// WHY: lexer tests must exercise the compact canonical store and inspect typed payloads directly.
#[test]
fn lexed_canonical_shapes_retain_typed_payloads() {
    // The raw-string branch runs before trivia skipping, so a backtick first reached after
    // whitespace on the same line is not recognized as a raw literal in code mode (pre-existing
    // lexer behavior, unchanged by this phase); the raw token leads the source instead.
    let (file_tokens, string_table) = tokenize_source(
        "`raw`\nvalue = -1_000\nsecond = 1.0e-10\npath = @core/math\ntext = \"text\"\nletter = 'q'\nflag = true\n",
    );
    let owner = file_tokens.tokens.as_ref();

    assert!(owner.len() >= 2);
    assert_eq!(
        file_tokens.file_id,
        owner.source(),
        "canonical token owner must retain the lexer's source identity"
    );

    let mut saw_negative_numeric = false;
    let mut saw_separator_numeric = false;
    let mut saw_exponent_numeric = false;
    let mut saw_path = false;
    let mut saw_quoted_string = false;
    let mut saw_raw_string = false;
    let mut saw_char = false;
    let mut saw_bool = false;

    for (index, token) in token_refs(owner).enumerate() {
        let token_index = TokenIndex::try_from_index(index).expect("fixture range fits");
        assert_eq!(
            token.index(),
            token_index,
            "canonical index mismatch at {index}"
        );
        match token.tag() {
            TokenTag::SYMBOL | TokenTag::STYLE_DIRECTIVE => {
                let id = token
                    .string_id()
                    .expect("string-shaped token must carry a handle");
                string_table.resolve(id);
            }
            TokenTag::STRING_SLICE_LITERAL | TokenTag::RAW_STRING_LITERAL => {
                let id = token
                    .string_id()
                    .expect("literal token must carry a handle");
                let text = string_table.resolve(id);
                if token.tag() == TokenTag::STRING_SLICE_LITERAL {
                    if text == "text" {
                        saw_quoted_string = true;
                    }
                } else if text == "raw" {
                    saw_raw_string = true;
                }
            }
            TokenTag::NUMERIC_LITERAL => {
                let numeric = numeric_payload(token).expect("numeric token must carry a literal");
                let source_text = string_table.resolve(numeric.source_text);
                let normalized_text = string_table.resolve(numeric.normalized_text);
                if numeric.sign == NumericLiteralSign::Negative {
                    saw_negative_numeric = true;
                    assert_eq!(source_text, "-1_000");
                }
                if normalized_text == "1000" {
                    saw_separator_numeric = true;
                }
                if normalized_text == "1.0e-10" {
                    saw_exponent_numeric = true;
                }
            }
            TokenTag::PATH => {
                let path_id = token
                    .path_syntax_id()
                    .expect("path token must carry a path handle");
                file_tokens
                    .path_syntax
                    .try_path(path_id)
                    .expect("path handle must resolve in the lexer's source table");
                saw_path = true;
            }
            TokenTag::CHAR_LITERAL => {
                assert_eq!(token.char_value(), Some('q'));
                saw_char = true;
            }
            TokenTag::BOOL_LITERAL => {
                assert_eq!(token.bool_value(), Some(true));
                saw_bool = true;
            }
            _ => {}
        }
    }

    assert!(
        saw_negative_numeric,
        "the fixture must contain a signed numeric literal"
    );
    assert!(
        saw_separator_numeric,
        "the fixture must contain a separator literal"
    );
    assert!(
        saw_exponent_numeric,
        "the fixture must contain an exponent literal"
    );
    assert!(saw_path, "the fixture must contain an authored path");
    assert!(
        saw_quoted_string,
        "the fixture must contain a quoted string literal"
    );
    assert!(
        saw_raw_string,
        "the fixture must contain a raw string literal"
    );
    assert!(saw_char, "the fixture must contain a char literal");
    assert!(saw_bool, "the fixture must contain a bool literal");
}

/// Token-count exhaustion stays on the typed user diagnostic lane without huge allocation.
///
/// WHAT: exercises the production half-open token-store count predicate at its maximum and
///       immediate overflow, keeps the distinct zero-based token-index and one-based numeric
///       handle domain edges visible, and proves the builder's capacity lane maps to
///       `SourceSpanCapacityResource::Token` through `map_source_token_build_error`.
/// WHY: the lexer reports `SourceTokenBuildError::Capacity` as a user diagnostic while malformed
///      trusted records stay infrastructure failures; allocating billions of tokens to reach the
///      edge would be absurd, so the bounded domain-plus-mapping check is the contract.
#[test]
fn token_count_exhaustion_reports_a_typed_user_capacity_diagnostic() {
    use crate::compiler_frontend::numeric_text::store::NumericLiteralId;

    let max_store_length = u32::MAX as usize;
    assert!(
        token_store_length_fits(max_store_length),
        "u32::MAX is the largest valid half-open token-store length"
    );
    assert!(
        token_store_append_fits(max_store_length - 1),
        "a store at u32::MAX - 1 may accept one append"
    );
    assert!(
        !token_store_append_fits(max_store_length),
        "an append resulting in u32::MAX + 1 tokens must be rejected"
    );
    let max_plus_one = max_store_length
        .checked_add(1)
        .expect("this target must represent the first invalid token-store length");
    assert!(
        !token_store_length_fits(max_plus_one),
        "the resulting length u32::MAX + 1 must be rejected"
    );

    assert!(
        TokenIndex::try_from_index(u32::MAX as usize).is_some(),
        "the token domain addresses its own maximum index"
    );
    assert_eq!(
        TokenIndex::try_from_index(max_plus_one),
        None,
        "the token index domain cannot address another token past its u32 domain"
    );
    assert_eq!(
        NumericLiteralId::try_from_index(u32::MAX as usize),
        None,
        "the positional numeric handle domain ends where the staged lane overflows"
    );

    let failure = map_source_token_build_error(SourceTokenBuildError::Capacity);
    let diagnostic = match failure {
        TokenizeFailure::Diagnosed(diagnostic) => diagnostic,
        TokenizeFailure::Infrastructure(error) => {
            panic!("capacity must map to the user lane, found infrastructure: {error:?}")
        }
    };
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::SourceSpanCapacity)
    );
    match diagnostic.payload {
        DiagnosticPayload::SourceSpanCapacity { resource, .. } => {
            assert_eq!(resource, SourceSpanCapacityResource::Token);
        }
        payload => panic!("expected token capacity payload, found {payload:?}"),
    }
}

/// A malformed trusted compact payload stays on the infrastructure invariant lane.
///
/// WHAT: pushes the absent path handle through the canonical source-token builder and asserts
///       `SourceTokenBuildError::Invariant`, not the capacity lane.
/// WHY: capacity exhaustion is user-controlled and diagnosed; a trusted record that cannot pack
///      is a compiler invariant violation and must never surface as a user diagnostic.
#[test]
fn malformed_trusted_record_reports_the_invariant_lane() {
    let source = SourceId::COMPILATION_ROOT;
    let malformed = PathSyntaxId::NONE;
    let mut builder = SourceTokensBuilder::with_capacity(source, 1);
    match builder.push_payload(
        TokenTag::PATH,
        0,
        malformed.raw(),
        LocalSpan::source_start(),
    ) {
        Err(SourceTokenBuildError::Invariant(_)) => {}
        Err(SourceTokenBuildError::Capacity) => {
            panic!("malformed trusted record must not report the capacity lane")
        }
        Ok(()) => panic!("absent path handle must not pack"),
    }
}
