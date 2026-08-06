//! Concise grouped counter summary rendering.
//!
//! WHAT: owns the static counter group table and the human summary
//!      renderer used after a command drains its timing snapshot.
//! WHY:  keeps the enabled module focused on collection and recording;
//!       counter presentation is benchmark-counted and independent of
//!       the timer record path.

#[cfg(feature = "benchmark_counters")]
use super::BenchmarkObservationSnapshot;

// ---------------------------------------------------------------------------
//  Counter summary
// ---------------------------------------------------------------------------

#[cfg(feature = "benchmark_counters")]
struct CounterSummaryGroup {
    label: &'static str,
    metrics: &'static [(&'static str, &'static str)],
}

#[cfg(feature = "benchmark_counters")]
const COUNTER_SUMMARY_GROUPS: &[CounterSummaryGroup] = &[
    CounterSummaryGroup {
        label: "inputs",
        metrics: &[
            ("module_count", "modules"),
            ("source_file_count", "files"),
            ("source_byte_count", "bytes"),
            ("prepared_file_count", "prepared"),
            ("token_count", "tokens"),
            ("header_count", "headers"),
            ("import_count", "imports"),
            ("top_level_declaration_count", "decls"),
        ],
    },
    CounterSummaryGroup {
        label: "stage0",
        metrics: &[
            ("source_tree_index.discovery_runs", "source scans"),
            ("source_tree_index.dirs_visited", "dirs"),
            ("source_tree_index.dirs_skipped", "skipped dirs"),
            ("source_tree_index.files_seen", "files seen"),
            ("source_tree_index.module_roots_found", "roots"),
            (
                "stage0.reachable_discovery.reachable_files",
                "reachable files",
            ),
            ("stage0.reachable_discovery.import_edges", "import edges"),
            ("stage0_source_cache_hit_count", "source hits"),
            ("stage0_source_cache_miss_count", "source misses"),
            ("stage0_source_bytes_loaded", "bytes loaded"),
        ],
    },
    CounterSummaryGroup {
        label: "scheduling",
        metrics: &[
            ("module_compilation_serial_count", "serial modules"),
            ("module_compilation_parallel_task_count", "parallel tasks"),
            ("file_preparation_serial_module_count", "serial file prep"),
            (
                "file_preparation_parallel_module_count",
                "parallel file prep",
            ),
            (
                "file_preparation_strategy_parallel_per_file_count",
                "per-file strategy",
            ),
            (
                "file_preparation_strategy_chunked_count",
                "chunked strategy",
            ),
        ],
    },
    CounterSummaryGroup {
        label: "frontend",
        metrics: &[
            ("dependency_header_count", "dep headers"),
            ("dependency_edge_count", "dep edges"),
            ("dependency_visit_count", "dep visits"),
            ("ast_header_count", "AST headers"),
            ("ast_function_count", "functions"),
            ("ast_struct_count", "structs"),
            ("ast_choice_count", "choices"),
            ("ast_constant_count", "constants"),
            ("ast_receiver_method_count", "receiver methods"),
            ("ast_generic_instance_count", "generic instances"),
            ("hir_block_count", "HIR blocks"),
            ("hir_statement_count", "HIR statements"),
            ("hir_function_count", "HIR functions"),
            ("borrow_function_count", "borrow functions"),
            ("borrow_block_count", "borrow blocks"),
            ("borrow_conflict_check_count", "borrow checks"),
            ("borrow_state_join_count", "borrow joins"),
            ("borrow_place_access_count", "borrow places"),
        ],
    },
    CounterSummaryGroup {
        label: "scope/type",
        metrics: &[
            ("actual_scope_frames", "scope frames"),
            ("scope_arena_capacity", "scope capacity"),
            (
                "type_environment_substitute_type_id_calls",
                "type substitutions",
            ),
            (
                "type_environment_substitution_cache_lookups",
                "substitution lookups",
            ),
            ("type_compatibility_cache_lookups", "compat lookups"),
            ("type_compatibility_cache_misses", "compat misses"),
        ],
    },
    CounterSummaryGroup {
        label: "string/remap",
        metrics: &[
            ("string_table_full_clones", "full clones"),
            ("string_table_merge_source_entries_scanned", "merge scanned"),
            ("string_table_delta_merge_calls", "delta merges"),
            ("string_table_delta_entries_scanned", "delta scanned"),
            (
                "string_table_delta_non_identity_remaps",
                "non-identity remaps",
            ),
            ("module_remap_string_ids_calls", "module remaps"),
            ("file_prepare_output_remap_calls", "file output remaps"),
            ("file_prepare_error_remap_calls", "file error remaps"),
        ],
    },
    CounterSummaryGroup {
        label: "templates/tir",
        metrics: &[
            ("template_count", "templates"),
            ("const_template_count", "const"),
            ("runtime_template_count", "runtime"),
            ("ast_template_atoms_parsed", "atoms"),
            (
                "ast_templates_folded_during_finalization",
                "finalized folds",
            ),
            ("ast_template_tir_sync_attempts", "TIR sync attempts"),
            ("ast_template_tir_sync_successes", "TIR sync success"),
            ("ast_tir_templates_created", "TIR templates"),
            ("ast_tir_nodes_created", "TIR nodes"),
            ("ast_tir_text_bytes_recorded", "TIR text bytes"),
            ("ast_tir_fold_nodes_visited", "TIR fold nodes"),
        ],
    },
    CounterSummaryGroup {
        label: "external packages",
        metrics: &[
            ("external_package_registry_clone_count", "registry clones"),
            ("external_package_definition_clone_count", "package clones"),
            (
                "external_function_definition_clone_count",
                "function clones",
            ),
            ("external_symbol_path_clone_count", "symbol clones"),
            ("external_abi_parameter_clone_count", "ABI clones"),
        ],
    },
];

/// Render a concise grouped counter summary from a collected snapshot.
///
/// Aggregates counter observations by metric name (summing repeated samples,
/// e.g. per-module discovery counters) and returns a small fixed set of
/// stage-oriented lines. Stable `MOTH_BENCH counter` output remains the full
/// machine-readable path; the human summary is deliberately compact.
#[cfg(feature = "benchmark_counters")]
pub(crate) fn render_counter_summary(snapshot: &BenchmarkObservationSnapshot) -> Vec<String> {
    if snapshot.counters.is_empty() {
        return Vec::new();
    }

    let mut aggregates = std::collections::BTreeMap::<&str, f64>::new();
    for metric in &snapshot.counters {
        *aggregates.entry(metric.name).or_default() += metric.value;
    }

    let mut lines = Vec::with_capacity(COUNTER_SUMMARY_GROUPS.len() + 2);
    lines.push("Counter summary:".to_owned());

    for group in COUNTER_SUMMARY_GROUPS {
        if let Some(line) = render_counter_summary_group(&aggregates, group) {
            lines.push(line);
        }
    }

    let other_nonzero_count = aggregates
        .iter()
        .filter(|(name, value)| **value != 0.0 && !counter_summary_includes_metric(name))
        .count();

    if other_nonzero_count > 0 {
        lines.push(format!(
            "  other nonzero counters: {other_nonzero_count} (see MOTH_BENCH lines)"
        ));
    }

    if lines.len() == 1 {
        lines.push("  no nonzero counters".to_owned());
    }

    lines
}

#[cfg(feature = "benchmark_counters")]
fn render_counter_summary_group(
    aggregates: &std::collections::BTreeMap<&str, f64>,
    group: &CounterSummaryGroup,
) -> Option<String> {
    let mut parts = Vec::new();

    for (metric_name, label) in group.metrics {
        let value = aggregates.get(metric_name).copied().unwrap_or(0.0);
        if value == 0.0 {
            continue;
        }
        parts.push(format!("{label} {}", format_counter_summary_value(value)));
    }

    if parts.is_empty() {
        None
    } else {
        Some(format!("  {}: {}", group.label, parts.join(", ")))
    }
}

#[cfg(feature = "benchmark_counters")]
fn counter_summary_includes_metric(name: &str) -> bool {
    COUNTER_SUMMARY_GROUPS.iter().any(|group| {
        group
            .metrics
            .iter()
            .any(|(metric_name, _label)| *metric_name == name)
    })
}

#[cfg(feature = "benchmark_counters")]
fn format_counter_summary_value(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}
