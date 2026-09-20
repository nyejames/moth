use super::*;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::resolver::{
    classify_template_from_effective_tir, prepare_template_tir_facts,
};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;
use std::rc::Rc;
use std::sync::Arc;

fn create_expression_for_test(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    expected_type: &mut ExpectedType,
    value_mode: &ValueMode,
    consume_closing_parenthesis: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<crate::compiler_frontend::ast::expressions::expression::Expression, ExpressionParseError>
{
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    create_expression(
        token_stream,
        context,
        &mut type_interner,
        expected_type,
        value_mode,
        consume_closing_parenthesis,
        string_table,
        path_fork,
    )
}

#[test]
fn slot_wrappers_remain_compile_time_templates_until_filled() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = template_tokens_from_source(
        "[: before [$slot] after]",
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
    .expect("wrapper template should parse");

    assert!(matches!(
        effective_tir_kind(&template, &context),
        TemplateType::String
    ));
    assert!(
        prepare_template_tir_facts(&template, &context.template_ir_store)
            .expect("wrapper classification should succeed")
            .has_unresolved_slot_occurrences,
        "wrapper template should have unresolved slots"
    );
    let expression = Expression::template(template, ValueMode::ImmutableOwned);
    assert!(
        expression
            .const_value_kind_with_template_classifier(&mut |template| {
                classify_template_from_effective_tir(template, &context.template_ir_store)
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
    let canonical_owner = wrapper_file_tokens
        .canonical_owner()
        .expect("test token stream must expose canonical source tokens");
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
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("wrapper template should parse");

    let declarations = vec![Declaration {
        id: path_fork
            .try_intern_child(scope, string_table.intern("header"))
            .expect("test path fits"),
        value: Expression::template(wrapper, ValueMode::ImmutableOwned),
        binding_span: None,
        config_qualifier: None,
    }];

    let file_tokens = template_tokens_from_source(
        "[header]",
        &mut string_table,
        &mut span_builder,
        &mut path_fork,
    );
    let source_path = file_tokens.source_path;
    let context = constant_template_context(&source_path, &declarations, &path_fork)
        .with_template_ir_store(wrapper_context.template_ir_store.clone());
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
    .expect("template using wrapper constant should parse");

    let folded = fold_template_in_context(&template, &context, &mut string_table);
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
    let file_tokens = template_tokens_from_source(
        "[value: before [$slot] after]",
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
    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
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
        prepare_template_tir_facts(&template, &context.template_ir_store)
            .expect("runtime wrapper classification should succeed")
            .has_unresolved_slot_occurrences,
        "runtime wrapper template should have unresolved slots"
    );
    let expression = Expression::template(template, ValueMode::ImmutableOwned);
    assert!(
        !expression
            .const_value_kind_with_template_classifier(&mut |template| {
                classify_template_from_effective_tir(template, &context.template_ir_store)
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
    let scope = path_fork
        .try_intern_portable_path("main.moth/#const_template0", &mut string_table)
        .expect("test path fits");
    let const_before = string_table.intern("const_before");
    let const_after = string_table.intern("const_after");
    let declarations = vec![
        Declaration {
            id: path_fork
                .try_intern_child(scope, const_before)
                .expect("test path fits"),
            value: Expression::string_slice(
                string_table.intern("Hello "),
                None,
                ValueMode::ImmutableOwned,
            ),
            binding_span: None,
            config_qualifier: None,
        },
        Declaration {
            id: path_fork
                .try_intern_child(scope, const_after)
                .expect("test path fits"),
            value: Expression::string_slice(
                string_table.intern("World!"),
                None,
                ValueMode::ImmutableOwned,
            ),
            binding_span: None,
            config_qualifier: None,
        },
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
            vec![],
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
    let mut expected_type = ExpectedType::Infer;

    let expression = create_expression_for_test(
        &mut token_stream,
        &context,
        &mut expected_type,
        &ValueMode::ImmutableOwned,
        false,
        &mut string_table,
        &mut path_fork,
    )
    .expect("constant template references should fold");

    let ExpressionKind::StringSlice(value) = expression.kind else {
        panic!("expected folded StringSlice expression in constant context");
    };

    assert_eq!(string_table.resolve(value), "Hello World!");
}

#[test]
fn non_constant_context_template_head_keeps_runtime_template() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let file_tokens = template_tokens_from_source(
        "[value]",
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
    let mut expected_type = ExpectedType::Infer;

    let expression = create_expression_for_test(
        &mut token_stream,
        &context,
        &mut expected_type,
        &ValueMode::ImmutableOwned,
        false,
        &mut string_table,
        &mut path_fork,
    )
    .expect("runtime template expression should parse");

    let ExpressionKind::Template(template) = expression.kind else {
        panic!("expected runtime template expression");
    };

    assert!(matches!(
        effective_tir_kind(&template, &context),
        TemplateType::StringFunction
    ));
}

fn assert_slot_is_tir_only_and_const(source: &str, slot_name: &str) {
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
    let template = Template::new(
        &mut token_stream,
        source_path,
        &context,
        vec![],
        &mut string_table,
        &mut path_fork,
    )
    .expect("template with slot should parse");

    assert!(
        prepare_template_tir_facts(&template, &context.template_ir_store)
            .expect("slot classification should succeed")
            .has_unresolved_slot_occurrences,
        "{slot_name} slot should be detected from TIR"
    );

    let expression = Expression::template(template, ValueMode::ImmutableOwned);
    assert!(
        expression
            .const_value_kind_with_template_classifier(&mut |template| {
                classify_template_from_effective_tir(template, &context.template_ir_store)
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
