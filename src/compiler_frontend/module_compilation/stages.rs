//! Thin wrappers that prep accumulated warnings before handing off captured stage failures.
//!
//! WHY: each stage keeps its diagnostics paired with the local string and path identities while
//!      the owning compiler state is live, without cloning the compiler's whole `StringTable`.

use crate::compiler_frontend::CompilerFrontend;
use crate::compiler_frontend::analysis::borrow_checker::{
    BorrowCheckReport, check_borrows as check_hir_borrows,
    refresh_call_allocations as update_allocation_facts,
    refresh_call_mutation_effects as finalize_mutation_effects,
    seed_call_summaries as seed_contracts,
};
use crate::compiler_frontend::arena::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, PreparedFailure};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::hir::functions::HirFunctionOriginLookup;
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::HirModuleLinkFacts;
use crate::compiler_frontend::module_metadata::HirLoweringResult;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::source::FrozenIdentityHandle;
use rustc_hash::FxHashSet;
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

pub(in crate::compiler_frontend::module_compilation) fn seed_call_summaries(
    compiler: &mut CompilerFrontend<'_>,
    hir_module: &HirModule,
    type_environment: &TypeEnvironment,
    warnings: &mut DiagnosticBag,
    frozen_identity_handle: Option<&FrozenIdentityHandle>,
) -> Result<BorrowCheckReport, PreparedFailure> {
    let result = seed_contracts(hir_module, &compiler.path_fork, &compiler.string_table);
    compiler.prepare_borrow_check_result(result, type_environment, warnings, frozen_identity_handle)
}

pub(in crate::compiler_frontend::module_compilation) fn refresh_call_allocations(
    compiler: &mut CompilerFrontend<'_>,
    hir: &HirModule,
    types: &TypeEnvironment,
    warnings: &mut DiagnosticBag,
    owner: Option<&FrozenIdentityHandle>,
    inputs: (
        &HirModuleLinkFacts,
        &FxHashSet<FunctionId>,
        &crate::compiler_frontend::canonical_type_identity::CanonicalTypeProjectionContext<'_>,
    ),
    report: &mut BorrowCheckReport,
) -> Result<(), PreparedFailure> {
    let result = update_allocation_facts(
        hir,
        types,
        compiler.external_package_registry.as_ref(),
        &compiler.path_fork,
        &compiler.string_table,
        inputs,
        report,
    );
    compiler.prepare_borrow_check_result(result, types, warnings, owner)
}

pub(in crate::compiler_frontend::module_compilation) fn check_borrows(
    compiler: &mut CompilerFrontend<'_>,
    hir_module: &HirModule,
    type_environment: &TypeEnvironment,
    warnings: &mut DiagnosticBag,
    frozen_identity_handle: Option<&FrozenIdentityHandle>,
    report: &mut BorrowCheckReport,
    projection: &crate::compiler_frontend::canonical_type_identity::CanonicalTypeProjectionContext<
        '_,
    >,
) -> Result<(), PreparedFailure> {
    let result = check_hir_borrows(
        hir_module,
        type_environment,
        compiler.external_package_registry.as_ref(),
        &compiler.path_fork,
        &compiler.string_table,
        Some((
            std::mem::take(&mut report.analysis.public_call_summaries),
            projection,
        )),
    );
    let checked = compiler.prepare_borrow_check_result(
        result,
        type_environment,
        warnings,
        frozen_identity_handle,
    )?;
    report.replace_validation(checked);
    Ok(())
}

pub(in crate::compiler_frontend::module_compilation) fn refresh_call_mutation_effects(
    compiler: &mut CompilerFrontend<'_>,
    hir_module: &HirModule,
    type_environment: &TypeEnvironment,
    warnings: &mut DiagnosticBag,
    frozen_identity_handle: Option<&FrozenIdentityHandle>,
    report: &mut BorrowCheckReport,
) -> Result<(), PreparedFailure> {
    let result = finalize_mutation_effects(
        hir_module,
        type_environment,
        compiler.external_package_registry.as_ref(),
        &compiler.path_fork,
        &compiler.string_table,
        report,
    );
    compiler.prepare_borrow_check_result(result, type_environment, warnings, frozen_identity_handle)
}
