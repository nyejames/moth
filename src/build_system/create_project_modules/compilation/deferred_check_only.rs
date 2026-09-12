//! Deferred check-only compilation internals.
//!
//! Check-only units execute only after canonical providers are published. Their successful
//! artefacts never enter a retained boundary; this child preserves the diagnostic-only lane.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{PremergeDiagnosticBatch, PremergeFailure};
use crate::compiler_frontend::module_compilation::ProviderMaterialisationRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::symbols::path_interner::PathInternerBuilder;

use super::super::generated_store::BoundaryGeneratedFunctionStore;
use super::super::module_artifact_store::ModuleArtifactStore;
use super::super::module_inventory;
use super::super::source_discovery::{ResolvedDependencyEdge, ResolvedSourcePackageDependency};
use super::canonical::{BoundaryCompilationContext, compile_check_only_batches};
use super::{
    build_provider_binding_index, build_source_package_dependency_index,
    seed_completed_package_materialisations,
};

/// Seed materialisation templates already published in a canonical boundary.
///
/// A deferred check-only pass cannot borrow the temporary registry used by canonical compilation,
/// so it reconstructs the immutable lookup from the retained successful artefacts. No transient
/// generated result is published into this registry.
fn seed_boundary_materialisations(
    registry: &mut ProviderMaterialisationRegistry,
    modules: &ModuleArtifactStore,
) -> Result<(), CompilerError> {
    for artifact in modules.successful_artefacts_in_module_id_order() {
        if let Some(context) = artifact.module.metadata.materialisation_context.as_ref() {
            registry.publish_context(context)?;
        }
    }
    Ok(())
}

/// Compile one boundary's transient jobs after its canonical artefacts are complete.
///
/// This is used for source packages after *all* source-package facades have published. Check-only
/// jobs can therefore consume any canonical module/package provider without adding package edges
/// or changing the retained boundary. Successful artefacts, generated deltas and resource
/// associations are dropped; only diagnostics and warnings are returned as premerge batches.
///
/// WHAT: stays in the premerge lane and calls the typed canonical batch helper directly.
/// WHY: the deferred lane never constructs the final vessel; the source-owner tail converts
///      each batch exactly once with its finalized snapshot.
pub(super) fn compile_check_only_jobs_after_canonical(
    context: BoundaryCompilationContext<'_>,
    provider_store: &ModuleArtifactStore,
    generated_store: &BoundaryGeneratedFunctionStore,
    check_only_jobs: Vec<module_inventory::CheckOnlyModuleCompilationJob>,
    provider_bindings: &[ResolvedDependencyEdge],
    source_package_dependencies: &[ResolvedSourcePackageDependency],
    string_table: &mut StringTable,
    path_interner: &mut PathInternerBuilder,
) -> Result<Vec<PremergeDiagnosticBatch>, PremergeFailure> {
    let mut provider_materialisations =
        seed_completed_package_materialisations(context.completed_packages())?;
    seed_boundary_materialisations(&mut provider_materialisations, provider_store)?;

    let provider_binding_index = build_provider_binding_index(provider_bindings)?;
    let source_package_dependency_index = build_source_package_dependency_index(
        &provider_binding_index,
        source_package_dependencies,
    )?;
    let build_config_index = context.build_config_resolution_index();

    compile_check_only_batches(
        &context,
        provider_store,
        generated_store,
        &provider_materialisations,
        check_only_jobs,
        provider_bindings,
        &provider_binding_index,
        source_package_dependencies,
        &source_package_dependency_index,
        &build_config_index,
        string_table,
        path_interner,
    )
}
