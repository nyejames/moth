//! Focused tests for runtime Float formatting lowering (Phase 7).
//!
//! WHAT: verifies that `cast Float -> String` and runtime Float template interpolation lower
//!       through `HirStatementKind::FormatFloat` instead of plain `HirExpressionKind::Cast` or
//!       target-native string coercion.
//! WHY: Float formatting is a Moth-owned contract shared by casts and templates; dedicated
//!      tests guard against regressions back to native stringification.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression_kind::ResolvedCastExpression;
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::builtins::casts::targets::{BuiltinCastPolicyId, BuiltinCastTarget};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_builder::{
    register_local, runtime_template_expression, setup_builder,
};
use crate::compiler_frontend::hir::ids::{FunctionId, LocalId};
use crate::compiler_frontend::hir::numeric::NumericFailureMode;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::tests::symbol;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use crate::compiler_frontend::tests::ast_fixture_support::reference_expr_with_type_id;
use crate::compiler_frontend::tests::hir_fixture_support::lower_hir;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::value_mode::ValueMode;

fn float_expr(
    value: f64,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> Expression {
    Expression::float(value, span, ValueMode::ImmutableOwned)
}

fn string_expr(
    value: &str,
    string_table: &mut StringTable,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> Expression {
    Expression::string_slice(string_table.intern(value), span, ValueMode::ImmutableOwned)
}

fn find_format_float_statements(
    builder: &HirBuilder<'_>,
) -> Vec<(
    NumericFailureMode,
    crate::compiler_frontend::datatypes::ids::TypeId,
)> {
    builder
        .module
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match &statement.kind {
            HirStatementKind::FormatFloat {
                failure_mode,
                result,
                ..
            } => {
                let result_type = builder
                    .local_type_id_or_error(*result, &statement.span)
                    .ok();
                result_type.map(|ty| (*failure_mode, ty))
            }
            _ => None,
        })
        .collect()
}

fn has_plain_float_to_string_cast(builder: &HirBuilder<'_>) -> bool {
    builder
        .module
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .any(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::CastOp {
                    policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Float),
                    ..
                }
            )
        })
        || builder
            .module
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match &statement.kind {
                HirStatementKind::Assign { value, .. } => Some(value),
                _ => None,
            })
            .any(|value| {
                matches!(
                    &value.kind,
                    HirExpressionKind::Cast {
                        policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Float),
                        ..
                    }
                )
            })
}

fn make_float_to_string_cast(
    source: Expression,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> Expression {
    let cast = ResolvedCastExpression {
        source: Box::new(source),
        source_type_id: builtin_type_ids::FLOAT,
        target_type_id: builtin_type_ids::STRING,
        target: BuiltinCastTarget::String,
        requires_optional_wrap_after_cast: false,
        evidence: ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Float),
        },
        handling: CastHandling::Infallible,
        span,
    };

    Expression::cast(cast, builtin_type_ids::STRING, &TypeEnvironment::new())
}

#[test]
fn float_validation_distinguishes_compiler_entry_from_declared_error_contract() {
    for compiler_entry in [false, true] {
        let mut path_fork = super::PathInternerFork::empty();
        let mut string_table = StringTable::new();
        let name = symbol("validated_float", &mut path_fork, &mut string_table);
        let mut builder = setup_builder(&mut string_table, &mut path_fork);
        let error_type = builder.test_register_builtin_error_type();
        let return_type = builder
            .type_environment
            .intern_fallible_carrier(builtin_type_ids::FLOAT, error_type);
        let function_id = FunctionId(1);
        builder.test_register_function_with_return_type(name, function_id, return_type);
        builder.test_set_current_function(function_id);
        if compiler_entry {
            builder.module.start_function = Some(function_id);
        }
        let source = builder
            .lower_expression(&float_expr(1.5, None))
            .expect("finite Float source must lower")
            .value;
        builder
            .emit_validated_float_value(source, &None)
            .expect("Float boundary guard must lower");
        let validation_modes: Vec<_> = builder
            .module
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match statement.kind {
                HirStatementKind::ValidateFloat { failure_mode, .. } => Some(failure_mode),
                _ => None,
            })
            .collect();
        let expected = if compiler_entry {
            NumericFailureMode::Trap
        } else {
            NumericFailureMode::ReturnError
        };
        assert_eq!(validation_modes, vec![expected]);
    }
}

#[test]
fn float_formatting_in_synthetic_error_entry_keeps_integrity_trap() {
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(
        "identity |value Float| -> Float:\n    return value\n;\n\
         value = identity(1.5)\n[:[value]]\n",
    );
    let module = lower_hir(ast, &mut string_table, &mut path_fork);
    let format_modes: Vec<_> = module
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match statement.kind {
            HirStatementKind::FormatFloat { failure_mode, .. } => Some(failure_mode),
            _ => None,
        })
        .collect();

    assert_eq!(format_modes, vec![NumericFailureMode::Trap]);
}

#[test]
fn float_formatting_source_declared_error_contract_keeps_return_error() {
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(
        "format_value |value Float| -> String, Error!:\n    text String = cast value\n    return text\n;\n",
    );
    let module = lower_hir(ast, &mut string_table, &mut path_fork);
    let format_modes: Vec<_> = module
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match statement.kind {
            HirStatementKind::FormatFloat { failure_mode, .. } => Some(failure_mode),
            _ => None,
        })
        .collect();

    assert_eq!(format_modes, vec![NumericFailureMode::ReturnError]);
}

#[test]
fn cast_float_to_string_lowers_to_format_float_statement() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let loc = None;
    let source = float_expr(1.5, loc);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let expr = make_float_to_string_cast(source, loc);

    let lowered = builder
        .lower_expression(&expr)
        .expect("Float -> String cast lowering should succeed");

    assert_eq!(lowered.value.ty, builtin_type_ids::STRING);
    assert!(
        !has_plain_float_to_string_cast(&builder),
        "Float -> String cast must not lower to a plain Cast expression or CastOp statement"
    );

    let format_floats = find_format_float_statements(&builder);
    assert_eq!(
        format_floats.len(),
        1,
        "Float -> String cast should emit exactly one FormatFloat statement"
    );
    assert!(matches!(format_floats[0].0, NumericFailureMode::Trap));
}

#[test]
fn cast_float_to_string_flushes_source_prelude_before_formatting() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let loc = None;
    let source_name = symbol("source_float", &mut path_fork, &mut string_table);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    builder.test_register_function_name(source_name, FunctionId(7));

    let source = Expression::function_call_with_typed_arguments(
        source_name,
        vec![],
        vec![builtin_type_ids::FLOAT],
        &mut builder.type_environment,
        loc,
    );
    let expr = make_float_to_string_cast(source, loc);

    let lowered = builder
        .lower_expression(&expr)
        .expect("Float -> String cast source prelude lowering should succeed");

    assert!(
        lowered.prelude.is_empty(),
        "Float formatting emits into the active block, so the source prelude must be flushed there"
    );

    let statements = builder.test_current_block_statements();
    assert_eq!(
        statements.len(),
        2,
        "function-call source should run before the FormatFloat statement"
    );

    assert!(
        matches!(
            &statements[0].kind,
            HirStatementKind::Call {
                target: CallTarget::Local(FunctionId(7)),
                ..
            }
        ),
        "source call should be emitted before formatting"
    );
    assert!(
        matches!(&statements[1].kind, HirStatementKind::FormatFloat { .. }),
        "FormatFloat should consume the source call result after it exists"
    );
}

#[test]
fn cast_float_to_string_return_error_in_builtin_error_function() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let loc = None;
    let fn_name = symbol("__test_fn_error", &mut path_fork, &mut string_table);
    let source = float_expr(1.5, loc);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let error_type_id = builder.test_register_builtin_error_type();
    let return_type = builder
        .type_environment
        .intern_fallible_carrier(builtin_type_ids::STRING, error_type_id);
    builder.test_register_function_with_return_type(fn_name, FunctionId(1), return_type);
    builder.test_set_current_function(FunctionId(1));

    let expr = make_float_to_string_cast(source, loc);
    let lowered = builder
        .lower_expression(&expr)
        .expect("Float -> String cast lowering in Error! function should succeed");

    let format_floats = find_format_float_statements(&builder);
    assert_eq!(format_floats.len(), 1);
    assert!(
        matches!(format_floats[0].0, NumericFailureMode::ReturnError),
        "builtin Error! functions should use ReturnError Float formatting failure mode"
    );
    assert!(
        matches!(
            lowered.value.kind,
            HirExpressionKind::FallibleUnwrapSuccess { .. }
        ),
        "recoverable Float formatting should continue with an unwrapped success value"
    );
    assert!(
        builder
            .module
            .blocks
            .iter()
            .any(|block| matches!(block.terminator, HirTerminator::FallibleBranch { .. })),
        "recoverable Float formatting should branch on the internal carrier"
    );
    assert!(
        builder
            .module
            .blocks
            .iter()
            .any(|block| matches!(block.terminator, HirTerminator::ReturnError(_))),
        "recoverable Float formatting should emit a builtin Error return edge"
    );
}

#[test]
fn runtime_float_template_interpolation_lowers_to_format_float_statement() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let loc = None;
    let value_name = symbol("value", &mut path_fork, &mut string_table);
    let value_ref = reference_expr_with_type_id(
        value_name,
        builtin_type_ids::FLOAT,
        loc,
        ValueMode::ImmutableReference,
    );

    let expr = runtime_template_expression(loc, vec![value_ref], &string_table);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    register_local(
        &mut builder,
        value_name,
        LocalId(10),
        builtin_type_ids::FLOAT,
        loc,
    );

    let _lowered = builder
        .lower_expression(&expr)
        .expect("Float template interpolation lowering should succeed");

    let format_floats = find_format_float_statements(&builder);
    assert_eq!(
        format_floats.len(),
        1,
        "runtime Float template interpolation should emit exactly one FormatFloat statement"
    );
    assert!(matches!(format_floats[0].0, NumericFailureMode::Trap));
}

#[test]
fn runtime_string_template_chunk_does_not_emit_format_float() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let loc = None;
    let text = string_expr("hello", &mut string_table, loc);

    let expr = runtime_template_expression(loc, vec![text], &string_table);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let _lowered = builder
        .lower_expression(&expr)
        .expect("String template chunk lowering should succeed");

    let format_floats = find_format_float_statements(&builder);
    assert!(
        format_floats.is_empty(),
        "String template chunks must not emit FormatFloat statements"
    );
}

#[test]
fn cast_float_to_string_optional_wrap_lowers_to_format_float() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let loc = None;
    let source = float_expr(1.5, loc);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let optional_string_type = builder
        .type_environment
        .intern_option(builtin_type_ids::STRING);

    let cast = ResolvedCastExpression {
        source: Box::new(source),
        source_type_id: builtin_type_ids::FLOAT,
        target_type_id: builtin_type_ids::STRING,
        target: BuiltinCastTarget::String,
        requires_optional_wrap_after_cast: true,
        evidence: ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Float),
        },
        handling: CastHandling::Infallible,
        span: loc,
    };

    let expr = Expression::cast(cast, optional_string_type, &builder.type_environment);

    let lowered = builder
        .lower_expression(&expr)
        .expect("optional Float -> String cast lowering should succeed");

    assert_eq!(lowered.value.ty, optional_string_type);

    let is_option_some = matches!(
        &lowered.value.kind,
        HirExpressionKind::VariantConstruct {
            carrier: crate::compiler_frontend::hir::expressions::HirVariantCarrier::Option,
            ..
        }
    );
    assert!(
        is_option_some,
        "optional Float -> String cast should wrap the formatted string in some(...)"
    );

    let format_floats = find_format_float_statements(&builder);
    assert_eq!(
        format_floats.len(),
        1,
        "optional Float -> String cast should still emit exactly one FormatFloat statement"
    );
}
