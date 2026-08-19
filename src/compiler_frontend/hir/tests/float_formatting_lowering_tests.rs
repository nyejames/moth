//! Focused tests for runtime Float formatting lowering (Phase 7).
//!
//! WHAT: verifies that `cast Float -> String` and runtime Float template interpolation lower
//!       through `HirStatementKind::FormatFloat` instead of plain `HirExpressionKind::Cast` or
//!       target-native string coercion.
//! WHY: Float formatting is a Moth-owned contract shared by casts and templates; dedicated
//!      tests guard against regressions back to native stringification.

use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::expressions::expression_kind::ResolvedCastExpression;
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::ast::templates::template::ReactiveSubscription;
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::builtins::casts::targets::{BuiltinCastPolicyId, BuiltinCastTarget};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_builder::{
    register_local, runtime_template_expression, setup_builder,
};
use crate::compiler_frontend::hir::ids::{FunctionId, LocalId};
use crate::compiler_frontend::hir::numeric::NumericFailureMode;
use crate::compiler_frontend::hir::reactivity::{
    HirReactiveSource, HirReactiveSourceKind, ReactiveSourceId,
};
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::tests::symbol;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::test_source_location;
use crate::compiler_frontend::tests::type_id_fixture_support::reference_expr;
use crate::compiler_frontend::value_mode::ValueMode;

fn float_expr(
    value: f64,
    location: crate::compiler_frontend::ast::ast_nodes::SourceLocation,
) -> Expression {
    Expression::float(value, location, ValueMode::ImmutableOwned)
}

fn string_expr(
    value: &str,
    string_table: &mut StringTable,
    location: crate::compiler_frontend::ast::ast_nodes::SourceLocation,
) -> Expression {
    Expression::string_slice(
        string_table.intern(value),
        location,
        ValueMode::ImmutableOwned,
    )
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
                    .local_type_id_or_error(*result, &statement.location)
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
                    policy: BuiltinCastPolicyId::FloatToString,
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
                        policy: BuiltinCastPolicyId::FloatToString,
                        ..
                    }
                )
            })
}

fn expression_contains_float_to_string_cast(expression: &HirExpression) -> bool {
    match &expression.kind {
        HirExpressionKind::Cast { source, policy } => {
            *policy == BuiltinCastPolicyId::FloatToString
                || expression_contains_float_to_string_cast(source)
        }

        HirExpressionKind::BinOp { left, right, .. } => {
            expression_contains_float_to_string_cast(left)
                || expression_contains_float_to_string_cast(right)
        }

        _ => false,
    }
}

fn make_float_to_string_cast(
    source: Expression,
    location: crate::compiler_frontend::ast::ast_nodes::SourceLocation,
) -> Expression {
    let cast = ResolvedCastExpression {
        source: Box::new(source),
        source_type_id: builtin_type_ids::FLOAT,
        target_type_id: builtin_type_ids::STRING,
        target: BuiltinCastTarget::String,
        requires_optional_wrap_after_cast: false,
        evidence: ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::FloatToString,
        },
        handling: CastHandling::Infallible,
        location: location.clone(),
    };

    Expression::cast(cast, builtin_type_ids::STRING, &TypeEnvironment::new())
}

#[test]
fn cast_float_to_string_lowers_to_format_float_statement() {
    let mut string_table = StringTable::new();
    let loc = test_source_location(1);
    let source = float_expr(1.5, loc.clone());

    let mut builder = setup_builder(&mut string_table);
    let expr = make_float_to_string_cast(source, loc.clone());

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
    let mut string_table = StringTable::new();
    let loc = test_source_location(1);
    let source_name = symbol("source_float", &mut string_table);

    let mut builder = setup_builder(&mut string_table);
    builder.test_register_function_name(source_name.clone(), FunctionId(7));

    let source = Expression::function_call_with_typed_arguments(
        source_name,
        vec![],
        vec![builtin_type_ids::FLOAT],
        &mut builder.type_environment,
        loc.clone(),
    );
    let expr = make_float_to_string_cast(source, loc.clone());

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
    let mut string_table = StringTable::new();
    let loc = test_source_location(1);
    let fn_name = symbol("__test_fn_error", &mut string_table);
    let source = float_expr(1.5, loc.clone());

    let mut builder = setup_builder(&mut string_table);
    let error_type_id = builder.test_register_builtin_error_type();
    let return_type = builder
        .type_environment
        .intern_fallible_carrier(builtin_type_ids::STRING, error_type_id);
    builder.test_register_function_with_return_type(fn_name, FunctionId(1), return_type);
    builder.test_set_current_function(FunctionId(1));

    let expr = make_float_to_string_cast(source, loc.clone());
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
    let mut string_table = StringTable::new();
    let loc = test_source_location(1);
    let value_name = symbol("value", &mut string_table);
    let value_ref = reference_expr(
        value_name.clone(),
        builtin_type_ids::FLOAT,
        loc.clone(),
        ValueMode::ImmutableReference,
    );

    let expr = runtime_template_expression(loc.clone(), vec![value_ref], &string_table);

    let mut builder = setup_builder(&mut string_table);
    register_local(
        &mut builder,
        value_name,
        LocalId(10),
        builtin_type_ids::FLOAT,
        loc.clone(),
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
    let mut string_table = StringTable::new();
    let loc = test_source_location(1);
    let text = string_expr("hello", &mut string_table, loc.clone());

    let expr = runtime_template_expression(loc.clone(), vec![text], &string_table);

    let mut builder = setup_builder(&mut string_table);
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
fn reactive_float_template_subscription_keeps_lazy_formatter_expression() {
    let mut string_table = StringTable::new();
    let loc = test_source_location(1);
    let value_path = symbol("value", &mut string_table);
    let value_local = LocalId(20);
    let source = ReactiveSource {
        path: value_path.clone(),
        kind: ReactiveSourceKind::Declaration,
    };

    let value_ref = reference_expr(
        value_path.clone(),
        builtin_type_ids::FLOAT,
        loc.clone(),
        ValueMode::ImmutableReference,
    )
    .with_reactive_source(source.clone());
    let subscription = ReactiveSubscription {
        source,
        type_id: builtin_type_ids::FLOAT,
        location: loc.clone(),
    };
    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(OwnedRuntimeTemplateNode::Sequence {
            children: vec![OwnedRuntimeTemplateNode::DynamicExpression {
                expression: Box::new(value_ref),
                reactive_subscription: Some(subscription),
            }],
        }),
        location: loc.clone(),
    };

    let mut builder = setup_builder(&mut string_table);
    register_local(
        &mut builder,
        value_path.clone(),
        value_local,
        builtin_type_ids::FLOAT,
        loc.clone(),
    );
    builder.side_table.bind_reactive_source(HirReactiveSource {
        id: ReactiveSourceId(0),
        local_id: value_local,
        path: value_path.clone(),
        kind: HirReactiveSourceKind::Declaration,
        type_id: builtin_type_ids::FLOAT,
        location: loc.clone(),
    });

    let expr = Expression::runtime_template_handoff(handoff, ValueMode::ImmutableOwned);
    let lowered = builder
        .lower_expression(&expr)
        .expect("reactive Float template subscription lowering should succeed");

    assert!(
        expression_contains_float_to_string_cast(&lowered.value),
        "reactive Float subscriptions should format lazily inside the snapshot expression"
    );
    assert!(
        builder.test_current_block_statements().is_empty(),
        "direct reactive subscriptions must not be materialized into eager FormatFloat statements"
    );
}

#[test]
fn cast_float_to_string_optional_wrap_lowers_to_format_float() {
    let mut string_table = StringTable::new();
    let loc = test_source_location(1);
    let source = float_expr(1.5, loc.clone());

    let mut builder = setup_builder(&mut string_table);
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
            policy: BuiltinCastPolicyId::FloatToString,
        },
        handling: CastHandling::Infallible,
        location: loc.clone(),
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
