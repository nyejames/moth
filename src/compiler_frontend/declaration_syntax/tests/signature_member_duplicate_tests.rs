//! Focused duplicate-member tests for the shared `| ... |` signature-member parser.
//!
//! WHAT: verifies that duplicate function parameters, struct fields, choice payload fields
//! and trait-requirement parameters are all rejected by the shared signature-member parser
//! with `DuplicateDeclaration` (`MOTH-RULE-0002`), before any HIR or infrastructure invariant
//! can fire.
//! WHY: the shared parser is the single owner of member-name uniqueness. Function-, struct-
//! and choice-specific duplicate validators would duplicate that ownership.

use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terminal};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, DiagnosticLabelMessage, DiagnosticLabelStyle, DiagnosticPayload,
    RuleDiagnosticKind,
};
use crate::compiler_frontend::declaration_syntax::record_body::parse_record_body;
use crate::compiler_frontend::declaration_syntax::signature_members::{
    SignatureMemberContext, parse_function_signature_syntax,
    parse_trait_requirement_signature_syntax,
};
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceSpan};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind, TokenizerEntryMode};

/// Tokenize `source` and position the stream on the first opening `|` so a wrapper parser
/// (`parse_record_body`, `parse_function_signature_syntax`, ...) can advance past it.
///
/// The caller owns `span_builder` and keeps it alive wherever the positioned stream's
/// token spans are still resolved.
fn stream_positioned_at_open_bracket(
    source: &str,
    string_table: &mut StringTable,
    span_builder: &mut ExtendedSpanBuilder,
) -> FileTokens {
    let mut path_fork = PathInternerFork::empty();
    let source_path = path_fork.try_intern_portable_path("test.moth", string_table).expect("test path fits");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut token_stream = tokenize(source, source_path, TokenizerEntryMode::SourceFile, &style_directives, string_table, &mut path_fork, crate::compiler_frontend::source::SourceId::COMPILATION_ROOT, span_builder)
    .expect("tokenization should succeed");

    let open_index = token_stream
        .tokens
        .iter()
        .position(|token| token.kind == TokenKind::TypeParameterBracket)
        .expect("test source must contain an opening `|`");
    token_stream.index = open_index;

    token_stream
}

fn duplicate_member_spans(
    token_stream: &FileTokens,
    string_table: &mut StringTable,
    expected_name: &str,
) -> (SourceSpan, SourceSpan) {
    let name = string_table.intern(expected_name);
    let spans = token_stream
        .tokens
        .iter()
        .filter_map(|token| match &token.kind {
            TokenKind::Symbol(candidate) if *candidate == name => {
                Some(SourceSpan::new(token_stream.file_id, token.span))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        spans.len(),
        2,
        "test source should contain exactly two `{expected_name}` members",
    );
    (spans[0], spans[1])
}

fn owner_path(string_table: &mut StringTable) -> PathId {
    let mut path_fork = PathInternerFork::empty();
    path_fork.try_intern_portable_path("test.moth", string_table).expect("test path fits")
}

/// Asserts `error` is the shared-parser duplicate-member diagnostic for `expected_name`:
/// stable `MOTH-RULE-0002` kind, current member primary, first member secondary, and the
/// scope-neutral rendered message.
fn assert_shared_duplicate_diagnostic(
    failure: &HeaderParseFailure,
    string_table: &StringTable,
    expected_name: &str,
    expected_first_span: SourceSpan,
    expected_duplicate_span: SourceSpan,
) {
    let error = match failure {
        HeaderParseFailure::Diagnostic(diagnostic) => diagnostic,
        HeaderParseFailure::Infrastructure(error) => {
            panic!("duplicate-member parser infrastructure failure: {error:?}")
        }
    };
    assert_eq!(
        error.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateDeclaration),
        "duplicate members must use MOTH-RULE-0002 DuplicateDeclaration",
    );

    let DiagnosticPayload::DuplicateDeclaration { name } = &error.payload else {
        panic!(
            "expected DuplicateDeclaration payload, got {:?}",
            error.payload
        );
    };
    assert_eq!(string_table.resolve(*name), expected_name);

    assert_eq!(
        error.primary_span,
        Some(expected_duplicate_span),
        "duplicate member must be the primary exact source span",
    );
    assert_eq!(
        error.labels.len(),
        1,
        "only the secondary (first) member label remains",
    );

    let secondary = &error.labels[0];
    assert_eq!(
        secondary.style,
        DiagnosticLabelStyle::Secondary,
        "remaining label must be the first member",
    );
    assert_eq!(
        secondary.span,
        Some(expected_first_span),
        "secondary label must retain the first member's exact source span",
    );
    assert_ne!(
        error.primary_span, secondary.span,
        "primary and secondary member spans must differ",
    );
    assert_eq!(
        secondary.message,
        Some(DiagnosticLabelMessage::PreviousDeclaration),
        "secondary label must mark the previous declaration",
    );

    let render_context = DiagnosticRenderContext::new(string_table);
    let guidance = terminal::format_payload_guidance(&error.payload, render_context);
    let expected_fragment = format!(
        "Cannot declare '{expected_name}' because that name is already visible in this scope"
    );
    assert!(
        guidance
            .iter()
            .any(|line| line.contains(&expected_fragment)),
        "expected scope-neutral duplicate message, got {guidance:?}",
    );
}

#[test]
fn duplicate_function_parameters_rejected_by_shared_parser() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = stream_positioned_at_open_bracket(
        "fn | value Int, value Int | -> Int :",
        &mut string_table,
        &mut span_builder,
    );
    let function_path = owner_path(&mut string_table);
    let mut warnings = Vec::new();
    let (expected_first_span, expected_duplicate_span) =
        duplicate_member_spans(&token_stream, &mut string_table, "value");
    let error = parse_function_signature_syntax(
        &mut token_stream,
        &mut warnings,
        &mut string_table,
        function_path,
        &mut path_fork,
        &mut span_builder,
    )
    .expect_err("duplicate function parameters must be rejected by the shared parser");

    assert_shared_duplicate_diagnostic(
        &error,
        &string_table,
        "value",
        expected_first_span,
        expected_duplicate_span,
    );
}

#[test]
fn duplicate_struct_fields_rejected_by_shared_parser() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = stream_positioned_at_open_bracket(
        "| value Int, value Int |",
        &mut string_table,
        &mut span_builder,
    );
    let struct_path = owner_path(&mut string_table);
    let mut warnings = Vec::new();
    let (expected_first_span, expected_duplicate_span) =
        duplicate_member_spans(&token_stream, &mut string_table, "value");
    let error = parse_record_body(
        &mut token_stream,
        &mut string_table,
        &mut warnings,
        SignatureMemberContext::StructField,
        struct_path,
        &mut path_fork,
        &mut span_builder,
    )
    .expect_err("duplicate struct fields must be rejected by the shared parser");

    assert_shared_duplicate_diagnostic(
        &error,
        &string_table,
        "value",
        expected_first_span,
        expected_duplicate_span,
    );
}

#[test]
fn duplicate_choice_payload_fields_rejected_by_shared_parser() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = stream_positioned_at_open_bracket(
        "| message String, message Int |",
        &mut string_table,
        &mut span_builder,
    );
    let choice_path = owner_path(&mut string_table);
    let mut warnings = Vec::new();
    let (expected_first_span, expected_duplicate_span) =
        duplicate_member_spans(&token_stream, &mut string_table, "message");
    let error = parse_record_body(
        &mut token_stream,
        &mut string_table,
        &mut warnings,
        SignatureMemberContext::ChoicePayloadField,
        choice_path,
        &mut path_fork,
        &mut span_builder,
    )
    .expect_err("duplicate choice payload fields must be rejected by the shared parser");

    assert_shared_duplicate_diagnostic(
        &error,
        &string_table,
        "message",
        expected_first_span,
        expected_duplicate_span,
    );
}

#[test]
fn duplicate_trait_requirement_parameters_rejected_by_shared_parser() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = stream_positioned_at_open_bracket(
        "| This, value Int, value Int | -> Int ;",
        &mut string_table,
        &mut span_builder,
    );
    let method_path = owner_path(&mut string_table);
    let mut warnings = Vec::new();
    let (expected_first_span, expected_duplicate_span) =
        duplicate_member_spans(&token_stream, &mut string_table, "value");
    let error = parse_trait_requirement_signature_syntax(
        &mut token_stream,
        &mut warnings,
        &mut string_table,
        method_path,
        &mut path_fork,
        &mut span_builder,
    )
    .expect_err("duplicate trait-requirement parameters must be rejected by the shared parser");

    assert_shared_duplicate_diagnostic(
        &error,
        &string_table,
        "value",
        expected_first_span,
        expected_duplicate_span,
    );
}

#[test]
fn distinct_members_parse_successfully_through_shared_parser() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream = stream_positioned_at_open_bracket(
        "| first Int, second String |",
        &mut string_table,
        &mut span_builder,
    );
    let struct_path = owner_path(&mut string_table);
    let mut warnings = Vec::new();

    let fields = parse_record_body(
        &mut token_stream,
        &mut string_table,
        &mut warnings,
        SignatureMemberContext::StructField,
        struct_path,
        &mut path_fork,
        &mut span_builder,
    )
    .expect("distinct member names must parse successfully");

    assert_eq!(fields.len(), 2, "both distinct members must be retained");
    assert_eq!(
        string_table.resolve(
            path_fork
                .component(fields[0].id)
                .expect("first field has a name"),
        ),
        "first",
    );
    assert_eq!(
        string_table.resolve(
            path_fork
                .component(fields[1].id)
                .expect("second field has a name"),
        ),
        "second",
    );
}
