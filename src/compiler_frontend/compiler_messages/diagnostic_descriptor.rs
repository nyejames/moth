//! Stable diagnostic family descriptors.
//!
//! WHAT: binds a family row to its stable code, short title, and default severity.
//! WHY: diagnostic codes and titles are user-visible contracts, independent of internal family layouts.

use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiagnosticDescriptor {
    pub code: &'static str,
    pub title: &'static str,
    pub default_severity: DiagnosticSeverity,
}

impl DiagnosticDescriptor {
    pub(crate) const fn new(
        code: &'static str,
        title: &'static str,
        default_severity: DiagnosticSeverity,
    ) -> Self {
        Self {
            code,
            title,
            default_severity,
        }
    }
}
