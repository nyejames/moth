//! Template head parsing orchestration.
//!
//! WHAT:
//! - Implements the top-level `parse_template_head(...)` loop.
//! - Owns token-category dispatch, separator handling, and stream-boundary checks.
//! - Delegates expression and directive behavior to focused helper modules.
//!
//! WHY:
//! - Keeps the main head parser readable while preserving strict control of which
//!   token kinds are valid in the head grammar.

use super::control_flow_suffix::{parse_if_suffix, parse_loop_suffix};
use super::core_directives::{
    mark_template_body_whitespace_style_controlled, maybe_parse_slot_or_insert_helper_directive,
    parse_core_style_directive,
};
use super::handler_directives::apply_handler_style_directive;
use super::head_expressions::{
    TemplateHeadExpressionContext, handle_template_value_in_template_head,
    push_template_head_expression, push_template_head_path_expression,
};
use super::reactive_subscriptions::parse_reactive_subscription;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template_build_state::TemplateBuildState;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBodyParseMode, TemplateControlFlowValidationMode,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateConstructionContext, walk_expression_payloads_with_nested_tir_views,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;

use crate::ast_log;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateDirectiveReason, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::style_directives::{
    StyleDirectiveKind, StyleDirectiveSpec, TemplateHeadCompatibility, TemplateHeadTag,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::utilities::token_scan::NestingDepth;
use crate::compiler_frontend::value_mode::ValueMode;

/// Template-head parsing is a stage-local join. It keeps a retained-data lifecycle failure in
/// `TemplateError` until template construction returns to the expression/emission boundary.
type TemplateHeadResult<T> = Result<T, TemplateError>;

/// Result of parsing a template head.
pub(crate) struct ParsedTemplateHead {
    pub(crate) body_mode: TemplateBodyParseMode,
    pub(crate) has_explicit_template_directive: bool,
}

pub(crate) struct TemplateHeadParseRequest<'a, 'types> {
    pub(crate) context: &'a ScopeContext,
    pub(crate) type_interner: &'a mut AstTypeInterner<'types>,
    pub(crate) build_state: &'a mut TemplateBuildState,
    pub(crate) construction_context: &'a mut TemplateConstructionContext,
    pub(crate) control_flow_validation: TemplateControlFlowValidationMode,
    pub(crate) string_table: &'a mut StringTable,
}

#[derive(Clone, Copy, Debug, Default)]
struct TemplateHeadState {
    seen_tags: TemplateHeadTag,
    blocked_future_tags: TemplateHeadTag,
    has_explicit_template_directive: bool,
}

fn enforce_head_compatibility(
    state: &TemplateHeadState,
    incoming: &TemplateHeadCompatibility,
    token_stream: &FileTokens,
) -> TemplateHeadResult<()> {
    if !state.blocked_future_tags.intersects(incoming.presence_tags)
        && !state.seen_tags.intersects(incoming.required_absent_tags)
    {
        Ok(())
    } else {
        Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::IncompatibleHeadItem,
            token_stream.current_location(),
        )
        .into())
    }
}

fn apply_head_compatibility(
    state: &mut TemplateHeadState,
    compatibility: &TemplateHeadCompatibility,
) {
    state.seen_tags |= compatibility.presence_tags;
    state.blocked_future_tags |= compatibility.blocks_future_tags;
}

fn parsed_template_head(
    body_mode: TemplateBodyParseMode,
    head_state: &TemplateHeadState,
) -> ParsedTemplateHead {
    ParsedTemplateHead {
        body_mode,
        has_explicit_template_directive: head_state.has_explicit_template_directive,
    }
}

fn should_inline_template_head_reference(
    token_stream: &FileTokens,
    context: &ScopeContext,
    declaration: &Declaration,
) -> bool {
    if token_stream.peek_next_token() != Some(&TokenKind::TemplateClose) {
        return true;
    }

    if context.kind.is_constant_context() {
        return true;
    }

    // Head-only runtime template references are value reads, not receiver
    // applications. Runtime slot handoffs already carry the composition-owned
    // wrapper/source plan, so copying an already-materialized template would
    // lose that plan when the surrounding template later crosses into HIR.
    !expression_contains_runtime_slot_handoff(&declaration.value, context)
}

fn expression_contains_runtime_slot_handoff(
    expression: &Expression,
    context: &ScopeContext,
) -> bool {
    let store = context.template_ir_store.borrow();

    let mut contains_runtime_slot_handoff = false;
    let walk_result =
        walk_expression_payloads_with_nested_tir_views(expression, &store, &mut |payload| {
            if matches!(
                payload.kind,
                ExpressionKind::RuntimeSlotApplicationHandoff(_)
            ) {
                contains_runtime_slot_handoff = true;
            }
            Ok(())
        });

    walk_result.is_err() || contains_runtime_slot_handoff
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TemplateHeadSeparatorState {
    ExpectItem,
    ExpectSeparatorOrBody,
}

/// Parses meaningful head items until `:`, `]`, or a control-flow suffix body.
///
/// The explicit early returns make token-state exits visible in this parser
/// state machine: each accepted boundary or diagnostic exits immediately.
pub fn parse_template_head(
    token_stream: &mut FileTokens,
    request: TemplateHeadParseRequest<'_, '_>,
) -> TemplateHeadResult<ParsedTemplateHead> {
    let TemplateHeadParseRequest {
        context,
        type_interner,
        build_state,
        construction_context,
        control_flow_validation,
        string_table,
    } = request;

    // Each meaningful head item must be separated with a comma before another
    // item can start. Naming the state keeps suffix and body-boundary handling
    // readable in this parser state machine.
    let mut separator_state = TemplateHeadSeparatorState::ExpectItem;
    let mut head_state = TemplateHeadState::default();
    let meaningful_item_compatibility = TemplateHeadCompatibility::fully_compatible_meaningful();
    token_stream.advance();

    let mut last_known_location = token_stream.current_location();
    while token_stream.index < token_stream.length {
        last_known_location = token_stream.current_location();
        let token = token_stream.current_token_kind().to_owned();

        ast_log!("Parsing template head: ", #token);

        // We are doing something similar to new_ast()
        // But with the specific template head syntax,
        // expressions are allowed and should be folded where possible.
        // Loops and if statements can end the template head.

        // EOF inside a template head means the source was truncated before a
        // closing ] delimiter. This is a malformed template, not a valid stream
        // boundary; the user needs a structured diagnostic.
        if token == TokenKind::Eof {
            return Err(CompilerDiagnostic::unexpected_end_of_file(
                Some(string_table.intern("]")),
                token_stream.current_location(),
            )
            .into());
        }

        // A closing ] without a body is a valid empty template.
        if token == TokenKind::TemplateClose {
            return Ok(parsed_template_head(
                TemplateBodyParseMode::Normal,
                &head_state,
            ));
        }

        if token == TokenKind::StartTemplateBody {
            if head_state
                .seen_tags
                .intersects(TemplateHeadTag::SLOT_DIRECTIVE)
            {
                return Err(CompilerDiagnostic::invalid_template_structure(
                    InvalidTemplateStructureReason::SlotInHead,
                    token_stream.current_location(),
                )
                .into());
            }

            token_stream.advance();
            return Ok(parsed_template_head(
                TemplateBodyParseMode::Normal,
                &head_state,
            ));
        }

        if separator_state == TemplateHeadSeparatorState::ExpectItem
            && !matches!(token, TokenKind::If | TokenKind::Loop)
            && let Some(control_flow_location) = find_unseparated_control_flow_suffix(token_stream)
        {
            return Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MissingCommaBeforeControlFlowSuffix,
                control_flow_location,
            )
            .into());
        }

        // Make sure there is a comma before the next token.
        if separator_state == TemplateHeadSeparatorState::ExpectSeparatorOrBody {
            if matches!(token, TokenKind::If | TokenKind::Loop) {
                return Err(CompilerDiagnostic::invalid_template_structure(
                    InvalidTemplateStructureReason::MissingCommaBeforeControlFlowSuffix,
                    token_stream.current_location(),
                )
                .into());
            }

            if token != TokenKind::Comma {
                return Err(CompilerDiagnostic::expected_token(
                    TokenKind::Comma,
                    Some(token),
                    token_stream.current_location(),
                )
                .into());
            }

            separator_state = TemplateHeadSeparatorState::ExpectItem;
            token_stream.advance();
            continue;
        }

        let mut defer_comma_advance = false;

        match token {
            TokenKind::If => {
                if head_state
                    .seen_tags
                    .intersects(TemplateHeadTag::SLOT_DIRECTIVE)
                {
                    return Err(CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::SlotInHead,
                        token_stream.current_location(),
                    )
                    .into());
                }

                let body_mode = parse_if_suffix(
                    token_stream,
                    context,
                    type_interner,
                    control_flow_validation,
                    string_table,
                )?;
                return Ok(parsed_template_head(body_mode, &head_state));
            }

            TokenKind::Loop => {
                if head_state
                    .seen_tags
                    .intersects(TemplateHeadTag::SLOT_DIRECTIVE)
                {
                    return Err(CompilerDiagnostic::invalid_template_structure(
                        InvalidTemplateStructureReason::SlotInHead,
                        token_stream.current_location(),
                    )
                    .into());
                }

                let body_mode = parse_loop_suffix(
                    token_stream,
                    context,
                    type_interner,
                    control_flow_validation,
                    string_table,
                )?;
                return Ok(parsed_template_head(body_mode, &head_state));
            }

            TokenKind::Else => {
                return Err(CompilerDiagnostic::invalid_template_structure(
                    InvalidTemplateStructureReason::ElseInTemplateHead,
                    token_stream.current_location(),
                )
                .into());
            }

            TokenKind::Reactive => {
                enforce_head_compatibility(
                    &head_state,
                    &meaningful_item_compatibility,
                    token_stream,
                )?;
                parse_reactive_subscription(
                    token_stream,
                    context,
                    type_interner.environment(),
                    construction_context,
                    string_table,
                )?;
                defer_comma_advance = true;
                apply_head_compatibility(&mut head_state, &meaningful_item_compatibility);
            }

            // Variable, template and dependency-namespace references.
            //
            // Known template references that should be inlined preserve their
            // wrapper/slot semantics. Everything else routes through the ordinary
            // expression parser so that namespace member access (`intro.content`),
            // bare dependency-namespace misuse (`intro`), field access and unknown names
            // all get structured diagnostics instead of generic `UnexpectedToken`.
            TokenKind::Symbol(name) => {
                enforce_head_compatibility(
                    &head_state,
                    &meaningful_item_compatibility,
                    token_stream,
                )?;
                let value_location = token_stream.current_location();

                // Extract an inlinable template before the mutable expression parse.
                // The borrow from get_reference is released once the template is cloned.
                let inlined_template = context.get_reference(&name).and_then(|reference| {
                    let declaration = reference.as_declaration();
                    match &declaration.value.kind {
                        ExpressionKind::Template(inserted_template)
                            if should_inline_template_head_reference(
                                token_stream,
                                context,
                                declaration,
                            ) =>
                        {
                            Some(inserted_template.as_ref().clone())
                        }
                        _ => None,
                    }
                });

                if let Some(inserted_template) = inlined_template {
                    handle_template_value_in_template_head(
                        &inserted_template,
                        context,
                        construction_context,
                        &value_location,
                    )?;
                } else {
                    // Resolve value_mode from the reference before the mutable
                    // expression parse so the context borrow does not overlap.
                    let value_mode = context
                        .get_reference(&name)
                        .map(|reference| reference.value.value_mode.to_owned())
                        .unwrap_or(ValueMode::ImmutableOwned);

                    let mut inferred = ExpectedType::Infer;
                    let expression = create_expression(
                        token_stream,
                        context,
                        type_interner,
                        &mut inferred,
                        &value_mode,
                        false,
                        string_table,
                    )
                    .map_err(TemplateError::from)?;

                    push_template_head_expression(
                        expression,
                        TemplateHeadExpressionContext {
                            context,
                            type_environment: type_interner.environment(),
                            construction_context,
                        },
                        &value_location,
                        string_table,
                    )?;
                    defer_comma_advance = true;
                }

                apply_head_compatibility(&mut head_state, &meaningful_item_compatibility);
            }

            // Receiver self-reference
            TokenKind::This => {
                let this_id = string_table.intern("this");
                if let Some(reference) = context.get_reference(&this_id) {
                    enforce_head_compatibility(
                        &head_state,
                        &meaningful_item_compatibility,
                        token_stream,
                    )?;
                    let value_location = token_stream.current_location();
                    let mut inferred = ExpectedType::Infer;
                    let expression = create_expression(
                        token_stream,
                        context,
                        type_interner,
                        &mut inferred,
                        &reference.value.value_mode,
                        false,
                        string_table,
                    )
                    .map_err(TemplateError::from)?;

                    push_template_head_expression(
                        expression,
                        TemplateHeadExpressionContext {
                            context,
                            type_environment: type_interner.environment(),
                            construction_context,
                        },
                        &value_location,
                        string_table,
                    )?;
                    defer_comma_advance = true;
                    apply_head_compatibility(&mut head_state, &meaningful_item_compatibility);
                } else {
                    return Err(CompilerDiagnostic::unexpected_token(
                        TokenKind::This,
                        token_stream.current_location(),
                    )
                    .into());
                }
            }

            // Constants can be inserted directly into parser TIR.
            // Literal values
            TokenKind::NumericLiteral(_)
            | TokenKind::BoolLiteral(_)
            | TokenKind::StringSliceLiteral(_)
            | TokenKind::RawStringLiteral(_) => {
                enforce_head_compatibility(
                    &head_state,
                    &meaningful_item_compatibility,
                    token_stream,
                )?;
                let value_location = token_stream.current_location();
                let mut inferred = ExpectedType::Infer;
                let expression = create_expression(
                    token_stream,
                    context,
                    type_interner,
                    &mut inferred,
                    &ValueMode::ImmutableOwned,
                    false,
                    string_table,
                )
                .map_err(TemplateError::from)?;

                push_template_head_expression(
                    expression,
                    TemplateHeadExpressionContext {
                        context,
                        type_environment: type_interner.environment(),
                        construction_context,
                    },
                    &value_location,
                    string_table,
                )?;
                defer_comma_advance = true;
                apply_head_compatibility(&mut head_state, &meaningful_item_compatibility);
            }

            // Path references
            TokenKind::Path(path_id) => {
                enforce_head_compatibility(
                    &head_state,
                    &meaningful_item_compatibility,
                    token_stream,
                )?;
                let path_root = token_stream
                    .path_syntax
                    .try_path_for_token(path_id, &token_stream.current_location())
                    .map_err(TemplateError::from)?
                    .root
                    .clone();
                push_template_head_path_expression(
                    path_id,
                    &path_root,
                    token_stream,
                    context,
                    type_interner,
                    construction_context,
                    string_table,
                )?;
                apply_head_compatibility(&mut head_state, &meaningful_item_compatibility);
            }

            // Parenthesized sub-expressions
            TokenKind::OpenParenthesis => {
                enforce_head_compatibility(
                    &head_state,
                    &meaningful_item_compatibility,
                    token_stream,
                )?;
                let value_location = token_stream.current_location();
                let mut inferred = ExpectedType::Infer;
                let expression = create_expression(
                    token_stream,
                    context,
                    type_interner,
                    &mut inferred,
                    &ValueMode::ImmutableOwned,
                    true,
                    string_table,
                )
                .map_err(TemplateError::from)?;

                push_template_head_expression(
                    expression,
                    TemplateHeadExpressionContext {
                        context,
                        type_environment: type_interner.environment(),
                        construction_context,
                    },
                    &value_location,
                    string_table,
                )?;
                defer_comma_advance = true;
                apply_head_compatibility(&mut head_state, &meaningful_item_compatibility);
            }

            // Style and setting directives
            TokenKind::StyleDirective(directive) => {
                // Template directives share the `$name` token shape with style directives.
                // Parse `$slot` / `$insert` first, then fall back to style handling.
                head_state.has_explicit_template_directive = true;
                let directive_name = string_table.resolve(directive).to_owned();
                let Some(spec) = context.style_directives.find(&directive_name) else {
                    return Err(CompilerDiagnostic::invalid_template_directive(
                        Some(directive),
                        InvalidTemplateDirectiveReason::UnknownDirective,
                        token_stream.current_location(),
                    )
                    .into());
                };

                enforce_head_compatibility(&head_state, &spec.head_compatibility, token_stream)?;

                let handled_slot_insert = maybe_parse_slot_or_insert_helper_directive(
                    &spec.kind,
                    token_stream,
                    build_state,
                    string_table,
                )?;

                if handled_slot_insert {
                    apply_head_compatibility(&mut head_state, &spec.head_compatibility);
                } else {
                    defer_comma_advance = parse_style_directive_from_spec(
                        token_stream,
                        context,
                        type_interner,
                        build_state,
                        &directive_name,
                        spec,
                        string_table,
                    )?;
                    apply_head_compatibility(&mut head_state, &spec.head_compatibility);
                }
            }

            // Separators
            TokenKind::Comma => {
                // Multiple commas in succession.
                return Err(CompilerDiagnostic::unexpected_token(
                    TokenKind::Comma,
                    token_stream.current_location(),
                )
                .into());
            }

            // Newlines / empty things in the template head are ignored.
            // Whitespace
            TokenKind::Newline => {
                token_stream.advance();
                continue;
            }

            _ => {
                return Err(CompilerDiagnostic::unexpected_token(
                    token,
                    token_stream.current_location(),
                )
                .into());
            }
        }

        // Guard against malformed or truncated synthetic token streams.
        if token_stream.index >= token_stream.length {
            return Err(CompilerDiagnostic::unexpected_end_of_file(
                Some(string_table.intern("]")),
                last_known_location,
            )
            .into());
        }

        if token_stream.current_token_kind() == &TokenKind::StartTemplateBody {
            token_stream.advance();
            return Ok(parsed_template_head(
                TemplateBodyParseMode::Normal,
                &head_state,
            ));
        }

        if token_stream.current_token_kind() == &TokenKind::Eof {
            return Err(CompilerDiagnostic::unexpected_end_of_file(
                Some(string_table.intern("]")),
                token_stream.current_location(),
            )
            .into());
        }

        if token_stream.current_token_kind() == &TokenKind::TemplateClose {
            return Ok(parsed_template_head(
                TemplateBodyParseMode::Normal,
                &head_state,
            ));
        }

        separator_state = TemplateHeadSeparatorState::ExpectSeparatorOrBody;
        if !defer_comma_advance {
            token_stream.advance();
        }
    }

    Err(CompilerDiagnostic::unexpected_end_of_file(
        Some(string_table.intern("]")),
        last_known_location,
    )
    .into())
}

/// Dispatches a `$directive` token using the already-resolved registry spec.
/// Returns `true` if the caller should defer separator-token advancement because
/// the directive parser consumed trailing tokens directly.
fn parse_style_directive_from_spec(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    build_state: &mut TemplateBuildState,
    directive_name: &str,
    spec: &StyleDirectiveSpec,
    string_table: &mut StringTable,
) -> TemplateHeadResult<bool> {
    let directive_result = match &spec.kind {
        StyleDirectiveKind::Core(kind) => parse_core_style_directive(
            token_stream,
            context,
            type_interner,
            build_state,
            directive_name,
            *kind,
            string_table,
        ),
        StyleDirectiveKind::Handler(handler_spec) => apply_handler_style_directive(
            token_stream,
            context,
            type_interner,
            build_state,
            directive_name,
            handler_spec,
            string_table,
        ),
    };

    if directive_result.is_ok() {
        // Any explicit style directive switches the template into style-controlled
        // whitespace mode. Individual formatters can opt into shared whitespace
        // passes explicitly via `Formatter` pre/post pass profiles.
        mark_template_body_whitespace_style_controlled(build_state);
    }

    directive_result?;

    Ok(false)
}

/// Scans ahead for unseparated `if` / `loop` suffix tokens.
///
/// Early returns make the first top-level boundary or suffix location explicit.
fn find_unseparated_control_flow_suffix(
    token_stream: &FileTokens,
) -> Option<crate::compiler_frontend::tokenizer::tokens::SourceLocation> {
    let mut nesting_depth = NestingDepth::default();
    let mut index = token_stream.index + 1;

    while index < token_stream.length {
        let token = &token_stream.tokens[index];

        if nesting_depth.is_top_level() {
            match token.kind {
                TokenKind::Comma | TokenKind::StartTemplateBody | TokenKind::TemplateClose => {
                    return None;
                }
                TokenKind::If | TokenKind::Loop => {
                    return Some(token.location.clone());
                }
                _ => {}
            }
        }

        nesting_depth.step(&token.kind);
        index += 1;
    }

    None
}
