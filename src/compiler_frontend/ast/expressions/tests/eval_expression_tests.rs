//! Expression evaluation and runtime-RPN parsing regression tests.
//!
//! WHAT: validates operator precedence, runtime expression node construction, and template
//!       expression parsing.
//! WHY: expression parsing is dense and easy to break during refactors; targeted tests catch
//!      shape drift before it reaches HIR lowering.

use super::*;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, DiagnosticOperator, DiagnosticPayload, InvalidBuiltinCallReason,
    TypeDiagnosticKind,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::paths::compile_time_paths::{
    CompileTimePath, CompileTimePathBase, CompileTimePathKind, CompileTimePaths,
};
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::start_function_body;
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use std::rc::Rc;
use std::sync::Arc;

fn first_start_declaration_expression(source: &str) -> Expression {
    nth_start_declaration_expression(source, 0)
}

fn nth_start_declaration_expression(source: &str, index: usize) -> Expression {
    let (ast, _string_table) = parse_single_file_ast(source);
    let start_function = ast
        .nodes
        .iter()
        .find(|node| matches!(node.kind, NodeKind::Function(_, _, _)))
        .expect("start function should exist");

    let NodeKind::Function(_, _, body) = &start_function.kind else {
        panic!("expected start function body");
    };
    let NodeKind::VariableDeclaration(declaration) = &body[index].kind else {
        panic!("expected start statement {index} to be a variable declaration");
    };

    declaration.value.to_owned()
}

fn assert_unsupported_operator(source: &str, expected_operator: DiagnosticOperator) {
    let diagnostic = parse_single_file_ast_diagnostic(source);

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnsupportedOperatorTypes {
            operator,
            ..
        } if operator == expected_operator
    ));
}

#[test]
fn ordinary_expression_rejects_path_string_concatenation() {
    let mut string_table = StringTable::new();
    let source_scope = InternedPath::from_single_str("#page.moth", &mut string_table);
    let asset_path = InternedPath::from_single_str("assets", &mut string_table)
        .join_str("logo.png", &mut string_table);
    let compile_time_paths = CompileTimePaths {
        paths: vec![CompileTimePath {
            source_path: asset_path.clone(),
            filesystem_path: std::env::temp_dir().join("moth_eval_expression_logo.png"),
            public_path: asset_path.clone(),
            base: CompileTimePathBase::EntryRoot,
            kind: CompileTimePathKind::File,
        }],
    };
    let context = ScopeContext::new_for_tests(
        ContextKind::Template,
        source_scope.clone(),
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    )
    .with_source_file_scope(source_scope.clone())
    .with_path_format_config(PathStringFormatConfig {
        origin: String::from("/moth"),
        ..PathStringFormatConfig::default()
    });

    let nodes = vec![
        ExpressionRpnItem::Operand(Expression::path(
            compile_time_paths,
            SourceLocation::default(),
        )),
        ExpressionRpnItem::Operator {
            operator: Operator::Add,
            location: SourceLocation::default(),
        },
        ExpressionRpnItem::Operand(Expression::string_slice(
            string_table.get_or_intern(String::from("?v=1")),
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        )),
    ];

    let mut current_type = ExpectedType::Infer;
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let error = evaluate_expression(
        &context,
        nodes,
        &mut type_interner,
        &mut current_type,
        &ValueMode::ImmutableOwned,
        &mut string_table,
    )
    .expect_err("ordinary expressions should stay strict");

    let crate::compiler_frontend::ast::expressions::eval_expression::ExpressionTypingError::Diagnostic(diagnostic) = error else {
        panic!("expected an expression type diagnostic");
    };
    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Type(TypeDiagnosticKind::UnsupportedOperatorTypes)
    );
    assert_eq!(diagnostic.kind.code(), "MOTH-TYPE-0003");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnsupportedOperatorTypes {
            operator: DiagnosticOperator::Add,
            ..
        }
    ));

    let recorded = context.take_rendered_path_usages();
    assert!(recorded.is_empty());
}

#[test]
fn unary_not_requires_boolean_operand() {
    assert_unsupported_operator("value = not 1\n", DiagnosticOperator::Not);
}

#[test]
fn logical_and_requires_bool_operands() {
    assert_unsupported_operator("value = true and 1\n", DiagnosticOperator::And);
}

#[test]
fn logical_and_reports_found_types_in_operand_order() {
    assert_unsupported_operator("value = 1 and true\n", DiagnosticOperator::And);
}

#[test]
fn logical_or_rejects_string_operands() {
    assert_unsupported_operator("value = \"a\" or \"b\"\n", DiagnosticOperator::Or);
}

#[test]
fn logical_mix_rejects_non_bool_rhs_after_comparison() {
    assert_unsupported_operator("value = 1 < 2 and 3\n", DiagnosticOperator::And);
}

#[test]
fn int_constructor_is_rejected_with_removed_scalar_constructor_diagnostic() {
    let diagnostic = parse_single_file_ast_diagnostic("value = Int(\"1\")\n");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidBuiltinCall {
            reason: InvalidBuiltinCallReason::ScalarConstructorRemoved,
            ..
        }
    ));
}

#[test]
fn float_constructor_is_rejected_with_removed_scalar_constructor_diagnostic() {
    let diagnostic = parse_single_file_ast_diagnostic("value = Float(1.5)\n");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidBuiltinCall {
            reason: InvalidBuiltinCallReason::ScalarConstructorRemoved,
            ..
        }
    ));
}

#[test]
fn bool_constructor_is_rejected_with_removed_scalar_constructor_diagnostic() {
    let diagnostic = parse_single_file_ast_diagnostic("value = Bool(true)\n");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidBuiltinCall {
            reason: InvalidBuiltinCallReason::ScalarConstructorRemoved,
            ..
        }
    ));
}

#[test]
fn logical_operator_rejects_option_operands_with_precise_found_type() {
    assert_unsupported_operator(
        "maybe String? = none\nvalue = maybe or true\n",
        DiagnosticOperator::Or,
    );
}

#[test]
fn comparison_operator_accepts_option_to_scalar_comparison() {
    let value =
        nth_start_declaration_expression("maybe String? = \"x\"\nvalue = maybe is \"x\"\n", 1);

    assert_eq!(value.diagnostic_type, DataType::Bool);
}

#[test]
fn comparison_operator_rejects_none_without_option_context() {
    assert_unsupported_operator("value = none is none\n", DiagnosticOperator::Equality);
}

#[test]
fn mixed_int_float_arithmetic_resolves_to_float() {
    let value = first_start_declaration_expression("value = 1 + 2.5\n");

    assert_eq!(value.diagnostic_type, DataType::Float);
}

#[test]
fn int_division_resolves_to_float() {
    let value = first_start_declaration_expression("value = 5 / 2\n");

    assert_eq!(value.diagnostic_type, DataType::Float);
}

#[test]
fn grouped_integer_subexpression_does_not_override_division_result_type() {
    let (ast, _string_table) =
        parse_single_file_ast("value #= ((10 * 10) + (20 * 20)) / 10\n\ntyped Float = value\n");
    let value = ast
        .module_constants
        .first()
        .expect("the inferred constant should be retained in module constants");

    assert_eq!(value.value.type_id, builtin_type_ids::FLOAT);
    assert_eq!(value.value.diagnostic_type, DataType::Float);
    assert!(
        matches!(value.value.kind, ExpressionKind::Float(result) if (result - 50.0).abs() < f64::EPSILON)
    );
}

#[test]
fn integer_division_resolves_to_int() {
    let value = first_start_declaration_expression("value = 5 // 2\n");

    assert_eq!(value.diagnostic_type, DataType::Int);
}

#[test]
fn integer_division_rejects_int_float_operands() {
    assert_unsupported_operator("value = 5 // 2.0\n", DiagnosticOperator::IntDivide);
}

#[test]
fn integer_division_rejects_float_int_operands() {
    assert_unsupported_operator("value = 5.0 // 2\n", DiagnosticOperator::IntDivide);
}

#[test]
fn multiline_expression_with_operator_on_next_line_resolves_correctly() {
    let value = first_start_declaration_expression("value = 1\n + 2\n + 3\n");

    assert_eq!(value.diagnostic_type, DataType::Int);
    assert!(
        matches!(value.kind, ExpressionKind::Int(6)),
        "expected folded Int(6), got {:?}",
        value.kind
    );
}

#[test]
fn multiline_expression_with_operator_at_end_of_line_resolves_correctly() {
    let value = first_start_declaration_expression("value = 1 +\n 2 +\n 3\n");

    assert_eq!(value.diagnostic_type, DataType::Int);
    assert!(
        matches!(value.kind, ExpressionKind::Int(6)),
        "expected folded Int(6), got {:?}",
        value.kind
    );
}

#[test]
fn multiline_comparison_expression_resolves_to_bool() {
    let value = first_start_declaration_expression("value = 1\n is\n 1\n");

    assert_eq!(value.diagnostic_type, DataType::Bool);
    assert!(
        matches!(value.kind, ExpressionKind::Bool(true)),
        "expected folded Bool(true), got {:?}",
        value.kind
    );
}

#[test]
fn mixed_int_float_comparison_resolves_to_bool() {
    let value = first_start_declaration_expression("value = 1 <= 2.5\n");

    assert_eq!(value.diagnostic_type, DataType::Bool);
}

#[test]
fn bool_relational_comparison_is_rejected() {
    assert_unsupported_operator("value = true < false\n", DiagnosticOperator::LessThan);
}

#[test]
fn string_equality_comparison_resolves_to_bool() {
    let value = first_start_declaration_expression("value = \"a\" is \"b\"\n");

    assert_eq!(value.diagnostic_type, DataType::Bool);
}

#[test]
fn char_relational_comparison_resolves_to_bool() {
    let value = first_start_declaration_expression("value = 'a' < 'b'\n");

    assert_eq!(value.diagnostic_type, DataType::Bool);
}

#[test]
fn fully_constant_boolean_and_comparison_expressions_fold() {
    let (ast, _string_table) = parse_single_file_ast("flag = not (1 < 2) or (3 < 4 and false)\n");
    let start_function = ast
        .nodes
        .iter()
        .find(|node| matches!(node.kind, NodeKind::Function(_, _, _)))
        .expect("start function should exist");

    let NodeKind::Function(_, _, body) = &start_function.kind else {
        panic!("expected start function body");
    };
    let NodeKind::VariableDeclaration(declaration) = &body[0].kind else {
        panic!("expected folded declaration");
    };

    assert!(
        matches!(declaration.value.kind, ExpressionKind::Bool(false)),
        "expected fully-folded boolean/comparison expression to collapse to Bool(false), got {:?}",
        declaration.value.kind
    );
    assert_eq!(declaration.value.diagnostic_type, DataType::Bool);
}

#[test]
fn string_slice_concatenation_with_variable_resolves_to_string() {
    let value = first_start_declaration_expression("str1 = \"Hello\"\nvalue = str1 + \" World\"\n");

    assert_eq!(value.diagnostic_type, DataType::StringSlice);
    assert_eq!(value.value_shape, ExpressionValueShape::PlainStringSlice);
    assert!(
        matches!(value.kind, ExpressionKind::StringSlice(_)),
        "expected folded string concatenation to collapse to a string slice, got {:?}",
        value.kind
    );
}

#[test]
fn folded_template_preserves_template_shape() {
    let value = first_start_declaration_expression("value = [:template body]\n");

    assert_eq!(value.type_id, builtin_type_ids::STRING);
    assert_eq!(value.value_shape, ExpressionValueShape::TemplateString);
    assert!(
        matches!(value.kind, ExpressionKind::StringSlice(_)),
        "expected folded template to collapse to a string slice kind, got {:?}",
        value.kind
    );
}

#[test]
fn copied_template_string_preserves_template_shape() {
    let value = nth_start_declaration_expression(
        "source String = [:template body]\nvalue = copy source\n",
        1,
    );

    assert_eq!(value.type_id, builtin_type_ids::STRING);
    assert_eq!(value.value_shape, ExpressionValueShape::TemplateString);
    assert!(
        matches!(value.kind, ExpressionKind::Copy(_)),
        "expected explicit copy to remain a copy expression, got {:?}",
        value.kind
    );
}

#[test]
fn template_shaped_string_operand_is_rejected() {
    let mut string_table = StringTable::new();

    let mut template_like_string = Expression::string_slice(
        string_table.get_or_intern(String::from("template text")),
        SourceLocation::default(),
        ValueMode::ImmutableOwned,
    );
    template_like_string.value_shape = ExpressionValueShape::TemplateString;

    let nodes = vec![
        ExpressionRpnItem::Operand(Expression::string_slice(
            string_table.get_or_intern(String::from("plain ")),
            SourceLocation::default(),
            ValueMode::ImmutableOwned,
        )),
        ExpressionRpnItem::Operand(template_like_string),
        ExpressionRpnItem::Operator {
            operator: Operator::Add,
            location: SourceLocation::default(),
        },
    ];

    let context = ScopeContext::new_for_tests(
        ContextKind::Template,
        InternedPath::from_single_str("#page.moth", &mut string_table),
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let mut current_type = ExpectedType::Infer;
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let error = evaluate_expression(
        &context,
        nodes,
        &mut type_interner,
        &mut current_type,
        &ValueMode::ImmutableOwned,
        &mut string_table,
    )
    .expect_err("template-shaped string values should not become plain string operands");

    let crate::compiler_frontend::ast::expressions::eval_expression::ExpressionTypingError::Diagnostic(diagnostic) = error else {
        panic!("expected an expression type diagnostic");
    };
    assert_eq!(diagnostic.kind.code(), "MOTH-TYPE-0003");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnsupportedOperatorTypes {
            operator: DiagnosticOperator::Add,
            ..
        }
    ));
}

#[test]
fn function_result_string_concatenation_is_allowed() {
    let (ast, string_table) =
        parse_single_file_ast("f || -> String:\n    return \"a\"\n;\nvalue = f() + \"b\"\n");
    let body = start_function_body(&ast, &string_table);
    let NodeKind::VariableDeclaration(declaration) = &body[0].kind else {
        panic!("expected start statement to be a variable declaration");
    };
    let value = &declaration.value;

    assert_eq!(value.diagnostic_type, DataType::StringSlice);
    assert_eq!(value.value_shape, ExpressionValueShape::PlainStringSlice);
    assert!(
        matches!(value.kind, ExpressionKind::Runtime(_)),
        "expected function-result concatenation to stay a runtime expression, got {:?}",
        value.kind
    );
}
