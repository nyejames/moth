//! Expression evaluation and runtime-RPN parsing regression tests.
//!
//! WHAT: validates operator precedence, runtime expression node construction, and template
//!       expression parsing.
//! WHY: expression parsing is dense and easy to break during refactors; targeted tests catch
//!      semantic drift before it reaches HIR lowering.

use super::*;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::{ConstStringPiece, ConstValuePayload};
use crate::compiler_frontend::ast::expressions::expression::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, DiagnosticOperator, DiagnosticPayload, InvalidBuiltinCallReason,
    NumberLiteralErrorReason, TypeDiagnosticKind, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::fixed_scalar::{FixedScalar, FixedScalarValue};
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::number::NumberScale;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::function_body_by_name;
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use std::rc::Rc;
use std::sync::Arc;

fn first_start_declaration_expression(source: &str) -> Expression {
    nth_start_declaration_expression(source, 0)
}

fn nth_start_declaration_expression(source: &str, index: usize) -> Expression {
    let (ast, _path_fork, _string_table) = parse_single_file_ast(source);
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

    assert!(
        matches!(
            &diagnostic.payload,
            DiagnosticPayload::UnsupportedOperatorTypes { operator, .. }
                if *operator == expected_operator
        ),
        "expected {expected_operator:?} for {source:?}, got {:?}",
        diagnostic.payload
    );
}

#[test]
fn ordinary_expression_rejects_path_string_concatenation() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_scope = path_fork
        .try_intern_portable_path("@page.moth", &mut string_table)
        .expect("test path fits");
    let context = ScopeContext::new_for_tests(
        ContextKind::Template,
        source_scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    )
    .with_source_file_scope(source_scope);

    let nodes = vec![
        ExpressionRpnItem::Operand(Expression::structural_string(
            vec![ConstStringPiece::SiteRoot],
            None,
        )),
        ExpressionRpnItem::Operator {
            operator: Operator::Add,
            span: None,
        },
        ExpressionRpnItem::Operand(Expression::string_slice(
            string_table.get_or_intern(String::from("?v=1")),
            None,
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
        &path_fork,
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
}
#[test]
fn structural_string_equality_is_refused_only_in_a_constant_context() {
    // WHY: the refusal belongs only to const-required positions; rejecting runtime
    // positions would remove legal expressive power.
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_scope = path_fork
        .try_intern_portable_path("@page.moth", &mut string_table)
        .expect("test path fits");
    let context = |kind| {
        ScopeContext::new_for_tests(
            kind,
            source_scope,
            Rc::new(TopLevelDeclarationTable::new(
                vec![],
                &PathInternerFork::empty(),
            )),
            Arc::new(ExternalPackageRegistry::new()),
            vec![],
            0,
        )
    };
    let nodes = |string_table: &mut StringTable| {
        vec![
            ExpressionRpnItem::Operand(Expression::structural_string(
                vec![ConstStringPiece::SiteRoot],
                None,
            )),
            ExpressionRpnItem::Operand(Expression::string_slice(
                string_table.intern("plain"),
                None,
                ValueMode::ImmutableOwned,
            )),
            ExpressionRpnItem::Operator {
                operator: Operator::Equality,
                span: None,
            },
        ]
    };

    let mut constant_type_environment = TypeEnvironment::new();
    let mut constant_compatibility_cache = TypeCompatibilityCache::new();
    let mut constant_type_interner = AstTypeInterner::new(
        &mut constant_type_environment,
        &mut constant_compatibility_cache,
    );
    let mut constant_expected_type = ExpectedType::Infer;
    let constant_error = evaluate_expression(
        &context(ContextKind::Constant),
        nodes(&mut string_table),
        &mut constant_type_interner,
        &mut constant_expected_type,
        &ValueMode::ImmutableOwned,
        &mut string_table,
        &path_fork,
    )
    .expect_err("constant equality must require final structural-string text");
    let crate::compiler_frontend::ast::expressions::eval_expression::ExpressionTypingError::Diagnostic(
        diagnostic,
    ) = constant_error
    else {
        panic!("expected a typed structural-string diagnostic");
    };
    assert_eq!(
        diagnostic.identity().reason_key,
        Some("compile_time_evaluation_error.structural_string_requires_final_text")
    );

    let mut runtime_type_environment = TypeEnvironment::new();
    let mut runtime_compatibility_cache = TypeCompatibilityCache::new();
    let mut runtime_type_interner = AstTypeInterner::new(
        &mut runtime_type_environment,
        &mut runtime_compatibility_cache,
    );
    let mut runtime_expected_type = ExpectedType::Infer;
    let runtime_expression = match evaluate_expression(
        &context(ContextKind::Function),
        nodes(&mut string_table),
        &mut runtime_type_interner,
        &mut runtime_expected_type,
        &ValueMode::ImmutableOwned,
        &mut string_table,
        &path_fork,
    ) {
        Ok(expression) => expression,
        Err(_) => {
            panic!("runtime equality must preserve the structural operation without a diagnostic")
        }
    };

    let ExpressionKind::Runtime(runtime_items) = runtime_expression.kind else {
        panic!("runtime equality should remain a complete runtime RPN expression");
    };
    assert_eq!(
        runtime_items.items.len(),
        3,
        "runtime context must preserve both operands and the equality operator"
    );
    assert!(matches!(
        runtime_items.items.last(),
        Some(ExpressionRpnItem::Operator {
            operator: Operator::Equality,
            ..
        })
    ));
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
fn fixed_scalar_constructors_are_rejected_with_removed_scalar_constructor_diagnostic() {
    // Explicit-width type names are type spellings, not conversion functions, so they must take
    // the same path as `Int(...)`/`Float(...)` instead of becoming an unknown-symbol call.
    for source in [
        "value = U8(200)\n",
        "value = I64(200)\n",
        "value = Byte(200)\n",
    ] {
        let diagnostic = parse_single_file_ast_diagnostic(source);
        assert!(
            matches!(
                diagnostic.payload,
                DiagnosticPayload::InvalidBuiltinCall {
                    reason: InvalidBuiltinCallReason::ScalarConstructorRemoved,
                    ..
                }
            ),
            "expected ScalarConstructorRemoved for {source:?}"
        );
    }
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
    let (ast, _path_fork, _string_table) =
        parse_single_file_ast("value #= ((10 * 10) + (20 * 20)) / 10\n\ntyped Float = value\n");
    let value_id = ast
        .const_values
        .iter_module_constant_views()
        .next()
        .expect("the inferred constant should be retained in module constants")
        .id;
    let value = ast
        .const_values
        .value(value_id)
        .expect("the inferred constant value should be retained");

    assert_eq!(value.metadata.type_id, builtin_type_ids::FLOAT);
    assert_eq!(value.metadata.diagnostic_type, DataType::Float);
    assert!(
        matches!(value.payload, ConstValuePayload::Float(result) if (result - 50.0).abs() < f64::EPSILON)
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
fn string_equality_accepts_quoted_and_runtime_template_values() {
    let value = nth_start_declaration_expression(
        "suffix = \"x\"\ntemplate_value = [:same[suffix]]\nvalue = \"samex\" is template_value\n",
        2,
    );

    assert_eq!(value.diagnostic_type, DataType::Bool);
}

#[test]
fn string_ordering_comparison_is_rejected() {
    assert_unsupported_operator("value = \"a\" < \"b\"\n", DiagnosticOperator::LessThan);
}

#[test]
fn char_relational_comparison_resolves_to_bool() {
    let value = first_start_declaration_expression("value = 'a' < 'b'\n");

    assert_eq!(value.diagnostic_type, DataType::Bool);
}

#[test]
fn fully_constant_boolean_and_comparison_expressions_fold() {
    let (ast, _path_fork, _string_table) =
        parse_single_file_ast("flag = not (1 < 2) or (3 < 4 and false)\n");
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
fn string_slice_concatenation_with_variable_is_rejected() {
    assert_unsupported_operator(
        "str1 = \"Hello\"\nvalue = str1 + \" World\"\n",
        DiagnosticOperator::Add,
    );
}

#[test]
fn folded_template_preserves_template_diagnostic_type() {
    let value = first_start_declaration_expression("value = [:template body]\n");

    assert_eq!(value.type_id, builtin_type_ids::STRING);
    assert_eq!(value.diagnostic_type, DataType::StringSlice);
    assert!(
        matches!(value.kind, ExpressionKind::StringSlice(_)),
        "expected folded template to collapse to a string slice kind, got {:?}",
        value.kind
    );
}

#[test]
fn copied_template_string_preserves_copy_expression_kind() {
    let value = nth_start_declaration_expression(
        "source String = [:template body]\nvalue = copy source\n",
        1,
    );

    assert_eq!(value.type_id, builtin_type_ids::STRING);
    assert_eq!(value.diagnostic_type, DataType::StringSlice);
    assert!(
        matches!(value.kind, ExpressionKind::Copy(_)),
        "expected explicit copy to remain a copy expression, got {:?}",
        value.kind
    );
}

#[test]
fn template_like_string_operands_reject_add() {
    let diagnostic = parse_single_file_ast_diagnostic("value = \"left\" + \"right\"\n");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnsupportedOperatorTypes {
            operator: DiagnosticOperator::Add,
            ..
        }
    ));
}

#[test]
fn function_result_string_concatenation_is_rejected() {
    assert_unsupported_operator(
        "f || -> String:\n    return \"a\"\n;\nvalue = f() + \"b\"\n",
        DiagnosticOperator::Add,
    );
}
#[test]
fn direct_fixed_literal_materialises_signed_minimum_from_token_sign() {
    let value = first_start_declaration_expression("small I8 = -128\n");

    assert_eq!(
        value.diagnostic_type,
        DataType::FixedScalar(FixedScalar::I8)
    );
    assert_eq!(
        value.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::I8)
    );
    assert!(
        matches!(
            value.kind,
            ExpressionKind::FixedScalar(scalar) if scalar == FixedScalarValue::signed(FixedScalar::I8, -128).expect("I8 minimum should materialise")
        ),
        "expected I8(-128), got {:?}",
        value.kind
    );
}

#[test]
fn direct_fixed_literal_rejects_parser_negation_as_operator_result() {
    let diagnostic = parse_single_file_ast_diagnostic("small I8 = 0 - 128\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::TypeMismatch {
            context: TypeMismatchContext::Declaration,
            ..
        }
    ));
}

#[test]
fn direct_fixed_literal_materialises_u64_max() {
    let value = first_start_declaration_expression("large U64 = 18_446_744_073_709_551_615\n");
    assert_eq!(
        value.diagnostic_type,
        DataType::FixedScalar(FixedScalar::U64)
    );
    assert!(
        matches!(
            value.kind,
            ExpressionKind::FixedScalar(scalar) if scalar == FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).expect("U64 maximum should materialise")
        ),
        "expected U64 max, got {:?}",
        value.kind
    );
}

#[test]
fn direct_fixed_literal_materialises_default_profile_large_u64() {
    let value = first_start_declaration_expression("large U64 = 18_000_000_000\n");

    assert_eq!(
        value.diagnostic_type,
        DataType::FixedScalar(FixedScalar::U64)
    );
    assert!(
        matches!(
            value.kind,
            ExpressionKind::FixedScalar(scalar) if scalar == FixedScalarValue::unsigned(FixedScalar::U64, 18_000_000_000).expect("default-profile U64 literal should materialise")
        ),
        "expected U64(18000000000), got {:?}",
        value.kind
    );
}

#[test]
fn direct_fixed_literal_materialises_byte_endpoints() {
    let low = first_start_declaration_expression("raw Byte = 0\n");
    assert!(
        matches!(
            low.kind,
            ExpressionKind::FixedScalar(scalar) if scalar == FixedScalarValue::unsigned(FixedScalar::Byte, 0).expect("Byte zero should materialise")
        ),
        "expected Byte(0), got {:?}",
        low.kind
    );

    let high = first_start_declaration_expression("raw Byte = 255\n");
    assert!(
        matches!(
            high.kind,
            ExpressionKind::FixedScalar(scalar) if scalar == FixedScalarValue::unsigned(FixedScalar::Byte, 255).expect("Byte maximum should materialise")
        ),
        "expected Byte(255), got {:?}",
        high.kind
    );
}

#[test]
fn direct_fixed_literal_materialises_f16_and_f32_bits() {
    let half = first_start_declaration_expression("half F16 = 0.1\n");
    assert_eq!(
        half.diagnostic_type,
        DataType::FixedScalar(FixedScalar::F16)
    );
    assert!(
        matches!(
            half.kind,
            ExpressionKind::FixedScalar(scalar) if scalar.as_f64().is_some_and(|value| value.to_bits() == crate::compiler_frontend::numeric_text::binary16::round_f64_to_f16(0.1).to_bits())
        ),
        "expected F16(0.1) bits, got {:?}",
        half.kind
    );

    let single = first_start_declaration_expression("single F32 = 0.1\n");
    assert_eq!(
        single.diagnostic_type,
        DataType::FixedScalar(FixedScalar::F32)
    );
    assert!(
        matches!(
            single.kind,
            ExpressionKind::FixedScalar(scalar) if scalar.as_f64().is_some_and(|value| value == f64::from(0.1f32))
        ),
        "expected F32(0.1) bits, got {:?}",
        single.kind
    );
}

#[test]
fn direct_fixed_literal_materialises_whole_literal_into_f64() {
    let value = first_start_declaration_expression("wide F64 = 2\n");

    assert_eq!(
        value.diagnostic_type,
        DataType::FixedScalar(FixedScalar::F64)
    );
    assert!(
        matches!(
            value.kind,
            ExpressionKind::FixedScalar(scalar) if scalar.as_f64() == Some(2.0)
        ),
        "expected F64(2.0), got {:?}",
        value.kind
    );
}

#[test]
fn profile_float_whole_literals_materialise_directly_with_float_identity_and_signed_zero() {
    let large = first_start_declaration_expression("large Float = 18_446_744_073_709_551_615\n");
    assert_eq!(large.type_id, builtin_type_ids::FLOAT);
    assert!(
        matches!(large.kind, ExpressionKind::Float(value) if value == 18_446_744_073_709_551_615_f64),
        "expected a profile Float beyond Int range, got {:?}",
        large.kind
    );

    let negative_zero = first_start_declaration_expression("negative Float = -0\n");
    assert_eq!(negative_zero.type_id, builtin_type_ids::FLOAT);
    assert!(
        matches!(negative_zero.kind, ExpressionKind::Float(value) if value.to_bits() == (-0.0_f64).to_bits()),
        "expected a sign-preserving profile Float zero, got {:?}",
        negative_zero.kind
    );

    let optional =
        first_start_declaration_expression("maybe Float? = 18_446_744_073_709_551_615\n");
    let ExpressionKind::Coerced { value: inner, .. } = &optional.kind else {
        panic!(
            "optional Float should wrap the directly materialised value, got {:?}",
            optional.kind
        );
    };
    assert_eq!(inner.type_id, builtin_type_ids::FLOAT);
    assert!(matches!(
        &inner.kind,
        ExpressionKind::Float(value) if *value == 18_446_744_073_709_551_615_f64
    ));

    let f32 = first_start_declaration_expression("single F32 = 18_446_744_073_709_551_615\n");
    assert_eq!(
        f32.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::F32)
    );
    assert!(matches!(f32.kind, ExpressionKind::FixedScalar(_)));

    let f64 = first_start_declaration_expression("double F64 = 18_446_744_073_709_551_615\n");
    assert_eq!(
        f64.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    assert!(matches!(f64.kind, ExpressionKind::FixedScalar(_)));
}

#[test]
fn profile_float_peer_materialises_whole_literals_without_int_range_narrowing() {
    let sum = nth_start_declaration_expression(
        "peer Float = 1.0\nsum = peer + 18_446_744_073_709_551_615\n",
        1,
    );

    assert_eq!(sum.type_id, builtin_type_ids::FLOAT);
    let ExpressionKind::Runtime(rpn) = &sum.kind else {
        panic!("a variable-plus-literal expression should remain runtime RPN");
    };
    assert!(
        rpn.items.iter().any(|item| matches!(
            item,
            ExpressionRpnItem::Operand(operand)
                if matches!(operand.kind, ExpressionKind::Float(value) if value == 18_446_744_073_709_551_615_f64)
        )),
        "the immediate Float peer should materialise the whole literal as profile Float"
    );
}

#[test]
fn numeric_receiving_context_keeps_integer_rules_and_does_not_retag_operator_results() {
    let int_overflow = parse_single_file_ast_diagnostic("large Int = 18_446_744_073_709_551_615\n");
    assert!(matches!(
        int_overflow.payload,
        DiagnosticPayload::InvalidNumberLiteral {
            reason: NumberLiteralErrorReason::OutsideIntRange,
            ..
        }
    ));

    let decimal_int = parse_single_file_ast_diagnostic("value Int = 1.0\n");
    assert!(matches!(
        decimal_int.payload,
        DiagnosticPayload::TypeMismatch {
            context: TypeMismatchContext::Declaration,
            ..
        }
    ));

    let unconstrained = first_start_declaration_expression("value = 1 + 1\n");
    assert_eq!(unconstrained.type_id, builtin_type_ids::INT);
    assert!(matches!(unconstrained.kind, ExpressionKind::Int(2)));

    let operator_result = parse_single_file_ast_diagnostic("value F32 = 1 + 1\n");
    assert!(matches!(
        operator_result.payload,
        DiagnosticPayload::TypeMismatch {
            context: TypeMismatchContext::Declaration,
            ..
        }
    ));
}

#[test]
fn direct_fixed_literal_materialises_option_of_fixed() {
    let value = nth_start_declaration_expression("maybe U8? = 200\n", 0);

    let ExpressionKind::Coerced { value: inner, .. } = &value.kind else {
        panic!(
            "option-of-fixed should wrap the materialised literal, got {:?}",
            value.kind
        );
    };
    assert!(
        matches!(
            inner.kind,
            ExpressionKind::FixedScalar(scalar) if scalar == FixedScalarValue::unsigned(FixedScalar::U8, 200).expect("U8 literal should materialise before wrapping")
        ),
        "expected U8(200) inside the option wrap, got {:?}",
        inner.kind
    );
}

#[test]
fn operator_result_into_fixed_reports_declaration_mismatch() {
    let diagnostic = parse_single_file_ast_diagnostic("small U8 = 1 + 1\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::TypeMismatch {
            context: TypeMismatchContext::Declaration,
            ..
        }
    ));
}

#[test]
fn large_operator_result_into_u64_reports_int_range_error() {
    let diagnostic = parse_single_file_ast_diagnostic("large U64 = 18_000_000_000 + 1\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidNumberLiteral {
            reason: NumberLiteralErrorReason::OutsideIntRange,
            ..
        }
    ));
}

#[test]
fn decimal_spelling_into_integer_fixed_reports_declaration_mismatch() {
    for source in ["small U8 = 1.0\n", "raw Byte = 1.0\n"] {
        let diagnostic = parse_single_file_ast_diagnostic(source);
        assert!(
            matches!(
                diagnostic.payload,
                DiagnosticPayload::TypeMismatch {
                    context: TypeMismatchContext::Declaration,
                    ..
                }
            ),
            "expected a Declaration mismatch for {source:?}"
        );
    }
}

#[test]
fn grouped_literal_into_fixed_materialises_destination() {
    let value = first_start_declaration_expression("small U8 = (5)\n");

    assert!(
        matches!(
            value.kind,
            ExpressionKind::FixedScalar(scalar) if scalar == FixedScalarValue::unsigned(FixedScalar::U8, 5).expect("grouped U8 literal should materialise")
        ),
        "expected U8(5), got {:?}",
        value.kind
    );
}

#[test]
fn grouped_operator_result_into_fixed_is_not_retagged() {
    let diagnostic = parse_single_file_ast_diagnostic("small U8 = (5) + 1\n");

    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::TypeMismatch {
                context: TypeMismatchContext::Declaration,
                ..
            }
        ),
        "grouped operator result must keep the Declaration mismatch, got {:?}",
        diagnostic.payload
    );
}

#[test]
fn parser_negation_into_signed_minimum_reports_declaration_mismatch() {
    let diagnostic = parse_single_file_ast_diagnostic("small I8 = -(128)\n");

    assert!(
        !matches!(
            diagnostic.payload,
            DiagnosticPayload::InvalidNumberLiteral { .. }
        ),
        "parser-owned negation is an operator result, not a fixed range error: {:?}",
        diagnostic.payload
    );
    assert!(
        matches!(
            diagnostic.payload,
            DiagnosticPayload::TypeMismatch {
                context: TypeMismatchContext::Declaration,
                ..
            }
        ),
        "expected a Declaration mismatch, got {:?}",
        diagnostic.payload
    );
}

#[test]
fn curly_literal_into_fixed_reports_baseline_mismatch() {
    let diagnostic = parse_single_file_ast_diagnostic("small U8 = {1}\n");

    let DiagnosticPayload::TypeMismatch {
        expected,
        found,
        context,
    } = diagnostic.payload
    else {
        panic!("expected a type mismatch, got {:?}", diagnostic.payload);
    };

    assert_eq!(context, TypeMismatchContext::Declaration);
    assert_ne!(found, builtin_type_ids::STRING);
    assert_eq!(expected, builtin_type_ids::fixed_scalar(FixedScalar::U8));
}

#[test]
fn unannotated_overflow_keeps_default_int_range_error() {
    let diagnostic = parse_single_file_ast_diagnostic("value = 3_000_000_000\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::InvalidNumberLiteral {
            reason: NumberLiteralErrorReason::OutsideIntRange,
            ..
        }
    ));
}

#[test]
fn fixed_width_arithmetic_uses_canonical_promotion_domains() {
    let cases = [
        ("U8", "+", "U8", FixedScalar::U32),
        ("I32", "+", "U32", FixedScalar::I64),
        ("I8", "*", "I16", FixedScalar::I32),
        ("U32", "-", "U64", FixedScalar::U64),
        ("U8", "/", "U8", FixedScalar::F64),
        ("U8", "//", "U8", FixedScalar::U32),
        ("I64", "%", "I8", FixedScalar::I64),
        ("F16", "*", "F16", FixedScalar::F32),
        ("F16", "+", "F64", FixedScalar::F64),
    ];

    for (left, operator, right, result) in cases {
        assert_binary_result_type(
            left,
            operator,
            right,
            builtin_type_ids::fixed_scalar(result),
        );
    }

    assert_binary_result_type("Int", "+", "Int", builtin_type_ids::INT);
    assert_binary_result_type("Float", "+", "Float", builtin_type_ids::FLOAT);
    assert_binary_result_type("Int", "+", "Float", builtin_type_ids::FLOAT);
    assert_binary_result_type("Float", "+", "Int", builtin_type_ids::FLOAT);
    assert_binary_result_type("Int", "/", "Int", builtin_type_ids::FLOAT);
}

#[test]
fn unsupported_fixed_width_arithmetic_keeps_operator_diagnostics() {
    let cases = [
        ("I64", "+", "U64", DiagnosticOperator::Add),
        ("I32", "+", "U64", DiagnosticOperator::Add),
        ("U8", "+", "Int", DiagnosticOperator::Add),
        ("Int", "+", "U8", DiagnosticOperator::Add),
        ("F32", "+", "Float", DiagnosticOperator::Add),
        ("U8", "+", "F32", DiagnosticOperator::Add),
        ("Byte", "+", "Byte", DiagnosticOperator::Add),
        ("F32", "//", "F32", DiagnosticOperator::IntDivide),
    ];

    for (left, operator, right, diagnostic_operator) in cases {
        assert_binary_rejected(left, operator, right, diagnostic_operator);
    }
}

#[test]
fn fixed_width_comparisons_use_compatibility_without_arithmetic_promotion() {
    for (left, operator, right) in [
        ("Byte", "is", "Byte"),
        ("Byte", "<", "Byte"),
        ("I64", "<", "U64"),
        ("F16", "is", "F64"),
    ] {
        assert_binary_result_type(left, operator, right, builtin_type_ids::BOOL);
    }

    assert_binary_rejected("Byte", "is", "U8", DiagnosticOperator::Equality);
    assert_binary_rejected("U8", "is", "Int", DiagnosticOperator::Equality);
}

#[test]
fn unary_minus_uses_the_numeric_negation_domain() {
    for (operand_type, result_scalar) in [
        ("I8", FixedScalar::I32),
        ("F16", FixedScalar::F32),
        ("I64", FixedScalar::I64),
    ] {
        let source = format!("operand {operand_type} = 1\nvalue = -operand\n");
        let value = nth_start_declaration_expression(&source, 1);
        assert_eq!(
            value.type_id,
            builtin_type_ids::fixed_scalar(result_scalar),
            "unexpected negation result for {operand_type}"
        );
    }

    assert_unsupported_operator("zero U8 = 0\nvalue = -zero\n", DiagnosticOperator::Subtract);
    assert_unsupported_operator(
        "zero #U8 = 0\nvalue = -zero\n",
        DiagnosticOperator::Subtract,
    );
    assert_unsupported_operator(
        "octet Byte = 0\nvalue = -octet\n",
        DiagnosticOperator::Subtract,
    );
}

#[test]
fn pending_numeric_literals_materialise_from_immediate_fixed_peers() {
    let right_peer = nth_start_declaration_expression("x U8 = 5\nvalue = x + 1\n", 1);
    assert_eq!(
        right_peer.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::U32)
    );
    assert_runtime_contains_fixed_literal(
        &right_peer,
        FixedScalarValue::unsigned(FixedScalar::U8, 1).expect("U8 literal should materialise"),
    );

    let left_peer = nth_start_declaration_expression("x U8 = 5\nvalue = 1 + x\n", 1);
    assert_eq!(
        left_peer.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::U32)
    );
    assert_runtime_contains_fixed_literal(
        &left_peer,
        FixedScalarValue::unsigned(FixedScalar::U8, 1).expect("U8 literal should materialise"),
    );

    let float_peer = nth_start_declaration_expression("f F32 = 1\nvalue = f * 0.5\n", 1);
    assert_eq!(
        float_peer.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::F32)
    );
    assert_runtime_contains_fixed_literal(
        &float_peer,
        FixedScalarValue::binary_float(FixedScalar::F32, 0.5)
            .expect("0.5 is exactly representable as F32"),
    );

    let byte_comparison =
        nth_start_declaration_expression("raw Byte = 255\nvalue = raw is 255\n", 1);
    assert_eq!(byte_comparison.type_id, builtin_type_ids::BOOL);
    assert_runtime_contains_fixed_literal(
        &byte_comparison,
        FixedScalarValue::unsigned(FixedScalar::Byte, 255)
            .expect("Byte maximum should materialise"),
    );

    let chained = nth_start_declaration_expression("x U8 = 5\nvalue = x + 1 + 1\n", 1);
    assert_eq!(
        chained.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::U32)
    );

    let grouped_literal = nth_start_declaration_expression("x U8 = 5\nvalue = x + (1)\n", 1);
    assert_eq!(
        grouped_literal.type_id,
        builtin_type_ids::fixed_scalar(FixedScalar::U32)
    );
    assert_runtime_contains_fixed_literal(
        &grouped_literal,
        FixedScalarValue::unsigned(FixedScalar::U8, 1).expect("U8 literal should materialise"),
    );
}

#[test]
fn peer_literal_materialisation_preserves_natural_and_grouped_operator_types() {
    let out_of_range = parse_single_file_ast_diagnostic("x U8 = 5\nvalue = x + 300\n");
    assert!(matches!(
        out_of_range.payload,
        DiagnosticPayload::InvalidNumberLiteral {
            reason: NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::U8),
            ..
        }
    ));

    assert_unsupported_operator("x U8 = 5\nvalue = x + 1.5\n", DiagnosticOperator::Add);
    assert_unsupported_operator("x U8 = 5\nvalue = x + (1 + 2)\n", DiagnosticOperator::Add);
    assert_unsupported_operator("x U8 = 5\nvalue = 1 + 1 + x\n", DiagnosticOperator::Add);
    assert_unsupported_operator("x U8 = 5\nvalue = x to 10\n", DiagnosticOperator::Range);
}

#[test]
fn optional_fixed_scalar_does_not_materialise_pending_peers_as_inner_scalars() {
    let diagnostic = parse_single_file_ast_diagnostic(
        "optional_value U8? = none\nresult = optional_value + 1\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnsupportedOperatorTypes {
            operator: DiagnosticOperator::Add,
            rhs: Some(type_id),
            ..
        } if type_id == builtin_type_ids::INT
    ));
}

#[test]
fn number_operator_domains_keep_scale_and_reject_invalid_families() {
    let add = binary_expression("Dec2", "+", "Int");
    assert!(matches!(add.diagnostic_type, DataType::Number(scale) if scale.get() == 2));

    let reversed_subtract = binary_expression("U64", "-", "Dec2");
    assert!(matches!(
        reversed_subtract.diagnostic_type,
        DataType::Number(scale) if scale.get() == 2
    ));

    let mixed_comparison = binary_expression("Dec2", "<", "I64");
    assert_eq!(mixed_comparison.type_id, builtin_type_ids::BOOL);

    let int_power = binary_expression("Dec2", "^", "Int");
    assert!(matches!(
        int_power.diagnostic_type,
        DataType::Number(scale) if scale.get() == 2
    ));

    let decimal_division = binary_expression("Dec2", "/", "Dec2");
    assert!(matches!(
        decimal_division.diagnostic_type,
        DataType::Number(scale) if scale.get() == 2
    ));

    let integer_division = binary_expression("Dec0", "//", "Dec0");
    assert!(matches!(
        integer_division.diagnostic_type,
        DataType::Number(scale) if scale.get() == 0
    ));

    let remainder = binary_expression("Dec2", "%", "Dec2");
    assert!(matches!(
        remainder.diagnostic_type,
        DataType::Number(scale) if scale.get() == 2
    ));

    for (left, operator, right, diagnostic_operator) in [
        ("Dec", "/", "Dec", DiagnosticOperator::Divide),
        ("Dec2", "//", "Dec2", DiagnosticOperator::IntDivide),
        ("Dec2", "+", "Dec3", DiagnosticOperator::Add),
        ("Dec2", "+", "Float", DiagnosticOperator::Add),
        ("Dec2", "+", "Byte", DiagnosticOperator::Add),
        ("Dec2", "^", "Dec2", DiagnosticOperator::Exponent),
        ("Dec2", "^", "I64", DiagnosticOperator::Exponent),
    ] {
        assert_binary_rejected(left, operator, right, diagnostic_operator);
    }
}

#[test]
fn pending_number_literals_use_the_immediate_number_peer_in_both_orders() {
    let source =
        "base ~Dec2 = 1.25\nright = base + 1\nleft = 1 + base\nsame = base is 1\nnegated = -base\n";
    let base = first_start_declaration_expression(source);
    let right = nth_start_declaration_expression(source, 1);
    let left = nth_start_declaration_expression(source, 2);
    let same = nth_start_declaration_expression(source, 3);
    let negated = nth_start_declaration_expression(source, 4);

    for expression in [&right, &left] {
        assert_eq!(expression.type_id, base.type_id);
        assert_runtime_contains_number_literal(expression, base.type_id, "100", 2);
    }
    assert_eq!(same.type_id, builtin_type_ids::BOOL);
    assert_runtime_contains_number_literal(&same, base.type_id, "100", 2);
    assert_eq!(negated.type_id, base.type_id);
    assert!(matches!(
        &negated.diagnostic_type,
        DataType::Number(scale) if scale.get() == 2
    ));
}

#[test]
fn number_power_keeps_pending_and_typed_exponents_in_profile_int() {
    let source = concat!(
        "base ~Dec2 = 1.5\n",
        "plain = base ^ 2\n",
        "negative = base ^ -2\n",
        "parenthesized = base ^ (2)\n",
        "negative_grouped = base ^ -(2)\n",
        "typed_result Dec2 = base ^ 2\n",
    );
    let base = first_start_declaration_expression(source);
    for index in 1..=5 {
        let expression = nth_start_declaration_expression(source, index);
        assert_eq!(expression.type_id, base.type_id);
        assert!(matches!(
            &expression.diagnostic_type,
            DataType::Number(scale) if scale.get() == 2
        ));
        assert_runtime_number_power_keeps_int_exponent(&expression);
    }

    let parameter_source = concat!(
        "power |base Dec2, exponent Int| -> Dec2:\n",
        "    return base ^ exponent\n",
        ";\n",
    );
    let (ast, path_fork, string_table) = parse_single_file_ast(parameter_source);
    let body = function_body_by_name(&ast, &path_fork, &string_table, "power");
    let NodeKind::Return(values) = &body[0].kind else {
        panic!("the typed-parameter function should return the power expression");
    };
    let expression = &values[0];
    assert_eq!(
        ast.type_environment.number_scale(expression.type_id),
        Some(NumberScale::new(2).expect("test Dec scale is valid"))
    );
    let ExpressionKind::Runtime(rpn) = &expression.kind else {
        panic!("a typed Int parameter keeps the power expression runtime-dependent");
    };
    let exponent_index = rpn
        .items
        .iter()
        .position(|item| {
            matches!(
                item,
                ExpressionRpnItem::Operator {
                    operator: Operator::Exponent,
                    ..
                }
            )
        })
        .expect("the function return should contain Dec power");
    let Some(ExpressionRpnItem::Operand(exponent)) = exponent_index
        .checked_sub(1)
        .and_then(|index| rpn.items.get(index))
    else {
        panic!("the typed exponent should immediately precede the power operator");
    };
    assert_eq!(exponent.type_id, builtin_type_ids::INT);
}

fn assert_binary_result_type(
    left_type: &str,
    operator: &str,
    right_type: &str,
    expected_type: TypeId,
) {
    let expression = binary_expression(left_type, operator, right_type);
    assert_eq!(
        expression.type_id, expected_type,
        "{left_type} {operator} {right_type} resolved to {:?}",
        expression.diagnostic_type
    );
}

fn assert_binary_rejected(
    left_type: &str,
    operator: &str,
    right_type: &str,
    diagnostic_operator: DiagnosticOperator,
) {
    let source =
        format!("left {left_type} = 1\nright {right_type} = 2\nvalue = left {operator} right\n");
    assert_unsupported_operator(&source, diagnostic_operator);
}

fn binary_expression(left_type: &str, operator: &str, right_type: &str) -> Expression {
    let source =
        format!("left {left_type} = 1\nright {right_type} = 2\nvalue = left {operator} right\n");
    nth_start_declaration_expression(&source, 2)
}

fn assert_runtime_contains_fixed_literal(expression: &Expression, expected: FixedScalarValue) {
    let ExpressionKind::Runtime(rpn) = &expression.kind else {
        panic!("expected a runtime expression, got {:?}", expression.kind);
    };

    assert!(
        rpn.items.iter().any(|item| matches!(
            item,
            ExpressionRpnItem::Operand(operand)
                if matches!(operand.kind, ExpressionKind::FixedScalar(value) if value == expected)
        )),
        "expected runtime RPN to contain {expected}, got {:?}",
        rpn.items
    );
}

fn assert_runtime_contains_number_literal(
    expression: &Expression,
    expected_type_id: TypeId,
    expected_coefficient: &str,
    expected_scale: u16,
) {
    let ExpressionKind::Runtime(rpn) = &expression.kind else {
        panic!("expected a runtime expression, got {:?}", expression.kind);
    };

    assert!(
        rpn.items.iter().any(|item| matches!(
            item,
            ExpressionRpnItem::Operand(operand)
                if operand.type_id == expected_type_id
                    && matches!(
                        &operand.kind,
                        ExpressionKind::Number(value)
                            if value.coefficient().to_string() == expected_coefficient
                                && value.scale().get() == expected_scale
                    )
        )),
        "expected runtime RPN to contain Number({expected_coefficient}, scale {expected_scale}), got {:?}",
        rpn.items
    );
}

fn assert_runtime_number_power_keeps_int_exponent(expression: &Expression) {
    let ExpressionKind::Runtime(rpn) = &expression.kind else {
        panic!("expected a runtime expression, got {:?}", expression.kind);
    };

    assert!(
        rpn.items.iter().any(|item| matches!(
            item,
            ExpressionRpnItem::Operand(operand)
                if operand.type_id == builtin_type_ids::INT
                    && matches!(&operand.kind, ExpressionKind::Int(_))
        )),
        "expected profile-Int exponent operand, got {:?}",
        rpn.items
    );
    assert!(
        !rpn.items.iter().any(|item| matches!(
            item,
            ExpressionRpnItem::Operand(operand)
                if matches!(&operand.kind, ExpressionKind::Number(_))
        )),
        "pending power exponent must not be materialised as Number: {:?}",
        rpn.items
    );
}
