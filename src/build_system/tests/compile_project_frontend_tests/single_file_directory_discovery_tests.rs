use super::super::create_project_modules_tests::{diagnosed_reports, successful_compilation_value};
use super::*;
use crate::build_system::create_project_modules::ProjectFrontendCompilation;
use crate::compiler_frontend::compiler_messages::DiagnosticDiscoveryStops;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
#[test]
fn single_file_remaps_module_type_environment_nominal_fields() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    let moth_path = dir.join("test.moth");
    fs::write(
        &moth_path,
        "Point = |\n    value I32,\n|\npoint = Point(1)\n",
    )
    .expect("should write .moth");

    let mut config = Config::new(moth_path.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let mut path_fork = PathInternerFork::empty();
    warning_owner.string_table_mut().intern("preexisting");

    let modules = successful_compilation_value(compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
    ));

    let module = modules
        .successful_module_views()
        .next()
        .expect("expected compiled module");
    let test_path = path_fork
        .try_intern_portable_path("test.moth", warning_owner.string_table_mut())
        .expect("test path fits");
    let point_path = path_fork
        .try_intern_child(test_path, warning_owner.string_table_mut().intern("Point"))
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
            warning_owner.string_table(),
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
            .map(|id| warning_owner.string_table().resolve(id)),
        Some("value")
    );
    assert_eq!(
        display_type(
            fields[0].type_id,
            &module.executable.type_environment,
            warning_owner.string_table(),
            &module.executable.path_table,
        ),
        "I32"
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
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let _path_fork = PathInternerFork::empty();

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
    );

    let reports = diagnosed_reports(&result);
    assert_eq!(reports.counts().errors, 1);
    assert_eq!(reports.counts().warnings, 0);
    assert!(
        reports
            .iter_severity(DiagnosticSeverity::Error)
            .any(|(_, view)| view.descriptor().code == "MOTH-IMPORT-0026"),
        "unsupported source files should use the authored entry diagnostic"
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
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let _path_fork = PathInternerFork::empty();

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
    );

    let Err(errors) = result else {
        panic!("missing entry file should produce an infrastructure error, not success");
    };
    assert_eq!(errors.iter().count(), 1);
    assert_eq!(errors.primary.kind, InfrastructureFailureKind::Filesystem);
    assert!(errors.additional.is_empty());
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
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let _path_fork = PathInternerFork::empty();

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
    );

    let reports = diagnosed_reports(&result);
    let (report, view) = reports
        .iter_severity(DiagnosticSeverity::Error)
        .next()
        .expect("expected one diagnostic");
    assert_eq!(view.descriptor().code, "MOTH-RULE-0060");
    let rendered =
        terse::format_terse_diagnostic_view(view, DiagnosticRenderContext::for_report(report));
    assert!(
        rendered.contains("Core package '@core/text' is not supported by this builder."),
        "unsupported builder package should render the exact selected package-path fact: {rendered}"
    );
}

#[test]
fn single_file_without_source_package_dependencies_skips_registered_package_materialisation() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    let package_root = dir.join("packages/unused");
    fs::create_dir_all(&package_root).expect("should create registered package root");
    fs::write(
        package_root.join("@mod.moth"),
        "$export\nunused || -> I32:\n    return missing_package_value\n;\n",
    )
    .expect("should write semantically invalid unused package");

    let moth_path = dir.join("main.moth");
    fs::write(&moth_path, "value = 1\n").expect("should write plain consumer");

    let mut config = Config::new(moth_path);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root("unused", package_root, PackageOrigin::Builder)
        .expect("test source root should register");

    let frontend = successful_compilation_value(compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    ));

    assert!(
        !frontend.has_diagnosed_or_blocked(),
        "an unreferenced registered package body must not add diagnostics"
    );
    assert_eq!(
        frontend.source_packages.len(),
        0,
        "an unreferenced registered package must not be published"
    );
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(dir.join("page/@entry.moth"), "x ~= 10\n").expect("should write page");
    fs::write(dir.join("layout/@entry.moth"), "y ~= 20\n").expect("should write layout");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let _path_fork = PathInternerFork::empty();

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
    );

    let modules = successful_compilation_value(result);
    let views = modules.successful_module_views().collect::<Vec<_>>();
    assert_eq!(
        views.len(),
        2,
        "expected exactly two successful modules in the retained boundary"
    );
    let shared_path_table = &views[0].executable.path_table;
    assert!(
        views.iter().all(|module| {
            std::sync::Arc::ptr_eq(&module.executable.path_table, shared_path_table)
        }),
        "all base modules in one boundary must share its final path table"
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("first/@entry.moth"),
        "Item = |\n    shared I32,\n    first_only String,\n|\nitem = Item(1, \"first\")\n",
    )
    .expect("should write first entry");
    fs::write(
        dir.join("second/@entry.moth"),
        "Item = |\n    shared I32,\n    second_only String,\n|\nitem = Item(1, \"second\")\n",
    )
    .expect("should write second entry");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let _path_fork = PathInternerFork::empty();

    let modules = successful_compilation_value(compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
    ));

    let second_module = modules
        .successful_module_views()
        .find(|module| {
            module
                .metadata
                .entry
                .as_ref()
                .is_some_and(|entry| entry.entry_point.ends_with("second/@entry.moth"))
        })
        .expect("expected @entry.moth module");
    let item_path = (0..second_module.executable.path_table.len())
        .filter_map(PathId::try_from_index)
        .find(|path| {
            second_module.executable.path_table.render_portable(
                *path,
                warning_owner.string_table(),
                &mut Vec::new(),
            ) == "second/@entry.moth/Item"
        })
        .expect("Item path should exist in the published path table");
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
                .map(|id| warning_owner.string_table().resolve(id))
        })
        .collect::<Vec<_>>();

    assert_eq!(
        display_type(
            item_type_id,
            &second_module.executable.type_environment,
            warning_owner.string_table(),
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
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let _path_fork = PathInternerFork::empty();

    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root("widgets", widget_lib, PackageOrigin::ProjectLocal)
        .expect("test source root should register");

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    );
    let reports = diagnosed_reports(&result);
    assert!(
        reports
            .iter_severity(DiagnosticSeverity::Error)
            .any(|(_, view)| {
                view.descriptor().code == "MOTH-CONFIG-0001"
                    && view.reason_key() == Some("invalid_config.source_file_folder_collision")
            }),
        "expected SourceFileFolderCollision diagnostic"
    );
}

#[test]
fn single_file_project_cutoff_after_package_work_refuses_module_admission() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    let package_root = dir.join("packages/shared");
    fs::create_dir_all(&package_root).expect("should create registered package root");
    fs::write(
        package_root.join("@mod.moth"),
        "$export\nrender || -> I32:\n    return shared_missing\n;\n",
    )
    .expect("should write diagnostically invalid package");
    let moth_path = dir.join("main.moth");
    fs::write(
        &moth_path,
        "@shared render\nvalue = render() catch:\n    then 0\n;\n",
    )
    .expect("should write package consumer");

    let mut config = Config::new(moth_path.clone());
    config.max_errors_per_project = std::num::NonZeroU32::new(1);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root("shared", package_root, PackageOrigin::Builder)
        .expect("test source root should register");
    crate::compiler_frontend::pipeline_test_support::reset_file_frontend_prepare_count_for_test(
        &dir,
    );

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    );

    // The package error meets the threshold during source-package work, so the synthetic entry
    // module is refused at boundary entry and never reaches module preparation. Stage 0 discovery
    // prepared the entry once before that budget decision, and an admitted module would reuse the
    // `MothPrepared` output instead of preparing the file again, so the counter observes exactly
    // one discovery preparation rather than zero. The refusal itself is proven by the recorded
    // project cutoff below; this test also keeps the package diagnostic and its rendered fact
    // intact across the cutoff.
    let reports = diagnosed_reports(&result);
    assert_eq!(
        reports.counts().errors,
        1,
        "the package diagnostic stays retained after the cutoff"
    );
    let rendered = terse::format_terse_diagnostic_reports(reports);
    assert!(
        rendered.iter().any(|line| line.contains("shared_missing")),
        "the package error must be reported: {rendered:?}"
    );
    assert_eq!(
        crate::compiler_frontend::pipeline_test_support::file_frontend_prepare_count_for_path_for_test(
            &moth_path
        ),
        1,
        "Stage 0 discovery prepares the entry source exactly once; no second preparation follows"
    );
    assert!(
        warning_owner
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::PROJECT_SCHEDULING_CUTOFF),
        "refusing the unvisited module is a recorded project cutoff"
    );
}

#[test]
fn check_project_with_root_entry_only_publishes_no_interface() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    fs::write(
        dir.join("config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(dir.join("@entry.moth"), "value ~= 1\n").expect("should write the entry root");

    let mut config = Config::new(dir);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let mut project_source_files = None;
    let mut project_source_builder = Some(
        crate::compiler_frontend::source::SourceDatabaseBuilder::new(
            crate::compiler_frontend::source::SourceDatabase::empty(),
        ),
    );

    let result = compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
        &mut project_source_files,
        &mut project_source_builder,
        &crate::compiler_frontend::build_config::BuildConfigInputSet::new(),
        FrontendCompilationMode::Check,
    );

    let modules = successful_compilation_value(result);
    let views = modules.successful_module_views().collect::<Vec<_>>();
    assert_eq!(
        views.len(),
        1,
        "an entry-only root directory is one structural module"
    );
    let artifacts = modules
        .project
        .successful_artefacts_in_module_id_order()
        .collect::<Vec<_>>();
    assert_eq!(artifacts.len(), 1);
    assert!(
        artifacts[0].normal_interface.is_none(),
        "an entry-only module must not publish a normal interface"
    );
    assert!(
        artifacts[0].support_interface.is_none(),
        "an entry-only module must not publish a support interface"
    );
}

/// Compile one explicitly selected file through the real synthetic single-file path.
///
/// WHY: selection by `Config::entry_dir` is part of the contract under test, so these cases must
///      not reuse the directory-lane helpers or build a Config from a directory.
fn compile_selected_file(
    entry: PathBuf,
    mode: FrontendCompilationMode,
) -> CompilationResult<ProjectFrontendCompilation> {
    let mut config = Config::new(entry);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let mut project_source_files = None;
    let mut project_source_builder = None;
    compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
        &mut project_source_files,
        &mut project_source_builder,
        &BuildConfigInputSet::new(),
        mode,
    )
}

#[test]
fn check_single_file_selected_root_keeps_its_authored_role_in_a_triple_root_directory() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    fs::write(
        dir.join("@mod.moth"),
        "$export\nnormal_label #= \"normal\"\n",
    )
    .expect("should write normal facade");
    fs::write(
        dir.join("@package.moth"),
        "$export\nsupport_label #= \"support\"\n",
    )
    .expect("should write support facade");
    fs::write(dir.join("@entry.moth"), "[:triple]\n").expect("should write entry root");
    fs::write(dir.join("helper.moth"), "helper #= 1\n").expect("should write shared helper");

    // Selecting one root in a triple-root directory compiles exactly that authored role as the
    // synthetic unit. Sibling roots stay bounded Stage 0 facts and must not leak a second surface
    // or replace the selected root's role.
    for (selected, normal, support, entry) in [
        ("@mod.moth", true, false, false),
        ("@package.moth", false, true, false),
        ("@entry.moth", false, false, true),
    ] {
        let result = compile_selected_file(dir.join(selected), FrontendCompilationMode::Check);
        let compilation = successful_compilation_value(result);
        let artifacts = compilation
            .project
            .successful_artefacts_in_module_id_order()
            .collect::<Vec<_>>();
        assert_eq!(artifacts.len(), 1, "{selected}: one synthetic module");
        let artifact = artifacts[0];
        assert_eq!(
            artifact.normal_interface.is_some(),
            normal,
            "{selected}: normal facade ownership"
        );
        assert_eq!(
            artifact.support_interface.is_some(),
            support,
            "{selected}: support facade ownership"
        );
        assert_eq!(
            artifact.module.metadata.entry.is_some(),
            entry,
            "{selected}: only the entry root owns executable entry metadata"
        );
        if entry {
            let metadata = artifact
                .module
                .metadata
                .entry
                .as_ref()
                .expect("the selected entry root carries entry metadata");
            assert!(
                metadata.entry_point.ends_with("@entry.moth"),
                "{selected}: the selected entry root is the entry point"
            );
        }
    }
}
