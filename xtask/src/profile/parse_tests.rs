//! Tests for profile JSON parsing and sample accounting.
//!
//! WHAT: Validates stack walking, string/function resolution, inclusive vs
//! self counts, recursion de-duplication, edge counting, weight handling,
//! and malformed index error handling using the minimal fixture profile.
//!
//! WHY: The parser must handle Firefox processed-profile JSON correctly
//! and defensively. These tests ensure the per-thread table format
//! (Samply 0.13.1) is parsed without panics and produces correct
//! sample accounting.

use super::*;

/// Path to the minimal test fixture profile.
fn fixture_path() -> &'static Path {
    Path::new("test_fixtures/profiles/minimal_firefox_profile.json")
}

/// Parse the fixture and return the summary.
///
/// The fixture has per-thread tables (Samply 0.13.1 format):
/// - Main thread: 5 functions, 3 weighted samples (3, 1, 2)
/// - Worker thread: 3 functions, 2 samples (1 valid, 1 null)
///
/// Expected totals:
/// - total_sample_weight = 3 + 1 + 2 + 1 = 7
/// - total_sample_count = 4 (3 main + 1 worker non-null)
fn parse_fixture() -> ParsedProfileSummary {
    let json = std::fs::read_to_string(fixture_path()).expect("fixture should be readable");
    parse_profile_json(&json, fixture_path()).expect("fixture should parse successfully")
}

/// Find a function by name in the parsed summary.
fn find_function<'a>(summary: &'a ParsedProfileSummary, name: &str) -> &'a ProfileFunctionSamples {
    summary
        .functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("Function '{}' not found in parsed profile", name))
}

// ----------------------------
//  Basic parsing
// ----------------------------

#[test]
fn fixture_parses_without_errors() {
    let summary = parse_fixture();
    // Should have 5 functions (merged across threads by name).
    assert_eq!(summary.functions.len(), 5);
}

#[test]
fn total_sample_count_includes_all_non_null_samples() {
    let summary = parse_fixture();
    // 3 main thread + 1 worker non-null = 4 samples.
    assert_eq!(summary.total_sample_count, 4);
}

#[test]
fn total_sample_weight_sums_all_weights() {
    let summary = parse_fixture();
    // Main: 3 + 1 + 2 = 6; Worker: 1 (default) = 7 total.
    assert!((summary.total_sample_weight - 7.0).abs() < 1e-9);
}

// ----------------------------
//  String/function resolution
// ----------------------------

#[test]
fn main_function_resolves_correctly() {
    let summary = parse_fixture();
    let main = find_function(&summary, "main");
    assert!(!main.name.is_empty());
}

#[test]
fn moth_function_names_resolve_through_tables() {
    let summary = parse_fixture();
    let resolve_type = find_function(&summary, "moth::compiler_frontend::ast::resolve_type");
    assert!(!resolve_type.name.is_empty());
}

#[test]
fn profile_shape_dump_reports_top_level_and_first_thread_shape() {
    let json = r#"{
        "meta": {"product": "samply", "version": "0.13.1"},
        "libs": [
            {"debugName": "moth"},
            {"name": "libsystem_kernel.dylib"}
        ],
        "resourceTable": {"lib": [0], "name": [1]},
        "nativeSymbols": {"0": []},
        "threads": [{
            "name": "main",
            "stringArray": ["0x1000", "moth::compiler_frontend::ast::build"],
            "funcTable": {"name": [0, 1], "resource": [0, 0]},
            "frameTable": {"func": [0, 1]},
            "stackTable": {"prefix": [null, 0], "frame": [0, 1]},
            "samples": {"stack": [1]}
        }]
    }"#;

    let dump = profile_shape_dump_from_json(json, Path::new("test")).expect("shape dump");

    assert_eq!(dump.meta_product, "samply");
    assert_eq!(dump.meta_version, "0.13.1");
    assert_eq!(dump.thread_count, 1);
    assert_eq!(
        dump.first_thread_func_table_keys,
        vec!["name".to_string(), "resource".to_string()]
    );
    assert_eq!(
        dump.first_20_func_names,
        vec![
            "0x1000".to_string(),
            "moth::compiler_frontend::ast::build".to_string()
        ]
    );
    assert_eq!(
        dump.resource_table_keys,
        vec!["lib".to_string(), "name".to_string()]
    );
    assert_eq!(dump.libs_count, Some(2));
    assert_eq!(
        dump.first_10_libs,
        vec!["moth".to_string(), "libsystem_kernel.dylib".to_string()]
    );
    assert!(dump.native_symbols_present);
}

#[test]
fn std_function_name_resolves() {
    let summary = parse_fixture();
    let alloc = find_function(&summary, "std::alloc::alloc");
    assert!(!alloc.name.is_empty());
}

// ----------------------------
//  Self samples
// ----------------------------

// Fixture trace (per-thread stack tables):
//
// Main thread:
//   Stack 0: frame 0 → func 0 → "main"              (prefix -1, root)
//   Stack 1: frame 1 → func 1 → "resolve_type"       (prefix 0 → main)
//   Stack 2: frame 3 → func 3 → "alloc"              (prefix 1 → resolve_type)
//   Stack 3: frame 2 → func 2 → "generate"            (prefix 0 → main)
//   Stack 4: frame 3 → func 3 → "alloc"              (prefix 3 → generate)
//   Stack 5: frame 4 → func 4 → "rayon::spawn"        (prefix 0 → main)
//   Samples: [5, 2, 4], weights: [3, 1, 2]
//
// Worker thread:
//   Stack 0: frame 0 → func 0 → "main"              (prefix -1, root)
//   Stack 1: frame 2 → func 2 → "alloc"              (prefix 0 → main)
//   Stack 2: frame 1 → func 1 → "resolve_type"       (prefix 1 → alloc)
//   Samples: [2, null], no weights (default 1.0)
//
// Sample 1 (main stack 5, w=3): main → rayon
// Sample 2 (main stack 2, w=1): main → resolve_type → alloc
// Sample 3 (main stack 4, w=2): main → generate → alloc
// Sample 4 (worker stack 2, w=1): main → alloc → resolve_type

#[test]
fn self_samples_count_only_leaf_functions() {
    let summary = parse_fixture();

    // rayon is leaf of main sample 1 (weight 3) -> self = 3
    let rayon = find_function(&summary, "rayon::ThreadPool::spawn");
    assert!((rayon.self_samples - 3.0).abs() < 1e-9);

    // alloc is leaf of main samples 2, 3 (weights 1+2) -> self = 3
    // alloc is NOT the leaf of worker sample (resolve_type is leaf there)
    let alloc = find_function(&summary, "std::alloc::alloc");
    assert!((alloc.self_samples - 3.0).abs() < 1e-9);

    // main is never the leaf in any sample -> self = 0
    let main = find_function(&summary, "main");
    assert!((main.self_samples - 0.0).abs() < 1e-9);

    // resolve_type is leaf of worker sample (weight 1) -> self = 1
    let resolve_type = find_function(&summary, "moth::compiler_frontend::ast::resolve_type");
    assert!((resolve_type.self_samples - 1.0).abs() < 1e-9);

    // generate is never the leaf -> self = 0
    let generate = find_function(&summary, "moth::compiler_frontend::hir::generate");
    assert!((generate.self_samples - 0.0).abs() < 1e-9);
}

#[test]
fn worker_thread_sample_contributes_to_self() {
    let summary = parse_fixture();
    // Worker sample (stack 2, weight 1): main → alloc → resolve_type
    // resolve_type is leaf -> self += 1 for resolve_type
    // alloc is NOT leaf in worker sample
    let resolve_type = find_function(&summary, "moth::compiler_frontend::ast::resolve_type");
    assert!((resolve_type.self_samples - 1.0).abs() < 1e-9);
}

// ----------------------------
//  Inclusive samples
// ----------------------------

#[test]
fn inclusive_samples_count_function_once_per_sample() {
    let summary = parse_fixture();

    // main appears in all 4 non-null samples: inclusive = 3 + 1 + 2 + 1 = 7
    let main = find_function(&summary, "main");
    assert!((main.inclusive_samples - 7.0).abs() < 1e-9);
}

#[test]
fn inclusive_samples_include_all_ancestors() {
    let summary = parse_fixture();

    // resolve_type appears in:
    //   main sample 2 (w=1): main → resolve_type → alloc (inclusive)
    //   worker sample (w=1): main → alloc → resolve_type (leaf, inclusive)
    // -> inclusive = 1 + 1 = 2
    let resolve_type = find_function(&summary, "moth::compiler_frontend::ast::resolve_type");
    assert!((resolve_type.inclusive_samples - 2.0).abs() < 1e-9);
}

#[test]
fn inclusive_samples_for_leaf_function() {
    let summary = parse_fixture();

    // generate appears only in main sample 3 (weight 2) -> inclusive = 2
    let generate = find_function(&summary, "moth::compiler_frontend::hir::generate");
    assert!((generate.inclusive_samples - 2.0).abs() < 1e-9);
}

#[test]
fn rayon_function_has_inclusive_samples() {
    let summary = parse_fixture();

    // rayon is in main sample 1 (weight 3) -> inclusive = 3, self = 3 (leaf)
    let rayon = find_function(&summary, "rayon::ThreadPool::spawn");
    assert!((rayon.inclusive_samples - 3.0).abs() < 1e-9);
    assert!((rayon.self_samples - 3.0).abs() < 1e-9);
}

// ----------------------------
//  Recursion de-duplication
// ----------------------------

#[test]
fn recursive_stack_does_not_double_count_inclusive() {
    // Build a profile with a recursive stack: main -> alloc -> alloc
    // Using per-thread tables (Samply 0.13.1 format).
    let json = r#"{
        "threads": [{
            "name": "Main",
            "isMainThread": true,
            "stringArray": ["main", "alloc"],
            "funcTable": {"name": [0, 1]},
            "frameTable": {"func": [0, 1]},
            "stackTable": {
                "frame": [0, 1, 1],
                "prefix": [-1, 0, 1]
            },
            "samples": {"stack": [2]}
        }]
    }"#;

    let summary = parse_profile_json(json, Path::new("test")).expect("should parse");

    // alloc appears twice in the stack (frames 1 and 2) but should only
    // be counted once for inclusive samples.
    let alloc = find_function(&summary, "alloc");
    assert!(
        (alloc.inclusive_samples - 1.0).abs() < 1e-9,
        "alloc inclusive should be 1 (not 2), got {}",
        alloc.inclusive_samples
    );
    // alloc is the leaf (frame 2) -> self = 1
    assert!((alloc.self_samples - 1.0).abs() < 1e-9);
}

// ----------------------------
//  Edge counting
// ----------------------------

#[test]
fn caller_edges_are_recorded() {
    let summary = parse_fixture();

    // resolve_type's callers:
    //   - main (from main sample 2, main→resolve_type→alloc, weight=1)
    //   - alloc (from worker sample, main→alloc→resolve_type, weight=1)
    let resolve_type = find_function(&summary, "moth::compiler_frontend::ast::resolve_type");
    assert_eq!(resolve_type.callers.len(), 2);

    let main_caller = resolve_type
        .callers
        .iter()
        .find(|e| e.function_name == "main")
        .expect("resolve_type should have main as a caller");
    assert!((main_caller.samples - 1.0).abs() < 1e-9);

    let alloc_caller = resolve_type
        .callers
        .iter()
        .find(|e| e.function_name == "std::alloc::alloc")
        .expect("resolve_type should have alloc as a caller");
    assert!((alloc_caller.samples - 1.0).abs() < 1e-9);
}

#[test]
fn callee_edges_are_recorded() {
    let summary = parse_fixture();

    // main's callees: resolve_type (1 edge), generate (1 edge), rayon (1 edge), alloc (1 edge from worker)
    let main = find_function(&summary, "main");
    assert!(main.callees.len() >= 3);

    let resolve_type_callee = main
        .callees
        .iter()
        .find(|e| e.function_name == "moth::compiler_frontend::ast::resolve_type")
        .expect("main should have resolve_type as callee");
    // main→resolve_type appears in main sample 2 (weight=1).
    assert!((resolve_type_callee.samples - 1.0).abs() < 1e-9);
}

#[test]
fn edge_percentages_are_calculated() {
    let summary = parse_fixture();
    let main = find_function(&summary, "main");

    for edge in &main.callees {
        assert!(edge.pct >= 0.0 && edge.pct <= 100.0);
    }
}

// ----------------------------
//  Thread tracking
// ----------------------------

#[test]
fn thread_names_are_tracked_per_function() {
    let summary = parse_fixture();

    // main appears in both threads.
    let main = find_function(&summary, "main");
    assert!(main.thread_names.contains(&"Main Thread".to_string()));
    assert!(main.thread_names.contains(&"Worker".to_string()));
}

#[test]
fn worker_only_function_has_single_thread() {
    // alloc appears in both threads, so it should have both names.
    let summary = parse_fixture();
    let alloc = find_function(&summary, "std::alloc::alloc");
    assert!(alloc.thread_names.len() >= 2);
}

// ----------------------------
//  Null stack handling
// ----------------------------

#[test]
fn null_stacks_are_skipped() {
    let summary = parse_fixture();
    // The fixture has 5 stacks total (3 main + 2 worker) but one is null.
    // total_sample_count should be 4, not 5.
    assert_eq!(summary.total_sample_count, 4);
}

// ----------------------------
//  Malformed index errors
// ----------------------------

#[test]
fn out_of_range_stack_index_returns_error() {
    let json = r#"{
        "threads": [{
            "name": "Main",
            "isMainThread": true,
            "stringArray": ["main"],
            "funcTable": {"name": [0]},
            "frameTable": {"func": [0]},
            "stackTable": {
                "frame": [0],
                "prefix": [-1]
            },
            "samples": {"stack": [99]}
        }]
    }"#;

    let result = parse_profile_json(json, Path::new("test"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("out of bounds"));
}

#[test]
fn out_of_range_prefix_returns_error() {
    let json = r#"{
        "threads": [{
            "name": "Main",
            "isMainThread": true,
            "stringArray": ["main"],
            "funcTable": {"name": [0]},
            "frameTable": {"func": [0]},
            "stackTable": {
                "frame": [0],
                "prefix": [99]
            },
            "samples": {"stack": [0]}
        }]
    }"#;

    let result = parse_profile_json(json, Path::new("test"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("out-of-range prefix"));
}

#[test]
fn forward_prefix_returns_error() {
    // A prefix that points forward creates an invalid chain.
    let json = r#"{
        "threads": [{
            "name": "Main",
            "isMainThread": true,
            "stringArray": ["main"],
            "funcTable": {"name": [0]},
            "frameTable": {"func": [0]},
            "stackTable": {
                "frame": [0, 0],
                "prefix": [1, -1]
            },
            "samples": {"stack": [1]}
        }]
    }"#;

    let result = parse_profile_json(json, Path::new("test"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("forward prefix"));
}

#[test]
fn missing_threads_returns_error() {
    let json = r#"{}"#;
    let result = parse_profile_json(json, Path::new("test"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("missing required 'threads'"));
}

#[test]
fn missing_thread_tables_returns_error() {
    // A thread without stringArray should fail.
    let json = r#"{
        "threads": [{
            "name": "Main",
            "isMainThread": true,
            "samples": {"stack": [0]}
        }]
    }"#;
    let result = parse_profile_json(json, Path::new("test"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("stringArray"));
}

// ----------------------------
//  Weight handling
// ----------------------------

#[test]
fn missing_weight_defaults_to_one() {
    let json = r#"{
        "threads": [{
            "name": "Main",
            "isMainThread": true,
            "stringArray": ["main"],
            "funcTable": {"name": [0]},
            "frameTable": {"func": [0]},
            "stackTable": {"frame": [0], "prefix": [-1]},
            "samples": {"stack": [0]}
        }]
    }"#;

    let summary = parse_profile_json(json, Path::new("test")).expect("should parse");
    assert!((summary.total_sample_weight - 1.0).abs() < 1e-9);
}

#[test]
fn samples_weight_type_is_accepted() {
    // The fixture uses weightType = "samples" on the main thread and should
    // parse without warnings about non-standard weight types.
    let summary = parse_fixture();
    let non_standard_warnings: Vec<_> = summary
        .warnings
        .iter()
        .filter(|w| w.contains("non-standard weightType"))
        .collect();
    // The main thread has weightType "samples" -> no warning.
    // The worker thread has no weightType -> no warning.
    assert!(non_standard_warnings.is_empty());
}

#[test]
fn non_samples_weight_type_generates_warning() {
    let json = r#"{
        "threads": [{
            "name": "Main",
            "isMainThread": true,
            "stringArray": ["main"],
            "funcTable": {"name": [0]},
            "frameTable": {"func": [0]},
            "stackTable": {"frame": [0], "prefix": [-1]},
            "samples": {
                "weightType": "time",
                "stack": [0],
                "weight": [100]
            }
        }]
    }"#;

    let summary = parse_profile_json(json, Path::new("test")).expect("should parse");
    let has_warning = summary
        .warnings
        .iter()
        .any(|w| w.contains("non-standard weightType"));
    assert!(has_warning);
}

#[test]
fn non_numeric_weight_generates_warning() {
    let json = r#"{
        "threads": [{
            "name": "Main",
            "isMainThread": true,
            "stringArray": ["main"],
            "funcTable": {"name": [0]},
            "frameTable": {"func": [0]},
            "stackTable": {"frame": [0], "prefix": [-1]},
            "samples": {
                "stack": [0],
                "weight": ["not_a_number"]
            }
        }]
    }"#;

    let summary = parse_profile_json(json, Path::new("test")).expect("should parse");
    let has_warning = summary
        .warnings
        .iter()
        .any(|w| w.contains("not a finite number"));
    assert!(has_warning);
    // Should fall back to 1.0.
    assert!((summary.total_sample_weight - 1.0).abs() < 1e-9);
}

#[test]
fn mismatched_weight_array_length_generates_warning() {
    let json = r#"{
        "threads": [{
            "name": "Main",
            "isMainThread": true,
            "stringArray": ["main"],
            "funcTable": {"name": [0]},
            "frameTable": {"func": [0]},
            "stackTable": {"frame": [0], "prefix": [-1]},
            "samples": {
                "stack": [0],
                "weight": [1, 2]
            }
        }]
    }"#;

    let summary = parse_profile_json(json, Path::new("test")).expect("should parse");
    let has_warning = summary
        .warnings
        .iter()
        .any(|w| w.contains("weight array length"));
    assert!(has_warning);
}

// ----------------------------
//  Unknown function handling
// ----------------------------

#[test]
fn out_of_range_func_name_resolves_to_unknown() {
    let json = r#"{
        "threads": [{
            "name": "Main",
            "isMainThread": true,
            "stringArray": ["main"],
            "funcTable": {"name": [99]},
            "frameTable": {"func": [0]},
            "stackTable": {"frame": [0], "prefix": [-1]},
            "samples": {"stack": [0]}
        }]
    }"#;

    let summary = parse_profile_json(json, Path::new("test")).expect("should parse");
    // func name index 99 is out of range -> resolves to "unknown".
    let unknown = find_function(&summary, "unknown");
    assert!((unknown.inclusive_samples - 1.0).abs() < 1e-9);
}

// ----------------------------
//  Cross-thread function merging
// ----------------------------

#[test]
fn same_function_name_merges_across_threads() {
    // Both threads have "main" in their string tables with independent indexes.
    // The parser should merge them into one function entry.
    let json = r#"{
        "threads": [
            {
                "name": "Thread A",
                "isMainThread": true,
                "stringArray": ["main", "foo"],
                "funcTable": {"name": [0, 1]},
                "frameTable": {"func": [0, 1]},
                "stackTable": {"frame": [0, 1], "prefix": [-1, 0]},
                "samples": {"stack": [1], "weight": [10]}
            },
            {
                "name": "Thread B",
                "isMainThread": false,
                "stringArray": ["bar", "main"],
                "funcTable": {"name": [0, 1]},
                "frameTable": {"func": [0, 1]},
                "stackTable": {"frame": [0, 1], "prefix": [-1, 0]},
                "samples": {"stack": [1], "weight": [5]}
            }
        ]
    }"#;

    let summary = parse_profile_json(json, Path::new("test")).expect("should parse");

    // "main" should appear once with combined inclusive from both threads.
    let main = find_function(&summary, "main");
    // Thread A: main is ancestor of foo (inclusive 10). Thread B: main is ancestor (inclusive 5).
    assert!((main.inclusive_samples - 15.0).abs() < 1e-9);
    assert!(main.thread_names.contains(&"Thread A".to_string()));
    assert!(main.thread_names.contains(&"Thread B".to_string()));
}

// ----------------------------
//  Sorting
// ----------------------------

#[test]
fn functions_are_sorted_by_inclusive_weight() {
    let summary = parse_fixture();
    for window in summary.functions.windows(2) {
        assert!(
            window[0].inclusive_samples >= window[1].inclusive_samples,
            "Functions not sorted: {} ({}) < {} ({})",
            window[0].name,
            window[0].inclusive_samples,
            window[1].name,
            window[1].inclusive_samples
        );
    }
}
