//! Borrow-checker diagnostic naming and source-location helpers.
//!
//! These helpers translate HIR IDs and side-table mappings into user-facing labels and error
//! locations without forcing the transfer code to duplicate lookup logic.

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    BorrowAccessKind, CompilerDiagnostic, DiagnosticPlace, InvalidMutableAccessReason,
};
use crate::compiler_frontend::hir::hir_side_table::{HirLocalOriginKind, HirLocation};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirValueId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reactivity::ReactiveSourceId;
use crate::compiler_frontend::hir::statements::HirStatement;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::string_interning::StringTable;

pub(super) struct BorrowDiagnostics<'a> {
    module: &'a HirModule,
    string_table: &'a StringTable,
}

impl<'a> BorrowDiagnostics<'a> {
    pub(super) fn new(module: &'a HirModule, string_table: &'a StringTable) -> Self {
        Self {
            module,
            string_table,
        }
    }

    pub(super) fn local_name(&self, local_id: LocalId) -> String {
        self.module
            .side_table
            .resolve_local_name(local_id, self.string_table)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{local_id}"))
    }

    pub(super) fn local_place(&self, local_id: LocalId) -> DiagnosticPlace {
        self.module
            .side_table
            .local_name_path(local_id)
            .and_then(|path| path.name())
            .map(DiagnosticPlace::Local)
            .unwrap_or(DiagnosticPlace::Unknown)
    }

    pub(super) fn local_origin_kind(&self, local_id: LocalId) -> Option<HirLocalOriginKind> {
        self.module.side_table.local_origin_kind(local_id)
    }

    pub(super) fn local_source_location(&self, local_id: LocalId) -> Option<SourceLocation> {
        self.module
            .side_table
            .hir_source_location_for_hir(HirLocation::Local(local_id))
            .or_else(|| {
                self.module
                    .side_table
                    .ast_location_for_hir(HirLocation::Local(local_id))
            })
            .cloned()
    }

    pub(super) fn reactive_source_id_for_local(
        &self,
        local_id: LocalId,
    ) -> Option<ReactiveSourceId> {
        self.module
            .side_table
            .reactive_source_id_for_local(local_id)
    }

    pub(super) fn function_name(&self, function_id: FunctionId) -> String {
        self.module
            .side_table
            .resolve_function_name(function_id, self.string_table)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{function_id}"))
    }

    pub(super) fn statement_error_location(&self, statement: &HirStatement) -> SourceLocation {
        statement.location.clone()
    }

    pub(super) fn terminator_error_location(
        &self,
        block_id: BlockId,
        _terminator: &HirTerminator,
    ) -> SourceLocation {
        self.module
            .side_table
            .hir_source_location_for_hir(HirLocation::Terminator(block_id))
            .or_else(|| {
                self.module
                    .side_table
                    .ast_location_for_hir(HirLocation::Terminator(block_id))
            })
            .or_else(|| {
                self.module
                    .side_table
                    .hir_source_location_for_hir(HirLocation::Block(block_id))
            })
            .or_else(|| {
                self.module
                    .side_table
                    .ast_location_for_hir(HirLocation::Block(block_id))
            })
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn function_error_location(&self, function_id: FunctionId) -> SourceLocation {
        self.module
            .side_table
            .hir_source_location_for_hir(HirLocation::Function(function_id))
            .or_else(|| {
                self.module
                    .side_table
                    .ast_location_for_hir(HirLocation::Function(function_id))
            })
            .cloned()
            .unwrap_or_default()
    }

    pub(super) fn value_error_location(
        &self,
        value_id: HirValueId,
        fallback: SourceLocation,
    ) -> SourceLocation {
        self.module
            .side_table
            .value_source_location(value_id)
            .or_else(|| self.module.side_table.value_ast_location(value_id))
            .cloned()
            .unwrap_or(fallback)
    }

    pub(super) fn internal_error(
        &self,
        message: impl Into<String>,
        location: SourceLocation,
    ) -> BorrowCheckError {
        CompilerError::new(message, location, ErrorType::Compiler).into()
    }

    pub(super) fn multiple_mutable_borrows(
        &self,
        place: DiagnosticPlace,
        conflicting_place: Option<DiagnosticPlace>,
        existing_location: Option<SourceLocation>,
        location: SourceLocation,
    ) -> BorrowCheckError {
        CompilerDiagnostic::multiple_mutable_borrows(
            place,
            conflicting_place,
            existing_location,
            location,
        )
        .into()
    }

    pub(super) fn shared_mutable_conflict(
        &self,
        place: DiagnosticPlace,
        existing_access: BorrowAccessKind,
        requested_access: BorrowAccessKind,
        conflicting_place: Option<DiagnosticPlace>,
        existing_location: Option<SourceLocation>,
        location: SourceLocation,
    ) -> BorrowCheckError {
        CompilerDiagnostic::shared_mutable_conflict(
            place,
            existing_access,
            requested_access,
            conflicting_place,
            existing_location,
            location,
        )
        .into()
    }

    pub(super) fn invalid_mutable_access(
        &self,
        place: DiagnosticPlace,
        reason: InvalidMutableAccessReason,
        conflicting_place: Option<DiagnosticPlace>,
        conflicting_location: Option<SourceLocation>,
        location: SourceLocation,
    ) -> BorrowCheckError {
        CompilerDiagnostic::invalid_mutable_access(
            place,
            reason,
            conflicting_place,
            conflicting_location,
            location,
        )
        .into()
    }

    pub(super) fn use_of_uninitialized_local(
        &self,
        place: DiagnosticPlace,
        location: SourceLocation,
    ) -> BorrowCheckError {
        CompilerDiagnostic::use_of_uninitialized_local(place, location).into()
    }
}
