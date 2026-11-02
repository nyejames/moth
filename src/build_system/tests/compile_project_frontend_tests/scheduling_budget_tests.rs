//! Project scheduling-cutoff and shared-allowance regression coverage.
//!
//! WHAT: drives the real directory frontend lane with real `config.moth` values, then inspects the
//!       retained graph boundary before the terminal wrapper converts it into reports.
//! WHY: the project threshold is a command-owned soft cutoff, not a result cap. These tests prove
//!       returned module diagnostics stay intact, unadmitted modules keep `Unavailable` slots, the
//!       cutoff flag is command scope, and check-only units share their owner's module allowance.

use super::*;
use crate::build_system::create_project_modules::compiled_boundary::{
    BlockedProvider, ProjectFrontendCompilation,
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

fn write_project(root: &Path, config_text: &str, files: &[(&str, &str)]) {
    fs::write(root.join("config.moth"), config_text).expect("config should be written");
    for (relative_path, contents) in files {
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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n    max_errors_per_project = 2,\n)\nhtml #= ()\n",
        &[
            ("src/first/@first.moth", THREE_FAILING_BODIES),
            ("src/second/@second.moth", ONE_FAILING_BODY),
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
            .interface(second_module)
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
            .any(|line| line.contains("first/@first.moth")),
        "the admitted module's diagnostics must stay rendered: {rendered:?}"
    );
    assert!(
        rendered
            .iter()
            .all(|line| !line.contains("second/@second.moth")
                && !line.contains("only_second_module")),
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
        "project_error_limit #I32? = none\nproject #= (\n    name = \"docs\",\n    entry_root = \"src\",\n    max_errors_per_project = project_error_limit,\n)\nhtml #= ()\n",
        &[
            ("src/first/@first.moth", THREE_FAILING_BODIES),
            ("src/second/@second.moth", ONE_FAILING_BODY),
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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n    max_errors_per_project = 3,\n)\nhtml #= ()\n",
        &[
            ("src/@page.moth", "#[:site]\n"),
            (
                "src/first/@first.moth",
                "answer || -> I32:\n    return 1\n;\n",
            ),
            ("src/second/@second.moth", THREE_FAILING_BODIES),
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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n    max_errors_per_project = 2,\n)\nhtml #= ()\n",
        &[
            ("src/@page.moth", "@shared broken\n#[:site]\n"),
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
            .interface(dependant_module)
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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n    max_errors_per_project = 30,\n    max_errors_per_module = 2,\n)\nhtml #= ()\n",
        &[
            (
                "src/first/@first.moth",
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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n    max_errors_per_project = 30,\n    max_errors_per_module = 2,\n)\nhtml #= ()\n",
        &[
            (
                "src/first/@first.moth",
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
fn diagnostic_recovery_project_budget_spans_source_package_work() {
    let _guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temp = tempfile::tempdir().expect("temp project should exist");
    let root = temp.path();
    // The package boundary retains one error before any project module is admitted, so one project
    // module error reaches the threshold of two and the next project module is refused. The budget
    // is command-scope state and never resets when package work hands over to project modules.
    write_project(
        root,
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n    max_errors_per_project = 2,\n)\nhtml #= ()\n",
        &[
            ("src/a/@page.moth", "owner_missing\n#[:site]\n"),
            ("src/b/@page.moth", "second_missing\n#[:site]\n"),
            (
                "src/uses/@page.moth",
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
    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root(
            "shared",
            root.join("packages/shared"),
            PackageOrigin::ProjectLocal,
        )
        .expect("the shared package root registers");
    let (frontend, mut warning_owner, project_source_files) = run_directory_lane_with_surface(
        root,
        FrontendCompilationMode::Canonical,
        &mut frontend_surface,
    );
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
        "project #= (\n    name = \"docs\",\n    entry_root = \"src\",\n    max_errors_per_project = 2,\n)\nhtml #= ()\n",
        &[
            (
                "src/first/@first.moth",
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
