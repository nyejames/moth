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
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::folded_value::OwnedFoldedString;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::single_source_compilation::{
    MothTemplateCompilationRequest, compile_moth_template_source,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::moth_template::bundle::{
    DIRECT_TEMPLATE_PROJECT_NAME, prepare_file_value_bundle,
};
use crate::projects::html_project::moth_template::input::{
    MothTemplateCompileRequest, MothTemplateSourceUnit,
};
use crate::projects::html_project::moth_template::output::{
    CompiledMothTemplateDocument, MothTemplateCompileOutput,
};
use crate::projects::html_project::moth_template::render::{
    document_url_context, render_structural_content,
};
use crate::projects::html_project::resource_output_plan::{
    HtmlResourceOutputPlan, PlannedResourceOutput,
};
use crate::projects::html_project::style_directives::html_project_style_directives;
use std::path::Path;

pub(crate) fn compile_moth_template(
    request: MothTemplateCompileRequest,
    string_table: &mut StringTable,
) -> Result<MothTemplateCompileOutput, CompilerMessages> {
    let mut resource_inputs = ResourceInputRegistry::new();
    let compiled =
        compile_moth_template_with_registry(request, string_table, &mut resource_inputs)?;

    Ok(MothTemplateCompileOutput {
        documents: compiled.documents,
        resources: compiled.resources,
        resource_inputs,
        warnings: compiled.warnings,
    })
}

/// Documents, planned outputs and warnings produced against a caller-owned registry.
#[derive(Debug)]
pub(crate) struct DirectTemplateRegistryCompile {
    documents: Vec<CompiledMothTemplateDocument>,
    resources: Vec<DeferredResourceOutput>,
    warnings: Vec<CompilerDiagnostic>,
}

/// Compile one request against a caller-owned registry so a failed plan still leaves its
/// registered sources inspectable.
pub(crate) fn compile_moth_template_with_registry(
    request: MothTemplateCompileRequest,
    string_table: &mut StringTable,
    resource_inputs: &mut ResourceInputRegistry,
) -> Result<DirectTemplateRegistryCompile, CompilerMessages> {
    let sources = request.collect_sources(string_table)?;

    // The project's directive vocabulary is the same for every source in one request, so it is
    // merged once rather than per document.
    let style_directives = StyleDirectiveRegistry::merged(&html_project_style_directives())
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;

    let mut documents = Vec::with_capacity(sources.len());
    let mut warnings = Vec::new();
    let mut resource_plan = HtmlResourceOutputPlan::new(DIRECT_TEMPLATE_PROJECT_NAME);

    for unit in sources {
        let MothTemplateSourceUnit {
            source_path,
            relative_path,
            source_text,
        } = &unit;

        // Every warning gathered so far belongs to the report in source order: earlier documents
        // first, then this one's.
        let step_warnings = warnings.clone();

        // Physical file-reference resolution and the content closure are build policy; the
        // compiler service folds the prepared bundle without touching the filesystem.
        let file_value_bundle = report_with_prior_warnings(
            prepare_file_value_bundle(&unit, &style_directives, string_table, resource_inputs),
            &step_warnings,
        )?;

        let folded = report_with_prior_warnings(
            compile_moth_template_source(
                MothTemplateCompilationRequest {
                    source_path: source_path.as_path(),
                    source_code: source_text.clone(),
                    style_directives: &style_directives,
                    file_value_resolution: Some(file_value_bundle),
                },
                string_table,
            ),
            &step_warnings,
        )?;
        let folded_warnings = folded.warnings;

        // The fold's compiler-produced associations name sources this same registry issued, so
        // publication must resolve; a missing attachment would silently drop the resource.
        report_with_prior_warnings(
            publish_module_resource_associations(
                resource_inputs,
                &folded.module_resources,
                string_table,
            ),
            &step_warnings,
        )?;

        let document_path = report_with_prior_warnings(
            document_url_context(relative_path.as_deref(), source_path, string_table),
            &documents_step_warnings(&step_warnings, &folded_warnings),
        )?;
        let content = report_with_prior_warnings(
            render_document_content(
                &folded.content,
                &folded.module_resources,
                &document_path,
                &mut resource_plan,
                string_table,
            ),
            &documents_step_warnings(&step_warnings, &folded_warnings),
        )?;

        warnings.extend(folded_warnings);
        documents.push(CompiledMothTemplateDocument {
            source_path: source_path.clone(),
            relative_path: relative_path.clone(),
            content,
        });
    }

    let resources = report_with_prior_warnings(
        deferred_resource_outputs(resource_inputs, resource_plan.into_records(), string_table),
        &warnings,
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
    string_table: &mut StringTable,
) -> Result<(), CompilerMessages> {
    // The fold produced this association batch wholesale, so publish it in the one
    // preflight -> reserve -> commit order the shared registry requires.
    let associations = module_resources.resource_source_associations();
    let publication = resource_inputs
        .preflight_resource_source_associations(associations)
        .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;

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
    string_table: &mut StringTable,
) -> Result<Vec<DeferredResourceOutput>, CompilerMessages> {
    let mut deferred_resources = Vec::with_capacity(planned_resources.len());

    for planned in planned_resources {
        let Some(source_id) = resource_inputs.source_for_origin(&planned.origin) else {
            return Err(CompilerMessages::from_error(
                CompilerError::compiler_error(format!(
                    "planned direct-template resource origin {:?} has no registered source \
                     attachment",
                    planned.origin
                )),
                string_table.clone(),
            ));
        };

        deferred_resources.push(DeferredResourceOutput {
            relative_output_path: planned.output_path,
            source_id,
        });
    }

    Ok(deferred_resources)
}

/// The warnings that precede one step of this document in source order.
fn documents_step_warnings<'a>(
    step_warnings: &'a [CompilerDiagnostic],
    folded_warnings: &'a [CompilerDiagnostic],
) -> Vec<CompilerDiagnostic> {
    step_warnings
        .iter()
        .chain(folded_warnings)
        .cloned()
        .collect()
}

/// Prepend the warnings gathered for earlier documents to any failure of this step.
fn report_with_prior_warnings<T>(
    step: Result<T, CompilerMessages>,
    prior_warnings: &[CompilerDiagnostic],
) -> Result<T, CompilerMessages> {
    step.map_err(|mut messages| {
        messages.prepend_diagnostics_preserving_context(prior_warnings.iter().cloned());
        messages
    })
}

/// Render one folded template at its document URL context against the request-wide plan.
///
/// Plain text keeps the dedicated text fast path without any link planning. Structural pieces
/// add only the resource origins still present in the folded content to the shared plan, then
/// render through that same plan at this document's URL context.
fn render_document_content(
    content: &OwnedFoldedString,
    resources: &ModuleResourceTable,
    document_path: &Path,
    resource_plan: &mut HtmlResourceOutputPlan,
    string_table: &mut StringTable,
) -> Result<String, CompilerMessages> {
    match content {
        OwnedFoldedString::Text(text) => Ok(text.clone()),

        OwnedFoldedString::Pieces(_) => render_structural_content(
            content,
            resources,
            document_path,
            resource_plan,
            string_table,
        ),
    }
}
