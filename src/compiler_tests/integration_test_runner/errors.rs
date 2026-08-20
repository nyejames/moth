//! Typed failure identity for integration fixture loading and runner harness failures.
//!
//! WHAT: gives the String-based `Result` boundaries in fixture/expectation loading and the
//!       integration runner a coarse structured `kind` alongside the rendered `message`.
//! WHY: the integration harness has several distinct failure boundaries. A bare `String`
//!      forces tests to identify the boundary by matching rendered prose, which is fragile
//!      (a substring can match the fixture path) and accepts the wrong failure lane. Keeping
//!      a coarse stable kind beside the human message lets tests prove which boundary rejected
//!      while rendering tests may still assert on prose.

use std::fmt;

/// A rejected fixture or expectation load with a coarse boundary identity.
///
/// `message` is the human-readable, pre-rendered description used for display; `kind` gives
/// the failure a structured identity so callers and tests need not reparse prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureLoadError {
    /// The boundary that rejected the load.
    pub kind: FixtureLoadErrorKind,
    /// Human-readable description for display and diagnostics.
    pub message: String,
}

/// Identifies the boundary that rejected fixture or expectation loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureLoadErrorKind {
    /// The filesystem rejected a read, metadata lookup or canonicalisation step.
    Filesystem,
    /// The manifest was malformed or violated its contract.
    Manifest,
    /// An expectation file failed TOML deserialization.
    ExpectationParse,
    /// An expectation file violated a semantic expectation contract.
    ExpectationContract,
    /// A fixture folder violated its structural contract.
    FixtureContract,
    /// A resolved path escaped its required containment boundary.
    PathBoundary,
}

impl FixtureLoadError {
    pub(crate) fn filesystem(message: String) -> Self {
        Self {
            kind: FixtureLoadErrorKind::Filesystem,
            message,
        }
    }

    pub(crate) fn manifest(message: String) -> Self {
        Self {
            kind: FixtureLoadErrorKind::Manifest,
            message,
        }
    }

    pub(crate) fn expectation_parse(message: String) -> Self {
        Self {
            kind: FixtureLoadErrorKind::ExpectationParse,
            message,
        }
    }

    pub(crate) fn expectation_contract(message: String) -> Self {
        Self {
            kind: FixtureLoadErrorKind::ExpectationContract,
            message,
        }
    }

    pub(crate) fn fixture_contract(message: String) -> Self {
        Self {
            kind: FixtureLoadErrorKind::FixtureContract,
            message,
        }
    }

    pub(crate) fn path_boundary(message: String) -> Self {
        Self {
            kind: FixtureLoadErrorKind::PathBoundary,
            message,
        }
    }
}

impl fmt::Display for FixtureLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// A runner harness failure with a coarse boundary identity.
///
/// `message` is the human-readable, pre-rendered description used for display; `kind` gives
/// the failure a stable identity so tests can prove which runner lane rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRunnerError {
    /// The runner boundary that failed.
    pub kind: TestRunnerErrorKind,
    /// Human-readable description for display and diagnostics.
    pub message: String,
}

/// Identifies the runner boundary that rejected an integration run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestRunnerErrorKind {
    /// Option validation rejected the invocation.
    Options,
    /// Suite policy produced a hard finding.
    SuitePolicy,
    /// Suite inventory report persistence failed.
    InventoryReport,
    /// Failure triage report persistence failed.
    TriageReport,
    /// Selection matched no cases.
    Selection,
    /// The rayon thread pool could not be created.
    ThreadPool,
    /// Fixture or expectation loading failed.
    Fixture,
}

impl TestRunnerError {
    pub(crate) fn options(message: String) -> Self {
        Self {
            kind: TestRunnerErrorKind::Options,
            message,
        }
    }

    pub(crate) fn suite_policy(message: String) -> Self {
        Self {
            kind: TestRunnerErrorKind::SuitePolicy,
            message,
        }
    }

    pub(crate) fn inventory_report(message: String) -> Self {
        Self {
            kind: TestRunnerErrorKind::InventoryReport,
            message,
        }
    }

    pub(crate) fn triage_report(message: String) -> Self {
        Self {
            kind: TestRunnerErrorKind::TriageReport,
            message,
        }
    }

    pub(crate) fn selection(message: String) -> Self {
        Self {
            kind: TestRunnerErrorKind::Selection,
            message,
        }
    }

    pub(crate) fn thread_pool(message: String) -> Self {
        Self {
            kind: TestRunnerErrorKind::ThreadPool,
            message,
        }
    }
}

impl From<FixtureLoadError> for TestRunnerError {
    fn from(error: FixtureLoadError) -> Self {
        Self {
            kind: TestRunnerErrorKind::Fixture,
            message: error.message,
        }
    }
}

impl fmt::Display for TestRunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}
