//! Thin wrappers that prep accumulated warnings before handing off captured stage failures.
//!
//! WHY: each stage keeps its diagnostics paired with the local string and path identities while
//!      the owning compiler state is live, without cloning the compiler's whole `StringTable`.

use crate::compiler_frontend::CompilerFrontend;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::arena::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticBag, prepared_batch::AstDiagnosticFailure,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::hir::functions::HirFunctionOriginLookup;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::module_metadata::HirLoweringResult;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::source::FrozenIdentityHandle;
use std::{cell::RefCell, rc::Rc};

#[cfg(test)]
#[path = "tests/stages_tests.rs"]
mod tests;

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
    warnings: &mut DiagnosticBag,
    function_origin_lookup: HirFunctionOriginLookup,
    module_resources: Option<Rc<RefCell<ModuleResourceTable>>>,
    frozen_identity_handle: Option<&FrozenIdentityHandle>,
    capacity_estimate: FrontendArenaCapacityEstimate,
) -> Result<HirLoweringResult, AstDiagnosticFailure> {
    compiler
        .generate_hir_with_owner(
            module_ast,
            function_origin_lookup,
            module_resources,
            capacity_estimate,
            frozen_identity_handle,
        )
        .map_err(|mut failure| {
            failure.prepend_diagnostics(
                warnings.take_diagnostics().into_iter().map(|diagnostic| {
                    attach_owner_to_diagnostic(diagnostic, frozen_identity_handle)
                }),
            );
            failure
        })
}

pub(in crate::compiler_frontend::module_compilation) fn check_borrows(
    compiler: &mut CompilerFrontend<'_>,
    hir_module: &HirModule,
    type_environment: &TypeEnvironment,
    warnings: &mut DiagnosticBag,
    frozen_identity_handle: Option<&FrozenIdentityHandle>,
) -> Result<BorrowCheckReport, AstDiagnosticFailure> {
    let prior_diagnostics = warnings.take_diagnostics();
    let prior_diagnostics_require_path_table =
        prior_diagnostics.iter().any(diagnostic_requires_path_table);
    match compiler.check_borrows_premerge(
        hir_module,
        type_environment,
        prior_diagnostics_require_path_table,
        frozen_identity_handle,
    ) {
        Ok(report) => {
            *warnings = DiagnosticBag::from_diagnostics(prior_diagnostics);
            Ok(report)
        }
        Err(mut failure) => {
            failure.prepend_diagnostics(
                prior_diagnostics.into_iter().map(|diagnostic| {
                    attach_owner_to_diagnostic(diagnostic, frozen_identity_handle)
                }),
            );
            Err(failure)
        }
    }
}

fn diagnostic_requires_path_table(diagnostic: &CompilerDiagnostic) -> bool {
    let mut required_paths = Vec::new();
    diagnostic
        .payload
        .required_path_payload_paths(&mut required_paths);
    !required_paths.is_empty()
}
