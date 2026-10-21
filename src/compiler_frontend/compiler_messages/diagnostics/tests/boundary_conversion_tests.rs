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
    BoundaryFailure, CompilerError, CompilerMessages, RenderSourceContext,
};
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot_builder::ForcedTypeDisplayExhaustionGuard;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::legacy_adapter::into_boundary_failure;
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::render::subject::DiagnosticSubject;
use crate::compiler_frontend::compiler_messages::render::{
    DiagnosticPrimaryPosition, DiagnosticRenderContext, compact_label_message_text,
    resolve_label_render_facts_from_root,
};
use crate::compiler_frontend::compiler_messages::{
    BorrowAccessKind, BorrowDiagnosticKind, CompilerDiagnostic, DiagnosticKind, DiagnosticLabel,
    DiagnosticLabelMessage, DiagnosticPayload, DiagnosticPlace, SourceSpanCapacityResource,
    TypeMismatchContext,
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
            let subject = DiagnosticSubject::new(diagnostic, context);
            let position = subject
                .primary_position(context)
                .map(|position| owned_position(&position));
            let payload = subject.payload(context);
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
fn assert_legacy_parity(messages: CompilerMessages, install: InstallMode) -> Vec<Rendered> {
    let legacy = legacy_render(&messages);
    let failure = into_boundary_failure(messages, install);
    let frozen = frozen_render(&failure);
    assert_render_parity(&legacy, &frozen);
    frozen
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
    let failure = into_boundary_failure(messages, InstallMode::Frozen);
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
    let failure = into_boundary_failure(messages, InstallMode::Frozen);
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

    let failure = into_boundary_failure(messages, InstallMode::Frozen);
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
    let failure = into_boundary_failure(messages, InstallMode::Frozen);
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
        let failure = into_boundary_failure(messages, InstallMode::Frozen);
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
    let failure = into_boundary_failure(messages, InstallMode::Frozen);
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

    let failure = into_boundary_failure(messages, InstallMode::Frozen);
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
    let _failure = into_boundary_failure(messages, InstallMode::Frozen);
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
    let _failure = into_boundary_failure(messages, InstallMode::Frozen);
}

/// Source contexts frozen before conversion must be refused; the converter takes them unfrozen.
#[test]
#[should_panic(expected = "pre-frozen render contexts reached the boundary converter")]
fn pre_frozen_render_contexts_are_rejected_by_the_converter() {
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
    messages
        .freeze_source_contexts()
        .expect("source contexts should freeze");
    let _failure = into_boundary_failure(messages, InstallMode::Frozen);
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
    let _failure = into_boundary_failure(messages, InstallMode::Frozen);
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
    let _failure = into_boundary_failure(messages, InstallMode::Frozen);
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
    let _failure = into_boundary_failure(messages, InstallMode::Frozen);
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
