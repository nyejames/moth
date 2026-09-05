//! Core build orchestration for Moth projects.
//!
//! WHAT: the canonical project build flow (`build_project`), the success-only
//!       `ProjectCompilation` aggregate, entry and linked-module assembly, the backend builder
//!       trait and the output records those builders return.
//! WHY: build tools compile once and pass the resulting project to the output subsystem without
//!       reimplementing frontend or backend orchestration.
//!
//! This module aggregates compiler results; it does not produce them. The module artefact lanes,
//! generated sidecars and semantic result values are owned by
//! `compiler_frontend::module_compilation`.
use crate::timing_scope;

use crate::build_system::BuildProfile;
use crate::build_system::create_project_modules::compiled_boundary::{
    CompiledGraphBoundary, CompiledModuleRef, CompletedSourcePackageRegistry, PackageBoundaryId,
    ProjectFrontendCompilation, compilation_module_views,
};
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::build_system::create_project_modules::{
    FrontendCompilationMode, compile_project_frontend_with_inputs, resolve_project_entry_root,
};
use crate::build_system::output::{
    BuilderKind, CleanupPolicy, OutputOwner, ValidatedDirectoryOutputSettings, ValidatedOutputPlan,
};
use crate::build_system::path_validation::check_if_valid_path;
use crate::build_system::project_config::{ProjectConfigParseServices, load_project_config};
use crate::build_system::resource_unions::{
    ResourceOriginUnion, append_entry_module_resources, append_exported_interface_resources,
    append_reachable_resource_uses,
};

use crate::compiler_frontend::Flag;
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::source_location::{CharPosition, SourceLocation};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, ProjectContextEscapeReason};
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::reachability::{
    HirReachability, collect_reachability_from_function_link_facts,
};
use crate::compiler_frontend::module_compilation::{Module, ModuleExternalImport};
use crate::compiler_frontend::paths::file_references::ResourceSourceId;
use crate::compiler_frontend::public_interface::{
    PublicDeclarationRecord, PublicDeclarationSemantics, PublicFunctionCategory,
    PublicSemanticInterface,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedFunctionIdentity, ModulePrivateExecutableIdentity, OriginDeclarationId,
    OriginFunctionId, StableModuleOriginIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceClass;

use crate::compiler_frontend::source::SourceDatabase;
use crate::compiler_frontend::style_directives::{StyleDirectiveRegistry, StyleDirectiveSpec};
use crate::compiler_frontend::symbols::compiler_symbols::CompilerSymbolSet;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use crate::builder_surface::BuilderSurface;
use crate::compiler_frontend::external_packages::{ExternalPackageId, ExternalPackageRegistry};
use crate::projects::settings::{Config, ProjectConfigError};

use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const FILE_MIN_UNIQUE_SYMBOLS_CAPACITY: usize = 32;

// -------------------------
//  Project Aggregation
// -------------------------

/// Success-only frontend payload consumed by project builders.
///
/// WHAT: owns the retained project and source-package graph boundaries, the explicit entry
///       assemblies selected from dormant root activity, and the generated sidecar lane inside
///       each boundary. It retains immutable compiler module artefacts and their dense
///       `ModuleId` mapping so the published public semantic interface and boundary identity
///       survive into builders and link owners.
/// WHY: project builders need a coherent project boundary with build-owned entry selection. A
///      diagnosed frontend never constructs this value, and backends no longer infer entries by
///      filtering a flat module vector. Entry selection resolves the project graph's normal entry
///      roots.
///
/// Frontend configuration values and the synthetic `@project` interface are boundary-local
/// compilation inputs, not retained state on this aggregate. Their semantic fingerprints and
/// project-context provenance remain available at the compiler/interface boundary for future
/// reuse; targeted module or configuration invalidation is still deferred.
pub struct ProjectCompilation {
    /// Retained project graph boundary with its artefact store and generated lane.
    project: CompiledGraphBoundary,
    /// Retained source-package boundaries, each with its own dense identity space.
    source_packages: CompletedSourcePackageRegistry,
    entries: Vec<EntryAssembly>,
    /// Optional project package-facade assembly selected from the public package surface.
    #[allow(dead_code)]
    // retained for future package-target backends; assembly still owns liveness
    package_assembly: Option<PackageAssembly>,
    source_function_names: Arc<std::collections::HashMap<OriginFunctionId, String>>,
    module_private_function_names:
        Arc<std::collections::HashMap<ModulePrivateExecutableIdentity, String>>,
    /// Project-boundary generated symbol map keyed by identity.
    generated_function_names: Arc<
        std::collections::HashMap<
            crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity,
            String,
        >,
    >,
    /// Per-package-boundary generated symbol maps keyed by identity.
    ///
    /// Equal generated identities may exist in unrelated boundaries, so each boundary owns its
    /// own name lookup while all names stay globally unique.
    package_generated_function_names: FxHashMap<
        PackageBoundaryId,
        Arc<
            std::collections::HashMap<
                crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity,
                String,
            >,
        >,
    >,
    /// Every generated symbol name in deterministic assignment order.
    ///
    /// Builders reserve the complete name set once so boundary-local lookup maps can stay
    /// identity-keyed without risking JS identifier collisions between boundaries.
    all_generated_function_names: Arc<Vec<String>>,
    /// Build-only physical resource inputs discovered by Stage 0.
    pub(crate) resource_inputs: ResourceInputRegistry,
}

/// Failure produced while proving a success-only project assembly.
///
/// Facade policy violations are user-authored source failures and retain their typed diagnostic
/// payload. Other assembly failures remain infrastructure errors because their retained boundary
/// facts are inconsistent or unavailable.
#[derive(Debug)]
pub(crate) enum ProjectAssemblyError {
    Diagnostic {
        diagnostic: Box<CompilerDiagnostic>,
        string_table: StringTable,
    },
    Infrastructure(CompilerError),
}

impl From<CompilerError> for ProjectAssemblyError {
    fn from(error: CompilerError) -> Self {
        Self::Infrastructure(error)
    }
}

impl ProjectAssemblyError {
    fn project_context_escape(
        reason: ProjectContextEscapeReason,
        location: SourceLocation,
        string_table: StringTable,
    ) -> Self {
        Self::Diagnostic {
            diagnostic: Box::new(CompilerDiagnostic::project_context_escape(reason, location)),
            string_table,
        }
    }

    pub(crate) fn into_messages(self, string_table: &mut StringTable) -> CompilerMessages {
        match self {
            Self::Diagnostic {
                mut diagnostic,
                string_table: diagnostic_table,
            } => {
                let remap = string_table.merge_from(&diagnostic_table);
                diagnostic.remap_string_ids(&remap);
                CompilerMessages::from_diagnostics(vec![*diagnostic], string_table.clone())
            }
            Self::Infrastructure(error) => {
                CompilerMessages::from_error(error, string_table.clone())
            }
        }
    }
}
/// WHAT: every lookup for an empty package boundary returns the same immutable map instead of
///       allocating a fresh `Arc<HashMap>` per call.
static EMPTY_GENERATED_NAMES: std::sync::LazyLock<
    Arc<std::collections::HashMap<GeneratedFunctionIdentity, String>>,
> = std::sync::LazyLock::new(|| Arc::new(std::collections::HashMap::new()));

impl ProjectCompilation {
    pub(crate) fn from_frontend(
        frontend: ProjectFrontendCompilation,
    ) -> Result<Self, ProjectAssemblyError> {
        let ProjectFrontendCompilation {
            project,
            source_packages,
            resource_inputs,
            transient_messages: _,
        } = frontend;
        Self::from_successful_boundaries(project, source_packages, resource_inputs)
    }

    pub(crate) fn from_successful_boundaries(
        project: CompiledGraphBoundary,
        source_packages: CompletedSourcePackageRegistry,
        resource_inputs: ResourceInputRegistry,
    ) -> Result<Self, ProjectAssemblyError> {
        project.require_all_successful()?;
        resource_inputs.validate()?;
        for package in source_packages.iter() {
            package.boundary.require_all_successful()?;
        }
        let module_at = |module_ref: CompiledModuleRef| -> &Module {
            boundary_module_at(&project, &source_packages, module_ref)
                .expect("module refs were validated during construction")
        };

        let owner_maps = build_execution_owner_maps(&project, &source_packages)?;
        let function_owner_by_origin = &owner_maps.function_owner_by_origin;
        let function_owner_by_private_identity = &owner_maps.function_owner_by_private_identity;
        let project_generated_owners = &owner_maps.project_generated_owners;
        let package_generated_owners = &owner_maps.package_generated_owners;
        let module_owner_by_origin = build_module_owner_by_origin(&project, &source_packages)?;
        validate_source_package_facades(
            &project,
            &source_packages,
            &module_owner_by_origin,
            &owner_maps,
        )?;

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

        // Generated symbol names stay globally unique (one JS bundle may mix boundaries) while
        // lookup maps stay keyed by identity within one boundary. Names are assigned in stable
        // generated identity order inside each boundary, with the project boundary first and
        // source packages in stable package-prefix order, so sidecar publication reordering can
        // never change a generated symbol.
        let mut generated_function_names =
            std::collections::HashMap::<GeneratedFunctionIdentity, String>::default();
        let mut package_generated_function_names = FxHashMap::<
            PackageBoundaryId,
            Arc<std::collections::HashMap<GeneratedFunctionIdentity, String>>,
        >::default();
        let mut all_generated_function_names = Vec::new();
        let mut assign_generated_names =
            |owners: &FxHashMap<GeneratedFunctionIdentity, (CompiledModuleRef, FunctionId)>,
             names: &mut std::collections::HashMap<GeneratedFunctionIdentity, String>,
             next_index: &mut usize| {
                let mut sorted = owners
                    .iter()
                    .map(|(identity, owner)| (identity.clone(), *owner))
                    .collect::<Vec<_>>();
                sorted.sort_by(|left, right| left.0.cmp(&right.0));
                for (identity, _) in sorted {
                    let name = format!("__moth_generated_fn_{next_index}");
                    *next_index += 1;
                    names.insert(identity, name.clone());
                    all_generated_function_names.push(name);
                }
            };
        let mut next_generated_index = 0usize;
        assign_generated_names(
            project_generated_owners,
            &mut generated_function_names,
            &mut next_generated_index,
        );
        // Package name assignment sorts on the stable package prefix, never on registration
        // order, so reversing package publication order cannot change any boundary's symbols.
        let mut sorted_packages = package_generated_owners
            .keys()
            .copied()
            .map(|package_id| {
                let prefix = source_packages
                    .package(package_id)?
                    .package_prefix()
                    .to_owned();
                Ok((prefix, package_id))
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;
        sorted_packages.sort();
        for (_, package_id) in sorted_packages {
            let mut names =
                std::collections::HashMap::<GeneratedFunctionIdentity, String>::default();
            assign_generated_names(
                &package_generated_owners[&package_id],
                &mut names,
                &mut next_generated_index,
            );
            package_generated_function_names.insert(package_id, Arc::new(names));
        }
        let generated_function_names = Arc::new(generated_function_names);
        let all_generated_function_names = Arc::new(all_generated_function_names);

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
                        ))
                        .into());
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
                        ))
                        .into());
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
                    let Some((generated_module_ref, generated_function_id)) = generated_owner_for(
                        project_generated_owners,
                        package_generated_owners,
                        reachable_module_ref,
                        identity,
                    )
                    .copied() else {
                        return Err(CompilerError::compiler_error(format!(
                            "Entry assembly could not resolve generated function identity {identity:?} in its calling boundary"
                        ))
                        .into());
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
            let mut resource_union = ResourceOriginUnion::new();
            append_entry_module_resources(&mut resource_union, module, &reachability)?;
            for linked in &linked_modules {
                append_reachable_resource_uses(
                    &mut resource_union,
                    module_at(linked.module_ref),
                    &linked.reachability,
                )?;
            }

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
                resource_union,
                external_imports,
                linked_modules,
            });
        }

        let package_assembly = assemble_project_package(
            &project,
            &source_packages,
            &module_owner_by_origin,
            &owner_maps,
        )?;

        Ok(Self {
            project,
            source_packages,
            entries,
            package_assembly,
            source_function_names,
            module_private_function_names,
            generated_function_names,
            package_generated_function_names,
            all_generated_function_names,
            resource_inputs,
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
                resource_union: &entry.resource_union,
                module,
                reachability: &entry.reachability,
                external_imports: &entry.external_imports,
                linked_modules: entry
                    .linked_modules
                    .iter()
                    .map(|linked| ProjectLinkedModule {
                        module: self.module_at(linked.module_ref),
                        reachability: &linked.reachability,

                        generated_function_names: self
                            .generated_function_names_for(linked.module_ref),
                    })
                    .collect(),
                source_function_names: Arc::clone(&self.source_function_names),
                module_private_function_names: Arc::clone(&self.module_private_function_names),
                generated_function_names: Arc::clone(&self.generated_function_names),
                all_generated_function_names: Arc::clone(&self.all_generated_function_names),
            });
        }

        entries
    }

    /// Move the build-only resource registry into the output-emission owner.
    ///
    /// Entry views borrow the compilation's graph, so emission takes the registry only after
    /// callers finish consuming those views.
    pub(crate) fn take_resource_inputs(&mut self) -> ResourceInputRegistry {
        std::mem::take(&mut self.resource_inputs)
    }

    #[allow(dead_code)] // package-target consumers are not threaded into HTML rendering in this slice
    pub(crate) fn package_assembly(&self) -> Option<&PackageAssembly> {
        self.package_assembly.as_ref()
    }

    /// The generated symbol lookup map for one module's owning boundary.
    fn generated_function_names_for(
        &self,
        module_ref: CompiledModuleRef,
    ) -> Arc<std::collections::HashMap<GeneratedFunctionIdentity, String>> {
        match module_ref {
            CompiledModuleRef::Project(_) | CompiledModuleRef::GeneratedProject(_) => {
                Arc::clone(&self.generated_function_names)
            }
            CompiledModuleRef::SourcePackage { package_id, .. }
            | CompiledModuleRef::GeneratedSourcePackage { package_id, .. } => self
                .package_generated_function_names
                .get(&package_id)
                .cloned()
                .unwrap_or_else(|| Arc::clone(&EMPTY_GENERATED_NAMES)),
        }
    }
}
struct ExecutionOwnerMaps {
    function_owner_by_origin: FxHashMap<OriginFunctionId, (CompiledModuleRef, FunctionId)>,
    function_owner_by_private_identity:
        FxHashMap<ModulePrivateExecutableIdentity, (CompiledModuleRef, FunctionId)>,
    project_generated_owners: FxHashMap<GeneratedFunctionIdentity, (CompiledModuleRef, FunctionId)>,
    package_generated_owners: FxHashMap<
        PackageBoundaryId,
        FxHashMap<GeneratedFunctionIdentity, (CompiledModuleRef, FunctionId)>,
    >,
}

fn build_execution_owner_maps(
    project: &CompiledGraphBoundary,
    source_packages: &CompletedSourcePackageRegistry,
) -> Result<ExecutionOwnerMaps, CompilerError> {
    let module_views = compilation_module_views(project, source_packages)?;
    let mut function_owner_by_origin = FxHashMap::default();
    let mut function_owner_by_private_identity = FxHashMap::default();
    let mut project_generated_owners = FxHashMap::default();
    let mut package_generated_owners = FxHashMap::<
        PackageBoundaryId,
        FxHashMap<GeneratedFunctionIdentity, (CompiledModuleRef, FunctionId)>,
    >::default();

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
            let owner = (module_ref, *function_id);
            match module_ref {
                CompiledModuleRef::GeneratedProject(_) => {
                    if project_generated_owners
                        .insert(identity.clone(), owner)
                        .is_some()
                    {
                        return Err(CompilerError::compiler_error(format!(
                            "Project boundary contains duplicate generated function identity {identity:?}"
                        )));
                    }
                }
                CompiledModuleRef::GeneratedSourcePackage { package_id, .. } => {
                    if package_generated_owners
                        .entry(package_id)
                        .or_default()
                        .insert(identity.clone(), owner)
                        .is_some()
                    {
                        return Err(CompilerError::compiler_error(format!(
                            "Source package @{} contains duplicate generated function identity {identity:?}",
                            source_packages.package(package_id)?.package_prefix()
                        )));
                    }
                }
                CompiledModuleRef::Project(_) | CompiledModuleRef::SourcePackage { .. } => {
                    return Err(CompilerError::compiler_error(format!(
                        "Base module {module_ref:?} owns generated function identity {identity:?}"
                    )));
                }
            }
        }
    }

    Ok(ExecutionOwnerMaps {
        function_owner_by_origin,
        function_owner_by_private_identity,
        project_generated_owners,
        package_generated_owners,
    })
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

/// Resolve a published interface for a base module reference.
///
/// Generated sidecars intentionally have no public-interface lane; package selection starts from
/// base export bindings and only reaches sidecars through their paired module/link facts.
fn boundary_interface_at<'a>(
    project: &'a CompiledGraphBoundary,
    source_packages: &'a CompletedSourcePackageRegistry,
    module_ref: CompiledModuleRef,
) -> Result<&'a PublicSemanticInterface, CompilerError> {
    match module_ref {
        CompiledModuleRef::Project(module_id) => {
            project.modules.interface(module_id)?.ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Project module ref {} has no published interface",
                    module_id.index()
                ))
            })
        }
        CompiledModuleRef::SourcePackage {
            package_id,
            module_id,
        } => source_packages
            .package(package_id)?
            .boundary
            .modules
            .interface(module_id)?
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Source-package module ref {} has no published interface",
                    module_id.index()
                ))
            }),
        CompiledModuleRef::GeneratedProject(_)
        | CompiledModuleRef::GeneratedSourcePackage { .. } => Err(CompilerError::compiler_error(
            format!("Generated module ref {module_ref:?} has no published public interface"),
        )),
    }
}

/// Resolve one generated identity only inside the boundary that made the call.
///
/// WHAT: generated sidecars are owned by their consuming boundary, so a module can address only
///       the generated targets its own boundary materialised.
/// WHY: equal generated identities may exist in unrelated boundaries; identity alone cannot
///       select an owner during entry assembly.
fn generated_owner_for<'a>(
    project_owners: &'a FxHashMap<GeneratedFunctionIdentity, (CompiledModuleRef, FunctionId)>,
    package_owners: &'a FxHashMap<
        PackageBoundaryId,
        FxHashMap<GeneratedFunctionIdentity, (CompiledModuleRef, FunctionId)>,
    >,
    calling_module: CompiledModuleRef,
    identity: &GeneratedFunctionIdentity,
) -> Option<&'a (CompiledModuleRef, FunctionId)> {
    match calling_module {
        CompiledModuleRef::Project(_) | CompiledModuleRef::GeneratedProject(_) => {
            project_owners.get(identity)
        }
        CompiledModuleRef::SourcePackage { package_id, .. }
        | CompiledModuleRef::GeneratedSourcePackage { package_id, .. } => package_owners
            .get(&package_id)
            .and_then(|owners| owners.get(identity)),
    }
}

/// Build-owned activation record for one compiled module's dormant root work.
///
/// The dense module ref is private to the owning `ProjectCompilation`; backends receive only the
/// owner-bound entry view returned by `ProjectCompilation::entries`.
pub(crate) struct EntryAssembly {
    module_ref: CompiledModuleRef,
    reachability: HirReachability,
    resource_union: ResourceOriginUnion,
    external_imports: Vec<ModuleExternalImport>,
    linked_modules: Vec<LinkedModuleAssembly>,
}

pub(crate) struct LinkedModuleAssembly {
    module_ref: CompiledModuleRef,
    reachability: HirReachability,
}

/// Assemble the optional project package facade from its externally visible export surface.
///
/// The facade has no implicit start. Exported concrete functions and receiver methods seed
/// reachability in their owning base modules; exported constants/defaults select descendant
/// interfaces without making their dormant roots executable. Every generated module reached by
/// those link facts remains paired with the sidecar module that owns its resource table.
fn assemble_project_package(
    project: &CompiledGraphBoundary,
    source_packages: &CompletedSourcePackageRegistry,
    module_owner_by_origin: &FxHashMap<StableModuleOriginIdentity, CompiledModuleRef>,
    execution_owners: &ExecutionOwnerMaps,
) -> Result<Option<PackageAssembly>, ProjectAssemblyError> {
    let Some((facade_module_ref, selected_base_modules, reachability_by_module)) =
        plan_project_package_facade(
            project,
            source_packages,
            module_owner_by_origin,
            execution_owners,
        )?
    else {
        return Ok(None);
    };
    let facade_interface = boundary_interface_at(project, source_packages, facade_module_ref)?;

    let facade_module = boundary_module_at(project, source_packages, facade_module_ref)?;
    let facade_reachability = reachability_by_module
        .get(&facade_module_ref)
        .cloned()
        .unwrap_or_default();
    let mut linked_modules = reachability_by_module
        .into_iter()
        .filter(|(module_ref, _)| *module_ref != facade_module_ref)
        .map(|(module_ref, reachability)| LinkedModuleAssembly {
            module_ref,
            reachability,
        })
        .collect::<Vec<_>>();
    linked_modules.sort_by_key(|linked| linked.module_ref);

    let mut selected_modules = selected_base_modules
        .into_iter()
        .filter(|module_ref| *module_ref != facade_module_ref)
        .collect::<Vec<_>>();
    selected_modules.sort();

    let mut resource_union = ResourceOriginUnion::new();
    append_exported_interface_resources(&mut resource_union, facade_interface)?;
    append_reachable_resource_uses(&mut resource_union, facade_module, &facade_reachability)?;
    for linked in &linked_modules {
        append_reachable_resource_uses(
            &mut resource_union,
            boundary_module_at(project, source_packages, linked.module_ref)?,
            &linked.reachability,
        )?;
    }

    Ok(Some(PackageAssembly {
        facade_module_ref,
        facade_reachability,
        selected_modules,
        linked_modules,
        resource_union,
    }))
}

fn add_package_function_root(
    roots_by_module: &mut FxHashMap<CompiledModuleRef, FxHashSet<FunctionId>>,
    function_owner_by_origin: &FxHashMap<OriginFunctionId, (CompiledModuleRef, FunctionId)>,
    origin: &OriginFunctionId,
    label: &str,
) -> Result<(), CompilerError> {
    let Some((module_ref, function_id)) = function_owner_by_origin.get(origin).copied() else {
        return Err(CompilerError::compiler_error(format!(
            "{label} could not resolve exported function origin {origin:?}"
        )));
    };
    roots_by_module
        .entry(module_ref)
        .or_default()
        .insert(function_id);
    Ok(())
}
fn add_package_receiver_method_roots(
    roots_by_module: &mut FxHashMap<CompiledModuleRef, FxHashSet<FunctionId>>,
    function_owner_by_origin: &FxHashMap<OriginFunctionId, (CompiledModuleRef, FunctionId)>,
    declaration: Option<&crate::compiler_frontend::public_interface::PublicDeclarationRecord>,
    label: &str,
) -> Result<(), CompilerError> {
    let Some(declaration) = declaration else {
        return Ok(());
    };
    let methods = match &declaration.semantics {
        PublicDeclarationSemantics::Struct(structure) => &structure.receiver_methods,
        PublicDeclarationSemantics::Choice(choice) => &choice.receiver_methods,
        _ => return Ok(()),
    };
    for method in methods {
        if matches!(
            &method.category,
            crate::compiler_frontend::public_interface::PublicReceiverMethodCategory::GenericTemplate
        ) {
            continue;
        }
        add_package_function_root(
            roots_by_module,
            function_owner_by_origin,
            &method.method_origin,
            label,
        )?;
    }
    Ok(())
}
struct FacadeRootPlan {
    selected_base_modules: FxHashSet<CompiledModuleRef>,
    roots_by_module: FxHashMap<CompiledModuleRef, FxHashSet<FunctionId>>,
}
type FacadeReachabilityByModule = FxHashMap<CompiledModuleRef, HirReachability>;
type ProjectPackageFacadePlan = (
    CompiledModuleRef,
    FxHashSet<CompiledModuleRef>,
    FacadeReachabilityByModule,
);

fn plan_facade_roots(
    facade_module_ref: CompiledModuleRef,
    facade_interface: &PublicSemanticInterface,
    module_owner_by_origin: &FxHashMap<StableModuleOriginIdentity, CompiledModuleRef>,
    function_owner_by_origin: &FxHashMap<OriginFunctionId, (CompiledModuleRef, FunctionId)>,
    label: &str,
) -> Result<FacadeRootPlan, CompilerError> {
    let mut selected_base_modules = FxHashSet::default();
    selected_base_modules.insert(facade_module_ref);
    let mut roots_by_module = FxHashMap::<CompiledModuleRef, FxHashSet<FunctionId>>::default();

    for binding in &facade_interface.export_bindings {
        let module_ref = module_owner_by_origin
            .get(binding.origin().module_origin())
            .copied()
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "{label} export {:?} has no owning compiled module",
                    binding.origin()
                ))
            })?;
        selected_base_modules.insert(module_ref);

        let declaration = facade_interface.declaration(binding.origin());
        match binding.origin() {
            OriginDeclarationId::Function(origin) => {
                let generic = matches!(
                    declaration.map(|declaration| &declaration.semantics),
                    Some(PublicDeclarationSemantics::Function(function))
                        if matches!(
                            &function.category,
                            PublicFunctionCategory::GenericTemplate(_)
                        )
                );
                if !generic {
                    add_package_function_root(
                        &mut roots_by_module,
                        function_owner_by_origin,
                        origin,
                        label,
                    )?;
                }
            }
            OriginDeclarationId::Type(_) => {
                add_package_receiver_method_roots(
                    &mut roots_by_module,
                    function_owner_by_origin,
                    declaration,
                    label,
                )?;
            }
            OriginDeclarationId::Constant(_) | OriginDeclarationId::Trait(_) => {}
        }
    }

    // A public nominal declaration can carry receiver methods selected through a signature even
    // when its type name is not itself an exported binding. Those methods are executable facade
    // roots, while unselected descendant-interface exports remain outside the closed surface.
    for declaration in &facade_interface.declarations {
        if matches!(&declaration.origin, OriginDeclarationId::Type(_)) {
            add_package_receiver_method_roots(
                &mut roots_by_module,
                function_owner_by_origin,
                Some(declaration),
                label,
            )?;
        }
    }

    Ok(FacadeRootPlan {
        selected_base_modules,
        roots_by_module,
    })
}
fn plan_project_package_facade(
    project: &CompiledGraphBoundary,
    source_packages: &CompletedSourcePackageRegistry,
    module_owner_by_origin: &FxHashMap<StableModuleOriginIdentity, CompiledModuleRef>,
    execution_owners: &ExecutionOwnerMaps,
) -> Result<Option<ProjectPackageFacadePlan>, ProjectAssemblyError> {
    let Some(facade_module_id) = project.structure.facade() else {
        return Ok(None);
    };
    let facade_module_ref = CompiledModuleRef::Project(facade_module_id);
    let facade_interface = boundary_interface_at(project, source_packages, facade_module_ref)?;
    let facade_module = boundary_module_at(project, source_packages, facade_module_ref)?;
    validate_facade_declaration_provenance(facade_interface, facade_module)?;
    let FacadeRootPlan {
        selected_base_modules,
        roots_by_module,
    } = plan_facade_roots(
        facade_module_ref,
        facade_interface,
        module_owner_by_origin,
        &execution_owners.function_owner_by_origin,
        "Project package",
    )?;
    let reachability_by_module = collect_facade_reachability(
        roots_by_module,
        project,
        source_packages,
        execution_owners,
        "Project package",
    )?;

    Ok(Some((
        facade_module_ref,
        selected_base_modules,
        reachability_by_module,
    )))
}
fn collect_facade_reachability(
    mut roots_by_module: FxHashMap<CompiledModuleRef, FxHashSet<FunctionId>>,
    project: &CompiledGraphBoundary,
    source_packages: &CompletedSourcePackageRegistry,
    execution_owners: &ExecutionOwnerMaps,
    label: &str,
) -> Result<FacadeReachabilityByModule, ProjectAssemblyError> {
    let mut pending_modules = roots_by_module.keys().copied().collect::<Vec<_>>();
    pending_modules.sort();
    let mut pending_modules = VecDeque::from(pending_modules);
    let mut reachability_by_module = FxHashMap::default();

    while let Some(reachable_module_ref) = pending_modules.pop_front() {
        let mut roots = roots_by_module
            .get(&reachable_module_ref)
            .into_iter()
            .flat_map(|roots| roots.iter().copied())
            .collect::<Vec<_>>();
        roots.sort_by_key(|function_id| function_id.0);
        let reachable_module = boundary_module_at(project, source_packages, reachable_module_ref)?;
        let reachability = collect_reachability_from_function_link_facts(
            &reachable_module.link_facts.functions,
            &roots,
        )?;
        if let Some(offending_function) = reachability.first_project_context_function() {
            let mut string_table = StringTable::new();
            let location = offending_function
                .diagnostic_location()
                .map(|location| location.to_source_location(&mut string_table))
                .unwrap_or_else(|| {
                    project_context_location_for_module(reachable_module, &mut string_table)
                });
            return Err(ProjectAssemblyError::project_context_escape(
                ProjectContextEscapeReason::ReachableExecutable,
                location,
                string_table,
            ));
        }

        for origin in &reachability.reachable_cross_module_functions {
            let Some((provider_module_ref, provider_function_id)) = execution_owners
                .function_owner_by_origin
                .get(origin)
                .copied()
            else {
                return Err(CompilerError::compiler_error(format!(
                    "{label} could not resolve cross-module function origin {origin:?}"
                ))
                .into());
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
            let Some((provider_module_ref, provider_function_id)) = execution_owners
                .function_owner_by_private_identity
                .get(identity)
                .copied()
            else {
                return Err(CompilerError::compiler_error(format!(
                    "{label} could not resolve module-private function identity {identity:?}"
                ))
                .into());
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
            let Some((generated_module_ref, generated_function_id)) = generated_owner_for(
                &execution_owners.project_generated_owners,
                &execution_owners.package_generated_owners,
                reachable_module_ref,
                identity,
            )
            .copied() else {
                return Err(CompilerError::compiler_error(format!(
                    "{label} could not resolve generated function identity {identity:?} in its calling boundary"
                ))
                .into());
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

    Ok(reachability_by_module)
}

fn validate_facade_declaration_provenance(
    facade_interface: &PublicSemanticInterface,
    facade_module: &Module,
) -> Result<(), ProjectAssemblyError> {
    for declaration in &facade_interface.declarations {
        if declaration
            .synthetic_interface_provenance
            .contains_class(SyntheticInterfaceClass::ProjectContext)
        {
            let (location, string_table) = project_context_location_for_declaration(
                facade_interface,
                declaration,
                facade_module,
            );
            return Err(ProjectAssemblyError::project_context_escape(
                ProjectContextEscapeReason::ExportedDeclaration,
                location,
                string_table,
            ));
        }
    }
    Ok(())
}

fn project_context_location_for_declaration(
    facade_interface: &PublicSemanticInterface,
    declaration: &PublicDeclarationRecord,
    facade_module: &Module,
) -> (SourceLocation, StringTable) {
    let mut string_table = StringTable::new();
    let public_name = facade_interface
        .export_bindings
        .iter()
        .find(|binding| binding.origin() == &declaration.origin)
        .map(|binding| binding.public_name());

    if let Some(public_name) = public_name
        && let Some(provenance) = facade_interface
            .export_diagnostic_provenance
            .iter()
            .find(|provenance| provenance.public_name == public_name)
    {
        let location = &provenance.location;
        let scope = InternedPath::from_components(
            location
                .scope_components
                .iter()
                .map(|component| string_table.intern(component))
                .collect(),
        );
        return (
            SourceLocation::new(
                scope,
                CharPosition {
                    line_number: location.start_line,
                    char_column: location.start_column,
                },
                CharPosition {
                    line_number: location.end_line,
                    char_column: location.end_column,
                },
            ),
            string_table,
        );
    }

    (
        project_context_location_for_module(facade_module, &mut string_table),
        string_table,
    )
}

fn project_context_location_for_module(
    module: &Module,
    string_table: &mut StringTable,
) -> SourceLocation {
    if module.metadata.entry_point.as_os_str().is_empty() {
        return SourceLocation::new(
            InternedPath::from_single_str("<package-facade>", string_table),
            CharPosition::default(),
            CharPosition::default(),
        );
    }

    SourceLocation::from_path(&module.metadata.entry_point, string_table)
}
fn build_module_owner_by_origin(
    project: &CompiledGraphBoundary,
    source_packages: &CompletedSourcePackageRegistry,
) -> Result<FxHashMap<StableModuleOriginIdentity, CompiledModuleRef>, CompilerError> {
    let mut module_owner_by_origin =
        FxHashMap::<StableModuleOriginIdentity, CompiledModuleRef>::default();
    for node in project.structure.nodes() {
        if module_owner_by_origin
            .insert(
                node.stable_origin().clone(),
                CompiledModuleRef::Project(node.module_id()),
            )
            .is_some()
        {
            return Err(CompilerError::compiler_error(format!(
                "Project package assembly found duplicate project module origin {:?}",
                node.stable_origin()
            )));
        }
    }
    for (package_index, package) in source_packages.iter().enumerate() {
        let package_id = PackageBoundaryId::from_index(package_index);
        for node in package.boundary.structure.nodes() {
            if module_owner_by_origin
                .insert(
                    node.stable_origin().clone(),
                    CompiledModuleRef::SourcePackage {
                        package_id,
                        module_id: node.module_id(),
                    },
                )
                .is_some()
            {
                return Err(CompilerError::compiler_error(format!(
                    "Project package assembly found duplicate module origin {:?}",
                    node.stable_origin()
                )));
            }
        }
    }
    Ok(module_owner_by_origin)
}
fn validate_source_package_facades(
    project: &CompiledGraphBoundary,
    source_packages: &CompletedSourcePackageRegistry,
    module_owner_by_origin: &FxHashMap<StableModuleOriginIdentity, CompiledModuleRef>,
    execution_owners: &ExecutionOwnerMaps,
) -> Result<(), ProjectAssemblyError> {
    for (package_index, package) in source_packages.iter().enumerate() {
        let package_id = PackageBoundaryId::from_index(package_index);
        let facade_module_ref = CompiledModuleRef::SourcePackage {
            package_id,
            module_id: package.root_module_id,
        };
        let facade_interface = package.root_interface()?;
        let facade_module = boundary_module_at(project, source_packages, facade_module_ref)?;
        let label = format!("Source package @{} facade", package.package_prefix());
        validate_facade_declaration_provenance(facade_interface, facade_module)?;
        let FacadeRootPlan {
            roots_by_module, ..
        } = plan_facade_roots(
            facade_module_ref,
            facade_interface,
            module_owner_by_origin,
            &execution_owners.function_owner_by_origin,
            &label,
        )?;
        collect_facade_reachability(
            roots_by_module,
            project,
            source_packages,
            execution_owners,
            &label,
        )?;
    }
    Ok(())
}
/// Validate facade declaration provenance and executable reachability before `check` renders.
///
/// `ProjectCompilation` performs this proof while assembling the success-only build payload.
/// `check` keeps its frontend payload instead, so it must run the same facade-only validation
/// without constructing entry or output assemblies.
pub(crate) fn validate_frontend_facade_boundaries(
    frontend: &ProjectFrontendCompilation,
) -> Result<(), ProjectAssemblyError> {
    if frontend.has_diagnosed_or_blocked() {
        return Ok(());
    }

    let owner_maps = build_execution_owner_maps(&frontend.project, &frontend.source_packages)?;
    let module_owner_by_origin =
        build_module_owner_by_origin(&frontend.project, &frontend.source_packages)?;
    validate_source_package_facades(
        &frontend.project,
        &frontend.source_packages,
        &module_owner_by_origin,
        &owner_maps,
    )?;
    let _ = plan_project_package_facade(
        &frontend.project,
        &frontend.source_packages,
        &module_owner_by_origin,
        &owner_maps,
    )?;
    Ok(())
}

/// Build-owned link plan for the optional project package facade.
///
/// The facade has no implicit `start`. Its exported function origins seed reachability, while
/// exported folded values and selected descendant interfaces contribute metadata-only resources.
#[allow(dead_code)] // package-target fields are consumed by later package backends
pub(crate) struct PackageAssembly {
    facade_module_ref: CompiledModuleRef,
    facade_reachability: HirReachability,
    selected_modules: Vec<CompiledModuleRef>,
    linked_modules: Vec<LinkedModuleAssembly>,
    resource_union: ResourceOriginUnion,
}
#[allow(dead_code)] // package-target fields are consumed by later package backends
impl PackageAssembly {
    pub(crate) fn resource_union(&self) -> &ResourceOriginUnion {
        &self.resource_union
    }

    pub(crate) fn selected_modules(&self) -> &[CompiledModuleRef] {
        &self.selected_modules
    }

    pub(crate) fn linked_modules(&self) -> &[LinkedModuleAssembly] {
        &self.linked_modules
    }

    pub(crate) fn facade_module_ref(&self) -> CompiledModuleRef {
        self.facade_module_ref
    }

    pub(crate) fn facade_reachability(&self) -> &HirReachability {
        &self.facade_reachability
    }
}

#[derive(Clone)]
pub(crate) struct ProjectLinkedModule<'a> {
    pub(crate) module: &'a Module,
    pub(crate) reachability: &'a HirReachability,
    /// Generated symbol lookup for the linked module's own boundary.
    pub(crate) generated_function_names: Arc<
        std::collections::HashMap<
            crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity,
            String,
        >,
    >,
}

/// Owner-bound view of one entry assembly and its selected compiled module.
#[derive(Clone)]
pub(crate) struct ProjectEntry<'a> {
    pub(crate) module: &'a Module,
    /// Exact stable-origin union consumed by the HTML resource planner.
    pub(crate) resource_union: &'a ResourceOriginUnion,
    pub(crate) reachability: &'a HirReachability,
    pub(crate) external_imports: &'a [ModuleExternalImport],
    pub(crate) linked_modules: Vec<ProjectLinkedModule<'a>>,
    pub(crate) source_function_names: Arc<std::collections::HashMap<OriginFunctionId, String>>,
    pub(crate) module_private_function_names:
        Arc<std::collections::HashMap<ModulePrivateExecutableIdentity, String>>,
    /// Generated symbol lookup for the entry module's project boundary.
    pub(crate) generated_function_names: Arc<
        std::collections::HashMap<
            crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity,
            String,
        >,
    >,
    /// Every generated symbol name assigned to this compilation, for JS identifier reservation.
    pub(crate) all_generated_function_names: Arc<Vec<String>>,
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
    /// The project boundary's source identities, with config registered before Stage 0 discovery.
    ///
    /// Single-file and config-free callers leave this absent so their own frontend database stays
    /// the sole identity context for source compilation.
    pub(crate) project_source_files: Option<Arc<SourceDatabase>>,
    /// The typed explicit build inputs this bootstrap was started with.
    ///
    /// WHAT: the `BuildConfigInputSet` the command layer parsed or the programmatic caller
    ///       supplied, carried on the shared Stage 0 state so build, check and dev see one
    ///       set of explicit inputs.
    /// WHY:  direct-project `#Config` resolution and later source-contract barriers consume
    ///       exactly this set; command and programmatic paths own production up to here and
    ///       the compiler config service will read it from here.
    #[allow(dead_code)] // consumed by the build-configuration resolution phases
    pub(crate) build_config_inputs: BuildConfigInputSet,
}

// -------------------------
//  Output Payload
// -------------------------
#[derive(Clone)]
pub struct OutputFile {
    relative_output_path: PathBuf,
    file_kind: FileKind,
}

#[derive(Clone)]
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

/// One resource output whose bytes are materialised by the central output writer.
///
/// WHAT: carries the validated destination and Stage 0 physical source identity without reading
///       the source bytes during backend planning.
/// WHY: output conflict validation must cover resource destinations before any resource IO.
#[derive(Debug, Clone)]
pub(crate) struct DeferredResourceOutput {
    pub(crate) relative_output_path: PathBuf,
    pub(crate) source_id: ResourceSourceId,
}

#[derive(Clone)]
pub struct Project {
    pub output_files: Vec<OutputFile>,
    pub entry_page_rel: Option<PathBuf>,
    /// Builder-owned cleanup contract for manifest tracking and stale artifact removal.
    pub cleanup_policy: CleanupPolicy,
    pub warnings: Vec<CompilerDiagnostic>,
    pub(crate) deferred_resources: Vec<DeferredResourceOutput>,
    pub(crate) resource_inputs: ResourceInputRegistry,
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
    build_config_inputs: &BuildConfigInputSet,
) -> Result<BuildResult, CompilerMessages> {
    let build_profile = BuildProfile::from_flags(flags);
    let mut path_string_table = StringTable::new();
    let valid_path = match check_if_valid_path(entry_path, &mut path_string_table) {
        Ok(path) => path,
        Err(error) => {
            return Err(CompilerMessages::from_error(error, path_string_table));
        }
    };

    // --------------------------------------------
    //   PERFORM THE CORE COMPILER FRONTEND BUILD
    // --------------------------------------------
    // This discovers all the modules, parses the config,
    // and compiles each module to HIR for backend lowering.
    let BuildBootstrap {
        mut config,
        style_directives,
        mut string_table,
        mut frontend_surface,
        validated_directory_output_settings,
        mut project_source_files,
        build_config_inputs,
    } = match bootstrap_project_build(project_builder, valid_path, build_config_inputs) {
        Ok(bootstrap) => bootstrap,
        Err(messages) => {
            return Err(messages);
        }
    };

    let frontend_compilation = match compile_project_frontend_with_inputs(
        &mut config,
        build_profile,
        validated_directory_output_settings.as_ref(),
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
        &mut project_source_files,
        &build_config_inputs,
        FrontendCompilationMode::Canonical,
    ) {
        Ok(frontend_compilation) => frontend_compilation,
        Err(messages) => {
            return Err(messages);
        }
    };
    if frontend_compilation.has_diagnosed_or_blocked() {
        return Err(frontend_compilation.into_render_messages(&mut string_table));
    }
    let project_compilation = ProjectCompilation::from_frontend(frontend_compilation)
        .map_err(|error| error.into_messages(&mut string_table))?;
    let mut warnings: Vec<CompilerDiagnostic> = project_compilation
        .modules()
        .flat_map(collect_module_warnings)
        .collect();

    // --------------------------------------------
    // BUILD PROJECT USING THE APPROPRIATE BUILDER
    // --------------------------------------------

    let project_result = crate::timed_stage!(
        crate::timing::TimingMetric::BuildBackendTotal,
        project_builder.backend.build_backend(
            project_compilation,
            &config,
            build_profile,
            flags,
            &mut string_table,
        )
    );
    let project = match project_result {
        Ok(project) => project,
        Err(mut compiler_messages) => {
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
    // Direct-project resolution records are a bootstrap-to-frontend handoff. No current build or
    // dev consumer retains them after the semantic boundary has consumed their values and
    // provenance; persistent retention belongs to the deferred incremental artefact owner.
    config.config_resolution_records.clear();

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
/// config validation into one reusable setup step. The caller's typed build-config inputs are
/// carried onto the bootstrap state unchanged.
/// WHY: directory builds and the dev server must share one bootstrap path so config/output
/// behavior does not drift between "build" and "serve" flows, and explicit inputs must ride
/// the same shared state instead of command-local storage.
pub(crate) fn bootstrap_project_build(
    project_builder: &ProjectBuilder,
    entry_path: PathBuf,
    build_config_inputs: &BuildConfigInputSet,
) -> Result<BuildBootstrap, CompilerMessages> {
    timing_scope!(
        timing_guard_build_bootstrap_total,
        crate::timing::TimingMetric::BuildBootstrapTotal
    );

    let mut config = Config::new(entry_path);

    let mut project_source_files = config.entry_dir.is_dir().then(SourceDatabase::empty);

    // Seed the build table with the compiler-owned symbols that per-file frontend tables will
    // also need as a stable prefix once file preparation becomes independent.
    let preseeded = CompilerSymbolSet::preseeded_table(FILE_MIN_UNIQUE_SYMBOLS_CAPACITY);
    let mut string_table = preseeded.string_table;
    // The bootstrap path only needs the preseeded table today. File-local preparation will keep
    // these typed IDs alongside its local outputs once fixed-symbol IDs are consumed directly.
    let _compiler_symbol_ids = preseeded.compiler_symbol_ids;

    // Compute the builder's frontend surface once so config loading and frontend compilation
    // see the same set of allowed config keys, external packages, and source-backed packages.
    let frontend_surface = project_builder.backend.frontend_surface();

    let frontend_style_directives = project_builder.backend.frontend_style_directives();
    let style_directives = match StyleDirectiveRegistry::merged(&frontend_style_directives) {
        Ok(style_directives) => style_directives,
        Err(error) => {
            return Err(CompilerMessages::from_error(error, string_table.clone()));
        }
    };
    // WHAT: Load and validate project config before compilation begins (Stage 0).
    // WHY: Backends and serving code both depend on the same validated config surface.
    let config_services = ProjectConfigParseServices {
        style_directives: &style_directives,
        frontend_surface: &frontend_surface,
        build_config_inputs,
    };
    let validated_directory_output_settings = match load_project_config(
        &mut config,
        &config_services,
        &mut string_table,
        project_source_files.as_mut(),
    ) {
        Ok(settings) => settings,
        Err(messages) => {
            return Err(messages);
        }
    };
    // WHAT: Validate backend-specific config requirements before compilation.
    // WHY: Backends should reject unsupported settings before frontend compilation does work.
    if let Err(error) = project_builder
        .backend
        .validate_project_config(&config, &mut string_table)
    {
        return Err(error.into_messages(string_table.clone()));
    }

    Ok(BuildBootstrap {
        config,
        style_directives,
        string_table,
        frontend_surface,
        validated_directory_output_settings,
        project_source_files: project_source_files.map(Arc::new),
        build_config_inputs: build_config_inputs.clone(),
    })
}

fn collect_module_warnings(module: &Module) -> Vec<CompilerDiagnostic> {
    module.metadata.warnings.clone()
}

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
