use super::*;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::{
    InvalidLoopHeaderCode, InvalidLoopHeaderView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::UnknownValueNameView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::InvalidExpressionView;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::DurableLabelMessage;
use crate::compiler_frontend::compiler_messages::{
    InvalidExpressionReason, InvalidTemplateStructureReason, TemplateHeadConflictOwner,
    TemplateHeadConflictReason, TemplateHeadItemKind,
};
use crate::compiler_frontend::numeric_text::token::NumericLiteralToken;
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

#[test]
fn template_head_unknown_symbol_reports_unknown_value_name_not_unexpected_token() {
    // Unknown names in a template head should produce a structured unknown-value-name
    // diagnostic, not a generic UnexpectedToken. This is the improvement from
    // routing symbol-led head items through the ordinary expression parser.
    let source = "[unknown_name]";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens =
        template_tokens_from_source(source, &mut string_table, &mut span_builder, &mut path_fork);
    let source_path = file_tokens.source_path;
    let context = runtime_template_context(
        &source_path,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    );
    let canonical_owner = file_tokens.canonical_owner();
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
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect_err("unknown name in template head should fail");
    let diagnostic = expect_template_diagnostic(diagnostic);
    let unknown_name = string_table.intern("unknown_name");

    let view = diagnostic
        .first()
        .expect("unknown-name diagnostic should compact to one record");
    assert_eq!(view.descriptor().code, "MOTH-RULE-0034");
    let facts = view
        .family_view::<UnknownValueNameView>()
        .expect("unknown-value-name diagnostic should retain typed facts");
    assert_eq!(facts.name, unknown_name);

    let SiteRef::Local(primary_span) = view.primary_site() else {
        panic!("expression parser diagnostic should retain its authored local symbol span");
    };
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
        TemplateHeadCompatibility::blocks_same(
            TemplateHeadTag::FORMATTER_DIRECTIVE,
            TemplateHeadConflictReason::MultipleFormatters,
        ),
        StyleDirectiveHandlerSpec::no_op(),
    );
    let style_directives = StyleDirectiveRegistry::merged(&[long_directive])
        .expect("the test formatter directive should merge");
    let source = format!("// π\n[$raw, ${long_directive_name}: body]");
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
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
    let canonical_owner = file_tokens.canonical_owner();
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
            &mut expression_builder,
            &mut const_values,
            vec![],
            &mut string_table,
            &mut path_fork,
        )
        .expect_err("incompatible formatter directives should fail"),
    );
    let view = diagnostic
        .first()
        .expect("incompatible head item should compact to one record");
    let facts = view
        .family_view::<InvalidTemplateStructureView>()
        .expect("incompatible head item should retain typed structure facts");
    assert_eq!(
        facts.reason,
        InvalidTemplateStructureCode::IncompatibleHeadItem
    );
    let conflict = facts
        .head_conflict
        .expect("incompatible head item should retain its typed conflict");
    assert_eq!(
        conflict.reason,
        TemplateHeadConflictReason::MultipleFormatters
    );
    assert_eq!(conflict.owner, TemplateHeadConflictOwner::EarlierItem);
    assert_eq!(conflict.incoming_kind, TemplateHeadItemKind::Directive);
    assert_eq!(conflict.earlier_kind, TemplateHeadItemKind::Directive);
    assert_eq!(
        view.string_ids(),
        [
            string_table.intern(&long_directive_name),
            string_table.intern("raw"),
        ]
        .as_slice()
    );
    let label = view
        .durable_labels()
        .next()
        .expect("the earlier conflicting item should carry a secondary label");
    assert!(matches!(
        label.message,
        DurableLabelMessage::EarlierTemplateHeadItem
    ));
    let SiteRef::Local(earlier_span) = label.site else {
        panic!("the earlier head item label should retain its local source span");
    };
    let earlier_range =
        earlier_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(
        &source[earlier_range.start() as usize..earlier_range.end() as usize],
        "$raw"
    );

    let SiteRef::Local(primary_span) = view.primary_site() else {
        panic!("incompatible head item should retain its local source span");
    };
    assert!(
        matches!(
            view.primary_label_message(),
            Some(DurableLabelMessage::ConflictsWithEarlierHeadItem)
        ),
        "the rejected head item should say why it was rejected",
    );
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
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source(
        "[stale_template]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let scope = file_tokens.source_path;
    let canonical_owner = file_tokens.canonical_owner();
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
    let stale_value = expression_builder
        .template(stale_template, ValueMode::ImmutableOwned)
        .expect("stale template expression should construct")
        .id();
    let declaration = make_test_variable(
        path_fork
            .try_intern_child(scope, stale_name)
            .expect("stale declaration path should intern"),
        stale_value,
        &expression_builder,
    );
    let style_directives = frontend_test_style_directives();
    let context = with_test_path_context(
        ScopeContext::new_for_tests(
            ContextKind::Template,
            scope,
            Rc::new(TopLevelDeclarationTable::new(vec![declaration], &path_fork)),
            Arc::new(ExternalPackageRegistry::default()),
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

    let opening_span = Some(token_stream.current_span());
    let error = match parse_template_head(
        &mut token_stream,
        TemplateHeadParseRequest {
            context: &context,
            type_interner: &mut type_interner,
            expression_builder: &mut expression_builder,
            const_values: &mut const_values,
            build_state: &mut build_state,
            construction_context: &mut construction_context,
            control_flow_validation: TemplateControlFlowValidationMode::RuntimeCapable,
            string_table: &mut string_table,
            path_fork: &mut path_fork,
            opening_span,
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("stale template authority must fail during head expression parsing"),
    };

    let TemplateError::Infrastructure(error) = error else {
        panic!("stale template authority must remain an infrastructure failure");
    };
    let error = &error.primary;
    assert!(
        error.message.contains("missing same-store template"),
        "unexpected infrastructure error: {error:?}"
    );
}

#[test]
fn template_head_path_lookup_preserves_infrastructure_failure() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source(
        "[@core/math]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let opener_index = file_tokens.opener_index;
    let source_path = file_tokens.source_path;
    let mut canonical_owner = file_tokens.canonical_owner();
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
        runtime_template_context(
            &scope,
            &mut expression_builder,
            &mut const_values,
            &mut string_table,
            &mut path_fork,
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

    let opening_span = Some(token_stream.current_span());
    let error = match parse_template_head(
        &mut token_stream,
        TemplateHeadParseRequest {
            context: &context,
            type_interner: &mut type_interner,
            expression_builder: &mut expression_builder,
            const_values: &mut const_values,
            build_state: &mut build_state,
            construction_context: &mut construction_context,
            control_flow_validation: TemplateControlFlowValidationMode::RuntimeCapable,
            string_table: &mut string_table,
            path_fork: &mut path_fork,
            opening_span,
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("a tampered template-head path handle must fail"),
    };

    let TemplateError::Infrastructure(error) = error else {
        panic!("template-head path corruption must remain an infrastructure failure");
    };
    let error = &error.primary;
    assert!(
        error.message.contains("absent PathSyntaxId marker")
            || error.message.contains("no payload")
            || error
                .message
                .contains("does not belong to the consumed path token"),
        "unexpected infrastructure error: {error:?}"
    );
}

#[test]
fn template_head_content_path_uses_stage0_resolution_without_project_resolver() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source(
        "[@docs/intro.mtf]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let path_syntax = file_tokens
        .canonical_owner()
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
    let content_value = expression_builder
        .string_slice(
            &mut const_values,
            content_string,
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("content string constant should construct")
        .id();
    let content_declaration = make_test_variable(content_path, content_value, &expression_builder);

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
        qualified_reference_targets: Arc::default(),
        module_origin: None,
        public_source_trait_origins: Arc::default(),
        frozen_identity_handle: FrozenIdentityHandle::new(),
    }))
    .with_declaring_file_id(source_file);

    let canonical_owner = file_tokens.canonical_owner();
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
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("content path in a template head should resolve from Stage 0")
    .template;
    let folded = fold_template_in_context(
        &template,
        &context,
        &mut string_table,
        &mut expression_builder,
        &mut const_values,
    );

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
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens =
        template_tokens_from_source(source, &mut string_table, &mut span_builder, &mut path_fork);
    let path_syntax = file_tokens
        .canonical_owner()
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
        qualified_reference_targets: Arc::default(),
        module_origin: None,
        public_source_trait_origins: Arc::default(),
        frozen_identity_handle: FrozenIdentityHandle::new(),
    }))
    .with_declaring_file_id(source_file);

    let canonical_owner = file_tokens.canonical_owner();
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
            &mut expression_builder,
            &mut const_values,
            vec![],
            &mut string_table,
            &mut path_fork,
        )
        .expect_err("an extensionless template-head path should fail"),
    );
    let view = diagnostic
        .first()
        .expect("extensionless path diagnostic should compact to one record");
    let facts = view
        .family_view::<InvalidExpressionView>()
        .expect("extensionless path diagnostic should retain typed expression facts");
    assert_eq!(
        facts.reason,
        InvalidExpressionReason::ExtensionlessFileValue
    );

    let SiteRef::Local(primary_span) = view.primary_site() else {
        panic!("path-value diagnostic should retain its authored local path span");
    };
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
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
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
    let canonical_owner = file_tokens.canonical_owner();
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
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("single-item head template should parse");

    assert!(matches!(
        effective_tir_kind(&template, &context),
        TemplateType::String
    ));
    let folded = fold_template_in_context(
        &template,
        &context,
        &mut string_table,
        &mut expression_builder,
        &mut const_values,
    );
    assert_eq!(string_table.resolve(folded), "3");
}

#[test]
fn parsed_template_tir_reference_carries_empty_view_context() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source(
        "[: body]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context = new_constant_context(source_path, &path_fork);
    let canonical_owner = file_tokens.canonical_owner();
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
        &mut expression_builder,
        &mut const_values,
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
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source(
        "[value if true: Visible]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context = runtime_template_context(
        &source_path,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    );
    let canonical_owner = file_tokens.canonical_owner();
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
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect_err("missing comma before template control-flow suffix should fail");
    let diagnostic = expect_template_diagnostic(diagnostic);

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::MissingCommaBeforeControlFlowSuffix,
    );
}

#[test]
fn template_if_suffix_must_be_final() {
    let diagnostic = parse_template_error("[if true, $raw: Visible]");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::ControlFlowSuffixNotFinal,
    );
}

#[test]
fn template_loop_suffix_must_be_final() {
    let diagnostic = parse_template_error("[loop 0 to 3 |i|, value: [i]]");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::ControlFlowSuffixNotFinal,
    );
}

#[test]
fn template_if_suffix_requires_condition() {
    let diagnostic = parse_template_error("[if: Visible]");

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::MissingTemplateIfCondition,
    );
}

#[test]
fn template_if_suffix_separator_retains_exact_multibyte_span() {
    let source = "[if true -- π\n, value: Visible]";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens =
        template_tokens_from_source(source, &mut string_table, &mut span_builder, &mut path_fork);
    let source_path = file_tokens.source_path;
    let context = new_constant_context(source_path, &path_fork);
    let canonical_owner = file_tokens.canonical_owner();
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
            &mut expression_builder,
            &mut const_values,
            vec![],
            &mut string_table,
            &mut path_fork,
        )
        .expect_err("a separator after an if suffix should fail"),
    );
    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::ControlFlowSuffixNotFinal,
    );
    let SiteRef::Local(primary_span) = diagnostic
        .first()
        .expect("suffix separator diagnostic should compact to one record")
        .primary_site()
    else {
        panic!("suffix separator diagnostic should retain its local source span");
    };
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

    assert_invalid_template_structure(
        &diagnostic,
        InvalidTemplateStructureReason::MissingTemplateLoopHeader,
    );
}

#[test]
fn template_conditional_loop_rejects_by_step() {
    let diagnostic = parse_template_error("[loop true by 2: Visible]");
    let view = diagnostic
        .first()
        .expect("the conditional-step diagnostic compacts to one record");
    assert_eq!(view.descriptor().code, "MOTH-SYNTAX-0029");
    let facts = view
        .family_view::<InvalidLoopHeaderView>()
        .expect("the loop header rejection keeps its typed reason");
    assert_eq!(facts.reason, InvalidLoopHeaderCode::StepOnConditionalLoop);
}

/// Parses one template with `style_directives` and returns the compacted failure, the string table
/// that interned names, and the span builder that owns the source spans.
fn parse_template_failure(
    source: &str,
    style_directives: &StyleDirectiveRegistry,
) -> (
    CompactDiagnosticBagForTest,
    StringTable,
    ExtendedSpanBuilder,
) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source_with_style_directives(
        source,
        style_directives,
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context =
        new_constant_context_with_style_directives(source_path, style_directives, &path_fork);
    let canonical_owner = file_tokens.canonical_owner();
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
            &mut expression_builder,
            &mut const_values,
            vec![],
            &mut string_table,
            &mut path_fork,
        )
        .expect_err("the template source should fail"),
    );
    (diagnostic, string_table, span_builder)
}

/// The message field of the terse record for one compacted diagnostic.
fn rendered_diagnostic_message(
    diagnostic: &CompactDiagnosticBagForTest,
    string_table: &StringTable,
) -> String {
    let view = diagnostic
        .first()
        .expect("template source diagnostic should compact to one record");
    terse::format_terse_diagnostic_view(view, DiagnosticRenderContext::new(string_table))
        .splitn(5, '|')
        .nth(4)
        .expect("terse rendering keeps the message field")
        .to_owned()
}

#[test]
fn template_head_formatter_conflict_keeps_the_reversed_order() {
    let style_directives = frontend_test_style_directives();
    let (diagnostic, mut string_table, _) =
        parse_template_failure("[$raw, $md: body]", &style_directives);
    let view = diagnostic
        .first()
        .expect("formatter conflict should compact to one record");
    assert_eq!(
        view.string_ids(),
        [string_table.intern("md"), string_table.intern("raw"),].as_slice()
    );
    assert_eq!(
        rendered_diagnostic_message(&diagnostic, &string_table),
        "`$raw` and `$md` cannot be combined because both control template body formatting. Choose one formatter."
    );
}

#[test]
fn duplicate_formatter_directive_names_the_repeated_directive() {
    let style_directives = frontend_test_style_directives();
    let (diagnostic, string_table, _) =
        parse_template_failure("[$md, $md: body]", &style_directives);

    assert_eq!(
        rendered_diagnostic_message(&diagnostic, &string_table),
        "`$md` cannot appear more than once in a template head because each formatter controls template body formatting. Choose one formatter."
    );
}

#[test]
fn repeated_comment_directive_uses_duplicate_wording() {
    let style_directives = frontend_test_style_directives();
    let (diagnostic, string_table, _) =
        parse_template_failure("[$note, $note: ignored]", &style_directives);
    let view = diagnostic
        .first()
        .expect("duplicate comment directive should compact to one record");
    let facts = view
        .family_view::<InvalidTemplateStructureView>()
        .expect("duplicate comment directive should retain typed structure facts");
    assert_eq!(
        facts
            .head_conflict
            .expect("duplicate note should retain its typed conflict")
            .reason,
        TemplateHeadConflictReason::DiscardedCommentDirectiveMustBeOnlyMeaningfulItem
    );
    assert_eq!(
        rendered_diagnostic_message(&diagnostic, &string_table),
        "`$note` cannot appear more than once in a template head."
    );
}

#[test]
fn repeated_slot_directive_uses_duplicate_wording() {
    let style_directives = frontend_test_style_directives();
    let (diagnostic, string_table, _) =
        parse_template_failure("[$slot, $slot: body]", &style_directives);
    let view = diagnostic
        .first()
        .expect("duplicate slot directive should compact to one record");
    let facts = view
        .family_view::<InvalidTemplateStructureView>()
        .expect("duplicate slot directive should retain typed structure facts");
    assert_eq!(
        facts
            .head_conflict
            .expect("duplicate slot should retain its typed conflict")
            .reason,
        TemplateHeadConflictReason::SlotDirectiveMustBeExclusive
    );
    assert_eq!(
        rendered_diagnostic_message(&diagnostic, &string_table),
        "`$slot` cannot appear more than once in a template head."
    );
}

#[test]
fn required_absent_conflict_labels_the_earlier_expression() {
    let style_directives = frontend_test_style_directives();
    let source = "[\"x\", $slot]";
    let (diagnostic, string_table, span_builder) =
        parse_template_failure(source, &style_directives);
    let view = diagnostic
        .first()
        .expect("slot exclusivity should compact to one record");
    let facts = view
        .family_view::<InvalidTemplateStructureView>()
        .expect("slot exclusivity should retain typed structure facts");
    let conflict = facts
        .head_conflict
        .expect("slot exclusivity should retain its typed conflict");
    assert_eq!(
        conflict.reason,
        TemplateHeadConflictReason::SlotDirectiveMustBeExclusive
    );
    assert_eq!(conflict.owner, TemplateHeadConflictOwner::IncomingItem);
    assert_eq!(conflict.incoming_kind, TemplateHeadItemKind::Directive);
    assert_eq!(conflict.earlier_kind, TemplateHeadItemKind::Expression);
    assert_eq!(
        rendered_diagnostic_message(&diagnostic, &string_table),
        "`$slot` cannot be combined with an expression because a slot declaration must be the template head's only meaningful item."
    );

    let mut labels = view.durable_labels();
    let earlier = labels
        .next()
        .expect("the earlier expression should carry a secondary label");
    assert!(matches!(
        earlier.message,
        DurableLabelMessage::EarlierTemplateHeadItem
    ));
    assert!(labels.next().is_none());

    let SiteRef::Local(earlier_span) = earlier.site else {
        panic!("the earlier expression label should retain its authored local span");
    };
    let range = earlier_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(
        &source[range.start() as usize..range.end() as usize],
        "\"x\""
    );
}

#[test]
fn compatible_head_items_still_parse_together() {
    assert_eq!(folded_template_output("[$fresh, \"x\"]"), "x");
}

/// Asserts one truncated template reports `MissingClosingBracket` at EOF and labels its opening.
fn assert_truncated_template(source: &str, expected_opening: (u32, u32)) {
    let style_directives = frontend_test_style_directives();
    let (diagnostic, string_table, span_builder) =
        parse_template_failure(source, &style_directives);
    let view = diagnostic
        .first()
        .expect("a truncated template should compact to one record");
    assert_eq!(view.descriptor().code, "MOTH-SYNTAX-0022");
    let facts = view
        .family_view::<InvalidTemplateStructureView>()
        .expect("a truncated template should retain typed structure facts");
    assert_eq!(
        facts.reason,
        InvalidTemplateStructureCode::MissingClosingBracket
    );
    assert_eq!(
        rendered_diagnostic_message(&diagnostic, &string_table),
        "This template is not closed before the end of the file. Add `]`."
    );

    let SiteRef::Local(primary_span) = view.primary_site() else {
        panic!("template truncation should retain its authored EOF span");
    };
    let primary_range =
        primary_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    let source_end = source.len() as u32;
    assert_eq!(
        (primary_range.start(), primary_range.end()),
        (source_end, source_end)
    );

    let labels: Vec<_> = view.durable_labels().collect();
    assert_eq!(labels.len(), 1, "one opening-bracket label is expected");
    assert!(matches!(
        labels[0].message,
        DurableLabelMessage::TemplateStartsHere
    ));
    let SiteRef::Local(opening_span) = labels[0].site else {
        panic!("the opening bracket label should retain its local span");
    };
    let opening_range =
        opening_span.resolve_with(span_builder.resolver_for(SourceId::COMPILATION_ROOT));
    assert_eq!(
        (opening_range.start(), opening_range.end()),
        expected_opening
    );
    assert_eq!(
        &source[opening_range.start() as usize..opening_range.end() as usize],
        "["
    );
}

#[test]
fn truncated_template_head_reports_missing_closing_bracket() {
    assert_truncated_template("[$md", (0, 1));
}

#[test]
fn truncated_template_body_reports_missing_closing_bracket() {
    assert_truncated_template("[: hello", (0, 1));
}

#[test]
fn unclosed_nested_template_labels_its_own_opening() {
    assert_truncated_template("[: a [: b", (5, 6));
}
