//! Borrow-checker fixture support for frontend unit tests.
//!
//! WHAT: runs borrow validation against already-lowered HIR modules.
//! WHY: borrow tests need a clear final-stage helper while parser, AST, and HIR fixtures remain
//!      independent of borrow-checker internals.

use crate::compiler_frontend::analysis::borrow_checker::{
    BorrowCheckError, BorrowCheckReport, check_borrows,
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::compiler_messages::{
    BorrowDiagnosticKind, DiagnosticKind, DiagnosticPayload, InvalidMutableAccessReason,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

pub(crate) fn run_borrow_checker(
    module: &HirModule,
    external_package_registry: &ExternalPackageRegistry,
    string_table: &StringTable,
) -> Result<BorrowCheckReport, BorrowCheckError> {
    let path_fork = PathInternerFork::empty();
    check_borrows(module, external_package_registry, &path_fork, string_table)
}

pub(crate) fn assert_borrow_error_kind(
    error: &BorrowCheckError,
    expected: BorrowDiagnosticKind,
) -> &DiagnosticPayload {
    let diagnostic = error
        .diagnostic()
        .expect("borrow user failure should remain a typed diagnostic");

    assert_eq!(diagnostic.kind, DiagnosticKind::Borrow(expected));

    &diagnostic.payload
}

pub(crate) fn assert_invalid_mutable_access_reason(
    error: &BorrowCheckError,
    expected: InvalidMutableAccessReason,
) {
    let payload = assert_borrow_error_kind(error, BorrowDiagnosticKind::InvalidMutableAccess);
    let DiagnosticPayload::InvalidMutableAccess { reason, .. } = payload else {
        panic!("expected invalid mutable access payload, found {payload:?}");
    };

    assert_eq!(*reason, expected);
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
