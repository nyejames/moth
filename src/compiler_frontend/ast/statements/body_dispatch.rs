//! Function-body statement dispatch loop.
//!
//! WHAT: routes one function/start-function token stream through statement-position parsing.
//! WHY: centralized dispatch keeps control flow readable while specialized helpers own detailed
//!      syntax handling (symbol statements, returns, expression statements).

use crate::ast_log;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::expressions::failure_classification::pending_function_failure_facts;
use crate::compiler_frontend::ast::module_ast::scope_context::EnclosingCallableOwner;
use crate::compiler_frontend::ast::numeric_construction::{
    BodyDraft, NumericContinuation, StatementExpression,
};
use crate::compiler_frontend::ast::statements::asserts::parse_assert_statement;
use crate::compiler_frontend::ast::statements::body_expr_stmt::push_statement_expression;
use crate::compiler_frontend::ast::statements::body_return::parse_return_statement;
use crate::compiler_frontend::ast::statements::body_symbol::{
    parse_symbol_statement, parse_this_statement, reject_numeric_dependency_in_multi_bind,
};
use crate::compiler_frontend::ast::statements::branching::create_branch;
use crate::compiler_frontend::ast::statements::declared_regions::classify_deferred_declared_region_header;
use crate::compiler_frontend::ast::statements::diagnostics::{
    UnexpectedScopeCloseContext, unexpected_scope_close, unexpected_statement_token,
};
use crate::compiler_frontend::ast::statements::loops::create_loop;
use crate::compiler_frontend::ast::statements::match_arm_boundaries::{
    current_line_contains_top_level_fat_arrow, current_token_starts_match_arm_header,
};
use crate::compiler_frontend::ast::statements::multi_bind::{
    MultiBindStatement, parse_multi_bind_statement,
};
use crate::compiler_frontend::ast::statements::numeric_body_completion::complete_numeric_body;
use crate::compiler_frontend::ast::statements::value_production::parse_values::ProducedValuesParseInput;
use crate::compiler_frontend::ast::statements::value_production::{
    ProducedValues, ThenValueContinuation, ThenValueSlot, ValueReceiverKind,
    is_missing_produced_value_boundary, parse_retained_produced_values,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    choices_control_flow, names_declarations,
};
use crate::compiler_frontend::compiler_messages::{
    DeferredFeatureReason, DiagnosticBag, InvalidAssignmentTargetReason,
    InvalidControlFlowStatementReason, InvalidFallibleHandlingReason, InvalidMatchArmReason,
    InvalidStandaloneStatementReason, ReservedNameOwner,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::syntax_errors::statement_position::check_statement_common_mistake;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::utilities::token_scan::NestingDepth;
use crate::compiler_frontend::value_mode::ValueMode;

/// AST body parsing carries authored diagnostics and internal retained-data failures separately.
///
/// The module emitter is the reporting boundary: it renders diagnostics and forwards
/// `InfrastructureFailure` values unchanged. Nested statement parsers therefore must not collapse this
/// result back into a diagnostic when a loop header meets a stream without canonical provenance.
type StatementDispatchResult<T> = Result<T, ExpressionParseError>;

/// One parsed statement body before its numeric-dependent positions resolve.
///
/// WHAT: an independent body root completes its construction state and returns finished nodes;
///       a nested body returns its authored-order draft, whose pending positions resolve when the
///       enclosing root completes.
/// WHY: one function body (or root/concrete-generic body) owns the numeric construction domain,
///      while nested blocks share it. Returning the draft from nested parses is what lets a
///      construct retain its body in authored order instead of re-parsing tokens later.
pub(crate) enum ParsedBody {
    /// An independent body root: pending state is complete and released.
    Complete(Vec<AstNode>),
    /// A nested body sharing an enclosing root's domain.
    Draft(BodyDraft),
}

impl ParsedBody {
    /// Demand the finished node list of an independent body root.
    ///
    /// WHAT: returns the completed nodes for a root parse, and a precise invariant failure when a
    ///       body that was entered as a root returned a nested draft instead.
    /// WHY: only a caller that passed a root context may publish body nodes; a draft at that
    ///      boundary means the context never owned a construction domain, and silently unwrapping
    ///      it would let unresolved origins reach later stages.
    pub(crate) fn expect_complete_nodes(
        self,
        position: &str,
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        match self {
            ParsedBody::Complete(nodes) => Ok(nodes),
            ParsedBody::Draft(_) => Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "{position} returned a nested numeric draft instead of completed body nodes"
                ),
            )
            .into()),
        }
    }
}

fn statement_dispatch_error(diagnostic: DiagnosticDraft) -> ExpressionParseError {
    diagnostic.into()
}

fn reserved_keyword_as_name_error(keyword: StringId, span: Option<SourceSpan>) -> DiagnosticDraft {
    names_declarations::reserved_name_collision(keyword, ReservedNameOwner::Keyword, span)
}

/// Produce a diagnostic for a deferred block keyword (`checked`, `async`).
///
/// If the keyword is followed by an assignment operator, treats it as an attempt to use the
/// keyword as a variable name and reports a reserved-name error instead.
fn deferred_block_error(
    token_stream: &AstCursor,
    string_table: &mut StringTable,
    keyword: &str,
    reason: DeferredFeatureReason,
) -> DiagnosticDraft {
    let span = Some(token_stream.current_span());
    let next_is_assignment = token_stream
        .peek_next_tag()
        .is_some_and(|tag| tag.is_assignment_operator());
    if next_is_assignment {
        let keyword_id = string_table.intern(keyword);
        return reserved_keyword_as_name_error(keyword_id, span);
    }

    names_declarations::deferred_feature_reason(reason, span)
}

/// Recognize only a named place followed by an assignment operator. A method call
/// interrupts this shape and remains owned by ordinary mutable-receiver parsing.
fn starts_prefixed_write(cursor: &AstCursor) -> bool {
    let mut index = cursor.position() + 1;
    let tag = |index| cursor.token_ref_at(index).map(|token| token.tag());
    if !matches!(tag(index), Some(TokenTag::SYMBOL | TokenTag::THIS)) {
        return false;
    }
    index += 1;
    while tag(index) == Some(TokenTag::DOT) && tag(index + 1) == Some(TokenTag::SYMBOL) {
        index += 2;
    }
    tag(index).is_some_and(TokenTag::is_assignment_operator)
}

/// Detect an assignment after a non-place marker operand without parsing and rewinding
/// a completed expression. Assignments nested in call arguments are not statement writes.
fn prefixed_nonplace_write(cursor: &AstCursor) -> bool {
    let mut index = cursor.position() + 1;
    let mut depth = NestingDepth::default();
    while let Some(token) = cursor.token_ref_at(index) {
        let tag = token.tag();
        if tag == TokenTag::EOF {
            return false;
        }
        if depth.is_top_level() {
            if matches!(
                tag,
                TokenTag::NEWLINE
                    | TokenTag::COMMA
                    | TokenTag::END
                    | TokenTag::ELSE
                    | TokenTag::CATCH
                    | TokenTag::THEN
                    | TokenTag::COLON
                    | TokenTag::CLOSE_PARENTHESIS
                    | TokenTag::CLOSE_CURLY
                    | TokenTag::TEMPLATE_CLOSE
            ) {
                return false;
            }
            if tag.is_assignment_operator() {
                return true;
            }
        }
        depth.step_tag(tag);
        index += 1;
    }
    false
}

#[allow(
    clippy::too_many_arguments,
    reason = "function body keeps stream, scope, warnings and mutable parse resources apart"
)]
pub(crate) fn parse_function_body_statements(
    token_stream: &mut AstCursor,
    mut context: ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    warnings: &mut DiagnosticBag,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> StatementDispatchResult<ParsedBody> {
    // Authored statement order for this body, including every position whose statement could not
    // finish until the body root had selected each numeric origin's single static type.
    let mut draft = BodyDraft::new();

    while token_stream.position() < token_stream.length() {
        let current_tag = token_stream.current_tag();

        ast_log!("Parsing Token: ", #current_tag);

        // Match-arm bodies end when the next line-initial arm header or `else` is reached.
        // Same-line accidental second arms are rejected here before statement dispatch.
        if context.kind == ContextKind::MatchArm {
            if let Some(candidate) = current_token_starts_match_arm_header(token_stream) {
                debug_assert_eq!(candidate.start_index, token_stream.position());
                break;
            }

            if token_stream.current_tag() != TokenTag::ELSE
                && current_line_contains_top_level_fat_arrow(token_stream)
            {
                return Err(statement_dispatch_error(
                    choices_control_flow::invalid_match_arm(
                        InvalidMatchArmReason::ArmMustStartNewLine,
                        Some(token_stream.current_span()),
                    ),
                ));
            }
        }

        if let Some(diagnostic) = classify_deferred_declared_region_header(token_stream) {
            return Err(statement_dispatch_error(diagnostic));
        }

        // Give the single target-list parser the original marker before ordinary write dispatch.
        if current_tag == TokenTag::MUTABLE
            && token_stream.peek_next_tag() == Some(TokenTag::SYMBOL)
            && let Some(multi_bind) = parse_multi_bind_statement(
                token_stream,
                &mut context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            )
            .map_err(reject_numeric_dependency_in_multi_bind)?
        {
            match multi_bind {
                MultiBindStatement::Completed(node) => draft.push_node(node),
                MultiBindStatement::Retained(continuation) => draft.push_pending(continuation),
            }
            continue;
        }

        match current_tag {
            TokenTag::MODULE_START => {
                token_stream.advance();
            }

            // Symbol statements (declarations, assignments, calls)
            TokenTag::SYMBOL => parse_symbol_statement(
                token_stream,
                &mut draft,
                &mut context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            )?,

            TokenTag::THIS => parse_this_statement(
                token_stream,
                &mut draft,
                &mut context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            )?,

            TokenTag::MUTABLE if starts_prefixed_write(token_stream) => {
                token_stream.advance();
                if token_stream.current_tag() == TokenTag::THIS {
                    parse_this_statement(
                        token_stream,
                        &mut draft,
                        &mut context,
                        type_interner,
                        expression_builder,
                        const_values,
                        string_table,
                        path_fork,
                    )?;
                } else {
                    parse_symbol_statement(
                        token_stream,
                        &mut draft,
                        &mut context,
                        type_interner,
                        expression_builder,
                        const_values,
                        string_table,
                        path_fork,
                    )?;
                }
            }
            TokenTag::MUTABLE if prefixed_nonplace_write(token_stream) => {
                return Err(names_declarations::invalid_assignment_target(
                    InvalidAssignmentTargetReason::MutableMarkerRequiresExistingPlace,
                    None,
                    None,
                    None,
                    Some(token_stream.current_span()),
                )
                .into());
            }

            // Deferred keyword-led semantic scopes
            TokenTag::CHECKED => {
                return Err(statement_dispatch_error(deferred_block_error(
                    token_stream,
                    &mut *string_table,
                    "checked",
                    DeferredFeatureReason::CheckedBlock,
                )));
            }

            TokenTag::ASYNC => {
                return Err(statement_dispatch_error(deferred_block_error(
                    token_stream,
                    &mut *string_table,
                    "async",
                    DeferredFeatureReason::AsyncBlock,
                )));
            }

            // Control flow
            TokenTag::LOOP => {
                token_stream.advance();

                create_loop(
                    token_stream,
                    context.new_child_control_flow(ContextKind::Loop, string_table, path_fork)?,
                    type_interner,
                    expression_builder,
                    const_values,
                    warnings,
                    string_table,
                    path_fork,
                    &mut draft,
                )?;
            }

            TokenTag::IF => {
                token_stream.advance();

                create_branch(
                    token_stream,
                    context.new_child_control_flow(ContextKind::Branch, string_table, path_fork)?,
                    type_interner,
                    expression_builder,
                    const_values,
                    warnings,
                    string_table,
                    path_fork,
                    &mut draft,
                )?;
            }

            TokenTag::ELSE => {
                if context.kind == ContextKind::Branch || context.kind == ContextKind::MatchArm {
                    break;
                } else {
                    return Err(statement_dispatch_error(
                        choices_control_flow::invalid_control_flow_statement(
                            InvalidControlFlowStatementReason::ElseOutsideIfOrMatch,
                            Some(token_stream.current_span()),
                        ),
                    ));
                }
            }

            // Whitespace
            TokenTag::NEWLINE => {
                token_stream.advance();
            }

            // Return, loop control, and result handling
            TokenTag::ASSERT => {
                parse_assert_statement(
                    token_stream,
                    &mut draft,
                    &context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    path_fork,
                )?;
            }

            TokenTag::RETURN | TokenTag::RETURN_BANG => {
                parse_return_statement(
                    token_stream,
                    &mut draft,
                    &context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    path_fork,
                )?;
            }

            TokenTag::BREAK => {
                if !context.is_inside_loop() {
                    return Err(statement_dispatch_error(
                        choices_control_flow::invalid_control_flow_statement(
                            InvalidControlFlowStatementReason::BreakOutsideLoop,
                            Some(token_stream.current_span()),
                        ),
                    ));
                }

                draft.push_node(AstNode {
                    kind: NodeKind::Break,
                    span: Some(token_stream.current_span()),
                    scope: context.scope,
                });
                token_stream.advance();
            }

            TokenTag::CONTINUE => {
                if !context.is_inside_loop() {
                    return Err(statement_dispatch_error(
                        choices_control_flow::invalid_control_flow_statement(
                            InvalidControlFlowStatementReason::ContinueOutsideLoop,
                            Some(token_stream.current_span()),
                        ),
                    ));
                }

                draft.push_node(AstNode {
                    kind: NodeKind::Continue,
                    span: Some(token_stream.current_span()),
                    scope: context.scope,
                });
                token_stream.advance();
            }

            TokenTag::THEN => {
                let then_span = Some(token_stream.current_span());
                token_stream.advance();

                let Some(active_target) = &context.active_value_target else {
                    let reason = if matches!(
                        context.kind,
                        ContextKind::Loop | ContextKind::CatchHandler | ContextKind::Template
                    ) {
                        InvalidFallibleHandlingReason::ThenCrossesBlockedConstruct
                    } else {
                        InvalidFallibleHandlingReason::ThenWithNoActiveValueTarget
                    };

                    return Err(statement_dispatch_error(
                        choices_control_flow::invalid_fallible_handling(reason, then_span),
                    ));
                };

                if token_stream.current_tag() == TokenTag::NEWLINE
                    || is_missing_produced_value_boundary(token_stream.current_tag())
                {
                    return Err(statement_dispatch_error(
                        choices_control_flow::invalid_fallible_handling(
                            InvalidFallibleHandlingReason::ThenRequiresValues,
                            Some(token_stream.current_span()),
                        ),
                    ));
                }

                if active_target.result_type_ids.is_empty()
                    && active_target.receiver_kind != ValueReceiverKind::Declaration
                    && active_target.expected_arity.is_none()
                {
                    return Err(statement_dispatch_error(
                        choices_control_flow::invalid_fallible_handling(
                            InvalidFallibleHandlingReason::FallbackValuesForErrorOnlyResult,
                            then_span,
                        ),
                    ));
                }

                // Every `then` position owns its numeric deferral: a numeric-dependent fallback is
                // retained for the body root, while a fully built list keeps the ordinary eager
                // path with no continuation.
                let values = parse_retained_produced_values(ProducedValuesParseInput {
                    token_stream,
                    context: &context,
                    type_interner,
                    expression_builder,
                    const_values,
                    target: active_target,
                    string_table,
                    path_fork,
                })?;

                if values
                    .iter()
                    .all(|value| matches!(value, StatementExpression::Built(_)))
                {
                    let mut expressions = Vec::with_capacity(values.len());
                    for value in values {
                        let StatementExpression::Built(expression) = value else {
                            return Err(InfrastructureFailure::internal(
                                CompilerStage::AstConstruction,
                                "a produced-value list reported ready while holding a retained \
                                 fallback",
                            )
                            .into());
                        };
                        expressions.push(expression);
                    }

                    draft.push_node(AstNode {
                        kind: NodeKind::ThenValue(ProducedValues {
                            expressions,
                            span: then_span,
                        }),
                        span: then_span,
                        scope: context.scope,
                    });
                } else {
                    // The receiving site may not be resolved yet, so the authored contract travels
                    // with the values and the body root completes them at their real slots.
                    let continuation = context.register_numeric_continuation(
                        NumericContinuation::ThenValue(ThenValueContinuation {
                            values: values.into_iter().map(ThenValueSlot::Pending).collect(),
                            slot_expected_types: active_target.slot_expected_types(),
                            receiver_kind: active_target.receiver_kind,
                            span: then_span,
                            scope: context.scope,
                        }),
                    )?;
                    draft.push_pending(continuation);
                }
            }

            // Scope terminators
            TokenTag::END => match context.kind {
                ContextKind::Expression => {
                    return Err(statement_dispatch_error(unexpected_scope_close(
                        UnexpectedScopeCloseContext::Expression,
                        token_stream,
                    )));
                }

                ContextKind::Template => {
                    return Err(statement_dispatch_error(unexpected_scope_close(
                        UnexpectedScopeCloseContext::Template,
                        token_stream,
                    )));
                }

                ContextKind::MatchArm => break,

                _ => {
                    token_stream.advance();
                    break;
                }
            },

            // Templates
            // Top-level runtime template in the entry start() body.
            // Each template becomes a PushStartRuntimeFragment so the HIR builder can
            // push the evaluated string directly to the runtime fragment list.
            TokenTag::TEMPLATE_HEAD => {
                if context.kind != ContextKind::Module {
                    return Err(statement_dispatch_error(
                        choices_control_flow::invalid_standalone_statement(
                            InvalidStandaloneStatementReason::StandaloneTemplate,
                            Some(token_stream.current_span()),
                        ),
                    ));
                }

                let fragment_span = Some(token_stream.current_span());
                let template = Template::new_with_type_interner(
                    token_stream,
                    context.scope,
                    &context,
                    type_interner,
                    expression_builder,
                    const_values,
                    vec![],
                    string_table,
                    path_fork,
                )?;
                let expression = expression_builder.template(template, ValueMode::MutableOwned)?;

                draft.push_node(AstNode {
                    kind: NodeKind::PushStartRuntimeFragment(expression.id()),
                    span: fragment_span,
                    scope: context.scope,
                })
            }

            // End of file
            TokenTag::EOF => {
                break;
            }

            // Expression statements
            TokenTag::OPEN_PARENTHESIS
            | TokenTag::NUMERIC_LITERAL
            | TokenTag::STRING_SLICE_LITERAL
            | TokenTag::BOOL_LITERAL
            | TokenTag::CHAR_LITERAL
            | TokenTag::COPY
            | TokenTag::MUTABLE => {
                // One statement-expression owner: a produced value takes the authored position,
                // and a numeric-dependent statement records its retained continuation there.
                push_statement_expression(
                    token_stream,
                    &mut draft,
                    &context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    path_fork,
                )?;
            }

            // Unrecognized tokens
            _ => {
                if let Some(diagnostic) =
                    check_statement_common_mistake(token_stream.current_tag(), token_stream)
                {
                    return Err(statement_dispatch_error(diagnostic));
                }

                return Err(statement_dispatch_error(unexpected_statement_token(
                    token_stream,
                    string_table,
                )?));
            }
        }
    }

    // Numeric-dependent statements complete here for an independent body root: it is the only
    // owner of the body's construction state, and no later stage may observe a pending origin,
    // recipe or suspended statement. A nested body shares its enclosing root's domain, so it
    // returns its authored-order draft and the enclosing construct resumes it.
    warnings.extend(context.take_emitted_warnings());
    if !context.is_numeric_body_root() {
        return Ok(ParsedBody::Draft(draft));
    }

    let completed = complete_numeric_body(
        draft,
        &context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    );

    // Completion may emit trailing diagnostics from resumed positions (for example a retained
    // match arm whose duplicate value only existed after the subject type was selected). They
    // belong to the same root context and follow the parse-time warnings above, and a failing
    // completion must still deliver the warnings emitted before its error.
    warnings.extend(context.take_emitted_warnings());

    let body_nodes = completed?;

    // Only completed callable bodies prove infallibility. Branch/loop bodies and unresolved
    // recursive edges cannot close the callee's failure contract on their own.
    if context
        .enclosing_callable
        .as_deref()
        .is_some_and(|contract| {
            contract.owner == EnclosingCallableOwner::Function && contract.error_type.is_none()
        })
        && context.active_generic_type_context().is_none()
        && let Some(functions) = &context.known_infallible_functions
    {
        let facts = pending_function_failure_facts(
            &body_nodes,
            &context.template_ir_store.borrow(),
            &*expression_builder,
        )?;
        if !facts.body.summary.has_pending_implicit()
            && facts.body.summary.summary.first_typed.is_none()
            && facts.assertion_message_calls.is_empty()
        {
            functions.borrow_mut().insert(context.scope);
        }
    }

    Ok(ParsedBody::Complete(body_nodes))
}
