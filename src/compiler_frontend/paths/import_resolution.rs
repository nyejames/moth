//! Import-resolution diagnostics and validation helpers.
//!
//! `ProjectPathResolver` owns the public import-resolution entry point, while this module owns the
//! import-specific boundary error and validation rules that are independent of resolver state.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidImportPathReason};
use crate::compiler_frontend::paths::compile_time_paths::CompileTimePathBase;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::fs;
use std::path::Path;

/// Failure while resolving an import path.
///
/// WHAT: keeps user-facing import diagnostics separate from filesystem/internal failures.
/// WHY: Stage 0 source discovery needs to preserve typed import diagnostics without routing them
/// through the older internal-error transport.
///
/// The `Diagnostic` variant boxes `CompilerDiagnostic` because it is large enough to trigger
/// `clippy::result_large_err` when stored inline in the `Result` enum. Boxing keeps the error
/// variant small; callers unbox at existing plain-diagnostic accumulation boundaries.
#[derive(Clone, Debug)]
pub(crate) enum ImportPathResolutionError {
    Diagnostic(Box<CompilerDiagnostic>),
    Infrastructure(CompilerError),
}

impl From<CompilerError> for ImportPathResolutionError {
    fn from(error: CompilerError) -> Self {
        ImportPathResolutionError::Infrastructure(error)
    }
}

/// WHAT: rejects import paths that escape their resolved base directory.
/// WHY: imports must stay within the project root (relative/entry) or package root.
///
/// NOTE: `string_table` is only used to intern the importer file path for diagnostics.
pub(crate) fn validate_import_boundary(
    canonical_file: &Path,
    base_kind: &CompileTimePathBase,
    filesystem_base: &Path,
    import_path: &InternedPath,
    importer_file: &Path,
    string_table: &mut StringTable,
) -> Result<(), ImportPathResolutionError> {
    let canonical_base =
        fs::canonicalize(filesystem_base).unwrap_or_else(|_| filesystem_base.to_path_buf());

    validate_import_boundary_against_base(
        canonical_file,
        base_kind,
        &canonical_base,
        import_path,
        importer_file,
        string_table,
    )
}

fn validate_import_boundary_against_base(
    target_path: &Path,
    base_kind: &CompileTimePathBase,
    canonical_base: &Path,
    import_path: &InternedPath,
    importer_file: &Path,
    string_table: &mut StringTable,
) -> Result<(), ImportPathResolutionError> {
    if !target_path.starts_with(canonical_base) {
        let reason = match base_kind {
            CompileTimePathBase::SourcePackageRoot => {
                InvalidImportPathReason::EscapesSourcePackageRoot
            }
            _ => InvalidImportPathReason::EscapesProjectRoot,
        };

        let location = SourceLocation::from_path(importer_file, string_table);
        let diagnostic =
            CompilerDiagnostic::invalid_import_path(import_path.clone(), reason, location);
        return Err(ImportPathResolutionError::Diagnostic(Box::new(diagnostic)));
    }

    Ok(())
}

/// WHAT: validates that the import path casing matches the on-disk filesystem casing.
/// WHY: import paths are logically case-sensitive even on case-insensitive filesystems.
///
/// NOTE: `string_table` is used to intern case-mismatch strings for the diagnostic payload.
pub(crate) fn validate_import_case_sensitivity(
    import_path: &InternedPath,
    base_kind: &CompileTimePathBase,
    filesystem_base: &Path,
    canonical_file: &Path,
    is_parent_fallback: bool,
    importer_file: &Path,
    string_table: &mut StringTable,
) -> Result<(), ImportPathResolutionError> {
    let canonical_base =
        fs::canonicalize(filesystem_base).unwrap_or_else(|_| filesystem_base.to_path_buf());
    let relative_canonical = match canonical_file.strip_prefix(&canonical_base) {
        Ok(relative) => relative,
        Err(_) => return Ok(()),
    };

    let relative_canonical = relative_canonical.with_extension("");
    let canonical_components: Vec<String> = relative_canonical
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(segment) => segment.to_str(),
            _ => None,
        })
        .map(str::to_owned)
        .collect();

    let user_components: Vec<String> = match base_kind {
        CompileTimePathBase::SourcePackageRoot => import_path
            .as_components()
            .iter()
            .skip(1)
            .map(|component| string_table.resolve(*component))
            .map(str::to_owned)
            .collect(),
        CompileTimePathBase::RelativeToFile => import_path
            .as_components()
            .iter()
            .skip_while(|component| string_table.resolve(**component) == ".")
            .map(|component| string_table.resolve(*component))
            .map(str::to_owned)
            .collect(),
        CompileTimePathBase::EntryRoot => import_path
            .as_components()
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

    if user_file_components.len() != canonical_components.len() {
        return Ok(());
    }

    for (user, canonical) in user_file_components.iter().zip(canonical_components.iter()) {
        if user != canonical {
            let location = SourceLocation::from_path(importer_file, string_table);
            let reason = InvalidImportPathReason::CaseMismatch {
                provided: string_table.intern(user),
                expected: string_table.intern(canonical),
            };
            let diagnostic =
                CompilerDiagnostic::invalid_import_path(import_path.clone(), reason, location);
            return Err(ImportPathResolutionError::Diagnostic(Box::new(diagnostic)));
        }
    }

    Ok(())
}
