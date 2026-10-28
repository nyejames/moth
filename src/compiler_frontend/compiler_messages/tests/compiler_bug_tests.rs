use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::compiler_bug::{
    CompilerBugReport, CompilerBugSource, format_compiler_bug_report,
};
use crate::compiler_frontend::source::{
    ExtendedSpanBuilder, FrozenIdentityContext, LocalSpan, SourceDatabase, SourceSpan,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
use std::panic::Location;

#[cfg(feature = "test_compiler_bug")]
#[test]
fn compiler_bug_test_trigger_produces_a_structured_report() {
    let payload = std::panic::catch_unwind(|| {
        crate::trigger_compiler_bug_for_test();
    })
    .expect_err("the test-only trigger must raise a compiler bug");

    assert!(
        crate::render_compiler_bug_payload(payload.as_ref()).is_some(),
        "the host boundary must recognize the structured trigger payload"
    );
}
#[test]
fn compiler_bug_payload_owns_stage_call_site_and_message() {
    let expected_line = line!() + 2;
    let panic = std::panic::catch_unwind(|| {
        crate::compiler_bug!(CompilerStage::AstConstruction, "broken invariant {}", 42);
    });
    let payload = match panic {
        Ok(()) => panic!("compiler_bug! must unwind"),
        Err(payload) => payload,
    };
    let report = match payload.downcast::<CompilerBugReport>() {
        Ok(report) => report,
        Err(_) => panic!("compiler_bug! must carry CompilerBugReport"),
    };

    assert_eq!(report.stage, CompilerStage::AstConstruction);
    assert_eq!(report.location.file(), file!());
    assert_eq!(report.location.line(), expected_line);
    assert_eq!(&*report.message, "broken invariant 42");
}

#[test]
fn compiler_bug_report_formats_exactly_without_source() {
    let location = Location::caller();
    let report = CompilerBugReport {
        stage: CompilerStage::BackendLowering,
        location,
        version: env!("CARGO_PKG_VERSION"),
        message: "invalid lowered state".into(),
        source: None,
    };
    let expected = format!(
        "COMPILER BUG: Moth encountered an internal compiler or build-system error.\n\
         This is a Moth toolchain defect, not a source-code error. Please report it at:\n\
         https://github.com/nyejames/moth/issues\n\n\
         Stage: Backend lowering\n\
         Rust location: {}:{}:{}\n\
         Compiler version: {}\n\n\
         Details:\n\
         invalid lowered state",
        location.file(),
        location.line(),
        location.column(),
        env!("CARGO_PKG_VERSION"),
    );

    assert_eq!(format_compiler_bug_report(&report), expected);
}

#[test]
fn compiler_bug_report_formats_resolved_source_and_bounded_excerpt() {
    let temporary_directory = tempfile::tempdir().expect("temporary directory should exist");
    let source_path = temporary_directory.path().join("main.moth");
    let authored_suffix = "x".repeat(180);
    fs::write(&source_path, format!("first\nβeta {authored_suffix}\n"))
        .expect("source snapshot should be written");

    let mut string_table = StringTable::new();
    let mut source_database = SourceDatabase::build(
        std::iter::once(source_path.as_path()),
        &source_path,
        None,
        &mut string_table,
    )
    .expect("source identity should build");
    let source_id = source_database
        .get_by_canonical_path(&source_path)
        .expect("source should be registered")
        .id;
    source_database
        .retain_text(source_id, format!("first\nβeta {authored_suffix}\n"))
        .expect("source snapshot should be retained");

    let mut extended_spans = ExtendedSpanBuilder::new();
    let local_span = LocalSpan::exact(6, 5, &mut extended_spans).expect("span should fit");
    let frozen = FrozenIdentityContext::from_parts(string_table, source_database);
    let source = CompilerBugSource::from_span(SourceSpan::new(source_id, local_span), &frozen)
        .expect("the retained span should resolve");
    assert_eq!(&*source.logical_display_path, "main.moth");
    assert_eq!(source.byte_range, 6..11);
    assert_eq!((source.start_line, source.start_column), (2, 1));
    assert_eq!((source.end_line, source.end_column), (2, 5));
    assert_eq!(source.excerpt.chars().count(), 160);

    let location = Location::caller();
    let report = CompilerBugReport {
        stage: CompilerStage::ConstantEvaluation,
        location,
        version: env!("CARGO_PKG_VERSION"),
        message: "folded value was missing".into(),
        source: Some(source),
    };
    let expected_excerpt = format!("βeta {}", "x".repeat(155));
    let expected = format!(
        concat!(
            "COMPILER BUG: Moth encountered an internal compiler or build-system error.\n",
            "This is a Moth toolchain defect, not a source-code error. Please report it at:\n",
            "https://github.com/nyejames/moth/issues\n\n",
            "Stage: Constant evaluation\n",
            "Rust location: {}:{}:{}\n",
            "Compiler version: {}\n\n",
            "Details:\n",
            "folded value was missing\n",
            "  --> main.moth:2:1\n",
            "     | {}"
        ),
        location.file(),
        location.line(),
        location.column(),
        env!("CARGO_PKG_VERSION"),
        expected_excerpt,
    );

    assert_eq!(format_compiler_bug_report(&report), expected);
}
