//! HTML project builder orchestration.
//!
//! WHAT: coordinates module output-path resolution, homepage checks, and backend selection.
//! WHY: project builders own artifact assembly policy while compiler backends stay generic.
use crate::backends::backend_feature_validation::{
    BackendFeatureValidationInput, validate_hir_backend_feature_support,
    validate_hir_linked_function_targets,
};
use crate::backends::external_package_validation::{
    BackendTarget, validate_hir_external_package_support,
};
use crate::build_system::BuildProfile;
use crate::build_system::backend_build_context::BackendBuildContext;
use crate::build_system::build::{
    BackendBuilder, BackendInvocationFailure, DeferredResourceOutput, FileKind, OutputFile,
    Project, ProjectCompilation, ProjectEntry,
};
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::build_system::output::{BuilderIdentity, CleanupPolicy, register_output_root_fields};
use crate::builder_surface::config_schema::{
    ConfigSchema, ConfigSchemaField, NamedConfigSectionSchema, UnknownFieldPolicy,
};
use crate::builder_surface::{BuilderSurface, SourceFileKind};
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config;
use crate::compiler_frontend::compiler_messages::{DiagnosticDraft, PreparedFailure};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::paths::resource_identity::StableResourceOwnerId;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::FrozenIdentityHandle;
use crate::compiler_frontend::style_directives::StyleDirectiveSpec;
use crate::compiler_frontend::symbols::path_interner::PathTable;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::utilities::basic::portable_path_text;
use crate::projects::html_project::binding_packages::web::register_web_binding_packages;
use crate::projects::html_project::compile_input::{
    HtmlModuleCompileContext, HtmlModuleCompileInput,
};
use crate::projects::html_project::diagnostics::{
    capture_html_diagnostic, duplicate_html_output_path_diagnostic,
    resource_output_path_reserved_diagnostic,
};
use crate::projects::html_project::document_config::parse_html_document_config;
use crate::projects::html_project::external_js::js_import_provider::JsExternalImportProvider;
use crate::projects::html_project::external_js::runtime_assets::register_js_runtime_asset_sources;
use crate::projects::html_project::external_js::runtime_emission_plan::HtmlExternalRuntimeEmissionPlan;
use crate::projects::html_project::external_js::runtime_glue::{
    emit_build_runtime_modules, is_glue_module_output_path, planned_runtime_module_output_paths,
};
use crate::projects::html_project::js_path::{HtmlJsCompileInput, compile_html_module_js};
use crate::projects::html_project::output_plan::{
    LogicalHtmlPathError, derive_logical_html_path, plan_wasm_output_from_route,
};
use crate::projects::html_project::page_metadata::extract_html_page_metadata;
use crate::projects::html_project::path_policy::HtmlEntryPathPlan;
use crate::projects::html_project::resource_output_plan::{
    HtmlResourceOutputPlan, ResourceDiagnosticSite, ResourceUrlContext, display_origin,
};
use crate::projects::html_project::structural_url_renderer::StructuralUrlRenderer;
use crate::projects::html_project::style_directives::html_project_style_directives;
use crate::projects::html_project::wasm::artifacts::{
    CompiledHtmlWasmModule, compile_html_module_wasm,
};
use crate::projects::routing::parse_html_site_config;
use crate::projects::settings::{Config, ProjectConfigError};
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn generated_source_identity_handle(
    hir: &HirModule,
    source_domain: Option<&StablePackageIdentity>,
) -> Option<FrozenIdentityHandle> {
    hir.function_ids_by_generated
        .keys()
        .next()
        .map(|_| match source_domain {
            Some(domain) => FrozenIdentityHandle::for_domain(domain.clone()),
            None => FrozenIdentityHandle::new(),
        })
}

fn attach_module_source_identity(
    diagnostic: &mut DiagnosticDraft,
    hir: &HirModule,
    source_domain: Option<&StablePackageIdentity>,
) {
    if let Some(handle) = generated_source_identity_handle(hir, source_domain) {
        diagnostic.attach_frozen_identity_handle_if_missing(handle);
    }
}

fn capture_module_diagnostic(
    mut diagnostic: DiagnosticDraft,
    hir: &HirModule,
    source_domain: Option<&StablePackageIdentity>,
    type_environment: &TypeEnvironment,
    path_table: &Arc<PathTable>,
    root_string_table: &StringTable,
) -> PreparedFailure {
    attach_module_source_identity(&mut diagnostic, hir, source_domain);
    capture_html_diagnostic(
        diagnostic,
        root_string_table,
        Some(type_environment),
        || Some(Arc::clone(path_table)),
    )
}

const HTML_SOURCE_PACKAGE_PREFIX: &str = "html";
const WEB_SITE_SOURCE_PACKAGE_PREFIX: &str = "web/site";

#[derive(Debug, Default)]
pub struct HtmlProjectBuilder {
    include_test_packages: bool,
}

impl HtmlProjectBuilder {
    /// Constructs the production HTML project builder.
    ///
    /// WHAT: builds ordinary HTML projects without the integration-test packages.
    pub fn new() -> Self {
        Self {
            include_test_packages: false,
        }
    }

    /// Constructs an HTML builder for an integration case.
    ///
    /// WHAT: enables the integration-test packages for the case build.
    pub(crate) fn for_integration_tests() -> Self {
        Self {
            include_test_packages: true,
        }
    }
}

impl BackendBuilder for HtmlProjectBuilder {
    type Error = Infallible;

    fn builder_identity(&self) -> BuilderIdentity {
        BuilderIdentity::html()
    }

    fn build_backend(
        &self,
        mut project_compilation: ProjectCompilation,
        config: &Config,
        build_profile: BuildProfile,
        flags: &[Flag],
        backend_context: &mut BackendBuildContext,
    ) -> Result<Project, BackendInvocationFailure<Self::Error>> {
        let (site_config, document_config) = {
            let mut diagnostic_strings = backend_context.diagnostic_strings();
            let site_config = match parse_html_site_config(config, &mut diagnostic_strings) {
                Ok(site_config) => site_config,
                Err(error) => {
                    return Err(error
                        .into_prepared_failure(diagnostic_strings.strings())
                        .into());
                }
            };
            let document_config = match parse_html_document_config(config, &mut diagnostic_strings)
            {
                Ok(document_config) => document_config,
                Err(error) => {
                    return Err(error
                        .into_prepared_failure(diagnostic_strings.strings())
                        .into());
                }
            };
            (site_config, document_config)
        };

        if project_compilation.module_count() == 0 {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                "HTML builder expected at least one compiled module but got 0.",
            )
            .into());
        }

        let wasm_enabled = flags.contains(&Flag::HtmlWasm);
        let entry_paths = HtmlEntryPathPlan::from_config(config)?;
        let mut resource_inputs = project_compilation.take_resource_inputs();

        let mut output_files = Vec::new();
        let mut output_paths = HashSet::new();
        // Each emitted path maps to its first owning entry and its index in `output_files`.
        let mut output_path_owners: HashMap<PathBuf, (PathBuf, usize)> = HashMap::new();
        let mut resource_output_plan = HtmlResourceOutputPlan::new(config.project_name.as_str());
        let mut entry_page_rel = None;
        let mut has_directory_homepage = false;
        let artifact_entries = project_compilation.entries();
        for entry in artifact_entries.iter().cloned() {
            let module = entry.module;
            // Derive the canonical page route once. JS-only and HTML+Wasm output modes consume
            // projections from this same value — downstream code must not re-derive route semantics.
            let route = match derive_logical_html_path(
                &module.metadata.entry_point,
                entry_paths.resolved_entry_root.as_deref(),
            ) {
                Ok(route) => route,
                Err(LogicalHtmlPathError::Infrastructure(error)) => return Err(error.into()),
                Err(LogicalHtmlPathError::EmptyRouteName(entry_point)) => {
                    let mut diagnostic_strings = backend_context.diagnostic_strings();
                    let entry_path = crate::compiler_frontend::utilities::basic::portable_path_text(
                        &entry_point,
                    );
                    let entry_path_id = diagnostic_strings.intern(&entry_path);
                    let diagnostic =
                        project_config::html_single_file_route_name_empty(entry_path_id, None);
                    return Err(capture_html_diagnostic(
                        diagnostic,
                        diagnostic_strings.strings(),
                        None,
                        || None,
                    )
                    .into());
                }
            };
            let wasm_route_plan = wasm_enabled.then(|| plan_wasm_output_from_route(&route));
            let planned_html_output_path = wasm_route_plan
                .as_ref()
                .map_or(route.logical_html_path.as_path(), |plan| {
                    plan.html_path.as_path()
                });
            let start_function = module
                .executable
                .hir
                .require_start_function("HTML page metadata extraction")?;
            let page_metadata_plan = {
                let mut diagnostic_strings = backend_context.diagnostic_strings();
                match extract_html_page_metadata(
                    &module.executable.hir,
                    start_function,
                    &module.executable.path_table,
                    &module.executable.resource_table,
                    &mut diagnostic_strings,
                ) {
                    Ok(plan) => plan,
                    Err(
                        crate::projects::html_project::page_metadata::PageMetadataError::Diagnostic(
                            diagnostic,
                        ),
                    ) => {
                        return Err(capture_module_diagnostic(
                            diagnostic,
                            &module.executable.hir,
                            entry.source_domain,
                            &module.executable.type_environment,
                            &module.executable.path_table,
                            diagnostic_strings.strings(),
                        )
                        .into());
                    }
                    Err(
                        crate::projects::html_project::page_metadata::PageMetadataError::Infrastructure(
                            error,
                        ),
                    ) => return Err(error.into()),
                }
            };
            {
                let mut diagnostic_strings = backend_context.diagnostic_strings();
                resource_output_plan.plan_entry(
                    &entry,
                    planned_html_output_path,
                    &page_metadata_plan,
                    &mut diagnostic_strings,
                )?;
            }
            if let Some(route_plan) = &wasm_route_plan {
                let mut diagnostic_strings = backend_context.diagnostic_strings();
                if let Some(js_path) = &route_plan.js_path {
                    resource_output_plan.reserve_builder_output_path(
                        js_path,
                        "JavaScript",
                        &mut diagnostic_strings,
                    )?;
                }
                if let Some(wasm_path) = &route_plan.wasm_path {
                    resource_output_plan.reserve_builder_output_path(
                        wasm_path,
                        "Wasm",
                        &mut diagnostic_strings,
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
                    route: &route,
                    structural_url_renderer: &structural_url_renderer,
                    project_name: config.project_name.as_str(),
                    document_config: &document_config,
                    build_profile,
                    wasm_enabled,
                },
                backend_context,
            )?;
            {
                let mut diagnostic_strings = backend_context.diagnostic_strings();
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
                        &mut diagnostic_strings,
                    )?;
                }
            }

            let html_output_path = compiled_artifacts.html_output_path.clone();
            for output_file in compiled_artifacts.output_files {
                let output_path = output_file.relative_output_path().to_path_buf();
                if let Some((existing_entry_point, existing_index)) =
                    output_path_owners.get(&output_path)
                {
                    if is_shared_glue_output(
                        &output_files[*existing_index],
                        existing_entry_point,
                        &output_file,
                        &module.metadata.entry_point,
                    )? {
                        continue;
                    }
                    let mut diagnostic_strings = backend_context.diagnostic_strings();
                    return Err(duplicate_output_path_error(
                        &module.metadata.entry_point,
                        existing_entry_point,
                        &output_path,
                        &mut diagnostic_strings,
                    )
                    .into());
                }
                output_paths.insert(output_path.clone());
                output_path_owners.insert(
                    output_path,
                    (module.metadata.entry_point.clone(), output_files.len()),
                );
                output_files.push(output_file);
            }

            if entry_paths.is_homepage_entry(&module.metadata.entry_point) {
                has_directory_homepage = true;
                entry_page_rel = Some(html_output_path.clone());
            } else if !entry_paths.is_directory_build() && entry_page_rel.is_none() {
                entry_page_rel = Some(html_output_path);
            }
        }

        {
            let mut diagnostic_strings = backend_context.diagnostic_strings();
            entry_paths.require_homepage_if_directory_build(
                has_directory_homepage,
                &mut diagnostic_strings,
            )?;
        }

        let runtime_emission_plan = HtmlExternalRuntimeEmissionPlan::from_import_sets(
            artifact_entries.iter().map(|entry| entry.external_imports),
        );

        // Provider JS assets become deferred resource outputs. Their canonical byte sources
        // attach to the shared registry and their declared destinations join the resource
        // plan, so a collision with a page resource fails naming both semantic origins.
        register_js_runtime_asset_sources(&runtime_emission_plan, &mut resource_inputs)?;
        {
            let mut diagnostic_strings = backend_context.diagnostic_strings();
            for asset in runtime_emission_plan.js_assets().values() {
                resource_output_plan.plan_provider_runtime_asset(
                    asset.origin.clone(),
                    asset.source_span.map(ResourceDiagnosticSite::project),
                    &mut diagnostic_strings,
                )?;
            }
        }
        let runtime_module_output_paths =
            planned_runtime_module_output_paths(runtime_emission_plan.runtime_module_specifiers());
        {
            let mut diagnostic_strings = backend_context.diagnostic_strings();
            for output_path in runtime_module_output_paths {
                resource_output_plan.reserve_compiler_output_path(
                    &output_path,
                    "JavaScript",
                    &mut diagnostic_strings,
                )?;
            }
        }

        output_files.extend(emit_build_runtime_modules(
            &runtime_emission_plan,
            &mut output_paths,
        )?);
        let deferred_resources = {
            let mut diagnostic_strings = backend_context.diagnostic_strings();
            emit_planned_resource_outputs(
                resource_output_plan,
                &resource_inputs,
                &output_files,
                &output_paths,
                &mut diagnostic_strings,
            )?
        };

        Ok(Project {
            output_files,
            entry_page_rel,
            cleanup_policy: CleanupPolicy::html(),
            deferred_resources,
            resource_inputs,
        })
    }

    fn validate_project_config(
        &self,
        config: &Config,
        diagnostic_strings: &mut DiagnosticStringBuilder,
    ) -> Result<(), ProjectConfigError> {
        // Validate HTML-specific configuration up front so build/dev runtime behavior stays
        // deterministic and all routing-policy mistakes are surfaced as config errors.
        parse_html_site_config(config, diagnostic_strings)?;
        parse_html_document_config(config, diagnostic_strings)?;

        Ok(())
    }

    fn frontend_style_directives(&self) -> Vec<StyleDirectiveSpec> {
        html_project_style_directives()
    }

    fn frontend_surface(&self) -> BuilderSurface {
        let mut builder_surface = BuilderSurface::with_mandatory_core();
        builder_surface
            .source_packages
            .register_filesystem_root(
                HTML_SOURCE_PACKAGE_PREFIX,
                BuilderSurface::builtin_source_package_root(HTML_SOURCE_PACKAGE_PREFIX),
                crate::builder_surface::PackageOrigin::Builder,
            )
            .expect("the built-in @html source package registers once");
        builder_surface.register_implicit_template_scope_source_package(HTML_SOURCE_PACKAGE_PREFIX);

        // `@web/site` needs an explicit dependency clause, so it stays out of template scope.
        builder_surface
            .source_packages
            .register_filesystem_root(
                WEB_SITE_SOURCE_PACKAGE_PREFIX,
                BuilderSurface::builtin_source_package_root(WEB_SITE_SOURCE_PACKAGE_PREFIX),
                crate::builder_surface::PackageOrigin::Builder,
            )
            .expect("the built-in @web/site source package registers once");

        crate::builder_surface::core_packages::register_optional_core_packages(
            &mut builder_surface.binding_packages,
        );

        let web_package_metadata =
            register_web_binding_packages(&mut builder_surface.binding_packages);
        builder_surface
            .builder_runtime_packages
            .extend(web_package_metadata);

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
            .validate()
            .expect("the HTML builder surface is valid");
        builder_surface
    }
}

impl HtmlProjectBuilder {
    fn register_html_config_keys(builder_surface: &mut BuilderSurface) {
        let mut html_schema = ConfigSchema::new("html section", UnknownFieldPolicy::Reject);
        Self::register_html_fields(&mut html_schema);

        let html_root = html_schema.root();
        register_output_root_fields(&mut html_schema, html_root)
            .expect("html schema is under construction");
        html_schema
            .validate_and_freeze()
            .expect("html schema is valid");

        builder_surface
            .config_schemas
            .register_project_section(NamedConfigSectionSchema {
                name: "html".into(),
                schema: html_schema,
                required: true,
            });
    }

    fn register_html_fields(schema: &mut ConfigSchema) {
        let root = schema.root();

        schema
            .register_field(root, ConfigSchemaField::string("origin"))
            .expect("html schema is under construction");
        schema
            .register_field(root, ConfigSchemaField::string("page_url_style"))
            .expect("html schema is under construction");
        schema
            .register_field(root, ConfigSchemaField::bool("redirect_index_html"))
            .expect("html schema is under construction");
        schema
            .register_field(root, ConfigSchemaField::string("html_lang"))
            .expect("html schema is under construction");
        schema
            .register_field(root, ConfigSchemaField::string("html_title_prefix"))
            .expect("html schema is under construction");
        schema
            .register_field(root, ConfigSchemaField::string("html_title_postfix"))
            .expect("html schema is under construction");
        schema
            .register_field(root, ConfigSchemaField::string("html_favicon"))
            .expect("html schema is under construction");
        schema
            .register_field(root, ConfigSchemaField::bool("html_inject_charset"))
            .expect("html schema is under construction");
        schema
            .register_field(root, ConfigSchemaField::bool("html_inject_viewport"))
            .expect("html schema is under construction");
        schema
            .register_field(root, ConfigSchemaField::bool("html_inject_color_scheme"))
            .expect("html schema is under construction");
        schema
            .register_field(root, ConfigSchemaField::bool("html_inject_core_css"))
            .expect("html schema is under construction");
        schema
            .register_field(root, ConfigSchemaField::string("html_body_style"))
            .expect("html schema is under construction");
    }

    /// Compile one module through the appropriate builder path (JS-only or HTML+Wasm).
    fn compile_one_module(
        &self,
        context: HtmlModuleCompileContext<'_>,
        backend_context: &mut BackendBuildContext,
    ) -> Result<CompiledHtmlModuleArtifacts, PreparedFailure> {
        let HtmlModuleCompileContext {
            entry:
                ProjectEntry {
                    source_domain,
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
            route,
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
        {
            let mut diagnostic_strings = backend_context.diagnostic_strings();
            validate_hir_external_package_support(
                reachability,
                module.link_facts.external_package_registry.as_ref(),
                backend_target,
                &mut diagnostic_strings,
            )
            .map_err(|diagnostic| {
                capture_module_diagnostic(
                    diagnostic,
                    &module.executable.hir,
                    source_domain,
                    &module.executable.type_environment,
                    &module.executable.path_table,
                    diagnostic_strings.strings(),
                )
            })?;
        }

        {
            let mut diagnostic_strings = backend_context.diagnostic_strings();
            validate_hir_backend_feature_support(
                BackendFeatureValidationInput {
                    hir: &module.executable.hir,
                    reachability,
                    target: backend_target,
                    type_environment: Some(&module.executable.type_environment),
                },
                &mut diagnostic_strings,
            )
            .map_err(|diagnostic| {
                capture_module_diagnostic(
                    diagnostic,
                    &module.executable.hir,
                    source_domain,
                    &module.executable.type_environment,
                    &module.executable.path_table,
                    diagnostic_strings.strings(),
                )
            })?;
        }

        for linked in &linked_modules {
            {
                let mut diagnostic_strings = backend_context.diagnostic_strings();
                validate_hir_external_package_support(
                    linked.reachability,
                    linked.module.link_facts.external_package_registry.as_ref(),
                    backend_target,
                    &mut diagnostic_strings,
                )
                .map_err(|diagnostic| {
                    capture_module_diagnostic(
                        diagnostic,
                        &linked.module.executable.hir,
                        linked.source_domain,
                        &linked.module.executable.type_environment,
                        &linked.module.executable.path_table,
                        diagnostic_strings.strings(),
                    )
                })?;
            }

            {
                let mut diagnostic_strings = backend_context.diagnostic_strings();
                validate_hir_backend_feature_support(
                    BackendFeatureValidationInput {
                        hir: &linked.module.executable.hir,
                        reachability: linked.reachability,
                        target: backend_target,
                        type_environment: Some(&linked.module.executable.type_environment),
                    },
                    &mut diagnostic_strings,
                )
                .map_err(|diagnostic| {
                    capture_module_diagnostic(
                        diagnostic,
                        &linked.module.executable.hir,
                        linked.source_domain,
                        &linked.module.executable.type_environment,
                        &linked.module.executable.path_table,
                        diagnostic_strings.strings(),
                    )
                })?;
            }
        }

        // Call-target gate: runs after every linked module passed its own shape validation, so a
        // callee's specific diagnostic keeps precedence over this call-site rejection.
        {
            let mut diagnostic_strings = backend_context.diagnostic_strings();
            validate_hir_linked_function_targets(
                BackendFeatureValidationInput {
                    hir: &module.executable.hir,
                    reachability,
                    target: backend_target,
                    type_environment: Some(&module.executable.type_environment),
                },
                &mut diagnostic_strings,
            )
            .map_err(|diagnostic| {
                capture_module_diagnostic(
                    diagnostic,
                    &module.executable.hir,
                    source_domain,
                    &module.executable.type_environment,
                    &module.executable.path_table,
                    diagnostic_strings.strings(),
                )
            })?;
        }

        let compile_input = HtmlModuleCompileInput {
            hir_module: &module.executable.hir,
            path_table: &module.executable.path_table,
            resource_table: &module.executable.resource_table,
            reachability,
            type_environment: &module.executable.type_environment,
            const_fragments: &module.metadata.const_top_level_fragments,
            page_metadata_plan,
            borrow_facts: module.executable.borrow_analysis.borrow_facts(),
            numeric_proofs: &module.executable.numeric_proofs,
            project_name,
            document_config,
            build_profile,
            root_activity: &module.metadata.root_activity,
            external_package_registry: Arc::clone(&module.link_facts.external_package_registry),
        };
        if wasm_enabled {
            let compiled_wasm = compile_html_module_wasm(
                &compile_input,
                backend_context.strings(),
                route,
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
                    structural_url_renderer,
                    compile_input: &compile_input,
                    route,
                },
                backend_context.strings(),
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
    string_table: &mut DiagnosticStringBuilder,
) -> Result<Vec<DeferredResourceOutput>, PreparedFailure> {
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

            let error = InfrastructureFailure::internal(
                CompilerStage::ProjectBuildAssembly,
                format!(
                    "planned {owner_kind} resource origin {:?} has no registered source attachment",
                    record.origin
                ),
            );
            return Err(error.into());
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
            return Err(capture_html_diagnostic(
                resource_output_path_reserved_diagnostic(
                    &record.output_path,
                    &display_origin(&record.origin),
                    artefact_kind,
                    record.first_authored_span,
                    string_table,
                ),
                string_table.strings(),
                None,
                || None,
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
    string_table: &mut DiagnosticStringBuilder,
) -> PreparedFailure {
    capture_html_diagnostic(
        duplicate_html_output_path_diagnostic(
            duplicate_entry_point,
            existing_entry_point,
            output_path,
            string_table,
        ),
        string_table.strings(),
        None,
        || None,
    )
}

/// Whether a repeated page output is generated glue the build already holds.
///
/// WHY: glue files are named by a digest of their source, so pages selecting the same wrappers
///      claim one path and share one file. Different bytes under that path mean the digest
///      collided, and picking either file would break the other page's imports.
fn is_shared_glue_output(
    existing: &OutputFile,
    existing_entry_point: &Path,
    repeated: &OutputFile,
    repeated_entry_point: &Path,
) -> Result<bool, InfrastructureFailure> {
    if !is_glue_module_output_path(repeated.relative_output_path()) {
        return Ok(false);
    }
    match (existing.file_kind(), repeated.file_kind()) {
        (FileKind::Js(existing_source), FileKind::Js(repeated_source))
            if existing_source == repeated_source =>
        {
            Ok(true)
        }
        _ => Err(InfrastructureFailure::internal(
            CompilerStage::ProjectBuildAssembly,
            format!(
                "Generated JS glue path '{}' has different contents for entries '{}' and '{}'.",
                portable_path_text(repeated.relative_output_path()),
                portable_path_text(existing_entry_point),
                portable_path_text(repeated_entry_point),
            ),
        )),
    }
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
