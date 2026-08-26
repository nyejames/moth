//! The canonical module compilation service.
//!
//! WHAT: one entry point that takes a ready module — prepared source plus completed provider
//!       interfaces — and returns a complete semantic result, a diagnosed result or a
//!       `CompilerError`.
//! WHY: the compiler owns local semantic compilation. Sequencing interface binding, declaration
//!      ordering, AST semantics, public-interface projection, HIR lowering and validation, borrow
//!      validation and generated completion is compiler work, so it lives behind one call rather
//!      than being assembled by the build system. Stage 0 decides when a module is ready and what
//!      happens to the result; it never invokes these stages individually.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::CompilerFrontend;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::arena::FrontendArenaCapacityEstimate;
use crate::compiler_frontend::ast::AstBuildResult;
use crate::compiler_frontend::compiler_errors::{
    CompilerError, CompilerMessages, merge_stage_messages,
};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, ModuleDiagnostics};
#[cfg(feature = "boracle")]
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::parse_file_headers::{
    BoundModuleHeaders, HeaderKind, PreparedHeaderSyntax, bind_module_headers,
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
    Module, ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts, ModuleRootActivity,
    ResolvedConstFragment,
};
use crate::compiler_frontend::module_compilation::context::ModuleCompilationContext;
use crate::compiler_frontend::module_compilation::external_imports::collect_external_import_candidates_for_source_files;
use crate::compiler_frontend::module_compilation::generated::artefacts::GeneratedFunctionDelta;
use crate::compiler_frontend::module_compilation::generated::convergence::{
    install_exact_concrete_call_summaries, run_generated_summary_convergence,
};
use crate::compiler_frontend::module_compilation::generated::known::KnownGeneratedFunctions;
use crate::compiler_frontend::module_compilation::generated::materialisation::materialise_generated_request_roots;
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
use crate::compiler_frontend::public_interface::{
    PublicInterfaceDraftBuilder, PublicInterfaceDraftBuilderInput, PublicSemanticInterface,
    SourceProviderDependencySet, build_direct_export_seed,
    build_public_source_nominal_origin_index, build_public_source_trait_origin_index,
};
use crate::compiler_frontend::semantic_identity::{ModuleRootRole, StableModuleOriginIdentity};
use crate::compiler_frontend::source_module_origin::SourceModuleOriginTable;
use crate::compiler_frontend::symbols::identity::{FileId, SourceFileTable};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::validated_generic_template_metadata::validate_materialisation_context_templates;
use crate::{borrow_log, timed_stage_attributed};

use rustc_hash::FxHashMap;
use std::path::Path;
#[cfg(feature = "boracle")]
use std::path::PathBuf;
use std::sync::Arc;

/// Validated-HIR payload returned by the internal Boracle compiler service.
#[cfg(feature = "boracle")]
pub(crate) struct BoracleModuleInput {
    pub(crate) hir: HirModule,
    pub(crate) external_package_registry: Arc<ExternalPackageRegistry>,
    pub(crate) entry_point: PathBuf,
}

/// Compile one ready module through the canonical local semantic sequence.
///
/// WHAT: binds retained header syntax against completed provider interfaces, orders local
///       declarations, runs AST semantics, projects the public interface, lowers and validates HIR,
///       collects link facts, borrow-validates, completes generated semantic work and closes the
///       public interface. It receives no source text or tokens and cannot rerun file preparation.
///       The active module origin and entry file are resolved from the retained active root
///       `FileId`, never reconstructed from source paths.
/// WHY: this is the one production owner of the binding -> ordering -> AST -> HIR -> borrow
///      sequence. Everything the sequence needs arrives as immutable input, and everything it
///      produces leaves as one typed outcome, so the build system can schedule it without knowing
///      how a module is compiled.
pub(crate) fn compile_module(
    context: &ModuleCompilationContext<'_>,
    prepared: PreparedModuleInput,
    known_generated: KnownGeneratedFunctions<'_>,
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> Result<ModuleCompilationOutcome, CompilerError> {
    // The entry file is a retained preparation identity, so it is resolved from the payload
    // rather than repeated as an argument the caller must keep in sync.
    let entry_file_path = prepared.entry_file_path()?.to_path_buf();
    let entry_file_path = entry_file_path.as_path();

    let PreparedModuleInput {
        active_root_file_id,
        source_module_origins,
        prepared_header_syntax,
        string_table,
        source_files,
        warnings,
        source_file_count,
        source_byte_count,
    } = prepared;

    // The active module origin is resolved from the per-file source-origin table using the
    // retained active root FileId, not from a loose origin argument. Preparation already
    // validated the active root's table origin against the expected active origin, so the
    // semantic projection re-derives the same origin from the table and validates every
    // directly-defined public header against it.
    let active_module_origin = source_module_origins
        .origin_for(active_root_file_id)?
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "semantic module compilation: active root file id {} has no module origin",
                active_root_file_id.0
            ))
        })?
        .clone();

    let mut compiler = CompilerFrontend::new(
        context.options.clone(),
        string_table,
        context.style_directives.to_owned(),
        Arc::clone(&context.external_packages),
        context.project_path_resolver.clone(),
    );
    compiler.set_source_files(source_files);

    let compile_result = run_semantic_stages(
        &mut compiler,
        context,
        known_generated,
        warnings,
        SemanticStageInputs {
            prepared_header_syntax,
            source_module_origins,
            active_root_file_id,
            active_module_origin,
            entry_file_path,
            source_file_count,
            source_byte_count,
        },
        SemanticStageRequest::Complete,
        #[cfg(feature = "timers")]
        timing_context,
    );

    // Normalize the deeper stages' mixed `CompilerMessages` once at this semantic boundary.
    // A successful compilation becomes `Success`. A failing stage becomes either
    // `Diagnosed` (user-facing diagnostics the renderer surfaces) or `Err(CompilerError)`
    // (an infrastructure failure recovered losslessly from its structured payload). This is
    // the single lossless ownership transfer; graph and render consumers never re-classify.
    match compile_result {
        Ok(SemanticStageOutput::Complete(output)) => {
            let CompleteSemanticStage {
                module,
                public_interface,
                generated_delta,
            } = *output;
            let string_table = compiler.string_table;
            Ok(ModuleCompilationOutcome::Success(Box::new(
                ModuleSemanticResult {
                    module,
                    generated_delta,
                    string_table,
                    public_interface,
                },
            )))
        }
        #[cfg(feature = "boracle")]
        Ok(SemanticStageOutput::Boracle(_)) => Err(CompilerError::compiler_error(
            "normal module compilation unexpectedly stopped at the Boracle prefix",
        )),
        Err(messages) => {
            // The failing stage already cloned the live `compiler.string_table` into the
            // messages, so the diagnosed payload carries every render identity produced so
            // far. `compiler` itself is no longer needed.
            match ModuleDiagnostics::from_messages(messages) {
                Ok(diagnostics) => Ok(ModuleCompilationOutcome::Diagnosed(diagnostics)),
                Err(error) => Err(error),
            }
        }
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
    known_generated: KnownGeneratedFunctions<'_>,
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> Result<BoracleModuleInput, CompilerMessages> {
    let entry_file_path = prepared
        .entry_file_path()
        .map_err(|error| CompilerMessages::from_error(error, StringTable::new()))?
        .to_path_buf();
    let entry_file_path = entry_file_path.as_path();

    let PreparedModuleInput {
        active_root_file_id,
        source_module_origins,
        prepared_header_syntax,
        string_table,
        source_files,
        warnings,
        source_file_count,
        source_byte_count,
    } = prepared;
    let active_module_origin = source_module_origins
        .origin_for(active_root_file_id)
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?
        .ok_or_else(|| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(format!(
                    "semantic Boracle compilation: active root file id {} has no module origin",
                    active_root_file_id.0
                )),
                &string_table,
            )
        })?
        .clone();

    let mut compiler = CompilerFrontend::new(
        context.options.clone(),
        string_table,
        context.style_directives.to_owned(),
        Arc::clone(&context.external_packages),
        context.project_path_resolver.clone(),
    );
    compiler.set_source_files(source_files);

    match run_semantic_stages(
        &mut compiler,
        context,
        known_generated,
        warnings,
        SemanticStageInputs {
            prepared_header_syntax,
            source_module_origins,
            active_root_file_id,
            active_module_origin,
            entry_file_path,
            source_file_count,
            source_byte_count,
        },
        SemanticStageRequest::Boracle,
        #[cfg(feature = "timers")]
        timing_context,
    ) {
        Ok(SemanticStageOutput::Boracle(input)) => Ok(*input),
        Ok(SemanticStageOutput::Complete(_)) => Err(CompilerMessages::from_error_ref(
            CompilerError::compiler_error(
                "Boracle compiler service unexpectedly completed the normal semantic pipeline",
            ),
            &compiler.string_table,
        )),
        Err(messages) => Err(messages),
    }
}

/// Everything one semantic stage run reads from the prepared payload.
///
/// WHAT: the retained syntax and module-local identity facts [`run_semantic_stages`] consumes,
///       after the string table and source-file table have moved into the `CompilerFrontend`.
/// WHY: seven values from one prepared module travel together through the whole stage sequence.
///      Naming the bundle keeps `compile_module` readable as setup, one stage run and one
///      classification, instead of a parameter list that hides which value came from where.
struct SemanticStageInputs<'a> {
    prepared_header_syntax: PreparedHeaderSyntax,
    source_module_origins: SourceModuleOriginTable,
    active_root_file_id: FileId,
    active_module_origin: StableModuleOriginIdentity,
    entry_file_path: &'a Path,
    source_file_count: usize,
    source_byte_count: usize,
}

enum SemanticStageOutput {
    Complete(Box<CompleteSemanticStage>),
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
    public_interface: PublicSemanticInterface,
    generated_delta: GeneratedFunctionDelta,
}

/// Run the local semantic sequence for one module, from bound headers to a complete result.
///
/// WHAT: binding -> ordering -> AST -> public-interface projection -> HIR -> borrow validation ->
///       generated completion, returning the three values a success is made of.
/// WHY: every stage in here fails with `CompilerMessages`, which mixes user diagnostics and
///      infrastructure failures. Collecting those failures at one `?` boundary lets
///      [`compile_module`] classify them exactly once. Keeping the sequence in its own function
///      means `compile_module` reads as three steps rather than wrapping four hundred lines.
fn run_semantic_stages(
    compiler: &mut CompilerFrontend,
    context: &ModuleCompilationContext<'_>,
    known_generated: KnownGeneratedFunctions<'_>,
    mut warnings: Vec<CompilerDiagnostic>,
    inputs: SemanticStageInputs<'_>,
    request: SemanticStageRequest,
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> Result<SemanticStageOutput, CompilerMessages> {
    #[cfg(not(feature = "boracle"))]
    let _ = request;

    let SemanticStageInputs {
        prepared_header_syntax,
        source_module_origins,
        active_root_file_id,
        active_module_origin,
        entry_file_path,
        source_file_count,
        source_byte_count,
    } = inputs;
    let mut generated_transaction = GeneratedFunctionTransaction::new(known_generated);
    let active_root_role = active_module_origin.role();
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
                &warnings,
            )
        }
    )?;

    let capacity_estimate =
        record_frontend_capacity_estimate(source_file_count, source_byte_count, &module_headers);

    // 2. Resolve dependencies and sort headers for linear processing.
    let sorted = timed_stage_attributed!(
        crate::timing::TimingMetric::FrontendOrderDeclarations,
        timing_context,
        sort_headers(compiler, module_headers, &warnings)
    )?;

    let root_activity = ModuleRootActivity {
        has_non_trivial_root_body: sorted.has_non_trivial_root_body,
        const_fragment_count: sorted.const_fragment_count,
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
    let export_seed = build_direct_export_seed(
        &source_module_origins,
        active_root_file_id,
        &sorted.headers,
        &sorted.module_symbols,
        context.source_provider_dependencies,
        compiler.external_package_registry.as_ref(),
        &compiler.string_table,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

    // Build the transient expanded public source-nominal origin index before `sorted`
    // moves into AST construction. Each origin is derived from the header's retained
    // FileId through the per-file SourceModuleOriginTable, so imported project-graph
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
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

    // Build the transient expanded public source-trait origin index before `sorted`
    // moves into AST construction. Analogous to the nominal origin index, this maps
    // each trait header whose canonical declaration path is targeted by a retained
    // public export entry to its stable OriginTraitId. Directly-defined, imported
    // project-graph and public-alias-target traits are included; private and unowned
    // source-package traits are excluded. The type-surface projection consumes this
    // index to resolve source-trait generic bounds to stable trait origin identities.
    let public_source_trait_origins = build_public_source_trait_origin_index(
        &source_module_origins,
        &sorted.headers,
        &sorted.module_symbols,
        &compiler.string_table,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

    // 3. Build the Abstract Syntax Tree (AST).
    // The build result carries the executable `Ast` plus the two closed side results
    // consumed before HIR: the public-interface projection input and the validated
    // donor-local generic-template map. HIR receives the executable `Ast` only. The
    // projection input carries the resolved receiver-method catalog and public type-root
    // table that step 4 joins with the pre-AST `DirectExportSeed` to build the post-AST
    // `CallableSeed` table, the one receiver and callable identity owner.
    let module_ast_build = build_ast_with_registered_types(
        context,
        compiler,
        sorted,
        entry_file_path,
        active_root_role,
        capacity_estimate,
        &mut warnings,
        #[cfg(feature = "timers")]
        timing_context,
    )?;

    // Destructure the build result once: the projection input and generic-template map
    // feed the draft and extraction owners, while only the executable `Ast` reaches HIR.
    let AstBuildResult {
        ast: mut module_ast,
        public_interface_projection_input,
        materialisation_context: mut materialisation_context_builder,
        deferred_generic_requests,
    } = module_ast_build;

    // 4. Build the one aggregate public-interface draft before HIR consumes the AST. The
    //    draft is the sole pre-HIR public-semantic handoff: the export-origin
    //    finalization, the canonical type-surface projection and the corrected
    //    trait-requirement projection are internal builder steps. They run from
    //    already-resolved facts (the receiver catalog, the resolved public type-root
    //    table, the resolved public trait-root vector, the `DirectExportSeed` and the
    //    module TypeEnvironment) without a second source scan or HIR scan, and are
    //    retained only on overall semantic success. The builder also produces the
    //    transient post-AST `CallableSeed` table, the one receiver and callable
    //    identity owner consumed by direct projection, declaration-record projection,
    //    HIR origin seeding and generic-template extraction. No donor-local TypeId,
    //    NominalTypeId, GenericParameterId, TraitId, CoreTraitKind or InternedPath
    //    crosses the module result boundary. It is not the final
    //    PublicSemanticInterface: reusable evidence is now an internal builder step
    //    and draft collection, generic template body extraction is already completed in
    //    step 4b, and concrete call-summary finalization is already completed after
    //    borrow validation, while provenance, re-export interfaces, cross-module call
    //    lowering and future generated-generic summaries remain for later phases.
    //    Folded constant values are owned by the AST module store and projected by value ID.
    let public_interface_build = timed_stage_attributed!(
        crate::timing::TimingMetric::FrontendPublicInterfaceProject,
        timing_context,
        PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
            export_seed,
            public_interface_projection_input,
            public_source_nominal_type_origins: &public_source_nominal_type_origins,
            public_source_trait_origins: &public_source_trait_origins,
            type_environment: &module_ast.type_environment,
            external_registry: compiler.external_package_registry.as_ref(),
            string_table: &compiler.string_table,
            generic_function_templates: materialisation_context_builder
                .context()
                .generic_function_templates(),
            const_values: &module_ast.const_values,
        })
        .build(),
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
    let public_interface_draft = public_interface_build.draft;

    let public_origins_by_path = public_interface_build
        .callable_seeds
        .iter()
        .map(|seed| (seed.path.clone(), seed.origin.clone()))
        .collect::<FxHashMap<_, _>>();
    let private_function_origin_seeds = materialisation_context_builder
        .install_concrete_executable_contracts(
            &active_module_origin,
            &public_origins_by_path,
            &public_source_nominal_type_origins,
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?
        .into_iter()
        .map(|(path, origin)| PrivateFunctionOriginSeed { path, origin })
        .collect::<Vec<_>>();

    let generated_requests = install_generated_request_contracts(
        &deferred_generic_requests,
        materialisation_context_builder.context(),
        materialisation_context_builder
            .context()
            .generic_function_templates(),
        compiler.external_package_registry.as_ref(),
        &mut module_ast,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
    let generated_request_ids =
        generated_transaction.register_requests(generated_requests.iter().map(|request| {
            GeneratedRequestFacts {
                identity: request.identity.clone(),
                display_name: request
                    .function_name
                    .map(|name| compiler.string_table.resolve(name).to_owned())
                    .unwrap_or_else(|| "<generated>".to_owned()),
                diagnostic_location: request.call_location.clone(),
            }
        }));
    // 4b. Extract validated generic-template body artefacts before HIR consumes AST
    //     state. The transient public callable seed table is the exact path-to-origin
    //     authority for every directly exported generic free function or receiver method;
    //     the donor-local template map is the authority for the validated body payload.
    //     The extraction/join owner moves matching templates out of the donor-local map
    //     and keys them by the exact `OriginFunctionId` already retained by the draft.
    //     Private and non-generic templates remain intentional exclusions.
    //     This runs after generic body validation and before HIR so the templates never
    //     re-enter donor AST state.
    validate_materialisation_context_templates(
        &public_interface_draft,
        &public_interface_build.callable_seeds,
        materialisation_context_builder.generic_function_templates_mut(),
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
    materialisation_context_builder
        .finalize_generic_template_identity_index()
        .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

    let function_origin_lookup = HirFunctionOriginLookup::from_public_and_private_seeds(
        public_interface_build.function_origin_seeds,
        private_function_origin_seeds,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

    // 5. Resolve const fragment StringIds to strings before AST is consumed by HIR.
    let const_top_level_fragments = module_ast
        .const_top_level_fragments
        .iter()
        .map(|fragment| ResolvedConstFragment {
            runtime_insertion_index: fragment.runtime_insertion_index,
            rendered_text: compiler.string_table.resolve(fragment.value).to_owned(),
        })
        .collect::<Vec<_>>();

    // 6. Lower AST to Higher-level Intermediate Representation (HIR).
    let hir_lowering = timed_stage_attributed!(
        crate::timing::TimingMetric::FrontendHir,
        timing_context,
        lower_hir(compiler, module_ast, &warnings, function_origin_lookup)
    )?;
    let HirLoweringResult {
        mut hir_module,
        type_environment,
        metadata: lowering_metadata,
    } = hir_lowering;

    // 7. Validate extracted non-HIR compiler metadata before a successful module is
    // returned. Invalid compiler metadata is an internal CompilerError.
    if let Err(error) = lowering_metadata.validate() {
        return Err(CompilerMessages::from_error_ref(
            error,
            &compiler.string_table,
        ));
    }

    // Link facts are the validated-HIR owner for direct call targets. The convergence
    // observation model consumes these facts after HIR validation rather than scanning
    // source or introducing a second HIR call graph.
    let function_link_facts = collect_module_function_link_facts(&hir_module)
        .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;

    #[cfg(feature = "boracle")]
    #[cfg(feature = "boracle")]
    if request == SemanticStageRequest::Boracle {
        return Ok(SemanticStageOutput::Boracle(Box::new(BoracleModuleInput {
            hir: hir_module,
            external_package_registry: Arc::clone(&compiler.external_package_registry),
            entry_point: entry_file_path.to_path_buf(),
        })));
    }

    // 8. Run static analysis (Borrow Checker).
    increment_frontend_counter(FrontendCounter::ConvergenceInitialBaseBorrowPasses);
    let bootstrap_borrow_analysis = timed_stage_attributed!(
        crate::timing::TimingMetric::FrontendBorrowInitial,
        timing_context,
        check_borrows(compiler, &hir_module, &warnings)
    )?;
    install_exact_concrete_call_summaries(
        &mut materialisation_context_builder,
        &hir_module,
        &bootstrap_borrow_analysis,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
    timed_stage_attributed!(
        crate::timing::TimingMetric::FrontendGeneratedMaterialise,
        timing_context,
        materialise_generated_request_roots(
            context,
            &generated_request_ids,
            &mut generated_transaction,
            materialisation_context_builder.context(),
            compiler,
            entry_file_path,
            #[cfg(feature = "timers")]
            timing_context,
        )
    )?;
    let borrow_analysis = run_generated_summary_convergence(
        compiler,
        &mut hir_module,
        &function_link_facts,
        &mut generated_transaction,
        bootstrap_borrow_analysis,
        &warnings,
        #[cfg(feature = "timers")]
        timing_context,
    )?;
    // Reinstall after convergence: the fixed point may have widened a callee summary the
    // pre-materialisation pass recorded, and the frozen context must carry the final one.
    install_exact_concrete_call_summaries(
        &mut materialisation_context_builder,
        &hir_module,
        &borrow_analysis,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
    let generated_delta = generated_transaction
        .finish()
        .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
    record_borrow_counters(&borrow_analysis);

    // Concrete call-summary finalization runs exactly once after HIR and borrow
    // validation produce the stable local-function relationship and complete call
    // summaries. The post-AST `CallableSeed` table owns the receiver and callable
    // identity that this finalization joins; only concrete-local callables receive a
    // summary record here. Generic templates remain declaration contracts whose
    // generated summaries belong to the sidecar delta this transaction completes later,
    // distinct from these direct concrete summaries. Private functions and implicit start retain local
    // summaries but never enter declaration records.
    let public_interface = timed_stage_attributed!(
        crate::timing::TimingMetric::FrontendPublicInterfaceFinalise,
        timing_context,
        {
            let local_public_interface = public_interface_draft
                .finalize_after_borrow_validation(&borrow_analysis.analysis, &hir_module)
                .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
            PublicSemanticInterface::close_from_local(
                local_public_interface,
                context.source_provider_dependencies,
                compiler.external_package_registry.as_ref(),
            )
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))
        }
    )?;
    let materialisation_context = materialisation_context_builder
        .freeze(&public_interface)
        .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?
        .map(Arc::new);

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

    // Collect provider-resolved imports used by this module after the frontend has
    // consumed them. HIR still carries only stable external IDs; this side payload is for
    // backend asset/glue planning. Source logical paths are derived from the retained
    // source identity table, not from raw source inputs.
    let source_logical_paths = collect_source_logical_paths_from_table(
        &compiler.source_files,
        &compiler.string_table,
        context.project_path_resolver.is_some(),
    );

    let external_import_candidates = collect_external_import_candidates_for_source_files(
        &source_logical_paths,
        external_dependency_resolution_table,
        context.builder_runtime_packages,
    );

    Ok(SemanticStageOutput::Complete(Box::new(
        CompleteSemanticStage {
            module: Module {
                executable: ModuleExecutable {
                    hir: hir_module,
                    type_environment,
                    borrow_analysis,
                },
                link_facts: ModuleLinkFacts {
                    external_package_registry: Arc::clone(&compiler.external_package_registry),
                    external_import_candidates,
                    functions: function_link_facts,
                },
                metadata: ModuleCompilerMetadata::from_hir_lowering(
                    entry_file_path.to_path_buf(),
                    warnings,
                    lowering_metadata,
                    const_top_level_fragments,
                    root_activity,
                    materialisation_context,
                ),
            },
            public_interface,
            generated_delta,
        },
    )))
}

/// Bind retained `PreparedHeaderSyntax` against provider interfaces.
///
/// WHAT: resolves public exports, builds the binding environment, canonicalizes dependency
///       edges, and completes constant initializer dependencies. Consumes only the retained
///       syntax carried in from preparation — it never retokenizes or reparses source.
/// WHY: these facts depend on provider interfaces and the project path resolver, so they
///      belong in the semantic phase after preparation has produced `PreparedHeaderSyntax`.
fn bind_retained_headers(
    compiler: &mut CompilerFrontend,
    prepared_header_syntax: PreparedHeaderSyntax,
    external_dependency_resolution_table: &ExternalImportResolutionTable,
    source_provider_dependencies: &SourceProviderDependencySet<'_>,
    warnings: &[CompilerDiagnostic],
) -> Result<BoundModuleHeaders, CompilerMessages> {
    let headers = bind_module_headers(
        prepared_header_syntax,
        compiler.external_package_registry.as_ref(),
        external_dependency_resolution_table,
        source_provider_dependencies,
        compiler.project_path_resolver.as_ref(),
        &mut compiler.string_table,
    )
    .map_err(|bag| {
        let mut messages = CompilerMessages::from_diagnostics(
            bag.into_diagnostics(),
            compiler.string_table.clone(),
        );
        messages.prepend_diagnostics_preserving_context(warnings.iter().cloned());
        messages
    })?;

    record_header_counters(&headers);
    Ok(headers)
}

fn sort_headers(
    compiler: &mut CompilerFrontend,
    module_headers: BoundModuleHeaders,
    warnings: &[CompilerDiagnostic],
) -> Result<SortedHeaders, CompilerMessages> {
    compiler.sort_headers(module_headers).map_err(|bag| {
        let mut messages = CompilerMessages::from_diagnostics(
            bag.into_diagnostics(),
            compiler.string_table.clone(),
        );
        messages.prepend_diagnostics_preserving_context(warnings.iter().cloned());
        messages
    })
}

// The timing context is a cfg-gated parameter that disappears from
// no-timer builds; bundling it would add a context struct for one field.
#[allow(clippy::too_many_arguments)]
fn build_ast_with_registered_types(
    context: &ModuleCompilationContext<'_>,
    compiler: &mut CompilerFrontend,
    sorted: SortedHeaders,
    entry_file_path: &Path,
    root_role: ModuleRootRole,
    capacity_estimate: FrontendArenaCapacityEstimate,
    warnings: &mut Vec<CompilerDiagnostic>,
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> Result<AstBuildResult, CompilerMessages> {
    match compiler.headers_to_ast(
        sorted,
        entry_file_path,
        root_role,
        context.build_profile,
        capacity_estimate,
        #[cfg(feature = "timers")]
        timing_context,
    ) {
        Ok(build_result) => {
            warnings.extend(build_result.ast.warnings.clone());
            Ok(build_result)
        }

        Err(messages) => Err(merge_stage_messages(
            messages,
            warnings,
            &compiler.string_table,
        )),
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

    // Phase 1 wires the scope-frame estimate because scope-frame arenas are the first typed arena
    // target. Phase 4 records actual frame allocation and arena capacity growth from the scope
    // arena owner; this site records only the policy estimate.
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

    let state_snapshot_count = report.analysis.block_entry_states.len()
        + report.analysis.block_exit_states.len()
        + report.analysis.statement_entry_states.len();
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

/// Render the module's source logical paths from the retained source identity table.
///
/// WHAT: iterates the `SourceFileTable` built during preparation and renders each identity's
///       portable logical path. Returns an empty vector when no project path resolver was used
///       during preparation, matching the prior raw-source path behaviour.
/// WHY: semantic compilation derives source logical paths from retained identities instead of
///      carrying raw source paths, so the preparation/semantic boundary stays free of
///      `PreparedSourceInput`. UTF-8 validity was already enforced when the table was built.
fn collect_source_logical_paths_from_table(
    source_files: &SourceFileTable,
    string_table: &StringTable,
    has_project_path_resolver: bool,
) -> Vec<String> {
    if !has_project_path_resolver {
        return Vec::new();
    }

    source_files
        .iter()
        .map(|identity| identity.logical_path.to_portable_string(string_table))
        .collect()
}
