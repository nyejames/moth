use super::*;
#[test]
fn synthetic_rebinding_makes_file_and_shell_identities_discovery_order_independent() {
    let forward = synthetic_identity_fixture(&["alpha", "beta"]);
    let reversed = synthetic_identity_fixture(&["beta", "alpha"]);

    assert_eq!(forward, reversed);
    assert_eq!(
        forward
            .iter()
            .map(|file| file.logical_path.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha.moth", "beta.moth", "main.moth"],
        "final source identities must use deterministic logical paths"
    );
    assert_eq!(
        forward.iter().map(|file| file.file_id).collect::<Vec<_>>(),
        vec![
            SourceId::from_index(1),
            SourceId::from_index(2),
            SourceId::from_index(3),
        ],
        "final SourceIds must come from the sorted complete closure"
    );
    assert_eq!(
        forward[2].shell_ids,
        vec![
            DependencyShellId::new(SourceId::from_index(3), 0),
            DependencyShellId::new(SourceId::from_index(3), 1)
        ]
    );
    assert_eq!(forward[2].selected_source_names, vec!["greet", "greet"]);
}

#[cfg(unix)]
#[test]
fn synthetic_discovery_keeps_authored_kind_when_canonical_extension_disagrees() {
    use std::os::unix::fs::symlink;

    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let payload = root.join("payload.bin");
    let entry = root.join("main.moth");
    fs::write(&payload, "#[:ok]\n").expect("should write unrecognized-extension payload");
    symlink(&payload, &entry).expect("should symlink recognized source name onto payload");

    let config = Config::new(root.clone());
    let resolver = configured_resolver(&config);
    let style_directives = test_style_directives();
    let (source_files, input_files, _string_table) =
        collect_synthetic_inputs_for_test(&entry, &resolver, &style_directives);

    let canonical = fs::canonicalize(&entry).expect("symlinked entry should canonicalize");
    assert_eq!(
        canonical
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("bin"),
        "canonical target must keep the unrecognized extension"
    );
    let record = source_files
        .sources()
        .get_by_canonical_path(&canonical)
        .expect("discovered source should retain its identity");
    assert_eq!(
        record.kind,
        Some(SourceKind::Compiler(SourceFileKind::Moth)),
        "authored .moth kind must survive a canonical .bin target"
    );
    assert!(
        input_files
            .iter()
            .any(|input| matches!(input.source, PreparedSourceKind::MothPrepared { .. })),
        "discovery must prepare the authored Moth source rather than reject it as provider-owned"
    );
}

#[cfg(unix)]
#[test]
fn indexed_discovery_keeps_authored_kind_when_canonical_extension_disagrees() {
    use std::os::unix::fs::symlink;

    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    fs::write(source_root.join("main.moth"), "value #= 1\n").expect("should write entry");
    let payload = source_root.join("payload.bin");
    fs::write(&payload, "template body\n").expect("should write unrecognized-extension payload");
    symlink(&payload, source_root.join("page.mtf"))
        .expect("should symlink a template name onto the payload");

    let config = Config::new(root.clone());
    let mut source_file_kinds = crate::builder_surface::SourceFileKindRegistry::new();
    source_file_kinds.register("mtf", SourceFileKind::MothTemplate);
    let resolver = configured_resolver_with_source_file_kinds(&config, &source_file_kinds);
    let mut string_table = StringTable::new();
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root =
        fs::canonicalize(resolve_project_entry_root(&config)).expect("entry root should resolve");
    let source_tree_index = super::source_tree_index::SourceTreeIndex::discover(
        entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &project_root,
            validated_output_settings: None,
        },
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &source_file_kinds,
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("source tree index should discover the symlinked template");

    let registration_index = source_tree_index.source_registration_index();
    let source_files = SourceDatabase::from_ordered_registration_index(
        &registration_index,
        resolver.entry_root(),
        Some(&resolver),
        &mut string_table,
    )
    .expect("indexed inventory should register");

    let canonical = fs::canonicalize(source_root.join("page.mtf"))
        .expect("symlinked template should canonicalize");
    assert_eq!(
        canonical
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("bin"),
        "canonical target must keep the unrecognized extension"
    );
    assert_eq!(
        source_files
            .get_by_canonical_path(&canonical)
            .and_then(|record| record.kind),
        Some(SourceKind::Compiler(SourceFileKind::MothTemplate)),
        "Stage 0's authored .mtf classification must reach the record"
    );
}

#[test]
fn stage0_source_ids_keep_module_origin_order_when_flat_portable_path_disagrees() {
    // WHY: a flat portable key sorts `a-b/@b.moth` before `a/@a.moth` because '-' precedes '/'.
    // Assigned SourceIds must follow module-origin order instead: module `a` before module `a-b`.
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();
    let source_root = root.join("src");
    fs::create_dir_all(source_root.join("a")).expect("should create module a");
    fs::create_dir_all(source_root.join("a-b")).expect("should create module a-b");
    fs::write(source_root.join("a/@a.moth"), "value #= 1\n").expect("should write module a root");
    fs::write(source_root.join("a-b/@b.moth"), "value #= 1\n")
        .expect("should write module a-b root");

    let mut config = Config::new(root.clone());
    config.entry_root = PathBuf::from("src");
    let resolver = configured_resolver(&config);
    let mut string_table = StringTable::new();
    let project_root = fs::canonicalize(&config.entry_dir).expect("project root should resolve");
    let entry_root =
        fs::canonicalize(resolve_project_entry_root(&config)).expect("entry root should resolve");
    let source_tree_index = super::source_tree_index::SourceTreeIndex::discover(
        entry_root,
        super::source_tree_index::SourceTreeProjectContext {
            project_root: &project_root,
            validated_output_settings: None,
        },
        &config,
        &crate::builder_surface::SourcePackageRegistry::default(),
        &crate::builder_surface::SourceFileKindRegistry::default(),
        &crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::default(),
        &mut string_table,
    )
    .expect("source tree index should discover sibling modules");

    let registration_index = source_tree_index.source_registration_index();
    let source_files = SourceDatabase::from_ordered_registration_index(
        &registration_index,
        resolver.entry_root(),
        Some(&resolver),
        &mut string_table,
    )
    .expect("indexed inventory should register");

    let path_table = source_files.paths();
    let mut path_scratch = Vec::new();
    let assigned_logical_paths = source_files
        .iter()
        .map(|record| {
            path_table.render_portable(record.logical_path, &string_table, &mut path_scratch)
        })
        .collect::<Vec<_>>();
    let mut flat_portable_order = assigned_logical_paths.clone();
    flat_portable_order.sort();

    assert_eq!(
        assigned_logical_paths,
        vec!["a/@a.moth", "a-b/@b.moth"],
        "indexed registration must assign SourceIds in Stage 0 module-origin order"
    );
    assert_ne!(
        assigned_logical_paths, flat_portable_order,
        "fixture must disagree with flat portable-key order"
    );
}

#[test]
fn synthetic_preparation_reuses_complete_outputs_for_one_final_header_pass() {
    let _test_guard = lock_source_read_counter_tests();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(root.join("main.moth"), "@helper greet\n").expect("should write entry");
    fs::write(
        root.join("helper.moth"),
        "greet||:\n    io.line([: [\"hello\"]])\n;\n",
    )
    .expect("should write helper");

    let entry_file_path =
        fs::canonicalize(root.join("main.moth")).expect("entry should canonicalize");
    let helper_file_path =
        fs::canonicalize(root.join("helper.moth")).expect("helper should canonicalize");
    let canonical_root = fs::canonicalize(&root).expect("fixture root should canonicalize");
    super::source_loading_test_support::reset_source_read_count_for_test(&canonical_root);
    crate::compiler_frontend::pipeline_test_support::reset_file_frontend_prepare_count_for_test(
        &canonical_root,
    );
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let _counter_capture =
        crate::compiler_frontend::instrumentation::capture_frontend_counters_for_test();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    crate::compiler_frontend::instrumentation::reset_frontend_counters();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let counter_guard =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");

    let config = Config::new(root.clone());
    let resolver = configured_resolver(&config);
    let style_directives = test_style_directives();
    let (mut span_owners, input_files, string_table) =
        collect_synthetic_inputs_for_test(&entry_file_path, &resolver, &style_directives);
    let (source_files, mut span_view) = span_owners.split();
    let local_string_table = string_table.fork_source().fork_for_module().into_parts().0;

    assert!(
        input_files
            .iter()
            .all(|input| matches!(input.source, PreparedSourceKind::MothPrepared { .. })),
        "every synthetic Moth source must carry its complete first preparation"
    );
    assert_eq!(
        super::source_loading_test_support::source_read_count_for_path_for_test(&entry_file_path),
        1,
        "the entry source must be read once"
    );
    assert_eq!(
        super::source_loading_test_support::source_read_count_for_path_for_test(&helper_file_path),
        1,
        "the imported source must be read once"
    );
    assert_eq!(
        crate::compiler_frontend::pipeline_test_support::file_frontend_prepare_count_for_path_for_test(&entry_file_path),
        1,
        "the entry source must receive one first preparation"
    );
    assert_eq!(
        crate::compiler_frontend::pipeline_test_support::file_frontend_prepare_count_for_path_for_test(&helper_file_path),
        1,
        "the imported source must receive one first preparation"
    );

    let source_byte_count = input_files
        .iter()
        .map(|input| {
            source_files
                .retained_text(input.source_id())
                .expect("synthetic source should retain its snapshot")
                .len()
        })
        .sum();
    let preparation_context = super::module_preparation::ModulePreparationContext {
        source_files,
        style_directives: &style_directives,
        project_path_resolver: Some(resolver),
    };
    let stable_origin = StableModuleOriginIdentity::from_relative_logical_path(
        StablePackageIdentity::project_local("synthetic-exactly-once"),
        Path::new(""),
        ModuleRootRole::Normal,
    )
    .expect("synthetic origin should construct");

    #[cfg(feature = "timers")]
    let prepared = preparation_context
        .prepare_module(
            stable_origin,
            input_files,
            &mut span_view,
            &entry_file_path,
            local_string_table,
            source_byte_count,
            None,
        )
        .expect("retained synthetic outputs should prepare once");
    #[cfg(not(feature = "timers"))]
    let prepared = preparation_context
        .prepare_module(
            stable_origin,
            input_files,
            &mut span_view,
            &entry_file_path,
            local_string_table,
            source_byte_count,
        )
        .expect("retained synthetic outputs should prepare once");
    assert_eq!(
        prepared
            .semantic
            .prepared_header_syntax
            .module_symbols
            .module_file_paths
            .len(),
        2,
        "one final header aggregation must retain both prepared source identities"
    );
    assert_eq!(
        super::source_loading_test_support::source_read_count_for_path_for_test(&entry_file_path),
        1,
        "final aggregation must not reread the entry source"
    );
    assert_eq!(
        super::source_loading_test_support::source_read_count_for_path_for_test(&helper_file_path),
        1,
        "final aggregation must not reread the imported source"
    );
    assert_eq!(
        crate::compiler_frontend::pipeline_test_support::file_frontend_prepare_count_for_path_for_test(&entry_file_path),
        1,
        "final aggregation must not prepare the entry source again"
    );
    assert_eq!(
        crate::compiler_frontend::pipeline_test_support::file_frontend_prepare_count_for_path_for_test(&helper_file_path),
        1,
        "final aggregation must not prepare the imported source again"
    );

    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    crate::compiler_frontend::instrumentation::log_frontend_counters();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let observations = counter_guard.finish();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let counter_value = |name: &str| {
        observations
            .counters
            .iter()
            .find(|counter| counter.name == name)
            .map(|counter| counter.value)
            .unwrap_or(-1.0)
    };
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    assert_eq!(counter_value("file_preparation_pass_count"), 2.0);
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    assert_eq!(counter_value("prepared_file_count"), 2.0);
}

#[test]
fn synthetic_diagnosed_preparation_is_not_consumed_again() {
    let _test_guard = lock_source_read_counter_tests();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let root = _temp.path().to_path_buf();

    fs::write(root.join("main.moth"), "@helper\n").expect("should write entry");
    fs::write(
        root.join("helper.moth"),
        "-- é🦋\n@core/math sin,\nvalue = 1\n",
    )
    .expect("should write malformed helper");

    let config = Config::new(root.clone());
    let resolver = configured_resolver(&config);
    let style_directives = test_style_directives();
    let entry_file_path =
        fs::canonicalize(root.join("main.moth")).expect("entry should canonicalize");
    let helper_file_path =
        fs::canonicalize(root.join("helper.moth")).expect("helper should canonicalize");
    let canonical_root = fs::canonicalize(&root).expect("fixture root should canonicalize");
    super::source_loading_test_support::reset_source_read_count_for_test(&canonical_root);
    crate::compiler_frontend::pipeline_test_support::reset_file_frontend_prepare_count_for_test(
        &canonical_root,
    );
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let _counter_capture =
        crate::compiler_frontend::instrumentation::capture_frontend_counters_for_test();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    crate::compiler_frontend::instrumentation::reset_frontend_counters();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let counter_guard =
        crate::timing::start_benchmark_collection(true).expect("timing session should start");
    let mut string_table = StringTable::new();
    let mut external_packages = ExternalPackageRegistry::new();
    let external_import_providers = crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry::empty();
    let mut external_import_cache =
        crate::builder_surface::external_import_providers::cache::ExternalImportProviderCache::new(
        );
    let mut external_dependency_resolution_table =
        crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable::new();
    let mut external_imports = super::source_discovery::ExternalImportDiscoveryState {
        external_packages: &mut external_packages,
        providers: &external_import_providers,
        cache: &mut external_import_cache,
        resolution_table: &mut external_dependency_resolution_table,
    };
    let source_file_kinds = crate::builder_surface::SourceFileKindRegistry::default();
    let mut resource_inputs = ResourceInputRegistry::new();

    let (failure, source_database) = match super::source_discovery::collect_reachable_input_files(
        &root.join("main.moth"),
        &resolver,
        &style_directives,
        &mut external_imports,
        &source_file_kinds,
        &mut resource_inputs,
        &mut string_table,
    ) {
        Ok(_) => panic!("malformed synthetic preparation should diagnose"),
        Err(error) => error.into_parts(),
    };
    let source_files =
        source_database.expect("diagnosed discovery must publish its finalized source context");
    let messages = failure.into_messages_with_source(&string_table, Arc::new(source_files));
    let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
    assert_eq!(
        diagnostics.len(),
        1,
        "a diagnosed synthetic preparation must not be consumed by a second preparation pass"
    );
    assert!(matches!(
        diagnostics[0].payload,
        DiagnosticPayload::InvalidDependencyClause {
            reason: InvalidDependencyClauseReason::ContinuationEnteredStatement,
            ..
        }
    ));

    let source_files = messages
        .source_database_for_diagnostic(0)
        .expect("diagnosed discovery must publish its finalized source context");
    let entry_id = source_files
        .get_by_canonical_path(&entry_file_path)
        .expect("the previously prepared entry must remain in the diagnosed context")
        .id;
    let helper_id = source_files
        .get_by_canonical_path(&helper_file_path)
        .expect("the failed source must remain in the diagnosed context")
        .id;
    assert_eq!(helper_id, SourceId::from_index(1));
    assert_eq!(entry_id, SourceId::from_index(2));
    let diagnostic_span = diagnostics[0]
        .primary_span
        .expect("preparation diagnosis must retain its exact span");
    assert_eq!(diagnostic_span.source(), helper_id);
    let diagnostic_range = diagnostic_span.byte_range(source_files);
    let helper_text = source_files.retained_text(helper_id).unwrap();
    assert_eq!(
        &helper_text[diagnostic_range.start() as usize..diagnostic_range.end() as usize],
        "value"
    );
    assert_eq!(diagnostics[0].labels.len(), 1);
    let comma_label = &diagnostics[0].labels[0];
    let comma_span = comma_label
        .span
        .expect("discovery retains every preparation label span");
    assert_eq!(
        comma_span.source(),
        helper_id,
        "related spans must use the finalized source identity"
    );
    let comma_range = comma_span.byte_range(source_files);
    assert_eq!(
        &helper_text[comma_range.start() as usize..comma_range.end() as usize],
        ","
    );
    assert_eq!(source_files.retained_text(entry_id), Some("@helper\n"));
    assert_eq!(
        source_files.retained_text(helper_id),
        Some("-- é🦋\n@core/math sin,\nvalue = 1\n")
    );
    assert_eq!(
        super::source_loading_test_support::source_read_count_for_path_for_test(&entry_file_path),
        1,
        "a diagnosed entry source must be read once"
    );
    assert_eq!(
        super::source_loading_test_support::source_read_count_for_path_for_test(&helper_file_path),
        1,
        "a diagnosed imported source must be read once"
    );
    assert_eq!(
        crate::compiler_frontend::pipeline_test_support::file_frontend_prepare_count_for_path_for_test(&entry_file_path),
        1,
        "a diagnosed entry source must be prepared once"
    );
    assert_eq!(
        crate::compiler_frontend::pipeline_test_support::file_frontend_prepare_count_for_path_for_test(&helper_file_path),
        1,
        "a diagnosed imported source must be prepared once"
    );

    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    crate::compiler_frontend::instrumentation::log_frontend_counters();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let observations = counter_guard.finish();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let counter_value = |name: &str| {
        observations
            .counters
            .iter()
            .find(|counter| counter.name == name)
            .map(|counter| counter.value)
            .unwrap_or(-1.0)
    };
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    assert_eq!(counter_value("file_preparation_pass_count"), 2.0);
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    assert_eq!(
        counter_value("prepared_file_count"),
        0.0,
        "diagnosed preparation attempts must not be counted as successful retained outputs"
    );
}
