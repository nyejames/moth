//! Canonical directory-boundary compilation internals.
//!
//! This child owns the immutable boundary/task contexts and canonical readiness-wave compiler.
//! The parent module retains package/project inventory orchestration and publication contracts.

use crate::timing_scope_attributed;

use crate::builder_surface::BuilderSurface;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::build_config::{
    BuildConfigContractFact, BuildConfigInputSet, BuildConfigResolutionIndex,
    BuilderConfigGlobalSet, ResolvedBuildConfigMap,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidDependencyClauseReason, ModuleDiagnostics,
};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::module_compilation::{
    CompiledModuleArtifact, KnownGeneratedFunctions, ModuleCompilationContext,
    ModuleCompilationOutcome, ModuleSemanticResult, ProviderMaterialisationRegistry,
    compile_module,
};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::project_globals::{
    ProjectGlobalsInterface, is_project_globals_dependency,
};
use crate::compiler_frontend::public_interface::{
    ProviderDependencyKind, SourceProviderDependency, SourceProviderDependencySet,
};
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::DependencyShellId;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use crate::projects::settings::Config;

use std::sync::Arc;

use rustc_hash::FxHashMap;

use super::super::compiled_boundary::{
    BlockedModule, BlockedProvider, CompiledGraphBoundary, CompletedSourcePackageRegistry,
    DiagnosedModule, PackageBoundaryId,
};
use super::super::config_boundary;
use super::super::generated_store::BoundaryGeneratedFunctionStore;
use super::super::module_artifact_store::{ModuleArtifactStore, ProviderSlot};
use super::super::module_identity::ModuleId;
use super::super::module_inventory;
use super::super::prepared_module::PreparedModule;
use super::super::project_module_graph::ProjectModuleGraph;
use super::super::resource_inputs::ResourceInputRegistry;
use super::super::source_discovery::{ResolvedDependencyEdge, ResolvedSourcePackageDependency};

use super::{
    ModuleBoundaryPublication, build_module_package_dependency_index, build_provider_binding_index,
    build_source_package_dependency_index, publish_module_and_generated,
    seed_completed_package_materialisations,
};

struct DirectoryModuleTaskResult {
    module_id: ModuleId,
    string_table_base_len: usize,
    outcome: DirectoryModuleTaskOutcome,
}

enum DirectoryModuleTaskOutcome {
    Success(Box<ModuleSemanticResult>),
    Diagnosed(ModuleDiagnostics),
    /// A transient check-only unit whose required canonical provider already failed.
    ///
    /// Check-only units have no graph slot, so this outcome is intentionally not retained in the
    /// final boundary. The provider's own diagnosed/blocked record remains authoritative.
    Blocked,
    Infrastructure(CompilerError),
}

/// Immutable inputs shared by one project or source-package boundary compilation.
///
/// WHAT: keeps the boundary-wide compiler services together while each module task adds only its
///       retained provider indexes and publication stores.
/// WHY: project and source-package callers should pass one typed boundary context to the wave
///      coordinator instead of relying on a long positional argument list whose order can drift.
pub(super) struct BoundaryCompilationContext<'a> {
    config: &'a Config,
    build_profile: FrontendBuildProfile,
    project_path_resolver: &'a ProjectPathResolver,
    style_directives: &'a StyleDirectiveRegistry,
    external_packages: &'a Arc<ExternalPackageRegistry>,
    builder_surface: &'a BuilderSurface,
    completed_packages: &'a CompletedSourcePackageRegistry,
    /// One immutable resolved configuration namespace for this project/package boundary.
    build_config_values: Arc<ResolvedBuildConfigMap>,
    /// Canonical source contracts retained by this boundary.
    ///
    /// Check-only jobs borrow this canonical view through an indexed resolver and resolve their
    /// own transient facts privately; sibling jobs and the retained canonical map never observe
    /// those transient declarations.
    canonical_source_facts: Vec<BuildConfigContractFact>,
    /// Explicit synthetic project-global provider, present only in the owning project boundary.
    project_globals: Option<&'a ProjectGlobalsInterface>,
    /// Inputs and facts retained for isolated check-only resolution; canonical jobs already consume
    /// the authoritative `build_config_values` map directly.
    build_config_inputs: BuildConfigInputSet,
    builder_globals: BuilderConfigGlobalSet,
    fixed_project_facts: Vec<BuildConfigContractFact>,
    direct_project_facts: Vec<BuildConfigContractFact>,
    implicit_template_package_ids: Vec<PackageBoundaryId>,
}

impl<'a> BoundaryCompilationContext<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        config: &'a Config,
        build_profile: FrontendBuildProfile,
        project_path_resolver: &'a ProjectPathResolver,
        style_directives: &'a StyleDirectiveRegistry,
        external_packages: &'a Arc<ExternalPackageRegistry>,
        builder_surface: &'a BuilderSurface,
        completed_packages: &'a CompletedSourcePackageRegistry,
        build_config_values: ResolvedBuildConfigMap,
        canonical_source_facts: Vec<BuildConfigContractFact>,
        build_config_inputs: BuildConfigInputSet,
        builder_globals: BuilderConfigGlobalSet,
        fixed_project_facts: Vec<BuildConfigContractFact>,
        direct_project_facts: Vec<BuildConfigContractFact>,
        project_globals: Option<&'a ProjectGlobalsInterface>,
    ) -> Self {
        let mut implicit_template_package_ids = builder_surface
            .implicit_template_scope_source_packages
            .iter()
            .filter_map(|prefix| completed_packages.by_prefix(prefix))
            .collect::<Vec<_>>();
        implicit_template_package_ids.sort_unstable();
        implicit_template_package_ids.dedup();

        Self {
            config,
            build_profile,
            project_path_resolver,
            style_directives,
            external_packages,
            builder_surface,
            completed_packages,
            build_config_values: Arc::new(build_config_values),
            canonical_source_facts,
            build_config_inputs,
            builder_globals,
            fixed_project_facts,
            direct_project_facts,
            project_globals,
            implicit_template_package_ids,
        }
    }

    pub(super) fn completed_packages(&self) -> &CompletedSourcePackageRegistry {
        self.completed_packages
    }

    pub(super) fn build_config_resolution_index(&self) -> BuildConfigResolutionIndex<'_> {
        BuildConfigResolutionIndex::from_validated(
            self.build_config_values.as_ref(),
            &self.canonical_source_facts,
            &self.fixed_project_facts,
            &self.direct_project_facts,
        )
    }
}

struct DirectoryModuleCompileContext<'boundary, 'services> {
    boundary: &'boundary BoundaryCompilationContext<'services>,
    provider_store: &'boundary ModuleArtifactStore,
    /// Declaring-module generic templates already published in this boundary.
    provider_materialisations: &'boundary ProviderMaterialisationRegistry,
    provider_bindings: &'boundary [ResolvedDependencyEdge],
    provider_binding_index: &'boundary FxHashMap<(ModuleId, DependencyShellId), usize>,
    source_package_dependencies: &'boundary [ResolvedSourcePackageDependency],
    source_package_dependency_index: &'boundary FxHashMap<(ModuleId, DependencyShellId), usize>,
}

impl<'boundary, 'services> DirectoryModuleCompileContext<'boundary, 'services> {
    fn new(
        boundary: &'boundary BoundaryCompilationContext<'services>,
        provider_store: &'boundary ModuleArtifactStore,
        provider_materialisations: &'boundary ProviderMaterialisationRegistry,
        provider_bindings: &'boundary [ResolvedDependencyEdge],
        provider_binding_index: &'boundary FxHashMap<(ModuleId, DependencyShellId), usize>,
        source_package_dependencies: &'boundary [ResolvedSourcePackageDependency],
        source_package_dependency_index: &'boundary FxHashMap<(ModuleId, DependencyShellId), usize>,
    ) -> Self {
        Self {
            boundary,
            provider_store,
            provider_materialisations,
            provider_bindings,
            provider_binding_index,
            source_package_dependencies,
            source_package_dependency_index,
        }
    }
}
/// Find an authored `@project` dependency on the project package facade.
///
/// The facade is an API-only root, but its retained dependency clauses still include private and
/// otherwise unreachable source declarations. Rejecting the exact reserved root here, before
/// provider binding or AST reachability, enforces the package boundary for every declaration.
fn facade_project_globals_dependency(
    prepared: &PreparedModule,
) -> Result<Option<CompilerDiagnostic>, CompilerError> {
    let active_origin = prepared
        .semantic
        .source_module_origins
        .origin_for(prepared.semantic.active_root_file_id)?
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "facade dependency validation found no owning origin for the active root",
            )
        })?;
    if active_origin.role() != ModuleRootRole::ProjectPackageFacade {
        return Ok(None);
    }

    for clauses in prepared
        .semantic
        .prepared_header_syntax
        .module_symbols
        .file_dependency_clauses_by_source
        .values()
    {
        for clause in clauses {
            if is_project_globals_dependency(
                &clause.dependency.path,
                &prepared.semantic.string_table,
            ) {
                return Ok(Some(CompilerDiagnostic::invalid_dependency_clause(
                    clause.binding.clause_kind(),
                    InvalidDependencyClauseReason::ProjectGlobalsFacadeDependencyNotAllowed,
                    clause.dependency.location.clone(),
                )));
            }
        }
    }
    Ok(None)
}

impl<'boundary, 'services> DirectoryModuleCompileContext<'boundary, 'services> {
    /// Build the per-module provider input set by direct retained-shell lookup.
    ///
    /// Canonical jobs use the boundary indexes built once per graph. Check-only jobs instead pass
    /// their own resolved module/package records; an isolated job never falls back to canonical
    /// shell indexes, because a shell from another source must not accidentally bind merely due
    /// to sharing the owner's `ModuleId`.
    fn build_source_provider_dependencies(
        &self,
        consumer_module_id: ModuleId,
        prepared: &PreparedModule,
        check_only_provider_bindings: Option<&[module_inventory::CheckOnlyProviderBinding]>,
        check_only_source_package_dependencies: Option<
            &[module_inventory::CheckOnlySourcePackageDependency],
        >,
    ) -> Result<SourceProviderDependencySet<'boundary>, CompilerError> {
        let check_only = check_only_provider_bindings.is_some()
            || check_only_source_package_dependencies.is_some();
        let mut transient_provider_index: FxHashMap<DependencyShellId, ModuleId> =
            FxHashMap::default();
        let mut transient_package_index: FxHashMap<DependencyShellId, &str> = FxHashMap::default();

        if let Some(bindings) = check_only_provider_bindings {
            for binding in bindings {
                if transient_provider_index
                    .insert(binding.dependency_shell_id, binding.provider_module_id)
                    .is_some()
                {
                    return Err(CompilerError::compiler_error(format!(
                        "check-only ModuleId {} resolved dependency shell {:?} to more than one provider module",
                        consumer_module_id.index(),
                        binding.dependency_shell_id
                    )));
                }
            }
        }
        if let Some(dependencies) = check_only_source_package_dependencies {
            for dependency in dependencies {
                if transient_provider_index.contains_key(&dependency.dependency_shell_id)
                    || transient_package_index
                        .insert(
                            dependency.dependency_shell_id,
                            dependency.dependency_prefix.as_str(),
                        )
                        .is_some()
                {
                    return Err(CompilerError::compiler_error(format!(
                        "check-only ModuleId {} resolved dependency shell {:?} to more than one provider",
                        consumer_module_id.index(),
                        dependency.dependency_shell_id
                    )));
                }
            }
        }

        let mut dependencies = Vec::new();
        for file_dependency_clauses in prepared
            .semantic
            .prepared_header_syntax
            .module_symbols
            .file_dependency_clauses_by_source
            .values()
        {
            for clause in file_dependency_clauses {
                let shell_id = clause.dependency.dependency_shell_id;
                if is_project_globals_dependency(
                    &clause.dependency.path,
                    &prepared.semantic.string_table,
                ) {
                    let Some(project_globals) = self.boundary.project_globals else {
                        return Err(CompilerError::compiler_error(format!(
                            "ModuleId {} attempted to bind reserved @project outside its owning project boundary",
                            consumer_module_id.index()
                        )));
                    };
                    dependencies.push(SourceProviderDependency {
                        kind: ProviderDependencyKind::Authored { shell: shell_id },
                        interface: project_globals.interface(),
                    });
                    continue;
                }
                if check_only {
                    if let Some(provider_module_id) =
                        transient_provider_index.get(&shell_id).copied()
                    {
                        let interface = self
                            .provider_store
                            .interface(provider_module_id)?
                            .ok_or_else(|| {
                                CompilerError::compiler_error(format!(
                                    "Check-only ModuleId {} started semantic binding before provider ModuleId {} published a complete interface",
                                    consumer_module_id.index(),
                                    provider_module_id.index()
                                ))
                            })?;
                        dependencies.push(SourceProviderDependency {
                            kind: ProviderDependencyKind::Authored { shell: shell_id },
                            interface,
                        });
                        continue;
                    }

                    if let Some(dependency_prefix) = transient_package_index.get(&shell_id).copied()
                    {
                        let package_id = self
                            .boundary
                            .completed_packages
                            .by_prefix(dependency_prefix)
                            .ok_or_else(|| {
                                CompilerError::compiler_error(format!(
                                    "Check-only ModuleId {} started semantic binding before source package @{} completed",
                                    consumer_module_id.index(),
                                    dependency_prefix
                                ))
                            })?;
                        let completed_package =
                            self.boundary.completed_packages.package(package_id)?;
                        dependencies.push(SourceProviderDependency {
                            kind: ProviderDependencyKind::Authored { shell: shell_id },
                            interface: completed_package.root_interface()?,
                        });
                    }

                    // Same-owner source dependencies and provider clauses handled by the external
                    // registry intentionally have no transient interface record. Do not consult
                    // the canonical indexes for this shell.
                    continue;
                }

                if let Some(binding_index) = self
                    .provider_binding_index
                    .get(&(consumer_module_id, shell_id))
                {
                    let binding = &self.provider_bindings[*binding_index];
                    let interface = self
                        .provider_store
                        .interface(binding.provider_module_id)?
                        .ok_or_else(|| {
                            CompilerError::compiler_error(format!(
                                "ModuleId {} started semantic binding before provider ModuleId {} published a complete interface",
                                consumer_module_id.index(),
                                binding.provider_module_id.index()
                            ))
                        })?;

                    dependencies.push(SourceProviderDependency {
                        kind: ProviderDependencyKind::Authored { shell: shell_id },
                        interface,
                    });
                    continue;
                }

                let Some(package_index) = self
                    .source_package_dependency_index
                    .get(&(consumer_module_id, shell_id))
                else {
                    continue;
                };
                let package_dependency = &self.source_package_dependencies[*package_index];
                let package_id = self
                    .boundary
                    .completed_packages
                    .by_prefix(package_dependency.dependency_prefix.as_str())
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "ModuleId {} started semantic binding before source package @{} completed",
                            consumer_module_id.index(),
                            package_dependency.dependency_prefix
                        ))
                    })?;
                let completed_package = self.boundary.completed_packages.package(package_id)?;

                dependencies.push(SourceProviderDependency {
                    kind: ProviderDependencyKind::Authored { shell: shell_id },
                    interface: completed_package.root_interface()?,
                });
            }
        }

        // Builder source-backed packages are implicitly available only to modules that actually
        // contain a `.mtf` semantic source. The package capability is supplied by the active
        // builder surface; generic orchestration must not infer it from a package-name list.
        if prepared.contains_moth_template {
            let implicit_provider_dependencies: Vec<SourceProviderDependency<'boundary>> = self
                .boundary
                .implicit_template_package_ids
                .iter()
                .map(|package_id| {
                    let package = self.boundary.completed_packages.package(*package_id)?;
                    let interface = package.root_interface()?;
                    Ok(SourceProviderDependency {
                        kind: ProviderDependencyKind::ImplicitTemplate {
                            package_prefix: package.package_prefix(),
                        },
                        interface,
                    })
                })
                .collect::<Result<_, CompilerError>>()?;

            dependencies.extend(implicit_provider_dependencies);
        }

        SourceProviderDependencySet::new(dependencies)
    }

    /// Return the first failed canonical provider for a transient job, if any.
    ///
    /// Check-only units have no graph slot of their own. A diagnosed or blocked provider therefore
    /// suppresses the dependent unit instead of letting interface lookup turn the provider's
    /// authoritative diagnostic into a secondary infrastructure error. An unavailable slot is
    /// still an internal scheduling failure: canonical publication should have completed first.
    fn check_only_blocked_provider(
        &self,
        consumer_module_id: ModuleId,
        provider_bindings: &[module_inventory::CheckOnlyProviderBinding],
        source_package_dependencies: &[module_inventory::CheckOnlySourcePackageDependency],
    ) -> Result<Option<BlockedProvider>, CompilerError> {
        for binding in provider_bindings {
            match self.provider_store.slot(binding.provider_module_id)? {
                ProviderSlot::Successful(_) => {}
                ProviderSlot::Diagnosed | ProviderSlot::Blocked => {
                    return Ok(Some(BlockedProvider::Module(binding.provider_module_id)));
                }
                ProviderSlot::Unavailable => {
                    return Err(CompilerError::compiler_error(format!(
                        "Check-only ModuleId {} became ready before provider ModuleId {} completed",
                        consumer_module_id.index(),
                        binding.provider_module_id.index()
                    )));
                }
            }
        }

        for dependency in source_package_dependencies {
            let package_id = self
                .boundary
                .completed_packages
                .by_prefix(dependency.dependency_prefix.as_str())
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Check-only ModuleId {} depends on unindexed source package @{}",
                        consumer_module_id.index(),
                        dependency.dependency_prefix
                    ))
                })?;
            let package = self.boundary.completed_packages.package(package_id)?;
            match package.root_slot()? {
                ProviderSlot::Successful(_) => {}
                ProviderSlot::Diagnosed | ProviderSlot::Blocked => {
                    return Ok(Some(BlockedProvider::SourcePackage(
                        package.package_identity.clone(),
                    )));
                }
                ProviderSlot::Unavailable => {
                    return Err(CompilerError::compiler_error(format!(
                        "Check-only ModuleId {} became ready before source package @{} completed its facade",
                        consumer_module_id.index(),
                        package.package_prefix()
                    )));
                }
            }
        }

        Ok(None)
    }

    fn compile(
        &self,
        job: module_inventory::ModuleCompilationJob,
        known_generated: KnownGeneratedFunctions<'_>,
    ) -> DirectoryModuleTaskResult {
        let module_inventory::ModuleCompilationJob {
            module_id,
            string_table_base_len: base_len,
            prepared,
            #[cfg(feature = "timers")]
            timing_module_key,
            ..
        } = job;

        // The dense graph `ModuleId` is the module key inside this boundary, so attribution
        // stays deterministic and independent of worker completion order.
        #[cfg(feature = "timers")]
        let module_context = Some(crate::timing::TimingContext::for_module(timing_module_key));

        #[cfg(feature = "timers")]
        {
            self.compile_prepared(
                module_id,
                base_len,
                prepared,
                known_generated,
                None,
                None,
                None,
                None,
                None,
                module_context,
            )
        }
        #[cfg(not(feature = "timers"))]
        {
            self.compile_prepared(
                module_id,
                base_len,
                prepared,
                known_generated,
                None,
                None,
                None,
                None,
                None,
            )
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn compile_prepared(
        &self,
        module_id: ModuleId,
        base_len: usize,
        prepared: PreparedModule,
        known_generated: KnownGeneratedFunctions<'_>,
        build_config_values_override: Option<&ResolvedBuildConfigMap>,
        check_only_provider_bindings: Option<&[module_inventory::CheckOnlyProviderBinding]>,
        check_only_source_package_dependencies: Option<
            &[module_inventory::CheckOnlySourcePackageDependency],
        >,
        external_packages: Option<Arc<ExternalPackageRegistry>>,
        external_dependency_resolution_table: Option<&ExternalImportResolutionTable>,
        #[cfg(feature = "timers")] module_context: Option<crate::timing::TimingContext>,
    ) -> DirectoryModuleTaskResult {
        match facade_project_globals_dependency(&prepared) {
            Ok(Some(diagnostic)) => {
                let messages = CompilerMessages::from_diagnostic(
                    diagnostic,
                    prepared.semantic.string_table.clone(),
                );
                let outcome = match ModuleDiagnostics::from_messages(messages) {
                    Ok(diagnostics) => DirectoryModuleTaskOutcome::Diagnosed(diagnostics),
                    Err(error) => DirectoryModuleTaskOutcome::Infrastructure(error),
                };
                return DirectoryModuleTaskResult {
                    module_id,
                    string_table_base_len: base_len,
                    outcome,
                };
            }
            Ok(None) => {}
            Err(error) => {
                return DirectoryModuleTaskResult {
                    module_id,
                    string_table_base_len: base_len,
                    outcome: DirectoryModuleTaskOutcome::Infrastructure(error),
                };
            }
        }

        // Semantic compilation is provider-dependent, so every required provider interface must
        // already be published before this call. Canonical jobs use the graph indexes; check-only
        // jobs use the isolated metadata prepared with their own source headers.
        let source_provider_dependencies = match self.build_source_provider_dependencies(
            module_id,
            &prepared,
            check_only_provider_bindings,
            check_only_source_package_dependencies,
        ) {
            Ok(dependencies) => dependencies,
            Err(error) => {
                return DirectoryModuleTaskResult {
                    module_id,
                    string_table_base_len: base_len,
                    outcome: DirectoryModuleTaskOutcome::Infrastructure(error),
                };
            }
        };
        // Canonical modules consume the one boundary-owned map directly. Only transient
        // check-only jobs provide an isolated override resolved from their own source facts.
        let effective_build_config_values = build_config_values_override
            .map(|values| Arc::new(values.clone()))
            .unwrap_or_else(|| Arc::clone(&self.boundary.build_config_values));

        let effective_external_packages =
            external_packages.unwrap_or_else(|| Arc::clone(self.boundary.external_packages));
        let effective_external_dependency_resolution_table = external_dependency_resolution_table
            .unwrap_or(
                &self
                    .boundary
                    .builder_surface
                    .external_dependency_resolution_table,
            );
        let compile_context = ModuleCompilationContext {
            options: self.boundary.config.frontend_options(),
            build_profile: self.boundary.build_profile,
            root_role_override: (check_only_provider_bindings.is_some()
                || check_only_source_package_dependencies.is_some())
            .then_some(ModuleRootRole::Support),
            project_path_resolver: Some(self.boundary.project_path_resolver.clone()),
            style_directives: self.boundary.style_directives,
            external_packages: effective_external_packages,
            build_config_values: effective_build_config_values,
            external_dependency_resolution_table: effective_external_dependency_resolution_table,
            source_provider_dependencies: &source_provider_dependencies,
            provider_materialisations: self.provider_materialisations,
            builder_runtime_packages: &self.boundary.builder_surface.builder_runtime_packages,
        };

        // The typed semantic boundary already classified user diagnostics from infrastructure
        // failures, so the task outcome carries the retained `ModuleDiagnostics` unchanged.
        timing_scope_attributed!(
            timing_guard_frontend_module_semantic_total_2,
            crate::timing::TimingMetric::FrontendModuleSemanticTotal,
            module_context,
        );
        #[cfg(feature = "timers")]
        let semantic_result = compile_module(
            &compile_context,
            prepared.semantic,
            known_generated,
            module_context,
        );
        #[cfg(not(feature = "timers"))]
        let semantic_result = compile_module(&compile_context, prepared.semantic, known_generated);
        #[cfg(feature = "timers")]
        timing_guard_frontend_module_semantic_total_2.finish();
        let outcome = match semantic_result {
            Ok(ModuleCompilationOutcome::Success(compiled)) => {
                DirectoryModuleTaskOutcome::Success(compiled)
            }
            Ok(ModuleCompilationOutcome::Diagnosed(diagnostics)) => {
                DirectoryModuleTaskOutcome::Diagnosed(diagnostics)
            }
            Err(error) => DirectoryModuleTaskOutcome::Infrastructure(error),
        };
        DirectoryModuleTaskResult {
            module_id,
            string_table_base_len: base_len,
            outcome,
        }
    }
}
fn compile_check_only_job(
    compile_context: &DirectoryModuleCompileContext<'_, '_>,
    job: module_inventory::CheckOnlyModuleCompilationJob,
    known_generated: KnownGeneratedFunctions<'_>,
    build_config_index: &BuildConfigResolutionIndex<'_>,
) -> DirectoryModuleTaskResult {
    let module_inventory::CheckOnlyModuleCompilationJob {
        owner_module_id: module_id,
        string_table_base_len: base_len,
        provider_bindings,
        source_package_dependencies,
        external_packages,
        external_dependency_resolution_table,
        mut prepared,
        ..
    } = job;
    match compile_context.check_only_blocked_provider(
        module_id,
        &provider_bindings,
        &source_package_dependencies,
    ) {
        Ok(Some(_provider)) => {
            return DirectoryModuleTaskResult {
                module_id,
                string_table_base_len: base_len,
                outcome: DirectoryModuleTaskOutcome::Blocked,
            };
        }
        Ok(None) => {}
        Err(error) => {
            return DirectoryModuleTaskResult {
                module_id,
                string_table_base_len: base_len,
                outcome: DirectoryModuleTaskOutcome::Infrastructure(error),
            };
        }
    }

    // Resolve this transient unit against borrowed canonical facts and only its own source
    // facts. The transient slice is private to this job: it validates compatibility and
    // selects values without copying or comparing a sibling check-only unit.
    let check_only_source_facts =
        config_boundary::source_contract_facts_for_current_module(&prepared);
    let check_only_inputs = build_config_index.filter_inputs_to_known_facts(
        &compile_context.boundary.build_config_inputs,
        &check_only_source_facts,
    );
    let check_only_build_config_values = match build_config_index
        .resolve_with_transient_source_facts(
            &check_only_source_facts,
            &check_only_inputs,
            &compile_context.boundary.builder_globals,
        ) {
        Ok(values) => values,
        Err(error) => {
            let fallback_location = error
                .contract_location()
                .cloned()
                .unwrap_or_else(SourceLocation::default);
            let messages = config_boundary::build_config_resolution_messages(
                error,
                fallback_location,
                &mut prepared.semantic.string_table,
            );
            let outcome = match ModuleDiagnostics::from_messages(messages) {
                Ok(diagnostics) => DirectoryModuleTaskOutcome::Diagnosed(diagnostics),
                Err(error) => DirectoryModuleTaskOutcome::Infrastructure(error),
            };
            return DirectoryModuleTaskResult {
                module_id,
                string_table_base_len: base_len,
                outcome,
            };
        }
    };

    #[cfg(feature = "timers")]
    {
        compile_context.compile_prepared(
            module_id,
            base_len,
            prepared,
            known_generated,
            Some(&check_only_build_config_values),
            Some(&provider_bindings),
            Some(&source_package_dependencies),
            Some(external_packages),
            Some(&external_dependency_resolution_table),
            None,
        )
    }
    #[cfg(not(feature = "timers"))]
    {
        compile_context.compile_prepared(
            module_id,
            base_len,
            prepared,
            known_generated,
            Some(&check_only_build_config_values),
            Some(&provider_bindings),
            Some(&source_package_dependencies),
            Some(external_packages),
            Some(&external_dependency_resolution_table),
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compile_check_only_jobs(
    context: &BoundaryCompilationContext<'_>,
    provider_store: &ModuleArtifactStore,
    generated_store: &BoundaryGeneratedFunctionStore,
    provider_materialisations: &ProviderMaterialisationRegistry,
    check_only_jobs: Vec<module_inventory::CheckOnlyModuleCompilationJob>,
    provider_bindings: &[ResolvedDependencyEdge],
    provider_binding_index: &rustc_hash::FxHashMap<(ModuleId, DependencyShellId), usize>,
    source_package_dependencies: &[ResolvedSourcePackageDependency],
    source_package_dependency_index: &rustc_hash::FxHashMap<(ModuleId, DependencyShellId), usize>,
    build_config_index: &BuildConfigResolutionIndex<'_>,
    string_table: &mut StringTable,
) -> Result<Vec<CompilerMessages>, CompilerMessages> {
    // Check-only units are semantically compiled after canonical publication, but their
    // successful artefacts, interfaces, generated deltas and resource associations are discarded.
    // Only their diagnostics/warnings cross the frontend result boundary.
    add_frontend_counter(
        FrontendCounter::ModuleCompilationSerialCount,
        check_only_jobs.len(),
    );
    let mut transient_messages = Vec::new();
    for check_only_job in check_only_jobs {
        let outcome = {
            let compile_context = DirectoryModuleCompileContext::new(
                context,
                provider_store,
                provider_materialisations,
                provider_bindings,
                provider_binding_index,
                source_package_dependencies,
                source_package_dependency_index,
            );
            compile_check_only_job(
                &compile_context,
                check_only_job,
                generated_store.known_generated(),
                build_config_index,
            )
        };
        match outcome.outcome {
            DirectoryModuleTaskOutcome::Success(compiled) => {
                let ModuleSemanticResult {
                    module,
                    generated_delta,
                    string_table: module_string_table,
                    ..
                } = *compiled;
                let mut warnings = module.metadata.warnings;
                warnings.extend(
                    generated_delta
                        .records()
                        .iter()
                        .flat_map(|record| record.sidecar.module.metadata.warnings.iter().cloned()),
                );
                if !warnings.is_empty() {
                    let mut messages =
                        CompilerMessages::from_diagnostics(warnings, module_string_table);
                    let remap = string_table
                        .merge_delta_from(&messages.string_table, outcome.string_table_base_len);
                    if !remap.is_identity() {
                        messages.remap_string_ids(&remap);
                    }
                    transient_messages.push(messages);
                }
            }
            DirectoryModuleTaskOutcome::Diagnosed(diagnostics) => {
                let mut messages = diagnostics.into_messages();
                let remap = string_table
                    .merge_delta_from(&messages.string_table, outcome.string_table_base_len);
                if !remap.is_identity() {
                    messages.remap_string_ids(&remap);
                }
                transient_messages.push(messages);
            }
            DirectoryModuleTaskOutcome::Blocked => {
                // The failed canonical provider's own diagnostics remain authoritative; a
                // dependent check-only unit contributes no cascade diagnostics.
            }
            DirectoryModuleTaskOutcome::Infrastructure(error) => {
                return Err(CompilerMessages::from_error_ref(error, string_table));
            }
        }
    }

    Ok(transient_messages)
}
#[allow(clippy::too_many_arguments)]
pub(super) fn compile_module_waves(
    context: BoundaryCompilationContext<'_>,
    graph: ProjectModuleGraph,
    module_waves: Vec<Vec<module_inventory::ModuleCompilationJob>>,
    check_only_jobs: Vec<module_inventory::CheckOnlyModuleCompilationJob>,
    provider_bindings: &[ResolvedDependencyEdge],
    source_package_dependencies: &[ResolvedSourcePackageDependency],
    resource_inputs: &mut ResourceInputRegistry,
    string_table: &mut StringTable,
) -> Result<(CompiledGraphBoundary, Vec<CompilerMessages>), CompilerMessages> {
    let mut provider_store = ModuleArtifactStore::new(graph.nodes().len());
    let mut generated_store = BoundaryGeneratedFunctionStore::default();
    // The compiler resolves declaring generic templates through this registry, so it never reads a
    // live build store while semantic analysis runs. Completed packages seed it; each successful
    // module in this boundary extends it as it publishes.
    let mut provider_materialisations =
        seed_completed_package_materialisations(context.completed_packages)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    // One direct lookup index per boundary so module binding never scans every provider edge,
    // source-package dependency or completed package for each retained dependency shell.
    let provider_binding_index = build_provider_binding_index(provider_bindings)
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    let source_package_dependency_index =
        build_source_package_dependency_index(&provider_binding_index, source_package_dependencies)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
    // Canonical fact ownership is indexed once per boundary. Transient units borrow this index
    // instead of cloning and concatenating the canonical fact vector for every unit.
    let build_config_index = context.build_config_resolution_index();

    // Index each consumer module's direct package dependencies once per boundary so readiness
    // walks only the packages that module actually depends on and never filters the full dependency
    // vector for every job.
    let module_package_dependencies = build_module_package_dependency_index(
        source_package_dependencies,
        context.completed_packages,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

    let mut diagnosed = Vec::new();
    let mut blocked = Vec::new();
    for wave in module_waves {
        add_frontend_counter(FrontendCounter::ModuleCompilationSerialCount, wave.len());
        let mut ready = Vec::new();
        for job in wave {
            let mut blocked_provider = None;
            for provider_id in graph
                .dependency_providers(job.module_id)
                .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?
            {
                match provider_store
                    .slot(*provider_id)
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?
                {
                    ProviderSlot::Successful(_) => {}
                    ProviderSlot::Diagnosed | ProviderSlot::Blocked => {
                        blocked_provider = Some(BlockedProvider::Module(*provider_id));
                        break;
                    }
                    ProviderSlot::Unavailable => {
                        let error = CompilerError::compiler_error(format!(
                            "ModuleId {} became ready before provider ModuleId {} completed",
                            job.module_id.index(),
                            provider_id.index()
                        ));
                        return Err(CompilerMessages::from_error_ref(error, string_table));
                    }
                }
            }

            if blocked_provider.is_none()
                && let Some(package_ids) = module_package_dependencies.get(&job.module_id)
            {
                for package_id in package_ids {
                    let package = context
                        .completed_packages
                        .package(*package_id)
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;

                    match package
                        .root_slot()
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?
                    {
                        ProviderSlot::Successful(_) => {}
                        ProviderSlot::Diagnosed | ProviderSlot::Blocked => {
                            blocked_provider = Some(BlockedProvider::SourcePackage(
                                package.package_identity.clone(),
                            ));
                            break;
                        }
                        ProviderSlot::Unavailable => {
                            let error = CompilerError::compiler_error(format!(
                                "ModuleId {} became ready before source package @{} completed its facade",
                                job.module_id.index(),
                                package.package_prefix()
                            ));
                            return Err(CompilerMessages::from_error_ref(error, string_table));
                        }
                    }
                }
            }

            if let Some(required_provider) = blocked_provider {
                provider_store
                    .mark_blocked(job.module_id)
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                blocked.push(BlockedModule {
                    module_id: job.module_id,
                    required_provider,
                });
            } else {
                ready.push(job);
            }
        }

        ready.sort_by_key(|job| job.module_id.index());
        for job in ready {
            // This owner publishes each successful module transaction before starting the next
            // ModuleId, so duplicate requests in one ready wave materialise exactly once. File
            // preparation remains parallel inside each module. Semantic module-wave parallelism
            // remains a separate future phase because generated deltas currently commit
            // deterministically through this serial publication owner.
            let outcome = {
                let compile_context = DirectoryModuleCompileContext::new(
                    &context,
                    &provider_store,
                    &provider_materialisations,
                    provider_bindings,
                    &provider_binding_index,
                    source_package_dependencies,
                    &source_package_dependency_index,
                );
                compile_context.compile(job, generated_store.known_generated())
            };
            match outcome.outcome {
                DirectoryModuleTaskOutcome::Success(compiled) => {
                    let compiled = *compiled;
                    let remap = string_table
                        .merge_delta_from(&compiled.string_table, outcome.string_table_base_len);
                    let ModuleSemanticResult {
                        mut module,
                        mut generated_delta,
                        resource_source_associations,
                        string_table: _,
                        public_interface,
                    } = compiled;
                    if !remap.is_identity() {
                        module.remap_string_ids(&remap);
                        generated_delta.remap_string_ids(&remap);
                    }
                    let artifact = CompiledModuleArtifact {
                        module,
                        interface: public_interface,
                    };
                    publish_module_and_generated(ModuleBoundaryPublication {
                        modules: &mut provider_store,
                        generated: &mut generated_store,
                        materialisations: &mut provider_materialisations,
                        resource_inputs,
                        module_id: outcome.module_id,
                        expected_origin: graph.node(outcome.module_id).stable_origin(),
                        artifact,
                        generated_delta,
                        resource_source_associations,
                    })
                    .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                }
                DirectoryModuleTaskOutcome::Diagnosed(diagnostics) => {
                    provider_store
                        .mark_diagnosed(outcome.module_id)
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                    let mut messages = diagnostics.into_messages();
                    let remap = string_table
                        .merge_delta_from(&messages.string_table, outcome.string_table_base_len);
                    if !remap.is_identity() {
                        messages.remap_string_ids(&remap);
                    }
                    let diagnostics = ModuleDiagnostics::from_messages(messages)
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
                    diagnosed.push(DiagnosedModule {
                        module_id: outcome.module_id,
                        diagnostics,
                    });
                }
                DirectoryModuleTaskOutcome::Blocked => {
                    return Err(CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(format!(
                            "canonical ModuleId {} unexpectedly became transient-blocked",
                            outcome.module_id.index()
                        )),
                        string_table,
                    ));
                }
                DirectoryModuleTaskOutcome::Infrastructure(error) => {
                    return Err(CompilerMessages::from_error_ref(error, string_table));
                }
            }
        }
    }

    let transient_messages = compile_check_only_jobs(
        &context,
        &provider_store,
        &generated_store,
        &provider_materialisations,
        check_only_jobs,
        provider_bindings,
        &provider_binding_index,
        source_package_dependencies,
        &source_package_dependency_index,
        &build_config_index,
        string_table,
    )?;

    let diagnosed_provider_exists = !diagnosed.is_empty()
        || context
            .completed_packages
            .iter()
            .any(|package| !package.boundary.diagnosed.is_empty());
    if !blocked.is_empty() && !diagnosed_provider_exists {
        return Err(CompilerMessages::from_error_ref(
            CompilerError::compiler_error(format!(
                "Graph retained {} blocked modules without a diagnosed provider",
                blocked.len()
            )),
            string_table,
        ));
    }

    let boundary = CompiledGraphBoundary {
        structure: graph,
        modules: provider_store,
        generated: generated_store,
        diagnosed,
        blocked,
    };
    boundary
        .finish()
        .map(|boundary| (boundary, transient_messages))
        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))
}
