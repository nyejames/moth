//! Source-directive recognition and attachment tests.
//!
//! WHAT: covers the `$config` source modifier and the typed diagnostics for wrong-context,
//!       duplicate, dangling and wrong-target uses.
//! WHY: directive attachment spans header parsing and AST body dispatch; these cases keep the
//!       authored spans and the recorded attachment fact on one focused owner.

use super::*;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates::{
    InvalidTemplateDirectiveCode, InvalidTemplateDirectiveView,
};
use crate::compiler_frontend::declaration_syntax::declaration_shell::DeclarationSyntax;
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, parse_single_file_ast_diagnostic,
};

fn assert_directive_reason<'a>(
    errors: &'a HeaderTestDiagnostics,
    expected: InvalidTemplateDirectiveCode,
) -> DiagnosticView<'a> {
    let diagnostic = errors
        .diagnostics
        .first()
        .expect("the directive failure should compact to one record");
    assert_eq!(diagnostic.descriptor().code, "MOTH-SYNTAX-0021");
    let facts = diagnostic
        .family_view::<InvalidTemplateDirectiveView>()
        .expect("the failure should retain typed directive facts");
    assert_eq!(facts.reason, expected);
    diagnostic
}

fn earlier_directive_span(diagnostic: DiagnosticView<'_>) -> Option<SourceSpan> {
    diagnostic.durable_labels().find_map(|label| {
        matches!(label.message, DurableLabelMessage::EarlierDirective).then_some(())?;
        match label.site {
            SiteRef::Absent => None,
            SiteRef::Local(span) | SiteRef::Foreign { span, .. } => Some(span),
        }
    })
}

fn config_header(headers: &BoundModuleHeaders) -> &Header {
    headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Constant { .. }))
        .expect("the marked compile-time binding should produce a constant header")
}

fn config_declaration(headers: &BoundModuleHeaders) -> &DeclarationSyntax {
    let HeaderKind::Constant { declaration } = &config_header(headers).kind else {
        panic!("expected a compile-time constant header");
    };
    declaration
}

fn config_header_from(headers: &[Header]) -> &Header {
    headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Constant { .. }))
        .expect("the marked compile-time binding should produce a constant header")
}

#[test]
fn config_modifier_attaches_to_a_typed_compile_time_binding() {
    // The modifier attaches to the declaration shell at the header stage; source contract
    // normalization reads that shell's own type annotation and initializer.
    let source = "$config\napi_url #String = \"https://example.test\"\n";
    let mut string_table = StringTable::new();
    let (output, _span_builder) = prepare_single_file(
        source,
        std::path::Path::new("src/@entry.moth"),
        std::path::Path::new("src/@entry.moth"),
        &mut string_table,
    );

    let header = config_header_from(&output.headers);
    let HeaderKind::Constant { declaration } = &header.kind else {
        panic!("expected a compile-time constant header");
    };
    assert!(declaration.binding_mode.is_compile_time());
    assert!(
        !matches!(declaration.type_annotation, ParsedTypeRef::Inferred),
        "the attachment requires an explicitly typed binding"
    );
    assert!(
        declaration.config_contract.is_some(),
        "the attached modifier must be recorded on the declaration shell"
    );
}

#[test]
fn config_modifier_attaches_across_comment_only_lines() {
    let source = "$config\n-- endpoint\n-- deployed per environment\napi_url #String = \"x\"\n";
    let headers = expect_header_success(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "comment-only lines must not break attachment",
    );

    assert!(
        !matches!(
            config_declaration(&headers).type_annotation,
            ParsedTypeRef::Inferred
        ),
        "the declaration keeps the authored explicit annotation"
    );
}

#[test]
fn blank_line_breaks_config_attachment() {
    let source = "$config\n\napi_url #String = \"x\"\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "a blank line must break attachment",
    );

    let diagnostic =
        assert_directive_reason(&errors, InvalidTemplateDirectiveCode::DanglingModifier);
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(source, "$config", 0)),
        "the dangling modifier is reported at its own authored span"
    );
}

#[test]
fn blank_line_between_comment_lines_breaks_config_attachment() {
    let source = "$config\n-- first\n\n-- second\napi_url #String = \"x\"\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "a blank line inside a comment run must break attachment",
    );

    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::DanglingModifier);
}

#[test]
fn config_modifier_at_eof_is_dangling() {
    let source = "$config\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "a modifier at end of file has no target",
    );

    let diagnostic =
        assert_directive_reason(&errors, InvalidTemplateDirectiveCode::DanglingModifier);
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(source, "$config", 0))
    );
}

#[test]
fn config_modifier_before_a_non_declaration_is_dangling() {
    let source = "$config\nvalue = 1\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "a runtime statement is not an eligible directive target",
    );

    let diagnostic =
        assert_directive_reason(&errors, InvalidTemplateDirectiveCode::DanglingModifier);
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(source, "$config", 0))
    );
}

#[test]
fn config_followed_directive_matrix_orders_context_before_duplicate() {
    // Name resolution precedes the pending-modifier comparison. A known template-only name is a
    // wrong-context use, while the same source-modifier name is the duplicate case.
    let cases = [
        (
            "$config\n$html\nvalue #= 1\n",
            InvalidTemplateDirectiveCode::WrongContext,
            "$html",
            0,
        ),
        (
            "$config\n$config\nvalue #= 1\n",
            InvalidTemplateDirectiveCode::DuplicateModifier,
            "$config",
            1,
        ),
    ];

    for (source, expected, directive, occurrence) in cases {
        let errors = expect_header_error(
            parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
            "the second directive must be classified before attachment",
        );
        let diagnostic = assert_directive_reason(&errors, expected);
        assert_eq!(
            diagnostic_primary_span(diagnostic),
            Some(source_span_for(source, directive, occurrence)),
            "the diagnostic is reported at the incoming directive's authored span"
        );
    }
}

#[test]
fn duplicate_config_modifiers_cite_both_authored_spans() {
    let source = "$config\n$config\napi_url #String = \"x\"\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "a repeated source modifier must be rejected",
    );

    let diagnostic =
        assert_directive_reason(&errors, InvalidTemplateDirectiveCode::DuplicateModifier);
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(source, "$config", 1))
    );
    assert_eq!(
        earlier_directive_span(diagnostic),
        Some(source_span_for(source, "$config", 0)),
        "the diagnostic labels the first authored modifier"
    );
}

#[test]
fn config_modifier_operand_is_a_malformed_operand() {
    let source = "$config(\"x\")\napi_url #String = \"x\"\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "a source modifier accepts no operand",
    );

    let diagnostic =
        assert_directive_reason(&errors, InvalidTemplateDirectiveCode::UnexpectedArguments);
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(source, "(", 0)),
        "the malformed operand is reported at the operand span"
    );
}

#[test]
fn config_attachment_observes_crlf_boundaries() {
    let attached = "$config\r\napi_url #String = \"x\"\r\n";
    let headers = expect_header_success(
        parse_single_file_headers_with_entry(attached, "src/@entry.moth", "src/@entry.moth"),
        "CRLF after a source modifier should attach like LF",
    );
    assert!(
        !matches!(
            config_declaration(&headers).type_annotation,
            ParsedTypeRef::Inferred
        ),
        "the declaration keeps the authored explicit annotation"
    );

    let broken = "$config\r\n\r\napi_url #String = \"x\"\r\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(broken, "src/@entry.moth", "src/@entry.moth"),
        "a CRLF blank line should break attachment like LF",
    );
    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::DanglingModifier);
}

#[test]
fn template_head_directive_in_source_is_wrong_context() {
    let source = "$html\nvalue #= 1\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "a template-head directive is not a source modifier",
    );

    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::WrongContext);
}

#[test]
fn config_modifier_rejects_an_inferred_compile_time_binding() {
    let source = "$config\nvalue #= 1\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "`$config` requires an explicit type annotation",
    );

    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::WrongTarget);
}

#[test]
fn config_modifier_rejects_a_non_binding_declaration() {
    let source = "$config\nload |input Bool| -> Bool:\n    return input\n;\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "`$config` cannot modify a function declaration",
    );

    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::WrongTarget);
}

#[test]
fn config_modifier_inside_a_function_body_is_wrong_target() {
    let source = "load |input Bool| -> Bool:\n    $config\n    return input\n;\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0021",
        Some("invalid_template_directive.wrong_target"),
    );
    let facts = diagnostic
        .family_view::<InvalidTemplateDirectiveView>()
        .expect("the nested directive keeps its typed facts");
    assert_eq!(facts.reason, InvalidTemplateDirectiveCode::WrongTarget);
}

#[test]
fn config_modifier_inside_a_block_value_is_wrong_target() {
    let source = "f #= {\n    $config\n    y #= 1\n}\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0021",
        Some("invalid_template_directive.wrong_target"),
    );
    let facts = diagnostic
        .family_view::<InvalidTemplateDirectiveView>()
        .expect("the block-value directive keeps its typed facts");
    assert_eq!(facts.reason, InvalidTemplateDirectiveCode::WrongTarget);
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(source, "$config", 0)),
        "the block-value directive is reported at its own span"
    );
}
