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
use crate::compiler_frontend::compiler_errors::{BoundaryFailure, CompilerError};
use crate::compiler_frontend::compiler_messages::PreparedFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
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
) -> Result<MothTemplateCompileOutput, BoundaryFailure> {
    let mut resource_inputs = ResourceInputRegistry::new();
    let compiled = compile_moth_template_with_registry(input, string_table, &mut resource_inputs)?;

    Ok(MothTemplateCompileOutput {
        documents: compiled.documents,
        resources: compiled.resources,
        resource_inputs,
        warnings: compiled.warnings,
    })
}

/// Documents, planned outputs and prepared warnings produced against a caller-owned registry.
#[derive(Debug)]
pub(crate) struct DirectTemplateRegistryCompile {
    documents: Vec<CompiledMothTemplateDocument>,
    resources: Vec<DeferredResourceOutput>,
    warnings: Vec<PreparedTemplateWarning>,
}

/// Compile one input against a caller-owned registry so a failed plan still leaves its
/// registered sources inspectable.
pub(crate) fn compile_moth_template_with_registry(
    input: MothTemplateInput,
    string_table: &mut StringTable,
    resource_inputs: &mut ResourceInputRegistry,
) -> Result<DirectTemplateRegistryCompile, BoundaryFailure> {
    let mut warnings = Vec::new();
    let mut path_fork = PathInternerFork::empty();
    let sources = match input.collect_sources(string_table, &mut path_fork) {
        Ok(sources) => sources,
        Err(failure) => {
            return Err(prepared_failure_to_boundary(
                failure,
                &mut warnings,
                None,
                string_table,
            ));
        }
    };

    let style_directives = match StyleDirectiveRegistry::merged(&html_project_style_directives()) {
        Ok(style_directives) => style_directives,
        Err(error) => {
            return Err(prepared_failure_to_boundary(
                PreparedFailure::from(error),
                &mut warnings,
                None,
                string_table,
            ));
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
            Err(failure) => {
                return Err(bundle_failure_to_boundary(
                    failure,
                    &mut warnings,
                    string_table,
                ));
            }
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
            Err(failure) => {
                return Err(template_failure_to_boundary(
                    failure,
                    &mut warnings,
                    string_table,
                ));
            }
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

        if let Err(failure) =
            publish_module_resource_associations(resource_inputs, &module_resources)
                .map_err(PreparedFailure::from)
        {
            return Err(prepared_failure_to_boundary(
                failure,
                &mut warnings,
                Some(source_database),
                string_table,
            ));
        }

        let document_path = match document_url_context(relative_path.as_deref(), &source_path)
            .map_err(PreparedFailure::from)
        {
            Ok(document_path) => document_path,
            Err(failure) => {
                return Err(prepared_failure_to_boundary(
                    failure,
                    &mut warnings,
                    Some(source_database),
                    string_table,
                ));
            }
        };
        let content = report_prepared_step(
            match &content {
                OwnedFoldedString::Text(text) => Ok(text.clone()),
                OwnedFoldedString::Pieces(_) => render_structural_content(
                    &content,
                    &module_resources,
                    &document_path,
                    &mut resource_plan,
                    string_table,
                ),
            },
            &mut warnings,
            Some(source_database),
            string_table,
        )?;

        documents.push(CompiledMothTemplateDocument {
            source_path,
            relative_path,
            content,
        });
    }

    let resources = report_prepared_step(
        deferred_resource_outputs(resource_inputs, resource_plan.into_records())
            .map_err(PreparedFailure::from),
        &mut warnings,
        None,
        string_table,
    )?;

    Ok(DirectTemplateRegistryCompile {
        documents,
        resources,
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
) -> Result<(), CompilerError> {
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
) -> Result<Vec<DeferredResourceOutput>, CompilerError> {
    let mut deferred_resources = Vec::with_capacity(planned_resources.len());

    for planned in planned_resources {
        let Some(source_id) = resource_inputs.source_for_origin(&planned.origin) else {
            return Err(CompilerError::compiler_error(format!(
                "planned direct-template resource origin {:?} has no registered source \
                 attachment",
                planned.origin
            )));
        };

        deferred_resources.push(DeferredResourceOutput {
            relative_output_path: planned.output_path,
            source_id,
        });
    }

    Ok(deferred_resources)
}

fn report_prepared_step<T>(
    step: Result<T, PreparedFailure>,
    warnings: &mut Vec<PreparedTemplateWarning>,
    source_database: Option<Arc<SourceDatabase>>,
    string_table: &StringTable,
) -> Result<T, BoundaryFailure> {
    match step {
        Ok(value) => Ok(value),
        Err(failure) => Err(prepared_failure_to_boundary(
            failure,
            warnings,
            source_database,
            string_table,
        )),
    }
}

fn prepared_failure_to_boundary(
    failure: PreparedFailure,
    warnings: &mut Vec<PreparedTemplateWarning>,
    source_database: Option<Arc<SourceDatabase>>,
    string_table: &StringTable,
) -> BoundaryFailure {
    let mut accumulator = BoundaryAccumulator::with_root_string_table(string_table);
    append_prior_warnings(&mut accumulator, warnings);
    append_prepared_failure(&mut accumulator, failure, source_database);
    accumulator.finish(InstallMode::Unfrozen)
}

fn bundle_failure_to_boundary(
    failure: MothTemplateBundleFailure,
    warnings: &mut Vec<PreparedTemplateWarning>,
    string_table: &StringTable,
) -> BoundaryFailure {
    let mut accumulator = BoundaryAccumulator::with_root_string_table(string_table);
    append_prior_warnings(&mut accumulator, warnings);
    failure.append_to_boundary(&mut accumulator);
    accumulator.finish(InstallMode::Unfrozen)
}

fn template_failure_to_boundary(
    failure: MothTemplateCompilationFailure,
    warnings: &mut Vec<PreparedTemplateWarning>,
    string_table: &StringTable,
) -> BoundaryFailure {
    let mut accumulator = BoundaryAccumulator::with_root_string_table(string_table);
    append_prior_warnings(&mut accumulator, warnings);
    let install = failure.append_to_boundary(&mut accumulator);
    accumulator.finish(install)
}

fn append_prior_warnings(
    accumulator: &mut BoundaryAccumulator,
    warnings: &mut Vec<PreparedTemplateWarning>,
) {
    for warning in std::mem::take(warnings) {
        accumulator.append_prepared(warning.batch, Some((warning.source_database, None)));
    }
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
            accumulator.append_infrastructure(errors, source_database);
        }
        PreparedFailure::Mixed { batch, error } => {
            accumulator.append_prepared(
                batch,
                source_database
                    .as_ref()
                    .map(|database| (Arc::clone(database), None)),
            );
            accumulator.append_infrastructure(error, source_database);
        }
    }
}
