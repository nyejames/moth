use super::*;
#[derive(Debug)]
struct DummyJsImportProvider {
    calls: Arc<AtomicUsize>,
}

impl DummyJsImportProvider {
    fn with_counter(calls: Arc<AtomicUsize>) -> Arc<Self> {
        Arc::new(Self { calls })
    }
}

impl ExternalImportProvider for DummyJsImportProvider {
    fn kind(&self) -> ExternalImportProviderKind {
        ExternalImportProviderKind::new("dummy-js")
    }

    fn supported_extensions(&self) -> &[ExternalFileExtension] {
        static SUPPORTED_EXTENSIONS: std::sync::OnceLock<Vec<ExternalFileExtension>> =
            std::sync::OnceLock::new();
        SUPPORTED_EXTENSIONS
            .get_or_init(|| vec![ExternalFileExtension::from("js")])
            .as_slice()
    }

    fn resolve_external_import(
        &self,
        request: ExternalImportRequest,
        context: &mut ExternalImportProviderContext,
    ) -> Result<Option<ResolvedExternalImport>, CompilerMessages> {
        self.calls.fetch_add(1, Ordering::SeqCst);

        let package_path = dummy_package_path(&request.canonical_source_path);
        let package_id = register_dummy_package(context, package_path)?;
        let widget_type_id = register_dummy_widget_type(context, package_id)?;
        let draw_function_id = register_dummy_draw_function(context, package_id)?;
        let make_widget_function_id =
            register_dummy_make_widget_function(context, package_id, widget_type_id)?;
        let use_widget_function_id =
            register_dummy_use_widget_function(context, package_id, widget_type_id)?;

        Ok(Some(ResolvedExternalImport {
            package_id,
            exported_types: vec![widget_type_id],
            exported_free_functions: vec![
                draw_function_id,
                make_widget_function_id,
                use_widget_function_id,
            ],
            runtime_asset: None,
            diagnostics: Vec::new(),
            required_runtime_imports: Vec::new(),
        }))
    }
}

fn dummy_package_path(canonical_source_path: &Path) -> String {
    let sanitized = canonical_source_path
        .to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();

    format!("@test/provider/{sanitized}")
}

fn register_dummy_package(
    context: &mut ExternalImportProviderContext,
    package_path: String,
) -> Result<crate::compiler_frontend::external_packages::ExternalPackageId, CompilerMessages> {
    context
        .package_registry
        .register_package(
            package_path,
            crate::builder_surface::PackageOrigin::ProjectLocal,
        )
        .map_err(|error| provider_error_to_messages(error, context.string_table))
}

/// Build one JS runtime asset fixture whose origin carries a stable provider owner.
fn fixture_dummy_js_runtime_asset(canonical_source_path: PathBuf) -> RuntimeAssetIdentity {
    let owner = StableResourceOwnerId::Provider(StableProviderResourceOwnerId::new(
        "html-js",
        StablePackageIdentity::binding(PackageOrigin::ProjectLocal, "@test/fixtures"),
    ));

    RuntimeAssetIdentity {
        origin: StableResourceOriginId::new(
            owner,
            PortableResourcePath::from_portable_spelling("_moth/js/fixture.js".to_owned())
                .expect("fixture asset logical path should be valid"),
        ),
        canonical_source_path,
        asset_kind: "js".to_owned(),
        source_span: None,
    }
}

fn register_dummy_widget_type(
    context: &mut ExternalImportProviderContext,
    package_id: crate::compiler_frontend::external_packages::ExternalPackageId,
) -> Result<ExternalTypeId, CompilerMessages> {
    context
        .package_registry
        .register_external_type(
            package_id,
            ExternalTypeSpec {
                name: "Widget".to_owned(),
                abi_type: ExternalAbiType::Handle,
            },
        )
        .map_err(|error| provider_error_to_messages(error, context.string_table))
}

fn register_dummy_draw_function(
    context: &mut ExternalImportProviderContext,
    package_id: crate::compiler_frontend::external_packages::ExternalPackageId,
) -> Result<ExternalFunctionId, CompilerMessages> {
    context
        .package_registry
        .register_external_function(
            package_id,
            ExternalFunctionSpec {
                name: "draw".to_owned(),
                parameters: Vec::new(),
                returns: vec![ExternalReturnSlot::fresh(ExternalAbiType::I32)],
                error_return_type: None,
                lowerings: ExternalFunctionLowerings::default(),
            },
        )
        .map_err(|error| provider_error_to_messages(error, context.string_table))
}

fn register_dummy_make_widget_function(
    context: &mut ExternalImportProviderContext,
    package_id: crate::compiler_frontend::external_packages::ExternalPackageId,
    widget_type_id: ExternalTypeId,
) -> Result<ExternalFunctionId, CompilerMessages> {
    context
        .package_registry
        .register_external_function(
            package_id,
            ExternalFunctionSpec {
                name: "make_widget".to_owned(),
                parameters: Vec::new(),
                returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::External(
                    widget_type_id,
                ))],
                error_return_type: None,
                lowerings: ExternalFunctionLowerings::default(),
            },
        )
        .map_err(|error| provider_error_to_messages(error, context.string_table))
}

fn register_dummy_use_widget_function(
    context: &mut ExternalImportProviderContext,
    package_id: crate::compiler_frontend::external_packages::ExternalPackageId,
    widget_type_id: ExternalTypeId,
) -> Result<ExternalFunctionId, CompilerMessages> {
    context
        .package_registry
        .register_external_function(
            package_id,
            ExternalFunctionSpec {
                name: "use_widget".to_owned(),
                parameters: vec![
                    crate::compiler_frontend::external_packages::ExternalParameter {
                        language_type: ExternalSignatureType::External(widget_type_id),
                        access_kind: ExternalAccessKind::Shared,
                    },
                ],
                returns: vec![ExternalReturnSlot::fresh(ExternalAbiType::I32)],
                error_return_type: None,
                lowerings: ExternalFunctionLowerings::default(),
            },
        )
        .map_err(|error| provider_error_to_messages(error, context.string_table))
}

fn provider_error_to_messages(
    error: CompilerError,
    string_table: &StringTable,
) -> CompilerMessages {
    CompilerMessages::from_error_ref(error, string_table)
}

fn builder_surface_with_dummy_js_provider(calls: Arc<AtomicUsize>) -> BuilderSurface {
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .external_import_providers
        .register(DummyJsImportProvider::with_counter(calls));
    frontend_surface
}

fn module_contains_external_call(
    module: &crate::compiler_frontend::module_compilation::Module,
) -> bool {
    module.executable.hir.blocks.iter().any(|block| {
        block.statements.iter().any(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Call {
                    target: CallTarget::External(_),
                    ..
                }
            )
        })
    })
}

fn module_contains_external_module_export(
    module: &crate::compiler_frontend::module_compilation::Module,
    export_name: &str,
) -> bool {
    module.executable.hir.blocks.iter().any(|block| {
        block.statements.iter().any(|statement| {
            let HirStatementKind::Call {
                target: CallTarget::External(function_id),
                ..
            } = &statement.kind
            else {
                return false;
            };

            module
                .link_facts
                .external_package_registry
                .get_function_by_id(*function_id)
                .and_then(|definition| definition.lowerings.js.as_ref())
                .is_some_and(|lowering| {
                    matches!(
                        lowering,
                        ExternalJsLowering::ExternalModuleExport { export_name: registered }
                            if registered == export_name
                    )
                })
        })
    })
}
// -------------------------
//  Provider metadata carry
// -------------------------

#[derive(Debug)]
struct DummyJsImportProviderWithLowering {
    calls: Arc<AtomicUsize>,
}

impl DummyJsImportProviderWithLowering {
    fn with_counter(calls: Arc<AtomicUsize>) -> Arc<Self> {
        Arc::new(Self { calls })
    }
}

impl ExternalImportProvider for DummyJsImportProviderWithLowering {
    fn kind(&self) -> ExternalImportProviderKind {
        ExternalImportProviderKind::new("dummy-js-with-lowering")
    }

    fn supported_extensions(&self) -> &[ExternalFileExtension] {
        static SUPPORTED_EXTENSIONS: std::sync::OnceLock<Vec<ExternalFileExtension>> =
            std::sync::OnceLock::new();
        SUPPORTED_EXTENSIONS
            .get_or_init(|| vec![ExternalFileExtension::from("js")])
            .as_slice()
    }

    fn resolve_external_import(
        &self,
        request: ExternalImportRequest,
        context: &mut ExternalImportProviderContext,
    ) -> Result<Option<ResolvedExternalImport>, CompilerMessages> {
        self.calls.fetch_add(1, Ordering::SeqCst);

        let package_path = dummy_package_path(&request.canonical_source_path);
        let package_id = register_dummy_package(context, package_path)?;
        let draw_function_id = register_dummy_draw_function_with_js_lowering(context, package_id)?;

        Ok(Some(ResolvedExternalImport {
            package_id,
            exported_types: Vec::new(),
            exported_free_functions: vec![draw_function_id],
            runtime_asset: Some(fixture_dummy_js_runtime_asset(
                request.canonical_source_path.clone(),
            )),
            diagnostics: Vec::new(),
            required_runtime_imports: Vec::new(),
        }))
    }
}

fn register_dummy_draw_function_with_js_lowering(
    context: &mut ExternalImportProviderContext,
    package_id: crate::compiler_frontend::external_packages::ExternalPackageId,
) -> Result<ExternalFunctionId, CompilerMessages> {
    context
        .package_registry
        .register_external_function(
            package_id,
            ExternalFunctionSpec {
                name: "draw".to_owned(),
                parameters: Vec::new(),
                returns: vec![ExternalReturnSlot::fresh(ExternalAbiType::I32)],
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction("draw".to_owned())),
                    wasm: None,
                },
            },
        )
        .map_err(|error| provider_error_to_messages(error, context.string_table))
}

fn builder_surface_with_dummy_js_provider_with_lowering(calls: Arc<AtomicUsize>) -> BuilderSurface {
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .external_import_providers
        .register(DummyJsImportProviderWithLowering::with_counter(calls));
    frontend_surface
}
#[test]
fn provider_created_package_registry_survives_into_module() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(dir.join("@page.moth"), "@drawing.js draw\nvalue = draw()\n")
        .expect("should write page");
    fs::write(dir.join("drawing.js"), "export function draw() {}\n").expect("should write js");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut frontend_surface =
        builder_surface_with_dummy_js_provider_with_lowering(Arc::clone(&calls));

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("provider-backed import should compile");

    let module = modules
        .successful_module_views()
        .next()
        .expect("expected one module");

    assert!(
        !module.link_facts.external_import_candidates.is_empty(),
        "module should carry provider external imports"
    );

    for import in &module.link_facts.external_import_candidates {
        let package = module
            .link_facts
            .external_package_registry
            .get_package_by_id(import.package_id)
            .expect(
                "package referenced by external_import_candidates should exist in module registry",
            );
        assert_eq!(
            package.metadata,
            crate::builder_surface::PackageMetadata::binding(
                crate::builder_surface::PackageOrigin::ProjectLocal
            ),
            "provider package should be ProjectLocal"
        );
    }
}

#[test]
fn provider_runtime_assets_deduped_for_repeated_imports() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "@drawing.js draw\n@other run\nvalue = draw()\nother_value = run()\n",
    )
    .expect("should write entry");
    fs::write(
        dir.join("other.moth"),
        "@drawing.js draw as render\nrun || -> Int:\n    return render()\n;\n",
    )
    .expect("should write helper");
    fs::write(dir.join("drawing.js"), "export function draw() {}\n").expect("should write js");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut frontend_surface =
        builder_surface_with_dummy_js_provider_with_lowering(Arc::clone(&calls));

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("provider-backed imports should compile");

    let module = modules
        .successful_module_views()
        .next()
        .expect("expected one module");

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "same canonical JS file should be resolved through the provider cache once"
    );
    assert_eq!(
        module.link_facts.external_import_candidates.len(),
        1,
        "same JS file imported twice should produce one deduped module external import"
    );
    assert!(
        module.link_facts.external_import_candidates[0]
            .runtime_asset
            .is_some(),
        "deduped import should carry runtime asset"
    );
}

#[test]
fn entry_runtime_metadata_ignores_unreachable_external_calls() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(dir.join("@page.moth"), "@other run\nvalue = 1\n").expect("should write entry");
    fs::write(
        dir.join("other.moth"),
        "@drawing.js get_number\nrun || -> Int, Error!:\n    return get_number()!\n;\n",
    )
    .expect("should write helper source");
    fs::write(
        dir.join("drawing.js"),
        "import { mothOk } from \"@moth/runtime\";\n/**\n * @moth.sig get_number || -> Int, Error!\n */\nexport function getNumber() { return mothOk(7); }\n",
    )
    .expect("should write js");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = builder_surface_with_html_js_provider();

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("unreachable provider-backed call should compile");

    let module = modules
        .successful_module_views()
        .next()
        .expect("expected one module");
    assert!(
        module_contains_external_module_export(module, "getNumber"),
        "HIR should keep the unreachable function body and provider package metadata"
    );
    assert!(
        !module.link_facts.external_import_candidates.is_empty(),
        "module link facts should retain provider candidates independently of entry reachability"
    );
    let project_compilation = ProjectCompilation::from_frontend(modules)
        .expect("compiled module should assemble an entry");
    let entries = project_compilation.entries();
    assert_eq!(
        entries.len(),
        1,
        "top-level runtime work should create one entry"
    );
    assert!(
        entries[0].external_imports.is_empty(),
        "entry runtime metadata should exclude packages used only by unreachable functions"
    );
    let entry = entries[0].clone();
    let selection = entry.reachability.backend_selection();
    let start_function_id = entry
        .module
        .executable
        .hir
        .start_function
        .expect("entry module should have start");
    let start_entry_block = entry
        .module
        .executable
        .hir
        .functions
        .iter()
        .find(|function| function.id == start_function_id)
        .expect("entry start function should exist")
        .entry;
    assert_eq!(selection.function_count(), 1);
    assert!(selection.contains_function(start_function_id));
    assert_eq!(
        selection.blocks_for_function(start_function_id),
        Some(&[start_entry_block][..])
    );
}

#[test]
fn entry_runtime_metadata_ignores_unreachable_source_package_wrappers() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "@html canvas\npage_canvas_id #= canvas\nvalue = 1\n",
    )
    .expect("should write page");

    let mut config = Config::new(dir.clone());
    let builder = crate::projects::html_project::html_project_builder::HtmlProjectBuilder::new();
    let style_directives = StyleDirectiveRegistry::merged(&builder.frontend_style_directives())
        .expect("HTML style directives should merge");
    let mut frontend_surface = builder.frontend_surface();
    let canvas_package_id = frontend_surface
        .binding_packages
        .resolve_package_id("@web/canvas")
        .expect("@web/canvas should be registered for HTML projects");
    let mut string_table = StringTable::new();

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("unused @html canvas wrapper should compile");

    let module = modules
        .successful_module_views()
        .next()
        .expect("expected one module");
    assert!(
        module
            .link_facts
            .external_package_registry
            .get_package_by_id(canvas_package_id)
            .is_some(),
        "the external package registry should stay fully populated"
    );
    assert!(
        module
            .link_facts
            .external_import_candidates
            .iter()
            .any(|import| import.package_id == canvas_package_id),
        "module link facts should retain the available @web/canvas runtime candidate"
    );
    let project_compilation = ProjectCompilation::from_frontend(modules)
        .expect("compiled module should assemble an entry");
    let entries = project_compilation.entries();
    assert_eq!(
        entries.len(),
        1,
        "top-level runtime work should create one entry"
    );
    assert!(
        entries[0]
            .external_imports
            .iter()
            .all(|import| import.package_id != canvas_package_id),
        "entry runtime metadata should exclude unreachable @web/canvas wrappers"
    );
}

#[test]
fn provider_backed_import_with_js_lowering_passes_html_build() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(dir.join("@page.moth"), "@drawing.js draw\nvalue = draw()\n")
        .expect("should write page");
    fs::write(dir.join("drawing.js"), "export function draw() {}\n").expect("should write js");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut frontend_surface =
        builder_surface_with_dummy_js_provider_with_lowering(Arc::clone(&calls));

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("provider-backed import should compile");

    let builder = crate::projects::html_project::html_project_builder::HtmlProjectBuilder::new();
    let project_compilation =
        crate::build_system::build::ProjectCompilation::from_frontend(modules)
            .expect("compiled modules should assemble entries");
    let project = builder
        .build_backend(
            project_compilation,
            &config,
            crate::build_system::BuildProfile::Dev,
            &[],
            &mut string_table,
        )
        .expect("HTML build should succeed with module-owned registry");

    assert!(
        !project.output_files.is_empty(),
        "HTML build should produce output files"
    );
}

#[cfg(feature = "timers")]
#[test]
fn linked_module_js_lowering_is_observed_separately() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let package_root = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "@phase6helper helper\nvalue = helper()\n",
    )
    .expect("should write page");
    fs::write(
        package_root.join("@mod.moth"),
        "export:\n    helper || -> Int:\n        return 7\n    ;\n;\n",
    )
    .expect("should write package module");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "phase6helper",
        package_root.clone(),
        PackageOrigin::Builder,
    );

    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("provider-backed import should compile");
    let builder = crate::projects::html_project::html_project_builder::HtmlProjectBuilder::new();
    let project_compilation =
        crate::build_system::build::ProjectCompilation::from_frontend(modules)
            .expect("compiled modules should assemble entries");
    let project = builder
        .build_backend(
            project_compilation,
            &config,
            crate::build_system::BuildProfile::Dev,
            &[],
            &mut string_table,
        )
        .expect("HTML build should succeed with module-owned registry");
    let snapshot = timing_session.finish();
    drop(project);

    assert!(
        snapshot.timings.iter().any(|aggregate| {
            aggregate.metric.descriptor().stable_name == "backend.js.lower_linked"
                && aggregate.samples > 0
        }),
        "linked-module JS lowering must be observed separately from entry lowering"
    );
    assert!(
        snapshot.timings.iter().any(|aggregate| {
            aggregate.metric.descriptor().stable_name == "backend.js.lower_entry"
                && aggregate.samples > 0
        }),
        "entry-module JS lowering must remain observed"
    );
}
#[test]
fn provider_backed_direct_selection_compiles_and_reuses_cache() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "@drawing.js draw as render\n@other run\nvalue = render()\nother_value = run()\n",
    )
    .expect("should write page");
    fs::write(
        dir.join("other.moth"),
        "@drawing.js draw as render_again\nrun || -> Int:\n    return render_again()\n;\n",
    )
    .expect("should write helper source");
    fs::write(dir.join("drawing.js"), "export function draw() {}\n").expect("should write js");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut frontend_surface = builder_surface_with_dummy_js_provider(Arc::clone(&calls));

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("provider-backed direct selections should compile");

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "same canonical JS file should be resolved through the provider once"
    );
    assert!(
        modules
            .successful_module_views()
            .any(module_contains_external_call),
        "HIR should lower provider-backed direct-selection calls to external function IDs"
    );
}

#[test]
fn provider_backed_namespace_binding_exposes_function_and_type_members() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "@drawing.js as drawing\nwidget drawing.Widget = drawing.make_widget()\nvalue = drawing.draw()\n",
    )
    .expect("should write page");
    fs::write(dir.join("drawing.js"), "export function draw() {}\n").expect("should write js");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut frontend_surface = builder_surface_with_dummy_js_provider(Arc::clone(&calls));

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("provider-backed namespace binding should compile");

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "namespace binding should resolve the JS file once"
    );
    assert!(
        modules
            .successful_module_views()
            .any(module_contains_external_call),
        "namespace member calls should lower to external function IDs"
    );
}

#[test]
fn provider_backed_same_bare_name_from_different_directories_gets_distinct_packages() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("a")).expect("should create a dir");
    fs::create_dir_all(dir.join("b")).expect("should create b dir");
    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "@a/use run_a\n@b/use run_b\nvalue_a = run_a()\nvalue_b = run_b()\n",
    )
    .expect("should write page");
    fs::write(
        dir.join("a/use.moth"),
        "@a/helper.js draw as draw_a\nrun_a || -> Int:\n    return draw_a()\n;\n",
    )
    .expect("should write a source");
    fs::write(
        dir.join("b/use.moth"),
        "@b/helper.js draw as draw_b\nrun_b || -> Int:\n    return draw_b()\n;\n",
    )
    .expect("should write b source");
    fs::write(dir.join("a/helper.js"), "export function draw() {}\n").expect("should write a js");
    fs::write(dir.join("b/helper.js"), "export function draw() {}\n").expect("should write b js");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut frontend_surface = builder_surface_with_dummy_js_provider(Arc::clone(&calls));

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("same bare JS filename in different directories should compile");

    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "different canonical JS files with the same basename should get separate provider results"
    );
    assert!(
        modules
            .successful_module_views()
            .any(module_contains_external_call),
        "calls through both provider-created packages should lower to external IDs"
    );
}

#[test]
fn provider_backed_opaque_type_passes_to_same_package_function() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "@drawing.js make_widget, use_widget\nwidget = make_widget()\nvalue = use_widget(widget)\n",
    )
    .expect("should write page");
    fs::write(dir.join("drawing.js"), "export function draw() {}\n").expect("should write js");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut frontend_surface = builder_surface_with_dummy_js_provider(Arc::clone(&calls));

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("same-package opaque type should pass to function expecting that exact type");

    assert!(
        modules
            .successful_module_views()
            .any(module_contains_external_call),
        "HIR should contain external calls for make_widget and use_widget"
    );
}

#[test]
fn provider_backed_opaque_type_from_different_package_is_rejected() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("a")).expect("should create a dir");
    fs::create_dir_all(dir.join("b")).expect("should create b dir");
    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "@a/drawing.js make_widget\n@b/drawing.js use_widget\nwidget = make_widget()\nvalue = use_widget(widget)\n",
    )
    .expect("should write page");
    fs::write(dir.join("a/drawing.js"), "export function draw() {}\n").expect("should write a js");
    fs::write(dir.join("b/drawing.js"), "export function draw() {}\n").expect("should write b js");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut frontend_surface = builder_surface_with_dummy_js_provider(Arc::clone(&calls));

    let mut frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("diagnosed modules are retained in the typed frontend outcome");
    let project_source = frontend.project_source_database.take();
    let messages = frontend
        .into_render_messages_with_frozen_identity(&mut string_table, project_source, None)
        .expect("cross-package diagnostics should install frozen render identity");

    assert!(
        messages.error_diagnostics().any(|diagnostic| {
            matches!(&diagnostic.payload, DiagnosticPayload::TypeMismatch { .. })
        }),
        "expected type mismatch diagnostic for cross-package opaque type, got {messages:?}"
    );
}

#[test]
fn directory_project_rejects_missing_entry_root() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    // Config declares an entry_root that does not exist.
    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"nonexistent\",\n|\n",
    )
    .expect("should write config");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();

    // Parse config so entry_root is applied to Config.
    let config_path = dir.join("config.moth");
    let frontend_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
    let build_config_inputs = crate::compiler_frontend::build_config::BuildConfigInputSet::new();
    let services = crate::build_system::project_config::ProjectConfigParseServices {
        style_directives: &style_directives,
        frontend_surface: &frontend_surface,
        build_config_inputs: &build_config_inputs,
    };
    let mut source_files = crate::compiler_frontend::source::SourceDatabase::empty();
    let parse_result = crate::build_system::project_config::compile_project_config_file(
        &mut config,
        &config_path,
        &services,
        &mut source_files,
        &mut string_table,
    );
    assert!(parse_result.is_ok(), "config parse should succeed");

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    );

    let Err(messages) = result else {
        panic!("expected Err for missing entry root");
    };
    assert!(
        messages.error_diagnostics().any(|diagnostic| {
            matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidConfig {
                    reason: InvalidConfigReason::ConfiguredEntryRootMissing { .. },
                    ..
                }
            )
        }),
        "expected ConfiguredEntryRootMissing for a nonexistent entry root, got {messages:?}"
    );
}

// ── Real HTML JS provider tests ───────────────────────────────────────────────

fn builder_surface_with_html_js_provider() -> BuilderSurface {
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .external_import_providers
        .register(std::sync::Arc::new(
            crate::projects::html_project::external_js::js_import_provider::JsExternalImportProvider::new(),
        ));
    frontend_surface
}

#[test]
fn html_js_provider_namespace_binding_resolves() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "@drawing.js as drawing\nvalue = drawing.draw()\n",
    )
    .expect("should write page");
    fs::write(
        dir.join("drawing.js"),
        "/**\n * @moth.sig draw || -> Int\n */\nexport function draw() { return 1; }\n",
    )
    .expect("should write js");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = builder_surface_with_html_js_provider();

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("real JS provider namespace binding should compile");

    assert!(
        modules
            .successful_module_views()
            .any(|module| module_contains_external_module_export(module, "draw")),
        "HIR should preserve namespace JS call export metadata"
    );
}

#[test]
fn directory_module_external_import_candidates_are_scoped_to_owned_sources() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    let src = dir.join("src");
    fs::create_dir_all(src.join("first")).expect("should create first module");
    fs::create_dir_all(src.join("second")).expect("should create second module");

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("first/@first.moth"),
        "@drawing.js draw\nvalue = draw()\n",
    )
    .expect("should write first module");
    fs::write(
        src.join("second/@second.moth"),
        "@drawing.js draw\nvalue = draw()\n",
    )
    .expect("should write second module");
    fs::write(
        src.join("first/drawing.js"),
        "/**\n * @moth.sig draw || -> Int\n */\nexport function draw() { return 1; }\n",
    )
    .expect("should write first JS provider");
    fs::write(
        src.join("second/drawing.js"),
        "/**\n * @moth.sig draw || -> Int\n */\nexport function draw() { return 2; }\n",
    )
    .expect("should write second JS provider");

    let mut config = Config::new(dir.clone());
    config.entry_root = PathBuf::from("src");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = builder_surface_with_html_js_provider();

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("sibling modules with distinct JS providers should compile");

    let artefacts = modules
        .project
        .successful_artefacts_in_module_id_order()
        .collect::<Vec<_>>();
    assert_eq!(
        artefacts.len(),
        2,
        "expected both sibling module artefacts to compile"
    );

    let first = artefacts
        .iter()
        .find(|artefact| {
            artefact
                .module
                .metadata
                .entry_point
                .ends_with(Path::new("first/@first.moth"))
        })
        .expect("expected first sibling module artefact");
    let second = artefacts
        .iter()
        .find(|artefact| {
            artefact
                .module
                .metadata
                .entry_point
                .ends_with(Path::new("second/@second.moth"))
        })
        .expect("expected second sibling module artefact");

    let first_provider_package_paths = first
        .module
        .link_facts
        .external_import_candidates
        .iter()
        .filter_map(|candidate| {
            first
                .module
                .link_facts
                .external_package_registry
                .get_package_by_id(candidate.package_id)
                .map(|package| package.path.as_str())
        })
        .filter(|path| path.starts_with("@html-js/"))
        .collect::<Vec<_>>();
    assert_eq!(
        first_provider_package_paths,
        vec!["@html-js/first/drawing.js"],
        "first module should retain only its own JS provider package"
    );

    let second_provider_package_paths = second
        .module
        .link_facts
        .external_import_candidates
        .iter()
        .filter_map(|candidate| {
            second
                .module
                .link_facts
                .external_package_registry
                .get_package_by_id(candidate.package_id)
                .map(|package| package.path.as_str())
        })
        .filter(|path| path.starts_with("@html-js/"))
        .collect::<Vec<_>>();
    assert_eq!(
        second_provider_package_paths,
        vec!["@html-js/second/drawing.js"],
        "second module should retain only its own JS provider package"
    );
}

#[test]
fn directory_module_external_import_candidates_are_scoped_to_owned_sources_when_display_order_differs()
 {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    let src = dir.join("src");
    fs::create_dir_all(src.join("a")).expect("should create a module");
    fs::create_dir_all(src.join("a-b")).expect("should create a-b module");

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("a/@a.moth"), "@drawing.js draw\nvalue = draw()\n")
        .expect("should write a module");
    fs::write(
        src.join("a-b/@b.moth"),
        "@drawing.js draw\nvalue = draw()\n",
    )
    .expect("should write a-b module");
    fs::write(
        src.join("a/drawing.js"),
        "/**\n * @moth.sig draw || -> Int\n */\nexport function draw() { return 1; }\n",
    )
    .expect("should write a JS provider");
    fs::write(
        src.join("a-b/drawing.js"),
        "/**\n * @moth.sig draw || -> Int\n */\nexport function draw() { return 2; }\n",
    )
    .expect("should write a-b JS provider");

    let mut config = Config::new(dir.clone());
    config.entry_root = PathBuf::from("src");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = builder_surface_with_html_js_provider();

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("sibling modules with distinct JS providers should compile");

    let artefacts = modules
        .project
        .successful_artefacts_in_module_id_order()
        .collect::<Vec<_>>();
    assert_eq!(
        artefacts.len(),
        2,
        "expected both sibling module artefacts to compile"
    );

    let module_a = artefacts
        .iter()
        .find(|artefact| {
            artefact
                .module
                .metadata
                .entry_point
                .ends_with(Path::new("a/@a.moth"))
        })
        .expect("expected a sibling module artefact");
    let module_a_b = artefacts
        .iter()
        .find(|artefact| {
            artefact
                .module
                .metadata
                .entry_point
                .ends_with(Path::new("a-b/@b.moth"))
        })
        .expect("expected a-b sibling module artefact");

    let module_a_provider_package_paths = module_a
        .module
        .link_facts
        .external_import_candidates
        .iter()
        .filter_map(|candidate| {
            module_a
                .module
                .link_facts
                .external_package_registry
                .get_package_by_id(candidate.package_id)
                .map(|package| package.path.as_str())
        })
        .filter(|path| path.starts_with("@html-js/"))
        .collect::<Vec<_>>();
    assert_eq!(
        module_a_provider_package_paths,
        vec!["@html-js/a/drawing.js"],
        "a module should retain only its own JS provider package"
    );

    let module_a_b_provider_package_paths = module_a_b
        .module
        .link_facts
        .external_import_candidates
        .iter()
        .filter_map(|candidate| {
            module_a_b
                .module
                .link_facts
                .external_package_registry
                .get_package_by_id(candidate.package_id)
                .map(|package| package.path.as_str())
        })
        .filter(|path| path.starts_with("@html-js/"))
        .collect::<Vec<_>>();
    assert_eq!(
        module_a_b_provider_package_paths,
        vec!["@html-js/a-b/drawing.js"],
        "a-b module should retain only its own JS provider package"
    );
}

#[test]
fn html_js_provider_direct_selection_resolves() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "@drawing.js draw as render\nvalue = render()\n",
    )
    .expect("should write page");
    fs::write(
        dir.join("drawing.js"),
        "/**\n * @moth.sig draw || -> Int\n */\nexport function draw() { return 1; }\n",
    )
    .expect("should write js");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = builder_surface_with_html_js_provider();

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("real JS provider direct selections should compile");

    assert!(
        modules
            .successful_module_views()
            .any(|module| module_contains_external_module_export(module, "draw")),
        "HIR should preserve direct-selection alias JS export metadata"
    );
}

#[test]
fn html_js_provider_direct_alias_for_function_and_opaque_type_resolves() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "@drawing.js Widget as Canvas, draw as render\nvalue = render()\n",
    )
    .expect("should write page");
    fs::write(
        dir.join("drawing.js"),
        "/**\n * @moth.opaque Widget\n */\n/**\n * @moth.sig draw || -> Int\n */\nexport function draw() { return 1; }\n",
    )
    .expect("should write js");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = builder_surface_with_html_js_provider();

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("direct alias for function and opaque type should compile");

    assert!(
        modules
            .successful_module_views()
            .any(|module| module_contains_external_module_export(module, "draw")),
        "HIR should contain provider export metadata for aliased JS function"
    );
}

#[test]
fn html_js_provider_receiver_method_in_project_local_js_rejected() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "@drawing.js make_canvas, fill_rect\ncanvas ~= make_canvas()\n~canvas.fill_rect(0.0, 0.0, 1.0, 1.0)\n",
    )
    .expect("should write page");
    fs::write(
        dir.join("drawing.js"),
        "/**\n * @moth.opaque Canvas\n */\n/**\n * @moth.sig make_canvas || -> Canvas\n */\nexport function makeCanvas() {\n    return {};\n}\n/**\n * @moth.sig fill_rect |this ~Canvas, x Float, y Float, width Float, height Float|\n */\nexport function fillRect(ctx, x, y, width, height) {}\n",
    )
    .expect("should write js with receiver-style signature");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = builder_surface_with_html_js_provider();

    let messages = match compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    ) {
        Ok(_) => panic!("project-local JS receiver-style signature should be rejected"),
        Err(messages) => messages,
    };

    assert!(
        messages.has_errors(),
        "expected at least one error diagnostic for project-local JS receiver-style signature"
    );
    assert_has_diagnostic_code(&messages, "MOTH-IMPORT-0022");
}

#[test]
fn html_js_provider_repeated_imports_reuse_cache() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "@drawing.js draw\n@other run\nvalue = draw()\nother_value = run()\n",
    )
    .expect("should write entry");
    fs::write(
        dir.join("other.moth"),
        "@drawing.js draw as render_again\nrun || -> Int:\n    return render_again()\n;\n",
    )
    .expect("should write helper source");
    fs::write(
        dir.join("drawing.js"),
        "/**\n * @moth.sig draw || -> Int\n */\nexport function draw() { return 1; }\n",
    )
    .expect("should write js");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = builder_surface_with_html_js_provider();

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("repeated JS imports should compile");

    let module = modules
        .successful_module_views()
        .next()
        .expect("expected one module");

    assert_eq!(
        module.link_facts.external_import_candidates.len(),
        1,
        "same JS file imported twice should produce one deduped module external import"
    );
}

#[test]
fn html_js_provider_fallible_function_with_error_return_compiles() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "@drawing.js Canvas, get_canvas\nrun || -> Canvas, Error!:\n    return get_canvas(\"game\")!\n;\n",
    )
    .expect("should write page");
    fs::write(
        dir.join("drawing.js"),
        "import { mothOk } from \"@moth/runtime\";\n/**\n * @moth.opaque Canvas\n */\n/**\n * @moth.sig get_canvas |id String| -> Canvas, Error!\n */\nexport function getCanvas(id) {\n    return mothOk({});\n}\n",
    )
    .expect("should write js");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = builder_surface_with_html_js_provider();

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("fallible JS function with Error! should compile");

    assert!(
        modules
            .successful_module_views()
            .any(|module| module_contains_external_module_export(module, "getCanvas")),
        "HIR should contain JS export metadata for fallible JS function"
    );
}
