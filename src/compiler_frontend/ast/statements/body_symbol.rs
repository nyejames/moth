//! Symbol-led function-body statement parsing.
//!
//! WHAT: parses statement forms that start with a symbol inside function/start-function bodies.
//! WHY: symbol-led statements are the densest statement branch (mutation, calls, declarations,
//! access chains and start dependency callability), so isolating them keeps dispatch readable.
//!
//! A symbol that resolves to a pending numeric binding is a body-owned origin whose single static
//! type is selected at the body root. Mutations of it record one ordered continuation instead of
//! being built early; a multi-bind keeps supported producers on that same completion path, while
//! unsupported RHS shapes report their authored rule diagnostic.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionOperationView, OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::mutation::{
    MutationOutcome, compound_assignment_operator_for_tag, handle_mutation, handle_mutation_target,
};
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_with_trailing_newline_policy;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::expressions::parse_expression_places::place_expression_from_expression;
use crate::compiler_frontend::ast::field_access::{
    ParsedPostfixExpression, PostfixChainAccess, RetainedMemberTarget, RetainedOperandInput,
    parse_field_access, retain_pending_operand_member_access, retained_member_target,
};
use crate::compiler_frontend::ast::numeric_construction::{
    AssignmentContinuation, BodyDraft, CompoundAssignmentContinuation, DeferredAssignmentTarget,
    DeferredExpressionSuffix, ExpressionStatementContinuation, NumericBindingId,
    NumericContinuation, PendingPlaceTarget, StatementExpression, binding_reference_shape,
    deferred_numeric_expression, numeric_witness_type, register_binding_witness,
    single_direct_pending_reference, union_alias_classes,
};
use crate::compiler_frontend::ast::numeric_known_shape::{
    exact_shape_type, project_exact_shape, register_shape_witness, union_shape_classes,
};
use crate::compiler_frontend::ast::receiver_methods::free_function_receiver_method_call_error;
use crate::compiler_frontend::ast::statements::body_expr_stmt::{
    is_expression_statement, push_statement_expression,
};
use crate::compiler_frontend::ast::statements::declarations::ResolvedDeclarationStatementKind;
use crate::compiler_frontend::ast::statements::declarations::{
    NewDeclarationOutcome, new_declaration,
};
use crate::compiler_frontend::ast::statements::multi_bind::{
    MultiBindStatement, parse_multi_bind_statement,
};
use crate::compiler_frontend::ast::statements::value_production::multi_bind::finalize_inferred_value_block;
use crate::compiler_frontend::ast::statements::value_production::receiver::try_parse_value_block_at_receiver_with_target;
use crate::compiler_frontend::ast::statements::value_production::types::ParsedReceiverValue;
use crate::compiler_frontend::ast::statements::value_production::{
    ActiveValueProductionTarget, ValueReceiverKind,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::builtins::error_type::is_reserved_builtin_symbol;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidAssignmentTargetReason, InvalidDeclarationReason,
    InvalidFieldAccessReason, InvalidMultiBindReason, InvalidStandaloneStatementReason,
    InvalidThisUsageReason, ReservedNameOwner, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::syntax_errors::statement_position::check_mistaken_keyword_symbol;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedType, cast_target_context_for_type_id, parse_expectation_for_type_id,
};
use crate::compiler_frontend::value_mode::ValueMode;

// --------------------------
//  Statement routing helpers
// --------------------------

/// Route one parsed mutation into the body draft.
///
/// WHAT: pushes the finished node, or records the retained continuation at the authored position.
/// WHY: a mutation whose right-hand side still holds numeric origins has no node yet; its target
///      and authored position are already fixed, so the body root resumes it exactly once.
fn push_mutation_outcome(outcome: MutationOutcome, draft: &mut BodyDraft) {
    match outcome {
        MutationOutcome::Ready(node) => draft.push_node(node),
        MutationOutcome::Retained(continuation) => draft.push_pending(continuation),
    }
}

fn push_accessed_symbol_statement(
    accessed_expression: OwnedExpr,
    draft: &mut BodyDraft,
    context: &ScopeContext,
    token_stream: &AstCursor,
    _symbol_id: StringId,
    _string_table: &StringTable,
    expressions: &AstExpressionBuilder,
) -> Result<(), ExpressionParseError> {
    let view = expressions.view_owned(accessed_expression)?;
    if is_expression_statement(view) {
        draft.push_node(AstNode {
            span: view.span(),
            kind: NodeKind::ExpressionStatement(accessed_expression.id()),
            scope: context.scope,
        });
        return Ok(());
    }

    // A bare field read (e.g., `obj.field`) does nothing, so it is rejected.
    if matches!(
        view.operation(),
        ExpressionOperationView::FieldAccess { .. }
    ) {
        return Err(CompilerDiagnostic::invalid_standalone_statement(
            InvalidStandaloneStatementReason::FieldRead,
            Some(token_stream.current_span()),
        )
        .into());
    }

    // Any other accessed expression is also not a valid standalone statement.
    Err(CompilerDiagnostic::invalid_standalone_statement(
        InvalidStandaloneStatementReason::Expression,
        Some(token_stream.current_span()),
    )
    .into())
}

/// Route one retained postfix expression at its authored position.
///
/// WHAT: rejects assignment to a temporary receiver and otherwise records the retained item as an
///       expression-statement continuation.
/// WHY: the operand owns the retained chain and any delayed invalid-member occurrence, so the
///      body root resumes that authored position through its ordinary continuation path.
fn push_retained_accessed_symbol_statement(
    item: ExpressionRpnItem,
    draft: &mut BodyDraft,
    context: &ScopeContext,
    token_stream: &AstCursor,
    expressions: &AstExpressionBuilder,
) -> Result<(), ExpressionParseError> {
    if token_stream.current_tag().is_assignment_operator() {
        return Err(CompilerDiagnostic::invalid_assignment_target(
            InvalidAssignmentTargetReason::TemporaryNotAssignable,
            None,
            None,
            None,
            None,
            None,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let span = item.source_span(expressions)?;
    let expression = StatementExpression::Retained(deferred_numeric_expression(
        vec![item],
        ExpectedType::Infer,
        ValueMode::ImmutableOwned,
        span,
        DeferredExpressionSuffix::None,
    ));
    let continuation = context.register_numeric_continuation(
        NumericContinuation::ExpressionStatement(ExpressionStatementContinuation {
            expression,
            span,
            scope: context.scope,
        }),
    )?;
    draft.push_pending(continuation);
    Ok(())
}

// --------------------------
//  `this` statement parsing
// --------------------------

#[allow(
    clippy::too_many_arguments,
    reason = "this statement keeps stream, body draft, scope and mutable parse resources apart"
)]
pub(crate) fn parse_this_statement(
    token_stream: &mut AstCursor,
    draft: &mut BodyDraft,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    let this_id = string_table.intern("this");
    let this_reference = context.resolve_this_reference(this_id, token_stream.current_span())?;
    // `this` is a receiver declaration, never a body-owned numeric origin.
    let this_declaration = this_reference
        .ready_declaration()
        .ok_or_else(|| {
            CompilerError::compiler_error("`this` resolved to a pending numeric binding")
        })?
        .to_owned();

    match token_stream.peek_next_tag() {
        // Direct reassignment of `this` is never allowed.
        Some(next_tag) if next_tag.is_assignment_operator() => {
            Err(CompilerDiagnostic::invalid_this_usage(
                InvalidThisUsageReason::Reassignment,
                Some(token_stream.current_span()),
            )
            .into())
        }

        // Field access on `this`: may be a mutation (`this.x = ...`) or a
        // method/collection call (`this.x()`).
        Some(TokenTag::DOT) => {
            token_stream.advance();
            let accessed_node = parse_field_access(
                token_stream,
                &this_declaration,
                context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            )?;
            let accessed_node = match accessed_node {
                ParsedPostfixExpression::Value(node) => node,
                ParsedPostfixExpression::RetainedOperand(item) => {
                    return push_retained_accessed_symbol_statement(
                        item,
                        draft,
                        context,
                        token_stream,
                        expression_builder,
                    );
                }
            };

            if token_stream.current_tag().is_assignment_operator() {
                let Some(target) =
                    place_expression_from_expression(expression_builder, accessed_node)?
                else {
                    return Err(CompilerDiagnostic::invalid_assignment_target(
                        InvalidAssignmentTargetReason::TemporaryNotAssignable,
                        None,
                        Some(
                            expression_builder
                                .view_owned(accessed_node)?
                                .require_single(ValueReceiverContext::ReceiverBase)?,
                        ),
                        None,
                        None,
                        None,
                        Some(token_stream.current_span()),
                    )
                    .into());
                };

                let mutation = handle_mutation_target(
                    token_stream,
                    &this_declaration,
                    target,
                    this_reference.binding_span(),
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    path_fork,
                )?;

                push_mutation_outcome(mutation, draft);
                return Ok(());
            }

            push_accessed_symbol_statement(
                accessed_node,
                draft,
                context,
                token_stream,
                this_id,
                string_table,
                expression_builder,
            )?;
            Ok(())
        }

        // Bare `this` or `this` used as the start of a general expression.
        _ => push_statement_expression(
            token_stream,
            draft,
            context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        ),
    }
}

// --------------------------
//  Symbol-led statement parsing
// --------------------------

/// One authored invalid member access on an unresolved numeric binding, retained with its span.
///
/// WHAT: the member name and authored span; the operand separately retains the binding reference.
/// WHY: completing that operand supplies the selected receiver type after ordinary binding
///      resolution, so the occurrence can report the ordinary delayed diagnostic without a
///      fabricated receiver or a duplicate continuation record.
#[derive(Debug, Clone)]
pub(crate) struct PendingNumericMemberAccess {
    pub(crate) member_name: StringId,
    pub(crate) member_span: Option<SourceSpan>,
}

/// The ordinary member diagnostic for one retained occurrence, after its binding resolved.
///
/// WHAT: the same unknown-member diagnostic a known receiver type reports, with the selected
///       receiver type and no member suggestions (a numeric scalar has no members to suggest).
/// WHY: the diagnostic must be issued after independent binding resolution and its normal
///      range/conflict handling, in the exact shape the completed member path uses.
pub(crate) fn pending_member_access_diagnostic(
    occurrence: &PendingNumericMemberAccess,
    receiver_type_id: TypeId,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_field_access(
        InvalidFieldAccessReason::UnknownMember,
        Some(occurrence.member_name),
        Some(receiver_type_id),
        Vec::new(),
        occurrence.member_span,
    )
}

/// Build the construction-only operand item for one pending numeric binding reference.
///
/// WHAT: reads the binding's authored value mode from its own live construction domain and keeps
///       the plain-reference shape.
/// WHY: member resolution on the reference needs the same retained item every other pending
///      binding use carries; `None` from the domain-checked lookup is a broken compiler invariant.
fn pending_binding_reference_item(
    context: &ScopeContext,
    binding: NumericBindingId,
    span: Option<SourceSpan>,
) -> Result<ExpressionRpnItem, ExpressionParseError> {
    let value_mode = context.pending_numeric_value_mode(binding).ok_or_else(|| {
        CompilerError::compiler_error(
            "a pending numeric binding reference had no live row in its own construction domain",
        )
    })?;
    Ok(ExpressionRpnItem::PendingNumericBinding {
        binding,
        span,
        value_mode,
        is_copy: false,
    })
}

/// Parse one right-hand side owned by a pending numeric target.
///
/// WHAT: an unselected target (a pending binding, or a member whose type is still open) keeps an
///       authored fragment whenever the value holds literal leaves (which the completed target type
///       types) or unresolved origins. A selected target (a plain member assignment to a concrete
///       type) parses under that type's ordinary receiving contract.
/// WHY: materialising a raw literal at the language default before the target's receiving context
///      is known would select the wrong domain; a selected type already is that receiving context.
#[allow(
    clippy::too_many_arguments,
    reason = "pending rhs keeps stream, scope, value mode, selected target and mutable parse resources apart"
)]
fn parse_pending_mutation_rhs(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    value_mode: &ValueMode,
    selected_target: Option<TypeId>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<StatementExpression, ExpressionParseError> {
    // A value-producing `if` at this assignment's receiving position belongs to the shared
    // value-production receiver owner, exactly as it does at a ready target. An unselected target
    // keeps one inferred slot for this receiver kind, with no slot TypeId invented; a selected
    // target supplies its known slot. The canonical missing-else and multiline-else diagnostics
    // come from that owner instead of the general dispatch's outside-receiver rejection.
    if let Some(value_block_result) = try_parse_value_block_at_receiver_with_target(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        ActiveValueProductionTarget::mixed(&[selected_target], ValueReceiverKind::Assignment),
        string_table,
        path_fork,
    ) {
        return match value_block_result {
            Ok(ParsedReceiverValue::Complete(expression)) => {
                Ok(StatementExpression::Built(expression.id()))
            }
            // The block's authored values still hold the pending numeric origins (or its bodies
            // hold continuations), so the assignment keeps the whole block and the body root
            // resolves it against the target's now-selected expectations.
            Ok(ParsedReceiverValue::Retained(retained)) => {
                let deferral = retained.into_deferral(value_mode).take_numeric_deferral(
                    "the value of a mutation whose target has no selected numeric type",
                )?;
                Ok(StatementExpression::Retained(deferred_numeric_expression(
                    deferral.items,
                    deferral.expected,
                    deferral.value_mode,
                    deferral.span,
                    deferral.suffix,
                )))
            }
            // A structurally complete block infers its single slot from its own produced values
            // through the value-production owner's finalizer; the target's pending type is never
            // used as a slot stand-in.
            Ok(ParsedReceiverValue::NeedsSlotInference { block, span }) => {
                let expression = finalize_inferred_value_block(
                    block,
                    span,
                    &[None],
                    1,
                    type_interner,
                    expression_builder,
                    const_values,
                )?;
                Ok(StatementExpression::Built(expression.id()))
            }
            // A nested retained construct inside the block keeps the ordinary deferral
            // conversion this right-hand side already owns; every authored diagnostic (the
            // missing-else and multiline forms) passes through unchanged.
            Err(error @ ExpressionParseError::NumericDeferral(_)) => {
                let deferral = error.take_numeric_deferral(
                    "the value of a mutation whose target has no selected numeric type",
                )?;
                Ok(StatementExpression::Retained(deferred_numeric_expression(
                    deferral.items,
                    deferral.expected,
                    deferral.value_mode,
                    deferral.span,
                    deferral.suffix,
                )))
            }
            Err(error) => Err(error),
        };
    }

    let (mut expected, mut cast_target) = match selected_target {
        None => (ExpectedType::Infer, CastTargetContext::None),
        Some(target_type_id) => (
            parse_expectation_for_type_id(target_type_id, type_interner.environment()),
            cast_target_context_for_type_id(
                target_type_id,
                type_interner.environment(),
                string_table,
                &*path_fork,
            ),
        ),
    };
    let input = ExpressionParseInput::ordinary(
        ExpressionParseResources {
            token_stream,
            scope_context: context,
            type_interner,
            expression_builder,
            const_values,
            expected_type: &mut expected,
            cast_target_context: &mut cast_target,
            value_mode,
            string_table,
            path_fork,
        },
        false,
    )
    .with_numeric_deferral_owner()
    .with_retained_pending_numeric_fragment();
    match create_expression_with_trailing_newline_policy(input) {
        Ok(expression) => Ok(StatementExpression::Built(expression.id())),
        Err(error) => {
            let deferral = error.take_numeric_deferral(
                "the value of a mutation whose target has no selected numeric type",
            )?;
            Ok(StatementExpression::Retained(deferred_numeric_expression(
                deferral.items,
                deferral.expected,
                deferral.value_mode,
                deferral.span,
                deferral.suffix,
            )))
        }
    }
}

/// The pending numeric binding one mutation statement writes, with its authored name and span.
///
/// WHY: the target travels as one record so the helper's argument list stays within the call
///      shape budget while its facts still arrive together.
struct PendingMutationTarget {
    binding: NumericBindingId,
    name: StringId,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
    declaration_span: Option<crate::compiler_frontend::source::SourceSpan>,
}

/// Retain a mutation whose target is a pending numeric binding.
///
/// WHAT: parses the authored operator and right-hand side, records one ordered continuation and
///       keeps the authored position in the body draft.
/// WHY: the mutation node needs the target's place, which does not exist until the body root has
///      selected the binding's single static type; the write-back conversion re-runs there with
///      the exact type. Mutability is a binding fact, so it is checked here in authored order.
#[allow(
    clippy::too_many_arguments,
    reason = "pending mutation keeps stream, body draft, scope, target and mutable parse resources apart"
)]
fn retain_pending_mutation(
    token_stream: &mut AstCursor,
    draft: &mut BodyDraft,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    target: PendingMutationTarget,
) -> Result<(), ExpressionParseError> {
    let PendingMutationTarget {
        binding,
        name: target_name,
        span: target_span,
        declaration_span,
    } = target;
    let operator_tag = token_stream.current_tag();
    let span = Some(token_stream.current_span());
    // The pending target is this statement's own assignment target, so the same protection the
    // known-type mutation owner applies must cover the right-hand side: a `catch` recovery inside
    // it must not read or mutate the target whose value does not exist yet.
    let rhs_context = context.with_pending_catch_assignment_targets(&[target_name]);
    let value_mode = context.pending_numeric_value_mode(binding).ok_or_else(|| {
        CompilerError::compiler_error(
            "a mutation target named a pending numeric binding with no live construction row",
        )
    })?;
    if !value_mode.is_mutable() {
        return Err(CompilerDiagnostic::invalid_assignment_target(
            InvalidAssignmentTargetReason::ImmutableBinding,
            Some(target_name),
            None,
            None,
            None,
            declaration_span,
            span,
        )
        .into());
    }
    let target = DeferredAssignmentTarget::Pending(PendingPlaceTarget {
        binding,
        span: target_span,
    });

    if operator_tag == TokenTag::ASSIGN {
        token_stream.advance();
        let value = parse_pending_mutation_rhs(
            token_stream,
            &rhs_context,
            type_interner,
            expression_builder,
            const_values,
            &value_mode,
            None,
            string_table,
            path_fork,
        )?;
        // A completed value of an exact numeric type fixes the target's single static type, and a
        // bare reference to another origin makes the two one numeric unknown (a copy/equality),
        // exactly like a declaration-level copy.
        match &value {
            StatementExpression::Built(expression) => {
                let mut state = context.numeric_state_mut();
                register_binding_witness(
                    &mut state,
                    type_interner.environment(),
                    binding,
                    expression_builder
                        .view(*expression)?
                        .require_single(ValueReceiverContext::Assignment)?,
                    span,
                );
            }
            StatementExpression::Retained(deferred) => {
                if let Some(reference) = single_direct_pending_reference(&deferred.items) {
                    let mut state = context.numeric_state_mut();
                    union_alias_classes(&mut state, binding, reference.binding);
                }
            }
        }
        let continuation = context.register_numeric_continuation(
            NumericContinuation::Assignment(AssignmentContinuation {
                target,
                value,
                span,
                scope: context.scope,
            }),
        )?;
        draft.push_pending(continuation);
        return Ok(());
    }

    let Some((operator, _label)) = compound_assignment_operator_for_tag(operator_tag) else {
        return Err(CompilerError::compiler_error(
            "a mutation target carried an assignment operator with no arithmetic operator",
        )
        .into());
    };
    token_stream.advance();
    let value = parse_pending_mutation_rhs(
        token_stream,
        &rhs_context,
        type_interner,
        expression_builder,
        const_values,
        &value_mode,
        None,
        string_table,
        path_fork,
    )?;
    if let StatementExpression::Built(expression) = &value
        && let Some(type_id) = numeric_witness_type(
            expression_builder
                .view(*expression)?
                .require_single(ValueReceiverContext::Assignment)?,
            type_interner.environment(),
        )
    {
        // A concrete right-hand side fixes the target's single static type: the compound update
        // is an assignment boundary like any other mutation.
        let mut state = context.numeric_state_mut();
        register_binding_witness(
            &mut state,
            type_interner.environment(),
            binding,
            type_id,
            span,
        );
    }
    let continuation = context.register_numeric_continuation(
        NumericContinuation::CompoundAssignment(CompoundAssignmentContinuation {
            target,
            operator,
            value,
            span,
            scope: context.scope,
        }),
    )?;
    draft.push_pending(continuation);
    Ok(())
}

/// The pending struct member one mutation statement writes through its numeric root.
///
/// WHAT: the authored member operand, its assignable member place and the root binding facts that
///       the statement's value mode, catch protection and declaration boundary come from.
/// WHY: the member is written through the root, so the target carries the root's declaration facts
///      instead of a binding of its own.
struct PendingMemberMutation {
    item: ExpressionRpnItem,
    member: RetainedMemberTarget,
    root: NumericBindingId,
    root_name: StringId,
    root_span: Option<SourceSpan>,
}

/// Retain a mutation whose target is a field of a pending numeric struct root.
///
/// WHAT: parses the authored operator and right-hand side, then records one ordered continuation
///       whose target is the retained member place.
/// WHY: the member's declared shape and the root's single static type are known only at body
///      completion, where the place resolves exactly like an ordinary field place; the root's
///      mutability and declaration span still gate the write there.
#[allow(
    clippy::too_many_arguments,
    reason = "member mutation keeps stream, body draft, scope, mutation and mutable parse resources apart"
)]
fn retain_pending_member_mutation(
    token_stream: &mut AstCursor,
    draft: &mut BodyDraft,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    mutation: PendingMemberMutation,
) -> Result<(), ExpressionParseError> {
    let PendingMemberMutation {
        item,
        member,
        root,
        root_name,
        root_span,
    } = mutation;
    let operator_tag = token_stream.current_tag();
    let span = Some(token_stream.current_span());
    // The member's root is this statement's own write target, so a `catch` in the right-hand side
    // must not read or mutate it before the write, exactly as for a pending binding target.
    let rhs_context = context.with_pending_catch_assignment_targets(&[root_name]);
    let value_mode = context.pending_numeric_value_mode(root).ok_or_else(|| {
        CompilerError::compiler_error(
            "a member mutation named a pending numeric root with no live construction row",
        )
    })?;
    if let Some(field_name) = member.field_name
        && !value_mode.is_mutable()
    {
        // The root is this statement's own write place, so an immutable root rejects the member
        // write before its right-hand side is parsed, as the ready field path does.
        return Err(CompilerDiagnostic::invalid_assignment_target(
            InvalidAssignmentTargetReason::ImmutableFieldRoot,
            None,
            None,
            Some(field_name),
            Some(root_name),
            root_span,
            span,
        )
        .into());
    }

    let operator = if operator_tag == TokenTag::ASSIGN {
        None
    } else {
        let Some((operator, _label)) = compound_assignment_operator_for_tag(operator_tag) else {
            return Err(CompilerError::compiler_error(
                "a member mutation carried an assignment operator with no arithmetic operator",
            )
            .into());
        };
        Some(operator)
    };
    token_stream.advance();
    // A plain assignment to a member whose type is selected takes that type's ordinary receiving
    // contract. A compound update, or a member whose type is still open, keeps the pending contract.
    let selected_target = if operator.is_none() {
        member_selected_type(context, type_interner, member)?
    } else {
        None
    };
    let value = parse_pending_mutation_rhs(
        token_stream,
        &rhs_context,
        type_interner,
        expression_builder,
        const_values,
        &value_mode,
        selected_target,
        string_table,
        path_fork,
    )?;
    register_member_mutation_witness(
        context,
        type_interner,
        expression_builder,
        member,
        &value,
        span,
        operator.as_ref(),
    )?;
    let target_span = item.source_span(expression_builder)?;
    let target = DeferredAssignmentTarget::Retained {
        target: deferred_numeric_expression(
            vec![item],
            ExpectedType::Infer,
            ValueMode::ImmutableOwned,
            target_span,
            DeferredExpressionSuffix::None,
        ),
        declaration_value_mode: value_mode,
        declaration_span: root_span,
    };
    let continuation = match operator {
        None => NumericContinuation::Assignment(AssignmentContinuation {
            target,
            value,
            span,
            scope: context.scope,
        }),
        Some(operator) => NumericContinuation::CompoundAssignment(CompoundAssignmentContinuation {
            target,
            operator,
            value,
            span,
            scope: context.scope,
        }),
    };
    let continuation = context.register_numeric_continuation(continuation)?;
    draft.push_pending(continuation);
    Ok(())
}

/// The concrete type a plain member write selects, when the member's type is already exact.
///
/// WHAT: a concrete member answers its own type; a declared member answers the exact type its shape
///       projects to, and an open numeric leaf answers `None`.
/// WHY: the ordinary receiving contract needs a selected type, but an unresolved leaf must stay
///      pending rather than take a family default.
fn member_selected_type(
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    member: RetainedMemberTarget,
) -> Result<Option<TypeId>, ExpressionParseError> {
    if let Some(type_id) = member.concrete_type {
        return Ok(Some(type_id));
    }
    let Some(shape) = member.declared_shape else {
        return Ok(None);
    };
    let mut state = context.numeric_state_mut();
    exact_shape_type(
        &mut state,
        type_interner.environment_mut_for_derived_types(),
        shape,
    )
}

/// Register the shape a member write shares with the value written into it.
///
/// WHAT: a concrete value fixes the member's numeric leaves. On a plain assignment, a bare reference
///       to another pending origin projects its exact receivers first, then joins only its still-open
///       leaves with the member's. A compound update only fixes the member from a concrete value,
///       matching the binding-target rules.
/// WHY: the member and the written value share one static numeric type, so the root's later
///      receivers observe the same type the write selected. Projecting first keeps the origin's
///      pre-binding receivers from importing into the member's already declared family.
fn register_member_mutation_witness(
    context: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
    expression_builder: &AstExpressionBuilder,
    member: RetainedMemberTarget,
    value: &StatementExpression,
    span: Option<SourceSpan>,
    operator: Option<&Operator>,
) -> Result<(), ExpressionParseError> {
    let Some(declared_shape) = member.declared_shape else {
        return Ok(());
    };
    let environment = type_interner.environment();
    let mut state = context.numeric_state_mut();
    match (value, operator) {
        (StatementExpression::Built(expression), _) => register_shape_witness(
            &mut state,
            environment,
            declared_shape,
            expression_builder
                .view(*expression)?
                .require_single(ValueReceiverContext::Assignment)?,
            span,
        ),
        (StatementExpression::Retained(deferred), None) => {
            let Some(reference) = single_direct_pending_reference(&deferred.items) else {
                return Ok(());
            };
            let Some(reference_shape) = binding_reference_shape(&state, reference.binding) else {
                return Ok(());
            };
            // Solved leaves of the origin arrive as exact witnesses carrying their receiver's
            // source span; only its still-open leaves can join the member's classes.
            let projected = project_exact_shape(&mut state, reference_shape)?;
            // A refused numeric pair stays separate, and the coercion path reports it.
            union_shape_classes(&mut state, environment, declared_shape, projected);
        }
        (StatementExpression::Retained(_), Some(_)) => {}
    }
    Ok(())
}

/// Report an escaped numeric-dependent multi-bind RHS as its ordinary source diagnostic.
///
/// WHAT: converts a deferral that the multi-bind owner could not retain into `UnsupportedRhs`,
///       preserving the authored RHS span for both whole and nested fragments.
/// WHY: deferral shape is construction detail, not a user-facing failure; only the explicitly
///      supported calls and value blocks have body-root resumption routes.
fn reject_numeric_dependency_in_multi_bind(error: ExpressionParseError) -> ExpressionParseError {
    match error {
        ExpressionParseError::NumericDeferral(deferral) => {
            let span = deferral.span.or_else(|| {
                deferral
                    .items
                    .first()
                    .and_then(ExpressionRpnItem::authored_span)
            });
            CompilerDiagnostic::invalid_multi_bind(
                InvalidMultiBindReason::UnsupportedRhs,
                None,
                span,
            )
            .into()
        }
        other => other,
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "symbol statement keeps stream, body draft, scope and mutable parse resources apart"
)]
pub(crate) fn parse_symbol_statement(
    token_stream: &mut AstCursor,
    draft: &mut BodyDraft,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    let Some(symbol_id) = token_stream.current_string_id_in(string_table)? else {
        return Err(CompilerDiagnostic::expected_symbol_statement(Some(
            token_stream.current_span(),
        ))
        .into());
    };
    // Declaration shape takes precedence over visible-name expression dispatch.
    if matches!(
        token_stream.peek_next_tag(),
        Some(TokenTag::TYPE_PARAMETER_BRACKET | TokenTag::TYPE)
    ) {
        new_declaration(
            token_stream,
            symbol_id,
            context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        )?;
        return Err(CompilerError::compiler_error(
            "a module-only declaration header escaped executable-body placement validation",
        )
        .into());
    }

    // Reject symbols that look like keywords in statement position.
    if let Some(error) = check_mistaken_keyword_symbol(symbol_id, token_stream, string_table) {
        return Err(error.into());
    }

    // Built-in type names cannot be used as value-level symbols.
    if is_reserved_builtin_symbol(string_table.resolve(symbol_id)) {
        return Err(CompilerDiagnostic::reserved_name_collision(
            symbol_id,
            ReservedNameOwner::BuiltinType,
            Some(token_stream.current_span()),
        )
        .into());
    }

    // Assignment targets are forbidden while recovering inside a catch block.
    if context.is_assignment_target_unavailable(symbol_id) {
        return Err(CompilerDiagnostic::invalid_assignment_target(
            InvalidAssignmentTargetReason::UnavailableInCatchRecovery,
            Some(symbol_id),
            None,
            None,
            None,
            None,
            Some(token_stream.current_span()),
        )
        .into());
    }

    if let Some(multi_bind) = parse_multi_bind_statement(
        token_stream,
        context,
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
        return Ok(());
    }

    // If the symbol already names a visible local, treat it as a use
    // (assignment, field access, or expression) rather than a new declaration.
    if let Some(existing_reference) = context.get_reference(&symbol_id) {
        let reference_span = Some(token_stream.current_span());
        match token_stream.peek_next_tag() {
            // Direct reassignment of an existing local variable.
            Some(next_tag) if next_tag.is_assignment_operator() => {
                token_stream.advance();
                if let Some(binding) = existing_reference.pending_binding() {
                    return retain_pending_mutation(
                        token_stream,
                        draft,
                        context,
                        type_interner,
                        expression_builder,
                        const_values,
                        string_table,
                        path_fork,
                        PendingMutationTarget {
                            binding,
                            name: symbol_id,
                            span: reference_span,
                            declaration_span: existing_reference.binding_span(),
                        },
                    );
                }
                let declaration = existing_reference
                    .ready_declaration()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "a mutation target reference had neither a declaration nor a pending binding",
                        )
                    })?
                    .to_owned();
                let mutation = handle_mutation(
                    token_stream,
                    &declaration,
                    existing_reference.binding_span(),
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    path_fork,
                )?;

                push_mutation_outcome(mutation, draft);
                return Ok(());
            }

            // Field access on an existing local: may be a mutation or a call.
            Some(TokenTag::DOT) => {
                if let Some(binding) = existing_reference.pending_binding() {
                    // A pending binding's receiver stays at its authored dot: a known concrete
                    // result shape retains its authored member steps as a statement value, while a
                    // numeric-only origin records the invalid-member occurrence for its delayed
                    // ordinary diagnostic so every later eligible use still registers.
                    token_stream.advance();
                    let mut expression = vec![pending_binding_reference_item(
                        context,
                        binding,
                        reference_span,
                    )?];
                    retain_pending_operand_member_access(
                        token_stream,
                        &mut expression,
                        RetainedOperandInput {
                            chain_access: PostfixChainAccess::shared(),
                            expected: ExpectedType::Infer,
                            value_mode: &ValueMode::ImmutableOwned,
                        },
                        context,
                        type_interner,
                        expression_builder,
                        const_values,
                        string_table,
                        path_fork,
                    )?;
                    let item = expression
                        .pop()
                        .expect("a retained operand keeps its authored position");
                    if token_stream.current_tag().is_assignment_operator()
                        && let Some(member) = retained_member_target(&item)
                    {
                        return retain_pending_member_mutation(
                            token_stream,
                            draft,
                            context,
                            type_interner,
                            expression_builder,
                            const_values,
                            string_table,
                            path_fork,
                            PendingMemberMutation {
                                item,
                                member,
                                root: binding,
                                root_name: symbol_id,
                                root_span: existing_reference.binding_span(),
                            },
                        );
                    }
                    return push_retained_accessed_symbol_statement(
                        item,
                        draft,
                        context,
                        token_stream,
                        expression_builder,
                    );
                }

                token_stream.advance();
                let declaration = existing_reference
                    .ready_declaration()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "a member-access receiver had neither a declaration nor a pending binding",
                        )
                    })?
                    .to_owned();
                let accessed_node = parse_field_access(
                    token_stream,
                    &declaration,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    path_fork,
                )?;
                let accessed_node = match accessed_node {
                    ParsedPostfixExpression::Value(node) => node,
                    ParsedPostfixExpression::RetainedOperand(item) => {
                        return push_retained_accessed_symbol_statement(
                            item,
                            draft,
                            context,
                            token_stream,
                            expression_builder,
                        );
                    }
                };

                if token_stream.current_tag().is_assignment_operator() {
                    let Some(target) =
                        place_expression_from_expression(expression_builder, accessed_node)?
                    else {
                        return Err(CompilerDiagnostic::invalid_assignment_target(
                            InvalidAssignmentTargetReason::TemporaryNotAssignable,
                            None,
                            Some(
                                expression_builder
                                    .view_owned(accessed_node)?
                                    .require_single(ValueReceiverContext::ReceiverBase)?,
                            ),
                            None,
                            None,
                            None,
                            Some(token_stream.current_span()),
                        )
                        .into());
                    };

                    let mutation = handle_mutation_target(
                        token_stream,
                        &declaration,
                        target,
                        existing_reference.binding_span(),
                        context,
                        type_interner,
                        expression_builder,
                        const_values,
                        string_table,
                        path_fork,
                    )?;

                    push_mutation_outcome(mutation, draft);
                    return Ok(());
                }

                push_accessed_symbol_statement(
                    accessed_node,
                    draft,
                    context,
                    token_stream,
                    symbol_id,
                    string_table,
                    expression_builder,
                )?;
                return Ok(());
            }

            // A type keyword after an existing symbol means the user is trying to
            // redeclare it with an explicit type, which is a shadowing error.
            Some(tag) if tag.is_builtin_scalar_type_name() || tag == TokenTag::MUTABLE => {
                let diagnostic = CompilerDiagnostic::shadowed_name(
                    symbol_id,
                    existing_reference.binding_span(),
                    Some(token_stream.current_span()),
                );
                return Err(diagnostic.into());
            }

            // Otherwise, parse the symbol as the start of a general expression statement.
            _ => {
                push_statement_expression(
                    token_stream,
                    draft,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    path_fork,
                )?;
                return Ok(());
            }
        }
    }

    // Declaration shapes must reach the binding owner, which diagnoses a collision at the name
    // token. Other host uses keep call parsing, including its malformed-call diagnostics.
    let starts_declaration = token_stream.peek_next_tag().is_some_and(|tag| {
        tag.is_builtin_scalar_type_name()
            || matches!(
                tag,
                TokenTag::ASSIGN
                    | TokenTag::MUTABLE
                    | TokenTag::HASH
                    | TokenTag::SYMBOL
                    | TokenTag::OPEN_CURLY
            )
    });
    if !starts_declaration
        && context
            .lookup_visible_external_function(symbol_id)
            .is_some()
    {
        // Host calls share the receiving expression owner with source calls, so a catch protects
        // the whole call and its operands rather than completing at this statement dispatcher.
        push_statement_expression(
            token_stream,
            draft,
            context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        )?;
        return Ok(());
    }

    // An open parenthesis after an unknown symbol means a call attempt.
    // Provide targeted diagnostics for receiver methods and external types.
    if token_stream.peek_next_tag() == Some(TokenTag::OPEN_PARENTHESIS) {
        if let Some(receiver_method_entry) =
            context.lookup_visible_receiver_method_by_name(symbol_id)
        {
            return Err(free_function_receiver_method_call_error(
                symbol_id,
                receiver_method_entry,
                Some(token_stream.current_span()),
                string_table,
            )
            .into());
        }

        if context.lookup_visible_external_type(symbol_id).is_some() {
            return Err(CompilerDiagnostic::invalid_declaration(
                InvalidDeclarationReason::ExternalTypeLiteralConstruction,
                Some(symbol_id),
                Some(token_stream.current_span()),
            )
            .into());
        }

        return Err(CompilerDiagnostic::unknown_value_name(
            symbol_id,
            Some(token_stream.current_span()),
        )
        .into());
    }

    // Namespace-record calls such as `canvas.fill_rect(...)` have no local binding for the
    // namespace symbol, but they are valid side-effect statements when the field access resolves
    // to a call. Route them through expression-statement validation before declaration parsing
    // interprets the leading symbol as a malformed declaration.
    if token_stream.peek_next_tag() == Some(TokenTag::DOT) {
        push_statement_expression(
            token_stream,
            draft,
            context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        )?;
        return Ok(());
    }

    // No existing reference and no call target: this must be a new declaration.
    let outcome = new_declaration(
        token_stream,
        symbol_id,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )?;

    let resolved_declaration = match outcome {
        NewDeclarationOutcome::PendingNumeric(continuation) => {
            // The binding is already visible through its frame slot; its declaration node resumes
            // at the body root in this authored position.
            draft.push_pending(continuation);
            return Ok(());
        }
        NewDeclarationOutcome::Ready(resolved) => resolved,
    };

    let declaration = resolved_declaration.declaration;
    let statement_kind = resolved_declaration.statement_kind;
    let is_compile_time_binding = resolved_declaration.is_compile_time_binding;
    let declaration_span = resolved_declaration.binding_span;

    // Lift struct definitions to the statement level; ordinary bindings remain variables.
    match statement_kind {
        ResolvedDeclarationStatementKind::StructDefinition(params) => {
            draft.push_node(AstNode {
                kind: NodeKind::StructDefinition(declaration.id, params),
                span: declaration_span,
                scope: context.scope,
            });
        }

        ResolvedDeclarationStatementKind::Variable => {
            draft.push_node(AstNode {
                kind: NodeKind::VariableDeclaration(declaration.to_owned()),
                span: declaration_span,
                scope: context.scope,
            });
        }
    }

    if is_compile_time_binding {
        context.add_compile_time_var(declaration, declaration_span, path_fork);
    } else {
        context.add_var(declaration, declaration_span, path_fork);
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/struct_parsing_tests.rs"]
mod struct_parsing_tests;
