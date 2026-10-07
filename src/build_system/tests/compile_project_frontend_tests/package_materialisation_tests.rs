use super::*;
use crate::build_system::create_project_modules::compiled_boundary::ProjectFrontendCompilation;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_messages::InvalidFallibleHandlingReason;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::failure_facts::{
    HirBuiltinFailureBoundary, HirBuiltinFailureSource,
};
use crate::compiler_frontend::hir::numeric::NumericFailureMode;
use crate::compiler_frontend::hir::reachability::collect_reachability_from_function_link_facts;
use crate::compiler_frontend::public_interface::{
    PublicDeclarationSemantics, PublicFunctionCategory,
};
use crate::compiler_frontend::semantic_identity::OriginDeclarationId;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::utilities::basic::portable_path_text;
use moth_lexical::numeric::profile::NumericProfile;

#[test]
fn source_package_invalid_numeric_default_retains_its_source_for_directory_and_single_file() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let default_line = "limit #Config of Int = 2147483648";
    let package_source =
        format!("{default_line}\nexport:\n    run || -> Int:\n        return 1\n    ;\n;\n");
    let consumer_source = "@broken run\nvalue = run()\n";

    for (entry_mode, direct_file) in [("ordinary direct file", true), ("directory", false)] {
        let temporary_directory = tempfile::tempdir().expect("should create temporary project");
        let project_root = temporary_directory.path().to_path_buf();
        let source_root = project_root.join("src");
        let package_root = project_root.join("packages/broken");
        fs::create_dir_all(&source_root).expect("should create project source root");
        fs::create_dir_all(&package_root).expect("should create source package root");
        fs::write(
            project_root.join("config.moth"),
            "project #= (name = \"docs\", entry_root = \"src\")\nhtml #= ()\n",
        )
        .expect("should write project config");
        fs::write(package_root.join("@mod.moth"), &package_source)
            .expect("should write invalid source-package config default");

        let entry = if direct_file {
            let entry = source_root.join("page.moth");
            fs::write(&entry, consumer_source).expect("should write ordinary selected page");
            entry
        } else {
            fs::write(source_root.join("@page.moth"), consumer_source)
                .expect("should write directory entry page");
            project_root.clone()
        };

        let mut config = Config::new(entry);
        config.project_name = "docs".to_owned();
        config.entry_root = PathBuf::from("src");
        let style_directives = StyleDirectiveRegistry::built_ins();
        let mut string_table = StringTable::new();
        let mut frontend_surface = BuilderSurface::with_mandatory_core();
        frontend_surface.source_packages.register_filesystem_root(
            "broken",
            package_root,
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
            Err(messages) => messages,
            Ok(_) => panic!("{entry_mode}: invalid package default unexpectedly compiled"),
        };
        assert!(
            !messages.has_infrastructure_error(),
            "{entry_mode}: {messages:?}"
        );
        let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1, "{entry_mode}: {messages:?}");
        assert_eq!(
            diagnostics[0].kind.code(),
            "MOTH-SYNTAX-0008",
            "{entry_mode}: {messages:?}"
        );

        let rendered = render_compiler_messages_html(&messages, &project_root);
        assert!(
            rendered.contains(default_line),
            "{entry_mode}: the diagnostic must render its package-owned source excerpt: {rendered}"
        );
        let position = messages
            .diagnostic_render_context(0)
            .primary_position(diagnostics[0])
            .expect("invalid package default should retain its authored source position");
        assert_eq!(
            portable_path_text(&position.path),
            "@mod.moth",
            "{entry_mode}: diagnostic paths are relative to their owning package boundary"
        );
        assert_eq!(position.line, default_line, "{entry_mode}");
        assert_eq!(position.start.line, 0, "{entry_mode}");
        assert_eq!(position.start.column, 23, "{entry_mode}");
        assert_eq!(position.end.line, 0, "{entry_mode}");
        assert_eq!(position.end.column, 33, "{entry_mode}");
    }
}

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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
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
    let _path_fork = PathInternerFork::empty();
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
        NumericProfile::STANDARD,
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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
    let _path_fork = PathInternerFork::empty();
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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(src.join("@page.moth"), "@broken run\nvalue = run()\n")
        .expect("should write blocked project consumer");
    fs::write(
        package.join("@mod.moth"),
        "export:\n    run || -> Int:\n        return missing_package_value\n    ;\n;\n",
    )
    .expect("should write diagnosed source package");

    for entry in [dir.clone(), src.join("@page.moth")] {
        let mut config = Config::new(entry);
        config.entry_root = PathBuf::from("src");
        let style_directives = StyleDirectiveRegistry::built_ins();
        let mut string_table = StringTable::new();
        let _path_fork = PathInternerFork::empty();
        let mut frontend_surface = BuilderSurface::with_mandatory_core();
        frontend_surface.source_packages.register_filesystem_root(
            "broken",
            package.clone(),
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
        let rendered = render_compiler_messages_html(&messages, &dir);
        assert!(
            rendered.contains("return missing_package_value"),
            "the diagnosed package must retain its own source snapshot: {rendered}"
        );
    }

    #[cfg(feature = "boracle")]
    {
        let config = Config::new(src.join("@page.moth"));
        let mut string_table = StringTable::new();
        let mut frontend_surface = BuilderSurface::with_mandatory_core();
        frontend_surface.source_packages.register_filesystem_root(
            "broken",
            package,
            PackageOrigin::Builder,
        );
        let result = crate::build_system::create_project_modules::compile_single_file_boracle(
            &config,
            NumericProfile::STANDARD,
            &StyleDirectiveRegistry::built_ins(),
            &mut frontend_surface,
            &mut string_table,
        );
        let messages = match result {
            Err(messages) => messages,
            Ok(_) => panic!("Boracle consumer of a diagnosed package unexpectedly compiled"),
        };
        assert!(!messages.has_infrastructure_error(), "{messages:?}");
        let diagnostics = messages.error_diagnostics().collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1, "{messages:?}");
        assert_eq!(diagnostics[0].kind.code(), "MOTH-RULE-0034");
        let position = messages
            .diagnostic_render_context(0)
            .primary_position(diagnostics[0])
            .expect("Boracle must retain the failed package's authored source span");
        assert!(position.line.contains("return missing_package_value"));
        let rendered = render_compiler_messages_html(&messages, &dir);
        assert!(rendered.contains("return missing_package_value"));
    }
}
#[test]
fn same_module_generated_sidecars_rebuild_const_templates_in_their_fresh_store() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _temp = tempfile::tempdir().expect("should create temp dir");
    let dir = _temp.path().to_path_buf();

    fs::write(
        dir.join("config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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
    let _path_fork = PathInternerFork::empty();
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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
    value = 2
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
    let _path_fork = PathInternerFork::empty();
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
        // Four materialisation passes, then rechecks only for `mutating_helper` and `caller`,
        // whose direct callee summaries widen. `seed_helper` and `independent` keep their
        // materialisation reports because convergence installs no changed summary into them.
        assert_eq!(
            counter_value("convergence_generated_sidecar_borrow_passes"),
            6.0
        );
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

/// Assert a published module's link facts select exactly its final HIR CFG.
///
/// WHY: link facts are collected before convergence and refreshed only when private failure
/// lane installation rewrites the HIR, so a stale collection would select pre-install blocks.
#[track_caller]
fn assert_link_facts_match_final_cfg(
    module: &crate::compiler_frontend::module_compilation::Module,
) {
    let hir = &module.executable.hir;
    let all_functions = hir
        .functions
        .iter()
        .map(|function| function.id)
        .collect::<Vec<_>>();
    let reachability =
        collect_reachability_from_function_link_facts(&module.link_facts.functions, &all_functions)
            .expect("published link facts should cover every HIR function");
    reachability
        .backend_selection()
        .validate_for_hir(hir)
        .expect("published link facts should describe the final CFG");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaneFixture {
    InstallsLane,
    PrunesHandler,
    Unchanged,
}

#[test]
fn published_link_facts_follow_private_failure_lane_rewrites() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let generic_identity = "identity type T |value T| -> T:\n    return value\n;\n\n";
    for (fixture, page, support) in [
        // A fallible private chain gets a builtin-Error lane.
        (
            LaneFixture::InstallsLane,
            "multiply |left Int, right Int| -> Int:\n    return left * right\n;\n\n\
             safe_product |left Int, right Int| -> Int, Error!:\n    return multiply(left, right)\n;\n\n\
             product = safe_product(6, 7) catch then 0\n\
             same Int = identity(product)\n\
             [: product=[same]]\n",
            None,
        ),
        // U8 + U8 into U32 is proven safe, so only the generic call keeps the catch. That call's
        // failure stays provisional until materialisation proves `identity` infallible, which
        // leaves the handler without an error edge for the installer to prune. Post-install HIR
        // validation rejects an unpruned handler, and pruning compacts block ids, so stale link
        // facts would no longer match. A non-generic callee is proven before HIR lowering and
        // never reaches pruning.
        (
            LaneFixture::PrunesHandler,
            "left U8 = 255\n\
             right U8 = 255\n\
             same U32 = identity(left) + right catch then 0\n\
             [: same=[same]]\n",
            None,
        ),
        // Every page has an entry whose provisional error carrier is narrowed, so only an
        // API-only support module can leave installation unchanged: it has no entry to narrow,
        // no catch to record and no fallible private function to give a lane.
        (
            LaneFixture::Unchanged,
            "@utils pass_through\nsame Int = pass_through(42)\n[: same=[same]]\n",
            Some(
                "export:\n    pass_through |value Int| -> Int:\n        return identity(value)\n    ;\n;\n",
            ),
        ),
    ] {
        let _temp = tempfile::tempdir().expect("should create temp dir");
        let dir = _temp.path().to_path_buf();
        fs::create_dir_all(dir.join("src")).expect("should create source root");
        fs::write(
            dir.join("config.moth"),
            "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
        )
        .expect("should write config");
        let page = match support {
            Some(support) => {
                fs::create_dir_all(dir.join("src/utils")).expect("should create support root");
                fs::write(
                    dir.join("src/utils/+package.moth"),
                    format!("{generic_identity}{support}"),
                )
                .expect("should write support package");
                page.to_owned()
            }
            None => format!("{generic_identity}{page}"),
        };
        fs::write(dir.join("src/@page.moth"), page).expect("should write page");

        let mut config = Config::new(dir.clone());
        let style_directives = StyleDirectiveRegistry::built_ins();
        let mut string_table = StringTable::new();
        let frontend = compile_project_frontend(
            &mut config,
            BuildProfile::Dev,
            None,
            &style_directives,
            &mut BuilderSurface::with_mandatory_core(),
            &mut string_table,
        )
        .unwrap_or_else(|error| panic!("{fixture:?}: link-fact fixture should compile: {error:?}"));

        let mut has_api_only_module = false;
        for artefact in frontend.project.successful_artefacts_in_module_id_order() {
            let hir = &artefact.module.executable.hir;
            let has_failure_lane = hir.blocks.iter().any(|block| {
                block.statements.iter().any(|statement| {
                    matches!(
                        statement.kind,
                        HirStatementKind::NumericOp {
                            failure_mode: NumericFailureMode::ReturnError,
                            ..
                        }
                    )
                })
            });
            assert_eq!(
                has_failure_lane,
                fixture == LaneFixture::InstallsLane,
                "{fixture:?}"
            );
            assert_link_facts_match_final_cfg(&artefact.module);
            has_api_only_module |= hir.start_function.is_none();
        }
        assert_eq!(has_api_only_module, fixture == LaneFixture::Unchanged);

        let mut sidecar_count = 0;
        for sidecar in frontend.project.generated.sidecars() {
            assert_link_facts_match_final_cfg(&sidecar.module);
            sidecar_count += 1;
        }
        assert_eq!(
            sidecar_count, 1,
            "{fixture:?}: the identity request needs one sidecar"
        );
    }
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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
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
    let _path_fork = PathInternerFork::empty();
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
        "project #= (
    name = \"docs\",
    entry_root = \"src\",
)
html #= ()\n",
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
    let _path_fork = PathInternerFork::empty();
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
        NumericProfile::STANDARD,
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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
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
    let _path_fork = PathInternerFork::empty();
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
        NumericProfile::STANDARD,
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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
    let _path_fork = PathInternerFork::empty();
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
            crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity::GenericInstance {
                base,
                ..
            } => base.defining_name(),
            crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity::ModulePrivateGenericInstance {
                base,
                ..
            } => base.defining_path(),
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
                    sidecar
                        .module
                        .executable
                        .path_table
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
                    sidecar
                        .module
                        .executable
                        .path_table
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
                    sidecar
                        .module
                        .executable
                        .path_table
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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

    let imported_marker_path = frontend
        .project
        .successful_artefacts_in_module_id_order()
        .find_map(|artifact| {
            let path_table = &artifact.module.executable.path_table;
            let environment = &artifact.module.executable.type_environment;
            (0..path_table.len())
                .filter_map(PathId::try_from_index)
                .find(|path| {
                    let Some(nominal_id) = environment.nominal_id_for_path(path) else {
                        return false;
                    };
                    let Some(type_id) = environment.type_id_for_nominal_id(nominal_id) else {
                        return false;
                    };
                    display_type(type_id, environment, &string_table, path_table) == "RemoteMarker"
                })
        })
        .expect("requester path table should contain the imported marker path");
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
            sidecar
                .module
                .executable
                .path_table
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
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
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
    let _path_fork = PathInternerFork::empty();
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
fn implicit_failure_generated_private_helper_facts_survive_materialisation_and_convergence() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp_dir = tempfile::tempdir().expect("should create temporary project");
    let dir = temp_dir.path().to_path_buf();
    let src = dir.join("src");
    fs::create_dir_all(&src).expect("should create project entry root");
    fs::write(
        dir.join("config.moth"),
        "project #= (\n    name = \"implicit-failure\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    ).expect("should write config");
    fs::write(
        src.join("@page.moth"),
        "multiply |left Int, right Int| -> Int:\n    return left * right\n;\n\
         product type T |marker T, left Int, right Int| -> Int:\n\
             return multiply(left, right)\n;\n\
         safe_product type T |marker T, left Int, right Int| -> Int, Error!:\n\
             return multiply(left, right)\n;\n\
         use_product |left Int, right Int| -> Int:\n    return product(0, left, right)\n;\n\
         use_safe_product |left Int, right Int| -> Int, Error!:\n\
             return safe_product(0, left, right)!\n;\n\
         result = use_product(2, 3)\n",
    )
    .expect("should write private generic helper source");

    let mut config = Config::new(dir);
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
    .expect("real private generic helper contracts should compile");

    let sidecars = frontend.project.generated.sidecars().collect::<Vec<_>>();
    assert_eq!(
        sidecars.len(),
        2,
        "both authored concrete requests must materialise"
    );
    let mut inferred_roots = 0;
    let mut error_slot_roots = 0;
    for sidecar in sidecars {
        let executable = &sidecar.module.executable;
        let hir = &executable.hir;
        let root = *hir
            .function_ids_by_generated
            .get(&sidecar.identity)
            .expect("sidecar identity must resolve to its exact generated root");
        let facts = &hir.function_failure_facts[&root];
        assert_eq!(facts.contributors.len(), 1);
        let contributor = &facts.contributors[0];
        let HirBuiltinFailureSource::Call(CallTarget::ModulePrivate(helper)) = &contributor.source
        else {
            panic!(
                "materialised donor-private call must retain its failure contributor: {contributor:?}"
            );
        };
        assert!(contributor.span.is_some());
        assert!(
            hir.module_private_call_summaries[helper].escapes_builtin_failure,
            "the real multiply body, not an injected summary, must establish helper failure",
        );
        let summary = &executable.borrow_analysis.analysis.public_call_summaries[&root];
        match facts.boundary {
            HirBuiltinFailureBoundary::InferPrivate => {
                inferred_roots += 1;
                assert!(summary.escapes_builtin_failure);
            }
            HirBuiltinFailureBoundary::BuiltinErrorSlot => {
                error_slot_roots += 1;
                assert!(!summary.escapes_builtin_failure);
            }
            boundary => panic!("unexpected generated failure contract: {boundary:?}"),
        }
    }
    assert_eq!((inferred_roots, error_slot_roots), (1, 1));
}

#[test]
fn implicit_failure_later_generated_request_reuses_converged_private_helper_summary() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp_dir = tempfile::tempdir().expect("should create temporary project");
    let dir = temp_dir.path().to_path_buf();
    let src = dir.join("src");
    let helper = src.join("helper");
    fs::create_dir_all(&helper).expect("should create declaring module");
    fs::write(
        dir.join("config.moth"),
        "project #= (\n    name = \"implicit-failure\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(
        helper.join("@mod.moth"),
        r#"multiply |left Int, right Int| -> Int:
    return left * right
;

export:
    product type T |marker T, left Int, right Int| -> Int, Error!:
        return multiply(left, right)
    ;
;
"#,
    )
    .expect("should write uninstantiated public generic and private helper");
    fs::write(
        src.join("@page.moth"),
        "@helper product\n\
         use_product || -> Int, Error!:\n    return product(0, 2, 3)!\n;\n\
         result = use_product() catch then 0\n",
    )
    .expect("should write later generated requester");

    let mut config = Config::new(dir);
    config.entry_root = PathBuf::from("src");
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
    .expect("later public generic request should compile with the private failure contract");
    assert!(
        !frontend.has_diagnosed_or_blocked(),
        "both declaring module and later requester must compile without source diagnoses",
    );

    let declaring_artefact = frontend
        .project
        .successful_artefacts_in_module_id_order()
        .find(|artefact| {
            artefact.interface.declarations.iter().any(|declaration| {
                matches!(
                    &declaration.origin,
                    OriginDeclarationId::Function(origin) if origin.defining_name() == "product"
                )
            })
        })
        .expect("declaring module should publish product");
    assert_eq!(
        declaring_artefact.interface.declarations.len(),
        1,
        "multiply must remain private to the declaring module",
    );
    let declaration = &declaring_artefact.interface.declarations[0];
    let PublicDeclarationSemantics::Function(product) = &declaration.semantics else {
        panic!("product must retain its public function contract");
    };
    assert!(matches!(
        product.category,
        PublicFunctionCategory::GenericTemplate(_),
    ));
    assert_eq!(
        product.error_return,
        Some(CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error)),
        "product must keep its public Error! slot",
    );

    let context = declaring_artefact
        .module
        .metadata
        .materialisation_context
        .as_ref()
        .expect("uninstantiated product must publish its declaring materialisation context");
    let declaring_hir = &declaring_artefact.module.executable.hir;
    assert!(
        declaring_hir.function_ids_by_generated.is_empty(),
        "the declaring module must not instantiate product",
    );
    let multiply = declaring_hir
        .function_ids_by_private_origin
        .keys()
        .find(|identity| identity.defining_name() == "multiply")
        .expect("declaring module must retain multiply's private identity");
    assert!(
        context
            .private_callable_summary(multiply)
            .expect("declaring context must retain multiply's exact summary")
            .escapes_builtin_failure,
        "the published declaring context must replace multiply's signature-only bootstrap false",
    );
    let mut sidecars = frontend.project.generated.sidecars();
    let sidecar = sidecars
        .next()
        .expect("the page request should materialise product");
    assert!(
        sidecars.next().is_none(),
        "only product should be generated"
    );
    let executable = &sidecar.module.executable;
    let hir = &executable.hir;
    let root = *hir
        .function_ids_by_generated
        .get(&sidecar.identity)
        .expect("product sidecar must resolve to its exact generated root");
    let facts = &hir.function_failure_facts[&root];
    assert_eq!(facts.contributors.len(), 1);
    let contributor = &facts.contributors[0];
    let HirBuiltinFailureSource::Call(CallTarget::ModulePrivate(helper)) = &contributor.source
    else {
        panic!("later product request must retain its private-call contributor: {contributor:?}");
    };
    assert_eq!(
        helper, multiply,
        "the sidecar must call the declaring private multiply"
    );
    assert!(
        hir.module_private_call_summaries[multiply].escapes_builtin_failure,
        "the later page request must consume the converged private summary, not bootstrap false",
    );
    assert_eq!(facts.boundary, HirBuiltinFailureBoundary::BuiltinErrorSlot);
    assert!(
        !executable.borrow_analysis.analysis.public_call_summaries[&root].escapes_builtin_failure,
        "product must consume private failure through Error! without widening its public effect",
    );
}

// A later generated request cannot leak escaping implicit failure through a missing or custom
// error slot. Dormant validation of the declaring template rejects it before publication, so the
// requesting transaction never materialises the instantiation.
fn compile_later_request_rejection(project_dir: &std::path::Path) -> ProjectFrontendCompilation {
    let mut config = Config::new(project_dir.to_path_buf());
    config.entry_root = PathBuf::from("src");
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut string_table = StringTable::new();
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("a diagnosed declaring module stays a retained frontend outcome")
}

fn assert_later_request_rejected(declaring_source: &str, requester_source: &str, reason_key: &str) {
    let temp_dir = tempfile::tempdir().expect("should create temporary project");
    let src = temp_dir.path().join("src");
    fs::create_dir_all(src.join("helper")).expect("should create declaring module");
    fs::write(
        temp_dir.path().join("config.moth"),
        "project #= (\n    name = \"implicit-failure\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(src.join("helper").join("@mod.moth"), declaring_source)
        .expect("should write declaring module");
    fs::write(src.join("@page.moth"), requester_source).expect("should write requester");

    let frontend = compile_later_request_rejection(temp_dir.path());
    assert!(
        frontend.project.generated.sidecars().next().is_none(),
        "no escaping later request may materialise",
    );
    assert_eq!(
        frontend.project.diagnosed.len(),
        1,
        "the declaring module owns the rejection"
    );
    assert_eq!(
        frontend.project.blocked.len(),
        1,
        "the requester stays blocked on its provider"
    );

    let diagnostics = frontend.project.diagnosed[0].diagnostics.diagnostics();
    assert_eq!(
        diagnostics.len(),
        1,
        "one escaping failure is reported once"
    );
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.identity().reason_key, Some(reason_key));
    let DiagnosticPayload::InvalidFallibleHandling {
        reason:
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { witness }
            | InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                witness, ..
            },
    } = &diagnostic.payload
    else {
        panic!("later request rejection must carry a failure witness: {diagnostic:?}");
    };
    // The witness names the declaring private hop, then the originating multiplication.
    assert_eq!(witness.codes, vec![BuiltinErrorCode::IntOverflow]);
    assert_eq!(witness.call_spans.len(), 1);
    assert_eq!(witness.elided_call_hops, 0);
    assert!(witness.call_spans[0].is_some() && witness.origin_span.is_some());
    let labels = diagnostic
        .labels
        .iter()
        .map(|label| (label.span, label.message.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        vec![
            (
                witness.call_spans[0],
                Some(DiagnosticLabelMessage::BuiltinFailureCall)
            ),
            (
                witness.origin_span,
                Some(DiagnosticLabelMessage::BuiltinFailureOrigin)
            ),
        ],
    );

    // Recompiling the same project reports the identical structured rejection.
    let repeat = compile_later_request_rejection(temp_dir.path());
    let repeated = &repeat.project.diagnosed[0].diagnostics.diagnostics()[0];
    assert_eq!(repeated.identity(), diagnostic.identity());
    assert_eq!(repeated.payload, diagnostic.payload);
}

#[test]
fn implicit_failure_later_generated_request_rejects_exported_function_without_error_slot() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    assert_later_request_rejected(
        "multiply |left Int, right Int| -> Int:\n    return left * right\n;\n\n\
         export:\n    product type T |marker T, left Int, right Int| -> Int:\n        \
         return multiply(left, right)\n    ;\n;\n",
        "@helper product\nresult = product(0, 2, 3)\n",
        "invalid_fallible_handling.unhandled_builtin_failure_in_exported_function",
    );
}

#[test]
fn implicit_failure_later_generated_request_rejects_custom_error_function() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    assert_later_request_rejected(
        "multiply |left Int, right Int| -> Int:\n    return left * right\n;\n\n\
         export:\n    Failure = |\n        message String,\n    |\n\n    \
         product type T |marker T, left Int, right Int| -> Int, Failure!:\n        \
         return multiply(left, right)\n    ;\n;\n",
        "@helper product\nresult = product(0, 2, 3) catch then 0\n",
        "invalid_fallible_handling.unhandled_builtin_failure_in_custom_error_function",
    );
}

// M02: a private helper that changes from infallible to fallible changes the
// exported caller's verdict even though every public signature text stays
// identical. The first compilation publishes a success-only export whose helper
// cannot fail; rewriting the helper body to a possibly overflowing op rejects
// the same exported caller. Each compilation here is fresh (no cross-build
// artefact reuse exists yet), so this proves the verdict follows the helper's
// current semantic failure summary rather than the unchanged public text — not
// cache invalidation.
#[test]
fn implicit_failure_private_helper_infallible_to_fallible_changes_exported_caller_verdict() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp_dir = tempfile::tempdir().expect("should create temporary project");
    let src = temp_dir.path().join("src");
    fs::create_dir_all(src.join("helper")).expect("should create declaring module");
    fs::write(
        temp_dir.path().join("config.moth"),
        "project #= (\n    name = \"implicit-failure\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    // The export block is byte-identical in both versions: only the private helper
    // body changes. Every public signature text stays the same; only the private
    // helper's semantic failure summary widens from infallible to fallible.
    const EXPORT_BLOCK: &str = "export:\n    product type T |marker T, left Int, right Int| -> Int:\n        return double(left)\n    ;\n;\n";
    let infallible_source =
        format!("double |value Int| -> Int:\n    return value\n;\n\n{EXPORT_BLOCK}");
    let fallible_source =
        format!("double |value Int| -> Int:\n    return value * value\n;\n\n{EXPORT_BLOCK}");
    assert_ne!(infallible_source, fallible_source);
    assert_eq!(
        infallible_source.len() + " * value".len(),
        fallible_source.len(),
        "only the helper body gains the overflowing op"
    );
    fs::write(src.join("helper").join("@mod.moth"), &infallible_source)
        .expect("should write infallible declaring module");
    fs::write(
        src.join("@page.moth"),
        "@helper product\n\
         use_product || -> Int:\n    return product(0, 2, 3)\n;\n\
         result = use_product()\n",
    )
    .expect("should write requester");

    let first = compile_later_request_rejection(temp_dir.path());
    assert!(
        first.project.generated.sidecars().next().is_some(),
        "the infallible helper version must materialise the later request"
    );
    assert!(
        first.project.diagnosed.is_empty(),
        "the infallible helper version must publish without rejection"
    );
    let declaring_artefact = first
        .project
        .successful_artefacts_in_module_id_order()
        .find(|artefact| {
            artefact.interface.declarations.iter().any(|declaration| {
                matches!(
                    &declaration.origin,
                    OriginDeclarationId::Function(origin) if origin.defining_name() == "product"
                )
            })
        })
        .expect("declaring module should publish product");
    let context = declaring_artefact
        .module
        .metadata
        .materialisation_context
        .as_ref()
        .expect("uninstantiated product must publish its declaring materialisation context");
    let helper_identity = declaring_artefact
        .module
        .executable
        .hir
        .function_ids_by_private_origin
        .keys()
        .find(|identity| identity.defining_name() == "double")
        .expect("declaring module must retain the helper private identity")
        .clone();
    assert!(
        !context
            .private_callable_summary(&helper_identity)
            .expect("declaring context must retain the helper summary")
            .escapes_builtin_failure,
        "the published caller contract must record the helper as infallible"
    );

    // Version two: identical public signature text, but the private helper now
    // multiplies, gaining a possibly overflowing op.
    fs::write(src.join("helper").join("@mod.moth"), &fallible_source)
        .expect("should write fallible declaring module");

    let second = compile_later_request_rejection(temp_dir.path());
    assert!(
        second.project.generated.sidecars().next().is_none(),
        "no escaping later request may materialise once the helper can fail"
    );
    assert_eq!(
        second.project.diagnosed.len(),
        1,
        "the declaring module owns the rejection after the helper widened"
    );
    assert_eq!(
        second.project.blocked.len(),
        1,
        "the requester stays blocked on its provider"
    );
    let diagnostics = second.project.diagnosed[0].diagnostics.diagnostics();
    assert_eq!(
        diagnostics.len(),
        1,
        "one escaping failure is reported once"
    );
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.identity().reason_key,
        Some("invalid_fallible_handling.unhandled_builtin_failure_in_exported_function")
    );
    let DiagnosticPayload::InvalidFallibleHandling {
        reason: InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { witness },
    } = &diagnostic.payload
    else {
        panic!("widened helper rejection must carry an export witness: {diagnostic:?}");
    };
    assert_eq!(witness.codes, vec![BuiltinErrorCode::IntOverflow]);
    assert!(witness.origin_span.is_some());
    // Recompiling the widened project reports the identical structured rejection,
    // so the verdict is repeatable and not a one-off ordering artefact.
    let repeat = compile_later_request_rejection(temp_dir.path());
    let repeated = &repeat.project.diagnosed[0].diagnostics.diagnostics()[0];
    assert_eq!(repeated.identity(), diagnostic.identity());
    assert_eq!(repeated.payload, diagnostic.payload);
}

// D02: the bounded-witness export source must render the identical terse diagnostic
// across repeated real-pipeline compilations, even when independent declarations
// sharing the helper module are textually permuted. The chain hops and the numeric
// origin stay byte-identical in place (their spans participate in the witness), so
// only the unrelated declarations move; their spans never enter the witness.
// There is no worker-count knob to vary: semantic module jobs stay serial (only
// file preparation parallelises inside a job), so repetition plus textual
// permutation is the available determinism probe.
#[test]
fn exported_private_chain_witness_rendering_is_stable_across_repeated_compilation() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp_dir = tempfile::tempdir().expect("should create temporary project");
    let src = temp_dir.path().join("src");
    fs::create_dir_all(&src).expect("should create entry root");
    fs::write(
        temp_dir.path().join("config.moth"),
        "project #= (\n    name = \"implicit-failure\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    const CHAIN_TAIL: &str = "multiply |left Int, right Int| -> Int:\n    return left * right\n;\n\
         first |left Int, right Int| -> Int:\n    return multiply(left, right)\n;\n\
         second |left Int, right Int| -> Int:\n    return first(left, right)\n;\n\
         third |left Int, right Int| -> Int:\n    return second(left, right)\n;\n\
         fourth |left Int, right Int| -> Int:\n    return third(left, right)\n;\n";
    const UNRELATED_A: &str = "unrelated_a || -> Int:\n    return 1\n;\n";
    const UNRELATED_B: &str = "unrelated_b || -> Int:\n    return 2\n;\n";
    fn write_chain(src: &std::path::Path, unrelated_first: &str, unrelated_second: &str) {
        fs::write(
            src.join("chain.moth"),
            format!("{unrelated_first}\n{unrelated_second}\n{CHAIN_TAIL}"),
        )
        .expect("should write chain helpers");
    }
    write_chain(&src, UNRELATED_A, UNRELATED_B);
    fs::write(
        src.join("@page.moth"),
        "@chain fourth\n\
         export:\n    product |left Int, right Int| -> Int:\n        return fourth(left, right)\n    ;\n;\n\
         result = product(2, 3)\n",
    )
    .expect("should write entry");
    fn terse_error_lines(project_dir: &std::path::Path) -> Vec<String> {
        let frontend = compile_later_request_rejection(project_dir);
        assert!(
            frontend.project.generated.sidecars().next().is_none(),
            "the escaping export must not materialise"
        );
        let project_source = frontend.project_source_database.clone();
        let mut string_table = StringTable::new();
        let messages = frontend
            .into_render_messages_with_frozen_identity(&mut string_table, project_source, None)
            .expect("the diagnosed project must produce render messages");
        messages
            .diagnostics()
            .enumerate()
            .map(|(index, diagnostic)| {
                terse::format_terse_diagnostic_with_context(
                    diagnostic,
                    messages.diagnostic_render_context(index),
                )
            })
            .collect()
    }
    let first = terse_error_lines(temp_dir.path());
    assert_eq!(
        first.len(),
        1,
        "the bounded chain must report exactly one error"
    );
    assert!(
        first[0].starts_with("E|MOTH-RULE-0051|@page.moth|"),
        "the witness rejection must render its code and signature position: {}",
        first[0]
    );
    assert!(
        first[0].ends_with("Witness omits 2 additional private call hop(s)."),
        "the bounded witness must render its elided-hop count: {}",
        first[0]
    );
    assert_eq!(
        terse_error_lines(temp_dir.path()),
        first,
        "recompiling the same source must render byte-identical diagnostics"
    );
    write_chain(&src, UNRELATED_B, UNRELATED_A);
    let permuted = terse_error_lines(temp_dir.path());
    assert_eq!(
        permuted, first,
        "permuting unrelated declarations must not move the rendered witness"
    );
    assert_eq!(
        terse_error_lines(temp_dir.path()),
        first,
        "the permuted source must itself render deterministically"
    );
    // The structured witness behind the rendering carries the exact bounded
    // content: one IntOverflow code, three kept call spans plus the origin, and
    // two elided middle hops.
    let frontend = compile_later_request_rejection(temp_dir.path());
    let diagnostics = frontend.project.diagnosed[0].diagnostics.diagnostics();
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.identity().reason_key,
        Some("invalid_fallible_handling.unhandled_builtin_failure_in_exported_function")
    );
    let DiagnosticPayload::InvalidFallibleHandling {
        reason: InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { witness },
    } = &diagnostic.payload
    else {
        panic!("the bounded chain rejection must carry an export witness: {diagnostic:?}");
    };
    assert_eq!(witness.codes, vec![BuiltinErrorCode::IntOverflow]);
    assert_eq!(witness.call_spans.len(), 3);
    assert_eq!(witness.elided_call_hops, 2);
    assert!(witness.origin_span.is_some());
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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
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
    let _path_fork = PathInternerFork::empty();
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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n)\nhtml #= ()\n",
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
    let _path_fork = PathInternerFork::empty();
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
        NumericProfile::STANDARD,
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
