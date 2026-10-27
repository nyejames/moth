//! Narrow string interning for command-owned diagnostics.
//!
//! Backend and project diagnostic producers may need to add payload text to the command's root
//! identity table. Keep that capability distinct from the immutable table view used by lookup-only
//! consumers.

use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

pub struct DiagnosticStringBuilder<'a> {
    strings: &'a mut StringTable,
}

impl<'a> DiagnosticStringBuilder<'a> {
    pub(crate) fn new(strings: &'a mut StringTable) -> Self {
        Self { strings }
    }

    /// Intern one diagnostic payload string in the command-owned identity domain.
    pub(crate) fn intern(&mut self, text: &str) -> StringId {
        self.strings.intern(text)
    }

    /// Intern one owned diagnostic payload string in the command-owned identity domain.
    pub(crate) fn get_or_intern(&mut self, text: String) -> StringId {
        self.strings.get_or_intern(text)
    }

    /// Borrow the immutable lookup view paired with this diagnostic builder.
    pub(crate) fn strings(&self) -> &StringTable {
        self.strings
    }
}
