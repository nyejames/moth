//! Tests for the AST-local scope-frame capacity budget policy.

use super::ScopeFrameCapacityBudget;

#[test]
fn scope_frame_capacity_budget_spends_module_estimate_once() {
    let mut budget = ScopeFrameCapacityBudget::new(10, 3);

    assert_eq!(budget.next_root_capacity(), 4);
    assert_eq!(budget.next_root_capacity(), 3);
    assert_eq!(budget.next_root_capacity(), 3);
    assert_eq!(budget.next_root_capacity(), 0);
}

#[test]
fn scope_frame_capacity_budget_handles_even_distribution() {
    let mut budget = ScopeFrameCapacityBudget::new(12, 4);

    assert_eq!(budget.next_root_capacity(), 3);
    assert_eq!(budget.next_root_capacity(), 3);
    assert_eq!(budget.next_root_capacity(), 3);
    assert_eq!(budget.next_root_capacity(), 3);
}

#[test]
fn scope_frame_capacity_budget_returns_zero_without_roots_or_frames() {
    let mut no_roots = ScopeFrameCapacityBudget::new(8, 0);
    assert_eq!(no_roots.next_root_capacity(), 0);

    let mut no_frames = ScopeFrameCapacityBudget::new(0, 4);
    assert_eq!(no_frames.next_root_capacity(), 0);
    assert_eq!(no_frames.next_root_capacity(), 0);
}

#[test]
fn recovery_admits_an_exact_fit_then_stops_at_the_work_guard() {
    let policy = super::AstRecoveryPolicy {
        module_error_allowance: None,
        body_work_allowance: 10,
    };
    let mut recovery = super::BodyDiagnosticRecovery::new(policy);

    // An exact fit is admitted and spends the whole allowance.
    assert_eq!(recovery.charge_additional_body(10), None);
    assert_eq!(
        recovery.charge_additional_body(1),
        Some(crate::compiler_frontend::compiler_messages::DiagnosticDiscoveryStops::AST_RECOVERY_WORK_GUARD)
    );
}

#[test]
fn recovery_stops_at_the_module_allowance_before_charging_work() {
    let policy = super::AstRecoveryPolicy {
        module_error_allowance: std::num::NonZeroU32::new(1),
        body_work_allowance: 10,
    };
    let mut recovery = super::BodyDiagnosticRecovery::new(policy);

    let diagnostic = crate::compiler_frontend::compiler_messages::diagnostics::families::
        names_declarations::unknown_value_name(
            crate::compiler_frontend::symbols::string_interning::StringTable::new().intern("x"),
            None,
        );
    assert!(!recovery.record_failed_body(diagnostic));

    assert_eq!(
        recovery.charge_additional_body(1),
        Some(crate::compiler_frontend::compiler_messages::DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE)
    );
}

#[test]
fn recovery_counts_only_errors_toward_the_module_allowance() {
    let policy = super::AstRecoveryPolicy {
        module_error_allowance: std::num::NonZeroU32::new(1),
        body_work_allowance: 10,
    };
    let mut recovery = super::BodyDiagnosticRecovery::new(policy);

    let warning =
        crate::compiler_frontend::compiler_messages::diagnostics::families::imports::unused_import(
            None,
        );
    assert!(!recovery.record_failed_body(warning));
    assert!(recovery.is_active());
    assert_eq!(recovery.charge_additional_body(1), None);

    let error = crate::compiler_frontend::compiler_messages::diagnostics::families::
        names_declarations::unknown_value_name(
            crate::compiler_frontend::symbols::string_interning::StringTable::new().intern("x"),
            None,
        );
    assert!(!recovery.record_failed_body(error));
    assert_eq!(
        recovery.charge_additional_body(1),
        Some(crate::compiler_frontend::compiler_messages::DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE)
    );
}

#[test]
fn capacity_failure_ends_recovery_with_earlier_diagnostics_preserved() {
    let policy = super::AstRecoveryPolicy {
        module_error_allowance: Some(std::num::NonZeroU32::new(5).expect("non-zero")),
        body_work_allowance: 10,
    };
    let mut recovery = super::BodyDiagnosticRecovery::new(policy);

    let first = crate::compiler_frontend::compiler_messages::diagnostics::families::
        names_declarations::unknown_value_name(
            crate::compiler_frontend::symbols::string_interning::StringTable::new().intern("x"),
            None,
        );
    assert!(!recovery.record_failed_body(first));

    let capacity =
        crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::
            logical_path_capacity();
    assert!(recovery.record_failed_body(capacity));
    assert_eq!(
        recovery.errors.as_slice().len(),
        2,
        "earlier user diagnostics survive the capacity failure that ends recovery"
    );
}

#[test]
fn expression_failure_split_keeps_infrastructure_fatal() {
    let policy = super::AstRecoveryPolicy {
        module_error_allowance: None,
        body_work_allowance: 10,
    };
    let mut recovery = super::BodyDiagnosticRecovery::new(policy);
    let diagnostic = crate::compiler_frontend::compiler_messages::diagnostics::families::
        names_declarations::unknown_value_name(
            crate::compiler_frontend::symbols::string_interning::StringTable::new().intern("x"),
            None,
        );
    assert!(!recovery.record_failed_body(diagnostic));
    assert!(recovery.is_active());

    // This split is the lane classifier used when a body fails. The emitter loop returns the
    // Fatal arm unchanged, so collected user drafts never reach a prepared batch.
    let infrastructure = crate::compiler_frontend::compiler_errors::InfrastructureFailure::internal(
        crate::compiler_frontend::compiler_messages::CompilerStage::AstConstruction,
        "test infrastructure failure",
    );
    let failure = super::AstEmitter::split_expression_failure(
        crate::compiler_frontend::ast::expressions::error::ExpressionParseError::Infrastructure(
            infrastructure.into(),
        ),
    );
    match failure {
        super::FunctionBodyFailure::Fatal(
            crate::compiler_frontend::compiler_messages::PreparedFailure::Infrastructure(errors),
        ) => assert_eq!(errors.iter().count(), 1),
        other => panic!("expected a retained infrastructure failure, found {other:?}"),
    }
}
