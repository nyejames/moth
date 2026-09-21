//! Header parser entry point.
//!
//! WHAT: parses individual token streams into per-file header outputs, then splits module-wide
//! header work into two explicit phases: provider-independent `PreparedHeaderSyntax` and
//! provider-dependent `BoundModuleHeaders`.
//! WHY: syntax preparation must complete before provider interfaces exist, so callers prepare
//! retained syntax first and bind it later without retokenizing or reparsing source.

use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::arena::{HeaderStats, TokenStats};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, CompilerDiagnostic, DiagnosticBag, InvalidConfigReason,
    InvalidDeclarationReason,
};
pub(crate) use crate::compiler_frontend::declaration_syntax::build_config_contract::SourceBuildConfigContract;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::binding_environment::{
    BindingEnvironmentInput, prepare_binding_environment,
};
use crate::compiler_frontend::headers::constant_dependencies::{
    ConstantDependencyInput, add_constant_initializer_dependencies,
};
use crate::compiler_frontend::headers::dependency_canonicalization::canonicalize_local_ordering_hints;
use crate::compiler_frontend::headers::file_parser::{finish_file_output, parse_headers_in_file};
use crate::compiler_frontend::headers::public_exports::build_public_exports;
use crate::compiler_frontend::headers::symbol_collection::build_module_symbols;
pub(crate) use crate::compiler_frontend::headers::types::SourcePreparationDelta;
pub use crate::compiler_frontend::headers::types::{
    BoundModuleHeaders, FileFrontendPrepareError, FileFrontendPrepareFailure,
    FileFrontendPrepareOutput, FileRole, Header, HeaderKind, HeaderParseOptions,
    LocalDeclarationOrderingHint, LocalDeclarationOrderingHintOrigin, PreparedHeaderSyntax,
    RetainedDependencyClause, TopLevelConstFragment,
};
use crate::compiler_frontend::headers::types::{
    HeaderParseContext, HeaderParseFailure, SourceTokenOwner,
};
// HeaderExportMode is re-exported for focused AST tests that construct Header values with
// explicit export modes. Production code calls HeaderExportMode::is_public() through the
// header field, so this re-export is only reached from test modules.
use crate::compiler_frontend::declaration_syntax::build_config_contract::{
    normalize_source_build_config_contract_from_token,
    normalize_source_build_config_contract_non_primitive,
};
#[cfg(test)]
pub use crate::compiler_frontend::headers::types::HeaderExportMode;
use crate::compiler_frontend::paths::path_resolution::ProjectPathResolver;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::semantic_identity::ModuleRootRole;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceDatabase, SourceId, SourceSpan};
use crate::compiler_frontend::source_packages::root_file::{
    file_name_is_config_file, file_name_is_module_root_file,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TokenCursor, TokenRange, TokenTag,
};
use rustc_hash::FxHashMap;
use std::mem;
use std::path::Path;
use std::sync::Arc;

/// Parse one tokenized file using the supplied string table.
///
/// WHAT: computes the file role, builds the header parse context, and delegates to the file parser.
/// WHY: fused frontend preparation owns local-table creation and merging in the pipeline layer,
/// while the header stage owns only header parsing against whichever table the caller provides.
///
/// The caller lends the source's original live span builder so per-file diagnostics and warnings
/// can retain their exact primary and related byte ranges before the result crosses a preparation boundary.
#[allow(
    clippy::too_many_arguments,
    reason = "header parsing keeps the canonical source owner, its semantic source path, preparing path table, entry path, options, mutable string/path/span state, and the two fragment offsets as separate inputs"
)]
pub fn parse_file_headers_with_table(
    owner: SourceTokenOwner,
    source_file: crate::compiler_frontend::symbols::path_interner::PathId,
    path_syntax: Arc<PathSyntaxTable>,
    entry_file_path: &Path,
    options: &HeaderParseOptions,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    const_template_offset: usize,
    runtime_fragment_offset: usize,
    span_builder: &mut ExtendedSpanBuilder,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    let file_id = owner.source_id();
    #[cfg(feature = "data_layout_memory_probe")]
    crate::compiler_frontend::instrumentation::record_source_tokens_path_table(
        owner.tokens_ref(),
        &path_syntax,
    );
    let HeaderParseOptions { entry_file_id, .. } = options;

    let is_entry_file = entry_file_id.map_or_else(
        || {
            let mut scratch = Vec::new();
            path_fork.render_native(source_file, string_table, &mut scratch) == entry_file_path
        },
        |expected_id| expected_id == file_id,
    );

    let mut scratch = Vec::new();
    let source_path = path_fork.render_native(source_file, string_table, &mut scratch);
    // Stage 0's module-root inventory admits only canonical `@*.moth`/`+*.moth` root names.
    // Keep the filename check here as the provider-independent role classifier; filesystem root
    // identity remains owned by Stage 0 and is not reconstructed from a relative logical path.
    let is_module_root_file_by_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(file_name_is_module_root_file);
    let is_config_file = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(file_name_is_config_file);

    let file_role = if is_entry_file {
        options
            .entry_file_role
            .unwrap_or(match options.active_root_role {
                ModuleRootRole::Normal => FileRole::ActiveModuleRoot,
                ModuleRootRole::Support | ModuleRootRole::ProjectPackageFacade => {
                    FileRole::ActiveApiOnlyModuleRoot
                }
            })
    } else if is_module_root_file_by_name {
        FileRole::ImportedModuleRoot
    } else {
        FileRole::Normal
    };

    let mut parse_context = HeaderParseContext {
        file_role,
        is_config_file,
        string_table,
        path_fork,
        span_builder,
        path_syntax: Arc::clone(&path_syntax),
        const_template_offset,
        runtime_fragment_offset,
    };
    let parsed = {
        let canonical = owner.tokens_ref();
        if canonical.source() != file_id {
            return Err(FileFrontendPrepareFailure::Infrastructure(
                CompilerError::compiler_error(
                    "header parse source token owner does not match its file identity",
                ),
            ));
        }
        let range = owner
            .full_range()
            .map_err(FileFrontendPrepareFailure::Infrastructure)?;
        let mut cursor = owner
            .cursor(range)
            .map_err(FileFrontendPrepareFailure::Infrastructure)?;
        parse_headers_in_file(
            &mut cursor,
            file_id,
            source_file,
            owner.len(),
            &mut parse_context,
        )
        .map(|state| (state, cursor.position()))
    };
    let file_output = match parsed {
        Ok((state, end_index)) => finish_file_output(
            owner,
            source_file,
            path_syntax,
            file_id,
            end_index,
            &mut parse_context,
            state,
        ),
        Err(error) => Err(error),
    };
    capture_preparation_spans(file_output, file_id)
}

/// Attach exact primary and related spans to warnings and diagnostics emitted for one source file.
///
/// WHAT: encodes the already-produced byte bounds at the header entry boundary, where the
///       caller still owns the source's original span builder.
/// WHY: this covers direct, pipeline and config preparation without threading span state through
///       shared declaration parsers. Infrastructure capture failures stay on the existing
///       preparation lane and cannot invent a range end.
fn capture_preparation_spans(
    mut file_output: Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure>,
    file_id: SourceId,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    match &mut file_output {
        Ok(output) => {
            for warning in &mut output.warnings {
                warning
                    .capture_preparation_span(file_id)
                    .map_err(FileFrontendPrepareFailure::Infrastructure)?;
            }
        }
        Err(FileFrontendPrepareFailure::Diagnosed(error)) => {
            for warning in &mut error.warnings {
                warning
                    .capture_preparation_span(file_id)
                    .map_err(FileFrontendPrepareFailure::Infrastructure)?;
            }
            error
                .diagnostic
                .capture_preparation_span(file_id)
                .map_err(FileFrontendPrepareFailure::Infrastructure)?;
        }
        Err(FileFrontendPrepareFailure::Infrastructure(_)) => {}
    }

    file_output
}

/// Parse headers from an already-tokenized file against a local string-table fork, then merge
/// the local delta back into the module/global table and remap all StringIds in the output.
///
/// WHAT: this is the per-file header-parsing half of preparation for callers that already ran
///       tokenization, such as config parsing that runs token-level validation first.
/// WHY: the caller lends the original source span builder so this boundary can capture diagnostic
///      ranges, then retains that same builder for later retained-token span resolution.
#[allow(
    clippy::too_many_arguments,
    reason = "file preparation keeps the canonical owner, its semantic source path, preparing path table, entry path, options, mutable string/path/span state, and the two fragment offsets as separate inputs"
)]
pub(crate) fn prepare_file_from_tokens(
    owner: SourceTokenOwner,
    source_file: crate::compiler_frontend::symbols::path_interner::PathId,
    path_syntax: Arc<PathSyntaxTable>,
    entry_file_path: &Path,
    options: &HeaderParseOptions,
    string_table: &mut StringTable,
    const_template_offset: usize,
    runtime_fragment_offset: usize,
    span_builder: &mut ExtendedSpanBuilder,
    path_fork: &mut PathInternerFork,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    // Preflight the public preparation boundary: every PathId this function dereferences through
    // the supplied fork must have been issued by it (or its inherited base). A caller carrying a
    // foreign file-owned path identity would otherwise reach unchecked depth/parent/component
    // reads and panic on out-of-domain indices.
    if path_fork.try_depth(source_file).is_none() {
        return Err(FileFrontendPrepareFailure::Infrastructure(
            CompilerError::compiler_error(format!(
                "token stream for {entry_file_path:?} carries source path {src:?} that was not issued by the supplied path table",
                src = source_file,
            )),
        ));
    }
    let fork_source = string_table.fork_source();
    let (mut local_string_table, base_len) = fork_source.fork_for_module().into_parts();
    let path_fork_source = path_fork.fork_source();
    let mut local_path_fork = path_fork_source.fork_for_module();

    let file_output = parse_file_headers_with_table(
        owner,
        source_file,
        path_syntax,
        entry_file_path,
        options,
        &mut local_string_table,
        &mut local_path_fork,
        const_template_offset,
        runtime_fragment_offset,
        span_builder,
    );

    let remap = string_table.merge_delta_from(&local_string_table, base_len);
    let path_remap = path_fork
        .merge_delta_from(&local_path_fork, &remap)
        .map_err(|error| {
            FileFrontendPrepareFailure::Infrastructure(CompilerError::compiler_error(format!(
                "file path merge failed: {error:?}"
            )))
        })?;

    match file_output {
        Ok(mut output) => {
            output
                .remap_string_ids(&remap)
                .map_err(FileFrontendPrepareFailure::Infrastructure)?;
            output
                .remap_path_ids(&path_remap)
                .map_err(FileFrontendPrepareFailure::Infrastructure)?;
            output
                .freeze_path_syntax(string_table, path_fork)
                .map_err(FileFrontendPrepareFailure::Infrastructure)?;
            Ok(output)
        }
        Err(FileFrontendPrepareFailure::Diagnosed(mut error)) => {
            error.remap_string_ids(&remap);
            error.remap_path_ids(&path_remap);
            Err(FileFrontendPrepareFailure::Diagnosed(error))
        }
        Err(error @ FileFrontendPrepareFailure::Infrastructure(_)) => Err(error),
    }
}

/// Failure lanes for aggregation while source span storage remains live.
#[derive(Debug)]
pub(crate) enum HeaderPreparationFailure {
    Diagnosed(DiagnosticBag),
    Infrastructure(CompilerError),
}

impl From<DiagnosticBag> for HeaderPreparationFailure {
    fn from(diagnostics: DiagnosticBag) -> Self {
        Self::Diagnosed(diagnostics)
    }
}

/// Aggregate per-file frontend preparation outputs into provider-independent
/// `PreparedHeaderSyntax`.
///
/// WHAT: moves declaration and dependency facts out of already-remapped per-file outputs to build
/// the module-wide symbol package, retained headers, fragments and statistics. The caller keeps
/// each source's live span builder, including when aggregation returns diagnostics.
/// WHY: this is the only phase that discovers module-wide top-level declaration syntax. It must
pub fn prepare_header_syntax(
    prepared_files: &mut [FileFrontendPrepareOutput],
    string_table: &mut StringTable,
    capture: &mut impl FnMut(SourceId, &mut CompilerDiagnostic) -> Result<(), CompilerError>,
    path_fork: &mut PathInternerFork,
) -> Result<PreparedHeaderSyntax, HeaderPreparationFailure> {
    let source_build_config_contracts =
        collect_source_build_config_contracts(prepared_files, string_table, capture, path_fork)?;
    let module_symbols = build_module_symbols(prepared_files, string_table, capture, path_fork)?;
    let mut headers: Vec<Header> = Vec::new();
    let mut source_token_owners: crate::compiler_frontend::headers::SourceTokenOwners =
        FxHashMap::default();
    let mut top_level_const_fragments = Vec::new();
    let mut runtime_fragment_count = 0usize;
    let mut has_non_trivial_root_body = false;
    let mut token_stats = TokenStats::default();

    for output in prepared_files {
        token_stats.add(&output.token_stats);
        if let Some(stream) = output.source_token_stream.take() {
            let owner = crate::compiler_frontend::headers::SourceTokenOwner::new(stream);
            if source_token_owners.insert(output.file_id, owner).is_some() {
                return Err(HeaderPreparationFailure::Infrastructure(
                    CompilerError::compiler_error(
                        "multiple canonical source token streams were prepared for one SourceId",
                    ),
                ));
            }
        }
        headers.extend(mem::take(&mut output.headers));
        top_level_const_fragments.extend(mem::take(&mut output.top_level_const_fragments));
        runtime_fragment_count = runtime_fragment_count
            .checked_add(output.runtime_fragment_count)
            .ok_or_else(|| {
                HeaderPreparationFailure::Infrastructure(CompilerError::compiler_error(
                    "runtime fragment count overflowed header aggregation",
                ))
            })?;
        has_non_trivial_root_body |= output.has_non_trivial_root_body;
    }

    let header_stats = HeaderStats::from_headers_and_symbols(&headers, &module_symbols);
    let const_fragment_count = top_level_const_fragments.len();

    Ok(PreparedHeaderSyntax {
        headers,
        source_token_owners,
        source_build_config_contracts,
        top_level_const_fragments,
        entry_runtime_fragment_count: runtime_fragment_count,
        const_fragment_count,
        has_non_trivial_root_body,
        token_stats,
        header_stats,
        module_symbols,
    })
}
/// Find a `#Config` marker in source-owned declaration/default ranges.
///
/// Declaration shells retain only checked ranges; this pass reads those ranges through the
/// canonical owner and never recreates a declaration-owned token vector.
pub(super) fn find_config_qualifier_marker_in_declaration_defaults(
    header: &Header,
    source_tokens: &SourceTokens,
    string_table: &StringTable,
) -> Result<Option<SourceSpan>, CompilerError> {
    fn marker_in_cursor(
        mut cursor: TokenCursor<'_>,
        string_table: &StringTable,
    ) -> Option<SourceSpan> {
        let mut previous = cursor.advance()?;
        loop {
            let crosses_segment = cursor.is_at_segment_start();
            let Some(current) = cursor.advance() else {
                break;
            };
            if !crosses_segment
                && previous.tag() == TokenTag::HASH
                && current.tag() == TokenTag::SYMBOL
                && current
                    .string_id()
                    .is_some_and(|name| string_table.resolve(name) == "Config")
            {
                return Some(previous.source_span());
            }
            if current.is_eof() {
                break;
            }
            previous = current;
        }
        None
    }

    let marker_in_range = |range: Option<TokenRange>| -> Result<Option<SourceSpan>, CompilerError> {
        let Some(range) = range else {
            return Ok(None);
        };
        let cursor = source_tokens.cursor(range).map_err(|error| {
            CompilerError::compiler_error(format!(
                "declaration marker range could not be resolved through its source owner: {error:?}"
            ))
        })?;
        Ok(marker_in_cursor(cursor, string_table))
    };

    match &header.kind {
        HeaderKind::Constant { declaration } => marker_in_range(declaration.initializer_range),
        HeaderKind::Function { signature, .. } => {
            for parameter in &signature.parameters {
                if let Some(marker) = marker_in_range(parameter.default_range)? {
                    return Ok(Some(marker));
                }
            }
            Ok(None)
        }
        HeaderKind::Struct { fields, .. } => {
            for field in fields {
                if let Some(marker) = marker_in_range(field.default_range)? {
                    return Ok(Some(marker));
                }
            }
            Ok(None)
        }
        HeaderKind::Choice { variants, .. } => {
            for variant in variants {
                let crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayloadSyntax::Record {
                    fields,
                } = &variant.payload
                else {
                    continue;
                };
                for field in fields {
                    if let Some(marker) = marker_in_range(field.default_range)? {
                        return Ok(Some(marker));
                    }
                }
            }
            Ok(None)
        }
        HeaderKind::Trait { declaration } => {
            for requirement in &declaration.requirements {
                for parameter in &requirement.signature.parameters {
                    if let Some(marker) = marker_in_range(parameter.default_range)? {
                        return Ok(Some(marker));
                    }
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

/// Find retained declaration/body `#Config` markers through the canonical source owner.
///
/// Contiguous declaration ranges use `Header::tokens`; the active start body uses its checked
/// `TokenSequenceId`. No body vector is materialized solely to inspect this two-token marker.
pub(super) fn find_config_qualifier_marker_in_header(
    header: &Header,
    source_tokens: Option<&SourceTokens>,
    string_table: &StringTable,
) -> Result<Option<(SourceSpan, bool)>, CompilerError> {
    let Some(source_tokens) = source_tokens else {
        if header.token_sequence.is_some() || !header.tokens.is_empty() {
            return Err(CompilerError::compiler_error(
                "retained header marker scan has no source token owner",
            ));
        }
        return Ok(None);
    };
    if header.tokens.source() != source_tokens.source() {
        return Err(CompilerError::compiler_error(
            "retained header marker scan does not match its source token owner",
        ));
    }

    if let Some(marker) =
        find_config_qualifier_marker_in_declaration_defaults(header, source_tokens, string_table)?
    {
        return Ok(Some((marker, true)));
    }
    let body_cursor = if let Some(sequence) = header.token_sequence {
        let view = source_tokens.token_sequence(sequence).map_err(|error| {
            CompilerError::compiler_error(format!(
                "retained header marker scan could not resolve its token sequence: {error:?}"
            ))
        })?;
        Some(view.cursor().map_err(|error| {
            CompilerError::compiler_error(format!(
                "retained header marker scan could not construct its token sequence cursor: {error:?}"
            ))
        })?)
    } else if header.tokens.is_empty() {
        None
    } else {
        Some(source_tokens.cursor(header.tokens).map_err(|error| {
            CompilerError::compiler_error(format!(
                "retained header marker scan could not construct its token range cursor: {error:?}"
            ))
        })?)
    };

    fn marker_in_cursor(
        mut cursor: TokenCursor<'_>,
        string_table: &StringTable,
    ) -> Option<SourceSpan> {
        let mut previous = cursor.advance()?;
        loop {
            let crosses_segment = cursor.is_at_segment_start();
            let Some(current) = cursor.advance() else {
                break;
            };
            if !crosses_segment
                && previous.tag() == TokenTag::HASH
                && current.tag() == TokenTag::SYMBOL
                && current
                    .string_id()
                    .is_some_and(|name| string_table.resolve(name) == "Config")
            {
                return Some(previous.source_span());
            }
            if current.is_eof() {
                break;
            }
            previous = current;
        }
        None
    }

    Ok(body_cursor
        .and_then(|cursor| marker_in_cursor(cursor, string_table))
        .map(|marker| (marker, true)))
}

/// Collect source-owned `#Config` contract shells and reject all non-declaration placements.
///
/// The declaration shell itself remains in `PreparedHeaderSyntax::headers` for the later
/// config-resolution barrier. This pass only creates the provider-independent contract carrier and
fn collect_source_build_config_contracts(
    prepared_files: &[FileFrontendPrepareOutput],
    string_table: &mut StringTable,
    capture: &mut impl FnMut(SourceId, &mut CompilerDiagnostic) -> Result<(), CompilerError>,
    path_fork: &PathInternerFork,
) -> Result<Vec<SourceBuildConfigContract>, HeaderPreparationFailure> {
    let mut contracts = Vec::new();
    let mut diagnostics = DiagnosticBag::new();

    for output in prepared_files {
        // The config.moth source has its own declaration-owned qualifier consumer. Leaving its
        // top-level qualifier diagnostics on that path keeps config.moth semantics unchanged.
        let is_config_file = path_fork
            .component(output.source_file)
            .is_some_and(|name| file_name_is_config_file(string_table.resolve(name)));
        if is_config_file {
            continue;
        }

        let source_tokens = output.source_token_stream.as_deref();
        if let Some(source_tokens) = source_tokens
            && source_tokens.source() != output.file_id
        {
            return Err(HeaderPreparationFailure::Infrastructure(
                CompilerError::compiler_error(
                    "config marker scan source token owner does not match its file identity",
                ),
            ));
        }

        for header in &output.headers {
            let report_marker = |span: SourceSpan, adjacent: bool| {
                if adjacent {
                    CompilerDiagnostic::invalid_config_reason(
                        path_fork.component(header.declaration_path),
                        InvalidConfigReason::ConfigQualifierInvalidPlacement,
                        Some(span),
                    )
                } else {
                    CompilerDiagnostic::common_syntax_mistake(
                        CommonSyntaxMistakeReason::InvalidConfigQualifierSpacing,
                        Some(span),
                    )
                }
            };

            if let Some((location, adjacent)) =
                find_config_qualifier_marker_in_header(header, source_tokens, string_table)
                    .map_err(HeaderPreparationFailure::Infrastructure)?
            {
                let mut diagnostic = report_marker(location, adjacent);
                capture(output.file_id, &mut diagnostic)
                    .map_err(HeaderPreparationFailure::Infrastructure)?;
                diagnostics.push(diagnostic);
                continue;
            }

            let HeaderKind::Constant { declaration } = &header.kind else {
                continue;
            };
            let Some(qualifier) = &declaration.config_qualifier else {
                continue;
            };
            let Some(name) = path_fork.component(header.declaration_path) else {
                let mut diagnostic = CompilerDiagnostic::invalid_config_reason(
                    None,
                    InvalidConfigReason::ConfigContractNameInvalid,
                    header.name_span,
                );
                capture(output.file_id, &mut diagnostic)
                    .map_err(HeaderPreparationFailure::Infrastructure)?;
                diagnostics.push(diagnostic);
                continue;
            };
            let Some(name_span) = header.name_span else {
                continue;
            };

            let normalized = if let Some(range) = declaration.initializer_range {
                let source = source_tokens.ok_or_else(|| {
                    HeaderPreparationFailure::Infrastructure(CompilerError::compiler_error(
                        "source config initializer has no canonical source token owner",
                    ))
                })?;
                let mut cursor = source.cursor(range).map_err(|error| {
                    HeaderPreparationFailure::Infrastructure(CompilerError::compiler_error(
                        format!("source config initializer range is invalid: {error:?}"),
                    ))
                })?;
                let token = cursor.advance().ok_or_else(|| {
                    HeaderPreparationFailure::Infrastructure(CompilerError::compiler_error(
                        "source config initializer range is empty",
                    ))
                })?;
                if range.len() == 1 {
                    normalize_source_build_config_contract_from_token(
                        name,
                        name_span,
                        qualifier,
                        Some(token),
                        string_table,
                    )
                } else {
                    normalize_source_build_config_contract_non_primitive(
                        name,
                        name_span,
                        qualifier,
                        token.source_span(),
                        string_table,
                    )
                }
            } else {
                normalize_source_build_config_contract_from_token(
                    name,
                    name_span,
                    qualifier,
                    None,
                    string_table,
                )
            };

            match normalized {
                Ok(contract) => contracts.push(contract),
                Err(HeaderParseFailure::Diagnostic(mut diagnostic)) => {
                    capture(output.file_id, &mut diagnostic)
                        .map_err(HeaderPreparationFailure::Infrastructure)?;
                    diagnostics.push(diagnostic);
                }
                Err(HeaderParseFailure::Infrastructure(error)) => {
                    return Err(HeaderPreparationFailure::Infrastructure(error));
                }
            }
        }
    }

    if diagnostics.has_errors() {
        return Err(HeaderPreparationFailure::Diagnosed(diagnostics));
    }
    Ok(contracts)
}

/// Bind retained `PreparedHeaderSyntax` against provider interfaces to produce
/// `BoundModuleHeaders`.
///
/// WHAT: resolves public exports, builds the header binding environment, canonicalizes dependency edges,
/// and completes constant initializer dependencies. Does not retokenize source or reparse
/// declaration syntax — it consumes only the retained `PreparedHeaderSyntax`.
/// WHY: these facts depend on provider interfaces and the project path resolver, so they cannot
/// be known during syntax preparation. Keeping binding separate lets the build system schedule
/// it after required providers have compiled.
#[allow(
    clippy::too_many_arguments,
    reason = "header binding keeps prepared syntax, external registries and resolution tables, provider dependencies, optional resolver, source database, and mutable string/path state as separate borrows"
)]
pub(in crate::compiler_frontend) fn bind_module_headers(
    prepared: PreparedHeaderSyntax,
    external_package_registry: &ExternalPackageRegistry,
    external_dependency_resolution_table: &ExternalImportResolutionTable,
    source_provider_dependencies: &crate::compiler_frontend::public_interface::SourceProviderDependencySet<
        '_,
    >,
    project_path_resolver: Option<&ProjectPathResolver>,
    source_files: &SourceDatabase,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<BoundModuleHeaders, HeaderPreparationFailure> {
    let PreparedHeaderSyntax {
        mut headers,
        source_token_owners,
        source_build_config_contracts,
        top_level_const_fragments,
        entry_runtime_fragment_count,
        const_fragment_count,
        has_non_trivial_root_body,
        token_stats,
        header_stats,
        mut module_symbols,
    } = prepared;
    validate_prelude_declaration_shells(
        &headers,
        external_package_registry,
        string_table,
        path_fork,
    )?;

    if let Some(resolver) = project_path_resolver {
        build_public_exports(
            &mut module_symbols,
            &headers,
            resolver,
            source_files,
            external_package_registry,
            source_provider_dependencies,
            string_table,
            path_fork,
        )
        .map_err(|failure| match failure {
            HeaderParseFailure::Diagnostic(diagnostic) => {
                HeaderPreparationFailure::Diagnosed(DiagnosticBag::from(diagnostic))
            }
            HeaderParseFailure::Infrastructure(error) => {
                HeaderPreparationFailure::Infrastructure(error)
            }
        })?;
    }

    let binding_environment = prepare_binding_environment(BindingEnvironmentInput {
        module_symbols: &module_symbols,
        external_package_registry,
        external_dependency_resolution_table,
        source_provider_dependencies,
        source_files,
        string_table,
        path_fork,
    })
    .map_err(|messages| {
        if let Some(error) = messages.infrastructure_error().cloned() {
            HeaderPreparationFailure::Infrastructure(error)
        } else {
            HeaderPreparationFailure::Diagnosed(DiagnosticBag::from_diagnostics(
                messages.into_diagnostics(),
            ))
        }
    })?;

    canonicalize_local_ordering_hints(
        &mut headers,
        &binding_environment,
        &module_symbols.source_paths_by_source_id,
        &module_symbols.file_dependency_clauses_by_source,
        &module_symbols.dependency_selections_by_source,
        string_table,
        path_fork,
    )?;

    let _constant_report = add_constant_initializer_dependencies(ConstantDependencyInput {
        headers: &mut headers,
        module_symbols: &module_symbols,
        binding_environment: &binding_environment,
        string_table,
        path_fork,
    })?;

    Ok(BoundModuleHeaders {
        headers,
        source_token_owners,
        source_build_config_contracts,
        top_level_const_fragments,
        entry_runtime_fragment_count,
        const_fragment_count,
        has_non_trivial_root_body,
        token_stats,
        header_stats,
        module_symbols,
        binding_environment,
    })
}

/// Validate provider-dependent prelude collisions from retained declaration shells.
///
/// WHAT: rejects declaration names that reuse prelude functions and generic parameters that reuse
/// prelude types, preserving their authored names and locations.
/// WHY: prelude membership is provider-dependent, so syntax preparation retains these shells
/// uniformly and binding validates them once the provider interface exists. Dependency-alias generic
fn validate_prelude_declaration_shells(
    headers: &[Header],
    external_package_registry: &ExternalPackageRegistry,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> Result<(), DiagnosticBag> {
    let mut collision_bag = DiagnosticBag::new();
    for header in headers {
        if let Some(name) = path_fork.component(header.declaration_path)
            && external_package_registry.is_prelude_function(string_table.resolve(name))
        {
            collision_bag.push(CompilerDiagnostic::reserved_builtin_name(
                name,
                header.name_span,
            ));
        }

        let generic_parameters = match &header.kind {
            HeaderKind::Function {
                generic_parameters, ..
            }
            | HeaderKind::Struct {
                generic_parameters, ..
            }
            | HeaderKind::Choice {
                generic_parameters, ..
            } => Some(generic_parameters),
            _ => None,
        };

        if let Some(generic_parameters) = generic_parameters {
            for parameter in &generic_parameters.parameters {
                if !external_package_registry.is_prelude_type(string_table.resolve(parameter.name))
                {
                    continue;
                }

                collision_bag.push(CompilerDiagnostic::invalid_declaration(
                    InvalidDeclarationReason::GenericParameterNameCollision {
                        parameter_name: parameter.name,
                    },
                    None,
                    parameter.span,
                ));
            }
        }
    }

    if collision_bag.has_errors() {
        return Err(collision_bag);
    }

    Ok(())
}

#[cfg(test)]
#[path = "tests/parse_file_headers_tests.rs"]
pub(crate) mod parse_file_headers_tests;
