use super::super::create_project_modules_tests::{diagnosed_reports, successful_compilation_value};
use super::*;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;

#[test]
fn directory_graph_retains_independent_diagnostics_without_blocked_consumer_cascades() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("provider")).expect("should create provider module");
    fs::create_dir_all(dir.join("consumer")).expect("should create second consumer module");
    fs::create_dir_all(dir.join("independent")).expect("should create independent module");
    fs::write(
        dir.join("config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(dir.join("@page.moth"), "@provider run\nvalue = run()\n")
        .expect("should write blocked consumer");
    fs::write(
        dir.join("consumer/@mod.moth"),
        "@provider run\nvalue = run()\n",
    )
    .expect("should write second blocked consumer");
    fs::write(
        dir.join("provider/+mod.moth"),
        "export:\n    run || -> I32:\n        return missing_provider_value\n    ;\n;\n",
    )
    .expect("should write diagnosed provider");
    fs::write(
        dir.join("independent/@mod.moth"),
        "value = missing_independent_value\n",
    )
    .expect("should write independent diagnosed module");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let string_table = StringTable::new();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
    );
    let reports = diagnosed_reports(&result);
    assert_eq!(
        reports.counts().errors,
        2,
        "the provider and independent branch should each diagnose once; blocked consumers should emit no cascades"
    );
    let diagnosed_paths = reports
        .iter_severity(DiagnosticSeverity::Error)
        .filter_map(|(report, view)| {
            DiagnosticRenderContext::for_report(report)
                .site_position(view.primary_site())
                .map(|position| position.path)
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
}

#[test]
fn registered_source_database_retains_exact_text_for_multiple_compiled_sources() {
    // Hold the shared instrumentation lock; see the mechanism test below.
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temporary project");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("src/about")).expect("should create nested module");
    fs::write(
        dir.join("config.moth"),
        "project #= (\n    name = \"snapshot\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    let root_source = "title String = \"home\"\n";
    let nested_source = "title String = \"about\"\n";
    fs::write(dir.join("src/@page.moth"), root_source).expect("should write root source");
    fs::write(dir.join("src/about/@page.moth"), nested_source).expect("should write nested source");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let string_table = StringTable::new();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);
    let mut builder_surface = BuilderSurface::with_mandatory_core();
    let mut project_source_files = None;
    let mut project_source_builder = None;
    let build_config_inputs = BuildConfigInputSet::new();
    let frontend = successful_compilation_value(compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut builder_surface,
        &mut warning_owner,
        &mut project_source_files,
        &mut project_source_builder,
        &build_config_inputs,
        FrontendCompilationMode::Canonical,
    ));

    let source_files = frontend
        .project_source_database
        .as_ref()
        .expect("compiled project should retain source database");
    for (relative_path, expected_text) in [
        ("src/@page.moth", root_source),
        ("src/about/@page.moth", nested_source),
    ] {
        let path =
            fs::canonicalize(dir.join(relative_path)).expect("source path should canonicalize");
        let record = source_files
            .get_by_canonical_path(&path)
            .expect("compiled source should have a registered record");
        assert_eq!(
            source_files.retained_text(record.id),
            Some(expected_text),
            "retained text should equal the complete source file for {relative_path}"
        );
    }
}

#[test]
fn selected_preload_invalid_utf8_is_a_source_diagnostic() {
    // Hold the shared instrumentation lock; see the mechanism test below.
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temporary project");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("src")).expect("should create source root");
    fs::write(
        dir.join("config.moth"),
        "project #= (\n    name = \"selected_failure\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    let selected_path = dir.join("src/@page.moth");
    fs::write(&selected_path, [b'o', b'k', 0xff, b'\n'])
        .expect("should write invalid UTF-8 source");

    let mut config = Config::new(dir);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let string_table = StringTable::new();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);
    let mut builder_surface = BuilderSurface::with_mandatory_core();
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut builder_surface,
        &mut warning_owner,
    );
    let reports = diagnosed_reports(&result);
    assert_eq!(reports.counts().errors, 1);
    let (report, diagnostic) = reports
        .iter()
        .next()
        .expect("invalid UTF-8 source should diagnose once");
    assert_eq!(diagnostic.descriptor().code, "MOTH-SYNTAX-0040");
    let rendered = terse::format_terse_diagnostic_view(
        diagnostic,
        DiagnosticRenderContext::for_report(report),
    );
    assert!(
        rendered.contains("Source 'src/@page.moth' is not valid UTF-8"),
        "the non-UTF-8 diagnostic must name the selected logical path: {rendered}"
    );
}

#[test]
fn unselected_preload_read_failure_stays_inert() {
    // Hold the shared instrumentation lock; see the mechanism test below.
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temporary project");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("src")).expect("should create source root");
    fs::write(
        dir.join("config.moth"),
        "project #= (\n    name = \"unselected_failure\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(dir.join("src/@page.moth"), "title String = \"home\"\n")
        .expect("should write selected source");
    let unselected_path = dir.join("src/unreachable.moth");
    fs::write(&unselected_path, [b'o', b'k', 0xff, b'\n'])
        .expect("should write invalid UTF-8 source");

    let mut config = Config::new(dir);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let string_table = StringTable::new();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);
    let mut builder_surface = BuilderSurface::with_mandatory_core();
    let mut project_source_files = None;
    let mut project_source_builder = None;
    let build_config_inputs = BuildConfigInputSet::new();
    let frontend = successful_compilation_value(compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut builder_surface,
        &mut warning_owner,
        &mut project_source_files,
        &mut project_source_builder,
        &build_config_inputs,
        FrontendCompilationMode::Canonical,
    ));

    // `Success` proves the selected project compiled; an unselected unreadable source must
    // neither add transient diagnostics nor suppress its independent successful module.
    assert!(
        frontend.transient_batches.is_empty(),
        "preloading an unreadable unselected source must not report anything: {:?}",
        frontend.transient_batches
    );
    assert_eq!(
        frontend.successful_module_views().count(),
        1,
        "the one selected module must still compile successfully"
    );

    let source_files = frontend
        .project_source_database
        .as_ref()
        .expect("successful build should retain source database");
    let unselected_path =
        fs::canonicalize(unselected_path).expect("unselected source path should canonicalize");
    let unselected_id = source_files
        .get_by_canonical_path(&unselected_path)
        .expect("unselected source should still have a registered slot")
        .id;
    assert!(source_files.retained_text(unselected_id).is_none());
    assert!(
        source_files.source_load_error(unselected_id).is_none(),
        "unselected source must remain pending without a filesystem read"
    );
}

#[test]
fn project_facade_rejects_own_project_globals_dependency_before_semantic_use() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temporary project");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("src")).expect("should create entry root");
    fs::write(
        dir.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(dir.join("src/@page.moth"), "value = 1\n").expect("should write project entry");
    // Keep the dependency unused so declaration-time validation cannot rely on binding or
    // reachability analysis to reject the facade's self-dependency.
    fs::write(
        dir.join("+package.moth"),
        "SITE_TITLE #= \"Moth\"\n@project\n",
    )
    .expect("should write project facade with a header naming warning");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let string_table = StringTable::new();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
    );
    let reports = diagnosed_reports(&result);
    assert_eq!(
        reports.counts().warnings,
        0,
        "the facade precheck must not include discarded preparation warnings"
    );
    let matching = reports
        .iter()
        .filter(|(_, view)| {
            view.descriptor().code == "MOTH-SYNTAX-0019"
                && view.reason_key()
                    == Some(
                        "invalid_dependency_clause.project_globals_facade_dependency_not_allowed",
                    )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        matching.len(),
        1,
        "the facade's own @project declaration should produce one structured diagnostic"
    );
    let (report, view) = matching[0];
    let position = DiagnosticRenderContext::for_report(report)
        .site_position(view.primary_site())
        .expect("facade diagnostic should retain its authored source position");
    assert!(
        position.path.ends_with("+package.moth"),
        "self-dependency diagnostic should point to the facade source"
    );
}

#[cfg(feature = "timers")]
#[test]
fn failed_directory_preparation_keeps_unfinished_module_metadata_out_of_completion() {
    // Keep the shared instrumentation lock against tests that start another session. An unguarded
    // compile can still register into this active process-global session, so the assertions below
    // select this fixture's boundary and unfinished module rather than counting the whole snapshot.
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(dir.join("@page.moth"), "@core/math sin,\n#[:ok]\n")
        .expect("should write malformed entry");

    let mut config = Config::new(dir.clone());
    config.project_name = "failed_directory_preparation_test_project".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let string_table = StringTable::new();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);
    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
    );

    let reports = diagnosed_reports(&result);
    assert_has_diagnostic_code(&result, "MOTH-SYNTAX-0019");
    assert_eq!(
        reports.counts().errors,
        1,
        "malformed Stage 0 input should produce exactly the syntax diagnostic"
    );

    let snapshot = timing_session.finish();
    let project_boundaries = snapshot
        .boundaries
        .iter()
        .filter(|boundary| {
            boundary.kind == crate::timing::TimingBoundaryKind::MainProject
                && boundary.display_name == config.project_name
        })
        .collect::<Vec<_>>();
    assert_eq!(
        project_boundaries.len(),
        1,
        "the failed project must register its own main-project boundary"
    );
    let project_boundary = project_boundaries[0].id;
    let unfinished_modules = snapshot
        .modules
        .iter()
        .filter(|module| {
            module.key.boundary() == project_boundary && !module.source_facts_finalized
        })
        .collect::<Vec<_>>();
    assert_eq!(
        unfinished_modules.len(),
        1,
        "the failed project must leave exactly one unfinished module"
    );
    let unfinished = unfinished_modules[0];
    assert_eq!(unfinished.logical_identity, config.project_name);
    assert_eq!(unfinished.source_file_count, 0);
    assert_eq!(unfinished.source_byte_count, 0);
    assert!(module_has_timing(
        unfinished,
        crate::timing::TimingMetric::FrontendPrepare
    ));
}

#[cfg(feature = "timers")]
#[test]
fn directory_frontend_registers_package_and_project_boundaries() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The collector is process-global, so serialize against other collector tests.

    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    // The package root lives outside the project root so the project boundary does not also
    // discover it as an owned module.
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let package_root = _temp.path().to_path_buf();
    // Unique names keep this test's records identifiable when unrelated parallel build tests
    // register their own boundaries into the shared process-global collection scope.
    const PACKAGE_NAME: &str = "phase4_helper";
    const PROJECT_NAME: &str = "phase4_demo_project";
    fs::create_dir_all(&dir).expect("should create project directory");
    fs::create_dir_all(&package_root).expect("should create package directory");
    fs::write(
        dir.join("config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(dir.join("@page.moth"), "value = 1\n").expect("should write project root");
    fs::write(
        package_root.join("@mod.moth"),
        "export:\n    helper || -> I32:\n        return 7\n    ;\n;\n",
    )
    .expect("should write package module");

    let mut config = Config::new(dir.clone());
    config.project_name = PROJECT_NAME.to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let string_table = StringTable::new();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        PACKAGE_NAME,
        package_root.clone(),
        PackageOrigin::Builder,
    );

    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let frontend = successful_compilation_value(compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    ));
    let snapshot = timing_session.finish();
    drop(frontend);

    let package_boundary = snapshot
        .boundaries
        .iter()
        .find(|boundary| boundary.display_name == format!("@{PACKAGE_NAME}"))
        .expect("the source package boundary should be registered")
        .id;
    let project_boundary = snapshot
        .boundaries
        .iter()
        .find(|boundary| boundary.display_name == PROJECT_NAME)
        .expect("the main project boundary should be registered")
        .id;
    let package_boundary_index = snapshot
        .boundaries
        .iter()
        .position(|boundary| boundary.id == package_boundary)
        .expect("registered boundary should be indexed");
    let project_boundary_index = snapshot
        .boundaries
        .iter()
        .position(|boundary| boundary.id == project_boundary)
        .expect("registered boundary should be indexed");
    assert!(
        package_boundary_index < project_boundary_index,
        "source packages register before the main project in deterministic order"
    );
    assert_eq!(
        snapshot
            .boundaries
            .iter()
            .filter(|boundary| {
                boundary.id == package_boundary || boundary.id == project_boundary
            })
            .map(|boundary| (boundary.id, boundary.module_count))
            .collect::<Vec<_>>(),
        vec![(package_boundary, 1), (project_boundary, 1)],
        "one module per boundary (package {package_boundary:?}, project {project_boundary:?}): {:#?}",
        snapshot.modules
    );
    assert!(boundary_has_timing(
        &snapshot,
        package_boundary,
        crate::timing::TimingMetric::BoundaryInventory
    ));
    assert!(boundary_has_timing(
        &snapshot,
        project_boundary,
        crate::timing::TimingMetric::BoundaryInventory
    ));
    assert!(boundary_has_timing(
        &snapshot,
        package_boundary,
        crate::timing::TimingMetric::BoundaryCompile
    ));
    assert!(boundary_has_timing(
        &snapshot,
        project_boundary,
        crate::timing::TimingMetric::BoundaryCompile
    ));

    let own_modules = snapshot
        .modules
        .iter()
        .filter(|module| {
            module.key.boundary() == package_boundary || module.key.boundary() == project_boundary
        })
        .collect::<Vec<_>>();
    assert_eq!(own_modules.len(), 2);
    assert!(
        own_modules
            .iter()
            .any(|module| module.logical_identity == format!("@{PACKAGE_NAME}")),
        "entry-root package modules reuse the boundary display name: {:?}",
        own_modules
    );
    assert!(
        own_modules
            .iter()
            .any(|module| module.logical_identity == PROJECT_NAME),
        "entry-root project modules reuse the boundary display name: {:?}",
        own_modules
    );
    assert!(
        own_modules
            .iter()
            .all(|module| !module.logical_identity.contains(&dir.display().to_string())),
        "module identities must never contain checkout-specific paths"
    );

    let semantic_total_count = own_modules
        .iter()
        .filter(|module| {
            module_has_timing(
                module,
                crate::timing::TimingMetric::FrontendModuleSemanticTotal,
            )
        })
        .count();
    assert_eq!(
        semantic_total_count,
        2,
        "every compilation mode records one semantic total per module (package {package_boundary:?}, project {project_boundary:?}): {:#?}",
        (
            snapshot
                .modules
                .iter()
                .filter(|module| module_has_timing(
                    module,
                    crate::timing::TimingMetric::FrontendModuleSemanticTotal,
                ))
                .map(|module| module.key)
                .collect::<Vec<_>>(),
            snapshot
                .boundaries
                .iter()
                .map(|boundary| (boundary.id, boundary.display_name.as_str()))
                .collect::<Vec<_>>(),
            snapshot
                .modules
                .iter()
                .map(|module| (module.key, module.logical_identity.as_str()))
                .collect::<Vec<_>>(),
        )
    );
    assert!(own_modules.iter().any(|module| {
        module.key.boundary() == package_boundary
            && module_has_timing(
                module,
                crate::timing::TimingMetric::FrontendModuleSemanticTotal,
            )
    }));
    assert!(own_modules.iter().any(|module| {
        module.key.boundary() == project_boundary
            && module_has_timing(
                module,
                crate::timing::TimingMetric::FrontendModuleSemanticTotal,
            )
    }));
}

#[cfg(feature = "timers")]
#[test]
fn directory_frontend_records_incremental_file_prepare_with_module_attribution() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();

    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(dir.join("@page.moth"), "value = 1\n").expect("should write project root");

    let mut config = Config::new(dir.clone());
    config.project_name = "moth_incremental_file_prepare_fixture".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let string_table = StringTable::new();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);

    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let frontend = successful_compilation_value(compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
    ));
    let snapshot = timing_session.finish();
    drop(frontend);

    let project_boundary = snapshot
        .boundaries
        .iter()
        .find(|boundary| {
            boundary.kind == crate::timing::TimingBoundaryKind::MainProject
                && boundary.display_name == config.project_name
        })
        .expect("the main project boundary should be registered")
        .id;
    let project_modules = snapshot
        .modules
        .iter()
        .filter(|module| {
            module.key.boundary() == project_boundary
                && module.logical_identity == config.project_name
        })
        .collect::<Vec<_>>();
    assert_eq!(
        project_modules.len(),
        1,
        "the incremental directory compile must register exactly one project module"
    );
    assert!(
        module_has_timing(
            project_modules[0],
            crate::timing::TimingMetric::FrontendPrepare
        ),
        "incremental directory discovery must record frontend.prepare for its own module"
    );
}

#[cfg(feature = "timers")]
#[test]
fn single_file_frontend_records_file_prepare_with_module_attribution() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();

    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    let moth_path = dir.join("test.moth");
    fs::write(&moth_path, "value = 1\n").expect("should write .moth");

    let mut config = Config::new(moth_path.clone());
    config.project_name = "moth_single_file_prepare_fixture".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let string_table = StringTable::new();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);

    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let frontend = successful_compilation_value(compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
    ));
    let snapshot = timing_session.finish();
    drop(frontend);

    let project_boundary = snapshot
        .boundaries
        .iter()
        .find(|boundary| {
            boundary.kind == crate::timing::TimingBoundaryKind::MainProject
                && boundary.display_name == config.project_name
        })
        .expect("the synthetic single-file boundary should be registered")
        .id;
    let project_modules = snapshot
        .modules
        .iter()
        .filter(|module| {
            module.key.boundary() == project_boundary
                && module.logical_identity == config.project_name
        })
        .collect::<Vec<_>>();
    assert_eq!(
        project_modules.len(),
        1,
        "the single-file compile must register exactly one project module"
    );
    assert!(
        module_has_timing(
            project_modules[0],
            crate::timing::TimingMetric::FrontendPrepare
        ),
        "single-file preparation must record frontend.prepare for its own module"
    );
}

#[cfg(feature = "timers")]
#[test]
fn ast_aggregate_metrics_recorded_with_timers() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();

    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    let moth_path = dir.join("test.moth");
    fs::write(&moth_path, "value = 1\n").expect("should write .moth");

    let mut config = Config::new(moth_path.clone());
    config.project_name = "moth_ast_timing_fixture".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let string_table = StringTable::new();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);

    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let frontend = successful_compilation_value(compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
    ));
    let snapshot = timing_session.finish();
    drop(frontend);

    let project_boundaries = snapshot
        .boundaries
        .iter()
        .filter(|boundary| {
            boundary.kind == crate::timing::TimingBoundaryKind::MainProject
                && boundary.display_name == config.project_name
        })
        .collect::<Vec<_>>();
    assert_eq!(
        project_boundaries.len(),
        1,
        "this single-file compile must register exactly one main-project boundary"
    );
    let project_boundary = project_boundaries[0].id;
    let project_modules = snapshot
        .modules
        .iter()
        .filter(|module| {
            module.key.boundary() == project_boundary
                && module.logical_identity == config.project_name
        })
        .collect::<Vec<_>>();
    assert_eq!(
        project_modules.len(),
        1,
        "this single-file compile must register exactly one module"
    );
    let project_module = project_modules[0];

    for metric in [
        "frontend.ast.environment",
        "frontend.ast.emit",
        "frontend.ast.finalise",
    ] {
        assert!(
            project_module.timings.iter().any(|aggregate| {
                aggregate.metric.descriptor().stable_name == metric && aggregate.samples > 0
            }),
            "{metric} must be recorded for this module whenever timers is enabled"
        );
    }
    let ast_total_samples = project_module
        .timings
        .iter()
        .find(|aggregate| aggregate.metric == crate::timing::TimingMetric::FrontendAstTotal)
        .map_or(0, |aggregate| aggregate.samples);
    assert_eq!(
        ast_total_samples, 1,
        "module AST construction must record one aggregate timing span"
    );
}

#[cfg(feature = "detailed_timers")]
#[test]
fn ast_aggregate_metrics_are_not_double_recorded_with_detailed_timers() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();

    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    let moth_path = dir.join("test.moth");
    fs::write(&moth_path, "value = 1\n").expect("should write .moth");

    let mut config = Config::new(moth_path.clone());
    config.project_name = "moth_detailed_ast_timing_fixture".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let string_table = StringTable::new();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);

    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let frontend = successful_compilation_value(compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
    ));
    let snapshot = timing_session.finish();
    drop(frontend);

    let project_boundaries = snapshot
        .boundaries
        .iter()
        .filter(|boundary| {
            boundary.kind == crate::timing::TimingBoundaryKind::MainProject
                && boundary.display_name == config.project_name
        })
        .collect::<Vec<_>>();
    assert_eq!(
        project_boundaries.len(),
        1,
        "this single-file compile must register exactly one main-project boundary"
    );
    let project_boundary = project_boundaries[0].id;
    let project_modules = snapshot
        .modules
        .iter()
        .filter(|module| {
            module.key.boundary() == project_boundary
                && module.logical_identity == config.project_name
        })
        .collect::<Vec<_>>();
    assert_eq!(
        project_modules.len(),
        1,
        "this single-file compile must register exactly one module"
    );
    let project_module = project_modules[0];

    for metric in [
        "frontend.ast.environment",
        "frontend.ast.emit",
        "frontend.ast.finalise",
    ] {
        let count = project_module
            .timings
            .iter()
            .filter(|aggregate| {
                aggregate.metric.descriptor().stable_name == metric && aggregate.samples > 0
            })
            .count();
        assert!(
            count >= 1,
            "{metric} must be recorded for this module under detailed_timers"
        );
    }
    let ast_total_samples = project_module
        .timings
        .iter()
        .find(|aggregate| aggregate.metric == crate::timing::TimingMetric::FrontendAstTotal)
        .map_or(0, |aggregate| aggregate.samples);
    assert_eq!(
        ast_total_samples, 1,
        "detailed AST construction must still record one aggregate span"
    );
}

#[cfg(feature = "timers")]
#[test]
fn unguarded_parallel_compile_registers_into_an_active_timing_session() {
    let dir_a_temp = tempfile::tempdir().expect("should create temp dir");
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // Keep the shared instrumentation lock against tests that start another session, but leave
    // this test's worker deliberately unguarded. Unique project names let the assertions select
    // both failed fixtures while unrelated compiles register into the same process-global session.
    let dir_a = dir_a_temp.path().to_path_buf();
    let project_a_name = "unguarded_parallel_fixture_a";
    let project_b_name = "unguarded_parallel_fixture_b";
    let unrelated_project_name = "unrelated_active_timing_fixture";
    fs::write(
        dir_a.join("config.moth"),
        format!("project #= (\n    name = \"{project_a_name}\",\n)\nhtml #= ()\n"),
    )
    .expect("should write config");
    fs::write(dir_a.join("@page.moth"), "@core/math sin,\n#[:ok]\n")
        .expect("should write malformed entry");
    let dir_b_temp = tempfile::tempdir().expect("should create temp dir");
    let dir_b = dir_b_temp.path().to_path_buf();
    fs::create_dir_all(dir_b.join("src")).expect("should create source root");
    fs::write(
        dir_b.join("config.moth"),
        format!(
            "project #= (\n    name = \"{project_b_name}\",\n    entry_root = \"src\",\n)\nhtml #= ()\n"
        ),
    )
    .expect("should write config");
    fs::write(dir_b.join("src/@page.moth"), [b'o', b'k', 0xff, b'\n'])
        .expect("should write invalid UTF-8 source");
    let dir_unrelated_temp = tempfile::tempdir().expect("should create unrelated temp project");
    let dir_unrelated = dir_unrelated_temp.path().to_path_buf();
    fs::create_dir_all(dir_unrelated.join("src")).expect("should create unrelated source root");
    fs::write(
        dir_unrelated.join("config.moth"),
        format!(
            "project #= (\n    name = \"{unrelated_project_name}\",\n    entry_root = \"src\",\n)\nhtml #= ()\n"
        ),
    )
    .expect("should write unrelated config");
    fs::write(
        dir_unrelated.join("src/@page.moth"),
        [b'o', b'k', 0xff, b'\n'],
    )
    .expect("should write unrelated invalid UTF-8 source");
    let mut config_a = Config::new(dir_a.clone());
    config_a.project_name = project_a_name.to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let string_table = StringTable::new();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);
    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let result_a = compile_project_frontend(
        &mut config_a,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
    );
    let reports_a = diagnosed_reports(&result_a);
    assert_has_diagnostic_code(&result_a, "MOTH-SYNTAX-0019");
    assert_eq!(
        reports_a.counts().errors,
        1,
        "first fixture should fail with exactly the syntax diagnostic before the unguarded worker starts"
    );

    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let thread_string_table = StringTable::new();
                let mut thread_warning_owner =
                    BoundaryAccumulator::with_root_string_table(thread_string_table);
                let thread_style = StyleDirectiveRegistry::built_ins();
                let mut thread_config = Config::new(dir_b.clone());
                thread_config.project_name = project_b_name.to_owned();
                let mut thread_surface = BuilderSurface::with_mandatory_core();
                let thread_result = compile_project_frontend(
                    &mut thread_config,
                    BuildProfile::Dev,
                    None,
                    &thread_style,
                    &mut thread_surface,
                    &mut thread_warning_owner,
                );

                // The concurrent fixture fails in its source-loading lane: invalid UTF-8 is a
                // source diagnostic naming the file, not an infrastructure read failure.
                assert_has_diagnostic_code(&thread_result, "MOTH-SYNTAX-0040");
                assert_eq!(
                    diagnosed_reports(&thread_result).counts().errors,
                    1,
                    "second fixture should fail with exactly the invalid UTF-8 diagnostic"
                );
            })
            .join()
            .expect("concurrent compile worker should not panic");
    });
    let mut unrelated_config = Config::new(dir_unrelated.clone());
    unrelated_config.project_name = unrelated_project_name.to_owned();
    let mut unrelated_warning_owner =
        BoundaryAccumulator::with_root_string_table(StringTable::new());
    let unrelated_result = compile_project_frontend(
        &mut unrelated_config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut unrelated_warning_owner,
    );
    assert_has_diagnostic_code(&unrelated_result, "MOTH-SYNTAX-0040");
    assert_eq!(
        diagnosed_reports(&unrelated_result).counts().errors,
        1,
        "the unrelated compile must register an independent failure during the active session"
    );
    let snapshot = timing_session.finish();
    let project_a_boundaries = snapshot
        .boundaries
        .iter()
        .filter(|boundary| {
            boundary.kind == crate::timing::TimingBoundaryKind::MainProject
                && boundary.display_name == project_a_name
        })
        .collect::<Vec<_>>();
    assert_eq!(
        project_a_boundaries.len(),
        1,
        "the first fixture must register its own main-project boundary"
    );
    let project_a_boundary = project_a_boundaries[0].id;
    let project_b_boundaries = snapshot
        .boundaries
        .iter()
        .filter(|boundary| {
            boundary.kind == crate::timing::TimingBoundaryKind::MainProject
                && boundary.display_name == project_b_name
        })
        .collect::<Vec<_>>();
    assert_eq!(
        project_b_boundaries.len(),
        1,
        "the unguarded worker must register its own main-project boundary"
    );
    let project_b_boundary = project_b_boundaries[0].id;
    let unrelated_boundaries = snapshot
        .boundaries
        .iter()
        .filter(|boundary| {
            boundary.kind == crate::timing::TimingBoundaryKind::MainProject
                && boundary.display_name == unrelated_project_name
        })
        .collect::<Vec<_>>();
    assert_eq!(
        unrelated_boundaries.len(),
        1,
        "the extra unrelated compile must register during this active session"
    );
    let unrelated_boundary = unrelated_boundaries[0].id;

    let fixture_boundaries = [project_a_boundary, project_b_boundary];
    let unfinished_fixture_modules = snapshot
        .modules
        .iter()
        .filter(|module| {
            !module.source_facts_finalized && fixture_boundaries.contains(&module.key.boundary())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        unfinished_fixture_modules.len(),
        2,
        "both failed fixtures must leave their own unfinished modules"
    );
    let project_b_identity = format!("{project_b_name}/src");
    let unrelated_identity = format!("{unrelated_project_name}/src");
    for (boundary, logical_identity) in [
        (project_a_boundary, project_a_name),
        (project_b_boundary, project_b_identity.as_str()),
        (unrelated_boundary, unrelated_identity.as_str()),
    ] {
        let modules = snapshot
            .modules
            .iter()
            .filter(|module| {
                module.key.boundary() == boundary && module.logical_identity == logical_identity
            })
            .collect::<Vec<_>>();
        assert_eq!(
            modules.len(),
            1,
            "each fixture should register its expected module exactly once: {boundary:?} {logical_identity}"
        );
        assert!(
            !modules[0].source_facts_finalized,
            "failed preparation should leave {logical_identity} unfinished"
        );
    }
}

/// A dependency module root's dormant runtime work is validated by that module's own
/// canonical compilation. Each case leaves a grouped delimiter open after the root's public
/// declaration, and the HTML-selected entry consumes that public value without activating it.
#[test]
fn dormant_dependency_root_grouped_delimiters_report_eof_in_child_source() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("src/components")).expect("should create dependency module root");
    fs::write(
        dir.join("config.moth"),
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    // The entry is HTML-selected by its authored root fragment and imports the public value.
    fs::write(
        dir.join("src/@page.moth"),
        "@components label\n[:[label]]\n",
    )
    .expect("should write entry page");

    for (case, dormant_work, expected_delimiter) in [
        ("collection", "values = {\n    1,\n    2,\n", "}"),
        ("grouping", "value = (\n    1 + 2\n", ")"),
    ] {
        let dependency_source = format!("export:\n    label #= \"components\"\n;\n{dormant_work}");
        fs::write(
            dir.join("src/components/@components.moth"),
            &dependency_source,
        )
        .expect("should write dependency module root");

        let mut config = Config::new(dir.clone());
        let style_directives = StyleDirectiveRegistry::built_ins();
        let string_table = StringTable::new();
        let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);
        let result = compile_project_frontend(
            &mut config,
            BuildProfile::Dev,
            None,
            &style_directives,
            &mut BuilderSurface::with_mandatory_core(),
            &mut warning_owner,
        );
        let reports = diagnosed_reports(&result);
        assert_eq!(
            reports.counts().errors,
            1,
            "{case}: the dependency root should diagnose once and the blocked page must not cascade"
        );
        let (report, view) = reports
            .iter_severity(DiagnosticSeverity::Error)
            .find(|(_, view)| view.descriptor().code == "MOTH-SYNTAX-0017")
            .expect("the dormant group must retain its unexpected-EOF view");
        assert_eq!(view.severity(), DiagnosticSeverity::Error, "{case}");
        let rendered =
            terse::format_terse_diagnostic_view(view, DiagnosticRenderContext::for_report(report));
        assert!(
            rendered.contains(&format!("expected '{expected_delimiter}'")),
            "{case}: the retained EOF view must render the exact missing-delimiter fact: {rendered}"
        );
        let span = match view.primary_site() {
            crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Local(
                span,
            )
            | crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Foreign {
                span,
                ..
            } => span,
            crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Absent => {
                panic!("{case}: unexpected EOF must retain its authored source span")
            }
        };
        let context = DiagnosticRenderContext::for_report(report);
        let identity = report.context();

        let dependency_path = fs::canonicalize(dir.join("src/components/@components.moth"))
            .expect("dependency source path should canonicalize");
        let source_slot = identity
            .get(span.source())
            .expect("the EOF span must resolve through the dependency's own source identity");
        assert_eq!(
            source_slot.canonical_os_path.as_deref(),
            Some(dependency_path.as_path()),
            "{case}: the missing delimiter must belong to the dependency source"
        );
        let position = context
            .site_position(view.primary_site())
            .expect("the EOF site should resolve through its dependency report");
        assert_eq!(position.host_path, Some(dependency_path.as_path()));
        let range = span.byte_range(identity);
        assert_eq!(
            (range.start(), range.end()),
            (
                dependency_source.len() as u32,
                dependency_source.len() as u32
            ),
            "{case}: the missing delimiter must point at the dependency source's exact EOF"
        );
    }
}
