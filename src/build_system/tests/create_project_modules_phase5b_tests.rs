use super::*;
// -------------------------
//  Phase 5b: graph-resolved local provider edges
// -------------------------

#[test]
fn local_dependency_edge_is_recorded_provider_before_consumer() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let (config, resolver, style_directives, module_a_root, module_b_root) =
        write_cross_module_project(&root);

    let (modules, graph, source_tree_index, _source_files, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    let module_a_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&module_a_root)
        .expect("module_a root should be a graph node");
    let module_b_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&module_b_root)
        .expect("module_b root should be a graph node");

    // The dependency flows module_a -> module_b, so the provider (module_b) must precede the
    // consumer (module_a) in the returned inventory order. The edge is provider-before-consumer,
    // never the reverse.
    assert!(
        graph.has_dependency_edge(module_b_id, module_a_id),
        "provider module_b must have an edge into consumer module_a"
    );
    assert!(
        !graph.has_dependency_edge(module_a_id, module_b_id),
        "the consumer must not edge into its provider"
    );

    // The returned modules follow the dependency-ordered wave order: module_b is the provider
    // and must appear in an earlier compile wave than consumer module_a. The inventory
    // preserves wave boundaries so the directory compiler can schedule providers before
    // consumers.
    let waves = modules.waves();
    let provider_wave = waves
        .iter()
        .position(|wave| {
            wave.iter().any(|module| {
                prepared_entry_file_path(&module.prepared.semantic)
                    .expect("prepared module retains its entry file identity")
                    .file_name()
                    == Some(OsStr::new("@api.moth"))
            })
        })
        .expect("module_b should appear in a compile wave");
    let consumer_wave = waves
        .iter()
        .position(|wave| {
            wave.iter().any(|module| {
                prepared_entry_file_path(&module.prepared.semantic)
                    .expect("prepared module retains its entry file identity")
                    .file_name()
                    == Some(OsStr::new("@pageA.moth"))
            })
        })
        .expect("module_a should appear in a compile wave");
    assert!(
        provider_wave < consumer_wave,
        "provider module_b must be in an earlier wave than consumer module_a"
    );
    assert_eq!(
        waves[provider_wave].len(),
        1,
        "the provider is the sole entry in its wave"
    );
    assert_eq!(
        waves[consumer_wave].len(),
        1,
        "the sole consumer is the only entry in its wave"
    );
}

#[test]
fn same_module_dependency_creates_no_project_graph_edge() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create src");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    // The single entry module depends on a sibling file inside its own module root.
    fs::write(src.join("@page.moth"), "@helper\n#[:page]\n").expect("should write entry");
    fs::write(src.join("helper.moth"), "value #= 1\n").expect("should write helper");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let (modules, graph, _source_tree_index, _source_files, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    let entry_root = graph.entry_modules().to_vec();
    assert_eq!(entry_root.len(), 1, "there is one normal entry module");

    // Same-module dependencies create no project-graph edge, so the graph has no edges and one wave.
    let waves = graph.compile_waves().expect("no-edge graph waves cleanly");
    assert_eq!(waves.len(), 1, "no edges means a single ready wave");
    assert_eq!(
        modules.waves().iter().map(|wave| wave.len()).sum::<usize>(),
        1
    );

    // The inventory preserves wave boundaries: the single no-edge entry is the sole module in
    // one ready wave.
    let inventory_waves = modules.waves();
    assert_eq!(
        inventory_waves.len(),
        1,
        "one no-edge entry produces one inventory wave"
    );
    assert_eq!(
        inventory_waves[0].len(),
        1,
        "the singleton wave contains the one no-edge entry"
    );
}

#[test]
fn independent_no_edge_entries_are_grouped_in_one_ready_wave() {
    // Two entry modules with no cross-module dependency edges must be grouped in the same
    // dependency-ready wave; the serial scheduler can then publish them in deterministic order.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let module_a = src.join("module_a");
    let module_b = src.join("module_b");
    fs::create_dir_all(&module_a).expect("should create module_a");
    fs::create_dir_all(&module_b).expect("should create module_b");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    // Two independent entry modules with no cross-module dependencies.
    fs::write(module_a.join("@pageA.moth"), "#[:pageA]\n").expect("should write pageA");
    fs::write(module_b.join("@pageB.moth"), "#[:pageB]\n").expect("should write pageB");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let (modules, graph, _source_tree_index, _source_files, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    // No cross-module edges means a single ready wave containing both entries.
    let graph_waves = graph.compile_waves().expect("no-edge graph waves cleanly");
    assert_eq!(graph_waves.len(), 1, "no edges means a single ready wave");

    let inventory_waves = modules.waves();
    assert_eq!(
        inventory_waves.len(),
        1,
        "two no-edge entries produce one inventory wave"
    );
    assert_eq!(
        inventory_waves[0].len(),
        2,
        "both no-edge entries are grouped in the same wave"
    );

    // The inventory wave preserves the graph's canonical ModuleId order exactly. Derive the
    // expected entry order from the graph wave rather than assuming a filename sort, then assert
    // the inventory matches it position-for-position. Every node in this no-edge wave is a normal
    // entry, so the graph wave order is the expected entry order.
    let expected_order: Vec<String> = graph_waves[0]
        .iter()
        .map(|module_id| {
            graph
                .node(*module_id)
                .root_file()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    let inventory_order: Vec<String> = inventory_waves[0]
        .iter()
        .map(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("prepared module retains its entry file identity")
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    assert_eq!(
        inventory_order, expected_order,
        "the inventory wave preserves the graph's canonical ModuleId order"
    );
}

#[test]
fn duplicate_dependency_deduplicates_edge_and_orders_provider_first() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let module_a = src.join("module_a");
    let module_b = module_a.join("module_b");
    fs::create_dir_all(&module_a).expect("should create module_a");
    fs::create_dir_all(&module_b).expect("should create module_b");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    // The consumer depends on its direct child twice, so the duplicate observation must be
    // idempotent.
    fs::write(
        module_a.join("@pageA.moth"),
        "@module_b\n@module_b\n#[:pageA]\n",
    )
    .expect("should write pageA");
    fs::write(module_b.join("@api.moth"), "export:\n    b #= 1\n;\n")
        .expect("should write module_b root");
    fs::write(module_b.join("impl.moth"), "impl #= 1\n").expect("should write module_b impl");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let (modules, graph, source_tree_index, _source_files, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    let module_a_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&fs::canonicalize(&module_a).unwrap())
        .expect("module_a root should be a graph node");
    let module_b_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&fs::canonicalize(&module_b).unwrap())
        .expect("module_b root should be a graph node");

    // The duplicate observation collapses to one provider-before-consumer edge.
    assert!(graph.has_dependency_edge(module_b_id, module_a_id));
    assert!(!graph.has_dependency_edge(module_a_id, module_b_id));

    // module_b is the provider and must appear in an earlier compile wave than its consumer.
    let waves = graph
        .compile_waves()
        .expect("duplicate-edge graph waves cleanly");
    let provider_wave = waves
        .iter()
        .position(|wave| wave.contains(&module_b_id))
        .expect("module_b should appear in a wave");
    let consumer_a_wave = waves
        .iter()
        .position(|wave| wave.contains(&module_a_id))
        .expect("module_a should appear in a wave");
    assert!(
        provider_wave < consumer_a_wave,
        "the provider must precede its consumer in compile-wave order"
    );

    // The inventory preserves the provider and consumer wave boundary.
    let inventory_waves = modules.waves();
    assert_eq!(
        inventory_waves.len(),
        2,
        "one provider wave and one consumer wave"
    );
    assert_eq!(
        inventory_waves[0].len(),
        1,
        "the provider is the sole entry in the first wave"
    );
    assert!(
        prepared_entry_file_path(&inventory_waves[0][0].prepared.semantic)
            .expect("prepared module retains its entry file identity")
            .file_name()
            .is_some_and(|name| name == "@api.moth"),
        "module_b is the provider in the first wave"
    );
    assert_eq!(
        inventory_waves[1].len(),
        1,
        "the consumer is the sole entry in the second wave"
    );
    assert!(
        prepared_entry_file_path(&inventory_waves[1][0].prepared.semantic)
            .expect("prepared module retains its entry file identity")
            .file_name()
            .is_some_and(|name| name == "@pageA.moth"),
        "module_a is the consumer in the second wave"
    );
}

#[test]
fn dependency_fact_retains_authored_source_location() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let (config, resolver, style_directives, module_a_root, module_b_root) =
        write_cross_module_project(&root);

    let (_modules, graph, source_tree_index, source_files, string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    let module_a_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&module_a_root)
        .expect("module_a root should be a graph node");
    let module_b_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&module_b_root)
        .expect("module_b root should be a graph node");

    let retained_span = graph
        .edge_source_span(module_b_id, module_a_id)
        .expect("the provider-before-consumer edge should retain its authored span");

    // The retained source identity is the declaring source file that authored the structural
    // provider reference.
    let scope_path = source_files
        .legacy_logical_path(retained_span.source())
        .to_portable_string(&string_table);
    assert!(
        scope_path.contains("@pageA.moth"),
        "retained span source should name the declaring module root file: {scope_path}"
    );
    // The dependency clause starts at byte zero of the declaring source file.
    assert_eq!(
        retained_span.byte_range(&source_files).start(),
        0,
        "retained span should point at the first authored source byte"
    );
}

#[test]
fn production_graph_completes_before_scheduling() {
    // Hidden invariant: the production discovery path completes the project module graph before
    // compile-wave scheduling, freezing adjacency into sorted `Vec<ModuleId>` storage. The
    // completed graph schedules cleanly from its frozen adjacency, and any later edge insertion
    // is rejected as mutation after completion.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let (config, resolver, style_directives, module_a_root, module_b_root) =
        write_cross_module_project(&root);

    let (modules, mut graph, source_tree_index, _source_files, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    let module_a_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&module_a_root)
        .expect("module_a root should be a graph node");
    let module_b_id = source_tree_index
        .module_identities()
        .module_id_for_directory(&module_b_root)
        .expect("module_b root should be a graph node");

    // The production graph is completed, so scheduling reads its frozen adjacency and reproduces
    // the same provider-before-consumer wave order the inventory already exposes.
    let graph_waves = graph
        .compile_waves()
        .expect("production graph should be completed and schedulable");
    let provider_wave = graph_waves
        .iter()
        .position(|wave| wave.contains(&module_b_id))
        .expect("provider module_b should appear in a wave");
    let consumer_wave = graph_waves
        .iter()
        .position(|wave| wave.contains(&module_a_id))
        .expect("consumer module_a should appear in a wave");
    assert!(
        provider_wave < consumer_wave,
        "frozen adjacency keeps the provider before its consumer"
    );

    // The inventory waves agree with the completed graph's provider-before-consumer order.
    let inventory_waves = modules.waves();
    assert_eq!(
        inventory_waves.len(),
        2,
        "the completed graph produces one provider wave and one consumer wave"
    );

    // Edge insertion after completion is mutation after the graph is frozen, reported as an
    // internal compiler failure rather than silently accepted.
    let mutation_error = graph
        .add_dependency_edge(module_b_id, module_a_id)
        .expect_err("mutation after production completion must be rejected");
    assert_eq!(mutation_error.error_type, ErrorType::Compiler);
    assert!(
        mutation_error.msg.contains("after completion"),
        "mutation error must name the phase violation: {}",
        mutation_error.msg
    );
}

#[test]
fn discovered_modules_carry_both_graph_assigned_identities() {
    // Hidden invariant: directory discovery must preserve both graph identities rather than
    // re-deriving either from an entry path. The dense ID remains the build-owned scheduling and
    // merge key; the stable origin remains the portable semantic identity.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let (config, resolver, style_directives, _module_a_root, _module_b_root) =
        write_cross_module_project(&root);

    let (modules, graph, _source_tree_index, source_files, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    assert!(
        modules.waves().iter().map(|wave| wave.len()).sum::<usize>() > 0,
        "the cross-module project should discover at least one normal entry module"
    );

    for module in modules.waves().iter().flatten() {
        let matching_node = graph.nodes().iter().find(|node| {
            node.root_file()
                == prepared_entry_canonical_file_path(&module.prepared.semantic, &source_files)
                    .expect("prepared module retains its entry file identity")
        });
        let matching_node = matching_node.expect(
            "every discovered module entry point must match a graph node's canonical root file",
        );
        assert_eq!(
            module.module_id,
            matching_node.module_id(),
            "discovered module ID must equal its graph-assigned dense identity (entry {:?})",
            prepared_entry_canonical_file_path(&module.prepared.semantic, &source_files)
                .expect("prepared module retains its entry file identity"),
        );
        assert_eq!(
            module.stable_origin,
            *matching_node.stable_origin(),
            "discovered module stable origin must equal its graph-assigned origin (entry {:?})",
            prepared_entry_canonical_file_path(&module.prepared.semantic, &source_files)
                .expect("prepared module retains its entry file identity"),
        );
    }
}

#[test]
fn discovered_module_origin_is_not_rederived_from_a_path_component() {
    // Hidden invariant: the stable origin carried by discovery is the graph-owned value type, not
    // a path-derived fallback. The discovered origins must be distinct `StableModuleOriginIdentity`
    // values keyed by canonical logical module path, and must round-trip through the graph node.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let (config, resolver, style_directives, _module_a_root, _module_b_root) =
        write_cross_module_project(&root);

    let (modules, _graph, _source_tree_index, _source_files, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    let modules: Vec<_> = modules.waves().iter().flatten().collect();

    // Each module carries a distinct stable origin keyed by its logical module path. The two
    // cross-module roots have different logical paths, so their origins must differ.
    let origins: Vec<StableModuleOriginIdentity> = modules
        .iter()
        .map(|module| module.stable_origin.clone())
        .collect();
    let unique: std::collections::HashSet<StableModuleOriginIdentity> =
        origins.iter().cloned().collect();
    assert_eq!(
        unique.len(),
        modules.len(),
        "each discovered module must carry its own distinct graph-assigned stable origin"
    );
}

#[test]
fn build_source_origin_lookup_maps_each_owned_file_to_its_node_origin() {
    // Hidden invariant: the source-origin lookup is a direct projection of the central
    // `SourceTreeIndex` ownership through the graph's owned source IDs. Every owned source
    // record's logical identity module origin must equal its containing graph node's stable
    // origin, and no canonical path may appear twice.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let (config, resolver, style_directives, _module_a_root, _module_b_root) =
        write_cross_module_project(&root);

    let (_modules, graph, source_tree_index, _source_files, _string_table) =
        discover_modules_and_graph_for_test(&config, &resolver, &style_directives);

    let lookup = graph
        .build_source_origin_lookup(&source_tree_index)
        .expect("the source-origin lookup should build for a valid cross-module project");

    // Every lookup entry's origin must equal the stable origin of the graph node that owns it.
    // The graph node carries no source records; owned source data is resolved through the
    // retained central index, so the lookup must cover exactly the index's owned source IDs.
    for node in graph.nodes() {
        for source_id in source_tree_index.owned_source_indices(node.module_id()) {
            let record = source_tree_index.source(*source_id);
            let lookup_origin = lookup
                .get(record.canonical_path())
                .expect("every owned source entry must be present in the lookup");
            assert_eq!(
                lookup_origin,
                node.stable_origin(),
                "an owned source entry's lookup origin must equal its containing node origin (path: {:?})",
                record.canonical_path().display(),
            );
        }
    }

    // No canonical path may appear under two different origins: the lookup is a function, not a
    // relation. Duplicates would have failed inside `build_source_origin_lookup`, so reaching
    // here with every entry validated confirms single-ownership.
    let unique_paths: HashSet<&std::path::Path> = lookup.keys().map(|p| p.as_path()).collect();
    let total_entries: usize = graph
        .nodes()
        .iter()
        .map(|node| {
            source_tree_index
                .owned_source_indices(node.module_id())
                .len()
        })
        .sum();
    assert_eq!(
        unique_paths.len(),
        total_entries,
        "every owned source path must be unique across all graph nodes"
    );
}

#[test]
fn canonical_module_job_excludes_cross_module_donor_sources() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let provider = src.join("a_provider");
    fs::create_dir_all(&provider).expect("should create provider module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "@z_local\n@a_provider value\n#[:entry]\n",
    )
    .expect("should write consumer root");
    fs::write(src.join("z_local.moth"), "local #= 1\n").expect("should write local source");
    fs::write(provider.join("@api.moth"), "export:\n    value #= 1\n;\n")
        .expect("should write provider root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let modules = discover_modules_for_test(&config, &resolver, &style_directives)
        .expect("module discovery should pass");
    let consumer = modules
        .waves()
        .iter()
        .flatten()
        .find(|module| {
            prepared_entry_file_path(&module.prepared.semantic)
                .expect("prepared module retains its entry file identity")
                .file_name()
                == Some(OsStr::new("@page.moth"))
        })
        .expect("consumer module should be discovered");
    let input_names = module_prepared_source_names(consumer);

    assert_eq!(
        input_names,
        vec!["@page.moth", "z_local.moth"],
        "the consumer job must contain only its canonical prepared sources; the provider reaches binding through its completed interface"
    );
}

#[test]
fn indexed_namespace_rejects_direct_entry_root_dependency() {
    // Path components starting with `@` are now rejected by the path parser before
    // namespace resolution. The `@` introducer is consumed by the lexer, so any
    // component starting with `@` is a `@@` form that has no valid dependency meaning.
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    fs::create_dir_all(&src).expect("should create source root");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@@page symbol\n#[:page]\n")
        .expect("should write entry root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let messages = match discover_modules_for_test(&config, &resolver, &style_directives) {
        Ok(_) => panic!("direct entry-root @@page dependency must be rejected by the path parser"),
        Err(messages) => messages,
    };
    assert_eq!(
        first_error_diagnostic(&messages).kind.code(),
        "MOTH-SYNTAX-0018"
    );
}

#[test]
fn indexed_namespace_rejects_direct_nested_child_root_dependency() {
    let _tmp_root = tempfile::tempdir().expect("should create temp dir");
    let root = _tmp_root.path().to_path_buf();
    let src = root.join("src");
    let child = src.join("child");
    fs::create_dir_all(&child).expect("should create child module");

    fs::write(
        root.join(settings::CONFIG_FILE_NAME),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@child/@home symbol\n#[:page]\n")
        .expect("should write parent root");
    fs::write(child.join("@home.moth"), "export:\n    symbol #= 1\n;\n")
        .expect("should write child root");

    let mut config = Config::new(root.clone());
    let style_directives = test_style_directives();
    parse_project_config_for_test(
        &mut config,
        &root.join(settings::CONFIG_FILE_NAME),
        &style_directives,
    )
    .expect("config should parse");
    let resolver = configured_resolver(&config);

    let messages = match discover_modules_for_test(&config, &resolver, &style_directives) {
        Ok(_) => {
            panic!("direct nested-child root dependencies must be rejected by the path parser")
        }
        Err(messages) => messages,
    };
    assert_eq!(
        first_error_diagnostic(&messages).kind.code(),
        "MOTH-SYNTAX-0018"
    );
}

#[test]
fn provider_binding_index_rejects_duplicate_shell_edges() {
    let shell = DependencyShellId::new(SourceId::from_index(0), 0);
    let edges = vec![
        ResolvedDependencyEdge {
            provider_module_id: ModuleId::from_index(1),
            consumer_module_id: ModuleId::from_index(0),
            dependency_shell_id: shell,
            graph_span: None,
        },
        ResolvedDependencyEdge {
            provider_module_id: ModuleId::from_index(2),
            consumer_module_id: ModuleId::from_index(0),
            dependency_shell_id: shell,
            graph_span: None,
        },
    ];

    let error = super::compilation::build_provider_binding_index(&edges)
        .expect_err("one retained shell must resolve to exactly one provider edge");

    assert!(
        error.msg.contains("more than one provider edge"),
        "unexpected error: {}",
        error.msg
    );
}

#[test]
fn source_package_dependency_index_rejects_cross_category_or_duplicate_shells() {
    let shell = DependencyShellId::new(SourceId::from_index(0), 0);
    let provider_edge = ResolvedDependencyEdge {
        provider_module_id: ModuleId::from_index(1),
        consumer_module_id: ModuleId::from_index(0),
        dependency_shell_id: shell,
        graph_span: None,
    };
    let provider_binding_index = super::compilation::build_provider_binding_index(&[provider_edge])
        .expect("one provider edge should index");

    let package_dependency = ResolvedSourcePackageDependency {
        consumer_module_id: ModuleId::from_index(0),
        dependency_prefix: "markdown".to_owned(),
        dependency_shell_id: shell,
    };

    let error = super::compilation::build_source_package_dependency_index(
        &provider_binding_index,
        &[package_dependency],
    )
    .expect_err("one shell cannot address both a provider module and a source package");

    assert!(
        error
            .msg
            .contains("both a provider module and a source package"),
        "unexpected error: {}",
        error.msg
    );
}
