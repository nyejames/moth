use super::*;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

#[test]
fn template_head_unknown_symbol_reports_unknown_value_name_not_unexpected_token() {
    // Unknown names in a template head should produce a structured UnknownName
    // diagnostic, not a generic UnexpectedToken. This is the improvement from
    // routing symbol-led head items through the ordinary expression parser.
    let source = "[unknown_name]";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens =
        template_tokens_from_source(source, &mut string_table, &mut span_builder, &mut path_fork);
    let source_path = file_tokens.source_path;
    let context = runtime_template_context(&source_path, &mut string_table, &mut path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");

    let diagnostic = Template::new(
        &mut token_stream,
        source_path,
        &context,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect_err("unknown name in template head should fail");
    let diagnostic = expect_template_diagnostic(diagnostic);

    let unknown_name = string_table.intern("unknown_name");
    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::UnknownName {
                name,
                namespace: NameNamespace::Value,
            } if name == unknown_name
        ),
        "expected UnknownName for unknown symbol in template head, got: {:?}",
        diagnostic.payload
    );

    let primary_span = diagnostic
        .primary_span
        .expect("expression parser diagnostics should retain the authored symbol span");
    assert_eq!(primary_span.source(), SourceId::COMPILATION_ROOT);
    let range = primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!((range.start(), range.end()), (1, 13));
    assert_eq!(
        &source[range.start() as usize..range.end() as usize],
        "unknown_name"
    );
}

#[test]
fn incompatible_head_item_retains_exact_extended_multibyte_span() {
    let long_directive_name = format!("formatter_{}", "x".repeat(1_020));
    let long_directive = StyleDirectiveSpec::handler(
        long_directive_name.clone(),
        TemplateBodyMode::Normal,
        TemplateHeadCompatibility::blocks_same(TemplateHeadTag::FORMATTER_DIRECTIVE),
        StyleDirectiveHandlerSpec::no_op(),
    );
    let style_directives = StyleDirectiveRegistry::merged(&[long_directive])
        .expect("the test formatter directive should merge");
    let source = format!("// π\n[$raw, ${long_directive_name}: body]");
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = template_tokens_from_source_with_style_directives(
        &source,
        &style_directives,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context =
        new_constant_context_with_style_directives(source_path, &style_directives, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");

    let diagnostic = expect_template_diagnostic(
        Template::new(
            &mut token_stream,
            source_path,
            &context,
            vec![],
            &mut string_table,
            &mut path_fork,
        )
        .expect_err("incompatible formatter directives should fail"),
    );
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::IncompatibleHeadItem
        }
    ));

    let primary_span = diagnostic
        .primary_span
        .expect("incompatible head item should retain its exact source span");
    assert_eq!(primary_span.source(), SourceId::COMPILATION_ROOT);
    let range = primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    let target = format!("${long_directive_name}");
    let expected_start = source
        .find(&target)
        .expect("the long directive should occur in the source") as u32;
    assert!(target.len() > 1_022, "the target must use an extended span");
    assert_eq!(
        (range.start(), range.end()),
        (expected_start, expected_start + target.len() as u32)
    );
    assert_eq!(
        &source[range.start() as usize..range.end() as usize],
        target
    );
}

#[test]
fn template_head_expression_preserves_infrastructure_failure() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = template_tokens_from_source(
        "[stale_template]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let scope = file_tokens.source_path;
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");
    let stale_name = string_table.intern("stale_template");
    let stale_template = Template {
        tir_reference: TemplateTirReference {
            root: TemplateIrId::new(99),
            phase: TemplateTirPhase::Parsed,
            context: TemplateViewContext::default(),
        },
        span: None,
    };
    let declaration = Declaration {
        id: path_fork
            .try_intern_child(scope, stale_name)
            .expect("stale declaration path should intern"),
        value: Expression::template(stale_template, ValueMode::ImmutableOwned),
        binding_span: None,
        config_qualifier: None,
    };
    let style_directives = frontend_test_style_directives();
    let context = with_test_path_context(
        ScopeContext::new_for_tests(
            ContextKind::Template,
            scope,
            Rc::new(TopLevelDeclarationTable::new(vec![declaration], &path_fork)),
            Arc::new(ExternalPackageRegistry::default()),
            vec![],
            0,
        ),
        &scope,
        &style_directives,
    );
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let mut build_state = TemplateBuildState::new();
    let mut construction_context = TemplateConstructionContext::new(
        context.template_ir_store.clone(),
        Some(token_stream.current_span()),
    );

    let error = match parse_template_head(
        &mut token_stream,
        TemplateHeadParseRequest {
            context: &context,
            type_interner: &mut type_interner,
            build_state: &mut build_state,
            construction_context: &mut construction_context,
            control_flow_validation: TemplateControlFlowValidationMode::RuntimeCapable,
            string_table: &mut string_table,
            path_fork: &mut path_fork,
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("stale template authority must fail during head expression parsing"),
    };

    let TemplateError::Infrastructure(error) = error else {
        panic!("stale template authority must remain an infrastructure failure");
    };
    assert!(
        error.msg.contains("missing same-store template"),
        "unexpected infrastructure error: {error:?}"
    );
}

#[test]
fn template_head_path_lookup_preserves_infrastructure_failure() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = template_tokens_from_source(
        "[@core/math]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let opener_index = file_tokens.opener_index;
    let source_path = file_tokens.source_path;
    let mut canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    drop(file_tokens);
    let path_token_index = canonical_owner
        .shapes()
        .iter()
        .position(|shape| shape.tag() == TokenTag::PATH)
        .expect("expected a path token in the template head");
    Arc::get_mut(&mut canonical_owner)
        .expect("canonical owner should be uniquely mutable before cursor construction")
        .corrupt_payload_for_test(path_token_index, TokenTag::PATH);
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(opener_index)
        .expect("template opener position must remain in the canonical range");
    let scope = source_path;
    let style_directives = frontend_test_style_directives();
    let context = with_test_path_context(
        runtime_template_context(&scope, &mut string_table, &mut path_fork),
        &scope,
        &style_directives,
    );
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let mut build_state = TemplateBuildState::new();
    let mut construction_context = TemplateConstructionContext::new(
        context.template_ir_store.clone(),
        Some(token_stream.current_span()),
    );

    let error = match parse_template_head(
        &mut token_stream,
        TemplateHeadParseRequest {
            context: &context,
            type_interner: &mut type_interner,
            build_state: &mut build_state,
            construction_context: &mut construction_context,
            control_flow_validation: TemplateControlFlowValidationMode::RuntimeCapable,
            string_table: &mut string_table,
            path_fork: &mut path_fork,
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("a tampered template-head path handle must fail"),
    };

    let TemplateError::Infrastructure(error) = error else {
        panic!("template-head path corruption must remain an infrastructure failure");
    };
    assert!(
        error.msg.contains("absent PathSyntaxId marker")
            || error.msg.contains("no payload")
            || error
                .msg
                .contains("does not belong to the consumed path token"),
        "unexpected infrastructure error: {error:?}"
    );
}

#[test]
fn template_head_content_path_uses_stage0_resolution_without_project_resolver() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = template_tokens_from_source(
        "[@docs/intro.mtf]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let path_syntax = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens")
        .shapes()
        .iter()
        .find_map(|shape| shape.path_syntax_id())
        .expect("expected a content path token in the template head");
    let source_path = file_tokens.source_path;

    let target_path = PathBuf::from("intro.mtf");
    let source_files = SourceDatabase::build(
        std::iter::once(&target_path),
        &target_path,
        None,
        &mut string_table,
    )
    .expect("content source identity should build without a project resolver");
    let source_slot = source_files
        .get_by_canonical_path(&target_path)
        .expect("content source identity should be present");
    let source_file = source_slot.id;
    let content_logical_path = source_slot.logical_path;
    let content_path =
        content_constant_path(content_logical_path, &mut path_fork, &mut string_table)
            .expect("content constant path should intern");
    let content_string = string_table.intern("resolved head content");
    let content_declaration = Declaration {
        id: content_path,
        value: Expression::string_slice(content_string, None, ValueMode::ImmutableOwned),
        binding_span: None,
        config_qualifier: None,
    };

    let mut resolved_references = ResolvedFileReferenceTable::new();
    resolved_references
        .push(ResolvedFileReference {
            source_file,
            path_syntax,
            class: PreparedFileReferenceClass::ContentSource,
            outcome: ResolvedFileReferenceOutcome::Target(
                ResolvedFileReferenceTarget::ContentSource {
                    source: source_file,
                },
            ),
        })
        .expect("content resolved row should be unique");

    let style_directives = frontend_test_style_directives();
    let context = ScopeContext::new_for_tests(
        ContextKind::Constant,
        source_path,
        Rc::new(TopLevelDeclarationTable::new(
            vec![content_declaration],
            &path_fork,
        )),
        Arc::new(ExternalPackageRegistry::default()),
        vec![],
        0,
    )
    .with_style_directives(&style_directives)
    .with_source_file_scope(source_path)
    .with_file_value_resolution(Rc::new(FileValueResolutionServices {
        stage0_resolution_facts: Some(Arc::new(Stage0ResolutionFacts::ordinary(
            resolved_references,
            source_files.into(),
        ))),
        module_resources: Rc::new(RefCell::new(ModuleResourceTable::new())),
        module_origin: None,
        frozen_identity_handle: FrozenIdentityHandle::new(),
    }))
    .with_declaring_file_id(source_file);

    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");
    let template = Template::new_const_required(
        &mut token_stream,
        source_path,
        &context,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("content path in a template head should resolve from Stage 0")
    .template;
    let folded = fold_template_in_context(&template, &context, &mut string_table);

    assert_eq!(
        folded, content_string,
        "template-head content should reuse the synthetic content constant value"
    );
}

#[test]
fn template_head_extensionless_path_retains_exact_span() {
    let source = "// π\n[@docs/intro]";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens =
        template_tokens_from_source(source, &mut string_table, &mut span_builder, &mut path_fork);
    let path_syntax = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens")
        .shapes()
        .iter()
        .find_map(|shape| shape.path_syntax_id())
        .expect("expected an extensionless path token in the template head");
    let source_path = file_tokens.source_path;

    let target_path = PathBuf::from("intro.mtf");
    let source_files = SourceDatabase::build(
        std::iter::once(&target_path),
        &target_path,
        None,
        &mut string_table,
    )
    .expect("path diagnostic source identity should build");
    let source_file = source_files
        .get_by_canonical_path(&target_path)
        .expect("path diagnostic source identity should be present")
        .id;

    let mut resolved_references = ResolvedFileReferenceTable::new();
    resolved_references
        .push(ResolvedFileReference {
            source_file,
            path_syntax,
            class: PreparedFileReferenceClass::Extensionless,
            outcome: ResolvedFileReferenceOutcome::NoPhysicalTarget,
        })
        .expect("extensionless resolved row should be unique");

    let style_directives = frontend_test_style_directives();
    let context = ScopeContext::new_for_tests(
        ContextKind::Constant,
        source_path,
        Rc::new(TopLevelDeclarationTable::new(vec![], &path_fork)),
        Arc::new(ExternalPackageRegistry::default()),
        vec![],
        0,
    )
    .with_style_directives(&style_directives)
    .with_source_file_scope(source_path)
    .with_file_value_resolution(Rc::new(FileValueResolutionServices {
        stage0_resolution_facts: Some(Arc::new(Stage0ResolutionFacts::ordinary(
            resolved_references,
            source_files.into(),
        ))),
        module_resources: Rc::new(RefCell::new(ModuleResourceTable::new())),
        module_origin: None,
        frozen_identity_handle: FrozenIdentityHandle::new(),
    }))
    .with_declaring_file_id(source_file);

    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");
    let diagnostic = expect_template_diagnostic(
        Template::new(
            &mut token_stream,
            source_path,
            &context,
            vec![],
            &mut string_table,
            &mut path_fork,
        )
        .expect_err("an extensionless template-head path should fail"),
    );
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidExpression {
            reason: InvalidExpressionReason::ExtensionlessFileValue
        }
    ));

    let primary_span = diagnostic
        .primary_span
        .expect("path-value diagnostics should retain the authored path span");
    assert_eq!(primary_span.source(), SourceId::COMPILATION_ROOT);
    let range = primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    let target = "@docs/intro";
    let expected_start = source
        .find(target)
        .expect("the path should occur in the source") as u32;
    assert_eq!(
        (range.start(), range.end()),
        (expected_start, expected_start + target.len() as u32)
    );
    assert_eq!(
        &source[range.start() as usize..range.end() as usize],
        target
    );
}

#[test]
fn single_item_template_head_with_close_is_foldable() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = path_fork
        .try_intern_portable_path("main.moth/#const_template0", &mut string_table)
        .expect("test path fits");
    let context = new_constant_context(scope.to_owned(), &path_fork);

    let mut builder = TestSourceTokensBuilder::new(SourceId::COMPILATION_ROOT);
    builder
        .push_static(TokenTag::TEMPLATE_HEAD, LocalSpan::source_start())
        .expect("template-head fixture token should be valid");
    builder
        .push_numeric(
            NumericLiteralToken::test_new("3", &mut string_table),
            LocalSpan::source_start(),
        )
        .expect("numeric fixture token should be valid");
    builder
        .push_static(TokenTag::TEMPLATE_CLOSE, LocalSpan::source_start())
        .expect("template-close fixture token should be valid");
    builder
        .push_static(TokenTag::EOF, LocalSpan::source_start())
        .expect("EOF fixture token should be valid");
    let file_tokens = TemplateSourceFixture {
        source_owner: SourceTokenOwner::new(
            builder.finish().expect("canonical fixture should finish"),
        ),
        source_path: scope,
        opener_index: 0,
    };
    let source_path = file_tokens.source_path;
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");

    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("single-item head template should parse");

    assert!(matches!(
        effective_tir_kind(&template, &context),
        TemplateType::String
    ));
    let folded = fold_template_in_context(&template, &context, &mut string_table);
    assert_eq!(string_table.resolve(folded), "3");
}

#[test]
fn parsed_template_tir_reference_carries_empty_view_context() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = template_tokens_from_source(
        "[: body]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context = new_constant_context(source_path, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");

    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("template source should parse");
    let reference = &template.tir_reference;

    assert_eq!(reference.context, TemplateViewContext::default());
}

#[test]
fn template_control_flow_suffix_requires_comma_after_head_items() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = template_tokens_from_source(
        "[value if true: Visible]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context = runtime_template_context(&source_path, &mut string_table, &mut path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");

    let diagnostic = Template::new(
        &mut token_stream,
        source_path,
        &context,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect_err("missing comma before template control-flow suffix should fail");
    let diagnostic = expect_template_diagnostic(diagnostic);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::MissingCommaBeforeControlFlowSuffix
        }
    ));
}

#[test]
fn template_if_suffix_must_be_final() {
    let diagnostic = parse_template_error("[if true, $raw: Visible]");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::ControlFlowSuffixNotFinal
        }
    ));
}

#[test]
fn template_loop_suffix_must_be_final() {
    let diagnostic = parse_template_error("[loop 0 to 3 |i|, value: [i]]");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::ControlFlowSuffixNotFinal
        }
    ));
}

#[test]
fn template_if_suffix_requires_condition() {
    let diagnostic = parse_template_error("[if: Visible]");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::MissingTemplateIfCondition
        }
    ));
}

#[test]
fn template_if_suffix_separator_retains_exact_multibyte_span() {
    let source = "[if true -- π\n, value: Visible]";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens =
        template_tokens_from_source(source, &mut string_table, &mut span_builder, &mut path_fork);
    let source_path = file_tokens.source_path;
    let context = new_constant_context(source_path, &path_fork);
    let canonical_owner = file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut token_stream = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    token_stream
        .set_position(file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");

    let diagnostic = expect_template_diagnostic(
        Template::new(
            &mut token_stream,
            source_path,
            &context,
            vec![],
            &mut string_table,
            &mut path_fork,
        )
        .expect_err("a separator after an if suffix should fail"),
    );
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::ControlFlowSuffixNotFinal
        }
    ));

    let primary_span = diagnostic
        .primary_span
        .expect("the suffix separator should retain its exact source span");
    assert_eq!(primary_span.source(), SourceId::COMPILATION_ROOT);
    let range = primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    let expected_start = source
        .find(',')
        .expect("the fixture should contain a comma") as u32;
    assert_eq!(
        (range.start(), range.end()),
        (expected_start, expected_start + 1)
    );
    assert_eq!(&source[range.start() as usize..range.end() as usize], ",");
}

#[test]
fn template_loop_suffix_requires_header() {
    let diagnostic = parse_template_error("[loop: Visible]");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidTemplateStructure {
            reason: InvalidTemplateStructureReason::MissingTemplateLoopHeader
        }
    ));
}
