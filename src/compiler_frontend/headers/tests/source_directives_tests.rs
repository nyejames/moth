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
use crate::compiler_frontend::style_directives::SourceDirectiveKind;
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
fn config_modifier_before_a_non_declaration_is_wrong_target() {
    let source = "$config\nvalue = 1\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "a runtime statement is a present ineligible directive target",
    );

    let diagnostic = assert_directive_reason(&errors, InvalidTemplateDirectiveCode::WrongTarget);
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

#[test]
fn export_modifier_marks_only_its_next_declaration_public() {
    let source =
        "$export\nButton = | label String |\nprivate_helper || -> String:\n    return \"x\"\n;\n";
    let headers = expect_header_success(
        parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@entry.moth"),
        "a facade should accept per-declaration `$export`",
    );

    let public_headers: Vec<_> = headers
        .headers
        .iter()
        .filter(|header| header.export_mode == HeaderExportMode::Public)
        .collect();
    assert_eq!(public_headers.len(), 1, "only the marked struct is public");
    assert!(matches!(public_headers[0].kind, HeaderKind::Struct { .. }));

    let private_helper = headers
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Function { .. }))
        .expect("the unmarked helper should still parse");
    assert_eq!(private_helper.export_mode, HeaderExportMode::Private);
}

#[test]
fn export_modifier_marks_a_direct_selection_dependency_public() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let output = prepare_active_root_with_role(
        "$export\n@button Button, render as render_button\n",
        &PathBuf::from("src/@mod.moth"),
        ModuleRootRole::Normal,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    )
    .expect("a facade should accept an exported direct-selection clause");

    assert_eq!(output.file_dependency_clauses.len(), 1);
    assert_eq!(
        output.file_dependency_clauses[0].export_mode,
        HeaderExportMode::Public
    );
    let selections = output.file_dependency_clauses[0]
        .selections(&output.dependency_selections)
        .expect("the exported clause retains its selections");
    assert_eq!(selections.len(), 2);
    assert_eq!(
        string_table.resolve(selections[0].source_name),
        "Button",
        "an unaliased selection keeps its source name"
    );
    assert_eq!(
        string_table.resolve(selections[1].source_name),
        "render",
        "an aliased selection keeps its authored source name"
    );
    assert_eq!(
        string_table.resolve(selections[1].local_name()),
        "render_button"
    );
}

#[test]
fn export_modifier_rejects_a_namespace_dependency() {
    let source = "$export\n@button\n";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let failure = match prepare_active_root_with_role(
        source,
        &PathBuf::from("src/@mod.moth"),
        ModuleRootRole::Normal,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    ) {
        Ok(_) => panic!("a public dependency needs explicit direct selections"),
        Err(failure) => failure,
    };

    let FileFrontendPrepareFailure::Diagnosed(error) = failure else {
        panic!("the extra fixture should diagnose the namespace export");
    };
    let diagnostics = diagnostic_view_for_test(error.diagnostic);
    assert!(diagnostics.views().any(|diagnostic| {
        diagnostic_has_family::<generics_traits_api::InvalidExportTargetView>(&diagnostic)
    }));
}

#[test]
fn export_modifier_is_rejected_outside_api_facade_roots() {
    let source = "$export\nvalue #= 1\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "entry roots have no public export surface",
    );

    let diagnostic = assert_directive_reason(
        &errors,
        InvalidTemplateDirectiveCode::DirectiveNotAllowedHere,
    );
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(source, "$export", 0))
    );
}

#[test]
fn export_modifier_does_not_create_a_config_contract() {
    let source = "$export\napi_url #String = \"https://example.test\"\n";
    let mut string_table = StringTable::new();
    let (output, _span_builder) = prepare_api_root_with_fork(
        source,
        std::path::Path::new("src/@mod.moth"),
        &mut string_table,
        &mut PathInternerFork::empty(),
    );

    let header = output
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Constant { .. }))
        .expect("the exported typed constant should parse");
    let HeaderKind::Constant { declaration } = &header.kind else {
        panic!("expected a compile-time constant header");
    };
    assert!(
        declaration.config_contract.is_none(),
        "`$export` must not turn the constant into a build-input contract"
    );
    assert!(
        declaration.initializer_range.is_some(),
        "the exported typed constant keeps its required initializer"
    );
}

#[test]
fn export_modifier_typed_constant_still_requires_an_initializer() {
    let source = "$export\napi_url #String\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@entry.moth"),
        "`$export` must not admit a missing initializer",
    );

    assert!(errors.diagnostics.views().any(|diagnostic| {
        diagnostic
            .family_view::<names_declarations::MissingDeclarationInitializerView>()
            .is_some()
    }));
}

#[test]
fn config_modifier_before_a_dependency_is_wrong_target() {
    let source = "$config\n@button Button\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "`$config` cannot modify a dependency clause",
    );

    let diagnostic = assert_directive_reason(&errors, InvalidTemplateDirectiveCode::WrongTarget);
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(source, "$config", 0))
    );
}

#[test]
fn export_modifier_before_executable_syntax_is_wrong_target() {
    let source = "$export\nvalue = 1\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@entry.moth"),
        "`$export` must not skip an executable statement",
    );

    let diagnostic = assert_directive_reason(&errors, InvalidTemplateDirectiveCode::WrongTarget);
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(source, "$export", 0))
    );
}

#[test]
fn export_modifier_before_a_runtime_template_is_wrong_target() {
    let source = "$export\n~[: hello ]\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@entry.moth"),
        "`$export` must not leak past a runtime template",
    );

    let diagnostic = assert_directive_reason(&errors, InvalidTemplateDirectiveCode::WrongTarget);
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(source, "$export", 0))
    );
}

#[test]
fn export_modifier_attaches_across_comments_and_crlf() {
    let source = "$export\n-- public name\nlabel #= \"x\"\n";
    let headers = expect_header_success(
        parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@entry.moth"),
        "comment-only lines must not break `$export` attachment",
    );
    assert!(headers.headers.iter().any(|header| {
        matches!(header.kind, HeaderKind::Constant { .. })
            && header.export_mode == HeaderExportMode::Public
    }));

    let crlf = "$export\r\nlabel #= \"x\"\r\n";
    let headers = expect_header_success(
        parse_single_file_headers_with_entry(crlf, "src/@mod.moth", "src/@entry.moth"),
        "CRLF after `$export` should attach like LF",
    );
    assert!(headers.headers.iter().any(|header| {
        matches!(header.kind, HeaderKind::Constant { .. })
            && header.export_mode == HeaderExportMode::Public
    }));
}

#[test]
fn blank_line_breaks_export_attachment() {
    let source = "$export\n\nlabel #= \"x\"\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@entry.moth"),
        "a blank physical line has no eligible target",
    );

    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::DanglingModifier);
}

#[test]
fn export_modifier_rejects_operands() {
    let cases = [
        ("$export(\"x\")\nvalue #= 1\n", "(", 0),
        ("$export:\nvalue #= 1\n", ":", 0),
        ("$export value #= 1\n", "value", 0),
    ];

    for (source, needle, occurrence) in cases {
        let errors = expect_header_error(
            parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@entry.moth"),
            "`$export` accepts no same-line operand",
        );
        let diagnostic =
            assert_directive_reason(&errors, InvalidTemplateDirectiveCode::UnexpectedArguments);
        assert_eq!(
            diagnostic_primary_span(diagnostic),
            Some(source_span_for(source, needle, occurrence)),
            "the operand or block-colon spelling is reported at its own span"
        );
    }
}

#[test]
fn duplicate_export_modifiers_cite_both_authored_spans() {
    let source = "$export\n$export\nvalue #= 1\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@entry.moth"),
        "a repeated source modifier must be rejected",
    );

    let diagnostic =
        assert_directive_reason(&errors, InvalidTemplateDirectiveCode::DuplicateModifier);
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(source, "$export", 1))
    );
    assert_eq!(
        earlier_directive_span(diagnostic),
        Some(source_span_for(source, "$export", 0)),
        "the diagnostic labels the first authored modifier"
    );
}

#[test]
fn config_and_export_cannot_share_a_constant_target() {
    for source in [
        "$config\n$export\napi_url #String = \"https://example.test\"\n",
        "$export\n$config\napi_url #String = \"https://example.test\"\n",
    ] {
        let errors = expect_header_error(
            parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@entry.moth"),
            "an exported constant cannot also carry a config contract",
        );
        let diagnostic =
            assert_directive_reason(&errors, InvalidTemplateDirectiveCode::WrongTarget);
        assert_eq!(
            diagnostic_primary_span(diagnostic),
            Some(source_span_for(source, "$export", 0)),
            "$export is the directive whose own target requirement fails"
        );
    }
}

#[test]
fn export_modifier_at_eof_is_dangling() {
    for source in ["$export", "$export\n"] {
        let errors = expect_header_error(
            parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@entry.moth"),
            "a modifier at end of file has no target",
        );
        let diagnostic =
            assert_directive_reason(&errors, InvalidTemplateDirectiveCode::DanglingModifier);
        assert_eq!(
            diagnostic_primary_span(diagnostic),
            Some(source_span_for(source, "$export", 0))
        );
    }
}

#[test]
fn export_modifier_marks_every_eligible_declaration_kind_public() {
    let source = "$export\nrender || -> String:\n    return \"x\"\n;\n\n\
$export\nButton = | label String |\n\n\
$export\nStatus :: Ready, Failed;\n\n\
$export\nUserId as I32\n\n\
$export\nDISPLAY must:\n    show |This| -> String\n;\n\n\
$export\ninferred #= 1\n\n\
$export\ntyped #I32 = 2\n";
    let headers = expect_header_success(
        parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@entry.moth"),
        "every eligible declaration kind should accept `$export`",
    );

    let public_headers: Vec<_> = headers
        .headers
        .iter()
        .filter(|header| header.export_mode == HeaderExportMode::Public)
        .collect();
    assert_eq!(
        public_headers.len(),
        7,
        "function, struct, choice, alias, trait and both constant shapes are public"
    );
    assert!(
        public_headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Function { .. })),
        "the exported function must be public"
    );
    assert!(
        public_headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Struct { .. })),
        "the exported struct must be public"
    );
    assert!(
        public_headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Choice { .. })),
        "the exported choice must be public"
    );
    assert!(
        public_headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::TypeAlias { .. })),
        "the exported transparent alias must be public"
    );
    assert!(
        public_headers
            .iter()
            .any(|header| matches!(header.kind, HeaderKind::Trait { .. })),
        "the exported trait must be public"
    );
    assert_eq!(
        public_headers
            .iter()
            .filter(|header| matches!(header.kind, HeaderKind::Constant { .. }))
            .count(),
        2,
        "both exported constant shapes must be public"
    );
}

#[test]
fn export_modifier_rejects_conformance_and_incompatibility() {
    let cases = [
        "DISPLAY must:\n    show |This| -> String\n;\n\nBox = | x I32 |\n\n$export\nBox must DISPLAY\n",
        "DISPLAY must:\n    show |This| -> String\n;\n\nOTHER must:\n    other |This| -> String\n;\n\nBox = | x I32 |\n\n$export\nDISPLAY must not OTHER\n",
    ];

    for source in cases {
        let errors = expect_header_error(
            parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@entry.moth"),
            "conformances and incompatibilities are not exportable declarations",
        );
        assert_directive_reason(&errors, InvalidTemplateDirectiveCode::WrongTarget);
    }
}

#[test]
fn export_modifier_rejects_a_const_template() {
    let source = "$export\n[if true: hello]\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@entry.moth"),
        "const templates are not exportable declarations",
    );

    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::WrongTarget);
}

#[test]
fn export_modifier_rejects_a_receiver_method() {
    let source = "Box = | x I32 |\n\n$export\nvalue |this Box| -> I32:\n    return this.x\n;\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@mod.moth", "src/@entry.moth"),
        "receiver methods are not exported independently",
    );

    assert!(errors.diagnostics.views().any(|diagnostic| {
        diagnostic_has_family::<generics_traits_api::InvalidReceiverDeclarationView>(&diagnostic)
    }));
}

#[test]
fn export_modifier_is_rejected_in_helper_and_config_files() {
    for (file_path, entry_path) in [
        ("src/impl.moth", "src/@entry.moth"),
        ("src/config.moth", "src/@entry.moth"),
    ] {
        let source = "$export\nvalue #= 1\n";
        let errors = expect_header_error(
            parse_single_file_headers_with_entry(source, file_path, entry_path),
            "only an API facade root may author `$export`",
        );
        assert_directive_reason(
            &errors,
            InvalidTemplateDirectiveCode::DirectiveNotAllowedHere,
        );
    }
}

#[test]
fn export_modifier_is_allowed_in_a_reached_facade() {
    let source = "$export\nvalue #= 1\n";
    let headers = expect_header_success(
        parse_single_file_headers_with_reached_role(
            source,
            "src/plugin/@mod.moth",
            FileRole::ReachedApiFacadeRoot(ModuleRootRole::Normal),
        ),
        "a reached facade keeps its export surface",
    );

    assert!(headers.headers.iter().any(|header| {
        matches!(header.kind, HeaderKind::Constant { .. })
            && header.export_mode == HeaderExportMode::Public
    }));
}

#[test]
fn export_modifier_rejects_invalid_dependency_shapes() {
    let cases = [
        (
            "$export\n@impl as impl_ns\n",
            InvalidDependencyCode::InvalidExportTarget,
        ),
        (
            "$export\n@impl Box,\n",
            InvalidDependencyCode::MissingSelectionAfterComma,
        ),
    ];

    for (source, expected) in cases {
        let mut string_table = StringTable::new();
        let mut path_fork = PathInternerFork::empty();
        let mut span_builder = ExtendedSpanBuilder::new();
        let failure = match prepare_active_root_with_role(
            source,
            &PathBuf::from("src/@mod.moth"),
            ModuleRootRole::Normal,
            &mut string_table,
            &mut span_builder,
            &mut path_fork,
        ) {
            Ok(_) => panic!("an invalid public dependency must be rejected"),
            Err(failure) => failure,
        };
        let FileFrontendPrepareFailure::Diagnosed(error) = failure else {
            panic!("the fixture should diagnose the invalid dependency");
        };
        let diagnostics = diagnostic_view_for_test(error.diagnostic);
        let diagnostic = diagnostics
            .first()
            .expect("the dependency failure should compact to one record");
        match expected {
            InvalidDependencyCode::InvalidExportTarget => assert!(diagnostic_has_family::<
                generics_traits_api::InvalidExportTargetView,
            >(&diagnostic)),
            InvalidDependencyCode::MissingSelectionAfterComma => {
                let facts = diagnostic
                    .family_view::<project_config::InvalidDependencyClauseView>()
                    .expect("trailing comma keeps its dependency-clause diagnostic");
                assert_eq!(
                    facts.reason,
                    InvalidDependencyClauseReason::MissingSelectionAfterComma
                );
            }
        }
    }
}

#[test]
fn exported_namespace_dependencies_report_public_target_before_provider_binding() {
    let cases = [
        ("$export\n@impl\n", "@impl"),
        ("$export\n@impl as impl_ns\n", "@impl"),
        ("$export\n@core/math\n", "@core/math"),
        ("$export\n@drawing.js\n", "@drawing.js"),
        ("$export\n@drawing.js as drawing\n", "@drawing.js"),
    ];

    for (source, path) in cases {
        let mut string_table = StringTable::new();
        let mut path_fork = PathInternerFork::empty();
        let mut span_builder = ExtendedSpanBuilder::new();
        let failure = match prepare_active_root_with_role(
            source,
            &PathBuf::from("src/@mod.moth"),
            ModuleRootRole::Normal,
            &mut string_table,
            &mut span_builder,
            &mut path_fork,
        ) {
            Ok(_) => panic!("a public namespace dependency must be rejected"),
            Err(failure) => failure,
        };
        let FileFrontendPrepareFailure::Diagnosed(error) = failure else {
            panic!("the fixture should diagnose the public namespace dependency");
        };
        let diagnostics = diagnostic_view_for_test(error.diagnostic);
        let diagnostic = diagnostics
            .first()
            .expect("the dependency failure should compact to one record");
        assert!(
            diagnostic_has_family::<generics_traits_api::InvalidExportTargetView>(&diagnostic),
            "`{source}` must report an invalid public export target"
        );
        assert_eq!(
            diagnostic_primary_span(diagnostic),
            Some(source_span_for(source, path, 0)),
            "the public-target failure must point at the namespace path for `{source}`"
        );
    }
}

enum InvalidDependencyCode {
    InvalidExportTarget,
    MissingSelectionAfterComma,
}

#[test]
fn template_head_rejects_export_source_modifier() {
    let failure = parse_single_file_ast_diagnostic("[: [$export: body] ]");
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0021",
        Some("invalid_template_directive.wrong_context"),
    );
    let facts = diagnostic
        .family_view::<InvalidTemplateDirectiveView>()
        .expect("the template-head directive keeps its typed facts");
    assert_eq!(facts.reason, InvalidTemplateDirectiveCode::WrongContext);
}

#[test]
fn doc_and_todo_stack_on_one_line_before_a_declaration() {
    let source = "$doc [: Public docs.] $todo [: Follow up.]\ncalculate |value I32| -> I32:\n    return value\n;\n";
    let headers = expect_header_success(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "stacked source directives should attach to one declaration",
    );
    let header = headers
        .headers
        .iter()
        .find(|header| !header.source_directives.is_empty())
        .expect("the declaration should retain its source directive run");
    assert_eq!(
        header
            .source_directives
            .iter()
            .map(|directive| directive.kind)
            .collect::<Vec<_>>(),
        [SourceDirectiveKind::Doc, SourceDirectiveKind::Todo]
    );
}

#[test]
fn parenthesized_doc_call_is_rejected() {
    let source = "$doc(\"Description\")\ncalculate || -> I32:\n    return 1\n;\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "the documented call spelling must not be accepted as a directive operand",
    );

    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::UnexpectedArguments);
}

#[test]
fn parenthesized_todo_call_is_rejected() {
    let source = "$todo(\"Follow up\")\ncalculate || -> I32:\n    return 1\n;\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "the documented call spelling must not be accepted as a directive operand",
    );

    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::UnexpectedArguments);
}

#[test]
fn whitespace_separated_grouped_doc_operand_is_accepted() {
    let source = "$doc (\"Description\")\ncalculate || -> I32:\n    return 1\n;\n";
    let headers = expect_header_success(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "whitespace keeps a grouped expression distinct from a directive call",
    );
    let header = headers
        .headers
        .iter()
        .find(|header| !header.source_directives.is_empty())
        .expect("the declaration should retain its source directive run");
    assert_eq!(header.source_directives[0].kind, SourceDirectiveKind::Doc);
}

#[test]
fn adjacent_same_line_directives_require_whitespace() {
    let source = "$doc \"first\"$todo \"second\"\ncalculate || -> I32:\n    return 1\n;\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "two directives on one line must be separated by whitespace",
    );

    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::MissingSeparator);
}

#[test]
fn adjacent_directives_after_an_earlier_line_break_require_whitespace() {
    let source = "$config\n$doc \"first\"$todo \"second\"\ngreeting #= \"hello\"\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "an earlier newline must not hide adjacency between later directives",
    );

    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::MissingSeparator);
}

#[test]
fn adjacent_statement_directives_require_whitespace() {
    let failure = parse_single_file_ast_diagnostic(
        "greeting || -> String:\n    $doc \"first\"$todo \"second\"\n    name #String = \"hello\"\n    return name\n;\n",
    );
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0021",
        Some("invalid_template_directive.missing_separator"),
    );
    let facts = diagnostic
        .family_view::<InvalidTemplateDirectiveView>()
        .expect("the statement directive keeps its typed facts");
    assert_eq!(facts.reason, InvalidTemplateDirectiveCode::MissingSeparator);
}

#[test]
fn duplicate_doc_on_one_line_cites_the_first_directive() {
    let source =
        "$doc \"first\" $doc \"second\"\ncalculate |value I32| -> I32:\n    return value\n;\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "a repeated directive name must fail even on one line",
    );
    let diagnostic =
        assert_directive_reason(&errors, InvalidTemplateDirectiveCode::DuplicateModifier);
    assert!(
        earlier_directive_span(diagnostic).is_some(),
        "the duplicate must cite the earlier authored directive"
    );
}

#[test]
fn target_on_the_directive_line_is_rejected() {
    let source = "$config api_url #String = \"https://example.test\"\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "the target must start on a later source line",
    );
    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::UnexpectedArguments);
}

#[test]
fn todo_before_a_direct_const_template_is_wrong_target() {
    let source = "$todo [: fragment note]\n[:body]\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "a source directive cannot attach to const fragment metadata",
    );
    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::WrongTarget);
}
