use super::*;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::resolver::{
    classify_template_from_effective_tir, prepare_template_tir_facts,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;
use std::rc::Rc;
use std::sync::Arc;

fn create_expression_for_test(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    expected_type: &mut ExpectedType,
    value_mode: &ValueMode,
    consume_closing_parenthesis: bool,
    string_table: &mut StringTable,
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
    )
}

#[test]
fn slot_wrappers_remain_compile_time_templates_until_filled() {
    let mut string_table = StringTable::new();
    let mut token_stream =
        template_tokens_from_source("[: before [$slot] after]", &mut string_table);
    let context = new_constant_context(token_stream.src_path.to_owned());

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
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
    let scope = InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);

    let mut wrapper_tokens = template_tokens_from_source(
        "[:<link rel=\"icon\" href=\"[$slot(\"favicon\")]\"><style>[$slot(\"css\")]</style>]",
        &mut string_table,
    );
    let wrapper_context = new_constant_context(wrapper_tokens.src_path.to_owned());
    let wrapper = Template::new(
        &mut wrapper_tokens,
        &wrapper_context,
        vec![],
        &mut string_table,
    )
    .expect("wrapper template should parse");

    let declarations = vec![Declaration {
        id: scope.append(string_table.intern("header")),
        value: Expression::template(wrapper, ValueMode::ImmutableOwned),
    }];

    let mut token_stream = template_tokens_from_source("[header]", &mut string_table);
    let context = constant_template_context(&token_stream.src_path, &declarations)
        .with_template_ir_store(wrapper_context.template_ir_store.clone());
    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
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
    let mut token_stream =
        template_tokens_from_source("[value: before [$slot] after]", &mut string_table);
    let context = runtime_template_context(&token_stream.src_path, &mut string_table);

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
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
    let scope = InternedPath::from_single_str("main.moth/#const_template0", &mut string_table);
    let const_before = string_table.intern("const_before");
    let const_after = string_table.intern("const_after");
    let declarations = vec![
        Declaration {
            id: scope.append(const_before),
            value: Expression::string_slice(
                string_table.intern("Hello "),
                SourceLocation {
                    scope: InternedPath::new(),
                    start_pos: CharPosition {
                        line_number: 1,
                        char_column: 0,
                    },
                    end_pos: CharPosition {
                        line_number: 1,
                        char_column: 120, // Arbitrary number
                    },
                },
                ValueMode::ImmutableOwned,
            ),
        },
        Declaration {
            id: scope.append(const_after),
            value: Expression::string_slice(
                string_table.intern("World!"),
                SourceLocation {
                    scope: InternedPath::new(),
                    start_pos: CharPosition {
                        line_number: 1,
                        char_column: 0,
                    },
                    end_pos: CharPosition {
                        line_number: 1,
                        char_column: 120, // Arbitrary number
                    },
                },
                ValueMode::ImmutableOwned,
            ),
        },
    ];

    let style_directives = frontend_test_style_directives();
    let context = with_test_path_context(
        ScopeContext::new_for_tests(
            ContextKind::Constant,
            scope.to_owned(),
            Rc::new(TopLevelDeclarationTable::new(declarations.clone())),
            Arc::new(ExternalPackageRegistry::default()),
            vec![],
            0,
        ),
        &scope,
        &style_directives,
    );
    let mut token_stream =
        template_tokens_from_source("[const_before, const_after]", &mut string_table);
    let mut expected_type = ExpectedType::Infer;

    let expression = create_expression_for_test(
        &mut token_stream,
        &context,
        &mut expected_type,
        &ValueMode::ImmutableOwned,
        false,
        &mut string_table,
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
    let mut token_stream = template_tokens_from_source("[value]", &mut string_table);
    let context = runtime_template_context(&token_stream.src_path, &mut string_table);
    let mut expected_type = ExpectedType::Infer;

    let expression = create_expression_for_test(
        &mut token_stream,
        &context,
        &mut expected_type,
        &ValueMode::ImmutableOwned,
        false,
        &mut string_table,
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
    let mut token_stream = template_tokens_from_source(source, &mut string_table);
    let context = new_constant_context(token_stream.src_path.to_owned());

    let template = Template::new(&mut token_stream, &context, vec![], &mut string_table)
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
