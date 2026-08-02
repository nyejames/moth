//! Integration-test result validation and assertion-family wiring.
//!
//! WHAT: sequences success and failure checks against the canonical expectation model.
//! WHY: each assertion family owns its checks while this module preserves their established
//!      order and converts failures into runner results.

mod artifacts;
mod diagnostics;
mod goldens;
mod rendered_output;
mod warnings;
mod wasm;

pub(crate) use goldens::discover_golden_expectation;
#[cfg(test)]
pub(crate) use rendered_output::execute_wasm_harness_for_test;
#[cfg(test)]
pub(crate) use rendered_output::{
    RuntimeEvent, SlotOutput, extract_script_blocks, parse_harness_output,
};

#[cfg(test)]
use super::GoldenMode;
#[cfg(test)]
use super::types::GoldenExpectation;
#[cfg(test)]
use super::types::RenderedOutputExpectation;
use super::{
    BackendId, CaseExecutionResult, FailureExpectation, FailureKind, SuccessExpectation,
    TestCaseSpec,
};
use crate::build_system::build::BuildResult;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerMessages;
use std::collections::BTreeMap;
use std::path::Path;

/// Unordered-code difference shared by diagnostic and warning assertions.
pub(super) struct CodeMultisetDifference {
    pub missing: BTreeMap<String, (usize, usize)>,
    pub unexpected: BTreeMap<String, (usize, usize)>,
    pub count_mismatches: BTreeMap<String, (usize, usize)>,
}

/// Compares stable codes without assigning diagnostic- or warning-specific policy or wording.
pub(super) fn compare_exact_code_multisets<'code>(
    expected_codes: impl IntoIterator<Item = &'code str>,
    actual_codes: impl IntoIterator<Item = &'code str>,
) -> Option<CodeMultisetDifference> {
    let expected_counts = count_code_multiset(expected_codes);
    let actual_counts = count_code_multiset(actual_codes);

    let mut missing = BTreeMap::new();
    let mut unexpected = BTreeMap::new();
    let mut count_mismatches = BTreeMap::new();

    for (code, expected_count) in &expected_counts {
        match actual_counts.get(code) {
            None => {
                missing.insert((*code).to_owned(), (*expected_count, 0));
            }
            Some(actual_count) if actual_count != expected_count => {
                count_mismatches.insert((*code).to_owned(), (*expected_count, *actual_count));
            }
            Some(_) => {}
        }
    }

    for (code, actual_count) in &actual_counts {
        if !expected_counts.contains_key(code) {
            unexpected.insert((*code).to_owned(), (0, *actual_count));
        }
    }

    if missing.is_empty() && unexpected.is_empty() && count_mismatches.is_empty() {
        return None;
    }

    Some(CodeMultisetDifference {
        missing,
        unexpected,
        count_mismatches,
    })
}

fn count_code_multiset<'code>(
    codes: impl IntoIterator<Item = &'code str>,
) -> BTreeMap<&'code str, usize> {
    let mut counts = BTreeMap::new();
    for code in codes {
        *counts.entry(code).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
pub(crate) fn normalize_text_for_comparison(text: &str) -> String {
    goldens::normalize_text_for_comparison(text)
}

#[cfg(test)]
pub(crate) fn compare_text_golden(
    expected: &str,
    actual: &str,
    mode: GoldenMode,
) -> Option<String> {
    goldens::compare_text_golden(expected, actual, mode)
}

#[cfg(test)]
pub(crate) fn validate_golden_outputs(
    build_result: &BuildResult,
    golden: &GoldenExpectation,
) -> Option<(String, FailureKind)> {
    goldens::validate_golden_outputs(build_result, golden)
}

#[cfg(test)]
pub(crate) fn validate_rendered_output_fragments(
    rendered_output: &str,
    expectation: &RenderedOutputExpectation,
) -> Option<(String, FailureKind)> {
    rendered_output::validate_rendered_output_fragments(rendered_output, expectation)
}

pub(crate) fn validate_success_result(
    case: &TestCaseSpec,
    build_result: BuildResult,
    expectation: &SuccessExpectation,
) -> CaseExecutionResult {
    if let Some(reason) =
        warnings::validate_warning_expectation(build_result.warnings.iter(), &expectation.warnings)
    {
        return fail(build_result, reason, FailureKind::ExpectationViolation);
    }

    if case.backend_id == BackendId::Html
        && let Some(reason) = artifacts::validate_html_baseline_contract(&build_result)
    {
        return fail(build_result, reason, FailureKind::ExpectationViolation);
    }

    if case.backend_id == BackendId::HtmlWasm
        && let Some(reason) = wasm::validate_html_wasm_baseline_contract(&build_result)
    {
        return fail(build_result, reason, FailureKind::ExpectationViolation);
    }

    if let Some(reason) =
        artifacts::validate_artifact_assertions(&build_result, &expectation.artifact_assertions)
    {
        return fail(build_result, reason, FailureKind::ExpectationViolation);
    }

    if let Some(reason) = artifacts::validate_artifacts_must_not_exist(
        &build_result,
        &expectation.artifacts_must_not_exist,
    ) {
        return fail(build_result, reason, FailureKind::ExpectationViolation);
    }

    if let Some((reason, kind)) =
        goldens::validate_golden_outputs(&build_result, &expectation.golden)
    {
        return fail(build_result, reason, kind);
    }

    if expectation.rendered_output.is_present() {
        let rendered_output_result = if case.backend_id == BackendId::HtmlWasm {
            rendered_output::validate_wasm_rendered_output(
                &build_result,
                &expectation.rendered_output,
            )
        } else {
            rendered_output::validate_rendered_output(&build_result, &expectation.rendered_output)
        };
        if let Some((reason, kind)) = rendered_output_result {
            return fail(build_result, reason, kind);
        }
    }

    CaseExecutionResult {
        passed: true,
        panic_message: None,
        build_result: Some(build_result),
        messages: None,
        failure_reason: None,
        failure_kind: None,
    }
}

fn fail(build_result: BuildResult, reason: String, kind: FailureKind) -> CaseExecutionResult {
    CaseExecutionResult {
        passed: false,
        panic_message: None,
        build_result: Some(build_result),
        messages: None,
        failure_reason: Some(reason),
        failure_kind: Some(kind),
    }
}

pub(crate) fn validate_failure_result(
    messages: CompilerMessages,
    expectation: &FailureExpectation,
    fixture_root: &Path,
) -> CaseExecutionResult {
    if let Some(reason) =
        warnings::validate_warning_expectation(messages.warnings(), &expectation.warnings)
    {
        return failure_messages(messages, reason);
    }

    if let Some(reason) = diagnostics::validate_diagnostics(&messages, expectation, fixture_root) {
        return failure_messages(messages, reason);
    }

    CaseExecutionResult {
        passed: true,
        panic_message: None,
        build_result: None,
        messages: Some(messages),
        failure_reason: None,
        failure_kind: None,
    }
}

fn failure_messages(messages: CompilerMessages, reason: String) -> CaseExecutionResult {
    CaseExecutionResult {
        passed: false,
        panic_message: None,
        build_result: None,
        messages: Some(messages),
        failure_reason: Some(reason),
        failure_kind: Some(FailureKind::ExpectationViolation),
    }
}

/// Checks whether every fragment appears in order without requiring adjacency.
fn contains_ordered_substrings(text: &str, substrings: &[String]) -> bool {
    let mut offset = 0usize;

    for substring in substrings {
        let Some(position) = text[offset..].find(substring) else {
            return false;
        };
        offset += position + substring.len();
    }

    true
}
