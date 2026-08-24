//! Compiler-produced module artefact lanes.
//!
//! WHAT: the sealed value one successful module compilation returns — executable state, backend-
//!       neutral link facts, compiler metadata and the published public interface.
//! WHY:  the compiler design overview defines these as compiler result lanes. Declaring them here
//!       lets semantic compilation build them without a dependency back into the build system,
//!       which stores, remaps and publishes them but never produces them.

use crate::builder_surface::external_import_providers::provider::{
    RequiredRuntimeImport, RuntimeAssetIdentity,
};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationContext;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::datatypes::environment::{
    TypeEnvironment, TypeEnvironmentRemapCache,
};
use crate::compiler_frontend::external_packages::{ExternalPackageId, ExternalPackageRegistry};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::HirModuleLinkFacts;
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::module_metadata::{HirLoweringMetadata, ModuleDocFragment};
use crate::compiler_frontend::paths::rendered_path_usage::RenderedPathUsage;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;

use std::path::PathBuf;
use std::sync::Arc;

/// Frontend output for one module root ready for backend lowering.
///
/// WHAT: a lane container with exactly an executable lane (typed HIR, paired type environment
///       and borrow facts), a link-facts lane (per-function runtime facts, external imports and
///       the effective registry), and a compiler-metadata lane (entry path, warnings, fragments,
///       root activity, docs and rendered paths).
/// WHY: backends consume one stable module payload shape regardless of project type, with
///      explicit ownership keeping HIR/type/borrow pairing obvious at call sites.
pub(crate) struct Module {
    pub(crate) executable: ModuleExecutable,
    pub(crate) link_facts: ModuleLinkFacts,
    pub(crate) metadata: ModuleCompilerMetadata,
}

impl Module {
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.remap_string_ids_with_type_environment_cache(
            remap,
            &mut TypeEnvironmentRemapCache::default(),
        );
    }

    pub(crate) fn remap_string_ids_with_type_environment_cache(
        &mut self,
        remap: &StringIdRemap,
        type_environment_cache: &mut TypeEnvironmentRemapCache,
    ) {
        increment_frontend_counter(FrontendCounter::ModuleRemapStringIdsCalls);

        self.executable
            .remap_string_ids_with_type_environment_cache(remap, type_environment_cache);
        self.link_facts.functions.remap_string_ids(remap);
        self.metadata.remap_string_ids(remap);
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
    pub(crate) fn remap_string_ids_with_type_environment_cache(
        &mut self,
        remap: &StringIdRemap,
        type_environment_cache: &mut TypeEnvironmentRemapCache,
    ) {
        self.hir.remap_string_ids(remap);
        self.type_environment
            .remap_string_ids_with_cache(remap, type_environment_cache);
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
    ///
    /// Shared by `Arc` so the boundary materialisation registry can keep resolving templates while
    /// the build system's artefact storage grows behind it.
    pub(crate) materialisation_context: Option<Arc<ModuleMaterialisationContext>>,
}

impl ModuleCompilerMetadata {
    pub(crate) fn from_hir_lowering(
        entry_point: PathBuf,
        warnings: Vec<CompilerDiagnostic>,
        lowering_metadata: HirLoweringMetadata,
        const_top_level_fragments: Vec<ResolvedConstFragment>,
        root_activity: ModuleRootActivity,
        materialisation_context: Option<Arc<ModuleMaterialisationContext>>,
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

/// A resolved const top-level fragment: a static string and its runtime insertion index.
///
/// WHAT: carries a fully resolved (not interned) const fragment string plus the count of
/// runtime fragments that precede it in source order.
/// WHY: builders merge const strings with the runtime fragment list returned by entry start()
/// using the insertion index to reconstruct source-order interleaving.
pub(crate) struct ResolvedConstFragment {
    /// Number of runtime fragments preceding this const fragment in source order.
    pub runtime_insertion_index: usize,
    /// The rendered text content of this const fragment.
    pub rendered_text: String,
}

/// Backend-facing identity for one external import used by a compiled module.
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
