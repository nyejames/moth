//! Compile-time path literal value contracts and resolution errors.
//!
//! Moth path literals are source-level values, not plain strings. These types carry the
//! resolved filesystem target, public rendering path, source spelling, and file/directory kind so
//! AST folding and backend rendering can share one representation.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_errors::compiler_error_to_diagnostic;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidCompileTimePathReason,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::{Path, PathBuf};

/// Whether a resolved compile-time path points at a file or a directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileTimePathKind {
    File,
    Directory,
}

/// How the path was resolved relative to the project layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileTimePathBase {
    /// Resolved relative to the importing file (`./` or `../`).
    RelativeToFile,
    /// First segment matched a source-backed package prefix.
    SourcePackageRoot,
    /// Fell through to the configured `entry_root`.
    EntryRoot,
}

/// A fully resolved compile-time path value.
///
/// WHAT: carries all semantic metadata the compiler needs for validation, typed representation,
/// and later string coercion of Moth path literals.
///
/// WHY: path literals must be first-class compile-time values so that origin prefix application,
/// file/directory distinction, and public-path formatting can be handled consistently in one
/// place.
#[derive(Clone, Debug)]
pub struct CompileTimePath {
    /// The original syntactic path as written in source, normalized to Moth components.
    /// Preserved for diagnostics and future path manipulation.
    pub source_path: InternedPath,

    /// The canonical filesystem path used for compile-time existence validation. This is an
    /// absolute path into the development tree.
    pub filesystem_path: PathBuf,

    /// The project-visible public path after resolution but before origin prefix application. This is
    /// the path that string coercion should render with an optional origin prefix.
    pub public_path: InternedPath,

    /// How the path resolved semantically. This determines whether the origin prefix is applied during
    /// string coercion.
    pub base: CompileTimePathBase,

    /// Whether the target is a file or a directory.
    pub kind: CompileTimePathKind,
}

/// A collection of one or more resolved compile-time path values.
///
/// WHAT: wraps multiple resolved paths from a single path expression.
/// WHY: grouped path syntax (`@dir {a, b}`) produces multiple paths from one token. This type
/// carries them as a unit so expressions and string coercion can handle the 1-or-many case
/// uniformly.
#[derive(Clone, Debug)]
pub struct CompileTimePaths {
    pub paths: Vec<CompileTimePath>,
}

/// Failure while resolving a general compile-time path literal.
///
/// WHAT: keeps source-authored path mistakes typed while preserving true filesystem/internal
/// failures as infrastructure.
/// WHY: path literals are user-facing language surface, so missing targets and semantic escapes
/// must not travel through `CompilerError`.
///
/// The `Diagnostic` variant boxes `CompilerDiagnostic` because it is large enough to trigger
/// `clippy::result_large_err` when stored inline in the `Result` enum. Boxing keeps the error
/// variant small; callers unbox at existing plain-diagnostic accumulation boundaries.
#[derive(Clone, Debug)]
pub(crate) enum CompileTimePathResolutionError {
    Diagnostic(Box<CompilerDiagnostic>),
    Infrastructure(CompilerError),
}

impl CompileTimePathResolutionError {
    pub(crate) fn into_diagnostic(self) -> CompilerDiagnostic {
        match self {
            CompileTimePathResolutionError::Diagnostic(diagnostic) => *diagnostic,
            CompileTimePathResolutionError::Infrastructure(error) => {
                compiler_error_to_diagnostic(&error)
            }
        }
    }
}

impl From<CompilerError> for CompileTimePathResolutionError {
    fn from(error: CompilerError) -> Self {
        CompileTimePathResolutionError::Infrastructure(error)
    }
}

impl From<CompileTimePathResolutionError> for CompilerDiagnostic {
    fn from(error: CompileTimePathResolutionError) -> Self {
        error.into_diagnostic()
    }
}

/// WHAT: checks that the resolved filesystem target exists and classifies it.
/// WHY: compile-time path validation requires the target to exist.
///
/// NOTE: `string_table` is only used to intern the importer file path for diagnostics.
pub(crate) fn classify_existing_target(
    filesystem_path: &Path,
    source_path: &InternedPath,
    importer_file: &Path,
    string_table: &mut StringTable,
) -> Result<CompileTimePathKind, CompileTimePathResolutionError> {
    if filesystem_path.is_file() {
        Ok(CompileTimePathKind::File)
    } else if filesystem_path.is_dir() {
        Ok(CompileTimePathKind::Directory)
    } else {
        let location = SourceLocation::from_path(importer_file, string_table);
        let diagnostic = CompilerDiagnostic::invalid_compile_time_path(
            source_path.clone(),
            InvalidCompileTimePathReason::MissingTarget,
            location,
        );

        Err(CompileTimePathResolutionError::Diagnostic(Box::new(
            diagnostic,
        )))
    }
}
