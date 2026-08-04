//! Core build orchestration for Moth projects.
//!
//! This module provides the canonical project build flow (`build_project`). Build tools can
//! compile once and pass the resulting project to the output subsystem without reimplementing
//! frontend/backend orchestration.

use crate::build_system::BuildProfile;
use crate::build_system::create_project_modules::compiled_boundary::{
    CompiledGraphBoundary, CompiledModuleRef, CompletedSourcePackageRegistry,
    ProjectFrontendCompilation, compilation_module_views,
};
use crate::build_system::create_project_modules::{
    compile_project_frontend, resolve_project_entry_root,
};
use crate::build_system::output::{
    BuilderKind, CleanupPolicy, OutputOwner, ValidatedDirectoryOutputSettings, ValidatedOutputPlan,
};
use crate::build_system::path_validation::check_if_valid_path;
use crate::build_system::project_config::{ProjectConfigParseServices, load_project_config};

use crate::compiler_frontend::Flag;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationContext;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::{
    HirModuleLinkFacts, HirReachability, collect_reachability_from_function_link_facts,
};
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::module_metadata::{HirLoweringMetadata, ModuleDocFragment};
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    ModulePrivateExecutableIdentity, OriginFunctionId,
};

#[cfg(test)]
use crate::build_system::create_project_modules::generated_worklist::BoundaryGeneratedFunctionStore;
#[cfg(test)]
use crate::build_system::create_project_modules::module_artifact_store::ModuleArtifactStore;
#[cfg(test)]
use crate::build_system::create_project_modules::module_identity::ModuleId;
#[cfg(test)]
use crate::build_system::create_project_modules::project_module_graph::ProjectModuleGraph;
#[cfg(test)]
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::style_directives::{StyleDirectiveRegistry, StyleDirectiveSpec};
use crate::compiler_frontend::symbols::compiler_symbols::CompilerSymbolSet;
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};

use crate::builder_surface::BuilderSurface;
use crate::builder_surface::external_import_providers::provider::{
    RequiredRuntimeImport, RuntimeAssetIdentity,
};
use crate::compiler_frontend::external_packages::{ExternalPackageId, ExternalPackageRegistry};
use crate::compiler_frontend::paths::rendered_path_usage::RenderedPathUsage;
use crate::projects::settings::{Config, ProjectConfigError};

use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const FILE_MIN_UNIQUE_SYMBOLS_CAPACITY: usize = 32;

// -------------------------
//  Build Payload Types
// -------------------------

/// A resolved const top-level fragment: a static string and its runtime insertion index.
///
/// WHAT: carries a fully resolved (not interned) const fragment string plus the count of
/// runtime fragments that precede it in source order.
/// WHY: builders merge const strings with the runtime fragment list returned by entry start()
/// using the insertion index to reconstruct source-order interleaving.
pub struct ResolvedConstFragment {
    /// Number of runtime fragments preceding this const fragment in source order.
    pub runtime_insertion_index: usize,
    /// The rendered text content of this const fragment.
    pub rendered_text: String,
}

/// Build-system-owned metadata for one external import used by a compiled module.
///
/// WHAT: carries the backend-facing identity for a provider-resolved external import after
///       deduplication across a module's source files.
/// WHY: backends emit runtime assets and generated glue based on this metadata without needing
///      the full per-source-file resolution table.
#[derive(Debug, Clone)]
// Kept ahead of the backend handoff: external provider/runtime metadata is recorded by the
// frontend today, while the current HTML path only reads the subset it can lower.
pub(crate) struct ModuleExternalImport {
    pub(crate) package_id: ExternalPackageId,
    pub(crate) runtime_asset: Option<RuntimeAssetIdentity>,
    pub(crate) required_runtime_imports: Vec<RequiredRuntimeImport>,
}

/// Header-derived root activity metadata passed to backend builders.
///
/// WHAT: records the builder-relevant dormant activity of one compiled module root.
/// WHY: header parsing already classifies root bodies and page fragments, so builders can apply
///      artifact policy without scanning tokens or HIR for the same facts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ModuleRootActivity {
    pub(crate) has_non_trivial_root_body: bool,
    pub(crate) const_fragment_count: usize,
    pub(crate) runtime_fragment_count: usize,
}

impl ModuleRootActivity {
    /// Return whether the HTML builder has any root activity from which to assemble a page.
    pub(crate) fn has_html_artifact_activity(&self) -> bool {
        self.has_non_trivial_root_body
            || self.const_fragment_count > 0
            || self.runtime_fragment_count > 0
    }
}

/// Module-local executable semantic state: validated HIR, paired type environment and borrow
/// facts.
///
/// WHAT: the sole owner of the typed HIR, its paired `TypeEnvironment` and the
///       `BorrowCheckReport` produced by borrow validation.
/// WHY: keeping these together in one executable lane makes the HIR/type/borrow pairing obvious
///      at every backend call site and lets string-ID remapping cover HIR, type identity and
///      source locations retained by borrow facts exactly once.
pub(crate) struct ModuleExecutable {
    pub(crate) hir: HirModule,
    pub(crate) type_environment: TypeEnvironment,
    pub(crate) borrow_analysis: BorrowCheckReport,
}

impl ModuleExecutable {
    /// Remap interned string IDs after string-table merging.
    ///
    /// WHY: HIR, type identity and borrow-fact source locations remap exactly once here.
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.hir.remap_string_ids(remap);
        self.type_environment.remap_string_ids(remap);
        self.borrow_analysis.remap_string_ids(remap);
    }
}

/// Backend-neutral link facts for one compiled module.
///
/// WHAT: owns deterministic base-function facts, available external runtime import candidates
///       and the effective external package registry resolved for this module.
/// WHY: entry assembly derives exact reachable unions from the function facts while backend
///      validation and lowering still need the same external symbol definitions the frontend used.
pub(crate) struct ModuleLinkFacts {
    /// Effective external package registry after provider resolution for this module.
    ///
    /// WHY: provider-backed import discovery mutates the registry during Stage 0; the module
    ///      must carry the effective registry so backends validate and lower against the same
    ///      symbols the frontend resolved, rather than reconstructing a fresh registry that
    ///      loses provider-created packages. R6A replaces this temporary complete registry with
    ///      stable binding identities after the canonical provider path exists.
    pub(crate) external_package_registry: Arc<ExternalPackageRegistry>,
    /// All provider and builder runtime imports available to this module's executable functions.
    /// Entry assembly filters these candidates through per-function reachability before any
    /// runtime asset, import-map or glue consumer sees them.
    pub(crate) external_import_candidates: Vec<ModuleExternalImport>,
    /// Direct facts for every base HIR function in deterministic function-ID order.
    pub(crate) functions: HirModuleLinkFacts,
}

/// Non-HIR compiler and builder-facing metadata for one compiled module.
///
/// WHAT: owns the resolved root-local entry path, module warnings, resolved const top-level
///       fragments, header-derived root activity, resolved documentation fragments, and
///       rendered-path usages.
/// WHY: these are compiler-metadata lanes, not executable HIR state or link facts. Consolidating
///      them into one owned lane on the `Module` payload keeps HIR limited to executable/semantic
///      IR and gives string-ID remapping, warning collection, and tracked-asset planning a single
///      owner. The architecture assigns resolved root-local entry metadata to this lane.
pub(crate) struct ModuleCompilerMetadata {
    /// Canonical entry file for the compiled module.
    pub(crate) entry_point: PathBuf,
    pub(crate) warnings: Vec<CompilerDiagnostic>,
    pub(crate) const_top_level_fragments: Vec<ResolvedConstFragment>,
    pub(crate) root_activity: ModuleRootActivity,
    pub(crate) doc_fragments: Vec<ModuleDocFragment>,
    pub(crate) rendered_path_usages: Vec<RenderedPathUsage>,
    /// Self-contained declaring-module semantics used by generated-function materialisation.
    pub(crate) materialisation_context: Option<ModuleMaterialisationContext>,
}

impl ModuleCompilerMetadata {
    pub(crate) fn from_hir_lowering(
        entry_point: PathBuf,
        warnings: Vec<CompilerDiagnostic>,
        lowering_metadata: HirLoweringMetadata,
        const_top_level_fragments: Vec<ResolvedConstFragment>,
        root_activity: ModuleRootActivity,
        materialisation_context: Option<ModuleMaterialisationContext>,
    ) -> Self {
        Self {
            entry_point,
            warnings,
            doc_fragments: lowering_metadata.doc_fragments,
            rendered_path_usages: lowering_metadata.rendered_path_usages,
            const_top_level_fragments,
            root_activity,
            materialisation_context,
        }
    }

    /// Remap interned string IDs after string-table merging.
    ///
    /// WHY: warnings, documentation locations, and rendered-path interned fields must all remap
    ///      exactly once. Const fragment rendered text is already a resolved `String`, root
    ///      activity carries no interned fields, and the entry path is a `PathBuf`.
    ///
    /// Materialisation metadata owns self-contained strings and stable semantic identities, so
    /// this remap covers only executable presentation fields that retain local `StringId` values.
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for warning in &mut self.warnings {
            warning.remap_string_ids(remap);
        }

        for fragment in &mut self.doc_fragments {
            fragment.location.remap_string_ids(remap);
        }

        for usage in &mut self.rendered_path_usages {
            usage.source_path.remap_string_ids(remap);
            usage.public_path.remap_string_ids(remap);
            usage.source_file_scope.remap_string_ids(remap);
            usage.render_location.remap_string_ids(remap);
        }
    }
}

/// Frontend output for one module root ready for backend lowering.
///
/// WHAT: a lane container with exactly an executable lane (typed HIR, paired type environment
///       and borrow facts), a link-facts lane (per-function runtime facts, external imports and
///       the effective registry), and a compiler-metadata lane (entry path, warnings, fragments,
///       root activity, docs and rendered paths).
/// WHY: backends consume one stable module payload shape regardless of project type, with
///      explicit ownership keeping HIR/type/borrow pairing obvious at call sites.
pub struct Module {
    pub(crate) executable: ModuleExecutable,
    pub(crate) link_facts: ModuleLinkFacts,
    pub(crate) metadata: ModuleCompilerMetadata,
}

/// One independently lowered concrete generic executable.
///
/// The stable identity is stored beside, rather than rediscovered from, its HIR module. Base
/// canonical modules and generated sidecars therefore remain distinct project-compilation lanes.
pub(crate) struct GeneratedFunctionSidecar {
    pub(crate) identity: crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity,
    pub(crate) module: Module,
}

impl GeneratedFunctionSidecar {
    pub(crate) fn new(
        identity: crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity,
        module: Module,
    ) -> Self {
        Self { identity, module }
    }

    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.module.remap_string_ids(remap);
    }
}

/// One successful canonical module artefact.
///
/// WHAT: pairs the backend-neutral executable/link/metadata lanes with the immutable semantic
/// interface published to later graph waves.
/// WHY: provider publication must point at a complete success value. Keeping both lanes in one
/// artefact prevents the scheduler from publishing a local draft or dropping the interface while
/// retaining only backend state.
pub(crate) struct CompiledModuleArtifact {
    pub(crate) module: Module,
    pub(crate) interface: PublicSemanticInterface,
}

/// Success-only frontend payload consumed by project builders.
///
/// WHAT: owns the retained project and source-package graph boundaries, the explicit entry
///       assemblies selected from dormant root activity, and the generated sidecar lane inside
///       each boundary. It retains immutable [`CompiledModuleArtifact`] values and their dense
///       `ModuleId` mapping so the published [`PublicSemanticInterface`] and boundary identity
///       survive into builders and link owners.
/// WHY: project builders need a coherent project boundary with build-owned entry selection. A
///      diagnosed frontend never constructs this value, and backends no longer infer entries by
///      filtering a flat module vector. Entry selection resolves the project graph's normal
///      entry identities through the retained dense mapping instead of mutating package module
///      metadata.
pub struct ProjectCompilation {
    /// Retained project graph boundary with its artefact store and generated lane.
    project: CompiledGraphBoundary,
    /// Retained source-package boundaries, each with its own dense identity space.
    source_packages: CompletedSourcePackageRegistry,
    entries: Vec<EntryAssembly>,
    source_function_names: Arc<std::collections::HashMap<OriginFunctionId, String>>,
    module_private_function_names:
        Arc<std::collections::HashMap<ModulePrivateExecutableIdentity, String>>,
    generated_function_names: Arc<
        std::collections::HashMap<
            crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity,
            String,
        >,
    >,
}

impl ProjectCompilation {
    pub(crate) fn from_frontend(
        frontend: ProjectFrontendCompilation,
    ) -> Result<Self, CompilerError> {
        let ProjectFrontendCompilation {
            project,
            source_packages,
        } = frontend;
        Self::from_successful_boundaries(project, source_packages)
    }

    #[cfg(test)]
    pub(crate) fn from_test_modules(modules: Vec<Module>) -> Result<Self, CompilerError> {
        let module_count = modules.len();
        let graph = ProjectModuleGraph::from_normal_roots(
            (0..module_count)
                .map(|index| {
                    let origin = StableModuleOriginIdentity::from_portable_path(
                        StablePackageIdentity::project_local("test"),
                        format!("module_{index}"),
                        ModuleRootRole::Normal,
                    );
                    let root_path = PathBuf::from(format!("@module_{index}.moth"));
                    (origin, root_path.clone(), root_path)
                })
                .collect(),
        );
        let mut module_store = ModuleArtifactStore::new(module_count);
        for (index, module) in modules.into_iter().enumerate() {
            let module_id = ModuleId::from_index(index);
            module_store.publish_success(
                module_id,
                CompiledModuleArtifact {
                    module,
                    interface: test_public_interface(index),
                },
            )?;
        }
        let project = CompiledGraphBoundary {
            structure: graph,
            modules: module_store,
            generated: BoundaryGeneratedFunctionStore::default(),
            diagnosed: Vec::new(),
            blocked: Vec::new(),
        };
        Self::from_successful_boundaries(project, CompletedSourcePackageRegistry::new())
    }

    fn from_successful_boundaries(
        project: CompiledGraphBoundary,
        source_packages: CompletedSourcePackageRegistry,
    ) -> Result<Self, CompilerError> {
        project.validate_invariants()?;
        ensure_success_only(&project)?;
        project.modules.ensure_all_successful()?;
        for package in source_packages.iter() {
            package.boundary.validate_invariants()?;
            ensure_success_only(&package.boundary)?;
            package.boundary.modules.ensure_all_successful()?;
        }

        let module_views = compilation_module_views(&project, &source_packages)?;
        let module_at = |module_ref: CompiledModuleRef| -> &Module {
            boundary_module_at(&project, &source_packages, module_ref)
                .expect("module refs were validated during construction")
        };
        let mut function_owner_by_origin = FxHashMap::default();
        let mut function_owner_by_private_identity = FxHashMap::default();
        let mut function_owner_by_generated = FxHashMap::default();
        for (module_ref, module) in module_views {
            for (origin, function_id) in &module.executable.hir.function_ids_by_origin {
                if function_owner_by_origin
                    .insert(origin.clone(), (module_ref, *function_id))
                    .is_some()
                {
                    return Err(CompilerError::compiler_error(format!(
                        "Project compilation contains duplicate source function origin {origin:?}"
                    )));
                }
            }
            for (origin, function_id) in &module.executable.hir.function_ids_by_private_origin {
                if function_owner_by_private_identity
                    .insert(origin.clone(), (module_ref, *function_id))
                    .is_some()
                {
                    return Err(CompilerError::compiler_error(format!(
                        "Project compilation contains duplicate private function origin {origin:?}"
                    )));
                }
            }
            for (identity, function_id) in &module.executable.hir.function_ids_by_generated {
                if function_owner_by_generated
                    .insert(identity.clone(), (module_ref, *function_id))
                    .is_some()
                {
                    return Err(CompilerError::compiler_error(format!(
                        "Project compilation contains duplicate generated function identity {identity:?}"
                    )));
                }
            }
        }

        let mut sorted_origins = function_owner_by_origin.keys().cloned().collect::<Vec<_>>();
        sorted_origins.sort();
        let source_function_names = Arc::new(
            sorted_origins
                .into_iter()
                .enumerate()
                .map(|(index, origin)| (origin, format!("__moth_src_fn_{index}")))
                .collect(),
        );
        let mut sorted_private_functions = function_owner_by_private_identity
            .iter()
            .map(|(identity, owner)| (identity.clone(), *owner))
            .collect::<Vec<_>>();
        sorted_private_functions
            .sort_by_key(|(_, (module_ref, function_id))| (*module_ref, function_id.0));
        let module_private_function_names = Arc::new(
            sorted_private_functions
                .into_iter()
                .enumerate()
                .map(|(index, (identity, _))| (identity, format!("__moth_private_fn_{index}")))
                .collect(),
        );
        let mut sorted_generated_functions = function_owner_by_generated
            .iter()
            .map(|(identity, owner)| (identity.clone(), *owner))
            .collect::<Vec<_>>();
        sorted_generated_functions
            .sort_by_key(|(_, (module_ref, function_id))| (*module_ref, function_id.0));
        let generated_function_names = Arc::new(
            sorted_generated_functions
                .into_iter()
                .enumerate()
                .map(|(index, (identity, _))| (identity, format!("__moth_generated_fn_{index}")))
                .collect(),
        );
        let mut entries = Vec::new();

        // Entry selection resolves the project graph's normal-entry identities through the
        // retained dense mapping. Source-package graphs are never queried as entry sources, and
        // their root activity stays immutable.
        for module_id in project.structure.entry_modules() {
            let module = &project
                .modules
                .artifact(*module_id)?
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "Project graph normal entry ModuleId {} has no successful artefact",
                        module_id.index()
                    ))
                })?
                .module;
            if !module.metadata.root_activity.has_html_artifact_activity() {
                continue;
            }

            let start_function = module
                .executable
                .hir
                .require_start_function("entry assembly")?;
            let root_module_ref = CompiledModuleRef::Project(*module_id);
            let mut roots_by_module =
                FxHashMap::<CompiledModuleRef, FxHashSet<FunctionId>>::default();
            roots_by_module
                .entry(root_module_ref)
                .or_default()
                .insert(start_function);
            let mut pending_modules = VecDeque::from([root_module_ref]);
            let mut reachability_by_module = FxHashMap::default();

            while let Some(reachable_module_ref) = pending_modules.pop_front() {
                let mut roots = roots_by_module[&reachable_module_ref]
                    .iter()
                    .copied()
                    .collect::<Vec<_>>();
                roots.sort_by_key(|function_id| function_id.0);
                let reachable_module = module_at(reachable_module_ref);
                let reachability = collect_reachability_from_function_link_facts(
                    &reachable_module.link_facts.functions,
                    &roots,
                )?;

                for origin in &reachability.reachable_cross_module_functions {
                    let Some((provider_module_ref, provider_function_id)) =
                        function_owner_by_origin.get(origin).copied()
                    else {
                        return Err(CompilerError::compiler_error(format!(
                            "Entry assembly could not resolve cross-module function origin {origin:?}"
                        )));
                    };
                    if roots_by_module
                        .entry(provider_module_ref)
                        .or_default()
                        .insert(provider_function_id)
                    {
                        pending_modules.push_back(provider_module_ref);
                    }
                }

                for identity in &reachability.reachable_module_private_functions {
                    let Some((provider_module_ref, provider_function_id)) =
                        function_owner_by_private_identity.get(identity).copied()
                    else {
                        return Err(CompilerError::compiler_error(format!(
                            "Entry assembly could not resolve module-private function identity {identity:?}"
                        )));
                    };
                    if roots_by_module
                        .entry(provider_module_ref)
                        .or_default()
                        .insert(provider_function_id)
                    {
                        pending_modules.push_back(provider_module_ref);
                    }
                }

                for identity in &reachability.reachable_generated_functions {
                    let Some((generated_module_ref, generated_function_id)) =
                        function_owner_by_generated.get(identity).copied()
                    else {
                        return Err(CompilerError::compiler_error(format!(
                            "Entry assembly could not resolve generated function identity {identity:?}"
                        )));
                    };
                    if roots_by_module
                        .entry(generated_module_ref)
                        .or_default()
                        .insert(generated_function_id)
                    {
                        pending_modules.push_back(generated_module_ref);
                    }
                }

                reachability_by_module.insert(reachable_module_ref, reachability);
            }

            let reachability =
                reachability_by_module
                    .remove(&root_module_ref)
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Entry assembly lost its root module reachability",
                        )
                    })?;
            let mut linked_modules = reachability_by_module
                .into_iter()
                .map(|(module_ref, reachability)| LinkedModuleAssembly {
                    module_ref,
                    reachability,
                })
                .collect::<Vec<_>>();
            linked_modules.sort_by_key(|linked| linked.module_ref);

            let mut external_imports = Vec::new();
            for (reachable_module, module_reachability) in std::iter::once((module, &reachability))
                .chain(
                    linked_modules
                        .iter()
                        .map(|linked| (module_at(linked.module_ref), &linked.reachability)),
                )
            {
                let reachable_package_ids = collect_reachable_external_package_ids(
                    module_reachability,
                    reachable_module
                        .link_facts
                        .external_package_registry
                        .as_ref(),
                )?;
                external_imports.extend(
                    reachable_module
                        .link_facts
                        .external_import_candidates
                        .iter()
                        .filter(|external_import| {
                            reachable_package_ids.contains(&external_import.package_id)
                        })
                        .cloned(),
                );
            }

            entries.push(EntryAssembly {
                module_ref: root_module_ref,
                reachability,
                external_imports,
                linked_modules,
            });
        }

        Ok(Self {
            project,
            source_packages,
            entries,
            source_function_names,
            module_private_function_names,
            generated_function_names,
        })
    }

    /// Iterate every successful module view in deterministic order.
    ///
    /// Project modules (base artefacts then generated sidecars) come first, then each source
    /// package boundary in package order. Builders read only the executable/link/metadata lanes;
    /// the retained boundaries own the immutable interfaces and dense identity mapping.
    pub(crate) fn modules(&self) -> impl Iterator<Item = &Module> + '_ {
        self.project.successful_module_views().chain(
            self.source_packages
                .iter()
                .flat_map(|package| package.boundary.successful_module_views()),
        )
    }

    /// Number of successful module views (base artefacts plus generated sidecars).
    pub(crate) fn module_count(&self) -> usize {
        self.modules().count()
    }

    fn module_at(&self, module_ref: CompiledModuleRef) -> &Module {
        boundary_module_at(&self.project, &self.source_packages, module_ref)
            .expect("module refs were validated during construction")
    }

    /// Resolve every entry through this compilation's own module store.
    ///
    /// Returning an owner-bound view prevents an assembly from one compilation being resolved
    /// against another compilation's same-index module.
    pub(crate) fn entries(&self) -> Vec<ProjectEntry<'_>> {
        let mut entries = Vec::with_capacity(self.entries.len());

        for entry in &self.entries {
            let module = self.module_at(entry.module_ref);
            entries.push(ProjectEntry {
                module,
                reachability: &entry.reachability,
                external_imports: &entry.external_imports,
                linked_modules: entry
                    .linked_modules
                    .iter()
                    .map(|linked| ProjectLinkedModule {
                        module: self.module_at(linked.module_ref),
                        reachability: &linked.reachability,
                    })
                    .collect(),
                source_function_names: Arc::clone(&self.source_function_names),
                module_private_function_names: Arc::clone(&self.module_private_function_names),
                generated_function_names: Arc::clone(&self.generated_function_names),
            });
        }

        entries
    }
}

fn collect_reachable_external_package_ids(
    reachability: &HirReachability,
    registry: &ExternalPackageRegistry,
) -> Result<FxHashSet<ExternalPackageId>, CompilerError> {
    let mut package_ids = FxHashSet::default();

    for function_id in &reachability.reachable_external_functions {
        let package_id = registry
            .resolve_function_package_id(*function_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Reachable external function {function_id:?} has no owning package"
                ))
            })?;
        package_ids.insert(package_id);
    }

    Ok(package_ids)
}

/// Reject a boundary that contains diagnosed or blocked modules.
///
/// `ProjectCompilation` is success-only by construction: `build` and `dev` must never assemble a
/// linkable payload from a partial graph.
fn ensure_success_only(boundary: &CompiledGraphBoundary) -> Result<(), CompilerError> {
    if let Some(diagnosed) = boundary.diagnosed.first() {
        return Err(CompilerError::compiler_error(format!(
            "Project compilation received a boundary with diagnosed ModuleId {}",
            diagnosed.module_id.index()
        )));
    }
    if let Some(blocked) = boundary.blocked.first() {
        return Err(CompilerError::compiler_error(format!(
            "Project compilation received a boundary with blocked ModuleId {}",
            blocked.module_id.index()
        )));
    }
    Ok(())
}

/// Resolve one dense module reference through the owning boundary's retained store.
///
/// Construction validates every reference, so a missing artefact or sidecar here is a proven
/// internal invariant rather than a user-facing failure.
fn boundary_module_at<'a>(
    project: &'a CompiledGraphBoundary,
    source_packages: &'a CompletedSourcePackageRegistry,
    module_ref: CompiledModuleRef,
) -> Result<&'a Module, CompilerError> {
    match module_ref {
        CompiledModuleRef::Project(module_id) => Ok(&project
            .modules
            .artifact(module_id)?
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Project module ref {} references a non-successful slot",
                    module_id.index()
                ))
            })?
            .module),
        CompiledModuleRef::SourcePackage {
            package_id,
            module_id,
        } => Ok(&source_packages
            .package(package_id)?
            .boundary
            .modules
            .artifact(module_id)?
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Source-package module ref {} references a non-successful slot",
                    module_id.index()
                ))
            })?
            .module),
        CompiledModuleRef::GeneratedProject(sidecar_index) => {
            Ok(&project.generated.sidecar_at(sidecar_index)?.module)
        }
        CompiledModuleRef::GeneratedSourcePackage {
            package_id,
            sidecar_index,
        } => Ok(&source_packages
            .package(package_id)?
            .boundary
            .generated
            .sidecar_at(sidecar_index)?
            .module),
    }
}

/// Build-owned activation record for one compiled module's dormant root work.
///
/// The dense module ref is private to the owning `ProjectCompilation`; backends receive only the
/// owner-bound entry view returned by `ProjectCompilation::entries`.
pub(crate) struct EntryAssembly {
    module_ref: CompiledModuleRef,
    reachability: HirReachability,
    external_imports: Vec<ModuleExternalImport>,
    linked_modules: Vec<LinkedModuleAssembly>,
}

pub(crate) struct LinkedModuleAssembly {
    module_ref: CompiledModuleRef,
    reachability: HirReachability,
}

#[derive(Clone, Copy)]
pub(crate) struct ProjectLinkedModule<'a> {
    pub(crate) module: &'a Module,
    pub(crate) reachability: &'a HirReachability,
}

/// Owner-bound view of one entry assembly and its selected compiled module.
#[derive(Clone)]
pub(crate) struct ProjectEntry<'a> {
    pub(crate) module: &'a Module,
    pub(crate) reachability: &'a HirReachability,
    pub(crate) external_imports: &'a [ModuleExternalImport],
    pub(crate) linked_modules: Vec<ProjectLinkedModule<'a>>,
    pub(crate) source_function_names: Arc<std::collections::HashMap<OriginFunctionId, String>>,
    pub(crate) module_private_function_names:
        Arc<std::collections::HashMap<ModulePrivateExecutableIdentity, String>>,
    pub(crate) generated_function_names: Arc<
        std::collections::HashMap<
            crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity,
            String,
        >,
    >,
}

impl Module {
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        increment_frontend_counter(FrontendCounter::ModuleRemapStringIdsCalls);

        self.executable.remap_string_ids(remap);
        self.link_facts.functions.remap_string_ids(remap);
        self.metadata.remap_string_ids(remap);
    }
}

/// Internal semantic draft for one module after local semantic compilation and before provider
/// publication.
///
/// WHAT: carries the semantic lanes produced by one module frontend compilation before the
/// canonical graph publishes a completed provider artefact. It owns the
/// validated base HIR, paired local type environment and borrow facts (inside `module`), the
/// complete base-function link facts (`module.link_facts`), compiler metadata
/// (`module.metadata`), the completed direct public interface and the module-local string
/// table that carries every diagnostic render identity for remap.
/// WHY: naming this internal phase separately from the backend `Module` handoff gives provider
/// completion and the generated-function worklist one build-owned result to evolve. This draft
/// is not a completed provider artefact or backend module result.
/// It implements no provider lookup and must not enter any successful
/// `GraphCompilationOutcome`. The stable module origin is retained through
/// `public_interface.draft.module_origin`; no dense `ModuleId` crosses this boundary
/// because standalone compilation has no graph-assigned identity. The current legacy generic
/// path still materialises requests inside AST before HIR lowering. R5F replaces that owner and
/// adds stable unresolved requests to this draft instead of introducing a placeholder field here.
pub(crate) struct ModuleSemanticDraft {
    /// Current executable, link-fact and compiler-metadata lanes: validated base HIR, paired type
    /// environment, borrow facts, complete base-function link facts and compiler metadata.
    pub module: Module,
    /// New generated identities, summaries and sidecars produced transactionally for this
    /// module. The boundary scheduler remaps and publishes this delta only after module success.
    pub generated_worklist_delta: crate::build_system::create_project_modules::generated_worklist::GeneratedFunctionWorklistDelta,
    /// The module-local string table carrying every diagnostic render identity produced during
    /// semantic compilation. Merged into the build table once per module at the compilation
    /// boundary so downstream consumers see a single remapped table.
    pub string_table: StringTable,
    /// The closed and publication-validated semantic interface. Provider-owned re-export facts
    /// have already joined through immutable completed interfaces, so the graph can publish this
    /// value directly after deterministic string-table merge.
    pub public_interface: PublicSemanticInterface,
}

// -------------------------
//  Backend Abstractions
// -------------------------

/// Unified build interface for all project types
pub trait BackendBuilder {
    /// Identify the builder that owns manifests and generated output artifacts.
    #[cfg(not(test))]
    fn builder_kind(&self) -> BuilderKind;

    /// Unit-test builders use one synthetic owner so their fixtures cannot claim a production
    /// HTML manifest.
    #[cfg(test)]
    fn builder_kind(&self) -> BuilderKind {
        BuilderKind::Test
    }

    /// Build the project with the given configuration
    fn build_backend(
        &self,
        project_compilation: ProjectCompilation,
        config: &Config, // Persistent settings across the whole project
        build_profile: BuildProfile,
        flags: &[Flag], // Settings only relevant to this build
        string_table: &mut StringTable,
    ) -> Result<Project, CompilerMessages>;

    /// Validate the project configuration
    fn validate_project_config(
        &self,
        config: &Config,
        string_table: &mut StringTable,
    ) -> Result<(), ProjectConfigError>;

    /// Project-specific frontend style directives provided by this backend.
    ///
    /// Frontend-owned directives are always present in registry construction and cannot be
    /// overridden by project builders. This hook supplies only project-owned additions for
    /// tokenization/template parsing.
    fn frontend_style_directives(&self) -> Vec<StyleDirectiveSpec>;

    /// Builder-provided surface.
    ///
    /// WHAT: returns the complete builder surface this builder exposes, including
    /// external platform packages (e.g. `@core/math`) and source-backed package roots
    /// (e.g. `@html`).
    /// WHY: backends own the runtime and package surface, so they must declare
    /// everything the compiler frontend is allowed to see and resolve.
    fn frontend_surface(&self) -> BuilderSurface;
}

/// Build-system entrypoint that owns the selected backend implementation.
///
/// WHAT: stores the backend strategy object used by `build_project`.
/// WHY: callers can swap backends while keeping one orchestration surface.
pub struct ProjectBuilder {
    pub backend: Box<dyn BackendBuilder + Send>,
}

impl ProjectBuilder {
    pub fn new(backend: Box<dyn BackendBuilder + Send>) -> Self {
        Self { backend }
    }
}

pub(crate) struct BuildBootstrap {
    pub(crate) config: Config,
    pub(crate) style_directives: StyleDirectiveRegistry,
    pub(crate) string_table: StringTable,
    pub(crate) frontend_surface: BuilderSurface,
    pub(crate) validated_directory_output_settings: Option<ValidatedDirectoryOutputSettings>,
}

// -------------------------
//  Output Payload
// -------------------------

pub struct OutputFile {
    relative_output_path: PathBuf,
    file_kind: FileKind,
}

pub enum FileKind {
    // This signals for the build system to not create this file.
    // Good for error checking / LSPs etc.
    NotBuilt,

    Wasm(Vec<u8>),
    Bytes(Vec<u8>),
    Js(String), // Either just glue code for web or pure JS backend
    Html(String),
    Directory, // So the build system can create empty folders if needed
}

impl OutputFile {
    /// Create an output artifact with an explicit relative path under the chosen output root.
    pub fn new(relative_output_path: PathBuf, file_kind: FileKind) -> Self {
        Self {
            relative_output_path,
            file_kind,
        }
    }

    /// Relative output path including any desired extension.
    pub fn relative_output_path(&self) -> &Path {
        &self.relative_output_path
    }

    pub(crate) fn file_kind(&self) -> &FileKind {
        &self.file_kind
    }
}

pub struct Project {
    pub output_files: Vec<OutputFile>,
    pub entry_page_rel: Option<PathBuf>,
    /// Builder-owned cleanup contract for manifest tracking and stale artifact removal.
    pub cleanup_policy: CleanupPolicy,
    pub warnings: Vec<CompilerDiagnostic>,
}

/// Result of a successful core build orchestration run.
pub struct BuildResult {
    pub project: Project,
    pub config: Config,
    pub warnings: Vec<CompilerDiagnostic>,
    pub string_table: StringTable,
    pub output_owner: OutputOwner,
    pub directory_output_plan: Option<ValidatedOutputPlan>,
}

// -------------------------
//  Build Orchestration
// -------------------------

/// Build a Moth project by running path validation, frontend compilation, and backend build.
///
/// This function intentionally does not write output files so callers can decide where artifacts
/// should be emitted.
pub fn build_project(
    project_builder: &ProjectBuilder,
    entry_path: &str,
    flags: &[Flag],
) -> Result<BuildResult, CompilerMessages> {
    let total_start = crate::timing::start_pipeline_timing();
    let build_profile = BuildProfile::from_flags(flags);
    let mut path_string_table = StringTable::new();
    let path_validation_start = crate::timing::start_pipeline_timing();
    let valid_path = match check_if_valid_path(entry_path, &mut path_string_table) {
        Ok(path) => {
            log_stage_timing("build_project.path_validation", path_validation_start);
            path
        }
        Err(error) => {
            log_stage_timing("build_project.path_validation", path_validation_start);
            log_stage_timing("build_project.total", total_start);
            return Err(CompilerMessages::from_error(error, path_string_table));
        }
    };

    // --------------------------------------------
    //   PERFORM THE CORE COMPILER FRONTEND BUILD
    // --------------------------------------------
    // This discovers all the modules, parses the config,
    // and compiles each module to HIR for backend lowering.
    let bootstrap_start = crate::timing::start_pipeline_timing();
    let BuildBootstrap {
        mut config,
        style_directives,
        mut string_table,
        mut frontend_surface,
        validated_directory_output_settings,
    } = match bootstrap_project_build(project_builder, valid_path) {
        Ok(bootstrap) => {
            log_stage_timing("build_project.bootstrap", bootstrap_start);
            bootstrap
        }
        Err(messages) => {
            log_stage_timing("build_project.bootstrap", bootstrap_start);
            log_stage_timing("build_project.total", total_start);
            return Err(messages);
        }
    };

    let compile_frontend_start = crate::timing::start_pipeline_timing();
    let frontend_compilation = match compile_project_frontend(
        &mut config,
        build_profile,
        validated_directory_output_settings.as_ref(),
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    ) {
        Ok(frontend_compilation) => {
            log_stage_timing(
                "build_project.compile_project_frontend",
                compile_frontend_start,
            );
            frontend_compilation
        }
        Err(messages) => {
            log_stage_timing(
                "build_project.compile_project_frontend",
                compile_frontend_start,
            );
            log_stage_timing("build_project.total", total_start);
            return Err(messages);
        }
    };
    if frontend_compilation.has_diagnosed_or_blocked() {
        log_stage_timing("build_project.total", total_start);
        return Err(frontend_compilation.into_render_messages(&mut string_table));
    }
    let project_compilation = ProjectCompilation::from_frontend(frontend_compilation)
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;
    let mut warnings: Vec<CompilerDiagnostic> = project_compilation
        .modules()
        .flat_map(collect_module_warnings)
        .collect();

    // --------------------------------------------
    // BUILD PROJECT USING THE APPROPRIATE BUILDER
    // --------------------------------------------

    let backend_start = crate::timing::start_pipeline_timing();
    let project = match project_builder.backend.build_backend(
        project_compilation,
        &config,
        build_profile,
        flags,
        &mut string_table,
    ) {
        Ok(project) => {
            log_stage_timing("build_project.backend", backend_start);
            project
        }
        Err(mut compiler_messages) => {
            log_stage_timing("build_project.backend", backend_start);
            log_stage_timing("build_project.total", total_start);
            compiler_messages.string_table = string_table;
            return Err(compiler_messages);
        }
    };

    warnings.extend(project.warnings.iter().cloned());

    let output_owner = OutputOwner {
        builder: project_builder.backend.builder_kind(),
        profile: build_profile,
    };
    let directory_output_plan = if config.entry_dir.is_dir() {
        let Some(validated_output_settings) = validated_directory_output_settings else {
            let error = CompilerError::compiler_error(
                "Directory output settings were not available after bootstrap validation.",
            );
            log_stage_timing("build_project.total", total_start);
            return Err(CompilerMessages::from_error(error, string_table));
        };

        Some(validated_output_settings.select(
            config.entry_dir.clone(),
            resolve_project_entry_root(&config),
            output_owner,
        ))
    } else {
        None
    };

    log_stage_timing("build_project.total", total_start);

    Ok(BuildResult {
        project,
        config,
        warnings,
        string_table,
        output_owner,
        directory_output_plan,
    })
}

/// Build the shared Stage 0/bootstrap state used by both CLI builds and the dev server.
///
/// WHAT: merges frontend/project directives, loads `config.moth`, and runs backend-specific
/// config validation into one reusable setup step.
/// WHY: directory builds and the dev server must share one bootstrap path so config/output
/// behavior does not drift between "build" and "serve" flows.
pub(crate) fn bootstrap_project_build(
    project_builder: &ProjectBuilder,
    entry_path: PathBuf,
) -> Result<BuildBootstrap, CompilerMessages> {
    let bootstrap_total_start = crate::timing::start_pipeline_timing();

    let config_init_start = crate::timing::start_pipeline_timing();
    let mut config = Config::new(entry_path);
    log_stage_timing("bootstrap.config_init", config_init_start);

    // Seed the build table with the compiler-owned symbols that per-file frontend tables will
    // also need as a stable prefix once file preparation becomes independent.
    let symbol_preseed_start = crate::timing::start_pipeline_timing();
    let preseeded = CompilerSymbolSet::preseeded_table(FILE_MIN_UNIQUE_SYMBOLS_CAPACITY);
    let mut string_table = preseeded.string_table;
    // The bootstrap path only needs the preseeded table today. File-local preparation will keep
    // these typed IDs alongside its local outputs once fixed-symbol IDs are consumed directly.
    let _compiler_symbol_ids = preseeded.compiler_symbol_ids;
    log_stage_timing("bootstrap.symbol_preseed", symbol_preseed_start);

    // Compute the builder's frontend surface once so config loading and frontend compilation
    // see the same set of allowed config keys, external packages, and source-backed packages.
    let frontend_surface_start = crate::timing::start_pipeline_timing();
    let frontend_surface = project_builder.backend.frontend_surface();
    log_stage_timing("bootstrap.frontend_surface", frontend_surface_start);

    let style_directives_start = crate::timing::start_pipeline_timing();
    let frontend_style_directives = project_builder.backend.frontend_style_directives();
    let style_directives = match StyleDirectiveRegistry::merged(&frontend_style_directives) {
        Ok(style_directives) => style_directives,
        Err(error) => {
            log_stage_timing("bootstrap.style_directives", style_directives_start);
            log_stage_timing("bootstrap.total", bootstrap_total_start);
            return Err(CompilerMessages::from_error(error, string_table.clone()));
        }
    };
    log_stage_timing("bootstrap.style_directives", style_directives_start);

    // WHAT: Load and validate project config before compilation begins (Stage 0).
    // WHY: Backends and serving code both depend on the same validated config surface.
    let config_services = ProjectConfigParseServices {
        style_directives: &style_directives,
        frontend_surface: &frontend_surface,
    };
    let load_project_config_start = crate::timing::start_pipeline_timing();
    let validated_directory_output_settings =
        match load_project_config(&mut config, &config_services, &mut string_table) {
            Ok(settings) => settings,
            Err(messages) => {
                log_stage_timing("bootstrap.load_project_config", load_project_config_start);
                log_stage_timing("bootstrap.total", bootstrap_total_start);
                return Err(messages);
            }
        };
    log_stage_timing("bootstrap.load_project_config", load_project_config_start);

    // WHAT: Validate backend-specific config requirements before compilation.
    // WHY: Backends should reject unsupported settings before frontend compilation does work.
    let backend_config_validate_start = crate::timing::start_pipeline_timing();
    if let Err(error) = project_builder
        .backend
        .validate_project_config(&config, &mut string_table)
    {
        log_stage_timing(
            "bootstrap.backend_config_validate",
            backend_config_validate_start,
        );
        log_stage_timing("bootstrap.total", bootstrap_total_start);
        return Err(error.into_messages(string_table.clone()));
    }
    log_stage_timing(
        "bootstrap.backend_config_validate",
        backend_config_validate_start,
    );

    log_stage_timing("bootstrap.total", bootstrap_total_start);

    Ok(BuildBootstrap {
        config,
        style_directives,
        string_table,
        frontend_surface,
        validated_directory_output_settings,
    })
}

/// Record a build-system stage timing through the central `timers` substrate.
///
/// WHAT: delegates to `timing::record_started_pipeline_timing`, which stores the
///      observation in the active collection scope and emits the stable
///      `MOTH_BENCH timing` line when the output mode permits.
/// WHY: `build_project` uses dotted `build_project.*` metric names while the output subsystem
///      records its own `output.*` stages through the same concise `timers` substrate.
///      The start token is zero-sized when `timers` is off, so regular builds
///      do not read clocks for these instrumentation-only measurements.
fn log_stage_timing(metric: &str, start: crate::timing::PipelineTimingStart) {
    crate::timing::record_started_pipeline_timing(metric, start);
}

fn collect_module_warnings(module: &Module) -> Vec<CompilerDiagnostic> {
    module.metadata.warnings.clone()
}

/// Build an immutable `PublicSemanticInterface` for one test-constructed artefact.
///
/// Test helpers wrap a bare `Module` into a real `CompiledModuleArtifact` inside a real graph
/// boundary; production publication always supplies the completed interface. The origin path is
/// unique per module so entry assembly and interface lookup behave like real artefacts.
#[cfg(test)]
fn test_public_interface(module_index: usize) -> PublicSemanticInterface {
    PublicSemanticInterface {
        module_origin: crate::compiler_frontend::semantic_identity::StableModuleOriginIdentity::from_portable_path(
            crate::compiler_frontend::semantic_identity::StablePackageIdentity::project_local("test"),
            format!("module_{module_index}"),
            crate::compiler_frontend::semantic_identity::ModuleRootRole::Normal,
        ),
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    }
}

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
