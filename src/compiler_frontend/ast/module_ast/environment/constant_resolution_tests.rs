//! Constant-header fold refusal lane tests.
//!
//! WHAT: pins how a runtime constant initializer's fold result maps onto header error lanes.
//! WHY: a broken compiler invariant must stay on the infrastructure lane instead of becoming the
//! source-level `ConstantInitializerNotFoldable` refusal, while text-unavailable refusals keep
//! their precise diagnostic.

use super::non_constant_initializer_fold_refusal;
use crate::compiler_frontend::ast::const_eval::{
    ConstantFoldError, ConstantFoldOutcome, constant_fold,
};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::compiler_messages::{
    AstCapacityResource, CompileTimeEvaluationErrorReason, CompilerDiagnostic, DiagnosticPayload,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn evaluation_diagnostic(reason: CompileTimeEvaluationErrorReason) -> CompilerDiagnostic {
    CompilerDiagnostic::compile_time_evaluation_error(reason, None, None)
}

fn diagnostic_reason(error: ExpressionParseError) -> CompileTimeEvaluationErrorReason {
    let ExpressionParseError::Diagnostic(diagnostic) = error else {
        panic!("expected a source diagnostic, found {error:?}");
    };
    let DiagnosticPayload::CompileTimeEvaluationError { reason, .. } = diagnostic.payload else {
        panic!("expected a compile-time evaluation diagnostic");
    };
    reason
}

#[test]
fn foreign_constant_content_stays_on_the_infrastructure_lane() {
    let mut string_table = StringTable::new();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut issuing_values = ConstValueStore::default();
    let root = expression_builder
        .fixed_scalar(
            &mut issuing_values,
            FixedScalarValue::signed(FixedScalar::I32, 2).expect("2 fits I32"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("literal fits")
        .id();
    let mut foreign_values = ConstValueStore::default();
    let fold = constant_fold(
        root,
        &mut expression_builder,
        &mut foreign_values,
        &mut string_table,
        None,
    );

    match non_constant_initializer_fold_refusal(fold) {
        Err(ExpressionParseError::Infrastructure(_)) => {}
        other => panic!("fold infrastructure error must survive the header: {other:?}"),
    }
}

#[test]
fn text_unavailable_fold_keeps_its_precise_diagnostic() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let root = expression_builder.reference_with_type_id(crate::compiler_frontend::symbols::path_interner::PathId::ROOT,
    crate::compiler_frontend::datatypes::ids::builtin_type_ids::STRING,
    None,
    ValueMode::ImmutableOwned,
    crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,).expect("reference fits").id();
    let fold = Ok(ConstantFoldOutcome::TextUnavailable {
        root,
        diagnostic: evaluation_diagnostic(
            CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText,
        ),
    });

    let error = non_constant_initializer_fold_refusal(fold)
        .expect_err("text-unavailable refusal must surface its own diagnostic");
    assert_eq!(
        diagnostic_reason(error),
        CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText
    );
}

#[test]
fn ordinary_non_constant_folds_defer_to_the_not_foldable_refusal() {
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let root = expression_builder.reference_with_type_id(crate::compiler_frontend::symbols::path_interner::PathId::ROOT,
    crate::compiler_frontend::datatypes::ids::builtin_type_ids::STRING,
    None,
    ValueMode::ImmutableOwned,
    crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,).expect("reference fits").id();
    let not_constant = Ok(ConstantFoldOutcome::NotConstant(root));
    let source_diagnostic = Err(ConstantFoldError::Diagnostic(evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::DivideByZero,
    )));

    for fold in [not_constant, source_diagnostic] {
        assert!(
            non_constant_initializer_fold_refusal(fold).is_ok(),
            "the header reports ConstantInitializerNotFoldable for ordinary refusals"
        );
    }
}

#[test]
fn builder_capacity_failure_propagates_instead_of_refusing() {
    let capacity =
        CompilerDiagnostic::compiler_capacity_exceeded(AstCapacityResource::ExpressionRows, None);
    let refusal =
        non_constant_initializer_fold_refusal(Err(ConstantFoldError::Diagnostic(capacity)));
    let Err(ExpressionParseError::Diagnostic(diagnostic)) = refusal else {
        panic!("expected the capacity diagnostic to propagate");
    };
    assert!(diagnostic.is_compiler_capacity_exceeded());
}
