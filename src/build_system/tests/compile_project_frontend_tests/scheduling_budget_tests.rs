//! Project scheduling-cutoff and shared-allowance regression coverage.
//!
//! WHAT: drives the real directory frontend lane with real `config.moth` values, then inspects the
//!       retained graph boundary before the terminal wrapper converts it into reports.
//! WHY: the project threshold is a command-owned soft cutoff, not a result cap. These tests prove
//!       returned module diagnostics stay intact, unadmitted modules keep `Unavailable` slots, the
//!       cutoff flag is command scope, check-only units share their owner's module allowance,
//!       package and deferred package work stop at the same threshold, and a diagnosed module
//!       publishes neither an artefact nor a semantic resource association.

use super::*;
use crate::build_system::create_project_modules::compiled_boundary::{
    BlockedProvider, CompiledSourcePackage, ProjectFrontendCompilation,
};
use crate::build_system::create_project_modules::module_artifact_store::ProviderSlot;
use crate::build_system::create_project_modules::module_identity::ModuleId;
use crate::build_system::project_config::{ProjectConfigParseServices, load_project_config};
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::compiler_messages::DiagnosticDiscoveryStops;
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::compiler_messages::render::terse;
use crate::compiler_frontend::source::{SourceDatabase, SourceDatabaseBuilder};
use crate::compiler_frontend::symbols::string_interning::StringTable;

const UNKNOWN_NAME_CODE: &str = "MOTH-RULE-0034";
const THREE_FAILING_BODIES: &str = "first || -> I32:\n    return missing_one\n;\n\n\
     second || -> I32:\n    return missing_two\n;\n\n\
     third || -> I32:\n    return missing_three\n;\n";
const ONE_FAILING_BODY: &str = "consumer || -> I32:\n    return only_second_module\n;\n";
/// The project threshold every package cutoff fixture in this file reaches.
const PROJECT_ERROR_LIMIT_TWO: &str =
    "project #= (\n    name = \"docs\",\n        max_errors_per_project = 2,\n)\nhtml #= ()\n";
/// A module root whose body binds a resolved resource before failing, plus an independent body
/// in an owned sibling file with the same shape.
const RESOURCE_FAILING_MODULE_ROOT: &str = "@extra later\nexport:\n    probe || -> String:\n        logo = @assets/broken.svg\n        return missing_one\n    ;\n;\n";
const RESOURCE_FAILING_MODULE_EXTRA: &str =
    "later || -> String:\n    other = @assets/broken.svg\n    return missing_two\n;\n";
/// The same two bodies repaired to return their resource bindings.
const RESOURCE_REPAIRED_MODULE_ROOT: &str = "@extra later\nexport:\n    probe || -> String:\n        logo = @assets/broken.svg\n        return logo\n    ;\n;\n";
const RESOURCE_REPAIRED_MODULE_EXTRA: &str =
    "later || -> String:\n    other = @assets/broken.svg\n    return other\n;\n";

/// Run the real directory frontend lane and keep the retained boundary for inspection.
///
/// WHY: `compile_project_frontend` consumes a diagnosed boundary into reports, but these tests
///      must also observe slot outcomes and the boundary cutoff record.
fn run_directory_lane(
    root: &Path,
    mode: FrontendCompilationMode,
) -> (
    ProjectFrontendCompilation,
    BoundaryAccumulator,
    Option<Arc<SourceDatabase>>,
) {
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    run_directory_lane_with_surface(root, mode, &mut frontend_surface)
}

/// Run the same lane against a builder's own surface, such as its external package providers.
fn run_directory_lane_with_surface(
    root: &Path,
    mode: FrontendCompilationMode,
    frontend_surface: &mut BuilderSurface,
) -> (
    ProjectFrontendCompilation,
    BoundaryAccumulator,
    Option<Arc<SourceDatabase>>,
) {
    let mut config = Config::new(root.to_path_buf());
    let style_directives = StyleDirectiveRegistry::built_ins();
    let build_config_inputs = BuildConfigInputSet::new();
    let services = ProjectConfigParseServices {
        style_directives: &style_directives,
        frontend_surface,
        build_config_inputs: &build_config_inputs,
    };
    let mut string_table = StringTable::new();
    let mut source_builder = Some(SourceDatabaseBuilder::new(SourceDatabase::empty()));
    let validated_output_settings = load_project_config(
        &mut config,
        &services,
        &mut source_builder,
        &mut string_table,
    )
    .unwrap_or_else(|_| panic!("project config should load"));
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(string_table);
    let mut project_source_files = None;
    let lane_result = {
        let (strings, mut warning_sink) = warning_owner.compiler_parts_mut();
        super::super::compilation::compile_directory_frontend_in_premerge_lane(
            &config,
            FrontendBuildProfile::Dev,
            validated_output_settings.as_ref(),
            &style_directives,
            frontend_surface,
            strings,
            &mut warning_sink,
            &mut project_source_files,
            &mut source_builder,
            &build_config_inputs,
            mode,
        )
    };
    let frontend = match lane_result {
        Ok(frontend) => frontend,
        Err(failure) => {
            let reports = super::super::compilation::premerge_failure_to_reports(
                failure,
                &mut warning_owner,
                project_source_files.take(),
            )
            .expect("a premerge failure should convert without infrastructure errors");
            panic!(
                "the directory lane should retain a boundary for diagnosis: {:?}",
                terse::format_terse_diagnostic_reports(&reports)
            );
        }
    };
    (frontend, warning_owner, project_source_files)
}

/// Run the directory lane with `shared` registered at its real package root.
///
/// WHY: package work only enters the lane through the builder surface, and every package
///      scheduling regression needs the same real-root registration.
fn run_shared_package_lane(
    root: &Path,
    mode: FrontendCompilationMode,
) -> (
    ProjectFrontendCompilation,
    BoundaryAccumulator,
    Option<Arc<SourceDatabase>>,
) {
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root(
            "shared",
            root.join("packages/shared"),
            PackageOrigin::ProjectLocal,
        )
        .expect("the shared package root registers");
    run_directory_lane_with_surface(root, mode, &mut frontend_surface)
}

/// Convert one retained diagnosed boundary into its frozen reports.
fn finish_diagnosed(
    frontend: ProjectFrontendCompilation,
    warning_owner: &mut BoundaryAccumulator,
    project_source_files: Option<Arc<SourceDatabase>>,
) -> DiagnosticReportSet {
    match frontend
        .into_compilation_result(warning_owner, project_source_files, None)
        .expect("a diagnosed request still freezes its reports")
    {
        CompilationOutcome::Diagnosed(reports) => reports,
        CompilationOutcome::Success { .. } => panic!("expected a diagnosed frontend result"),
    }
}

/// Resolve the graph slot for one exact logical module path.
fn module_id_with_path(frontend: &ProjectFrontendCompilation, path: &str) -> ModuleId {
    frontend
        .project
        .structure
        .nodes()
        .iter()
        .find(|node| node.stable_origin().logical_module_path() == path)
        .unwrap_or_else(|| panic!("expected a module graph node with path {path:?}"))
        .module_id()
}

/// Resolve the graph slot for one module directory path suffix.
fn module_id_for_path(frontend: &ProjectFrontendCompilation, suffix: &str) -> ModuleId {
    frontend
        .project
        .structure
        .nodes()
        .iter()
        .find(|node| node.stable_origin().logical_module_path().ends_with(suffix))
        .unwrap_or_else(|| panic!("expected a module graph node ending with {suffix}"))
        .module_id()
}

fn assert_slot_is_successful(frontend: &ProjectFrontendCompilation, suffix: &str) {
    assert!(
        matches!(
            frontend
                .project
                .modules
                .slot(module_id_for_path(frontend, suffix))
                .expect("module slot should resolve"),
            ProviderSlot::Successful(_)
        ),
        "{suffix} should reach a complete artefact"
    );
}

/// Resolve the package-local graph slot for one module directory path suffix.
fn package_module_id(package: &CompiledSourcePackage, suffix: &str) -> ModuleId {
    package
        .boundary
        .structure
        .nodes()
        .iter()
        .find(|node| node.stable_origin().logical_module_path().ends_with(suffix))
        .unwrap_or_else(|| {
            panic!(
                "expected a @{} module graph node ending with {suffix:?}",
                package.package_prefix()
            )
        })
        .module_id()
}

/// Assert the shared package-and-project cutoff tail, then freeze the reports.
///
/// WHY: every request stopped at the project threshold leaves identical command-scope evidence on
///      both retained boundaries while each test keeps its own slot-level assertions inline.
fn finish_stopped_package_request(
    frontend: ProjectFrontendCompilation,
    warning_owner: &mut BoundaryAccumulator,
    project_source_files: Option<Arc<SourceDatabase>>,
    expected_primary_errors: u32,
) -> DiagnosticReportSet {
    let package = frontend
        .source_packages
        .iter()
        .next()
        .expect("a stopped package request retains its boundary");
    assert_eq!(
        package
            .boundary
            .scheduling_cutoff
            .map(|cutoff| cutoff.returned_primary_errors()),
        Some(expected_primary_errors),
        "the package boundary must retain the canonical cutoff proof"
    );
    assert!(
        package.boundary.blocked.is_empty(),
        "a refused admission must not invent a blocked provider"
    );
    assert_eq!(
        frontend
            .project
            .scheduling_cutoff
            .map(|cutoff| cutoff.returned_primary_errors()),
        Some(expected_primary_errors),
        "the project boundary must record the same spent threshold"
    );
    assert!(
        frontend.project.diagnosed.is_empty(),
        "the project boundary must not invent a diagnosis"
    );
    assert!(
        frontend.project.blocked.is_empty(),
        "an exhausted budget refuses admission instead of blocking"
    );
    assert!(
        warning_owner
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::PROJECT_SCHEDULING_CUTOFF),
        "the cutoff flag arrives through the command warning sink"
    );
    let stop_line = warning_owner
        .discovery_stops()
        .stop_line()
        .expect("a stopped request renders one stop line");
    assert!(stop_line.contains("project scheduling cutoff"));
    assert!(
        !stop_line.contains('\n'),
        "one cutoff must render exactly one line: {stop_line:?}"
    );

    // The success-only gate used by build assembly must reject the retained project boundary and
    // every retained package before the reports are frozen.
    let rejection = frontend
        .project
        .require_all_successful()
        .expect_err("a stopped project boundary is never linkable");
    assert!(
        rejection.message.contains("project scheduling cutoff"),
        "unexpected success-only rejection: {rejection:?}"
    );
    for package in frontend.source_packages.iter() {
        let rejection = package
            .boundary
            .require_all_successful()
            .expect_err("a stopped package boundary is never linkable");
        assert!(
            rejection.message.contains("project scheduling cutoff"),
            "unexpected package success-only rejection: {rejection:?}"
        );
    }

    let reports = finish_diagnosed(frontend, warning_owner, project_source_files);
    assert_eq!(reports.counts().errors, expected_primary_errors);
    reports
}

fn write_project(root: &Path, config_text: &str, files: &[(&str, &str)]) {
    fs::write(root.join("config.moth"), config_text).expect("config should be written");
    for (relative_path, contents) in files {
        let relative_path = relative_path.strip_prefix("src/").unwrap_or(relative_path);
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().expect("each fixture path has a parent"))
            .expect("fixture directory should be created");
        fs::write(path, contents).expect("fixture source should be written");
    }
}

#[test]
fn diagnostic_recovery_project_soft_cutoff() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    write_project(
        root,
        "project #= (\n    name = \"docs\",\n        max_errors_per_project = 2,\n)\nhtml #= ()\n",
        &[
            ("src/first/@entry.moth", THREE_FAILING_BODIES),
            ("src/second/@entry.moth", ONE_FAILING_BODY),
        ],
    );

    let (frontend, mut warning_owner, project_source_files) =
        run_directory_lane(root, FrontendCompilationMode::Canonical);
    let second_module = module_id_for_path(&frontend, "second");

    // The admitted module keeps its whole module-bounded batch even though it overshoots the
    // threshold; the refused module was never compiled and keeps its untouched slot.
    assert_eq!(
        frontend
            .project
            .modules
            .slot(module_id_for_path(&frontend, "first"))
            .expect("module slot should resolve"),
        ProviderSlot::Diagnosed
    );
    assert_eq!(
        frontend
            .project
            .modules
            .slot(second_module)
            .expect("module slot should resolve"),
        ProviderSlot::Unavailable
    );
    assert_eq!(frontend.project.diagnosed.len(), 1);
    assert_eq!(
        frontend.project.diagnosed[0]
            .diagnostics
            .primary_error_count(),
        3,
        "all three returned module errors must remain retained"
    );
    assert!(
        frontend
            .project
            .modules
            .normal_interface(second_module)
            .expect("slot lookup should resolve")
            .is_none(),
        "an unvisited slot can never satisfy a provider"
    );
    let cutoff = frontend
        .project
        .scheduling_cutoff
        .expect("the boundary must record the refused admission");
    assert_eq!(cutoff.returned_primary_errors(), 3);
    // The command flag has one owner: `refuse_scheduling_admission` merges it through the warning
    // sink at refusal time. Report retention must never re-derive it from boundary records.
    assert!(
        warning_owner
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::PROJECT_SCHEDULING_CUTOFF),
        "the cutoff flag must arrive through the warning sink before report freezing"
    );

    let reports = finish_diagnosed(frontend, &mut warning_owner, project_source_files);
    assert_eq!(reports.counts().errors, 3);
    assert_eq!(reports.counts().warnings, 0);
    assert!(
        reports
            .iter_severity(DiagnosticSeverity::Error)
            .all(|(_, view)| view.descriptor().code == UNKNOWN_NAME_CODE),
        "every retained error must keep its typed code"
    );
    let rendered = terse::format_terse_diagnostic_reports(&reports);
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("first/@entry.moth")),
        "the admitted module's diagnostics must stay rendered: {rendered:?}"
    );
    assert!(
        rendered.iter().all(
            |line| !line.contains("second/@entry.moth") && !line.contains("only_second_module")
        ),
        "the refused module must not contribute diagnostics: {rendered:?}"
    );

    // The cutoff flag is command scope: it never rides on a module batch.
    let stops = warning_owner.discovery_stops();
    assert!(stops.contains(DiagnosticDiscoveryStops::PROJECT_SCHEDULING_CUTOFF));
    assert!(
        stops
            .stop_line()
            .expect("a stopped request renders a stop line")
            .contains("project scheduling cutoff")
    );
}

#[test]
fn diagnostic_recovery_project_soft_cutoff_none_compiles_every_module() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    write_project(
        root,
        "project_error_limit #I32? = none\nproject #= (\n    name = \"docs\",\n        max_errors_per_project = project_error_limit,\n)\nhtml #= ()\n",
        &[
            ("src/first/@entry.moth", THREE_FAILING_BODIES),
            ("src/second/@entry.moth", ONE_FAILING_BODY),
        ],
    );

    let (frontend, mut warning_owner, project_source_files) =
        run_directory_lane(root, FrontendCompilationMode::Canonical);
    assert!(frontend.project.scheduling_cutoff.is_none());
    assert_eq!(frontend.project.diagnosed.len(), 2);
    assert_eq!(
        frontend
            .project
            .modules
            .slot(module_id_for_path(&frontend, "second"))
            .expect("module slot should resolve"),
        ProviderSlot::Diagnosed
    );

    let reports = finish_diagnosed(frontend, &mut warning_owner, project_source_files);
    assert_eq!(reports.counts().errors, 4);
    assert!(
        !warning_owner
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::PROJECT_SCHEDULING_CUTOFF)
    );
}

#[test]
fn diagnostic_recovery_project_soft_cutoff_on_last_module_records_no_stop() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    // `first` succeeds and compiles first (ModuleId order); `second` is the last admitted job and
    // meets the threshold exactly, so no further work was skipped.
    write_project(
        root,
        "project #= (\n    name = \"docs\",\n        max_errors_per_project = 3,\n)\nhtml #= ()\n",
        &[
            ("src/@entry.moth", "#[:site]\n"),
            (
                "src/first/@entry.moth",
                "answer || -> I32:\n    return 1\n;\n",
            ),
            ("src/second/@entry.moth", THREE_FAILING_BODIES),
        ],
    );

    let (frontend, mut warning_owner, project_source_files) =
        run_directory_lane(root, FrontendCompilationMode::Canonical);
    assert!(
        frontend.project.scheduling_cutoff.is_none(),
        "reaching the threshold on the last module claims no skipped work"
    );
    assert_slot_is_successful(&frontend, "first");
    assert_eq!(
        frontend
            .project
            .modules
            .slot(module_id_for_path(&frontend, "second"))
            .expect("module slot should resolve"),
        ProviderSlot::Diagnosed
    );

    let reports = finish_diagnosed(frontend, &mut warning_owner, project_source_files);
    assert_eq!(reports.counts().errors, 3);
    assert!(
        !warning_owner
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::PROJECT_SCHEDULING_CUTOFF),
        "an exact last-module cutoff must not claim further skipped work"
    );
}

#[test]
fn diagnostic_recovery_project_soft_cutoff_counts_shared_provider_once() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    // One provider module retains two errors and meets the threshold of two exactly. Its importing
    // page is classified blocked before admission, so the command records no cutoff and no stop
    // flag: the provider diagnosis was charged once and the blocked dependant never charges.
    write_project(
        root,
        "project #= (\n    name = \"docs\",\n        max_errors_per_project = 2,\n)\nhtml #= ()\n",
        &[
            ("src/@entry.moth", "@shared broken\n#[:site]\n"),
            (
                "src/shared/@mod.moth",
                "export:\n    broken || -> I32:\n        return shared_missing_one\n    ;\n\n\
                 also_broken || -> I32:\n        return shared_missing_two\n    ;\n;\n",
            ),
        ],
    );

    let (frontend, mut warning_owner, project_source_files) =
        run_directory_lane(root, FrontendCompilationMode::Canonical);
    assert!(frontend.project.scheduling_cutoff.is_none());
    assert_eq!(frontend.project.diagnosed.len(), 1);
    assert_eq!(
        frontend.project.diagnosed[0]
            .diagnostics
            .primary_error_count(),
        2
    );
    assert_eq!(frontend.project.blocked.len(), 1);
    let provider_module = module_id_for_path(&frontend, "shared");
    let dependant_module = module_id_with_path(&frontend, "");
    assert_eq!(frontend.project.blocked[0].module_id, dependant_module);
    assert!(
        matches!(
            frontend.project.blocked[0].required_provider,
            BlockedProvider::Module(provider) if provider == provider_module
        ),
        "the dependant must name the failed provider without a source cascade"
    );
    assert_eq!(
        frontend.project.modules.slot(dependant_module).unwrap(),
        ProviderSlot::Blocked
    );
    assert!(
        frontend
            .project
            .modules
            .normal_interface(dependant_module)
            .expect("slot lookup should resolve")
            .is_none(),
        "a blocked dependant never publishes an interface"
    );

    let reports = finish_diagnosed(frontend, &mut warning_owner, project_source_files);
    assert_eq!(
        reports.counts().errors,
        2,
        "the shared provider diagnosis counts once and the blocked dependant adds no cascade"
    );
    assert!(
        !warning_owner
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::PROJECT_SCHEDULING_CUTOFF)
    );
}

#[test]
fn diagnostic_recovery_check_only_units_share_owner_allowance() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    // The canonical owner is clean, so its whole allowance belongs to its check-only units. The
    // first orphan retains both slots; the second orphan must not be admitted at all.
    write_project(
        root,
        "project #= (\n    name = \"docs\",\n        max_errors_per_project = 30,\n    max_errors_per_module = 2,\n)\nhtml #= ()\n",
        &[
            (
                "src/first/@entry.moth",
                "owner || -> I32:\n    return 1\n;\n",
            ),
            (
                "src/first/a_orphan.moth",
                "a || -> I32:\n    return orphan_a_missing_one\n;\n\n\
                 b || -> I32:\n    return orphan_a_missing_two\n;\n",
            ),
            (
                "src/first/b_orphan.moth",
                "b || -> I32:\n    return orphan_b_missing\n;\n",
            ),
        ],
    );

    let (frontend, mut warning_owner, project_source_files) =
        run_directory_lane(root, FrontendCompilationMode::Check);
    assert!(
        frontend.project.scheduling_cutoff.is_none(),
        "a spent module allowance is not a project scheduling cutoff"
    );
    assert_slot_is_successful(&frontend, "first");
    assert!(
        warning_owner
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE),
        "the skipped second orphan unit must claim the module allowance stop"
    );

    let reports = finish_diagnosed(frontend, &mut warning_owner, project_source_files);
    assert_eq!(reports.counts().errors, 2);
    let rendered = terse::format_terse_diagnostic_reports(&reports);
    assert!(
        rendered.iter().any(|line| line.contains("a_orphan.moth")),
        "the first orphan's retained errors must stay: {rendered:?}"
    );
    assert!(
        rendered
            .iter()
            .all(|line| !line.contains("b_orphan.moth") && !line.contains("orphan_b_missing")),
        "the orphan beyond the owner's allowance must not be compiled: {rendered:?}"
    );
}

#[test]
fn diagnostic_recovery_check_only_allowance_subtracts_owner_errors() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    // The owner module already retains one of its two allowance slots, so only one check-only unit
    // can be admitted: the first orphan fills the owner's remaining slot and the second is skipped.
    write_project(
        root,
        "project #= (\n    name = \"docs\",\n        max_errors_per_project = 30,\n    max_errors_per_module = 2,\n)\nhtml #= ()\n",
        &[
            (
                "src/first/@entry.moth",
                "owner || -> I32:\n    return owner_missing\n;\n",
            ),
            (
                "src/first/a_orphan.moth",
                "a || -> I32:\n    return orphan_a_missing\n;\n",
            ),
            (
                "src/first/b_orphan.moth",
                "b || -> I32:\n    return orphan_b_missing\n;\n",
            ),
        ],
    );

    let (frontend, mut warning_owner, project_source_files) =
        run_directory_lane(root, FrontendCompilationMode::Check);
    assert!(frontend.project.scheduling_cutoff.is_none());
    assert_eq!(frontend.project.diagnosed.len(), 1);
    assert_eq!(
        frontend.project.diagnosed[0]
            .diagnostics
            .primary_error_count(),
        1,
        "the owner module retains one of its two allowance slots"
    );
    assert!(
        warning_owner
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE),
        "the orphan beyond the owner's remaining slot must claim the module allowance stop"
    );

    let reports = finish_diagnosed(frontend, &mut warning_owner, project_source_files);
    assert_eq!(
        reports.counts().errors,
        2,
        "the owner's error plus exactly one admitted orphan fills the allowance"
    );
    let rendered = terse::format_terse_diagnostic_reports(&reports);
    assert!(
        rendered.iter().any(|line| line.contains("a_orphan.moth")),
        "the orphan inside the owner's remaining allowance must be retained: {rendered:?}"
    );
    assert!(
        rendered
            .iter()
            .all(|line| !line.contains("b_orphan.moth") && !line.contains("orphan_b_missing")),
        "the second orphan exceeds the owner's remaining allowance: {rendered:?}"
    );
}

#[test]
fn diagnostic_recovery_paired_roots_report_together_before_the_budget_stops_later_work() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    // One module owns both a support facade and an entry root whose bodies fail independently.
    // The module transaction retains both root diagnostics as one returned batch even though the
    // batch overshoots the project threshold. Only the later module is refused, so a scheduling
    // cutoff never suppresses a co-located root diagnostic.
    write_project(
        root,
        "project #= (\n    name = \"docs\",\n    max_errors_per_project = 1,\n)\nhtml #= ()\n",
        &[
            (
                "src/@package.moth",
                "export:\n    support_value || -> I32:\n        return support_missing\n    ;\n;\n",
            ),
            (
                "src/@entry.moth",
                "entry_value || -> I32:\n    return entry_missing\n;\n",
            ),
            ("src/other/@entry.moth", "other_missing\n#[:site]\n"),
        ],
    );

    let (frontend, mut warning_owner, project_source_files) =
        run_directory_lane(root, FrontendCompilationMode::Canonical);
    assert_eq!(
        frontend.project.diagnosed.len(),
        1,
        "the paired module is the only admitted diagnosed module"
    );
    assert_eq!(
        frontend.project.diagnosed[0].module_id,
        module_id_with_path(&frontend, ""),
        "both root diagnostics belong to the selected-root structural module"
    );
    assert_eq!(
        frontend.project.diagnosed[0]
            .diagnostics
            .primary_error_count(),
        2,
        "the support facade and the entry root each keep their own retained error"
    );
    assert_eq!(
        frontend
            .project
            .scheduling_cutoff
            .map(|cutoff| cutoff.returned_primary_errors()),
        Some(2),
        "the intact batch charged both retained errors before the threshold stopped admission"
    );
    assert_eq!(
        frontend
            .project
            .modules
            .slot(module_id_with_path(&frontend, "other"))
            .expect("the later module slot should resolve"),
        ProviderSlot::Unavailable,
        "the later module is never compiled after the project threshold is reached"
    );
    assert!(
        warning_owner
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::PROJECT_SCHEDULING_CUTOFF),
        "refusing the later module records the command scheduling cutoff"
    );

    let reports = finish_diagnosed(frontend, &mut warning_owner, project_source_files);
    assert_eq!(
        reports.counts().errors,
        2,
        "a returned module batch is never truncated to the project threshold"
    );
    let rendered = terse::format_terse_diagnostic_reports(&reports);
    assert!(
        rendered.iter().any(|line| line.contains("support_missing")),
        "the support facade error must stay: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|line| line.contains("entry_missing")),
        "the entry root error must stay: {rendered:?}"
    );
    assert!(
        rendered
            .iter()
            .all(|line| !line.contains("other_missing") && !line.contains("other/@entry.moth")),
        "the refused module must contribute no diagnostic: {rendered:?}"
    );
}

#[test]
fn diagnostic_recovery_project_budget_spans_source_package_work() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    // The package boundary retains one error before any project module is admitted, so one project
    // module error reaches the threshold of two and the next project module is refused. The budget
    // is command-scope state and never resets when package work hands over to project modules.
    write_project(
        root,
        "project #= (\n    name = \"docs\",\n        max_errors_per_project = 2,\n)\nhtml #= ()\n",
        &[
            ("src/a/@entry.moth", "owner_missing\n#[:site]\n"),
            ("src/b/@entry.moth", "second_missing\n#[:site]\n"),
            (
                "src/uses/@entry.moth",
                "@shared render\nvalue = render() catch:\n    then \"\"\n;\n#[:site]\n",
            ),
            (
                "packages/shared/@mod.moth",
                "export:\n    render || -> String:\n        return shared_missing\n    ;\n;\n",
            ),
        ],
    );

    // The package is registered at its real project root, so the directory lane discovers and
    // compiles it before any project module is admitted.
    let (frontend, mut warning_owner, project_source_files) =
        run_shared_package_lane(root, FrontendCompilationMode::Canonical);
    assert_eq!(
        frontend
            .source_packages
            .iter()
            .map(|package| package.boundary.diagnosed.len())
            .sum::<usize>(),
        1,
        "the package facade failure is retained before project modules are admitted"
    );
    assert_eq!(
        frontend.project.diagnosed.len(),
        1,
        "only the first project module is admitted after the package error"
    );
    assert_eq!(
        frontend.project.blocked.len(),
        1,
        "the module that consumed the failed package facade is blocked, not charged"
    );
    assert_eq!(
        frontend
            .project
            .scheduling_cutoff
            .map(|cutoff| cutoff.returned_primary_errors()),
        Some(2),
        "the package error and the admitted module error reach the threshold of two"
    );
    assert!(
        warning_owner
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::PROJECT_SCHEDULING_CUTOFF),
        "refusing the later module is a project scheduling cutoff"
    );

    let reports = finish_diagnosed(frontend, &mut warning_owner, project_source_files);
    assert_eq!(reports.counts().errors, 2);
}

#[test]
fn diagnostic_recovery_stopped_check_only_unit_treats_unvisited_provider_as_stopped_work() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    // `first` meets the threshold of two, so `second` is refused and its slot stays Unavailable.
    // The orphan check-only unit imports `@second`: after the cutoff that unavailable provider is
    // stopped work, not a compiler bug, so the unit is refused through the cutoff path and never
    // panics or contributes a diagnostic of its own.
    write_project(
        root,
        "project #= (\n    name = \"docs\",\n        max_errors_per_project = 2,\n)\nhtml #= ()\n",
        &[
            (
                "src/first/@entry.moth",
                "a || -> I32:\n    return first_missing_one\n;\n\n\
                 b || -> I32:\n    return first_missing_two\n;\n\n\
                 c || -> I32:\n    return first_missing_three\n;\n",
            ),
            (
                "src/second/@mod.moth",
                "export:\n    value || -> I32:\n        return 1\n    ;\n;\n",
            ),
            (
                "src/orphan.moth",
                "@second value\nconsumer || -> I32:\n    return orphan_missing\n;\n",
            ),
        ],
    );

    let (frontend, mut warning_owner, project_source_files) =
        run_directory_lane(root, FrontendCompilationMode::Check);
    assert_eq!(frontend.project.diagnosed.len(), 1);
    assert_eq!(
        frontend
            .project
            .scheduling_cutoff
            .map(|cutoff| cutoff.returned_primary_errors()),
        Some(3),
        "the first module's retained batch met the threshold before the second was admitted"
    );
    assert_eq!(
        frontend
            .project
            .modules
            .slot(module_id_with_path(&frontend, "second"))
            .expect("the second module slot should resolve"),
        ProviderSlot::Unavailable,
        "the refused module was never compiled"
    );
    assert!(
        warning_owner
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::PROJECT_SCHEDULING_CUTOFF),
        "the stopped check-only unit must not hide the recorded project cutoff"
    );

    let reports = finish_diagnosed(frontend, &mut warning_owner, project_source_files);
    assert_eq!(
        reports.counts().errors,
        3,
        "the refused orphan contributes no diagnostic of its own"
    );
    let rendered = terse::format_terse_diagnostic_reports(&reports);
    assert!(
        rendered
            .iter()
            .all(|line| !line.contains("orphan") && !line.contains("orphan_missing")),
        "the unadmitted orphan must not be compiled: {rendered:?}"
    );
}

#[test]
fn diagnostic_recovery_resource_module_diagnosis_keeps_file_values_isolated() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    // Both bodies of the diagnosed module bind a resolved resource before their independent
    // unknown name. The sibling module is clean and owns a different resource. Recovery must
    // retain exactly the two authored errors: no missing-target or file-value cascade may leak
    // from the failed bodies, and only the clean module may publish its resource association.
    write_project(
        root,
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
        &[
            ("src/@entry.moth", "#[:site]\n"),
            ("src/broken/@mod.moth", RESOURCE_FAILING_MODULE_ROOT),
            ("src/broken/extra.moth", RESOURCE_FAILING_MODULE_EXTRA),
            (
                "src/ok/@mod.moth",
                "export:\n    greeting || -> String:\n        logo = @assets/ok.svg\n        return logo\n    ;\n;\n",
            ),
            ("src/broken/assets/broken.svg", "<svg/>"),
            ("src/ok/assets/ok.svg", "<svg/>"),
        ],
    );

    let (frontend, mut warning_owner, project_source_files) =
        run_directory_lane(root, FrontendCompilationMode::Canonical);
    let broken_module = module_id_for_path(&frontend, "broken");
    let ok_module = module_id_for_path(&frontend, "ok");
    let broken_origin = StableResourceOriginId::module_owned(
        frontend
            .project
            .structure
            .node(broken_module)
            .stable_origin()
            .clone(),
        PortableResourcePath::from_relative_logical_path(Path::new("assets/broken.svg"))
            .expect("the broken resource path is portable"),
    );
    let ok_origin = StableResourceOriginId::module_owned(
        frontend
            .project
            .structure
            .node(ok_module)
            .stable_origin()
            .clone(),
        PortableResourcePath::from_relative_logical_path(Path::new("assets/ok.svg"))
            .expect("the clean resource path is portable"),
    );

    assert_eq!(
        frontend.project.modules.slot(broken_module).unwrap(),
        ProviderSlot::Diagnosed
    );
    assert!(
        frontend
            .project
            .modules
            .normal_interface(broken_module)
            .expect("slot lookup should resolve")
            .is_none(),
        "a diagnosed module never publishes an interface"
    );
    assert!(
        frontend
            .project
            .successful_artefacts_in_module_id_order()
            .all(
                |artifact| artifact.normal_interface.as_ref().is_none_or(|interface| {
                    interface.module_origin.logical_module_path() != Path::new("broken")
                })
            ),
        "a diagnosed module never publishes an artefact"
    );
    assert_slot_is_successful(&frontend, "ok");
    assert_eq!(frontend.project.diagnosed.len(), 1);
    assert_eq!(
        frontend.project.diagnosed[0]
            .diagnostics
            .primary_error_count(),
        2,
        "both independent body errors must be retained without a resource cascade"
    );
    assert!(frontend.project.scheduling_cutoff.is_none());
    assert!(warning_owner.discovery_stops().is_empty());

    // Stage 0 registered both physical files, but the diagnosed module's failed semantic result
    // published no origin association. The clean sibling keeps exactly its own origin.
    assert_eq!(frontend.resource_inputs.records().len(), 2);
    assert!(
        frontend
            .resource_inputs
            .missing_watch_interests()
            .is_empty(),
        "every authored resource target resolved, so no watch interest may remain"
    );
    assert!(
        frontend
            .resource_inputs
            .source_for_origin(&broken_origin)
            .is_none(),
        "a diagnosed module must not register a semantic resource association"
    );
    assert!(
        frontend
            .resource_inputs
            .source_for_origin(&ok_origin)
            .is_some(),
        "the clean module commits its own resource association"
    );
    let ok_artefact = frontend
        .project
        .successful_artefacts_in_module_id_order()
        .find(|artifact| {
            artifact.normal_interface.as_ref().is_some_and(|interface| {
                interface.module_origin.logical_module_path() == Path::new("ok")
            })
        })
        .expect("the clean module should publish an artefact");
    let ok_origins = ok_artefact.module.executable.resource_table.origins();
    assert_eq!(
        ok_origins.len(),
        1,
        "the clean module must not inherit the diagnosed module's resource"
    );
    assert_eq!(ok_origins[0].origin, ok_origin);

    // The success-only gate used by build assembly must reject the retained diagnosed boundary.
    let rejection = frontend
        .project
        .require_all_successful()
        .expect_err("a diagnosed project boundary is never linkable");
    assert!(
        rejection.message.contains("diagnosed ModuleId"),
        "unexpected success-only rejection: {rejection:?}"
    );

    let reports = finish_diagnosed(frontend, &mut warning_owner, project_source_files);
    assert_eq!(reports.counts().errors, 2);
    let sites = reports
        .iter_severity(DiagnosticSeverity::Error)
        .map(|(report, view)| {
            assert_eq!(
                view.descriptor().code,
                UNKNOWN_NAME_CODE,
                "only the authored unknown names may remain"
            );
            let position = DiagnosticRenderContext::for_report(report)
                .site_position(view.primary_site())
                .expect("a retained error keeps its authored site");
            (
                position.path,
                position.byte_range.start,
                position.line.trim().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    let root_site = sites
        .iter()
        .find(|(path, ..)| path.ends_with("broken/@mod.moth"))
        .expect("the first body error keeps its authored file");
    assert_eq!(
        root_site.1,
        RESOURCE_FAILING_MODULE_ROOT
            .find("missing_one")
            .expect("the fixture names the missing value") as u32
    );
    assert_eq!(root_site.2, "return missing_one");
    let extra_site = sites
        .iter()
        .find(|(path, ..)| path.ends_with("broken/extra.moth"))
        .expect("the independent body keeps its authored file");
    assert_eq!(
        extra_site.1,
        RESOURCE_FAILING_MODULE_EXTRA
            .find("missing_two")
            .expect("the fixture names the missing value") as u32
    );
    assert_eq!(extra_site.2, "return missing_two");

    // Repairing both bodies on the same root proves the repaired source compiles and now
    // publishes the association; each lane run builds fresh registries.
    fs::write(root.join("broken/@mod.moth"), RESOURCE_REPAIRED_MODULE_ROOT)
        .expect("the repaired root source should be written");
    fs::write(
        root.join("broken/extra.moth"),
        RESOURCE_REPAIRED_MODULE_EXTRA,
    )
    .expect("the repaired extra source should be written");

    let (repaired, mut warning_owner, project_source_files) =
        run_directory_lane(root, FrontendCompilationMode::Canonical);
    let repaired_broken_module = module_id_for_path(&repaired, "broken");
    let repaired_origin = StableResourceOriginId::module_owned(
        repaired
            .project
            .structure
            .node(repaired_broken_module)
            .stable_origin()
            .clone(),
        PortableResourcePath::from_relative_logical_path(Path::new("assets/broken.svg"))
            .expect("the broken resource path is portable"),
    );
    assert_slot_is_successful(&repaired, "broken");
    assert!(
        repaired.project.diagnosed.is_empty(),
        "the repaired module compiles without diagnostics"
    );
    assert!(repaired.project.scheduling_cutoff.is_none());
    assert!(warning_owner.discovery_stops().is_empty());
    assert!(
        repaired
            .resource_inputs
            .source_for_origin(&repaired_origin)
            .is_some(),
        "the repaired module now publishes the previously absent association"
    );
    let outcome = repaired
        .into_compilation_result(&mut warning_owner, project_source_files, None)
        .expect("a repaired request compiles");
    assert!(matches!(outcome, CompilationOutcome::Success { .. }));
}

#[test]
fn diagnostic_recovery_package_cutoff_leaves_wave_one_root_unvisited() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    // `helper` and `z_broken` are wave 0; the package root depends on `helper` and stays in wave 1.
    // `z_broken` reaches the threshold after `helper` published, so the root is never admitted and
    // the project boundary finds the budget already spent when it starts.
    write_project(
        root,
        PROJECT_ERROR_LIMIT_TWO,
        &[
            (
                "src/@entry.moth",
                "@shared render\nvalue = render() catch:\n    then 0\n;\n#[:site]\n",
            ),
            (
                "packages/shared/@mod.moth",
                "@helper value\nexport:\n    render || -> I32:\n        return value()\n    ;\n;\n",
            ),
            (
                "packages/shared/helper/@mod.moth",
                "export:\n    value || -> I32:\n        return 1\n    ;\n;\n",
            ),
            (
                "packages/shared/z_broken/@entry.moth",
                "a || -> I32:\n    return pkg_missing_one\n;\n\n\
                 b || -> I32:\n    return pkg_missing_two\n;\n",
            ),
        ],
    );

    let (frontend, mut warning_owner, project_source_files) =
        run_shared_package_lane(root, FrontendCompilationMode::Canonical);
    assert_eq!(frontend.source_packages.len(), 1);
    let package = frontend
        .source_packages
        .iter()
        .next()
        .expect("the registered package should publish");
    let helper_module = package_module_id(package, "helper");
    let broken_module = package_module_id(package, "z_broken");
    assert!(
        matches!(
            package
                .boundary
                .modules
                .slot(helper_module)
                .expect("package slot should resolve"),
            ProviderSlot::Successful(_)
        ),
        "the wave-zero helper publishes before the threshold is met"
    );
    assert_eq!(
        package
            .boundary
            .modules
            .slot(broken_module)
            .expect("package slot should resolve"),
        ProviderSlot::Diagnosed
    );
    assert_eq!(package.boundary.diagnosed.len(), 1);
    assert_eq!(
        package.boundary.diagnosed[0]
            .diagnostics
            .primary_error_count(),
        2,
        "the retained package batch keeps both module errors once"
    );
    assert_eq!(
        package
            .boundary
            .scheduling_cutoff
            .map(|cutoff| cutoff.returned_primary_errors()),
        Some(2),
        "refusing the wave-one root requires the package cutoff proof"
    );
    assert_eq!(
        package
            .root_slot()
            .expect("package root slot should resolve"),
        ProviderSlot::Unavailable,
        "the package facade was never admitted"
    );
    assert!(
        package
            .boundary
            .modules
            .normal_interface(package.root_module_id)
            .expect("package slot lookup should resolve")
            .is_none(),
        "an unvisited package root publishes no facade interface"
    );

    let page_module = module_id_with_path(&frontend, "");
    assert_eq!(
        frontend.project.modules.slot(page_module).unwrap(),
        ProviderSlot::Unavailable
    );
    assert!(
        frontend
            .project
            .modules
            .normal_interface(page_module)
            .expect("project slot lookup should resolve")
            .is_none()
    );

    let reports =
        finish_stopped_package_request(frontend, &mut warning_owner, project_source_files, 2);
    for (report, view) in reports.iter_severity(DiagnosticSeverity::Error) {
        assert_eq!(view.descriptor().code, UNKNOWN_NAME_CODE);
        let position = DiagnosticRenderContext::for_report(report)
            .site_position(view.primary_site())
            .expect("the frozen package diagnostic keeps its authored site");
        assert!(
            position.path.ends_with("z_broken/@entry.moth"),
            "the retained diagnostic must keep its package-logical source identity: {:?}",
            position.path
        );
        assert!(
            position.line.trim() == "return pkg_missing_one"
                || position.line.trim() == "return pkg_missing_two",
            "unexpected authored line: {:?}",
            position.line
        );
    }
    let rendered = terse::format_terse_diagnostic_reports(&reports);
    assert!(
        rendered
            .iter()
            .all(|line| line.contains("z_broken/@entry.moth")),
        "only the admitted module may render diagnostics: {rendered:?}"
    );
}

#[test]
fn diagnostic_recovery_package_cutoff_can_diagnose_its_own_root() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    // The package root itself is diagnosed in wave 0, so the cutoff proof must accept a diagnosed
    // root while leaving the later independent child unvisited.
    write_project(
        root,
        PROJECT_ERROR_LIMIT_TWO,
        &[
            (
                "src/@entry.moth",
                "@shared a\nvalue = a() catch:\n    then 0\n;\n#[:site]\n",
            ),
            (
                "packages/shared/@mod.moth",
                "export:\n    a || -> I32:\n        return root_missing_one\n    ;\n\n\
                 b || -> I32:\n        return root_missing_two\n    ;\n;\n",
            ),
            (
                "packages/shared/later/@mod.moth",
                "export:\n    value || -> I32:\n        return 1\n    ;\n;\n",
            ),
        ],
    );

    let (frontend, mut warning_owner, project_source_files) =
        run_shared_package_lane(root, FrontendCompilationMode::Canonical);
    let package = frontend
        .source_packages
        .iter()
        .next()
        .expect("the registered package should publish");
    let later_module = package_module_id(package, "later");
    assert_eq!(
        package
            .boundary
            .modules
            .slot(package.root_module_id)
            .expect("package root slot should resolve"),
        ProviderSlot::Diagnosed
    );
    assert!(
        package
            .boundary
            .modules
            .normal_interface(package.root_module_id)
            .expect("package slot lookup should resolve")
            .is_none()
    );
    assert_eq!(
        package
            .boundary
            .modules
            .slot(later_module)
            .expect("package slot should resolve"),
        ProviderSlot::Unavailable,
        "the later independent child was never admitted"
    );
    assert!(
        package
            .boundary
            .modules
            .normal_interface(later_module)
            .expect("package slot lookup should resolve")
            .is_none()
    );
    assert_eq!(package.boundary.diagnosed.len(), 1);
    assert_eq!(
        package.boundary.diagnosed[0]
            .diagnostics
            .primary_error_count(),
        2
    );

    let _reports =
        finish_stopped_package_request(frontend, &mut warning_owner, project_source_files, 2);
}

/// Write the deferred check-only package fixture.
///
/// The two unselected package-root sources are examined in path order after every canonical
/// package facade publishes, so `a_orphan` is examined first and `b_orphan` is the unit left
/// uncompiled. `broken` is retained before the limit is reached so the deferred lane can charge
/// further work or be refused after a package cutoff.
fn write_deferred_check_only_package_project(
    root: &Path,
    project_error_limit: u32,
    first_orphan: &str,
    second_orphan: &str,
) {
    let config = format!(
        "project #= (\n    name = \"docs\",\n        max_errors_per_project = {project_error_limit},\n)\nhtml #= ()\n"
    );
    write_project(
        root,
        &config,
        &[
            (
                "src/@entry.moth",
                "@shared render\nvalue = render() catch:\n    then 0\n;\n#[:site]\n",
            ),
            (
                "packages/shared/@mod.moth",
                "export:\n    render || -> I32:\n        return 7\n    ;\n;\n",
            ),
            (
                "packages/shared/broken/@entry.moth",
                "a || -> I32:\n    return pkg_missing_one\n;\n\n\
                 b || -> I32:\n    return pkg_missing_two\n;\n",
            ),
            (
                "packages/shared/z_late/@mod.moth",
                "export:\n    late_value || -> I32:\n        return 1\n    ;\n;\n",
            ),
            ("packages/shared/a_orphan.moth", first_orphan),
            ("packages/shared/b_orphan.moth", second_orphan),
        ],
    );
}

#[test]
fn diagnostic_recovery_deferred_package_check_only_refuses_an_unvisited_provider() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    // The package cutoff leaves `z_late` unvisited. The first deferred orphan imports it: with the
    // budget already closed that provider is stopped work, not a compiler bug, so the unit is
    // refused through the cutoff path and the lane continues to no further work.
    write_deferred_check_only_package_project(
        root,
        2,
        "@z_late late_value\norphan || -> I32:\n    return orphan_unvisited_missing\n;\n",
        "orphan || -> I32:\n    return orphan_skipped_missing\n;\n",
    );

    let (frontend, mut warning_owner, project_source_files) =
        run_shared_package_lane(root, FrontendCompilationMode::Check);
    let package = frontend
        .source_packages
        .iter()
        .next()
        .expect("the registered package should publish");
    let late_module = package_module_id(package, "z_late");
    assert!(
        matches!(
            package
                .boundary
                .modules
                .slot(package.root_module_id)
                .expect("package root slot should resolve"),
            ProviderSlot::Successful(_)
        ),
        "the canonical package facade published before the deferred lane ran"
    );
    assert_eq!(
        package
            .boundary
            .modules
            .slot(late_module)
            .expect("package slot should resolve"),
        ProviderSlot::Unavailable,
        "the canonical cutoff left the later module unvisited"
    );
    assert!(
        frontend.transient_batches.is_empty(),
        "the refused deferred unit contributes no transient batch"
    );
    assert!(
        !warning_owner
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE),
        "stopped work reports the project cutoff, not a spent module allowance"
    );

    // Deferred refusals never install into the package boundary: its cutoff is unchanged from
    // canonical work.
    let reports =
        finish_stopped_package_request(frontend, &mut warning_owner, project_source_files, 2);
    let rendered = terse::format_terse_diagnostic_reports(&reports);
    assert!(
        rendered.iter().all(|line| !line.contains("a_orphan.moth")
            && !line.contains("b_orphan.moth")
            && !line.contains("orphan_unvisited_missing")
            && !line.contains("orphan_skipped_missing")),
        "refused deferred work must never appear in reports: {rendered:?}"
    );
}

#[test]
fn diagnostic_recovery_deferred_package_check_only_stops_after_an_admitted_unit() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    // The canonical package completes below the limit of three. The first deferred orphan is
    // admitted, charges the third error and closes the budget; the second eligible orphan is then
    // refused. Its absence from reports proves the deferred lane observed the threshold, and the
    // package boundary keeps no cutoff record because canonical work was complete.
    write_deferred_check_only_package_project(
        root,
        3,
        "orphan || -> I32:\n    return orphan_admitted_missing\n;\n",
        "orphan_b || -> I32:\n    return orphan_skipped_missing\n;\n",
    );

    let (frontend, mut warning_owner, project_source_files) =
        run_shared_package_lane(root, FrontendCompilationMode::Check);
    let package = frontend
        .source_packages
        .iter()
        .next()
        .expect("the registered package should publish");
    assert!(matches!(
        package
            .boundary
            .modules
            .slot(package.root_module_id)
            .expect("package root slot should resolve"),
        ProviderSlot::Successful(_)
    ));
    assert_eq!(
        package.boundary.diagnosed.len(),
        1,
        "deferred diagnostics never join the package boundary"
    );
    assert_eq!(
        package.boundary.diagnosed[0]
            .diagnostics
            .primary_error_count(),
        2
    );
    assert!(
        package.boundary.scheduling_cutoff.is_none(),
        "canonical package work finished below the threshold"
    );
    assert_eq!(
        frontend.transient_batches.len(),
        1,
        "exactly the first deferred unit was admitted"
    );
    // The domain tag proves this batch came from the deferred package lane rather than the project
    // transient lane; no other assertion observes the tag.
    assert!(frontend.transient_batches[0].package_id.is_some());
    assert_eq!(frontend.transient_batches[0].batch.primary_error_count(), 1);
    assert_eq!(
        frontend
            .project
            .scheduling_cutoff
            .map(|cutoff| cutoff.returned_primary_errors()),
        Some(3)
    );
    assert!(
        frontend.project.diagnosed.is_empty(),
        "the project boundary must not invent a diagnosis"
    );
    assert!(
        warning_owner
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::PROJECT_SCHEDULING_CUTOFF)
    );
    assert!(
        !warning_owner
            .discovery_stops()
            .contains(DiagnosticDiscoveryStops::MODULE_ERROR_ALLOWANCE)
    );

    // Mirror the build assembly success-only gate. The project boundary carries the cutoff while
    // the completed package boundary only retains its diagnosed canonical module.
    let rejection = frontend
        .project
        .require_all_successful()
        .expect_err("a stopped project boundary is never linkable");
    assert!(
        rejection.message.contains("project scheduling cutoff"),
        "unexpected success-only rejection: {rejection:?}"
    );
    for package in frontend.source_packages.iter() {
        let rejection = package
            .boundary
            .require_all_successful()
            .expect_err("a diagnosed package boundary is never linkable");
        assert!(
            rejection.message.contains("diagnosed ModuleId"),
            "unexpected package success-only rejection: {rejection:?}"
        );
    }

    let reports = finish_diagnosed(frontend, &mut warning_owner, project_source_files);
    assert_eq!(
        reports.counts().errors,
        3,
        "the canonical batch plus exactly one admitted deferred unit"
    );
    let rendered = terse::format_terse_diagnostic_reports(&reports);
    assert!(
        rendered.iter().any(|line| line.contains("a_orphan.moth")),
        "the admitted deferred unit must keep its diagnostics: {rendered:?}"
    );
    assert!(
        rendered
            .iter()
            .all(|line| !line.contains("b_orphan.moth") && !line.contains("orphan_skipped_missing")),
        "the orphan beyond the threshold must not be compiled: {rendered:?}"
    );
}

/// WHAT: an invalid qualified selection and a cyclic qualified dependency report typed user
///       diagnostics under a project error threshold, and the whole request repeats byte-identically.
/// WHY:  Stage 0 preparation feeds both per-file header diagnostics and Stage 3 declaration
///       ordering. Neither may panic, reach `compiler_bug!`, or depend on worker completion order
///       when the project budget stops later admissions.
#[test]
fn qualified_cycle_and_invalid_selection_under_project_budget_are_deterministic() {
    const PROJECT_ERROR_LIMIT_TWO: &str =
        "project #= (\n    name = \"docs\",\n        max_errors_per_project = 2,\n)\nhtml #= ()\n";
    // Alphabetical module order is `a_invalid`, `b_cycle`, `zz_last`. The invalid qualified
    // selection spends one binding error, so the cyclic module is still admitted and both
    // user-diagnostic kinds are retained; `zz_last` then proves the cutoff refused further work.
    // Keeping the invalid module first means the cycle's exact diagnostic count does not decide
    // which failure kind survives.
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    write_project(
        root,
        PROJECT_ERROR_LIMIT_TWO,
        &[
            (
                "src/a_invalid/@entry.moth",
                "broken = @helper missing\n#[:site]\n",
            ),
            ("src/a_invalid/helper.moth", "present #= 1\n"),
            ("src/b_cycle/@entry.moth", "@alpha a\n#[:site]\n"),
            ("src/b_cycle/alpha.moth", "a #= @beta b\n"),
            ("src/b_cycle/beta.moth", "b #= @alpha a\n"),
            ("src/zz_last/@entry.moth", "#[:site]\n"),
        ],
    );

    let first = {
        let (frontend, mut warning_owner, project_source_files) =
            run_directory_lane(root, FrontendCompilationMode::Canonical);
        for suffix in ["a_invalid", "b_cycle"] {
            assert_eq!(
                frontend
                    .project
                    .modules
                    .slot(module_id_for_path(&frontend, suffix))
                    .expect("module slot should resolve"),
                ProviderSlot::Diagnosed,
                "{suffix} should retain its admitted user diagnostics"
            );
        }
        assert_eq!(
            frontend
                .project
                .modules
                .slot(module_id_for_path(&frontend, "zz_last"))
                .expect("module slot should resolve"),
            ProviderSlot::Unavailable,
            "the refused module was never compiled"
        );
        let cutoff = frontend
            .project
            .scheduling_cutoff
            .expect("spending the threshold before zz_last must record a cutoff");
        assert!(
            cutoff.returned_primary_errors() >= 2,
            "the recorded cutoff proof must reach the configured threshold"
        );
        assert!(
            warning_owner
                .discovery_stops()
                .contains(DiagnosticDiscoveryStops::PROJECT_SCHEDULING_CUTOFF),
            "the cutoff flag must arrive through the warning sink"
        );
        assert!(
            frontend.project.blocked.is_empty(),
            "user diagnostics must not be reclassified as blocked providers"
        );
        let reports = finish_diagnosed(frontend, &mut warning_owner, project_source_files);
        assert_eq!(
            reports.counts().errors,
            cutoff.returned_primary_errors(),
            "the retained report count must match the cutoff proof"
        );
        assert!(
            reports
                .iter_severity(DiagnosticSeverity::Error)
                .all(|(_, view)| view.descriptor().code.starts_with("MOTH-")),
            "every failure must be a typed user diagnostic"
        );
        let codes: Vec<_> = reports
            .iter_severity(DiagnosticSeverity::Error)
            .map(|(_, view)| view.descriptor().code)
            .collect();
        assert!(
            codes.contains(&"MOTH-RULE-0033"),
            "the cyclic qualified constants must report the declaration-cycle diagnostic: {codes:?}"
        );
        assert!(
            codes.contains(&"MOTH-IMPORT-0005"),
            "the missing qualified target must report the dependency-target diagnostic: {codes:?}"
        );
        let rendered = terse::format_terse_diagnostic_reports(&reports);
        assert!(
            rendered.iter().all(|line| !line.contains("zz_last")),
            "the refused module must not contribute diagnostics: {rendered:?}"
        );
        rendered
    };

    let second = {
        let (frontend, mut warning_owner, project_source_files) =
            run_directory_lane(root, FrontendCompilationMode::Canonical);
        let reports = finish_diagnosed(frontend, &mut warning_owner, project_source_files);
        terse::format_terse_diagnostic_reports(&reports)
    };
    assert_eq!(
        first, second,
        "repeated qualified failure reports must be byte-identical"
    );
}
