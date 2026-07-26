//! Integration test runner execution.
//!
//! WHAT: runs the integration test suite and renders results.

use crate::compiler_tests::integration_test_runner::{
    BackendId, CaseExecutionResult, FailureTriageEntry, SummaryCounts, TestCaseSpec,
    TestRunnerOptions, TestSuiteSpec,
};
use rayon::prelude::*;
use saying::say;
use std::collections::BTreeMap;

use super::{SEPARATOR_LINE_LENGTH, execution, fixture, reporting};

/// Normalises a relative path string to forward slashes for cross-platform comparison.
pub(crate) fn normalize_relative_path_text(path: &str) -> String {
    path.replace('\\', "/")
}

/// Normalises a `Path` to a forward-slash string for cross-platform comparison.
pub(crate) fn normalize_relative_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Runs or lists the selected cases from the `tests/cases` directory.
pub(crate) fn run_all_test_cases(
    options: TestRunnerOptions,
) -> Result<super::IntegrationRunSummary, String> {
    options.validate()?;

    let suite = fixture::load_test_suite()?;
    run_loaded_suite(
        suite,
        options,
        execution::execute_test_case,
        super::SUITE_INVENTORY_REPORT_PATH,
        super::FAILURE_TRIAGE_REPORT_PATH,
    )
}

pub(crate) fn run_loaded_suite<F>(
    suite: TestSuiteSpec,
    options: TestRunnerOptions,
    execute_case: F,
    inventory_report_path: &str,
    triage_report_path: &str,
) -> Result<super::IntegrationRunSummary, String>
where
    F: Fn(&TestCaseSpec) -> CaseExecutionResult + Send + Sync,
{
    options.validate()?;

    // Evaluate the complete loaded suite before selection so every normal mode shares one
    // policy boundary and no compiler callback can run for a hard-invalid suite.
    let policy_evaluation = super::policy::evaluate_suite(&suite);

    if options.audit {
        let report = reporting::build_suite_inventory_report(
            &suite.cases,
            &policy_evaluation,
            reporting::discover_repository_commit(),
        );
        reporting::write_suite_inventory_report(inventory_report_path, &report)?;
        println!(
            "Wrote integration suite inventory to {} ({} cases, {} backend executions).",
            inventory_report_path,
            report.manifest_case_count,
            report.expanded_backend_execution_count
        );

        if policy_evaluation.has_hard_findings() {
            return Err(super::policy::format_hard_findings(&policy_evaluation));
        }

        return Ok(SummaryCounts::default().into());
    }

    if policy_evaluation.has_hard_findings() {
        return Err(super::policy::format_hard_findings(&policy_evaluation));
    }

    let cases = select_cases(suite.cases, &options);

    if options.list {
        print!("{}", reporting::format_case_listing(&cases));
        return Ok(SummaryCounts::default().into());
    }

    if cases.is_empty() && options.has_selection_filters() {
        return Err(String::from(
            "No integration test cases matched the requested selection filters.",
        ));
    }

    println!("Running Moth test cases...\n");
    let timer = std::time::Instant::now();
    let mut indexed_results = if let Some(thread_count) = test_thread_count_from_env()? {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(thread_count)
            .build()
            .map_err(|error| format!("Failed to create test runner thread pool: {error}"))?;

        pool.install(|| {
            cases
                .into_par_iter()
                .enumerate()
                .map(|(index, case)| {
                    let result = execute_case(&case);
                    (index, case, result)
                })
                .collect::<Vec<_>>()
        })
    } else {
        cases
            .into_par_iter()
            .enumerate()
            .map(|(index, case)| {
                let result = execute_case(&case);
                (index, case, result)
            })
            .collect::<Vec<_>>()
    };

    indexed_results.sort_by_key(|(index, _, _)| *index);

    let mut total_summary = SummaryCounts::default();
    let mut backend_summaries = BTreeMap::<BackendId, SummaryCounts>::new();
    let mut failure_triage_entries = Vec::new();

    let case_results = indexed_results
        .into_iter()
        .map(|(_, case, result)| {
            total_summary.record(&case, &result);
            backend_summaries
                .entry(case.backend_id)
                .or_default()
                .record(&case, &result);

            if !result.passed {
                failure_triage_entries.push(FailureTriageEntry {
                    case: case.display_name.clone(),
                    backend: case.backend_id.as_str().to_string(),
                    expected_outcome: reporting::expected_outcome_label(&case.expected),
                    failure_reason: reporting::observed_failure_reason(&result),
                    failure_kind: result.failure_kind,
                    panic_message: result.panic_message.clone(),
                });
            }

            (case, result)
        })
        .collect::<Vec<_>>();

    let failures: Vec<_> = case_results.iter().filter(|(_, r)| !r.passed).collect();
    if !failures.is_empty() {
        say!(Cyan "Failures:");
        say!(Dark White "=".repeat(SEPARATOR_LINE_LENGTH));
        for (case, result) in &failures {
            println!("  {}", case.display_name);
            reporting::render_case_result(case, result, options.show_warnings);
            say!(Dark White "-".repeat(SEPARATOR_LINE_LENGTH));
        }
        println!();
    }

    println!();
    say!(Dark White "=".repeat(SEPARATOR_LINE_LENGTH));
    print!("Test Results Summary. Took: ");
    say!(Green #timer.elapsed());

    say!("\n  Total tests:             ", Yellow total_summary.total_tests);
    say!(
        "  Successful compilations: ",
        Blue total_summary.passed_tests
    );
    say!(
        "  Failed compilations:     ",
        Blue total_summary.failed_tests
    );
    say!(
        "  Expected failures:       ",
        Blue total_summary.expected_failures
    );
    say!(
        "  Unexpected successes:    ",
        Blue total_summary.unexpected_successes
    );

    say!();
    if total_summary.incorrect_results() == 0 {
        say!(
            "  Correct results:   ",
            Green Bold total_summary.correct_results(),
            Dark White " / ",
            total_summary.total_tests
        );
    } else {
        say!(
            "  Incorrect results: ",
            Red Bold total_summary.incorrect_results(),
            Dark White " / ",
            total_summary.total_tests
        );
    }

    reporting::render_backend_summary(&backend_summaries);

    if total_summary.incorrect_results() == 0 {
        say!("\nAll tests behaved as expected.");
    } else if total_summary.total_tests > 0 {
        let percentage = reporting::format_pass_percentage(
            total_summary.correct_results(),
            total_summary.total_tests,
        );
        say!(
            Yellow "\n",
            Bright Yellow percentage,
            " %",
            Reset " of tests behaved as expected"
        );
    }

    reporting::write_failure_triage_report(
        triage_report_path,
        total_summary,
        &failure_triage_entries,
    )?;

    say!(Dark White "=".repeat(SEPARATOR_LINE_LENGTH));
    Ok(total_summary.into())
}

pub(crate) fn select_cases(
    cases: Vec<TestCaseSpec>,
    options: &TestRunnerOptions,
) -> Vec<TestCaseSpec> {
    cases
        .into_iter()
        .filter(|case| {
            options
                .case_id
                .as_deref()
                .is_none_or(|case_id| case.case_id == case_id)
                && options
                    .tag_filters
                    .iter()
                    .all(|tag| case.tags.iter().any(|case_tag| case_tag == tag))
                && options
                    .contract
                    .as_deref()
                    .is_none_or(|contract| case.contract.as_deref() == Some(contract))
                && options
                    .backend_filter
                    .is_none_or(|backend| case.backend_id == backend)
        })
        .collect()
}

fn test_thread_count_from_env() -> Result<Option<usize>, String> {
    let Some(raw) = std::env::var_os("MOTH_TEST_THREADS") else {
        return Ok(None);
    };

    let threads = raw
        .to_string_lossy()
        .parse::<usize>()
        .map_err(|_| "MOTH_TEST_THREADS must be a positive integer".to_string())?;

    if threads == 0 {
        return Err("MOTH_TEST_THREADS must be greater than 0".to_string());
    }

    Ok(Some(threads))
}
