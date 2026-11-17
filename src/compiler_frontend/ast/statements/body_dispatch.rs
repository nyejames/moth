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
    UnexpectedScopeCloseContext, last_child_closer_span, scope_closer_layout_error,
    unexpected_scope_close, unexpected_statement_token,
};
use crate::compiler_frontend::ast::statements::loops::create_loop;
use crate::compiler_frontend::ast::statements::match_arm_boundaries::{
    current_token_starts_colon_separated_arm_header, current_token_starts_match_arm_header,
    same_line_match_arm_anchor,
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
use crate::compiler_frontend::ast::templates::template::{Template, TemplateType};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{AstConstraintBoundary, AstConstraintStatementSpan};
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    choices_control_flow, names_declarations, templates, token_lexical,
};
use crate::compiler_frontend::compiler_messages::{
    DeferredFeatureReason, DiagnosticBag, InvalidAssignmentTargetReason,
    InvalidControlFlowStatementReason, InvalidFallibleHandlingReason, InvalidMatchArmReason,
    InvalidStandaloneStatementReason, InvalidTemplateDirectiveReason, ReservedNameOwner,
    SourceScopeConstruct,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::style_directives::{
    ConstraintDirectiveSpans, SourceConstraintSet, SourceOperandFoldResources,
    parse_statement_source_directive_run,
};
use crate::compiler_frontend::symbols::identifier_policy;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::syntax_errors::statement_position::check_statement_common_mistake;
use crate::compiler_frontend::tokenizer::line_scanning::ScopeOpening;
use crate::compiler_frontend::tokenizer::tokens::{TokenIndex, TokenTag};
use crate::compiler_frontend::utilities::token_scan::NestingDepth;
use crate::compiler_frontend::value_mode::ValueMode;

/// AST body parsing carries authored diagnostics and internal retained-data failures separately.
///
/// The module emitter is the reporting boundary: it renders diagnostics and forwards
/// `InfrastructureFailure` values unchanged. Nested statement parsers therefore must not collapse this
/// result back into a diagnostic when a nested header reads a stream without canonical provenance.
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

/// Parses one template that statement dispatch owns and rejects a root `$note`.
///
/// WHAT: a `$note` belongs inside a template body, so a root note reports its placement error at
///       the directive. Every other parsed template returns to the caller.
/// WHY: the parsed root kind is authoritative. A literal or identifier merely spelled `note` is
///      ordinary content, and only the template parser knows which directive the head holds.
fn parse_statement_template(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> StatementDispatchResult<Template> {
    let template_span = Some(token_stream.current_span());
    let template = Template::new_with_type_interner(
        token_stream,
        context.scope,
        context,
        type_interner,
        expression_builder,
        const_values,
        vec![],
        string_table,
        path_fork,
    )?;
    let is_comment = {
        let store = context.template_ir_store.borrow();
        store
            .get_template(template.tir_reference.root)
            .map(|template_ir| matches!(template_ir.kind, TemplateType::Comment))
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "parsed statement template kind was missing from its module-local TIR store.",
                )
            })?
    };
    if !is_comment {
        return Ok(template);
    }
    Err(statement_dispatch_error(
        templates::note_outside_template_body(
            string_table.intern("note"),
            template.comment_directive_span.or(template_span),
        ),
    ))
}

/// The signature colon that encloses a captured function body's statements.
///
/// WHAT: reads the canonical token immediately before the body's first token.
/// WHY: header capture returns the body as a half-open range that begins after the signature `:`,
///      so that colon is the still-open opener for the body's first nested opener. Entry start is
///      segmented and owns no enclosing scope, so only `Function` bodies resolve one. An empty body
///      holds no opener to compare. A body that begins its own token store is re-parsed from
///      generated or synthetic body text, so it has no authored enclosing opener either.
fn function_signature_opening(
    token_stream: &AstCursor,
) -> Result<Option<ScopeOpening>, ExpressionParseError> {
    let Some(opener_index) = token_stream
        .current()
        .and_then(|first| first.index().index().checked_sub(1))
        .and_then(TokenIndex::try_from_index)
    else {
        return Ok(None);
    };
    let opener = token_stream
        .canonical_token_ref_at(opener_index)
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "a captured function body could not read its signature colon",
            )
        })?;
    if opener.tag() != TokenTag::COLON {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "a captured function body did not begin after its signature colon",
        )
        .into());
    }
    Ok(Some(ScopeOpening {
        site: opener.index(),
        opener: opener.index(),
        label_span: opener.source_span(),
        construct: SourceScopeConstruct::FunctionBody,
    }))
}

/// Builds the typed wrong-target diagnostic for a source directive in a nested position.
///
/// WHAT: a known `$config` token is never a statement or block-value entry, so every statement
///       and block-value start routes here instead of the generic unexpected-token path.
/// WHY: one helper keeps the directive named and the span authored across body dispatch and
///      curly-brace value parsing.
pub(crate) fn source_directive_wrong_target_error(
    token_stream: &AstCursor,
    string_table: &mut StringTable,
) -> Result<ExpressionParseError, ExpressionParseError> {
    let directive = token_stream
        .current_string_id_in(string_table)?
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "statement directive token had no string payload",
            )
        })?;
    Ok(templates::invalid_template_directive(
        Some(directive),
        InvalidTemplateDirectiveReason::WrongTarget,
        Some(token_stream.current_span()),
    )
    .into())
}

fn reserved_keyword_as_name_error(keyword: StringId, span: Option<SourceSpan>) -> DiagnosticDraft {
    names_declarations::reserved_name_collision(keyword, ReservedNameOwner::Keyword, span)
}

/// Returns the reserved-name diagnostic when an unmatched keyword writes a binding.
///
/// WHAT: accepts a keyword or word-operator token followed by an assignment operator, a
///       type-shaped token, a comma (first multi-bind target), or a symbol, with an assignment
///       later on the same logical line.
/// WHY: `by = 1`, `by I32 = 1` and `by, bx = pair()` are name attempts, so the reserved-word
///      diagnostic explains the authored mistake. Other unmatched keyword forms keep their
///      ordinary unexpected-token path.
fn keyword_binding_name_error(
    token_stream: &AstCursor,
    string_table: &mut StringTable,
) -> Option<DiagnosticDraft> {
    let next = token_stream.peek_next_tag()?;
    let name_shape = next.is_assignment_operator()
        || next.is_builtin_scalar_type_name()
        || matches!(
            next,
            TokenTag::SYMBOL | TokenTag::DATATYPE_NONE | TokenTag::OPEN_CURLY | TokenTag::COMMA
        );
    if !name_shape || !logical_line_has_assignment(token_stream) {
        return None;
    }

    identifier_policy::reject_reserved_word_token(
        token_stream.current_tag(),
        Some(token_stream.current_span()),
        string_table,
    )
}

/// True when the current logical line contains a top-level assignment operator.
///
/// A binding writes `=` after its name, and a typed declaration writes it after the type. The scan
/// stops at the line boundary so a keyword in a call or operator expression stays unexpected-token
/// syntax rather than becoming a reserved-name report.
fn logical_line_has_assignment(token_stream: &AstCursor) -> bool {
    let mut depth = NestingDepth::default();
    let mut index = token_stream.position();
    while let Some(token) = token_stream.token_ref_at(index) {
        let tag = token.tag();
        if matches!(tag, TokenTag::NEWLINE | TokenTag::END | TokenTag::EOF) {
            break;
        }
        if depth.is_top_level() && tag.is_assignment_operator() {
            return true;
        }
        depth.step_tag(tag);
        index += 1;
    }
    false
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

/// Constraints attached to one upcoming statement by a source-directive run.
///
/// WHAT: the set to restore once the statement finishes, the effective set to record, and the
///       target's first authored token.
/// WHY: a statement annotation applies only to the immediately following statement subtree. This
///      loop-local record keeps the reset and the verification fact at the one point where the
///      parser knows both the enclosing and attached sets without threading a second context.
struct PendingStatementConstraints {
    enclosing: SourceConstraintSet,
    effective: SourceConstraintSet,
    start_span: SourceSpan,
    directive_spans: ConstraintDirectiveSpans,
}

/// Returns the last non-newline token span before the cursor.
///
/// WHAT: walks backwards from the current position, skipping newline tokens.
/// WHY: an annotated statement's authored range must end at its final meaningful token even when
///      the statement parser leaves its trailing newline for the next loop iteration.
fn last_statement_token_span(cursor: &AstCursor<'_>) -> Option<SourceSpan> {
    let mut index = cursor.position().min(cursor.length());
    while index > 0 {
        index -= 1;
        let token = cursor.token_ref_at(index)?;
        if token.tag() != TokenTag::NEWLINE {
            return Some(token.source_span());
        }
    }
    None
}

/// Restores the enclosing constraint set and records the completed statement boundary.
fn finish_pending_statement_constraints(
    context: &mut ScopeContext,
    cursor: &AstCursor<'_>,
    pending: PendingStatementConstraints,
) {
    context.source_constraints.set(pending.enclosing);
    let Some(function) = context.constraint_function else {
        return;
    };
    let end = last_statement_token_span(cursor).unwrap_or(pending.start_span);
    context
        .source_constraint_boundaries
        .borrow_mut()
        .push(AstConstraintBoundary {
            function,
            scope: context.scope,
            statement: Some(AstConstraintStatementSpan {
                start: pending.start_span,
                end,
            }),
            constraints: pending.effective,
            directive_spans: pending.directive_spans,
        });
}

/// Parse the statements of one function, entry or nested block body.
///
/// `body_opening` is the effective opening site of an authored block body: the colon that opened
/// it, or the `else`/`=>` token that introduces a terminal branch. End of file inside such a body
/// is a missing `;` and is rejected with a label on that opener. `None` marks a body whose end of
/// file is not a missing closer: the compiler-owned entry start, function bodies whose token range
/// is already balanced by header capture, and match arms whose range ends with the enclosing
/// match's own report.
#[allow(
    clippy::too_many_arguments,
    reason = "function body keeps stream, scope, warnings and mutable parse resources apart"
)]
pub(crate) fn parse_function_body_statements(
    token_stream: &mut AstCursor,
    mut context: ScopeContext,
    body_opening: Option<ScopeOpening>,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    warnings: &mut DiagnosticBag,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> StatementDispatchResult<ParsedBody> {
    // Header capture already balanced a function body at its own closer, so that `;` never reaches
    // this loop. The signature colon still encloses every opener the body contains, and it is the
    // canonical token immediately before the captured range.
    if body_opening.is_none() && context.kind == ContextKind::Function {
        context.enclosing_opener = function_signature_opening(token_stream)?;
    }

    // Authored statement order for this body, including every position whose statement could not
    // finish until the body root had selected each numeric origin's single static type.
    let mut draft = BodyDraft::new();
    // A constraint run attaches to exactly the next statement. `directive_just_attached` marks the
    // one loop iteration that parses that statement, so the reset below runs only on the
    // following iteration, after the whole statement subtree has been parsed.
    let mut pending_statement_constraints: Option<PendingStatementConstraints> = None;
    let mut directive_just_attached = false;

    while token_stream.position() < token_stream.length() {
        // The iteration right after attachment parses the annotated statement itself; the
        // following iteration is the first one allowed to close the attachment.
        if directive_just_attached {
            directive_just_attached = false;
        } else if let Some(pending) = pending_statement_constraints.take() {
            finish_pending_statement_constraints(&mut context, token_stream, pending);
        }

        let current_tag = token_stream.current_tag();

        ast_log!("Parsing Token: ", #current_tag);

        if current_tag == TokenTag::STYLE_DIRECTIVE {
            let in_match_arm = context.kind == ContextKind::MatchArm;
            let attachment = parse_statement_source_directive_run(
                token_stream,
                &mut context,
                SourceOperandFoldResources {
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    path_fork,
                },
                |cursor| {
                    in_match_arm
                        && (current_token_starts_match_arm_header(cursor).is_some()
                            || current_token_starts_colon_separated_arm_header(cursor))
                },
            )?;
            if !attachment.constraints.is_empty() {
                let enclosing = context.source_constraints.get();
                let effective = enclosing.union(attachment.constraints);
                context.source_constraints.set(effective);
                pending_statement_constraints = Some(PendingStatementConstraints {
                    enclosing,
                    effective,
                    start_span: attachment.target_span,
                    directive_spans: attachment.directive_spans,
                });
                directive_just_attached = true;
            }
            continue;
        }

        // Statement-boundary `~[...]` is the explicit runtime fragment in the compiler-owned
        // entry body. Consume the marker so the template parser sees the opening head. The
        // previous-token check runs only for this marker, not for every authored statement.
        if current_tag == TokenTag::MUTABLE
            && context.kind == ContextKind::Module
            && token_stream.peek_next_tag() == Some(TokenTag::TEMPLATE_HEAD)
            && token_stream.previous().is_none_or(|token| {
                matches!(
                    token.tag(),
                    TokenTag::MODULE_START | TokenTag::NEWLINE | TokenTag::END
                )
            })
        {
            token_stream.advance();
            let fragment_span = Some(token_stream.current_span());
            let template = parse_statement_template(
                token_stream,
                &context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            )?;
            let expression = expression_builder.template(template, ValueMode::MutableOwned)?;

            draft.push_node(AstNode {
                kind: NodeKind::PushStartRuntimeFragment(expression.id()),
                span: fragment_span,
                scope: context.scope,
            });
            continue;
        }

        // Match-arm bodies end when the next line-initial arm header or `else` is reached.
        // Same-line accidental second arms are rejected here before statement dispatch.
        if context.kind == ContextKind::MatchArm {
            if let Some(candidate) = current_token_starts_match_arm_header(token_stream) {
                debug_assert_eq!(candidate.start_index, token_stream.position());
                break;
            }

            // A `name:` header is either a declared region or an arm that used the wrong
            // separator. This scan cannot tell them apart, and a match body accepts `=>` only,
            // so stop here and let the match parser name the missing arm separator.
            if current_token_starts_colon_separated_arm_header(token_stream) {
                break;
            }

            if token_stream.current_tag() != TokenTag::ELSE
                && let Some(arm_anchor) = same_line_match_arm_anchor(token_stream)
            {
                return Err(statement_dispatch_error(
                    choices_control_flow::invalid_match_arm(
                        InvalidMatchArmReason::ArmMustStartNewLine,
                        Some(arm_anchor),
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
                    let reason = match context.kind {
                        ContextKind::Loop if context.loop_blocks_value_target => {
                            InvalidFallibleHandlingReason::ThenCrossesBlockedConstruct
                        }
                        ContextKind::Loop => InvalidFallibleHandlingReason::ThenInsideLoopBody,
                        ContextKind::CatchHandler | ContextKind::Template => {
                            InvalidFallibleHandlingReason::ThenCrossesBlockedConstruct
                        }
                        _ => InvalidFallibleHandlingReason::ThenWithNoActiveValueTarget,
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

                // Top-level code owns no block, so a `;` here can only be a stray closer.
                // Accepting it would end the start body and silently drop what follows.
                ContextKind::Module => {
                    return Err(statement_dispatch_error(unexpected_scope_close(
                        UnexpectedScopeCloseContext::TopLevel,
                        token_stream,
                    )));
                }

                ContextKind::MatchArm => break,

                // Every other body kind consumes its own `;`, so this is the one owner of that
                // closer's placement and trailing content.
                _ => {
                    if let Some(opening) = body_opening
                        && let Some(diagnostic) = scope_closer_layout_error(token_stream, opening)
                    {
                        return Err(statement_dispatch_error(diagnostic));
                    }
                    token_stream.advance();
                    break;
                }
            },

            // Header parsing owns each line-start direct `[...]` fragment as page or builder-slot
            // metadata, and runtime fragments are marked with `~` and handled above. Any template
            // that still reaches statement dispatch is authored input in statement position, such
            // as a template inside a function body or an executable block. It is parsed first so
            // its own diagnostic wins, as for an operand after a value template.
            TokenTag::TEMPLATE_HEAD => {
                let template_span = Some(token_stream.current_span());
                parse_statement_template(
                    token_stream,
                    &context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    path_fork,
                )?;
                return Err(statement_dispatch_error(
                    choices_control_flow::invalid_standalone_statement(
                        InvalidStandaloneStatementReason::StandaloneTemplate,
                        template_span,
                    ),
                ));
            }

            // End of file
            TokenTag::EOF => {
                // An authored block body closes with `;`. Reaching the file end inside one means
                // that block was never closed, so the statements before it must not publish as if
                // the body had ended normally.
                if let Some(opening) = body_opening {
                    // The last `;` after this opener closed the innermost child scope. It is
                    // exact here because this body is the innermost open scope at end of file.
                    let inner_closer = last_child_closer_span(token_stream, opening.opener);
                    return Err(statement_dispatch_error(
                        token_lexical::unclosed_source_scope(
                            opening.construct,
                            Some(token_stream.current_span()),
                            opening.label_span,
                            inner_closer,
                        ),
                    ));
                }
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

            // A directive is never a statement. Known source modifiers and template directives
            // both reach this arm, so the diagnostic names the authored directive instead of
            // collapsing into the generic unexpected-token path.
            TokenTag::STYLE_DIRECTIVE => {
                return Err(source_directive_wrong_target_error(
                    token_stream,
                    string_table,
                )?);
            }

            // Unrecognized tokens
            _ => {
                if let Some(diagnostic) = keyword_binding_name_error(token_stream, string_table) {
                    return Err(statement_dispatch_error(diagnostic));
                }

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

    // A constraint attached to the body's final statement must still record its boundary when the
    // loop ends without another iteration.
    if let Some(pending) = pending_statement_constraints.take() {
        finish_pending_statement_constraints(&mut context, token_stream, pending);
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
