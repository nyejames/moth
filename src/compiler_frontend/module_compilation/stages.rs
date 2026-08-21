//! Thin wrappers that attach accumulated warnings to a failing frontend stage.
//!
//! WHY: each stage returns its own `CompilerMessages`, but a module's warnings are accumulated
//!      across stages. Merging them in one place keeps every call site in the module compilation
//!      service and generated materialisation reading as a named step.

use crate::compiler_frontend::CompilerFrontend;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::compiler_errors::{CompilerMessages, merge_stage_messages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::hir::functions::HirFunctionOriginLookup;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::module_metadata::HirLoweringResult;

pub(in crate::compiler_frontend::module_compilation) fn lower_hir(
    compiler: &mut CompilerFrontend,
    module_ast: Ast,
    warnings: &[CompilerDiagnostic],
    function_origin_lookup: HirFunctionOriginLookup,
) -> Result<HirLoweringResult, CompilerMessages> {
    compiler
        .generate_hir(module_ast, function_origin_lookup)
        .map_err(|messages| merge_stage_messages(messages, warnings, &compiler.string_table))
}

pub(in crate::compiler_frontend::module_compilation) fn check_borrows(
    compiler: &CompilerFrontend,
    hir_module: &HirModule,
    warnings: &[CompilerDiagnostic],
) -> Result<BorrowCheckReport, CompilerMessages> {
    compiler
        .check_borrows(hir_module)
        .map_err(|messages| merge_stage_messages(messages, warnings, &compiler.string_table))
}
