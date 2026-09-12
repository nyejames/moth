use super::*;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
#[test]
fn single_file_remaps_module_type_environment_nominal_fields() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    let moth_path = dir.join("test.moth");
    fs::write(
        &moth_path,
        "Point = |\n    value Int,\n|\npoint = Point(1)\n",
    )
    .expect("should write .moth");

    let mut config = Config::new(moth_path.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
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
        .successful_module_views()
        .next()
        .expect("expected compiled module");
    let test_path = path_fork
        .try_intern_portable_path("test.moth", &mut string_table)
        .expect("test path fits");
    let point_path = path_fork
        .try_intern_child(test_path, string_table.intern("Point"))
        .expect("test path fits");
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
            &string_table,
            &module.executable.path_table,
        ),
        "Point"
    );
    let fields = module
        .executable
        .type_environment
        .fields_for(point_type_id)
        .expect("Point fields should resolve through remapped TypeEnvironment");
    assert_eq!(fields.len(), 1);
    assert_eq!(
        module
            .executable
            .path_table
            .component(fields[0].name)
            .map(|id| string_table.resolve(id)),
        Some("value")
    );
    assert_eq!(
        display_type(
            fields[0].type_id,
            &module.executable.type_environment,
            &string_table,
            &module.executable.path_table,
        ),
        "Int"
    );
}

#[test]
fn single_file_rejects_wrong_extension() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    let txt_path = dir.join("test.txt");
    fs::write(&txt_path, "x ~= 10\n").expect("should write .txt");

    let mut config = Config::new(txt_path);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    );

    let Err(messages) = result else {
        panic!("expected Err for wrong extension");
    };
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("expected at least one error");
    let error_text = terse::format_terse_diagnostic_with_context(
        diagnostic,
        messages.diagnostic_render_context(0),
    );
    assert!(
        error_text.contains(".moth"),
        "expected error to mention .moth, got: {error_text}"
    );
}

#[test]
fn single_file_rejects_missing_file() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    let missing_path = dir.join("does_not_exist.moth");

    let mut config = Config::new(missing_path);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    );

    let Err(messages) = result else {
        panic!("missing entry file should produce an error, not success");
    };
    assert_exact_infrastructure_error(&messages, &ErrorType::File);
}

#[test]
fn single_file_rejects_optional_core_package_not_exposed_by_builder() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    let moth_path = dir.join("test.moth");
    fs::write(&moth_path, "@core/text length\nvalue = length(\"abc\")\n")
        .expect("should write .moth");

    let mut config = Config::new(moth_path);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    );

    let Err(messages) = result else {
        panic!("optional core package should require builder opt-in");
    };
    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("expected one diagnostic");
    let DiagnosticPayload::UnsupportedBuilderPackage { package_path } = diagnostic.payload else {
        panic!("unexpected diagnostic payload: {:?}", diagnostic.payload);
    };
    assert_eq!(messages.string_table.resolve(package_path), "@core/text");
}

// ── Directory-project flow ────────────────────────────────────────────────────

#[test]
fn directory_project_discovers_multiple_entry_modules() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("page")).expect("should create page dir");
    fs::create_dir_all(dir.join("layout")).expect("should create layout dir");
    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(dir.join("page/@page.moth"), "x ~= 10\n").expect("should write page");
    fs::write(dir.join("layout/@layout.moth"), "y ~= 20\n").expect("should write layout");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();

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
        result
            .expect("checked above")
            .successful_module_views()
            .count(),
        2,
        "expected exactly two modules"
    );
}

#[test]
fn directory_project_remaps_delta_collisions_across_modules() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("first")).expect("should create first module dir");
    fs::create_dir_all(dir.join("second")).expect("should create second module dir");
    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
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
    let mut path_fork = PathInternerFork::empty();

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
        .successful_module_views()
        .find(|module| {
            module
                .metadata
                .entry_point
                .file_name()
                .and_then(|name| name.to_str())
                == Some("@b.moth")
        })
        .expect("expected @b.moth module");
    let module_path = path_fork
        .try_intern_filesystem_path(Path::new("second/@b.moth"), &mut string_table)
        .expect("test path should be UTF-8");
    let item_path = path_fork
        .try_intern_child(module_path, string_table.intern("Item"))
        .expect("test path fits");
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
        .map(|field| {
            second_module
                .executable
                .path_table
                .component(field.name)
                .map(|id| string_table.resolve(id))
        })
        .collect::<Vec<_>>();

    assert_eq!(
        display_type(
            item_type_id,
            &second_module.executable.type_environment,
            &string_table,
            &second_module.executable.path_table,
        ),
        "Item"
    );
    assert_eq!(field_names, vec![Some("shared"), Some("second_only")]);
}
#[test]
fn single_file_rejects_source_package_moth_folder_collision() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

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
    let mut path_fork = PathInternerFork::empty();

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

    let Err(messages) = result else {
        panic!("single-file build should reject source-backed package .moth/folder collision");
    };

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
}
