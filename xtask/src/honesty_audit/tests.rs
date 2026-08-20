//! Self-tests for the honesty audit.
//!
//! Each rule is proved against fixture text rather than against the tree, for two reasons. A rule
//! whose only evidence is "the tree has none of these" passes just as well when the rule is
//! broken, and the audit's own source is exempt from the scan, so its rule table is the one place
//! these fragments can be written down.
//!
//! Every hard rule gets a pair: the shape it must find, and the shape it must not. The second is
//! the one that matters. An audit that reports the doc comment warning against a call is an audit
//! whose findings a reader learns to skip.

use super::{
    ANY, DeclaredMeasurement, Disposition, HARD_RULES, HONESTY_INVENTORY_SCHEMA_VERSION,
    HonestyLedger, LedgerFinding, LineMatcher, REVIEW_RULES, RuleScope, RuleView,
    SUITE_WEAK_CONTRACT_REVIEWS, SuiteInventoryEvidence, ViewTest, check_ledger_integrity,
    civil_from_days, disposition_suite_counters, in_scope, owns_tests, scan_lines, started_report,
    utc_timestamp,
};
use crate::report_file::ReportRunIdentity;
use std::collections::BTreeSet;
use std::time::{Duration, UNIX_EPOCH};

/// The codes of every hard rule that matches `content`, treated as a test-owning file.
fn hard_hits(content: &str) -> Vec<&'static str> {
    let lines = scan_lines(content);
    let owns = owns_tests(&lines);

    HARD_RULES
        .iter()
        .filter(|rule| {
            lines
                .iter()
                .any(|line| in_scope(rule.scope, owns) && rule.matcher.matches(line))
        })
        .map(|rule| rule.code)
        .collect()
}

/// The codes of every review rule that matches `content`.
fn review_hits(content: &str) -> Vec<&'static str> {
    let lines = scan_lines(content);
    let owns = owns_tests(&lines);

    REVIEW_RULES
        .iter()
        .filter(|rule| {
            lines
                .iter()
                .any(|line| in_scope(rule.scope, owns) && rule.matcher.matches(line))
        })
        .map(|rule| rule.code)
        .collect()
}

/// Wrap `body` in the minimum that makes a file test-owning.
fn in_a_test_file(body: &str) -> String {
    format!("#[test]\nfn a_case() {{\n{body}\n}}\n")
}

// ---------------------------------------------------------------------------
// Hard rules: the shape each one must find.
// ---------------------------------------------------------------------------

#[test]
fn finds_the_retired_temporary_directory_helper() {
    let source = in_a_test_file("    let root = test_support::temp_dir(\"case\");");

    assert!(hard_hits(&source).contains(&"retired_temp_dir_helper"));
}

#[test]
fn finds_a_tmp_literal_handed_to_a_filesystem_call() {
    let source = in_a_test_file("    fs::create_dir_all(\"/tmp/moth-case\").expect(\"created\");");

    assert!(hard_hits(&source).contains(&"tmp_literal_reaching_the_filesystem"));
}

#[test]
fn finds_a_discarded_current_directory_restoration() {
    let source = in_a_test_file("    let _ = std::env::set_current_dir(&original);");

    assert!(
        hard_hits(&source).contains(&"discarded_directory_or_environment_restoration"),
        "a failed restore leaves every later test running against the wrong directory"
    );
}

#[test]
fn finds_a_discarded_environment_restoration() {
    let source = in_a_test_file("    let _ = unsafe { std::env::remove_var(\"MOTH_HOME\") };");

    assert!(hard_hits(&source).contains(&"discarded_directory_or_environment_restoration"));
}

#[test]
fn finds_a_discarded_thread_join() {
    let source = in_a_test_file("    let _ = worker.join();");

    assert!(hard_hits(&source).contains(&"discarded_thread_join"));
}

#[test]
fn finds_an_ignore_with_no_reason() {
    let source = "#[test]\n#[ignore]\nfn a_case() {}\n";

    assert!(hard_hits(source).contains(&"unowned_ignore"));
}

#[test]
fn finds_a_report_written_straight_to_its_final_path() {
    let source = "fn write() {\n    fs::write(\"target/test-reports/coverage.json\", body)\n}\n";

    assert!(
        hard_hits(source).contains(&"unatomic_report_write"),
        "this rule applies to every file, not only test-owning ones"
    );
}

// ---------------------------------------------------------------------------
// Hard rules: the shapes each one must not find.
// ---------------------------------------------------------------------------

/// Prose about a banned call is not the banned call.
#[test]
fn a_doc_comment_describing_a_banned_shape_is_not_a_finding() {
    let source = in_a_test_file(
        "    // WHY: `let _ = handle.join()` silently discards a worker panic.\n\
         \x20   /// Never write `let _ = std::env::set_current_dir(&original);` here.\n\
         \x20   /* test_support::temp_dir was retired; use tempfile::tempdir. */",
    );

    assert_eq!(
        hard_hits(&source),
        Vec::<&str>::new(),
        "a comment naming a banned shape must not be reported as using it"
    );
}

/// A fragment inside a string literal is not code either.
#[test]
fn a_fixture_string_containing_a_banned_shape_is_not_a_finding() {
    let source = in_a_test_file("    let fixture = \"let _ = worker.join();\";");

    assert_eq!(hard_hits(&source), Vec::<&str>::new());
}

/// An `#[ignore]` that names its reason is governed, which is the whole rule.
#[test]
fn an_ignore_that_names_its_reason_is_not_a_finding() {
    let source = "#[test]\n#[ignore = \"needs a Node runtime; owned by the harness plan\"]\nfn a_case() {}\n";

    assert!(!hard_hits(source).contains(&"unowned_ignore"));
}

/// A `/tmp` value nothing acts on is a path value, not a workspace.
#[test]
fn a_tmp_literal_no_call_acts_on_is_a_review_finding_and_not_a_hard_one() {
    let source = in_a_test_file("    let current = PathBuf::from(\"/tmp\");");

    assert!(!hard_hits(&source).contains(&"tmp_literal_reaching_the_filesystem"));
    assert!(review_hits(&source).contains(&"inert_tmp_path_literals"));
}

/// The two `/tmp` rules must not both claim the same line.
///
/// A line counted as inert *and* as reaching the filesystem would make the review count and the
/// hard count describe overlapping sets, and a reader could not add them up.
#[test]
fn a_tmp_literal_is_either_inert_or_reaches_the_filesystem_never_both() {
    let acting = in_a_test_file("    fs::write(\"/tmp/case.json\", body).expect(\"written\");");

    assert!(hard_hits(&acting).contains(&"tmp_literal_reaching_the_filesystem"));
    assert!(
        !review_hits(&acting).contains(&"inert_tmp_path_literals"),
        "a literal a call acts on is not inert"
    );
}

/// A join with an argument is a path or string join, not a thread join.
#[test]
fn a_path_join_is_not_a_discarded_thread_join() {
    let source = in_a_test_file("    let _ = root.join(\"nested\");");

    assert!(!hard_hits(&source).contains(&"discarded_thread_join"));
}

/// A test-scoped rule must not fire on production code, where the same shape is often correct.
#[test]
fn a_test_scoped_rule_does_not_apply_to_a_file_that_owns_no_tests() {
    let source =
        "fn restore(original: &Path) {\n    let _ = std::env::set_current_dir(original);\n}\n";

    assert!(!hard_hits(source).contains(&"discarded_directory_or_environment_restoration"));
    assert!(
        hard_hits(source).is_empty(),
        "no every-file rule matches this fixture"
    );
}

// ---------------------------------------------------------------------------
// Rule-table invariants.
// ---------------------------------------------------------------------------

/// Every rule code names one rule.
///
/// The scan collects hits into a map keyed by code, so a duplicate would silently merge two
/// rules' findings under one description.
#[test]
fn every_rule_code_is_unique_across_both_tables() {
    let mut codes = BTreeSet::new();

    for code in HARD_RULES
        .iter()
        .map(|rule| rule.code)
        .chain(REVIEW_RULES.iter().map(|rule| rule.code))
    {
        assert!(codes.insert(code), "'{code}' is declared twice");
    }
}

/// Every review rule owes a reader a disposition *and* the reason it is the right one.
#[test]
fn every_review_rule_records_why_its_disposition_holds() {
    for rule in REVIEW_RULES {
        assert!(
            rule.rationale.len() > 40,
            "'{}' has no substantive rationale for its {:?} disposition",
            rule.code,
            rule.disposition
        );
        assert!(
            !rule.description.is_empty(),
            "'{}' does not say what it finds",
            rule.code
        );
    }
}

/// The plan names six dispositions; a disposition no rule uses is one nobody decided.
#[test]
fn every_disposition_the_audit_offers_is_used_by_something() {
    let used: BTreeSet<Disposition> = REVIEW_RULES
        .iter()
        .map(|rule| rule.disposition)
        .chain(
            SUITE_WEAK_CONTRACT_REVIEWS
                .iter()
                .map(|(_, disposition, _)| *disposition),
        )
        .collect();

    for disposition in [
        Disposition::Hardened,
        Disposition::NarrowApiWithOneValidOutcome,
        Disposition::IntentionallySmokeLevel,
        Disposition::RenderedProseIsTheContract,
        Disposition::PlatformSpecificByRealApiOwnership,
        Disposition::MovedToStructuredAudit,
    ] {
        assert!(
            used.contains(&disposition),
            "no rule or suite counter carries {disposition:?}"
        );
    }
}

/// The audit must not become a blanket ban on the categories the plan says to keep legal.
#[test]
fn the_broad_categories_the_plan_protects_are_review_rules_and_never_hard_ones() {
    let hard_codes: Vec<&str> = HARD_RULES.iter().map(|rule| rule.code).collect();

    for protected in [
        "broad_failure_assertions",
        "broad_contains_assertions",
        "weak_positive_assertions",
    ] {
        assert!(
            !hard_codes.contains(&protected),
            "'{protected}' must stay a review category: is_err, contains and any all have honest \
             uses, and a gate that fails on each of them is a gate that gets turned off"
        );
        assert!(
            REVIEW_RULES.iter().any(|rule| rule.code == protected),
            "'{protected}' must still be counted"
        );
    }
}

// ---------------------------------------------------------------------------
// The matcher itself.
// ---------------------------------------------------------------------------

#[test]
fn a_matcher_requiring_the_other_view_needs_both_halves() {
    let matcher = LineMatcher {
        view: RuleView::Literal,
        primary: ViewTest {
            required: &["/tmp"],
            any_of: &[],
            absent: &[],
        },
        other: ViewTest {
            required: &[],
            any_of: &["fs::write("],
            absent: &[],
        },
    };

    let both = scan_lines("fs::write(\"/tmp/x\", b);");
    let literal_only = scan_lines("let path = \"/tmp/x\";");
    let code_only = scan_lines("fs::write(target, b);");

    assert!(matcher.matches(&both[0]));
    assert!(!matcher.matches(&literal_only[0]));
    assert!(!matcher.matches(&code_only[0]));
}

#[test]
fn an_unconstrained_view_test_matches_anything() {
    assert!(ANY.matches(""));
    assert!(ANY.matches("anything at all"));
}

#[test]
fn scanning_splits_a_line_into_code_and_literal_views_keeping_positions() {
    let lines = scan_lines("let a = \"text\"; // note\n");

    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].number, 1);
    assert_eq!(lines[0].code, "let a =       ;        ");
    assert_eq!(lines[0].literals, "        \"text\"         ");
    assert_eq!(lines[0].text, "let a = \"text\"; // note");
}

#[test]
fn a_file_is_test_owning_only_when_a_test_attribute_is_code() {
    assert!(owns_tests(&scan_lines("#[test]\nfn a() {}\n")));
    assert!(owns_tests(&scan_lines("#[cfg(test)]\nmod tests;\n")));
    assert!(
        !owns_tests(&scan_lines("// this module has no #[test] items\n")),
        "a comment mentioning the attribute does not make a file test-owning"
    );
}

#[test]
fn rule_scope_decides_which_files_a_rule_reads() {
    assert!(in_scope(RuleScope::EveryFile, false));
    assert!(in_scope(RuleScope::EveryFile, true));
    assert!(in_scope(RuleScope::TestOwningFiles, true));
    assert!(!in_scope(RuleScope::TestOwningFiles, false));
}

// ---------------------------------------------------------------------------
// Ledger integrity.
// ---------------------------------------------------------------------------

fn ledger_with(findings: Vec<LedgerFinding>) -> HonestyLedger {
    HonestyLedger {
        schema_version: 1,
        work_id: "test-suite-honesty".to_string(),
        base_revision: "f41f93a7a".to_string(),
        declared_measurements: vec![DeclaredMeasurement {
            name: "unit_tests".to_string(),
            command: "cargo test --workspace".to_string(),
            result: "4396 passed".to_string(),
            recorded_by_phase: "Phase 8".to_string(),
        }],
        findings,
        infrastructure_added: Vec::new(),
    }
}

fn finding(code: &str, severity: &str, status: &str) -> LedgerFinding {
    LedgerFinding {
        code: code.to_string(),
        severity: severity.to_string(),
        status: status.to_string(),
        description: "a described finding".to_string(),
        disposition: "a recorded decision".to_string(),
        owning_phase: "Phase 1".to_string(),
    }
}

#[test]
fn a_complete_ledger_has_no_integrity_findings() {
    let ledger = ledger_with(vec![
        finding("one", "hard", "resolved"),
        finding("two", "review", "open"),
    ]);

    assert_eq!(check_ledger_integrity(&ledger), Vec::new());
}

/// A finding with no disposition is the exact thing this campaign removes.
#[test]
fn a_finding_with_no_disposition_is_an_integrity_finding() {
    let mut entry = finding("undecided", "review", "resolved");
    entry.disposition = "   ".to_string();

    let findings = check_ledger_integrity(&ledger_with(vec![entry]));

    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, "undecided");
    assert!(findings[0].problem.contains("disposition"));
}

#[test]
fn a_duplicate_finding_code_is_an_integrity_finding() {
    let findings = check_ledger_integrity(&ledger_with(vec![
        finding("same", "review", "resolved"),
        finding("same", "hard", "resolved"),
    ]));

    assert_eq!(findings.len(), 1);
    assert!(findings[0].problem.contains("declared 2 times"));
}

#[test]
fn an_unknown_severity_or_status_is_an_integrity_finding() {
    let findings = check_ledger_integrity(&ledger_with(vec![
        finding("bad_severity", "critical", "resolved"),
        finding("bad_status", "hard", "wontfix"),
    ]));

    let problems: Vec<&str> = findings
        .iter()
        .map(|finding| finding.problem.as_str())
        .collect();
    assert_eq!(problems.len(), 2);
    assert!(problems[0].contains("severity 'critical'"));
    assert!(problems[1].contains("status 'wontfix'"));
}

/// The tracked ledger this repository ships must itself be complete.
#[test]
fn the_tracked_ledger_parses_and_is_internally_consistent() {
    let ledger: HonestyLedger = serde_json::from_str(include_str!(
        "../../../docs/roadmap/evidence/honesty_ledger.json"
    ))
    .expect("the tracked ledger should parse");

    assert_eq!(check_ledger_integrity(&ledger), Vec::new());
    assert!(
        !ledger.findings.is_empty(),
        "an empty ledger would pass every check by declaring nothing"
    );
    assert!(
        ledger
            .findings
            .iter()
            .all(|finding| !(finding.severity == "hard" && finding.status == "open")),
        "an open hard finding fails the audit; Patch A cannot close with one"
    );
}

// ---------------------------------------------------------------------------
// Composed suite evidence.
// ---------------------------------------------------------------------------

/// A counter nobody measured must not be reported as zero.
#[test]
fn an_absent_suite_inventory_reports_unknown_counters_rather_than_zero() {
    let reviews = disposition_suite_counters(&SuiteInventoryEvidence::Absent {
        path: "target/test-reports/integration_suite_inventory.json".to_string(),
    });

    assert_eq!(reviews.len(), SUITE_WEAK_CONTRACT_REVIEWS.len());
    assert!(
        reviews.iter().all(|review| review.occurrences.is_none()),
        "'no report was read' is not the same fact as 'the suite counted none'"
    );
}

#[test]
fn a_present_suite_inventory_supplies_the_counters_it_names() {
    let evidence = SuiteInventoryEvidence::Present {
        path: "p".to_string(),
        schema_version: 8,
        run_id: "1-0-2".to_string(),
        run_command: "tests --audit".to_string(),
        run_completed: true,
        hard_policy_violation_count: 0,
        advisory_finding_count: 84,
        summary: serde_json::json!({ "smoke_role_cases": 3 }),
    };

    let reviews = disposition_suite_counters(&evidence);
    let smoke = reviews
        .iter()
        .find(|review| review.counter == "smoke_role_cases")
        .expect("the counter is dispositioned");

    assert_eq!(smoke.occurrences, Some(3));
    assert_eq!(smoke.disposition, Disposition::IntentionallySmokeLevel);
    assert_eq!(
        reviews
            .iter()
            .find(|review| review.counter == "acceptance_only_backend_blocks")
            .expect("the counter is dispositioned")
            .occurrences,
        None,
        "a counter the summary does not name stays unknown"
    );
}

// ---------------------------------------------------------------------------
// The generated timestamp.
// ---------------------------------------------------------------------------

/// `generated_at` is the field a hand-maintained inventory got wrong, so it is measured here.
#[test]
fn the_timestamp_is_the_utc_civil_time_of_the_moment_it_is_given() {
    assert_eq!(utc_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
    assert_eq!(
        utc_timestamp(UNIX_EPOCH + Duration::from_secs(1_771_545_600)),
        "2026-02-20T00:00:00Z"
    );
    assert_eq!(
        utc_timestamp(UNIX_EPOCH + Duration::from_secs(1_771_545_600 + 45_296)),
        "2026-02-20T12:34:56Z"
    );
}

#[test]
fn the_civil_date_conversion_handles_leap_days_and_century_rules() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    // 2000 is a leap year; 1900 was not, which is the rule a naive conversion gets wrong.
    assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    assert_eq!(civil_from_days(11_017), (2000, 3, 1));
    assert_eq!(civil_from_days(-25_567), (1900, 1, 1));
    assert_eq!(civil_from_days(20_454), (2026, 1, 1));
}

#[test]
fn the_inventory_schema_version_is_the_one_consumers_are_told_to_expect() {
    assert_eq!(HONESTY_INVENTORY_SCHEMA_VERSION, 1);
}

/// The report a run writes before it scans anything must claim no verdict.
///
/// `passed: false` in the started report matters more than the zero counts: a reader who finds
/// this file has found an interrupted run, and the one thing it must not say is that the audit
/// passed.
#[test]
fn the_report_written_before_the_scan_claims_no_verdict() {
    let started = started_report(ReportRunIdentity::started("honesty-audit", None));

    assert!(!started.run.completed);
    assert!(
        !started.gate.passed,
        "an audit that has not run has not passed"
    );
    assert_eq!(started.scanned_file_count, 0);
    assert_eq!(started.hard_findings, Vec::new());
    assert_eq!(started.declared_findings, Vec::new());
    assert!(matches!(
        started.composed.integration_suite_inventory,
        SuiteInventoryEvidence::Absent { .. }
    ));
}
