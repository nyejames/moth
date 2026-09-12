//! Low-level Moth path normalization helpers.
//!
//! These helpers translate already-tokenized `PathId` components into filesystem candidate
//! paths and public path values. They do not own dependency visibility, public-surface policy, or
//! diagnostic construction.

use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use std::path::{Path, PathBuf};

/// A source-file dependency candidate derived from one extensionless dependency path.
///
/// WHAT: carries the concrete filesystem candidate plus its typed source kind.
/// WHY: Stage 0 must keep Moth `.moth` and builder-supported kinds such as Moth template `.mtf`
///      distinct before later frontend stages choose the right preparation path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DependencyCandidate {
    pub(crate) path: PathBuf,
    pub(crate) kind: SourceFileKind,
    pub(crate) support: DependencyCandidateSupport,
    pub(crate) is_parent_fallback: bool,
}

/// Whether a dependency candidate can be used by the active builder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DependencyCandidateSupport {
    Supported,
    RecognizedButUnsupported,
}

/// WHAT: checks whether a dependency path contains any `..` components.
/// WHY: parent-directory traversal is not supported in Moth dependencies.
pub(crate) fn dependency_contains_dotdot(
    dependency_path: PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> bool {
    let mut scratch = Vec::new();
    path_fork
        .resolve_components(dependency_path, &mut scratch)
        .iter()
        .any(|component| string_table.resolve(*component) == "..")
}

pub(crate) fn is_relative_dependency_path(
    dependency_path: PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> bool {
    let mut scratch = Vec::new();
    matches!(
        path_fork
            .resolve_components(dependency_path, &mut scratch)
            .first()
            .map(|component| string_table.resolve(*component)),
        Some(".") | Some("..")
    )
}

pub(crate) fn join_and_normalize_path(
    base: &Path,
    dependency_path: PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> PathBuf {
    let mut scratch = Vec::new();
    let components = path_fork.resolve_components(dependency_path, &mut scratch);
    join_and_normalize_components(base, components, string_table)
}

pub(crate) fn join_and_normalize_components(
    base: &Path,
    components: &[StringId],
    string_table: &StringTable,
) -> PathBuf {
    let mut joined = base.to_path_buf();

    for component in components {
        match string_table.resolve(*component) {
            "." => {}
            ".." => {
                joined.pop();
            }
            segment => joined.push(segment),
        }
    }

    joined
}

pub(crate) fn candidate_dependency_files_for_source_kinds(
    normalized_dependency_path: &Path,
    dependency_component_len: usize,
    source_file_kinds: &SourceFileKindRegistry,
) -> Vec<DependencyCandidate> {
    let mut candidates = Vec::new();

    add_source_kind_candidates(
        &mut candidates,
        normalized_dependency_path.to_path_buf(),
        false,
        source_file_kinds,
    );

    if dependency_component_len > 1
        && let Some(parent) = normalized_dependency_path.parent()
    {
        add_source_kind_candidates(
            &mut candidates,
            parent.to_path_buf(),
            true,
            source_file_kinds,
        );
    }

    candidates
}

fn add_source_kind_candidates(
    candidates: &mut Vec<DependencyCandidate>,
    base_path: PathBuf,
    is_parent_fallback: bool,
    source_file_kinds: &SourceFileKindRegistry,
) {
    for recognized in SourceFileKind::recognized_kinds() {
        let path = with_extension(base_path.clone(), recognized.extension);
        let support = if source_file_kinds.supports_recognized_extension(recognized.extension) {
            DependencyCandidateSupport::Supported
        } else {
            DependencyCandidateSupport::RecognizedButUnsupported
        };

        candidates.push(DependencyCandidate {
            path,
            kind: recognized.kind,
            support,
            is_parent_fallback,
        });
    }
}

fn with_extension(path: PathBuf, extension: &str) -> PathBuf {
    if path.extension().and_then(|value| value.to_str()) == Some(extension) {
        path
    } else {
        path.with_extension(extension)
    }
}
