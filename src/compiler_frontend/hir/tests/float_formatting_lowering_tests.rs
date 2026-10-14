//! Focused tests for runtime F64 formatting lowering.
//!
//! WHAT: verifies that runtime F64 template interpolation lowers through
//!       `HirStatementKind::FormatFloat`, while `cast F64 -> String` takes the ordinary
//!       numeric-to-string `HirExpressionKind::Cast` policy path shared by every numeric domain.
//! WHY: F64 formatting is a Moth-owned contract shared by casts and templates; dedicated
//!      tests guard against regressions back to native stringification and against a
//!      cast-only formatter special case.

use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, CastFacts, ExprId,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId,
};
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
use crate::compiler_frontend::hir::statements::{HirLocalDestination, HirStatementKind};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::hir::tests::symbol;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use crate::compiler_frontend::tests::ast_fixture_support::reference_expr_with_type_id;
use crate::compiler_frontend::tests::hir_fixture_support::lower_hir;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn float_expr(
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: f64,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> ExprId {
    expressions
        .fixed_scalar(
            constants,
            FixedScalarValue::binary_float(FixedScalar::F64, value)
                .expect("test value is finite and exactly representable at F64"),
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture float fits store")
        .id()
}

fn string_expr(
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: &str,
    string_table: &mut StringTable,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> ExprId {
    expressions
        .string_slice(
            constants,
            string_table.intern(value),
            span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string fits store")
        .id()
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
                    .local_type_id_or_error(result.local(), &statement.span)
                    .ok();
                result_type.map(|ty| (*failure_mode, ty))
            }
            _ => None,
        })
        .collect()
}

fn make_float_to_string_cast(
    expressions: &mut AstExpressionBuilder,
    source: ExprId,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> ExprId {
    let cast = CastFacts {
        source_type_id: builtin_type_ids::fixed_scalar(FixedScalar::F64),
        target_type_id: builtin_type_ids::STRING,
        requires_optional_wrap_after_cast: false,
        fallibility: BuiltinCastFallibility::Infallible,
        evidence: ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Fixed(FixedScalar::F64)),
        },
        handling: CastHandling::Implicit,
        span,
    };

    expressions
        .cast(expressions.own(source), cast, builtin_type_ids::STRING)
        .expect("fixture cast fits store")
        .id()
}

#[test]
fn float_validation_distinguishes_compiler_entry_from_declared_error_contract() {
    for compiler_entry in [false, true] {
        let mut path_fork = super::PathInternerFork::empty();
        let mut string_table = StringTable::new();
        let mut expressions = AstExpressionBuilder::new().unwrap();
        let mut constants = ConstValueStore::default();
        let name = symbol("validated_float", &mut path_fork, &mut string_table);
        let mut builder = setup_builder(&mut string_table, &mut path_fork);
        let error_type = builder.test_register_builtin_error_type();
        let function_id = FunctionId(1);
        builder.test_register_function_with_returns(
            name,
            function_id,
            vec![builtin_type_ids::fixed_scalar(FixedScalar::F64)],
            Some(error_type),
        );
        builder.test_set_current_function(function_id);
        if compiler_entry {
            builder.module.start_function = Some(function_id);
        }
        let source_expr = float_expr(&mut expressions, &mut constants, 1.5, None);
        builder.module_const_values = std::mem::take(&mut constants);
        let source = builder
            .lower_expression(
                expressions
                    .view(source_expr)
                    .expect("fixture expression belongs to builder"),
            )
            .expect("finite F64 source must lower")
            .value;
        builder
            .emit_validated_float_value(source, &None)
            .expect("F64 boundary guard must lower");
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
        if compiler_entry {
            assert_eq!(validation_modes, vec![NumericFailureMode::Trap]);
        } else {
            assert!(validation_modes.is_empty());
            assert!(builder.module.blocks.iter().any(|block| matches!(
                &block.terminator,
                HirTerminator::Invoke {
                    operation: HirFallibleOperation::ValidateFloat { .. },
                    ..
                }
            )));
        }
    }
}

#[test]
fn float_formatting_in_synthetic_error_entry_keeps_integrity_trap() {
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(
        "identity |value F64| -> F64:\n    return value\n;\n\
         value = identity(1.5)\n[:[value]]\n",
    );
    let module = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    );
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
fn cast_float_to_string_lowers_to_numeric_to_string_cast() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let source = float_expr(&mut expressions, &mut constants, 1.5, loc);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let expr = make_float_to_string_cast(&mut expressions, source, loc);

    builder.module_const_values = std::mem::take(&mut constants);
    let lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("fixture expression belongs to builder"),
        )
        .expect("F64 -> String cast lowering should succeed");

    assert_eq!(
        builder.module.expressions.expression(lowered.value).ty,
        builtin_type_ids::STRING
    );
    assert!(
        matches!(
            &builder.module.expressions.expression(lowered.value).kind,
            HirExpressionKind::Cast {
                policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Fixed(
                    FixedScalar::F64
                )),
                ..
            }
        ),
        "F64 -> String cast should lower through the ordinary numeric-to-string policy"
    );

    let format_floats = find_format_float_statements(&builder);
    assert!(
        format_floats.is_empty(),
        "casts take the ordinary numeric-to-string path and emit no FormatFloat statement"
    );
}

#[test]
fn cast_float_to_string_keeps_source_prelude_ahead_of_the_conversion() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let source_name = symbol("source_float", &mut path_fork, &mut string_table);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    builder.test_register_function_name(source_name, FunctionId(7));

    let source = expressions
        .function_call_with_typed_arguments(
            source_name,
            vec![],
            vec![builtin_type_ids::fixed_scalar(FixedScalar::F64)],
            loc,
        )
        .expect("fixture call fits store");
    let expr = make_float_to_string_cast(&mut expressions, source.id(), loc);

    builder.module_const_values = std::mem::take(&mut constants);
    let lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("fixture expression belongs to builder"),
        )
        .expect("F64 -> String cast source prelude lowering should succeed");

    assert_eq!(
        lowered.prelude.len(),
        1,
        "the source call stays in the returned prelude so it runs before the conversion"
    );
    let HirStatementKind::Call {
        target: CallTarget::Local(FunctionId(7)),
        results,
        ..
    } = &lowered.prelude[0].kind
    else {
        panic!("source call should define its result local in the returned prelude");
    };
    let [HirLocalDestination::Define(call_result)] = results.as_slice() else {
        panic!("source call should define exactly one result");
    };
    let call_result = *call_result;
    assert!(
        matches!(
            &builder.module.expressions.expression(lowered.value).kind,
            HirExpressionKind::Cast { source, .. }
                if matches!(
                    &builder.module.expressions.expression(*source).kind,
                    HirExpressionKind::Load(place) if place.root == call_result
                )
        ),
        "the cast should consume the source call result after it exists"
    );
    assert!(
        find_format_float_statements(&builder).is_empty(),
        "casts take the ordinary numeric-to-string path and emit no FormatFloat statement"
    );
}

#[test]
fn float_formatting_in_declared_error_function_uses_invoke() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let fn_name = symbol("__test_fn_error", &mut path_fork, &mut string_table);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let error_type_id = builder.test_register_builtin_error_type();
    builder.test_register_function_with_returns(
        fn_name,
        FunctionId(1),
        vec![builtin_type_ids::STRING],
        Some(error_type_id),
    );
    builder.test_set_current_function(FunctionId(1));

    let source_expr = float_expr(&mut expressions, &mut constants, 1.5, loc);
    builder.module_const_values = std::mem::take(&mut constants);
    let source = builder
        .lower_expression(
            expressions
                .view(source_expr)
                .expect("fixture expression belongs to builder"),
        )
        .expect("F64 source should lower in an Error! function")
        .value;
    let lowered = builder
        .emit_formatted_float_value(source, &loc)
        .expect("recoverable F64 formatting should lower in an Error! function");

    assert!(find_format_float_statements(&builder).is_empty());
    assert!(
        matches!(
            &builder.module.expressions.expression(lowered).kind,
            HirExpressionKind::Load(_)
        ),
        "recoverable F64 formatting should continue with its edge-local success value"
    );
    assert!(
        builder.module.blocks.iter().any(|block| matches!(
            &block.terminator,
            HirTerminator::Invoke {
                operation: HirFallibleOperation::FormatFloat { .. },
                ..
            }
        )),
        "recoverable F64 formatting should invoke native result channels"
    );
    assert!(
        builder
            .module
            .blocks
            .iter()
            .any(|block| matches!(block.terminator, HirTerminator::ReturnError(_))),
        "recoverable F64 formatting should emit a builtin Error return edge"
    );
}

#[test]
fn runtime_float_template_interpolation_lowers_to_format_float_statement() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let value_name = symbol("value", &mut path_fork, &mut string_table);
    let value_ref = reference_expr_with_type_id(
        &mut expressions,
        value_name,
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        loc,
        ValueMode::ImmutableReference,
    );

    let expr = runtime_template_expression(
        &mut expressions,
        &constants,
        loc,
        vec![value_ref],
        &string_table,
    );

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    register_local(
        &mut builder,
        value_name,
        LocalId(10),
        builtin_type_ids::fixed_scalar(FixedScalar::F64),
        loc,
    );

    builder.module_const_values = std::mem::take(&mut constants);
    let _lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("fixture expression belongs to builder"),
        )
        .expect("F64 template interpolation lowering should succeed");

    let format_floats = find_format_float_statements(&builder);
    assert_eq!(
        format_floats.len(),
        1,
        "runtime F64 template interpolation should emit exactly one FormatFloat statement"
    );
    assert!(matches!(format_floats[0].0, NumericFailureMode::Trap));
}

#[test]
fn runtime_string_template_chunk_does_not_emit_format_float() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let text = string_expr(
        &mut expressions,
        &mut constants,
        "hello",
        &mut string_table,
        loc,
    );

    let expr =
        runtime_template_expression(&mut expressions, &constants, loc, vec![text], &string_table);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    builder.module_const_values = std::mem::take(&mut constants);
    let _lowered = builder
        .lower_expression(
            expressions
                .view(expr)
                .expect("fixture expression belongs to builder"),
        )
        .expect("String template chunk lowering should succeed");

    let format_floats = find_format_float_statements(&builder);
    assert!(
        format_floats.is_empty(),
        "String template chunks must not emit FormatFloat statements"
    );
}

#[test]
fn cast_float_to_string_optional_wrap_wraps_the_numeric_to_string_cast() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let loc = None;
    let source = float_expr(&mut expressions, &mut constants, 1.5, loc);

    let mut builder = setup_builder(&mut string_table, &mut path_fork);
    let optional_string_type = builder
        .type_environment
        .intern_option(builtin_type_ids::STRING);

    let cast = CastFacts {
        source_type_id: builtin_type_ids::fixed_scalar(FixedScalar::F64),
        target_type_id: builtin_type_ids::STRING,
        requires_optional_wrap_after_cast: true,
        fallibility: BuiltinCastFallibility::Infallible,
        evidence: ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Fixed(FixedScalar::F64)),
        },
        handling: CastHandling::Implicit,
        span: loc,
    };

    let expr = expressions
        .cast(expressions.own(source), cast, optional_string_type)
        .expect("fixture optional cast fits store");

    builder.module_const_values = std::mem::take(&mut constants);
    let lowered = builder
        .lower_expression(
            expressions
                .view_owned(expr)
                .expect("fixture expression belongs to builder"),
        )
        .expect("optional F64 -> String cast lowering should succeed");

    assert_eq!(
        builder.module.expressions.expression(lowered.value).ty,
        optional_string_type
    );

    let HirExpressionKind::VariantConstruct { fields, .. } =
        &builder.module.expressions.expression(lowered.value).kind
    else {
        panic!("optional F64 -> String cast should wrap the converted string in some(...)");
    };
    let wrapped = builder.module.expressions.variant_fields(*fields)[0].value;
    assert!(
        matches!(
            &builder.module.expressions.expression(wrapped).kind,
            HirExpressionKind::Cast {
                policy: BuiltinCastPolicyId::NumericToString(NumericScalar::Fixed(
                    FixedScalar::F64
                )),
                ..
            }
        ),
        "the optional wrap should contain the ordinary numeric-to-string cast"
    );
    assert!(
        find_format_float_statements(&builder).is_empty(),
        "casts take the ordinary numeric-to-string path and emit no FormatFloat statement"
    );
}
