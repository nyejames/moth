//! Borrow-checker diagnostic naming and source-span helpers.
//!
//! These helpers translate HIR IDs and side-table mappings into user-facing labels and exact
//! source spans without forcing the transfer code to duplicate lookup logic.

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::compiler_messages::{
    BorrowAccessKind, CompilerDiagnostic, DiagnosticPlace, InvalidMutableAccessReason,
};
use crate::compiler_frontend::hir::hir_side_table::{HirLocalOriginKind, HirLocation};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirValueId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reactivity::ReactiveSourceId;
use crate::compiler_frontend::hir::statements::HirStatement;
use crate::compiler_frontend::source::SourceSpan;
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

    pub(super) fn local_source_span(&self, local_id: LocalId) -> Option<SourceSpan> {
        self.module
            .side_table
            .hir_source_span_for_hir(HirLocation::Local(local_id))
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

    pub(super) fn statement_error_span(&self, statement: &HirStatement) -> Option<SourceSpan> {
        statement.span
    }

    pub(super) fn terminator_error_span(&self, block_id: BlockId) -> Option<SourceSpan> {
        self.module.side_table.terminator_span(block_id).copied()
    }

    pub(super) fn function_error_span(&self, function_id: FunctionId) -> Option<SourceSpan> {
        self.module
            .side_table
            .hir_source_span_for_hir(HirLocation::Function(function_id))
    }

    pub(super) fn module_error_span(&self) -> Option<SourceSpan> {
        self.module
            .start_function
            .or_else(|| self.module.functions.first().map(|function| function.id))
            .and_then(|function_id| self.function_error_span(function_id))
    }

    pub(super) fn value_error_span(
        &self,
        value_id: HirValueId,
        fallback: Option<SourceSpan>,
    ) -> Option<SourceSpan> {
        self.module
            .side_table
            .value_source_span(value_id)
            .or_else(|| self.module.side_table.value_ast_span(value_id))
            .or(fallback)
    }

    pub(super) fn internal_error(
        &self,
        message: impl Into<String>,
        source_span: Option<SourceSpan>,
    ) -> BorrowCheckError {
        CompilerError::new(message, source_span, ErrorType::Compiler).into()
    }

    pub(super) fn multiple_mutable_borrows(
        &self,
        place: DiagnosticPlace,
        conflicting_place: Option<DiagnosticPlace>,
        existing_span: Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> BorrowCheckError {
        CompilerDiagnostic::multiple_mutable_borrows(place, conflicting_place, existing_span, span)
            .into()
    }

    pub(super) fn shared_mutable_conflict(
        &self,
        place: DiagnosticPlace,
        existing_access: BorrowAccessKind,
        requested_access: BorrowAccessKind,
        conflicting_place: Option<DiagnosticPlace>,
        existing_span: Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> BorrowCheckError {
        CompilerDiagnostic::shared_mutable_conflict(
            place,
            existing_access,
            requested_access,
            conflicting_place,
            existing_span,
            span,
        )
        .into()
    }

    pub(super) fn invalid_mutable_access(
        &self,
        place: DiagnosticPlace,
        reason: InvalidMutableAccessReason,
        conflicting_place: Option<DiagnosticPlace>,
        conflicting_span: Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> BorrowCheckError {
        CompilerDiagnostic::invalid_mutable_access(
            place,
            reason,
            conflicting_place,
            conflicting_span,
            span,
        )
        .into()
    }

    pub(super) fn use_of_uninitialized_local(
        &self,
        place: DiagnosticPlace,
        span: Option<SourceSpan>,
    ) -> BorrowCheckError {
        CompilerDiagnostic::use_of_uninitialized_local(place, span).into()
    }
}
