//! Boundary conversion tests for `into_boundary_failure`.
//!
//! WHAT: each test builds a legacy vessel, renders it through the legacy context, converts it, and renders the
//! frozen result. Positions, messages, guidance, and secondary labels must agree field by field.
//! WHY: the converter is the only bridge between the legacy and frozen render paths. A silent divergence would
//! change diagnostic output without any other test noticing.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::exact_span;
use crate::compiler_frontend::compiler_messages::compiler_errors::{
    BoundaryFailure, CompilerError, CompilerMessages, RenderPathContext, RenderSourceContext,
    RenderTypeContext,
};
use crate::compiler_frontend::compiler_messages::diagnostic_bag::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot_builder::ForcedTypeDisplayExhaustionGuard;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::{
    BoundaryAccumulator, compact_render_for_test, into_boundary_failure,
};
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticPrimaryPosition, DiagnosticRenderContext, compact_label_message_text,
    resolve_label_render_facts_from_root,
};
use crate::compiler_frontend::compiler_messages::{
    BorrowAccessKind, BorrowDiagnosticKind, CompilerDiagnostic, DiagnosticKind, DiagnosticLabel,
    DiagnosticLabelMessage, DiagnosticPayload, DiagnosticPlace, PremergeDiagnosticBatch,
    SourceSpanCapacityResource, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityContext, FrozenIdentityHandle, LocalSpan, SourceDatabase,
    SourceId, SourceSpan,
};
use crate::compiler_frontend::symbols::path_interner::{
    ForcedExhaustionGuard, PathInternerBuilder,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// One rendered diagnostic, projected to owned data so legacy and frozen results compare directly.
#[derive(Debug)]
struct Rendered {
    position: Option<OwnedPosition>,
    message: String,
    guidance: Vec<String>,
    labels: Vec<RenderedLabel>,
}

/// One secondary label as the render path reads it: its text and its resolved position.
///
/// Every label is kept, in declaration order, so a label without a position compares as `None` on both sides.
#[derive(Debug, PartialEq)]
struct RenderedLabel {
    text: Option<String>,
    position: Option<OwnedPosition>,
}

/// The primary position as the render path reports it. Coordinates are zero-based scalar columns.
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

/// Renders every legacy diagnostic through its own legacy context, the path the boundary must match.
fn legacy_render(messages: &CompilerMessages) -> Vec<Rendered> {
    messages
        .diagnostic_slice()
        .iter()
        .enumerate()
        .map(|(index, diagnostic)| {
            let context = messages.diagnostic_render_context(index);
            let position = context
                .primary_position(diagnostic)
                .map(|position| owned_position(&position));
            let compact = compact_render_for_test(diagnostic, context);
            let view = compact.view();
            let render = view
                .family()
                .render
                .expect("matched family rows declare a renderer");
            let payload = render(&view, context);
            Rendered {
                position,
                message: payload.message,
                guidance: payload.guidance,
                labels: legacy_labels(diagnostic, context),
            }
        })
        .collect()
}

/// Reads each secondary label's legacy text and position, in declaration order.
fn legacy_labels(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
) -> Vec<RenderedLabel> {
    legacy_label_texts(diagnostic, context)
        .into_iter()
        .zip(&diagnostic.labels)
        .map(|(text, label)| RenderedLabel {
            text,
            position: context
                .label_position(label)
                .map(|position| owned_position(&position)),
        })
        .collect()
}

/// Reads each label's legacy text through the shared label resolver, one label at a time.
///
/// The resolver drops a spanned label with no position, but the projection keeps every label. So each label is
/// resolved from a spanless copy holding only that label. A spanless label keeps its text, which is the text the
/// rendered list shows.
fn legacy_label_texts(
    diagnostic: &CompilerDiagnostic,
    context: DiagnosticRenderContext<'_>,
) -> Vec<Option<String>> {
    let mut single = diagnostic.clone();
    diagnostic
        .labels
        .iter()
        .map(|label| {
            single.labels = vec![DiagnosticLabel {
                span: None,
                ..label.clone()
            }];
            // The copy has no span, so the resolver never reads the root for it.
            resolve_label_render_facts_from_root(&single, context, Path::new(""))
                .into_iter()
                .next()
                .map(|fact| fact.message)
        })
        .collect()
}

/// Renders every frozen diagnostic through the report that owns it, the path the frozen renderers use.
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
                guidance: payload.guidance,
                labels: frozen_labels(view, context),
            }
        })
        .collect()
}

/// Reads each secondary label's frozen text and position through the report that owns it, in declaration order.
fn frozen_labels<'a>(
    view: DiagnosticView<'a>,
    context: DiagnosticRenderContext<'a>,
) -> Vec<RenderedLabel> {
    let types = view.types();
    view.durable_labels()
        .map(|label| RenderedLabel {
            // Like the legacy resolver, keep only non-empty text: an empty text reads as `None`.
            text: compact_label_message_text(label.message, types, context)
                .filter(|text| !text.is_empty()),
            position: context
                .site_position(label.site)
                .map(|position| owned_position(&position)),
        })
        .collect()
}

/// Asserts that the frozen render matches the legacy render field by field.
fn assert_render_parity(legacy: &[Rendered], frozen: &[Rendered]) {
    assert_eq!(
        frozen.len(),
        legacy.len(),
        "frozen render should cover every diagnostic"
    );
    for (frozen_row, legacy_row) in frozen.iter().zip(legacy) {
        assert_eq!(
            frozen_row.position, legacy_row.position,
            "primary position should match the legacy render"
        );
        assert_eq!(
            frozen_row.message, legacy_row.message,
            "message should match the legacy render"
        );
        assert_eq!(
            frozen_row.guidance, legacy_row.guidance,
            "guidance should match the legacy render"
        );
        assert_eq!(
            frozen_row.labels, legacy_row.labels,
            "secondary labels should match the legacy render"
        );
    }
}

/// Converts `messages` and asserts that the frozen render equals the legacy render of the same vessel.
fn assert_legacy_parity(mut messages: CompilerMessages, install: InstallMode) -> Vec<Rendered> {
    let legacy = legacy_render(&messages);
    if install == InstallMode::Frozen {
        messages.mark_frozen();
    }
    let failure = into_boundary_failure(messages);
    let frozen = frozen_render(&failure);
    assert_render_parity(&legacy, &frozen);
    frozen
}

/// Convert a vessel in frozen mode after recording that mode on the vessel.
fn into_frozen_boundary_failure(mut messages: CompilerMessages) -> BoundaryFailure {
    messages.mark_frozen();
    into_boundary_failure(messages)
}

/// Compares one accumulator run with the canonical merge-then-convert path.
fn assert_accumulator_matches_merge(
    make_vessels: impl Fn() -> (CompilerMessages, CompilerMessages),
    install: InstallMode,
    expected_record_counts: (u32, u32, u32),
    expected_error_count: usize,
    expected_infrastructure_causes: usize,
) -> (BoundaryFailure, BoundaryFailure) {
    let (first, second) = make_vessels();
    let mut merged = first;
    merged.append_messages_preserving_context(second);
    if install == InstallMode::Frozen {
        merged.mark_frozen();
    }
    let merged_failure = into_boundary_failure(merged);

    let (first, second) = make_vessels();
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append(first);
    accumulator.append(second);
    let accumulated_failure = accumulator.finish(install);

    let merged_render = frozen_render(&merged_failure);
    let accumulated_render = frozen_render(&accumulated_failure);
    assert_render_parity(&merged_render, &accumulated_render);

    let counts = |failure: &BoundaryFailure| {
        let counts = failure.reports.counts();
        (counts.errors, counts.warnings, counts.notes)
    };
    assert_eq!(counts(&merged_failure), expected_record_counts);
    assert_eq!(counts(&accumulated_failure), expected_record_counts);
    assert_eq!(merged_failure.error_count(), expected_error_count);
    assert_eq!(accumulated_failure.error_count(), expected_error_count);
    assert_eq!(
        merged_failure.warning_count(),
        expected_record_counts.1 as usize
    );
    assert_eq!(
        accumulated_failure.warning_count(),
        expected_record_counts.1 as usize
    );
    let infrastructure_causes = |failure: &BoundaryFailure| {
        failure
            .infrastructure_errors()
            .map_or(0, |errors| 1 + errors.additional.len())
    };
    assert_eq!(
        infrastructure_causes(&merged_failure),
        expected_infrastructure_causes
    );
    assert_eq!(
        infrastructure_causes(&accumulated_failure),
        expected_infrastructure_causes
    );

    let metadata = |failure: &BoundaryFailure| {
        failure
            .reports
            .iter()
            .map(|(_, view)| (view.descriptor().code, view.reason_key(), view.severity()))
            .collect::<Vec<_>>()
    };
    assert_eq!(metadata(&merged_failure), metadata(&accumulated_failure));

    (merged_failure, accumulated_failure)
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

/// Two databases that number their only source identically, so only the row's domain tells them apart.
fn colliding_owner_messages(
    directory: &Path,
    domain_a: Option<StablePackageIdentity>,
    domain_b: Option<StablePackageIdentity>,
) -> CompilerMessages {
    let mut string_table = StringTable::new();
    let (alpha_db, alpha_id) =
        retained_database(directory, "alpha.moth", "alpha", &mut string_table);
    let (bravo_db, bravo_id) =
        retained_database(directory, "bravo.moth", "bravo", &mut string_table);
    assert_eq!(
        alpha_id, bravo_id,
        "both databases should number their only source identically"
    );

    let alpha_name = string_table.intern("alpha");
    let bravo_name = string_table.intern("bravo");
    let mut messages = CompilerMessages::from_diagnostics(
        vec![
            CompilerDiagnostic::unknown_value_name(alpha_name, Some(inline_span(alpha_id, 0, 5))),
            CompilerDiagnostic::unknown_value_name(bravo_name, Some(inline_span(bravo_id, 0, 5))),
        ],
        string_table,
    );
    messages.install_source_contexts(
        [
            RenderSourceContext {
                diagnostic_range: 0..1,
                source_database: Arc::new(alpha_db),
                domain: domain_a,
            },
            RenderSourceContext {
                diagnostic_range: 1..2,
                source_database: Arc::new(bravo_db),
                domain: domain_b,
            },
        ],
        0,
    );
    messages
}

/// Colliding source ids must still resolve through each diagnostic's own database.
#[test]
fn colliding_source_ids_resolve_through_their_own_database() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    for install in [InstallMode::Frozen, InstallMode::Unfrozen] {
        let messages = colliding_owner_messages(dir.path(), None, None);
        let frozen = assert_legacy_parity(messages, install);
        assert_eq!(primary(&frozen[0]).line, "alpha");
        assert_eq!(primary(&frozen[1]).line, "bravo");
    }
}

/// Named domains keep colliding source ids apart and report each file's own path.
#[test]
fn colliding_source_ids_resolve_through_their_named_domains() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let messages = colliding_owner_messages(
        dir.path(),
        Some(StablePackageIdentity::project_local("pkg-a")),
        Some(StablePackageIdentity::project_local("pkg-b")),
    );
    let frozen = assert_legacy_parity(messages, InstallMode::Frozen);
    assert_eq!(primary(&frozen[0]).line, "alpha");
    assert_eq!(primary(&frozen[1]).line, "bravo");
    assert!(primary(&frozen[0]).path.ends_with("alpha.moth"));
    assert!(primary(&frozen[1]).path.ends_with("bravo.moth"));
}

/// An identity-less database row renders through the legacy database branch in Unfrozen mode.
#[test]
fn identity_less_database_row_uses_the_legacy_database_branch() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();
    let (db, id) = retained_database(dir.path(), "main.moth", "main", &mut string_table);
    let name = string_table.intern("main");
    let mut messages = CompilerMessages::from_diagnostics(
        vec![CompilerDiagnostic::unknown_value_name(
            name,
            Some(inline_span(id, 0, 4)),
        )],
        string_table,
    );
    messages.set_source_database(Arc::new(db));

    let frozen = assert_legacy_parity(messages, InstallMode::Unfrozen);
    let position = primary(&frozen[0]);
    assert_eq!(position.line, "main");
    assert_eq!(position.start, (0, 0));
    assert_eq!(position.end, (0, 4));
}

/// A vessel whose module never merged into the build root still holds its module fork. Its inherited and local names
/// both render.
#[test]
fn module_fork_vessel_renders_its_inherited_and_local_names() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut root = StringTable::new();
    let (db, id) = retained_database(dir.path(), "main.moth", "main alpha", &mut root);
    let inherited = root.intern("main");
    let (mut fork, _) = root.fork_for_module().into_parts();
    let local = fork.intern("alpha");
    let mut messages = CompilerMessages::from_diagnostics(
        vec![
            CompilerDiagnostic::unknown_value_name(inherited, Some(inline_span(id, 0, 4))),
            CompilerDiagnostic::unknown_value_name(local, Some(inline_span(id, 5, 5))),
        ],
        fork,
    );
    messages.set_source_database(Arc::new(db));

    let frozen = assert_legacy_parity(messages, InstallMode::Unfrozen);
    assert!(frozen[0].message.contains("main"));
    assert!(frozen[1].message.contains("alpha"));
}

/// When several rows cover one diagnostic, the first covering row owns it.
#[test]
fn first_covering_row_owns_the_diagnostic() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();
    let (a_db, a_id) = retained_database(dir.path(), "a.moth", "main", &mut string_table);
    let (b_db, _b_id) = retained_database(dir.path(), "b.moth", "main", &mut string_table);
    let name = string_table.intern("main");
    let mut messages = CompilerMessages::from_diagnostics(
        vec![CompilerDiagnostic::unknown_value_name(
            name,
            Some(inline_span(a_id, 0, 4)),
        )],
        string_table,
    );
    messages.install_source_contexts(
        [
            RenderSourceContext {
                diagnostic_range: 0..1,
                source_database: Arc::new(a_db),
                domain: None,
            },
            RenderSourceContext {
                diagnostic_range: 0..1,
                source_database: Arc::new(b_db),
                domain: None,
            },
        ],
        0,
    );

    let frozen = assert_legacy_parity(messages, InstallMode::Frozen);
    let host_path = primary(&frozen[0])
        .host_path
        .as_deref()
        .expect("the owning database should report a host path");
    assert!(host_path.ends_with("a.moth"));
}

/// A type environment without a path table is filtered out, so type ids render raw on both sides.
#[test]
fn type_environment_without_a_path_table_renders_raw_type_ids() {
    let diagnostic = CompilerDiagnostic::type_mismatch(
        TypeId(999),
        TypeId(1000),
        TypeMismatchContext::Assignment,
        None,
    );
    let messages = CompilerMessages::from_diagnostics(vec![diagnostic], StringTable::new())
        .with_type_context_for_all_diagnostics(TypeEnvironment::new());

    let frozen = assert_legacy_parity(messages, InstallMode::Frozen);
    assert_eq!(
        frozen[0].message,
        "Type mismatch in assignment: expected TypeId(999), found TypeId(1000)"
    );
}

/// A diagnostic with no source owner and no span renders without a position.
#[test]
fn unowned_diagnostic_renders_without_a_position() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("dup");
    let messages = CompilerMessages::from_diagnostics(
        vec![CompilerDiagnostic::duplicate_declaration(name, None, None)],
        string_table,
    );

    let frozen = assert_legacy_parity(messages, InstallMode::Frozen);
    assert!(frozen[0].position.is_none());
}

/// An explicit path table supplies the path that a path-only diagnostic renders.
#[test]
fn explicit_path_table_renders_a_path_only_diagnostic() {
    let mut string_table = StringTable::new();
    let mut paths = PathInternerBuilder::new();
    let path = paths
        .try_intern_portable_path("pkg/mod.moth", &mut string_table)
        .expect("portable path should intern");
    let mut messages = CompilerMessages::from_diagnostics(
        vec![CompilerDiagnostic::missing_import_target(path, None)],
        string_table,
    );
    messages
        .attach_path_table_if_missing(Arc::new(paths.freeze()))
        .expect("explicit path table should attach");

    let frozen = assert_legacy_parity(messages, InstallMode::Frozen);
    assert!(frozen[0].message.contains("pkg/mod.moth"));
}

/// Two named domains over one database share a single frozen identity context.
#[test]
fn named_domains_over_one_database_share_one_frozen_identity() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();
    let (db, id) = retained_database(dir.path(), "main.moth", "main", &mut string_table);
    let name = string_table.intern("main");
    let mut messages = CompilerMessages::from_diagnostics(
        vec![
            CompilerDiagnostic::unknown_value_name(name, Some(inline_span(id, 0, 4))),
            CompilerDiagnostic::unknown_value_name(name, Some(inline_span(id, 0, 4))),
        ],
        string_table,
    );
    let shared = Arc::new(db);
    messages.install_source_contexts(
        [
            RenderSourceContext {
                diagnostic_range: 0..1,
                source_database: Arc::clone(&shared),
                domain: Some(StablePackageIdentity::project_local("pkg-a")),
            },
            RenderSourceContext {
                diagnostic_range: 1..2,
                source_database: shared,
                domain: Some(StablePackageIdentity::project_local("pkg-b")),
            },
        ],
        0,
    );

    let legacy = legacy_render(&messages);
    let failure = into_frozen_boundary_failure(messages);
    assert_render_parity(&legacy, &frozen_render(&failure));
    assert!(std::ptr::eq(
        failure.reports.report(0).context(),
        failure.reports.report(1).context(),
    ));
    assert!(Arc::ptr_eq(
        &failure.reports.report(0).shared_context(),
        &failure.reports.report(1).shared_context(),
    ));
}

/// Interleaved domain runs must keep the legacy diagnostic order while each report keeps its own records.
#[test]
fn interleaved_domain_runs_keep_legacy_order() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();
    let (db, id) = retained_database(dir.path(), "main.moth", "main", &mut string_table);
    let name = string_table.intern("main");
    let mut messages = CompilerMessages::from_diagnostics(
        vec![
            CompilerDiagnostic::unknown_value_name(name, Some(inline_span(id, 0, 4))),
            CompilerDiagnostic::unknown_value_name(name, Some(inline_span(id, 0, 4))),
            CompilerDiagnostic::unknown_value_name(name, Some(inline_span(id, 0, 4))),
        ],
        string_table,
    );
    let shared = Arc::new(db);
    messages.install_source_contexts(
        [
            RenderSourceContext {
                diagnostic_range: 0..1,
                source_database: Arc::clone(&shared),
                domain: Some(StablePackageIdentity::project_local("pkg-a")),
            },
            RenderSourceContext {
                diagnostic_range: 1..2,
                source_database: Arc::clone(&shared),
                domain: Some(StablePackageIdentity::project_local("pkg-b")),
            },
            RenderSourceContext {
                diagnostic_range: 2..3,
                source_database: shared,
                domain: Some(StablePackageIdentity::project_local("pkg-a")),
            },
        ],
        0,
    );

    let legacy = legacy_render(&messages);
    let failure = into_frozen_boundary_failure(messages);
    assert_render_parity(&legacy, &frozen_render(&failure));
    assert_eq!(failure.reports.report(0).len(), 2);
    assert_eq!(failure.reports.report(1).len(), 1);
}

/// An infrastructure cause keeps its database as a frozen identity even with no diagnostics to render.
#[test]
fn infrastructure_cause_row_keeps_its_frozen_identity() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();
    let (db, _id) = retained_database(dir.path(), "main.moth", "main", &mut string_table);
    let mut messages = CompilerMessages::from_diagnostics(Vec::new(), string_table);
    messages.set_infrastructure_error(CompilerError::compiler_error("cause"));
    messages.set_source_database(Arc::new(db));

    let failure = into_frozen_boundary_failure(messages);
    assert_eq!(failure.error_count(), 1);
    assert!(
        failure
            .infrastructure_errors()
            .expect("infrastructure errors should survive conversion")
            .frozen_identity_for_cause(0)
            .is_some()
    );
}

/// A forced type-display capacity fault keeps the diagnostic's position and replaces only its message.
#[test]
fn forced_type_display_capacity_keeps_the_diagnostic_position() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();
    let (db, id) = retained_database(dir.path(), "main.moth", "main", &mut string_table);
    let mut messages = CompilerMessages::from_diagnostics(
        vec![CompilerDiagnostic::type_mismatch(
            TypeId(999),
            TypeId(1000),
            TypeMismatchContext::Assignment,
            Some(inline_span(id, 0, 4)),
        )],
        string_table,
    );
    messages.set_source_database(Arc::new(db));

    // The legacy render must run before the guard: it panics once display capture is forced to fail.
    let legacy = legacy_render(&messages);
    let _guard = ForcedTypeDisplayExhaustionGuard::new();
    let failure = into_frozen_boundary_failure(messages);
    let frozen = frozen_render(&failure);

    assert_eq!(
        frozen[0].message,
        "This source exceeds the compiler's capacity for diagnostic type displays."
    );
    assert_eq!(frozen[0].position, legacy[0].position);
    assert_eq!(failure.reports.report(0).len(), 1);
}

/// A path-table capacity fault during conversion renders exactly like a legacy capacity diagnostic.
#[test]
fn path_table_capacity_fault_matches_the_legacy_capacity_render() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();
    let mut paths = PathInternerBuilder::new();
    let new_path = paths
        .try_intern_portable_path("pkg/new/mod.moth", &mut string_table)
        .expect("portable path should intern");
    let explicit = Arc::new(paths.freeze());
    let (db, id) = retained_database(dir.path(), "main.moth", "main", &mut string_table);

    let baseline = CompilerMessages::from_diagnostics(
        vec![CompilerDiagnostic::source_table_capacity(
            SourceSpanCapacityResource::LogicalPath,
        )],
        string_table.clone(),
    );
    let expected = legacy_render(&baseline);

    let mut messages = CompilerMessages::from_diagnostics(
        vec![CompilerDiagnostic::missing_import_target(
            new_path,
            Some(inline_span(id, 0, 4)),
        )],
        string_table,
    );
    messages.install_source_contexts(
        [RenderSourceContext {
            diagnostic_range: 0..1,
            source_database: Arc::new(db),
            domain: None,
        }],
        0,
    );
    messages
        .attach_path_table_if_missing(explicit)
        .expect("explicit path table should attach");

    let frozen = {
        let _exhausted = ForcedExhaustionGuard::new();
        let failure = into_frozen_boundary_failure(messages);
        frozen_render(&failure)
    };
    assert_render_parity(&expected, &frozen);
}

/// An explicit path table whose ids differ from the database's is remapped into the owner's builder.
#[test]
fn merged_explicit_path_ids_remap_into_the_owner_builder() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();
    let mut paths = PathInternerBuilder::new();
    let pkg_path = paths
        .try_intern_portable_path("pkg/mod.moth", &mut string_table)
        .expect("portable path should intern");
    let main_path = paths
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("portable path should intern");
    let explicit = Arc::new(paths.freeze());
    let (db, id) = retained_database(dir.path(), "main.moth", "main", &mut string_table);
    assert!(
        !db.clone_path_builder()
            .merge_snapshot(&explicit)
            .expect("explicit table should merge")
            .is_identity(),
        "the explicit table's ids should differ from the database's, so the merge must remap"
    );

    let mut messages = CompilerMessages::from_diagnostics(
        vec![
            CompilerDiagnostic::missing_import_target(main_path, Some(inline_span(id, 0, 4))),
            CompilerDiagnostic::missing_import_target(pkg_path, None),
        ],
        string_table,
    );
    messages.install_source_contexts(
        [RenderSourceContext {
            diagnostic_range: 0..1,
            source_database: Arc::new(db),
            domain: None,
        }],
        0,
    );
    messages
        .attach_path_table_if_missing(explicit)
        .expect("explicit path table should attach");

    let frozen = assert_legacy_parity(messages, InstallMode::Frozen);
    assert!(frozen[0].message.contains("main.moth"));
    assert!(frozen[1].message.contains("pkg/mod.moth"));
    assert_eq!(primary(&frozen[0]).line, "main");
}

/// An explicit path table that extends the database's own ids merges as identity and keeps its ids.
#[test]
fn identity_path_merge_leaves_explicit_path_ids_unchanged() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();
    let (db, id) = retained_database(dir.path(), "main.moth", "main", &mut string_table);
    let mut paths = db.clone_path_builder();
    let main_path = paths
        .try_intern_portable_path("main.moth", &mut string_table)
        .expect("portable path should intern");
    let explicit = Arc::new(paths.freeze());
    assert!(
        db.clone_path_builder()
            .merge_snapshot(&explicit)
            .expect("explicit table should merge")
            .is_identity(),
        "an explicit table that extends the database's ids should merge as identity"
    );

    let mut messages = CompilerMessages::from_diagnostics(
        vec![CompilerDiagnostic::missing_import_target(
            main_path,
            Some(inline_span(id, 0, 4)),
        )],
        string_table,
    );
    messages.install_source_contexts(
        [RenderSourceContext {
            diagnostic_range: 0..1,
            source_database: Arc::new(db),
            domain: None,
        }],
        0,
    );
    messages
        .attach_path_table_if_missing(explicit)
        .expect("explicit path table should attach");

    let frozen = assert_legacy_parity(messages, InstallMode::Frozen);
    assert!(frozen[0].message.contains("main.moth"));
}

/// A span with no source owner renders without a position, on both sides.
#[test]
fn span_without_a_source_owner_renders_without_a_position() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("missing");
    let messages = CompilerMessages::from_diagnostics(
        vec![CompilerDiagnostic::unknown_value_name(
            name,
            Some(exact_span(0, 5)),
        )],
        string_table,
    );

    let frozen = assert_legacy_parity(messages, InstallMode::Unfrozen);
    assert!(frozen[0].position.is_none());
}

/// Record counts, warnings, and infrastructure errors are reported separately and agree with the legacy render.
#[test]
fn boundary_failure_counts_records_and_infrastructure_separately() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("dup");
    let mut messages = CompilerMessages::from_diagnostics(
        vec![
            CompilerDiagnostic::duplicate_declaration(name, None, None),
            CompilerDiagnostic::unreachable_match_arm(None),
            CompilerDiagnostic::duplicate_declaration(name, None, None),
        ],
        string_table,
    );
    messages.set_infrastructure_error(CompilerError::compiler_error("cause"));

    let legacy = legacy_render(&messages);
    let failure = into_frozen_boundary_failure(messages);
    assert_eq!(failure.error_count(), 3);
    assert_eq!(failure.warning_count(), 1);
    assert!(failure.has_infrastructure_error());
    assert_render_parity(&legacy, &frozen_render(&failure));
}

/// A donor primary handle resolves through the identity it installed, not the database row's.
#[test]
fn donor_primary_handle_resolves_through_its_installed_identity() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();

    let (baseline_db, baseline_id) =
        retained_database(dir.path(), "main.moth", "main", &mut string_table);
    let name = string_table.intern("main");
    let mut baseline = CompilerMessages::from_diagnostics(
        vec![CompilerDiagnostic::unknown_value_name(
            name,
            Some(inline_span(baseline_id, 0, 4)),
        )],
        string_table.clone(),
    );
    baseline.install_source_contexts(
        [RenderSourceContext {
            diagnostic_range: 0..1,
            source_database: Arc::new(baseline_db),
            domain: None,
        }],
        0,
    );
    let expected = assert_legacy_parity(baseline, InstallMode::Frozen);

    let (db, id) = retained_database(dir.path(), "main.moth", "main", &mut string_table);
    let handle = FrozenIdentityHandle::for_domain(StablePackageIdentity::project_local("pkg-x"));
    let mut diagnostic = CompilerDiagnostic::unknown_value_name(name, Some(inline_span(id, 0, 4)));
    diagnostic.primary_frozen_identity_handle = Some(handle.clone());
    let mut messages = CompilerMessages::from_diagnostics(vec![diagnostic], string_table);
    messages.install_source_contexts(
        [RenderSourceContext {
            diagnostic_range: 0..1,
            source_database: Arc::new(db),
            domain: Some(StablePackageIdentity::project_local("pkg-x")),
        }],
        0,
    );

    let failure = into_frozen_boundary_failure(messages);
    let frozen = frozen_render(&failure);
    assert_eq!(frozen[0].position, expected[0].position);
    assert!(std::ptr::eq(
        handle.get().expect("the donor handle should be installed"),
        failure.reports.report(0).context(),
    ));
}

/// A legacy-only borrow conflict has no compact family, so the converter must refuse it.
#[test]
#[should_panic(
    expected = "legacy diagnostic has no compact family; the boundary converter cannot represent it"
)]
fn legacy_only_borrow_conflict_is_rejected_at_the_boundary() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("value");
    let diagnostic = CompilerDiagnostic::new(
        DiagnosticKind::Borrow(BorrowDiagnosticKind::BorrowConflict),
        None,
        DiagnosticPayload::BorrowConflict {
            place: DiagnosticPlace::Local(name),
            existing_access: BorrowAccessKind::Shared,
            requested_access: BorrowAccessKind::Mutable,
        },
    );
    let messages = CompilerMessages::from_diagnostics(vec![diagnostic], string_table);
    let _failure = into_frozen_boundary_failure(messages);
}

/// A database still held outside the messages cannot be taken over at the frozen handoff.
#[test]
#[should_panic(expected = "source database was unexpectedly shared at the frozen render handoff")]
fn database_shared_outside_the_messages_is_rejected_at_the_handoff() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();
    let (db, id) = retained_database(dir.path(), "main.moth", "main", &mut string_table);
    let shared = Arc::new(db);
    let name = string_table.intern("main");
    let mut messages = CompilerMessages::from_diagnostics(
        vec![CompilerDiagnostic::unknown_value_name(
            name,
            Some(inline_span(id, 0, 4)),
        )],
        string_table,
    );
    messages.set_source_database(Arc::clone(&shared));
    let _failure = into_frozen_boundary_failure(messages);
}

/// A vessel marked frozen installs its source identity as part of boundary conversion.
#[test]
fn marked_frozen_mode_installs_identity_during_conversion() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let identity = StablePackageIdentity::project_local("pkg-x");
    let fixture = |relative_name: &str| {
        let mut string_table = StringTable::new();
        let (db, id) = retained_database(dir.path(), relative_name, "main", &mut string_table);
        let name = string_table.intern("main");
        let handle = FrozenIdentityHandle::for_domain(identity.clone());
        let mut diagnostic =
            CompilerDiagnostic::unknown_value_name(name, Some(inline_span(id, 0, 4)));
        diagnostic.primary_frozen_identity_handle = Some(handle.clone());
        let mut messages = CompilerMessages::from_diagnostics(vec![diagnostic], string_table);
        messages.install_source_contexts(
            [RenderSourceContext {
                diagnostic_range: 0..1,
                source_database: Arc::new(db),
                domain: Some(identity.clone()),
            }],
            0,
        );
        (messages, handle)
    };

    let (messages, unmarked_handle) = fixture("unmarked.moth");
    assert!(unmarked_handle.get().is_none());
    let unmarked_failure = into_boundary_failure(messages);
    assert!(unmarked_handle.get().is_none());
    let (report, view) = unmarked_failure
        .reports
        .iter()
        .next()
        .expect("the unmarked diagnostic should remain");
    assert!(
        DiagnosticRenderContext::for_report(report)
            .site_position(view.primary_site())
            .is_none(),
        "an unmarked vessel must not install the identity-named handle"
    );

    let (mut messages, frozen_handle) = fixture("frozen.moth");
    let frozen_path = dir.path().join("src/frozen.moth");
    assert!(frozen_handle.get().is_none());
    messages.mark_frozen();
    let frozen_failure = into_boundary_failure(messages);
    let installed = frozen_handle
        .get()
        .expect("frozen conversion should install the identity-named handle");
    let (report, view) = frozen_failure
        .reports
        .iter()
        .next()
        .expect("the frozen diagnostic should remain");
    assert!(
        std::ptr::eq(installed, report.context()),
        "the installed handle should own the report's identity context"
    );
    let position = DiagnosticRenderContext::for_report(report)
        .site_position(view.primary_site())
        .expect("the frozen primary site should resolve through its installed owner");
    assert_eq!(position.host_path, Some(frozen_path.as_path()));
    assert_eq!(position.line, "main");
}

/// A primary donor handle whose identity was never installed must be refused, not resolved against the row.
#[test]
#[should_panic(
    expected = "diagnostic primary reached the frozen render boundary without its donor identity"
)]
fn primary_donor_without_an_installed_identity_is_rejected() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();
    let (db, id) = retained_database(dir.path(), "main.moth", "main", &mut string_table);
    let name = string_table.intern("main");
    let mut diagnostic = CompilerDiagnostic::unknown_value_name(name, Some(inline_span(id, 0, 4)));
    diagnostic.primary_frozen_identity_handle = Some(FrozenIdentityHandle::for_domain(
        StablePackageIdentity::project_local("pkg-x"),
    ));
    let mut messages = CompilerMessages::from_diagnostics(vec![diagnostic], string_table);
    messages.set_source_database(Arc::new(db));
    let _failure = into_frozen_boundary_failure(messages);
}

/// A label donor handle whose identity was never installed must be refused, not resolved against the row.
#[test]
#[should_panic(
    expected = "diagnostic label reached the frozen render boundary without its donor identity"
)]
fn label_donor_without_an_installed_identity_is_rejected() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();
    let (db, id) = retained_database(dir.path(), "main.moth", "main", &mut string_table);
    let name = string_table.intern("main");
    let label = DiagnosticLabel::secondary_with_frozen_identity(
        Some(inline_span(id, 0, 4)),
        Some(DiagnosticLabelMessage::PreviousDeclaration),
        FrozenIdentityHandle::for_domain(StablePackageIdentity::project_local("pkg-x")),
    );
    let diagnostic =
        CompilerDiagnostic::duplicate_declaration(name, None, None).with_labels(vec![label]);
    let mut messages = CompilerMessages::from_diagnostics(vec![diagnostic], string_table);
    messages.set_source_database(Arc::new(db));
    let _failure = into_frozen_boundary_failure(messages);
}

/// Infrastructure source rows frozen before conversion must be refused by the converter.
#[test]
#[should_panic(expected = "pre-frozen infrastructure source owner reached the boundary converter")]
fn pre_frozen_infrastructure_owner_is_rejected_by_the_converter() {
    let mut messages = CompilerMessages::from_diagnostics(Vec::new(), StringTable::new());
    messages.set_infrastructure_error(CompilerError::compiler_error("cause"));
    messages
        .infrastructure_errors
        .as_mut()
        .expect("infrastructure error should be installed")
        .set_frozen_source_rows(vec![(
            0..1,
            Arc::new(FrozenIdentityContext::from_parts(
                StringTable::new(),
                SourceDatabase::empty(),
            )),
        )]);
    let _failure = into_frozen_boundary_failure(messages);
}

/// A local secondary label resolves its position through the requester's own database on both sides.
#[test]
fn local_secondary_label_renders_its_position_on_both_sides() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();
    let (db, id) = retained_database(dir.path(), "main.moth", "main", &mut string_table);
    let name = string_table.intern("main");
    let label = DiagnosticLabel::secondary(
        Some(inline_span(id, 0, 4)),
        Some(DiagnosticLabelMessage::PreviousDeclaration),
    );
    let diagnostic =
        CompilerDiagnostic::duplicate_declaration(name, None, None).with_labels(vec![label]);
    let mut messages = CompilerMessages::from_diagnostics(vec![diagnostic], string_table);
    messages.set_source_database(Arc::new(db));

    let frozen = assert_legacy_parity(messages, InstallMode::Frozen);
    let rendered = &frozen[0].labels[0];
    assert_eq!(rendered.text.as_deref(), Some("previous declaration here"));
    let position = rendered
        .position
        .as_ref()
        .expect("local label position should resolve");
    assert_eq!(position.line, "main");
    assert_eq!(position.start, (0, 0));
    assert_eq!(position.end, (0, 4));
}

/// A donor label whose identity is installed before the render resolves its position through that identity.
///
/// The legacy side resolves a donor label only through an installed identity. The converter leaves an installed
/// handle alone, so both sides read the position from the donor context, not the requester's database.
#[test]
fn installed_donor_label_renders_its_position_through_its_identity() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();
    let (db, _) = retained_database(dir.path(), "main.moth", "main", &mut string_table);
    let name = string_table.intern("main");

    let donor_dir = tempfile::tempdir().expect("should create donor temp dir");
    let mut donor_strings = StringTable::new();
    let (donor_db, donor_id) =
        retained_database(donor_dir.path(), "donor.moth", "donor", &mut donor_strings);
    let donor_context = Arc::new(FrozenIdentityContext::from_parts(donor_strings, donor_db));
    let handle = FrozenIdentityHandle::for_domain(StablePackageIdentity::project_local("pkg-x"));
    handle
        .install(donor_context)
        .expect("donor identity should install once");
    let label = DiagnosticLabel::secondary_with_frozen_identity(
        Some(inline_span(donor_id, 0, 5)),
        Some(DiagnosticLabelMessage::PreviousDeclaration),
        handle,
    );
    let diagnostic =
        CompilerDiagnostic::duplicate_declaration(name, None, None).with_labels(vec![label]);
    let mut messages = CompilerMessages::from_diagnostics(vec![diagnostic], string_table);
    messages.set_source_database(Arc::new(db));

    let frozen = assert_legacy_parity(messages, InstallMode::Frozen);
    let rendered = &frozen[0].labels[0];
    assert_eq!(rendered.text.as_deref(), Some("previous declaration here"));
    let position = rendered
        .position
        .as_ref()
        .expect("donor label position should resolve");
    assert_eq!(position.line, "donor");
    assert_eq!(position.start, (0, 0));
    assert_eq!(position.end, (0, 5));
}

/// A named donor label with no installed identity renders position-less on both sides in `Unfrozen` mode.
///
/// `Unfrozen` installs no identity, so the donor position cannot resolve on either side. The span points into the
/// requester's own source, so any fallback to that database would show a position. None must appear.
#[test]
fn uninstalled_donor_label_renders_position_less_on_both_sides() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let mut string_table = StringTable::new();
    let (db, id) = retained_database(dir.path(), "main.moth", "main", &mut string_table);
    let name = string_table.intern("main");
    let label = DiagnosticLabel::secondary_with_frozen_identity(
        Some(inline_span(id, 0, 4)),
        Some(DiagnosticLabelMessage::PreviousDeclaration),
        FrozenIdentityHandle::for_domain(StablePackageIdentity::project_local("pkg-x")),
    );
    let diagnostic =
        CompilerDiagnostic::duplicate_declaration(name, None, None).with_labels(vec![label]);
    let mut messages = CompilerMessages::from_diagnostics(vec![diagnostic], string_table);
    messages.set_source_database(Arc::new(db));

    let frozen = assert_legacy_parity(messages, InstallMode::Unfrozen);
    let rendered = &frozen[0].labels[0];
    assert_eq!(rendered.text.as_deref(), Some("previous declaration here"));
    assert!(
        rendered.position.is_none(),
        "an uninstalled donor label should render position-less"
    );
}

/// Distinct vessel string tables with colliding ids are remapped before the run freezes.
#[test]
fn accumulator_parity_remaps_colliding_vessel_strings() {
    let make_vessels = || {
        let mut first_strings = StringTable::new();
        let alpha = first_strings.intern("alpha");
        let mut second_strings = StringTable::new();
        let bravo = second_strings.intern("bravo");
        assert_eq!(alpha, bravo, "fixture string ids should collide");

        let first = CompilerMessages::from_diagnostics(
            vec![CompilerDiagnostic::duplicate_declaration(alpha, None, None)],
            first_strings,
        );
        let second = CompilerMessages::from_diagnostics(
            vec![CompilerDiagnostic::duplicate_declaration(bravo, None, None)],
            second_strings,
        );
        (first, second)
    };

    let (_, accumulated) =
        assert_accumulator_matches_merge(make_vessels, InstallMode::Unfrozen, (2, 0, 0), 2, 0);
    let rendered = frozen_render(&accumulated);
    assert!(rendered[0].message.contains("alpha"));
    assert!(rendered[1].message.contains("bravo"));
}

/// Shared database rows reuse one identity domain or share its frozen owner across identities.
#[test]
fn accumulator_parity_shares_a_database_across_identity_domains() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let identity_a = StablePackageIdentity::project_local("pkg-a");

    for (identity_b, one_domain) in [
        (identity_a.clone(), true),
        (StablePackageIdentity::project_local("pkg-b"), false),
    ] {
        let make_vessels = || {
            let mut strings = StringTable::new();
            let (database, source) =
                retained_database(dir.path(), "shared.moth", "missing", &mut strings);
            let name = strings.intern("missing");
            let database = Arc::new(database);
            let make_messages = |domain: StablePackageIdentity| {
                let mut messages = CompilerMessages::from_diagnostics(
                    vec![CompilerDiagnostic::unknown_value_name(
                        name,
                        Some(inline_span(source, 0, 7)),
                    )],
                    strings.clone(),
                );
                messages.install_source_contexts(
                    [RenderSourceContext {
                        diagnostic_range: 0..1,
                        source_database: Arc::clone(&database),
                        domain: Some(domain),
                    }],
                    0,
                );
                messages
            };
            (
                make_messages(identity_a.clone()),
                make_messages(identity_b.clone()),
            )
        };

        let (merged, accumulated) =
            assert_accumulator_matches_merge(make_vessels, InstallMode::Frozen, (2, 0, 0), 2, 0);
        assert_eq!(
            merged.reports.report(0).len(),
            if one_domain { 2 } else { 1 }
        );
        assert_eq!(
            accumulated.reports.report(0).len(),
            if one_domain { 2 } else { 1 }
        );
        if !one_domain {
            assert_eq!(merged.reports.report(1).len(), 1);
            assert_eq!(accumulated.reports.report(1).len(), 1);
            assert!(std::ptr::eq(
                merged.reports.report(0).context(),
                merged.reports.report(1).context(),
            ));
            assert!(std::ptr::eq(
                accumulated.reports.report(0).context(),
                accumulated.reports.report(1).context(),
            ));
        }
    }
}

/// One explicit table shared by both vessels contributes once to its owner's path builder.
#[test]
fn accumulator_parity_deduplicates_a_shared_explicit_path_table() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let make_vessels = || {
        let mut strings = StringTable::new();
        let mut paths = PathInternerBuilder::new();
        let package_path = paths
            .try_intern_portable_path("pkg/mod.moth", &mut strings)
            .expect("package path should intern");
        let main_path = paths
            .try_intern_portable_path("main.moth", &mut strings)
            .expect("main path should intern");
        let explicit = Arc::new(paths.freeze());
        let (database, source) = retained_database(dir.path(), "main.moth", "main", &mut strings);
        let database = Arc::new(database);
        let make_messages = |path| {
            let mut messages = CompilerMessages::from_diagnostics(
                vec![CompilerDiagnostic::missing_import_target(
                    path,
                    Some(inline_span(source, 0, 4)),
                )],
                strings.clone(),
            );
            messages.install_source_contexts(
                [RenderSourceContext {
                    diagnostic_range: 0..1,
                    source_database: Arc::clone(&database),
                    domain: None,
                }],
                0,
            );
            messages
                .attach_path_table_if_missing(Arc::clone(&explicit))
                .expect("explicit path table should attach");
            messages
        };
        (make_messages(main_path), make_messages(package_path))
    };

    let (_, accumulated) =
        assert_accumulator_matches_merge(make_vessels, InstallMode::Frozen, (2, 0, 0), 2, 0);
    let rendered = frozen_render(&accumulated);
    assert!(rendered[0].message.contains("main.moth"));
    assert!(rendered[1].message.contains("pkg/mod.moth"));
    assert_eq!(primary(&rendered[0]).line, "main");
    assert_eq!(primary(&rendered[1]).line, "main");
}

/// Each vessel's type snapshot and explicit path table survive independent string and path remaps.
#[test]
fn accumulator_parity_captures_each_vessel_type_display() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let identity = StablePackageIdentity::project_local("type-owners");
    let make_vessels = || {
        let mut base_strings = StringTable::new();
        let (database, _) = retained_database(dir.path(), "main.moth", "main", &mut base_strings);
        let database = Arc::new(database);

        let mut alpha_strings = base_strings.clone();
        let alpha_name = alpha_strings.intern("TAlpha");
        let mut alpha_paths = PathInternerBuilder::new();
        let alpha_import_path = alpha_paths
            .try_intern_portable_path("pkg/alpha-only.moth", &mut alpha_strings)
            .expect("Alpha import path should intern");
        let alpha_explicit = Arc::new(alpha_paths.freeze());

        let mut beta_strings = base_strings;
        let beta_name = beta_strings.intern("TBeta");
        assert_eq!(alpha_name, beta_name, "type-name ids should collide");
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
        assert!(
            !Arc::ptr_eq(&alpha_explicit, &beta_explicit),
            "each vessel should own a distinct explicit table"
        );

        let make_messages = |strings, name, import_path, explicit| {
            let mut environment = TypeEnvironment::new();
            let generic_type = environment.register_synthetic_generic_parameter(name);
            let boolean = environment.builtins().bool;
            let mut messages = CompilerMessages::from_diagnostics(
                vec![
                    CompilerDiagnostic::type_mismatch(
                        generic_type,
                        boolean,
                        TypeMismatchContext::Assignment,
                        None,
                    ),
                    CompilerDiagnostic::missing_import_target(import_path, None),
                ],
                strings,
            )
            .with_type_context_for_all_diagnostics(environment);
            messages.install_source_contexts(
                [RenderSourceContext {
                    diagnostic_range: 0..2,
                    source_database: Arc::clone(&database),
                    domain: Some(identity.clone()),
                }],
                0,
            );
            messages
                .attach_path_table_if_missing(explicit)
                .expect("explicit paths should cover both diagnostics");
            messages
        };

        (
            make_messages(alpha_strings, alpha_name, alpha_import_path, alpha_explicit),
            make_messages(beta_strings, beta_name, beta_import_path, beta_explicit),
        )
    };

    let (merged, accumulated) =
        assert_accumulator_matches_merge(make_vessels, InstallMode::Frozen, (4, 0, 0), 4, 0);
    for failure in [&merged, &accumulated] {
        let rendered = frozen_render(failure);
        assert_eq!(rendered.len(), 4);
        assert!(rendered[0].message.contains("TAlpha"));
        assert!(rendered[1].message.contains("pkg/alpha-only.moth"));
        assert!(rendered[2].message.contains("TBeta"));
        assert!(rendered[3].message.contains("pkg/beta-only.moth"));
    }
}

/// Infrastructure causes from both vessels retain their own frozen source owners and order.
#[test]
fn accumulator_parity_preserves_infrastructure_causes_and_owners() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let make_vessels = || {
        let mut strings = StringTable::new();
        let (first_database, first_source) =
            retained_database(dir.path(), "first.moth", "first", &mut strings);
        let (second_database, second_source) =
            retained_database(dir.path(), "second.moth", "second", &mut strings);
        let mut first = CompilerMessages::from_diagnostics(Vec::new(), strings.clone());
        first.set_infrastructure_error(CompilerError::compiler_error("first cause"));
        first.set_source_database(Arc::new(first_database));
        let mut second = CompilerMessages::from_diagnostics(Vec::new(), strings);
        second.set_infrastructure_error(CompilerError::compiler_error("second cause"));
        second.set_source_database(Arc::new(second_database));
        (first, second, first_source, second_source)
    };
    let (first, second, first_source, second_source) = make_vessels();
    let mut merged = first;
    merged.append_messages_preserving_context(second);
    merged.mark_frozen();
    let merged_failure = into_boundary_failure(merged);

    let (first, second, accumulated_first_source, accumulated_second_source) = make_vessels();
    assert_eq!(first_source, accumulated_first_source);
    assert_eq!(second_source, accumulated_second_source);
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append(first);
    accumulator.append(second);
    let accumulated_failure = accumulator.finish(InstallMode::Frozen);

    assert_render_parity(
        &frozen_render(&merged_failure),
        &frozen_render(&accumulated_failure),
    );
    for failure in [&merged_failure, &accumulated_failure] {
        assert_eq!(failure.reports.counts().errors, 0);
        assert_eq!(failure.reports.counts().warnings, 0);
        assert_eq!(failure.reports.counts().notes, 0);
        assert_eq!(failure.error_count(), 1);
        assert_eq!(failure.warning_count(), 0);
        let infrastructure = failure
            .infrastructure_errors()
            .expect("the infrastructure lane should survive conversion");
        assert_eq!(1 + infrastructure.additional.len(), 2);
        for (cause, source, expected_path) in [
            (0, first_source, "first.moth"),
            (1, second_source, "second.moth"),
        ] {
            let context = infrastructure
                .frozen_identity_for_cause(cause)
                .expect("each cause should keep its source owner");
            let path = context
                .source_logical_path(source)
                .expect("cause source should keep its logical path");
            let rendered_path = context.render_path(path, &mut Vec::new());
            assert!(rendered_path.ends_with(expected_path));
        }
    }
}

/// A frozen identity handle can obtain its donor row from a later vessel in the same run.
#[test]
fn accumulator_parity_installs_a_donor_from_another_vessel() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let identity = StablePackageIdentity::project_local("donor-package");
    let make_vessels = || {
        let mut strings = StringTable::new();
        let (database, source) = retained_database(dir.path(), "donor.moth", "donor", &mut strings);
        let name = strings.intern("missing");
        let handle = FrozenIdentityHandle::for_domain(identity.clone());
        let mut diagnostic =
            CompilerDiagnostic::unknown_value_name(name, Some(inline_span(source, 0, 5)));
        diagnostic.primary_frozen_identity_handle = Some(handle.clone());
        let first = CompilerMessages::from_diagnostics(vec![diagnostic], strings.clone());

        let mut second = CompilerMessages::from_diagnostics(Vec::new(), strings);
        second.install_source_contexts(
            [RenderSourceContext {
                diagnostic_range: 0..0,
                source_database: Arc::new(database),
                domain: Some(identity.clone()),
            }],
            0,
        );
        (first, second, handle)
    };

    let (first, second, merged_handle) = make_vessels();
    let mut merged = first;
    merged.append_messages_preserving_context(second);
    merged.mark_frozen();
    let merged_failure = into_boundary_failure(merged);
    assert!(merged_handle.get().is_some());

    let (first, second, accumulated_handle) = make_vessels();
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append(first);
    accumulator.append(second);
    let accumulated_failure = accumulator.finish(InstallMode::Frozen);
    let installed = accumulated_handle
        .get()
        .expect("the donor row from the second vessel should install the handle");

    let merged_render = frozen_render(&merged_failure);
    let accumulated_render = frozen_render(&accumulated_failure);
    assert_render_parity(&merged_render, &accumulated_render);
    assert_eq!(
        (
            accumulated_failure.reports.counts().errors,
            accumulated_failure.reports.counts().warnings,
            accumulated_failure.reports.counts().notes,
        ),
        (1, 0, 0)
    );
    assert_eq!(accumulated_failure.error_count(), 1);
    assert_eq!(accumulated_failure.warning_count(), 0);
    assert!(std::ptr::eq(
        installed,
        accumulated_failure.reports.report(0).context()
    ));
    assert!(primary(&accumulated_render[0]).path.ends_with("donor.moth"));
    assert_eq!(primary(&accumulated_render[0]).line, "donor");
    let metadata = |failure: &BoundaryFailure| {
        failure
            .reports
            .iter()
            .map(|(_, view)| (view.descriptor().code, view.reason_key(), view.severity()))
            .collect::<Vec<_>>()
    };
    assert_eq!(metadata(&merged_failure), metadata(&accumulated_failure));
}

type PreparedTestBatch = (
    PremergeDiagnosticBatch,
    Option<(Arc<SourceDatabase>, Option<StablePackageIdentity>)>,
);

/// Compare the direct prepared carrier with an independently rebuilt legacy-vessel route.
fn assert_prepared_batch_parity(
    prepared_batches: Vec<PreparedTestBatch>,
    vessel_batches: Vec<PreparedTestBatch>,
    install: InstallMode,
) -> (BoundaryFailure, BoundaryFailure) {
    let mut prepared = BoundaryAccumulator::new();
    for (batch, source) in prepared_batches {
        prepared.append_prepared(PreparedDiagnosticBatch::from_premerge(batch), source);
    }
    let prepared_failure = prepared.finish(install);

    let mut vessel = BoundaryAccumulator::new();
    for (batch, source) in vessel_batches {
        let mut messages = batch.into_messages();
        if let Some((source_database, domain)) = source {
            let diagnostic_count = messages.diagnostic_slice().len();
            messages.install_source_contexts(
                [RenderSourceContext {
                    diagnostic_range: 0..diagnostic_count,
                    source_database,
                    domain,
                }],
                0,
            );
        }
        vessel.append(messages);
    }
    let vessel_failure = vessel.finish(install);

    assert_render_parity(
        &frozen_render(&vessel_failure),
        &frozen_render(&prepared_failure),
    );
    let counts = |failure: &BoundaryFailure| {
        let counts = failure.reports.counts();
        (counts.errors, counts.warnings, counts.notes)
    };
    assert_eq!(counts(&prepared_failure), counts(&vessel_failure));
    let metadata = |failure: &BoundaryFailure| {
        failure
            .reports
            .iter()
            .map(|(_, view)| (view.descriptor().code, view.reason_key(), view.severity()))
            .collect::<Vec<_>>()
    };
    assert_eq!(metadata(&prepared_failure), metadata(&vessel_failure));
    (prepared_failure, vessel_failure)
}

fn assert_prepared_batches_match_vessel(
    make_batches: impl Fn() -> Vec<PreparedTestBatch>,
    install: InstallMode,
) -> (BoundaryFailure, BoundaryFailure) {
    assert_prepared_batch_parity(make_batches(), make_batches(), install)
}

#[test]
fn prepared_batch_carrier_stays_within_its_size_budget() {
    assert!(std::mem::size_of::<PreparedDiagnosticBatch>() <= 48);
}

/// Colliding local ids remap a captured type name and a later unknown-name draft across distinct paths.
#[test]
fn prepared_batch_parity_remaps_type_names_paths_and_path_row_offsets() {
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
            let path_end = diagnostics.len();
            PremergeDiagnosticBatch::from_parts(
                diagnostics,
                strings,
                vec![RenderTypeContext {
                    diagnostic_range: 0..1,
                    type_environment: environment,
                }],
                vec![RenderPathContext {
                    diagnostic_range: 0..path_end,
                    path_table: explicit,
                }],
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

    let (prepared, vessel) =
        assert_prepared_batches_match_vessel(make_batches, InstallMode::Frozen);
    for failure in [&prepared, &vessel] {
        let rendered = frozen_render(failure);
        assert_eq!(rendered.len(), 5);
        assert!(rendered[0].message.contains("TAlpha"));
        assert!(rendered[1].message.contains("pkg/alpha-only.moth"));
        assert!(rendered[2].message.contains("TBeta"));
        assert!(rendered[3].message.contains("pkg/beta-only.moth"));
        assert!(rendered[4].message.contains("missing-beta"));
    }
}

/// Shared explicit tables stay shared, while later distinct tables retain their own path rows.
#[test]
fn prepared_batch_parity_preserves_shared_and_distinct_explicit_path_tables() {
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
            PremergeDiagnosticBatch::from_parts(
                vec![CompilerDiagnostic::missing_import_target(
                    path,
                    Some(inline_span(source, 0, 4)),
                )],
                strings.clone(),
                Vec::new(),
                vec![RenderPathContext {
                    diagnostic_range: 0..1,
                    path_table,
                }],
            )
        };
        let owner = || Some((Arc::clone(&database), Some(identity.clone())));
        vec![
            (make_batch(alpha, Arc::clone(&shared_table)), owner()),
            (make_batch(beta, Arc::clone(&shared_table)), owner()),
            (make_batch(gamma, Arc::clone(&distinct_table)), owner()),
        ]
    };

    let (prepared, vessel) =
        assert_prepared_batches_match_vessel(make_batches, InstallMode::Frozen);
    for failure in [&prepared, &vessel] {
        let rendered = frozen_render(failure);
        assert_eq!(rendered.len(), 3);
        assert!(rendered[0].message.contains("pkg/alpha-shared.moth"));
        assert!(rendered[1].message.contains("pkg/beta-shared.moth"));
        assert!(rendered[2].message.contains("vendor/gamma-distinct.moth"));
    }
}

/// An empty prepared donor row must still install the identity handle used by an earlier batch.
#[test]
fn prepared_batch_parity_installs_a_foreign_donor_without_its_own_diagnostic() {
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
        let diagnosed = PremergeDiagnosticBatch::from_diagnostic(diagnostic, strings.clone());
        let empty = PremergeDiagnosticBatch::from_diagnostics(Vec::new(), strings);
        vec![
            (diagnosed, None),
            (
                empty,
                Some((Arc::clone(&donor_database), Some(donor_identity.clone()))),
            ),
        ]
    };

    let (prepared, vessel) =
        assert_prepared_batches_match_vessel(make_batches, InstallMode::Frozen);
    for failure in [&prepared, &vessel] {
        let rendered = frozen_render(failure);
        assert_eq!(rendered.len(), 1);
        assert!(primary(&rendered[0]).path.ends_with("donor.moth"));
        assert_eq!(primary(&rendered[0]).line, "donor");
    }
}

#[test]
fn prepared_batch_without_type_context_retains_unresolved_type_ids() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let make_batches = || {
        let mut strings = StringTable::new();
        let (database, source) = retained_database(dir.path(), "main.moth", "main", &mut strings);
        let diagnostic = CompilerDiagnostic::type_mismatch(
            TypeId(999),
            TypeId(1000),
            TypeMismatchContext::Assignment,
            Some(inline_span(source, 0, 4)),
        );
        vec![(
            PremergeDiagnosticBatch::from_diagnostic(diagnostic, strings),
            Some((Arc::new(database), None)),
        )]
    };

    let (prepared, vessel) =
        assert_prepared_batches_match_vessel(make_batches, InstallMode::Frozen);
    for failure in [&prepared, &vessel] {
        let rendered = frozen_render(failure);
        assert!(rendered[0].message.contains("TypeId("));
    }
}

#[test]
fn prepared_batch_type_display_capacity_fault_keeps_the_vessel_span_and_code() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let make_batches = || {
        let mut strings = StringTable::new();
        let (database, source) = retained_database(dir.path(), "main.moth", "main", &mut strings);
        let diagnostic = CompilerDiagnostic::type_mismatch(
            TypeId(999),
            TypeId(1000),
            TypeMismatchContext::Assignment,
            Some(inline_span(source, 0, 4)),
        );
        vec![(
            PremergeDiagnosticBatch::from_diagnostic(diagnostic, strings),
            Some((Arc::new(database), None)),
        )]
    };
    let prepared_batches = make_batches();
    let vessel_batches = make_batches();
    let _exhausted = ForcedTypeDisplayExhaustionGuard::new();
    let (prepared, vessel) =
        assert_prepared_batch_parity(prepared_batches, vessel_batches, InstallMode::Frozen);

    for failure in [&prepared, &vessel] {
        let rendered = frozen_render(failure);
        assert_eq!(
            rendered[0].message,
            "This source exceeds the compiler's capacity for diagnostic type displays."
        );
        assert_eq!(failure.reports.report(0).len(), 1);
    }
    assert_eq!(
        primary(&frozen_render(&prepared)[0]),
        primary(&frozen_render(&vessel)[0])
    );
}

#[test]
fn prepared_batch_path_capacity_fault_matches_the_vessel_code_and_span() {
    let dir = tempfile::tempdir().expect("should create temp dir");
    let make_batches = || {
        let mut strings = StringTable::new();
        let mut paths = PathInternerBuilder::new();
        let new_path = paths
            .try_intern_portable_path("pkg/new/mod.moth", &mut strings)
            .expect("portable path should intern");
        let explicit = Arc::new(paths.freeze());
        let (database, source) = retained_database(dir.path(), "main.moth", "main", &mut strings);
        let batch = PremergeDiagnosticBatch::from_parts(
            vec![CompilerDiagnostic::missing_import_target(
                new_path,
                Some(inline_span(source, 0, 4)),
            )],
            strings,
            Vec::new(),
            vec![RenderPathContext {
                diagnostic_range: 0..1,
                path_table: explicit,
            }],
        );
        vec![(batch, Some((Arc::new(database), None)))]
    };
    let prepared_batches = make_batches();
    let vessel_batches = make_batches();
    let _exhausted = ForcedExhaustionGuard::new();
    let (prepared, vessel) =
        assert_prepared_batch_parity(prepared_batches, vessel_batches, InstallMode::Frozen);

    assert_eq!(prepared.reports.report(0).len(), 1);
    assert_eq!(vessel.reports.report(0).len(), 1);
}

#[test]
fn prepared_batch_owning_site_capture_matches_premerge_contexts() {
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
        DiagnosticBag::from_diagnostics(diagnostics.clone()),
        strings.clone(),
        Some(&environment),
        true,
        Some(Arc::clone(&path_table)),
    );
    let premerge = PremergeDiagnosticBatch::from_parts(
        diagnostics,
        strings,
        vec![RenderTypeContext {
            diagnostic_range: 0..2,
            type_environment: environment,
        }],
        vec![RenderPathContext {
            diagnostic_range: 0..2,
            path_table,
        }],
    );
    let expected = PreparedDiagnosticBatch::from_premerge(premerge);

    let mut captured_run = BoundaryAccumulator::new();
    captured_run.append_prepared(captured, None);
    let captured = captured_run.finish(InstallMode::Frozen);
    let mut expected_run = BoundaryAccumulator::new();
    expected_run.append_prepared(expected, None);
    let expected = expected_run.finish(InstallMode::Frozen);

    let expected_render = frozen_render(&expected);
    let captured_render = frozen_render(&captured);
    assert_render_parity(&expected_render, &captured_render);
    assert!(captured_render[0].message.contains("TOwningSite"));
    assert!(
        captured_render[1]
            .message
            .contains("pkg/owning-site-only.moth")
    );
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
fn prepared_batch_owning_site_type_capacity_fault_matches_vessel_span() {
    let dir = tempfile::tempdir().expect("should create temp dir");

    let mut captured_strings = StringTable::new();
    let (captured_database, captured_source) =
        retained_database(dir.path(), "main.moth", "main", &mut captured_strings);
    let captured_name = captured_strings.intern("TCapturedCapacity");
    let mut captured_environment = TypeEnvironment::new();
    let captured_type = captured_environment.register_synthetic_generic_parameter(captured_name);
    let captured_bool = captured_environment.builtins().bool;
    let captured_diagnostic = CompilerDiagnostic::type_mismatch(
        captured_type,
        captured_bool,
        TypeMismatchContext::Assignment,
        Some(inline_span(captured_source, 0, 4)),
    );
    let captured_path_table = Arc::new(PathInternerBuilder::new().freeze());

    let mut vessel_strings = StringTable::new();
    let (vessel_database, vessel_source) =
        retained_database(dir.path(), "main.moth", "main", &mut vessel_strings);
    let vessel_name = vessel_strings.intern("TCapturedCapacity");
    let mut vessel_environment = TypeEnvironment::new();
    let vessel_type = vessel_environment.register_synthetic_generic_parameter(vessel_name);
    let vessel_bool = vessel_environment.builtins().bool;
    let vessel_diagnostic = CompilerDiagnostic::type_mismatch(
        vessel_type,
        vessel_bool,
        TypeMismatchContext::Assignment,
        Some(inline_span(vessel_source, 0, 4)),
    );
    let mut vessel_messages =
        CompilerMessages::from_diagnostics(vec![vessel_diagnostic], vessel_strings)
            .with_type_context_for_all_diagnostics(vessel_environment);
    vessel_messages.set_source_database(Arc::new(vessel_database));
    vessel_messages
        .attach_path_table_if_missing(Arc::clone(&captured_path_table))
        .expect("test type context should pair with the path table");
    let legacy = legacy_render(&vessel_messages);

    let _exhausted = ForcedTypeDisplayExhaustionGuard::new();
    let captured = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(captured_diagnostic),
        captured_strings,
        Some(&captured_environment),
        true,
        Some(captured_path_table),
    );
    let mut captured_run = BoundaryAccumulator::new();
    captured_run.append_prepared(captured, Some((Arc::new(captured_database), None)));
    let captured = captured_run.finish(InstallMode::Frozen);
    let vessel = into_frozen_boundary_failure(vessel_messages);

    let captured_render = frozen_render(&captured);
    let vessel_render = frozen_render(&vessel);
    assert_render_parity(&vessel_render, &captured_render);
    assert_eq!(primary(&captured_render[0]), primary(&legacy[0]));
    let codes = |failure: &BoundaryFailure| {
        failure
            .reports
            .iter()
            .map(|(_, view)| view.descriptor().code)
            .collect::<Vec<_>>()
    };
    assert_eq!(codes(&captured), codes(&vessel));
}
