//! Invariants for expression failure side data and recovered contributor boundaries.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::assertion_message_effects::{
    EnclosingExitEffect, classify_assertion_message_effect, pending_expression_failure_facts,
};
use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind, FallibleHandling};
use crate::compiler_frontend::ast::expressions::failure_facts::{ExpressionFailureFacts, FailureDisposition};
use crate::compiler_frontend::ast::statements::value_production::types::{ProducedValues, ValueBlock, ValueCatchBlock};
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::builtins::error_type::builtin_error_type_path;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::value_mode::ValueMode;

fn runtime_checked_value(operator: NumericOperator) -> Expression {
    let mut expression = Expression::new(
        ExpressionKind::Reference(PathId::ROOT),
        None,
        builtin_type_ids::INT,
        DataType::Int,
        ValueMode::ImmutableOwned,
    );
    expression.failure_facts.record_numeric_operation(operator, NumericScalar::Int, None);
    expression
}

#[test]
fn arithmetic_codes_exclude_boundary_and_format_invariants() {
    let mut facts = ExpressionFailureFacts::default();
    facts.record_numeric_operation(NumericOperator::Power, NumericScalar::Int, None);
    assert_eq!(facts.implicit[0].codes, vec![BuiltinErrorCode::IntOverflow, BuiltinErrorCode::InvalidExponent]);

    let mut remainder = ExpressionFailureFacts::default();
    remainder.record_numeric_operation(NumericOperator::Remainder, NumericScalar::Int, None);
    assert_eq!(remainder.implicit[0].codes, vec![BuiltinErrorCode::DivideByZero]);

    let mut float = ExpressionFailureFacts::default();
    float.record_numeric_operation(NumericOperator::Divide, NumericScalar::Float, None);
    assert_eq!(float.implicit[0].codes, vec![BuiltinErrorCode::DivideByZero, BuiltinErrorCode::FloatNonFinite]);
}

#[test]
fn call_keeps_argument_failure_without_changing_success_type() {
    let argument = runtime_checked_value(NumericOperator::Add);
    let expected = argument.failure_facts.clone();
    let expression = Expression::new(
        ExpressionKind::FunctionCall {
            name: PathId::ROOT,
            args: vec![CallArgument::positional(argument, CallAccessMode::Shared, None)],
            result_type_ids: vec![builtin_type_ids::STRING],
        },
        None,
        builtin_type_ids::STRING,
        DataType::StringSlice,
        ValueMode::ImmutableOwned,
    );
    assert_eq!(expression.type_id, builtin_type_ids::STRING);
    assert_eq!(expression.failure_facts, expected);
    assert_eq!(
        classify_assertion_message_effect(&expression, &TemplateIrStore::new()).expect("valid AST"),
        Some(EnclosingExitEffect::InferredFailure(None)),
    );
}

#[test]
fn catch_handler_failure_goes_outward_without_reentering_protected_work() {
    let (ast, mut path_fork, mut string_table) =
        parse_single_file_ast("typed || -> Int, Error!:\n    return 1\n;\n");
    let error_path = builtin_error_type_path(&mut path_fork, &mut string_table);
    let error_type_id = ast
        .type_environment
        .nominal_id_for_path(&error_path)
        .and_then(|nominal_id| ast.type_environment.type_id_for_nominal_id(nominal_id))
        .expect("the fixture resolves canonical builtin Error");
    let mut protected = runtime_checked_value(NumericOperator::Add);
    protected.failure_facts.disposition = FailureDisposition::HandledByCatch {
        error_type_id,
    };
    let handler_value = runtime_checked_value(NumericOperator::IntegerDivide);
    let expected = handler_value.failure_facts.clone();
    let expression = Expression::new(
        ExpressionKind::ValueBlock {
            block: Box::new(ValueBlock::Catch(ValueCatchBlock {
                handled_value: Box::new(protected),
                handler: FallibleHandling::Handler {
                    error: None,
                    body: vec![AstNode {
                        kind: NodeKind::ThenValue(ProducedValues {
                            expressions: vec![handler_value],
                            span: None,
                        }),
                        span: None,
                        scope: PathId::ROOT,
                    }],
                },
                result_type_ids: vec![builtin_type_ids::INT],
            })),
        },
        None,
        builtin_type_ids::INT,
        DataType::Int,
        ValueMode::ImmutableOwned,
    );
    let facts = pending_expression_failure_facts(&expression, &TemplateIrStore::new())
        .expect("valid AST");
    assert_eq!(facts, expected);
}
