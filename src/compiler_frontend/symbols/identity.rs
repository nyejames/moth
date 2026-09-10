//! Frontend dependency identities.
//!
//! Source identities and their database belong to [`crate::compiler_frontend::source`]. This
//! module retains only the dependency identities that join authored dependency shells and their
//! selected names.

use crate::compiler_frontend::source::SourceId;

/// Build-local identity of one retained dependency shell inside one source file.
///
/// WHAT: pairs the source file's stable `SourceId` with the shell's ordinal within that file so
///       Stage 0 edges and header dependency shells join by identity instead of path text.
/// WHY: provider binding must not compare path components or suffixes; the header preparation
///      pass assigns one ID per retained shell and the graph keeps that exact ID on its edges.
///      Every retained shell carries a real source identity; synthetic tests obtain a real test
///      `SourceId` from test support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DependencyShellId {
    pub source: SourceId,
    pub ordinal: u32,
}

impl DependencyShellId {
    pub fn new(source: SourceId, ordinal: u32) -> Self {
        Self { source, ordinal }
    }
}

/// Identity of one selected name inside one retained dependency clause.
///
/// WHAT: pairs the authored clause's `DependencyShellId` with the selection's clause-local
///       index so public-interface projection can retain the exact selected binding.
/// WHY: provider resolution belongs to the whole shell. Only consumers that need to identify a
///      particular exported name carry this narrower identity; Stage 0 and provider joins use the
///      shell directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DependencySelectionId {
    pub shell: DependencyShellId,
    pub selected_index: u32,
}

impl DependencySelectionId {
    pub fn new(shell: DependencyShellId, selected_index: u32) -> Self {
        Self {
            shell,
            selected_index,
        }
    }
}
