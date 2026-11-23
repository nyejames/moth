//! Source-directive recognition and attachment tests.
//!
//! WHAT: covers source-modifier recognition, attachment and placement diagnostics, including the
//!       prepared-file exclusion of `$export` operands that AST rejects.
//! WHY: directive attachment spans header parsing and AST body dispatch; these cases keep the
//!       authored spans and the recorded attachment fact on one focused owner.

use super::*;
use crate::compiler_frontend::compiler_messages::PathKind;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    CaseConflictingPublicExportRouteView, InvalidReceiverDeclarationCode,
    InvalidReceiverDeclarationView,
};
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
        durable_label_span(label.site)
    })
}

fn previous_declaration_span(diagnostic: DiagnosticView<'_>) -> Option<SourceSpan> {
    diagnostic.durable_labels().find_map(|label| {
        matches!(label.message, DurableLabelMessage::PreviousDeclaration).then_some(())?;
        durable_label_span(label.site)
    })
}

fn durable_label_span(site: SiteRef<'_>) -> Option<SourceSpan> {
    match site {
        SiteRef::Absent => None,
        SiteRef::Local(span) | SiteRef::Foreign { span, .. } => Some(span),
    }
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
        .filter(|header| header.export_mode.is_public())
        .collect();
    assert_eq!(public_headers.len(), 1, "only the marked struct is public");
    assert!(
        public_headers[0].export_mode.route().is_none(),
        "an operand-free `$export` keeps the root destination"
    );
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
        HeaderExportMode::Public { route: None }
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
fn statement_non_export_operands_keep_their_file_references() {
    // Only `$export` is impossible inside a retained body, so its operand row is consumed while
    // the AST owns that rejection. A `$doc` or `$todo` operand stays an ordinary expression and
    // its content path remains graph-active.
    for directive in ["$doc", "$todo"] {
        let source =
            format!("load || -> Bool:\n    {directive} [@docs/intro.mtf]\n    return true\n;\n");
        let mut string_table = StringTable::new();
        let (output, _span_builder) = prepare_api_root_with_fork(
            &source,
            std::path::Path::new("src/@mod.moth"),
            &mut string_table,
            &mut PathInternerFork::empty(),
        );

        let references = output.structural_file_references.references();
        assert_eq!(
            references.len(),
            1,
            "a {directive} operand must stay an ordinary file reference"
        );
        assert_eq!(
            references[0].class,
            crate::compiler_frontend::paths::file_references::PreparedFileReferenceClass::ContentSource
        );
        assert_eq!(
            references[0].span,
            source_span_for(&source, "@docs/intro.mtf", 0)
        );
    }
}

#[test]
fn valid_top_level_export_route_stays_a_namespace_operand() {
    // The accepted route surface is namespace-only, so preparation must not queue a provider or
    // content source for the route path itself.
    let source = "$export @forms\nButton = | label String |\n";
    let mut string_table = StringTable::new();
    let (output, _span_builder) = prepare_api_root_with_fork(
        source,
        std::path::Path::new("src/@mod.moth"),
        &mut string_table,
        &mut PathInternerFork::empty(),
    );

    assert!(
        output.structural_file_references.references().is_empty(),
        "a legal export route must not become a structural file reference"
    );
}

fn assert_export_operand_left_no_path_facts(output: &FileFrontendPrepareOutput, source: &str) {
    assert!(
        output.qualified_references.is_empty(),
        "an export operand must not become a qualified occurrence: {source}"
    );
    assert!(
        output.structural_file_references.references().is_empty(),
        "an export operand must not become a structural file reference: {source}"
    );
}

#[test]
fn export_operands_inside_retained_ranges_are_consumed() {
    // An export that can never attach to an eligible target is rejected by AST, but its authored
    // operand row must never become a Stage 0 provider, content or ordering fact first.
    let sources = [
        "render || -> String:\n    $export @docs.mtf\n    return \"x\"\n;\n",
        "render || -> String:\n    $export @missing symbol\n    return \"x\"\n;\n",
        "render || -> String:\n    if true:\n        $export @missing symbol\n        return \"x\"\n    ;\n    return \"y\"\n;\n",
        "load #|| -> Bool:\n    $export @helper value\n    return true\n;\n",
        "count #I32 = cast \"0\" catch:\n    $export @missing extra\n    then 0\n;\n",
        "load |value I32 = cast \"0\" catch:\n    $export @missing extra\n    then 0\n| -> I32:\n    return value\n;\n",
        "helper #= 1\n\nrender || -> String:\n    return $export @helper value\n;\n",
        "render || -> String:\n    return ($export @missing symbol)\n;\n",
        "classify |value I32| -> I32:\n    if value is:\n        1 => return 1\n        $export @forms\n        2 => return 2\n    ;\n;\n",
        "classify |value I32| -> I32:\n    if value is:\n        1 => return 1\n        $export @forms\n        next:\n    ;\n;\n",
        "render |flag Bool| -> String:\n    if flag: return \"yes\" else $export @forms;\n    return \"no\"\n;\n",
        "render || -> String:\n    $export @forms\n    $export @forms\n    return \"x\"\n;\n",
        "render || -> String:\n    $export\n    return \"x\"\n;\n",
        "load || -> Bool:\n    $doc \"first\"$export @forms\n    return true\n;\n",
        "render || -> String:\n    return [: [$export @missing symbol: body] ]\n;\n",
    ];
    for source in sources {
        let mut string_table = StringTable::new();
        let mut span_builder = ExtendedSpanBuilder::new();
        let output = prepare_active_root_with_role(
            source,
            std::path::Path::new("src/@mod.moth"),
            ModuleRootRole::Normal,
            &mut string_table,
            &mut span_builder,
            &mut PathInternerFork::empty(),
        )
        .unwrap_or_else(|failure| panic!("source failed preparation: {source}\n{failure:?}"));
        assert_export_operand_left_no_path_facts(&output, source);
    }
}

#[test]
fn export_operand_inside_the_entry_start_body_is_consumed() {
    let source = "value = 1\n\nif value is 1:\n    $export @docs.mtf\n    value = 2\n;\n";
    let mut string_table = StringTable::new();
    let (output, _span_builder) = prepare_single_file_with_fork(
        source,
        std::path::Path::new("src/@entry.moth"),
        std::path::Path::new("src/@entry.moth"),
        &mut string_table,
        &mut PathInternerFork::empty(),
    );
    assert_export_operand_left_no_path_facts(&output, source);
}

#[test]
fn export_operand_only_leaves_trailing_paths_to_ordinary_discovery() {
    // Only the immediate route operand is directive-owned. Later path tokens on the malformed
    // line keep ordinary conservative discovery, so their provider occurrence survives.
    let source =
        "render || -> String:\n    $export @route leaf as @missing symbol\n    return \"x\"\n;\n";
    let mut string_table = StringTable::new();
    let mut span_builder = ExtendedSpanBuilder::new();
    let output = prepare_active_root_with_role(
        source,
        std::path::Path::new("src/@mod.moth"),
        ModuleRootRole::Normal,
        &mut string_table,
        &mut span_builder,
        &mut PathInternerFork::empty(),
    )
    .unwrap_or_else(|failure| panic!("source failed preparation: {source}\n{failure:?}"));

    assert_eq!(output.qualified_references.len(), 1);
    assert_eq!(
        output.qualified_references[0].dependency.span,
        source_span_for(source, "@missing", 0),
        "the trailing provider selection keeps its authored span"
    );
    assert!(
        output.structural_file_references.references().is_empty(),
        "the route operand must not become a file value and `leaf` is not a path"
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
        matches!(header.kind, HeaderKind::Constant { .. }) && header.export_mode.is_public()
    }));

    let crlf = "$export\r\nlabel #= \"x\"\r\n";
    let headers = expect_header_success(
        parse_single_file_headers_with_entry(crlf, "src/@mod.moth", "src/@entry.moth"),
        "CRLF after `$export` should attach like LF",
    );
    assert!(headers.headers.iter().any(|header| {
        matches!(header.kind, HeaderKind::Constant { .. }) && header.export_mode.is_public()
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
            "`$export` accepts at most one bare route operand",
        );
        let diagnostic = assert_directive_reason(
            &errors,
            InvalidTemplateDirectiveCode::InvalidExportRouteOperand,
        );
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
        .filter(|header| header.export_mode.is_public())
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
        ("src/@config.moth", "src/@entry.moth"),
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
        matches!(header.kind, HeaderKind::Constant { .. }) && header.export_mode.is_public()
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

fn export_route_diagnostics(source: &str) -> HeaderTestDiagnostics {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    match prepare_active_root_with_role(
        source,
        &PathBuf::from("src/@mod.moth"),
        ModuleRootRole::Normal,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    ) {
        Ok(_) => panic!("the export route should be rejected"),
        Err(FileFrontendPrepareFailure::Diagnosed(error)) => HeaderTestDiagnostics {
            diagnostics: diagnostic_view_for_test(error.diagnostic),
        },
        Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
            panic!("the export route fixture should diagnose source, not infrastructure: {error:?}")
        }
    }
}

fn rendered_public_route(
    output: &FileFrontendPrepareOutput,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> String {
    let route = output
        .headers
        .iter()
        .find_map(|header| header.export_mode.route())
        .expect("the exported declaration should retain a public route");
    path_fork.render_portable(route.path, string_table, &mut Vec::new())
}

#[test]
fn export_route_is_retained_on_a_declaration() {
    let source = "$export @tables\nButton = | label String |\n";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (output, _span_builder) = prepare_api_root_with_fork(
        source,
        std::path::Path::new("src/@mod.moth"),
        &mut string_table,
        &mut path_fork,
    );

    let header = output
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Struct { .. }))
        .expect("the exported struct should parse");
    let route = header
        .export_mode
        .route()
        .expect("`$export @tables` should retain its route");
    assert_eq!(route.span, source_span_for(source, "@tables", 0));
    assert_eq!(
        path_fork.render_portable(route.path, &string_table, &mut Vec::new()),
        "tables"
    );
    assert!(
        output.structural_file_references.references().is_empty(),
        "a route path row must not become a structural file reference"
    );
    assert!(
        output.qualified_references.is_empty(),
        "a route path row must not become a qualified provider reference"
    );
    assert!(
        output.file_dependency_clauses.is_empty(),
        "a declaration route must not create a dependency clause"
    );
}

#[test]
fn export_route_retains_a_multi_component_suffix() {
    let source = "$export @tables/layout\nTableLayout = | columns I32 |\n";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (output, _span_builder) = prepare_api_root_with_fork(
        source,
        std::path::Path::new("src/@mod.moth"),
        &mut string_table,
        &mut path_fork,
    );

    assert_eq!(
        rendered_public_route(&output, &path_fork, &string_table),
        "tables/layout"
    );
    assert!(output.structural_file_references.references().is_empty());
}

#[test]
fn export_route_is_retained_on_a_support_package_facade() {
    let source = "$export @forms\nButton = | label String |\n";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let output = prepare_active_root_with_role(
        source,
        &PathBuf::from("src/@package.moth"),
        ModuleRootRole::Support,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    )
    .expect("a support facade should accept a routed export");

    assert_eq!(
        rendered_public_route(&output, &path_fork, &string_table),
        "forms"
    );
}

#[test]
fn export_route_survives_crlf_and_a_comment_only_line() {
    let source = "$export @forms\r\n-- public form\r\nButton = | label String |\r\n";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let output = prepare_active_root_with_role(
        source,
        &PathBuf::from("src/@mod.moth"),
        ModuleRootRole::Normal,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    )
    .expect("CRLF and a comment-only line must not break route attachment");

    assert_eq!(
        rendered_public_route(&output, &path_fork, &string_table),
        "forms"
    );
}

#[test]
fn doc_and_route_stack_on_one_target() {
    let source = "$doc [: Public docs.] $export @forms\nButton = | label String |\n";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (output, _span_builder) = prepare_api_root_with_fork(
        source,
        std::path::Path::new("src/@mod.moth"),
        &mut string_table,
        &mut path_fork,
    );

    let header = output
        .headers
        .iter()
        .find(|header| matches!(header.kind, HeaderKind::Struct { .. }))
        .expect("the documented exported struct should parse");
    assert_eq!(
        header
            .source_directives
            .iter()
            .map(|directive| directive.kind)
            .collect::<Vec<_>>(),
        [SourceDirectiveKind::Doc],
        "`$export` retains one route fact on `export_mode`, not a second annotation copy"
    );
    assert_eq!(
        rendered_public_route(&output, &path_fork, &string_table),
        "forms"
    );
}

#[test]
fn export_route_is_retained_on_a_multi_selection_clause() {
    let source = "$export @forms\n@button Button, render as render_button\n";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let output = prepare_active_root_with_role(
        source,
        &PathBuf::from("src/@mod.moth"),
        ModuleRootRole::Normal,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    )
    .expect("a facade should accept a routed multi-selection export");

    assert_eq!(output.file_dependency_clauses.len(), 1);
    let clause = &output.file_dependency_clauses[0];
    let route = clause
        .export_mode
        .route()
        .expect("the clause should retain its route");
    assert_eq!(route.span, source_span_for(source, "@forms", 0));
    assert_eq!(
        path_fork.render_portable(route.path, &string_table, &mut Vec::new()),
        "forms"
    );
    let selections = clause
        .selections(&output.dependency_selections)
        .expect("the exported clause retains its selections");
    assert_eq!(selections.len(), 2);
    assert_eq!(string_table.resolve(selections[0].source_name), "Button");
    assert!(selections[0].local_alias.is_none());
    assert_eq!(string_table.resolve(selections[1].source_name), "render");
    let alias = selections[1]
        .local_alias
        .as_ref()
        .expect("the second selection carries its authored alias");
    assert_eq!(string_table.resolve(alias.name), "render_button");
    assert_eq!(alias.span, source_span_for(source, "render_button", 0));
    assert!(
        output.structural_file_references.references().is_empty(),
        "neither the provider path nor the route path is a structural file value"
    );
    assert!(output.qualified_references.is_empty());
}

#[test]
fn same_leaf_may_publish_under_distinct_routes() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    prepare_active_root_with_role(
        "$export @forms\n@button Button as Control\n$export @forms/validation\n@widget Button as Control\n",
        &PathBuf::from("src/@mod.moth"),
        ModuleRootRole::Normal,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    )
    .expect("one leaf under two distinct routes is two public identities");
}

#[test]
fn two_symbols_may_share_one_route() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    prepare_active_root_with_role(
        "$export @forms\ncells #= 1\n$export @forms\nheading #= 2\n",
        &PathBuf::from("src/@mod.moth"),
        ModuleRootRole::Normal,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    )
    .expect("one route is shared by several exported leaves");
}

#[test]
fn case_only_route_conflict_cites_both_route_operands() {
    for (source, earlier, later) in [
        (
            "$export @forms\nButton = | label String |\n$export @Forms\nLabel = | text String |\n",
            "@forms",
            "@Forms",
        ),
        (
            "$export @forms/Validation\nButton = | label String |\n$export @Forms/validation\nLabel = | text String |\n",
            "@forms/Validation",
            "@Forms/validation",
        ),
    ] {
        let diagnostics = export_route_diagnostics(source);
        let diagnostic = diagnostics
            .diagnostics
            .first()
            .expect("the route conflict should compact to one record");
        assert_eq!(diagnostic.descriptor().code, "MOTH-RULE-0092");
        assert!(
            diagnostic
                .family_view::<CaseConflictingPublicExportRouteView>()
                .is_some()
        );
        assert_eq!(
            diagnostic_primary_span(diagnostic),
            Some(source_span_for(source, later, 0))
        );
        assert_eq!(
            previous_declaration_span(diagnostic),
            Some(source_span_for(source, earlier, 0)),
            "the earlier authored route operand should be the related site"
        );
    }
}

#[test]
fn route_conflict_labels_the_authored_order_across_collection_passes() {
    // The re-export clause comes first in source but is collected after declaration routes, so the
    // diagnostic must order claims by authored source, not by collection pass.
    let source = "$export @Forms\n@button Button\n$export @forms\nLabel = | text String |\n";
    let diagnostics = export_route_diagnostics(source);
    let diagnostic = diagnostics
        .diagnostics
        .first()
        .expect("the cross-pass route conflict should compact to one record");
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(source, "@forms", 0)),
        "the later authored declaration route is the duplicate"
    );
    assert_eq!(
        previous_declaration_span(diagnostic),
        Some(source_span_for(source, "@Forms", 0)),
        "the earlier authored clause route should be the related site"
    );
}

#[test]
fn parent_and_child_export_routes_remain_legal() {
    let source = "$export @forms\ncells #= 1\n$export @forms/validation\nvalidate #= 2\n";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    prepare_active_root_with_role(
        source,
        &PathBuf::from("src/@mod.moth"),
        ModuleRootRole::Normal,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    )
    .expect("a parent route and its child are distinct public routes");
}

#[test]
fn duplicate_export_routes_cite_both_authored_spans() {
    for source in [
        "$export @a\n$export @b\nButton = | label String |\n",
        "$export @a $export @b\nButton = | label String |\n",
    ] {
        let diagnostics = export_route_diagnostics(source);
        let diagnostic = assert_directive_reason(
            &diagnostics,
            InvalidTemplateDirectiveCode::DuplicateModifier,
        );
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
}

#[test]
fn export_route_rejects_trailing_tokens_and_second_operands() {
    for (source, needle) in [
        (
            "$export @tables forms\nButton = | label String |\n",
            "forms",
        ),
        ("$export @a @b\nButton = | label String |\n", "@b"),
        ("$export @tables(\"x\")\nButton = | label String |\n", "("),
    ] {
        let diagnostics = export_route_diagnostics(source);
        let diagnostic = assert_directive_reason(
            &diagnostics,
            InvalidTemplateDirectiveCode::InvalidExportRouteOperand,
        );
        assert_eq!(
            diagnostic_primary_span(diagnostic),
            Some(source_span_for(source, needle, 0)),
            "the offending token owns the operand diagnostic for `{source}`"
        );
    }
}

#[test]
fn export_route_rejects_invalid_path_spellings() {
    for (source, needle, expected) in [
        (
            "$export @/\nButton = | label String |\n",
            "@/",
            InvalidTemplateDirectiveCode::InvalidExportRoutePublicRoot,
        ),
        (
            "$export @./forms\nButton = | label String |\n",
            "@./forms",
            InvalidTemplateDirectiveCode::InvalidExportRouteCurrentDirectorySegment,
        ),
        (
            "$export @tables.js\nButton = | label String |\n",
            "@tables.js",
            InvalidTemplateDirectiveCode::InvalidExportRouteExtensionSegment,
        ),
        (
            "$export @..\nButton = | label String |\n",
            "@..",
            InvalidTemplateDirectiveCode::InvalidExportRouteParentDirectorySegment,
        ),
        (
            "$export @../forms\nButton = | label String |\n",
            "@../forms",
            InvalidTemplateDirectiveCode::InvalidExportRouteParentDirectorySegment,
        ),
        (
            "$export @tables.moth\nButton = | label String |\n",
            "@tables.moth",
            InvalidTemplateDirectiveCode::InvalidExportRouteExtensionSegment,
        ),
        (
            "$export @tables.moth/layout\nButton = | label String |\n",
            "@tables.moth/layout",
            InvalidTemplateDirectiveCode::InvalidExportRouteExtensionSegment,
        ),
    ] {
        let diagnostics = export_route_diagnostics(source);
        let diagnostic = assert_directive_reason(&diagnostics, expected);
        assert_eq!(
            diagnostic_primary_span(diagnostic),
            Some(source_span_for(source, needle, 0)),
            "the route operand owns the spelling diagnostic for `{source}`"
        );
    }
}

#[test]
fn export_route_rejects_lexically_invalid_path_components() {
    for (source, expected_prefix, expected_kind) in [
        (
            "$export @forms/../x\nButton = | label String |\n",
            "@forms/..",
            PathKind::InvalidComponent,
        ),
        (
            "$export @forms/./x\nButton = | label String |\n",
            "@forms/.",
            PathKind::InvalidComponent,
        ),
        (
            "$export @forms/\nButton = | label String |\n",
            "@forms/",
            PathKind::TrailingSeparator,
        ),
    ] {
        let diagnostics = export_route_diagnostics(source);
        let diagnostic = diagnostics
            .diagnostics
            .first()
            .expect("the invalid component should compact to one record");
        assert_eq!(diagnostic.descriptor().code, "MOTH-SYNTAX-0018");
        let facts = diagnostic
            .family_view::<token_lexical::InvalidPathView>()
            .expect("the invalid component keeps its typed invalid-path facts");
        assert_eq!(facts.path_kind, expected_kind);
        assert_eq!(
            diagnostic_primary_span(diagnostic),
            Some(source_span_for(source, expected_prefix, 0)),
            "the path prefix through the offending component owns its lexical diagnostic"
        );
    }
}

#[test]
fn blank_line_dangles_a_routed_export() {
    let source = "$export @x\n\nButton = | label String |\n";
    let diagnostics = export_route_diagnostics(source);
    let diagnostic =
        assert_directive_reason(&diagnostics, InvalidTemplateDirectiveCode::DanglingModifier);
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(source, "$export", 0))
    );
}

#[test]
fn routed_export_keeps_existing_target_eligibility() {
    let runtime_statement = "$export @forms\nvalue = 1\n";
    let diagnostics = export_route_diagnostics(runtime_statement);
    let diagnostic =
        assert_directive_reason(&diagnostics, InvalidTemplateDirectiveCode::WrongTarget);
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(runtime_statement, "$export", 0))
    );

    let receiver_method =
        "Box = | x I32 |\n\n$export @forms\nvalue |this Box| -> I32:\n    return this.x\n;\n";
    let diagnostics = export_route_diagnostics(receiver_method);
    let diagnostic = diagnostics
        .diagnostics
        .views()
        .find(|diagnostic| diagnostic_has_family::<InvalidReceiverDeclarationView>(diagnostic))
        .expect("the routed receiver method should keep its receiver diagnostic");
    let facts = diagnostic
        .family_view::<InvalidReceiverDeclarationView>()
        .expect("the receiver diagnostic retains its typed facts");
    assert_eq!(
        facts.code,
        InvalidReceiverDeclarationCode::ReceiverMethodImportOrExportNotAllowed,
        "a routed export still cannot publish a receiver method"
    );
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(receiver_method, "value", 0)),
        "the receiver declaration owns the rejection span"
    );

    let conformance = "DISPLAY must:\n    show |This| -> String\n;\n\nBox = | x I32 |\n\n$export @forms\nBox must DISPLAY\n";
    let diagnostics = export_route_diagnostics(conformance);
    let diagnostic =
        assert_directive_reason(&diagnostics, InvalidTemplateDirectiveCode::WrongTarget);
    assert_eq!(
        diagnostic_primary_span(diagnostic),
        Some(source_span_for(conformance, "$export", 0))
    );
}

#[test]
fn routed_export_rejects_namespace_clauses() {
    for source in [
        "$export @forms\n@impl\n",
        "$export @forms\n@impl as impl_ns\n",
    ] {
        let diagnostics = export_route_diagnostics(source);
        let diagnostic = diagnostics
            .diagnostics
            .first()
            .expect("the invalid export target should compact to one record");
        assert!(
            diagnostic_has_family::<generics_traits_api::InvalidExportTargetView>(&diagnostic),
            "`{source}` must keep the invalid public export target diagnostic"
        );
        assert_eq!(
            diagnostic_primary_span(diagnostic),
            Some(source_span_for(source, "@impl", 0))
        );
    }
}

#[test]
fn routed_export_keeps_placement_rejections() {
    for (file_path, entry_path) in [
        ("src/@entry.moth", "src/@entry.moth"),
        ("src/impl.moth", "src/@entry.moth"),
        ("src/@config.moth", "src/@entry.moth"),
    ] {
        let source = "$export @forms\nvalue #= 1\n";
        let errors = expect_header_error(
            parse_single_file_headers_with_entry(source, file_path, entry_path),
            "only an API facade root may author a routed `$export`",
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
}

enum InvalidDependencyCode {
    InvalidExportTarget,
    MissingSelectionAfterComma,
}

#[test]
fn template_head_rejects_export_source_modifier() {
    for source in ["[: [$export: body] ]", "[: [$export @forms: body] ]"] {
        let failure = parse_single_file_ast_diagnostic(source);
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

#[test]
fn constraint_modifiers_attach_to_free_function_method_and_generic_function() {
    let source = "\
$fast_math $infallible
calculate |value I32| -> I32:
    return value
;

Box = | value I32 |

$safe_math $fast_math
read |this Box| -> I32:
    return this.value
;

$fast_math
identity type T |value T| -> T:
    return value
;
";
    let headers = expect_header_success(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "constraints must attach to every callable header shape",
    );

    let attached: Vec<Vec<SourceDirectiveKind>> = headers
        .headers
        .iter()
        .filter(|header| matches!(header.kind, HeaderKind::Function { .. }))
        .map(|header| {
            header
                .source_directives
                .iter()
                .map(|directive| directive.kind)
                .collect()
        })
        .collect();
    assert_eq!(
        attached,
        [
            vec![
                SourceDirectiveKind::FastMath,
                SourceDirectiveKind::Infallible
            ],
            vec![SourceDirectiveKind::SafeMath, SourceDirectiveKind::FastMath],
            vec![SourceDirectiveKind::FastMath],
        ],
        "methods and generic functions are HeaderKind::Function and keep the authored order"
    );
}

#[test]
fn constraint_modifiers_stack_in_either_order() {
    for (directives, expected) in [
        (
            "$safe_math $fast_math",
            [SourceDirectiveKind::SafeMath, SourceDirectiveKind::FastMath],
        ),
        (
            "$fast_math $safe_math",
            [SourceDirectiveKind::FastMath, SourceDirectiveKind::SafeMath],
        ),
    ] {
        let source = format!("{directives}\ncalculate || -> I32:\n    return 1\n;\n");
        let headers = expect_header_success(
            parse_single_file_headers_with_entry(&source, "src/@entry.moth", "src/@entry.moth"),
            "different constraint names combine regardless of order",
        );
        let header = headers
            .headers
            .iter()
            .find(|header| !header.source_directives.is_empty())
            .expect("the function should retain the run");
        assert_eq!(
            header
                .source_directives
                .iter()
                .map(|directive| directive.kind)
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn constraint_modifiers_reject_non_function_headers() {
    let cases = [
        "$fast_math\nButton = | label String |\n",
        "$safe_math\nStatus :: Ready, Failed;\n",
        "$infallible\nUserId as I32\n",
        "$fast_math\nDISPLAY must:\n    show |This| -> String\n;\n",
        "$safe_math\ninferred #= 1\n",
        "$infallible\ntyped #I32 = 2\n",
        "$fast_math\n[if true: hello]\n",
    ];

    for source in cases {
        let errors = expect_header_error(
            parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
            "lexical constraints target callables only",
        );
        assert_directive_reason(&errors, InvalidTemplateDirectiveCode::WrongTarget);
    }
}

#[test]
fn constraint_modifiers_reject_const_function_headers() {
    let cases = [
        "$fast_math\nsquare #|value I32| -> I32:\n    return value * value\n;\n",
        "$safe_math\nsquare #|value I32| -> I32:\n    return value * value\n;\n",
        "$infallible\nsquare #|value I32| -> I32:\n    return value * value\n;\n",
    ];

    for source in cases {
        let errors = expect_header_error(
            parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
            "const functions execute only at compile time and cannot carry runtime constraints",
        );
        assert_directive_reason(
            &errors,
            InvalidTemplateDirectiveCode::CompileTimeFunctionTarget,
        );
    }
}

#[test]
fn constraint_modifiers_reject_dependency_targets() {
    let source = "$fast_math\n@impl Box,\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "dependency clauses are not constraint targets",
    );
    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::WrongTarget);
}

#[test]
fn constraint_modifier_parentheses_are_rejected() {
    let source = "$fast_math()\ncalculate || -> I32:\n    return 1\n;\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "a parameterless constraint must not use call spelling",
    );
    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::UnexpectedArguments);
}

#[test]
fn duplicate_constraint_modifiers_cite_both_authored_spans() {
    let source = "$fast_math $fast_math\ncalculate || -> I32:\n    return 1\n;\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(source, "src/@entry.moth", "src/@entry.moth"),
        "repeating one constraint name on one target is a duplicate",
    );
    let diagnostic =
        assert_directive_reason(&errors, InvalidTemplateDirectiveCode::DuplicateModifier);
    assert_eq!(
        earlier_directive_span(diagnostic),
        Some(source_span_for(source, "$fast_math", 0)),
        "the duplicate must cite the earlier authored directive"
    );
}

#[test]
fn blank_line_and_eof_break_constraint_attachment() {
    let blank = "$fast_math\n\ncalculate || -> I32:\n    return 1\n;\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(blank, "src/@entry.moth", "src/@entry.moth"),
        "a blank line must break constraint attachment",
    );
    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::DanglingModifier);

    let eof = "$fast_math\n";
    let errors = expect_header_error(
        parse_single_file_headers_with_entry(eof, "src/@entry.moth", "src/@entry.moth"),
        "a constraint with no following target is dangling",
    );
    assert_directive_reason(&errors, InvalidTemplateDirectiveCode::DanglingModifier);
}
