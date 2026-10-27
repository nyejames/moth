//! Borrow-checker fixture support for frontend unit tests.
//!
//! WHAT: runs borrow validation against already-lowered HIR modules.
//! WHY: borrow tests need a clear final-stage helper while parser, AST, and HIR fixtures remain
//!      independent of borrow-checker internals.

use crate::compiler_frontend::analysis::borrow_checker::{
    BorrowCheckError, BorrowCheckReport, check_borrows,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::InvalidMutableAccessView;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::diagnostics::view::FamilyView;
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, InvalidMutableAccessReason};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

pub(crate) fn run_borrow_checker(
    module: &HirModule,
    external_package_registry: &ExternalPackageRegistry,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Result<BorrowCheckReport, BorrowCheckError> {
    check_borrows(module, external_package_registry, path_fork, string_table)
}

pub(crate) fn assert_borrow_error_view<View: FamilyView>(error: BorrowCheckError) -> View {
    let diagnostic = error
        .into_draft_or_infrastructure()
        .expect("borrow user failure should remain a typed diagnostic");
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let view = compact
        .first()
        .expect("borrow user failure should compact to a typed diagnostic");

    view.family_view::<View>()
        .expect("borrow user failure should compact to the expected typed family view")
}

pub(crate) fn assert_invalid_mutable_access_reason(
    error: BorrowCheckError,
    expected: InvalidMutableAccessReason,
) {
    let facts = assert_borrow_error_view::<InvalidMutableAccessView>(error);
    assert_eq!(facts.reason, expected);
}

pub(crate) fn assert_infrastructure_error_contains(
    error: &BorrowCheckError,
    expected_type: InfrastructureFailureKind,
    expected_messages: &[&str],
) {
    let infrastructure_error = error
        .infrastructure()
        .expect("invalid borrow metadata should remain an infrastructure failure");

    assert_eq!(infrastructure_error.kind, expected_type);

    assert!(
        expected_messages
            .iter()
            .any(|expected| infrastructure_error.message.contains(expected)),
        "expected infrastructure error to contain one of {expected_messages:?}, got: {}",
        infrastructure_error.message,
    );
}
