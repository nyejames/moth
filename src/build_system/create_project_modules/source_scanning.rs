//! Single-pass source loading, tokenisation and structural-import extraction.
//!
//! Tokenizes a single source file and returns the import paths declared in it.
//! Source discovery retains the same `SourceDiscoveryError` boundary so syntax diagnostics and
//! file/tooling failures stay typed until the Stage 0 boundary.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::paths::const_paths::{
    ScannedProviderReference, collect_provider_references_from_tokens,
};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::{InternedPath, NonUtf8PathComponent};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenizerEntryMode};

use std::path::Path;

use super::source_discovery_error::SourceDiscoveryError;
use super::source_loading::extract_source_code;

/// Import scan output that keeps the already-read source available to Stage 0.
///
/// WHAT: pairs scanned provider references with the Moth source text used to discover
///      them.
/// WHY: reachable-file discovery consumes the references directly, using `path` for current
///      resolution while retaining `path_location` for the graph boundary, and reuses the source
///      when assembling `PreparedSourceInput` values instead of reading each scanned `.moth` file again.
#[derive(Clone)]
pub(super) struct ScannedImportSource {
    pub(super) imports: Vec<ScannedProviderReference>,
    pub(super) source_code: String,
    /// Exact token stream from the single Stage 0 lexical pass over this Moth file.
    ///
    /// WHAT: the `FileTokens` produced by the same `tokenize` call that discovered `imports`.
    /// WHY: frontend header preparation consumes this retained stream instead of re-tokenizing
    ///      the source text, so each discovered `.moth` file is lexed exactly once.
    pub(super) tokens: FileTokens,
}

pub(super) fn scan_imports_with_source(
    file_path: &Path,
    style_directives: &StyleDirectiveRegistry,
    string_table: &mut StringTable,
) -> Result<ScannedImportSource, SourceDiscoveryError> {
    let source =
        extract_source_code(file_path, string_table).map_err(SourceDiscoveryError::from)?;

    scan_imports_from_source(file_path, source, style_directives, string_table)
}

pub(super) fn scan_imports_from_source(
    file_path: &Path,
    source: String,
    style_directives: &StyleDirectiveRegistry,
    string_table: &mut StringTable,
) -> Result<ScannedImportSource, SourceDiscoveryError> {
    let interned_path = match InternedPath::try_from_filesystem_path(file_path, string_table) {
        Ok(path) => path,
        Err(NonUtf8PathComponent { path }) => {
            return Err(SourceDiscoveryError::from(CompilerError::file_error(
                &path,
                format!(
                    "Source file path {path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
                ),
                string_table,
            )));
        }
    };

    // Tokenize the file to find path declarations. Callers may supply source text that was read
    // during an earlier Stage 0 classification pass so provider-free discovery does not re-read
    // the same Moth file before assembling `PreparedSourceInput` values.
    let tokens = tokenize(
        &source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        style_directives,
        string_table,
        None,
    )
    .map_err(SourceDiscoveryError::Diagnostic)?;

    let imports = collect_provider_references_from_tokens(&tokens.tokens)
        .map_err(SourceDiscoveryError::Diagnostic)?;

    Ok(ScannedImportSource {
        imports,
        source_code: source,
        tokens,
    })
}
