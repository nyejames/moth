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
use crate::compiler_frontend::analysis::numeric_proofs::NumericProofs;
use crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationContext;
use crate::compiler_frontend::builder_root_slots::ConstFragmentDestination;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::datatypes::environment::{
    TypeEnvironment, TypeEnvironmentRemapCache,
};
use crate::compiler_frontend::external_packages::{ExternalPackageId, ExternalPackageRegistry};
use crate::compiler_frontend::folded_value::OwnedFoldedString;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::HirModuleLinkFacts;
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::module_metadata::{HirLoweringMetadata, ModuleDocFragment};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathIdRemap, PathTable};
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;

use std::path::PathBuf;
use std::sync::Arc;

/// Frontend output for one module root ready for backend lowering.
///
/// WHAT: a lane container with exactly an executable lane (typed HIR, its paired resource table,
///       type environment and borrow facts), a link-facts lane (per-function runtime facts,
///       fragments, root activity and docs).
/// WHY: backends consume one stable module payload shape regardless of project type, with explicit
///      ownership keeping HIR/type/borrow/resource pairing obvious at call sites.
pub(crate) struct Module {
    pub(crate) executable: ModuleExecutable,
    pub(crate) link_facts: ModuleLinkFacts,
    pub(crate) metadata: ModuleCompilerMetadata,
}

impl Module {
    pub(crate) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.executable.remap_path_ids(remap);
        self.metadata.remap_path_ids(remap);
    }

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
    }
}

/// One successful canonical module artefact.
///
/// WHAT: pairs the backend-neutral executable/link/metadata lanes with the immutable API surfaces
///       the module's present facade roots published to later graph waves.
/// WHY: provider publication must point at a complete success value without retaining diagnostics
///      in a semantic artefact. An entry-only module publishes neither surface, and each surface
///      is present exactly when its facade root exists.
pub(crate) struct CompiledModuleArtifact {
    pub(crate) module: Module,
    /// The normal `@mod` facade surface, present exactly when the module owns a normal root.
    pub(crate) normal_interface: Option<PublicSemanticInterface>,
    /// The support `@package` facade surface, present exactly when the module owns a support root.
    pub(crate) support_interface: Option<PublicSemanticInterface>,
}

/// Module-local executable semantic state: validated HIR, paired type environment and borrow
/// facts.
///
/// WHAT: the sole owner of the typed HIR, its paired `TypeEnvironment`, the resource table that
///       issues every structural-string `ResourceId`, and the `BorrowCheckReport` produced by
///       borrow validation.
/// WHY: keeping the table here pairs every dense resource handle with the HIR that carries it,
///      while keeping HIR/type/borrow/resource state together for backend lowering.
pub(crate) struct ModuleExecutable {
    pub(crate) hir: HirModule,
    pub(crate) resource_table: ModuleResourceTable,
    pub(crate) type_environment: TypeEnvironment,
    pub(crate) borrow_analysis: BorrowCheckReport,
    /// Conservative bounded-integer proof facts computed from this exact validated HIR.
    ///
    /// WHAT: statement-keyed proven-safe operation/narrowing facts paired with this executable;
    ///       the empty default table retains every runtime check.
    /// WHY: the table is computed inside the compiler service after HIR validation and before
    ///      publication, so backends consume facts that pair with the immutable statement ids
    ///      they lower. Facts carry no interned strings, paths or dense layouts, so they need
    ///      no remap when the executable is published.
    pub(crate) numeric_proofs: NumericProofs,
    /// Immutable path identity table shared by the enclosing project/package compilation boundary.
    /// It covers every `PathId` retained by this executable after publication.
    ///
    /// The semantic compiler owns a module-local fork while producing the executable. The build
    /// publication tail installs one merged boundary table here and shares that `Arc` across
    /// completed module and generated-sidecar artefacts before backend consumption.
    pub(crate) path_table: Arc<PathTable>,
}

impl ModuleExecutable {
    /// Remap every path identity retained by the executable lane after publication merge.
    pub(crate) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.hir.remap_path_ids(remap);
        self.type_environment.remap_path_ids(remap);
    }

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
    ///      loses provider-created packages. The registry is carried whole, not as stable
    ///      binding identities.
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
/// WHAT: owns the optional entry-root metadata, resolved documentation fragments and the
///       declaring-module materialisation context.
/// WHY: these are compiler-metadata lanes, not executable HIR state or link facts. Consolidating
///      them into one owned lane on the `Module` payload keeps HIR limited to executable/semantic
///      IR and gives backend assembly a single metadata owner. Entry metadata is an explicit
///      typed absence for API-only and support-only modules, so no consumer can mistake the
///      preparation anchor for an executable entry.
pub(crate) struct ModuleCompilerMetadata {
    /// Start-bearing root metadata. Present exactly when this module owns an entry root or a
    /// legacy combined normal root; `None` for API-only and support-only modules.
    pub(crate) entry: Option<EntryRootMetadata>,
    #[allow(dead_code)] // Retained for deferred documentation-metadata consumers.
    pub(crate) doc_fragments: Vec<ModuleDocFragment>,
    /// Self-contained declaring-module semantics used by generated-function materialisation.
    ///
    /// Shared by `Arc` so the boundary materialisation registry can keep resolving templates while
    /// the build system's artefact storage grows behind it.
    pub(crate) materialisation_context: Option<Arc<ModuleMaterialisationContext>>,
}

/// Entry-root metadata carried only by a module with a start-bearing root.
///
/// WHAT: owns the resolved entry path, resolved const top-level fragments with their body/named
///       destinations and authored spans, and the header-derived dormant root activity.
/// WHY: an entryless module has no dormant start, no entry fragment schedule and no entry path,
///      so these facts live behind one typed presence rather than as empty placeholders.
pub(crate) struct EntryRootMetadata {
    /// Canonical entry file: `@entry.moth`, or the legacy combined normal root under the
    /// migration bridge.
    pub(crate) entry_point: PathBuf,
    pub(crate) const_top_level_fragments: Vec<ResolvedConstFragment>,
    pub(crate) root_activity: ModuleRootActivity,
}

impl ModuleCompilerMetadata {
    pub(crate) fn from_hir_lowering(
        entry: Option<EntryRootMetadata>,
        lowering_metadata: HirLoweringMetadata,
        materialisation_context: Option<Arc<ModuleMaterialisationContext>>,
    ) -> Self {
        Self {
            entry,
            doc_fragments: lowering_metadata.doc_fragments,
            materialisation_context,
        }
    }

    /// Borrow the entry metadata, or report that an entry consumer received an entryless module.
    ///
    /// WHY: entry selection, HTML artefact planning and entry resource assembly are only valid
    ///      for a start-bearing root. Callers must receive the typed absence rather than
    ///      fabricating an entry path or an empty fragment schedule.
    pub(crate) fn require_entry(
        &self,
        consumer: &str,
    ) -> Result<&EntryRootMetadata, InfrastructureFailure> {
        self.entry.as_ref().ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!("{consumer} received a module without entry metadata"),
            )
        })
    }

    /// WHY: entry metadata owns every remapped field; `entry_point` is a `PathBuf`, const-fragment
    ///      spans are global, values are already owned folded strings and root activity has no
    ///      interned fields. Only the materialisation context carries path identities.
    pub(crate) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        if let Some(context) = &mut self.materialisation_context {
            Arc::make_mut(context).remap_path_ids(remap);
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

/// A resolved const top-level fragment: an owned folded string, its destination, its authored
/// source span and runtime insertion index.
///
/// WHAT: carries the fully resolved module-local structural string value, the validated default
///       body or named builder destination, the authored source span when available, and the
///       count of runtime fragments that precede it in source order.
/// WHY: builders merge body fragments with the runtime fragment list using the insertion index
///      to reconstruct source-order interleaving. Named destinations stay out of body rendering
///      but remain in the module artefact for builder-specific interpretation and resource
///      liveness.
pub(crate) struct ResolvedConstFragment {
    /// Number of runtime fragments preceding this const fragment in source order.
    pub runtime_insertion_index: usize,
    /// Default body lane or one builder-registered named destination.
    pub destination: ConstFragmentDestination,
    /// The authored source span of this const fragment, when available.
    pub span: Option<SourceSpan>,
    /// The owned structural string value of this const fragment.
    pub value: OwnedFoldedString,
}

/// Backend-facing identity for one external import used by a compiled module.
///
/// WHAT: carries the backend-facing identity for a provider-resolved external import after
///       deduplication across a module's source files.
/// WHY: backends emit runtime assets and generated glue based on this metadata without needing
///      the full per-source-file resolution table.
#[derive(Debug, Clone)]
// Kept ahead of the backend handoff: external provider/runtime metadata is recorded by the
// frontend, while the current HTML path only reads the subset it can lower.
pub(crate) struct ModuleExternalImport {
    pub(crate) package_id: ExternalPackageId,
    pub(crate) runtime_asset: Option<RuntimeAssetIdentity>,
    pub(crate) required_runtime_imports: Vec<RequiredRuntimeImport>,
}
