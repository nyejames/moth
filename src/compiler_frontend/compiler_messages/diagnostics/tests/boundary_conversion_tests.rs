//! Behavioural tests for prepared diagnostics crossing the frozen boundary.
//!
//! These tests exercise owner-edge capture, direct accumulator appends, remapping, and frozen
//! rendering without constructing a legacy message vessel.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::compiler_frontend::compiler_errors::{
    BoundaryFailure, CompilerError, ErrorType, InfrastructureErrors,
};
use crate::compiler_frontend::compiler_messages::diagnostic_bag::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot_builder::ForcedTypeDisplayExhaustionGuard;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    CompilerCapacityExceededView, SourceSpanCapacityView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticPrimaryPosition, DiagnosticRenderContext, compact_label_message_text,
};
use crate::compiler_frontend::compiler_messages::{
    CapacityResource, CompilerDiagnostic, DiagnosticCapacityResource, DiagnosticLabel,
    DiagnosticLabelMessage, SourceSpanCapacityResource, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityHandle, LocalSpan, SourceDatabase, SourceId, SourceSpan,
};
use crate::compiler_frontend::symbols::path_interner::{
    ForcedExhaustionGuard, PathInternerBuilder,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[derive(Debug, PartialEq)]
struct Rendered {
    position: Option<OwnedPosition>,
    message: String,
}

#[derive(Debug, PartialEq)]
struct OwnedPosition {
    path: PathBuf,
    host_path: Option<PathBuf>,
    start: (u32, u32),
    end: (u32, u32),
    line: String,
}

fn owned_position(position: &DiagnosticPrimaryPosition<'_>) -> OwnedPosition {
    OwnedPosition {
        path: position.path.clone(),
        host_path: position.host_path.map(Path::to_path_buf),
        start: (position.start.line, position.start.column),
        end: (position.end.line, position.end.column),
        line: position.line.to_owned(),
    }
}

fn primary(rendered: &Rendered) -> &OwnedPosition {
    rendered
        .position
        .as_ref()
        .expect("primary position should resolve")
}

fn frozen_render(failure: &BoundaryFailure) -> Vec<Rendered> {
    failure
        .reports
        .iter()
        .map(|(report, view)| {
            let context = DiagnosticRenderContext::new(report.context().strings())
                .with_frozen_identity(report.context());
            let position = context
                .site_position(view.primary_site())
                .map(|position| owned_position(&position));
            let render = view
                .family()
                .render
                .expect("matched family rows declare a renderer");
            let payload = render(&view, context);
            Rendered {
                position,
                message: payload.message,
            }
        })
        .collect()
}

#[derive(Debug, PartialEq)]
struct RenderedLabel {
    position: Option<OwnedPosition>,
    message: String,
}

fn frozen_labels(failure: &BoundaryFailure) -> Vec<RenderedLabel> {
    let mut labels = Vec::new();
    for (report, view) in failure.reports.iter() {
        let context = DiagnosticRenderContext::for_report(report);
        labels.extend(view.durable_labels().map(|label| {
            RenderedLabel {
                position: context
                    .site_position(label.site)
                    .map(|position| owned_position(&position)),
                message: compact_label_message_text(label.message, view.types(), context)
                    .unwrap_or_default(),
            }
        }));
    }
    labels
}

/// Writes `text` to `src/<relative_name>` under `directory` and builds a database that retains it.
fn retained_database(
    directory: &Path,
    relative_name: &str,
    text: &str,
    string_table: &mut StringTable,
) -> (SourceDatabase, SourceId) {
    let source_root = directory.join("src");
    fs::create_dir_all(&source_root).expect("should create source root");
    let source_path = source_root.join(relative_name);
    fs::write(&source_path, text).expect("should write source");

    let mut source_database = SourceDatabase::build(
        std::iter::once(source_path.as_path()),
        &source_root,
        None,
        string_table,
    )
    .expect("source identity should build");
    let source_id = source_database
        .get_by_canonical_path(&source_path)
        .expect("source should be registered")
        .id;
    source_database
        .retain_text(source_id, text.to_owned())
        .expect("compiled source should be retained");

    (source_database, source_id)
}

fn inline_span(source: SourceId, start: u32, length: u32) -> SourceSpan {
    let mut builder = ExtendedSpanBuilder::new();
    SourceSpan::new(
        source,
        LocalSpan::exact(start, length, &mut builder).expect("test span should fit inline"),
    )
}

type PreparedTestBatch = (
    PreparedDiagnosticBatch,
    Option<(Arc<SourceDatabase>, Option<StablePackageIdentity>)>,
);

/// Freeze direct prepared captures with their owning source contexts.
fn finish_prepared_batches(
    batches: Vec<PreparedTestBatch>,
    install: InstallMode,
) -> BoundaryFailure {
    let mut accumulator = BoundaryAccumulator::new();
    for (batch, source) in batches {
        accumulator.append_prepared(batch, source);
    }
    accumulator.finish(install)
}

#[test]
fn prepared_batch_carrier_stays_within_its_size_budget() {
    assert!(std::mem::size_of::<PreparedDiagnosticBatch>() <= 48);
}

/// Colliding local ids remap a captured type name and a later unknown-name draft across distinct paths.
#[test]
fn prepared_batch_capture_remaps_type_names_paths_and_path_row_offsets() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let identity = StablePackageIdentity::project_local("prepared-type-paths");
    let make_batches = || {
        let mut base_strings = StringTable::new();
        let (database, source) =
            retained_database(dir.path(), "main.moth", "main", &mut base_strings);
        let database = Arc::new(database);

        let mut alpha_strings = base_strings.clone();
        let alpha_name = alpha_strings.intern("TAlpha");
        let alpha_unused = alpha_strings.intern("unused");
        let mut alpha_paths = PathInternerBuilder::new();
        let alpha_import_path = alpha_paths
            .try_intern_portable_path("pkg/alpha-only.moth", &mut alpha_strings)
            .expect("Alpha import path should intern");
        let alpha_explicit = Arc::new(alpha_paths.freeze());

        let mut beta_strings = base_strings;
        let beta_name = beta_strings.intern("TBeta");
        let beta_missing = beta_strings.intern("missing-beta");
        assert_eq!(alpha_name, beta_name, "type-name ids should collide");
        assert_eq!(
            alpha_unused, beta_missing,
            "the later unknown-name id should collide with the adopted table"
        );
        let mut beta_paths = PathInternerBuilder::new();
        let beta_import_path = beta_paths
            .try_intern_portable_path("pkg/beta-only.moth", &mut beta_strings)
            .expect("Beta import path should intern");
        let beta_explicit = Arc::new(beta_paths.freeze());
        for explicit in [&alpha_explicit, &beta_explicit] {
            assert!(
                !database
                    .clone_path_builder()
                    .merge_snapshot(explicit)
                    .expect("explicit paths should merge into the owner")
                    .is_identity(),
                "each explicit path table should require a path-id remap"
            );
        }
        assert!(!Arc::ptr_eq(&alpha_explicit, &beta_explicit));

        let make_batch = |strings, name, import_path, explicit, unknown_name| {
            let mut environment = TypeEnvironment::new();
            let generic_type = environment.register_synthetic_generic_parameter(name);
            let boolean = environment.builtins().bool;
            let mut diagnostics = vec![
                CompilerDiagnostic::type_mismatch(
                    generic_type,
                    boolean,
                    TypeMismatchContext::Assignment,
                    Some(inline_span(source, 0, 4)),
                ),
                CompilerDiagnostic::missing_import_target(
                    import_path,
                    Some(inline_span(source, 0, 4)),
                ),
            ];
            if let Some(name) = unknown_name {
                diagnostics.push(CompilerDiagnostic::unknown_value_name(
                    name,
                    Some(inline_span(source, 0, 4)),
                ));
            }
            PreparedDiagnosticBatch::capture_from_bag(
                DiagnosticBag::from_diagnostics(diagnostics),
                strings,
                Some(&environment),
                true,
                Some(explicit),
            )
        };
        let owner = || Some((Arc::clone(&database), Some(identity.clone())));
        vec![
            (
                make_batch(
                    alpha_strings,
                    alpha_name,
                    alpha_import_path,
                    alpha_explicit,
                    None,
                ),
                owner(),
            ),
            (
                make_batch(
                    beta_strings,
                    beta_name,
                    beta_import_path,
                    beta_explicit,
                    Some(beta_missing),
                ),
                owner(),
            ),
        ]
    };

    let failure = finish_prepared_batches(make_batches(), InstallMode::Frozen);
    let rendered = frozen_render(&failure);
    assert_eq!(rendered.len(), 5);
    assert!(rendered[0].message.contains("TAlpha"));
    assert!(rendered[1].message.contains("pkg/alpha-only.moth"));
    assert!(rendered[2].message.contains("TBeta"));
    assert!(rendered[3].message.contains("pkg/beta-only.moth"));
    assert!(rendered[4].message.contains("missing-beta"));
}

/// Shared explicit tables stay shared, while later distinct tables retain their own path rows.
#[test]
fn prepared_batch_capture_preserves_shared_and_distinct_explicit_path_tables() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let identity = StablePackageIdentity::project_local("prepared-path-owners");
    let make_batches = || {
        let mut strings = StringTable::new();
        let mut shared_paths = PathInternerBuilder::new();
        let alpha = shared_paths
            .try_intern_portable_path("pkg/alpha-shared.moth", &mut strings)
            .expect("Alpha path should intern");
        let beta = shared_paths
            .try_intern_portable_path("pkg/beta-shared.moth", &mut strings)
            .expect("Beta path should intern");
        let shared_table = Arc::new(shared_paths.freeze());
        let mut distinct_paths = PathInternerBuilder::new();
        let gamma = distinct_paths
            .try_intern_portable_path("vendor/gamma-distinct.moth", &mut strings)
            .expect("Gamma path should intern");
        let distinct_table = Arc::new(distinct_paths.freeze());
        assert!(!Arc::ptr_eq(&shared_table, &distinct_table));

        let (database, source) = retained_database(dir.path(), "main.moth", "main", &mut strings);
        for explicit in [&shared_table, &distinct_table] {
            assert!(
                !database
                    .clone_path_builder()
                    .merge_snapshot(explicit)
                    .expect("explicit paths should merge into the owner")
                    .is_identity(),
                "explicit table should require a path-id remap"
            );
        }
        let database = Arc::new(database);
        let make_batch = |path, path_table| {
            PreparedDiagnosticBatch::capture_from_bag(
                DiagnosticBag::from(CompilerDiagnostic::missing_import_target(
                    path,
                    Some(inline_span(source, 0, 4)),
                )),
                strings.clone(),
                None,
                true,
                Some(path_table),
            )
        };
        let owner = || Some((Arc::clone(&database), Some(identity.clone())));
        vec![
            (make_batch(alpha, Arc::clone(&shared_table)), owner()),
            (make_batch(beta, Arc::clone(&shared_table)), owner()),
            (make_batch(gamma, Arc::clone(&distinct_table)), owner()),
        ]
    };

    let failure = finish_prepared_batches(make_batches(), InstallMode::Frozen);
    let rendered = frozen_render(&failure);
    assert_eq!(rendered.len(), 3);
    assert!(rendered[0].message.contains("pkg/alpha-shared.moth"));
    assert!(rendered[1].message.contains("pkg/beta-shared.moth"));
    assert!(rendered[2].message.contains("vendor/gamma-distinct.moth"));
}

/// An empty prepared donor row must still install the identity handle used by an earlier batch.
#[test]
fn prepared_batch_capture_installs_a_foreign_donor_without_its_own_diagnostic() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let donor_identity = StablePackageIdentity::project_local("prepared-donor");
    let make_batches = || {
        let mut strings = StringTable::new();
        let (donor_database, donor_source) =
            retained_database(dir.path(), "donor.moth", "donor", &mut strings);
        let donor_database = Arc::new(donor_database);
        let name = strings.intern("missing");

        let handle = FrozenIdentityHandle::for_domain(donor_identity.clone());
        let mut diagnostic =
            CompilerDiagnostic::unknown_value_name(name, Some(inline_span(donor_source, 0, 5)));
        diagnostic.primary_frozen_identity_handle = Some(handle);
        let diagnosed = PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from(diagnostic),
            strings.clone(),
            None,
            false,
            None,
        );
        let empty = PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::new(),
            strings,
            None,
            false,
            None,
        );
        vec![
            (diagnosed, None),
            (
                empty,
                Some((Arc::clone(&donor_database), Some(donor_identity.clone()))),
            ),
        ]
    };

    let failure = finish_prepared_batches(make_batches(), InstallMode::Frozen);
    let rendered = frozen_render(&failure);
    assert_eq!(rendered.len(), 1);
    assert!(primary(&rendered[0]).path.ends_with("donor.moth"));
    assert_eq!(primary(&rendered[0]).line, "donor");
}

#[test]
fn prepared_batch_without_type_context_retains_unresolved_type_ids() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut strings = StringTable::new();
    let (database, source) = retained_database(dir.path(), "main.moth", "main", &mut strings);
    let diagnostic = CompilerDiagnostic::type_mismatch(
        TypeId(999),
        TypeId(1000),
        TypeMismatchContext::Assignment,
        Some(inline_span(source, 0, 4)),
    );
    let batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        strings,
        None,
        false,
        None,
    );
    let failure = finish_prepared_batches(
        vec![(batch, Some((Arc::new(database), None)))],
        InstallMode::Frozen,
    );
    let rendered = frozen_render(&failure);
    assert_eq!(
        rendered[0].message,
        "Type mismatch in assignment: expected TypeId(999), found TypeId(1000)",
    );
}

#[test]
fn prepared_batch_type_display_capacity_fault_keeps_the_source_span_and_code() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let _exhausted = ForcedTypeDisplayExhaustionGuard::new();
    let mut strings = StringTable::new();
    let (database, source) = retained_database(dir.path(), "main.moth", "main", &mut strings);
    let diagnostic = CompilerDiagnostic::type_mismatch(
        TypeId(999),
        TypeId(1000),
        TypeMismatchContext::Assignment,
        Some(inline_span(source, 0, 4)),
    );
    let batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        strings,
        None,
        false,
        None,
    );
    let failure = finish_prepared_batches(
        vec![(batch, Some((Arc::new(database), None)))],
        InstallMode::Frozen,
    );
    let rendered = frozen_render(&failure);
    assert_eq!(
        rendered[0].message,
        "This source exceeds the compiler's capacity for diagnostic type displays."
    );
    let (report, view) = failure
        .reports
        .iter()
        .next()
        .expect("the capacity diagnostic remains frozen");
    let facts = view
        .family_view::<CompilerCapacityExceededView>()
        .expect("the type-display capacity replacement keeps its typed resource");
    assert!(matches!(
        facts.resource,
        CapacityResource::Store(DiagnosticCapacityResource::TypeDisplays)
    ));
    assert_eq!(report.len(), 1);
    assert_eq!(primary(&rendered[0]).path, PathBuf::from("src/main.moth"));
    assert_eq!(primary(&rendered[0]).line, "main");
    let position = primary(&rendered[0]);
    let expected_host_path = dir.path().join("src/main.moth");
    assert_eq!(
        position.host_path.as_deref(),
        Some(expected_host_path.as_path())
    );
    assert_eq!(position.start, (0, 0));
    assert_eq!(position.end, (0, 4));
}

#[test]
fn prepared_batch_path_capacity_fault_matches_source_capacity_diagnostic() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let make_batches = || {
        let mut strings = StringTable::new();
        let mut paths = PathInternerBuilder::new();
        let new_path = paths
            .try_intern_portable_path("pkg/new/mod.moth", &mut strings)
            .expect("portable path should intern");
        let explicit = Arc::new(paths.freeze());
        let (database, source) = retained_database(dir.path(), "main.moth", "main", &mut strings);
        let batch = PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from(CompilerDiagnostic::missing_import_target(
                new_path,
                Some(inline_span(source, 0, 4)),
            )),
            strings,
            None,
            true,
            Some(explicit),
        );
        vec![(batch, Some((Arc::new(database), None)))]
    };
    let expected_batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(CompilerDiagnostic::source_table_capacity(
            SourceSpanCapacityResource::LogicalPath,
        )),
        StringTable::new(),
        None,
        false,
        None,
    );
    let expected_failure =
        finish_prepared_batches(vec![(expected_batch, None)], InstallMode::Frozen);
    let expected_render = frozen_render(&expected_failure);

    let batches = make_batches();
    let _exhausted = ForcedExhaustionGuard::new();
    let failure = finish_prepared_batches(batches, InstallMode::Frozen);
    let rendered = frozen_render(&failure);
    assert_eq!(rendered.len(), 1);
    assert_eq!(failure.reports.report(0).len(), 1);
    assert_eq!(rendered[0].message, expected_render[0].message);
    assert_eq!(rendered[0].position, expected_render[0].position);

    let expected_code = expected_failure
        .reports
        .iter()
        .next()
        .expect("the baseline capacity report is present")
        .1
        .descriptor()
        .code;
    let (report, view) = failure
        .reports
        .iter()
        .next()
        .expect("the path capacity replacement is frozen");
    assert_eq!(view.descriptor().code, expected_code);
    let facts = view
        .family_view::<SourceSpanCapacityView>()
        .expect("path overflow retains a source-span capacity fact");
    assert_eq!(facts.resource, SourceSpanCapacityResource::LogicalPath);
    assert_eq!(report.len(), 1);
}

#[test]
fn prepared_batch_without_source_owner_keeps_path_facts_and_absent_positions() {
    let mut strings = StringTable::new();
    let first_name = strings.intern("unowned");
    let second_name = strings.intern("unresolved_span");
    let mut paths = PathInternerBuilder::new();
    let import_path = paths
        .try_intern_portable_path("pkg/mod.moth", &mut strings)
        .expect("portable path should intern");
    let batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(vec![
            CompilerDiagnostic::unknown_value_name(first_name, None),
            CompilerDiagnostic::unknown_value_name(
                second_name,
                Some(inline_span(SourceId::from_index(1), 0, 4)),
            ),
            CompilerDiagnostic::missing_import_target(import_path, None),
        ]),
        strings,
        None,
        true,
        Some(Arc::new(paths.freeze())),
    );
    let failure = finish_prepared_batches(vec![(batch, None)], InstallMode::Frozen);
    let rendered = frozen_render(&failure);
    assert_eq!(rendered.len(), 3);
    assert!(rendered[0].position.is_none());
    assert!(rendered[1].position.is_none());
    assert!(rendered[2].position.is_none());
    assert!(rendered[2].message.contains("pkg/mod.moth"));
}

#[test]
fn append_infrastructure_keeps_its_frozen_source_identity() {
    let dir = tempfile::tempdir().expect("should create temporary directory");
    let mut strings = StringTable::new();
    let (database, source) = retained_database(dir.path(), "main.moth", "main", &mut strings);
    let database = Arc::new(database);
    let mut accumulator = BoundaryAccumulator::with_root_string_table(&strings);
    accumulator.append_infrastructure(
        Box::new(InfrastructureErrors::new(CompilerError::compiler_error(
            "cause",
        ))),
        Some(database),
    );
    let failure = accumulator.finish(InstallMode::Frozen);
    assert_eq!(failure.error_count(), 1);
    let infrastructure = failure
        .infrastructure_errors()
        .expect("infrastructure errors should survive freezing");
    let context = infrastructure
        .frozen_identity_for_cause(0)
        .expect("the infrastructure cause should retain its source identity");
    let path = context
        .source_logical_path(source)
        .expect("the cause source should retain its logical path");
    assert!(
        context
            .render_path(path, &mut Vec::new())
            .ends_with("main.moth")
    );
}

#[test]
fn prepared_batches_with_named_domains_share_their_database_identity() {
    let dir = tempfile::tempdir().expect("should create temporary directory");
    let mut strings = StringTable::new();
    let (database, source) = retained_database(dir.path(), "main.moth", "main", &mut strings);
    let name = strings.intern("missing");
    let make_batch = || {
        PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from(CompilerDiagnostic::unknown_value_name(
                name,
                Some(inline_span(source, 0, 4)),
            )),
            strings.clone(),
            None,
            false,
            None,
        )
    };
    let database = Arc::new(database);
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(
        make_batch(),
        Some((
            Arc::clone(&database),
            Some(StablePackageIdentity::project_local("pkg-a")),
        )),
    );
    accumulator.append_prepared(
        make_batch(),
        Some((
            database,
            Some(StablePackageIdentity::project_local("pkg-b")),
        )),
    );
    let failure = accumulator.finish(InstallMode::Frozen);
    assert_eq!(failure.reports.report(0).len(), 1);
    assert_eq!(failure.reports.report(1).len(), 1);
    assert!(std::ptr::eq(
        failure.reports.report(0).context(),
        failure.reports.report(1).context(),
    ));
    assert!(Arc::ptr_eq(
        &failure.reports.report(0).shared_context(),
        &failure.reports.report(1).shared_context(),
    ));
}

#[test]
fn prepared_batch_owning_site_capture_retains_type_and_path_contexts() {
    let mut strings = StringTable::new();
    let name = strings.intern("TOwningSite");
    let mut paths = PathInternerBuilder::new();
    let import_path = paths
        .try_intern_portable_path("pkg/owning-site-only.moth", &mut strings)
        .expect("path should intern");
    let path_table = Arc::new(paths.freeze());
    let mut environment = TypeEnvironment::new();
    let named_type = environment.register_synthetic_generic_parameter(name);
    let boolean = environment.builtins().bool;
    let diagnostics = vec![
        CompilerDiagnostic::type_mismatch(
            named_type,
            boolean,
            TypeMismatchContext::Assignment,
            None,
        ),
        CompilerDiagnostic::missing_import_target(import_path, None),
    ];
    let captured = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(diagnostics),
        strings,
        Some(&environment),
        true,
        Some(path_table),
    );
    let failure = finish_prepared_batches(vec![(captured, None)], InstallMode::Frozen);
    let rendered = frozen_render(&failure);
    assert_eq!(rendered.len(), 2);
    assert!(rendered[0].message.contains("TOwningSite"));
    assert!(rendered[1].message.contains("pkg/owning-site-only.moth"));
}

#[test]
fn prepared_batch_owning_site_capture_keeps_separate_environment_snapshots() {
    let mut run = BoundaryAccumulator::new();

    let mut first_strings = StringTable::new();
    let first_name = first_strings.intern("TFirstOwner");
    let mut first_environment = TypeEnvironment::new();
    let first_type = first_environment.register_synthetic_generic_parameter(first_name);
    let first_bool = first_environment.builtins().bool;
    let first_diagnostic = CompilerDiagnostic::type_mismatch(
        first_type,
        first_bool,
        TypeMismatchContext::Assignment,
        None,
    );
    let first_batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(first_diagnostic),
        first_strings,
        Some(&first_environment),
        true,
        Some(Arc::new(PathInternerBuilder::new().freeze())),
    );
    run.append_prepared(first_batch, None);

    let mut second_strings = StringTable::new();
    let second_name = second_strings.intern("TSecondOwner");
    let mut second_environment = TypeEnvironment::new();
    let second_type = second_environment.register_synthetic_generic_parameter(second_name);
    let second_bool = second_environment.builtins().bool;
    let second_diagnostic = CompilerDiagnostic::type_mismatch(
        second_type,
        second_bool,
        TypeMismatchContext::Assignment,
        None,
    );
    let second_batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(second_diagnostic),
        second_strings,
        Some(&second_environment),
        true,
        Some(Arc::new(PathInternerBuilder::new().freeze())),
    );
    run.append_prepared(second_batch, None);

    let rendered = frozen_render(&run.finish(InstallMode::Frozen));
    assert_eq!(rendered.len(), 2);
    assert!(rendered[0].message.contains("TFirstOwner"));
    assert!(!rendered[0].message.contains("TSecondOwner"));
    assert!(rendered[1].message.contains("TSecondOwner"));
    assert!(!rendered[1].message.contains("TFirstOwner"));
}

#[test]
fn prepared_batch_owning_site_capture_preserves_unresolved_type_ids() {
    let mut run = BoundaryAccumulator::new();
    let unavailable = CompilerDiagnostic::type_mismatch(
        TypeId(999),
        TypeId(1000),
        TypeMismatchContext::Assignment,
        None,
    );
    let unavailable_batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(unavailable),
        StringTable::new(),
        None,
        true,
        Some(Arc::new(PathInternerBuilder::new().freeze())),
    );
    run.append_prepared(unavailable_batch, None);

    let mut strings = StringTable::new();
    let name = strings.intern("TUnavailablePathTable");
    let mut environment = TypeEnvironment::new();
    let named_type = environment.register_synthetic_generic_parameter(name);
    let boolean = environment.builtins().bool;
    let unavailable_path_table = CompilerDiagnostic::type_mismatch(
        named_type,
        boolean,
        TypeMismatchContext::Assignment,
        None,
    );
    let unavailable_path_batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(unavailable_path_table),
        strings,
        Some(&environment),
        false,
        None,
    );
    run.append_prepared(unavailable_path_batch, None);

    let rendered = frozen_render(&run.finish(InstallMode::Frozen));
    assert_eq!(rendered.len(), 2);
    assert!(rendered[0].message.contains("TypeId(999)"));
    assert!(rendered[0].message.contains("TypeId(1000)"));
    assert!(rendered[1].message.contains("TypeId("));
    assert!(!rendered[1].message.contains("TUnavailablePathTable"));
}
#[test]
fn append_infrastructure_keeps_each_cause_in_its_own_source_domain() {
    let dir = tempfile::tempdir().expect("should create temporary directory");
    let mut strings = StringTable::new();
    let (first_database, first_source) =
        retained_database(dir.path(), "first.moth", "first", &mut strings);
    let (second_database, second_source) =
        retained_database(dir.path(), "second.moth", "second", &mut strings);

    let first_error = CompilerError::new(
        "first cause",
        Some(inline_span(first_source, 0, 5)),
        ErrorType::Compiler,
    );
    let second_error = CompilerError::new(
        "second cause",
        Some(inline_span(second_source, 0, 6)),
        ErrorType::Compiler,
    );
    let mut accumulator = BoundaryAccumulator::with_root_string_table(&strings);
    accumulator.append_infrastructure(
        Box::new(InfrastructureErrors::new(first_error)),
        Some(Arc::new(first_database)),
    );
    accumulator.append_infrastructure(
        Box::new(InfrastructureErrors::new(second_error)),
        Some(Arc::new(second_database)),
    );

    let failure = accumulator.finish(InstallMode::Frozen);
    let counts = failure.reports.counts();
    assert_eq!(counts.errors, 0);
    assert_eq!(counts.warnings, 0);
    assert_eq!(counts.notes, 0);
    assert_eq!(failure.error_count(), 1);
    assert_eq!(failure.warning_count(), 0);
    let errors = failure
        .infrastructure_errors()
        .expect("both infrastructure causes should survive freezing");
    let causes = errors.iter().collect::<Vec<_>>();
    assert_eq!(causes.len(), 2);
    assert_eq!(causes[0].msg, "first cause");
    assert_eq!(causes[1].msg, "second cause");
    for (index, source, expected_suffix) in [
        (0, first_source, "first.moth"),
        (1, second_source, "second.moth"),
    ] {
        let identity = errors
            .frozen_identity_for_cause(index)
            .expect("each cause should retain its own frozen source identity");
        let path = identity
            .source_logical_path(source)
            .expect("cause source should resolve in its own database");
        assert!(
            identity
                .render_path(path, &mut Vec::new())
                .ends_with(expected_suffix),
            "cause {index} should resolve through {expected_suffix}",
        );
    }
}

#[test]
fn prepared_local_secondary_label_keeps_its_exact_position() {
    let dir = tempfile::tempdir().expect("should create temporary directory");
    let mut strings = StringTable::new();
    let name = strings.intern("main");
    let (database, source) = retained_database(dir.path(), "main.moth", "main", &mut strings);
    let span = inline_span(source, 0, 4);
    let diagnostic = CompilerDiagnostic::duplicate_declaration(name, Some(span), Some(span));
    let batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        strings,
        None,
        false,
        None,
    );
    let failure = finish_prepared_batches(
        vec![(batch, Some((Arc::new(database), None)))],
        InstallMode::Frozen,
    );
    let labels = frozen_labels(&failure);
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].message, "previous declaration here");
    let position = labels[0]
        .position
        .as_ref()
        .expect("the local secondary label should resolve");
    assert_eq!(position.path, PathBuf::from("src/main.moth"));
    let expected_host_path = dir.path().join("src/main.moth");
    assert_eq!(
        position.host_path.as_deref(),
        Some(expected_host_path.as_path())
    );
    assert_eq!(position.line, "main");
    assert_eq!(position.start, (0, 0));
    assert_eq!(position.end, (0, 4));
}

#[test]
fn prepared_uninstalled_donor_label_keeps_its_message_without_a_position() {
    let dir = tempfile::tempdir().expect("should create temporary directory");
    let mut requester_strings = StringTable::new();
    let (requester_database, requester_source) = retained_database(
        dir.path(),
        "caller.moth",
        "caller\n",
        &mut requester_strings,
    );
    let name = requester_strings.intern("missing");

    let mut donor_strings = StringTable::new();
    let (donor_database, donor_source) =
        retained_database(dir.path(), "donor.moth", "prior\n", &mut donor_strings);
    assert_eq!(
        requester_source, donor_source,
        "the requester and donor fixtures should deliberately collide on source id",
    );
    drop(donor_database);

    let donor_handle =
        FrozenIdentityHandle::for_domain(StablePackageIdentity::project_local("uninstalled-label"));
    let diagnostic = CompilerDiagnostic::duplicate_declaration(
        name,
        None,
        Some(inline_span(requester_source, 0, 7)),
    )
    .with_labels(vec![DiagnosticLabel::secondary_with_frozen_identity(
        Some(inline_span(donor_source, 0, 5)),
        Some(DiagnosticLabelMessage::PreviousDeclaration),
        donor_handle.clone(),
    )]);
    let batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        requester_strings,
        None,
        false,
        None,
    );
    let failure = finish_prepared_batches(
        vec![(batch, Some((Arc::new(requester_database), None)))],
        InstallMode::Unfrozen,
    );

    assert!(donor_handle.get().is_none());
    let labels = frozen_labels(&failure);
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].message, "previous declaration here");
    assert!(
        labels[0].position.is_none(),
        "an uninstalled donor label must not fall back to the requester source",
    );
}

#[test]
fn prepared_module_fork_renders_inherited_and_local_names() {
    let mut root_strings = StringTable::new();
    let inherited_main = root_strings.intern("main");
    let (mut module_strings, _) = root_strings.fork_for_module().into_parts();
    let local_alpha = module_strings.intern("alpha");
    let batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(vec![
            CompilerDiagnostic::unknown_value_name(inherited_main, None),
            CompilerDiagnostic::unknown_value_name(local_alpha, None),
        ]),
        module_strings,
        None,
        false,
        None,
    );
    let failure = finish_prepared_batches(vec![(batch, None)], InstallMode::Frozen);

    let rendered = frozen_render(&failure);
    assert_eq!(rendered.len(), 2);
    assert!(rendered[0].message.contains("main"));
    assert!(rendered[1].message.contains("alpha"));
}

#[test]
fn prepared_identity_path_merge_keeps_the_diagnostic_path() {
    let dir = tempfile::tempdir().expect("should create temporary directory");
    let mut strings = StringTable::new();
    let (database, _) = retained_database(dir.path(), "main.moth", "main", &mut strings);
    let owner_path_count = database.clone_path_builder().len();
    let mut explicit_builder = database.clone_path_builder();
    let explicit_path = explicit_builder
        .try_intern_portable_path("main.moth", &mut strings)
        .expect("the explicit-only path should intern");
    assert!(explicit_builder.len() > owner_path_count);
    let explicit = Arc::new(explicit_builder.freeze());
    let identity_merge = database
        .clone_path_builder()
        .merge_snapshot(&explicit)
        .expect("the identity-extending snapshot should merge");
    assert!(identity_merge.is_identity());

    let batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(CompilerDiagnostic::missing_import_target(
            explicit_path,
            None,
        )),
        strings,
        None,
        true,
        Some(explicit),
    );
    let failure = finish_prepared_batches(
        vec![(batch, Some((Arc::new(database), None)))],
        InstallMode::Frozen,
    );
    let rendered = frozen_render(&failure);
    assert_eq!(rendered.len(), 1);
    assert!(rendered[0].message.contains("main.moth"));
}
