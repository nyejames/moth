//! Per-file header splitting.
//!
//! WHAT: orchestrates one tokenized Moth file into top-level declaration headers, dependency
//! records, const-fragment metadata, and the implicit entry `start` body.
//! WHY: file-level control flow is different from declaration parsing, dependency recording and
//! const-template handling. This module keeps the high-level loop visible while delegated modules
//! own details.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_for_tag,
};
use crate::compiler_frontend::compiler_messages::{CapacityResource, CompilerStage};
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, ExportOutsideModuleRootReason, InvalidConfigReason,
    InvalidDeclarationReason, InvalidReceiverDeclarationReason,
};
use crate::compiler_frontend::declaration_syntax::build_config_contract::find_config_qualifier_marker_in_cursor;
use crate::compiler_frontend::headers::file_dependency_clauses::{
    parse_and_record_private_dependency, parse_and_record_public_dependency,
};
use crate::compiler_frontend::headers::file_state::HeaderFileParseState;
use crate::compiler_frontend::headers::hash_items::{HashItemRequest, handle_hash_item};
use crate::compiler_frontend::headers::header_dispatch::{
    HeaderDeclarationMetadata, create_header,
};
use crate::compiler_frontend::headers::ordering_hints::collect_content_source_ordering_hints;
use crate::compiler_frontend::headers::parse_file_headers::find_config_qualifier_marker_in_declaration_defaults;
use crate::compiler_frontend::headers::qualified_references::{
    collect_qualified_references, push_header_expression_ranges,
};
use crate::compiler_frontend::headers::start_capture::capture_runtime_template_range_from_cursor;
use crate::compiler_frontend::headers::symbol_collection::is_receiver_method_candidate;
use crate::compiler_frontend::headers::top_level_classifier::{
    HeaderFileItem, classify_current_item_ref, classify_export_block_item_ref,
    starts_duplicate_top_level_header_declaration_at_source,
    starts_specialized_generic_conformance_declaration_at_source,
    starts_trait_declaration_after_must_at_source, statement_boundary_at_source,
};
use crate::compiler_frontend::headers::types::{
    DependencySelection, FileFrontendPrepareFailure, FileFrontendPrepareOutput, FileRole, Header,
    HeaderBuildContext, HeaderExportMode, HeaderKind, HeaderParseContext, HeaderParseFailure,
    RetainedDependencyClause, SourceTokenOwner,
};
use crate::compiler_frontend::paths::file_references::classify_prepared_file_references;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxTable;
use crate::compiler_frontend::source::{SourceId, SourceSpan};
use crate::compiler_frontend::source_packages::root_file::file_name_is_config_file;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, TokenCursor, TokenIndex, TokenRange, TokenRef, TokenTag,
};
use crate::compiler_frontend::utilities::token_scan::NestingDepth;
use rustc_hash::FxHashSet;
use std::sync::Arc;

type FileParserResult<T> = Result<T, HeaderParseFailure>;

fn diagnostic_failure(diagnostic: DiagnosticDraft) -> HeaderParseFailure {
    HeaderParseFailure::Diagnostic(diagnostic)
}
/// Resolve one checked source-owned token at a canonical cursor index.
///
/// All Stage 0 facts come from the canonical source owner; retained parser state stores only
/// checked source ranges and sequence handles.
fn source_token_at_index<'a>(
    canonical: &'a SourceTokens,
    file_id: SourceId,
    index: TokenIndex,
    owner: &str,
) -> FileParserResult<TokenRef<'a>> {
    if canonical.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                format!("{owner} source token owner does not match its file identity"),
            ),
        ));
    }
    canonical.token(index).map_err(|error| {
        HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            format!("{owner} token index exceeded its source token owner: {error:?}"),
        ))
    })
}

fn record_start_body_token_from_source(
    state: &mut HeaderFileParseState,
    canonical: &SourceTokens,
    file_id: SourceId,
    index: TokenIndex,
) -> FileParserResult<()> {
    let token = source_token_at_index(canonical, file_id, index, "start-body token")?;
    state
        .record_start_body_token_ref(token)
        .map_err(HeaderParseFailure::Infrastructure)
}

// Top-level declarations are same-module-visible by default; cross-module public visibility
// comes only from the root `export:` block. Non-declaration statements are collected into the
// compiler-owned entry header for that file.
pub(super) fn parse_headers_in_file(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    source_file: PathId,
    token_count: usize,
    context: &mut HeaderParseContext<'_>,
) -> Result<HeaderFileParseState, FileFrontendPrepareFailure> {
    let mut state = HeaderFileParseState::new(token_count);
    match parse_headers_in_file_inner(cursor, file_id, source_file, context, &mut state) {
        Ok(()) => Ok(state),
        Err(HeaderParseFailure::Diagnostic(diagnostic)) => Err(
            FileFrontendPrepareFailure::Diagnosed(state.into_error(diagnostic)),
        ),
        Err(HeaderParseFailure::Infrastructure(error)) => {
            Err(FileFrontendPrepareFailure::Infrastructure(error))
        }
    }
}

fn parse_headers_in_file_inner(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    source_file: PathId,
    context: &mut HeaderParseContext<'_>,
    state: &mut HeaderFileParseState,
) -> FileParserResult<()> {
    let canonical = cursor.source_tokens();
    if canonical.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "header walk source token owner does not match its file identity",
            ),
        ));
    }

    // Declaration handlers consume their own balanced regions. Track only runtime tokens left
    // to this walk, where a physical newline inside an expression is not a file boundary.
    let mut runtime_nesting = NestingDepth::default();
    loop {
        let current_index = cursor.position();
        let current = cursor.current().ok_or_else(|| {
            HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "header cursor exceeded its source token owner",
            ))
        })?;
        let current_tag = current.tag();
        if !runtime_nesting.is_top_level() && current_tag != TokenTag::EOF {
            if current_tag == TokenTag::TEMPLATE_HEAD {
                // A rejected foreign-root body has no AST consumer, so validate nested template
                // balance here.
                cursor.advance();
                let range = capture_runtime_template_range_from_cursor(
                    current,
                    cursor,
                    file_id,
                    context.string_table,
                )?;
                state
                    .record_start_body_source_range_from_source_tokens(range, canonical)
                    .map_err(HeaderParseFailure::Infrastructure)?;
                continue;
            }
            state
                .record_start_body_token_ref(current)
                .map_err(HeaderParseFailure::Infrastructure)?;
            runtime_nesting.step_tag(current_tag);
            cursor.advance();
            continue;
        }
        let follower_index = current_index.index().checked_add(1).ok_or_else(|| {
            HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "header token follower index overflowed its source range",
            ))
        })?;
        let follower = TokenIndex::try_from_index(follower_index)
            .and_then(|index| canonical.token(index).ok());
        let at_boundary = statement_boundary_at_source(canonical, current_index.index());
        let current_span = current.source_span();
        let item = classify_current_item_ref(current, follower, at_boundary);
        cursor.advance();
        match item {
            HeaderFileItem::Symbol(name_id) => {
                handle_symbol_item(
                    cursor,
                    file_id,
                    source_file,
                    state,
                    context,
                    SymbolItemRequest {
                        name_id,
                        current_span,
                        current_index,
                    },
                )?;
            }

            HeaderFileItem::BuiltinTypeConformanceTarget(type_name) => {
                let name_id = context.string_table.intern(type_name);
                handle_symbol_item(
                    cursor,
                    file_id,
                    source_file,
                    state,
                    context,
                    SymbolItemRequest {
                        name_id,
                        current_span,
                        current_index,
                    },
                )?;
            }

            HeaderFileItem::Dependency => {
                parse_and_record_private_dependency(cursor, file_id, state, context, current_span)?;
            }

            HeaderFileItem::Export => {
                reject_non_block_export(cursor, file_id, source_file, context, current_span)?;
            }

            HeaderFileItem::ExportBlock => {
                handle_export_block(cursor, file_id, source_file, state, context, current_span)?;
            }

            HeaderFileItem::Hash {
                at_statement_boundary,
            } => {
                handle_hash_item(
                    cursor,
                    file_id,
                    source_file,
                    state,
                    context,
                    HashItemRequest {
                        current_index,
                        current_span,
                        at_statement_boundary,
                    },
                )?;
            }

            HeaderFileItem::ReservedTraitSyntax => {
                handle_trait_keyword_header_item(current_tag, current_span)?;
            }

            HeaderFileItem::RuntimeTemplate => {
                handle_runtime_template_item(cursor, file_id, state, context)?;
            }

            HeaderFileItem::Eof => {
                record_start_body_token_from_source(state, canonical, file_id, current_index)?;
                break;
            }

            HeaderFileItem::StartBodyToken => {
                runtime_nesting.step_tag(current_tag);
                record_start_body_token_from_source(state, canonical, file_id, current_index)?;
            }
        }
    }

    Ok(())
}

/// Reject an authored `export:` clause whose file role owns no export surface.
///
/// WHAT: entry roots receive a root-specific reason; helpers, config and non-facade roots keep
///       the generic module-root rule.
/// WHY: `@entry.moth` has no export surface, while ordinary helpers are not module roots at all.
fn reject_export_outside_module_root(
    context: &HeaderParseContext<'_>,
    source_file: PathId,
    export_span: SourceSpan,
) -> FileParserResult<()> {
    let reason = if context.is_config_file {
        Some(ExportOutsideModuleRootReason::NoExportSurface)
    } else {
        match context.file_role {
            FileRole::ActiveEntryRoot => Some(ExportOutsideModuleRootReason::EntryRoot),
            role if role.is_export_capable() => None,
            _ => Some(ExportOutsideModuleRootReason::NoExportSurface),
        }
    };

    match reason {
        Some(reason) => Err(diagnostic_failure(
            generics_traits_api::export_outside_module_root(source_file, reason, Some(export_span)),
        )),
        None => Ok(()),
    }
}

fn reject_non_block_export(
    cursor: &TokenCursor<'_>,
    file_id: SourceId,
    source_file: PathId,
    context: &mut HeaderParseContext<'_>,
    export_span: SourceSpan,
) -> FileParserResult<()> {
    reject_export_outside_module_root(context, source_file, export_span)?;

    // Without the block delimiter, the token is not an export target. Keep this diagnostic in
    // header parsing instead of interpreting the following tokens through another syntax path.
    let found = cursor.current().ok_or_else(|| {
        HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "export-block cursor exceeded its source token owner",
        ))
    })?;
    if found.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "export-block source token owner does not match its file identity",
            ),
        ));
    }
    Err(diagnostic_failure(
        token_lexical::expected_token_from_projections(
            crate::compiler_frontend::compiler_messages::DiagnosticToken::from_static_tag(
                TokenTag::COLON,
            ),
            Some(
                crate::compiler_frontend::compiler_messages::DiagnosticToken::from_token_ref(found),
            ),
            Some(export_span),
        ),
    ))
}
fn handle_export_block(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    source_file: PathId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    export_span: SourceSpan,
) -> FileParserResult<()> {
    reject_export_outside_module_root(context, source_file, export_span)?;

    if state.seen_export_block.is_some() {
        return Err(diagnostic_failure(
            generics_traits_api::duplicate_export_block(Some(export_span)),
        ));
    }

    // The classifier only produces ExportBlock when the current token is `:`, but consume it
    // here so the item parser starts at the first ordinary top-level item.
    let colon = cursor.current().ok_or_else(|| {
        HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "export-block cursor exceeded its source token owner",
        ))
    })?;
    if colon.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "export-block source token owner does not match its file identity",
            ),
        ));
    }
    if colon.tag() != TokenTag::COLON {
        return Err(diagnostic_failure(
            token_lexical::expected_token_from_projections(
                crate::compiler_frontend::compiler_messages::DiagnosticToken::from_static_tag(
                    TokenTag::COLON,
                ),
                Some(
                    crate::compiler_frontend::compiler_messages::DiagnosticToken::from_token_ref(
                        colon,
                    ),
                ),
                Some(export_span),
            ),
        ));
    }
    state.seen_export_block = Some(export_span);
    state.export_mode = HeaderExportMode::Public;
    cursor.advance();

    loop {
        let current = cursor.current().ok_or_else(|| {
            HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "export-block cursor exceeded its source token owner",
            ))
        })?;
        let current_tag = current.tag();
        if current_tag == TokenTag::END || current_tag == TokenTag::EOF {
            break;
        }
        if current_tag == TokenTag::NEWLINE {
            cursor.advance();
            continue;
        }

        let current_index = current.index();
        let follower_index = current_index.index().checked_add(1).ok_or_else(|| {
            HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "export-block follower index overflowed its source range",
            ))
        })?;
        let follower = TokenIndex::try_from_index(follower_index)
            .and_then(|index| cursor.source_tokens().token(index).ok());
        let current_span = current.source_span();
        let current_tag = current.tag();
        let item = classify_export_block_item_ref(current, follower);
        cursor.advance();
        parse_export_block_item(
            cursor,
            file_id,
            source_file,
            state,
            context,
            ExportBlockItemRequest {
                item,
                current_tag,
                current_span,
                current_index,
            },
        )?;
        state.export_block_item_count =
            state
                .export_block_item_count
                .checked_add(1)
                .ok_or_else(|| {
                    HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                        CompilerStage::HeaderParsing,
                        "export-block item count overflowed its source state",
                    ))
                })?;
    }

    let current = cursor.current().ok_or_else(|| {
        HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "export-block cursor exceeded its source token owner",
        ))
    })?;
    if current.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "export-block source token owner does not match its file identity",
            ),
        ));
    }
    if current.tag() == TokenTag::EOF {
        return Err(diagnostic_failure(token_lexical::unexpected_end_of_file(
            Some(context.string_table.intern(";")),
            Some(current.source_span()),
        )));
    }

    // The block terminator belongs to this parser mode and must not become an entry-body
    // token for the surrounding file.
    cursor.advance();
    state.export_mode = HeaderExportMode::Private;

    if state.export_block_item_count == 0 {
        return Err(diagnostic_failure(
            generics_traits_api::invalid_export_target(Some(export_span)),
        ));
    }

    Ok(())
}

struct ExportBlockItemRequest {
    item: HeaderFileItem,
    current_tag: TokenTag,
    current_span: SourceSpan,
    current_index: TokenIndex,
}

fn parse_export_block_item(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    source_file: PathId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    request: ExportBlockItemRequest,
) -> FileParserResult<()> {
    let ExportBlockItemRequest {
        item,
        current_tag,
        current_span,
        current_index,
    } = request;
    match item {
        HeaderFileItem::Symbol(name_id) => handle_symbol_item(
            cursor,
            file_id,
            source_file,
            state,
            context,
            SymbolItemRequest {
                name_id,
                current_span,
                current_index,
            },
        ),

        HeaderFileItem::BuiltinTypeConformanceTarget(type_name) => {
            let name_id = context.string_table.intern(type_name);
            handle_symbol_item(
                cursor,
                file_id,
                source_file,
                state,
                context,
                SymbolItemRequest {
                    name_id,
                    current_span,
                    current_index,
                },
            )
        }

        HeaderFileItem::Dependency => {
            parse_and_record_public_dependency(cursor, file_id, state, context, current_span)
        }

        HeaderFileItem::Export | HeaderFileItem::ExportBlock => Err(diagnostic_failure(
            generics_traits_api::invalid_export_target(Some(current_span)),
        )),

        HeaderFileItem::Hash {
            at_statement_boundary,
        } => handle_hash_item(
            cursor,
            file_id,
            source_file,
            state,
            context,
            HashItemRequest {
                current_index,
                current_span,
                at_statement_boundary,
            },
        ),

        HeaderFileItem::RuntimeTemplate | HeaderFileItem::StartBodyToken => {
            Err(diagnostic_failure(
                generics_traits_api::invalid_export_target(Some(current_span)),
            ))
        }

        HeaderFileItem::ReservedTraitSyntax => {
            if let Some(keyword) = reserved_trait_keyword_for_tag(current_tag) {
                return Err(diagnostic_failure(reserved_trait_keyword_error(
                    keyword,
                    Some(current_span),
                )));
            }

            Err(diagnostic_failure(
                generics_traits_api::invalid_export_target(Some(current_span)),
            ))
        }

        HeaderFileItem::Eof => Err(diagnostic_failure(token_lexical::unexpected_end_of_file(
            Some(context.string_table.intern(";")),
            Some(current_span),
        ))),
    }
}

/// One already-read top-level symbol with the source facts its header dispatch needs.
struct SymbolItemRequest {
    name_id: StringId,
    current_span: SourceSpan,
    current_index: TokenIndex,
}

fn handle_symbol_item(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    source_file: PathId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    symbol: SymbolItemRequest,
) -> FileParserResult<()> {
    let export_mode = state.export_mode;
    let SymbolItemRequest {
        name_id,
        current_span,
        current_index,
    } = symbol;
    let Some(follower_index) = TokenIndex::try_from_index(cursor.position().index()) else {
        crate::compiler_bug!(
            CompilerStage::HeaderParsing,
            "symbol lookahead index exceeded its checked domain",
        );
    };
    let (follower_tag, starts_duplicate, starts_trait, starts_specialized) = {
        let canonical = cursor.source_tokens();
        if canonical.source() != file_id {
            return Err(HeaderParseFailure::Infrastructure(
                InfrastructureFailure::internal(
                    CompilerStage::HeaderParsing,
                    "symbol lookahead source token owner does not match its file identity",
                ),
            ));
        }
        let follower = canonical.token(follower_index).map_err(|_| {
            HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "symbol lookahead index exceeded its source token owner",
            ))
        })?;
        (
            follower.tag(),
            starts_duplicate_top_level_header_declaration_at_source(canonical, follower_index),
            starts_trait_declaration_after_must_at_source(canonical, follower_index),
            starts_specialized_generic_conformance_declaration_at_source(canonical, follower_index),
        )
    };

    if export_mode.is_public() && !starts_duplicate {
        return Err(diagnostic_failure(
            generics_traits_api::invalid_export_target(Some(current_span)),
        ));
    }

    if let Some(first_span) = state.encountered_symbols.get(&name_id) {
        let is_conformance_declaration =
            (follower_tag == TokenTag::MUST && !starts_trait) || starts_specialized;

        if !is_conformance_declaration && starts_duplicate {
            return Err(diagnostic_failure(
                names_declarations::duplicate_declaration(name_id, Some(current_span)).with_label(Some(*first_span), Some(crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage::PreviousDeclaration)),
            ));
        }

        if !is_conformance_declaration {
            record_start_body_token_from_source(
                state,
                cursor.source_tokens(),
                file_id,
                current_index,
            )?;
            return Ok(());
        }
    }

    if state.start_body_symbols.contains(&name_id) && !starts_duplicate {
        record_start_body_token_from_source(state, cursor.source_tokens(), file_id, current_index)?;
        return Ok(());
    }

    let mut build_context = HeaderBuildContext {
        warnings: &mut state.warnings,
        source_file,
        file_dependency_clauses: &state.file_dependency_clauses,
        dependency_selections: &state.dependency_selections,
        string_table: context.string_table,
        path_fork: context.path_fork,
        file_role: context.file_role,
    };
    let declaration_path = build_context
        .path_fork
        .try_intern_child(source_file, name_id)
        .ok_or_else(|| {
            HeaderParseFailure::Diagnostic(token_lexical::compiler_capacity_exceeded(
                CapacityResource::PathNodes,
                Some(current_span),
            ))
        })?;
    let header = create_header(
        declaration_path,
        cursor,
        file_id,
        HeaderDeclarationMetadata {
            declaration_span: current_span,
            declaration_order: current_index.index(),
            export_mode,
        },
        &mut build_context,
        context.span_builder,
    )?;

    if export_mode.is_public()
        && matches!(
            &header.kind,
            HeaderKind::TraitConformance { .. } | HeaderKind::TraitIncompatibility { .. }
        )
    {
        return Err(diagnostic_failure(
            generics_traits_api::invalid_export_target(Some(current_span)),
        ));
    }

    if export_mode.is_public()
        && let HeaderKind::Function { signature, .. } = &header.kind
        && is_receiver_method_candidate(signature, context.string_table, context.path_fork)
    {
        return Err(diagnostic_failure(
            generics_traits_api::invalid_receiver_declaration(
                InvalidReceiverDeclarationReason::ReceiverMethodImportOrExportNotAllowed,
                Some(current_span),
            ),
        ));
    }

    match &header.kind {
        HeaderKind::StartFunction => {
            record_start_body_token_from_source(
                state,
                cursor.source_tokens(),
                file_id,
                current_index,
            )?;
            state.register_start_body_symbol(name_id);
            // Only an assignment introduces a runtime binding a constant context could name.
            // A call or other executable statement head is not a value declaration.
            if follower_tag.is_assignment_operator() {
                state.register_start_body_binding(name_id, current_span);
            }
        }
        HeaderKind::TraitConformance { .. } | HeaderKind::TraitIncompatibility { .. } => {
            state.register_header(header);
        }

        _ => {
            let name_span = header
                .name_span
                .expect("authored declaration headers carry a source span");
            state.register_header(header);
            state.encountered_symbols.insert(name_id, name_span);
        }
    }

    Ok(())
}

fn handle_trait_keyword_header_item(
    current_tag: TokenTag,
    current_span: SourceSpan,
) -> FileParserResult<()> {
    if let Some(keyword) = reserved_trait_keyword_for_tag(current_tag) {
        return Err(diagnostic_failure(reserved_trait_keyword_error(
            keyword,
            Some(current_span),
        )));
    }

    Ok(())
}

fn handle_runtime_template_item(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
) -> FileParserResult<()> {
    // Runtime top-level templates stay in the start-function body and are evaluated in source
    // order by the compiler-owned entry. The runtime fragment count lets later const fragments record their
    // insertion point relative to already-seen runtime fragments.
    let opening_index = cursor.position().index().checked_sub(1).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "runtime template opening preceded the source token index",
        ))
    })?;
    let opening_index = TokenIndex::try_from_index(opening_index).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "runtime template opening exceeded the source token index space",
        ))
    })?;
    let canonical = cursor.source_tokens();
    if canonical.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "runtime template source token owner does not match its file identity",
            ),
        ));
    }
    let opening = canonical.token(opening_index).map_err(|error| {
        HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            format!("runtime template opening exceeded its source token owner: {error:?}",),
        ))
    })?;
    let range =
        capture_runtime_template_range_from_cursor(opening, cursor, file_id, context.string_table)?;
    state
        .record_start_body_source_range_from_source_tokens(range, canonical)
        .map_err(HeaderParseFailure::Infrastructure)?;
    if context.file_role.synthesizes_entry_start() {
        state.runtime_fragment_count =
            state.runtime_fragment_count.checked_add(1).ok_or_else(|| {
                HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                    CompilerStage::HeaderParsing,
                    "runtime fragment count overflowed its source state",
                ))
            })?;
    }

    Ok(())
}

/// Scan the captured start-body ranges of a reached foreign root for a misplaced `#Config`
/// marker.
///
/// A reached foreign facade keeps its declaration surface and rejects its top-level body, so
/// no later AST stage can report a config marker authored there. Preparation scans the rejected
/// ranges first so the specific config diagnostic wins over the generic root-runtime rejection.
fn find_config_marker_in_start_ranges(
    state: &HeaderFileParseState,
    canonical: &SourceTokens,
    file_id: SourceId,
    string_table: &StringTable,
    span_builder: &crate::compiler_frontend::source::ExtendedSpanBuilder,
) -> Result<Option<(SourceSpan, bool)>, InfrastructureFailure> {
    if canonical.source() != file_id {
        return Err(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "start-body config scan source token owner does not match its file identity",
        ));
    }
    for range in &state.start_body_ranges {
        if range.source() != file_id {
            return Err(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "start-body config scan encountered a foreign source range",
            ));
        }
        let cursor = canonical.cursor(*range).map_err(|error| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                format!("start-body config scan exceeded source token bounds: {error:?}"),
            )
        })?;
        if let Some(marker) =
            find_config_qualifier_marker_in_cursor(cursor, string_table, span_builder)
        {
            return Ok(Some(marker));
        }
    }
    Ok(None)
}
pub(super) fn finish_file_output(
    owner: SourceTokenOwner,
    source_file: crate::compiler_frontend::symbols::path_interner::PathId,
    path_syntax: Arc<PathSyntaxTable>,
    file_id: SourceId,
    end_index: TokenIndex,
    context: &mut HeaderParseContext<'_>,
    state: HeaderFileParseState,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
    let canonical = owner.tokens_ref();
    if matches!(context.file_role, FileRole::ReachedApiFacadeRoot(_))
        && let Some((marker_span, adjacent)) = find_config_marker_in_start_ranges(
            &state,
            canonical,
            file_id,
            context.string_table,
            context.span_builder,
        )
        .map_err(FileFrontendPrepareFailure::Infrastructure)?
    {
        let diagnostic = if adjacent {
            project_config::invalid_config_reason(
                None,
                InvalidConfigReason::ConfigQualifierInvalidPlacement,
                Some(marker_span),
            )
        } else {
            token_lexical::common_syntax_mistake(
                CommonSyntaxMistakeReason::InvalidConfigQualifierSpacing,
                Some(marker_span),
            )
        };
        return Err(FileFrontendPrepareFailure::Diagnosed(
            state.into_error(diagnostic),
        ));
    }

    if let Some(diagnostic) = dependency_generic_parameter_collision(
        &state.headers,
        &state.file_dependency_clauses,
        &state.dependency_selections,
        context.string_table,
        &*context.path_fork,
    ) {
        return Err(FileFrontendPrepareFailure::Diagnosed(
            state.into_error(diagnostic),
        ));
    }

    if matches!(
        context.file_role,
        FileRole::Normal | FileRole::ActiveApiOnlyModuleRoot(_) | FileRole::ReachedApiFacadeRoot(_)
    ) && state.has_non_trivial_start_body()
    {
        let span = state
            .first_executable_start_body_span()
            .expect("non-trivial entry body has an executable token span");
        return Err(FileFrontendPrepareFailure::Diagnosed(state.into_error(
            names_declarations::invalid_top_level_runtime_statement(Some(span)),
        )));
    }

    let mut output = if context.file_role.synthesizes_entry_start() {
        state
            .into_entry_output(
                owner,
                source_file,
                path_syntax,
                end_index,
                context.file_role,
            )
            .map_err(FileFrontendPrepareFailure::Infrastructure)?
    } else {
        state.into_non_entry_output(owner, source_file, path_syntax, context.file_role)
    };
    let canonical_owner = output
        .source_token_stream
        .as_ref()
        .map(Arc::clone)
        .ok_or_else(|| {
            FileFrontendPrepareFailure::Infrastructure(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "prepared file output is missing its canonical source token owner",
            ))
        })?;
    if let Err(failure) = attach_structural_file_facts(
        &mut output,
        &canonical_owner,
        context.string_table,
        context.path_fork,
    ) {
        return Err(FileFrontendPrepareFailure::from_header_parse(
            failure,
            output.warnings,
        ));
    }
    Ok(output)
}

/// Classify this file's graph-active file-value paths and derive shell ordering facts from them.
///
/// WHAT: recognizes expression-qualified occurrences and classifies every non-consumed path row
///       once, then records content-source ordering hints from the classified rows into each
///       declaration shell that folds before body emission.
/// WHY: classification is the single graph-activity fact source, and the content ordering edges
///       must come from the same prepared rows at token level rather than an expression parse.
///       Source `#Config` initializers are excluded until their primitive-only contract validation
///       succeeds, so an invalid default cannot affect Stage 0 source discovery.
fn attach_structural_file_facts(
    output: &mut FileFrontendPrepareOutput,
    canonical: &SourceTokens,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), HeaderParseFailure> {
    let is_config_file = path_fork
        .component(output.source_file)
        .is_some_and(|name| file_name_is_config_file(string_table.resolve(name)));
    if canonical.source() != output.file_id {
        return Err(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "source token owner does not match its file stream identity",
        )
        .into());
    }
    let mut consumed_path_syntax_ids = Vec::new();
    consumed_path_syntax_ids.extend(
        output
            .file_dependency_clauses
            .iter()
            .map(|clause| clause.dependency.path_syntax),
    );
    let mut qualified_reference_ranges = Vec::new();
    fn append_source_range_paths(
        output: &mut Vec<crate::compiler_frontend::paths::path_syntax::PathSyntaxId>,
        canonical: &SourceTokens,
        range: TokenRange,
    ) -> Result<(), InfrastructureFailure> {
        let cursor = canonical.cursor(range).map_err(|error| {
            InfrastructureFailure::internal(CompilerStage::HeaderParsing, format!(
                "retained declaration range could not be resolved for path collection: {error:?}"
            ))
        })?;
        append_source_paths(output, cursor);
        Ok(())
    }
    fn append_source_paths(
        output: &mut Vec<crate::compiler_frontend::paths::path_syntax::PathSyntaxId>,
        mut cursor: TokenCursor<'_>,
    ) {
        while let Some(token) = cursor.advance() {
            if let Some(path_syntax_id) = token.path_syntax_id() {
                output.push(path_syntax_id);
            }
            if token.is_eof() {
                break;
            }
        }
    }

    for (header_index, header) in output.headers.iter().enumerate() {
        if header.tokens.source() != output.file_id {
            return Err(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "retained header range does not match its source token owner",
            )
            .into());
        }
        let declaration_owned_marker = matches!(
            &header.kind,
            HeaderKind::Constant { declaration } if declaration.config_qualifier.is_some()
        );
        let body_cursor =
            if let Some(sequence) = header.token_sequence {
                let view = canonical.token_sequence(sequence).map_err(|error| {
                    InfrastructureFailure::internal(
                        CompilerStage::HeaderParsing,
                        format!("retained header token sequence could not be resolved: {error:?}"),
                    )
                })?;
                Some(view.cursor().map_err(|error| {
                InfrastructureFailure::internal(CompilerStage::HeaderParsing, format!(
                    "retained header token sequence cursor could not be constructed: {error:?}"
                ))
            })?)
            } else if header.tokens.is_empty() {
                None
            } else {
                Some(canonical.cursor(header.tokens).map_err(|error| {
                    InfrastructureFailure::internal(
                        CompilerStage::HeaderParsing,
                        format!("retained header token range could not be resolved: {error:?}"),
                    )
                })?)
            };
        let body_has_config_marker = body_cursor
            .is_some_and(|cursor| source_cursor_contains_config_marker(cursor, string_table));
        let declaration_has_config_marker = if declaration_owned_marker {
            false
        } else {
            find_config_qualifier_marker_in_declaration_defaults(header, canonical, string_table)?
                .is_some()
        };
        if is_config_file {
            // Config syntax paths are values, not qualified-reference occurrences.
            continue;
        }
        let config_owned =
            declaration_owned_marker || body_has_config_marker || declaration_has_config_marker;
        if !config_owned {
            // Ordinary expression context: recognize `PATH SYMBOL` occurrences in this header's
            // body, initializer and defaults. Configuration-qualified bodies stay out of scope.
            push_header_expression_ranges(
                header,
                header_index,
                canonical,
                &mut qualified_reference_ranges,
            )?;
            continue;
        }

        // Config-qualified shells are not expression syntax, but their path rows are still
        // consumed so the file-reference classifier never treats them as file values. Reuse the
        // same range collection the expression lane uses instead of repeating its header match.
        let mut config_owned_ranges = Vec::new();
        push_header_expression_ranges(header, header_index, canonical, &mut config_owned_ranges)?;
        for expression_range in config_owned_ranges {
            append_source_range_paths(
                &mut consumed_path_syntax_ids,
                canonical,
                expression_range.range,
            )?;
        }
    }

    let first_occurrence_shell_ordinal = u32::try_from(output.file_dependency_clauses.len())
        .map_err(|_| {
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "dependency clause count exceeded the file-local shell ordinal domain",
            )
        })?;
    let qualified_references = collect_qualified_references(
        canonical,
        qualified_reference_ranges,
        output.file_id,
        first_occurrence_shell_ordinal,
        output.path_syntax.table(),
        string_table,
        path_fork,
    )?;
    consumed_path_syntax_ids.extend(
        qualified_references
            .iter()
            .map(|occurrence| occurrence.dependency.path_syntax),
    );
    output.qualified_references = qualified_references;

    output.structural_file_references = classify_prepared_file_references(
        output.path_syntax.table(),
        consumed_path_syntax_ids,
        output.file_id,
        path_fork,
        string_table,
    );

    collect_content_source_ordering_hints(
        &mut output.headers,
        canonical,
        &output.structural_file_references,
        output.path_syntax.table(),
        string_table,
        path_fork,
    )
    .map_err(HeaderParseFailure::Infrastructure)
}

/// Scan one retained source-owned range for a `#Config` marker without materializing it.
///
/// The structural fact pass only needs marker presence; placement and adjacency diagnostics
/// remain on the span-aware `find_config_qualifier_marker_in_cursor` header paths.
fn source_cursor_contains_config_marker(
    mut cursor: TokenCursor<'_>,
    string_table: &StringTable,
) -> bool {
    let Some(mut previous) = cursor.advance() else {
        return false;
    };
    loop {
        let crosses_segment = cursor.is_at_segment_start();
        let Some(current) = cursor.advance() else {
            break;
        };
        if !crosses_segment
            && previous.tag() == crate::compiler_frontend::tokenizer::tokens::TokenTag::HASH
            && current.tag() == crate::compiler_frontend::tokenizer::tokens::TokenTag::SYMBOL
            && current
                .string_id()
                .is_some_and(|name| string_table.resolve(name) == "Config")
        {
            return true;
        }
        if current.is_eof() {
            break;
        }
        previous = current;
    }
    false
}

/// Validate generic parameter names against every dependency binding retained by this file.
///
/// WHAT: applies the file-wide dependency collision policy after header parsing has retained all
///       clauses and declaration shells, regardless of their source order.
/// WHY: dependency visibility is independent of declaration position, while header dispatch must
///       parse generic syntax before later dependency clauses have been encountered.
fn dependency_generic_parameter_collision(
    headers: &[Header],
    dependency_clauses: &[RetainedDependencyClause],
    dependency_selections: &[DependencySelection],
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Option<DiagnosticDraft> {
    let forbidden_names = dependency_generic_parameter_forbidden_names(
        dependency_clauses,
        dependency_selections,
        string_table,
        path_fork,
    );

    for header in headers {
        let generic_parameters = match &header.kind {
            HeaderKind::Function {
                generic_parameters, ..
            }
            | HeaderKind::Struct {
                generic_parameters, ..
            }
            | HeaderKind::Choice {
                generic_parameters, ..
            } => generic_parameters,
            _ => continue,
        };

        for parameter in &generic_parameters.parameters {
            if forbidden_names.contains(&parameter.name) {
                return Some(names_declarations::invalid_declaration(
                    InvalidDeclarationReason::GenericParameterNameCollision {
                        parameter_name: parameter.name,
                    },
                    None,
                    parameter.span,
                ));
            }
        }
    }

    None
}

fn dependency_generic_parameter_forbidden_names(
    dependency_clauses: &[RetainedDependencyClause],
    dependency_selections: &[DependencySelection],
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> FxHashSet<StringId> {
    let mut forbidden_names = FxHashSet::default();

    for clause in dependency_clauses {
        let selections = clause
            .selections(dependency_selections)
            .expect("validated file dependency selection range");
        if selections.is_empty() {
            if let Some(local_name) = clause.effective_namespace_local_name(string_table, path_fork)
            {
                forbidden_names.insert(local_name);
            }
        } else {
            for selection in selections {
                forbidden_names.insert(selection.local_name());
            }
        }
    }

    forbidden_names
}

#[cfg(test)]
#[path = "tests/structural_file_reference_tests.rs"]
mod structural_file_reference_tests;
