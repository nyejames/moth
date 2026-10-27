//! Final-output contracts for cause classification, aggregation and safe presentation.

use crate::compiler_frontend::compiler_errors::{
    CompilationOutcome, CompilationResult, InfrastructureDetail, InfrastructureErrors,
    InfrastructureFailure, InfrastructureFailureKind,
};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostic_bag::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    imports::invalid_import_path, names_declarations::unknown_value_name,
    project_config::invalid_config_reason, types_borrows_targets::type_mismatch,
};
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::display_messages::format_infrastructure_errors;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::compiler_messages::render::dev_server::render_compilation_result_html;
use crate::compiler_frontend::compiler_messages::render::terse::format_terse_compilation_result;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticSeverity, InvalidConfigReason, InvalidImportPathReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use std::io;
use std::path::Path;
use std::sync::Arc;

const BUG_INTRODUCTION: &str =
    "COMPILER BUG: Moth encountered an internal compiler or build-system error.";
const BUG_GUIDANCE: &str =
    "This is a Moth toolchain defect, not a source-code error. Please report it at:";
const OPERATIONAL_INTRODUCTION: &str =
    "OPERATIONAL FAILURE: Moth could not complete a filesystem, host or tooling operation.";
const ISSUE_URL: &str = "https://github.com/nyejames/moth/issues";

fn failure_with_causes(
    causes: impl IntoIterator<Item = InfrastructureFailure>,
) -> CompilationResult<()> {
    let mut accumulator = BoundaryAccumulator::new();
    for cause in causes {
        accumulator.append_infrastructure(cause.into());
    }
    accumulator
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed)
}

fn failure_with_prepared_batch(
    batch: PreparedDiagnosticBatch,
    infrastructure_error: Option<InfrastructureFailure>,
) -> CompilationResult<()> {
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(batch, None);
    if let Some(error) = infrastructure_error {
        accumulator.append_infrastructure(error.into());
    }
    accumulator
        .finish(InstallMode::Frozen)
        .map(CompilationOutcome::Diagnosed)
}

fn render_failure(failure: CompilationResult<()>) -> (CompilationResult<()>, [String; 3]) {
    let errors = failure
        .as_ref()
        .expect_err("rendering an infrastructure failure requires infrastructure errors");
    let rendered = [
        format_infrastructure_errors(errors),
        format_terse_compilation_result(&failure).join("\n"),
        render_compilation_result_html(&failure, Path::new(".")),
    ];

    (failure, rendered)
}
fn infrastructure_errors(result: &CompilationResult<()>) -> &InfrastructureErrors {
    let Err(errors) = result else {
        panic!("expected outer infrastructure errors");
    };
    errors
}

/// Build one cause of `kind` through its production constructor.
fn cause_of_kind(kind: InfrastructureFailureKind, message: &str) -> InfrastructureFailure {
    match kind {
        InfrastructureFailureKind::Internal => {
            InfrastructureFailure::internal(CompilerStage::Output, message)
        }
        InfrastructureFailureKind::HirTransformation => {
            InfrastructureFailure::hir_transformation(message)
        }
        InfrastructureFailureKind::LirTransformation => {
            InfrastructureFailure::lir_transformation(message)
        }
        InfrastructureFailureKind::WasmGeneration => {
            InfrastructureFailure::wasm_generation(message)
        }
        InfrastructureFailureKind::Filesystem => InfrastructureFailure::filesystem(
            CompilerStage::Output,
            Path::new("cause.moth"),
            message,
        ),
        InfrastructureFailureKind::DevelopmentServer => {
            InfrastructureFailure::development_server(Path::new("cause.moth"), message)
        }
    }
}

fn render_views(error: InfrastructureFailure) -> (CompilationResult<()>, [String; 3]) {
    render_failure(failure_with_causes([error]))
}

/// Let a boundary regression carry its actual failure through the production render paths.
pub(crate) fn assert_compiler_bug_views(error: InfrastructureFailure, invariant_detail: &str) {
    let original = error.message.clone();
    let (failure, [terminal, terse, html]) = render_views(error);
    for view in [&terminal, &terse, &html] {
        assert!(view.contains(BUG_INTRODUCTION), "{view}");
        assert!(view.contains(BUG_GUIDANCE), "{view}");
        assert!(view.contains(ISSUE_URL), "{view}");
        assert!(!view.contains(OPERATIONAL_INTRODUCTION), "{view}");
    }
    assert!(terminal.contains(&*original), "{terminal}");
    assert!(terse.contains(invariant_detail), "{terse}");
    assert!(html.contains(invariant_detail), "{html}");
    let Err(errors) = failure else {
        panic!("compiler bug view fixture should produce infrastructure errors");
    };
    assert_eq!(errors.primary.message, original);
}

#[test]
fn every_cause_type_has_truthful_introduction_in_all_views() {
    // One exhaustive local table protects all consumers from defaulting a new subsystem to blame.
    for (kind, compiler_bug) in [
        (InfrastructureFailureKind::Internal, true),
        (InfrastructureFailureKind::HirTransformation, true),
        (InfrastructureFailureKind::LirTransformation, true),
        (InfrastructureFailureKind::WasmGeneration, true),
        (InfrastructureFailureKind::Filesystem, false),
        (InfrastructureFailureKind::DevelopmentServer, false),
    ] {
        let error = cause_of_kind(kind, "original cause detail").with_detail(
            InfrastructureDetail::PrimarySuggestion("specific next action".into()),
        );
        assert_eq!(error.kind, kind);
        let stage_line = format!("Stage: {}", error.stage);
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
            assert!(view.contains(&stage_line), "{view}");
            assert!(view.contains("specific next action"), "{view}");
            assert_eq!(view.contains(ISSUE_URL), compiler_bug, "{view}");
            if compiler_bug {
                assert!(view.contains(BUG_GUIDANCE), "{view}");
            }
        }
        if compiler_bug {
            assert!(html.contains(&format!("href=\"{ISSUE_URL}\"")), "{html}");
        }
        if infrastructure_errors(&failure).primary.kind == InfrastructureFailureKind::Filesystem {
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
            &*infrastructure_errors(&failure).primary.message,
            "original cause detail"
        );
    }
}

#[test]
fn mixed_and_same_class_causes_render_in_production_order_after_append() {
    for (first, second) in [
        (
            InfrastructureFailureKind::Internal,
            InfrastructureFailureKind::Filesystem,
        ),
        (
            InfrastructureFailureKind::Filesystem,
            InfrastructureFailureKind::Internal,
        ),
        (
            InfrastructureFailureKind::Internal,
            InfrastructureFailureKind::Internal,
        ),
        (
            InfrastructureFailureKind::Filesystem,
            InfrastructureFailureKind::Filesystem,
        ),
    ] {
        let compiler_bug_count = usize::from(first == InfrastructureFailureKind::Internal)
            + usize::from(second == InfrastructureFailureKind::Internal)
            + 1;
        let mut primary = cause_of_kind(first, "first cause detail");
        primary.stage = CompilerStage::HeaderParsing;
        primary.host_path = Some(Path::new("primary.moth").into());
        let mut additional = cause_of_kind(second, "second cause detail");
        additional.stage = CompilerStage::BackendLowering;
        additional.host_path = Some(Path::new("additional.moth").into());
        let third = InfrastructureFailure::internal(CompilerStage::Output, "third cause detail");
        let (failure, [terminal, terse, html]) =
            render_failure(failure_with_causes([primary, additional, third]));
        for view in [&terminal, &terse, &html] {
            let first = view.find("first cause detail").expect("first detail");
            let second = view.find("second cause detail").expect("second detail");
            let third = view.find("third cause detail").expect("third detail");
            assert!(first < second && second < third, "{view}");
            assert!(view.contains("Additional cause 1:"), "{view}");
            assert!(view.contains("Additional cause 2:"), "{view}");
            assert!(view.contains("Stage: Header parsing"), "{view}");
            assert!(view.contains("Stage: Backend lowering"), "{view}");
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
        assert_eq!(infrastructure_errors(&failure).iter().count(), 3);
        assert_eq!(
            infrastructure_errors(&failure)
                .iter()
                .map(|error| &*error.message)
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
    let primary = InfrastructureFailure::filesystem(CompilerStage::Output, Path::new(path), detail)
        .with_detail(InfrastructureDetail::PrimarySuggestion(guidance.into()));
    let mut additional = InfrastructureFailure::internal(CompilerStage::Output, detail);
    additional.host_path = Some(Path::new(path).into());
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
    for error in infrastructure_errors(&failure).iter() {
        assert_eq!(&*error.message, detail);
        assert_eq!(error.host_path.as_deref(), Some(Path::new(path)));
    }
}

#[test]
fn permission_denial_is_operational_without_platform_permission_assumptions() {
    let denied = io::Error::new(
        io::ErrorKind::PermissionDenied,
        "permission denied reading provider",
    );
    let error = InfrastructureFailure::filesystem(
        CompilerStage::SourceLoading,
        Path::new("provider.js"),
        denied.to_string(),
    );
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
fn normal_diagnostics_and_success_warnings_use_separate_result_lanes() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let import = paths
        .try_intern_portable_path("invalid/import", &mut strings)
        .expect("path");
    let key = strings.intern("unknown_config_key");
    let warning_name = strings.intern("warning_name");
    let path_table = Arc::new(paths.snapshot_table());
    let expected_codes = ["MOTH-IMPORT-0016", "MOTH-CONFIG-0001", "MOTH-TYPE-0001"];

    let make_batch = |include_warning| {
        let mut diagnostics = vec![
            invalid_import_path(
                import,
                InvalidImportPathReason::ParentDirectorySegment,
                None,
            ),
            invalid_config_reason(Some(key), InvalidConfigReason::UnknownKey { key }, None),
            type_mismatch(
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                builtin_type_ids::STRING,
                TypeMismatchContext::FunctionArgument,
                None,
            ),
        ];
        if include_warning {
            diagnostics.insert(
                0,
                unknown_value_name(warning_name, None).with_severity(DiagnosticSeverity::Warning),
            );
        }
        PreparedDiagnosticBatch::capture_from_bag(
            DiagnosticBag::from_diagnostics(diagnostics),
            strings.clone(),
            None,
            Some(Arc::clone(&path_table)),
        )
    };

    let diagnosed = failure_with_prepared_batch(make_batch(false), None);
    let terse = format_terse_compilation_result(&diagnosed);
    let html = render_compilation_result_html(&diagnosed, Path::new("."));
    assert_eq!(terse.len(), expected_codes.len());
    for (line, expected) in terse.iter().zip(expected_codes.iter()) {
        assert_eq!(line.split('|').nth(1), Some(*expected), "{line}");
    }
    assert!(terse.iter().all(|line| !line.starts_with("W|")));
    for view in [terse.join("\n"), html] {
        assert!(!view.contains(BUG_INTRODUCTION), "{view}");
        assert!(!view.contains(OPERATIONAL_INTRODUCTION), "{view}");
        assert!(!view.contains("MOTH-INFRA-0001"), "{view}");
    }

    let warning_batch = PreparedDiagnosticBatch::capture_from_bag(
        DiagnosticBag::from(
            unknown_value_name(warning_name, None).with_severity(DiagnosticSeverity::Warning),
        ),
        strings.clone(),
        None,
        Some(Arc::clone(&path_table)),
    );
    let warning_result = failure_with_prepared_batch(warning_batch, None);
    let Ok(CompilationOutcome::Diagnosed(warnings)) = warning_result else {
        panic!("warning fixture should produce a report set");
    };
    let successful: CompilationResult<()> = Ok(CompilationOutcome::Success {
        value: (),
        warnings: Some(warnings),
    });
    let warning_lines = format_terse_compilation_result(&successful);
    assert_eq!(warning_lines.len(), 1);
    assert!(warning_lines[0].starts_with("W|"));
    let warning_html = render_compilation_result_html(&successful, Path::new("."));
    assert!(warning_html.contains("warning_name"));

    let failure = failure_with_prepared_batch(
        make_batch(true),
        Some(InfrastructureFailure::filesystem(
            CompilerStage::SourceLoading,
            Path::new("output.html"),
            "late output write failed",
        )),
    );
    let with_infrastructure = format_terse_compilation_result(&failure);
    assert_eq!(with_infrastructure.len(), 1);
    assert!(with_infrastructure[0].starts_with("E|MOTH-INFRA-0001|"));
    assert!(with_infrastructure[0].contains("late output write failed"));
    assert!(!with_infrastructure[0].contains("warning_name"));
    for code in expected_codes {
        assert!(!with_infrastructure[0].contains(code));
    }
    let html = render_compilation_result_html(&failure, Path::new("."));
    assert!(html.contains("late output write failed"));
    assert!(!html.contains("warning_name"));
    for code in expected_codes {
        assert!(!html.contains(code));
    }
}
