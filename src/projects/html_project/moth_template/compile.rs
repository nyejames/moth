//! Direct Moth template compile orchestration.
//!
//! WHAT: turns one normalized request into ordered source units, builds each unit's Stage 0
//!       file-value bundle, compiles each through the compiler's direct Moth template service,
//!       publishes the folds' resource associations onto one request-owned registry, plans every
//!       surviving resource origin onto one request-wide output plan, and packages the rendered
//!       documents, deferred resource outputs and warnings.
//! WHY:  source collection, the project's style vocabulary and the output shape are project
//!       policy. The stage sequence that folds template source into `content` values is
//!       compiler-owned, so this module composes no frontend stage itself. Resource identity,
//!       placement and output-path conflicts belong to the shared builder plan, and emission
//!       stays with the caller: this layer only returns the byte-free destinations and the
//!       physical source mapping they resolve against.

use crate::build_system::build::DeferredResourceOutput;
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureErrors, InfrastructureFailure,
};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::PreparedFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::folded_value::OwnedFoldedString;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::single_source_compilation::{
    MothTemplateCompilationFailure, MothTemplateCompilationRequest, compile_moth_template_source,
};
use crate::compiler_frontend::source::SourceDatabase;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::moth_template::bundle::{
    DIRECT_TEMPLATE_PROJECT_NAME, MothTemplateBundleFailure, prepare_file_value_bundle,
};
use crate::projects::html_project::moth_template::input::MothTemplateInput;
use crate::projects::html_project::moth_template::output::{
    CompiledMothTemplateDocument, MothTemplateCompileOutput, PreparedTemplateWarning,
};
use crate::projects::html_project::moth_template::render::{
    document_url_context, render_structural_content,
};
use crate::projects::html_project::resource_output_plan::{
    HtmlResourceOutputPlan, PlannedResourceOutput,
};
use crate::projects::html_project::style_directives::html_project_style_directives;
use std::sync::Arc;

pub(crate) fn compile_moth_template(
    input: MothTemplateInput,
    string_table: &mut StringTable,
) -> CompilationResult<MothTemplateCompileOutput> {
    let mut resource_inputs = ResourceInputRegistry::new();
    match compile_moth_template_with_registry(input, string_table, &mut resource_inputs)? {
        CompilationOutcome::Success { value, warnings } => Ok(CompilationOutcome::Success {
            value: MothTemplateCompileOutput {
                documents: value.documents,
                resources: value.resources,
                resource_inputs,
            },
            warnings,
        }),
        CompilationOutcome::Diagnosed(reports) => Ok(CompilationOutcome::Diagnosed(reports)),
    }
}

/// Documents and planned outputs produced against a caller-owned registry.
#[derive(Debug)]
pub(crate) struct DirectTemplateRegistryCompile {
    documents: Vec<CompiledMothTemplateDocument>,
    resources: Vec<DeferredResourceOutput>,
}

/// Compile one input against a caller-owned registry so a failed plan still leaves its
/// registered sources inspectable.
pub(crate) fn compile_moth_template_with_registry(
    input: MothTemplateInput,
    string_table: &mut StringTable,
    resource_inputs: &mut ResourceInputRegistry,
) -> CompilationResult<DirectTemplateRegistryCompile> {
    let mut warnings = Vec::new();
    let mut path_fork = PathInternerFork::empty();
    let sources = match input.collect_sources(string_table, &mut path_fork) {
        Ok(sources) => sources,
        Err(failure) => return prepared_failure_to_result(failure, None, string_table),
    };

    let style_directives = match StyleDirectiveRegistry::merged(&html_project_style_directives()) {
        Ok(style_directives) => style_directives,
        Err(error) => {
            return prepared_failure_to_result(PreparedFailure::from(error), None, string_table);
        }
    };

    let mut documents = Vec::with_capacity(sources.len());
    let mut resource_plan = HtmlResourceOutputPlan::new(DIRECT_TEMPLATE_PROJECT_NAME);

    for mut unit in sources {
        let source_path = unit.source_path.clone();
        let relative_path = unit.relative_path.clone();

        let file_value_bundle = match prepare_file_value_bundle(
            &mut unit,
            &style_directives,
            string_table,
            resource_inputs,
        ) {
            Ok(bundle) => bundle,
            Err(failure) => return bundle_failure_to_result(failure, string_table),
        };

        let folded = match compile_moth_template_source(
            MothTemplateCompilationRequest {
                source_path: source_path.as_path(),
                source_code: None,
                style_directives: &style_directives,
                file_value_resolution: Some(file_value_bundle),
            },
            string_table,
        ) {
            Ok(folded) => folded,
            Err(failure) => return template_failure_to_result(failure, string_table),
        };
        let content = folded.content;
        let module_resources = folded.module_resources;
        let source_database = folded.source_database;
        if let Some(batch) = folded.warnings {
            warnings.push(PreparedTemplateWarning {
                batch,
                source_database: Arc::clone(&source_database),
            });
        }

        if let Err(error) = publish_module_resource_associations(resource_inputs, &module_resources)
        {
            return prepared_failure_to_result(
                PreparedFailure::from(error),
                Some(source_database),
                string_table,
            );
        }

        let document_path = match document_url_context(relative_path.as_deref(), &source_path) {
            Ok(document_path) => document_path,
            Err(error) => {
                return prepared_failure_to_result(
                    PreparedFailure::from(error),
                    Some(source_database),
                    string_table,
                );
            }
        };
        let content = match &content {
            OwnedFoldedString::Text(text) => text.clone(),
            OwnedFoldedString::Pieces(_) => {
                let mut diagnostic_strings = DiagnosticStringBuilder::new(string_table);
                match render_structural_content(
                    &content,
                    &module_resources,
                    &document_path,
                    &mut resource_plan,
                    &mut diagnostic_strings,
                ) {
                    Ok(content) => content,
                    Err(failure) => {
                        return prepared_failure_to_result(
                            failure,
                            Some(source_database),
                            string_table,
                        );
                    }
                }
            }
        };

        documents.push(CompiledMothTemplateDocument {
            source_path,
            relative_path,
            content,
        });
    }

    let resources = match deferred_resource_outputs(resource_inputs, resource_plan.into_records()) {
        Ok(resources) => resources,
        Err(error) => {
            return prepared_failure_to_result(PreparedFailure::from(error), None, string_table);
        }
    };
    let warnings = finish_successful_warnings(warnings, string_table)?;

    Ok(CompilationOutcome::Success {
        value: DirectTemplateRegistryCompile {
            documents,
            resources,
        },
        warnings,
    })
}

/// Publish the fold's compiler-produced origin/source associations on the request registry.
///
/// The Stage 0 resolver registered the physical sources on this same registry while walking the
/// content closure, so every association here must name a known source and origin pair;
/// otherwise the fold contradicted its own Stage 0 facts.
fn publish_module_resource_associations(
    resource_inputs: &mut ResourceInputRegistry,
    module_resources: &ModuleResourceTable,
) -> Result<(), InfrastructureFailure> {
    let associations = module_resources.resource_source_associations();
    let publication = resource_inputs.preflight_resource_source_associations(associations)?;

    resource_inputs.reserve_resource_source_associations(&publication);
    resource_inputs.commit_resource_source_associations(publication);

    Ok(())
}

/// Pair one document's planned resource outputs with their registered physical sources.
///
/// A planned output with no registered source attachment is an internal invariant violation
/// rather than a guessable filesystem source, mirroring the builder's planned-output emission.
fn deferred_resource_outputs(
    resource_inputs: &ResourceInputRegistry,
    planned_resources: Vec<PlannedResourceOutput>,
) -> Result<Vec<DeferredResourceOutput>, InfrastructureFailure> {
    let mut deferred_resources = Vec::with_capacity(planned_resources.len());

    for planned in planned_resources {
        let Some(source_id) = resource_inputs.source_for_origin(&planned.origin) else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "planned direct-template resource origin {:?} has no registered source \
                 attachment",
                    planned.origin
                ),
            ));
        };

        deferred_resources.push(DeferredResourceOutput {
            relative_output_path: planned.output_path,
            source_id,
        });
    }

    Ok(deferred_resources)
}

fn prepared_failure_to_result<T>(
    failure: PreparedFailure,
    source_database: Option<Arc<SourceDatabase>>,
    string_table: &mut StringTable,
) -> CompilationResult<T> {
    let mut accumulator =
        BoundaryAccumulator::with_root_string_table(string_table.take_for_handoff());
    append_prepared_failure(&mut accumulator, failure, source_database);
    accumulator
        .finish(InstallMode::Unfrozen)
        .map(CompilationOutcome::Diagnosed)
}

fn bundle_failure_to_result<T>(
    failure: MothTemplateBundleFailure,
    string_table: &mut StringTable,
) -> CompilationResult<T> {
    let mut accumulator =
        BoundaryAccumulator::with_root_string_table(string_table.take_for_handoff());
    failure.append_to_boundary(&mut accumulator);
    accumulator
        .finish(InstallMode::Unfrozen)
        .map(CompilationOutcome::Diagnosed)
}

fn template_failure_to_result<T>(
    failure: MothTemplateCompilationFailure,
    string_table: &mut StringTable,
) -> CompilationResult<T> {
    let mut accumulator =
        BoundaryAccumulator::with_root_string_table(string_table.take_for_handoff());
    let install = failure.append_to_boundary(&mut accumulator);
    accumulator
        .finish(install)
        .map(CompilationOutcome::Diagnosed)
}

fn finish_successful_warnings(
    warnings: Vec<PreparedTemplateWarning>,
    string_table: &mut StringTable,
) -> Result<Option<DiagnosticReportSet>, InfrastructureErrors> {
    if warnings
        .iter()
        .all(|warning| warning.batch.diagnostic_count() == 0)
    {
        return Ok(None);
    }

    let mut accumulator =
        BoundaryAccumulator::with_root_string_table(string_table.take_for_handoff());
    for warning in warnings {
        accumulator.append_prepared(warning.batch, Some((warning.source_database, None)));
    }
    accumulator.finish(InstallMode::Unfrozen).map(Some)
}

fn append_prepared_failure(
    accumulator: &mut BoundaryAccumulator,
    failure: PreparedFailure,
    source_database: Option<Arc<SourceDatabase>>,
) {
    match failure {
        PreparedFailure::Diagnosed(batch) => {
            accumulator.append_prepared(batch, source_database.map(|database| (database, None)));
        }
        PreparedFailure::Infrastructure(errors) => {
            accumulator.append_infrastructure(errors);
        }
    }
}
