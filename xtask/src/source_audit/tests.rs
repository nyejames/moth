//! Self-tests for the broad-source architecture audit.
//!
//! Each rule is proved against fixture text rather than against whatever the tree happens to
//! contain, so a rule keeps its meaning when the tree changes.

use super::{
    AUDIT_IMPLEMENTATION_FILES, AUDITED_SOURCE_ROOTS, SourceRule, audit_source_fragment,
    audit_sources, started_report,
};
use crate::report_file::ReportRunIdentity;
use std::fs;
use std::path::Path;

/// The banned name, assembled so this file does not contain it either.
fn removed_conversion_name() -> String {
    ["to", "_", "legacy", "_", "error"].concat()
}

#[test]
fn reports_the_removed_legacy_error_conversion_by_name() {
    let source = format!(
        "fn {}(value: u8) -> u8 {{ value }}\n",
        removed_conversion_name()
    );

    let findings = audit_source_fragment("src/example.rs", &source);

    assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
    assert_eq!(findings[0].rule, SourceRule::RemovedLegacyConversionName);
    assert_eq!(findings[0].file, "src/example.rs");
    assert!(
        findings[0].message.contains(&removed_conversion_name()),
        "the finding should name what it found: {}",
        findings[0].message
    );
}

#[test]
fn accepts_a_file_that_does_not_name_the_removed_conversion() {
    let findings =
        audit_source_fragment("src/example.rs", "fn render(value: u8) -> u8 { value }\n");

    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn carries_a_timer_rule_hit_through_as_a_typed_finding() {
    let findings = audit_source_fragment(
        "src/build_system/example.rs",
        "struct Context {\n  timing_context: Option<TimingContext>,\n}",
    );

    assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
    assert_eq!(findings[0].rule, SourceRule::TimerErasure);
    assert_eq!(findings[0].file, "src/build_system/example.rs");
    assert!(
        findings[0].message.starts_with("timer-only field"),
        "unexpected message: {}",
        findings[0].message
    );
}

#[test]
fn the_timing_facade_is_exempt_from_the_rules_that_keep_callers_off_it() {
    // The facade is what every other file must go through, so calling the enabled implementation
    // is exactly its job. Deriving this from the path keeps the exemption in one place.
    let findings = audit_source_fragment("src/timing/enabled/mod.rs", "timing::enabled::record();");

    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn a_caller_outside_the_facade_is_still_reported() {
    let findings = audit_source_fragment("src/projects/example.rs", "timing::enabled::record();");

    assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
    assert_eq!(findings[0].rule, SourceRule::TimerErasure);
}

#[test]
fn one_file_can_report_more_than_one_rule() {
    let source = format!(
        "struct Context {{\n  timing_context: Option<TimingContext>,\n}}\nfn {}() {{}}\n",
        removed_conversion_name()
    );

    let findings = audit_source_fragment("src/example.rs", &source);

    let mut rules: Vec<SourceRule> = findings.iter().map(|finding| finding.rule).collect();
    rules.dedup();
    assert_eq!(
        rules,
        vec![
            SourceRule::TimerErasure,
            SourceRule::RemovedLegacyConversionName
        ]
    );
}

#[test]
fn the_audit_reads_the_whole_workspace_and_currently_reports_nothing() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask manifest has a parent");

    let (audited_file_count, findings) =
        audit_sources(workspace_root).expect("the workspace should be readable");

    assert!(
        audited_file_count > 100,
        "the audit should read the whole tree, not a corner of it: {audited_file_count}"
    );
    assert!(findings.is_empty(), "source audit findings: {findings:?}");
}

#[test]
fn the_audit_skips_only_its_own_implementation_files() {
    // These contain the fragments the rules search for. Any other exemption would be a rule that
    // silently stops applying somewhere.
    assert_eq!(
        AUDIT_IMPLEMENTATION_FILES,
        [
            "xtask/src/timers_erasure_check.rs",
            "xtask/src/source_audit.rs",
            "xtask/src/source_audit/tests.rs"
        ]
    );
}

#[test]
fn reports_violations_in_extracted_crate_sources_and_tests() {
    let workspace = tempfile::tempdir().expect("fixture workspace");
    for root in AUDITED_SOURCE_ROOTS {
        fs::create_dir_all(workspace.path().join(root)).expect("configured source root");
    }

    for relative in [
        "crates/moth-lexical/src/tests/violation.rs",
        "crates/moth-mon/src/tests/violation.rs",
        "crates/moth-mon/tests/violation.rs",
        "crates/moth-host/src/tests/violation.rs",
        "crates/moth-cli/src/tests/violation.rs",
        "crates/moth-cli/tests/violation.rs",
    ] {
        let path = workspace.path().join(relative);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture directory");
        fs::write(&path, format!("fn {}() {{}}\n", removed_conversion_name()))
            .expect("violation fixture");
    }

    let (_, findings) = audit_sources(workspace.path()).expect("fixture scan");
    let observed: Vec<_> = findings
        .iter()
        .map(|finding| (finding.file.as_str(), finding.rule))
        .collect();
    assert_eq!(
        observed,
        [
            (
                "crates/moth-lexical/src/tests/violation.rs",
                SourceRule::RemovedLegacyConversionName,
            ),
            (
                "crates/moth-mon/src/tests/violation.rs",
                SourceRule::RemovedLegacyConversionName,
            ),
            (
                "crates/moth-mon/tests/violation.rs",
                SourceRule::RemovedLegacyConversionName,
            ),
            (
                "crates/moth-host/src/tests/violation.rs",
                SourceRule::RemovedLegacyConversionName,
            ),
            (
                "crates/moth-cli/src/tests/violation.rs",
                SourceRule::RemovedLegacyConversionName,
            ),
            (
                "crates/moth-cli/tests/violation.rs",
                SourceRule::RemovedLegacyConversionName,
            ),
        ]
    );
}

#[test]
fn a_missing_extracted_test_root_fails_the_scan() {
    let workspace = tempfile::tempdir().expect("fixture workspace");
    for root in AUDITED_SOURCE_ROOTS {
        if *root != "crates/moth-mon/tests" {
            fs::create_dir_all(workspace.path().join(root)).expect("configured source root");
        }
    }

    let error = audit_sources(workspace.path()).expect_err("missing root must not be skipped");
    assert!(error.contains("failed to read"));
    assert!(error.contains("tests"));
}

#[test]
fn reports_the_removed_diagnostic_payload_variant_by_name() {
    let source = format!(
        "enum DiagnosticPayload {{\n    {} {{ message: String }},\n}}\n",
        removed_payload_variant_name()
    );

    let findings = audit_source_fragment("src/example.rs", &source);

    assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
    assert_eq!(findings[0].rule, SourceRule::RemovedLegacyPayloadVariant);
    assert!(
        findings[0]
            .message
            .contains(&removed_payload_variant_name()),
        "the finding should name what it found: {}",
        findings[0].message
    );
}

/// The banned variant name, assembled so this file does not contain it either.
fn removed_payload_variant_name() -> String {
    ["Legacy", "Error"].concat()
}

#[test]
fn reports_lossy_path_text_interned_as_a_diagnostic_fact() {
    let source = "\
fn report(diagnostic_strings: &mut DiagnosticStringBuilder, path: &Path) {
    let path_id = diagnostic_strings.intern(&path.to_string_lossy());
}
";

    let findings = audit_source_fragment("src/build_system/example.rs", source);

    assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
    assert_eq!(findings[0].rule, SourceRule::UnportableDiagnosticPathFact);
    assert_eq!(findings[0].file, "src/build_system/example.rs");
    assert!(
        findings[0].message.contains("intern_path"),
        "the finding should point at the owned operation: {}",
        findings[0].message
    );
    assert!(
        findings[0].message.contains("to_string_lossy"),
        "the finding should name what it found: {}",
        findings[0].message
    );
}

#[test]
fn reports_displayed_path_text_interned_as_a_diagnostic_fact() {
    let direct = "\
fn report(diagnostic_strings: &mut DiagnosticStringBuilder, path: &Path) {
    let direct = diagnostic_strings.get_or_intern(path.display().to_string());
}
";
    let formatted = "\
fn report(diagnostic_strings: &mut DiagnosticStringBuilder, path: &Path) {
    let formatted = diagnostic_strings.intern(&format!(\"{}\", path.display()));
}
";

    for source in [direct, formatted] {
        let findings = audit_source_fragment("src/projects/example.rs", source);

        assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
        assert_eq!(findings[0].rule, SourceRule::UnportableDiagnosticPathFact);
        assert!(
            findings[0].message.contains("Path::display"),
            "the finding should name the display conversion: {}",
            findings[0].message
        );
    }
}

#[test]
fn reports_multiline_diagnostic_path_interning() {
    let source = "\
fn report(diagnostic_strings: &mut DiagnosticStringBuilder, path: &Path) {
    let lossy = diagnostic_strings
        .intern(
            &path.to_string_lossy(),
        );
    let displayed = diagnostic_strings.get_or_intern(
        path.display().to_string(),
    );
}
";

    let findings = audit_source_fragment("src/compiler_frontend/example.rs", source);

    assert_eq!(findings.len(), 2, "unexpected findings: {findings:?}");
    assert!(
        findings
            .iter()
            .all(|finding| finding.rule == SourceRule::UnportableDiagnosticPathFact)
    );
}

#[test]
fn accepts_native_io_identity_url_and_portable_path_text_use() {
    let source = "\
fn report(
    diagnostic_strings: &mut DiagnosticStringBuilder,
    table: &mut StringTable,
    path: &Path,
    validated_output: &ValidatedOutputPath,
) {
    let contents = fs::read_to_string(path);
    let canonical = path.canonicalize();
    let path_id = diagnostic_strings.intern_path(path);
    let source_path = table.try_intern_source_path(path);
    let output = diagnostic_strings.intern(validated_output.portable_relative_path());
    let url = diagnostic_strings.intern(&percent_encode_url_segment(path));
    let visible = portable_path_text(path);
    // An intermediate binding hides the conversion from the source rule.
    let intermediate = path.display().to_string();
    let message = format!(\"failed to read '{}'\", path.display());
    let url_id = diagnostic_strings.intern(&url);
}
";

    let findings = audit_source_fragment("src/build_system/example.rs", source);

    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn scopes_the_diagnostic_path_rule_to_production_src_sources() {
    let source = "\
fn report(diagnostic_strings: &mut DiagnosticStringBuilder, path: &Path) {
    let path_id = diagnostic_strings.intern(&path.to_string_lossy());
}
";

    for relative in [
        "src/example_tests.rs",
        "src/build_system/tests/example.rs",
        "crates/moth-cli/src/example.rs",
        "xtask/src/example.rs",
    ] {
        let findings = audit_source_fragment(relative, source);
        assert!(findings.is_empty(), "{relative}: {findings:?}");
    }
}

#[test]
fn ignores_diagnostic_path_interning_spellings_in_comments() {
    let source = "\
// let path_id = diagnostic_strings.intern(&path.to_string_lossy());
fn report() {
    let value = 1; // diagnostic_strings.get_or_intern(path.display().to_string());
    /*
     * diagnostic_strings.intern(&path.to_string_lossy());
     */
}
";

    let findings = audit_source_fragment("src/build_system/example.rs", source);

    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn ignores_diagnostic_path_interning_spellings_in_string_literals() {
    let source = r##"
fn report() {
    let example = "diagnostic_strings.intern(&path.to_string_lossy())";
    let raw = r#"diagnostic_strings.get_or_intern(path.display().to_string())"#;
}
"##;

    let findings = audit_source_fragment("src/build_system/example.rs", source);

    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn reports_a_flagged_call_containing_a_parenthesis_character_literal() {
    let source = "\
fn report(diagnostic_strings: &mut DiagnosticStringBuilder, path: &Path) {
    let path_id = diagnostic_strings.intern(&format!(\"{}{}\", '(', path.display()));
}
";

    let findings = audit_source_fragment("src/build_system/example.rs", source);

    assert_eq!(findings.len(), 1, "unexpected findings: {findings:?}");
    assert_eq!(findings[0].rule, SourceRule::UnportableDiagnosticPathFact);
}

#[test]
fn tolerates_a_truncated_interning_call() {
    // An argument that never closes must be skipped, not matched across the rest of the file.
    let source = "\
fn report(diagnostic_strings: &mut DiagnosticStringBuilder, path: &Path) {
    let path_id = diagnostic_strings.intern(&path.to_string_lossy();
}
";

    let findings = audit_source_fragment("src/build_system/example.rs", source);

    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn accepts_a_typed_display_call_with_arguments_in_an_interned_argument() {
    let source = "\
fn report(diagnostic_strings: &mut DiagnosticStringBuilder, snapshot: &TypeDisplaySnapshot) {
    let type_text = diagnostic_strings.intern(&snapshot.display(type_id, context).to_string());
}
";

    let findings = audit_source_fragment("src/build_system/example.rs", source);

    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

/// The report a run writes before it walks anything must say it measured nothing yet.
///
/// This is what stops an interrupted run from leaving the previous successful report in place,
/// where a reader would take it for this run's evidence. The zero counts are only safe to write
/// because `completed: false` is written with them.
#[test]
fn the_report_written_before_the_walk_claims_no_result() {
    let started = started_report(ReportRunIdentity::started("source-audit", None));

    assert!(!started.run.completed);
    assert_eq!(started.audited_file_count, 0);
    assert_eq!(started.findings, Vec::new());
    assert_eq!(
        started.audited_roots,
        AUDITED_SOURCE_ROOTS
            .iter()
            .map(|root| (*root).to_string())
            .collect::<Vec<String>>(),
        "the roots are known before the walk, so the started report names them"
    );
}
