//! The canonical module compilation service.
//!
//! WHAT: one entry point that takes a ready module — prepared source plus completed provider
//!       interfaces — and returns a complete semantic result, a diagnosed result or a
//!       `InfrastructureFailure`.
//! WHY: the compiler owns local semantic compilation. Sequencing interface binding, declaration
//!      ordering, AST semantics, public-interface projection, HIR lowering and validation, borrow
//!      validation and generated completion is compiler work, so it lives behind one call rather
//!      than being assembled by the build system. Stage 0 decides when a module is ready and what
//!      happens to the result; it never invokes these stages individually.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::analysis::numeric_proofs::analyse_numeric_proofs;
use crate::compiler_frontend::arena::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::AstBuildResult;
use crate::compiler_frontend::compiler_errors::{InfrastructureErrors, InfrastructureFailure};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, DiagnosticDraft, PreparedFailure,
};
#[cfg(feature = "boracle")]
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::owned_folded_string_from_const_string;
use crate::compiler_frontend::headers::parse_file_headers::{
    BoundModuleHeaders, HeaderKind, HeaderPreparationFailure, PreparedHeaderSyntax,
    bind_module_headers,
};
use crate::compiler_frontend::hir::functions::{
    HirFunctionOriginLookup, PrivateFunctionOriginSeed,
};
#[cfg(feature = "boracle")]
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::collect_module_function_link_facts;
use crate::compiler_frontend::instrumentation::{
    FrontendCounter, add_frontend_counter, increment_frontend_counter,
};
use crate::compiler_frontend::module_compilation::artefact::{
    EntryRootMetadata, Module, ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts,
    ModuleRootActivity, ResolvedConstFragment,
};
use crate::compiler_frontend::module_compilation::context::ModuleCompilationContext;
use crate::compiler_frontend::module_compilation::external_imports::collect_external_import_candidates;
use crate::compiler_frontend::module_compilation::generated::artefacts::GeneratedFunctionDelta;
use crate::compiler_frontend::module_compilation::generated::convergence::{
    install_exact_concrete_call_summaries, run_generated_summary_convergence,
};
use crate::compiler_frontend::module_compilation::generated::known::KnownGeneratedFunctions;
use crate::compiler_frontend::module_compilation::generated::materialisation::materialise_generated_request_roots;
use crate::compiler_frontend::module_compilation::generated::never_proofs::GeneratedNeverProofs;
use crate::compiler_frontend::module_compilation::generated::requests::install_generated_request_contracts;
use crate::compiler_frontend::module_compilation::generated::transaction::{
    GeneratedFunctionTransaction, GeneratedRequestFacts,
};
use crate::compiler_frontend::module_compilation::outcome::{
    ModuleCompilationOutcome, ModuleSemanticResult,
};
use crate::compiler_frontend::module_compilation::prepared::PreparedModuleInput;
use crate::compiler_frontend::module_compilation::stages::{check_borrows, lower_hir};
use crate::compiler_frontend::module_dependencies::SortedHeaders;
use crate::compiler_frontend::module_metadata::HirLoweringResult;
use crate::compiler_frontend::paths::file_references::ResolvedFileReferenceTable;
use crate::compiler_frontend::paths::module_resources::ResourceSourceAssociation;
use crate::compiler_frontend::public_interface::{
    PublicInterfaceDraftBuildFailure, PublicInterfaceDraftBuilder,
    PublicInterfaceDraftBuilderInput, PublicSemanticInterface, SourceProviderDependencySet,
    build_direct_export_seed, build_public_source_nominal_origin_index,
    build_public_source_trait_origin_index,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, OriginFunctionId, StableModuleOriginIdentity,
};
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::validated_generic_template_metadata::validate_materialisation_context_templates;
use crate::compiler_frontend::{AstBuildRequest, CompilerFrontend};
use crate::{borrow_log, timed_stage_attributed};

use rustc_hash::FxHashMap;
use std::path::Path;
#[cfg(feature = "boracle")]
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// Validated-HIR payload returned by the internal Boracle compiler service.
#[cfg(feature = "boracle")]
pub(crate) struct BoracleModuleInput {
    pub(crate) hir: HirModule,
    pub(crate) external_package_registry: Arc<ExternalPackageRegistry>,
    /// The preparation anchor that located this module's source. Boracle analyses any module,
    /// including entryless API-only roots, so this is a source identity rather than an entry.
    pub(crate) source_path: PathBuf,
}

/// Compile one ready module through the canonical local semantic sequence.
///
/// WHAT: binds retained header syntax against completed provider interfaces, orders local
///       declarations, runs AST semantics, projects one interface per present facade root, lowers
///       and validates HIR, collects link facts, borrow-validates, completes generated semantic
///       work and closes each public interface. It receives no source text or tokens and cannot
///       rerun file preparation. The module origin is resolved from the retained preparation
///       anchor `SourceId`, and the executable entry file from the entry root `SourceId`, never
///       reconstructed from source paths.
/// WHY: this is the one production owner of the binding -> ordering -> AST -> HIR -> borrow
///      sequence. Everything the sequence needs arrives as immutable input, and everything it
///      produces leaves as one typed outcome, so the build system can schedule it without knowing
///      how a module is compiled.
pub(crate) fn compile_module(
    context: &ModuleCompilationContext<'_>,
    prepared: PreparedModuleInput,
    preparation_warnings: Vec<DiagnosticDraft>,
    known_generated: KnownGeneratedFunctions<'_>,
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> Result<ModuleCompilationOutcome, Box<InfrastructureErrors>> {
    // The preparation anchor and executable entry root are retained identities, so both are
    // resolved from the payload rather than repeated as arguments the caller must keep in sync.
    let module_anchor_path = prepared
        .preparation_anchor_path(context.source_files)?
        .to_path_buf();
    let module_anchor_path = module_anchor_path.as_path();
    let entry_point_path = prepared.entry_point_path(context.source_files)?;
    let synthesizes_entry_start = prepared.synthesizes_entry_start();
    let preparation_anchor = prepared.preparation_anchor()?;
    let facade_surface_roots = prepared.facade_surface_roots();

    let PreparedModuleInput {
        candidate_source_ids,
        source_module_origins,
        prepared_header_syntax,
        resolved_file_references,
        string_table,
        path_fork,
        source_file_count,
        source_byte_count,
        ..
    } = prepared;
    resolved_file_references.validate()?;
    // The module origin is resolved from the shared boundary source-origin table using the
    // retained preparation-anchor `SourceId`, not from a loose origin argument. It anchors
    // resource and generated-identity provenance for the one structural module; it grants no
    // surface or entry authority. Preparation already validated the anchor's table origin, so
    // the semantic projection re-derives the same origin from the table.
    let module_origin = source_module_origins
        .origin_for(preparation_anchor)?
        .unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "semantic module compilation: preparation anchor file id {} has no module origin",
                preparation_anchor.index()
            )
        })
        .clone();
    let mut compiler = CompilerFrontend::new(
        context.options.clone(),
        string_table,
        path_fork,
        context.style_directives,
        &context.external_packages,
        context.project_path_resolver,
        context.source_files,
    );

    let compile_result = run_semantic_stages(
        &mut compiler,
        context,
        known_generated,
        preparation_warnings,
        SemanticStageInputs {
            facade_surface_roots,
            candidate_source_ids,
            prepared_header_syntax,
            source_module_origins,
            preparation_anchor,
            module_origin,
            module_anchor_path,
            entry_point_path,
            synthesizes_entry_start,
            source_file_count,
            source_byte_count,
            resolved_file_references,
        },
        SemanticStageRequest::Complete,
        #[cfg(feature = "timers")]
        timing_context,
    );

    // Normalize premerge and prepared semantic failures once at this boundary. A successful
    // compilation becomes `Success`; a diagnosed failure becomes `Diagnosed` after boundary
    // validation, while infrastructure failures propagate unchanged.
    match compile_result {
        Ok(SemanticStageOutput::Complete(output, sidecar_warnings)) => {
            let CompleteSemanticStage {
                module,
                warnings,
                normal_interface,
                support_interface,
                generated_delta,
                resource_source_associations,
            } = *output;
            let string_table = compiler.string_table;
            let path_fork = compiler.path_fork;
            Ok(ModuleCompilationOutcome::Success {
                compiled: Box::new(ModuleSemanticResult {
                    module,
                    generated_delta,
                    resource_source_associations,
                    string_table,
                    path_fork,
                    normal_interface,
                    support_interface,
                }),
                warnings,
                sidecar_warnings,
            })
        }
        #[cfg(feature = "boracle")]
        Ok(SemanticStageOutput::Boracle(_)) => crate::compiler_bug!(
            CompilerStage::ProjectBuildAssembly,
            "normal module compilation unexpectedly stopped at the Boracle prefix"
        ),
        Err(PreparedFailure::Diagnosed(batch)) => match batch.classify_premerge() {
            Ok(prepared) => Ok(ModuleCompilationOutcome::Diagnosed(prepared)),
            Err(error) => Err(error.into()),
        },
        Err(PreparedFailure::Infrastructure(error)) => Err(error),
    }
}

/// Compile one prepared module through validated HIR for the internal Boracle service.
///
/// This is a separate compiler service because Boracle intentionally stops before alpha borrow
/// acceptance, generated convergence and backend-facing module assembly. Normal module
/// compilation therefore has no Boracle outcome to handle.
#[cfg(feature = "boracle")]
pub(crate) fn compile_module_for_boracle(
    context: &ModuleCompilationContext<'_>,
    prepared: PreparedModuleInput,
    preparation_warnings: Vec<DiagnosticDraft>,
    known_generated: KnownGeneratedFunctions<'_>,
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> Result<BoracleModuleInput, PreparedFailure> {
    // Both typed lanes stay intact until the final build boundary performs the one vessel
    // conversion. Infrastructure errors propagate without building a second vessel or cloning
    // the local table.
    let module_anchor_path = prepared
        .preparation_anchor_path(context.source_files)?
        .to_path_buf();
    let module_anchor_path = module_anchor_path.as_path();
    let entry_point_path = prepared.entry_point_path(context.source_files)?;
    let synthesizes_entry_start = prepared.synthesizes_entry_start();
    let preparation_anchor = prepared.preparation_anchor()?;
    let facade_surface_roots = prepared.facade_surface_roots();
    let PreparedModuleInput {
        candidate_source_ids,
        source_module_origins,
        prepared_header_syntax,
        resolved_file_references,
        string_table,
        path_fork,
        source_file_count,
        source_byte_count,
        ..
    } = prepared;
    resolved_file_references.validate()?;
    let module_origin = source_module_origins
        .origin_for(preparation_anchor)?
        .unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "semantic Boracle compilation: preparation anchor file id {} has no module origin",
                preparation_anchor.index()
            )
        })
        .clone();
    let mut compiler = CompilerFrontend::new(
        context.options.clone(),
        string_table,
        path_fork,
        context.style_directives,
        &context.external_packages,
        context.project_path_resolver,
        context.source_files,
    );
    match run_semantic_stages(
        &mut compiler,
        context,
        known_generated,
        preparation_warnings,
        SemanticStageInputs {
            prepared_header_syntax,
            facade_surface_roots,
            candidate_source_ids,
            source_module_origins,
            preparation_anchor,
            module_origin,
            module_anchor_path,
            entry_point_path,
            synthesizes_entry_start,
            source_file_count,
            source_byte_count,
            resolved_file_references,
        },
        SemanticStageRequest::Boracle,
        #[cfg(feature = "timers")]
        timing_context,
    ) {
        Ok(SemanticStageOutput::Boracle(input)) => Ok(*input),
        Ok(SemanticStageOutput::Complete(..)) => crate::compiler_bug!(
            CompilerStage::ProjectBuildAssembly,
            "Boracle compiler service unexpectedly completed the normal semantic pipeline"
        ),
        Err(failure) => Err(failure),
    }
}

/// Everything one semantic stage run reads from the prepared payload.
///
/// WHAT: the retained syntax, ordered candidate source IDs and module-local identity facts
///       [`run_semantic_stages`] consumes, after the string table remains local to the compiler
///       and the source database is borrowed from the enclosing compilation boundary. The
///       source-origin table is an immutable boundary handle shared by all module runs in that
///       identity domain.
/// WHY: nine values from one prepared module travel together through the whole stage sequence.
///      Naming the bundle keeps `compile_module` readable as setup, one stage run and one
///      classification, instead of a parameter list that hides which value came from where.
struct SemanticStageInputs<'a> {
    prepared_header_syntax: PreparedHeaderSyntax,
    /// The present API facade roots in the fixed normal, then support slots. Each present slot
    /// projects one interface; two empty slots mean the module publishes none.
    facade_surface_roots: [Option<(SourceId, ModuleRootRole)>; 2],
    candidate_source_ids: Vec<SourceId>,
    source_module_origins: Arc<SourceModuleOriginTable>,
    /// The one root selected to anchor preparation. It identifies which projection input came from
    /// the AST anchor, not which surfaces the module publishes.
    preparation_anchor: SourceId,
    /// The stable origin of the preparation anchor. It anchors the AST module identity for
    /// resource and generated-identity provenance.
    module_origin: StableModuleOriginIdentity,
    /// The preparation-anchor path, used only to locate the module's AST entry directory.
    module_anchor_path: &'a Path,
    /// The executable entry-root path, or a typed absence for an entryless module.
    entry_point_path: Option<&'a Path>,
    /// Whether preparation synthesized the compiler-owned `start` body.
    synthesizes_entry_start: bool,
    source_file_count: usize,
    source_byte_count: usize,
    /// Stage 0's settled file-reference targets, consumed by dependency sorting for content edges.
    resolved_file_references: ResolvedFileReferenceTable,
}

enum SemanticStageOutput {
    Complete(Box<CompleteSemanticStage>, Vec<DiagnosticBag>),
    #[cfg(feature = "boracle")]
    Boracle(Box<BoracleModuleInput>),
}

/// Private request used by the two named compiler services to share the validated-HIR prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SemanticStageRequest {
    Complete,
    #[cfg(feature = "boracle")]
    Boracle,
}

struct CompleteSemanticStage {
    module: Module,
    warnings: DiagnosticBag,
    normal_interface: Option<PublicSemanticInterface>,
    support_interface: Option<PublicSemanticInterface>,
    generated_delta: GeneratedFunctionDelta,
    resource_source_associations: Vec<ResourceSourceAssociation>,
}

/// Run the local semantic sequence for one module, from bound headers to a complete result.
///
/// WHAT: binding -> ordering -> AST -> public-interface projection -> HIR -> borrow validation ->
///       generated completion, returning the complete module, public interface, generated delta
///       and resource-source associations a success is made of.
/// WHY: binding/order retain premerge failures, while AST/HIR/borrow diagnostics cross as captured
///       prepared batches. The module boundary classifies each lane as it completes and keeps
///       this sequence readable as one semantic pipeline.
fn run_semantic_stages(
    compiler: &mut CompilerFrontend<'_>,
    context: &ModuleCompilationContext<'_>,
    known_generated: KnownGeneratedFunctions<'_>,
    mut warnings: Vec<DiagnosticDraft>,
    inputs: SemanticStageInputs<'_>,
    request: SemanticStageRequest,
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> Result<SemanticStageOutput, PreparedFailure> {
    #[cfg(not(feature = "boracle"))]
    let _ = request;
    let SemanticStageInputs {
        prepared_header_syntax,
        facade_surface_roots,
        candidate_source_ids,
        source_module_origins,
        preparation_anchor,
        module_origin,
        module_anchor_path,
        entry_point_path,
        synthesizes_entry_start,
        source_file_count,
        source_byte_count,
        resolved_file_references,
    } = inputs;

    // Resolve the ordered candidate source IDs before header syntax is consumed. These are the
    // module's complete owned candidates, including sources not reached by the header walk, so
    // external-import resolution stays scoped to this module's source table.
    let source_logical_paths = if context.project_path_resolver.is_some() {
        let path_table = compiler.source_files.paths();
        let mut path_scratch = Vec::new();
        candidate_source_ids
            .iter()
            .map(|source_id| {
                compiler
                    .source_files
                    .get(*source_id)
                    .map(|identity| {
                        path_table.render_portable(
                            identity.logical_path,
                            &compiler.string_table,
                            &mut path_scratch,
                        )
                    })
                    .ok_or_else(|| {
                        crate::compiler_bug!(
                            CompilerStage::ProjectBuildAssembly,
                            "semantic module compilation: candidate source ID {} is not in the boundary source file table",
                            source_id.index()
                        )
                    })
            })
            .collect::<Result<Vec<_>, PreparedFailure>>()?
    } else {
        Vec::new()
    };

    let mut generated_transaction = GeneratedFunctionTransaction::new(known_generated);
    let external_dependency_resolution_table = context.external_dependency_resolution_table;

    // 1. Bind retained header syntax against provider interfaces.
    let module_headers = timed_stage_attributed!(
        crate::timing::TimingMetric::FrontendBindHeaders,
        timing_context,
        {
            bind_retained_headers(
                compiler,
                prepared_header_syntax,
                external_dependency_resolution_table,
                context.source_provider_dependencies,
            )
        }
    )?;

    let capacity_estimate =
        record_frontend_capacity_estimate(source_file_count, source_byte_count, &module_headers);

    // 2. Resolve dependencies and sort headers for linear processing.
    let sorted = timed_stage_attributed!(
        crate::timing::TimingMetric::FrontendOrderDeclarations,
        timing_context,
        sort_headers(compiler, module_headers, &resolved_file_references,)
    )?;

    let root_activity = ModuleRootActivity {
        runtime_fragment_count: sorted.entry_runtime_fragment_count,
    };

    // Project the pre-AST `DirectExportSeed` from the bound, sorted declaration shells
    // and header-built public export metadata. This is the immediate consumer of the
    // per-file source-origin side table: the bindings and the public nominal-type origin
    // index depend only on header shells, so they are projected here before `sorted`
    // moves into AST construction. `DirectExportSeed` is the pre-AST export-identity
    // authority: it carries only header-shell facts and resolves no callable identities.
    // The post-AST `CallableSeed` table, built inside the public-interface draft builder,
    // joins this seed with the resolved receiver-method catalog to own receiver and
    // callable identity, so best-effort header receiver names never mask valid generic
    // receiver methods or preempt AST receiver diagnostics. The draft is retained only
    // on overall semantic success, so a diagnosed module exposes no component.
    // One seed per present facade root. An entry-only module has no facade root, so it projects
    // no export surface and no draft at all rather than fabricating an empty interface.
    let mut facade_seeds = Vec::with_capacity(2);
    for (surface_root_file_id, role) in facade_surface_roots.into_iter().flatten() {
        let seed = build_direct_export_seed(
            &source_module_origins,
            surface_root_file_id,
            &sorted.headers,
            &sorted.module_symbols,
            context.source_provider_dependencies,
            compiler.external_package_registry.as_ref(),
            &compiler.string_table,
            &compiler.path_fork,
        )?;
        let seed_role = seed.module_origin().role();
        if seed_role != role {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "semantic module compilation: facade root file id {} projects a {seed_role:?} origin instead of its inventory role {role:?}",
                    surface_root_file_id.index()
                ),
            )
            .into());
        }
        facade_seeds.push((role, surface_root_file_id, seed));
    }

    // Build the transient expanded public source-nominal origin index before `sorted`
    // moves into AST construction. Each origin is derived from the header's retained
    // SourceId through the per-file SourceModuleOriginTable, so imported project-graph
    // nominals resolve to their defining provider origin. The type-surface projection
    // consumes this index to resolve imported nominal references in this module's public
    // signatures and fields.
    // The index mirrors the AST `source_path_is_public_from_root_file` nameability owner:
    // a nominal is included when a retained module-root or source-package public export
    // entry targets its canonical source path, so a privately-authored nominal exposed
    // through a public alias resolves to its graph-derived module origin. The retained
    // `module_symbols` is borrowed before `sorted` moves into AST construction.
    let public_source_nominal_type_origins = build_public_source_nominal_origin_index(
        &source_module_origins,
        &sorted.headers,
        &sorted.module_symbols,
        &compiler.string_table,
        &compiler.path_fork,
    )?;

    // Build the transient expanded public source-trait origin index before `sorted`
    // moves into AST construction. Analogous to the nominal origin index, this maps
    // each trait header whose canonical declaration path is targeted by a retained
    // public export entry to its stable OriginTraitId. Directly-defined, imported
    // project-graph and public-alias-target traits are included; private and unowned
    // source-package traits are excluded. The type-surface projection consumes this
    // index to resolve source-trait generic bounds to stable trait origin identities.
    let public_source_trait_origins = std::sync::Arc::new(build_public_source_trait_origin_index(
        &source_module_origins,
        &sorted.headers,
        &sorted.module_symbols,
        &compiler.string_table,
        &compiler.path_fork,
    )?);

    // 3. Build the Abstract Syntax Tree (AST).
    // The build result carries the executable `Ast` plus the two closed side results
    // consumed before HIR: the public-interface projection input and the validated
    // donor-local generic-template map. HIR receives the executable `Ast` only. The
    // projection input carries the resolved receiver-method catalog and public type-root
    // table that joins with the pre-AST `DirectExportSeed` to build the post-AST
    // `CallableSeed` table, the one receiver and callable identity owner.
    let module_ast_build = build_ast_with_registered_types(
        context,
        compiler,
        sorted,
        module_anchor_path,
        synthesizes_entry_start,
        capacity_estimate,
        &mut warnings,
        resolved_file_references,
        Some(module_origin.clone()),
        std::sync::Arc::clone(&public_source_trait_origins),
        #[cfg(feature = "timers")]
        timing_context,
    )?;

    // Destructure the build result once: the projection input and generic-template map
    // feed the draft and extraction owners, while only the executable `Ast` reaches HIR.
    let AstBuildResult {
        ast: mut module_ast,
        public_interface_projection_input,
        mut support_public_interface_projection_input,
        module_resources,
        materialisation_context: materialisation_context_builder,
        deferred_generic_requests,
        assertion_validation_generic_requests,
        exact_never_proofs,
    } = module_ast_build;

    let module_resources = module_resources.unwrap_or_else(|| {
        crate::compiler_bug!(
            CompilerStage::ProjectBuildAssembly,
            "module AST finalization did not retain its module resource table"
        )
    });
    let mut materialisation_context_builder =
        materialisation_context_builder.unwrap_or_else(|| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "module AST finalization did not retain its materialisation context"
            )
        });
    // Build the one aggregate public-interface draft before HIR consumes the AST. The draft is
    // the sole pre-HIR public-semantic handoff: the export-origin finalization, the canonical
    // type-surface projection and the corrected trait-requirement projection are internal
    // builder steps. They run from already-resolved facts (the receiver catalog, the resolved
    // public type-root table, the resolved public trait-root vector, the `DirectExportSeed` and
    // the module TypeEnvironment) without a second source scan or HIR scan, and are retained
    // only on overall semantic success. The builder also produces the transient post-AST
    // `CallableSeed` table, the one receiver and callable identity owner consumed by direct
    // projection, declaration-record projection, HIR origin seeding and generic-template extraction.
    // The public-interface draft carries stable declaration identities, not donor-local TypeId,
    // NominalTypeId, GenericParameterId, TraitId, CoreTraitKind or PathId. Folded constant values
    // are owned by the AST module store and projected by value ID.
    //
    // The draft is not the final `PublicSemanticInterface`. Concrete call-summary finalization runs
    // after borrow validation, and provider closure assembles the completed public surface.
    // Export-diagnostic and synthetic-interface provenance are already retained in the draft.
    let module_resource_table = module_resources.borrow();
    // The AST built its main projection input from the preparation anchor. Every other present
    // facade root has its own support projection input. A support facade alone is the anchor, so
    // it consumes the main input rather than duplicating the same projection.
    let mut public_interface_projection_input = Some(public_interface_projection_input);
    let mut surface_specs = Vec::with_capacity(facade_seeds.len());
    for (role, surface_root_file_id, seed) in facade_seeds {
        let projection_input = if surface_root_file_id == preparation_anchor {
            public_interface_projection_input.take().ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    "semantic module compilation: the anchor facade root has no public-interface projection input",
                )
            })?
        } else {
            support_public_interface_projection_input
                .take()
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "semantic module compilation: the {role:?} facade root is not the preparation anchor and has no public-interface projection input"
                        ),
                    )
                })?
        };
        surface_specs.push((role, seed, projection_input));
    }

    let mut surface_builds = Vec::with_capacity(surface_specs.len());
    for (surface_role, seed, projection_input) in surface_specs {
        let build_result = timed_stage_attributed!(
            crate::timing::TimingMetric::FrontendPublicInterfaceProject,
            timing_context,
            PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
                export_seed: seed,
                public_interface_projection_input: projection_input,
                public_source_nominal_type_origins: &public_source_nominal_type_origins,
                public_source_trait_origins: &public_source_trait_origins,
                type_environment: &module_ast.type_environment,
                external_registry: compiler.external_package_registry.as_ref(),
                string_table: &compiler.string_table,
                path_fork: &compiler.path_fork,
                generic_function_templates: materialisation_context_builder
                    .context()
                    .generic_function_templates(),
                const_values: &module_ast.const_values,
                module_resources: Some(&module_resource_table),
            })
            .build(),
        );
        let build = match build_result {
            Ok(result) => result,
            Err(PublicInterfaceDraftBuildFailure::Infrastructure(error)) => {
                return Err(error.into());
            }
            Err(PublicInterfaceDraftBuildFailure::Diagnostic(diagnostic)) => {
                let mut diagnostic_drafts = std::mem::take(&mut warnings);
                diagnostic_drafts.push(diagnostic);
                let batch = match PreparedDiagnosticBatch::capture_with_required_paths(
                    DiagnosticBag::from_diagnostics(diagnostic_drafts),
                    compiler.string_table.take_for_handoff(),
                    Some(&module_ast.type_environment),
                    || Some(Arc::new(compiler.path_fork.shared_table())),
                ) {
                    Ok(batch) => batch,
                    Err(error) => return Err(PreparedFailure::from(error)),
                };
                return Err(PreparedFailure::Diagnosed(batch));
            }
        };
        surface_builds.push((surface_role, build));
    }
    let public_origins_by_path = {
        let mut origins = FxHashMap::default();
        for (_, build) in &surface_builds {
            for seed in &build.callable_seeds {
                if let Some(existing) = origins.insert(seed.path, seed.origin.clone())
                    && existing != seed.origin
                {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "semantic module compilation: callable path {:?} resolves to conflicting public origins across module surfaces",
                            seed.path
                        ),
                    )
                    .into());
                }
            }
        }
        origins
    };

    // Release the shared-table borrow before HIR: lowering re-interns handoff origins with
    // `borrow_mut` against the same module resource table.
    drop(module_resource_table);
    let private_function_origin_seeds = materialisation_context_builder
        .install_concrete_executable_contracts(
            &module_ast.const_values,
            &module_origin,
            &public_origins_by_path,
            &public_source_nominal_type_origins,
            &module_resources.borrow(),
            &compiler.path_fork,
        )?
        .into_iter()
        .map(|(path, origin)| PrivateFunctionOriginSeed { path, origin })
        .collect::<Vec<_>>();

    // Inert assertion witnesses need exact concrete summaries, not executable demand. Install
    // their stable targets beside active contracts so HIR can own those validation edges after
    // AST release. Active occurrences win shared-instance diagnostic sites; convergence drops
    // validation-only sidecars through the final executable reachability barrier.
    let generated_requests = install_generated_request_contracts(
        deferred_generic_requests
            .iter()
            .chain(assertion_validation_generic_requests.iter()),
        materialisation_context_builder.context(),
        materialisation_context_builder
            .context()
            .generic_function_templates(),
        compiler.external_package_registry.as_ref(),
        &compiler.path_fork,
        &mut module_ast,
    )?;
    let generated_request_ids =
        generated_transaction.register_requests(generated_requests.iter().map(|request| {
            GeneratedRequestFacts {
                identity: request.identity.clone(),
                display_name: request
                    .function_name
                    .map(|name| compiler.string_table.resolve(name).to_owned())
                    .unwrap_or_else(|| "<generated>".to_owned()),
                call_span: request.call_span,
            }
        }));
    // The Never proof union starts with this module's exact-target nodes translated through its
    // own registered spellings. Each sidecar adds its nodes under its own domain's spellings, so
    // equal `__generic_instance_<TypeId>` spellings from different domains never merge.
    let mut generated_never_proofs = GeneratedNeverProofs::new(
        exact_never_proofs,
        &generated_requests,
        &generated_transaction,
    );
    // 4b. Extract validated generic-template body artefacts before HIR consumes AST
    //     state. The transient public callable seed table is the exact path-to-origin
    //     authority for every directly exported generic free function or receiver method;
    //     the donor-local template map is the authority for the validated body payload.
    //     The extraction/join owner moves matching templates out of the donor-local map
    //     and keys them by the exact `OriginFunctionId` already retained by the draft.
    //     Private and non-generic templates remain intentional exclusions.
    //     This runs after generic body validation and before HIR so the templates never
    //     re-enter donor AST state.
    for (_, build) in &surface_builds {
        validate_materialisation_context_templates(
            &build.draft,
            &build.callable_seeds,
            materialisation_context_builder.generic_function_templates_mut(),
        )?;
    }
    materialisation_context_builder.finalize_generic_template_identity_index()?;
    // Detach syntax, defaults and module constants while AST value owners are still live.
    // Same-module generated requests and publication consume this single stable capture.
    // Materialisation is module-wide, not surface-local. A support generic body may depend on a
    // support-only public declaration or evidence that the normal draft does not export, so detach
    // the deduplicated union of both validated drafts in surface order.
    let mut materialisation_declarations = Vec::new();
    let mut materialisation_evidence = Vec::new();
    for (_, build) in &surface_builds {
        for declaration in &build.draft.declarations {
            if !materialisation_declarations.iter().any(
                |existing: &crate::compiler_frontend::public_interface::PublicDeclarationRecord| {
                    existing.origin == declaration.origin
                },
            ) {
                materialisation_declarations.push(declaration.clone());
            }
        }
        for evidence in &build.draft.reusable_evidence {
            if !materialisation_evidence.iter().any(
                |existing: &crate::compiler_frontend::public_interface::PublicEvidenceRecord| {
                    existing.identity == evidence.identity
                },
            ) {
                materialisation_evidence.push(evidence.clone());
            }
        }
    }
    let mut materialisation_context_builder = materialisation_context_builder.detach(
        &module_ast.const_values,
        &materialisation_declarations,
        &materialisation_evidence,
        &module_resources.borrow(),
        &compiler.path_fork,
    )?;

    let mut function_origin_seeds = Vec::new();
    let mut function_seed_origins = FxHashMap::<PathId, OriginFunctionId>::default();
    let mut surface_drafts = Vec::with_capacity(surface_builds.len());
    for (role, mut build) in surface_builds.drain(..) {
        for seed in std::mem::take(&mut build.function_origin_seeds) {
            if let Some(existing) = function_seed_origins.get(&seed.path) {
                if existing != &seed.origin {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        format!(
                            "semantic module compilation: declaration path {:?} resolves to conflicting public function origins across module surfaces",
                            seed.path
                        ),
                    )
                    .into());
                }
                continue;
            }
            function_seed_origins.insert(seed.path, seed.origin.clone());
            function_origin_seeds.push(seed);
        }
        surface_drafts.push((role, build.draft));
    }
    let function_origin_lookup = HirFunctionOriginLookup::from_public_and_private_seeds(
        function_origin_seeds,
        private_function_origin_seeds,
    )?;

    let const_top_level_fragments = {
        let module_resources = module_resources.borrow();
        module_ast
            .const_top_level_fragments
            .iter()
            .map(|fragment| {
                owned_folded_string_from_const_string(
                    &fragment.value,
                    &module_resources,
                    &compiler.string_table,
                )
                .map(|value| ResolvedConstFragment {
                    runtime_insertion_index: fragment.runtime_insertion_index,
                    destination: fragment.destination.clone(),
                    span: Some(fragment.span),
                    value,
                })
            })
            .collect::<Result<Vec<_>, InfrastructureFailure>>()?
    };
    // Entry metadata exists exactly when the module has a start-bearing root. An entryless
    // module must not carry a dormant start, an entry fragment schedule or root activity facts.
    let entry_metadata = match entry_point_path {
        Some(entry_point_path) => Some(EntryRootMetadata {
            entry_point: entry_point_path.to_path_buf(),
            const_top_level_fragments,
            root_activity,
        }),
        None => {
            if !const_top_level_fragments.is_empty() {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    "semantic module compilation: an entryless module retained const top-level fragments",
                )
                .into());
            }
            debug_assert_eq!(root_activity, ModuleRootActivity::default());
            None
        }
    };

    let mut warning_bag = DiagnosticBag::from_diagnostics(std::mem::take(&mut warnings));
    // 6. Lower AST to Higher-level Intermediate Representation (HIR).
    let hir_lowering = timed_stage_attributed!(
        crate::timing::TimingMetric::FrontendHir,
        timing_context,
        lower_hir(
            compiler,
            module_ast,
            &mut warning_bag,
            function_origin_lookup,
            Some(Rc::clone(&module_resources)),
            None,
            capacity_estimate,
        )
    )?;
    let HirLoweringResult {
        mut hir_module,
        mut type_environment,
        metadata: lowering_metadata,
    } = hir_lowering;

    // Link facts are the validated-HIR owner for direct call targets. Summary convergence
    // consumes these facts after HIR validation rather than scanning
    // source or introducing a second HIR call graph. Convergence refreshes them only when the
    // private failure lane rewrites the CFG.
    let mut function_link_facts = collect_module_function_link_facts(&hir_module)?;

    #[cfg(feature = "boracle")]
    if request == SemanticStageRequest::Boracle {
        // This analysis-only handoff has no summary-dependent structural producer.
        hir_module.expressions.freeze();
        return Ok(SemanticStageOutput::Boracle(Box::new(BoracleModuleInput {
            hir: hir_module,
            external_package_registry: Arc::clone(compiler.external_package_registry),
            source_path: module_anchor_path.to_path_buf(),
        })));
    }

    // 8. Run static analysis (Borrow Checker).
    increment_frontend_counter(FrontendCounter::ConvergenceInitialBaseBorrowPasses);
    let bootstrap_borrow_analysis = timed_stage_attributed!(
        crate::timing::TimingMetric::FrontendBorrowInitial,
        timing_context,
        check_borrows(
            compiler,
            &hir_module,
            &type_environment,
            &mut warning_bag,
            None
        )
    )?;
    // Exact-target Never proof nodes accumulate in materialisation order: this module's own
    // declarations first, then every generated sidecar's. The declaring module's template graph
    // cannot see a cycle that closes through a sidecar's trait-bound call, so the domain-keyed
    // union is checked once materialisation finishes.
    let generated_materialisation = timed_stage_attributed!(
        crate::timing::TimingMetric::FrontendGeneratedMaterialise,
        timing_context,
        materialise_generated_request_roots(
            context,
            &generated_request_ids,
            &mut generated_transaction,
            materialisation_context_builder.context(),
            &mut generated_never_proofs,
            compiler,
            #[cfg(feature = "timers")]
            timing_context,
        )
    );
    generated_materialisation?;
    if let Some((first_cyclic, diagnostic)) = generated_never_proofs.cycle() {
        // Requester nodes precede sidecar nodes, so a requester index means this module's own
        // source declaration lies on the cycle and owns the report. An instance-only cycle needs
        // instance-to-instance edges, which are nested requests, so depth-first materialisation
        // has already rejected it as recursive generic instantiation.
        if first_cyclic >= generated_never_proofs.requester_count() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                "generated Never proof cycle has no requester declaration; recursive generic \
                 instantiation should have rejected it during materialisation",
            )
            .into());
        }
        let batch = PreparedDiagnosticBatch::capture_with_required_paths(
            DiagnosticBag::from(diagnostic),
            compiler.string_table.take_for_handoff(),
            None,
            || Some(Arc::new(compiler.path_fork.shared_table())),
        )
        .map_err(PreparedFailure::from)?;
        return Err(PreparedFailure::Diagnosed(batch));
    }
    let borrow_analysis = run_generated_summary_convergence(
        context,
        compiler,
        &mut hir_module,
        &mut function_link_facts,
        &mut generated_transaction,
        bootstrap_borrow_analysis,
        &mut type_environment,
        &mut warning_bag,
        #[cfg(feature = "timers")]
        timing_context,
    )?;
    // Private failure lanes and final link facts are complete. Publication may still
    // rebase interned scalar identities, but expression topology is fixed.
    hir_module.expressions.freeze();
    // Reinstall after convergence: the fixed point may have widened a callee summary the
    // pre-materialisation pass recorded, and the frozen context must carry the final one.
    install_exact_concrete_call_summaries(
        &mut materialisation_context_builder,
        &hir_module,
        &borrow_analysis,
    )?;
    let (generated_delta, sidecar_warnings) = generated_transaction.finish();
    record_borrow_counters(&borrow_analysis);

    // Concrete call-summary finalization runs exactly once after HIR and borrow
    // validation produce the stable local-function relationship and complete call
    // summaries. The post-AST `CallableSeed` table owns the receiver and callable
    // identity that this finalization joins; only concrete-local callables receive a
    // summary record here. Generic templates remain declaration contracts whose
    // generated summaries belong to the sidecar delta this transaction completes later,
    // distinct from these direct concrete summaries. Private functions and the entry retain local
    // summaries but never enter declaration records.
    let mut public_interfaces = Vec::with_capacity(surface_drafts.len());
    let all_public_function_origins = public_origins_by_path
        .values()
        .cloned()
        .collect::<rustc_hash::FxHashSet<_>>();
    for (role, draft) in surface_drafts {
        let interface = timed_stage_attributed!(
            crate::timing::TimingMetric::FrontendPublicInterfaceFinalise,
            timing_context,
            {
                let local_public_interface = draft
                    .finalize_after_borrow_validation_with_allowed_origins(
                        &borrow_analysis.analysis,
                        &hir_module,
                        &all_public_function_origins,
                    )?;
                PublicSemanticInterface::close_from_local(
                    local_public_interface,
                    context.source_provider_dependencies,
                    compiler.external_package_registry.as_ref(),
                )
            }
        )?;
        public_interfaces.push((role, interface));
    }
    let materialisation_context = {
        let public_interface_refs = public_interfaces
            .iter()
            .map(|(_, interface)| interface)
            .collect::<Vec<_>>();
        materialisation_context_builder
            .freeze(&public_interface_refs, &public_origins_by_path)?
            .map(Arc::new)
    };
    // Each present facade root contributes exactly one closed surface. An entry-only module
    // contributes none, so both options stay absent rather than borrowing another root's surface.
    let mut normal_interface = None;
    let mut support_interface = None;
    for (role, interface) in public_interfaces {
        match role {
            ModuleRootRole::Normal => normal_interface = Some(interface),
            ModuleRootRole::Support => support_interface = Some(interface),
            ModuleRootRole::Entry
            | ModuleRootRole::Shared
            | ModuleRootRole::SyntheticProjectContext => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    format!(
                        "semantic module compilation: a facade draft carries the non-facade role {role:?}"
                    ),
                )
                .into());
            }
        }
    }
    let resource_table = Rc::try_unwrap(module_resources)
        .map(|cell| cell.into_inner())
        .unwrap_or_else(|_| {
            crate::compiler_bug!(
                CompilerStage::ProjectBuildAssembly,
                "ordinary module resource table still has a live shared handle after HIR lowering"
            )
        });
    let resource_source_associations = resource_table.resource_source_associations().to_vec();

    // -------------------------
    //  Finalize Module Build
    // -------------------------

    borrow_log!("=== BORROW CHECKER OUTPUT ===");
    borrow_log!(format!(
        "Borrow checking completed successfully (states={} functions={} blocks={} conflicts_checked={} stmt_facts={} term_facts={} value_facts={})",
        borrow_analysis.analysis.total_state_snapshots(),
        borrow_analysis.stats.functions_analyzed,
        borrow_analysis.stats.blocks_analyzed,
        borrow_analysis.stats.conflicts_checked,
        borrow_analysis.analysis.statement_facts.len(),
        borrow_analysis.analysis.terminator_facts.len(),
        borrow_analysis.analysis.value_facts.len()
    ));
    borrow_log!("=== END BORROW CHECKER OUTPUT ===");

    // Final call facts include external bindings reached through source facades. Candidates
    // remain conservative availability; entry assembly still selects exact executable demand.
    let external_import_candidates = collect_external_import_candidates(
        &source_logical_paths,
        &function_link_facts,
        compiler.external_package_registry.as_ref(),
        external_dependency_resolution_table,
        context.builder_runtime_packages,
    )?;

    let numeric_proofs = analyse_numeric_proofs(&hir_module, &type_environment);

    Ok(SemanticStageOutput::Complete(
        Box::new(CompleteSemanticStage {
            module: Module {
                executable: ModuleExecutable {
                    hir: hir_module,
                    resource_table,
                    type_environment,
                    borrow_analysis,
                    numeric_proofs,
                    path_table: Arc::new(
                        crate::compiler_frontend::symbols::path_interner::PathInternerBuilder::new(
                        )
                        .freeze(),
                    ),
                },
                link_facts: ModuleLinkFacts {
                    external_package_registry: Arc::clone(compiler.external_package_registry),
                    external_import_candidates,
                    functions: function_link_facts,
                },
                metadata: ModuleCompilerMetadata::from_hir_lowering(
                    entry_metadata,
                    lowering_metadata,
                    materialisation_context,
                ),
            },
            warnings: warning_bag,
            normal_interface,
            support_interface,
            generated_delta,
            resource_source_associations,
        }),
        sidecar_warnings,
    ))
}

/// Bind retained `PreparedHeaderSyntax` against provider interfaces.
///
/// WHAT: resolves public exports, builds the binding environment, canonicalizes dependency
///       edges, and completes constant initializer dependencies. Consumes only the retained
///       syntax carried in from preparation — it never retokenizes or reparses source.
/// WHY: these facts depend on provider interfaces and the project path resolver, so they
///      belong in the semantic phase after preparation has produced `PreparedHeaderSyntax`.
fn bind_retained_headers(
    compiler: &mut CompilerFrontend<'_>,
    prepared_header_syntax: PreparedHeaderSyntax,
    external_dependency_resolution_table: &ExternalImportResolutionTable,
    source_provider_dependencies: &SourceProviderDependencySet<'_>,
) -> Result<BoundModuleHeaders, PreparedFailure> {
    let headers = bind_module_headers(
        prepared_header_syntax,
        compiler.external_package_registry.as_ref(),
        external_dependency_resolution_table,
        source_provider_dependencies,
        compiler.project_path_resolver,
        compiler.source_files.as_ref(),
        &mut compiler.string_table,
        &mut compiler.path_fork,
    )
    .map_err(|failure| match failure {
        HeaderPreparationFailure::Diagnosed(bag) => {
            let string_table = compiler.string_table.take_for_handoff();
            let path_fork = &compiler.path_fork;
            match PreparedDiagnosticBatch::capture_with_required_paths(
                bag,
                string_table,
                None,
                || Some(Arc::new(path_fork.shared_table())),
            ) {
                Ok(batch) => PreparedFailure::Diagnosed(batch),
                Err(error) => PreparedFailure::from(error),
            }
        }
        HeaderPreparationFailure::Infrastructure(error) => PreparedFailure::Infrastructure(error),
    })?;

    record_header_counters(&headers);
    Ok(headers)
}

fn sort_headers(
    compiler: &mut CompilerFrontend<'_>,
    module_headers: BoundModuleHeaders,
    resolved_file_references: &ResolvedFileReferenceTable,
) -> Result<SortedHeaders, PreparedFailure> {
    compiler.sort_headers(module_headers, resolved_file_references)
}

// The timing context is a cfg-gated parameter that disappears from
// no-timer builds; bundling it would add a context struct for one field.
#[allow(
    clippy::too_many_arguments,
    reason = "AST construction keeps module context, mutable frontend and warnings, owned sorted headers and resolved references, module anchor, entry-start fact, arena estimate, module origin and the timers-only context separate"
)]
fn build_ast_with_registered_types(
    context: &ModuleCompilationContext<'_>,
    compiler: &mut CompilerFrontend<'_>,
    sorted: SortedHeaders,
    module_anchor_path: &Path,
    synthesizes_entry_start: bool,
    capacity_estimate: FrontendArenaCapacityEstimate,
    warnings: &mut Vec<DiagnosticDraft>,
    resolved_file_references: ResolvedFileReferenceTable,
    module_origin: Option<StableModuleOriginIdentity>,
    public_source_trait_origins: std::sync::Arc<
        rustc_hash::FxHashMap<
            crate::compiler_frontend::symbols::path_interner::PathId,
            crate::compiler_frontend::semantic_identity::OriginTraitId,
        >,
    >,
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> Result<AstBuildResult, PreparedFailure> {
    match compiler.headers_to_ast(
        AstBuildRequest {
            sorted,
            module_anchor_path,
            synthesizes_entry_start,
            build_profile: context.build_profile,
            capacity_estimate,
            resolved_file_references,
            module_origin,
            public_source_trait_origins,
            build_config_values: Arc::clone(&context.build_config_values),
            builder_root_slots: context.builder_root_slots,
            provider_materialisations: Some(context.provider_materialisations),
        },
        #[cfg(feature = "timers")]
        timing_context,
    ) {
        Ok(mut build_result) => {
            warnings.extend(std::mem::take(&mut build_result.ast.warnings));
            Ok(build_result)
        }
        Err(failure) => Err(failure),
    }
}

fn record_header_counters(headers: &BoundModuleHeaders) {
    add_frontend_counter(FrontendCounter::HeaderCount, headers.headers.len());

    let top_level_declaration_count = headers
        .headers
        .iter()
        .filter(|header| {
            matches!(
                header.kind,
                HeaderKind::Function { .. }
                    | HeaderKind::Constant { .. }
                    | HeaderKind::Struct { .. }
                    | HeaderKind::Choice { .. }
                    | HeaderKind::TypeAlias { .. }
            )
        })
        .count();
    add_frontend_counter(
        FrontendCounter::TopLevelDeclarationCount,
        top_level_declaration_count,
    );
}

fn record_frontend_capacity_estimate(
    source_file_count: usize,
    source_byte_count: usize,
    headers: &BoundModuleHeaders,
) -> FrontendArenaCapacityEstimate {
    let const_fragment_count = headers.const_fragment_count;
    let capacity = FrontendArenaCapacityEstimate::new(
        source_file_count,
        source_byte_count,
        headers.token_stats,
        headers.header_stats,
        const_fragment_count,
        headers.entry_runtime_fragment_count,
    );

    // Scope-frame estimates are recorded here; actual frame allocations and arena capacity growth
    // are recorded by the scope arena owner. This site records only the policy estimate.
    add_frontend_counter(FrontendCounter::EstimatedScopeFrames, capacity.scope_frames);
    add_frontend_counter(
        FrontendCounter::CappedCapacityEstimates,
        capacity.capped_field_count,
    );

    capacity
}

fn record_borrow_counters(report: &BorrowCheckReport) {
    add_frontend_counter(
        FrontendCounter::BorrowFunctionCount,
        report.stats.functions_analyzed,
    );
    add_frontend_counter(
        FrontendCounter::BorrowBlockCount,
        report.stats.blocks_analyzed,
    );
    add_frontend_counter(
        FrontendCounter::BorrowConflictCheckCount,
        report.stats.conflicts_checked,
    );

    let state_snapshot_count = {
        #[cfg(any(test, feature = "show_borrow_checker"))]
        {
            report.analysis.block_entry_states.len()
                + report.analysis.block_exit_states.len()
                + report.analysis.statement_entry_states.len()
        }
        #[cfg(not(any(test, feature = "show_borrow_checker")))]
        {
            0
        }
    };
    add_frontend_counter(
        FrontendCounter::BorrowStateSnapshotCount,
        state_snapshot_count,
    );
    add_frontend_counter(
        FrontendCounter::BorrowStatementVisitCount,
        report.stats.statements_analyzed,
    );
    add_frontend_counter(
        FrontendCounter::BorrowTerminatorVisitCount,
        report.stats.terminators_analyzed,
    );
    add_frontend_counter(
        FrontendCounter::BorrowWorklistIterationCount,
        report.stats.worklist_iterations,
    );
    add_frontend_counter(
        FrontendCounter::BorrowStateJoinCount,
        report.stats.state_joins,
    );

    add_frontend_counter(
        FrontendCounter::BorrowStatementFactCount,
        report.analysis.statement_facts.len(),
    );
    add_frontend_counter(
        FrontendCounter::BorrowTerminatorFactCount,
        report.analysis.terminator_facts.len(),
    );
    add_frontend_counter(
        FrontendCounter::BorrowValueFactCount,
        report.analysis.value_facts.len(),
    );
}
