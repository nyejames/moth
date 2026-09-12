use super::*;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
#[test]
fn source_package_config_inputs_are_isolated_from_project_inputs() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temporary project");
    let dir = _temp.path().to_path_buf();
    let package = dir.join("packages/config_input");
    let src = dir.join("src");
    fs::create_dir_all(&package).expect("should create source package root");
    fs::create_dir_all(&src).expect("should create project entry root");
    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        src.join("@mod.moth"),
        "@config_input helper\nproject_snapshot = 1\nenabled #Config of Bool\nvalue = helper()\n",
    )
    .expect("should write project source contract");
    fs::write(
        package.join("@mod.moth"),
        "enabled #Config of Bool\nexport:\n    helper || -> Int:\n        return 2\n    ;\n;\n",
    )
    .expect("should write source-package contract");

    let mut build_config_inputs = BuildConfigInputSet::new();
    build_config_inputs
        .insert(BuildConfigInputEntry::new(
            BuildInputName::new("enabled").expect("test input name should be valid"),
            PrimitiveBuildValue::Bool(true),
            BuildConfigValueLocation::Command(BuildCommandLocation::new(0)),
        ))
        .expect("project input should be unique");

    let mut config = Config::new(dir.clone());
    config.entry_root = PathBuf::from("src");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut project_source_files = None;
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "config_input",
        package,
        PackageOrigin::Builder,
    );

    let result = compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
        &mut project_source_files,
        &build_config_inputs,
        FrontendCompilationMode::Canonical,
    );
    let Err(mut messages) = result else {
        panic!("source-package config contract must not consume the project input");
    };
    messages.set_source_database(Arc::clone(
        project_source_files
            .as_ref()
            .expect("directory frontend should retain the project source database"),
    ));
    let observed_identities = messages
        .error_diagnostics()
        .map(|diagnostic| diagnostic.identity())
        .collect::<Vec<_>>();

    let matching = messages
        .diagnostics()
        .enumerate()
        .filter(|(_, diagnostic)| {
            diagnostic.severity
                == crate::compiler_frontend::compiler_messages::DiagnosticSeverity::Error
                && matches!(
                    &diagnostic.payload,
                    DiagnosticPayload::InvalidConfig {
                        reason: InvalidConfigReason::MissingConfigInput,
                        ..
                    }
                )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        matching.len(),
        1,
        "the isolated source package should report exactly one missing-input diagnostic; observed {observed_identities:?}"
    );
    let (diagnostic_index, diagnostic) = matching[0];
    let DiagnosticPayload::InvalidConfig {
        key: Some(key),
        reason: InvalidConfigReason::MissingConfigInput,
    } = &diagnostic.payload
    else {
        unreachable!("the diagnostic was filtered to the missing-input payload");
    };
    assert_eq!(messages.string_table.resolve(*key), "enabled");
    let span = diagnostic
        .primary_span
        .expect("source-package config diagnostic should retain its authored span");
    assert_ne!(
        span.source(),
        crate::compiler_frontend::source::SourceId::COMPILATION_ROOT,
        "source-package diagnostics must not use the synthetic compilation-root span",
    );
    assert!(
        messages
            .diagnostic_render_context(diagnostic_index)
            .primary_position(diagnostic)
            .is_some(),
        "source-package diagnostics should retain source context",
    );
    let rendered = render_compiler_messages_html(&messages, &dir);
    assert!(
        rendered.contains("enabled #Config of Bool"),
        "the missing-input frame should use the package snapshot: {rendered}"
    );
    assert!(
        !rendered.contains("project_snapshot = 1"),
        "the package diagnostic must not use the colliding project snapshot: {rendered}"
    );
}

#[test]
fn directory_graph_retains_diagnostics_from_later_independent_source_packages() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _tmp_dir = tempfile::tempdir().expect("should create temp dir");
    let dir = _tmp_dir.path().to_path_buf();
    let first_package = dir.join("packages/first");
    let second_package = dir.join("packages/second");
    fs::create_dir_all(&first_package).expect("should create first package");
    fs::create_dir_all(&second_package).expect("should create second package");
    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
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
    let mut path_fork = PathInternerFork::empty();
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

    let mut frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("diagnosed source packages are retained in the typed frontend outcome");
    let project_source = frontend.project_source_database.take();
    let messages = frontend
        .into_render_messages_with_frozen_identity(&mut string_table, project_source, None)
        .expect("source-package diagnostics should install frozen render identity");

    assert!(
        messages.error_count() >= 2,
        "both diagnosed source packages should retain their errors"
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
            .any(|path| path.ends_with("packages/first/@mod.moth")),
        "first package diagnostic should be retained: {diagnosed_paths:?}"
    );
    assert!(
        diagnosed_paths
            .iter()
            .any(|path| path.ends_with("packages/second/@mod.moth")),
        "later independent package should still compile: {diagnosed_paths:?}"
    );
}

#[test]
fn project_consumers_blocked_by_diagnosed_source_package_are_not_infrastructure_errors() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _tmp_dir = tempfile::tempdir().expect("should create temp dir");
    let dir = _tmp_dir.path().to_path_buf();
    let package = dir.join("packages/broken");
    let src = dir.join("src");
    fs::create_dir_all(&package).expect("should create package root");
    fs::create_dir_all(&src).expect("should create entry root");
    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@broken run\nvalue = run()\n")
        .expect("should write blocked project consumer");
    fs::write(
        package.join("@mod.moth"),
        "export:\n    run || -> Int:\n        return missing_package_value\n    ;\n;\n",
    )
    .expect("should write diagnosed source package");

    let mut config = Config::new(dir.clone());
    config.entry_root = PathBuf::from("src");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "broken",
        package,
        PackageOrigin::Builder,
    );

    let mut frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("a diagnosed package with blocked project consumers is a retained outcome");

    assert_eq!(
        frontend
            .source_packages
            .get(0)
            .expect("package boundary retained")
            .boundary
            .diagnosed
            .len(),
        1,
        "package diagnostic should be retained in its own boundary"
    );
    assert_eq!(
        frontend.project.blocked.len(),
        1,
        "project consumer should be blocked, not an infrastructure failure"
    );
    assert_eq!(
        frontend.project.diagnosed.len(),
        0,
        "the project boundary itself should have no diagnostic"
    );

    let project_source = frontend.project_source_database.take();
    let messages = frontend
        .into_render_messages_with_frozen_identity(&mut string_table, project_source, None)
        .expect("package diagnostics should install frozen render identity");
    assert_eq!(
        messages.error_count(),
        1,
        "the package diagnostic should render once"
    );
}

#[test]
fn same_module_generated_sidecars_rebuild_const_templates_in_their_fresh_store() {
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
    let mut path_fork = PathInternerFork::empty();
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
    let sidecars = frontend
        .project
        .generated
        .sidecars()
        .chain(
            frontend
                .source_packages
                .iter()
                .flat_map(|package| package.boundary.generated.sidecars()),
        )
        .collect::<Vec<_>>();

    assert_eq!(
        sidecars.len(),
        1,
        "the concrete wrap request needs one sidecar"
    );
}

#[test]
fn generated_sidecar_refreshes_active_base_public_summary() {
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
        r#"export:
    public_helper |value ~Int| -> Int:
        return seed_helper(~value, "seed")
    ;
;

seed_helper type T |value ~Int, marker T| -> Int:
    value = value + 1
    return value
;

mutating_helper type T |value ~Int, marker T| -> Int:
    return public_helper(~value)
;

caller type T |value ~Int, marker T| -> Int:
    return mutating_helper(~value, marker)
;

independent type T |value T| -> T:
    return value
;

counter ~Int = 1
result Int = caller(~counter, "seed")
independent_result Int = independent(42)
"#,
    )
    .expect("should write active-base public fixture");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let _counter_capture =
        crate::compiler_frontend::instrumentation::capture_frontend_counters_for_test();
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let _counter_guard = {
        crate::compiler_frontend::instrumentation::reset_frontend_counters();
        Some(crate::timing::start_benchmark_collection(true).expect("timing session should start"))
    };
    let frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    )
    .expect("active-base public generic call should compile");
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    let convergence_observations = {
        crate::compiler_frontend::instrumentation::log_frontend_counters();
        _counter_guard
            .expect("counter timing session should exist")
            .finish()
    };
    #[cfg(all(feature = "timers", feature = "benchmark_counters"))]
    {
        let counter_value = |name: &str| {
            convergence_observations
                .counters
                .iter()
                .find(|counter| counter.name == name)
                .map(|counter| counter.value)
                .unwrap_or(-1.0)
        };
        assert_eq!(counter_value("convergence_initial_base_borrow_passes"), 1.0);
        assert_eq!(counter_value("convergence_base_borrow_passes"), 2.0);
        assert_eq!(
            counter_value("convergence_generated_sidecar_borrow_passes"),
            9.0
        );
        assert_eq!(
            counter_value("convergence_complete_generated_summary_map_builds"),
            0.0
        );
        assert_eq!(
            counter_value("convergence_generated_summary_map_clones"),
            0.0
        );
        assert_eq!(
            counter_value("convergence_private_summary_map_rebuilds"),
            0.0
        );
        assert_eq!(counter_value("convergence_stable_sidecars_rechecked"), 0.0);
        assert_eq!(counter_value("convergence_max_iterations"), 0.0);
    }

    let base_module = frontend
        .project
        .successful_artefacts_in_module_id_order()
        .map(|artifact| &artifact.module)
        .next()
        .expect("project should retain a base module");
    let (active_origin, active_function_id) = base_module
        .executable
        .hir
        .function_ids_by_origin
        .iter()
        .find(|(origin, _)| origin.defining_name() == "public_helper")
        .map(|(origin, function_id)| (origin.clone(), *function_id))
        .expect("public helper should retain its stable origin");
    assert_eq!(
        frontend.project.generated.sidecars().count(),
        4,
        "the base-to-generated chain and independent request should materialise once each"
    );
    let sidecar = frontend
        .project
        .generated
        .sidecars()
        .find(|sidecar| {
            sidecar.module.executable.hir.blocks.iter().any(|block| {
                block.statements.iter().any(|statement| {
                    matches!(
                        &statement.kind,
                        HirStatementKind::Call {
                            target: CallTarget::CrossModule(origin),
                            ..
                        } if origin == &active_origin
                    )
                })
            })
        })
        .expect("the generated caller should retain the active-base CrossModule call");
    let exact_summary = base_module
        .executable
        .borrow_analysis
        .analysis
        .public_call_summaries
        .get(&active_function_id)
        .expect("base report should retain the public helper summary");
    assert_eq!(
        exact_summary.parameters[0].mutation,
        PublicCallMutationEffect::Writes,
        "the helper summary should be widened through its generated callee"
    );
    assert!(
        sidecar.module.executable.hir.blocks.iter().any(|block| {
            block.statements.iter().any(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Call {
                        target: CallTarget::CrossModule(origin),
                        ..
                    } if origin == &active_origin
                )
            })
        }),
        "the generated sidecar should retain the active-base CrossModule call"
    );
    assert_eq!(
        sidecar
            .module
            .executable
            .hir
            .imported_call_summaries
            .get(&active_origin),
        Some(exact_summary),
        "the sidecar should receive the exact active-base public summary"
    );
}

#[test]
fn generated_materialisation_preserves_exact_request_span_in_recursive_diagnostic() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    let function_name = "l".repeat(1024);
    fs::create_dir_all(dir.join("src")).expect("should create source root");

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("src/helpers.moth"),
        format!("{function_name} type T |value T| -> T:\n    return {function_name}(value)\n;\n"),
    )
    .expect("should write generic helper");
    let entry_source =
        format!("prefix String = \"é\"\n@helpers {function_name}\nvalue = {function_name}(1)\n");
    fs::write(dir.join("src/@page.moth"), &entry_source).expect("should write entry source");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    )
    .expect("diagnosed recursive materialisation should remain a retained frontend outcome");
    let project_source = frontend.project_source_database.take();
    let messages = frontend
        .into_render_messages_with_frozen_identity(&mut string_table, project_source, None)
        .expect("generated diagnostics should install frozen render identity");

    let diagnostic = messages
        .error_diagnostics()
        .next()
        .expect("recursive materialisation should retain one error diagnostic");
    assert!(matches!(
        &diagnostic.payload,
        DiagnosticPayload::InvalidGenericInstantiation {
            reason: InvalidGenericInstantiationReason::RecursiveFunctionInstantiation,
            ..
        }
    ));
    let frozen_identity = messages
        .frozen_identity_context_for_diagnostic(0)
        .expect("generated diagnostic should retain its frozen identity context");
    let entry_path = fs::canonicalize(dir.join("src/@page.moth"))
        .expect("entry source path should canonicalize");
    let span = diagnostic
        .primary_span
        .expect("recursive generated diagnostic should retain the request span");
    let entry_id = span.source();
    let entry_slot = frozen_identity
        .get(entry_id)
        .expect("frozen identity should resolve the request source");
    assert_eq!(
        entry_slot.canonical_os_path.as_deref(),
        Some(entry_path.as_path()),
        "request span should resolve to the entry source in the frozen identity"
    );
    let range = span.byte_range(frozen_identity);
    let final_call = format!("{function_name}(1)");
    let expected_start = entry_source
        .rfind(&final_call)
        .expect("the final generic call should be present") as u32;
    assert_eq!(range.start(), expected_start);
    assert_eq!(range.end(), expected_start + function_name.len() as u32);
}

#[test]
fn imported_generic_materialisation_preserves_donor_identity_with_colliding_sources_and_extended_label()
 {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    let package_temp = tempfile::tempdir().expect("should create package temp dir");
    let package_root = package_temp.path().to_path_buf();
    let function_name = String::from("outer");
    let inner_name = String::from("inner");
    let parameter_name = "p".repeat(1024);
    let project_path = dir.join("src/@page.moth");
    let package_path = package_root.join("@mod.moth");
    fs::create_dir_all(dir.join("src")).expect("should create project source root");

    fs::write(
        dir.join("config.moth"),
        "project #= |
    name = \"docs\",
    entry_root = \"src\",
|
html #= ||\n",
    )
    .expect("should write config");
    let package_source = format!(
        "{inner_name} type U |text String, result U| -> U:\n    return result\n;\n\nexport:\n    {function_name} type T |{parameter_name} T| -> Int:\n        if \"one\" is:\n            \"one\" => return 1\n            \"one\" => return 1\n            else => return 1\n        ;\n        return {inner_name}({parameter_name}, 1)\n    ;\n;\n",
    );
    let project_source = format!("@pkg {function_name}\nvalue Int = {function_name}(1)\n");
    fs::write(&package_path, &package_source).expect("should write package source");
    fs::write(&project_path, &project_source).expect("should write project source");

    let mut config = Config::new(dir.clone());
    config.entry_root = PathBuf::from("src");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "pkg",
        package_root,
        PackageOrigin::Builder,
    );
    let mut project_source_files = None;
    let frontend = compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
        &mut project_source_files,
        &BuildConfigInputSet::new(),
        FrontendCompilationMode::Canonical,
    )
    .expect("imported generic materialisation should remain retained");

    let project_source_database = project_source_files
        .as_ref()
        .expect("project source database should be retained");
    let package_source_database = frontend
        .source_packages
        .source_database(crate::build_system::create_project_modules::compiled_boundary::PackageBoundaryId::from_index(0))
        .expect("package source database should be retained");
    let project_source_id = project_source_database
        .iter()
        .next()
        .expect("project source database should contain the entry source")
        .id;
    let package_source_id = package_source_database
        .iter()
        .next()
        .expect("package source database should contain the package source")
        .id;
    assert_eq!(
        project_source_id, package_source_id,
        "project and package databases should begin with colliding SourceId values"
    );

    let messages = frontend
        .into_render_messages_with_frozen_identity(
            &mut string_table,
            project_source_files.take(),
            None,
        )
        .expect("imported generic diagnostics should install frozen render identity");
    let (diagnostic_index, diagnostic) = messages
        .diagnostics()
        .enumerate()
        .find(|(_, diagnostic)| {
            matches!(&diagnostic.payload, DiagnosticPayload::TypeMismatch { .. })
        })
        .expect("imported generic materialisation should retain one type mismatch diagnostic");

    let context = messages.diagnostic_render_context(diagnostic_index);
    let primary = context
        .primary_position(diagnostic)
        .expect("request primary span should resolve through the project snapshot");
    let project_path = fs::canonicalize(project_path).expect("project path should canonicalize");
    let package_path = fs::canonicalize(package_path).expect("package path should canonicalize");
    assert_eq!(primary.host_path, Some(project_path.as_path()));
    assert!(
        primary.path.ends_with("@page.moth"),
        "request primary path should be the project snapshot: {}",
        primary.path.display()
    );
    assert_eq!(
        diagnostic
            .primary_span
            .expect("materialised diagnostic should retain the project request span")
            .source(),
        project_source_id,
        "request span should retain the project database's colliding source ID"
    );

    let body_label = diagnostic
        .labels
        .iter()
        .find(|label| label.message == Some(DiagnosticLabelMessage::GenericInstantiationBodySite))
        .expect("generic diagnostic should retain a donor body label");
    let declaration_label = diagnostic
        .labels
        .iter()
        .find(|label| {
            label.message == Some(DiagnosticLabelMessage::GenericInstantiationDeclarationSite)
        })
        .expect("generic diagnostic should retain a donor declaration label");
    for label in [body_label, declaration_label] {
        let position = context
            .label_position(label)
            .expect("donor label should resolve through its frozen identity handle");
        assert_eq!(position.host_path, Some(package_path.as_path()));
        assert!(
            position.path.ends_with("@mod.moth"),
            "donor label should use the package snapshot: {}",
            position.path.display()
        );
        assert!(
            !position.line.is_empty(),
            "donor label should retain its package source line"
        );
        assert_eq!(
            label
                .span
                .expect("donor label should retain a source span")
                .source(),
            package_source_id,
            "donor label should retain the package database's colliding source ID"
        );
    }

    let extended_label = [body_label, declaration_label]
        .into_iter()
        .find(|label| {
            let Some(span) = label.span else {
                return false;
            };
            let Some(identity) = label
                .frozen_identity_handle
                .as_ref()
                .and_then(|handle| handle.get())
            else {
                return false;
            };
            let range = span.byte_range(identity);
            range.end() - range.start() > 1023
        })
        .expect("at least one donor label should resolve an extended span-table row");
    let extended_position = context
        .label_position(extended_label)
        .expect("extended donor label should resolve through package identity");
    assert_eq!(extended_position.host_path, Some(package_path.as_path()));
    let project_identity = messages
        .frozen_identity_context_for_diagnostic(diagnostic_index)
        .expect("request diagnostic should retain the project frozen identity");
    let project_slot = project_identity
        .get(
            extended_label
                .span
                .expect("extended label should have a span")
                .source(),
        )
        .expect("the colliding source ID should resolve in the project identity");
    assert_eq!(
        project_slot.canonical_os_path.as_deref(),
        Some(project_path.as_path()),
        "a package span must not be resolved through the project identity context"
    );
    let (warning_index, warning) = messages
        .diagnostics()
        .enumerate()
        .find(|(_, diagnostic)| {
            diagnostic.severity == DiagnosticSeverity::Warning && diagnostic.primary_span.is_some()
        })
        .expect("generated generic body should retain a spanful warning");
    let warning_context = messages.diagnostic_render_context(warning_index);
    let warning_position = warning_context
        .primary_position(warning)
        .expect("generated warning should resolve through its donor identity");
    assert_eq!(warning_position.host_path, Some(package_path.as_path()));
    assert!(
        warning_position.path.ends_with("@mod.moth"),
        "generated warning should use the package snapshot: {}",
        warning_position.path.display()
    );
}

#[test]
fn imported_nested_generic_materialisation_preserves_call_site_identity_with_colliding_sources() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    let package_temp = tempfile::tempdir().expect("should create package temp dir");
    let package_root = package_temp.path().to_path_buf();
    let project_path = dir.join("src/@page.moth");
    let package_path = package_root.join("@mod.moth");
    fs::create_dir_all(dir.join("src")).expect("should create project source root");

    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        &package_path,
        r#"inner type U |text String, value U| -> U:
    return text
;

export:
    outer type T |value T| -> T:
        return inner("prefix", value)
    ;
;
"#,
    )
    .expect("should write nested generic package source");
    fs::write(&project_path, "@pkg outer\nvalue Int = outer(1)\n")
        .expect("should write project source");

    let mut config = Config::new(dir.clone());
    config.entry_root = PathBuf::from("src");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "pkg",
        package_root,
        PackageOrigin::Builder,
    );
    let mut project_source_files = None;
    let frontend = compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
        &mut project_source_files,
        &BuildConfigInputSet::new(),
        FrontendCompilationMode::Canonical,
    )
    .expect("nested imported generic diagnostics should remain retained");

    let project_source_database = project_source_files
        .as_ref()
        .expect("project source database should be retained");
    let package_source_database = frontend
        .source_packages
        .source_database(crate::build_system::create_project_modules::compiled_boundary::PackageBoundaryId::from_index(0))
        .expect("package source database should be retained");
    let project_source_id = project_source_database
        .iter()
        .next()
        .expect("project source database should contain the entry source")
        .id;
    let package_source_id = package_source_database
        .iter()
        .next()
        .expect("package source database should contain the package source")
        .id;
    assert_eq!(
        project_source_id, package_source_id,
        "project and package databases should begin with colliding SourceId values"
    );

    let project_path = fs::canonicalize(project_path).expect("project path should canonicalize");
    let package_path = fs::canonicalize(package_path).expect("package path should canonicalize");
    let messages = frontend
        .into_render_messages_with_frozen_identity(
            &mut string_table,
            project_source_files.take(),
            None,
        )
        .expect("nested imported generic diagnostics should install frozen identities");
    let (diagnostic_index, diagnostic) = messages
        .diagnostics()
        .enumerate()
        .find(|(_, diagnostic)| {
            diagnostic
                .primary_span
                .is_some_and(|span| span.source() == package_source_id)
        })
        .expect("nested generic failure should retain its package-owned call-site span");
    let context = messages.diagnostic_render_context(diagnostic_index);
    let primary = context
        .primary_position(diagnostic)
        .expect("nested generic call-site span should resolve");
    assert_eq!(primary.host_path, Some(package_path.as_path()));
    assert!(
        primary.path.ends_with("@mod.moth"),
        "nested generic call site should use package source identity: {}",
        primary.path.display()
    );
    assert_ne!(
        primary.host_path,
        Some(project_path.as_path()),
        "nested generic call site must not use the colliding project snapshot"
    );
}

#[test]
fn generated_sidecars_reconstruct_complete_generic_nominal_members() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("provider")).expect("should create provider module");
    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
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
        r#"@provider forward

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
    let mut path_fork = PathInternerFork::empty();
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
    let sidecars = frontend
        .project
        .generated
        .sidecars()
        .chain(
            frontend
                .source_packages
                .iter()
                .flat_map(|package| package.boundary.generated.sidecars()),
        )
        .collect::<Vec<_>>();

    assert_eq!(
        sidecars.len(),
        6,
        "each outer request and nested private identity request needs one sidecar"
    );
    let mut saw_box = false;
    let mut saw_maybe = false;
    let mut saw_private_box = false;
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
                saw_box = true;
                let fields = environment
                    .fields_for(instance_type_id)
                    .expect("generated Box instance should expose substituted fields");
                assert_eq!(fields.len(), 1);
                assert_eq!(
                    sidecar.module.executable.path_table
                        .component(fields[0].name)
                        .map(|id| string_table.resolve(id)),
                    Some("value")
                );
                assert_eq!(fields[0].type_id, builtin_type_ids::INT);
            }
            "Maybe" => {
                saw_maybe = true;
                let variants = environment
                    .variants_for(instance_type_id)
                    .expect("generated Maybe instance should expose substituted variants");
                assert_eq!(variants.len(), 2);
                assert_eq!(string_table.resolve(variants[0].name), "Some");
                assert_eq!(string_table.resolve(variants[1].name), "Empty");
                let ChoiceVariantPayloadDefinition::Record { fields } = &variants[0].payload else {
                    panic!("Some should retain its record payload");
                };
                assert_eq!(fields.len(), 1);
                assert_eq!(
                    sidecar.module.executable.path_table
                        .component(fields[0].name)
                        .map(|id| string_table.resolve(id)),
                    Some("value")
                );
                assert_eq!(fields[0].type_id, builtin_type_ids::STRING);
                assert!(matches!(
                    variants[1].payload,
                    ChoiceVariantPayloadDefinition::Unit
                ));
            }
            name if name.ends_with("PrivateBox") => {
                saw_private_box = true;
                let fields = environment
                    .fields_for(instance_type_id)
                    .expect("generated private Box instance should expose substituted fields");
                assert_eq!(fields.len(), 1);
                assert_eq!(
                    sidecar.module.executable.path_table
                        .component(fields[0].name)
                        .map(|id| string_table.resolve(id)),
                    Some("value")
                );
                assert_eq!(fields[0].type_id, builtin_type_ids::BOOL);
            }
            other => panic!("unexpected generic nominal request base {other}"),
        }
    }
    assert!(saw_box && saw_maybe && saw_private_box);
}

#[test]
fn generated_sidecars_remap_inherited_nominals_after_multi_module_publication() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("provider")).expect("should create provider module");
    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(
        dir.join("provider/@mod.moth"),
        r#"export:
    RemoteMarker = |
        value Int,
    |

    seed || -> Int:
        return 1
    ;
;
"#,
    )
    .expect("should write provider");
    fs::write(
        dir.join("@page.moth"),
        r#"@provider seed, RemoteMarker as LocalMarker

inner type T |marker LocalMarker, value T| -> T:
    unused Int = seed()
    marker_value Int = marker.value
    return value
;

outer type T |marker LocalMarker, value T| -> T:
    return inner(marker, value)
;

result String = outer(LocalMarker(1), "trigger")
"#,
    )
    .expect("should write entry");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    string_table.intern("preexisting-global-name");
    let frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    )
    .expect("nested generated sidecars should publish after the provider module");

    let provider_path = path_fork
        .try_intern_portable_path("provider", &mut string_table)
        .expect("test path fits");
    let imported_marker_path = path_fork
        .try_intern_child(provider_path, string_table.intern("RemoteMarker"))
        .expect("test path fits");
    let published_alias_owners = frontend
        .project
        .successful_artefacts_in_module_id_order()
        .filter(|artifact| {
            artifact
                .module
                .executable
                .type_environment
                .nominal_id_for_path(&imported_marker_path)
                .is_some()
        })
        .count();
    assert_eq!(
        published_alias_owners, 1,
        "only the requesting module should publish its local nominal alias"
    );

    let sidecars = frontend.project.generated.sidecars().collect::<Vec<_>>();
    assert_eq!(
        sidecars.len(),
        2,
        "outer and nested inner should materialise"
    );

    for sidecar in sidecars {
        let environment = &sidecar.module.executable.type_environment;
        let marker_nominal_id = environment
            .nominal_id_for_path(&imported_marker_path)
            .expect("sidecar should resolve the inherited import path in the global string domain");
        let marker_type_id = environment
            .type_id_for_nominal_id(marker_nominal_id)
            .expect("sidecar should retain the inherited Marker type");
        assert_eq!(
            display_type(
                marker_type_id,
                environment,
                &string_table,
                &sidecar.module.executable.path_table,
            ),
            "RemoteMarker"
        );

        let fields = environment
            .fields_for(marker_type_id)
            .expect("sidecar should retain inherited Marker fields");
        assert_eq!(fields.len(), 1);
        assert_eq!(
            sidecar.module.executable.path_table
                .component(fields[0].name)
                .map(|id| string_table.resolve(id)),
            Some("value")
        );
    }
}

#[test]
fn generated_sidecars_reconstruct_hidden_facade_nominal_closure() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();
    fs::create_dir_all(dir.join("facade/provider")).expect("should create provider module");
    fs::create_dir_all(dir.join("generics")).expect("should create generic provider module");
    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
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
    @provider Wrapper, make
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
        r#"@facade Wrapper, make
@generics identity

wrapped Wrapper = identity(make())
"#,
    )
    .expect("should write entry");

    let mut config = Config::new(dir.clone());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
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
    let sidecars = frontend
        .project
        .generated
        .sidecars()
        .chain(
            frontend
                .source_packages
                .iter()
                .flat_map(|package| package.boundary.generated.sidecars()),
        )
        .collect::<Vec<_>>();

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
}

#[test]
fn source_package_warning_retained_by_frontend_outcome() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _tmp_dir = tempfile::tempdir().expect("should create temp dir");
    let dir = _tmp_dir.path().to_path_buf();
    let package = dir.join("packages/warnpkg");
    let src = dir.join("src");
    fs::create_dir_all(&package).expect("should create package root");
    fs::create_dir_all(&src).expect("should create entry root");
    fs::write(
        dir.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "value = 1\n").expect("should write project root");
    fs::write(
        package.join("@mod.moth"),
        "value ~= \"hello\"\nresult ~= \"unset\"\n\nif value is:\n    \"one\" => result = \"one\"\n    \"one\" => result = \"one\"\n    else => result = \"other\"\n;\n",
    )
    .expect("should write warning package root");

    let mut config = Config::new(dir.clone());
    config.entry_root = PathBuf::from("src");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "warnpkg",
        package,
        PackageOrigin::Builder,
    );
    let mut frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("warning source package should compile");

    let warning_codes = frontend
        .successful_module_views()
        .flat_map(|module| {
            module
                .metadata
                .warnings
                .iter()
                .map(|warning| warning.kind.code().to_owned())
        })
        .collect::<Vec<_>>();
    assert!(
        warning_codes.iter().any(|code| code == "MOTH-RULE-0022"),
        "source-package warning should be retained: {warning_codes:?}"
    );

    let project_source = frontend.project_source_database.take();
    let messages = frontend
        .into_render_messages_with_frozen_identity(&mut string_table, project_source, None)
        .expect("source-package warnings should install frozen render identity");
    assert!(
        messages.warning_count() >= 1,
        "render boundary should retain the source-package warning"
    );
}

struct SuccessfulPackageWarningBuilder {
    package_root: PathBuf,
}

impl BackendBuilder for SuccessfulPackageWarningBuilder {
    fn build_backend(
        &self,
        _project_compilation: ProjectCompilation,
        _config: &Config,
        _build_profile: BuildProfile,
        _flags: &[crate::compiler_frontend::Flag],
        _string_table: &mut StringTable,
    ) -> Result<Project, CompilerMessages> {
        Ok(Project {
            output_files: Vec::new(),
            entry_page_rel: None,
            cleanup_policy: CleanupPolicy::generic(Vec::<&str>::new()),
            warnings: Vec::new(),
            deferred_resources: Vec::new(),
            resource_inputs: ResourceInputRegistry::new(),
        })
    }

    fn validate_project_config(
        &self,
        _config: &Config,
        _string_table: &mut StringTable,
    ) -> Result<(), ProjectConfigError> {
        Ok(())
    }

    fn frontend_style_directives(
        &self,
    ) -> Vec<crate::compiler_frontend::style_directives::StyleDirectiveSpec> {
        Vec::new()
    }

    fn frontend_surface(&self) -> BuilderSurface {
        let mut surface = BuilderSurface::with_mandatory_core();
        surface.source_packages.register_filesystem_root(
            "pkg",
            self.package_root.clone(),
            PackageOrigin::Builder,
        );
        surface
    }
}

#[test]
fn successful_source_package_warning_uses_package_snapshot_for_colliding_logical_path() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
    let project_root = temporary_directory.path().to_path_buf();
    let project_source_root = project_root.join("src");
    let package_root = project_root.join("packages/pkg");
    fs::create_dir_all(&project_source_root).expect("should create project source root");
    fs::create_dir_all(&package_root).expect("should create source package root");
    fs::write(
        project_root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let project_text = "project_snapshot = 1\n";
    let package_text = "value ~= \"hello\"\nresult ~= \"unset\"\n\nif value is:\n    \"one\" => result = \"one\"\n    \"one\" => result = \"one\"\n    else => result = \"other\"\n;\n";
    fs::write(project_source_root.join("@mod.moth"), project_text)
        .expect("should write project module");
    fs::write(package_root.join("@mod.moth"), package_text).expect("should write package module");

    let builder = ProjectBuilder::new(Box::new(SuccessfulPackageWarningBuilder { package_root }));
    let build_result = build_project(
        &builder,
        project_root
            .to_str()
            .expect("temporary project path should be valid UTF-8"),
        &[],
        &BuildConfigInputSet::new(),
    )
    .expect("successful build should retain the package warning");

    assert!(
        build_result
            .warnings
            .iter()
            .any(|warning| warning.kind.code() == "MOTH-RULE-0022"),
        "successful build should retain the source-package warning"
    );

    let mut messages = CompilerMessages::from_diagnostics(
        build_result.warnings.clone(),
        build_result.string_table.clone(),
    );
    messages.install_source_contexts(build_result.warning_source_contexts.clone(), 0);
    if let Some(source_database) = build_result.source_database.as_ref() {
        messages.set_source_database(Arc::clone(source_database));
    }
    let rendered = render_compiler_messages_html(&messages, &project_root);

    assert!(
        rendered.contains("&quot;one&quot; =&gt; result = &quot;one&quot;"),
        "the package warning should render the package snapshot: {rendered}"
    );
    assert!(
        !rendered.contains(project_text.trim_end()),
        "the package warning must not render the colliding project snapshot: {rendered}"
    );
}

#[test]
fn source_package_diagnostic_uses_package_snapshot_for_colliding_logical_path() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temporary_directory = tempfile::tempdir().expect("should create temporary directory");
    let project_root = temporary_directory.path().to_path_buf();
    let project_source_root = project_root.join("src");
    let package_root = project_root.join("packages/pkg");
    fs::create_dir_all(&project_source_root).expect("should create project source root");
    fs::create_dir_all(&package_root).expect("should create source package root");
    fs::write(
        project_root.join("config.moth"),
        "project #= |\n    name = \"docs\",\n    entry_root = \"src\",\n|\nhtml #= ||\n",
    )
    .expect("should write config");

    let project_text = "project_snapshot = 1\n";
    let package_text = "package_snapshot Int = undefined_thing\n";
    fs::write(project_source_root.join("@mod.moth"), project_text)
        .expect("should write project module");
    fs::write(package_root.join("@mod.moth"), package_text).expect("should write package module");

    let mut config = Config::new(project_root.clone());
    config.entry_root = PathBuf::from("src");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "pkg",
        package_root,
        PackageOrigin::Builder,
    );

    let mut project_source_files = None;
    let frontend = compile_project_frontend_with_inputs(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
        &mut project_source_files,
        &BuildConfigInputSet::new(),
        FrontendCompilationMode::Canonical,
    )
    .expect("the package diagnostic should remain a retained frontend outcome");
    let messages = frontend
        .into_render_messages_with_frozen_identity(
            &mut string_table,
            project_source_files.take(),
            None,
        )
        .expect("package diagnostics should install frozen render identity");
    let rendered = render_compiler_messages_html(&messages, &project_root);

    assert!(
        rendered.contains(package_text.trim_end()),
        "the package diagnostic should render the package snapshot: {rendered}"
    );
    assert!(
        !rendered.contains(project_text.trim_end()),
        "the package diagnostic must not render the colliding project snapshot: {rendered}"
    );
}
