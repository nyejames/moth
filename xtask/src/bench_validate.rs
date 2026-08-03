//! Thin aggregate preflight entry point for every benchmark manifest case.
//!
//! WHAT: loads benchmark inputs once, builds one release compiler and delegates
//! every case execution to the shared preflight authority.
//! WHY: benchmark validation must not maintain a second command or diagnostic
//! parsing implementation.

use crate::benchmark_execution::{
    BenchmarkExecutionContext, format_case_failures, preflight_cases,
};
use crate::benchmark_repository::verify_after_operation;
use crate::benchmark_run::PreparedBenchmarkRun;
use crate::benchmark_workspace::{BenchmarkExecutionWorkspace, finalise_workspace};
use crate::compiler_binary::build_release_compiler_with_timers;

/// Preflight all benchmark cases without recording history or summaries.
pub fn validate_all_benchmarks() -> Result<(), String> {
    let prepared = PreparedBenchmarkRun::load()?;

    let compiler = build_release_compiler_with_timers(&prepared.manifest.repository_root)?;
    let workspace = BenchmarkExecutionWorkspace::create(&prepared.manifest.repository_root)?;
    let context =
        BenchmarkExecutionContext::new(&prepared.manifest, compiler.as_path(), &workspace);

    let result = match preflight_cases(&context, &prepared.manifest.cases) {
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

    let result = finalise_workspace(&workspace, result);
    verify_after_operation(
        &prepared.snapshot,
        &prepared.manifest.repository_root,
        result,
    )
}

#[cfg(test)]
mod tests;
