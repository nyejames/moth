//! Low-level Moth path normalization helpers.
//!
//! These helpers translate already-tokenized `InternedPath` components into filesystem candidate
//! paths and public path values. They do not own dependency visibility, public-surface policy, or
//! diagnostic construction.

use crate::builder_surface::{SourceFileKind, SourceFileKindRegistry};
use crate::compiler_frontend::paths::compile_time_paths::CompileTimePathBase;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
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
    dependency_path: &InternedPath,
    string_table: &StringTable,
) -> bool {
    dependency_path
        .as_components()
        .iter()
        .any(|component| string_table.resolve(*component) == "..")
}

pub(crate) fn is_relative_dependency_path(
    dependency_path: &InternedPath,
    string_table: &StringTable,
) -> bool {
    matches!(
        dependency_path
            .as_components()
            .first()
            .map(|component| string_table.resolve(*component)),
        Some(".") | Some("..")
    )
}

pub(crate) fn join_and_normalize_path(
    base: &Path,
    dependency_path: &InternedPath,
    string_table: &StringTable,
) -> PathBuf {
    let mut joined = base.to_path_buf();

    for component in dependency_path.as_components() {
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

/// WHAT: builds the project-visible public path from a resolved path literal.
/// WHY: the public path is what string coercion renders; it differs from the filesystem path by
/// stripping the base and keeping the user-visible segments.
pub(crate) fn build_public_path(
    source_path: &InternedPath,
    base_kind: &CompileTimePathBase,
    string_table: &StringTable,
) -> InternedPath {
    match base_kind {
        // Relative paths keep their original form as the public path.
        CompileTimePathBase::RelativeToFile => source_path.clone(),

        // Source-backed package and entry-root paths keep the visible segments. For source-backed package paths
        // the first segment is the package prefix, which must be preserved. For entry-root paths,
        // all segments are visible. In both cases the source path already contains the correct
        // visible segments, so we can reuse it directly.
        CompileTimePathBase::SourcePackageRoot | CompileTimePathBase::EntryRoot => {
            // Strip leading `.` or `..` defensively; these should not be present for non-relative
            // paths.
            let components = source_path.as_components();
            let skip = components
                .iter()
                .take_while(|component| {
                    let segment = string_table.resolve(**component);
                    segment == "." || segment == ".."
                })
                .count();

            if skip == 0 {
                source_path.clone()
            } else {
                InternedPath::from_components(components[skip..].to_vec())
            }
        }
    }
}

/// WHAT: best-effort canonicalization that works even when the leaf doesn't exist yet.
/// WHY: project-root validation needs a canonical path for prefix comparison, but the target file
/// may not exist. Missing target diagnostics are reported separately.
pub(crate) fn canonicalize_best_effort(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }

    let mut existing = path.to_path_buf();
    let mut tail_components: Vec<String> = Vec::new();

    while !existing.exists() {
        if let Some(name) = existing.file_name().and_then(|name| name.to_str()) {
            tail_components.push(name.to_owned());
        }
        if !existing.pop() {
            return path.to_path_buf();
        }
    }

    let mut result = fs::canonicalize(&existing).unwrap_or(existing);
    for component in tail_components.iter().rev() {
        result.push(component);
    }

    result
}
