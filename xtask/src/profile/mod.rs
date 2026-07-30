//! Profiling module - Samply-backed profiling workflow for benchmark cases
//!
//! WHAT: Provides a profiling workflow that builds a dedicated profiling
//! compiler binary, runs Samply against benchmark cases, extracts function
//! hotspots, and generates agent-readable summaries with source-owner hints.
//!
//! WHY: Profiling gives attribution evidence for optimization work. The
//! workflow uses shared preflight before separating observation passes
//! (detailed timer/counter data) from Samply stack sampling.
//!
//! # What this module owns
//! - Profiling command parsing and option types (`ProfileOptions`, `ProfileFilterMode`)
//! - Profiling build helpers (via `compiler_binary` module)
//! - Artifact layout, observation logging, Samply runner integration
//! - Profile JSON parsing and hotspot extraction
//! - Agent summaries and drift reports
//! - Profile history recording
//!
//! # What this module does NOT own
//! - Normal benchmark orchestration (see `bench.rs`)
//! - Frontend benchmark orchestration (see `frontend_bench.rs`)
//! - Release compiler binary building (see `compiler_binary.rs`)
//! - Benchmark observation parsing (see `bench_observations.rs`)
//! - Benchmark history and reports (see `bench_history.rs`, `bench_report.rs`)

pub(crate) mod artifacts;
pub(crate) mod buckets;
pub(crate) mod drift;
pub(crate) mod history;
pub(crate) mod hotspots;
mod json;
pub(crate) mod observations;
pub(crate) mod options;
pub(crate) mod parse;
pub(crate) mod runner;
pub(crate) mod summary;

// Re-export the narrow surface needed by main.rs and mode.rs.
pub(crate) use options::{ProfileOptions, ProfileParseResult, parse_profile_args};

use crate::bench_history::get_git_revision;
use crate::bench_time::BenchmarkTimestamp;
use crate::benchmark_execution::{
    BenchmarkExecutionContext, format_case_failures, preflight_cases,
};
use crate::benchmark_manifest::{
    BenchmarkCase, BenchmarkManifest, BenchmarkRunner, load_benchmark_manifest,
};
use crate::benchmark_workspace::BenchmarkExecutionWorkspace;
use crate::compiler_binary::{CompilerBinary, build_profiling_compiler_with_timers};
use std::collections::HashMap;
use std::path::PathBuf;

use artifacts::{ProfileCaseManifest, ProfileRunPaths, write_index_md, write_run_manifest};
use drift::{
    DriftCaseInput, DriftHotFunction, compute_drift, find_comparable_previous,
    format_drift_markdown, format_drift_summary_section, no_previous_drift_report,
};
use history::{HistoryCaseRecord, HistoryHotFunction, PROFILE_RUNS_JSONL_PATH};
use hotspots::{HotspotExtractionResult, extract_hotspots};
use observations::run_observation;
use options::ProfileFilterMode;
use parse::{parse_profile, parse_profile_shape_dump};
use runner::{SamplyRecordCapabilities, SamplyRunInput, check_samply_available, run_samply};
use summary::{
    CaseSummaryData, append_drift_to_agent_summary, generate_agent_summary, generate_case_summary,
    generate_root_hotspots_json,
};

/// Root path for all profiling local data, relative to repo root.
const PROFILES_ROOT: &str = "benchmarks/local-data/profiles";

/// Run the profiling benchmark workflow.
///
/// WHAT: Loads benchmark cases, applies case filtering, builds the profiling
/// compiler, preflights every selected case, runs observation passes, records
/// Samply profiles, and writes local artifacts under
/// `benchmarks/local-data/profiles/`.
///
/// WHY: This orchestrator ties together failure-safe preflight, artifact layout,
/// observation logging, Samply recording and case filtering. The observation
/// pass gives timer/counter data without profiler overhead; the Samply pass
/// gives stack samples for hotspot extraction.
///
/// If any case fails, the run directory is left for debugging but no history
/// is appended.
pub(crate) fn run_profile_benchmarks(options: ProfileOptions) -> Result<(), String> {
    // Load and filter benchmark cases.
    let manifest = load_benchmark_manifest().map_err(|error| error.to_string())?;
    let selected_cases = select_profile_cases(&manifest, options.case_filter.as_deref())?;

    // Verify Samply is available and learn the version-specific record flags before doing work.
    let samply_capabilities = check_samply_available()?;
    if options.presymbolicate
        && samply_capabilities
            .presymbolication_flag
            .command_flag()
            .is_none()
    {
        eprintln!(
            "Warning: --presymbolicate was requested, but this Samply build exposes no presymbolication flag; function hotspots are likely to remain raw addresses."
        );
    }

    // Build the profiling compiler with debug info and frame pointers for Samply.
    println!("Building profiling compiler...");
    let profiling_binary = build_profiling_compiler_with_timers(&manifest.repository_root)?;
    let moth_path = profiling_binary.as_path();
    let symbol_dirs = profiling_binary.symbol_dirs.clone();
    let workspace = BenchmarkExecutionWorkspace::create(&manifest.repository_root)?;
    let execution_context = BenchmarkExecutionContext::new(&manifest, moth_path, &workspace);

    println!(
        "Preflighting {} CLI profile case(s) with the profiling binary...",
        selected_cases.len()
    );
    preflight_cases(&execution_context, &selected_cases)
        .map_err(|failures| format_case_failures("profile preflight", &failures))?;

    // Get the short commit hash for the run id.
    let commit = get_git_revision().commit;

    // Create the run directory.
    let profiles_root = PathBuf::from(PROFILES_ROOT);
    let run_paths = ProfileRunPaths::create(&profiles_root, commit.as_deref())?;

    println!(
        "Profiling run {} — {} cases, filter: {}",
        run_paths.run_id,
        selected_cases.len(),
        options.filter.display_label(),
    );

    // Run each selected case: observation → Samply → write artifacts.
    // Accumulate both manifests (for run-manifest.json) and summary data
    // (for root agent-summary.md and profile-hotspots.json).
    let mut case_manifests = Vec::new();
    let mut case_summaries: Vec<(
        observations::ProfileObservation,
        Option<HotspotExtractionResult>,
    )> = Vec::new();

    for case in &selected_cases {
        let invocation = execution_context
            .resolve_cli_invocation(case)
            .map_err(|error| error.to_string())?;
        print!("  {} ", case.id);

        // Observation pass (timer/counter data without profiler overhead).
        let observation = run_observation(&execution_context, case)?;
        print!("~{:.0}ms ", observation.wall_ms);

        // Write per-case artifacts (stdout, stderr, observations).
        // Summary is deferred until after hotspot extraction so it can include
        // hotspot data, hints, and sample counts.
        let case_paths = run_paths.case_paths(&case.id);
        case_paths.create_dir()?;
        case_paths.write_stdout(&observation.stdout)?;
        case_paths.write_stderr(&observation.stderr)?;
        case_paths.write_observations_json(&observation)?;

        // Samply recording pass (stack samples for hotspot extraction).
        let samply_input = SamplyRunInput {
            moth_path: moth_path.to_path_buf(),
            current_directory: invocation.current_directory.clone(),
            command: invocation.command.as_str().to_owned(),
            args: invocation.args.clone(),
            output_path: case_paths.profile_json.clone(),
            samply_rate_hz: options.samply_rate_hz,
            presymbolicate: options.presymbolicate,
            presymbolication_flag: samply_capabilities.presymbolication_flag,
            symbol_dirs: symbol_dirs.clone(),
        };

        let samply_run = run_samply(&samply_input)?;

        if !samply_run.success {
            let smoke_diagnostic = format_symbolication_smoke_diagnostic(
                &samply_capabilities,
                &profiling_binary,
                options.presymbolicate,
                samply_run.presymbolication_flag.display_label(),
                &case_paths.profile_json,
                None,
                "samply_failed",
            );
            return Err(format!(
                "Samply recording failed for case '{}'.\n\
                 Command: {}\n\
                 Observation artifacts were written under '{}' before Samply failed.\n\
                 {}\n\
                 Stdout: {}\n\
                 Stderr: {}",
                case.id,
                samply_run.command_line,
                case_paths.case_dir.display(),
                smoke_diagnostic,
                samply_run.stdout.trim(),
                samply_run.stderr.trim()
            ));
        }

        print!("[samply {:.0}ms] ", samply_run.duration_ms);

        // Hotspot extraction: parse the profile and extract hotspots for
        // non-RawIndex modes. RawIndex skips parsing to save time.
        let hotspot_result = if options.filter != ProfileFilterMode::RawIndex {
            let parsed = parse_profile(&case_paths.profile_json)
                .map_err(|e| format!("Failed to parse profile for case '{}': {}", case.id, e))?;

            let mut result = extract_hotspots(&parsed, options.filter, observation.wall_ms);
            if result.symbolication.is_failed() {
                match parse_profile_shape_dump(&case_paths.profile_json) {
                    Ok(shape) => {
                        artifacts::write_profile_shape_dump(&case_paths, &shape)?;
                    }
                    Err(error) => {
                        result
                            .warnings
                            .push(format!("Profile shape dump failed: {error}"));
                    }
                }
            }

            print_symbolication_smoke_diagnostic(
                &samply_capabilities,
                &profiling_binary,
                options.presymbolicate,
                samply_run.presymbolication_flag.display_label(),
                &case_paths.profile_json,
                &result,
            );
            artifacts::write_hotspots_json(&case_paths, &result)?;
            print!("[{} hotspots] ", result.functions.len());
            Some(result)
        } else {
            None
        };

        // Build manifest entry for this case.
        case_manifests.push(ProfileCaseManifest {
            case_id: case.id.clone(),
            group_name: case.group_name.clone(),
            command: invocation.command.as_str().to_owned(),
            args: invocation.args.clone(),
            observation_wall_ms: observation.wall_ms,
            profile_path: format!("cases/{}/profile.json.gz", case.id),
            stdout_path: format!("cases/{}/stdout.log", case.id),
            stderr_path: format!("cases/{}/stderr.log", case.id),
            summary_path: format!("cases/{}/summary.md", case.id),
        });

        // Accumulate for root summary generation.
        case_summaries.push((observation, hotspot_result));

        println!("done");
    }

    // ---------------------------------------------------------------
    //  Enriched per-case summaries and root summary artifacts
    // ---------------------------------------------------------------

    // Generate enriched per-case summary.md files that include hotspots and hints.
    // This overwrites the basic summary that was not written during case processing.
    let mut summary_data_refs: Vec<CaseSummaryData<'_>> = Vec::new();
    for (observation, hotspot_result) in &case_summaries {
        if let Some(hotspots) = hotspot_result {
            let profile_path = format!("cases/{}/profile.json.gz", observation.case_id);
            let data = CaseSummaryData {
                observation,
                hotspots,
                profile_relative_path: profile_path,
                filter: options.filter,
            };
            generate_case_summary(&run_paths, &data)?;
            summary_data_refs.push(data);
        }
    }

    // Write root artifacts.
    write_run_manifest(
        &run_paths,
        &run_paths.run_id,
        commit.as_deref(),
        options.filter,
        options.samply_rate_hz,
        &case_manifests,
    )?;

    write_index_md(
        &run_paths,
        &run_paths.run_id,
        options.filter,
        &case_manifests,
    )?;

    // Generate root summary artifacts when hotspot data is available.
    if !summary_data_refs.is_empty() {
        generate_root_hotspots_json(
            &run_paths,
            &summary_data_refs,
            &run_paths.run_id,
            commit.as_deref(),
            options.filter,
            options.samply_rate_hz,
        )?;

        generate_agent_summary(
            &run_paths,
            &summary_data_refs,
            &run_paths.run_id,
            options.filter,
        )?;
    }

    // ---------------------------------------------------------------
    //  Phase 6: Profile history and drift reports
    // ---------------------------------------------------------------

    // Compute drift against the latest comparable previous record.
    let drift_report = if options.filter != ProfileFilterMode::RawIndex {
        let history_path = std::path::Path::new(PROFILE_RUNS_JSONL_PATH);
        let previous_records = history::read_profile_runs(history_path).unwrap_or_default();

        let system = crate::bench_system::load_or_create_system(
            crate::bench_system::SystemIdentityMode::ReadOnly,
        )
        .unwrap_or(None);
        let system_uuid = system
            .as_ref()
            .map(|s| s.system_uuid.as_str())
            .unwrap_or("unknown");

        let previous = find_comparable_previous(
            &previous_records,
            system_uuid,
            options.filter.display_label(),
            options.samply_rate_hz,
            &run_paths.run_id,
        );

        if let Some(prev) = previous {
            // Build current case inputs for drift comparison.
            let mut drift_cases = Vec::new();
            let mut wall_times = HashMap::new();

            for (observation, hotspot_result) in &case_summaries {
                wall_times.insert(observation.case_id.clone(), observation.wall_ms);

                if let Some(hotspots) = hotspot_result {
                    let hot_functions: Vec<DriftHotFunction> = hotspots
                        .functions
                        .iter()
                        .map(|f| DriftHotFunction {
                            name: f.name.clone(),
                            bucket_label: f.bucket.label.clone(),
                            inclusive_samples: f.inclusive_samples,
                            inclusive_pct: f.inclusive_pct,
                        })
                        .collect();

                    drift_cases.push(DriftCaseInput {
                        case_id: observation.case_id.clone(),
                        command: observation.command.clone(),
                        args: observation.command_args.clone(),
                        stage_timings: observation.observations.stage_timings.clone(),
                        counters: observation.observations.counters.clone(),
                        hot_functions,
                    });
                }
            }

            compute_drift(&drift_cases, prev, &wall_times)
        } else {
            no_previous_drift_report()
        }
    } else {
        no_previous_drift_report()
    };

    // Write profile-drift.md.
    let drift_md = format_drift_markdown(&drift_report);
    let drift_path = run_paths.root.join("profile-drift.md");
    std::fs::write(&drift_path, drift_md).map_err(|e| {
        format!(
            "Failed to write profile-drift.md '{}': {}",
            drift_path.display(),
            e
        )
    })?;

    // Append drift section to agent-summary.md only when the summary exists
    // (i.e. not in raw-index mode, which skips parsed hotspots and summary).
    if options.filter != ProfileFilterMode::RawIndex {
        let drift_section = format_drift_summary_section(&drift_report);
        append_drift_to_agent_summary(&run_paths, &drift_section)?;
    }

    // Append profile history record after the full run succeeds.
    if options.filter != ProfileFilterMode::RawIndex {
        let history_cases: Vec<HistoryCaseRecord> = case_summaries
            .iter()
            .filter_map(|(observation, hotspot_result)| {
                hotspot_result.as_ref().map(|hotspots| {
                    let hot_functions: Vec<HistoryHotFunction> = hotspots
                        .functions
                        .iter()
                        .map(|f| HistoryHotFunction {
                            name: f.name.clone(),
                            bucket_label: f.bucket.label.clone(),
                            inclusive_samples: f.inclusive_samples,
                            self_samples: f.self_samples,
                            inclusive_pct: f.inclusive_pct,
                            self_pct: f.self_pct,
                        })
                        .collect();

                    let top_bucket_label = hot_functions
                        .first()
                        .map(|f| f.bucket_label.clone())
                        .unwrap_or_else(|| "unknown".to_string());

                    HistoryCaseRecord {
                        case_id: observation.case_id.clone(),
                        group_name: observation.group_name.clone(),
                        command: observation.command.clone(),
                        args: observation.command_args.clone(),
                        observation_wall_ms: observation.wall_ms,
                        sample_count: hotspots.total_sample_count,
                        sample_weight: hotspots.total_sample_weight,
                        stage_timings: observation.observations.stage_timings.clone(),
                        counters: observation.observations.counters.clone(),
                        hot_functions,
                        top_bucket_label,
                        run_directory_path: run_paths.root.to_str().unwrap_or("").to_string(),
                    }
                })
            })
            .collect();

        let ts = BenchmarkTimestamp::now();
        let timestamp = ts.format_run_header();

        let history_record = history::build_history_record(
            &run_paths.run_id,
            &timestamp,
            commit.as_deref(),
            options.filter.display_label(),
            options.samply_rate_hz,
            history_cases,
        )?;

        let history_path = std::path::Path::new(PROFILE_RUNS_JSONL_PATH);
        history::append_profile_run(history_path, &history_record)?;
    }

    println!(
        "\nProfiling artifacts written to: {}",
        run_paths.root.display()
    );

    Ok(())
}

/// Filter benchmark cases by an optional authored case ID.
///
/// Returns all cases if the filter is `None`. Returns only matching cases
/// otherwise. Returns an error if the filter matches no case.
fn select_profile_cases(
    manifest: &BenchmarkManifest,
    case_filter: Option<&str>,
) -> Result<Vec<BenchmarkCase>, String> {
    let Some(filter) = case_filter else {
        return Ok(manifest.cli_cases().cloned().collect());
    };

    let Some(case) = manifest.cases.iter().find(|case| case.id == filter) else {
        return Err(format!(
            "Case filter '{}' matched no benchmark case. Use 'bench-report' to see available cases.",
            filter
        ));
    };

    match case.runner {
        BenchmarkRunner::Cli { .. } => Ok(vec![case.clone()]),
        BenchmarkRunner::Frontend { .. } => Err(format!(
            "Frontend benchmark case '{}' cannot be profiled with Samply. Select a CLI benchmark case.",
            case.id
        )),
    }
}

fn print_symbolication_smoke_diagnostic(
    samply: &SamplyRecordCapabilities,
    profiling_binary: &CompilerBinary,
    presymbolicate_requested: bool,
    selected_presymbolication_flag: &str,
    profile_path: &std::path::Path,
    hotspots: &HotspotExtractionResult,
) {
    let hot_function_counts = SymbolicationHotFunctionCounts {
        hot_function_count: hotspots.symbolication.hot_function_count,
        raw_address_function_count: hotspots.symbolication.raw_address_function_count,
    };

    let text = format_symbolication_smoke_diagnostic(
        samply,
        profiling_binary,
        presymbolicate_requested,
        selected_presymbolication_flag,
        profile_path,
        Some(hot_function_counts),
        hotspots.symbolication.status.as_str(),
    );

    println!();
    println!("{text}");
}

struct SymbolicationHotFunctionCounts {
    hot_function_count: usize,
    raw_address_function_count: usize,
}

fn format_symbolication_smoke_diagnostic(
    samply: &SamplyRecordCapabilities,
    profiling_binary: &CompilerBinary,
    presymbolicate_requested: bool,
    selected_presymbolication_flag: &str,
    profile_path: &std::path::Path,
    hot_function_counts: Option<SymbolicationHotFunctionCounts>,
    symbolication_status: &str,
) -> String {
    let profiling_symbols = profiling_binary.profiling_symbols.as_ref();
    let debug_info_setting = profiling_symbols
        .map(|symbols| symbols.debug_info_setting)
        .unwrap_or("unknown");
    let dsym_path = profiling_symbols
        .map(|symbols| symbols.dsym_path.display().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let dsym_uuid_match = profiling_symbols
        .map(|symbols| symbols.dsym_uuid_match.as_str())
        .unwrap_or("unknown");

    let selected_flag = if presymbolicate_requested {
        selected_presymbolication_flag.to_string()
    } else {
        format!(
            "not requested (available: {})",
            samply.presymbolication_flag.display_label()
        )
    };

    let hot_functions = hot_function_counts
        .as_ref()
        .map(|counts| counts.hot_function_count.to_string())
        .unwrap_or_else(|| "unavailable".to_string());
    let raw_address_hot_functions = hot_function_counts
        .as_ref()
        .map(|counts| counts.raw_address_function_count.to_string())
        .unwrap_or_else(|| "unavailable".to_string());

    format!(
        "Symbolication smoke diagnostic:\n\
           Samply version: {}\n\
           Presymbolication flag: {}\n\
           Profiling binary: {}\n\
           Debug info setting: {}\n\
           dSYM path: {}\n\
           dSYM UUID matches binary: {}\n\
           Profile file: {}\n\
           Hot functions: {}\n\
           Raw-address hot functions: {}\n\
           Symbolication status: {}",
        samply.version,
        selected_flag,
        profiling_binary.path.display(),
        debug_info_setting,
        dsym_path,
        dsym_uuid_match,
        profile_path.display(),
        hot_functions,
        raw_address_hot_functions,
        symbolication_status,
    )
}

#[cfg(test)]
mod tests;
