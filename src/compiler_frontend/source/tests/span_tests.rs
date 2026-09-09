use super::{
    ExtendedSpanBuilder, LocalSpan, SourceDatabase, SourceDatabaseBuilder, SourceId,
    SpanCapacityReason,
    span::{SourceSpan, SpanJoinError},
};

use std::cmp::Ordering;

use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::headers::parse_file_headers::FileFrontendPrepareFailure;
use crate::compiler_frontend::pipeline::CompilerFrontend;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::test_support::database_with_retained_text;

fn compiler_bug_panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = panic.downcast_ref::<&str>() {
        return (*message).to_owned();
    }
    panic!("compiler bug panic should carry a string message");
}

#[test]
fn installing_extended_spans_preserves_live_record_resolution() {
    let (mut database, source_id) = database_with_retained_text("source snapshot");
    let mut builder = ExtendedSpanBuilder::new();
    let inline = LocalSpan::exact(12, 8, &mut builder).expect("inline span");
    let extended = LocalSpan::exact(5_000, 2_000, &mut builder).expect("extended span");
    let empty_extended = LocalSpan::insertion_point(4 * 1024 * 1024, &mut builder)
        .expect("extended insertion point");

    let live_ranges = [
        inline.resolve_with(builder.resolver()),
        extended.resolve_with(builder.resolver()),
        empty_extended.resolve_with(builder.resolver()),
    ];
    let table = builder.freeze();
    database
        .install_extended_spans(source_id, table)
        .expect("the loaded source should install its table once");

    let record = database.source_record(source_id);
    assert_eq!(inline.resolve(record), live_ranges[0]);
    assert_eq!(extended.resolve(record), live_ranges[1]);
    assert_eq!(empty_extended.resolve(record), live_ranges[2]);
    assert!(!inline.is_empty(record));
    assert!(empty_extended.is_empty(record));
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
    let database = sources
        .finish()
        .expect("all producers returned their builders");
    let first = first_span.resolve(database.source_record(source_id));
    let repeated = repeated_span.resolve(database.source_record(source_id));
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
#[should_panic]
fn source_finalization_rejects_a_checked_out_span_builder() {
    let (database, source_id) = database_with_retained_text("source snapshot");
    let mut sources = SourceDatabaseBuilder::new(database);
    let _active_producer = sources.split().1.take_span_builder(source_id);
    let _ = sources.finish();
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
fn installing_extended_spans_twice_is_a_compiler_bug() {
    let (mut database, source_id) = database_with_retained_text("source snapshot");
    database
        .install_extended_spans(source_id, ExtendedSpanBuilder::new().freeze())
        .expect("the first table installation should succeed");

    let error = database
        .install_extended_spans(source_id, ExtendedSpanBuilder::new().freeze())
        .expect_err("a source can install its table only once");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains(&source_id.index().to_string()) && error.msg.contains("more than once"),
        "the duplicate installation should name the source and lifecycle violation: {}",
        error.msg
    );
}

#[test]
fn installing_extended_spans_for_a_source_that_never_loaded_is_a_compiler_bug() {
    let source_path = PathBuf::from("/project/pending.moth");
    let mut string_table = StringTable::new();
    let mut database = SourceDatabase::build(
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

    let error = database
        .install_extended_spans(source_id, ExtendedSpanBuilder::new().freeze())
        .expect_err("an unloaded source cannot install an extended-span table");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains(&source_id.index().to_string())
            && error.msg.contains("before its source was loaded"),
        "the unloaded-source failure should name the source and lifecycle violation: {}",
        error.msg
    );
}

#[test]
fn installing_extended_spans_for_the_compilation_root_is_a_compiler_bug() {
    let mut database = SourceDatabase::empty();
    let source_id = SourceId::from_index(0);

    let error = database
        .install_extended_spans(source_id, ExtendedSpanBuilder::new().freeze())
        .expect_err("the compilation root cannot install an extended-span table");

    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(
        error.msg.contains(&source_id.index().to_string())
            && error.msg.contains("compilation root"),
        "the root failure should name the source and lifecycle violation: {}",
        error.msg
    );
}

#[test]
fn inline_span_resolves_through_a_record_before_table_installation() {
    let (database, source_id) = database_with_retained_text("source snapshot");
    let mut builder = ExtendedSpanBuilder::new();
    let inline = LocalSpan::exact(12, 8, &mut builder).expect("inline span");
    assert!(builder.is_empty());

    let record = database.source_record(source_id);
    let range = inline.resolve(record);
    assert_eq!((range.start(), range.end()), (12, 20));
    assert!(!inline.is_empty(record));
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
    let message = compiler_bug_panic_message(panic);

    assert!(
        message.contains(&source_id.index().to_string())
            && message.contains("builder")
            && message.contains("never installed"),
        "the missing-table panic should name the source and lifecycle violation: {message}"
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
        let message = compiler_bug_panic_message(panic);

        assert!(
            message.contains(&source_id.index().to_string()) && message.contains("compiler bug"),
            "the missing source-record panic should name source {}: {message}",
            source_id.index()
        );
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
    assert_eq!(root.start(&database), 0);
    assert_eq!(root.end(&database), 0);
    assert!(root.contains(root, &database));
}

#[test]
fn non_empty_compilation_root_spans_fail_loudly_as_compiler_bugs() {
    let database = SourceDatabase::empty();
    let mut builder = ExtendedSpanBuilder::new();

    for local in [
        LocalSpan::exact(4, 3, &mut builder).expect("non-empty inline root span"),
        LocalSpan::insertion_point(7, &mut builder).expect("non-zero empty root span"),
        LocalSpan::exact(5_000, 2_000, &mut builder).expect("extended root span"),
    ] {
        let span = SourceSpan::new(SourceId::COMPILATION_ROOT, local);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            span.byte_range(&database);
        }))
        .expect_err("a non-empty compilation-root span must not resolve silently");
        let message = compiler_bug_panic_message(panic);

        assert!(
            message.contains("compilation root")
                && message.contains(&SourceId::COMPILATION_ROOT.index().to_string())
                && message.contains("compiler bug"),
            "the root range rejection should name the identity as a compiler bug: {message}"
        );
    }

    // The rejection must come from the span contract itself, not from a record lookup accident.
    let non_empty = SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact(4, 3, &mut builder).expect("non-empty inline root span"),
    );
    let overlap_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        non_empty.overlaps(non_empty, &database);
    }))
    .expect_err("a non-empty root span must fail the same way through overlaps");
    let message = compiler_bug_panic_message(overlap_panic);
    assert!(
        message.contains("compilation root") && message.contains("compiler bug"),
        "overlaps must route through the same root span contract: {message}"
    );
}

#[test]
fn database_span_operations_match_live_resolvers_and_reject_cross_source_pairs() {
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

    let mut first_builder = ExtendedSpanBuilder::new();
    let outer_local = LocalSpan::exact(5_000, 5_000, &mut first_builder).expect("outer span");
    let inner_local = LocalSpan::exact(6_000, 10, &mut first_builder).expect("inner span");
    let first_outer = SourceSpan::new(first_id, outer_local);
    let first_inner = SourceSpan::new(first_id, inner_local);
    let live_outer_range = outer_local.resolve_with(first_builder.resolver());
    let live_inner_range = inner_local.resolve_with(first_builder.resolver());
    let live_overlap = first_outer.overlaps_with(first_inner, first_builder.resolver_for(first_id));
    let live_containment =
        first_outer.contains_with(first_inner, first_builder.resolver_for(first_id));
    let first_table = first_builder.freeze();

    let mut second_builder = ExtendedSpanBuilder::new();
    let coincident_local =
        LocalSpan::exact(5_000, 5_000, &mut second_builder).expect("coincident span");
    let second_coincident = SourceSpan::new(second_id, coincident_local);
    let second_table = second_builder.freeze();

    database
        .install_extended_spans(first_id, first_table)
        .expect("first source should install its table");
    database
        .install_extended_spans(second_id, second_table)
        .expect("second source should install its table");

    assert_eq!(first_outer.byte_range(&database), live_outer_range);
    assert_eq!(first_inner.byte_range(&database), live_inner_range);
    assert_eq!(first_outer.start(&database), live_outer_range.start());
    assert_eq!(first_outer.end(&database), live_outer_range.end());
    assert_eq!(first_outer.overlaps(first_inner, &database), live_overlap);
    assert_eq!(
        first_outer.contains(first_inner, &database),
        live_containment
    );

    let first_coincident = SourceSpan::new(first_id, outer_local);
    assert_eq!(
        first_coincident.byte_range(&database),
        second_coincident.byte_range(&database)
    );
    assert!(!first_coincident.overlaps(second_coincident, &database));
    assert!(!first_coincident.contains(second_coincident, &database));
    assert!(!second_coincident.contains(first_coincident, &database));
    let absent_source = SourceSpan::new(SourceId::from_index(99), outer_local);
    let absent_cross_source = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        absent_source.overlaps(first_coincident, &database)
    }));
    assert!(
        absent_cross_source.is_ok(),
        "cross-source overlap must reject before source lookup"
    );
    assert!(!absent_cross_source.unwrap());
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
fn zero_length_span_at_offset_zero_round_trips() {
    let mut builder = ExtendedSpanBuilder::new();
    let span = LocalSpan::insertion_point(0, &mut builder).expect("empty span at byte 0");
    let range = span.resolve_with(builder.resolver());

    assert!(builder.is_empty());
    assert!(span.is_empty_with(builder.resolver()));
    assert_eq!(range.start(), 0);
    assert_eq!(range.end(), 0);
}

#[test]
fn zero_length_span_past_inline_start_limit_round_trips() {
    let mut builder = ExtendedSpanBuilder::new();
    let offset = 4 * 1024 * 1024;
    let span =
        LocalSpan::insertion_point(offset, &mut builder).expect("empty span past inline start");
    let range = span.resolve_with(builder.resolver());

    assert_eq!(builder.len(), 1);
    assert!(span.is_empty_with(builder.resolver()));
    assert_eq!(range.start(), offset);
    assert_eq!(range.end(), offset);
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

    // The real exhausted table must keep token construction failures in preparation's
    // infrastructure lane. The lexer diagnoses the unterminated literal before token
    // construction, then capture hits the exhausted table and aborts terminally: the
    // diagnosis is replaced by the capacity failure carrying the offending exact range,
    // so no span is silently dropped.
    let source = format!("\"{}\"", "x".repeat(1023));
    let path = Path::new("capacity.moth");
    let mut strings = StringTable::new();
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
        &mut builder,
    )
    .expect_err("minting a long token must report exhausted source storage");
    let FileFrontendPrepareFailure::Infrastructure(error) = failure else {
        panic!("token storage failure must not become an authored-source diagnosis");
    };
    assert_eq!(error.error_type, ErrorType::File);

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
        &mut malformed_builder,
    )
    .expect_err("capture must abort terminally when the source's span table is exhausted");
    // The unterminated literal is diagnosed before token construction; capture then hits the
    // exhausted table. No source span may be fabricated after local-span allocation fails.
    let FileFrontendPrepareFailure::Infrastructure(error) = failure else {
        panic!("capture exhaustion must surface the terminal capacity failure");
    };
    assert_eq!(error.error_type, ErrorType::File);
    assert_eq!(error.source_span, None);
    assert_eq!(error.host_path, None);
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
    assert!(joined.contains_with(inline, resolver));
    assert!(joined.contains_with(extended, resolver));
}

#[test]
fn cross_source_overlap_and_containment_are_false_when_byte_ranges_coincide() {
    let mut builder = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(0, 10, &mut builder).expect("shared byte range");
    let left = SourceSpan::new(SourceId::from_index(1), local);
    let right = SourceSpan::new(SourceId::from_index(2), local);
    let resolver = builder.resolver();

    assert!(!left.overlaps_with(right, resolver));
    assert!(!left.contains_with(right, resolver));
    assert!(!right.contains_with(left, resolver));
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
    let foreign_message = compiler_bug_panic_message(foreign_panic);
    assert!(
        foreign_message.contains(&SourceId::from_index(1).index().to_string())
            && foreign_message.contains(&SourceId::from_index(2).index().to_string())
            && foreign_message.contains("compiler bug"),
        "the wrong-source rejection should name both identities as a compiler bug: {foreign_message}"
    );

    let bare_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        first_span.resolve_with(bare_resolver);
    }))
    .expect_err("a global span must not resolve through an unqualified resolver");
    let bare_message = compiler_bug_panic_message(bare_panic);
    assert!(
        bare_message.contains("unqualified") && bare_message.contains("compiler bug"),
        "the unqualified rejection should require resolver_for(source): {bare_message}"
    );

    let own_range = first_span.resolve_with(first_builder.resolver_for(SourceId::from_index(1)));
    assert_eq!(
        (own_range.start(), own_range.end()),
        (5_000, 7_000),
        "the same-source qualified resolver must keep resolving its own table"
    );

    let empty_builder = ExtendedSpanBuilder::new();
    let second = SourceSpan::new(SourceId::from_index(2), LocalSpan::source_start());
    let empty_foreign = empty_builder.resolver_for(SourceId::from_index(3));
    assert!(
        !first_span.overlaps_with(second, empty_foreign),
        "cross-source operands still answer false without resolving"
    );
    assert!(
        !first_span.contains_with(second, empty_foreign),
        "cross-source operands still answer false without resolving"
    );
    assert_eq!(
        first_span.source_order_with(second, empty_foreign),
        Ordering::Less,
        "cross-source operands still order by identity without resolving"
    );
}

#[test]
fn source_order_sorts_by_source_then_start_then_end() {
    let mut builder = ExtendedSpanBuilder::new();
    let early_short = LocalSpan::exact(0, 5, &mut builder).expect("early short");
    let early_long = LocalSpan::exact(0, 10, &mut builder).expect("early long");
    let later = LocalSpan::exact(10, 10, &mut builder).expect("later");
    let other_source = LocalSpan::exact(0, 1, &mut builder).expect("other source");

    let mut spans = [
        SourceSpan::new(SourceId::from_index(2), other_source),
        SourceSpan::new(SourceId::from_index(1), later),
        SourceSpan::new(SourceId::from_index(1), early_long),
        SourceSpan::new(SourceId::from_index(1), early_short),
    ];
    let first = builder.resolver_for(SourceId::from_index(1));
    let second = builder.resolver_for(SourceId::from_index(2));
    spans.sort_by(|left, right| left.source_order_with(*right, first));

    let resolve = |span: SourceSpan| {
        if span.source() == SourceId::from_index(1) {
            span.resolve_with(first)
        } else {
            span.resolve_with(second)
        }
    };

    assert_eq!(
        spans.map(|span| (span.source(), resolve(span))),
        [
            (SourceId::from_index(1), early_short.resolve_with(first)),
            (SourceId::from_index(1), early_long.resolve_with(first)),
            (SourceId::from_index(1), later.resolve_with(first)),
            (SourceId::from_index(2), other_source.resolve_with(second)),
        ]
    );
}

#[test]
fn spans_in_one_source_overlap_only_where_they_share_a_byte() {
    let mut builder = ExtendedSpanBuilder::new();
    let source = SourceId::from_index(1);
    let span = |start, length, builder: &mut ExtendedSpanBuilder| {
        SourceSpan::new(
            source,
            LocalSpan::exact(start, length, builder).expect("test span"),
        )
    };

    let first = span(0, 10, &mut builder);
    let straddling = span(5, 10, &mut builder);
    let adjacent = span(10, 5, &mut builder);
    let nested = span(2, 3, &mut builder);
    let resolver = builder.resolver_for(source);

    assert!(first.overlaps_with(straddling, resolver));
    assert!(straddling.overlaps_with(first, resolver));
    assert!(first.overlaps_with(nested, resolver));
    assert!(first.overlaps_with(first, resolver));
    assert!(
        !first.overlaps_with(adjacent, resolver),
        "a range ending where the next begins shares no byte with it"
    );
}

/// An insertion point sits inside a range without sharing a byte with it.
///
/// Overlap is the non-emptiness of the intersection, so an empty span overlaps nothing at all.
/// Containment is the question consumers actually ask of an insertion point, and it answers yes.
#[test]
fn an_empty_span_is_contained_but_overlaps_nothing() {
    let mut builder = ExtendedSpanBuilder::new();
    let source = SourceId::from_index(1);
    let range = SourceSpan::new(
        source,
        LocalSpan::exact(0, 10, &mut builder).expect("range"),
    );
    let interior = SourceSpan::new(
        source,
        LocalSpan::insertion_point(5, &mut builder).expect("interior insertion point"),
    );
    let on_start = SourceSpan::new(
        source,
        LocalSpan::insertion_point(0, &mut builder).expect("insertion point on the start"),
    );
    let on_end = SourceSpan::new(
        source,
        LocalSpan::insertion_point(10, &mut builder).expect("insertion point on the end"),
    );
    let resolver = builder.resolver_for(source);

    for point in [interior, on_start, on_end] {
        assert!(!range.overlaps_with(point, resolver));
        assert!(!point.overlaps_with(range, resolver));
        assert!(range.contains_with(point, resolver));
    }

    assert!(!interior.overlaps_with(interior, resolver));
    assert!(!range.is_empty_with(resolver));
    assert!(interior.is_empty_with(resolver));
}

#[test]
fn containment_is_directional_and_reflexive_within_one_source() {
    let mut builder = ExtendedSpanBuilder::new();
    let source = SourceId::from_index(1);
    let outer = SourceSpan::new(
        source,
        LocalSpan::exact(0, 10, &mut builder).expect("outer"),
    );
    let inner = SourceSpan::new(source, LocalSpan::exact(2, 3, &mut builder).expect("inner"));
    let straddling = SourceSpan::new(
        source,
        LocalSpan::exact(5, 10, &mut builder).expect("straddling"),
    );
    let resolver = builder.resolver_for(source);

    assert!(outer.contains_with(inner, resolver));
    assert!(!inner.contains_with(outer, resolver));
    assert!(outer.contains_with(outer, resolver));
    assert!(!outer.contains_with(straddling, resolver));
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
