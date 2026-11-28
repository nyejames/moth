//! Borrow-checker fixture support for frontend unit tests.
//!
//! WHAT: runs borrow validation against already-lowered HIR modules.
//! WHY: borrow tests need a clear final-stage helper while parser, AST, and HIR fixtures remain
//!      independent of borrow-checker internals.

use crate::compiler_frontend::analysis::borrow_checker::{
    BorrowCheckError, BorrowCheckReport, check_borrows, check_borrows_with_projection_for_test,
    summarize_call_allocations_with_projection,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::InvalidMutableAccessView;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::diagnostics::view::FamilyView;
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, InvalidMutableAccessReason};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::hir_fixture_support::{LoweredHirFixture, PreparedHirFixture};

pub(crate) fn run_prepared_borrow_checker(
    fixture: &PreparedHirFixture,
) -> Result<BorrowCheckReport, BorrowCheckError> {
    check_borrows_with_projection_for_test(
        &fixture.hir,
        &fixture.hir.type_environment,
        &fixture.packages,
        &fixture.paths,
        &fixture.strings,
        &fixture.type_projection(),
    )
}

pub(crate) fn summarize_prepared_call_allocations(
    fixture: &PreparedHirFixture,
) -> Result<BorrowCheckReport, BorrowCheckError> {
    summarize_call_allocations_with_projection(
        &fixture.hir,
        &fixture.hir.type_environment,
        &fixture.packages,
        &fixture.paths,
        &fixture.strings,
        &fixture.type_projection(),
    )
}

pub(crate) fn run_borrow_checker(
    fixture: &LoweredHirFixture,
    external_package_registry: &ExternalPackageRegistry,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Result<BorrowCheckReport, BorrowCheckError> {
    run_borrow_checker_with_types(
        &fixture.hir_module,
        &fixture.type_environment,
        external_package_registry,
        path_fork,
        string_table,
    )
}

pub(crate) fn run_borrow_checker_with_types(
    module: &HirModule,
    type_environment: &TypeEnvironment,
    external_package_registry: &ExternalPackageRegistry,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> Result<BorrowCheckReport, BorrowCheckError> {
    check_borrows(
        module,
        type_environment,
        external_package_registry,
        path_fork,
        string_table,
        None,
    )
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
