//! Dependency-resolution diagnostics and validation helpers.
//!
//! `ProjectPathResolver` owns the public dependency-resolution entry point, while this module owns the
//! dependency-specific boundary error and validation rules that are independent of resolver state.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidImportPathReason};
use crate::compiler_frontend::paths::compile_time_paths::CompileTimePathBase;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
use std::path::Path;

/// Failure while resolving a dependency path.
///
/// User-facing dependency failures remain plain `CompilerDiagnostic` values;
/// filesystem and internal failures remain typed `CompilerError` values.
#[derive(Clone, Debug)]
pub(crate) enum DependencyPathResolutionError {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(CompilerError),
}

impl From<CompilerError> for DependencyPathResolutionError {
    fn from(error: CompilerError) -> Self {
        DependencyPathResolutionError::Infrastructure(error)
    }
}

/// WHAT: rejects dependency paths that escape their resolved base directory.
/// WHY: dependencies must stay within the project root (relative/entry) or package root.
pub(crate) fn validate_dependency_boundary(
    canonical_file: &Path,
    base_kind: &CompileTimePathBase,
    filesystem_base: &Path,
    dependency_path: PathId,
    path_fork: &PathInternerFork,
) -> Result<(), DependencyPathResolutionError> {
    let canonical_base =
        fs::canonicalize(filesystem_base).unwrap_or_else(|_| filesystem_base.to_path_buf());
    validate_dependency_boundary_against_base(
        canonical_file,
        base_kind,
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
    path_fork: &PathInternerFork,
) -> Result<(), DependencyPathResolutionError> {
    if !target_path.starts_with(canonical_base) {
        let reason = match base_kind {
            CompileTimePathBase::SourcePackageRoot => {
                InvalidImportPathReason::EscapesSourcePackageRoot
            }
            _ => InvalidImportPathReason::EscapesProjectRoot,
        };
        let diagnostic =
            CompilerDiagnostic::invalid_import_path(dependency_path.clone(), reason, None);
        return Err(DependencyPathResolutionError::Diagnostic(diagnostic));
    }
    Ok(())
}

/// WHAT: validates that the dependency path casing matches the on-disk filesystem casing.
/// WHY: dependency paths are logically case-sensitive even on case-insensitive filesystems.
///
pub(crate) fn validate_dependency_case_sensitivity(
    dependency_path: PathId,
    path_fork: &PathInternerFork,
    base_kind: &CompileTimePathBase,
    filesystem_base: &Path,
    canonical_file: &Path,
    is_parent_fallback: bool,
    string_table: &mut StringTable,
) -> Result<(), DependencyPathResolutionError> {
    let canonical_base =
        fs::canonicalize(filesystem_base).unwrap_or_else(|_| filesystem_base.to_path_buf());
    let relative_canonical = match canonical_file.strip_prefix(&canonical_base) {
        Ok(relative) => relative,
        Err(_) => return Ok(()),
    };

    let relative_canonical = relative_canonical.with_extension("");
    let canonical_components = canonical_normal_components(&relative_canonical);

    let mut path_components = Vec::new();
    let path_components = path_fork.resolve_components(dependency_path, &mut path_components);
    let user_components: Vec<String> = match base_kind {
        CompileTimePathBase::SourcePackageRoot => path_components
            .iter()
            .skip(1)
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
            provided: string_table.intern(&provided),
            expected: string_table.intern(&expected),
        };
        let diagnostic =
            CompilerDiagnostic::invalid_import_path(dependency_path.clone(), reason, None);
        return Err(DependencyPathResolutionError::Diagnostic(diagnostic));
    }

    Ok(())
}

/// Compare an authored module-root-relative spelling with a canonical filesystem path.
///
/// WHAT: centralizes the exact-case policy shared by extensionless dependency candidates and
///       explicit file-value paths. The dependency path caller strips its selected extension
///       before calling this helper; file-value callers retain the complete filename.
/// WHY: case-insensitive hosts must not turn two distinct authored spellings into one graph fact.
#[cfg(test)]
pub(crate) fn exact_case_mismatch_for_components(
    authored_components: &[String],
    canonical_base: &Path,
    canonical_file: &Path,
    strip_final_extension: bool,
) -> Option<(String, String)> {
    let relative = canonical_file.strip_prefix(canonical_base).ok()?;
    let relative = if strip_final_extension {
        relative.with_extension("")
    } else {
        relative.to_path_buf()
    };
    let canonical_components = canonical_normal_components(&relative);
    first_case_mismatch(authored_components, &canonical_components)
}

fn canonical_normal_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(segment) => segment.to_str(),
            _ => None,
        })
        .map(str::to_owned)
        .collect()
}

fn first_case_mismatch(
    authored_components: &[String],
    canonical_components: &[String],
) -> Option<(String, String)> {
    if authored_components.len() != canonical_components.len() {
        return None;
    }

    authored_components
        .iter()
        .zip(canonical_components)
        .find_map(|(authored, canonical)| {
            (authored != canonical).then(|| (authored.clone(), canonical.clone()))
        })
}
