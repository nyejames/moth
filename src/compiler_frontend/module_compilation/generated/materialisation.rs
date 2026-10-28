//! Materialising one concrete generic request to a completed sidecar.
//!
//! WHAT: resolves the declaring template, materialises its AST for the requested identity, drives
//!       nested requests depth-first, lowers and borrow-validates the generated HIR, collects its
//!       link facts and external imports, then completes the request in the module transaction.
//! WHY:  every step here is compiler semantics over compiler state. Stage 0 supplies the immutable
//!       provider-materialisation registry and receives the finished delta; it never mutates
//!       generated HIR or reruns generated borrow analysis.

use crate::compiler_frontend::CompilerFrontend;
use crate::compiler_frontend::analysis::numeric_proofs::NumericProofs;
use crate::compiler_frontend::ast::AstBuildResult;
use crate::compiler_frontend::ast::generic_functions::{
    MaterialisedGenericAst, ModuleMaterialisationInput, ModuleMaterialisationPreparation,
    recursive_generic_function_instantiation,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, PreparedFailure};
use crate::compiler_frontend::hir::failure_facts::HirBuiltinFailureBoundary;
use crate::compiler_frontend::hir::functions::HirFunctionOriginLookup;
use crate::compiler_frontend::hir::reachability::{
    collect_module_function_link_facts, collect_reachability_from_function_link_facts,
};
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::module_compilation::artefact::{
    Module, ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts, ModuleRootActivity,
};
use crate::compiler_frontend::module_compilation::context::ModuleCompilationContext;
use crate::compiler_frontend::module_compilation::external_imports::{
    collect_external_import_candidates_for_packages, collect_external_import_package_ids,
};
use crate::compiler_frontend::module_compilation::generated::artefacts::GeneratedFunctionSidecar;
use crate::compiler_frontend::module_compilation::generated::convergence::exact_generated_sidecar_summary;
use crate::compiler_frontend::module_compilation::generated::provider_materialisations::{
    DeclaringMaterialisation, declaring_materialisation,
};
use crate::compiler_frontend::module_compilation::generated::requests::install_generated_request_contracts;
use crate::compiler_frontend::module_compilation::generated::transaction::{
    GeneratedFunctionTransaction, GeneratedRequestEntry, GeneratedRequestFacts, GeneratedRequestId,
};
use crate::compiler_frontend::module_compilation::stages::{check_borrows, lower_hir};
use crate::compiler_frontend::module_metadata::HirLoweringResult;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

/// The identity and call-site span one generated request needs while materialising.
struct MaterialisingRequest {
    identity: GeneratedFunctionIdentity,
    display_name: String,
    call_span: Option<SourceSpan>,
    #[cfg(feature = "timers")]
    timing_context: Option<crate::timing::TimingContext>,
}

/// Materialise every request this module emitted from its own AST.
pub(in crate::compiler_frontend::module_compilation) fn materialise_generated_request_roots<
    'build,
>(
    context: &ModuleCompilationContext<'build>,
    request_ids: &[GeneratedRequestId],
    transaction: &mut GeneratedFunctionTransaction<'_>,
    requester_context: &ModuleMaterialisationPreparation,
    compiler: &mut CompilerFrontend<'_>,
    entry_file_path: &Path,
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> Result<(), PreparedFailure> {
    for request_id in request_ids {
        let identity = transaction.identity(*request_id).clone();
        let (display_name, call_span) = transaction.request_facts(*request_id);
        materialise_generated_request(
            context,
            *request_id,
            &MaterialisingRequest {
                identity,
                display_name,
                call_span,
                #[cfg(feature = "timers")]
                timing_context,
            },
            transaction,
            requester_context,
            compiler,
            entry_file_path,
        )?;
    }
    Ok(())
}

fn materialise_generated_request<'build>(
    context: &ModuleCompilationContext<'build>,
    request_id: GeneratedRequestId,
    request: &MaterialisingRequest,
    transaction: &mut GeneratedFunctionTransaction<'_>,
    requester_context: &ModuleMaterialisationPreparation,
    compiler: &mut CompilerFrontend<'_>,
    entry_file_path: &Path,
) -> Result<(), PreparedFailure> {
    match transaction.enter(request_id) {
        GeneratedRequestEntry::Complete => return Ok(()),
        GeneratedRequestEntry::Recursive => {
            let display_name = compiler.string_table.intern(&request.display_name);
            let diagnostic =
                recursive_generic_function_instantiation(Some(display_name), request.call_span);
            let diagnostic = if request.call_span.is_some() {
                diagnostic.with_primary_frozen_identity_handle(
                    requester_context.frozen_identity_handle.clone(),
                )
            } else {
                diagnostic
            };
            let batch = PreparedDiagnosticBatch::capture_with_required_paths(
                DiagnosticBag::from(diagnostic),
                compiler.string_table.take_for_handoff(),
                None,
                || Some(Arc::new(compiler.path_fork.shared_table())),
            )
            .map_err(PreparedFailure::from)?;
            return Err(PreparedFailure::Diagnosed(batch));
        }
        GeneratedRequestEntry::Materialise => {}
    }
    let declaring_context = declaring_materialisation(
        context.provider_materialisations,
        request.identity.declaration(),
        requester_context,
    )
    .ok_or_else(|| {
        PreparedFailure::from(InfrastructureFailure::internal(CompilerStage::ProjectBuildAssembly, format!(
            "Generated request for '{}' has no completed declaring-module materialisation context",
            request.identity.declaration().defining_name()
        )))
    })?;
    let (declaring_source_identity_handle, materialised) = match &declaring_context {
        DeclaringMaterialisation::Prepared(context) => {
            let (declaring_context, _) = context
                .declaring_context_for_identity(request.identity.declaration())
                .ok_or_else(|| {
                    PreparedFailure::from(InfrastructureFailure::internal(
                        CompilerStage::ProjectBuildAssembly,
                        "Prepared generic request lost its selected declaring context",
                    ))
                })?;
            let declaring_source_identity_handle = declaring_context.frozen_identity_handle();
            let materialised = context.materialise_ast(
                &request.identity,
                requester_context,
                request.call_span,
                &mut compiler.string_table,
                &mut compiler.path_fork,
                #[cfg(feature = "timers")]
                request.timing_context,
            )?;
            (declaring_source_identity_handle, materialised)
        }
        DeclaringMaterialisation::Published {
            context: declaring_context,
            template_index,
            rebase_required,
        } => {
            let declaring_source_identity_handle = declaring_context.frozen_identity_handle();
            let retained_identity_tables = declaring_context
                .retained_identity_tables()
                .map_err(PreparedFailure::from)?;
            // A same-boundary provider may have been published after this requester's preparation
            // fork was created. Its paths are then in the live boundary domain, but its retained
            // context deliberately has no growing table snapshot. Rebase from the boundary pair
            // for this request; only a completed package retains its own foreign pair.
            let boundary_rebase_available = retained_identity_tables.is_none()
                && context.global_path_table.is_some()
                && context.global_string_table.is_some();
            let materialised = if *rebase_required || boundary_rebase_available {
                let rebased_context = if let Some((path_table, source_strings)) =
                    retained_identity_tables
                {
                    declaring_context.rebased_for_requester_with(
                        path_table,
                        source_strings,
                        &mut compiler.path_fork,
                        &mut compiler.string_table,
                    )
                } else if let (Some(path_table), Some(source_strings)) =
                    (context.global_path_table, context.global_string_table)
                {
                    declaring_context.rebased_for_requester_with(
                        path_table,
                        source_strings,
                        &mut compiler.path_fork,
                        &mut compiler.string_table,
                    )
                } else {
                    declaring_context
                        .rebased_for_requester(&mut compiler.path_fork, &mut compiler.string_table)
                }
                .map_err(PreparedFailure::from)?;
                rebased_context.materialise_ast_at(
                    *template_index,
                    ModuleMaterialisationInput {
                        identity: &request.identity,
                        requester_context,
                        requester_call_span: request.call_span,
                        string_table: &mut compiler.string_table,
                        path_fork: &mut compiler.path_fork,
                        external_package_registry: &context.external_packages,
                        style_directives: context.style_directives,
                        build_profile: context.build_profile,
                        template_const_loop_iteration_limit: context
                            .options
                            .template_const_loop_iteration_limit,
                        #[cfg(feature = "timers")]
                        timing_context: request.timing_context,
                    },
                )
            } else {
                declaring_context.materialise_ast_at(
                    *template_index,
                    ModuleMaterialisationInput {
                        identity: &request.identity,
                        requester_context,
                        requester_call_span: request.call_span,
                        string_table: &mut compiler.string_table,
                        path_fork: &mut compiler.path_fork,
                        external_package_registry: &context.external_packages,
                        style_directives: context.style_directives,
                        build_profile: context.build_profile,
                        template_const_loop_iteration_limit: context
                            .options
                            .template_const_loop_iteration_limit,
                        #[cfg(feature = "timers")]
                        timing_context: request.timing_context,
                    },
                )
            }?;
            (declaring_source_identity_handle, materialised)
        }
    };
    let MaterialisedGenericAst {
        build_result,
        instance_path,
    } = materialised;
    let AstBuildResult {
        ast: mut generated_ast,
        materialisation_context: generated_context_builder,
        deferred_generic_requests: nested_requests,
        assertion_validation_generic_requests: nested_validation_requests,
        module_resources,
        ..
    } = build_result;
    let module_resources = module_resources.ok_or_else(|| {
        PreparedFailure::from(InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            "generated AST finalization did not retain its sidecar resource table",
        ))
    })?;
    // Nested executable requests and inert assertion validation both need their declaring
    // preparation until concrete summaries converge. Only executable reachability publishes
    // their completed sidecars.
    let nested = match generated_context_builder {
        Some(builder) => {
            let nested_requests = install_generated_request_contracts(
                nested_requests
                    .iter()
                    .chain(nested_validation_requests.iter()),
                builder.context(),
                builder.context().generic_function_templates(),
                context.external_packages.as_ref(),
                &compiler.path_fork,
                &mut generated_ast,
            )
            .map_err(PreparedFailure::from)?;
            let generated_context = builder
                .finish_preparation(
                    &generated_ast.const_values,
                    &module_resources.borrow(),
                    &compiler.path_fork,
                )
                .map_err(PreparedFailure::from)?;
            let nested_request_ids =
                transaction.register_requests(nested_requests.iter().map(|request| {
                    GeneratedRequestFacts {
                        identity: request.identity.clone(),
                        display_name: request
                            .function_name
                            .map(|name| compiler.string_table.resolve(name).to_owned())
                            .unwrap_or_else(|| "<generated>".to_owned()),
                        call_span: request.call_span,
                    }
                }));
            Some((generated_context, nested_request_ids))
        }
        None if nested_requests.is_empty() && nested_validation_requests.is_empty() => None,
        None => {
            return Err(PreparedFailure::from(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                "generated AST finalization dropped the requester context of its nested requests",
            )));
        }
    };

    // Generated materialisation and lowering intern strings and paths straight into the
    // requester's tables. Snapshotting either table per request made generic instantiation
    // quadratic, and shared tables need no merge. Both tables return to the requester on
    // every exit so failures still attach the full tables.
    let requester_string_table = compiler.string_table.take_for_handoff();
    let requester_path_fork = std::mem::replace(&mut compiler.path_fork, PathInternerFork::empty());
    let mut generated_compiler = CompilerFrontend::new(
        context.options.clone(),
        requester_string_table,
        requester_path_fork,
        context.style_directives,
        &context.external_packages,
        context.project_path_resolver,
        context.source_files,
    );
    let lowered = (|| -> Result<(Module, DiagnosticBag), PreparedFailure> {
        if let Some((generated_context, nested_request_ids)) = nested {
            for nested_request_id in &nested_request_ids {
                let nested_identity = transaction.identity(*nested_request_id).clone();
                let (nested_name, nested_span) = transaction.request_facts(*nested_request_id);
                materialise_generated_request(
                    context,
                    *nested_request_id,
                    &MaterialisingRequest {
                        identity: nested_identity,
                        display_name: nested_name,
                        call_span: nested_span,
                        #[cfg(feature = "timers")]
                        timing_context: request.timing_context,
                    },
                    transaction,
                    &generated_context,
                    &mut generated_compiler,
                    entry_file_path,
                )?;
            }
            // Nested materialisation consumes only detached facts. Release its requester services
            // before lowering takes sole ownership of the sidecar resources.
            drop(generated_context);
        }
        let mut generated_warnings =
            DiagnosticBag::from_diagnostics(std::mem::take(&mut generated_ast.warnings));

        let generated_lowering = lower_hir(
            &mut generated_compiler,
            generated_ast,
            &mut generated_warnings,
            HirFunctionOriginLookup::default(),
            Some(Rc::clone(&module_resources)),
            Some(&declaring_source_identity_handle),
            Default::default(),
        )?;
        let HirLoweringResult {
            mut hir_module,
            type_environment,
            metadata: lowering_metadata,
        } = generated_lowering;
        let resource_table = Rc::try_unwrap(module_resources)
            .map(|cell| cell.into_inner())
            .map_err(|_| {
                PreparedFailure::from(InfrastructureFailure::internal(CompilerStage::ProjectBuildAssembly, "generated sidecar resource table still has a live shared handle after HIR lowering"))
            })?;
        let function_id = hir_module
            .functions
            .iter()
            .find_map(|function| {
                (hir_module.side_table.function_name_path(function.id) == Some(instance_path))
                    .then_some(function.id)
            })
            .ok_or_else(|| {
                PreparedFailure::from(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    "Generated HIR omitted its requested root function",
                ))
            })?;
        hir_module
            .function_ids_by_generated
            .insert(request.identity.clone(), function_id);
        let root_failure_facts = hir_module
            .function_failure_facts
            .get_mut(&function_id)
            .ok_or_else(|| {
                PreparedFailure::from(InfrastructureFailure::internal(
                    CompilerStage::ProjectBuildAssembly,
                    "Generated HIR omitted its requested root failure facts",
                ))
            })?;
        if matches!(
            request.identity.declaration(),
            GeneratedDeclarationIdentity::Public(_)
        ) && root_failure_facts.boundary == HirBuiltinFailureBoundary::InferPrivate
        {
            root_failure_facts.boundary = HirBuiltinFailureBoundary::ExportedNoSlot;
        }
        increment_frontend_counter(FrontendCounter::ConvergenceGeneratedSidecarBorrowPasses);
        let borrow_analysis = check_borrows(
            &mut generated_compiler,
            &hir_module,
            &type_environment,
            &mut generated_warnings,
            Some(&declaring_source_identity_handle),
        )?;
        let functions =
            collect_module_function_link_facts(&hir_module).map_err(PreparedFailure::from)?;
        let reachability =
            collect_reachability_from_function_link_facts(&functions, &[function_id])
                .map_err(PreparedFailure::from)?;
        let reachable_package_ids = collect_external_import_package_ids(
            reachability.reachable_external_calls.iter(),
            &context.external_packages,
        );
        let external_import_candidates = collect_external_import_candidates_for_packages(
            &reachable_package_ids,
            context.external_dependency_resolution_table,
            context.builder_runtime_packages,
            &context.external_packages,
        )
        .map_err(PreparedFailure::from)?;
        // The publication owner in convergence computes the paired table from the final HIR.
        // Nothing reads this empty table before then, and an empty table retains every check.
        Ok((
            Module {
                executable: ModuleExecutable {
                    hir: hir_module,
                    resource_table,
                    type_environment,
                    borrow_analysis,
                    numeric_proofs: NumericProofs::default(),
                    path_table: Arc::new(
                        crate::compiler_frontend::symbols::path_interner::PathInternerBuilder::new(
                        )
                        .freeze(),
                    ),
                },
                link_facts: ModuleLinkFacts {
                    external_package_registry: Arc::clone(&context.external_packages),
                    external_import_candidates,
                    functions,
                },
                metadata: ModuleCompilerMetadata {
                    entry_point: entry_file_path.to_path_buf(),
                    const_top_level_fragments: Vec::new(),
                    root_activity: ModuleRootActivity::default(),
                    doc_fragments: lowering_metadata.doc_fragments,
                    materialisation_context: None,
                },
            },
            generated_warnings,
        ))
    })();
    let CompilerFrontend {
        string_table: requester_string_table,
        path_fork: requester_path_fork,
        ..
    } = generated_compiler;
    compiler.string_table = requester_string_table;
    compiler.path_fork = requester_path_fork;
    let (generated_module, generated_warnings) = lowered?;

    // The boundary install tail replaces each sidecar placeholder with the one final path table
    // after all publications, so successful materialisation does not snapshot the growing table.
    let summary = exact_generated_sidecar_summary(&request.identity, &generated_module);
    transaction.complete(
        request_id,
        summary,
        GeneratedFunctionSidecar::new(request.identity.clone(), generated_module),
        generated_warnings,
    );
    Ok(())
}
