//! Self-tests for the feature-lane matrix.
//!
//! These prove the scanner reads the tree the way the check claims, and that the lane table and
//! the workspace manifests agree. The lane-table tests are the gate itself: if the union of lane
//! features stops matching the declared features, a feature-gated test silently stops running.

use super::{
    COVERAGE_REPORT_SCHEMA_VERSION, FEATURE_LANES, FeatureLane, cfg_feature_names,
    declared_features, lanes_enabling, relative_display_path,
};
use std::collections::BTreeSet;
use std::path::Path;

/// The manifests the lane table must agree with, read at compile time so the test cannot drift
/// from the tree it claims to check.
const MOTH_MANIFEST: &str = include_str!("../../../Cargo.toml");
const XTASK_MANIFEST: &str = include_str!("../../Cargo.toml");

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
        owns: "collector-backed counters",
    };

    assert_eq!(
        lane.command_line(),
        "cargo test -p moth --quiet --features timers,benchmark_counters -- --format terse"
    );
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
fn relative_paths_use_forward_slashes_under_the_workspace_root() {
    let root = Path::new("/work/moth");
    let path = root.join("src").join("timing").join("mod.rs");

    assert_eq!(
        relative_display_path(root, &path).expect("an ASCII path is valid UTF-8"),
        "src/timing/mod.rs"
    );
}

#[test]
fn the_coverage_schema_version_is_the_one_consumers_are_told_to_expect() {
    assert_eq!(COVERAGE_REPORT_SCHEMA_VERSION, 1);
}
