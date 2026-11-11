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
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_for_tag,
};
use crate::compiler_frontend::compiler_messages::{CapacityResource, CompilerStage};
use crate::compiler_frontend::compiler_messages::{
    InvalidDeclarationReason, InvalidReceiverDeclarationReason, InvalidTemplateDirectiveReason,
};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::headers::file_dependency_clauses::parse_and_record_dependency;
use crate::compiler_frontend::headers::file_state::HeaderFileParseState;
use crate::compiler_frontend::headers::header_dispatch::{
    HeaderDeclarationMetadata, create_header,
};
use crate::compiler_frontend::headers::ordering_hints::collect_content_source_ordering_hints;
use crate::compiler_frontend::headers::qualified_references::{
    collect_qualified_references, push_header_expression_ranges,
};
use crate::compiler_frontend::headers::start_capture::capture_runtime_template_range_from_cursor;
use crate::compiler_frontend::headers::symbol_collection::is_receiver_method_candidate;
use crate::compiler_frontend::headers::top_level_classifier::{
    HeaderFileItem, classify_current_item_ref, classify_symbol_statement_start_at_source,
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
use crate::compiler_frontend::style_directives::{
    SourceDirectiveAnnotation, SourceDirectiveKind, SourceModifierOperand,
    capture_source_directive_operand,
};
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

/// Records a statement-attached directive run into the entry body.
///
/// WHAT: carries the run's authored tokens up to the target token, where the AST statement parser
///       owns attachment.
/// WHY: top-level executable statements are re-parsed from the synthesized entry body. Rejecting
///      their directives at the header walk would make `$todo` unusable on the only source that
///      can own entry runtime statements.
fn record_pending_run_for_statement(
    canonical: &SourceTokens,
    state: &mut HeaderFileParseState,
    pending: Option<PendingSourceDirectiveRun>,
    target_index: TokenIndex,
) -> FileParserResult<()> {
    let Some(pending) = pending else {
        return Ok(());
    };
    let range =
        TokenRange::try_new_for(canonical, pending.start_index, target_index).map_err(|error| {
            HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                format!("source-directive run did not form a valid token range: {error:?}"),
            ))
        })?;
    state
        .record_start_body_source_range_from_source_tokens(range, canonical)
        .map_err(HeaderParseFailure::Infrastructure)
}

// Top-level declarations are same-module-visible by default; cross-module public visibility
// comes only from `$export`-marked facade items. Non-declaration statements are collected into
// the compiler-owned entry header for that file.

/// One contiguous source-directive run waiting for its target construct.
///
/// WHAT: retains the first authored directive token and every parsed annotation in order.
/// WHY: a run attaches as one unit once the target item is known. Keeping the start index lets a
///      run whose target is an executable statement be recorded into the entry body, where the
///      statement parser owns attachment without a second header lookahead.
#[derive(Clone)]
struct PendingSourceDirectiveRun {
    start_index: TokenIndex,
    annotations: Vec<SourceDirectiveAnnotation>,
    /// Set when the walk observes a line break after the most recent directive.
    ///
    /// WHAT: resets when the run gains another directive.
    /// WHY: the target must start on a later line than the final directive, not merely later than
    ///      the first one.
    saw_line_break: bool,
}

impl PendingSourceDirectiveRun {
    fn first(&self) -> Option<&SourceDirectiveAnnotation> {
        self.annotations.first()
    }

    fn contains_name(&self, name_id: StringId) -> Option<SourceSpan> {
        self.annotations
            .iter()
            .find(|annotation| annotation.name_id == name_id)
            .map(|annotation| annotation.span)
    }
}

/// Build the diagnostic for a source directive that never reached an eligible target.
fn dangling_source_directive(annotation: &SourceDirectiveAnnotation) -> DiagnosticDraft {
    templates::invalid_template_directive(
        Some(annotation.name_id),
        InvalidTemplateDirectiveReason::DanglingModifier,
        Some(annotation.span),
    )
}

/// Build the diagnostic for a source directive whose next item cannot accept it.
fn wrong_target_source_directive(annotation: &SourceDirectiveAnnotation) -> DiagnosticDraft {
    templates::invalid_template_directive(
        Some(annotation.name_id),
        InvalidTemplateDirectiveReason::WrongTarget,
        Some(annotation.span),
    )
}

/// Whether this file may author the directive's declared target at all.
///
/// WHAT: `$export` belongs only to an API facade root; `$config` keeps its existing placement
///       contract, which its declaration-level target check owns. `$doc` and `$todo` are ordinary
///       source directives with no file-role restriction.
/// WHY: an ineligible file is a placement failure at the directive, not a missing or wrong target
///      on the next item.
fn source_directive_allowed_in_file(
    kind: SourceDirectiveKind,
    context: &HeaderParseContext<'_>,
) -> bool {
    match kind {
        SourceDirectiveKind::Config | SourceDirectiveKind::Doc | SourceDirectiveKind::Todo => true,
        SourceDirectiveKind::Export => {
            !context.is_config_file && context.file_role.is_export_capable()
        }
    }
}

/// Resolve one recognised directive token to its source-directive contract.
///
/// WHY: the lexer only emits registered names, and the header walk resolves the name's declared
///      context before parsing attachment. A known template-only name stays a typed
///      wrong-context error instead of falling into unknown-name or generic syntax diagnostics.
fn source_directive_for(
    context: &HeaderParseContext<'_>,
    name_id: StringId,
    span: SourceSpan,
) -> FileParserResult<(SourceDirectiveKind, SourceModifierOperand)> {
    let directive_name = context.string_table.resolve(name_id);
    let Some(spec) = context.style_directives.find(directive_name) else {
        return Err(diagnostic_failure(templates::invalid_template_directive(
            Some(name_id),
            InvalidTemplateDirectiveReason::UnknownDirective,
            Some(span),
        )));
    };

    if !spec.contexts.allows_source_modifier() {
        return Err(diagnostic_failure(templates::invalid_template_directive(
            Some(name_id),
            InvalidTemplateDirectiveReason::WrongContext,
            Some(span),
        )));
    }
    match spec.kind {
        crate::compiler_frontend::style_directives::StyleDirectiveKind::SourceModifier {
            target,
            operand,
        } => Ok((target.directive_kind(), operand)),
        _ => Err(diagnostic_failure(templates::invalid_template_directive(
            Some(name_id),
            InvalidTemplateDirectiveReason::WrongContext,
            Some(span),
        ))),
    }
}

/// Whether one declaration header accepts the source directive's declared target.
///
/// WHAT: compares the built declaration kind against the spec's eligible-target contract.
/// WHY: attachment is a syntactic fact, but recording the directive on an ineligible declaration
///      would hand a later stage a target its contract cannot accept.
fn header_accepts_source_directive(kind: &HeaderKind, directive: SourceDirectiveKind) -> bool {
    match directive {
        SourceDirectiveKind::Config => {
            matches!(
                kind,
                HeaderKind::Constant { declaration }
                    if declaration.binding_mode.is_compile_time()
                        && !matches!(declaration.type_annotation, ParsedTypeRef::Inferred)
            )
        }
        SourceDirectiveKind::Export => kind.is_authored_public_export_declaration(),
        SourceDirectiveKind::Doc | SourceDirectiveKind::Todo => matches!(
            kind,
            HeaderKind::Function { .. }
                | HeaderKind::Struct { .. }
                | HeaderKind::Choice { .. }
                | HeaderKind::TypeAlias { .. }
                | HeaderKind::Trait { .. }
                | HeaderKind::Constant { .. }
        ),
    }
}

/// Whether a dependency clause accepts the directive's declared target.
fn dependency_accepts_source_directive(directive: SourceDirectiveKind) -> bool {
    matches!(directive, SourceDirectiveKind::Export)
}

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
    // `:` opens one executable control or value-producing block in the entry body; its matching
    // `;` closes it. Header parsing sees direct templates before the AST statement owner, so it
    // must keep a template inside such a block in the runtime body instead of promoting it to
    // unconditional const metadata. Parentheses, collections and braces already live in
    // `runtime_nesting`, so only top-level colons and semicolons update this counter.
    let mut executable_block_depth = 0usize;
    // One source-modifier run awaiting the next eligible top-level declaration or dependency
    // clause.
    let mut pending_source_directive: Option<PendingSourceDirectiveRun> = None;
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
                let range = capture_runtime_template_range_from_cursor(current, cursor, file_id)?;
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
        if runtime_nesting.is_top_level() {
            match current_tag {
                TokenTag::COLON => {
                    executable_block_depth = executable_block_depth.saturating_add(1);
                }
                TokenTag::END => {
                    executable_block_depth = executable_block_depth.saturating_sub(1);
                }
                _ => {}
            }
        }
        let classified_item = classify_current_item_ref(current, follower, at_boundary);
        let item = if current_tag == TokenTag::STYLE_DIRECTIVE
            && pending_source_directive.is_some()
            && let Some(name_id) = current.string_id()
        {
            // A same-line directive continues the pending run even though it is not a fresh
            // statement boundary. The duplicate check still runs in the directive arm.
            HeaderFileItem::StyleDirective(name_id)
        } else if executable_block_depth > 0
            && matches!(
                classified_item,
                HeaderFileItem::DirectTemplate
                    | HeaderFileItem::RuntimeTemplate
                    | HeaderFileItem::Symbol(_)
                    | HeaderFileItem::StyleDirective(_)
                    | HeaderFileItem::Dependency
                    | HeaderFileItem::BuiltinTypeConformanceTarget(_)
                    | HeaderFileItem::ReservedTraitSyntax
            )
        {
            // A template, directive run or declaration inside `if`/`loop`/`choice`/`catch`
            // belongs to that statement body. The AST statement owner decides attachment and
            // whether the authored form is valid there; the header walk must not register a
            // nested local as a module header.
            HeaderFileItem::StartBodyToken
        } else {
            classified_item
        };

        if let Some(pending) = pending_source_directive.as_ref() {
            if current_tag == TokenTag::NEWLINE {
                // A blank physical line ends the directive run; comment-only lines keep it
                // because comments produce no token and do not add a line break to this token.
                if current.shape().newline_has_blank_line() {
                    return Err(diagnostic_failure(dangling_source_directive(
                        pending
                            .first()
                            .expect("a pending source directive run is never empty"),
                    )));
                }
                if let Some(pending) = pending_source_directive.as_mut() {
                    pending.saw_line_break = true;
                }
                cursor.advance();
                continue;
            }

            if !pending.saw_line_break
                && !matches!(
                    item,
                    HeaderFileItem::StyleDirective(_) | HeaderFileItem::Eof
                )
            {
                return Err(diagnostic_failure(wrong_target_source_directive(
                    pending
                        .first()
                        .expect("a pending source directive run is never empty"),
                )));
            }

            match &item {
                // The next directive is handled by its own arm so a repeated name can cite both
                // authored spans.
                HeaderFileItem::StyleDirective(_) => {}
                HeaderFileItem::Dependency => {
                    if let Some(annotation) = pending
                        .annotations
                        .iter()
                        .find(|annotation| !dependency_accepts_source_directive(annotation.kind))
                    {
                        return Err(diagnostic_failure(wrong_target_source_directive(
                            annotation,
                        )));
                    }
                }
                HeaderFileItem::Symbol(_) => {
                    let starts_declaration =
                        classify_symbol_statement_start_at_source(canonical, follower_index)
                            .starts_header_declaration();
                    if !starts_declaration {
                        if let Some(annotation) = pending.annotations.iter().find(|annotation| {
                            !matches!(
                                annotation.kind,
                                SourceDirectiveKind::Doc | SourceDirectiveKind::Todo
                            )
                        }) {
                            return Err(diagnostic_failure(wrong_target_source_directive(
                                annotation,
                            )));
                        }
                        // A top-level entry binding is a runtime-body local declaration. The
                        // statement owner distinguishes it from an ordinary expression and
                        // attaches `$doc` only when the declaration shape is eligible.
                        record_pending_run_for_statement(
                            canonical,
                            state,
                            pending_source_directive.take(),
                            current_index,
                        )?;
                        continue;
                    }
                }
                // Declaration kinds that cannot accept a directive are still real targets, so
                // attachment reports a wrong target rather than a missing one.
                HeaderFileItem::BuiltinTypeConformanceTarget(_) => {}
                HeaderFileItem::DirectTemplate => {
                    return Err(diagnostic_failure(wrong_target_source_directive(
                        pending
                            .first()
                            .expect("a pending source directive run is never empty"),
                    )));
                }
                HeaderFileItem::RuntimeTemplate
                | HeaderFileItem::StartBodyToken
                | HeaderFileItem::ReservedTraitSyntax => {
                    if let Some(annotation) = pending.annotations.iter().find(|annotation| {
                        !matches!(
                            annotation.kind,
                            SourceDirectiveKind::Doc | SourceDirectiveKind::Todo
                        )
                    }) {
                        return Err(diagnostic_failure(wrong_target_source_directive(
                            annotation,
                        )));
                    }
                    // Statement attachment validates `$doc` against a declaration and accepts
                    // `$todo` on executable statements. Carry the complete run into the entry
                    // body instead of rejecting declaration targets here.
                    record_pending_run_for_statement(
                        canonical,
                        state,
                        pending_source_directive.take(),
                        current_index,
                    )?;
                    continue;
                }
                HeaderFileItem::Eof => {
                    return Err(diagnostic_failure(dangling_source_directive(
                        pending
                            .first()
                            .expect("a pending source directive run is never empty"),
                    )));
                }
            }
        }

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
                        source_directives: pending_source_directive
                            .take()
                            .map(|run| run.annotations)
                            .unwrap_or_default(),
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
                        source_directives: pending_source_directive
                            .take()
                            .map(|run| run.annotations)
                            .unwrap_or_default(),
                    },
                )?;
            }

            HeaderFileItem::Dependency => {
                let annotations = pending_source_directive
                    .take()
                    .map(|run| run.annotations)
                    .unwrap_or_default();
                let is_export = annotations
                    .iter()
                    .any(|annotation| annotation.kind == SourceDirectiveKind::Export);
                let export_mode = if is_export {
                    HeaderExportMode::Public
                } else {
                    HeaderExportMode::Private
                };
                parse_and_record_dependency(
                    cursor,
                    file_id,
                    state,
                    context,
                    export_mode,
                    current_span,
                )?;
            }

            HeaderFileItem::DirectTemplate => {
                handle_direct_template_item(
                    cursor,
                    file_id,
                    source_file,
                    state,
                    context,
                    current_span,
                )?;
            }

            HeaderFileItem::ReservedTraitSyntax => {
                handle_trait_keyword_header_item(current_tag, current_span)?;
            }

            HeaderFileItem::RuntimeTemplate => {
                let opening_index = cursor.position();
                let opening = cursor.current().ok_or_else(|| {
                    HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                        CompilerStage::HeaderParsing,
                        "runtime template marker exceeded its source token owner",
                    ))
                })?;
                if opening.tag() != TokenTag::TEMPLATE_HEAD {
                    return Err(HeaderParseFailure::Infrastructure(
                        InfrastructureFailure::internal(
                            CompilerStage::HeaderParsing,
                            format!(
                                "runtime template marker was not followed by a template opening: {:?}",
                                opening.tag().descriptor().text()
                            ),
                        ),
                    ));
                }
                cursor.advance();
                handle_runtime_template_item(
                    cursor,
                    file_id,
                    state,
                    context,
                    opening_index,
                    current_index,
                )?;
            }

            HeaderFileItem::StyleDirective(name_id) => {
                // Resolve the incoming name before comparing it with the pending run. A
                // template-only directive that follows `$config` is a wrong-context use, not a
                // duplicate of the pending source directive.
                let (kind, operand_contract) =
                    source_directive_for(context, name_id, current_span)?;
                if !source_directive_allowed_in_file(kind, context) {
                    return Err(diagnostic_failure(templates::invalid_template_directive(
                        Some(name_id),
                        InvalidTemplateDirectiveReason::DirectiveNotAllowedHere,
                        Some(current_span),
                    )));
                }

                if let Some(previous_span) = pending_source_directive
                    .as_ref()
                    .and_then(|pending| pending.contains_name(name_id))
                {
                    return Err(diagnostic_failure(
                        templates::invalid_template_directive(
                            Some(name_id),
                            InvalidTemplateDirectiveReason::DuplicateModifier,
                            Some(current_span),
                        )
                        .with_label(
                            Some(previous_span),
                            Some(LabelDraftMessage::EarlierDirective),
                        ),
                    ));
                }
                if pending_source_directive.is_some()
                    && current.shape().style_directive_is_attached()
                {
                    return Err(diagnostic_failure(templates::invalid_template_directive(
                        Some(name_id),
                        InvalidTemplateDirectiveReason::MissingSeparator,
                        Some(current_span),
                    )));
                }

                let operand = capture_source_directive_operand(
                    cursor,
                    operand_contract,
                    name_id,
                    current_span,
                )?;
                let annotation = SourceDirectiveAnnotation {
                    kind,
                    name_id,
                    span: current_span,
                    operand,
                };
                match &mut pending_source_directive {
                    Some(pending) => {
                        pending.annotations.push(annotation);
                        pending.saw_line_break = false;
                    }
                    None => {
                        pending_source_directive = Some(PendingSourceDirectiveRun {
                            start_index: current_index,
                            annotations: vec![annotation],
                            saw_line_break: false,
                        })
                    }
                }
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

/// One already-read top-level symbol with the source facts its header dispatch needs.
struct SymbolItemRequest {
    name_id: StringId,
    current_span: SourceSpan,
    current_index: TokenIndex,
    /// Ordered source directives that attach to this declaration.
    source_directives: Vec<SourceDirectiveAnnotation>,
}

fn handle_symbol_item(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    source_file: PathId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    symbol: SymbolItemRequest,
) -> FileParserResult<()> {
    let SymbolItemRequest {
        name_id,
        current_span,
        current_index,
        source_directives,
    } = symbol;
    // `$export` is the only authored public-surface form. An unmarked declaration stays private.
    let export_mode = if source_directives
        .iter()
        .any(|directive| directive.kind == SourceDirectiveKind::Export)
    {
        HeaderExportMode::Public
    } else {
        HeaderExportMode::Private
    };
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
    let mut header = create_header(
        declaration_path,
        cursor,
        file_id,
        HeaderDeclarationMetadata {
            declaration_span: current_span,
            declaration_order: current_index.index(),
            export_mode,
            config_input_span: source_directives
                .iter()
                .find(|directive| directive.kind == SourceDirectiveKind::Config)
                .map(|directive| directive.span),
        },
        &mut build_context,
        context.span_builder,
    )?;

    if let Some(directive) = source_directives
        .iter()
        .find(|directive| !header_accepts_source_directive(&header.kind, directive.kind))
    {
        return Err(diagnostic_failure(wrong_target_source_directive(directive)));
    }
    header.source_directives = source_directives;

    // Structural directive targets are already settled by the pending-run preflight and
    // `header_accepts_source_directive`; only receiver-method ownership needs an extra check.
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

/// Parses one bare direct top-level template as compile-time output.
///
/// WHAT: reuses the retained const-template construction path and records its runtime insertion
///       index. The opening token is already consumed by the header walk.
/// WHY: an executable entry's bare `[...]` is page or builder-slot metadata, never a runtime body
///      fragment. Reusing the existing path keeps slot registration, folding and builder ordering
///      unchanged while the spelling loses its `#` prefix.
fn handle_direct_template_item(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    source_file: PathId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    current_span: SourceSpan,
) -> FileParserResult<()> {
    if context.file_role == FileRole::Normal {
        return Err(names_declarations::deferred_feature(
            context
                .string_table
                .intern("top-level const templates in ordinary source files"),
            Some(current_span),
        )
        .into());
    }

    // A reached foreign facade keeps its declaration surface but owns no start or root runtime
    // in this compilation.
    if matches!(context.file_role, FileRole::ReachedApiFacadeRoot(_)) {
        return Err(
            names_declarations::invalid_top_level_runtime_statement(Some(current_span)).into(),
        );
    }

    if matches!(context.file_role, FileRole::ActiveApiOnlyModuleRoot(_)) {
        return Err(
            names_declarations::invalid_top_level_runtime_statement(Some(current_span)).into(),
        );
    }

    let opening_index = cursor
        .position()
        .index()
        .checked_sub(1)
        .and_then(TokenIndex::try_from_index)
        .ok_or_else(|| {
            HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "direct template opening preceded the source token index",
            ))
        })?;
    let canonical = cursor.source_tokens();
    if canonical.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(
            InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "direct template source token owner does not match its file identity",
            ),
        ));
    }
    let opening = canonical.token(opening_index).map_err(|error| {
        HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            format!("direct template opening exceeded its source token owner: {error:?}"),
        ))
    })?;

    let const_template_number = context
        .const_template_offset
        .checked_add(state.const_template_count)
        .ok_or_else(|| {
            HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "const-template number overflowed its module source range",
            ))
        })?;

    let mut build_context = HeaderBuildContext {
        warnings: &mut state.warnings,
        source_file,
        file_dependency_clauses: &state.file_dependency_clauses,
        dependency_selections: &state.dependency_selections,
        string_table: context.string_table,
        path_fork: context.path_fork,
        file_role: context.file_role,
    };
    let header =
        crate::compiler_frontend::headers::const_fragments::create_top_level_const_template(
            source_file,
            opening,
            const_template_number,
            cursor,
            &mut build_context,
            context.span_builder,
        )?;

    state.const_template_count = state.const_template_count.checked_add(1).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "const-template count overflowed its source state",
        ))
    })?;

    let runtime_insertion_index = context
        .runtime_fragment_offset
        .checked_add(state.runtime_fragment_count)
        .ok_or_else(|| {
            HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
                CompilerStage::HeaderParsing,
                "runtime insertion index overflowed its module source range",
            ))
        })?;
    let fragment_path = header.declaration_path;
    let fragment = crate::compiler_frontend::headers::types::TopLevelConstFragment {
        runtime_insertion_index,
        span: header
            .name_span
            .expect("authored const-template headers carry a source span"),
        header_path: fragment_path,
    };
    state.register_top_level_const_fragment(fragment, header);

    Ok(())
}

fn handle_runtime_template_item(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    state: &mut HeaderFileParseState,
    context: &mut HeaderParseContext<'_>,
    opening_index: TokenIndex,
    marker_index: TokenIndex,
) -> FileParserResult<()> {
    // Runtime top-level templates stay in the start-function body and are evaluated in source
    // order by the compiler-owned entry. The runtime fragment count lets later const fragments
    // record their insertion point relative to already-seen runtime fragments.
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
    let range = capture_runtime_template_range_from_cursor(opening, cursor, file_id)?;
    // The body dispatcher needs the authored `~` marker to distinguish the runtime fragment from
    // compile-time page metadata, so the retained start range begins at the marker.
    let range = TokenRange::new(file_id, marker_index, range.end()).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
            CompilerStage::HeaderParsing,
            "runtime template marker range was reversed",
        ))
    })?;
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

pub(super) fn finish_file_output(
    owner: SourceTokenOwner,
    source_file: crate::compiler_frontend::symbols::path_interner::PathId,
    path_syntax: Arc<PathSyntaxTable>,
    end_index: TokenIndex,
    context: &mut HeaderParseContext<'_>,
    state: HeaderFileParseState,
) -> Result<FileFrontendPrepareOutput, FileFrontendPrepareFailure> {
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
///       Source `$config` initializers are excluded until their primitive-only contract validation
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
        if is_config_file {
            // Config syntax paths are values, not qualified-reference occurrences.
            continue;
        }
        let config_owned = matches!(
            &header.kind,
            HeaderKind::Constant { declaration } if declaration.config_contract.is_some()
        );
        if !config_owned {
            // Ordinary expression context: recognize `PATH SYMBOL` occurrences in this header's
            // body, initializer and defaults.
            push_header_expression_ranges(
                header,
                header_index,
                canonical,
                &mut qualified_reference_ranges,
            )?;
            continue;
        }

        // A contract's declared type is contract vocabulary, but its initializer and body path
        // rows are still consumed so the file-reference classifier never treats them as file
        // values. Reuse the same range collection the expression lane uses instead of repeating
        // its header match.
        let mut config_owned_ranges = Vec::new();
        push_header_expression_ranges(header, header_index, canonical, &mut config_owned_ranges)?;
        for expression_range in config_owned_ranges {
            // A source-directive operand is metadata syntax, not part of the config contract
            // value. Keep it in the ordinary expression lane so its content and qualified
            // references stay visible to ordering and file-reference discovery.
            if header
                .source_directives
                .iter()
                .any(|directive| directive.operand == Some(expression_range.range))
            {
                qualified_reference_ranges.push(
                    crate::compiler_frontend::headers::qualified_references::HeaderExpressionRange::const_required(
                        header_index,
                        expression_range.range,
                    ),
                );
                continue;
            }
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
