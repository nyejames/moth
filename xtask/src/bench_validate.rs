//! Thin aggregate preflight entry point for every benchmark manifest case.
//!
//! WHAT: loads benchmark inputs once, builds one release compiler and delegates
//! every case execution to the shared preflight authority.
//! WHY: benchmark validation must not maintain a second command or diagnostic
//! parsing implementation.

use crate::benchmark_execution::{
    BenchmarkExecutionContext, format_case_failures, preflight_cases,
};
use crate::benchmark_fingerprint::compute_benchmark_fingerprints;
use crate::benchmark_manifest::load_benchmark_manifest;
use crate::benchmark_repository::{BenchmarkRepositorySnapshot, verify_after_operation};
use crate::benchmark_workspace::BenchmarkExecutionWorkspace;
use crate::compiler_binary::build_release_compiler_with_timers;

/// Preflight all benchmark cases without recording history or summaries.
pub fn validate_all_benchmarks() -> Result<(), String> {
    let manifest = load_benchmark_manifest().map_err(|error| error.to_string())?;

    // Capture repository state before any compiler construction or preflight.
    let snapshot = BenchmarkRepositorySnapshot::capture(&manifest.repository_root)
        .map_err(|error| error.to_string())?;

    let compiler = build_release_compiler_with_timers(&manifest.repository_root)?;
    let _fingerprints =
        compute_benchmark_fingerprints(&manifest).map_err(|error| error.to_string())?;
    let workspace = BenchmarkExecutionWorkspace::create(&manifest.repository_root)?;
    let context = BenchmarkExecutionContext::new(&manifest, compiler.as_path(), &workspace);

    let result = match preflight_cases(&context, &manifest.cases) {
        Ok(executions) => {
            for execution in &executions {
                println!("  {execution}");
            }
            println!(
                "All {} benchmark cases passed shared preflight.",
                executions.len()
            );
            Ok(())
        }
        Err(failures) => Err(format_case_failures("preflight", &failures)),
    };

    verify_after_operation(&snapshot, &manifest.repository_root, result)
}

#[cfg(test)]
mod tests;
