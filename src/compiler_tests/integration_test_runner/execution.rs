//! Case execution for the integration test suite.
//!
//! WHAT: runs a single `TestCaseSpec` through `build_project` and captures the outcome.
//! WHY: isolating execution here keeps panic handling, builder selection, and flag injection
//!      in one place so the orchestrator only has to call one function per case.

use super::{BackendId, CaseExecutionResult, ExpectedOutcome, FailureKind, TestCaseSpec};
use crate::build_system::build::{ProjectBuilder, build_project};
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

pub(crate) fn execute_test_case(case: &TestCaseSpec) -> CaseExecutionResult {
    let builder = backend_builder_for_case(case.backend_id);
    let flags = case.flags.clone();

    // `build_project` takes a UTF-8 entry path. Converting lossily would hand the compiler a
    // different path than the fixture owns, so an unrepresentable entry is a harness failure.
    let Some(entry_path) = case.entry_path.to_str() else {
        return CaseExecutionResult {
            passed: false,
            panic_message: None,
            build_result: None,
            messages: None,
            failure_reason: Some(format!(
                "Case entry path {:?} is not valid UTF-8, and the build entry protocol requires UTF-8.",
                case.entry_path
            )),
            failure_kind: Some(FailureKind::HarnessFailed),
        };
    };

    let execution = catch_unwind(AssertUnwindSafe(|| {
        build_project(&builder, entry_path, &flags, &BuildConfigInputSet::new())
    }));

    // Policy: unsupported or incomplete user input must surface structured compiler diagnostics.
    // Panics are always treated as failing outcomes and are only captured for robustness/triage.
    match execution {
        Ok(build_result) => match &case.expected {
            ExpectedOutcome::Success(expectation) => match build_result {
                Ok(build_result) => {
                    super::assertions::validate_success_result(case, build_result, expectation)
                }
                Err(messages) => CaseExecutionResult {
                    passed: false,
                    panic_message: None,
                    build_result: None,
                    messages: Some(messages),
                    failure_reason: Some(
                        "Expected a successful build, but compilation failed.".to_string(),
                    ),
                    failure_kind: Some(FailureKind::ExpectationViolation),
                },
            },
            ExpectedOutcome::Failure(expectation) => match build_result {
                Ok(build_result) => CaseExecutionResult {
                    passed: false,
                    panic_message: None,
                    build_result: Some(build_result),
                    messages: None,
                    failure_reason: Some(
                        "Expected a compilation failure, but the case built successfully."
                            .to_string(),
                    ),
                    failure_kind: Some(FailureKind::ExpectationViolation),
                },
                Err(messages) => super::assertions::validate_failure_result(
                    messages,
                    expectation,
                    &case.fixture_root,
                ),
            },
        },
        Err(payload) => panic_case_result(payload),
    }
}

pub(crate) fn panic_case_result(payload: Box<dyn Any + Send>) -> CaseExecutionResult {
    CaseExecutionResult {
        passed: false,
        panic_message: Some(format_panic_payload(payload)),
        build_result: None,
        messages: None,
        failure_reason: Some("The compiler panicked while running this case.".to_string()),
        failure_kind: Some(FailureKind::HarnessFailed),
    }
}

fn backend_builder_for_case(_backend_id: BackendId) -> ProjectBuilder {
    // This backend-builder seam is explicit even though both current backends
    // route through the HTML builder, so future non-HTML backends can slot in cleanly.
    ProjectBuilder::new(Box::new(HtmlProjectBuilder::for_integration_tests()))
}

fn format_panic_payload(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => (*message).to_string(),
            Err(_) => "non-string panic payload".to_string(),
        },
    }
}
