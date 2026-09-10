use super::*;
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
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
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
    let mut frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    )
    .expect("diagnosed modules are retained in the typed frontend outcome");
    let project_source = frontend.project_source_database.take();
    let messages = frontend
        .into_render_messages_with_frozen_identity(&mut string_table, project_source, None)
        .expect("frontend diagnostics should install frozen render identity");

    assert_eq!(
        messages.error_count(),
        2,
        "the provider and independent branch should each diagnose once; blocked consumers should emit no cascades"
    );
    let diagnosed_paths = messages
        .diagnostics()
        .enumerate()
        .filter(|(_, diagnostic)| {
            diagnostic.severity
                == crate::compiler_frontend::compiler_messages::DiagnosticSeverity::Error
        })
        .filter_map(|(diagnostic_index, diagnostic)| {
            Some(
                messages
                    .diagnostic_render_context(diagnostic_index)
                    .primary_position(diagnostic)?
                    .path,
            )
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
    let _temp = tempfile::tempdir().expect("should create temporary project");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("src/about")).expect("should create nested module");
    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"snapshot\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    let root_source = "title String = \"home\"\n";
    let nested_source = "title String = \"about\"\n";
    fs::write(dir.join("src/@page.moth"), root_source).expect("should write root source");
    fs::write(dir.join("src/about/@page.moth"), nested_source).expect("should write nested source");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut builder_surface = BuilderSurface::with_mandatory_core();
    let mut project_source_files = None;
    let build_config_inputs = BuildConfigInputSet::new();
    compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut builder_surface,
        &mut string_table,
        &mut project_source_files,
        &build_config_inputs,
        FrontendCompilationMode::Canonical,
    )
    .expect("multiple source modules should compile");

    let source_files =
        project_source_files.expect("compiled project should retain source database");
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
fn selected_preload_read_failure_stays_in_the_existing_file_error_lane() {
    let _temp = tempfile::tempdir().expect("should create temporary project");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("src")).expect("should create source root");
    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"selected_failure\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    let selected_path = dir.join("src/@page.moth");
    fs::write(&selected_path, [b'o', b'k', 0xff, b'\n'])
        .expect("should write invalid UTF-8 source");

    let mut config = Config::new(dir);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut builder_surface = BuilderSurface::with_mandatory_core();
    let error = match compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut builder_surface,
        &mut string_table,
    ) {
        Ok(_) => panic!("selected unreadable source should fail its existing preparation lane"),
        Err(error) => error,
    };
    let error_value = error
        .infrastructure_error()
        .expect("selected preload failure should remain an infrastructure file error");
    assert_eq!(&error_value.error_type, &ErrorType::File);
    assert!(
        error_value
            .msg
            .contains("Error reading file when adding new moth files to parse"),
        "selected source should preserve the existing read failure message: {}",
        error_value.msg,
    );
    let canonical_selected_path =
        fs::canonicalize(selected_path).expect("selected source path should canonicalize");
    assert_eq!(
        error_value.host_path.as_deref(),
        Some(canonical_selected_path.as_path()),
    );
}

#[test]
fn unselected_preload_read_failure_stays_inert() {
    let _temp = tempfile::tempdir().expect("should create temporary project");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("src")).expect("should create source root");
    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"unselected_failure\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(dir.join("src/@page.moth"), "title String = \"home\"\n")
        .expect("should write selected source");
    let unselected_path = dir.join("src/unreachable.moth");
    fs::write(&unselected_path, [b'o', b'k', 0xff, b'\n'])
        .expect("should write invalid UTF-8 source");

    let mut config = Config::new(dir);
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut builder_surface = BuilderSurface::with_mandatory_core();
    let mut project_source_files = None;
    let build_config_inputs = BuildConfigInputSet::new();
    let frontend = compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut builder_surface,
        &mut string_table,
        &mut project_source_files,
        &build_config_inputs,
        FrontendCompilationMode::Canonical,
    )
    .expect("unselected unreadable source should not abort the build");

    // `Ok` alone proves nothing: a diagnosed outcome still returns `Ok` and carries its
    // diagnostics in the payload, so an invented failure for the unselected file would pass an
    // expect-only assertion. Inert means the build reports nothing at all about it.
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

    let source_files =
        project_source_files.expect("successful build should retain source database");
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
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(dir.join("src/@page.moth"), "value = 1\n").expect("should write project entry");
    // Keep the dependency unused so declaration-time validation cannot rely on binding or
    // reachability analysis to reject the facade's self-dependency.
    fs::write(dir.join("+package.moth"), "@project\n").expect("should write project facade");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    )
    .expect("a facade dependency diagnostic should remain a retained frontend outcome");
    let project_source = frontend.project_source_database.take();
    let messages = frontend
        .into_render_messages_with_frozen_identity(&mut string_table, project_source, None)
        .expect("facade diagnostics should install frozen render identity");

    let matching = messages
        .diagnostics()
        .enumerate()
        .filter(|(_, diagnostic)| {
            matches!(
                &diagnostic.payload,
                DiagnosticPayload::InvalidDependencyClause {
                    reason: InvalidDependencyClauseReason::ProjectGlobalsFacadeDependencyNotAllowed,
                    ..
                }
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        matching.len(),
        1,
        "the facade's own @project declaration should produce one structured diagnostic"
    );
    let (diagnostic_index, diagnostic) = matching[0];
    assert_eq!(diagnostic.kind.code(), "MOTH-SYNTAX-0019");
    let position = messages
        .diagnostic_render_context(diagnostic_index)
        .primary_position(diagnostic)
        .expect("facade diagnostic should retain its authored source position");
    assert!(
        position.path.ends_with("+package.moth"),
        "self-dependency diagnostic should point to the facade source"
    );
}

#[cfg(feature = "timers")]
#[test]
fn failed_directory_preparation_keeps_unfinished_module_metadata_out_of_completion() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(dir.join("@page.moth"), "@core/math sin,\n#[:ok]\n")
        .expect("should write malformed entry");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");

    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    );
    let Err(messages) = result else {
        panic!("malformed Stage 0 input should fail preparation");
    };
    assert_has_diagnostic_code(&messages, "MOTH-SYNTAX-0019");
    assert_eq!(
        messages.error_count(),
        1,
        "malformed Stage 0 input should produce exactly the syntax diagnostic"
    );

    let snapshot = timing_session.finish();
    let unfinished_modules = snapshot
        .modules
        .iter()
        .filter(|module| !module.source_facts_finalized)
        .collect::<Vec<_>>();
    assert_eq!(unfinished_modules.len(), 1);
    let unfinished = unfinished_modules[0];
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
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(dir.join("@page.moth"), "value = 1\n").expect("should write project root");
    fs::write(
        package_root.join("@mod.moth"),
        "export:\n    helper || -> Int:\n        return 7\n    ;\n;\n",
    )
    .expect("should write package module");

    let mut config = Config::new(dir.clone());
    config.project_name = PROJECT_NAME.to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        PACKAGE_NAME,
        package_root.clone(),
        PackageOrigin::Builder,
    );

    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("a clean source package and project should compile");
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
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(dir.join("@page.moth"), "value = 1\n").expect("should write project root");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();

    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    )
    .expect("a clean directory project should compile");
    let snapshot = timing_session.finish();
    drop(frontend);

    let project_boundary = snapshot
        .boundaries
        .iter()
        .find(|boundary| boundary.display_name == config.project_name)
        .expect("the main project boundary should be registered")
        .id;
    let prepare_modules = snapshot
        .modules
        .iter()
        .filter(|module| {
            module.key.boundary() == project_boundary
                && module_has_timing(module, crate::timing::TimingMetric::FrontendPrepare)
        })
        .collect::<Vec<_>>();

    assert!(
        !prepare_modules.is_empty(),
        "incremental directory discovery must record frontend.prepare for the project boundary"
    );
    assert!(
        prepare_modules
            .iter()
            .all(|module| module.key.boundary() == project_boundary),
        "every project-boundary preparation observation must carry the owning module"
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
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();

    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    )
    .expect("a clean single-file project should compile");
    let snapshot = timing_session.finish();
    drop(frontend);

    let project_boundary = snapshot
        .boundaries
        .iter()
        .find(|boundary| boundary.display_name == config.project_name)
        .expect("the synthetic single-file boundary should be registered")
        .id;
    let prepare_modules = snapshot
        .modules
        .iter()
        .filter(|module| {
            module.key.boundary() == project_boundary
                && module_has_timing(module, crate::timing::TimingMetric::FrontendPrepare)
        })
        .collect::<Vec<_>>();
    assert!(
        !prepare_modules.is_empty(),
        "single-file preparation must record frontend.prepare: boundaries={:#?} modules={:#?}",
        snapshot
            .boundaries
            .iter()
            .map(|boundary| (boundary.id, boundary.display_name.as_str()))
            .collect::<Vec<_>>(),
        snapshot
            .modules
            .iter()
            .filter(|module| module_has_timing(
                module,
                crate::timing::TimingMetric::FrontendPrepare
            ))
            .map(|module| module.key)
            .collect::<Vec<_>>()
    );
    assert!(
        prepare_modules
            .iter()
            .all(|module| module.key.boundary() == project_boundary),
        "single-file preparation must carry the synthetic module key"
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
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();

    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    )
    .expect("a clean single-file project should compile");
    let snapshot = timing_session.finish();
    drop(frontend);

    for metric in [
        "frontend.ast.environment",
        "frontend.ast.emit",
        "frontend.ast.finalise",
    ] {
        assert!(
            snapshot.timings.iter().any(|aggregate| {
                aggregate.metric.descriptor().stable_name == metric && aggregate.samples > 0
            }),
            "{metric} must be recorded whenever timers is enabled"
        );
    }
    let ast_total_count = snapshot
        .timings
        .iter()
        .filter(|aggregate| {
            aggregate.metric.descriptor().stable_name == "frontend.ast.total"
                && aggregate.samples > 0
        })
        .count();
    assert_eq!(
        ast_total_count, 1,
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
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();

    let timing_session =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    )
    .expect("a clean single-file project should compile");
    let snapshot = timing_session.finish();
    drop(frontend);

    for metric in [
        "frontend.ast.environment",
        "frontend.ast.emit",
        "frontend.ast.finalise",
    ] {
        let count = snapshot
            .timings
            .iter()
            .filter(|aggregate| {
                aggregate.metric.descriptor().stable_name == metric && aggregate.samples > 0
            })
            .count();
        assert!(
            count >= 1,
            "{metric} must be recorded under detailed_timers"
        );
    }
    let ast_total_count = snapshot
        .timings
        .iter()
        .filter(|aggregate| {
            aggregate.metric.descriptor().stable_name == "frontend.ast.total"
                && aggregate.samples > 0
        })
        .count();
    assert_eq!(
        ast_total_count, 1,
        "detailed AST construction must still record one aggregate span"
    );
}
