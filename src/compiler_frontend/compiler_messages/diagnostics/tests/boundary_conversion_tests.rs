//! Behavioural tests for prepared diagnostics crossing the frozen boundary.
//!
//! These tests exercise owner-edge capture, direct accumulator appends, remapping, and frozen
//! rendering of prepared diagnostic batches.

use crate::compiler_frontend::compiler_messages::CompilerStage;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureErrors, InfrastructureFailure,
    InfrastructureFailureKind,
};
use crate::compiler_frontend::compiler_messages::diagnostic_bag::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot_builder::ForcedTypeDisplayExhaustionGuard;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::validate_preparation_draft_spans;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    CompilerCapacityExceededView, SourceSpanCapacityView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    choices_control_flow,
    imports::{self, missing_import_target},
    names_declarations::{self, duplicate_declaration, unknown_value_name},
    token_lexical::{self, source_span_capacity},
    types_borrows_targets::{self, TypeMismatchView, type_mismatch},
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::{
    DurableLabelMessage, LabelDraftMessage,
};
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::{
    compact_bag_for_test, exact_span_in_source,
};
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticPrimaryPosition, DiagnosticRenderContext, compact_label_message_text, terminal, terse,
};
use crate::compiler_frontend::compiler_messages::{
    BuiltinFailureOriginKind, BuiltinFailureWitness, CapacityResource, DiagnosticCapacityResource,
    DiagnosticSeverity, InvalidFallibleHandlingReason, PreparedFailure, SourceSpanCapacityResource,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::definitions::StructTypeDefinition;
use crate::compiler_frontend::datatypes::display::TypeDisplayShape;
use crate::compiler_frontend::datatypes::display::TypeDisplayView;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId, builtin_type_ids};
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityHandle, LocalSpan, SourceDatabase, SourceId, SourceSpan,
};
use crate::compiler_frontend::symbols::path_interner::{
    ForcedExhaustionGuard, PathId, PathInternerBuilder, PathInternerFork,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

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

fn diagnosed_reports(
    result: &CompilationResult<()>,
) -> &crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet {
    match result {
        Ok(CompilationOutcome::Diagnosed(reports)) => reports,
        other => panic!("expected diagnosed result, got {other:?}"),
    }
}

fn frozen_render(result: &CompilationResult<()>) -> Vec<Rendered> {
    diagnosed_reports(result)
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

fn frozen_labels(result: &CompilationResult<()>) -> Vec<RenderedLabel> {
    let mut labels = Vec::new();
    for (report, view) in diagnosed_reports(result).iter() {
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
) -> CompilationResult<()> {
    let mut accumulator = BoundaryAccumulator::new();
    for (batch, source) in batches {
        accumulator.append_prepared(batch, source);
    }
    accumulator
        .finish(install)
        .map(CompilationOutcome::Diagnosed)
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
        let (mut database, source) =
            retained_database(dir.path(), "main.moth", "main", &mut base_strings);

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
                    .fork_path_builder()
                    .merge_snapshot(explicit)
                    .expect("explicit paths should merge into the owner")
                    .is_identity(),
                "each explicit path table should require a path-id remap"
            );
        }
        assert!(!Arc::ptr_eq(&alpha_explicit, &beta_explicit));
        let database = Arc::new(database);

        let make_batch = |strings, name, import_path, explicit, unknown_name| {
            let mut environment = TypeEnvironment::new();
            let generic_type = environment.register_synthetic_generic_parameter(name);
            let boolean = environment.builtins().bool;
            let mut diagnostics = vec![
                type_mismatch(
                    generic_type,
                    boolean,
                    TypeMismatchContext::Assignment,
                    Some(inline_span(source, 0, 4)),
                ),
                missing_import_target(import_path, Some(inline_span(source, 0, 4))),
            ];
            if let Some(name) = unknown_name {
                diagnostics.push(unknown_value_name(name, Some(inline_span(source, 0, 4))));
            }
            PreparedDiagnosticBatch::capture_from_bag(
                DiagnosticBag::from_diagnostics(diagnostics),
                strings,
                Some(&environment),
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

        let (mut database, source) =
            retained_database(dir.path(), "main.moth", "main", &mut strings);
        for explicit in [&shared_table, &distinct_table] {
            assert!(
                !database
                    .fork_path_builder()
                    .merge_snapshot(explicit)
                    .expect("explicit paths should merge into the owner")
                    .is_identity(),
                "explicit table should require a path-id remap"
            );
        }
        let database = Arc::new(database);
        let make_batch = |path, path_table| {
            PreparedDiagnosticBatch::capture_from_bag(
                DiagnosticBag::from(missing_import_target(path, Some(inline_span(source, 0, 4)))),
                strings.clone(),
                None,
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
        let diagnostic = unknown_value_name(name, Some(inline_span(donor_source, 0, 5)))
            .with_primary_frozen_identity_handle(handle);
        let diagnosed = PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from(diagnostic),
            strings.clone(),
            None,
            None,
        );
        let empty =
            PreparedDiagnosticBatch::capture_from_bag(DiagnosticBag::new(), strings, None, None);
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
fn diagnosed_owner_without_a_type_environment_keeps_unresolved_type_ids() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut strings = StringTable::new();
    let (database, source) = retained_database(dir.path(), "main.moth", "main", &mut strings);
    let diagnostic = type_mismatch(
        TypeId(999),
        TypeId(1000),
        TypeMismatchContext::Assignment,
        Some(inline_span(source, 0, 4)),
    );
    let failure =
        PreparedFailure::capture_diagnosed(DiagnosticBag::from(diagnostic), strings, None, None);
    let PreparedFailure::Diagnosed(batch) = failure else {
        panic!("an owner without a type environment still retains a diagnosed batch");
    };
    let failure = finish_prepared_batches(
        vec![(batch, Some((Arc::new(database), None)))],
        InstallMode::Frozen,
    );
    let rendered = frozen_render(&failure);
    assert_eq!(
        &*rendered[0].message,
        "Type mismatch in assignment: expected TypeId(999), found TypeId(1000)",
    );
}

#[test]
fn prepared_batch_type_display_capacity_fault_keeps_the_source_span_and_code() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let _exhausted = ForcedTypeDisplayExhaustionGuard::new();
    let mut strings = StringTable::new();
    let (database, source) = retained_database(dir.path(), "main.moth", "main", &mut strings);
    let diagnostic = type_mismatch(
        TypeId(999),
        TypeId(1000),
        TypeMismatchContext::Assignment,
        Some(inline_span(source, 0, 4)),
    );
    let batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        strings,
        None,
        None,
    );
    let failure = finish_prepared_batches(
        vec![(batch, Some((Arc::new(database), None)))],
        InstallMode::Frozen,
    );
    let rendered = frozen_render(&failure);
    assert_eq!(
        &*rendered[0].message,
        "This source exceeds the compiler's capacity for diagnostic type displays."
    );
    let (report, view) = diagnosed_reports(&failure)
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
            DiagnosticBag::from(missing_import_target(
                new_path,
                Some(inline_span(source, 0, 4)),
            )),
            strings,
            None,
            Some(explicit),
        );
        vec![(batch, Some((Arc::new(database), None)))]
    };
    let mut expected_strings = StringTable::new();
    let (expected_database, expected_source) =
        retained_database(dir.path(), "main.moth", "main", &mut expected_strings);
    let expected_batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(source_span_capacity(
            0,
            u32::MAX,
            SourceSpanCapacityResource::LogicalPath,
            Some(inline_span(expected_source, 0, 4)),
        )),
        expected_strings,
        None,
        None,
    );
    let expected_failure = finish_prepared_batches(
        vec![(expected_batch, Some((Arc::new(expected_database), None)))],
        InstallMode::Frozen,
    );
    let expected_render = frozen_render(&expected_failure);

    let batches = make_batches();
    let _exhausted = ForcedExhaustionGuard::new();
    let failure = finish_prepared_batches(batches, InstallMode::Frozen);
    let rendered = frozen_render(&failure);
    assert_eq!(rendered.len(), 1);
    assert_eq!(diagnosed_reports(&failure).report(0).len(), 1);
    assert_eq!(&*rendered[0].message, expected_render[0].message);
    assert_eq!(rendered[0].position, expected_render[0].position);
    let position = rendered[0]
        .position
        .as_ref()
        .expect("path capacity replacement should retain its source location");
    assert_eq!(position.path, PathBuf::from("src/main.moth"));
    assert_eq!(position.line, "main");
    assert_eq!(position.start, (0, 0));
    assert_eq!(position.end, (0, 4));

    let expected_code = diagnosed_reports(&expected_failure)
        .iter()
        .next()
        .expect("the baseline capacity report is present")
        .1
        .descriptor()
        .code;
    let (report, view) = diagnosed_reports(&failure)
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
            unknown_value_name(first_name, None),
            unknown_value_name(
                second_name,
                Some(inline_span(SourceId::from_index(1), 0, 4)),
            ),
            missing_import_target(import_path, None),
        ]),
        strings,
        None,
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
fn prepared_batches_with_named_domains_share_their_database_identity() {
    let dir = tempfile::tempdir().expect("should create temporary directory");
    let mut strings = StringTable::new();
    let (database, source) = retained_database(dir.path(), "main.moth", "main", &mut strings);
    let name = strings.intern("missing");
    let make_batch = || {
        PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from(unknown_value_name(name, Some(inline_span(source, 0, 4)))),
            strings.clone(),
            None,
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
    let failure = accumulator
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed);
    let reports = diagnosed_reports(&failure);
    assert_eq!(reports.report(0).len(), 1);
    assert_eq!(reports.report(1).len(), 1);
    assert!(std::ptr::eq(
        reports.report(0).context(),
        reports.report(1).context(),
    ));
    assert!(Arc::ptr_eq(
        &reports.report(0).shared_context(),
        &reports.report(1).shared_context(),
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
        type_mismatch(named_type, boolean, TypeMismatchContext::Assignment, None),
        missing_import_target(import_path, None),
    ];
    let captured = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from_diagnostics(diagnostics),
        strings,
        Some(&environment),
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
    let first_diagnostic = type_mismatch(
        first_type,
        first_bool,
        TypeMismatchContext::Assignment,
        None,
    );
    let first_batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(first_diagnostic),
        first_strings,
        Some(&first_environment),
        Some(Arc::new(PathInternerBuilder::new().freeze())),
    );
    run.append_prepared(first_batch, None);

    let mut second_strings = StringTable::new();
    let second_name = second_strings.intern("TSecondOwner");
    let mut second_environment = TypeEnvironment::new();
    let second_type = second_environment.register_synthetic_generic_parameter(second_name);
    let second_bool = second_environment.builtins().bool;
    let second_diagnostic = type_mismatch(
        second_type,
        second_bool,
        TypeMismatchContext::Assignment,
        None,
    );
    let second_batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(second_diagnostic),
        second_strings,
        Some(&second_environment),
        Some(Arc::new(PathInternerBuilder::new().freeze())),
    );
    run.append_prepared(second_batch, None);

    let result = run
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed);
    let rendered = frozen_render(&result);
    assert_eq!(rendered.len(), 2);
    assert!(rendered[0].message.contains("TFirstOwner"));
    assert!(!rendered[0].message.contains("TSecondOwner"));
    assert!(rendered[1].message.contains("TSecondOwner"));
    assert!(!rendered[1].message.contains("TFirstOwner"));
}

#[test]
fn prepared_local_secondary_label_keeps_its_exact_position() {
    let dir = tempfile::tempdir().expect("should create temporary directory");
    let mut strings = StringTable::new();
    let name = strings.intern("main");
    let (database, source) = retained_database(dir.path(), "main.moth", "main", &mut strings);
    let span = inline_span(source, 0, 4);
    let diagnostic = duplicate_declaration(name, Some(span))
        .with_label(Some(span), Some(LabelDraftMessage::PreviousDeclaration));
    let batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        strings,
        None,
        None,
    );
    let failure = finish_prepared_batches(
        vec![(batch, Some((Arc::new(database), None)))],
        InstallMode::Frozen,
    );
    let labels = frozen_labels(&failure);
    assert_eq!(labels.len(), 1);
    assert_eq!(&*labels[0].message, "previous declaration here");
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
    let diagnostic = duplicate_declaration(name, Some(inline_span(requester_source, 0, 7)))
        .with_label(
            Some(inline_span(donor_source, 0, 5)),
            Some(LabelDraftMessage::PreviousDeclaration),
        )
        .with_label_frozen_identity_handle(0, donor_handle.clone());
    let batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(diagnostic),
        requester_strings,
        None,
        None,
    );
    let failure = finish_prepared_batches(
        vec![(batch, Some((Arc::new(requester_database), None)))],
        InstallMode::Unfrozen,
    );

    assert!(donor_handle.get().is_none());
    let labels = frozen_labels(&failure);
    assert_eq!(labels.len(), 1);
    assert_eq!(&*labels[0].message, "previous declaration here");
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
            unknown_value_name(inherited_main, None),
            unknown_value_name(local_alpha, None),
        ]),
        module_strings,
        None,
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
    let (mut database, _) = retained_database(dir.path(), "main.moth", "main", &mut strings);
    let owner_path_count = database.fork_path_builder().len();
    let mut explicit_builder = database.fork_path_builder();
    let explicit_path = explicit_builder
        .try_intern_portable_path("main.moth", &mut strings)
        .expect("the explicit-only path should intern");
    assert!(explicit_builder.len() > owner_path_count);
    let explicit = Arc::new(explicit_builder.freeze());
    let identity_merge = database
        .fork_path_builder()
        .merge_snapshot(&explicit)
        .expect("the identity-extending snapshot should merge");
    assert!(identity_merge.is_identity());

    let batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(missing_import_target(explicit_path, None)),
        strings,
        None,
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

fn assert_infrastructure_causes(errors: &InfrastructureErrors, expected: &[InfrastructureFailure]) {
    let causes = errors.iter().collect::<Vec<_>>();
    assert_eq!(causes.len(), expected.len());
    for (cause, expected) in causes.into_iter().zip(expected) {
        assert_eq!(cause.kind, expected.kind);
        assert_eq!(cause.message, expected.message);
        assert_eq!(cause.stage, expected.stage);
        assert_eq!(cause.host_path, expected.host_path);
        assert_eq!(cause.details, expected.details);
    }
}

#[test]
fn prepared_infrastructure_failure_discards_unrelated_warnings() {
    let mut string_table = StringTable::new();
    let warning = names_declarations::unknown_value_name(string_table.intern("maybe_unused"), None)
        .with_severity(DiagnosticSeverity::Warning);
    let batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(warning),
        string_table,
        None,
        None,
    );
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(batch, None);
    accumulator.append_infrastructure(
        InfrastructureFailure::internal(
            CompilerStage::DiagnosticReporting,
            "backend failed after warnings",
        )
        .into(),
    );
    let failure: CompilationResult<()> = accumulator
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed);
    let Err(errors) = failure else {
        panic!("infrastructure failure must discard unrelated warning reports");
    };
    assert_infrastructure_causes(
        &errors,
        &[InfrastructureFailure::internal(
            CompilerStage::DiagnosticReporting,
            "backend failed after warnings",
        )],
    );
}

#[test]
fn append_infrastructure_preserves_every_ordered_cause() {
    for (first_type, second_type) in [
        (
            InfrastructureFailureKind::Internal,
            InfrastructureFailureKind::Filesystem,
        ),
        (
            InfrastructureFailureKind::Filesystem,
            InfrastructureFailureKind::Internal,
        ),
        (
            InfrastructureFailureKind::Filesystem,
            InfrastructureFailureKind::Filesystem,
        ),
    ] {
        let make_error = |index: usize, kind| {
            let stage = [
                CompilerStage::SourceLoading,
                CompilerStage::HeaderParsing,
                CompilerStage::AstConstruction,
                CompilerStage::Output,
            ][index];
            let message = format!("cause {index}");
            let path = format!("cause_{index}.moth");
            let mut error = match kind {
                InfrastructureFailureKind::Filesystem => {
                    InfrastructureFailure::filesystem(stage, Path::new(&path), message)
                }
                _ => InfrastructureFailure::internal(stage, message),
            };
            error.host_path = Some(Path::new(&path).into());
            error
        };
        let expected = [
            make_error(0, first_type),
            make_error(1, second_type),
            make_error(2, second_type),
            make_error(3, first_type),
        ];
        let mut first = InfrastructureErrors::new(expected[0].clone());
        first.push(expected[1].clone());
        let mut incoming = InfrastructureErrors::new(expected[2].clone());
        incoming.push(expected[3].clone());
        let mut accumulator = BoundaryAccumulator::new();
        accumulator.append_infrastructure(Box::new(first));
        accumulator.append_infrastructure(Box::new(incoming));
        let errors = accumulator
            .finish(InstallMode::Frozen)
            .expect_err("infrastructure causes remain on the outer error lane");
        assert_infrastructure_causes(&errors, &expected);
    }
}

#[test]
fn capture_with_required_paths_rejects_foreign_domain_table_behind_rendered_path_payload() {
    let mut batch_table = StringTable::new();
    batch_table.intern("batch");
    batch_table.intern("second");
    let mut batch_fork = PathInternerFork::empty();
    let batch_leaf = batch_fork
        .try_intern_portable_path("batch/import", &mut batch_table)
        .expect("batch path fits");
    let draft = imports::missing_import_target(batch_leaf, None);

    let mut foreign_strings = StringTable::new();
    foreign_strings.intern("foreign-pad-a");
    foreign_strings.intern("foreign-pad-b");
    foreign_strings.intern("foreign-pad-c");
    let mut foreign_fork = PathInternerFork::empty();
    foreign_fork
        .try_intern_portable_path("foreign/import", &mut foreign_strings)
        .expect("foreign path fits");
    let foreign_table = Arc::new(foreign_fork.snapshot_table());
    let error = PreparedDiagnosticBatch::capture_with_required_paths(
        DiagnosticBag::from(draft),
        batch_table,
        None,
        || Some(foreign_table),
    )
    .expect_err("a foreign table behind a rendered path must be rejected");
    assert!(
        error.message.contains("path node"),
        "pairing should identify the unresolved path node: {}",
        error.message
    );
}

#[test]
fn capture_with_required_paths_rejects_rendered_path_beyond_candidate_table() {
    let batch_table = StringTable::new();
    let orphan_path = PathId::try_from_index(9).expect("index 9 fits the compact path domain");
    let draft = imports::missing_import_target(orphan_path, None);
    let mut local_strings = StringTable::new();
    let mut local_fork = PathInternerFork::empty();
    local_fork
        .try_intern_portable_path("local/import", &mut local_strings)
        .expect("local path fits");
    let local_table = Arc::new(local_fork.snapshot_table());
    assert!(
        PreparedDiagnosticBatch::capture_with_required_paths(
            DiagnosticBag::from(draft),
            batch_table,
            None,
            || Some(local_table),
        )
        .is_err(),
        "a retained path outside the candidate table must fail pairing"
    );
}

#[test]
fn capture_with_required_paths_skips_foreign_table_without_path_payloads() {
    let mut batch_table = StringTable::new();
    let name = batch_table.intern("local");
    let draft = names_declarations::unknown_value_name(name, None);
    let mut foreign_strings = StringTable::new();
    let mut foreign_fork = PathInternerFork::empty();
    foreign_fork
        .try_intern_portable_path("foreign/import", &mut foreign_strings)
        .expect("foreign path fits");
    let foreign_table = Arc::new(foreign_fork.snapshot_table());
    let mut requested_path_table = false;
    let prepared = PreparedDiagnosticBatch::capture_with_required_paths(
        DiagnosticBag::from(draft),
        batch_table,
        None,
        || {
            requested_path_table = true;
            Some(foreign_table)
        },
    )
    .expect("a path-free diagnostic does not need a candidate table");
    assert!(
        !requested_path_table,
        "path-free diagnostics must not request or retain a caller-supplied path table"
    );

    let mut aggregate_table = StringTable::new();
    aggregate_table.intern("aggregate-padding");
    let mut accumulator = BoundaryAccumulator::with_root_string_table(aggregate_table);
    accumulator.append_prepared(prepared, None);
    let failure: CompilationResult<()> = accumulator
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed);
    let rendered = terse::format_terse_compilation_result(&failure);
    assert!(
        rendered.iter().any(|line| line.contains("local")),
        "path-free rendering lost its authored name after aggregation: {rendered:?}"
    );
}

#[test]
fn capture_with_required_paths_rejects_unresolvable_unused_path_table_node() {
    let mut batch_table = StringTable::new();
    batch_table.intern("local");
    batch_table.intern("leaf");
    let mut batch_fork = PathInternerFork::empty();
    let required_path = batch_fork
        .try_intern_portable_path("local/leaf", &mut batch_table)
        .expect("required path fits");
    let draft = imports::missing_import_target(required_path, None);
    let mut candidate_strings = StringTable::new();
    candidate_strings.intern("local");
    candidate_strings.intern("leaf");
    candidate_strings.intern("unused");
    candidate_strings.intern("extra");
    let mut candidate_fork = PathInternerFork::empty();
    let candidate_path = candidate_fork
        .try_intern_portable_path("local/leaf", &mut candidate_strings)
        .expect("candidate required path fits");
    candidate_fork
        .try_intern_portable_path("unused/extra", &mut candidate_strings)
        .expect("candidate unused path fits");
    assert_eq!(
        candidate_path, required_path,
        "valid required path exercises same node identity"
    );
    let error = PreparedDiagnosticBatch::capture_with_required_paths(
        DiagnosticBag::from(draft),
        batch_table,
        None,
        || Some(Arc::new(candidate_fork.snapshot_table())),
    )
    .expect_err("an unresolvable unused node must reject a retained table");
    assert!(
        error.message.contains("path node"),
        "producer boundary should identify invalid retained node: {}",
        error.message
    );
}

#[test]
fn capture_from_bag_preserves_nominal_type_rendering_without_path_payloads() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let nominal_path = path_fork
        .try_intern_portable_path("NominalType", &mut string_table)
        .expect("nominal path fits");
    let mut type_environment = TypeEnvironment::new();
    let (_, nominal_type) = type_environment.register_nominal_struct(StructTypeDefinition {
        id: NominalTypeId(0),
        path: nominal_path,
        fields: Box::new([]),
        generic_parameters: None,
        const_record: false,
    });
    let draft = types_borrows_targets::type_mismatch(
        nominal_type,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        TypeMismatchContext::Assignment,
        None,
    );
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(draft),
        string_table,
        Some(&type_environment),
        Some(Arc::new(path_fork.snapshot_table())),
    );
    let mut aggregate_table = StringTable::new();
    aggregate_table.intern("aggregate-padding");
    let mut accumulator = BoundaryAccumulator::with_root_string_table(aggregate_table);
    accumulator.append_prepared(prepared, None);
    let failure = accumulator
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed);
    let (_, view) = diagnosed_reports(&failure)
        .iter()
        .next()
        .expect("one prepared mismatch freezes");
    let mismatch = view
        .family_view::<TypeMismatchView>()
        .expect("type-mismatch facts remain typed");
    assert_eq!(mismatch.context, TypeMismatchContext::Assignment);
    assert_eq!(
        view.types().shape(mismatch.expected),
        TypeDisplayShape::Struct {
            path: nominal_path,
            const_record: false
        }
    );
    assert_eq!(
        view.types().shape(mismatch.found),
        TypeDisplayShape::FixedScalar(FixedScalar::I32)
    );
    let rendered = terse::format_terse_compilation_result(&failure);
    assert!(
        rendered
            .iter()
            .any(|line| line
                .contains("Type mismatch in assignment: expected NominalType, found I32")),
        "nominal rendering lost context after remap: {rendered:?}"
    );
}

#[test]
fn capture_with_required_paths_rejects_unresolved_rendered_ancestor() {
    let mut batch_table = StringTable::new();
    batch_table.intern("batch-padding");
    batch_table.intern("leaf");
    let mut foreign_strings = StringTable::new();
    foreign_strings.intern("foreign-padding");
    foreign_strings.intern("leaf");
    foreign_strings.intern("ancestor");
    let mut foreign_fork = PathInternerFork::empty();
    let rendered_leaf = foreign_fork
        .try_intern_portable_path("ancestor/leaf", &mut foreign_strings)
        .expect("foreign path fits");
    foreign_fork
        .try_intern_portable_path("ancestor/leaf/unreferenced", &mut foreign_strings)
        .expect("unreferenced path fits");
    let draft = imports::missing_import_target(rendered_leaf, None);
    assert!(
        PreparedDiagnosticBatch::capture_with_required_paths(
            DiagnosticBag::from(draft),
            batch_table,
            None,
            || Some(Arc::new(foreign_fork.snapshot_table())),
        )
        .is_err(),
        "an unresolved rendered ancestor must fail pairing"
    );
}

#[test]
fn preparation_capture_validates_source_identity_without_encoding_or_rebinding() {
    let source = SourceId::from_index(5);
    let other_source = SourceId::from_index(6);
    let mut builder = ExtendedSpanBuilder::new();
    let primary = exact_span_in_source(source, 10, 4, &mut builder);
    let related = exact_span_in_source(source, 30, 3, &mut builder);
    let draft =
        token_lexical::unterminated_string_literal(Some(primary)).with_label(Some(related), None);
    validate_preparation_draft_spans(&draft, source)
        .expect("spans from the prepared source validate");
    assert_eq!(draft.primary_span(), Some(primary));
    assert_eq!(draft.label_span(0), Some(related));
    let error = validate_preparation_draft_spans(&draft, other_source)
        .expect_err("foreign source span is rejected");
    assert_eq!(error.kind, InfrastructureFailureKind::Internal);
    assert_eq!(draft.primary_span(), Some(primary));
    assert_eq!(draft.label_span(0), Some(related));
}

#[test]
fn preparation_capture_allows_spanless_diagnostics() {
    let draft = token_lexical::unterminated_string_literal(None);
    validate_preparation_draft_spans(&draft, SourceId::from_index(7))
        .expect("spanless diagnostics need no synthesized provenance");
    let compact = compact_bag_for_test(DiagnosticBag::from(draft), None);
    let view = compact.first().expect("spanless draft compacts");
    assert!(matches!(view.primary_site(), SiteRef::Absent));
    assert_eq!(view.durable_labels().count(), 0);
}

#[test]
fn failure_writeback_label_keeps_target_type_in_frozen_prepared_output() {
    let mut string_table = StringTable::new();
    let source_path = Path::new("/project/writeback.moth");
    let mut sources = SourceDatabase::build(
        std::iter::once(source_path),
        source_path,
        None,
        &mut string_table,
    )
    .expect("test source identity should build");
    let source = sources
        .get_by_canonical_path(source_path)
        .expect("registered source")
        .id;
    sources
        .retain_text(source, "total += amount\n".to_owned())
        .expect("source text");
    let mut builder = ExtendedSpanBuilder::new();
    let origin_span = exact_span_in_source(source, 0, 5, &mut builder);
    let type_environment = TypeEnvironment::new();
    let target = builtin_type_ids::fixed_scalar(FixedScalar::U8);
    let draft = choices_control_flow::invalid_fallible_handling(
        InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction {
            witness: Box::new(BuiltinFailureWitness {
                call_spans: vec![],
                origin_span: Some(origin_span),
                origin: BuiltinFailureOriginKind::CompoundWriteBack { target },
                elided_call_hops: 0,
            }),
        },
        None,
    )
    .with_label(
        Some(origin_span),
        Some(LabelDraftMessage::BuiltinWriteBackOrigin {
            target_type_id: target,
        }),
    );
    let prepared = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(draft),
        string_table,
        Some(&type_environment),
        Some(Arc::new(sources.paths().clone())),
    );
    let mut aggregate_table = StringTable::new();
    aggregate_table.intern("aggregate-padding");
    let mut accumulator = BoundaryAccumulator::with_root_string_table(aggregate_table);
    accumulator.append_prepared(prepared, Some((Arc::new(sources), None)));
    let failure = accumulator
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed);
    let mut found_writeback_type = false;
    for (_, view) in diagnosed_reports(&failure).iter() {
        for label in view.durable_labels() {
            if let DurableLabelMessage::BuiltinWriteBackOrigin { target_type } = label.message {
                assert_eq!(
                    view.types().shape(target_type),
                    TypeDisplayShape::FixedScalar(FixedScalar::U8)
                );
                found_writeback_type = true;
            }
        }
    }
    assert!(
        found_writeback_type,
        "frozen diagnostic retains write-back target type"
    );
    let labels = diagnosed_reports(&failure)
        .iter()
        .flat_map(|(report, view)| {
            terminal::format_label_messages_view(view, DiagnosticRenderContext::for_report(report))
        })
        .collect::<Vec<_>>();
    assert!(
        labels.iter().any(|label| label.contains("U8")),
        "{labels:?}"
    );
}
