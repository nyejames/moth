//! Tests for hotspot extraction from parsed profile data.
//!
//! WHAT: Validates filter-mode thresholds, percentage calculations,
//! millisecond estimation, Moth vs non-Moth prioritization,
//! and edge population.
//!
//! WHY: Hotspot extraction is the bridge between raw parser output and
//! agent-readable summaries. Correct filtering ensures agents see the
//! most relevant functions without noise.

use super::*;
use crate::profile::parse::{ParsedProfileSummary, ProfileFunctionSamples, parse_profile_json};
use std::path::Path;

/// Path to the minimal test fixture profile.
fn fixture_path() -> &'static Path {
    Path::new("test_fixtures/profiles/minimal_firefox_profile.json")
}

/// Parse the fixture and return the summary.
fn parse_fixture() -> ParsedProfileSummary {
    let json = std::fs::read_to_string(fixture_path()).expect("fixture should be readable");
    parse_profile_json(&json, fixture_path()).expect("fixture should parse")
}

// ----------------------------
//  Percentage calculations
// ----------------------------

#[test]
fn inclusive_pct_is_relative_to_total_weight() {
    let summary = parse_fixture();
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, 1000.0);

    // main has inclusive = 7, total weight = 7 -> 100%
    let main = result
        .functions
        .iter()
        .find(|f| f.name == "main")
        .expect("main should appear in deep mode");
    assert!((main.inclusive_pct - 100.0).abs() < 0.1);
}

#[test]
fn self_pct_is_relative_to_total_weight() {
    let summary = parse_fixture();
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, 1000.0);

    // alloc has self = 3 (main thread leaf only), total weight = 7 -> 42.86%
    let alloc = result
        .functions
        .iter()
        .find(|f| f.name == "std::alloc::alloc")
        .expect("alloc should appear");
    assert!((alloc.self_pct - (3.0 / 7.0 * 100.0)).abs() < 0.1);
}

#[test]
fn zero_total_weight_yields_zero_percentages() {
    let json = r#"{
        "threads": [{
            "name": "Main",
            "isMainThread": true,
            "stringArray": ["main"],
            "funcTable": {"name": [0]},
            "frameTable": {"func": [0]},
            "stackTable": {"frame": [0], "prefix": [-1]},
            "samples": {"stack": []}
        }]
    }"#;

    let summary = parse_profile_json(json, Path::new("test")).expect("should parse");
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, 1000.0);
    // No samples -> no functions extracted.
    assert!(result.functions.is_empty());
}

// ----------------------------
//  Millisecond estimation
// ----------------------------

#[test]
fn estimated_ms_uses_observation_wall_time() {
    let summary = parse_fixture();
    let wall_time = 1000.0;
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, wall_time);

    // main: inclusive_pct = 100%, so estimated_inclusive_ms = 1000.0
    let main = result
        .functions
        .iter()
        .find(|f| f.name == "main")
        .expect("main should appear");
    let expected_ms = wall_time * main.inclusive_pct / 100.0;
    assert!((main.estimated_inclusive_ms - expected_ms).abs() < 0.01);
}

#[test]
fn estimated_self_ms_uses_wall_time() {
    let summary = parse_fixture();
    let wall_time = 2000.0;
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, wall_time);

    // alloc: self_pct = 3/7 * 100 ~42.86%, so estimated_self_ms ~857.1
    let alloc = result
        .functions
        .iter()
        .find(|f| f.name == "std::alloc::alloc")
        .expect("alloc should appear");
    let expected_ms = wall_time * alloc.self_pct / 100.0;
    assert!((alloc.estimated_self_ms - expected_ms).abs() < 0.01);
}

// ----------------------------
//  Filter mode thresholds
// ----------------------------

#[test]
fn terse_mode_limits_to_8_functions() {
    let summary = parse_fixture();
    let result = extract_hotspots(&summary, ProfileFilterMode::Terse, 1000.0);
    assert!(result.functions.len() <= 8);
}

#[test]
fn normal_mode_limits_to_20_functions() {
    let summary = parse_fixture();
    let result = extract_hotspots(&summary, ProfileFilterMode::Normal, 1000.0);
    assert!(result.functions.len() <= 20);
}

#[test]
fn deep_mode_limits_to_50_functions() {
    let summary = parse_fixture();
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, 1000.0);
    assert!(result.functions.len() <= 50);
}

#[test]
fn terse_mode_filters_by_minimum_inclusive_pct() {
    // Terse minimum inclusive = 2.0%
    // With total weight 7, functions need inclusive >= 0.14 to pass.
    // resolve_type inclusive = 2 -> 28.6% (passes)
    let summary = parse_fixture();
    let result = extract_hotspots(&summary, ProfileFilterMode::Terse, 1000.0);

    for func in &result.functions {
        // Each function must meet either inclusive or self threshold.
        let is_moth = func.bucket.label != "unknown"
            && func.bucket.label != "other"
            && func.bucket.label != "std"
            && func.bucket.label != "core"
            && func.bucket.label != "alloc"
            && func.bucket.label != "rayon"
            && func.bucket.label != "samply/profiler";

        let passes = func.inclusive_pct >= 2.0
            || (is_moth && func.self_pct >= 1.0)
            || (!is_moth && func.self_pct >= NON_MOTH_MIN_SELF_PCT);

        assert!(
            passes,
            "Function '{}' with inclusive={:.1}% self={:.1}% bucket={} should not appear in terse mode",
            func.name, func.inclusive_pct, func.self_pct, func.bucket.label
        );
    }
}

#[test]
fn deep_mode_includes_edges() {
    let summary = parse_fixture();
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, 1000.0);

    // At least some functions should have callers/callees populated.
    let has_edges = result
        .functions
        .iter()
        .any(|f| !f.top_callers.is_empty() || !f.top_callees.is_empty());
    assert!(has_edges, "Deep mode should populate caller/callee edges");
}

#[test]
fn terse_mode_excludes_edges() {
    let summary = parse_fixture();
    let result = extract_hotspots(&summary, ProfileFilterMode::Terse, 1000.0);

    for func in &result.functions {
        assert!(
            func.top_callers.is_empty(),
            "Terse mode should not populate caller edges"
        );
        assert!(
            func.top_callees.is_empty(),
            "Terse mode should not populate callee edges"
        );
    }
}

#[test]
fn raw_address_functions_mark_symbolication_failed() {
    let summary = ParsedProfileSummary {
        total_sample_count: 200,
        total_sample_weight: 200.0,
        functions: vec![
            ProfileFunctionSamples {
                name: "0x1234abcd".to_string(),
                inclusive_samples: 120.0,
                self_samples: 60.0,
                thread_names: vec!["Main".to_string()],
                callers: Vec::new(),
                callees: Vec::new(),
            },
            ProfileFunctionSamples {
                name: "0x5678abcd".to_string(),
                inclusive_samples: 80.0,
                self_samples: 40.0,
                thread_names: vec!["Main".to_string()],
                callers: Vec::new(),
                callees: Vec::new(),
            },
        ],
        warnings: Vec::new(),
    };

    let result = extract_hotspots(&summary, ProfileFilterMode::Terse, 100.0);

    assert_eq!(
        result.symbolication.status,
        SymbolicationStatus::AddressOnly
    );
    assert!(result.symbolication.is_failed());
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("Symbolication failed"))
    );
}

// ----------------------------
//  Moth vs non-Moth prioritization
// ----------------------------

#[test]
fn non_moth_function_filtered_by_higher_self_threshold() {
    // Create a profile where a non-Moth function has low self time.
    // Using per-thread tables (Samply 0.13.1 format).
    let json = r#"{
        "threads": [{
            "name": "Main",
            "isMainThread": true,
            "stringArray": ["main", "std::mem::drop"],
            "funcTable": {"name": [0, 1]},
            "frameTable": {"func": [0, 1]},
            "stackTable": {
                "frame": [0, 1],
                "prefix": [-1, 0]
            },
            "samples": {
                "weightType": "samples",
                "stack": [0, 1, 1],
                "weight": [50, 2, 2]
            }
        }]
    }"#;

    let summary = parse_profile_json(json, Path::new("test")).expect("should parse");
    // std::mem::drop: inclusive = 4/54 ~7.4%, self = 4/54 ~7.4%
    // This exceeds the non-Moth threshold of 5%.
    let result = extract_hotspots(&summary, ProfileFilterMode::Normal, 1000.0);
    let drop_func = result.functions.iter().find(|f| f.name == "std::mem::drop");
    // It should appear because self_pct > 5%.
    assert!(
        drop_func.is_some(),
        "std::mem::drop with >5% self should appear"
    );
}

#[test]
fn low_self_non_moth_function_is_filtered() {
    // Create a profile where a non-Moth function has low self time.
    // Using per-thread tables (Samply 0.13.1 format).
    let json = r#"{
        "threads": [{
            "name": "Main",
            "isMainThread": true,
            "stringArray": ["moth::main", "std::mem::drop"],
            "funcTable": {"name": [0, 1]},
            "frameTable": {"func": [0, 1]},
            "stackTable": {
                "frame": [0, 1],
                "prefix": [-1, 0]
            },
            "samples": {
                "weightType": "samples",
                "stack": [0, 1],
                "weight": [99, 1]
            }
        }]
    }"#;

    let summary = parse_profile_json(json, Path::new("test")).expect("should parse");
    // With weighted accounting:
    //   std::mem::drop: inclusive = 1, self = 1, total_weight = 100
    //   inclusive_pct = 1.0%, self_pct = 1.0%
    //   1.0% < terse minimum (2.0%) for inclusive
    //   1.0% < NON_MOTH_MIN_SELF_PCT (5.0%) for non-Moth self path
    // -> should be filtered out
    let result = extract_hotspots(&summary, ProfileFilterMode::Terse, 1000.0);
    let drop_func = result.functions.iter().find(|f| f.name == "std::mem::drop");
    assert!(
        drop_func.is_none(),
        "std::mem::drop with inclusive=1% self=1% should be filtered in terse mode"
    );
}

// ----------------------------
//  Warnings
// ----------------------------

#[test]
fn low_sample_count_generates_warning() {
    let summary = parse_fixture();
    // Fixture has 4 samples, well below the 100 threshold.
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, 1000.0);
    let has_low_sample_warning = result
        .warnings
        .iter()
        .any(|w| w.contains("below") && w.contains("unreliable"));
    assert!(has_low_sample_warning, "Should warn about low sample count");
}

#[test]
fn parser_warnings_are_preserved() {
    let summary = parse_fixture();
    let original_warning_count = summary.warnings.len();
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, 1000.0);
    // At least the original warnings should be present.
    assert!(result.warnings.len() >= original_warning_count);
}

// ----------------------------
//  Summary statistics
// ----------------------------

#[test]
fn result_preserves_total_sample_count() {
    let summary = parse_fixture();
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, 1000.0);
    assert_eq!(result.total_sample_count, summary.total_sample_count);
}

#[test]
fn result_preserves_total_sample_weight() {
    let summary = parse_fixture();
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, 1000.0);
    assert!((result.total_sample_weight - summary.total_sample_weight).abs() < 1e-9);
}

#[test]
fn result_preserves_wall_time() {
    let summary = parse_fixture();
    let wall_time = 1234.5;
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, wall_time);
    assert!((result.wall_time_ms - wall_time).abs() < 0.01);
}

// ----------------------------
//  Owner bucket mapping
// ----------------------------

#[test]
fn moth_functions_have_correct_bucket_labels() {
    let summary = parse_fixture();
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, 1000.0);

    let resolve_type = result
        .functions
        .iter()
        .find(|f| f.name == "moth::compiler_frontend::ast::resolve_type");
    if let Some(f) = resolve_type {
        assert_eq!(f.bucket.label, "AST");
        assert_eq!(f.bucket.suggested_paths, vec!["src/compiler_frontend/ast/"]);
    }

    let generate = result
        .functions
        .iter()
        .find(|f| f.name == "moth::compiler_frontend::hir::generate");
    if let Some(f) = generate {
        assert_eq!(f.bucket.label, "HIR");
    }
}

#[test]
fn non_moth_functions_have_correct_bucket_labels() {
    let summary = parse_fixture();
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, 1000.0);

    let alloc = result
        .functions
        .iter()
        .find(|f| f.name == "std::alloc::alloc");
    if let Some(f) = alloc {
        assert_eq!(f.bucket.label, "std");
    }
}

// ----------------------------
//  Edge population in deep mode
// ----------------------------

#[test]
fn deep_mode_populates_top_callers() {
    let summary = parse_fixture();
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, 1000.0);

    // resolve_type should have main as a caller.
    let resolve_type = result
        .functions
        .iter()
        .find(|f| f.name == "moth::compiler_frontend::ast::resolve_type");
    if let Some(f) = resolve_type {
        let main_caller = f.top_callers.iter().find(|e| e.function_name == "main");
        assert!(
            main_caller.is_some(),
            "resolve_type should have main as a top caller"
        );
    }
}

#[test]
fn deep_mode_populates_top_callees() {
    let summary = parse_fixture();
    let result = extract_hotspots(&summary, ProfileFilterMode::Deep, 1000.0);

    // main should have resolve_type as a callee.
    let main = result.functions.iter().find(|f| f.name == "main");
    if let Some(f) = main {
        let resolve_type_callee = f
            .top_callees
            .iter()
            .find(|e| e.function_name == "moth::compiler_frontend::ast::resolve_type");
        assert!(
            resolve_type_callee.is_some(),
            "main should have resolve_type as a top callee"
        );
    }
}
