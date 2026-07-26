use super::*;
use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticRenderContext, terminal::format_payload_guidance,
};
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, CompilerDiagnostic, DiagnosticCompoundAssignmentOperator,
    DiagnosticKind, DiagnosticOperator, DiagnosticPayload, InvalidStringEscapeReason,
    MissingWhitespace, NumberLiteralErrorReason, SymbolicSpacingConstruct, SymbolicSpacingError,
    SyntaxDiagnosticKind,
};
use crate::compiler_frontend::numeric_text::token::NumericLiteralSign;
use crate::compiler_frontend::style_directives::{
    StyleDirectiveHandlerSpec, StyleDirectiveRegistry, StyleDirectiveSpec,
    TemplateHeadCompatibility,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_tests::test_support::frontend_test_style_directives;
use crate::projects::html_project::style_directives::html_project_style_directives;

fn html_project_test_style_directives() -> StyleDirectiveRegistry {
    StyleDirectiveRegistry::merged(&html_project_style_directives())
        .expect("html project style directives should merge with core directives")
}

fn tokenize_source(source: &str) -> (FileTokens, StringTable) {
    let style_directives = frontend_test_style_directives();
    tokenize_source_with_registry(source, &style_directives)
}

fn tokenize_html_source(source: &str) -> (FileTokens, StringTable) {
    let style_directives = html_project_test_style_directives();
    tokenize_source_with_registry(source, &style_directives)
}

fn tokenize_source_error(source: &str) -> (CompilerDiagnostic, StringTable) {
    let mut string_table = StringTable::new();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let source_path = InternedPath::from_single_str("test.moth", &mut string_table);
    let diagnostic = tokenize(
        source,
        &source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        None,
    )
    .expect_err("tokenization should fail");
    (*diagnostic, string_table)
}

fn tokenize_source_with_registry(
    source: &str,
    style_directives: &StyleDirectiveRegistry,
) -> (FileTokens, StringTable) {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("test.moth", &mut string_table);
    let file_tokens = tokenize(
        source,
        &source_path,
        TokenizerEntryMode::SourceFile,
        style_directives,
        &mut string_table,
        None,
    )
    .expect("tokenization should succeed");
    (file_tokens, string_table)
}

fn tokenize_source_with_directives(
    source: &str,
    directives: &[StyleDirectiveSpec],
) -> (FileTokens, StringTable) {
    let mut string_table = StringTable::new();
    let source_path = InternedPath::from_single_str("test.moth", &mut string_table);
    let registry = StyleDirectiveRegistry::merged(directives)
        .expect("test style directives should merge with core directives");
    let file_tokens = tokenize(
        source,
        &source_path,
        TokenizerEntryMode::SourceFile,
        &registry,
        &mut string_table,
        None,
    )
    .expect("tokenization should succeed");
    (file_tokens, string_table)
}

fn tokenize_moth_template_source(source: &str) -> (FileTokens, StringTable) {
    let mut string_table = StringTable::new();
    let style_directives = frontend_test_style_directives();
    let source_path = InternedPath::from_single_str("test.mtf", &mut string_table);
    let file_tokens = tokenize(
        source,
        &source_path,
        TokenizerEntryMode::for_source_file_kind(SourceFileKind::MothTemplate)
            .expect("Moth template should tokenize"),
        &style_directives,
        &mut string_table,
        None,
    )
    .expect("Moth template tokenization should succeed");
    (file_tokens, string_table)
}

fn tokenize_moth_template_error(source: &str) -> (CompilerDiagnostic, StringTable) {
    let mut string_table = StringTable::new();
    let style_directives = frontend_test_style_directives();
    let source_path = InternedPath::from_single_str("test.mtf", &mut string_table);
    let diagnostic = tokenize(
        source,
        &source_path,
        TokenizerEntryMode::for_source_file_kind(SourceFileKind::MothTemplate)
            .expect("Moth template should tokenize"),
        &style_directives,
        &mut string_table,
        None,
    )
    .expect_err("Moth template tokenization should fail");
    (*diagnostic, string_table)
}

fn find_token_index(tokens: &[Token], predicate: impl Fn(&TokenKind) -> bool) -> usize {
    tokens
        .iter()
        .position(|token| predicate(&token.kind))
        .expect("expected token to be present")
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

fn numeric_literal_signs(file_tokens: &FileTokens) -> Vec<NumericLiteralSign> {
    file_tokens
        .tokens
        .iter()
        .filter_map(|token| match &token.kind {
            TokenKind::NumericLiteral(token) => Some(token.sign),
            _ => None,
        })
        .collect()
}

fn collect_literal_texts(file_tokens: &FileTokens, string_table: &StringTable) -> Vec<String> {
    file_tokens
        .tokens
        .iter()
        .filter_map(|token| match token.kind {
            TokenKind::StringSliceLiteral(id) | TokenKind::RawStringLiteral(id) => {
                Some(string_table.resolve(id).to_owned())
            }
            _ => None,
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
    assert_eq!(
        diagnostic.primary_location.start_pos.line_number,
        diagnostic.primary_location.end_pos.line_number
    );
    assert_eq!(
        diagnostic.primary_location.end_pos.char_column
            - diagnostic.primary_location.start_pos.char_column
            + 1,
        expected_span_width
    );

    match &diagnostic.payload {
        DiagnosticPayload::InvalidStringEscape { reason } => {
            assert_eq!(*reason, expected_reason);
        }
        payload => panic!("expected invalid string escape payload, found {payload:?}"),
    }
}

#[test]
fn quoted_string_decodes_every_accepted_escape() {
    // Source escapes: \\ \" \n \r \t decode to backslash, quote, newline, carriage return, tab.
    let (file_tokens, string_table) = tokenize_source(r#"value = "a\\b\"c\nd\re\tf""#);
    let texts = collect_literal_texts(&file_tokens, &string_table);

    assert_eq!(texts, vec!["a\\b\"c\nd\re\tf"]);
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
    let (diagnostic, string_table) = tokenize_moth_template_error("]");

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
    assert_eq!(
        diagnostic
            .primary_location
            .scope
            .to_portable_string(&string_table),
        "test.mtf"
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
    let template_closes = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateClose))
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

    assert!(
        file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::IntDivide)),
        "expected '//' to tokenize as IntDivide"
    );
    assert!(
        !file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::DivideAssign)),
        "integer division token should not be confused with '/='"
    );
}

#[test]
fn tokenizes_double_slash_equals_as_integer_division_assignment_operator() {
    let (file_tokens, _string_table) = tokenize_source("value ~= 10\nvalue //= 3\n");

    assert!(
        file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::IntDivideAssign)),
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
fn tokenizes_lowercase_exponent_literals() {
    let (file_tokens, string_table) = tokenize_source("value = 1e6 1e-6 1e+6 1.0e+21\n");
    let numeric_texts: Vec<String> = file_tokens
        .tokens
        .iter()
        .filter_map(|token| match &token.kind {
            TokenKind::NumericLiteral(token) => {
                Some(string_table.resolve(token.normalized_text).to_owned())
            }
            _ => None,
        })
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
    let numeric_texts: Vec<String> = file_tokens
        .tokens
        .iter()
        .filter_map(|token| match &token.kind {
            TokenKind::NumericLiteral(token) => {
                Some(string_table.resolve(token.normalized_text).to_owned())
            }
            _ => None,
        })
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

    assert!(
        file_tokens.tokens.iter().any(|token| matches!(
            &token.kind,
            TokenKind::NumericLiteral(token) if token.sign == NumericLiteralSign::Negative
        )),
        "`-1` after a spaced binary operator should remain one signed numeric token"
    );
    assert!(
        !file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::Negative)),
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

    let pattern_token = file_tokens
        .tokens
        .windows(2)
        .find_map(|tokens| match tokens {
            [
                Token {
                    kind: TokenKind::NumericLiteral(numeric_token),
                    ..
                },
                Token {
                    kind: TokenKind::FatArrow,
                    ..
                },
            ] if numeric_token.sign == NumericLiteralSign::Negative => Some(numeric_token),
            _ => None,
        })
        .expect("expected a numeric literal immediately before the negative arm arrow");

    assert_eq!(pattern_token.sign, NumericLiteralSign::Negative);
    assert_eq!(
        string_table.resolve(pattern_token.source_text),
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

    let pattern_index = file_tokens
        .tokens
        .iter()
        .position(|token| {
            matches!(
                &token.kind,
                TokenKind::NumericLiteral(numeric_token)
                    if numeric_token.sign == NumericLiteralSign::Negative
            )
        })
        .expect("expected a negative numeric literal in the guarded match arm");

    assert!(matches!(
        file_tokens
            .tokens
            .get(pattern_index + 1)
            .map(|token| &token.kind),
        Some(TokenKind::If)
    ));
    assert!(
        file_tokens.tokens[pattern_index..]
            .iter()
            .any(|token| matches!(token.kind, TokenKind::FatArrow))
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

    let pattern_index = file_tokens
        .tokens
        .iter()
        .position(|token| {
            matches!(
                &token.kind,
                TokenKind::NumericLiteral(numeric_token)
                    if numeric_token.sign == NumericLiteralSign::Negative
            )
        })
        .expect("expected a negative numeric literal in the multiline guarded match arm");

    assert!(matches!(
        file_tokens
            .tokens
            .get(pattern_index + 1)
            .map(|token| &token.kind),
        Some(TokenKind::If)
    ));
    assert!(
        file_tokens.tokens[pattern_index..]
            .windows(2)
            .any(|tokens| {
                matches!(
                    tokens,
                    [
                        Token {
                            kind: TokenKind::BoolLiteral(true),
                            ..
                        },
                        Token {
                            kind: TokenKind::FatArrow,
                            ..
                        }
                    ]
                )
            })
    );
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

    let pattern_index = file_tokens
        .tokens
        .iter()
        .position(|token| {
            matches!(
                &token.kind,
                TokenKind::NumericLiteral(numeric_token)
                    if numeric_token.sign == NumericLiteralSign::Negative
            )
        })
        .expect("expected a negative numeric literal in the named-argument guarded match arm");

    assert!(matches!(
        file_tokens
            .tokens
            .get(pattern_index + 1)
            .map(|token| &token.kind),
        Some(TokenKind::If)
    ));

    let arm_arrow_index = file_tokens.tokens[pattern_index..]
        .iter()
        .position(|token| matches!(token.kind, TokenKind::FatArrow))
        .map(|offset| pattern_index + offset)
        .expect("expected the named-argument guarded match arm arrow");
    assert!(matches!(
        file_tokens
            .tokens
            .get(
                arm_arrow_index
                    .checked_sub(1)
                    .expect("arrow must have a preceding token")
            )
            .map(|token| &token.kind),
        Some(TokenKind::CloseParenthesis)
    ));
}

#[test]
fn tokenizes_attached_unary_negation_for_non_numeric_operands() {
    let (file_tokens, _string_table) = tokenize_source("value = -count\nother = total * -count\n");

    let negative_count = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::Negative))
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
        "count /= 4\n",
        "count //= 3\n",
        "count %= 5\n",
        "count ^= 2\n",
        "count ~= 1\n",
    ] {
        let (file_tokens, _string_table) = tokenize_source(source);
        assert!(
            file_tokens.tokens.iter().any(|token| {
                matches!(
                    token.kind,
                    TokenKind::AddAssign
                        | TokenKind::SubtractAssign
                        | TokenKind::MultiplyAssign
                        | TokenKind::DivideAssign
                        | TokenKind::IntDivideAssign
                        | TokenKind::ModulusAssign
                        | TokenKind::ExponentAssign
                        | TokenKind::Mutable
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

/// Compound assignments with left spacing but missing right spacing (`count +=1`) must fail.
#[test]
fn rejects_compound_assignment_missing_right_spacing() {
    for (source, operator) in [
        ("count +=1\n", DiagnosticCompoundAssignmentOperator::Add),
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
            file_tokens
                .tokens
                .iter()
                .any(|token| matches!(token.kind, TokenKind::Mutable)),
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
        let numeric_token = file_tokens
            .tokens
            .iter()
            .find_map(|token| match &token.kind {
                TokenKind::NumericLiteral(t) => Some(t),
                _ => None,
            })
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
    let numeric_token = file_tokens
        .tokens
        .iter()
        .find_map(|token| match &token.kind {
            TokenKind::NumericLiteral(t) => Some(t),
            _ => None,
        })
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

    let numeric_token = file_tokens
        .tokens
        .iter()
        .find_map(|token| match &token.kind {
            TokenKind::NumericLiteral(t) => Some(t),
            _ => None,
        })
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
    tokenize_source("value = identity<Int>(42)\n");
    tokenize_source(r#"scores = {"Priya" =}"#);
}

#[test]
fn tokenizes_reserved_trait_keywords_as_reserved_tokens() {
    let (file_tokens, _string_table) = tokenize_source("must This\n");

    assert!(
        matches!(file_tokens.tokens[0].kind, TokenKind::ModuleStart),
        "token streams always begin with the module sentinel"
    );
    assert!(
        matches!(file_tokens.tokens[1].kind, TokenKind::Must),
        "expected 'must' to lex as a reserved trait token"
    );
    assert!(
        matches!(file_tokens.tokens[2].kind, TokenKind::TraitThis),
        "expected 'This' to lex as a reserved trait token"
    );
    assert!(
        !matches!(file_tokens.tokens[1].kind, TokenKind::Symbol(_)),
        "'must' should not remain a user symbol"
    );
    assert!(
        !matches!(file_tokens.tokens[2].kind, TokenKind::Symbol(_)),
        "'This' should not remain a user symbol"
    );
}

#[test]
fn tokenizes_generic_keywords_as_reserved_tokens() {
    let (file_tokens, _string_table) = tokenize_source("type of\n");

    assert!(
        matches!(file_tokens.tokens[1].kind, TokenKind::Type),
        "expected 'type' to lex as a reserved keyword token"
    );
    assert!(
        matches!(file_tokens.tokens[2].kind, TokenKind::Of),
        "expected 'of' to lex as a reserved keyword token"
    );
    assert!(
        !matches!(file_tokens.tokens[1].kind, TokenKind::Symbol(_)),
        "'type' should not remain a user symbol"
    );
    assert!(
        !matches!(file_tokens.tokens[2].kind, TokenKind::Symbol(_)),
        "'of' should not remain a user symbol"
    );
}

#[test]
fn tokenizes_lowercase_this_as_reserved_receiver_keyword() {
    let (file_tokens, _string_table) = tokenize_source("this this_value This _this\n");

    assert!(
        matches!(file_tokens.tokens[0].kind, TokenKind::ModuleStart),
        "token streams always begin with the module sentinel"
    );
    assert!(
        matches!(file_tokens.tokens[1].kind, TokenKind::This),
        "expected 'this' to lex as a reserved receiver token"
    );
    assert!(
        matches!(file_tokens.tokens[2].kind, TokenKind::Symbol(_)),
        "expected 'this_value' to remain a user symbol"
    );
    assert!(
        matches!(file_tokens.tokens[3].kind, TokenKind::TraitThis),
        "expected 'This' to lex as a reserved trait token"
    );
    assert!(
        matches!(file_tokens.tokens[4].kind, TokenKind::Symbol(_)),
        "expected '_this' to remain a user symbol (shadow policy rejects it later)"
    );
    assert!(
        !matches!(file_tokens.tokens[1].kind, TokenKind::Symbol(_)),
        "'this' should not remain a user symbol"
    );
}

#[test]
fn tokenizes_statement_block_keywords_as_reserved_tokens() {
    let (file_tokens, _string_table) = tokenize_source("block checked async\n");

    assert!(
        matches!(file_tokens.tokens[1].kind, TokenKind::Block),
        "expected 'block' to lex as a statement block token"
    );
    assert!(
        matches!(file_tokens.tokens[2].kind, TokenKind::Checked),
        "expected 'checked' to lex as a reserved checked block token"
    );
    assert!(
        matches!(file_tokens.tokens[3].kind, TokenKind::Async),
        "expected 'async' to lex as a reserved async block token"
    );
    assert!(
        !file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::Symbol(_))),
        "statement block keywords should not remain user symbols"
    );
}

#[test]
fn tokenizes_assert_as_reserved_keyword() {
    let (file_tokens, _string_table) = tokenize_source("assert\n");

    assert!(
        matches!(file_tokens.tokens[1].kind, TokenKind::Assert),
        "expected 'assert' to lex as a reserved keyword token"
    );
    assert!(
        !matches!(file_tokens.tokens[1].kind, TokenKind::Symbol(_)),
        "'assert' should not remain a user symbol"
    );
}

#[test]
fn tokenizes_attached_bang_keyword_forms_as_compound_tokens() {
    let (file_tokens, _string_table) = tokenize_source("return! err\ncast! text\n");

    assert!(
        file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::ReturnBang)),
        "expected attached 'return!' to lex as a single ReturnBang token"
    );
    assert!(
        file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::CastBang)),
        "expected attached 'cast!' to lex as a single CastBang token"
    );
}

#[test]
fn tokenizes_spaced_bang_keyword_forms_as_separate_tokens() {
    let (file_tokens, _string_table) = tokenize_source("return ! err\ncast ! text\n");

    assert!(
        !file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::ReturnBang | TokenKind::CastBang)),
        "spaced keyword/bang pairs must not become compound tokens"
    );
    assert!(
        file_tokens
            .tokens
            .iter()
            .filter(|token| matches!(token.kind, TokenKind::Bang))
            .count()
            >= 2,
        "expected spaced keyword/bang pairs to keep standalone bang tokens"
    );
}

#[test]
fn tokenizes_export_as_reserved_keyword() {
    let (file_tokens, _string_table) = tokenize_source("export\n");

    assert!(
        matches!(file_tokens.tokens[1].kind, TokenKind::Export),
        "expected 'export' to lex as a reserved keyword token"
    );
    assert!(
        !matches!(file_tokens.tokens[1].kind, TokenKind::Symbol(_)),
        "'export' should not remain a user symbol"
    );
}

#[test]
fn template_body_preserves_export_as_literal_text() {
    let (file_tokens, string_table) = tokenize_source("[: this contains export keyword]");

    let body_literal = file_tokens
        .tokens
        .iter()
        .find_map(|token| match token.kind {
            TokenKind::StringSliceLiteral(id) => {
                let value = string_table.resolve(id);
                value.contains("export").then_some(value)
            }
            _ => None,
        })
        .expect("expected template body text to preserve 'export' as literal text");

    assert!(
        !file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::Export)),
        "export inside a template body should not tokenize as a keyword"
    );
    assert!(body_literal.contains("export"));
}

#[test]
fn tokenizes_panic_as_normal_symbol() {
    let (file_tokens, string_table) = tokenize_source("panic\n");

    assert!(
        matches!(file_tokens.tokens[1].kind, TokenKind::Symbol(id) if string_table.resolve(id) == "panic"),
        "expected 'panic' to tokenize as a normal symbol, not a keyword"
    );
}

#[test]
fn tokenizes_standalone_underscore_as_wildcard_but_prefixed_names_as_symbols() {
    let (file_tokens, string_table) = tokenize_source("_ _true __value\n");

    assert!(
        matches!(file_tokens.tokens[1].kind, TokenKind::Wildcard),
        "expected standalone '_' to remain wildcard"
    );
    assert!(
        matches!(file_tokens.tokens[2].kind, TokenKind::Symbol(id) if string_table.resolve(id) == "_true"),
        "expected '_true' to tokenize as a symbol identifier"
    );
    assert!(
        matches!(file_tokens.tokens[3].kind, TokenKind::Symbol(id) if string_table.resolve(id) == "__value"),
        "expected '__value' to tokenize as a symbol identifier"
    );
}

#[test]
fn tokenizes_in_as_symbol_after_loop_syntax_removal() {
    let (file_tokens, string_table) = tokenize_source("in\n");

    assert!(
        matches!(file_tokens.tokens[1].kind, TokenKind::Symbol(id) if string_table.resolve(id) == "in"),
        "expected 'in' to tokenize as a normal symbol after loop-syntax removal"
    );
}

#[test]
fn tokenizes_pipe_bindings_in_loop_headers() {
    let (file_tokens, string_table) = tokenize_source("loop items |item, index|:\n;\n");

    let loop_index = find_token_index(&file_tokens.tokens, |kind| matches!(kind, TokenKind::Loop));
    let items_index = find_token_index(
        &file_tokens.tokens,
        |kind| matches!(kind, TokenKind::Symbol(id) if string_table.resolve(*id) == "items"),
    );
    let item_index = find_token_index(
        &file_tokens.tokens,
        |kind| matches!(kind, TokenKind::Symbol(id) if string_table.resolve(*id) == "item"),
    );
    let index_index = find_token_index(
        &file_tokens.tokens,
        |kind| matches!(kind, TokenKind::Symbol(id) if string_table.resolve(*id) == "index"),
    );
    let first_pipe = find_token_index(&file_tokens.tokens, |kind| {
        matches!(kind, TokenKind::TypeParameterBracket)
    });
    let second_pipe = file_tokens
        .tokens
        .iter()
        .enumerate()
        .skip(first_pipe + 1)
        .find_map(|(idx, token)| {
            matches!(token.kind, TokenKind::TypeParameterBracket).then_some(idx)
        })
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

    let items_index = find_token_index(
        &file_tokens.tokens,
        |kind| matches!(kind, TokenKind::Symbol(id) if string_table.resolve(*id) == "items"),
    );
    let item_index = find_token_index(
        &file_tokens.tokens,
        |kind| matches!(kind, TokenKind::Symbol(id) if string_table.resolve(*id) == "item"),
    );
    let comma_index =
        find_token_index(&file_tokens.tokens, |kind| matches!(kind, TokenKind::Comma));
    let index_index = find_token_index(
        &file_tokens.tokens,
        |kind| matches!(kind, TokenKind::Symbol(id) if string_table.resolve(*id) == "index"),
    );

    assert!(items_index < item_index);
    assert!(item_index < comma_index);
    assert!(comma_index < index_index);
}

#[test]
fn tokenizes_none_question_mark_bang_and_catch_markers() {
    let (file_tokens, _string_table) = tokenize_source(
        "value String? = none\npersist()!\nrecover = may_fail() catch:\n    then \"\"\n;\n",
    );

    assert!(
        file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::QuestionMark)),
        "expected '?' optional-type marker token"
    );
    assert!(
        file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::NoneLiteral)),
        "expected lowercase 'none' literal token"
    );
    assert!(
        file_tokens
            .tokens
            .iter()
            .filter(|token| matches!(token.kind, TokenKind::Bang))
            .count()
            >= 1,
        "expected bang token for propagation call handling"
    );
    assert!(
        file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::Catch)),
        "expected catch token for fallback call handling"
    );
    assert!(
        file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::Then)),
        "expected then token for catch fallback handling"
    );
}

#[test]
fn tokenizes_style_directives_inside_template_heads() {
    let (file_tokens, string_table) = tokenize_source("[$md, $fresh: body]");

    let outer_head = find_token_index(&file_tokens.tokens, |kind| {
        matches!(kind, TokenKind::TemplateHead)
    });
    let markdown = find_token_index(
        &file_tokens.tokens,
        |kind| matches!(kind, TokenKind::StyleDirective(id) if string_table.resolve(*id) == "md"),
    );
    let fresh = find_token_index(
        &file_tokens.tokens,
        |kind| matches!(kind, TokenKind::StyleDirective(id) if string_table.resolve(*id) == "fresh"),
    );

    assert!(outer_head < markdown);
    assert!(markdown < fresh);
    assert!(matches!(
        file_tokens.tokens[markdown].kind,
        TokenKind::StyleDirective(..)
    ));
    assert!(matches!(
        file_tokens.tokens[fresh].kind,
        TokenKind::StyleDirective(..)
    ));
}

#[test]
fn tokenizes_qualified_choice_inside_nested_template_head_before_body_delimiter() {
    let (file_tokens, _string_table) =
        tokenize_source("[: [handle_status(Status::Running): body]]");

    let template_head_count = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateHead))
        .count();
    let body_start_indices: Vec<usize> = file_tokens
        .tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| {
            matches!(token.kind, TokenKind::StartTemplateBody).then_some(index)
        })
        .collect();
    let double_colon_index = find_token_index(&file_tokens.tokens, |kind| {
        matches!(kind, TokenKind::DoubleColon)
    });

    assert_eq!(template_head_count, 2);
    assert_eq!(body_start_indices.len(), 2);
    assert!(body_start_indices[0] < double_colon_index);
    assert!(double_colon_index < body_start_indices[1]);
    assert_eq!(
        file_tokens
            .tokens
            .iter()
            .filter(|token| matches!(token.kind, TokenKind::DoubleColon))
            .count(),
        1
    );
}

#[test]
fn rejects_legacy_reset_style_directive_name() {
    let mut string_table = StringTable::new();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let source_path = InternedPath::from_single_str("test.moth", &mut string_table);
    let error = tokenize(
        "[$reset: body]",
        &source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        None,
    )
    .expect_err("legacy reset directive should be rejected");

    match &error.payload {
        DiagnosticPayload::InvalidStyleDirective { directive_name, .. } => {
            assert_eq!(string_table.resolve(*directive_name), "reset");
        }
        payload => panic!("expected invalid style directive payload, found {payload:?}"),
    }
    assert!(error.primary_location.start_pos.char_column > 0);
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

    let outer_head = find_token_index(&file_tokens.tokens, |kind| {
        matches!(kind, TokenKind::TemplateHead)
    });
    let children = find_token_index(
        &file_tokens.tokens,
        |kind| matches!(kind, TokenKind::StyleDirective(id) if string_table.resolve(*id) == "children"),
    );
    let open_paren = find_token_index(&file_tokens.tokens, |kind| {
        matches!(kind, TokenKind::OpenParenthesis)
    });
    let child_template = file_tokens
        .tokens
        .iter()
        .enumerate()
        .skip(open_paren + 1)
        .find_map(|(index, token)| matches!(token.kind, TokenKind::TemplateHead).then_some(index))
        .expect("expected child template opener");
    let close = file_tokens
        .tokens
        .iter()
        .enumerate()
        .skip(child_template + 1)
        .find_map(|(index, token)| matches!(token.kind, TokenKind::TemplateClose).then_some(index))
        .expect("expected the child template to close");
    let close_paren = file_tokens
        .tokens
        .iter()
        .enumerate()
        .skip(close + 1)
        .find_map(|(index, token)| {
            matches!(token.kind, TokenKind::CloseParenthesis).then_some(index)
        })
        .expect("expected ')' after the child template");
    let comma = file_tokens
        .tokens
        .iter()
        .enumerate()
        .skip(close_paren + 1)
        .find_map(|(index, token)| matches!(token.kind, TokenKind::Comma).then_some(index))
        .expect("expected a comma after the child template");
    let markdown = file_tokens
        .tokens
        .iter()
        .enumerate()
        .skip(comma + 1)
        .find_map(|(index, token)| {
            matches!(token.kind, TokenKind::StyleDirective(id) if string_table.resolve(id) == "md")
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
    let style_directives = StyleDirectiveRegistry::built_ins();
    let source_path = InternedPath::from_single_str("test.moth", &mut string_table);

    let result = tokenize(
        "[$[:prefix], $md:\nhello\n]",
        &source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        None,
    );
    assert!(
        result.is_err(),
        "legacy '$[' child-template syntax should fail"
    );
}

#[test]
fn tokenizes_reactive_marker_outside_template_heads() {
    let (file_tokens, _string_table) = tokenize_source("$String\n");

    assert!(
        matches!(file_tokens.tokens[1].kind, TokenKind::Reactive),
        "`$` in ordinary code should be the reactive marker"
    );
    assert!(
        matches!(file_tokens.tokens[2].kind, TokenKind::DatatypeString),
        "reactive marker should not turn the following identifier into a style directive"
    );
    assert!(
        !file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::StyleDirective(_))),
        "ordinary code should not produce style directive tokens"
    );
}

#[test]
fn tokenizes_template_reactive_subscription_marker() {
    let (file_tokens, _string_table) = tokenize_source("[:[$(count)]]");

    assert!(
        file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::Reactive)),
        "`$(` in a template head should produce the reactive marker"
    );
    assert!(
        !file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::StyleDirective(_))),
        "`$(` is subscription syntax, not a style directive"
    );
}

#[test]
fn unknown_style_directives_fail_under_strict_registry() {
    let mut string_table = StringTable::new();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let source_path = InternedPath::from_single_str("test.moth", &mut string_table);

    let result = tokenize(
        "[$unknown: value]",
        &source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        None,
    );
    let error = result.expect_err("unknown directive should fail during tokenization");

    match &error.payload {
        DiagnosticPayload::InvalidStyleDirective { directive_name, .. } => {
            assert_eq!(string_table.resolve(*directive_name), "unknown");
        }
        payload => panic!("expected invalid style directive payload, found {payload:?}"),
    }
    assert!(error.primary_location.start_pos.char_column > 0);
}

#[test]
fn tokenizes_slot_and_insert_directives_inside_template_heads() {
    let (file_tokens, string_table) =
        tokenize_source("[wrapper: [$slot][$slot(\"style\")][$insert(\"style\"): blue]]");

    let slot_directive_count = file_tokens
        .tokens
        .iter()
        .filter(|token| {
            matches!(token.kind, TokenKind::StyleDirective(id) if string_table.resolve(id) == "slot")
        })
        .count();
    let has_insert_directive = file_tokens.tokens.iter().any(|token| {
        matches!(token.kind, TokenKind::StyleDirective(id) if string_table.resolve(id) == "insert")
    });

    assert_eq!(slot_directive_count, 2);
    assert!(has_insert_directive);
    assert!(
        file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::StringSliceLiteral(_)))
    );
}

#[test]
fn rejects_numeric_slot_directive_prefixes() {
    let mut string_table = StringTable::new();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let source_path = InternedPath::from_single_str("test.moth", &mut string_table);

    let result = tokenize(
        "[wrapper: [$1: first]]",
        &source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        &mut string_table,
        None,
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

    let template_heads = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateHead))
        .count();
    let template_closes = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateClose))
        .count();

    assert_eq!(
        template_heads, 1,
        "code template bodies should not tokenize nested '[' as template opens"
    );
    assert_eq!(template_closes, 1);

    let body_literal = file_tokens
        .tokens
        .iter()
        .find_map(|token| match token.kind {
            TokenKind::StringSliceLiteral(id) => {
                let value = string_table.resolve(id);
                value
                    .contains("[string_slice, a_mutable_string]")
                    .then_some(value)
            }
            _ => None,
        })
        .expect("expected code template body text to include literal square brackets");

    assert!(body_literal.contains("concatenated"));
}

#[test]
fn css_template_body_keeps_selector_brackets_as_literal_text() {
    let (file_tokens, string_table) =
        tokenize_html_source("[$css:\n.button[data-kind=\"cta\"] { color: red; }\n]");

    let template_heads = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateHead))
        .count();
    let template_closes = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateClose))
        .count();

    assert_eq!(
        template_heads, 1,
        "css template bodies should not tokenize selector brackets as nested templates"
    );
    assert_eq!(template_closes, 1);

    let body_literal = file_tokens
        .tokens
        .iter()
        .find_map(|token| match token.kind {
            TokenKind::StringSliceLiteral(id) => {
                let value = string_table.resolve(id);
                value.contains("[data-kind=\"cta\"]").then_some(value)
            }
            _ => None,
        })
        .expect("expected css template body text to include selector brackets");

    assert!(body_literal.contains(".button"));
}

#[test]
fn html_template_body_tokenizes_attribute_brackets_using_normal_rules() {
    let (file_tokens, string_table) =
        tokenize_html_source("[$html:\n<div data-tags=\"[one,two]\">Hello</div>\n]");

    let template_heads = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateHead))
        .count();
    let template_closes = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateClose))
        .count();

    assert_eq!(
        template_heads, 2,
        "normal template-body parsing should tokenize '[one,two]' as a nested template in $html"
    );
    assert_eq!(template_closes, 2);

    assert!(
        file_tokens.tokens.iter().any(
            |token| matches!(token.kind, TokenKind::Symbol(id) if string_table.resolve(id) == "one")
        ),
        "expected nested template symbol 'one' from bracket content"
    );
    assert!(
        file_tokens.tokens.iter().any(
            |token| matches!(token.kind, TokenKind::Symbol(id) if string_table.resolve(id) == "two")
        ),
        "expected nested template symbol 'two' from bracket content"
    );

    let preserves_literal_attribute_brackets =
        file_tokens.tokens.iter().any(|token| match token.kind {
            TokenKind::StringSliceLiteral(id) => {
                let value = string_table.resolve(id);
                value.contains("data-tags=\"[one,two]\"")
            }
            _ => false,
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

    let template_heads = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateHead))
        .count();
    let template_closes = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateClose))
        .count();

    assert_eq!(
        template_heads, 3,
        "normal template-body parsing should still tokenize slot templates inside quoted attributes"
    );
    assert_eq!(template_closes, 3);

    let slot_directives = file_tokens
        .tokens
        .iter()
        .filter(|token| {
            matches!(token.kind, TokenKind::StyleDirective(id) if string_table.resolve(id) == "slot")
        })
        .count();
    assert_eq!(slot_directives, 2);
}

#[test]
fn html_template_body_tokenizes_symbol_wrappers_with_general_template_rules() {
    let (file_tokens, string_table) =
        tokenize_html_source("[$html:\n[title, center: LANGUAGE BASICS]\n]");

    let template_heads = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateHead))
        .count();
    let template_closes = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateClose))
        .count();

    assert_eq!(
        template_heads, 2,
        "normal template-body parsing should tokenize wrapper syntax in $html bodies"
    );
    assert_eq!(template_closes, 2);

    assert!(file_tokens.tokens.iter().any(
        |token| matches!(token.kind, TokenKind::Symbol(id) if string_table.resolve(id) == "title")
    ));
    assert!(file_tokens.tokens.iter().any(
        |token| matches!(token.kind, TokenKind::Symbol(id) if string_table.resolve(id) == "center")
    ));
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

    let template_heads = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateHead))
        .count();
    let template_closes = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateClose))
        .count();

    assert_eq!(template_heads, 1);
    assert_eq!(template_closes, 1);
    let body_literal = file_tokens
        .tokens
        .iter()
        .find_map(|token| match token.kind {
            TokenKind::StringSliceLiteral(id) => {
                let value = string_table.resolve(id);
                value.contains("[data-kind=\"cta\"]").then_some(value)
            }
            _ => None,
        })
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

        let template_heads = file_tokens
            .tokens
            .iter()
            .filter(|token| matches!(token.kind, TokenKind::TemplateHead))
            .count();
        let template_closes = file_tokens
            .tokens
            .iter()
            .filter(|token| matches!(token.kind, TokenKind::TemplateClose))
            .count();

        assert_eq!(template_heads, 1);
        assert_eq!(template_closes, 1);
        assert!(file_tokens.tokens.iter().any(|token| {
            matches!(token.kind, TokenKind::StyleDirective(id) if string_table.resolve(id) == directive)
        }));
        assert!(
            !file_tokens.tokens.iter().any(|token| {
                matches!(token.kind, TokenKind::StringSliceLiteral(id) if string_table.resolve(id).contains("discarded"))
            }),
            "expected ${directive} body text to be discarded during tokenization"
        );
    }
}

#[test]
fn doc_template_body_keeps_nested_templates_as_template_tokens() {
    let (file_tokens, string_table) = tokenize_source("[$doc:\n[: child]\n]");

    let template_heads = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateHead))
        .count();
    let template_closes = file_tokens
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, TokenKind::TemplateClose))
        .count();

    assert_eq!(
        template_heads, 2,
        "expected doc body nested template to tokenize as a child template"
    );
    assert_eq!(template_closes, 2);
    assert!(file_tokens.tokens.iter().any(|token| {
        matches!(token.kind, TokenKind::StyleDirective(id) if string_table.resolve(id) == "doc")
    }));
}

// ----------------------
//  Missing `@` import prefix
// ----------------------

fn assert_import_path_missing_at_prefix(
    diagnostic: &CompilerDiagnostic,
    string_table: &StringTable,
    expected_authored_path: &str,
    expected_start_line: i32,
    expected_start_column: i32,
) {
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::CommonSyntaxMistake)
    );
    assert_eq!(diagnostic.kind.code(), "MOTH-SYNTAX-0031");

    match &diagnostic.payload {
        DiagnosticPayload::CommonSyntaxMistake { reason } => match reason {
            CommonSyntaxMistakeReason::ImportPathMissingAtPrefix { authored_path } => {
                assert_eq!(
                    string_table.resolve(*authored_path),
                    expected_authored_path,
                    "authored import path spelling mismatch"
                );
            }
            other => panic!("expected ImportPathMissingAtPrefix, found {other:?}"),
        },
        payload => panic!("expected common syntax mistake payload, found {payload:?}"),
    }

    let start = diagnostic.primary_location.start_pos;
    let end = diagnostic.primary_location.end_pos;
    assert_eq!(start.line_number, expected_start_line);
    assert_eq!(start.char_column, expected_start_column);
    assert_eq!(end.line_number, expected_start_line);
    assert_eq!(
        end.char_column - start.char_column,
        expected_authored_path.chars().count() as i32,
        "missing-@ import path span should cover the complete authored path"
    );
}

#[test]
fn rejects_missing_at_prefix_paths_with_complete_spelling_and_span() {
    let cases = [
        ("import core\n", "core", 0, 7),
        ("import as-path\n", "as-path", 0, 7),
        ("import test/pkg-a\n", "test/pkg-a", 0, 7),
        ("import ./utils\n", "./utils", 0, 7),
        ("import ./drawing.js\n", "./drawing.js", 0, 7),
        (
            "import vendor/drawing.js as drawing\n",
            "vendor/drawing.js",
            0,
            7,
        ),
        ("import components{card}\n", "components", 0, 7),
        ("import\n./utils\n", "./utils", 1, 0),
    ];

    for (source, authored_path, start_line, start_column) in cases {
        let (diagnostic, string_table) = tokenize_source_error(source);
        assert_import_path_missing_at_prefix(
            &diagnostic,
            &string_table,
            authored_path,
            start_line,
            start_column,
        );
    }
}

#[test]
fn parent_relative_path_does_not_receive_missing_at_prefix_correction() {
    // `../` is not supported with `@`, so the tokenizer must not suggest `@../`.
    // The bare `import ../utils` tokenizes without a missing-`@` diagnostic; the
    // existing import-path rejection owns the parent-relative mistake.
    let (file_tokens, _string_table) = tokenize_source("import ../utils\n");
    assert!(
        !file_tokens
            .tokens
            .iter()
            .any(|token| matches!(&token.kind, TokenKind::Path(_))),
        "parent-relative bare import should not tokenize as a path"
    );
}

#[test]
fn valid_at_prefixed_imports_and_operators_remain_unaffected() {
    for source in [
        "import @core/math\n",
        "import @./utils\n",
        "value = a / b\n",
        "value = a // b\n",
        "-- comment\n",
    ] {
        let (file_tokens, _string_table) = tokenize_source(source);
        assert!(
            file_tokens
                .tokens
                .iter()
                .any(|token| matches!(token.kind, TokenKind::Eof)),
            "expected valid source to tokenize: {source}"
        );
    }

    let (at_core, string_table) = tokenize_source("import @core/math\n");
    assert!(
        at_core.tokens.iter().any(|token| matches!(
            &token.kind,
            TokenKind::Path(items) if items.iter().any(|item| item
                .path
                .to_portable_string(&string_table) == "core/math")
        )),
        "valid @-prefixed import should produce a path token"
    );
}

#[test]
fn keyword_led_import_does_not_receive_missing_at_prefix_correction() {
    // `as` directly after `import` is an alias with no path. Keep it on the import-clause path.
    let (file_tokens, _string_table) = tokenize_source("import as drawing\n");
    assert!(
        file_tokens
            .tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::As)),
        "keyword-led `import as ...` should tokenize `as` as the As keyword, not consume it as a missing-@ path"
    );
}

#[test]
fn missing_at_prefix_renders_exact_message_and_suggestion() {
    let (diagnostic, string_table) = tokenize_source_error("import vendor/drawing.js as drawing\n");
    let context = DiagnosticRenderContext::new(&string_table);
    let guidance = format_payload_guidance(&diagnostic.payload, context);

    assert_eq!(
        guidance[0],
        "Import paths must begin with `@`. Write `import @vendor/drawing.js`."
    );
    assert_eq!(guidance[1], "Suggestion: Insert `@` before the import path");
}
