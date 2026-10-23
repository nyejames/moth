//! Borrow-checker fixture support for frontend unit tests.
//!
//! WHAT: runs borrow validation against already-lowered HIR modules.
//! WHY: borrow tests need a clear final-stage helper while parser, AST, and HIR fixtures remain
//!      independent of borrow-checker internals.

use crate::compiler_frontend::analysis::borrow_checker::{
    BorrowCheckError, BorrowCheckReport, check_borrows,
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::InvalidMutableAccessView;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::{
    CompactDiagnosticBagForTest, compact_bag_for_test,
};
use crate::compiler_frontend::compiler_messages::{
    BorrowDiagnosticKind, DiagnosticBag, DiagnosticKind, InvalidMutableAccessReason,
};
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

pub(crate) fn assert_borrow_error_kind(
    error: BorrowCheckError,
    expected: BorrowDiagnosticKind,
) -> CompactDiagnosticBagForTest {
    let diagnostic = error
        .into_draft_or_infrastructure()
        .expect("borrow user failure should remain a typed diagnostic");
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let view = compact
        .first()
        .expect("borrow user failure should compact to a typed diagnostic");

    assert_eq!(
        view.descriptor().code,
        DiagnosticKind::Borrow(expected).descriptor().code
    );
    compact
}

pub(crate) fn assert_invalid_mutable_access_reason(
    error: BorrowCheckError,
    expected: InvalidMutableAccessReason,
) {
    let compact = assert_borrow_error_kind(error, BorrowDiagnosticKind::InvalidMutableAccess);
    let view = compact
        .first()
        .expect("invalid mutable access should compact to a typed diagnostic");
    let facts = view
        .family_view::<InvalidMutableAccessView>()
        .expect("invalid mutable access should retain its typed family facts");

    assert_eq!(facts.reason, expected);
}

pub(crate) fn assert_infrastructure_error_contains(
    error: &BorrowCheckError,
    expected_type: ErrorType,
    expected_messages: &[&str],
) {
    let infrastructure_error = error
        .infrastructure()
        .expect("invalid borrow metadata should remain an infrastructure failure");

    assert_eq!(infrastructure_error.error_type, expected_type);

    assert!(
        expected_messages
            .iter()
            .any(|expected| infrastructure_error.msg.contains(expected)),
        "expected infrastructure error to contain one of {expected_messages:?}, got: {}",
        infrastructure_error.msg,
    );
}
