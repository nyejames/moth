//! Dependency-resolution diagnostics and validation helpers.
//!
//! `ProjectPathResolver` owns the public dependency-resolution entry point, while this module owns the
//! dependency-specific boundary error and validation rules that are independent of resolver state.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::InvalidImportPathReason;
use crate::compiler_frontend::compiler_messages::diagnostics::diagnostic_string_builder::DiagnosticStringBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports;
use crate::compiler_frontend::paths::compile_time_paths::CompileTimePathBase;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::projects::settings::CONFIG_FILE_NAME;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Failure while resolving a dependency path.
///
/// User-facing dependency failures remain diagnostic drafts;
/// filesystem and internal failures remain typed `InfrastructureFailure` values.
#[derive(Debug)]
pub(crate) enum DependencyPathResolutionError {
    Diagnostic(DiagnosticDraft),
    Infrastructure(InfrastructureFailure),
}

impl From<InfrastructureFailure> for DependencyPathResolutionError {
    fn from(error: InfrastructureFailure) -> Self {
        DependencyPathResolutionError::Infrastructure(error)
    }
}

/// The filesystem base selected for one dependency path.
///
/// `matched_components` counts the leading authored components the base already represents: a
/// source package's complete registered prefix, or zero for entry-root and file-relative bases.
/// Joining and case validation skip exactly these components. Candidate selection counts a
/// matched prefix as one logical component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DependencyPathBase {
    pub(crate) kind: CompileTimePathBase,
    pub(crate) root: PathBuf,
    pub(crate) matched_components: usize,
}

/// Reject physical targets outside their owning boundary or reserved as build-only config.
pub(crate) fn validate_dependency_boundary(
    canonical_file: &Path,
    base: &DependencyPathBase,
    dependency_path: PathId,
    path_fork: &PathInternerFork,
) -> Result<(), DependencyPathResolutionError> {
    let canonical_base = fs::canonicalize(&base.root).unwrap_or_else(|_| base.root.clone());
    validate_dependency_boundary_against_base(
        canonical_file,
        &base.kind,
        &canonical_base,
        dependency_path,
        path_fork,
    )
}
fn validate_dependency_boundary_against_base(
    target_path: &Path,
    base_kind: &CompileTimePathBase,
    canonical_base: &Path,
    dependency_path: PathId,
    _path_fork: &PathInternerFork,
) -> Result<(), DependencyPathResolutionError> {
    if !target_path.starts_with(canonical_base) {
        let reason = match base_kind {
            CompileTimePathBase::SourcePackageRoot => {
                InvalidImportPathReason::EscapesSourcePackageRoot
            }
            _ => InvalidImportPathReason::EscapesProjectRoot,
        };
        let diagnostic = imports::invalid_import_path(dependency_path, reason, None);
        return Err(DependencyPathResolutionError::Diagnostic(diagnostic));
    }
    if target_path
        .file_name()
        .is_some_and(|name| name == CONFIG_FILE_NAME)
    {
        return Err(DependencyPathResolutionError::Diagnostic(
            imports::direct_special_file_import(dependency_path, None),
        ));
    }
    Ok(())
}

/// WHAT: validates that the dependency path casing matches the on-disk filesystem casing.
/// WHY: dependency paths are logically case-sensitive even on case-insensitive filesystems.
///      A canonical component that has no UTF-8 spelling cannot enter Moth identity, so it fails
///      through the filesystem lane instead of being dropped from the comparison.
pub(crate) fn validate_dependency_case_sensitivity(
    dependency_path: PathId,
    path_fork: &PathInternerFork,
    base: &DependencyPathBase,
    canonical_file: &Path,
    is_parent_fallback: bool,
    strings: &mut DiagnosticStringBuilder<'_>,
) -> Result<(), DependencyPathResolutionError> {
    let canonical_base = fs::canonicalize(&base.root).unwrap_or_else(|_| base.root.clone());
    let relative_canonical = match canonical_file.strip_prefix(&canonical_base) {
        Ok(relative) => relative,
        Err(_) => return Ok(()),
    };

    // Authored components are UTF-8 strings, so only a canonical component below the base can be
    // unrepresentable. It cannot enter Moth identity, and comparing around it would silently
    // shorten or misalign the component lists.
    let Some(canonical_components) = canonical_normal_components(relative_canonical) else {
        return Err(DependencyPathResolutionError::Infrastructure(
            InfrastructureFailure::filesystem(
                CompilerStage::DependencyResolution,
                canonical_file,
                format!(
                    "Resolved dependency target {canonical_file:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
                ),
            ),
        ));
    };

    let mut path_components = Vec::new();
    let path_components = path_fork.resolve_components(dependency_path, &mut path_components);
    let string_table = strings.strings();
    let user_components: Vec<String> = match base.kind {
        CompileTimePathBase::SourcePackageRoot => path_components
            .iter()
            .skip(base.matched_components)
            .map(|component| string_table.resolve(*component))
            .map(str::to_owned)
            .collect(),
        CompileTimePathBase::RelativeToFile => path_components
            .iter()
            .skip_while(|component| string_table.resolve(**component) == ".")
            .map(|component| string_table.resolve(*component))
            .map(str::to_owned)
            .collect(),
        CompileTimePathBase::EntryRoot => path_components
            .iter()
            .map(|component| string_table.resolve(*component))
            .map(str::to_owned)
            .collect(),
    };

    let user_file_components = if is_parent_fallback {
        if user_components.len() < 2 {
            return Ok(());
        }
        &user_components[..user_components.len() - 1]
    } else {
        &user_components[..]
    };

    if let Some((provided, expected)) =
        first_case_mismatch(user_file_components, &canonical_components)
    {
        let reason = InvalidImportPathReason::CaseMismatch {
            provided: strings.intern(provided),
            expected: strings.intern(expected),
        };
        let diagnostic = imports::invalid_import_path(dependency_path, reason, None);
        return Err(DependencyPathResolutionError::Diagnostic(diagnostic));
    }

    Ok(())
}

/// Collects the UTF-8 normal components of the canonical relative `path`, dropping the final
/// file-name extension.
///
/// `None` reports a normal component with no UTF-8 spelling: authored path components are strings,
/// so such a component cannot enter Moth identity and must not disappear from the comparison.
fn canonical_normal_components(path: &Path) -> Option<Vec<String>> {
    let mut components = Vec::new();
    for component in path.components() {
        if let Component::Normal(segment) = component {
            components.push(segment.to_str()?.to_owned());
        }
    }

    if let Some(file_name) = components.last_mut() {
        // The complete file name is already known to be UTF-8; removing an extension only
        // truncates it, so the remaining spelling stays UTF-8.
        let without_extension = Path::new(file_name.as_str()).with_extension("");
        let stripped_len = without_extension
            .to_str()
            .expect("removing an extension cannot make a UTF-8 file name invalid")
            .len();
        file_name.truncate(stripped_len);
    }

    Some(components)
}

fn first_case_mismatch<'a>(
    authored_components: &'a [String],
    canonical_components: &'a [String],
) -> Option<(&'a str, &'a str)> {
    if authored_components.len() != canonical_components.len() {
        return None;
    }

    authored_components
        .iter()
        .zip(canonical_components)
        .find_map(|(authored, canonical)| {
            (authored != canonical).then_some((authored.as_str(), canonical.as_str()))
        })
}
