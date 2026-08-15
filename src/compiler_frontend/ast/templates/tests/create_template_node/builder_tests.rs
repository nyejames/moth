use super::*;
use crate::compiler_frontend::ast::templates::template::TemplateType;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidTemplateDirectiveReason,
};
use crate::compiler_frontend::style_directives::{
    StyleDirectiveArgumentType, StyleDirectiveEffects, StyleDirectiveHandlerSpec,
    StyleDirectiveRegistry, StyleDirectiveSpec, TemplateHeadCompatibility,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn formatter_directive_is_unknown_without_builder_registration() {
    let error = template_parse_error("[$formatter(markdown, 10): body]");

    assert!(
        error.contains("Style directive '$formatter' is unsupported here"),
        "unexpected error message: {error}"
    );
}

#[test]
fn unknown_style_directives_error_cleanly() {
    let error = template_parse_error("[$unknown: body]");

    assert!(
        error.contains("Style directive '$unknown' is unsupported here"),
        "unexpected error message: {error}"
    );
}

#[test]
fn ignore_is_rejected_as_unsupported_style_directive() {
    let error = template_parse_error("[$ignore: body]");

    assert!(
        error.contains("Style directive '$ignore' is unsupported here"),
        "unexpected error message: {error}"
    );
}

#[test]
fn template_head_fallback_unknown_directive_uses_standard_metadata() {
    let tokenization_registry =
        StyleDirectiveRegistry::merged(&[StyleDirectiveSpec::handler_no_op(
            "brand",
            TemplateBodyMode::Normal,
        )])
        .expect("test directive should merge for tokenization");
    let parser_registry = frontend_test_style_directives();

    // Re-parse with a context that lacks '$brand' to exercise template-head fallback dispatch.
    let mut string_table = StringTable::new();
    let mut token_stream = template_tokens_from_source_with_style_directives(
        "[$brand: body]",
        &tokenization_registry,
        &mut string_table,
    );
    let context = new_constant_context_with_style_directives(
        token_stream.src_path.to_owned(),
        &parser_registry,
    );
    let fallback_error = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect_err("template-head fallback should reject missing registry directives");
    let fallback_error = expect_template_diagnostic(fallback_error);

    match &fallback_error.payload {
        DiagnosticPayload::InvalidTemplateDirective {
            directive_name,
            reason: InvalidTemplateDirectiveReason::UnknownDirective,
        } => {
            let directive_name =
                directive_name.expect("unknown directive should preserve its name");
            assert_eq!(string_table.resolve(directive_name), "brand");
        }
        payload => panic!("expected unknown directive payload, found {payload:?}"),
    }
    assert!(fallback_error.primary_location.start_pos.char_column > 0);
}

#[test]
fn builder_registered_style_directive_parses_as_noop_scaffold() {
    let mut string_table = StringTable::new();
    let directives = vec![StyleDirectiveSpec::handler_no_op(
        "brand",
        TemplateBodyMode::Normal,
    )];
    let registry = StyleDirectiveRegistry::merged(&directives)
        .expect("provided directive should merge with core directives");
    let mut token_stream = template_tokens_from_source_with_directives(
        "[$brand: body]",
        &directives,
        &mut string_table,
    );
    let context =
        new_constant_context(token_stream.src_path.to_owned()).with_style_directives(&registry);

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("builder-registered directives should parse in scaffold mode");

    assert_eq!(effective_tir_style(&template, &context).id, "");
    assert!(matches!(
        effective_tir_kind(&template, &context),
        TemplateType::String
    ));
}

#[test]
fn builder_effects_only_handler_updates_style_without_formatter() {
    let mut string_table = StringTable::new();
    let directives = vec![StyleDirectiveSpec::handler(
        "brand",
        TemplateBodyMode::Normal,
        TemplateHeadCompatibility::fully_compatible_meaningful(),
        StyleDirectiveHandlerSpec::new(
            None,
            StyleDirectiveEffects {
                style_id: Some("brand"),
                ..StyleDirectiveEffects::default()
            },
            None,
        ),
    )];
    let registry = StyleDirectiveRegistry::merged(&directives)
        .expect("provided directive should merge with core directives");
    let mut token_stream = template_tokens_from_source_with_directives(
        "[$brand: body]",
        &directives,
        &mut string_table,
    );
    let context =
        new_constant_context(token_stream.src_path.to_owned()).with_style_directives(&registry);

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("effects-only directive should parse");

    let effective_style = effective_tir_style(&template, &context);
    assert_eq!(effective_style.id, "brand");
    assert!(effective_style.formatter.is_none());
}

#[test]
fn builder_registered_noop_directive_rejects_parenthesized_arguments_by_default() {
    let mut string_table = StringTable::new();
    let directives = vec![StyleDirectiveSpec::handler_no_op(
        "brand",
        TemplateBodyMode::Normal,
    )];
    let registry = StyleDirectiveRegistry::merged(&directives)
        .expect("provided directive should merge with core directives");
    let mut token_stream = template_tokens_from_source_with_directives(
        "[$brand(\"tone\"): body]",
        &directives,
        &mut string_table,
    );
    let context =
        new_constant_context(token_stream.src_path.to_owned()).with_style_directives(&registry);

    let error = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect_err("default no-op directives should reject parenthesized arguments");
    let error = expect_template_diagnostic(error);

    match &error.payload {
        DiagnosticPayload::InvalidTemplateDirective {
            reason: InvalidTemplateDirectiveReason::DirectiveNotAllowedHere,
            ..
        } => {}
        payload => panic!("expected invalid template directive payload, found {payload:?}"),
    }
}

#[test]
fn builder_registered_handler_directive_accepts_declared_optional_argument_type() {
    let mut string_table = StringTable::new();
    let directives = vec![StyleDirectiveSpec::handler(
        "brand",
        TemplateBodyMode::Normal,
        TemplateHeadCompatibility::fully_compatible_meaningful(),
        StyleDirectiveHandlerSpec::new(
            Some(StyleDirectiveArgumentType::String),
            Default::default(),
            None,
        ),
    )];
    let registry = StyleDirectiveRegistry::merged(&directives)
        .expect("provided directive should merge with core directives");
    let mut token_stream = template_tokens_from_source_with_directives(
        "[$brand(\"theme\"): body]",
        &directives,
        &mut string_table,
    );
    let context =
        new_constant_context(token_stream.src_path.to_owned()).with_style_directives(&registry);

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("provided directives should parse optional arguments when configured");

    assert!(matches!(
        effective_tir_kind(&template, &context),
        TemplateType::String
    ));
}

#[test]
fn builder_registered_handler_directive_rejects_multiple_arguments() {
    let mut string_table = StringTable::new();
    let directives = vec![StyleDirectiveSpec::handler(
        "brand",
        TemplateBodyMode::Normal,
        TemplateHeadCompatibility::fully_compatible_meaningful(),
        StyleDirectiveHandlerSpec::new(
            Some(StyleDirectiveArgumentType::String),
            Default::default(),
            None,
        ),
    )];
    let registry = StyleDirectiveRegistry::merged(&directives)
        .expect("provided directive should merge with core directives");
    let mut token_stream = template_tokens_from_source_with_directives(
        "[$brand(\"theme\", \"extra\"): body]",
        &directives,
        &mut string_table,
    );
    let context =
        new_constant_context(token_stream.src_path.to_owned()).with_style_directives(&registry);

    let error = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect_err("handler directives should reject multiple arguments");
    let error = expect_template_diagnostic(error);
    assert!(matches!(
        &error.payload,
        DiagnosticPayload::UnexpectedToken {
            found: TokenKind::Comma,
        }
    ));
}

#[test]
fn builder_registered_handler_directive_rejects_runtime_argument_values() {
    let mut string_table = StringTable::new();
    let directives = vec![StyleDirectiveSpec::handler(
        "brand",
        TemplateBodyMode::Normal,
        TemplateHeadCompatibility::fully_compatible_meaningful(),
        StyleDirectiveHandlerSpec::new(
            Some(StyleDirectiveArgumentType::String),
            Default::default(),
            None,
        ),
    )];
    let registry = StyleDirectiveRegistry::merged(&directives)
        .expect("provided directive should merge with core directives");
    let mut token_stream = template_tokens_from_source_with_directives(
        "[$brand(value): body]",
        &directives,
        &mut string_table,
    );
    let context = runtime_template_context_with_style_directives(
        &token_stream.src_path,
        &registry,
        &mut string_table,
    );

    let error = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect_err("handler directives should reject runtime-only argument values");
    let error = expect_template_diagnostic(error);
    assert!(matches!(
        &error.payload,
        DiagnosticPayload::InvalidTemplateDirective {
            reason: InvalidTemplateDirectiveReason::InvalidArgument { detail: None },
            ..
        }
    ));
}

#[test]
fn builder_registered_style_directive_preserves_raw_body_whitespace() {
    let mut string_table = StringTable::new();
    let directives = vec![StyleDirectiveSpec::handler_no_op(
        "brand",
        TemplateBodyMode::Normal,
    )];
    let registry = StyleDirectiveRegistry::merged(&directives)
        .expect("provided directive should merge with core directives");
    let mut token_stream = template_tokens_from_source_with_directives(
        "[$brand:\n    Hello\n    World\n]",
        &directives,
        &mut string_table,
    );
    let context =
        new_constant_context(token_stream.src_path.to_owned()).with_style_directives(&registry);

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("builder-registered directives should parse in scaffold mode");
    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(string_table.resolve(folded), "\n    Hello\n    World\n");
}

#[test]
fn builder_directive_cannot_override_builtin_slot_name() {
    let directives = vec![StyleDirectiveSpec::handler_no_op(
        "slot",
        TemplateBodyMode::Normal,
    )];
    let error = StyleDirectiveRegistry::merged(&directives)
        .expect_err("frontend-owned directive overrides should fail during registry merge");
    assert!(
        error
            .msg
            .contains("cannot override frontend-owned directive")
    );
}
