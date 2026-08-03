//! Tests for summary generation, hint logic, and root artifact formatting.

use super::*;
use crate::bench_types::{BenchmarkCaseObservations, BenchmarkMetric};
use crate::profile::buckets::ProfileOwnerBucketMatch;
use crate::profile::hotspots::{
    HotspotExtractionResult, ProfileHotFunction, SymbolicationHealth, SymbolicationStatus,
    is_raw_address_function_name,
};
use crate::profile::observations::ProfileObservation;
use crate::profile::options::ProfileFilterMode;

// ---------------------------------------------------------------------------
//  Test helpers
// ---------------------------------------------------------------------------

fn make_observation(
    case_id: &str,
    wall_ms: f64,
    stage_timings: Vec<BenchmarkMetric>,
    counters: Vec<BenchmarkMetric>,
) -> ProfileObservation {
    ProfileObservation {
        case_id: case_id.to_string(),
        group_name: "test".to_string(),
        command: "check".to_string(),
        command_args: vec!["test.moth".to_string()],
        wall_ms,
        observations: BenchmarkCaseObservations {
            stage_timings,
            counters,
        },
        stdout: String::new(),
        stderr: String::new(),
    }
}

fn make_hotspots(
    functions: Vec<ProfileHotFunction>,
    warnings: Vec<String>,
) -> HotspotExtractionResult {
    let hot_function_count = functions.len();
    let raw_address_function_count = functions
        .iter()
        .filter(|function| is_raw_address_function_name(&function.name))
        .count();
    let raw_address_ratio = if hot_function_count == 0 {
        0.0
    } else {
        raw_address_function_count as f64 / hot_function_count as f64
    };
    let status = if hot_function_count == 0 {
        SymbolicationStatus::NoFunctions
    } else if raw_address_ratio >= 0.5 {
        SymbolicationStatus::AddressOnly
    } else {
        SymbolicationStatus::Healthy
    };

    HotspotExtractionResult {
        functions,
        warnings,
        total_sample_count: 1000,
        total_sample_weight: 1000.0,
        wall_time_ms: 100.0,
        symbolication: SymbolicationHealth {
            status,
            hot_function_count,
            raw_address_function_count,
            raw_address_ratio,
        },
    }
}

fn make_hot_function(
    name: &str,
    bucket_label: &str,
    inclusive_pct: f64,
    self_pct: f64,
) -> ProfileHotFunction {
    ProfileHotFunction {
        name: name.to_string(),
        bucket: ProfileOwnerBucketMatch {
            label: bucket_label.to_string(),
            suggested_paths: vec![format!(
                "src/{}/",
                bucket_label.to_lowercase().replace(' ', "_")
            )],
        },
        inclusive_samples: inclusive_pct * 10.0,
        self_samples: self_pct * 10.0,
        inclusive_pct,
        self_pct,
        estimated_inclusive_ms: inclusive_pct,
        estimated_self_ms: self_pct,
        top_callers: Vec::new(),
        top_callees: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
//  Hint generation tests
// ---------------------------------------------------------------------------

#[test]
fn hint_ast_stage_with_ast_bucket() {
    let obs = make_observation(
        "test_case",
        1000.0,
        vec![BenchmarkMetric {
            name: "ast_ms".to_string(),
            value: 800.0,
        }],
        vec![],
    );
    let hotspot_fn = make_hot_function(
        "moth::compiler_frontend::ast::type_resolution::resolve_type",
        "AST",
        31.4,
        6.8,
    );
    let hotspots = make_hotspots(vec![hotspot_fn], vec![]);
    let data = CaseSummaryData {
        observation: &obs,
        hotspots: &hotspots,
        profile_relative_path: "cases/test_case/profile.json.gz".to_string(),
        filter: ProfileFilterMode::Terse,
    };

    let hint = generate_hint(&data);
    assert!(hint.contains("AST"), "Hint should mention AST: {}", hint);
    assert!(
        hint.contains("type/environment") || hint.contains("inspect"),
        "Hint should suggest inspection: {}",
        hint
    );
}

#[test]
fn hint_file_prepare_stage_with_tokenization_bucket() {
    let obs = make_observation(
        "test_case",
        1000.0,
        vec![BenchmarkMetric {
            name: "file_prepare_ms".to_string(),
            value: 600.0,
        }],
        vec![],
    );
    let hotspot_fn = make_hot_function(
        "moth::compiler_frontend::tokenizer::tokenize_file",
        "Tokenization",
        45.0,
        12.0,
    );
    let hotspots = make_hotspots(vec![hotspot_fn], vec![]);
    let data = CaseSummaryData {
        observation: &obs,
        hotspots: &hotspots,
        profile_relative_path: "cases/test_case/profile.json.gz".to_string(),
        filter: ProfileFilterMode::Terse,
    };

    let hint = generate_hint(&data);
    assert!(
        hint.contains("File preparation") || hint.contains("Tokenization"),
        "Hint should mention file preparation or tokenization: {}",
        hint
    );
}

#[test]
fn hint_alloc_dominates_self_time() {
    let obs = make_observation("test_case", 1000.0, vec![], vec![]);
    let hotspot_fn = make_hot_function("alloc::vec::Vec::push", "alloc", 25.0, 15.0);
    let hotspots = make_hotspots(vec![hotspot_fn], vec![]);
    let data = CaseSummaryData {
        observation: &obs,
        hotspots: &hotspots,
        profile_relative_path: "cases/test_case/profile.json.gz".to_string(),
        filter: ProfileFilterMode::Terse,
    };

    let hint = generate_hint(&data);
    assert!(
        hint.contains("Allocation") || hint.contains("alloc"),
        "Hint should mention allocation: {}",
        hint
    );
}

#[test]
fn hint_rayon_dominates() {
    let obs = make_observation("test_case", 1000.0, vec![], vec![]);
    let hotspot_fn = make_hot_function("rayon::core::registry::Registry::new", "rayon", 40.0, 8.0);
    let hotspots = make_hotspots(vec![hotspot_fn], vec![]);
    let data = CaseSummaryData {
        observation: &obs,
        hotspots: &hotspots,
        profile_relative_path: "cases/test_case/profile.json.gz".to_string(),
        filter: ProfileFilterMode::Terse,
    };

    let hint = generate_hint(&data);
    assert!(
        hint.contains("Rayon") || hint.contains("rayon") || hint.contains("parallel"),
        "Hint should mention rayon or parallel: {}",
        hint
    );
}

#[test]
fn hint_unsymbolicated_functions() {
    let obs = make_observation("test_case", 1000.0, vec![], vec![]);
    let hotspot_fn = make_hot_function("0x7fff20304050", "unknown", 50.0, 50.0);
    let hotspots = make_hotspots(vec![hotspot_fn], vec![]);
    let data = CaseSummaryData {
        observation: &obs,
        hotspots: &hotspots,
        profile_relative_path: "cases/test_case/profile.json.gz".to_string(),
        filter: ProfileFilterMode::Terse,
    };

    let hint = generate_hint(&data);
    assert!(
        hint.contains("symbolicated")
            || hint.contains("presymbolicate")
            || hint.contains("raw addresses"),
        "Hint should mention symbolication: {}",
        hint
    );
}

#[test]
fn hint_no_hot_functions() {
    let obs = make_observation("test_case", 1000.0, vec![], vec![]);
    let hotspots = make_hotspots(vec![], vec![]);
    let data = CaseSummaryData {
        observation: &obs,
        hotspots: &hotspots,
        profile_relative_path: "cases/test_case/profile.json.gz".to_string(),
        filter: ProfileFilterMode::Terse,
    };

    let hint = generate_hint(&data);
    assert!(
        hint.contains("threshold") || hint.contains("filter"),
        "Hint should suggest filter change: {}",
        hint
    );
}

#[test]
fn hint_non_moth_dominant() {
    let obs = make_observation("test_case", 1000.0, vec![], vec![]);
    let hotspot_fn = make_hot_function("std::collections::HashMap::insert", "std", 60.0, 30.0);
    let hotspots = make_hotspots(vec![hotspot_fn], vec![]);
    let data = CaseSummaryData {
        observation: &obs,
        hotspots: &hotspots,
        profile_relative_path: "cases/test_case/profile.json.gz".to_string(),
        filter: ProfileFilterMode::Terse,
    };

    let hint = generate_hint(&data);
    assert!(
        hint.contains("non-Moth") || hint.contains("caller edges"),
        "Hint should mention non-Moth functions: {}",
        hint
    );
}

// ---------------------------------------------------------------------------
//  Unsymbolicated detection tests
// ---------------------------------------------------------------------------

#[test]
fn unsymbolicated_hex_address() {
    assert!(is_raw_address_function_name("0x7fff20304050"));
    assert!(is_raw_address_function_name("0x10abcdef"));
    assert!(is_raw_address_function_name("0X1234"));
}

#[test]
fn unsymbolicated_normal_names() {
    assert!(!is_raw_address_function_name(
        "moth::compiler_frontend::ast::build"
    ));
    assert!(!is_raw_address_function_name(
        "std::collections::HashMap::insert"
    ));
    assert!(!is_raw_address_function_name("unknown"));
    assert!(!is_raw_address_function_name(""));
}

// ---------------------------------------------------------------------------
//  Signal scoring tests
// ---------------------------------------------------------------------------

#[test]
fn signal_score_includes_wall_time() {
    let obs_slow = make_observation("slow", 2000.0, vec![], vec![]);
    let obs_fast = make_observation("fast", 100.0, vec![], vec![]);
    let hotspots = make_hotspots(vec![], vec![]);
    let data_slow = CaseSummaryData {
        observation: &obs_slow,
        hotspots: &hotspots,
        profile_relative_path: "cases/slow/profile.json.gz".to_string(),
        filter: ProfileFilterMode::Terse,
    };
    let data_fast = CaseSummaryData {
        observation: &obs_fast,
        hotspots: &hotspots,
        profile_relative_path: "cases/fast/profile.json.gz".to_string(),
        filter: ProfileFilterMode::Terse,
    };

    assert!(combined_signal_score(&data_slow) > combined_signal_score(&data_fast));
}

#[test]
fn signal_score_includes_hotspot_pct() {
    let obs = make_observation("case", 1000.0, vec![], vec![]);
    let hotspots_with = make_hotspots(
        vec![make_hot_function(
            "moth::compiler_frontend::ast::build",
            "AST",
            40.0,
            10.0,
        )],
        vec![],
    );
    let hotspots_without = make_hotspots(vec![], vec![]);
    let data_with = CaseSummaryData {
        observation: &obs,
        hotspots: &hotspots_with,
        profile_relative_path: "cases/case/profile.json.gz".to_string(),
        filter: ProfileFilterMode::Terse,
    };
    let data_without = CaseSummaryData {
        observation: &obs,
        hotspots: &hotspots_without,
        profile_relative_path: "cases/case/profile.json.gz".to_string(),
        filter: ProfileFilterMode::Terse,
    };

    assert!(combined_signal_score(&data_with) > combined_signal_score(&data_without));
}

// ---------------------------------------------------------------------------
//  Root hotspots JSON tests
// ---------------------------------------------------------------------------

#[test]
fn root_hotspots_json_is_valid() {
    let obs = make_observation(
        "test_case",
        1200.0,
        vec![BenchmarkMetric {
            name: "ast_ms".to_string(),
            value: 800.0,
        }],
        vec![BenchmarkMetric {
            name: "token_count".to_string(),
            value: 12000.0,
        }],
    );
    let hotspot_fn = make_hot_function(
        "moth::compiler_frontend::ast::type_resolution::resolve_type",
        "AST",
        31.4,
        6.8,
    );
    let hotspots = make_hotspots(vec![hotspot_fn], vec!["Low sample count.".to_string()]);
    let data = CaseSummaryData {
        observation: &obs,
        hotspots: &hotspots,
        profile_relative_path: "cases/test_case/profile.json.gz".to_string(),
        filter: ProfileFilterMode::Terse,
    };

    let root = build_root_hotspots(
        &[data],
        "2026-06-18T10-30-abc123",
        Some("abc1234"),
        ProfileFilterMode::Terse,
        None,
    );
    let json = format_root_hotspots_json(&root).expect("root hotspots should serialize");

    // Must be valid JSON.
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("JSON should parse");
    assert!(parsed.is_object());
    assert_eq!(parsed["format_version"], SUMMARY_FORMAT_VERSION);
    assert_eq!(parsed["run_id"], "2026-06-18T10-30-abc123");
    assert_eq!(parsed["case_count"], 1);
    assert_eq!(parsed["filter"], "terse");

    // Check case data.
    let cases = parsed["cases"].as_array().expect("cases should be array");
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0]["case_id"], "test_case");
    assert!(cases[0].get("case_name").is_none());
    assert_eq!(cases[0]["observation_wall_ms"], 1200.0);
    assert!(!cases[0]["hot_functions"].as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
//  Agent summary markdown tests
// ---------------------------------------------------------------------------

#[test]
fn agent_summary_contains_case_id() {
    let obs = make_observation(
        "check_benchmarks_test_bst",
        500.0,
        vec![BenchmarkMetric {
            name: "ast_ms".to_string(),
            value: 300.0,
        }],
        vec![],
    );
    let hotspot_fn = make_hot_function("moth::compiler_frontend::ast::build", "AST", 20.0, 5.0);
    let hotspots = make_hotspots(vec![hotspot_fn], vec![]);
    let data = CaseSummaryData {
        observation: &obs,
        hotspots: &hotspots,
        profile_relative_path: "cases/check_benchmarks_test_bst/profile.json.gz".to_string(),
        filter: ProfileFilterMode::Terse,
    };

    let md = format_agent_summary_md(&[data], "test-run-id", ProfileFilterMode::Terse);
    assert!(md.contains("check_benchmarks_test_bst"));
    assert!(md.contains("Profiling agent summary"));
    assert!(md.contains("Strongest signals"));
    assert!(md.contains("~500ms"));
}

#[test]
fn agent_summary_respects_case_limit() {
    let mut cases = Vec::new();
    for i in 0..10 {
        let obs = make_observation(
            &format!("case_{}", i),
            (i as f64 + 1.0) * 100.0,
            vec![],
            vec![],
        );
        let hotspots = make_hotspots(vec![], vec![]);
        cases.push((obs, hotspots));
    }

    let data_refs: Vec<CaseSummaryData<'_>> = cases
        .iter()
        .map(|(obs, hotspots)| CaseSummaryData {
            observation: obs,
            hotspots,
            profile_relative_path: format!("cases/{}/profile.json.gz", obs.case_id),
            filter: ProfileFilterMode::Terse,
        })
        .collect();

    let md = format_agent_summary_md(&data_refs, "test-run", ProfileFilterMode::Terse);

    // Terse limit is 3 cases.
    assert!(md.contains("case_9")); // Highest score
    assert!(md.contains("case_8"));
    assert!(md.contains("case_7"));
    // The others should be omitted.
    assert!(!md.contains("case_0"));
    assert!(md.contains("additional cases omitted"));
}

// ---------------------------------------------------------------------------
//  Per-case summary tests
// ---------------------------------------------------------------------------

#[test]
fn enriched_case_summary_includes_hotspots_and_samply_command() {
    let obs = make_observation(
        "test_case",
        1200.0,
        vec![BenchmarkMetric {
            name: "ast_ms".to_string(),
            value: 800.0,
        }],
        vec![BenchmarkMetric {
            name: "token_count".to_string(),
            value: 12000.0,
        }],
    );
    let hotspot_fn = make_hot_function(
        "moth::compiler_frontend::ast::type_resolution::resolve_type",
        "AST",
        31.4,
        6.8,
    );
    let hotspots = make_hotspots(vec![hotspot_fn], vec!["Low sample count.".to_string()]);
    let data = CaseSummaryData {
        observation: &obs,
        hotspots: &hotspots,
        profile_relative_path: "cases/test_case/profile.json.gz".to_string(),
        filter: ProfileFilterMode::Terse,
    };

    // Create a temporary run paths for the test.
    let tmp = tempfile::tempdir().expect("temp dir");
    let run_paths = ProfileRunPaths {
        run_id: "test-run".to_string(),
        root: tmp.path().to_path_buf(),
    };
    // Create the case directory.
    let case_paths = run_paths.case_paths("test_case");
    case_paths.create_dir().expect("create case dir");

    let md = format_enriched_case_summary(&data, &run_paths);
    assert!(md.contains("test_case"));
    assert!(md.contains("Sample count: 1000"));
    assert!(md.contains("Hot functions"));
    assert!(md.contains("resolve_type"));
    assert!(md.contains("AST"));
    assert!(md.contains("Warnings"));
    assert!(md.contains("Low sample count"));
    assert!(md.contains("samply load"));
    assert!(md.contains("## Hint"));
}

// ---------------------------------------------------------------------------
//  Bucket summary tests
// ---------------------------------------------------------------------------

#[test]
fn bucket_summary_aggregates_by_label() {
    let functions = vec![
        make_hot_function("moth::compiler_frontend::ast::build", "AST", 20.0, 5.0),
        make_hot_function("moth::compiler_frontend::ast::resolve", "AST", 15.0, 3.0),
        make_hot_function(
            "moth::compiler_frontend::tokenizer::tokenize",
            "Tokenization",
            10.0,
            8.0,
        ),
    ];

    let summary = build_bucket_summary(&functions);

    assert_eq!(summary.len(), 2);
    assert_eq!(summary[0].label, "AST");
    assert_eq!(summary[0].function_count, 2);
    assert!((summary[0].total_inclusive_pct - 35.0).abs() < 0.1);
    assert_eq!(summary[1].label, "Tokenization");
    assert_eq!(summary[1].function_count, 1);
}

// ---------------------------------------------------------------------------
//  Truncation tests
// ---------------------------------------------------------------------------

#[test]
fn truncate_short_name() {
    assert_eq!(truncate_function_name("short", 80), "short");
}

#[test]
fn truncate_long_name() {
    let long = "a".repeat(100);
    let result = truncate_function_name(&long, 20);
    assert!(result.len() <= 20);
    assert!(result.ends_with("..."));
}
