//! Single-pass source loading, tokenisation and retained dependency-clause preparation.
//!
//! Tokenizes a single source file once and returns the complete prepared file output produced from
//! the same lexical pass. Stage 0 reads its dependency clauses from that output without rescanning
//! tokens, while final module preparation consumes the output's headers and selection table.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareFailure, FileFrontendPrepareOutput, HeaderParseOptions,
};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::identity::SourceFileTable;
use crate::compiler_frontend::symbols::interned_path::{InternedPath, NonUtf8PathComponent};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenizerEntryMode};
use crate::compiler_frontend::{
    CompilerFrontend, FrontendFilePrepareContext, FrontendFilePrepareInput,
    FrontendFilePrepareSource,
};

use std::path::Path;

use super::source_discovery_error::SourceDiscoveryError;
use super::source_loading::extract_source_code;

/// Source scan output retained by synthetic discovery.
///
/// WHAT: pairs the complete provider-independent file output with its source byte length.
/// WHY: reachable-file discovery consumes dependency clauses from the same output that later
///      module aggregation owns, so synthetic preparation has one tokenization and one header
///      parse rather than retaining a second raw token stream.
pub(super) struct PreparedDiscoverySource {
    pub(super) prepared_output: FileFrontendPrepareOutput,
    pub(super) source_byte_len: usize,
    pub(super) source_kind: SourceFileKind,
}

pub(super) fn prepare_discovery_source(
    file_path: &Path,
    style_directives: &StyleDirectiveRegistry,
    project_path_resolver: &Option<ProjectPathResolver>,
    entry_file_path: &Path,
    source_files: &mut SourceFileTable,
    string_table: &mut StringTable,
) -> Result<PreparedDiscoverySource, SourceDiscoveryError> {
    let source =
        extract_source_code(file_path, string_table).map_err(SourceDiscoveryError::from)?;

    prepare_discovery_source_text(
        file_path,
        source,
        style_directives,
        project_path_resolver,
        entry_file_path,
        source_files,
        string_table,
    )
}

pub(super) fn prepare_discovery_source_text(
    file_path: &Path,
    source: String,
    style_directives: &StyleDirectiveRegistry,
    project_path_resolver: &Option<ProjectPathResolver>,
    entry_file_path: &Path,
    source_files: &mut SourceFileTable,
    string_table: &mut StringTable,
) -> Result<PreparedDiscoverySource, SourceDiscoveryError> {
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

    // Tokenize the file once. Callers may supply source text that was read during an earlier
    // Stage 0 classification pass so provider-free discovery does not re-read the same Moth
    // file before assembling `PreparedSourceInput` values.
    let tokens = tokenize(
        &source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        style_directives,
        string_table,
        None,
    )
    .map_err(SourceDiscoveryError::Diagnostic)?;

    // Register this file in the traversal-local identity table so header preparation can stamp
    // real shells; the table is discarded after discovery and `prepare_module` rebinds identity.
    source_files.insert(
        file_path.to_path_buf(),
        entry_file_path,
        project_path_resolver.as_ref(),
        string_table,
    )?;

    let prepared_output = prepare_discovery_file(
        file_path,
        tokens,
        style_directives,
        project_path_resolver,
        entry_file_path,
        source_files,
        string_table,
    )?;

    Ok(PreparedDiscoverySource {
        prepared_output,
        source_byte_len: source.len(),
        source_kind: SourceFileKind::Moth,
    })
}

/// Prepare one tokenized Moth-template body during synthetic discovery.
pub(super) fn prepare_discovery_template_source(
    file_path: &Path,
    style_directives: &StyleDirectiveRegistry,
    project_path_resolver: &Option<ProjectPathResolver>,
    entry_file_path: &Path,
    source_files: &mut SourceFileTable,
    string_table: &mut StringTable,
) -> Result<PreparedDiscoverySource, SourceDiscoveryError> {
    let source =
        extract_source_code(file_path, string_table).map_err(SourceDiscoveryError::from)?;
    let source_byte_len = source.len();

    source_files.insert(
        file_path.to_path_buf(),
        entry_file_path,
        project_path_resolver.as_ref(),
        string_table,
    )?;
    let prepared_output = prepare_discovery_output(
        FrontendFilePrepareSource::MothTemplate {
            source_code: source,
            source_path: file_path.to_path_buf(),
        },
        style_directives,
        project_path_resolver,
        entry_file_path,
        source_files,
        string_table,
    )?;

    Ok(PreparedDiscoverySource {
        prepared_output,
        source_byte_len,
        source_kind: SourceFileKind::MothTemplate,
    })
}

/// Prepare one scanned Moth file and retain its complete provider-independent output.
///
/// WHAT: runs the same retained header preparation that later feeds binding, so discovery and
///      binding consume one clause owner.
/// WHY: Stage 0 needs dependency clauses immediately, while module compilation later needs the
///      complete headers, selection table and header-owned token substreams from that same pass.
fn prepare_discovery_file(
    file_path: &Path,
    tokens: FileTokens,
    style_directives: &StyleDirectiveRegistry,
    project_path_resolver: &Option<ProjectPathResolver>,
    entry_file_path: &Path,
    source_files: &SourceFileTable,
    string_table: &mut StringTable,
) -> Result<FileFrontendPrepareOutput, SourceDiscoveryError> {
    prepare_discovery_output(
        FrontendFilePrepareSource::Moth {
            source_path: file_path.to_path_buf(),
            tokens: Box::new(tokens),
        },
        style_directives,
        project_path_resolver,
        entry_file_path,
        source_files,
        string_table,
    )
}

fn prepare_discovery_output(
    source: FrontendFilePrepareSource,
    style_directives: &StyleDirectiveRegistry,
    project_path_resolver: &Option<ProjectPathResolver>,
    entry_file_path: &Path,
    source_files: &SourceFileTable,
    string_table: &mut StringTable,
) -> Result<FileFrontendPrepareOutput, SourceDiscoveryError> {
    // Fork a local string table so preparation never mutates the shared table while merging.
    let fork_source = string_table.fork_source();
    let base_len = fork_source.base_len();
    let (mut local_table, _) = fork_source.fork_for_module().into_parts();

    let entry_file_id = source_files
        .get_by_canonical_path(entry_file_path)
        .map(|identity| identity.file_id);
    let options = HeaderParseOptions {
        entry_file_id,
        project_path_resolver: project_path_resolver.clone(),
        entry_file_role: None,
        active_root_role: ModuleRootRole::Normal,
    };
    let prepare_context = FrontendFilePrepareContext {
        source_files,
        style_directives,
        entry_file_path,
        options: &options,
    };
    let input = FrontendFilePrepareInput {
        source,
        const_template_offset: 0,
        runtime_fragment_offset: 0,
    };

    let mut output = match CompilerFrontend::prepare_file_frontend_local(
        &prepare_context,
        input,
        &mut local_table,
    ) {
        Ok(output) => output,
        Err(FileFrontendPrepareFailure::Diagnosed(mut error)) => {
            let remap = string_table.merge_delta_from(&local_table, base_len);
            error.remap_string_ids(&remap);
            let mut messages =
                CompilerMessages::from_diagnostics(vec![*error.diagnostic], string_table.clone());
            messages.prepend_diagnostics_preserving_context(error.warnings);
            return Err(SourceDiscoveryError::Messages(messages));
        }
        Err(FileFrontendPrepareFailure::Infrastructure(error)) => {
            return Err(SourceDiscoveryError::Infrastructure(error));
        }
    };

    let remap = string_table.merge_delta_from(&local_table, base_len);
    output
        .remap_string_ids(&remap)
        .map_err(SourceDiscoveryError::from)?;

    Ok(output)
}
