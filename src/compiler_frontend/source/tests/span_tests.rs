use super::{
    ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceDatabaseBuilder, SourceId,
    SpanCapacityReason,
    span::{SourceSpan, SpanJoinError},
};

use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::compiler_bug::CompilerBugReport;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::SourceSpanCapacityView;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, SourceSpanCapacityResource};

use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
use crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareFailure;
use crate::compiler_frontend::pipeline::CompilerFrontend;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::test_support::database_with_retained_text;

fn assert_source_loading_compiler_bug(
    panic: Box<dyn std::any::Any + Send>,
    expected_message: &str,
) {
    let report = panic
        .downcast_ref::<CompilerBugReport>()
        .unwrap_or_else(|| panic!("compiler bug panic should carry a CompilerBugReport"));
    assert_eq!(report.stage, CompilerStage::SourceLoading);
    assert_eq!(report.message.as_ref(), expected_message);
}

#[test]
fn source_finalization_preserves_live_record_resolution() {
    let (database, source_id) = database_with_retained_text("source snapshot");
    let mut sources = SourceDatabaseBuilder::new(database);
    let mut builder = sources.split().1.take_span_builder(source_id);
    let inline = LocalSpan::exact(12, 8, &mut builder).expect("inline span");
    let extended = LocalSpan::exact(5_000, 2_000, &mut builder).expect("extended span");
    let empty_extended =
        LocalSpan::exact(4 * 1024 * 1024, 0, &mut builder).expect("extended zero-length span");

    let live_ranges = [
        inline.resolve_with(builder.resolver()),
        extended.resolve_with(builder.resolver()),
        empty_extended.resolve_with(builder.resolver()),
    ];
    sources.retain_span_builder(source_id, builder);
    sources
        .finish_source(source_id)
        .expect("the source should finalize after its producer returns");
    sources
        .assemble_finalized_tables()
        .expect("the frozen table should assemble into its source record");
    let database = sources
        .finish()
        .expect("the assembled table should satisfy source invariants");

    let inline_range = SourceSpan::new(source_id, inline).byte_range(&database);
    let extended_range = SourceSpan::new(source_id, extended).byte_range(&database);
    let empty_extended_range = SourceSpan::new(source_id, empty_extended).byte_range(&database);
    assert_eq!(inline_range, live_ranges[0]);
    assert_eq!(extended_range, live_ranges[1]);
    assert_eq!(empty_extended_range, live_ranges[2]);
    assert!(!(inline_range.start() == inline_range.end()));
    assert!(empty_extended_range.start() == empty_extended_range.end());
}

#[test]
fn source_database_finalizes_one_record_while_another_remains_live() {
    let first_path = PathBuf::from("/project/first.moth");
    let second_path = PathBuf::from("/project/second.moth");
    let mut string_table = StringTable::new();
    let mut database = SourceDatabase::build(
        [&first_path, &second_path],
        &first_path,
        None,
        &mut string_table,
    )
    .expect("source identities should build");
    let first_id = database
        .get_by_canonical_path(&first_path)
        .expect("first source should be registered")
        .id;
    let second_id = database
        .get_by_canonical_path(&second_path)
        .expect("second source should be registered")
        .id;
    database
        .retain_text(first_id, "a".repeat(8_000))
        .expect("first source text should be retained");
    database
        .retain_text(second_id, "b".repeat(8_000))
        .expect("second source text should be retained");

    let mut sources = SourceDatabaseBuilder::new(database);
    let mut first_builder = sources.split().1.take_span_builder(first_id);
    let first_local = LocalSpan::exact(5_000, 1_000, &mut first_builder).expect("first span");
    let first_range = first_local.resolve_with(first_builder.resolver());
    sources.retain_span_builder(first_id, first_builder);

    let mut second_builder = sources.split().1.take_span_builder(second_id);
    let second_first_local =
        LocalSpan::exact(5_000, 1_000, &mut second_builder).expect("second source first span");
    let second_first_range = second_first_local.resolve_with(second_builder.resolver());
    sources.retain_span_builder(second_id, second_builder);

    sources
        .finish_source(first_id)
        .expect("the first source should finalize independently");
    assert!(
        sources
            .is_source_finalized(first_id)
            .expect("the first source should remain registered")
    );
    assert!(
        !sources
            .is_source_finalized(second_id)
            .expect("the second source should remain registered"),
        "the second source must remain live while the first is frozen"
    );
    let duplicate_error = sources
        .finish_source(first_id)
        .expect_err("the first source must not finalize twice");
    assert_eq!(duplicate_error.kind, InfrastructureFailureKind::Internal);

    let mut second_builder = sources.split().1.take_span_builder(second_id);
    let second_later_local =
        LocalSpan::exact(7_000, 1_000, &mut second_builder).expect("second source later span");
    let second_later_range = second_later_local.resolve_with(second_builder.resolver());
    sources.retain_span_builder(second_id, second_builder);
    sources
        .finish_source(second_id)
        .expect("the second source should finalize after its later producer");

    sources
        .assemble_finalized_tables()
        .expect("both frozen tables should assemble into their records");
    let database = sources
        .finish()
        .expect("both assembled tables should satisfy source invariants");
    assert_eq!(
        SourceSpan::new(first_id, first_local).byte_range(&database),
        first_range
    );
    assert_eq!(
        SourceSpan::new(second_id, second_first_local).byte_range(&database),
        second_first_range
    );
    assert_eq!(
        SourceSpan::new(second_id, second_later_local).byte_range(&database),
        second_later_range
    );
}

#[test]
fn repeated_source_preparation_preserves_earlier_extended_spans() {
    let text = format!("{}{}", "a".repeat(2_000), "b".repeat(2_000));
    let (database, source_id) = database_with_retained_text(&text);
    let mut sources = SourceDatabaseBuilder::new(database);
    let mut first = sources.split().1.take_span_builder(source_id);
    let first_span = LocalSpan::exact(0, 2_000, &mut first).expect("first extended span");
    sources.retain_span_builder(source_id, first);

    let (_, mut builders) = sources.split();
    let mut repeated = builders.take_span_builder(source_id);
    let repeated_span =
        LocalSpan::exact(2_000, 2_000, &mut repeated).expect("repeated extended span");
    builders.retain_span_builder(source_id, repeated);
    sources
        .finish_source(source_id)
        .expect("the source should finalize after all producers return");
    sources
        .assemble_finalized_tables()
        .expect("the frozen table should assemble");
    let database = sources
        .finish()
        .expect("all producers returned their builders");
    let first = SourceSpan::new(source_id, first_span).byte_range(&database);
    let repeated = SourceSpan::new(source_id, repeated_span).byte_range(&database);
    let snapshot = database
        .retained_text(source_id)
        .expect("retained snapshot");
    assert_eq!(
        &snapshot[first.start() as usize..first.end() as usize],
        "a".repeat(2_000)
    );
    assert_eq!(
        &snapshot[repeated.start() as usize..repeated.end() as usize],
        "b".repeat(2_000)
    );
}

#[test]
fn source_finalization_rejects_a_checked_out_span_builder() {
    let (database, source_id) = database_with_retained_text("source snapshot");
    let mut sources = SourceDatabaseBuilder::new(database);
    let _active_producer = sources.split().1.take_span_builder(source_id);
    let error = sources
        .finish_source(source_id)
        .expect_err("a checked-out builder must be returned before finalization");
    assert_eq!(error.kind, InfrastructureFailureKind::Internal);
    assert!(error.message.contains("outstanding span producer"));
}

#[test]
#[should_panic(expected = "source context escaped before span finalization")]
fn source_finalization_rejects_an_outstanding_transient_share() {
    let (database, _source_id) = database_with_retained_text("source snapshot");
    let builder = SourceDatabaseBuilder::new(database);
    let _transient = Arc::clone(builder.sources());
    let _ = builder.finish();
}

#[test]
fn finalized_source_rejects_span_builder_take_and_retain() {
    let (database, source_id) = database_with_retained_text("source snapshot");
    let mut sources = SourceDatabaseBuilder::new(database);
    sources
        .finish_source(source_id)
        .expect("an unprepared loaded source can finalize with an empty table");

    let take_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = sources.split().1.take_span_builder(source_id);
    }))
    .expect_err("a finalized source must not yield another builder");
    assert_source_loading_compiler_bug(
        take_panic,
        &format!(
            "source {} was already finalized and cannot take another span builder; this is a compiler bug",
            source_id.index()
        ),
    );

    let retain_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sources.retain_span_builder(source_id, ExtendedSpanBuilder::new());
    }))
    .expect_err("a finalized source must not accept another builder");
    assert_source_loading_compiler_bug(
        retain_panic,
        &format!(
            "source {} was already finalized and cannot retain another span builder; this is a compiler bug",
            source_id.index()
        ),
    );
}

#[test]
fn source_finalization_rejects_a_second_finalization() {
    let (database, source_id) = database_with_retained_text("source snapshot");
    let mut sources = SourceDatabaseBuilder::new(database);
    sources
        .finish_source(source_id)
        .expect("the loaded source should finalize once");

    let error = sources
        .finish_source(source_id)
        .expect_err("a source can finalize only once");

    assert_eq!(error.kind, InfrastructureFailureKind::Internal);
    assert!(
        error.message.contains(&source_id.index().to_string())
            && error.message.contains("finalized more than once"),
        "the duplicate finalization should name the source and lifecycle violation: {}",
        error.message
    );
}

#[test]
fn selected_source_finalization_before_retention_survives_adoption() {
    let source_path = PathBuf::from("/project/pending.moth");
    let mut string_table = StringTable::new();
    let database = SourceDatabase::build(
        std::iter::once(&source_path),
        &source_path,
        None,
        &mut string_table,
    )
    .expect("source identity should build");
    let source_id = database
        .get_by_canonical_path(&source_path)
        .expect("source should be registered")
        .id;

    let mut sources = SourceDatabaseBuilder::new(database);
    let mut span_builder = sources.split().1.take_span_builder(source_id);
    let local = LocalSpan::exact(5_000, 2_000, &mut span_builder).expect("extended span");
    let expected_range = local.resolve_with(span_builder.resolver());
    sources.retain_span_builder(source_id, span_builder);
    sources
        .finish_source(source_id)
        .expect("a selected pending source should freeze after its producer returns");
    assert!(
        sources
            .is_source_finalized(source_id)
            .expect("the pending source should remain registered")
    );

    sources
        .sources_mut()
        .retain_text(source_id, "x".repeat(8_000))
        .expect("selected source text should be retained");
    sources.adopt_pending_span_builders();
    assert!(
        sources
            .is_source_finalized(source_id)
            .expect("the retained source should remain registered")
    );
    sources
        .assemble_finalized_tables()
        .expect("the adopted table should assemble into its record");
    let database = sources
        .finish()
        .expect("the assembled source should satisfy lifecycle invariants");
    assert_eq!(
        SourceSpan::new(source_id, local).byte_range(&database),
        expected_range
    );
}

#[test]
fn source_finalization_rejects_the_compilation_root() {
    let database = SourceDatabase::empty();
    let mut sources = SourceDatabaseBuilder::new(database);

    let error = sources
        .finish_source(SourceId::COMPILATION_ROOT)
        .expect_err("the compilation root cannot finalize an extended-span table");

    assert_eq!(error.kind, InfrastructureFailureKind::Internal);
    assert!(
        error
            .message
            .contains(&SourceId::COMPILATION_ROOT.index().to_string())
            && error.message.contains("before its source was loaded"),
        "the root failure should name the source and lifecycle violation: {}",
        error.message
    );
}

#[test]
fn extended_span_without_an_installed_table_names_its_source_identity() {
    let (database, source_id) = database_with_retained_text("source snapshot");
    let mut builder = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(5_000, 2_000, &mut builder).expect("extended span");
    let span = SourceSpan::new(source_id, local);

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        span.byte_range(&database);
    }))
    .expect_err("an extended span cannot resolve before its builder is installed");
    assert_source_loading_compiler_bug(
        panic,
        &format!(
            "extended span builder for source identity {} was never installed; this is a compiler bug",
            source_id.index()
        ),
    );
}

#[test]
fn database_span_reads_reject_missing_source_records() {
    let source_path = PathBuf::from("/project/pending.moth");
    let mut string_table = StringTable::new();
    let database = SourceDatabase::build(
        std::iter::once(&source_path),
        &source_path,
        None,
        &mut string_table,
    )
    .expect("source identity should build");
    let pending_id = database
        .get_by_canonical_path(&source_path)
        .expect("source should be registered")
        .id;
    let mut builder = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(12, 8, &mut builder).expect("inline span");

    for source_id in [
        SourceId::from_index(0),
        pending_id,
        SourceId::from_index(99),
    ] {
        let span = SourceSpan::new(source_id, local);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            span.byte_range(&database);
        }))
        .expect_err("a span must not resolve without its source record");
        let expected_message = if source_id == SourceId::COMPILATION_ROOT {
            format!(
                "span on compilation root source identity {} resolves only the exact empty range [0, 0), not [12, 20); this is a compiler bug",
                source_id.index()
            )
        } else if source_id == pending_id {
            format!(
                "source identity {} has no loaded source record; this is a compiler bug",
                source_id.index()
            )
        } else {
            format!(
                "source identity {} is absent from the source database; this is a compiler bug",
                source_id.index()
            )
        };
        assert_source_loading_compiler_bug(panic, &expected_message);
    }
}

#[test]
fn compilation_root_span_resolves_only_its_exact_empty_range() {
    let source_path = PathBuf::from("/project/main.moth");
    let mut string_table = StringTable::new();
    let database = SourceDatabase::build(
        std::iter::once(&source_path),
        &source_path,
        None,
        &mut string_table,
    )
    .expect("source identity should build");

    let root = SourceSpan::new(SourceId::COMPILATION_ROOT, LocalSpan::source_start());
    let range = root.byte_range(&database);
    assert_eq!((range.start(), range.end()), (0, 0));
}

#[test]
fn non_empty_compilation_root_spans_fail_loudly_as_compiler_bugs() {
    let database = SourceDatabase::empty();
    let mut builder = ExtendedSpanBuilder::new();

    let root_identity = SourceId::COMPILATION_ROOT.index();
    for (local, expected_message) in [
        (
            LocalSpan::exact(4, 3, &mut builder).expect("non-empty inline root span"),
            format!(
                "span on compilation root source identity {root_identity} resolves only the exact empty range [0, 0), not [4, 7); this is a compiler bug"
            ),
        ),
        (
            LocalSpan::exact(7, 0, &mut builder).expect("non-zero empty root span"),
            format!(
                "span on compilation root source identity {root_identity} resolves only the exact empty range [0, 0), not [7, 7); this is a compiler bug"
            ),
        ),
        (
            LocalSpan::exact(5_000, 2_000, &mut builder).expect("extended root span"),
            format!(
                "span on compilation root source identity {root_identity} names extended row 0, but the root owns no extended-span table; only the exact empty range [0, 0) resolves on the root; this is a compiler bug"
            ),
        ),
    ] {
        let span = SourceSpan::new(SourceId::COMPILATION_ROOT, local);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            span.byte_range(&database);
        }))
        .expect_err("a non-empty compilation-root span must not resolve silently");
        assert_source_loading_compiler_bug(panic, &expected_message);
    }
}

#[test]
fn database_span_resolution_uses_each_source_extended_table() {
    let first_path = PathBuf::from("/project/first.moth");
    let second_path = PathBuf::from("/project/second.moth");
    let mut string_table = StringTable::new();
    let mut database = SourceDatabase::build(
        [&first_path, &second_path],
        &first_path,
        None,
        &mut string_table,
    )
    .expect("source identities should build");
    let first_id = database
        .get_by_canonical_path(&first_path)
        .expect("first source should be registered")
        .id;
    let second_id = database
        .get_by_canonical_path(&second_path)
        .expect("second source should be registered")
        .id;
    database
        .retain_text(first_id, "first source".to_owned())
        .expect("first source text should be retained");
    database
        .retain_text(second_id, "second source".to_owned())
        .expect("second source text should be retained");

    let mut sources = SourceDatabaseBuilder::new(database);
    let mut first_builder = sources.split().1.take_span_builder(first_id);
    let first_local = LocalSpan::exact(5_000, 5_000, &mut first_builder).expect("first span");
    let first_span = SourceSpan::new(first_id, first_local);
    let first_range = first_local.resolve_with(first_builder.resolver());
    sources.retain_span_builder(first_id, first_builder);

    let mut second_builder = sources.split().1.take_span_builder(second_id);
    let second_local = LocalSpan::exact(7_000, 3_000, &mut second_builder).expect("second span");
    let second_span = SourceSpan::new(second_id, second_local);
    let second_range = second_local.resolve_with(second_builder.resolver());
    sources.retain_span_builder(second_id, second_builder);

    sources
        .finish_source(first_id)
        .expect("first source should finalize");
    sources
        .finish_source(second_id)
        .expect("second source should finalize");
    sources
        .assemble_finalized_tables()
        .expect("both source tables should assemble");
    let database = sources
        .finish()
        .expect("source span tables should satisfy lifecycle invariants");

    assert_eq!(first_span.byte_range(&database), first_range);
    assert_eq!(second_span.byte_range(&database), second_range);
}

#[test]
fn local_and_source_spans_use_the_non_zero_option_niche() {
    assert_eq!(size_of::<LocalSpan>(), 4);
    assert_eq!(size_of::<Option<LocalSpan>>(), 4);
    assert_eq!(size_of::<SourceSpan>(), 8);
    assert_eq!(size_of::<Option<SourceSpan>>(), 8);
}

#[test]
fn largest_inline_length_encodes_inline_and_one_more_spills() {
    let mut builder = ExtendedSpanBuilder::new();
    let inline_span = LocalSpan::exact(0, 1022, &mut builder).expect("max inline length");
    let inline_range = inline_span.resolve_with(builder.resolver());

    assert!(builder.is_empty());
    assert_eq!(inline_range.start(), 0);
    assert_eq!(inline_range.end(), 1022);

    let extended_span =
        LocalSpan::exact(0, 1023, &mut builder).expect("one past max inline length");
    let extended_range = extended_span.resolve_with(builder.resolver());

    assert_eq!(builder.len(), 1);
    assert_eq!(extended_range.start(), 0);
    assert_eq!(extended_range.end(), 1023);
}

#[test]
fn largest_inline_start_encodes_inline_and_one_more_spills() {
    let mut builder = ExtendedSpanBuilder::new();
    let inline_start = 4 * 1024 * 1024 - 1;
    let inline_span = LocalSpan::exact(inline_start, 0, &mut builder).expect("max inline start");
    let inline_range = inline_span.resolve_with(builder.resolver());

    assert!(builder.is_empty());
    assert_eq!(inline_range.start(), inline_start);
    assert_eq!(inline_range.end(), inline_start);

    let spilled_start = 4 * 1024 * 1024;
    let extended_span =
        LocalSpan::exact(spilled_start, 0, &mut builder).expect("one past max inline start");
    let extended_range = extended_span.resolve_with(builder.resolver());

    assert_eq!(builder.len(), 1);
    assert_eq!(extended_range.start(), spilled_start);
    assert_eq!(extended_range.end(), spilled_start);
}

#[test]
fn largest_inline_start_and_length_encode_inline_without_spilling() {
    let mut builder = ExtendedSpanBuilder::new();
    let start = 4 * 1024 * 1024 - 1;
    let span = LocalSpan::exact(start, 1022, &mut builder)
        .expect("max inline start packed with max inline length");
    let range = span.resolve_with(builder.resolver());

    assert!(builder.is_empty());
    assert_eq!(range.start(), start);
    assert_eq!(range.end(), start + 1022);
}

#[test]
fn last_usable_extended_index_encodes_and_one_past_it_is_capacity_error() {
    let mut builder = ExtendedSpanBuilder::new();
    let last_usable_index = 4_194_302_u32;

    for _ in 0..=last_usable_index {
        LocalSpan::exact(0, 1023, &mut builder)
            .expect("every index through the last usable extended slot must encode");
    }

    assert_eq!(builder.len(), last_usable_index as usize + 1);

    let error = LocalSpan::exact(0, 1023, &mut builder)
        .expect_err("one past the last usable extended index must fail");

    assert_eq!(error.start(), 0);
    assert_eq!(error.length(), 1023);
    assert_eq!(error.reason(), SpanCapacityReason::ExtendedTableFull);

    // The real exhausted table must classify a long authored token as a typed source diagnostic
    // at the tokenizer boundary. The rejected interval remains in typed facts, while the primary
    // span uses the file-owned inline source-start anchor and does not append another row.
    let source = format!("\"{}\"", "x".repeat(1023));
    let path = Path::new("capacity.moth");
    let mut strings = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let malformed_path = Path::new("unterminated.moth");
    let malformed_source = source[..source.len() - 1].to_owned();
    let mut sources =
        SourceDatabase::build([path, malformed_path], path, None, &mut strings).unwrap();
    let source_id = sources.get_by_canonical_path(path).unwrap().id;
    let malformed_id = sources.get_by_canonical_path(malformed_path).unwrap().id;
    sources.retain_text(source_id, source.clone()).unwrap();
    sources
        .retain_text(malformed_id, malformed_source.clone())
        .unwrap();
    let failure = CompilerFrontend::tokenize_source(
        &sources,
        &StyleDirectiveRegistry::built_ins(),
        &source,
        path,
        TokenizerEntryMode::SourceFile,
        &mut strings,
        &mut path_fork,
        &mut builder,
    )
    .expect_err("minting a long token must report exhausted source storage");
    let FileFrontendPrepareFailure::Diagnosed(error) = failure else {
        panic!("token storage exhaustion must remain an authored-source diagnosis");
    };
    let compact = compact_bag_for_test(DiagnosticBag::from(error.diagnostic), None);
    let diagnostic = compact
        .first()
        .expect("source-capacity diagnostic should compact");
    assert_eq!(diagnostic.descriptor().code, "MOTH-SYNTAX-0036");
    assert!(
        matches!(
            diagnostic.primary_site(),
            SiteRef::Local(span)
                if span == SourceSpan::new(source_id, LocalSpan::source_start())
        ),
        "capacity diagnostics must retain the owning file's inline source-start anchor"
    );
    let facts = diagnostic
        .family_view::<SourceSpanCapacityView>()
        .expect("capacity diagnostics retain typed source-capacity facts");
    assert_eq!((facts.start, facts.length), (0, source.len()));
    assert_eq!(facts.resource, SourceSpanCapacityResource::ExtendedSpan);

    assert_eq!(builder.len(), last_usable_index as usize + 1);
    let mut malformed_builder = ExtendedSpanBuilder::new();
    for _ in 0..=last_usable_index {
        LocalSpan::exact(0, 1023, &mut malformed_builder).unwrap();
    }
    let failure = CompilerFrontend::tokenize_source(
        &sources,
        &StyleDirectiveRegistry::built_ins(),
        &malformed_source,
        malformed_path,
        TokenizerEntryMode::SourceFile,
        &mut strings,
        &mut path_fork,
        &mut malformed_builder,
    )
    .expect_err("the unterminated literal must report exhausted source storage");
    let FileFrontendPrepareFailure::Diagnosed(error) = failure else {
        panic!("source-owned span exhaustion must remain an authored-source diagnosis");
    };
    let compact = compact_bag_for_test(DiagnosticBag::from(error.diagnostic), None);
    let diagnostic = compact
        .first()
        .expect("malformed-source capacity diagnostic should compact");
    assert_eq!(diagnostic.descriptor().code, "MOTH-SYNTAX-0036");
    assert!(
        matches!(
            diagnostic.primary_site(),
            SiteRef::Local(span)
                if span == SourceSpan::new(malformed_id, LocalSpan::source_start())
        ),
        "the malformed file must remain the diagnostic's owning source",
    );
    let facts = diagnostic
        .family_view::<SourceSpanCapacityView>()
        .expect("malformed-source capacity diagnostic retains typed facts");
    assert_eq!((facts.start, facts.length), (0, malformed_source.len()));
    assert_eq!(facts.resource, SourceSpanCapacityResource::ExtendedSpan);
    assert_eq!(malformed_builder.len(), last_usable_index as usize + 1);
}

#[test]
fn join_spills_to_extended_when_cover_exceeds_inline_length() {
    let mut builder = ExtendedSpanBuilder::new();
    let left = LocalSpan::exact(0, 600, &mut builder).expect("left");
    let right = LocalSpan::exact(500, 600, &mut builder).expect("right");
    let joined = left.join(right, &mut builder).expect("extended join");
    let range = joined.resolve_with(builder.resolver());

    assert_eq!(builder.len(), 1);
    assert_eq!(range.start(), 0);
    assert_eq!(range.end(), 1100);
}

#[test]
fn join_of_adjacent_spans_is_contiguous_cover() {
    let mut builder = ExtendedSpanBuilder::new();
    let left = LocalSpan::exact(0, 5, &mut builder).expect("left");
    let right = LocalSpan::exact(5, 5, &mut builder).expect("right");
    let joined = left.join(right, &mut builder).expect("adjacent join");
    let range = joined.resolve_with(builder.resolver());

    assert_eq!(range.start(), 0);
    assert_eq!(range.end(), 10);
}

#[test]
fn join_of_overlapping_spans_is_smallest_cover() {
    let mut builder = ExtendedSpanBuilder::new();
    let left = LocalSpan::exact(0, 8, &mut builder).expect("left");
    let right = LocalSpan::exact(4, 8, &mut builder).expect("right");
    let joined = left.join(right, &mut builder).expect("overlapping join");
    let range = joined.resolve_with(builder.resolver());

    assert_eq!(range.start(), 0);
    assert_eq!(range.end(), 12);
}

#[test]
fn cross_source_join_is_rejected() {
    let mut builder = ExtendedSpanBuilder::new();
    let left_local = LocalSpan::exact(0, 4, &mut builder).expect("left");
    let right_local = LocalSpan::exact(0, 4, &mut builder).expect("right");
    let left = SourceSpan::new(SourceId::from_index(1), left_local);
    let right = SourceSpan::new(SourceId::from_index(2), right_local);

    match left.join(right, &mut builder) {
        Err(SpanJoinError::DifferentSources {
            left: left_source,
            right: right_source,
        }) => {
            assert_eq!(left_source, SourceId::from_index(1));
            assert_eq!(right_source, SourceId::from_index(2));
        }
        other => panic!("expected a different-sources join error, got {other:?}"),
    }
}

/// A same-source join keeps the identity and covers both operands, extended rows included.
#[test]
fn same_source_join_keeps_the_identity_and_covers_an_extended_operand() {
    let mut builder = ExtendedSpanBuilder::new();
    let source = SourceId::from_index(3);
    let inline = SourceSpan::new(
        source,
        LocalSpan::exact(10, 4, &mut builder).expect("inline"),
    );
    let extended = SourceSpan::new(
        source,
        LocalSpan::exact(9_000, 5_000, &mut builder).expect("extended"),
    );

    let joined = extended
        .join(inline, &mut builder)
        .expect("same-source join");
    let resolver = builder.resolver_for(source);
    let range = joined.resolve_with(resolver);

    assert_eq!(joined.source(), source);
    assert_eq!((range.start(), range.end()), (10, 14_000));
}

/// A global span must never resolve through another source's extended table.
///
/// The foreign table here is deliberately long enough to serve the span's extended index: the
/// rejection must come from the source-identity check, not from an index-bounds accident. The
/// wrong pairing is a compiler bug even when the row it would return happens to exist.
#[test]
fn source_span_rejects_resolution_through_another_sources_resolver() {
    let mut first_builder = ExtendedSpanBuilder::new();
    let first_span = SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(5_000, 2_000, &mut first_builder).expect("first extended span"),
    );

    let mut second_builder = ExtendedSpanBuilder::new();
    let _second_row = LocalSpan::exact(7, 3, &mut second_builder).expect("second source's own row");
    let foreign_resolver = second_builder.resolver_for(SourceId::from_index(2));
    let bare_resolver = second_builder.resolver();

    let foreign_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        first_span.resolve_with(foreign_resolver);
    }))
    .expect_err("a global span must not resolve through another source's resolver");
    assert_source_loading_compiler_bug(
        foreign_panic,
        "span from source identity 1 cannot resolve through source identity 2; this is a compiler bug",
    );

    let bare_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        first_span.resolve_with(bare_resolver);
    }))
    .expect_err("a global span must not resolve through an unqualified resolver");
    assert_source_loading_compiler_bug(
        bare_panic,
        "span from source identity 1 resolved through an unqualified resolver; global spans require resolver_for(source); this is a compiler bug",
    );

    let own_range = first_span.resolve_with(first_builder.resolver_for(SourceId::from_index(1)));
    assert_eq!(
        (own_range.start(), own_range.end()),
        (5_000, 7_000),
        "the same-source qualified resolver must keep resolving its own table"
    );
}

/// The cover is the minimum start and the maximum end, whichever operand each comes from.
///
/// A cover that still fits inline must not consume an extended row.
#[test]
fn join_covers_both_operands_whatever_order_they_arrive_in() {
    let mut builder = ExtendedSpanBuilder::new();
    let early = LocalSpan::exact(10, 5, &mut builder).expect("early");
    let late = LocalSpan::exact(40, 6, &mut builder).expect("late");

    let forward = early.join(late, &mut builder).expect("forward join");
    let reversed = late.join(early, &mut builder).expect("reversed join");
    let resolver = builder.resolver();

    assert!(builder.is_empty());
    assert_eq!(
        forward.resolve_with(resolver),
        reversed.resolve_with(resolver)
    );
    assert_eq!(
        (
            forward.resolve_with(resolver).start(),
            forward.resolve_with(resolver).end()
        ),
        (10, 46)
    );
}

/// Joining a nested span must not widen the outer one.
#[test]
fn join_of_a_nested_span_returns_the_outer_cover() {
    let mut builder = ExtendedSpanBuilder::new();
    let outer = LocalSpan::exact(10, 20, &mut builder).expect("outer");
    let inner = LocalSpan::exact(15, 2, &mut builder).expect("inner");

    let joined = outer.join(inner, &mut builder).expect("nested join");
    let range = joined.resolve_with(builder.resolver());

    assert_eq!((range.start(), range.end()), (10, 30));
}

/// Each extended row must be reachable through its own span.
///
/// A codec that appended correctly but always encoded index zero would resolve every extended
/// span to the first row, which no boundary or capacity test can see.
#[test]
fn distinct_extended_rows_resolve_through_their_own_spans() {
    let mut builder = ExtendedSpanBuilder::new();
    let ranges = [(0_u32, 1023_u32), (7, 5000), (4_194_304, 0), (12, 1023)];
    let spans = ranges.map(|(start, length)| {
        LocalSpan::exact(start, length, &mut builder).expect("extended span")
    });

    assert_eq!(
        builder.len(),
        ranges.len(),
        "every range must take its own row"
    );

    let resolver = builder.resolver();
    let resolved = spans.map(|span| {
        let range = span.resolve_with(resolver);
        (range.start(), range.end() - range.start())
    });

    assert_eq!(resolved, ranges);
}

fn next_generated_span_value(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

#[test]
fn generated_spans_round_trip_through_live_and_frozen_resolvers() {
    use super::span_encoding::{
        DecodedSpan, INLINE_MAX_LENGTH, INLINE_START_LIMIT, decode_logical,
    };

    let boundary_starts = [
        0,
        1,
        INLINE_START_LIMIT - 1,
        INLINE_START_LIMIT,
        INLINE_START_LIMIT + 1,
    ];
    let boundary_lengths = [
        0,
        1,
        INLINE_MAX_LENGTH - 1,
        INLINE_MAX_LENGTH,
        INLINE_MAX_LENGTH + 1,
    ];
    let mut ranges = Vec::new();

    for start in boundary_starts {
        for length in boundary_lengths {
            ranges.push((start, length));
        }
    }
    for start in [u32::MAX - 1, u32::MAX] {
        ranges.push((start, 0));
    }

    let mut generator_state = 0x9e37_79b9;
    let extended_start_width = u64::from(u32::MAX - INLINE_START_LIMIT) + 1;

    for sample in 0..2_048 {
        let raw_start = next_generated_span_value(&mut generator_state);
        let start = match sample % 4 {
            0 => raw_start % INLINE_START_LIMIT,
            1 => {
                (u64::from(INLINE_START_LIMIT) + u64::from(raw_start) % extended_start_width) as u32
            }
            2 => raw_start,
            _ => u32::MAX - raw_start % 65_536,
        };
        let maximum_length = u64::from(u32::MAX - start);
        let raw_length = next_generated_span_value(&mut generator_state);
        let length = match sample % 8 {
            0 => raw_length % (INLINE_MAX_LENGTH + 1),
            1 if maximum_length >= u64::from(INLINE_MAX_LENGTH + 1) => {
                let minimum_extended_length = u64::from(INLINE_MAX_LENGTH + 1);
                (minimum_extended_length
                    + u64::from(raw_length) % (maximum_length - minimum_extended_length + 1))
                    as u32
            }
            _ => (u64::from(raw_length) % (maximum_length + 1)) as u32,
        };
        ranges.push((start, length));
    }

    let mut builder = ExtendedSpanBuilder::new();
    let mut spans = Vec::with_capacity(ranges.len());
    let mut inline_count = 0usize;
    let mut extended_count = 0usize;

    for &(start, length) in &ranges {
        let span = LocalSpan::exact(start, length, &mut builder)
            .expect("generated range must have a representable end");
        let expected_inline = start < INLINE_START_LIMIT && length <= INLINE_MAX_LENGTH;
        let actual_inline = matches!(
            decode_logical(span.logical_word()),
            DecodedSpan::Inline { .. }
        );

        assert_eq!(
            actual_inline,
            expected_inline,
            "range {start}..{} selected the wrong encoding regime",
            start + length
        );
        if actual_inline {
            inline_count += 1;
        } else {
            extended_count += 1;
        }
        spans.push(span);
    }

    assert!(
        inline_count >= 100 && extended_count >= 100,
        "generated sweep must exercise both regimes (inline={inline_count}, extended={extended_count})"
    );

    let live_ranges: Vec<_> = spans
        .iter()
        .map(|&span| span.resolve_with(builder.resolver()))
        .collect();
    let frozen_table = builder.freeze();

    for ((start, length), (span, live_range)) in ranges
        .iter()
        .copied()
        .zip(spans.iter().copied().zip(live_ranges))
    {
        let frozen_range = span.resolve_with(frozen_table.resolver());
        let expected_end = start + length;

        assert_eq!(
            (live_range.start(), live_range.end()),
            (start, expected_end),
            "live resolver changed generated range {start}..{expected_end}"
        );
        assert_eq!(
            (frozen_range.start(), frozen_range.end()),
            (start, expected_end),
            "frozen resolver changed generated range {start}..{expected_end}"
        );
        assert_eq!(
            live_range, frozen_range,
            "live and frozen resolvers disagree for generated range {start}..{expected_end}"
        );
    }
}

/// The `Option<LocalSpan>` niche depends on the codec never producing the all-ones logical word.
///
/// Nothing at the `LocalSpan` level can observe that word: `store_logical` panics before a span
/// exists, so the invariant lives in the codec's admissible domain. The inline extreme is a
/// compile-time assertion in `span_encoding`; this pins the extended bound and the reason for it.
#[test]
fn the_first_rejected_extended_index_is_the_one_that_would_reserve_the_logical_word() {
    use super::span_encoding::{
        DecodedSpan, LENGTH_BITS, LENGTH_SENTINEL, MAX_EXTENDED_INDEX, decode_logical,
        encode_extended_index,
    };

    let last_usable =
        encode_extended_index(MAX_EXTENDED_INDEX).expect("the last usable index must encode");

    assert!(last_usable < u32::MAX);
    assert!(matches!(
        decode_logical(last_usable),
        DecodedSpan::Extended { index } if index == MAX_EXTENDED_INDEX
    ));

    assert_eq!(
        encode_extended_index(MAX_EXTENDED_INDEX + 1),
        None,
        "the index past the last usable one must leave the codec's domain"
    );
    assert_eq!(
        ((MAX_EXTENDED_INDEX + 1) << LENGTH_BITS) | LENGTH_SENTINEL,
        u32::MAX,
        "that index is rejected because its encoding would be the reserved word"
    );
}

#[test]
fn exact_reports_end_overflow_at_u32_boundary_without_wrapping() {
    use super::span_encoding::{
        DecodedSpan, INLINE_MAX_LENGTH, INLINE_START_LIMIT, decode_logical,
    };

    let mut builder = ExtendedSpanBuilder::new();
    let inline = LocalSpan::exact(INLINE_START_LIMIT - 1, INLINE_MAX_LENGTH, &mut builder)
        .expect("the largest inline range should be representable");
    assert!(matches!(
        decode_logical(inline.logical_word()),
        DecodedSpan::Inline { .. }
    ));

    let extended = LocalSpan::exact(INLINE_START_LIMIT - 1, INLINE_MAX_LENGTH + 1, &mut builder)
        .expect("one past the inline length limit should spill to an extended row");
    assert!(matches!(
        decode_logical(extended.logical_word()),
        DecodedSpan::Extended { .. }
    ));

    // Inline starts and lengths are too small to overflow u32; the end boundary itself is
    // therefore exercised by extended ranges, while the preceding cases prove both exact paths.
    for start in [INLINE_START_LIMIT - 1, INLINE_START_LIMIT, u32::MAX - 1] {
        let maximum_length = u32::MAX - start;
        let span = LocalSpan::exact(start, maximum_length, &mut builder)
            .expect("a range ending at u32::MAX should be representable");
        let resolved = span.resolve_with(builder.resolver());
        assert_eq!((resolved.start(), resolved.end()), (start, u32::MAX));

        let error = LocalSpan::exact(start, maximum_length + 1, &mut builder)
            .expect_err("a range ending past u32::MAX must be rejected");
        assert_eq!(error.start(), start);
        assert_eq!(error.length(), maximum_length + 1);
        assert_eq!(error.reason(), SpanCapacityReason::EndUnrepresentable);
    }
}

#[test]
fn end_unrepresentable_capacity_is_a_compiler_bug() {
    let mut builder = ExtendedSpanBuilder::new();
    let error = LocalSpan::exact(u32::MAX, 1, &mut builder)
        .expect_err("a range beyond u32::MAX must be rejected");
    let source = SourceId::from_index(7);
    let panic = std::panic::catch_unwind(|| {
        crate::compiler_frontend::tokenizer::lexer::map_span_capacity_error(source, error)
    })
    .expect_err("an unrepresentable end must panic as a compiler bug");
    let report = panic
        .downcast_ref::<CompilerBugReport>()
        .expect("compiler bug panic should carry a structured report");
    assert_eq!(report.stage, CompilerStage::Tokenization);
    assert_eq!(
        report.message.as_ref(),
        "span at byte offset 4294967295 with length 1 ends past u32::MAX in a source that was accepted as addressable; this is a compiler bug"
    );
}
