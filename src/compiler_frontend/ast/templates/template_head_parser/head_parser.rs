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
use super::directive_args::directive_has_arguments;
use super::handler_directives::apply_handler_style_directive;
use super::head_expressions::{
    TemplateHeadExpressionContext, handle_template_value_in_template_head,
    push_parsed_template_head_expression, push_template_head_path_expression,
};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{Declaration, DeclarationValue};
use crate::compiler_frontend::ast::const_eval::{
    ExplicitConstantPayload, explicit_constant_operand,
};
use crate::compiler_frontend::ast::const_values::store::{ConstValueId, ConstValueStore};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExprId, ExpressionOperationView, OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_with_trailing_newline_policy;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::module_ast::scope_context::ValueDeclarationRef;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template_build_state::TemplateBuildState;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBodyParseMode, TemplateControlFlowValidationMode,
};
use crate::compiler_frontend::ast::templates::tir::public_const_rebuild::rebuild_const_template_reference;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateConstructionContext, walk_expression_payloads_with_nested_tir_views,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;

use crate::ast_log;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    templates, token_lexical,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticToken, InvalidTemplateDirectiveReason, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::style_directives::{
    StyleDirectiveKind, StyleDirectiveSpec, TemplateHeadCompatibility, TemplateHeadTag,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::{CastTargetContext, ExpectedType};
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
    pub(crate) expression_builder: &'a mut AstExpressionBuilder,
    pub(crate) const_values: &'a mut ConstValueStore,
    pub(crate) build_state: &'a mut TemplateBuildState,
    pub(crate) construction_context: &'a mut TemplateConstructionContext,
    pub(crate) control_flow_validation: TemplateControlFlowValidationMode,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork:
        &'a mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
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
    token_stream: &AstCursor,
) -> TemplateHeadResult<()> {
    if !state.blocked_future_tags.intersects(incoming.presence_tags)
        && !state.seen_tags.intersects(incoming.required_absent_tags)
    {
        Ok(())
    } else {
        Err(with_current_token_span(
            token_stream,
            templates::invalid_template_structure(
                InvalidTemplateStructureReason::IncompatibleHeadItem,
                current_source_span(token_stream),
            ),
        )
        .into())
    }
}

/// Attach the current authored head-item span when a constructor did not
/// already provide one.
fn with_current_token_span(
    token_stream: &AstCursor,
    mut diagnostic: DiagnosticDraft,
) -> DiagnosticDraft {
    if diagnostic.primary_span().is_none() {
        diagnostic.set_primary_span(current_source_span(token_stream));
    }
    diagnostic
}

/// Read-only current-token span on the canonical cursor view.
///
/// WHAT: reports the exact authored span of the head-item token without
/// advancing the stream.
/// WHY: head dispatch needs only token-local span facts on the canonical cursor.
fn current_source_span(token_stream: &AstCursor) -> Option<SourceSpan> {
    Some(token_stream.current_span())
}

fn with_source_span(
    mut diagnostic: DiagnosticDraft,
    source_span: Option<SourceSpan>,
) -> DiagnosticDraft {
    if diagnostic.primary_span().is_none() {
        diagnostic.set_primary_span(source_span);
    }
    diagnostic
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

/// Source of a known template reference that a head may inline.
enum InlinableHeadTemplate {
    /// Authored template initializer owned by this compilation unit.
    Authored(Template),
    /// Imported template constant, rebuilt into this unit's TIR store before inlining.
    ImportedConstant(ConstValueId),
}

fn should_inline_template_head_reference(
    token_stream: &AstCursor,
    context: &ScopeContext,
    declaration: &Declaration,
    expression_builder: &AstExpressionBuilder,
) -> bool {
    if peek_next_tag(token_stream) != Some(TokenTag::TEMPLATE_CLOSE) {
        return true;
    }

    if context.kind.is_constant_context() {
        return true;
    }

    // Head-only runtime template references are value reads, not receiver
    // applications. Runtime slot handoffs already carry the composition-owned
    // wrapper/source plan, so copying an already-materialized template would
    // lose that plan when the surrounding template later crosses into HIR.
    !declaration.value.initializer().is_some_and(|initializer| {
        expression_contains_runtime_slot_handoff(initializer, context, expression_builder)
    })
}

/// Read-only next-token lookahead on the canonical cursor view.
///
/// WHAT: probes the token after the current head item without advancing the stream.
/// WHY: the inline-reference check is pure lookahead on the cached cursor view.
fn peek_next_tag(token_stream: &AstCursor) -> Option<TokenTag> {
    token_stream.peek_next_tag()
}

fn expression_contains_runtime_slot_handoff(
    expression: ExprId,
    context: &ScopeContext,
    expression_builder: &AstExpressionBuilder,
) -> bool {
    let store = context.template_ir_store.borrow();

    let mut contains_runtime_slot_handoff = false;
    let expression = expression_builder
        .view(expression)
        .expect("declaration initializer belongs to the phase expression owner");
    let walk_result = walk_expression_payloads_with_nested_tir_views(
        expression,
        &store,
        expression_builder,
        &mut |payload| {
            if matches!(
                payload.operation(),
                ExpressionOperationView::RuntimeSlotApplication(_)
            ) {
                contains_runtime_slot_handoff = true;
            }
            Ok(())
        },
    );

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
    token_stream: &mut AstCursor<'_>,
    request: TemplateHeadParseRequest<'_, '_>,
) -> TemplateHeadResult<ParsedTemplateHead> {
    let TemplateHeadParseRequest {
        context,
        type_interner,
        expression_builder,
        const_values,
        build_state,
        construction_context,
        control_flow_validation,
        string_table,
        path_fork,
    } = request;

    // Each meaningful head item must be separated with a comma before another
    // item can start. Naming the state keeps suffix and body-boundary handling
    // readable in this parser state machine.
    let mut separator_state = TemplateHeadSeparatorState::ExpectItem;
    let mut head_state = TemplateHeadState::default();
    let meaningful_item_compatibility = TemplateHeadCompatibility::fully_compatible_meaningful();
    token_stream.advance();

    let mut last_known_span = current_source_span(token_stream);
    while token_stream.position() < token_stream.length() {
        last_known_span = current_source_span(token_stream);
        let token = token_stream.current_tag();

        ast_log!("Parsing template head: ", #token);

        // We are doing something similar to new_ast()
        // But with the specific template head syntax,
        // expressions are allowed and should be folded where possible.
        // Loops and if statements can end the template head.

        // EOF inside a template head means the source was truncated before a
        // closing ] delimiter. This is a malformed template, not a valid stream
        // boundary; the user needs a structured diagnostic.
        if token == TokenTag::EOF {
            return Err(with_current_token_span(
                token_stream,
                token_lexical::unexpected_end_of_file(
                    Some(string_table.intern("]")),
                    current_source_span(token_stream),
                ),
            )
            .into());
        }

        // A closing ] without a body is a valid empty template.
        if token == TokenTag::TEMPLATE_CLOSE {
            return Ok(parsed_template_head(
                TemplateBodyParseMode::Normal,
                &head_state,
            ));
        }

        if token == TokenTag::START_TEMPLATE_BODY {
            if head_state
                .seen_tags
                .intersects(TemplateHeadTag::SLOT_DIRECTIVE)
            {
                return Err(with_current_token_span(
                    token_stream,
                    templates::invalid_template_structure(
                        InvalidTemplateStructureReason::SlotInHead,
                        current_source_span(token_stream),
                    ),
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
            && !matches!(token, TokenTag::IF | TokenTag::LOOP)
            && let Some(control_flow_span) = find_unseparated_control_flow_suffix(token_stream)
        {
            return Err(with_source_span(
                templates::invalid_template_structure(
                    InvalidTemplateStructureReason::MissingCommaBeforeControlFlowSuffix,
                    Some(control_flow_span),
                ),
                Some(control_flow_span),
            )
            .into());
        }

        // Make sure there is a comma before the next token.
        if separator_state == TemplateHeadSeparatorState::ExpectSeparatorOrBody {
            if matches!(token, TokenTag::IF | TokenTag::LOOP) {
                return Err(with_current_token_span(
                    token_stream,
                    templates::invalid_template_structure(
                        InvalidTemplateStructureReason::MissingCommaBeforeControlFlowSuffix,
                        current_source_span(token_stream),
                    ),
                )
                .into());
            }

            if token != TokenTag::COMMA {
                let found = token_stream
                    .current_diagnostic_token(string_table)
                    .map_err(|error| {
                        CompilerError::compiler_error(format!(
                            "template-head separator diagnostic token payload was malformed: {error:?}"
                        ))
                    })?
                    .or_else(|| Some(DiagnosticToken::from_static_tag(token)));
                return Err(with_current_token_span(
                    token_stream,
                    token_lexical::expected_token_from_projections(
                        DiagnosticToken::from_static_tag(TokenTag::COMMA),
                        found,
                        current_source_span(token_stream),
                    ),
                )
                .into());
            }

            separator_state = TemplateHeadSeparatorState::ExpectItem;
            token_stream.advance();
            continue;
        }

        let mut defer_comma_advance = false;

        match token {
            TokenTag::IF => {
                if head_state
                    .seen_tags
                    .intersects(TemplateHeadTag::SLOT_DIRECTIVE)
                {
                    return Err(with_current_token_span(
                        token_stream,
                        templates::invalid_template_structure(
                            InvalidTemplateStructureReason::SlotInHead,
                            current_source_span(token_stream),
                        ),
                    )
                    .into());
                }

                let body_mode = parse_if_suffix(
                    token_stream,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    control_flow_validation,
                    string_table,
                    path_fork,
                )?;
                return Ok(parsed_template_head(body_mode, &head_state));
            }

            TokenTag::LOOP => {
                if head_state
                    .seen_tags
                    .intersects(TemplateHeadTag::SLOT_DIRECTIVE)
                {
                    return Err(with_current_token_span(
                        token_stream,
                        templates::invalid_template_structure(
                            InvalidTemplateStructureReason::SlotInHead,
                            current_source_span(token_stream),
                        ),
                    )
                    .into());
                }

                let body_mode = parse_loop_suffix(
                    token_stream,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    control_flow_validation,
                    string_table,
                    path_fork,
                )?;
                return Ok(parsed_template_head(body_mode, &head_state));
            }

            // Variable, template and dependency-namespace references.
            //
            // Known template references that should be inlined preserve their
            // wrapper/slot semantics. Everything else routes through the ordinary
            // expression parser so that namespace member access (`intro.content`),
            // bare dependency-namespace misuse (`intro`), field access and unknown names
            // all get structured diagnostics instead of generic `UnexpectedToken`.
            TokenTag::SYMBOL => {
                let name = token_stream
                    .current_string_id_in(string_table)?
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "template-head symbol token had no string payload",
                        )
                    })?;
                enforce_head_compatibility(
                    &head_state,
                    &meaningful_item_compatibility,
                    token_stream,
                )?;
                let value_span = current_source_span(token_stream);
                let value_reference = context.get_value_reference(&name, value_span)?;

                // Extract an inlinable template before the mutable expression parse.
                // The borrow from get_reference is released once the template is cloned.
                // Known template references inline their template value. An imported template
                // constant is rebuilt into this store first, so wrapper and slot composition
                // applies the same way it does to an authored template.
                let inlinable_reference = value_reference.as_ref().and_then(|reference| {
                    // A pending numeric binding has no declaration and is never an inlinable
                    // template value, so it keeps the ordinary expression path below and
                    // stays pending for body completion.
                    let declaration = reference.ready_declaration()?;
                    let source = match &declaration.value {
                        // Imported constants are declared by their folded ConstValueId and have no
                        // initializer expression. The rebuild decides whether the payload is a template.
                        DeclarationValue::Default(value) => {
                            InlinableHeadTemplate::ImportedConstant(*value)
                        }
                        _ => {
                            let initializer = declaration.value.initializer()?;
                            match expression_builder
                                .view(initializer)
                                .expect(
                                    "declaration initializer belongs to the phase expression owner",
                                )
                                .operation()
                            {
                                ExpressionOperationView::Template(template) => {
                                    InlinableHeadTemplate::Authored(template.clone())
                                }
                                ExpressionOperationView::Constant(value) => {
                                    InlinableHeadTemplate::ImportedConstant(value)
                                }
                                _ => return None,
                            }
                        }
                    };
                    should_inline_template_head_reference(
                        token_stream,
                        context,
                        declaration,
                        expression_builder,
                    )
                    .then_some(source)
                });

                let inlined_template = match inlinable_reference {
                    Some(InlinableHeadTemplate::Authored(template)) => Some(template),
                    Some(InlinableHeadTemplate::ImportedConstant(value)) => {
                        rebuild_const_template_reference(
                            value,
                            &mut context.template_ir_store.borrow_mut(),
                            expression_builder,
                            const_values,
                            context
                                .file_value_resolution
                                .as_ref()
                                .map(|services| &*services.module_resources),
                            string_table,
                            value_span,
                        )?
                    }
                    None => None,
                };

                if let Some(inserted_template) = inlined_template {
                    handle_template_value_in_template_head(
                        &inserted_template,
                        context,
                        construction_context,
                        value_span,
                    )?;
                } else {
                    // Resolve value_mode from the reference before the mutable
                    // expression parse so the context borrow does not overlap.
                    let value_mode = match value_reference.as_ref() {
                        Some(reference) => head_reference_value_mode(context, reference)?,
                        // Unknown names keep the ordinary parse path, which reports the
                        // structured unknown-name diagnostic for the authored token.
                        None => ValueMode::ImmutableOwned,
                    };

                    let expression = parse_head_expression(
                        token_stream,
                        context,
                        type_interner,
                        expression_builder,
                        const_values,
                        &value_mode,
                        false,
                        string_table,
                        path_fork,
                    );

                    push_parsed_template_head_expression(
                        expression,
                        TemplateHeadExpressionContext {
                            context,
                            type_environment: type_interner.environment(),
                            expression_builder,
                            const_values,
                            construction_context,
                            path_fork: &*path_fork,
                            value_mode,
                        },
                        value_span,
                        string_table,
                    )?;
                    defer_comma_advance = true;
                }

                apply_head_compatibility(&mut head_state, &meaningful_item_compatibility);
            }

            // Receiver self-reference
            TokenTag::THIS => {
                let this_id = string_table.intern("this");
                // `this` names the reserved receiver binding, which is never a pending numeric
                // body local, so a pending reference here is a broken invariant rather than an
                // absent name.
                let receiver_value_mode = context
                    .get_value_reference(&this_id, current_source_span(token_stream))?
                    .map(|reference| {
                        let declaration = reference.ready_declaration().ok_or_else(|| {
                            CompilerError::compiler_error(
                                "template-head `this` resolved to a pending numeric binding",
                            )
                        })?;
                        Ok::<ValueMode, CompilerError>(declaration.value_mode.to_owned())
                    })
                    .transpose()?;
                if let Some(value_mode) = receiver_value_mode {
                    enforce_head_compatibility(
                        &head_state,
                        &meaningful_item_compatibility,
                        token_stream,
                    )?;
                    let value_span = current_source_span(token_stream);
                    let expression = parse_head_expression(
                        token_stream,
                        context,
                        type_interner,
                        expression_builder,
                        const_values,
                        &value_mode,
                        false,
                        string_table,
                        path_fork,
                    );
                    push_parsed_template_head_expression(
                        expression,
                        TemplateHeadExpressionContext {
                            context,
                            type_environment: type_interner.environment(),
                            expression_builder,
                            const_values,
                            construction_context,
                            path_fork: &*path_fork,
                            value_mode,
                        },
                        value_span,
                        string_table,
                    )?;
                    defer_comma_advance = true;
                    apply_head_compatibility(&mut head_state, &meaningful_item_compatibility);
                } else {
                    let span = current_source_span(token_stream);
                    let diagnostic = token_stream
                        .current_diagnostic_token(string_table)
                        .map_err(|error| {
                            CompilerError::compiler_error(format!(
                                "template-head `this` diagnostic token payload was malformed: {error:?}"
                            ))
                        })?
                        .map(|found| token_lexical::unexpected_token_from_tag(found, span))
                        .unwrap_or_else(|| {
                            token_lexical::unexpected_token_from_tag(DiagnosticToken::from_static_tag(TokenTag::THIS),
                            span,)
                        });
                    return Err(with_current_token_span(token_stream, diagnostic).into());
                }
            }

            // Constants can be inserted directly into parser TIR.
            // Literal values
            TokenTag::NUMERIC_LITERAL
            | TokenTag::BOOL_LITERAL
            | TokenTag::STRING_SLICE_LITERAL
            | TokenTag::RAW_STRING_LITERAL => {
                enforce_head_compatibility(
                    &head_state,
                    &meaningful_item_compatibility,
                    token_stream,
                )?;
                let value_span = current_source_span(token_stream);
                let expression = parse_head_expression(
                    token_stream,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    &ValueMode::ImmutableOwned,
                    false,
                    string_table,
                    path_fork,
                );

                push_parsed_template_head_expression(
                    expression,
                    TemplateHeadExpressionContext {
                        context,
                        type_environment: type_interner.environment(),
                        expression_builder,
                        const_values,
                        construction_context,
                        path_fork: &*path_fork,
                        value_mode: ValueMode::ImmutableOwned,
                    },
                    value_span,
                    string_table,
                )?;
                defer_comma_advance = true;
                apply_head_compatibility(&mut head_state, &meaningful_item_compatibility);
            }

            // Path references
            TokenTag::PATH => {
                let path_id = token_stream.current_path_syntax_id().ok_or_else(|| {
                    CompilerError::compiler_error("template-head path token had no payload")
                })?;
                if path_id.is_none() {
                    return Err(CompilerError::compiler_error(
                        "template-head path token had an absent PathSyntaxId marker",
                    )
                    .into());
                }
                token_stream.current_path_syntax()?.ok_or_else(|| {
                    CompilerError::compiler_error(
                        "template-head path token had no validated syntax row",
                    )
                })?;
                enforce_head_compatibility(
                    &head_state,
                    &meaningful_item_compatibility,
                    token_stream,
                )?;
                push_template_head_path_expression(
                    path_id,
                    token_stream,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    construction_context,
                    string_table,
                    path_fork,
                )?;
                apply_head_compatibility(&mut head_state, &meaningful_item_compatibility);
            }

            // Parenthesized sub-expressions
            TokenTag::OPEN_PARENTHESIS => {
                enforce_head_compatibility(
                    &head_state,
                    &meaningful_item_compatibility,
                    token_stream,
                )?;
                let value_span = current_source_span(token_stream);
                let expression = parse_head_expression(
                    token_stream,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    &ValueMode::ImmutableOwned,
                    true,
                    string_table,
                    path_fork,
                );

                push_parsed_template_head_expression(
                    expression,
                    TemplateHeadExpressionContext {
                        context,
                        type_environment: type_interner.environment(),
                        expression_builder,
                        const_values,
                        construction_context,
                        path_fork: &*path_fork,
                        value_mode: ValueMode::ImmutableOwned,
                    },
                    value_span,
                    string_table,
                )?;
                defer_comma_advance = true;
                apply_head_compatibility(&mut head_state, &meaningful_item_compatibility);
            }

            // Style and setting directives
            TokenTag::STYLE_DIRECTIVE => {
                let directive = token_stream
                    .current_string_id_in(string_table)?
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "template-head directive token had no string payload",
                        )
                    })?;
                // Template directives share the `$name` token shape with style directives.
                // Parse `$slot` / `$insert` first, then fall back to style handling.
                head_state.has_explicit_template_directive = true;
                let directive_name = string_table.resolve(directive).to_owned();
                let Some(spec) = context.style_directives.find(&directive_name) else {
                    return Err(with_current_token_span(
                        token_stream,
                        templates::invalid_template_directive(
                            Some(directive),
                            InvalidTemplateDirectiveReason::UnknownDirective,
                            current_source_span(token_stream),
                        ),
                    )
                    .into());
                };
                enforce_head_compatibility(&head_state, &spec.head_compatibility, token_stream)?;

                let slot_insert_had_arguments = directive_has_arguments(token_stream);
                let handled_slot_insert = maybe_parse_slot_or_insert_helper_directive(
                    &spec.kind,
                    token_stream,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    build_state,
                    string_table,
                    path_fork,
                )?;

                if handled_slot_insert {
                    defer_comma_advance = slot_insert_had_arguments;
                    apply_head_compatibility(&mut head_state, &spec.head_compatibility);
                } else {
                    defer_comma_advance = parse_style_directive_from_spec(
                        token_stream,
                        context,
                        type_interner,
                        expression_builder,
                        const_values,
                        build_state,
                        &directive_name,
                        spec,
                        string_table,
                        path_fork,
                    )?;
                    apply_head_compatibility(&mut head_state, &spec.head_compatibility);
                }
            }

            // Separators
            TokenTag::COMMA => {
                // Multiple commas in succession.
                let span = current_source_span(token_stream);
                let diagnostic = token_stream
                    .current_diagnostic_token(string_table)
                    .map_err(|error| {
                        CompilerError::compiler_error(format!(
                            "template-head comma diagnostic token payload was malformed: {error:?}"
                        ))
                    })?
                    .map(|found| token_lexical::unexpected_token_from_tag(found, span))
                    .unwrap_or_else(|| {
                        token_lexical::unexpected_token_from_tag(
                            DiagnosticToken::from_static_tag(TokenTag::COMMA),
                            span,
                        )
                    });
                return Err(with_current_token_span(token_stream, diagnostic).into());
            }

            // Newlines / empty things in the template head are ignored.
            // Whitespace
            TokenTag::NEWLINE => {
                token_stream.advance();
                continue;
            }

            _ => {
                let span = current_source_span(token_stream);
                let diagnostic = token_stream
                    .current_diagnostic_token(string_table)
                    .map_err(|error| {
                        CompilerError::compiler_error(format!(
                            "template-head unexpected-token diagnostic token payload was malformed: {error:?}"
                        ))
                    })?
                    .map(|found| token_lexical::unexpected_token_from_tag(found, span))
                    .unwrap_or_else(|| {
                        token_lexical::unexpected_token_from_tag(DiagnosticToken::from_static_tag(token),
                        span,)
                    });
                return Err(with_current_token_span(token_stream, diagnostic).into());
            }
        }

        // Guard against malformed or truncated synthetic token streams.
        if token_stream.position() >= token_stream.length() {
            return Err(with_source_span(
                token_lexical::unexpected_end_of_file(
                    Some(string_table.intern("]")),
                    last_known_span,
                ),
                last_known_span,
            )
            .into());
        }

        if token_stream.current_tag() == TokenTag::START_TEMPLATE_BODY {
            token_stream.advance();
            return Ok(parsed_template_head(
                TemplateBodyParseMode::Normal,
                &head_state,
            ));
        }

        if token_stream.current_tag() == TokenTag::EOF {
            return Err(with_current_token_span(
                token_stream,
                token_lexical::unexpected_end_of_file(
                    Some(string_table.intern("]")),
                    current_source_span(token_stream),
                ),
            )
            .into());
        }

        if token_stream.current_tag() == TokenTag::TEMPLATE_CLOSE {
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

    Err(with_source_span(
        token_lexical::unexpected_end_of_file(Some(string_table.intern("]")), last_known_span),
        last_known_span,
    )
    .into())
}

/// Dispatches a `$directive` token using the already-resolved registry spec.
/// Returns `true` if the caller should defer separator-token advancement because
/// the directive parser consumed trailing tokens directly.
#[allow(
    clippy::too_many_arguments,
    reason = "directive dispatch keeps the token stream, scope, mutable interner/build/string/path state, and the directive name and registry spec as separate borrows"
)]
fn parse_style_directive_from_spec(
    token_stream: &mut AstCursor<'_>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    build_state: &mut TemplateBuildState,
    directive_name: &str,
    spec: &StyleDirectiveSpec,
    string_table: &mut StringTable,
    path_fork: &mut crate::compiler_frontend::symbols::path_interner::PathInternerFork,
) -> TemplateHeadResult<bool> {
    let had_arguments = directive_has_arguments(token_stream);
    let directive_result = match &spec.kind {
        StyleDirectiveKind::Core(kind) => parse_core_style_directive(
            token_stream,
            context,
            type_interner,
            expression_builder,
            const_values,
            build_state,
            directive_name,
            *kind,
            string_table,
            path_fork,
        ),
        StyleDirectiveKind::Handler(handler_spec) => apply_handler_style_directive(
            token_stream,
            context,
            type_interner,
            expression_builder,
            const_values,
            build_state,
            directive_name,
            handler_spec,
            string_table,
            path_fork,
        ),
    };

    if directive_result.is_ok() {
        // Any explicit style directive switches the template into style-controlled
        // whitespace mode. Individual formatters can opt into shared whitespace
        // passes explicitly via `Formatter` pre/post pass profiles.
        mark_template_body_whitespace_style_controlled(build_state);
    }

    directive_result?;

    Ok(had_arguments)
}

/// Parses one template-head expression as the owner of its complete fragment.
///
/// WHAT: builds the ordinary expression input marked as the numeric-deferral owner, so an
///       unresolvable numeric fragment is returned as a retainable deferral.
/// WHY: the head position owns the whole authored expression syntax, and only this boundary can
///      retain that fragment for body-root completion. Nested parses keep raising non-owning
///      deferrals, which the enclosing construct must reject rather than mis-retain.
#[allow(
    clippy::too_many_arguments,
    reason = "value mode and closing-parenthesis policy are extra inputs beside the parser borrows"
)]
fn parse_head_expression(
    token_stream: &mut AstCursor<'_>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    value_mode: &ValueMode,
    consume_closing_parenthesis: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<OwnedExpr, ExpressionParseError> {
    let mut inferred = ExpectedType::Infer;
    let mut cast_target_context = CastTargetContext::None;
    let input = ExpressionParseInput::ordinary(
        ExpressionParseResources {
            token_stream,
            scope_context: context,
            type_interner,
            expected_type: &mut inferred,
            cast_target_context: &mut cast_target_context,
            value_mode,
            string_table,
            expression_builder,
            const_values,
            path_fork,
        },
        consume_closing_parenthesis,
    )
    .with_numeric_deferral_owner();

    let expression = create_expression_with_trailing_newline_policy(input)?;
    // WHY: an explicit String `#` constant interpolated into a template reads as its value, like an
    // operator operand does, so `[greeting]` folds to text instead of staying a runtime read.
    let replacement = explicit_constant_operand(
        expression,
        ExplicitConstantPayload::Text,
        context,
        path_fork,
        expression_builder,
        const_values,
    )?;
    Ok(replacement.unwrap_or(expression))
}

/// Resolves the value mode for one template-head symbol reference.
///
/// WHAT: reads the authored mode from a ready declaration, or from the body-owned pending
///       numeric row that owns an unresolved binding.
/// WHY: a pending numeric binding has no declaration and must not be treated as an absent
///      name or defaulted before body completion, so the construction row owns the mode.
fn head_reference_value_mode(
    context: &ScopeContext,
    reference: &ValueDeclarationRef<'_>,
) -> Result<ValueMode, CompilerError> {
    match reference {
        ValueDeclarationRef::Local { declaration, .. } => Ok(declaration.value_mode.to_owned()),
        ValueDeclarationRef::Shared(declaration) => Ok(declaration.value_mode.to_owned()),
        ValueDeclarationRef::PendingNumeric { binding, .. } => {
            context.pending_numeric_value_mode(*binding).ok_or_else(|| {
                CompilerError::compiler_error(
                    "pending numeric binding reference had no construction row value mode",
                )
            })
        }
    }
}

/// Find a control-flow suffix that follows a head item without a separating comma.
///
/// WHAT: reports the span of the first top-level `if`/`loop` reached before a comma or a body
/// boundary, starting from the token after the current one.
/// WHY: the head parser turns that span into `MissingCommaBeforeControlFlowSuffix`.
fn find_unseparated_control_flow_suffix(token_stream: &mut AstCursor) -> Option<SourceSpan> {
    let resume = token_stream.position();
    let end = token_stream.length();
    let mut nesting_depth = NestingDepth::default();
    let mut suffix_span = None;
    token_stream.advance();
    while token_stream.position() < end && !token_stream.is_at_end() {
        let tag = token_stream.current_tag();
        // `Eof` never advances on either cursor lane, so the search ends here.
        if tag == TokenTag::EOF {
            break;
        }
        if nesting_depth.is_top_level() {
            match tag {
                TokenTag::COMMA | TokenTag::START_TEMPLATE_BODY | TokenTag::TEMPLATE_CLOSE => break,
                TokenTag::IF | TokenTag::LOOP => {
                    suffix_span = Some(token_stream.current_span());
                    break;
                }
                _ => {}
            }
        }
        nesting_depth.step_tag(tag);
        token_stream.advance();
    }
    token_stream
        .set_position(resume)
        .expect("suffix span scan resume stays inside the active parser view");
    suffix_span
}
