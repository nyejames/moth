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
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, Operator};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, DiagnosticPayload,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::profile::NumericProfile;

fn evaluation_diagnostic(reason: CompileTimeEvaluationErrorReason) -> CompilerDiagnostic {
    CompilerDiagnostic::compile_time_evaluation_error(reason, None, None, None)
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
fn escaped_pending_fold_stays_on_the_infrastructure_lane() {
    let mut string_table = StringTable::new();
    let fold = constant_fold(
        vec![
            ExpressionRpnItem::Operand(Expression::int(
                2,
                Default::default(),
                ValueMode::ImmutableOwned,
            )),
            ExpressionRpnItem::PendingGroup {
                nodes: Vec::new(),
                span: None,
            },
            ExpressionRpnItem::Operator {
                operator: Operator::Add,
                span: None,
            },
        ],
        &mut string_table,
        NumericProfile::STANDARD,
        None,
    );

    match non_constant_initializer_fold_refusal(fold) {
        Err(ExpressionParseError::Infrastructure(error)) => assert!(
            error
                .msg
                .contains("evaluate_expression must resolve it first"),
            "unexpected invariant message: {error:?}"
        ),
        other => panic!("fold infrastructure error must survive the header: {other:?}"),
    }
}

#[test]
fn text_unavailable_fold_keeps_its_precise_diagnostic() {
    let fold = Ok(ConstantFoldOutcome::TextUnavailable {
        items: Vec::new(),
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
    let not_constant = Ok(ConstantFoldOutcome::NotConstant(Vec::new()));
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
