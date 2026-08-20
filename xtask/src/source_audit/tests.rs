//! Self-tests for the broad-source architecture audit.
//!
//! Each rule is proved against fixture text rather than against whatever the tree happens to
//! contain, so a rule keeps its meaning when the tree changes.

use super::{
    AUDIT_IMPLEMENTATION_FILES, AUDITED_SOURCE_ROOTS, SOURCE_AUDIT_SCHEMA_VERSION, SourceRule,
    audit_source_fragment, audit_sources, relative_display_path,
};
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
fn the_audit_walks_both_workspace_source_trees() {
    assert_eq!(AUDITED_SOURCE_ROOTS, ["src", "xtask/src"]);
}

#[test]
fn findings_name_files_with_forward_slashes_under_the_workspace_root() {
    let root = Path::new("/work/moth");
    let path = root.join("xtask").join("src").join("source_audit.rs");

    assert_eq!(
        relative_display_path(root, &path).expect("an ASCII path is valid UTF-8"),
        "xtask/src/source_audit.rs"
    );
}

#[test]
fn the_report_schema_version_is_the_one_consumers_are_told_to_expect() {
    assert_eq!(SOURCE_AUDIT_SCHEMA_VERSION, 1);
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
