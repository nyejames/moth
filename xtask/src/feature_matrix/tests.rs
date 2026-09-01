//! Self-tests for the feature-lane matrix.
//!
//! These prove the scanner reads the tree the way the check claims, and that the lane table and
//! the workspace manifests agree. The lane-table tests are the gate itself: if the union of lane
//! features stops matching the declared features, a feature-gated test silently stops running.

use super::{
    COVERAGE_REPORT_SCHEMA_VERSION, FEATURE_LANES, FeatureLane, FeatureLaneKind, LaneFailure,
    LaneOutcome, LaneResult, MATRIX_RESULTS_SCHEMA_VERSION, MatrixResultsReport, cfg_feature_names,
    declared_features, lane_report, lanes_enabling, standard_execution_lanes,
};
use crate::report_file::ReportRunIdentity;
use std::collections::BTreeSet;

/// The manifests the lane table must agree with, read at compile time so the test cannot drift
/// from the tree it claims to check.
const MOTH_MANIFEST: &str = include_str!("../../../Cargo.toml");
const XTASK_MANIFEST: &str = include_str!("../../Cargo.toml");
const ROOT_JUSTFILE: &str = include_str!("../../../justfile");

fn lane_features(package: &str) -> BTreeSet<String> {
    FEATURE_LANES
        .iter()
        .filter(|lane| lane.package == package)
        .flat_map(|lane| lane.features.iter().map(|name| (*name).to_string()))
        .collect()
}

#[test]
fn every_declared_moth_feature_is_enabled_by_a_lane() {
    let declared = declared_features(MOTH_MANIFEST).expect("the moth manifest should parse");

    let uncovered: Vec<&String> = declared
        .iter()
        .filter(|feature| lanes_enabling("moth", feature).is_empty())
        .collect();

    assert!(
        uncovered.is_empty(),
        "features with no executing lane: {uncovered:?}"
    );
}

#[test]
fn no_lane_enables_a_feature_the_package_does_not_declare() {
    let declared = declared_features(MOTH_MANIFEST).expect("the moth manifest should parse");

    let unknown: Vec<&&str> = FEATURE_LANES
        .iter()
        .filter(|lane| lane.package == "moth")
        .flat_map(|lane| lane.features.iter())
        .filter(|feature| !declared.contains(**feature))
        .collect();

    assert!(
        unknown.is_empty(),
        "lanes name undeclared features: {unknown:?}"
    );
}

#[test]
fn the_xtask_package_declares_no_features_for_a_lane_to_select() {
    let declared = declared_features(XTASK_MANIFEST).expect("the xtask manifest should parse");

    assert_eq!(declared, BTreeSet::new());
    assert_eq!(lane_features("xtask"), BTreeSet::new());
}

#[test]
fn lane_names_are_unique() {
    let mut names: Vec<&str> = FEATURE_LANES.iter().map(|lane| lane.name).collect();
    let total = names.len();
    names.sort_unstable();
    names.dedup();

    assert_eq!(names.len(), total, "duplicate lane names in the matrix");
}

#[test]
fn a_lane_without_features_runs_its_package_unconfigured() {
    let lane = FeatureLane {
        name: "default",
        package: "moth",
        features: &[],
        kind: FeatureLaneKind::Standard,
        owns: "the shipped configuration",
    };

    assert_eq!(
        lane.command_line(),
        "cargo test -p moth --quiet -- --format terse"
    );
}

#[test]
fn a_lane_command_names_every_feature_it_enables() {
    let lane = FeatureLane {
        name: "timers-counters",
        package: "moth",
        features: &["timers", "benchmark_counters"],
        kind: FeatureLaneKind::Standard,
        owns: "collector-backed counters",
    };

    assert_eq!(
        lane.command_line(),
        "cargo test -p moth --quiet --features timers,benchmark_counters -- --format terse"
    );
}

#[test]
fn boracle_is_an_opt_in_lane_owned_by_just_boracle() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");

    assert_eq!(lane.package, "moth");
    assert_eq!(lane.features, &["boracle"]);
    assert_eq!(
        lane.kind,
        FeatureLaneKind::OptIn {
            command: "just boracle"
        }
    );
    assert_eq!(lane.owned_command(), "just boracle");
    assert_eq!(lane_report(lane).command, "just boracle");
    assert_eq!(lane_report(lane).lane_kind, "opt_in");

    let coverage = lanes_enabling("moth", "boracle");
    assert!(coverage.standard_lanes.is_empty());
    assert_eq!(coverage.opt_in_lanes, vec!["boracle", "boracle-campaign"]);
    assert!(
        super::validate_opt_in_lane(lane, ROOT_JUSTFILE).is_ok(),
        "the declared Boracle owner must remain connected to the Justfile recipe"
    );
}

#[test]
fn the_boracle_campaign_is_a_separate_opt_in_lane() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle-campaign")
        .expect("the Boracle campaign lane should exist");

    assert_eq!(lane.package, "moth");
    assert_eq!(lane.features, &["boracle", "boracle_campaign"]);
    assert_eq!(
        lane.kind,
        FeatureLaneKind::OptIn {
            command: "just boracle-campaign"
        }
    );
    assert_eq!(lane.owned_command(), "just boracle-campaign");

    // The campaign is the only lane that compiles the generated differential sweep, so its
    // feature must have exactly one owner and must never reach a standard lane.
    let coverage = lanes_enabling("moth", "boracle_campaign");
    assert!(coverage.standard_lanes.is_empty());
    assert_eq!(coverage.opt_in_lanes, vec!["boracle-campaign"]);
    assert!(
        super::validate_opt_in_lane(lane, ROOT_JUSTFILE).is_ok(),
        "the declared campaign owner must remain connected to the Justfile recipe"
    );
}

#[test]
fn opt_in_ownership_rejects_a_missing_recipe() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");

    let error = super::validate_opt_in_lane(lane, "other:\n    cargo test --features boracle\n")
        .expect_err("a missing recipe must not count as coverage");

    assert!(error.contains("does not define a recipe"));
}

#[test]
fn opt_in_ownership_rejects_a_non_just_owner_command() {
    let lane = FeatureLane {
        name: "boracle",
        package: "moth",
        features: &["boracle"],
        kind: FeatureLaneKind::OptIn {
            command: "cargo test --features boracle",
        },
        owns: "the deterministic Boracle developer gate",
    };

    let error = super::validate_opt_in_lane(&lane, ROOT_JUSTFILE)
        .expect_err("an owner outside the Just command contract must be rejected");

    assert!(error.contains("not a `just <recipe>` command"));
}

#[test]
fn opt_in_ownership_rejects_a_recipe_with_the_wrong_feature() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");
    let justfile = "boracle:\n    cargo test -p moth --features show_hir\n";

    let error = super::validate_opt_in_lane(lane, justfile)
        .expect_err("a recipe disconnected from the feature must not count as coverage");

    assert!(error.contains("does not run Cargo with '--features boracle'"));
}

#[test]
fn opt_in_ownership_rejects_a_renamed_feature_token() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");
    let justfile = "boracle:\n    cargo test -p moth --features boracle_renamed\n";

    let error = super::validate_opt_in_lane(lane, justfile)
        .expect_err("a renamed feature must not count as Boracle coverage");

    assert!(error.contains("does not run Cargo with '--features boracle'"));
}

#[test]
fn opt_in_ownership_rejects_a_non_executing_feature_decoy() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");
    let justfile = "boracle:\n    @echo \"cargo test --features boracle\"\n";

    let error = super::validate_opt_in_lane(lane, justfile)
        .expect_err("a printed command must not count as Boracle coverage");

    assert!(error.contains("does not run Cargo with '--features boracle'"));
}

#[test]
fn opt_in_ownership_rejects_feature_text_after_a_shell_comment() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");
    let justfile = "boracle:\n    cargo test # --features boracle\n";

    let error = super::validate_opt_in_lane(lane, justfile)
        .expect_err("commented feature text must not count as Boracle coverage");

    assert!(error.contains("does not run Cargo with '--features boracle'"));
}

#[test]
fn opt_in_ownership_rejects_feature_text_after_a_shell_separator() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");
    let justfile = "boracle:\n    cargo test; echo --features boracle\n";

    let error = super::validate_opt_in_lane(lane, justfile)
        .expect_err("a later shell command must not count as Boracle coverage");

    assert!(error.contains("does not run Cargo with '--features boracle'"));
}

#[test]
fn opt_in_ownership_accepts_a_valid_cargo_owner_with_an_inline_comment() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");
    let justfile = "boracle:\n    cargo test -p moth --features boracle # explanation; ignored\n";

    assert!(
        super::validate_opt_in_lane(lane, justfile).is_ok(),
        "shell punctuation after a valid Cargo command belongs to the ignored comment"
    );
}

#[test]
fn opt_in_ownership_rejects_metadata_without_test_execution() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");
    let justfile = "boracle:\n    cargo metadata -p moth --features boracle\n";

    let error = super::validate_opt_in_lane(lane, justfile)
        .expect_err("metadata must not count as an executing feature lane");

    assert!(error.contains("does not run Cargo with '--features boracle'"));
}

#[test]
fn opt_in_ownership_rejects_a_no_run_test_command() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");
    let justfile = "boracle:\n    cargo test -p moth --no-run --features boracle\n";

    let error = super::validate_opt_in_lane(lane, justfile)
        .expect_err("a no-run test command must not count as execution");

    assert!(error.contains("does not run Cargo with '--features boracle'"));
}

#[test]
fn opt_in_ownership_rejects_a_test_command_for_another_package() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");
    let justfile = "boracle:\n    cargo test -p xtask --features boracle\n";

    let error = super::validate_opt_in_lane(lane, justfile)
        .expect_err("another package must not count as Boracle execution");

    assert!(error.contains("does not run Cargo with '--features boracle'"));
}

#[test]
fn opt_in_ownership_rejects_duplicate_feature_selectors() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");
    let justfile = "boracle:\n    cargo test -p moth --features boracle --features show_hir\n";

    let error = super::validate_opt_in_lane(lane, justfile)
        .expect_err("duplicate feature selectors must not count as exact ownership");

    assert!(error.contains("does not run Cargo with '--features boracle'"));
}

#[test]
fn opt_in_ownership_rejects_duplicate_package_selectors() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");
    let justfile = "boracle:\n    cargo test -p moth --package xtask --features boracle\n";

    let error = super::validate_opt_in_lane(lane, justfile)
        .expect_err("duplicate package selectors must not count as exact ownership");

    assert!(error.contains("does not run Cargo with '--features boracle'"));
}

#[test]
fn opt_in_ownership_rejects_all_features() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");
    let justfile = "boracle:\n    cargo test -p moth --features boracle --all-features\n";

    let error = super::validate_opt_in_lane(lane, justfile)
        .expect_err("all-features must not count as exact Boracle ownership");

    assert!(error.contains("does not run Cargo with '--features boracle'"));
}

#[test]
fn opt_in_ownership_rejects_an_attached_duplicate_package_selector() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");
    let justfile = "boracle:\n    cargo test -p moth --package=moth --features boracle\n";

    let error = super::validate_opt_in_lane(lane, justfile)
        .expect_err("attached duplicate package selectors must not count as exact ownership");

    assert!(error.contains("does not run Cargo with '--features boracle'"));
}

#[test]
fn opt_in_ownership_rejects_an_attached_duplicate_feature_selector() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");
    let justfile = "boracle:\n    cargo test -p moth --features boracle --features=show_hir\n";

    let error = super::validate_opt_in_lane(lane, justfile)
        .expect_err("attached duplicate feature selectors must not count as exact ownership");

    assert!(error.contains("does not run Cargo with '--features boracle'"));
}

#[test]
fn opt_in_ownership_rejects_a_compact_package_selector() {
    let lane = FEATURE_LANES
        .iter()
        .find(|lane| lane.name == "boracle")
        .expect("the Boracle lane should exist");
    let justfile = "boracle:\n    cargo test -pmoth --features boracle\n";

    let error = super::validate_opt_in_lane(lane, justfile)
        .expect_err("compact package selectors must not count as exact ownership");

    assert!(error.contains("does not run Cargo with '--features boracle'"));
}

#[test]
fn the_standard_execution_set_excludes_every_boracle_lane() {
    let standard_names: Vec<&str> = standard_execution_lanes().map(|lane| lane.name).collect();

    assert!(!standard_names.contains(&"boracle"));
    assert!(!standard_names.contains(&"boracle-campaign"));

    let opt_in_count = FEATURE_LANES
        .iter()
        .filter(|lane| matches!(lane.kind, FeatureLaneKind::OptIn { .. }))
        .count();

    assert_eq!(opt_in_count, 2);
    assert_eq!(standard_names.len(), FEATURE_LANES.len() - opt_in_count);
}

#[test]
fn declared_features_reads_every_key_of_the_features_table() {
    let manifest = "[package]\nname = \"x\"\n\n[features]\nalpha = []\nbeta = [\"alpha\"]\n";

    assert_eq!(
        declared_features(manifest).expect("manifest should parse"),
        BTreeSet::from(["alpha".to_string(), "beta".to_string()])
    );
}

#[test]
fn declared_features_is_empty_when_the_manifest_declares_none() {
    let manifest = "[package]\nname = \"x\"\n";

    assert_eq!(
        declared_features(manifest).expect("manifest should parse"),
        BTreeSet::new()
    );
}

#[test]
fn declared_features_rejects_a_features_key_that_is_not_a_table() {
    let error = declared_features("features = 3\n").expect_err("a scalar features key is invalid");

    assert_eq!(error, "[features] is not a table");
}

#[test]
fn scanner_reads_plain_nested_and_negated_cfg_forms() {
    let source = r#"
#[cfg(feature = "timers")]
fn a() {}

#[cfg(not(feature = "timers"))]
fn b() {}

#[cfg(all(test, feature = "benchmark_counters"))]
mod c {}

#[cfg_attr(not(feature = "benchmark_counters"), allow(dead_code))]
struct D;
"#;

    assert_eq!(
        cfg_feature_names(source),
        vec![
            "timers".to_string(),
            "timers".to_string(),
            "benchmark_counters".to_string(),
            "benchmark_counters".to_string(),
        ]
    );
}

#[test]
fn scanner_ignores_a_cfg_attribute_written_inside_a_rust_string_literal() {
    // xtask's erasure gate carries cfg attributes as scan input. Counting those would attribute
    // a moth feature to the xtask package, which declares none.
    let source = r##"
const SAMPLE: &str = "#[cfg(feature = \"timers\")]";
#[cfg(feature = "show_hir")]
fn real() {}
"##;

    assert_eq!(cfg_feature_names(source), vec!["show_hir".to_string()]);
}

#[test]
fn scanner_ignores_a_feature_comparison_outside_a_cfg_span() {
    let source = "let matched = feature == \"timers\";\n";

    assert!(cfg_feature_names(source).is_empty());
}

#[test]
fn scanner_stops_at_an_unbalanced_cfg_span() {
    // A truncated attribute must not pull the rest of the file into the scan.
    let source = "#[cfg(feature = \n";

    assert!(cfg_feature_names(source).is_empty());
}

#[test]
fn scanner_does_not_read_past_the_end_of_one_cfg_span() {
    let source = "#[cfg(unix)]\nfn a() {}\nconst F: &str = \"feature = \\\"timers\\\"\";\n";

    assert!(cfg_feature_names(source).is_empty());
}

#[test]
fn scanner_ignores_a_cfg_attribute_written_in_a_comment() {
    // This module's own doc comment names cfg attributes as prose.
    let source = "// #[cfg(feature = \"timers\")]\n/* #[cfg(feature = \"show_ast\")] */\n";

    assert!(cfg_feature_names(source).is_empty());
}

#[test]
fn scanner_ignores_a_cfg_attribute_written_in_a_raw_string() {
    let source = "const SCAN_INPUT: &str = r#\"#[cfg(feature = \"timers\")]\"#;\n";

    assert!(cfg_feature_names(source).is_empty());
}

#[test]
fn a_quote_character_literal_does_not_desynchronise_the_scan() {
    let source = "fn q(c: char) -> bool { c == '\"' }\n#[cfg(feature = \"show_hir\")]\nfn a() {}\n";

    assert_eq!(cfg_feature_names(source), vec!["show_hir".to_string()]);
}

#[test]
fn a_lifetime_does_not_desynchronise_the_scan() {
    let source =
        "fn q<'a>(v: &'a str) -> &'a str { v }\n#[cfg(feature = \"show_ast\")]\nfn a() {}\n";

    assert_eq!(cfg_feature_names(source), vec!["show_ast".to_string()]);
}

#[test]
fn scanner_ignores_an_identifier_that_merely_ends_in_cfg() {
    let source = "let value = build_cfg(feature_flag);\n";

    assert!(cfg_feature_names(source).is_empty());
}

#[test]
fn the_coverage_schema_version_is_the_one_consumers_are_told_to_expect() {
    assert_eq!(COVERAGE_REPORT_SCHEMA_VERSION, 3);
}

#[test]
fn the_matrix_results_schema_version_is_the_one_consumers_are_told_to_expect() {
    assert_eq!(MATRIX_RESULTS_SCHEMA_VERSION, 2);
}

/// The coverage map must not be able to state a lane outcome.
///
/// `feature-lane-check` writes this report without running a lane, so any outcome field on it
/// would be a value no run had measured. Keeping the two reports apart is what makes that
/// impossible rather than merely unlikely.
#[test]
fn the_coverage_report_states_lane_coverage_and_never_lane_outcomes() {
    let serialised = serde_json::to_value(lane_report(&FEATURE_LANES[0]))
        .expect("a lane report should serialise");
    let lane = serialised.as_object().expect("a lane is an object");

    let mut fields: Vec<&str> = lane.keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        vec![
            "command",
            "features",
            "lane_kind",
            "name",
            "owns",
            "package"
        ]
    );
}

/// A matrix that stops partway must report the lanes it never reached.
#[test]
fn an_unfinished_matrix_report_marks_every_unreached_lane_pending() {
    let report = MatrixResultsReport {
        schema_version: MATRIX_RESULTS_SCHEMA_VERSION,
        run: ReportRunIdentity::started("feature-matrix", None),
        lanes: vec![
            LaneResult {
                lane: lane_report(&FEATURE_LANES[0]),
                result: LaneOutcome::Passed,
            },
            LaneResult {
                lane: lane_report(&FEATURE_LANES[1]),
                result: LaneOutcome::Pending,
            },
        ],
    };

    assert!(!report.run.completed);
    assert_eq!(report.passed(), 1);
    assert!(
        report.failures().is_empty(),
        "a lane that never ran is unmeasured, not failed"
    );
}

/// A failed lane must carry the failure a reader would need to reproduce it.
#[test]
fn a_failed_lane_records_how_it_failed() {
    assert_eq!(
        LaneFailure::Exit(Some(101)).into_outcome(),
        LaneOutcome::Failed {
            exit_code: Some(101)
        }
    );
    assert_eq!(
        LaneFailure::Launch("no such file".to_string()).into_outcome(),
        LaneOutcome::LaunchFailed {
            error: "no such file".to_string()
        }
    );

    let report = MatrixResultsReport {
        schema_version: MATRIX_RESULTS_SCHEMA_VERSION,
        run: ReportRunIdentity::started("feature-matrix", None),
        lanes: vec![LaneResult {
            lane: lane_report(&FEATURE_LANES[0]),
            result: LaneFailure::Exit(Some(101)).into_outcome(),
        }],
    };
    assert_eq!(report.failures().len(), 1);
    assert_eq!(report.passed(), 0);
}
