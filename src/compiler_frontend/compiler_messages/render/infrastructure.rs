//! Cause-derived infrastructure presentation shared by terminal, terse and HTML output.
//!
//! Renderers use the typed cause kind, stage, host path, message and details without consulting
//! source identities or changing the stored failure.

use crate::compiler_frontend::compiler_errors::{
    InfrastructureClass, InfrastructureDetail, InfrastructureFailure,
};

pub(crate) const ISSUE_URL: &str = "https://github.com/nyejames/moth/issues";

/// The code and title of the infrastructure failure record. Infrastructure failures are not
/// diagnostic descriptors, so this constant is the single production spelling of the code; the
/// dev-server card, the terse record and the benchmark report all read it.
pub(crate) const INFRASTRUCTURE_FAILURE_CODE: &str = "MOTH-INFRA-0001";
pub(crate) const INFRASTRUCTURE_FAILURE_TITLE: &str = "Infrastructure failure";

impl InfrastructureClass {
    pub(crate) fn introduction(self) -> &'static str {
        match self {
            Self::CompilerBug => {
                "COMPILER BUG: Moth encountered an internal compiler or build-system error.\nThis is a Moth toolchain defect, not a source-code error. Please report it at:"
            }
            Self::Operational => {
                "OPERATIONAL FAILURE: Moth could not complete a filesystem, host or tooling operation."
            }
        }
    }
}

pub(crate) fn format_error_guidance_lines(error: &InfrastructureFailure) -> Vec<String> {
    let mut lines = vec![format!("Stage: {}", error.stage)];
    for detail in error.details.iter() {
        match detail {
            InfrastructureDetail::PrimarySuggestion(suggestion) => {
                lines.push(suggestion.to_string());
            }
            InfrastructureDetail::OutputRejectionReason(_) => {}
        }
    }
    lines
}
