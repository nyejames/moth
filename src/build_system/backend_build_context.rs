//! Narrow backend access to command-owned build diagnostics.
//!
//! Backend builders receive immutable compiler identity lookup and controlled diagnostic-owned
//! string interning through this context.

use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::symbols::string_interning::StringTable;

pub struct BackendBuildContext<'a> {
    diagnostics: &'a mut BoundaryAccumulator,
}

impl<'a> BackendBuildContext<'a> {
    pub(crate) fn new(diagnostics: &'a mut BoundaryAccumulator) -> Self {
        Self { diagnostics }
    }

    /// Borrow immutable compiler identity for backend lookups.
    pub(crate) fn strings(&self) -> &StringTable {
        self.diagnostics.string_table()
    }

    /// Borrow the command-owned diagnostic-only interner.
    pub(crate) fn diagnostic_strings(&mut self) -> DiagnosticStringBuilder<'_> {
        DiagnosticStringBuilder::new(self.diagnostics.string_table_mut())
    }
}
