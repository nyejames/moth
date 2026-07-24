use super::*;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::templates::template::{
    CommentDirectiveKind, SlotKey, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_head_parser::directive_args::{
    parse_optional_parenthesized_expression, parse_optional_slot_target_argument,
    parse_required_parenthesized_expression, parse_required_slot_name_argument,
    reject_unexpected_directive_arguments,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, SyntaxDiagnosticKind,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use std::sync::Arc;

type DirectiveStyleTestResult<T> = Result<T, Box<CompilerDiagnostic>>;

fn directive_tokens(source: &str, string_table: &mut StringTable) -> FileTokens {
    let scope = InternedPath::from_single_str("main.moth/#const_template0", string_table);
    let style_directives = frontend_test_style_directives();
    let mut tokens = tokenize(
        source,
        &scope,
        TokenizerEntryMode::SourceFile,
        &style_directives,
        string_table,
        None,
    )
    .expect("tokenization should succeed");

    tokens.index = tokens
        .tokens
        .iter()
        .position(|token| matches!(token.kind, TokenKind::StyleDirective(_)))
        .expect("expected a style directive token");

    tokens
}

fn test_context(scope: InternedPath) -> ScopeContext {
    let cwd = std::env::temp_dir();
    let resolver = ProjectPathResolver::new(
        cwd.clone(),
        cwd,
        crate::compiler_frontend::source_packages::root_file::PreparedSourcePackageRoots::empty(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
    )
    .expect("test path resolver should be valid");
    ScopeContext::new_for_tests(
        ContextKind::Constant,
        scope.clone(),
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::default()),
        vec![],
        0,
    )
    .with_project_path_resolver(Some(resolver))
    .with_source_file_scope(scope)
    .with_path_format_config(PathStringFormatConfig::default())
}

fn parse_optional_parenthesized_expression_for_test(
    tokens: &mut FileTokens,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> DirectiveStyleTestResult<
    Option<crate::compiler_frontend::ast::expressions::expression::Expression>,
> {
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let directive_name = string_table.intern("test_directive");
    parse_optional_parenthesized_expression(
        directive_name,
        tokens,
        context,
        &mut type_interner,
        string_table,
    )
}

fn parse_required_parenthesized_expression_for_test(
    tokens: &mut FileTokens,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> DirectiveStyleTestResult<crate::compiler_frontend::ast::expressions::expression::Expression> {
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let directive_name = string_table.intern("test_directive");
    parse_required_parenthesized_expression(
        directive_name,
        tokens,
        context,
        &mut type_interner,
        string_table,
    )
}

// ------------------------------------------------------------------------
// reject_unexpected_directive_arguments
// ------------------------------------------------------------------------

#[test]
fn reject_arguments_succeeds_when_no_parens() {
    let mut string_table = StringTable::new();
    let tokens = directive_tokens("[$note]", &mut string_table);
    let directive_name = string_table.intern("note");
    let result = reject_unexpected_directive_arguments(directive_name, &tokens);
    assert!(result.is_ok());
}

#[test]
fn reject_arguments_fails_when_parens_present() {
    let mut string_table = StringTable::new();
    let tokens = directive_tokens("[$note()]", &mut string_table);
    let directive_name = string_table.intern("note");
    let result = reject_unexpected_directive_arguments(directive_name, &tokens);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err().payload,
        DiagnosticPayload::InvalidTemplateDirective {
            reason: crate::compiler_frontend::compiler_messages::InvalidTemplateDirectiveReason::UnexpectedArguments,
            ..
        }
    ));
}

// ------------------------------------------------------------------------
// parse_optional_slot_target_argument
// ------------------------------------------------------------------------

#[test]
fn optional_slot_target_no_parens_returns_default() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$slot]", &mut string_table);
    let directive_name = string_table.intern("slot");
    let result = parse_optional_slot_target_argument(directive_name, &mut tokens, &string_table);
    assert_eq!(result.unwrap(), SlotKey::Default);
}

#[test]
fn optional_slot_target_named_string() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$slot(\"style\")]", &mut string_table);
    let directive_name = string_table.intern("slot");
    let result = parse_optional_slot_target_argument(directive_name, &mut tokens, &string_table);
    assert!(matches!(result.unwrap(), SlotKey::Named(_)));
}

#[test]
fn optional_slot_target_positive_positional() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$slot(1)]", &mut string_table);
    let directive_name = string_table.intern("slot");
    let result = parse_optional_slot_target_argument(directive_name, &mut tokens, &string_table);
    assert_eq!(result.unwrap(), SlotKey::Positional(1));
}

#[test]
fn optional_slot_target_zero_errors() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$slot(0)]", &mut string_table);
    let directive_name = string_table.intern("slot");
    let result = parse_optional_slot_target_argument(directive_name, &mut tokens, &string_table);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err().payload,
        DiagnosticPayload::InvalidTemplateDirective {
            reason: crate::compiler_frontend::compiler_messages::InvalidTemplateDirectiveReason::InvalidSlotTarget,
            ..
        }
    ));
}

#[test]
fn optional_slot_target_negative_errors() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$slot(-1)]", &mut string_table);
    let directive_name = string_table.intern("slot");
    let result = parse_optional_slot_target_argument(directive_name, &mut tokens, &string_table);
    assert!(result.is_err());
}

#[test]
fn optional_slot_target_empty_parens_errors() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$slot()]", &mut string_table);
    let directive_name = string_table.intern("slot");
    let result = parse_optional_slot_target_argument(directive_name, &mut tokens, &string_table);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err().payload,
        DiagnosticPayload::InvalidTemplateDirective {
            reason: crate::compiler_frontend::compiler_messages::InvalidTemplateDirectiveReason::EmptyArguments,
            ..
        }
    ));
}

#[test]
fn optional_slot_target_missing_close_paren_errors() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$slot(\"style\"]", &mut string_table);
    let directive_name = string_table.intern("slot");
    let result = parse_optional_slot_target_argument(directive_name, &mut tokens, &string_table);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err().payload,
        DiagnosticPayload::ExpectedToken {
            expected: TokenKind::CloseParenthesis,
            ..
        }
    ));
}

// ------------------------------------------------------------------------
// parse_required_slot_name_argument
// ------------------------------------------------------------------------

#[test]
fn required_slot_name_missing_parens_errors() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$insert]", &mut string_table);
    let directive_name = string_table.intern("insert");
    let result = parse_required_slot_name_argument(directive_name, &mut tokens);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err().payload,
        DiagnosticPayload::ExpectedToken {
            expected: TokenKind::OpenParenthesis,
            ..
        }
    ));
}

#[test]
fn required_slot_name_string_literal_ok() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$insert(\"style\")]", &mut string_table);
    let directive_name = string_table.intern("insert");
    let result = parse_required_slot_name_argument(directive_name, &mut tokens);
    assert!(result.is_ok());
}

#[test]
fn required_slot_name_positional_rejected() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$insert(1)]", &mut string_table);
    let directive_name = string_table.intern("insert");
    let result = parse_required_slot_name_argument(directive_name, &mut tokens);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err().payload,
        DiagnosticPayload::InvalidTemplateDirective {
            reason: crate::compiler_frontend::compiler_messages::InvalidTemplateDirectiveReason::InvalidInsertTarget,
            ..
        }
    ));
}

#[test]
fn required_slot_name_empty_parens_errors() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$insert()]", &mut string_table);
    let directive_name = string_table.intern("insert");
    let result = parse_required_slot_name_argument(directive_name, &mut tokens);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err().payload,
        DiagnosticPayload::InvalidTemplateDirective {
            reason: crate::compiler_frontend::compiler_messages::InvalidTemplateDirectiveReason::EmptyArguments,
            ..
        }
    ));
}

#[test]
fn optional_string_literal_not_a_string_errors() {
    // After the $code refactor into the HTML project builder, argument type
    // validation lives in normalize_provided_style_argument_value (called by
    // apply_handler_style_directive), not in parse_optional_parenthesized_expression.
    let error = template_parse_error("[$code(42): body]");
    assert!(!error.is_empty());
}

// ------------------------------------------------------------------------
// parse_optional_parenthesized_expression
// ------------------------------------------------------------------------

#[test]
fn optional_expression_no_parens_returns_none() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$css]", &mut string_table);
    let context = test_context(tokens.src_path.to_owned());
    let result =
        parse_optional_parenthesized_expression_for_test(&mut tokens, &context, &mut string_table);
    assert!(matches!(result, Ok(None)));
}

#[test]
fn optional_expression_with_parens_returns_some() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$code(\"wrap\")]", &mut string_table);
    let context = test_context(tokens.src_path.to_owned());
    let result =
        parse_optional_parenthesized_expression_for_test(&mut tokens, &context, &mut string_table);
    assert!(matches!(result, Ok(Some(_))));
}

#[test]
fn optional_expression_empty_parens_errors() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$code()]", &mut string_table);
    let context = test_context(tokens.src_path.to_owned());
    let result =
        parse_optional_parenthesized_expression_for_test(&mut tokens, &context, &mut string_table);
    assert!(result.is_err());
}

#[test]
fn optional_expression_whitespace_only_parens_use_generic_empty_arguments() {
    // A whitespace-only argument stays on the generic empty-arguments owner
    // rather than the `$children`-specific reason, proving non-`$children`
    // directives do not receive children wording.
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$code(\n)]", &mut string_table);
    let context = test_context(tokens.src_path.to_owned());
    let result =
        parse_optional_parenthesized_expression_for_test(&mut tokens, &context, &mut string_table);
    let diagnostic = *result.expect_err("whitespace-only directive argument should error");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateDirective {
            reason: crate::compiler_frontend::compiler_messages::InvalidTemplateDirectiveReason::EmptyArguments,
            ..
        }
    ));
}

#[test]
fn optional_expression_extra_comma_errors() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$children(\"a\", \"b\")]", &mut string_table);
    let context = test_context(tokens.src_path.to_owned());
    let result =
        parse_optional_parenthesized_expression_for_test(&mut tokens, &context, &mut string_table);
    assert!(result.is_err());
    let diagnostic = *result.unwrap_err();
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedToken {
            found: TokenKind::Comma,
        }
    ));
}

// ------------------------------------------------------------------------
// parse_required_parenthesized_expression
// ------------------------------------------------------------------------

#[test]
fn required_expression_missing_parens_errors() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$children]", &mut string_table);
    let context = test_context(tokens.src_path.to_owned());
    let result =
        parse_required_parenthesized_expression_for_test(&mut tokens, &context, &mut string_table);
    assert!(result.is_err());
    let diagnostic = *result.unwrap_err();
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::ExpectedToken {
            expected: TokenKind::OpenParenthesis,
            ..
        }
    ));
}

#[test]
fn required_expression_compile_time_constant_ok() {
    let mut string_table = StringTable::new();
    let mut tokens = directive_tokens("[$children(\"wrap\")]", &mut string_table);
    let context = test_context(tokens.src_path.to_owned());
    let result =
        parse_required_parenthesized_expression_for_test(&mut tokens, &context, &mut string_table);
    assert!(result.is_ok());
    let expr = result.unwrap();
    assert!(matches!(expr.kind, ExpressionKind::StringSlice(_)));
}

#[test]
fn note_and_todo_templates_do_not_render_content() {
    let note_rendered = folded_template_output("[:before[$note:ignored]after]");
    let todo_rendered = folded_template_output("[:before[$todo:ignored]after]");

    assert_eq!(note_rendered, "beforeafter");
    assert_eq!(todo_rendered, "beforeafter");
}

#[test]
fn note_and_todo_directives_reject_arguments() {
    let note_error = template_parse_error("[$note(\"x\"): ignored]");
    let todo_error = template_parse_error("[$todo(\"x\"): ignored]");

    assert!(!note_error.is_empty());
    assert!(!todo_error.is_empty());
}

#[test]
fn slot_directive_must_be_alone_in_template_head() {
    let before_error = template_parse_error("[\"prefix\", $slot]");
    let after_error = template_parse_error("[$slot, \"suffix\"]");

    assert!(before_error.contains("incompatible"));
    assert!(after_error.contains("incompatible"));
}

#[test]
fn comment_directives_must_be_alone_in_template_head() {
    for directive in ["note", "todo", "doc"] {
        let before = format!("[\"prefix\", ${directive}]");
        let after = format!("[${directive}, \"suffix\"]");
        let before_error = template_parse_error(&before);
        let after_error = template_parse_error(&after);

        assert!(
            before_error.contains("incompatible"),
            "expected mixed-head compatibility error for '{before}', got: {before_error}"
        );
        assert!(
            after_error.contains("incompatible"),
            "expected mixed-head compatibility error for '{after}', got: {after_error}"
        );
    }
}

#[test]
fn insert_directive_can_coexist_with_other_meaningful_head_items() {
    let mut string_table = StringTable::new();
    let mut token_stream =
        template_tokens_from_source("[\"prefix\", $insert(\"style\"): body]", &mut string_table);
    let context = new_constant_context(token_stream.src_path.to_owned());

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("insert directive should coexist with other meaningful head items");

    assert!(matches!(
        effective_tir_kind(&template, &context),
        TemplateType::SlotInsert(_)
    ));
}

#[test]
fn duplicate_insert_directives_are_rejected_in_one_head() {
    let error = template_parse_error("[$insert(\"a\"), $insert(\"b\"):]");
    assert!(error.contains("incompatible"));
}

#[test]
fn formatter_directives_are_exclusive_per_template_head() {
    let error = template_parse_error("[$md, $raw: body]");
    assert!(error.contains("incompatible"));
}

#[test]
fn project_owned_formatter_directives_are_exclusive_per_template_head() {
    let style_directives = html_project_test_style_directives();
    let error =
        template_parse_error_with_style_directives("[$html, $css: body]", &style_directives);
    assert!(error.contains("incompatible"));
}

#[test]
fn non_formatter_and_formatter_directives_can_coexist() {
    let mut string_table = StringTable::new();
    let mut token_stream = template_tokens_from_source("[1, $md:\n# Hello\n]", &mut string_table);
    let context = new_constant_context(token_stream.src_path.to_owned());
    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("non-formatter and formatter directives should coexist in the same head");

    assert_eq!(effective_tir_style(&template, &context).id, "markdown");
}

#[test]
fn doc_templates_treat_brackets_as_literal_text() {
    let mut string_table = StringTable::new();
    let mut token_stream = template_tokens_from_source("[$doc:\n[value]\n]", &mut string_table);
    let context = runtime_template_context(&token_stream.src_path, &mut string_table);

    // With suppress_child_templates, brackets are balanced literal text,
    // not nested child templates. Parsing succeeds even with a runtime context.
    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("doc template should parse brackets as literal text");

    let folded = fold_template_in_context(&template, &context, &mut string_table);
    let result = string_table.resolve(folded);
    // Each bracket and the symbol become separate atoms, each markdown-formatted
    // as individual paragraphs. The key assertion is that all three appear in the output.
    assert!(
        result.contains("["),
        "Opening bracket should appear as literal text: {result}"
    );
    assert!(
        result.contains("value"),
        "Inner symbol should appear as literal text: {result}"
    );
    assert!(
        result.contains("]"),
        "Closing bracket should appear as literal text: {result}"
    );
}

#[test]
fn doc_templates_are_markdown_formatted_by_default() {
    let mut string_table = StringTable::new();
    let mut token_stream = template_tokens_from_source("[$doc:\n# Heading\n]", &mut string_table);
    let context = new_constant_context(token_stream.src_path.to_owned());

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("doc template should parse");
    assert!(matches!(
        effective_tir_kind(&template, &context),
        TemplateType::Comment(CommentDirectiveKind::Doc)
    ));

    let folded = fold_template_in_context(&template, &context, &mut string_table);
    assert!(string_table.resolve(folded).contains("<h1>Heading</h1>"));
}

#[test]
fn doc_brackets_remain_literal_text() {
    let mut string_table = StringTable::new();
    let mut token_stream = template_tokens_from_source("[$doc:\n[: child]\n]", &mut string_table);
    let context = new_constant_context(token_stream.src_path.to_owned());

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("doc template should parse");

    let folded = fold_template_in_context(&template, &context, &mut string_table);
    let result = string_table.resolve(folded);
    // Each bracket, colon, and body text become separate atoms, markdown-formatted individually.
    assert!(
        result.contains("["),
        "Opening bracket should appear as literal text: {result}"
    );
    assert!(
        result.contains("child"),
        "Body text should appear as literal text: {result}"
    );
    assert!(
        result.contains("]"),
        "Closing bracket should appear as literal text: {result}"
    );
}

#[test]
fn css_without_argument_uses_css_formatter() {
    let style_directives = html_project_test_style_directives();
    let mut string_table = StringTable::new();
    let mut token_stream = template_tokens_from_source_with_style_directives(
        "[$css:\n.button { color: red; }\n]",
        &style_directives,
        &mut string_table,
    );
    let context = new_constant_context_with_style_directives(
        token_stream.src_path.to_owned(),
        &style_directives,
    );
    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("css template should parse");

    let effective_style = effective_tir_style(&template, &context);
    assert_eq!(effective_style.id, "css");
    assert!(effective_style.formatter.is_some());
}

#[test]
fn css_inline_argument_parses_correctly() {
    let style_directives = html_project_test_style_directives();
    let mut string_table = StringTable::new();
    let mut token_stream = template_tokens_from_source_with_style_directives(
        "[$css(\"inline\"):\ncolor: blue;\n]",
        &style_directives,
        &mut string_table,
    );
    let context = new_constant_context_with_style_directives(
        token_stream.src_path.to_owned(),
        &style_directives,
    );
    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("inline css template should parse");

    let effective_style = effective_tir_style(&template, &context);
    assert_eq!(effective_style.id, "css");
    assert!(effective_style.formatter.is_some());
}

#[test]
fn css_inline_argument_must_be_quoted_string_literal() {
    let style_directives = html_project_test_style_directives();
    let error = template_parse_error_with_style_directives(
        "[$css(inline): color: blue;]",
        &style_directives,
    );
    assert!(
        error.contains("compile-time string argument")
            || error.contains("not declared")
            || error.contains("Undefined variable")
            || error.contains("Unknown value name")
            || error.contains("Unknown Value name"),
        "unexpected error message: {error}"
    );
}

#[test]
fn css_rejects_unknown_arguments() {
    let style_directives = html_project_test_style_directives();
    let error = template_parse_error_with_style_directives(
        "[$css(\"scoped\"): color: blue;]",
        &style_directives,
    );
    assert!(!error.is_empty());
}

#[test]
fn const_css_template_emits_malformed_css_warnings() {
    let style_directives = html_project_test_style_directives();
    let warnings = template_warnings_with_style_directives(
        "[$css:\n.button { color red; }\n]",
        false,
        &style_directives,
    );

    assert!(!warnings.is_empty());
    assert!(warnings.iter().all(|warning| {
        matches!(
            warning.kind,
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MalformedCssTemplate)
        )
    }));
    assert!(warnings.iter().any(|warning| matches!(
        warning.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::MalformedCssTemplate)
    )));
}

#[test]
fn css_validation_warnings_keep_non_default_locations() {
    let style_directives = html_project_test_style_directives();
    let warnings = template_warnings_with_style_directives(
        "[$css:\n.button { color red; }\n.button { color blue }\n]",
        false,
        &style_directives,
    );

    assert!(!warnings.is_empty());
    assert!(
        warnings
            .iter()
            .all(|warning| !is_default_error_location(&warning.primary_location)),
        "css warnings should keep meaningful source locations"
    );
}

#[test]
fn inline_css_warns_when_blocks_are_used() {
    let style_directives = html_project_test_style_directives();
    let warnings = template_warnings_with_style_directives(
        "[$css(\"inline\"):\n.button { color: red; }\n]",
        false,
        &style_directives,
    );

    assert!(warnings.iter().any(|warning| matches!(
        warning.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::MalformedCssTemplate)
    )));
}

#[test]
fn runtime_css_templates_emit_warnings_for_static_body_segments() {
    let style_directives = html_project_test_style_directives();
    let warnings = template_warnings_with_style_directives(
        "[value, $css:\n.button { color red; }\n]",
        true,
        &style_directives,
    );
    assert!(!warnings.is_empty());
    assert!(warnings.iter().all(|warning| matches!(
        warning.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::MalformedCssTemplate)
    )));
}

#[test]
fn code_without_argument_uses_generic_highlighting() {
    let rendered = folded_template_output("[$code:\nloop(x + 1)\n]");

    assert!(rendered.contains("<code class='codeblock'>"));
    assert!(rendered.contains("<span class='moth-code-parenthesis'>(</span>"));
    assert!(!rendered.contains("moth-code-keyword"));
}

#[test]
fn code_moth_argument_highlights_moth_rules() {
    let rendered = folded_template_output("[$code(\"moth\"):\nloop x\n-- hi\n]");

    assert!(rendered.contains("<span class='moth-code-keyword'>loop</span>"));
    assert!(rendered.contains("<span class='moth-code-comment'>-- hi</span>"));
}

#[test]
fn code_javascript_argument_highlights_js_comments() {
    let rendered = folded_template_output("[$code(\"js\"):\nconst x = 1\n// hi\n]");

    assert!(rendered.contains("<span class='moth-code-keyword'>const</span>"));
    assert!(rendered.contains("<span class='moth-code-comment'>// hi</span>"));
}

#[test]
fn code_python_argument_highlights_python_comments() {
    let rendered = folded_template_output("[$code(\"py\"):\ndef run():\n# hi\n]");

    assert!(rendered.contains("<span class='moth-code-keyword'>def</span>"));
    assert!(rendered.contains("<span class='moth-code-comment'># hi</span>"));
}

#[test]
fn code_typescript_argument_highlights_typescript_types() {
    let rendered = folded_template_output("[$code(\"ts\"):\ntype Name = string\n]");

    assert!(rendered.contains("<span class='moth-code-keyword'>type</span>"));
    assert!(rendered.contains("<span class='moth-code-type'>string</span>"));
}

#[test]
fn code_empty_parentheses_error_cleanly() {
    let error = template_parse_error("[$code(): body]");

    assert!(
        error.contains("empty parentheses") || error.contains("EmptyArguments"),
        "unexpected error message: {error}"
    );
}

#[test]
fn code_rejects_unknown_language_aliases() {
    let error = template_parse_error("[$code(\"unknown\"): body]");

    assert!(!error.is_empty());
}

#[test]
fn code_rejects_multiple_language_arguments() {
    let error = template_parse_error("[$code(\"moth\", \"js\"): body]");

    assert!(
        error.contains("Unexpected token `,`."),
        "unexpected error message: {error}"
    );
}

#[test]
fn runtime_templates_with_code_format_only_static_body_strings() {
    let mut string_table = StringTable::new();
    let mut token_stream =
        template_tokens_from_source("[value, $code(\"moth\"):\nloop x\n]", &mut string_table);
    let context = runtime_template_context(&token_stream.src_path, &mut string_table);

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
        .expect("template should parse");

    let store = context.template_ir_store.borrow();
    assert!(tir_root_has_head_dynamic_expression(
        &template,
        &store,
        |expression| matches!(
            expression.kind,
            ExpressionKind::Reference(_) | ExpressionKind::StringSlice(_)
        )
    ));

    // Formatted body text is authoritative in the TIR root.
    let body_texts = collect_body_text_from_tir(&template, &store, &string_table);
    assert!(
        body_texts
            .iter()
            .any(|text| text.contains("<code class='codeblock'>")),
        "expected code HTML block in formatted body text"
    );
    assert!(
        body_texts
            .iter()
            .any(|text| text.contains("<span class='moth-code-keyword'>loop</span>")),
        "expected highlighted keyword span in formatted body text"
    );
}

#[test]
fn code_templates_keep_nested_square_brackets_as_literal_body_text() {
    let rendered = folded_template_output(
        "[$code(\"moth\"):\nconcatenated_strings = [string_slice, a_mutable_string]\n]",
    );

    assert!(rendered.contains("<code class='codeblock'>"));
    assert!(rendered.contains("concatenated_strings"));
    assert!(rendered.contains("string_slice"));
    assert!(rendered.contains("a_mutable_string"));
    assert!(rendered.contains("<span class='moth-code-parenthesis'>[</span>"));
    assert!(rendered.contains("<span class='moth-code-parenthesis'>]</span>"));
}
