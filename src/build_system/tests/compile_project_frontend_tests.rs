use super::compile_project_frontend;
use crate::build_system::BuildProfile;
use crate::build_system::build::{BackendBuilder, ProjectCompilation};
use crate::builder_surface::BuilderSurface;
use crate::builder_surface::PackageOrigin;
use crate::builder_surface::external_import_providers::provider::{
    ExternalFileExtension, ExternalImportProvider, ExternalImportProviderContext,
    ExternalImportProviderKind, ExternalImportRequest, ResolvedExternalImport,
    RuntimeAssetIdentity,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terse};
use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, InvalidConfigReason};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::definitions::ChoiceVariantPayloadDefinition;
use crate::compiler_frontend::datatypes::display::display_type;
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalAbiType, ExternalAccessKind, ExternalFunctionId, ExternalFunctionLowerings,
    ExternalFunctionSpec, ExternalJsLowering, ExternalReturnSlot, ExternalSignatureType,
    ExternalTypeId, ExternalTypeSpec,
};
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_tests::test_support::temp_dir;
use crate::projects::settings::Config;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn directory_graph_retains_independent_diagnostics_without_blocked_consumer_cascades() {
    let dir = temp_dir("graph_outcomes_independent_diagnostics");
    fs::create_dir_all(dir.join("provider")).expect("should create provider module");
    fs::create_dir_all(dir.join("consumer")).expect("should create second consumer module");
    fs::create_dir_all(dir.join("independent")).expect("should create independent module");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @provider { run }\nvalue = run()\n",
    )
    .expect("should write blocked consumer");
    fs::write(
        dir.join("consumer/@mod.moth"),
        "import @provider { run }\nvalue = run()\n",
    )
    .expect("should write second blocked consumer");
    fs::write(
        dir.join("provider/+mod.moth"),
        "export:\n    run || -> Int:\n        return missing_provider_value\n    ;\n;\n",
    )
    .expect("should write diagnosed provider");
    fs::write(
        dir.join("independent/@mod.moth"),
        "value = missing_independent_value\n",
    )
    .expect("should write independent diagnosed module");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let messages = match compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    ) {
        Ok(_) => panic!("both provider and independent modules should be diagnosed"),
        Err(messages) => messages,
    };

    assert_eq!(
        messages.error_count(),
        2,
        "the provider and independent branch should each diagnose once; blocked consumers should emit no cascades"
    );
    let diagnosed_paths = messages
        .error_diagnostics()
        .map(|diagnostic| {
            diagnostic
                .primary_location
                .scope
                .to_path_buf(&messages.string_table)
        })
        .collect::<Vec<_>>();
    assert!(
        diagnosed_paths
            .iter()
            .any(|path| path.ends_with("provider/+mod.moth")),
        "provider diagnostic should be retained: {diagnosed_paths:?}"
    );
    assert!(
        diagnosed_paths
            .iter()
            .any(|path| path.ends_with("independent/@mod.moth")),
        "independent branch should continue and retain its diagnostic: {diagnosed_paths:?}"
    );
    assert!(
        diagnosed_paths
            .iter()
            .all(|path| { !path.ends_with("@page.moth") && !path.ends_with("consumer/@mod.moth") }),
        "blocked consumers should not be semantically compiled: {diagnosed_paths:?}"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn directory_graph_retains_diagnostics_from_later_independent_source_packages() {
    let dir = temp_dir("graph_outcomes_source_package_diagnostics");
    let first_package = dir.join("packages/first");
    let second_package = dir.join("packages/second");
    fs::create_dir_all(&first_package).expect("should create first package");
    fs::create_dir_all(&second_package).expect("should create second package");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(dir.join("@page.moth"), "value = 1\n").expect("should write project root");
    fs::write(
        first_package.join("@mod.moth"),
        "export:\n    first || -> Int:\n        return missing_first_package_value\n    ;\n;\n",
    )
    .expect("should write first diagnosed package");
    fs::write(
        second_package.join("@mod.moth"),
        "export:\n    second || -> Int:\n        return missing_second_package_value\n    ;\n;\n",
    )
    .expect("should write second diagnosed package");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "first",
        first_package,
        PackageOrigin::Builder,
    );
    frontend_surface.source_packages.register_filesystem_root(
        "second",
        second_package,
        PackageOrigin::Builder,
    );

    let messages = match compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    ) {
        Ok(_) => panic!("both independent source packages should be diagnosed"),
        Err(messages) => messages,
    };

    assert!(
        messages.error_count() >= 2,
        "both diagnosed source packages should retain their errors"
    );
    let diagnosed_paths = messages
        .error_diagnostics()
        .map(|diagnostic| {
            diagnostic
                .primary_location
                .scope
                .to_path_buf(&messages.string_table)
        })
        .collect::<Vec<_>>();
    assert!(
        diagnosed_paths
            .iter()
            .any(|path| path.ends_with("packages/first/@mod.moth")),
        "first package diagnostic should be retained: {diagnosed_paths:?}"
    );
    assert!(
        diagnosed_paths
            .iter()
            .any(|path| path.ends_with("packages/second/@mod.moth")),
        "later independent package should still compile: {diagnosed_paths:?}"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn same_module_generated_sidecars_rebuild_const_templates_in_their_fresh_store() {
    let dir = temp_dir("generated_const_template_projection");
    fs::create_dir_all(&dir).expect("should create project root");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        r#"shell #= [:<span>[$slot]</span>]
unused_insert #= [$insert("unused"): unused]

wrap type T |value T| -> String:
    return [shell: generated]
;

result = wrap(42)
io.line(result)
"#,
    )
    .expect("should write entry");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    let frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("same-module generated constants should use their own TIR store");
    let (_, _, sidecars) = frontend.into_parts();

    assert_eq!(
        sidecars.len(),
        1,
        "the concrete wrap request needs one sidecar"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn generated_sidecars_reconstruct_complete_generic_nominal_members() {
    let dir = temp_dir("generated_nominal_blueprints");
    fs::create_dir_all(dir.join("provider")).expect("should create provider module");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("provider/@mod.moth"),
        r#"identity type T |value T| -> T:
    return value
;

export:
    forward type T |value T| -> T:
        return identity(value)
    ;
;
"#,
    )
    .expect("should write provider");
    fs::write(
        dir.join("@page.moth"),
        r#"import @provider { forward }

export:
    Box type T = |
        value T,
    |

    Maybe type T ::
        Some | value T |,
        Empty,
    ;
;

PrivateBox type T = |
    value T,
|

box Box of Int = Box(42)
same_box Box of Int = forward(box)
maybe Maybe of String = Maybe::Some("stable")
same_maybe Maybe of String = forward(maybe)
private_box PrivateBox of Bool = PrivateBox(true)
same_private_box PrivateBox of Bool = forward(private_box)
"#,
    )
    .expect("should write entry");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    let frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("public generic nominal arguments should materialise");
    let (_, _, sidecars) = frontend.into_parts();

    assert_eq!(
        sidecars.len(),
        6,
        "each outer request and nested private identity request needs one sidecar"
    );
    for sidecar in sidecars {
        let argument = sidecar
            .identity
            .type_arguments()
            .first()
            .expect("generated request should have one type argument");
        let base_name = match argument {
            crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity::GenericInstance(
                instance,
            ) => instance.base().defining_name(),
            crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity::ModulePrivateGenericInstance(
                instance,
            ) => instance.base().defining_path(),
            _ => panic!("request argument should retain generic-instance identity"),
        };
        let environment = &sidecar.module.executable.type_environment;
        let instance_type_id = environment
            .type_id_for_canonical_identity(argument)
            .expect("generated environment should intern the request type");

        match base_name {
            "Box" => {
                let fields = environment
                    .fields_for(instance_type_id)
                    .expect("generated Box instance should expose substituted fields");
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].type_id, builtin_type_ids::INT);
            }
            "Maybe" => {
                let variants = environment
                    .variants_for(instance_type_id)
                    .expect("generated Maybe instance should expose substituted variants");
                assert_eq!(variants.len(), 2);
                let ChoiceVariantPayloadDefinition::Record { fields } = &variants[0].payload else {
                    panic!("Some should retain its record payload");
                };
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].type_id, builtin_type_ids::STRING);
                assert!(matches!(
                    variants[1].payload,
                    ChoiceVariantPayloadDefinition::Unit
                ));
            }
            name if name.ends_with("PrivateBox") => {
                let fields = environment
                    .fields_for(instance_type_id)
                    .expect("generated private Box instance should expose substituted fields");
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].type_id, builtin_type_ids::BOOL);
            }
            other => panic!("unexpected generic nominal request base {other}"),
        }
    }

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn generated_sidecars_reconstruct_hidden_facade_nominal_closure() {
    let dir = temp_dir("generated_hidden_facade_nominal");
    fs::create_dir_all(dir.join("facade/provider")).expect("should create provider module");
    fs::create_dir_all(dir.join("generics")).expect("should create generic provider module");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("facade/provider/@mod.moth"),
        r#"export:
    Hidden = |
        value Int,
    |

    Wrapper = |
        hidden Hidden,
    |

    make || -> Wrapper:
        return Wrapper(Hidden(42))
    ;
;
"#,
    )
    .expect("should write provider");
    fs::write(
        dir.join("facade/@mod.moth"),
        r#"export:
    import @provider { Wrapper, make }
;
"#,
    )
    .expect("should write facade");
    fs::write(
        dir.join("generics/@mod.moth"),
        r#"export:
    identity type T |value T| -> T:
        return value
    ;
;
"#,
    )
    .expect("should write generic provider");
    fs::write(
        dir.join("@page.moth"),
        r#"import @facade { Wrapper, make }
import @generics { identity }

wrapped Wrapper = identity(make())
"#,
    )
    .expect("should write entry");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    let frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("facade-hidden nominal closure should materialise");
    let (_, _, sidecars) = frontend.into_parts();

    assert_eq!(sidecars.len(), 1);
    let sidecar = &sidecars[0];
    let wrapper_identity = sidecar
        .identity
        .type_arguments()
        .first()
        .expect("identity request should retain Wrapper");
    let environment = &sidecar.module.executable.type_environment;
    let wrapper_type_id = environment
        .type_id_for_canonical_identity(wrapper_identity)
        .expect("generated environment should intern Wrapper");
    let wrapper_fields = environment
        .fields_for(wrapper_type_id)
        .expect("generated Wrapper should retain its field");
    assert_eq!(wrapper_fields.len(), 1);

    let hidden_fields = environment
        .fields_for(wrapper_fields[0].type_id)
        .expect("facade-hidden provider nominal should retain its fields");
    assert_eq!(hidden_fields.len(), 1);
    assert_eq!(hidden_fields[0].type_id, builtin_type_ids::INT);

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

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

fn module_contains_external_call(module: &crate::build_system::build::Module) -> bool {
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
    module: &crate::build_system::build::Module,
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

fn assert_has_diagnostic_code(messages: &CompilerMessages, expected_code: &str) {
    let actual_codes = messages
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.kind.code())
        .collect::<Vec<_>>();

    assert!(
        actual_codes.contains(&expected_code),
        "expected diagnostic code {expected_code}, got {actual_codes:?}"
    );
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
            runtime_asset: Some(RuntimeAssetIdentity {
                canonical_source_path: request.canonical_source_path.clone(),
                asset_kind: "js".to_owned(),
            }),
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
    let dir = temp_dir("provider_registry_survives");
    fs::create_dir_all(&dir).expect("should create temp dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @./drawing.js { draw }\nvalue = draw()\n",
    )
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
        .project_modules()
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

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn provider_runtime_assets_deduped_for_repeated_imports() {
    let dir = temp_dir("provider_runtime_assets_deduped");
    fs::create_dir_all(&dir).expect("should create temp dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @./drawing.js { draw }\nimport @other { run }\nvalue = draw()\nother_value = run()\n",
    )
    .expect("should write entry");
    fs::write(
        dir.join("other.moth"),
        "import @./drawing.js { draw as render }\nrun || -> Int:\n    return render()\n;\n",
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
        .project_modules()
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

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn entry_runtime_metadata_ignores_unreachable_external_calls() {
    let dir = temp_dir("provider_runtime_metadata_unreachable");
    fs::create_dir_all(&dir).expect("should create temp dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(dir.join("@page.moth"), "import @other { run }\nvalue = 1\n")
        .expect("should write entry");
    fs::write(
        dir.join("other.moth"),
        "import @./drawing.js { get_number }\nrun || -> Int, Error!:\n    return get_number()!\n;\n",
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
        .project_modules()
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

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn entry_runtime_metadata_ignores_unreachable_source_package_wrappers() {
    let dir = temp_dir("builder_runtime_metadata_unreachable");
    fs::create_dir_all(&dir).expect("should create temp dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @html { canvas }\npage_canvas_id #= canvas\nvalue = 1\n",
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
        .project_modules()
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

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn provider_backed_import_with_js_lowering_passes_html_build() {
    let dir = temp_dir("provider_js_lowering_html");
    fs::create_dir_all(&dir).expect("should create temp dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @./drawing.js { draw }\nvalue = draw()\n",
    )
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

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn single_file_remaps_module_type_environment_nominal_fields() {
    let dir = temp_dir("single_file_type_env_remap");
    fs::create_dir_all(&dir).expect("should create temp dir");
    let moth_path = dir.join("test.moth");
    fs::write(
        &moth_path,
        "Point = |\n    value Int,\n|\npoint = Point(1)\n",
    )
    .expect("should write .moth");

    let mut config = Config::new(moth_path.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    string_table.intern("preexisting");

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    )
    .expect("expected Ok for nominal type module");

    let module = modules
        .project_modules()
        .next()
        .expect("expected compiled module");
    let point_path = InternedPath::from_single_str("test.moth", &mut string_table)
        .join_str("Point", &mut string_table);
    let nominal_id = module
        .executable
        .type_environment
        .nominal_id_for_path(&point_path)
        .expect("Point nominal path should be remapped into build string table");
    let point_type_id = module
        .executable
        .type_environment
        .type_id_for_nominal_id(nominal_id)
        .expect("Point nominal type id should be registered");

    assert_eq!(
        display_type(
            point_type_id,
            &module.executable.type_environment,
            &string_table
        ),
        "Point"
    );
    let fields = module
        .executable
        .type_environment
        .fields_for(point_type_id)
        .expect("Point fields should resolve through remapped TypeEnvironment");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name.name_str(&string_table), Some("value"));
    assert_eq!(
        display_type(
            fields[0].type_id,
            &module.executable.type_environment,
            &string_table
        ),
        "Int"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn single_file_rejects_wrong_extension() {
    let dir = temp_dir("single_file_wrong_ext");
    fs::create_dir_all(&dir).expect("should create temp dir");
    let txt_path = dir.join("test.txt");
    fs::write(&txt_path, "x ~= 10\n").expect("should write .txt");

    let mut config = Config::new(txt_path);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    );

    assert!(result.is_err(), "expected Err for wrong extension");
    let messages = result.err().expect("checked above");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("expected at least one error");
    let error_text = terse::format_terse_diagnostic_with_context(
        diagnostic,
        DiagnosticRenderContext::new(&messages.string_table),
    );
    assert!(
        error_text.contains(".moth"),
        "expected error to mention .moth, got: {error_text}"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn single_file_rejects_missing_file() {
    let dir = temp_dir("single_file_missing");
    fs::create_dir_all(&dir).expect("should create temp dir");
    let missing_path = dir.join("does_not_exist.moth");

    let mut config = Config::new(missing_path);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    );

    assert!(result.is_err(), "expected Err for missing file");
    assert!(
        result.err().expect("checked above").error_count() > 0,
        "expected at least one error"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn single_file_rejects_optional_core_package_not_exposed_by_builder() {
    let dir = temp_dir("single_file_optional_core_not_exposed");
    fs::create_dir_all(&dir).expect("should create temp dir");
    let moth_path = dir.join("test.moth");
    fs::write(
        &moth_path,
        "import @core/text {length}\nvalue = length(\"abc\")\n",
    )
    .expect("should write .moth");

    let mut config = Config::new(moth_path);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    );

    assert!(
        result.is_err(),
        "optional core package should require builder opt-in"
    );
    let messages = result.err().expect("checked above");
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("expected one diagnostic");
    let DiagnosticPayload::UnsupportedBuilderPackage { package_path } = diagnostic.payload else {
        panic!("unexpected diagnostic payload: {:?}", diagnostic.payload);
    };
    assert_eq!(messages.string_table.resolve(package_path), "@core/text");

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

// ── Directory-project flow ────────────────────────────────────────────────────

#[test]
fn directory_project_discovers_multiple_entry_modules() {
    let dir = temp_dir("dir_multi_module");
    fs::create_dir_all(dir.join("page")).expect("should create page dir");
    fs::create_dir_all(dir.join("layout")).expect("should create layout dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(dir.join("page/@page.moth"), "x ~= 10\n").expect("should write page");
    fs::write(dir.join("layout/@layout.moth"), "y ~= 20\n").expect("should write layout");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    );

    assert!(
        result.is_ok(),
        "expected Ok for multi-module directory project"
    );
    assert_eq!(
        result.expect("checked above").project_modules().count(),
        2,
        "expected exactly two modules"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn directory_project_remaps_delta_collisions_across_modules() {
    let dir = temp_dir("dir_delta_remap_collision");
    fs::create_dir_all(dir.join("first")).expect("should create first module dir");
    fs::create_dir_all(dir.join("second")).expect("should create second module dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("first/@a.moth"),
        "Item = |\n    shared Int,\n    first_only String,\n|\nitem = Item(1, \"first\")\n",
    )
    .expect("should write first entry");
    fs::write(
        dir.join("second/@b.moth"),
        "Item = |\n    shared Int,\n    second_only String,\n|\nitem = Item(1, \"second\")\n",
    )
    .expect("should write second entry");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();

    let modules = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    )
    .expect("expected Ok for multi-module directory project");

    let second_module = modules
        .project_modules()
        .find(|module| {
            module
                .metadata
                .entry_point
                .file_name()
                .and_then(|name| name.to_str())
                == Some("@b.moth")
        })
        .expect("expected @b.moth module");
    let item_path =
        InternedPath::try_from_filesystem_path(Path::new("second/@b.moth"), &mut string_table)
            .expect("test path should be UTF-8")
            .join_str("Item", &mut string_table);
    let nominal_id = second_module
        .executable
        .type_environment
        .nominal_id_for_path(&item_path)
        .expect("Item nominal path should be remapped for the second module");
    let item_type_id = second_module
        .executable
        .type_environment
        .type_id_for_nominal_id(nominal_id)
        .expect("Item nominal type should be registered");
    let fields = second_module
        .executable
        .type_environment
        .fields_for(item_type_id)
        .expect("Item fields should resolve through remapped TypeEnvironment");
    let field_names = fields
        .iter()
        .map(|field| field.name.name_str(&string_table))
        .collect::<Vec<_>>();

    assert_eq!(
        display_type(
            item_type_id,
            &second_module.executable.type_environment,
            &string_table
        ),
        "Item"
    );
    assert_eq!(field_names, vec![Some("shared"), Some("second_only")]);

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn provider_backed_grouped_import_compiles_and_reuses_cache() {
    let dir = temp_dir("provider_grouped_import_cache");
    fs::create_dir_all(&dir).expect("should create temp dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @./drawing.js { draw as render }\nimport @other { run }\nvalue = render()\nother_value = run()\n",
    )
    .expect("should write page");
    fs::write(
        dir.join("other.moth"),
        "import @./drawing.js { draw as render_again }\nrun || -> Int:\n    return render_again()\n;\n",
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
    .expect("provider-backed grouped imports should compile");

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "same canonical JS file should be resolved through the provider once"
    );
    assert!(
        modules.project_modules().any(module_contains_external_call),
        "HIR should lower provider-backed grouped calls to external function IDs"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn provider_backed_namespace_import_exposes_function_and_type_members() {
    let dir = temp_dir("provider_namespace_import");
    fs::create_dir_all(&dir).expect("should create temp dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @./drawing.js\nwidget drawing.Widget = drawing.make_widget()\nvalue = drawing.draw()\n",
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
    .expect("provider-backed namespace import should compile");

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "namespace import should resolve the JS file once"
    );
    assert!(
        modules.project_modules().any(module_contains_external_call),
        "namespace member calls should lower to external function IDs"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn provider_backed_same_bare_name_from_different_directories_gets_distinct_packages() {
    let dir = temp_dir("provider_same_bare_name_distinct_dirs");
    fs::create_dir_all(dir.join("a")).expect("should create a dir");
    fs::create_dir_all(dir.join("b")).expect("should create b dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @a/use { run_a }\nimport @b/use { run_b }\nvalue_a = run_a()\nvalue_b = run_b()\n",
    )
    .expect("should write page");
    fs::write(
        dir.join("a/use.moth"),
        "import @./helper.js { draw as draw_a }\nrun_a || -> Int:\n    return draw_a()\n;\n",
    )
    .expect("should write a source");
    fs::write(
        dir.join("b/use.moth"),
        "import @./helper.js { draw as draw_b }\nrun_b || -> Int:\n    return draw_b()\n;\n",
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
        modules.project_modules().any(module_contains_external_call),
        "calls through both provider-created packages should lower to external IDs"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn provider_backed_opaque_type_passes_to_same_package_function() {
    let dir = temp_dir("provider_opaque_same_package");
    fs::create_dir_all(&dir).expect("should create temp dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @./drawing.js { make_widget, use_widget }\nwidget = make_widget()\nvalue = use_widget(widget)\n",
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
        modules.project_modules().any(module_contains_external_call),
        "HIR should contain external calls for make_widget and use_widget"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn provider_backed_opaque_type_from_different_package_is_rejected() {
    let dir = temp_dir("provider_opaque_cross_package_rejected");
    fs::create_dir_all(dir.join("a")).expect("should create a dir");
    fs::create_dir_all(dir.join("b")).expect("should create b dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @./a/drawing.js { make_widget }\nimport @./b/drawing.js { use_widget }\nwidget = make_widget()\nvalue = use_widget(widget)\n",
    )
    .expect("should write page");
    fs::write(dir.join("a/drawing.js"), "export function draw() {}\n").expect("should write a js");
    fs::write(dir.join("b/drawing.js"), "export function draw() {}\n").expect("should write b js");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut frontend_surface = builder_surface_with_dummy_js_provider(Arc::clone(&calls));

    let messages = match compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    ) {
        Ok(_) => panic!("cross-package opaque type mismatch should be rejected"),
        Err(messages) => messages,
    };

    assert!(
        messages.error_diagnostics().any(|diagnostic| {
            matches!(&diagnostic.payload, DiagnosticPayload::TypeMismatch { .. })
        }),
        "expected type mismatch diagnostic for cross-package opaque type, got {messages:?}"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn directory_project_rejects_missing_entry_root() {
    let dir = temp_dir("dir_missing_entry_root");
    fs::create_dir_all(&dir).expect("should create temp dir");
    // Config declares an entry_root that does not exist.
    fs::write(dir.join("config.moth"), "entry_root #= \"nonexistent\"\n")
        .expect("should write config");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();

    // Parse config so entry_root is applied to Config.
    let config_path = dir.join("config.moth");
    let frontend_surface = crate::builder_surface::BuilderSurface::with_mandatory_core();
    let services = crate::build_system::project_config::ProjectConfigParseServices {
        style_directives: &style_directives,
        frontend_surface: &frontend_surface,
    };
    let parse_result = crate::build_system::project_config::parse_project_config_file(
        &mut config,
        &config_path,
        &services,
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

    assert!(result.is_err(), "expected Err for missing entry root");
    let messages = result.err().expect("checked above");
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

    fs::remove_dir_all(&dir).expect("should remove temp dir");
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
fn html_js_provider_namespace_import_resolves() {
    let dir = temp_dir("html_js_provider_namespace");
    fs::create_dir_all(&dir).expect("should create temp dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @./drawing.js\nvalue = drawing.draw()\n",
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
    .expect("real JS provider namespace import should compile");

    assert!(
        modules
            .project_modules()
            .any(|module| module_contains_external_module_export(module, "draw")),
        "HIR should preserve namespace JS call export metadata"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn html_js_provider_grouped_import_resolves() {
    let dir = temp_dir("html_js_provider_grouped");
    fs::create_dir_all(&dir).expect("should create temp dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @./drawing.js { draw as render }\nvalue = render()\n",
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
    .expect("real JS provider grouped import should compile");

    assert!(
        modules
            .project_modules()
            .any(|module| module_contains_external_module_export(module, "draw")),
        "HIR should preserve grouped alias JS export metadata"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn html_js_provider_grouped_alias_for_function_and_opaque_type_resolves() {
    let dir = temp_dir("html_js_provider_grouped_alias");
    fs::create_dir_all(&dir).expect("should create temp dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @./drawing.js { Widget as Canvas, draw as render }\nvalue = render()\n",
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
    .expect("grouped alias for function and opaque type should compile");

    assert!(
        modules
            .project_modules()
            .any(|module| module_contains_external_module_export(module, "draw")),
        "HIR should contain provider export metadata for aliased JS function"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn html_js_provider_receiver_method_in_project_local_js_rejected() {
    let dir = temp_dir("html_js_provider_receiver_method_rejected");
    fs::create_dir_all(&dir).expect("should create temp dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @./drawing.js { make_canvas, fill_rect }\ncanvas ~= make_canvas()\n~canvas.fill_rect(0.0, 0.0, 1.0, 1.0)\n",
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

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn html_js_provider_repeated_imports_reuse_cache() {
    let dir = temp_dir("html_js_provider_cache_reuse");
    fs::create_dir_all(&dir).expect("should create temp dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @./drawing.js { draw }\nimport @other { run }\nvalue = draw()\nother_value = run()\n",
    )
    .expect("should write entry");
    fs::write(
        dir.join("other.moth"),
        "import @./drawing.js { draw as render_again }\nrun || -> Int:\n    return render_again()\n;\n",
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
        .project_modules()
        .next()
        .expect("expected one module");

    assert_eq!(
        module.link_facts.external_import_candidates.len(),
        1,
        "same JS file imported twice should produce one deduped module external import"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn html_js_provider_fallible_function_with_error_return_compiles() {
    let dir = temp_dir("html_js_provider_fallible");
    fs::create_dir_all(&dir).expect("should create temp dir");
    fs::write(dir.join("config.moth"), "").expect("should write config");
    fs::write(
        dir.join("@page.moth"),
        "import @./drawing.js { Canvas, get_canvas }\nrun || -> Canvas, Error!:\n    return get_canvas(\"game\")!\n;\n",
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
            .project_modules()
            .any(|module| module_contains_external_module_export(module, "getCanvas")),
        "HIR should contain JS export metadata for fallible JS function"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}

#[test]
fn single_file_rejects_source_package_moth_folder_collision() {
    let dir = temp_dir("single_file_source_package_collision");
    fs::create_dir_all(&dir).expect("should create temp dir");

    // Source-backed package with one valid normal module root plus a .moth/folder collision.
    let widget_lib = dir.join("lib").join("widgets");
    fs::create_dir_all(widget_lib.join("widget")).expect("should create widget folder sibling");
    fs::write(widget_lib.join("widget.moth"), "value #= 1\n")
        .expect("should write colliding widget.moth");
    fs::write(widget_lib.join("@mod.moth"), "value #= 2\n")
        .expect("should write valid normal module root");

    // Main single file that does NOT import the ambiguous source-backed package path.
    let main_path = dir.join("main.moth");
    fs::write(&main_path, "x ~= 1\n").expect("should write main file");

    let mut config = Config::new(main_path.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();

    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "widgets",
        widget_lib,
        PackageOrigin::ProjectLocal,
    );

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    );

    assert!(
        result.is_err(),
        "single-file build should reject source-backed package .moth/folder collision"
    );
    let messages = result.err().expect("checked above");

    assert!(
        messages.error_diagnostics().any(|diagnostic| {
            matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidConfig {
                    reason: InvalidConfigReason::SourceFileFolderCollision { .. },
                    ..
                }
            )
        }),
        "expected SourceFileFolderCollision diagnostic, got {messages:?}"
    );

    fs::remove_dir_all(&dir).expect("should remove temp dir");
}
