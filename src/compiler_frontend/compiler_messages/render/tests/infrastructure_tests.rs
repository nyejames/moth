//! Final-output contracts for cause classification, aggregation and safe presentation.

use crate::backends::error_types::BackendErrorType;
use crate::compiler_frontend::compiler_errors::{
    BoundaryFailure, CompilerError, CompilerErrorMetadataKey, ErrorType,
};
use crate::compiler_frontend::compiler_messages::diagnostic_bag::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::display_messages::format_infrastructure_errors;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::render::dev_server::render_boundary_messages_html;
use crate::compiler_frontend::compiler_messages::render::infrastructure::{
    INFRASTRUCTURE_FAILURE_CODE, INFRASTRUCTURE_FAILURE_TITLE,
};
use crate::compiler_frontend::compiler_messages::render::terse::format_terse_boundary_messages;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity,
    InfrastructureDiagnosticKind, InvalidConfigReason, InvalidImportPathReason, NameNamespace,
    RuleDiagnosticKind, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const BUG_INTRODUCTION: &str =
    "COMPILER BUG: Moth encountered an internal compiler or build-system error.";
const BUG_GUIDANCE: &str =
    "This is a Moth toolchain defect, not a source-code error. Please report it at:";
const OPERATIONAL_INTRODUCTION: &str =
    "OPERATIONAL FAILURE: Moth could not complete a filesystem, host or tooling operation.";
const ISSUE_URL: &str = "https://github.com/nyejames/moth/issues";

fn failure_with_causes(causes: impl IntoIterator<Item = CompilerError>) -> BoundaryFailure {
    let mut accumulator = BoundaryAccumulator::new();
    for cause in causes {
        accumulator.append_infrastructure(cause.into(), None);
    }
    accumulator.finish(InstallMode::Frozen)
}

fn failure_with_prepared_batch(
    batch: PreparedDiagnosticBatch,
    infrastructure_error: Option<CompilerError>,
) -> BoundaryFailure {
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(batch, None);
    if let Some(error) = infrastructure_error {
        accumulator.append_infrastructure(error.into(), None);
    }
    accumulator.finish(InstallMode::Frozen)
}

fn render_failure(failure: BoundaryFailure) -> (BoundaryFailure, [String; 3]) {
    let rendered = [
        format_infrastructure_errors(failure.infrastructure_errors().expect("outer failure")),
        format_terse_boundary_messages(&failure).join("\n"),
        render_boundary_messages_html(&failure, Path::new(".")),
    ];
    (failure, rendered)
}

fn render_views(error: CompilerError) -> (BoundaryFailure, [String; 3]) {
    render_failure(failure_with_causes([error]))
}

/// Let a boundary regression carry its actual failure through the production render paths.
pub(crate) fn assert_compiler_bug_views(error: CompilerError, invariant_detail: &str) {
    let original = error.msg.clone();
    let (failure, [terminal, terse, html]) = render_views(error);
    for view in [&terminal, &terse, &html] {
        assert!(view.contains(BUG_INTRODUCTION), "{view}");
        assert!(view.contains(BUG_GUIDANCE), "{view}");
        assert!(view.contains(ISSUE_URL), "{view}");
        assert!(!view.contains(OPERATIONAL_INTRODUCTION), "{view}");
    }
    assert!(terminal.contains(&original), "{terminal}");
    assert!(terse.contains(invariant_detail), "{terse}");
    assert!(html.contains(invariant_detail), "{html}");
    assert_eq!(
        failure
            .infrastructure_errors()
            .expect("boundary cause")
            .primary
            .msg,
        original
    );
}

#[test]
fn every_cause_type_has_truthful_introduction_in_all_views() {
    // One exhaustive local table protects all consumers from defaulting a new subsystem to blame.
    for (error_type, compiler_bug) in [
        (ErrorType::Compiler, true),
        (ErrorType::HirTransformation, true),
        (
            ErrorType::Backend(BackendErrorType::LirTransformation),
            true,
        ),
        (ErrorType::Backend(BackendErrorType::WasmGeneration), true),
        (ErrorType::File, false),
        (ErrorType::DevServer, false),
    ] {
        let mut error = CompilerError::new("original cause detail", None, error_type);
        error.new_metadata_entry(
            CompilerErrorMetadataKey::CompilationStage,
            "selected stage".into(),
        );
        error.new_metadata_entry(
            CompilerErrorMetadataKey::PrimarySuggestion,
            "specific next action".into(),
        );
        let (failure, [terminal, terse, html]) = render_views(error);
        let expected = if compiler_bug {
            BUG_INTRODUCTION
        } else {
            OPERATIONAL_INTRODUCTION
        };
        let unwanted = if compiler_bug {
            OPERATIONAL_INTRODUCTION
        } else {
            BUG_INTRODUCTION
        };
        for view in [&terminal, &terse, &html] {
            assert!(view.contains(expected), "{view}");
            assert!(!view.contains(unwanted), "{view}");
            assert!(
                view.find(expected) < view.find("original cause detail"),
                "{view}"
            );
            assert!(view.contains("Details:"), "{view}");
            assert!(view.contains("Stage: selected stage"), "{view}");
            assert!(view.contains("specific next action"), "{view}");
            assert_eq!(view.contains(ISSUE_URL), compiler_bug, "{view}");
            if compiler_bug {
                assert!(view.contains(BUG_GUIDANCE), "{view}");
            }
        }
        if compiler_bug {
            assert!(html.contains(&format!("href=\"{ISSUE_URL}\"")), "{html}");
        }
        if failure
            .infrastructure_errors()
            .expect("boundary cause")
            .primary
            .error_type
            == ErrorType::File
        {
            assert!(
                !terminal.contains("Missing File or Directory"),
                "{terminal}"
            );
        }
        assert_eq!(terse.split('|').count(), 5, "{terse}");
        assert_eq!(
            html.matches("data-diagnostic-code=\"MOTH-INFRA-0001\"")
                .count(),
            1
        );
        assert!(!html.contains("source-frame"));
        assert_eq!(
            failure
                .infrastructure_errors()
                .expect("boundary cause")
                .primary
                .msg,
            "original cause detail"
        );
    }
}

#[test]
fn mixed_and_same_class_causes_render_in_production_order_after_append() {
    for (first, second) in [
        (ErrorType::Compiler, ErrorType::File),
        (ErrorType::File, ErrorType::Compiler),
        (ErrorType::Compiler, ErrorType::Compiler),
        (ErrorType::File, ErrorType::File),
    ] {
        let compiler_bug_count = usize::from(first == ErrorType::Compiler)
            + usize::from(second == ErrorType::Compiler)
            + 1;
        let mut primary = CompilerError::new("first cause detail", None, first);
        primary.host_path = Some(PathBuf::from("primary.moth"));
        primary.new_metadata_entry(
            CompilerErrorMetadataKey::CompilationStage,
            "first stage".into(),
        );
        let mut additional = CompilerError::new("second cause detail", None, second);
        additional.host_path = Some(PathBuf::from("additional.moth"));
        additional.new_metadata_entry(
            CompilerErrorMetadataKey::CompilationStage,
            "second stage".into(),
        );
        let third = CompilerError::compiler_error("third cause detail");
        let (failure, [terminal, terse, html]) =
            render_failure(failure_with_causes([primary, additional, third]));
        for view in [&terminal, &terse, &html] {
            let first = view.find("first cause detail").expect("first detail");
            let second = view.find("second cause detail").expect("second detail");
            let third = view.find("third cause detail").expect("third detail");
            assert!(first < second && second < third, "{view}");
            assert!(view.contains("Additional cause 1:"), "{view}");
            assert!(view.contains("Additional cause 2:"), "{view}");
            assert!(view.contains("Stage: first stage"), "{view}");
            assert!(view.contains("Stage: second stage"), "{view}");
            assert!(view.contains("additional.moth"), "{view}");
            assert_eq!(view.matches("first cause detail").count(), 1);
            assert_eq!(view.matches("second cause detail").count(), 1);
            assert_eq!(view.matches("third cause detail").count(), 1);
            assert!(view.contains("primary.moth"), "{view}");
            assert_eq!(
                view.matches(BUG_INTRODUCTION).count(),
                compiler_bug_count,
                "{view}"
            );
            assert_eq!(
                view.matches(OPERATIONAL_INTRODUCTION).count(),
                3 - compiler_bug_count,
                "{view}"
            );
        }
        assert_eq!(terse.split('|').collect::<Vec<_>>()[2], "primary.moth");
        assert_eq!(terse.matches(ISSUE_URL).count(), 1, "{terse}");
        assert_eq!(html.matches("<article ").count(), 1, "{html}");
        assert_eq!(failure.error_count(), 1);
        assert_eq!(
            failure
                .infrastructure_errors()
                .expect("causes")
                .iter()
                .map(|error| error.msg.as_str())
                .collect::<Vec<_>>(),
            [
                "first cause detail",
                "second cause detail",
                "third cause detail"
            ]
        );
    }
}

#[test]
fn hostile_causes_are_escaped_without_losing_raw_details_or_terse_fields() {
    let detail = "<script>\"quoted\" & 'single' | first\nsecond";
    let path = "<output>&\"'|\npage.html";
    let guidance = "<repair> & \"quoted\" 'single' | next\nline";
    let mut span_builder = ExtendedSpanBuilder::new();
    let unpaired_span = SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(0, 1, &mut span_builder).expect("test span"),
    );
    let mut primary = CompilerError::new(detail, Some(unpaired_span), ErrorType::File);
    primary.host_path = Some(PathBuf::from(path));
    for key in [
        CompilerErrorMetadataKey::CompilationStage,
        CompilerErrorMetadataKey::PrimarySuggestion,
    ] {
        primary.new_metadata_entry(key, guidance.into());
    }
    let mut additional = CompilerError::new(detail, Some(unpaired_span), ErrorType::Compiler);
    additional.host_path = Some(PathBuf::from(path));
    additional.new_metadata_entry(
        CompilerErrorMetadataKey::SuggestedReplacement,
        guidance.into(),
    );
    let (failure, [terminal, terse, html]) =
        render_failure(failure_with_causes([primary, additional]));
    assert!(terminal.contains(detail));
    assert!(terminal.contains(guidance));
    for escaped in [
        "&lt;script&gt;&quot;quoted&quot; &amp; &#39;single&#39;",
        "&lt;output&gt;&amp;&quot;&#39;",
        "&lt;repair&gt; &amp; &quot;quoted&quot; &#39;single&#39;",
    ] {
        assert!(html.contains(escaped), "{html}");
    }
    assert!(!html.contains("<script>"));
    assert!(!html.contains("<repair>"));
    assert!(!html.contains("source-frame"));
    assert_eq!(html.matches("<article ").count(), 1);
    assert!(!terse.contains(['\n', '\r']));
    let fields = terse.split('|').collect::<Vec<_>>();
    assert_eq!(fields.len(), 5, "{terse}");
    assert_eq!(fields[3], "");
    assert!(fields[4].contains("/ first second"), "{terse}");
    assert!(
        fields[4].contains("Path: <output>&\"'/ page.html"),
        "{terse}"
    );
    assert_eq!(terse.matches(ISSUE_URL).count(), 1);
    for error in failure.infrastructure_errors().expect("causes").iter() {
        assert_eq!(error.msg, detail);
        assert_eq!(error.source_span, Some(unpaired_span));
        assert_eq!(error.host_path.as_deref(), Some(Path::new(path)));
    }
}

#[test]
fn permission_denial_is_operational_without_platform_permission_assumptions() {
    let denied = io::Error::new(
        io::ErrorKind::PermissionDenied,
        "permission denied reading provider",
    );
    let error = CompilerError::file_error(Path::new("provider.js"), denied.to_string());
    let (_, [terminal, terse, html]) = render_views(error);
    for view in [terminal, terse, html] {
        assert!(view.contains(OPERATIONAL_INTRODUCTION), "{view}");
        assert!(
            view.contains("permission denied reading provider"),
            "{view}"
        );
        assert!(!view.contains(BUG_INTRODUCTION), "{view}");
        assert!(!view.contains(ISSUE_URL), "{view}");
    }
}

#[test]
fn normal_import_config_and_type_diagnostics_keep_codes_without_infrastructure_blame() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let import = paths
        .try_intern_portable_path("invalid/import", &mut strings)
        .expect("path");
    let key = strings.intern("unknown_config_key");
    let warning_name = strings.intern("warning_name");
    let diagnostics = vec![
        CompilerDiagnostic::with_severity(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownValueName),
            DiagnosticSeverity::Warning,
            None,
            DiagnosticPayload::UnknownName {
                name: warning_name,
                namespace: NameNamespace::Value,
            },
        ),
        CompilerDiagnostic::invalid_import_path(
            import,
            InvalidImportPathReason::ParentDirectorySegment,
            None,
        ),
        CompilerDiagnostic::invalid_config_reason(
            Some(key),
            InvalidConfigReason::UnknownKey { key },
            None,
        ),
        CompilerDiagnostic::type_mismatch(
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            builtin_type_ids::STRING,
            TypeMismatchContext::FunctionArgument,
            None,
        ),
    ];
    let expected_codes: Vec<_> = diagnostics[1..]
        .iter()
        .map(|diagnostic| diagnostic.kind.descriptor().code)
        .collect();
    let path_table = Arc::new(paths.snapshot_table());

    let make_batch = || {
        PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from_diagnostics(diagnostics.clone()),
            strings.clone(),
            None,
            true,
            Some(Arc::clone(&path_table)),
        )
    };
    let failure = failure_with_prepared_batch(make_batch(), None);
    let terse = format_terse_boundary_messages(&failure);
    let html = render_boundary_messages_html(&failure, Path::new("."));
    assert_eq!(terse.len(), 4);
    for (line, expected) in terse.iter().zip(expected_codes.iter()) {
        assert_eq!(line.split('|').nth(1), Some(*expected), "{line}");
    }
    assert!(terse.last().expect("warning").starts_with("W|"));
    for view in [terse.join("\n"), html] {
        assert!(!view.contains(BUG_INTRODUCTION), "{view}");
        assert!(!view.contains(OPERATIONAL_INTRODUCTION), "{view}");
        assert!(!view.contains("MOTH-INFRA-0001"), "{view}");
    }

    let failure = failure_with_prepared_batch(
        make_batch(),
        Some(CompilerError::file_error(
            Path::new("output.html"),
            "late output write failed",
        )),
    );
    let with_infrastructure = format_terse_boundary_messages(&failure);
    assert_eq!(&with_infrastructure[..3], &terse[..3]);
    assert!(with_infrastructure[3].starts_with("E|MOTH-INFRA-0001|"));
    assert_eq!(with_infrastructure[4], terse[3]);
    let html = render_boundary_messages_html(&failure, Path::new("."));
    assert!(
        html.find("late output write failed")
            .expect("infrastructure detail")
            < html.find("warning_name").expect("warning"),
        "{html}"
    );
}

#[test]
fn infrastructure_constants_match_the_failure_descriptor() {
    let descriptor =
        DiagnosticKind::Infrastructure(InfrastructureDiagnosticKind::InfrastructureFailure)
            .descriptor();
    assert_eq!(INFRASTRUCTURE_FAILURE_CODE, descriptor.code);
    assert_eq!(INFRASTRUCTURE_FAILURE_TITLE, descriptor.title);
}
