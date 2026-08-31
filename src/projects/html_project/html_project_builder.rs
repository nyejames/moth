//! HTML project builder orchestration.
//!
//! WHAT: coordinates module output-path resolution, homepage checks, and backend selection.
//! WHY: project builders own artifact assembly policy while compiler backends stay generic.
use crate::backends::backend_feature_validation::{
    BackendFeatureValidationError, BackendFeatureValidationInput,
    validate_hir_backend_feature_support,
};
use crate::backends::external_package_validation::{
    BackendTarget, validate_hir_external_package_support,
};
use crate::build_system::BuildProfile;
use crate::build_system::build::{
    BackendBuilder, DeferredResourceOutput, FileKind, OutputFile, Project, ProjectCompilation,
    ProjectEntry,
};
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::build_system::output::{BuilderKind, CleanupPolicy};
use crate::builder_surface::{BuilderSurface, SourceFileKind};
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, ErrorType};
use crate::compiler_frontend::paths::resource_identity::StableResourceOwnerId;
use crate::compiler_frontend::style_directives::StyleDirectiveSpec;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::binding_packages::web::canvas::register_web_canvas_package;
use crate::projects::html_project::compile_input::{
    HtmlModuleCompileContext, HtmlModuleCompileInput,
};
use crate::projects::html_project::diagnostics::{
    duplicate_html_output_path_messages, resource_output_path_reserved_messages,
};
use crate::projects::html_project::document_config::parse_html_document_config;
use crate::projects::html_project::external_js::js_import_provider::JsExternalImportProvider;
use crate::projects::html_project::external_js::runtime_assets::register_js_runtime_asset_sources;
use crate::projects::html_project::external_js::runtime_emission_plan::HtmlExternalRuntimeEmissionPlan;
use crate::projects::html_project::external_js::runtime_glue::{
    emit_build_runtime_modules, planned_runtime_module_output_paths,
};
use crate::projects::html_project::js_path::{
    HtmlJsCompileInput, compile_html_module_js, html_output_path,
};
use crate::projects::html_project::output_plan::plan_wasm_output_from_logical_html_path;
use crate::projects::html_project::page_metadata::extract_html_page_metadata;
use crate::projects::html_project::path_policy::HtmlEntryPathPlan;
use crate::projects::html_project::resource_output_plan::{
    HtmlResourceOutputPlan, ResourceUrlContext, display_origin,
};
use crate::projects::html_project::structural_url_renderer::StructuralUrlRenderer;
use crate::projects::html_project::style_directives::html_project_style_directives;
use crate::projects::html_project::wasm::artifacts::{
    CompiledHtmlWasmModule, compile_html_module_wasm,
};
use crate::projects::routing::parse_html_site_config;
use crate::projects::settings::{Config, ProjectConfigError};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const HTML_SOURCE_PACKAGE_PREFIX: &str = "html";

#[derive(Debug)]
pub struct HtmlProjectBuilder {
    include_test_packages: bool,
}

impl HtmlProjectBuilder {
    /// Constructs the HTML project builder.
    ///
    /// WHAT: initializes a stateless builder implementation.
    /// WHY: builder policy is encoded in methods rather than runtime state.
    pub fn new() -> Self {
        Self {
            include_test_packages: false,
        }
    }

    /// Constructs a builder that includes integration-test external packages.
    ///
    /// WHAT: used by the integration test runner so test fixtures can bind
    ///       `@test/pkg-a` and `@test/pkg-b` symbols.
    pub fn for_integration_tests() -> Self {
        Self {
            include_test_packages: true,
        }
    }
}

impl BackendBuilder for HtmlProjectBuilder {
    fn builder_kind(&self) -> BuilderKind {
        BuilderKind::Html
    }

    fn build_backend(
        &self,
        mut project_compilation: ProjectCompilation,
        config: &Config,
        build_profile: BuildProfile,
        flags: &[Flag],
        string_table: &mut StringTable,
    ) -> Result<Project, CompilerMessages> {
        let site_config = parse_html_site_config(config, string_table)
            .map_err(|error| error.into_messages(string_table.clone()))?;

        let document_config = parse_html_document_config(config, string_table)
            .map_err(|error| error.into_messages(string_table.clone()))?;

        if project_compilation.module_count() == 0 {
            return Err(CompilerMessages::from_error(
                CompilerError::compiler_error(
                    "HTML builder expected at least one compiled module but got 0.",
                ),
                string_table.clone(),
            ));
        }

        let wasm_enabled = flags.contains(&Flag::HtmlWasm);
        let entry_paths = HtmlEntryPathPlan::from_config(config, string_table)?;
        let mut resource_inputs = project_compilation.take_resource_inputs();

        let mut output_files = Vec::new();
        let mut output_paths = HashSet::new();
        let mut output_path_owners: HashMap<PathBuf, PathBuf> = HashMap::new();
        let mut resource_output_plan = HtmlResourceOutputPlan::new(config.project_name.as_str());
        let mut entry_page_rel = None;
        let mut has_directory_homepage = false;
        let artifact_entries = project_compilation.entries();
        for entry in artifact_entries.iter().cloned() {
            let module = entry.module;
            // Derive the canonical page route once. Both JS-only and HTML+Wasm output modes
            // consume this same path — downstream code must not re-derive route semantics.
            let logical_html_output_path = html_output_path(
                &module.metadata.entry_point,
                entry_paths.resolved_entry_root.as_deref(),
                string_table,
            )
            .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;
            let wasm_route_plan = if wasm_enabled {
                Some(
                    plan_wasm_output_from_logical_html_path(&logical_html_output_path)
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?,
                )
            } else {
                None
            };
            let planned_html_output_path = wasm_route_plan
                .as_ref()
                .map_or(logical_html_output_path.as_path(), |plan| {
                    plan.html_path.as_path()
                });
            let start_function = module
                .executable
                .hir
                .require_start_function("HTML page metadata extraction")
                .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;
            let page_metadata_plan = extract_html_page_metadata(
                &module.executable.hir,
                start_function,
                &module.executable.resource_table,
                string_table,
            )
            .map_err(|diagnostic| {
                CompilerMessages::from_diagnostic_ref(*diagnostic, string_table)
            })?;
            resource_output_plan.plan_entry(
                &entry,
                planned_html_output_path,
                &page_metadata_plan,
                string_table,
            )?;
            if let Some(route_plan) = &wasm_route_plan {
                if let Some(js_path) = &route_plan.js_path {
                    resource_output_plan.reserve_builder_output_path(
                        js_path,
                        "JavaScript",
                        string_table,
                    )?;
                }
                if let Some(wasm_path) = &route_plan.wasm_path {
                    resource_output_plan.reserve_builder_output_path(
                        wasm_path,
                        "Wasm",
                        string_table,
                    )?;
                }
            }
            let resource_url_context =
                ResourceUrlContext::PageDocument(planned_html_output_path.to_path_buf());
            let structural_url_renderer = StructuralUrlRenderer::new(
                &resource_output_plan,
                &resource_url_context,
                site_config.origin.as_str(),
            );

            let compiled_artifacts = self.compile_one_module(
                HtmlModuleCompileContext {
                    entry,
                    page_metadata_plan: &page_metadata_plan,
                    logical_html_output_path: &logical_html_output_path,
                    structural_url_renderer: &structural_url_renderer,
                    project_name: config.project_name.as_str(),
                    document_config: &document_config,
                    build_profile,
                    wasm_enabled,
                },
                string_table,
            )?;
            for output_file in &compiled_artifacts.output_files {
                let artefact_kind = match output_file.file_kind() {
                    FileKind::Html(_) => "HTML page",
                    FileKind::Js(_) => "JavaScript",
                    FileKind::Wasm(_) => "Wasm",
                    FileKind::Bytes(_) | FileKind::NotBuilt | FileKind::Directory => "builder",
                };
                resource_output_plan.reserve_builder_output_path(
                    output_file.relative_output_path(),
                    artefact_kind,
                    string_table,
                )?;
            }

            let html_output_path = compiled_artifacts.html_output_path.clone();
            for output_file in compiled_artifacts.output_files {
                let output_path = output_file.relative_output_path().to_path_buf();
                if let Some(existing_entry_point) = output_path_owners.get(&output_path) {
                    return Err(duplicate_output_path_error(
                        &module.metadata.entry_point,
                        existing_entry_point,
                        &output_path,
                        string_table,
                    ));
                }
                output_paths.insert(output_path.clone());
                output_path_owners.insert(output_path.clone(), module.metadata.entry_point.clone());
                output_files.push(output_file);
            }

            if entry_paths.is_homepage_entry(&module.metadata.entry_point) {
                has_directory_homepage = true;
                entry_page_rel = Some(html_output_path.clone());
            } else if !entry_paths.is_directory_build() && entry_page_rel.is_none() {
                entry_page_rel = Some(html_output_path);
            }
        }

        entry_paths.require_homepage_if_directory_build(
            config,
            has_directory_homepage,
            string_table,
        )?;

        let runtime_emission_plan = HtmlExternalRuntimeEmissionPlan::from_import_sets(
            artifact_entries.iter().map(|entry| entry.external_imports),
        );

        // Provider JS assets become deferred resource outputs. Their canonical byte sources
        // attach to the shared registry and their declared destinations join the resource
        // plan, so a collision with a page resource fails naming both semantic origins.
        register_js_runtime_asset_sources(&runtime_emission_plan, &mut resource_inputs)
            .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?;
        for asset in runtime_emission_plan.js_assets().values() {
            resource_output_plan.plan_provider_runtime_asset(
                asset.origin.clone(),
                asset.authored_import_location.clone(),
                string_table,
            )?;
        }
        let runtime_module_output_paths =
            planned_runtime_module_output_paths(runtime_emission_plan.runtime_module_specifiers());
        for output_path in runtime_module_output_paths {
            resource_output_plan.reserve_builder_output_path(
                &output_path,
                "JavaScript",
                string_table,
            )?;
        }

        output_files.extend(emit_build_runtime_modules(
            &runtime_emission_plan,
            &mut output_paths,
            string_table,
        )?);
        let deferred_resources = emit_planned_resource_outputs(
            resource_output_plan,
            &resource_inputs,
            &output_files,
            &output_paths,
            string_table,
        )?;

        Ok(Project {
            output_files,
            entry_page_rel,
            cleanup_policy: CleanupPolicy::html(),
            warnings: Vec::new(),
            deferred_resources,
            resource_inputs,
        })
    }

    fn validate_project_config(
        &self,
        config: &Config,
        string_table: &mut StringTable,
    ) -> Result<(), ProjectConfigError> {
        // Validate HTML-specific configuration up front so build/dev runtime behavior stays
        // deterministic and all routing-policy mistakes are surfaced as config errors.
        parse_html_site_config(config, string_table)?;
        parse_html_document_config(config, string_table)?;

        Ok(())
    }

    fn frontend_style_directives(&self) -> Vec<StyleDirectiveSpec> {
        html_project_style_directives()
    }

    fn frontend_surface(&self) -> BuilderSurface {
        let mut builder_surface = BuilderSurface::with_mandatory_core();
        builder_surface.source_packages.register_filesystem_root(
            HTML_SOURCE_PACKAGE_PREFIX,
            BuilderSurface::builtin_source_package_root(HTML_SOURCE_PACKAGE_PREFIX),
            crate::builder_surface::PackageOrigin::Builder,
        );
        builder_surface.register_implicit_template_scope_source_package(HTML_SOURCE_PACKAGE_PREFIX);

        builder_surface.expose_html_core_packages();

        let canvas_metadata = register_web_canvas_package(&mut builder_surface.binding_packages);
        builder_surface
            .builder_runtime_packages
            .push(canvas_metadata);

        Self::register_html_config_keys(&mut builder_surface);

        builder_surface.source_file_kinds.register(
            SourceFileKind::MothTemplate.extension(),
            SourceFileKind::MothTemplate,
        );
        builder_surface.source_file_kinds.register(
            SourceFileKind::PlainMarkdown.extension(),
            SourceFileKind::PlainMarkdown,
        );

        builder_surface
            .external_import_providers
            .register(std::sync::Arc::new(JsExternalImportProvider::new()));

        if self.include_test_packages {
            builder_surface.binding_packages = builder_surface
                .binding_packages
                .with_test_packages_for_integration();
        }

        builder_surface
    }
}

impl HtmlProjectBuilder {
    /// Register HTML-backend-specific config keys into the builder surface's key registry.
    ///
    /// WHY: Stage 0 config loading must know which keys are valid before backend semantic
    /// validation runs. Keeping registration here keeps HTML-specific meaning out of the core.
    fn register_html_config_keys(builder_surface: &mut BuilderSurface) {
        let registry = &mut builder_surface.config_keys;

        // Routing / site keys
        registry.register_backend_string("origin");
        registry.register_backend_string("page_url_style");
        registry.register_backend_bool("redirect_index_html");

        // HTML document shell keys
        registry.register_backend_string("html_lang");
        registry.register_backend_string("html_title_prefix");
        registry.register_backend_string("html_title_postfix");
        registry.register_backend_string("html_favicon");
        registry.register_backend_bool("html_inject_charset");
        registry.register_backend_bool("html_inject_viewport");
        registry.register_backend_bool("html_inject_color_scheme");
        registry.register_backend_bool("html_inject_core_css");
        registry.register_backend_string("html_body_style");
    }

    /// Compile one module through the appropriate builder path (JS-only or HTML+Wasm).
    fn compile_one_module(
        &self,
        context: HtmlModuleCompileContext<'_>,
        string_table: &mut StringTable,
    ) -> Result<CompiledHtmlModuleArtifacts, CompilerMessages> {
        let HtmlModuleCompileContext {
            entry:
                ProjectEntry {
                    module,
                    reachability,
                    external_imports,
                    linked_modules,
                    source_function_names,
                    module_private_function_names,
                    generated_function_names,
                    all_generated_function_names,
                    ..
                },
            page_metadata_plan,
            logical_html_output_path,
            structural_url_renderer,
            project_name,
            document_config,
            build_profile,
            wasm_enabled,
        } = context;

        // Validate that every selected external call has lowering metadata for the target.
        // WHY: fail early with a structured Rule error at the call site rather than a vague
        // backend-internal error during lowering.
        let backend_target = if wasm_enabled {
            BackendTarget::Wasm
        } else {
            BackendTarget::Js
        };
        validate_hir_external_package_support(
            reachability,
            module.link_facts.external_package_registry.as_ref(),
            backend_target,
            string_table,
        )
        .map_err(|diagnostic| CompilerMessages::from_diagnostic_ref(*diagnostic, string_table))?;

        validate_hir_backend_feature_support(
            BackendFeatureValidationInput {
                hir: &module.executable.hir,
                reachability,
                target: backend_target,
                type_environment: Some(&module.executable.type_environment),
            },
            string_table,
        )
        .map_err(|error| match error {
            BackendFeatureValidationError::Diagnostic(diagnostic) => {
                CompilerMessages::from_diagnostic_ref(*diagnostic, string_table)
                    .with_type_context_for_all_diagnostics(
                        module.executable.type_environment.clone(),
                    )
            }
            BackendFeatureValidationError::Infrastructure(error) => {
                CompilerMessages::from_error_ref(*error, string_table)
            }
        })?;

        for linked in &linked_modules {
            validate_hir_external_package_support(
                linked.reachability,
                linked.module.link_facts.external_package_registry.as_ref(),
                backend_target,
                string_table,
            )
            .map_err(|diagnostic| {
                CompilerMessages::from_diagnostic_ref(*diagnostic, string_table)
            })?;
            validate_hir_backend_feature_support(
                BackendFeatureValidationInput {
                    hir: &linked.module.executable.hir,
                    reachability: linked.reachability,
                    target: backend_target,
                    type_environment: Some(&linked.module.executable.type_environment),
                },
                string_table,
            )
            .map_err(|error| match error {
                BackendFeatureValidationError::Diagnostic(diagnostic) => {
                    CompilerMessages::from_diagnostic_ref(*diagnostic, string_table)
                        .with_type_context_for_all_diagnostics(
                            linked.module.executable.type_environment.clone(),
                        )
                }
                BackendFeatureValidationError::Infrastructure(error) => {
                    CompilerMessages::from_error_ref(*error, string_table)
                }
            })?;
        }

        let compile_input = HtmlModuleCompileInput {
            hir_module: &module.executable.hir,
            resource_table: &module.executable.resource_table,
            reachability,
            type_environment: &module.executable.type_environment,
            const_fragments: &module.metadata.const_top_level_fragments,
            page_metadata_plan,
            borrow_analysis: &module.executable.borrow_analysis,
            project_name,
            document_config,
            build_profile,
            root_activity: &module.metadata.root_activity,
            external_package_registry: Arc::clone(&module.link_facts.external_package_registry),
        };
        if wasm_enabled {
            let compiled_wasm = compile_html_module_wasm(
                &compile_input,
                string_table,
                logical_html_output_path,
                structural_url_renderer,
            )?;
            Ok(CompiledHtmlModuleArtifacts::from_wasm(compiled_wasm))
        } else {
            let compiled_js = compile_html_module_js(
                HtmlJsCompileInput {
                    module,
                    external_imports,
                    linked_modules: &linked_modules,
                    source_function_names,
                    module_private_function_names,
                    generated_function_names,
                    all_generated_function_names,
                    compile_input: &compile_input,
                    structural_url_renderer,
                    output_path: logical_html_output_path.to_path_buf(),
                },
                string_table,
            )?;
            Ok(CompiledHtmlModuleArtifacts {
                output_files: compiled_js.output_files,
                html_output_path: compiled_js.html_output_path,
            })
        }
    }
}

/// Resolve the byte-free resource plan into deferred destinations after all builder output paths
/// are reserved.
///
/// Every planned origin must carry an explicit Stage 0 or provider source attachment before the
/// writer may materialise it, so a planned output with no registered source is an internal
/// invariant violation rather than a guessable filesystem source.
fn emit_planned_resource_outputs(
    resource_output_plan: HtmlResourceOutputPlan,
    resource_inputs: &ResourceInputRegistry,
    output_files: &[OutputFile],
    output_paths: &HashSet<PathBuf>,
    string_table: &mut StringTable,
) -> Result<Vec<DeferredResourceOutput>, CompilerMessages> {
    let planned_resource_outputs = resource_output_plan.into_records();
    let mut pending_output_paths = HashSet::with_capacity(planned_resource_outputs.len());
    let mut deferred_resources = Vec::with_capacity(planned_resource_outputs.len());

    // Resolve every source attachment and reserve every destination before touching resource IO.
    for record in planned_resource_outputs {
        let Some(source_id) = resource_inputs.source_for_origin(&record.origin) else {
            let owner_kind = if matches!(record.origin.owner(), StableResourceOwnerId::Provider(_))
            {
                "provider-owned"
            } else {
                "module-owned"
            };

            let error = CompilerError::new(
                format!(
                    "planned {owner_kind} resource origin {:?} has no registered source \
                     attachment",
                    record.origin
                ),
                record.first_authored_location.clone(),
                ErrorType::Compiler,
            );
            return Err(CompilerMessages::from_error_ref(error, string_table));
        };
        if output_paths.contains(&record.output_path)
            || !pending_output_paths.insert(record.output_path.clone())
        {
            let artefact_kind = output_files
                .iter()
                .find(|output_file| output_file.relative_output_path() == record.output_path)
                .map(|output_file| match output_file.file_kind() {
                    FileKind::Html(_) => "HTML page",
                    FileKind::Js(_) => "JavaScript",
                    FileKind::Wasm(_) => "Wasm",
                    FileKind::Bytes(_) | FileKind::NotBuilt | FileKind::Directory => "builder",
                })
                .unwrap_or("builder");
            return Err(resource_output_path_reserved_messages(
                &record.output_path,
                &display_origin(&record.origin),
                artefact_kind,
                &record.first_authored_location,
                string_table,
            ));
        }

        deferred_resources.push(DeferredResourceOutput {
            relative_output_path: record.output_path,
            source_id,
        });
    }

    Ok(deferred_resources)
}

fn duplicate_output_path_error(
    duplicate_entry_point: &Path,
    existing_entry_point: &Path,
    output_path: &Path,
    string_table: &mut StringTable,
) -> CompilerMessages {
    duplicate_html_output_path_messages(
        duplicate_entry_point,
        existing_entry_point,
        output_path,
        string_table,
    )
}

struct CompiledHtmlModuleArtifacts {
    /// Full emitted output set for one module (HTML only or HTML+Wasm trio).
    output_files: Vec<OutputFile>,
    /// HTML entry path used for homepage selection and serving/open behavior.
    html_output_path: PathBuf,
}

impl CompiledHtmlModuleArtifacts {
    /// Wraps Wasm-mode output into the builder's common artifact shape.
    fn from_wasm(compiled_wasm: CompiledHtmlWasmModule) -> Self {
        // Keep the debug struct alive through compilation so toggles can expose it without
        // changing external interfaces.
        let _debug = compiled_wasm.debug;
        Self {
            output_files: compiled_wasm.output_files,
            html_output_path: compiled_wasm.html_output_path,
        }
    }
}

#[cfg(test)]
#[path = "tests/html_project_builder_tests.rs"]
mod tests;
