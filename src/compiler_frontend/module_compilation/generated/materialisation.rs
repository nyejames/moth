//! Materialising one concrete generic request to a completed sidecar.
//!
//! WHAT: resolves the declaring template, materialises its AST for the requested identity, drives
//!       nested requests depth-first, lowers and borrow-validates the generated HIR, collects its
//!       link facts and external imports, then completes the request in the module transaction.
//! WHY:  every step here is compiler semantics over compiler state. Stage 0 supplies the immutable
//!       provider-materialisation registry and receives the finished delta; it never mutates
//!       generated HIR or reruns generated borrow analysis.

use crate::compiler_frontend::CompilerFrontend;
use crate::compiler_frontend::ast::AstBuildResult;
use crate::compiler_frontend::ast::generic_functions::{
    MaterialisedGenericAst, ModuleMaterialisationInput, ModuleMaterialisationPreparation,
    recursive_generic_function_instantiation,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::hir::functions::HirFunctionOriginLookup;
use crate::compiler_frontend::hir::reachability::{
    collect_module_function_link_facts, collect_reachability_from_function_link_facts,
};
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::module_compilation::artefact::{
    Module, ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts, ModuleRootActivity,
};
use crate::compiler_frontend::module_compilation::context::ModuleCompilationContext;
use crate::compiler_frontend::module_compilation::external_imports::collect_external_import_candidates_for_packages;
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
use crate::compiler_frontend::semantic_identity::GeneratedFunctionIdentity;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use rustc_hash::FxHashSet;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

/// The identity and diagnostic facts one generated request needs while materialising.
struct MaterialisingRequest {
    identity: GeneratedFunctionIdentity,
    display_name: String,
    diagnostic_location: SourceLocation,
    #[cfg(feature = "timers")]
    timing_context: Option<crate::timing::TimingContext>,
}

/// Materialise every request this module emitted from its own AST.
pub(in crate::compiler_frontend::module_compilation) fn materialise_generated_request_roots(
    context: &ModuleCompilationContext<'_>,
    request_ids: &[GeneratedRequestId],
    transaction: &mut GeneratedFunctionTransaction<'_>,
    requester_context: &ModuleMaterialisationPreparation,
    compiler: &mut CompilerFrontend,
    entry_file_path: &Path,
    #[cfg(feature = "timers")] timing_context: Option<crate::timing::TimingContext>,
) -> Result<(), CompilerMessages> {
    for request_id in request_ids {
        let identity = transaction
            .identity(*request_id)
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?
            .clone();
        let (display_name, diagnostic_location) = transaction
            .request_facts(*request_id)
            .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
        materialise_generated_request(
            context,
            *request_id,
            &MaterialisingRequest {
                identity,
                display_name,
                diagnostic_location,
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

/// Materialise one request, completing every nested request it raises first.
fn materialise_generated_request(
    context: &ModuleCompilationContext<'_>,
    request_id: GeneratedRequestId,
    request: &MaterialisingRequest,
    transaction: &mut GeneratedFunctionTransaction<'_>,
    requester_context: &ModuleMaterialisationPreparation,
    compiler: &mut CompilerFrontend,
    entry_file_path: &Path,
) -> Result<(), CompilerMessages> {
    match transaction
        .enter(request_id)
        .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?
    {
        GeneratedRequestEntry::Complete => return Ok(()),
        GeneratedRequestEntry::Recursive => {
            return Err(CompilerMessages::from_diagnostic(
                recursive_generic_function_instantiation(
                    Some(compiler.string_table.intern(&request.display_name)),
                    request.diagnostic_location.clone(),
                ),
                compiler.string_table.clone(),
            ));
        }
        GeneratedRequestEntry::Materialise => {}
    }
    let declaring_context = declaring_materialisation(
        context.provider_materialisations,
        request.identity.declaration(),
        requester_context,
    )
        .ok_or_else(|| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(format!(
                    "Generated request for '{}' has no completed declaring-module materialisation context",
                    request.identity.declaration().defining_name()
                )),
                &compiler.string_table,
            )
        })?;
    let materialised = match declaring_context {
        DeclaringMaterialisation::Published {
            context: declaring_context,
            template_index,
        } => declaring_context.materialise_ast_at(
            template_index,
            ModuleMaterialisationInput {
                identity: &request.identity,
                requester_context,
                requester_call_location: &request.diagnostic_location,
                external_package_registry: context.external_packages.as_ref(),
                style_directives: context.style_directives,
                build_profile: context.build_profile,
                project_path_resolver: context
                    .project_path_resolver
                    .clone()
                    .or_else(|| requester_context.project_path_resolver.clone()),
                template_const_loop_iteration_limit: context
                    .options
                    .template_const_loop_iteration_limit,
                #[cfg(feature = "timers")]
                timing_context: request.timing_context,
            },
        ),
        DeclaringMaterialisation::Preparing(declaring_context) => declaring_context
            .materialise_ast(
                &request.identity,
                requester_context,
                &request.diagnostic_location,
                context
                    .project_path_resolver
                    .clone()
                    .or_else(|| requester_context.project_path_resolver.clone()),
                #[cfg(feature = "timers")]
                request.timing_context,
            ),
    }?;
    let MaterialisedGenericAst {
        build_result,
        string_table: generated_string_table,
        instance_path,
    } = materialised;
    let AstBuildResult {
        ast: mut generated_ast,
        materialisation_context: generated_context_builder,
        deferred_generic_requests: nested_requests,
        module_resources,
        ..
    } = build_result;
    let module_resources = module_resources.ok_or_else(|| {
        CompilerMessages::from_error_ref(
            CompilerError::compiler_error(
                "generated AST finalization did not retain its sidecar resource table",
            ),
            &compiler.string_table,
        )
    })?;
    let generated_context = generated_context_builder
        .finish_preparation()
        .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))?;
    let nested_requests = install_generated_request_contracts(
        &nested_requests,
        &generated_context,
        generated_context.generic_function_templates(),
        context.external_packages.as_ref(),
        &mut generated_ast,
    )
    .map_err(|error| CompilerMessages::from_error_ref(error, &generated_string_table))?;
    let nested_request_ids = transaction.register_requests(nested_requests.iter().map(|request| {
        GeneratedRequestFacts {
            identity: request.identity.clone(),
            display_name: request
                .function_name
                .map(|name| generated_string_table.resolve(name).to_owned())
                .unwrap_or_else(|| "<generated>".to_owned()),
            diagnostic_location: request.call_location.clone(),
        }
    }));

    let first_nested_sidecar = transaction.sidecar_count();
    let mut generated_compiler = CompilerFrontend::new(
        context.options.clone(),
        generated_string_table,
        context.style_directives.to_owned(),
        Arc::clone(&context.external_packages),
        context.project_path_resolver.clone(),
    );
    for nested_request_id in &nested_request_ids {
        let nested_identity = transaction
            .identity(*nested_request_id)
            .map_err(|error| {
                CompilerMessages::from_error_ref(error, &generated_compiler.string_table)
            })?
            .clone();
        let (nested_name, nested_location) = transaction
            .request_facts(*nested_request_id)
            .map_err(|error| {
                CompilerMessages::from_error_ref(error, &generated_compiler.string_table)
            })?;
        materialise_generated_request(
            context,
            *nested_request_id,
            &MaterialisingRequest {
                identity: nested_identity,
                display_name: nested_name,
                diagnostic_location: nested_location,
                #[cfg(feature = "timers")]
                timing_context: request.timing_context,
            },
            transaction,
            &generated_context,
            &mut generated_compiler,
            entry_file_path,
        )?;
    }
    // The generated preparation retains a second handle only while nested requests are
    // materialised; release it before lowering so the sidecar can own its table immutably.
    drop(generated_context);

    let generated_warnings = generated_ast.warnings.clone();
    let generated_lowering = lower_hir(
        &mut generated_compiler,
        generated_ast,
        &generated_warnings,
        HirFunctionOriginLookup::default(),
        Some(Rc::clone(&module_resources)),
    )?;
    let HirLoweringResult {
        mut hir_module,
        type_environment,
        metadata: lowering_metadata,
    } = generated_lowering;
    let resource_table = Rc::try_unwrap(module_resources)
        .map(|cell| cell.into_inner())
        .map_err(|_| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error(
                    "generated sidecar resource table still has a live shared handle after HIR lowering",
                ),
                &generated_compiler.string_table,
            )
        })?;
    let function_id = hir_module
        .functions
        .iter()
        .find_map(|function| {
            (hir_module.side_table.function_name_path(function.id) == Some(&instance_path))
                .then_some(function.id)
        })
        .ok_or_else(|| {
            CompilerMessages::from_error_ref(
                CompilerError::compiler_error("Generated HIR omitted its requested root function"),
                &generated_compiler.string_table,
            )
        })?;
    hir_module
        .function_ids_by_generated
        .insert(request.identity.clone(), function_id);
    increment_frontend_counter(FrontendCounter::ConvergenceGeneratedSidecarBorrowPasses);
    let borrow_analysis = check_borrows(&generated_compiler, &hir_module, &generated_warnings)?;
    let functions = collect_module_function_link_facts(&hir_module).map_err(|error| {
        CompilerMessages::from_error_ref(error, &generated_compiler.string_table)
    })?;
    let reachability = collect_reachability_from_function_link_facts(&functions, &[function_id])
        .map_err(|error| {
            CompilerMessages::from_error_ref(error, &generated_compiler.string_table)
        })?;
    let mut reachable_package_ids = FxHashSet::default();
    for external_function_id in &reachability.reachable_external_functions {
        let package_id = context
            .external_packages
            .resolve_function_package_id(*external_function_id)
            .ok_or_else(|| {
                CompilerMessages::from_error_ref(
                    CompilerError::compiler_error(format!(
                        "Generated external function {external_function_id:?} has no owning package"
                    )),
                    &generated_compiler.string_table,
                )
            })?;
        reachable_package_ids.insert(package_id);
    }
    let external_import_candidates = collect_external_import_candidates_for_packages(
        &reachable_package_ids,
        context.external_dependency_resolution_table,
        context.builder_runtime_packages,
    );
    let mut generated_module = Module {
        executable: ModuleExecutable {
            hir: hir_module,
            resource_table,
            type_environment,
            borrow_analysis,
        },
        link_facts: ModuleLinkFacts {
            external_package_registry: Arc::clone(&context.external_packages),
            external_import_candidates,
            functions,
        },
        metadata: ModuleCompilerMetadata {
            entry_point: entry_file_path.to_path_buf(),
            warnings: generated_warnings,
            const_top_level_fragments: Vec::new(),
            root_activity: ModuleRootActivity::default(),
            doc_fragments: lowering_metadata.doc_fragments,
            rendered_path_usages: lowering_metadata.rendered_path_usages,
            materialisation_context: None,
        },
    };
    let summary =
        exact_generated_sidecar_summary(&request.identity, &generated_module).map_err(|error| {
            CompilerMessages::from_error_ref(error, &generated_compiler.string_table)
        })?;
    let generated_remap = requester_context.merge_materialisation_string_table_into(
        &mut compiler.string_table,
        &generated_compiler.string_table,
    );
    if !generated_remap.is_identity() {
        transaction.remap_sidecars_and_module_from(
            first_nested_sidecar,
            &mut generated_module,
            &generated_remap,
        );
    }
    transaction
        .complete(
            request_id,
            summary,
            GeneratedFunctionSidecar::new(request.identity.clone(), generated_module),
        )
        .map_err(|error| CompilerMessages::from_error_ref(error, &compiler.string_table))
}
