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
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages, ErrorType};
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terse};
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
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
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableProviderResourceOwnerId, StableResourceOriginId,
    StableResourceOwnerId,
};
use crate::compiler_frontend::public_call_summary::PublicCallMutationEffect;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_tests::test_diagnostics::assert_exact_infrastructure_error;
use crate::projects::settings::Config;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "timers")]
fn boundary_has_timing(
    snapshot: &crate::timing::BenchmarkObservationSnapshot,
    boundary: crate::timing::TimingBoundaryId,
    metric: crate::timing::TimingMetric,
) -> bool {
    snapshot
        .boundaries
        .iter()
        .find(|record| record.id == boundary)
        .is_some_and(|record| {
            record
                .timings
                .iter()
                .any(|aggregate| aggregate.metric == metric && aggregate.samples > 0)
        })
}

#[cfg(feature = "timers")]
fn module_has_timing(
    module: &crate::timing::TimingModuleRecord,
    metric: crate::timing::TimingMetric,
) -> bool {
    module
        .timings
        .iter()
        .any(|aggregate| aggregate.metric == metric && aggregate.samples > 0)
}

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
    let frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut string_table,
    )
    .expect("diagnosed modules are retained in the typed frontend outcome");
    let messages = frontend.into_render_messages(&mut string_table);

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

    let frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("diagnosed source packages are retained in the typed frontend outcome");
    let messages = frontend.into_render_messages(&mut string_table);

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
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "broken",
        package,
        PackageOrigin::Builder,
    );

    let frontend = compile_project_frontend(
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

    let messages = frontend.into_render_messages(&mut string_table);
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
                assert_eq!(fields[0].name.name_str(&string_table), Some("value"));
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
                assert_eq!(fields[0].name.name_str(&string_table), Some("value"));
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
                assert_eq!(fields[0].name.name_str(&string_table), Some("value"));
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

    let imported_marker_path = InternedPath::from_single_str("provider", &mut string_table)
        .join_str("RemoteMarker", &mut string_table);
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
            display_type(marker_type_id, environment, &string_table),
            "RemoteMarker"
        );

        let fields = environment
            .fields_for(marker_type_id)
            .expect("sidecar should retain inherited Marker fields");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name.name_str(&string_table), Some("value"));
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
        authored_import_location: SourceLocation::default(),
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
        DiagnosticRenderContext::new(&messages.string_table),
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

    let frontend = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut string_table,
    )
    .expect("diagnosed modules are retained in the typed frontend outcome");
    let messages = frontend.into_render_messages(&mut string_table);

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
    let services = crate::build_system::project_config::ProjectConfigParseServices {
        style_directives: &style_directives,
        frontend_surface: &frontend_surface,
    };
    let parse_result = crate::build_system::project_config::compile_project_config_file(
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

#[test]
fn diagnosed_provider_retains_independent_successful_module() {
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
    fs::write(dir.join("independent/@mod.moth"), "value = 7\n")
        .expect("should write independent successful module");

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
    .expect("typed outcome retains successes beside diagnostics");

    assert_eq!(
        frontend.project.diagnosed.len(),
        1,
        "provider diagnostic should appear once"
    );
    assert_eq!(
        frontend.project.blocked.len(),
        2,
        "both consumers of the diagnosed provider should be blocked"
    );
    let successful_paths = frontend
        .successful_module_views()
        .map(|module| module.metadata.entry_point.clone())
        .collect::<Vec<_>>();
    assert!(
        successful_paths
            .iter()
            .any(|path| path.ends_with("independent/@mod.moth")),
        "independent successful module should be retained: {successful_paths:?}"
    );

    let messages = frontend.into_render_messages(&mut string_table);
    assert_eq!(
        messages.error_count(),
        1,
        "provider diagnostic should be rendered once"
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
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface.source_packages.register_filesystem_root(
        "warnpkg",
        package,
        PackageOrigin::Builder,
    );
    let frontend = compile_project_frontend(
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

    let messages = frontend.into_render_messages(&mut string_table);
    assert!(
        messages.warning_count() >= 1,
        "render boundary should retain the source-package warning"
    );
}
