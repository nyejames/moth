//! Thin wrappers that prep accumulated warnings before handing off captured stage failures.
//!
//! WHY: each stage keeps its diagnostics paired with the local string and path identities while
//!      the owning compiler state is live, without cloning the compiler's whole `StringTable`.

use crate::compiler_frontend::CompilerFrontend;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::arena::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, PreparedFailure};
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

pub(in crate::compiler_frontend::module_compilation) fn lower_hir(
    compiler: &mut CompilerFrontend<'_>,
    module_ast: Ast,
    warnings: &mut DiagnosticBag,
    function_origin_lookup: HirFunctionOriginLookup,
    module_resources: Option<Rc<RefCell<ModuleResourceTable>>>,
    frozen_identity_handle: Option<&FrozenIdentityHandle>,
    capacity_estimate: FrontendArenaCapacityEstimate,
) -> Result<HirLoweringResult, PreparedFailure> {
    compiler.generate_hir_with_owner(
        module_ast,
        function_origin_lookup,
        module_resources,
        capacity_estimate,
        frozen_identity_handle,
        Some(warnings),
    )
}

pub(in crate::compiler_frontend::module_compilation) fn check_borrows(
    compiler: &mut CompilerFrontend<'_>,
    hir_module: &HirModule,
    type_environment: &TypeEnvironment,
    warnings: &mut DiagnosticBag,
    frozen_identity_handle: Option<&FrozenIdentityHandle>,
) -> Result<BorrowCheckReport, PreparedFailure> {
    compiler.check_borrows_premerge(
        hir_module,
        type_environment,
        warnings,
        frozen_identity_handle,
    )
}
