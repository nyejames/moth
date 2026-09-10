//! Thin wrappers that classify stage failures into the premerge handoff lane.
//!
//! WHY: each deeper stage still returns its legacy `CompilerMessages` vessel, but this module
//!      classifies it immediately and prepends accumulated warnings into the diagnosed
//!      `PremergeDiagnosticBatch` without cloning the compiler's whole `StringTable`.

use crate::compiler_frontend::CompilerFrontend;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, PremergeFailure};
use crate::compiler_frontend::hir::functions::HirFunctionOriginLookup;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::module_metadata::HirLoweringResult;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::source::FrozenIdentityHandle;
use std::{cell::RefCell, rc::Rc};

fn attach_owner_to_diagnostic(
    mut diagnostic: CompilerDiagnostic,
    frozen_identity_handle: Option<&FrozenIdentityHandle>,
) -> CompilerDiagnostic {
    if let Some(handle) = frozen_identity_handle {
        diagnostic.attach_frozen_identity_handle_if_missing(handle.clone());
    }
    diagnostic
}

pub(in crate::compiler_frontend::module_compilation) fn lower_hir(
    compiler: &mut CompilerFrontend<'_>,
    module_ast: Ast,
    warnings: &[CompilerDiagnostic],
    function_origin_lookup: HirFunctionOriginLookup,
    module_resources: Option<Rc<RefCell<ModuleResourceTable>>>,
    frozen_identity_handle: Option<&FrozenIdentityHandle>,
) -> Result<HirLoweringResult, PremergeFailure> {
    compiler
        .generate_hir(module_ast, function_origin_lookup, module_resources)
        .map_err(|messages| {
            let mut failure = PremergeFailure::from(messages);
            if let Some(handle) = frozen_identity_handle {
                failure.set_frozen_identity_handle_if_missing(handle.clone());
            }
            failure.prepend_diagnostics(
                warnings.iter().cloned().map(|diagnostic| {
                    attach_owner_to_diagnostic(diagnostic, frozen_identity_handle)
                }),
            );
            failure
        })
}

pub(in crate::compiler_frontend::module_compilation) fn check_borrows(
    compiler: &mut CompilerFrontend<'_>,
    hir_module: &HirModule,
    warnings: &[CompilerDiagnostic],
    frozen_identity_handle: Option<&FrozenIdentityHandle>,
) -> Result<BorrowCheckReport, PremergeFailure> {
    compiler
        .check_borrows_premerge(hir_module, frozen_identity_handle)
        .map_err(|mut failure| {
            failure.prepend_diagnostics(
                warnings.iter().cloned().map(|diagnostic| {
                    attach_owner_to_diagnostic(diagnostic, frozen_identity_handle)
                }),
            );
            failure
        })
}
