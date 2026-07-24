//! HTML project builder orchestration.
//!
//! WHAT: coordinates module output-path resolution, homepage checks, and backend selection.
//! WHY: project builders own artifact assembly policy while compiler backends stay generic.
use crate::backends::backend_feature_validation::{
    BackendFeatureValidationError, BackendFeatureValidationInput, BackendFeatureValidationRoot,
    validate_hir_backend_feature_support,
};
use crate::backends::external_package_validation::{
    BackendTarget, ExternalPackageValidationError, validate_hir_external_package_support,
};
use crate::build_system::build::{BackendBuilder, CleanupPolicy, Module, OutputFile, Project};
use crate::builder_surface::{BuilderSurface, SourceFileKind};
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::style_directives::StyleDirectiveSpec;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::binding_packages::web::canvas::register_web_canvas_package;
use crate::projects::html_project::compile_input::HtmlModuleCompileInput;
use crate::projects::html_project::diagnostics::{
    duplicate_html_output_path_messages, tracked_asset_builder_output_conflict_messages,
    tracked_asset_output_conflict_messages,
};
use crate::projects::html_project::document_config::parse_html_document_config;
use crate::projects::html_project::external_js::js_import_provider::JsExternalImportProvider;
use crate::projects::html_project::external_js::runtime_assets::emit_external_js_runtime_assets;
use crate::projects::html_project::external_js::runtime_emission_plan::HtmlExternalRuntimeEmissionPlan;
use crate::projects::html_project::external_js::runtime_glue::emit_build_runtime_modules;
use crate::projects::html_project::js_path::{compile_html_module_js, html_output_path};
use crate::projects::html_project::path_policy::HtmlEntryPathPlan;
use crate::projects::html_project::style_directives::html_project_style_directives;
use crate::projects::html_project::tracked_assets::{
    emit_tracked_assets, plan_module_tracked_assets,
};
use crate::projects::html_project::wasm::artifacts::{
    CompiledHtmlWasmModule, compile_html_module_wasm,
};
use crate::projects::html_project::wasm::export_plan::build_html_wasm_export_plan;
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
    /// WHAT: used by the integration test runner so test fixtures can import
    ///       `@test/pkg-a` and `@test/pkg-b` symbols.
    pub fn for_integration_tests() -> Self {
        Self {
            include_test_packages: true,
        }
    }
}

impl BackendBuilder for HtmlProjectBuilder {
    fn build_backend(
        &self,
        modules: Vec<Module>,
        config: &Config,
        flags: &[Flag],
        string_table: &mut StringTable,
    ) -> Result<Project, CompilerMessages> {
        // Record the full backend build duration on every exit path (success or error).
        let _total_guard = crate::timing::PipelineTimingGuard::new("backend.html.total");

        {
            let _site_config_guard =
                crate::timing::PipelineTimingGuard::new("backend.html.site_config");
            parse_html_site_config(config, string_table)
                .map_err(|error| error.into_messages(string_table.clone()))?;
        }

        let document_config = {
            let _document_config_guard =
                crate::timing::PipelineTimingGuard::new("backend.html.document_config");
            parse_html_document_config(config, string_table)
                .map_err(|error| error.into_messages(string_table.clone()))?
        };

        if modules.is_empty() {
            return Err(CompilerMessages::from_error(
                CompilerError::compiler_error(
                    "HTML builder expected at least one compiled module but got 0.",
                ),
                string_table.clone(),
            ));
        }

        let release_build = flags.contains(&Flag::Release);
        let wasm_enabled = flags.contains(&Flag::HtmlWasm);
        let entry_paths = {
            let _entry_path_guard =
                crate::timing::PipelineTimingGuard::new("backend.html.entry_path_plan");
            HtmlEntryPathPlan::from_config(config, string_table)?
        };

        let mut output_files = Vec::new();
        let mut output_paths = HashSet::new();
        let mut output_path_owners: HashMap<PathBuf, PathBuf> = HashMap::new();
        let mut entry_page_rel = None;
        let mut has_directory_homepage = false;
        let artifact_modules: Vec<&Module> = modules
            .iter()
            .filter(|module| module.metadata.root_activity.has_html_artifact_activity())
            .collect();
        let mut compiled_html_output_paths = Vec::with_capacity(artifact_modules.len());
        let mut warnings = Vec::new();

        {
            let _module_compile_guard =
                crate::timing::PipelineTimingGuard::new("backend.html.module_compile_total");
            for module in artifact_modules.iter().copied() {
                // Derive the canonical page route once. Both JS-only and HTML+Wasm output modes
                // consume this same path — downstream code must not re-derive route semantics.
                let logical_html_output_path = html_output_path(
                    &module.metadata.entry_point,
                    entry_paths.resolved_entry_root.as_deref(),
                    string_table,
                )
                .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;

                let compiled_artifacts = self.compile_one_module(
                    module,
                    &logical_html_output_path,
                    config.project_name.as_str(),
                    &document_config,
                    release_build,
                    wasm_enabled,
                    string_table,
                )?;

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
                    output_path_owners
                        .insert(output_path.clone(), module.metadata.entry_point.clone());
                    output_files.push(output_file);
                }
                compiled_html_output_paths.push((module, html_output_path.clone()));

                if entry_paths.is_homepage_entry(&module.metadata.entry_point) {
                    has_directory_homepage = true;
                    entry_page_rel = Some(html_output_path.clone());
                } else if !entry_paths.is_directory_build() && entry_page_rel.is_none() {
                    entry_page_rel = Some(html_output_path);
                }
            }
        }

        entry_paths.require_homepage_if_directory_build(
            config,
            has_directory_homepage,
            string_table,
        )?;

        let runtime_emission_plan =
            HtmlExternalRuntimeEmissionPlan::from_modules(artifact_modules.iter().copied());

        {
            let _runtime_assets_guard =
                crate::timing::PipelineTimingGuard::new("backend.html.external_runtime_assets");
            output_files.extend(emit_external_js_runtime_assets(
                &runtime_emission_plan,
                &mut output_paths,
                string_table,
            )?);
        }

        {
            let _runtime_glue_guard =
                crate::timing::PipelineTimingGuard::new("backend.html.external_runtime_glue");
            output_files.extend(emit_build_runtime_modules(
                &runtime_emission_plan,
                &mut output_paths,
                string_table,
            )?);
        }

        let mut tracked_assets = Vec::new();
        let mut tracked_asset_sources_by_output: HashMap<PathBuf, PathBuf> = HashMap::new();
        {
            let _tracked_assets_plan_guard =
                crate::timing::PipelineTimingGuard::new("backend.html.tracked_assets_plan");
            for (module, html_output_path) in &compiled_html_output_paths {
                let planned_assets =
                    plan_module_tracked_assets(module, html_output_path, string_table)?;
                warnings.extend(planned_assets.warnings);

                for asset in planned_assets.assets {
                    let output_path = asset.emitted_output_path.clone();

                    if let Some(existing_source) = tracked_asset_sources_by_output.get(&output_path)
                    {
                        if *existing_source == asset.source_filesystem_path {
                            continue;
                        }

                        return Err(conflicting_tracked_asset_output_error(
                            &asset.source_filesystem_path,
                            existing_source,
                            &output_path,
                            string_table,
                        ));
                    }

                    if !output_paths.insert(output_path.clone()) {
                        return Err(tracked_asset_conflicts_with_existing_output_error(
                            &asset.source_filesystem_path,
                            &output_path,
                            string_table,
                        ));
                    }

                    tracked_asset_sources_by_output
                        .insert(output_path.clone(), asset.source_filesystem_path.clone());
                    tracked_assets.push(asset);
                }
            }
        }
        {
            let _tracked_assets_emit_guard =
                crate::timing::PipelineTimingGuard::new("backend.html.tracked_assets_emit");
            output_files.extend(emit_tracked_assets(&tracked_assets, string_table)?);
        }

        Ok(Project {
            output_files,
            entry_page_rel,
            cleanup_policy: CleanupPolicy::html(),
            warnings,
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

        // Empty dev/release folders are allowed and resolved by core build output logic.
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
    #[allow(clippy::too_many_arguments)]
    fn compile_one_module(
        &self,
        module: &Module,
        logical_html_output_path: &Path,
        project_name: &str,
        document_config: &crate::projects::html_project::document_config::HtmlDocumentConfig,
        release_build: bool,
        wasm_enabled: bool,
        string_table: &mut StringTable,
    ) -> Result<CompiledHtmlModuleArtifacts, CompilerMessages> {
        // Validate that every external function call in the HIR has lowering metadata for the
        // target backend. WHY: fail early with a structured Rule error at the call site rather
        // than a vague backend-internal error during lowering.
        let backend_target = if wasm_enabled {
            BackendTarget::Wasm
        } else {
            BackendTarget::Js
        };
        validate_hir_external_package_support(
            &module.executable.hir,
            module.link_facts.external_package_registry.as_ref(),
            backend_target,
            string_table,
        )
        .map_err(|error| match error {
            ExternalPackageValidationError::Diagnostic(diagnostic) => {
                CompilerMessages::from_diagnostic_ref(*diagnostic, string_table)
            }
            ExternalPackageValidationError::Infrastructure(error) => {
                CompilerMessages::from_error_ref(*error, string_table)
            }
        })?;

        let backend_validation_root = if wasm_enabled {
            // HTML-Wasm validates from the functions exported by the builder so dead helper bodies
            // do not surface backend diagnostics for code the page never executes.
            let export_plan = build_html_wasm_export_plan(&module.executable.hir)
                .map_err(|error| CompilerMessages::from_error(error, string_table.clone()))?;
            BackendFeatureValidationRoot::ExplicitRoots(
                export_plan
                    .function_exports
                    .iter()
                    .map(|function_export| function_export.function_id)
                    .collect(),
            )
        } else {
            BackendFeatureValidationRoot::StartFunction
        };

        validate_hir_backend_feature_support(
            BackendFeatureValidationInput {
                hir: &module.executable.hir,
                target: backend_target,
                root: backend_validation_root,
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

        let compile_input = HtmlModuleCompileInput {
            hir_module: &module.executable.hir,
            type_environment: &module.executable.type_environment,
            const_fragments: &module.metadata.const_top_level_fragments,
            borrow_analysis: &module.executable.borrow_analysis,
            project_name,
            document_config,
            release_build,
            root_activity: &module.metadata.root_activity,
            external_package_registry: Arc::clone(&module.link_facts.external_package_registry),
        };
        if wasm_enabled {
            let compiled_wasm =
                compile_html_module_wasm(&compile_input, string_table, logical_html_output_path)?;
            Ok(CompiledHtmlModuleArtifacts::from_wasm(compiled_wasm))
        } else {
            let compiled_js = compile_html_module_js(
                module,
                &compile_input,
                string_table,
                logical_html_output_path.to_path_buf(),
            )?;
            Ok(CompiledHtmlModuleArtifacts {
                output_files: compiled_js.output_files,
                html_output_path: compiled_js.html_output_path,
            })
        }
    }
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

fn conflicting_tracked_asset_output_error(
    source_path: &Path,
    existing_source_path: &Path,
    output_path: &Path,
    string_table: &mut StringTable,
) -> CompilerMessages {
    tracked_asset_output_conflict_messages(
        source_path,
        existing_source_path,
        output_path,
        string_table,
    )
}

fn tracked_asset_conflicts_with_existing_output_error(
    source_path: &Path,
    output_path: &Path,
    string_table: &mut StringTable,
) -> CompilerMessages {
    tracked_asset_builder_output_conflict_messages(source_path, output_path, string_table)
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
