//! Case execution for the integration test suite.
//!
//! WHAT: executes one case inside a worker that owns its build context through finalization and
//!       expectation checks.
//! WHY: only the frozen case outcome crosses the unwind boundary, so a panic cannot leak mutable
//!      compiler state into the runner.

use super::{BackendId, CaseExecutionResult, ExpectedOutcome, FailureKind, TestCaseSpec};
use crate::build_system::build::{BuilderSelection, build_project};
use crate::compiler_frontend::build_config::BuildConfigInputSet;
use crate::compiler_frontend::compiler_errors::CompilationOutcome;
use crate::projects::html_project::html_project_builder::HtmlProjectBuilder;
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

pub(crate) fn execute_test_case(case: &TestCaseSpec) -> CaseExecutionResult {
    execute_test_case_with_completion_hook(case, backend_builder_for_case(case.backend_id), || {})
}

pub(crate) fn execute_test_case_with_completion_hook(
    case: &TestCaseSpec,
    builder_selection: BuilderSelection,
    after_worker_drop: impl FnOnce(),
) -> CaseExecutionResult {
    let case = case.clone();
    let entry_path = match absolute_entry_path(&case.entry_path) {
        Ok(entry_path) => entry_path,
        Err(reason) => {
            return failed_case_result(reason, FailureKind::HarnessFailed);
        }
    };

    let build_config_inputs = BuildConfigInputSet::new();
    // The worker owns the case request and build state, and is discarded after this case.
    let execution = catch_unwind(AssertUnwindSafe(move || {
        CaseWorker {
            case,
            entry_path,
            build_config_inputs,
            builder: builder_selection,
        }
        .run()
    }));
    after_worker_drop();
    match execution {
        Ok(result) => result,
        Err(payload) => panic_case_result(payload),
    }
}

fn absolute_entry_path(entry_path: &Path) -> Result<PathBuf, String> {
    if entry_path.is_absolute() {
        return Ok(entry_path.to_path_buf());
    }

    std::env::current_dir()
        .map(|current_directory| current_directory.join(entry_path))
        .map_err(|error| format!("Could not make case entry path {entry_path:?} absolute: {error}"))
}

/// Owns one case's request and build state so no mutable compiler context crosses its unwind boundary.
struct CaseWorker {
    case: TestCaseSpec,
    entry_path: PathBuf,
    build_config_inputs: BuildConfigInputSet,
    builder: BuilderSelection,
}

impl CaseWorker {
    fn run(self) -> CaseExecutionResult {
        let result = self.build_and_check();
        drop(self);
        result
    }

    fn build_and_check(&self) -> CaseExecutionResult {
        let Some(entry_path) = self.entry_path.to_str() else {
            return failed_case_result(
                format!(
                    "Case entry path {:?} is not valid UTF-8, and the build entry protocol requires UTF-8.",
                    self.entry_path
                ),
                FailureKind::HarnessFailed,
            );
        };

        let build_result = match build_project(
            self.builder.backend.as_ref(),
            entry_path,
            &self.case.flags,
            &self.build_config_inputs,
        )
        .unwrap_or_else(|never| match never {})
        {
            Ok(CompilationOutcome::Success { value, .. }) => value.finish_output(Ok(())),
            result => result,
        };

        match &self.case.expected {
            ExpectedOutcome::Success(expectation) => match build_result {
                Ok(CompilationOutcome::Success { value, warnings }) => {
                    super::assertions::validate_success_result(
                        &self.case,
                        value,
                        warnings,
                        expectation,
                    )
                }
                Ok(CompilationOutcome::Diagnosed(reports)) => CaseExecutionResult {
                    passed: false,
                    panic_message: None,
                    messages: Some(Ok(CompilationOutcome::Diagnosed(reports))),
                    failure_reason: Some(
                        "Expected a successful build, but compilation failed.".to_string(),
                    ),
                    failure_kind: Some(FailureKind::ExpectationViolation),
                },
                Err(errors) => CaseExecutionResult {
                    passed: false,
                    panic_message: None,
                    messages: Some(Err(errors)),
                    failure_reason: Some(
                        "Expected a successful build, but compilation failed.".to_string(),
                    ),
                    failure_kind: Some(FailureKind::ExpectationViolation),
                },
            },
            ExpectedOutcome::Failure(expectation) => match build_result {
                Ok(CompilationOutcome::Success { value, warnings }) => {
                    drop(value);
                    CaseExecutionResult {
                        passed: false,
                        panic_message: None,
                        messages: warnings.map(|reports| {
                            Ok(CompilationOutcome::Success {
                                value: (),
                                warnings: Some(reports),
                            })
                        }),
                        failure_reason: Some(
                            "Expected a compilation failure, but the case built successfully."
                                .to_string(),
                        ),
                        failure_kind: Some(FailureKind::ExpectationViolation),
                    }
                }
                Ok(CompilationOutcome::Diagnosed(reports)) => {
                    super::assertions::validate_failure_result(
                        Ok(CompilationOutcome::Diagnosed(reports)),
                        expectation,
                        &self.case.fixture_root,
                    )
                }
                Err(errors) => super::assertions::validate_failure_result(
                    Err(errors),
                    expectation,
                    &self.case.fixture_root,
                ),
            },
        }
    }
}

pub(crate) fn panic_case_result(payload: Box<dyn Any + Send>) -> CaseExecutionResult {
    let panic_message = crate::render_compiler_bug_payload(payload.as_ref()).unwrap_or_else(|| {
        "Compiler bug while running this case; details were emitted by the panic hook.".to_string()
    });
    CaseExecutionResult {
        passed: false,
        panic_message: Some(panic_message),
        messages: None,
        failure_reason: Some("The compiler panicked while running this case.".to_string()),
        failure_kind: Some(FailureKind::HarnessFailed),
    }
}

fn failed_case_result(reason: String, failure_kind: FailureKind) -> CaseExecutionResult {
    CaseExecutionResult {
        passed: false,
        panic_message: None,
        messages: None,
        failure_reason: Some(reason),
        failure_kind: Some(failure_kind),
    }
}

fn backend_builder_for_case(_backend_id: BackendId) -> BuilderSelection {
    // This backend-builder seam is explicit even though both current backends
    // route through the HTML builder, so future non-HTML backends can slot in cleanly.
    BuilderSelection::new(HtmlProjectBuilder::for_integration_tests())
}
