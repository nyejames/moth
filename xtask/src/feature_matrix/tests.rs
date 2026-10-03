//! Self-tests for the feature-lane matrix.
//!
//! Complete temporary inventories exercise the actual coverage builder, including extracted
//! source/test owners and required-root failures. Scanner fixtures protect lexical boundaries.

use super::{
    FEATURE_LANES, FeatureLane, FeatureLaneKind, LaneFailure, LaneOutcome, LaneResult,
    MATRIX_RESULTS_SCHEMA_VERSION, MatrixResultsReport, build_coverage_report_for_audit,
    cfg_feature_names, declared_features, lane_report, lanes_enabling, standard_execution_lanes,
};
use crate::report_file::ReportRunIdentity;
use crate::source_tree::relative_display_path;
use crate::test_fs::assert_path_missing;
use std::collections::BTreeSet;
use std::fs;
use tempfile::{TempDir, tempdir};

// These manifests and recipes are parser inputs, not source-text assertions.
const MOTH_MANIFEST: &str = include_str!("../../../Cargo.toml");
const ROOT_JUSTFILE: &str = include_str!("../../../justfile");

fn fixture_workspace() -> TempDir {
    let workspace = tempdir().expect("temporary coverage workspace");
    fs::write(workspace.path().join("justfile"), ROOT_JUSTFILE).expect("fixture recipes");

    for (package, manifest, roots) in super::PACKAGE_SOURCES {
        for root in *roots {
            fs::create_dir_all(workspace.path().join(root)).expect("required fixture source root");
        }

        let contents = if *package == "moth" {
            MOTH_MANIFEST.to_string()
        } else {
            format!("[package]\nname = \"{package}\"\nversion = \"0.1.0\"\n")
        };
        fs::write(workspace.path().join(manifest), contents).expect("fixture package manifest");
    }

    workspace
}

#[test]
fn extracted_source_and_test_owners_report_exact_undeclared_cfg_features() {
    let workspace = fixture_workspace();
    let violations = [
        (
            "moth-lexical",
            "crates/moth-lexical/src/identifier.rs",
            "identifer_policy",
        ),
        (
            "moth-lexical",
            "crates/moth-lexical/src/tests/numeric.rs",
            "numeric_texxt",
        ),
        ("moth-mon", "crates/moth-mon/src/reader.rs", "mon_decodde"),
        (
            "moth-mon",
            "crates/moth-mon/src/tests/schema.rs",
            "schema_defaullt",
        ),
        (
            "moth-mon",
            "crates/moth-mon/tests/public_api.rs",
            "public_coddec",
        ),
    ];

    for (_, file, feature) in violations {
        let path = workspace.path().join(file);
        fs::create_dir_all(path.parent().expect("fixture file parent"))
            .expect("fixture source directory");
        let source = format!("#[cfg(feature = \"{feature}\")]\n#[test]\nfn gated() {{}}\n");
        fs::write(path, source).expect("misspelled cfg fixture");
    }

    let report =
        build_coverage_report_for_audit(workspace.path()).expect("complete inventory should scan");
    let mut expected_findings: Vec<String> = violations
        .iter()
        .map(|(package, file, feature)| {
            format!(
                "{file}: cfg names feature '{feature}', which package '{package}' does not declare"
            )
        })
        .collect();
    expected_findings.sort();
    let mut findings = report.findings;
    findings.sort();
    assert_eq!(findings, expected_findings);

    let expected_files: BTreeSet<&str> = violations.iter().map(|(_, file, _)| *file).collect();
    let actual_files: BTreeSet<&str> = report
        .undeclared_cfg_features
        .iter()
        .map(|site| {
            assert_eq!(site.occurrences, 1);
            assert!(site.has_test_items);
            site.file.as_str()
        })
        .collect();
    assert_eq!(actual_files, expected_files);
    assert_eq!(
        report.scanned_source_roots,
        [
            "src",
            "crates/moth-lexical/src",
            "crates/moth-mon/src",
            "crates/moth-mon/tests",
            "xtask/src",
        ]
    );
}

#[test]
fn coverage_fails_closed_when_any_configured_root_is_missing() {
    for (_, _, roots) in super::PACKAGE_SOURCES {
        for root in *roots {
            let workspace = fixture_workspace();
            let missing = workspace.path().join(root);
            fs::remove_dir_all(&missing).expect("remove required fixture root");
            assert_path_missing(&missing);

            let error = build_coverage_report_for_audit(workspace.path())
                .expect_err("a missing configured root must fail coverage");
            let portable_error = error.replace('\\', "/");
            let missing_display = relative_display_path(workspace.path(), &missing)
                .expect("missing root has a portable path");
            assert!(
                portable_error.contains(&format!("/{missing_display}'")),
                "failure must name the missing root: {error}"
            );
            assert!(error.starts_with("failed to read '"));
        }
    }
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
