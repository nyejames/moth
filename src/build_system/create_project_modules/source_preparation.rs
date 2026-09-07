//! Single-pass source loading, tokenisation and retained dependency-clause preparation.
//!
//! Tokenizes a single source file once and returns the complete prepared file result produced from
//! the same lexical pass. Stage 0 reads its dependency clauses from a successful output without
//! rescanning tokens, while final module preparation consumes successful outputs' headers and
//! selection tables. A diagnosed tokenizer or header rejection keeps the original source snapshot,
//! span builder and local preparation result on the same owner. Loading and registration failures
//! precede that owner; later preparation failures retain the loaded snapshot.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::headers::parse_file_headers::{
    FileFrontendPrepareError, FileFrontendPrepareFailure, FileFrontendPrepareOutput,
    HeaderParseOptions,
};
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::source::{SourceDatabase, SourceKind};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::{InternedPath, NonUtf8PathComponent};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{
    TokenizeFailure, TokenizeOutput, TokenizerEntryMode,
};
use crate::compiler_frontend::{
    CompilerFrontend, FrontendFilePrepareContext, FrontendFilePrepareInput,
    FrontendFilePrepareSource,
};

use std::path::Path;

use super::source_discovery_error::SourceDiscoveryError;
use super::source_loading::extract_source_code;

/// Source scan result retained by synthetic discovery.
///
/// WHAT: pairs the per-file preparation outcome with its source byte length and loaded source
///       text.
/// WHY: reachable-file discovery consumes dependency clauses from a successful output that later
///      module aggregation owns, so synthetic preparation has one tokenization and one header
///      parse; a diagnosed rejection retains its producer-local builder and warning payload for
///      final-domain publication, and the loaded text moves into the final source record without
///      another read in either lane.
pub(super) struct PreparedDiscoverySource {
    pub(super) prepared_output: Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure>,
    pub(super) source_byte_len: usize,
    pub(super) source_code: String,
}

pub(super) fn prepare_discovery_source(
    file_path: &Path,
    style_directives: &StyleDirectiveRegistry,
    project_path_resolver: &Option<ProjectPathResolver>,
    entry_file_path: &Path,
    source_files: &mut SourceDatabase,
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
    source_files: &mut SourceDatabase,
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

    // Register the file before tokenization because `FileTokens` is minted with the traversal-local
    // source identity. Discovery rebinds every prepared result to the final sorted database once
    // the closure is complete, and this table dies with the traversal.
    let source_id = source_files.insert(
        file_path.to_path_buf(),
        SourceKind::Compiler(SourceFileKind::Moth),
        entry_file_path,
        project_path_resolver.as_ref(),
        string_table,
    )?;

    // Tokenize the file once. Callers may supply source text that was read during an earlier
    // Stage 0 classification pass so provider-free discovery does not re-read the same Moth
    // file before assembling `PreparedSourceInput` values.
    let tokenized = match tokenize(
        &source,
        &interned_path,
        TokenizerEntryMode::SourceFile,
        style_directives,
        string_table,
        source_id,
    ) {
        Ok(output) => output,
        Err(TokenizeFailure {
            file_id,
            diagnostic,
            span_builder,
        }) => {
            return Ok(PreparedDiscoverySource {
                prepared_output: Err(FileFrontendPrepareFailure::Diagnosed(
                    FileFrontendPrepareError {
                        file_id,
                        warnings: Vec::new(),
                        diagnostic,
                        span_builder,
                    },
                )),
                source_byte_len: source.len(),
                source_code: source,
            });
        }
    };

    let prepared_output = prepare_discovery_file(
        file_path,
        tokenized,
        style_directives,
        project_path_resolver,
        entry_file_path,
        source_files,
        string_table,
    );
    Ok(PreparedDiscoverySource {
        prepared_output,
        source_byte_len: source.len(),
        source_code: source,
    })
}

/// Prepare one tokenized Moth-template body during synthetic discovery.
pub(super) fn prepare_discovery_template_source(
    file_path: &Path,
    style_directives: &StyleDirectiveRegistry,
    project_path_resolver: &Option<ProjectPathResolver>,
    entry_file_path: &Path,
    source_files: &mut SourceDatabase,
    string_table: &mut StringTable,
) -> Result<PreparedDiscoverySource, SourceDiscoveryError> {
    let source =
        extract_source_code(file_path, string_table).map_err(SourceDiscoveryError::from)?;
    let source_byte_len = source.len();

    source_files.insert(
        file_path.to_path_buf(),
        SourceKind::Compiler(SourceFileKind::MothTemplate),
        entry_file_path,
        project_path_resolver.as_ref(),
        string_table,
    )?;
    let prepared_output = prepare_discovery_output(
        FrontendFilePrepareSource::MothTemplate {
            source_code: source.as_str(),
            source_path: file_path.to_path_buf(),
        },
        style_directives,
        project_path_resolver,
        entry_file_path,
        source_files,
        string_table,
    );
    Ok(PreparedDiscoverySource {
        prepared_output,
        source_byte_len,
        source_code: source,
    })
}

/// Prepare one scanned Moth file and retain its complete provider-independent result.
///
/// WHAT: runs the same retained header preparation that later feeds binding, so discovery and
///      binding consume one clause owner.
/// WHY: Stage 0 needs dependency clauses immediately, while module compilation later needs the
///      complete headers, selection table and header-owned token substreams from that same pass.
///      A diagnosed header rejection keeps the lexical pass's builder instead of freezing it.
fn prepare_discovery_file(
    file_path: &Path,
    tokenized: TokenizeOutput,
    style_directives: &StyleDirectiveRegistry,
    project_path_resolver: &Option<ProjectPathResolver>,
    entry_file_path: &Path,
    source_files: &SourceDatabase,
    string_table: &mut StringTable,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    let TokenizeOutput {
        file_tokens,
        span_builder,
    } = tokenized;
    prepare_discovery_output(
        FrontendFilePrepareSource::Moth {
            source_path: file_path.to_path_buf(),
            tokens: Box::new(file_tokens),
            span_builder,
        },
        style_directives,
        project_path_resolver,
        entry_file_path,
        source_files,
        string_table,
    )
}

fn prepare_discovery_output(
    source: FrontendFilePrepareSource<'_>,
    style_directives: &StyleDirectiveRegistry,
    project_path_resolver: &Option<ProjectPathResolver>,
    entry_file_path: &Path,
    source_files: &SourceDatabase,
    string_table: &mut StringTable,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    // Fork a local string table so preparation never mutates the shared table while merging.
    let fork_source = string_table.fork_source();
    let base_len = fork_source.base_len();
    let (mut local_table, _) = fork_source.fork_for_module().into_parts();

    let entry_file_id = source_files
        .get_by_canonical_path(entry_file_path)
        .map(|identity| identity.id);
    let options = HeaderParseOptions {
        entry_file_id,
        project_path_resolver: project_path_resolver.as_ref(),
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

    let outcome =
        CompilerFrontend::prepare_file_frontend_local(&prepare_context, input, &mut local_table);
    let remap = string_table.merge_delta_from(&local_table, base_len);

    match outcome {
        Ok(mut output) => {
            output
                .remap_string_ids(&remap)
                .map_err(FileFrontendPrepareFailure::Infrastructure)?;
            Ok(output)
        }

        Err(FileFrontendPrepareFailure::Diagnosed(mut error)) => {
            error.remap_string_ids(&remap);
            Err(FileFrontendPrepareFailure::Diagnosed(error))
        }

        Err(FileFrontendPrepareFailure::Infrastructure(mut error)) => {
            error.remap_string_ids(&remap);
            Err(FileFrontendPrepareFailure::Infrastructure(error))
        }
    }
}
