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
    CompilerDiagnostic, DiagnosticKind, DiagnosticLabelMessage, DiagnosticLabelStyle,
    DiagnosticPayload, RuleDiagnosticKind,
};
use crate::compiler_frontend::declaration_syntax::record_body::parse_record_body;
use crate::compiler_frontend::declaration_syntax::signature_members::{
    SignatureMemberContext, parse_function_signature_syntax,
    parse_trait_requirement_signature_syntax,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind, TokenizerEntryMode};

/// Tokenize `source` and position the stream on the first opening `|` so a wrapper parser
/// (`parse_record_body`, `parse_function_signature_syntax`, ...) can advance past it.
fn stream_positioned_at_open_bracket(source: &str, string_table: &mut StringTable) -> FileTokens {
    let source_path = InternedPath::from_single_str("test.moth", string_table);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut token_stream = tokenize(
        source,
        &source_path,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        string_table,
        None,
    )
    .expect("tokenization should succeed");

    let open_index = token_stream
        .tokens
        .iter()
        .position(|token| token.kind == TokenKind::TypeParameterBracket)
        .expect("test source must contain an opening `|`");
    token_stream.index = open_index;

    token_stream
}

fn owner_path(string_table: &mut StringTable) -> InternedPath {
    InternedPath::from_single_str("test.moth", string_table)
}

/// Asserts `error` is the shared-parser duplicate-member diagnostic for `expected_name`:
/// stable `MOTH-RULE-0002` kind, current member primary, first member secondary, and the
/// scope-neutral rendered message.
fn assert_shared_duplicate_diagnostic(
    error: &CompilerDiagnostic,
    string_table: &StringTable,
    expected_name: &str,
) {
    assert_eq!(
        error.kind,
        DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateDeclaration),
        "duplicate members must use MOTH-RULE-0002 DuplicateDeclaration",
    );

    let DiagnosticPayload::DuplicateDeclaration {
        name,
        first_location,
    } = &error.payload
    else {
        panic!(
            "expected DuplicateDeclaration payload, got {:?}",
            error.payload
        );
    };
    assert_eq!(string_table.resolve(*name), expected_name);

    let first_location = first_location
        .as_ref()
        .expect("shared-parser duplicate must carry the first member location");

    assert_eq!(
        error.labels.len(),
        2,
        "primary (current) and secondary (first) member labels must both be present",
    );

    let primary = &error.labels[0];
    let secondary = &error.labels[1];
    assert_eq!(
        primary.style,
        DiagnosticLabelStyle::Primary,
        "first label must be the current (duplicate) member",
    );
    assert_eq!(
        primary.location, error.primary_location,
        "primary label must point at the duplicate member",
    );
    assert_eq!(
        secondary.style,
        DiagnosticLabelStyle::Secondary,
        "second label must be the first member",
    );
    assert_eq!(
        secondary.location, *first_location,
        "secondary label must point at the first member",
    );
    assert!(
        primary.location != secondary.location,
        "primary and secondary member locations must differ",
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
    let mut token_stream = stream_positioned_at_open_bracket(
        "fn | value Int, value Int | -> Int :",
        &mut string_table,
    );
    let function_path = owner_path(&mut string_table);
    let mut warnings = Vec::new();

    let error = parse_function_signature_syntax(
        &mut token_stream,
        &mut warnings,
        &mut string_table,
        &function_path,
    )
    .expect_err("duplicate function parameters must be rejected by the shared parser");

    assert_shared_duplicate_diagnostic(&error, &string_table, "value");
}

#[test]
fn duplicate_struct_fields_rejected_by_shared_parser() {
    let mut string_table = StringTable::new();
    let mut token_stream =
        stream_positioned_at_open_bracket("| value Int, value Int |", &mut string_table);
    let struct_path = owner_path(&mut string_table);
    let mut warnings = Vec::new();

    let error = parse_record_body(
        &mut token_stream,
        &mut string_table,
        &mut warnings,
        SignatureMemberContext::StructField,
        &struct_path,
    )
    .expect_err("duplicate struct fields must be rejected by the shared parser");

    assert_shared_duplicate_diagnostic(&error, &string_table, "value");
}

#[test]
fn duplicate_choice_payload_fields_rejected_by_shared_parser() {
    let mut string_table = StringTable::new();
    let mut token_stream =
        stream_positioned_at_open_bracket("| message String, message Int |", &mut string_table);
    let choice_path = owner_path(&mut string_table);
    let mut warnings = Vec::new();

    let error = parse_record_body(
        &mut token_stream,
        &mut string_table,
        &mut warnings,
        SignatureMemberContext::ChoicePayloadField,
        &choice_path,
    )
    .expect_err("duplicate choice payload fields must be rejected by the shared parser");

    assert_shared_duplicate_diagnostic(&error, &string_table, "message");
}

#[test]
fn duplicate_trait_requirement_parameters_rejected_by_shared_parser() {
    let mut string_table = StringTable::new();
    let mut token_stream = stream_positioned_at_open_bracket(
        "| This, value Int, value Int | -> Int ;",
        &mut string_table,
    );
    let method_path = owner_path(&mut string_table);
    let mut warnings = Vec::new();

    let error = parse_trait_requirement_signature_syntax(
        &mut token_stream,
        &mut warnings,
        &mut string_table,
        &method_path,
    )
    .expect_err("duplicate trait-requirement parameters must be rejected by the shared parser");

    assert_shared_duplicate_diagnostic(&error, &string_table, "value");
}

#[test]
fn distinct_members_parse_successfully_through_shared_parser() {
    let mut string_table = StringTable::new();
    let mut token_stream =
        stream_positioned_at_open_bracket("| first Int, second String |", &mut string_table);
    let struct_path = owner_path(&mut string_table);
    let mut warnings = Vec::new();

    let fields = parse_record_body(
        &mut token_stream,
        &mut string_table,
        &mut warnings,
        SignatureMemberContext::StructField,
        &struct_path,
    )
    .expect("distinct member names must parse successfully");

    assert_eq!(fields.len(), 2, "both distinct members must be retained");
    assert_eq!(
        string_table.resolve(fields[0].id.name().expect("first field has a name")),
        "first",
    );
    assert_eq!(
        string_table.resolve(fields[1].id.name().expect("second field has a name")),
        "second",
    );
}
