//! Narrow string interning for command-owned diagnostics.
//!
//! Backend and project diagnostic producers may need to add payload text to the command's root
//! identity table, including path facts. Paths render through the sole portable display formatter
//! so their text stays stable across host platforms. Keep that capability distinct from the
//! immutable table view used by lookup-only consumers.

use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;
use std::path::Path;

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

    /// Intern one diagnostic path fact in the command-owned identity domain.
    ///
    /// The path is rendered through the sole portable display formatter, so the stored text is
    /// platform-neutral. The native `Path` remains authoritative for IO and path identity.
    pub(crate) fn intern_path(&mut self, path: &Path) -> StringId {
        self.get_or_intern(portable_path_text(path))
    }

    /// Borrow the immutable lookup view paired with this diagnostic builder.
    pub(crate) fn strings(&self) -> &StringTable {
        self.strings
    }
}
