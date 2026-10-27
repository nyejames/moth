use super::*;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView,
};
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_with_trailing_newline_policy;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrStore, TemplatePreparationFacts, TemplatePreparationMode, TemplateTirPhase, TirView,
    prepare_tir_view,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::type_coercion::parse_context::CastTargetContext;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// Prepares facts from the template's effective TIR view.
///
/// WHY: builds the same view that const classification uses, then runs the
/// shared preparation entry point, so the assertions read production facts.
fn prepare_effective_template_facts(
    template: &Template,
    store: &Rc<RefCell<TemplateIrStore>>,
    expression_builder: &AstExpressionBuilder,
    const_values: &ConstValueStore,
) -> Result<TemplatePreparationFacts, TemplateError> {
    let reference = &template.tir_reference;
    let store = store.borrow();
    let view = TirView::with_minimum_phase(
        &store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Composed,
        reference.context,
    )?;
    Ok(prepare_tir_view(
        &view,
        expression_builder,
        const_values,
        TemplatePreparationMode::Value,
        None,
    )?
    .facts)
}

#[allow(
    clippy::too_many_arguments,
    reason = "test helper mirrors the parser's expected type, value mode, and closing-parenthesis policy"
)]
fn create_expression_for_test(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    expected_type: &mut ExpectedType,
    value_mode: &ValueMode,
    consume_closing_parenthesis: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ExprId, ExpressionParseError> {
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let mut cast_target_context = CastTargetContext::None;
    create_expression_with_trailing_newline_policy(ExpressionParseInput::ordinary(
        ExpressionParseResources {
            token_stream,
            scope_context: context,
            type_interner: &mut type_interner,
            expression_builder,
            const_values,
            expected_type,
            cast_target_context: &mut cast_target_context,
            value_mode,
            string_table,
            path_fork,
        },
        consume_closing_parenthesis,
    ))
    .map(|expression| expression.id())
}

#[test]
fn slot_wrappers_remain_compile_time_templates_until_filled() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source(
        "[: before [$slot] after]",
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
    .expect("wrapper template should parse");

    assert!(matches!(
        effective_tir_kind(&template, &context),
        TemplateType::String
    ));
    assert!(
        prepare_effective_template_facts(
            &template,
            &context.template_ir_store,
            &expression_builder,
            &const_values,
        )
        .expect("wrapper classification should succeed")
        .has_unresolved_slot_occurrences,
        "wrapper template should have unresolved slots"
    );
    let expression = expression_builder
        .template(template, ValueMode::ImmutableOwned)
        .expect("template expression should construct")
        .id();
    let view = expression_builder
        .view(expression)
        .expect("template expression belongs to builder");
    assert!(
        view.const_value_kind_with_template_classifier(&const_values, &mut |template| {
            classify_template_from_effective_tir(
                template,
                &context.template_ir_store,
                &expression_builder,
                &const_values,
                None,
            )
        })
        .expect("const classification should succeed")
        .is_compile_time_value(),
        "wrapper template with unfilled slots should be compile-time constant"
    );
}

#[test]
fn folding_nested_wrapper_constant_with_unfilled_named_slots_renders_empty_strings() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let scope = path_fork
        .try_intern_portable_path("main.moth/#const_template0", &mut string_table)
        .expect("test path fits");

    let wrapper_file_tokens = template_tokens_from_source(
        "[:<link rel=\"icon\" href=\"[$slot(\"favicon\")]\"><style>[$slot(\"css\")]</style>]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let wrapper_source_path = wrapper_file_tokens.source_path;
    let wrapper_context = new_constant_context(wrapper_source_path, &path_fork);
    let canonical_owner = wrapper_file_tokens.canonical_owner();
    let canonical_range = canonical_owner
        .full_range()
        .expect("test token stream must expose canonical source range");
    let mut wrapper_tokens = AstCursor::from_source_tokens(&canonical_owner, canonical_range)
        .expect("test token stream must expose an AST cursor");
    wrapper_tokens
        .set_position(wrapper_file_tokens.opener_index)
        .expect("test token stream position must remain in canonical range");
    let wrapper = Template::new(
        &mut wrapper_tokens,
        wrapper_source_path,
        &wrapper_context,
        &mut expression_builder,
        &mut const_values,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("wrapper template should parse");

    let wrapper_value = expression_builder
        .template(wrapper, ValueMode::ImmutableOwned)
        .expect("wrapper template expression should construct")
        .id();
    let declarations = vec![make_test_variable(
        path_fork
            .try_intern_child(scope, string_table.intern("header"))
            .expect("test path fits"),
        wrapper_value,
        &expression_builder,
    )];

    let file_tokens = template_tokens_from_source(
        "[header]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context = constant_template_context(&source_path, &declarations, &path_fork)
        .with_template_ir_store(wrapper_context.template_ir_store.clone());
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
    .expect("template using wrapper constant should parse");

    let folded = fold_template_in_context(
        &template,
        &context,
        &mut string_table,
        &mut expression_builder,
        &mut const_values,
    );
    let rendered = string_table.resolve(folded);

    assert!(rendered.contains("rel=\"icon\""));
    assert!(rendered.contains("href=\"\""));
    assert!(rendered.contains("<style></style>"));
    assert!(!rendered.contains("$slot("));
}

#[test]
fn wrapper_templates_with_runtime_references_are_not_compile_time_constants() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source(
        "[value: before [$slot] after]",
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
    .expect("runtime wrapper template should parse");

    assert!(matches!(
        effective_tir_kind(&template, &context),
        TemplateType::StringFunction
    ));
    assert!(
        prepare_effective_template_facts(
            &template,
            &context.template_ir_store,
            &expression_builder,
            &const_values,
        )
        .expect("runtime wrapper classification should succeed")
        .has_unresolved_slot_occurrences,
        "runtime wrapper template should have unresolved slots"
    );
    let expression = expression_builder
        .template(template, ValueMode::ImmutableOwned)
        .expect("template expression should construct")
        .id();
    let view = expression_builder
        .view(expression)
        .expect("template expression belongs to builder");
    assert!(
        !view
            .const_value_kind_with_template_classifier(&const_values, &mut |template| {
                classify_template_from_effective_tir(
                    template,
                    &context.template_ir_store,
                    &expression_builder,
                    &const_values,
                    None,
                )
            })
            .expect("const classification should succeed")
            .is_compile_time_value(),
        "runtime wrapper template should not be compile-time constant"
    );
}

#[test]
fn constant_context_template_head_with_constant_references_folds_to_string_slice() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let scope = path_fork
        .try_intern_portable_path("main.moth/#const_template0", &mut string_table)
        .expect("test path fits");
    let const_before = string_table.intern("const_before");
    let const_after = string_table.intern("const_after");
    let before_value = expression_builder
        .string_slice(
            &mut const_values,
            string_table.intern("Hello "),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("test string constant should construct")
        .id();
    let after_value = expression_builder
        .string_slice(
            &mut const_values,
            string_table.intern("World!"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("test string constant should construct")
        .id();
    let declarations = vec![
        make_test_variable(
            path_fork
                .try_intern_child(scope, const_before)
                .expect("test path fits"),
            before_value,
            &expression_builder,
        ),
        make_test_variable(
            path_fork
                .try_intern_child(scope, const_after)
                .expect("test path fits"),
            after_value,
            &expression_builder,
        ),
    ];

    let style_directives = frontend_test_style_directives();
    let context = with_test_path_context(
        ScopeContext::new_for_tests(
            ContextKind::Constant,
            scope.to_owned(),
            Rc::new(TopLevelDeclarationTable::new(
                declarations.clone(),
                &path_fork,
            )),
            Arc::new(ExternalPackageRegistry::default()),
            0,
        ),
        &scope,
        &style_directives,
    );
    let file_tokens = template_tokens_from_source(
        "[const_before, const_after]",
        &mut string_table,
        &mut span_builder,
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
    let mut expected_type = ExpectedType::Infer;

    let expression = create_expression_for_test(
        &mut token_stream,
        &context,
        &mut expression_builder,
        &mut const_values,
        &mut expected_type,
        &ValueMode::ImmutableOwned,
        false,
        &mut string_table,
        &mut path_fork,
    )
    .expect("constant template references should fold");

    let ExpressionOperationView::Constant(id) = expression_builder
        .view(expression)
        .expect("expression belongs to builder")
        .operation()
    else {
        panic!("expected folded constant string expression in constant context");
    };
    let value = const_values
        .string_value(id)
        .expect("folded constant context expression should be a string");

    assert_eq!(string_table.resolve(value), "Hello World!");
}

#[test]
fn non_constant_context_template_head_keeps_runtime_template() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let file_tokens = template_tokens_from_source(
        "[value]",
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
    let mut expected_type = ExpectedType::Infer;

    let expression = create_expression_for_test(
        &mut token_stream,
        &context,
        &mut expression_builder,
        &mut const_values,
        &mut expected_type,
        &ValueMode::ImmutableOwned,
        false,
        &mut string_table,
        &mut path_fork,
    )
    .expect("runtime template expression should parse");

    let ExpressionOperationView::Template(template) = expression_builder
        .view(expression)
        .expect("expression belongs to builder")
        .operation()
    else {
        panic!("expected runtime template expression");
    };

    assert!(matches!(
        effective_tir_kind(template, &context),
        TemplateType::StringFunction
    ));
}

fn assert_slot_is_tir_only_and_const(source: &str, slot_name: &str) {
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
    .expect("template with slot should parse");

    assert!(
        prepare_effective_template_facts(
            &template,
            &context.template_ir_store,
            &expression_builder,
            &const_values,
        )
        .expect("slot classification should succeed")
        .has_unresolved_slot_occurrences,
        "{slot_name} slot should be detected from TIR"
    );

    let expression = expression_builder
        .template(template, ValueMode::ImmutableOwned)
        .expect("template expression should construct")
        .id();
    let view = expression_builder
        .view(expression)
        .expect("template expression belongs to builder");
    assert!(
        view.const_value_kind_with_template_classifier(&const_values, &mut |template| {
            classify_template_from_effective_tir(
                template,
                &context.template_ir_store,
                &expression_builder,
                &const_values,
                None,
            )
        })
        .expect("const classification should succeed")
        .is_compile_time_value(),
        "{slot_name} slot template should be compile-time constant from TIR"
    );
}

#[test]
fn default_slot_is_recorded_in_tir_and_const() {
    assert_slot_is_tir_only_and_const("[: before [$slot] after]", "default");
}

#[test]
fn named_slot_is_recorded_in_tir_and_const() {
    assert_slot_is_tir_only_and_const("[: before [$slot(\"name\")] after]", "named");
}
