//! Postfix/member chain parsing implementation.
//!
//! WHAT: drives chained postfix parsing and dispatches each member step to focused handlers.
//! WHY: field access, receiver methods, and compiler-owned builtin members evolve independently,
//! so the chain driver should stay thin while policy lives in dedicated modules.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::ConstRecordState;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::parse_expression::complete_pending_expression;
use crate::compiler_frontend::ast::place_access::{
    ReceiverSourceState, classify_expression_source_state,
};
use crate::compiler_frontend::ast::templates::tir::public_const_rebuild::rebuild_const_template_reference;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::compiler_messages::{
    InvalidAssignmentTargetReason, InvalidFieldAccessReason, InvalidReceiverCallReason,
    ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

use super::collection_builtin::parse_collection_builtin_member_typed;
use super::field_member::{parse_field_member_access_typed, parse_member_name_typed};
use super::map_builtin::parse_map_builtin_member_typed;
use super::receiver_calls::{ParsedReceiverMethodCall, parse_receiver_method_call_typed};
use super::{MemberStepContext, ParsedBuiltinMemberCall, PostfixChainAccess, ReceiverAccessMode};

/// One parsed postfix/member chain in an operand or place position.
///
/// WHAT: either the completed value, or the single pending receiver call the chain ended in.
/// WHY: a receiver call whose numeric-dependent argument was retained has no value or type yet, so
///      the chain hands its authored operand position the construction-only item instead of a
///      fabricated value; the enclosing expression boundary then decides whether its own fragment
///      completes at the site or defers to the body root with the item in place.
pub(crate) enum ParsedPostfixExpression {
    Value(OwnedExpr),
    RetainedOperand(ExpressionRpnItem),
}

/// Internal chain result before a position converts it into an expression payload.
enum ParsedPostfixChain {
    Node(AstNode),
    RetainedOperand(ExpressionRpnItem),
}

/// Read-only previous-token span through the AST cursor.
///
/// WHAT: reports the span of the token immediately before the stream position.
/// WHY: missing-member and receiver base-span diagnostics need the owning dot or
/// base token without retaining a borrowed view.
pub(super) fn previous_token_span(token_stream: &AstCursor<'_>) -> Option<SourceSpan> {
    token_stream.previous_span().or_else(|| {
        token_stream.position().checked_sub(1).and_then(|previous| {
            token_stream
                .token_ref_at(previous)
                .map(|token| token.source_span())
        })
    })
}

fn next_token_tag(token_stream: &AstCursor<'_>) -> Option<TokenTag> {
    token_stream.token_ref_at_offset(1).map(|token| token.tag())
}
/// Narrow tag boundary: classifies the already-read lookahead tag without re-reading
/// the cursor, preserving the exact single-lookahead semantics.
fn next_token_is_assignment_operator(next_token: Option<TokenTag>) -> bool {
    next_token.is_some_and(TokenTag::is_assignment_operator)
}

/// Builds the expression payload for a declaration reference without choosing an AST node shape.
///
/// WHAT: preserves constant-context inlining, placeholder typing and value shape for the
/// field-access receiver path.
/// WHY: expression and place parsing share declaration inlining policy, while postfix members
/// bind the completed receiver ID through the same phase owner.
pub(crate) fn reference_expression_from_declaration(
    reference_arg: &Declaration,
    context: &ScopeContext,
    _type_interner: &AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    base_span: Option<SourceSpan>,
) -> Result<OwnedExpr, ExpressionParseError> {
    let initializer = reference_arg.value.initializer();
    let default = reference_arg.value.default();
    let const_record_state = match (initializer, default) {
        (Some(id), _) => {
            expression_builder
                .view(id)?
                .classification()
                .const_record_state
        }
        (_, Some(id)) => {
            const_values
                .metadata(id)
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "declaration default outside constant owner",
                    )
                })?
                .const_record_state
        }
        _ => ConstRecordState::RuntimeValue,
    };

    if context.kind.is_constant_context() {
        if context.is_unresolved_constant_placeholder(reference_arg) {
            let placeholder_type_id = context
                .expected_result_type_ids
                .first()
                .copied()
                .unwrap_or(reference_arg.type_id);
            return Ok(expression_builder.reference_with_type_id(
                reference_arg.id,
                placeholder_type_id,
                base_span,
                ValueMode::ImmutableOwned,
                const_record_state,
            )?);
        }
        if const_record_state != ConstRecordState::ConstRecord
            && let Some(initializer) = initializer
        {
            let mut input = expression_builder
                .view(initializer)?
                .to_completed_input(expression_builder)?;
            input.span = base_span;
            input.value_mode = ValueMode::ImmutableOwned;
            return Ok(expression_builder.commit_replacement(input)?);
        }
        if const_record_state != ConstRecordState::ConstRecord
            && let Some(default) = default
        {
            // WHY: an imported template constant keeps only its stable projection. A constant
            //      reference rebuilds it as a `Template` so head and slot routing see template
            //      semantics. Every other payload stays a plain constant.
            let rebuilt = {
                let mut template_store = context.template_ir_store.borrow_mut();
                rebuild_const_template_reference(
                    default,
                    &mut template_store,
                    expression_builder,
                    const_values,
                    context
                        .file_value_resolution
                        .as_ref()
                        .map(|services| &*services.module_resources),
                    string_table,
                    base_span,
                )?
            };
            // WHY: a reference occurrence carries its constant's synthetic provenance, as the
            //      baseline clone of the declaration expression did. Template folding reads it
            //      from the occurrence, so the constant's provenance must not be lost here.
            let provenance = const_values
                .metadata(default)
                .map(|metadata| metadata.synthetic_interface_provenance.clone())
                .unwrap_or_default();
            let reference = match rebuilt {
                Some(template) => {
                    expression_builder.template(template, ValueMode::ImmutableOwned)?
                }
                None => expression_builder.constant(
                    default,
                    reference_arg.type_id,
                    base_span,
                    ValueMode::ImmutableOwned,
                )?,
            };
            return Ok(
                expression_builder.with_synthetic_interface_provenance(reference, provenance)?
            );
        }
    }

    Ok(expression_builder.reference_with_type_id(
        reference_arg.id,
        reference_arg.type_id,
        base_span,
        if context.kind.is_constant_context() {
            ValueMode::ImmutableOwned
        } else {
            reference_arg.value_mode
        },
        const_record_state,
    )?)
}

fn receiver_reference_node(
    reference_arg: &Declaration,
    context: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    base_span: Option<SourceSpan>,
) -> Result<AstNode, ExpressionParseError> {
    Ok(AstNode {
        kind: NodeKind::ExpressionStatement(
            reference_expression_from_declaration(
                reference_arg,
                context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                base_span,
            )?
            .id(),
        ),
        scope: context.scope.to_owned(),
        span: base_span,
    })
}

fn receiver_node_type_id(
    node: &AstNode,
    expression_builder: &AstExpressionBuilder,
) -> Result<TypeId, ExpressionParseError> {
    let NodeKind::ExpressionStatement(expression) = &node.kind else {
        crate::compiler_bug!(
            CompilerStage::AstConstruction,
            "postfix receiver must be an expression statement",
        );
    };
    Ok(expression_builder
        .view(*expression)?
        .require_single(ValueReceiverContext::ReceiverBase)?)
}

pub(super) fn type_id_is_external(type_id: TypeId, type_interner: &AstTypeInterner<'_>) -> bool {
    matches!(
        type_interner.environment().get(type_id),
        Some(TypeDefinition::External(..))
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "postfix chain parsing keeps the token stream, receiver expression and span, chain access, boundary catch permission, scope, and mutable interner/string/path state as separate borrows"
)]
pub(crate) fn parse_postfix_chain_expression(
    token_stream: &mut AstCursor<'_>,
    receiver_expression: OwnedExpr,
    receiver_span: Option<SourceSpan>,
    chain_access: PostfixChainAccess,
    allow_boundary_catch: bool,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedPostfixExpression, ExpressionParseError> {
    expression_builder.view_owned(receiver_expression)?;
    let receiver_node = AstNode {
        kind: NodeKind::ExpressionStatement(receiver_expression.id()),
        scope: context.scope.to_owned(),
        span: receiver_span,
    };

    let postfix = parse_postfix_chain_typed(
        token_stream,
        receiver_node,
        chain_access,
        allow_boundary_catch,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )?;

    match postfix {
        ParsedPostfixChain::Node(node) => expression_from_postfix_node(&node)
            .map(|expression| ParsedPostfixExpression::Value(expression_builder.own(expression))),
        ParsedPostfixChain::RetainedOperand(item) => {
            Ok(ParsedPostfixExpression::RetainedOperand(item))
        }
    }
}

pub(crate) fn expression_from_postfix_node(
    postfix_node: &AstNode,
) -> Result<ExprId, ExpressionParseError> {
    match &postfix_node.kind {
        NodeKind::ExpressionStatement(expression) => Ok(*expression),

        unexpected_kind => crate::compiler_bug!(
            CompilerStage::AstConstruction,
            "Expected postfix expression node, found {unexpected_kind:?}",
        ),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "postfix chain parsing keeps the token stream, receiver node, chain access, boundary catch permission, scope, and mutable interner/string/path state as separate borrows"
)]
fn parse_postfix_chain_typed(
    token_stream: &mut AstCursor<'_>,
    mut receiver_node: AstNode,
    chain_access: PostfixChainAccess,
    allow_boundary_catch: bool,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedPostfixChain, ExpressionParseError> {
    let receiver_access_mode = chain_access.mode;
    let authored_marker_span = chain_access.authored_marker_span;
    let mut encountered_receiver_call = false;

    // ----------------------------
    //  Walk postfix member-access chain
    // ----------------------------
    while token_stream.position() < token_stream.length()
        && token_stream.current_tag() == TokenTag::DOT
    {
        token_stream.advance();

        // An access that ends immediately after the authored dot has no member name. The dot
        // we just consumed is the owning boundary: point at it directly so EOF (or a stream
        // with no trailing Eof token) never falls back to the receiver start or a default span.
        // Non-EOF missing-member boundaries keep their offending-token span through
        // `parse_member_name_typed` below.
        if token_stream.position() >= token_stream.length()
            || token_stream.current_tag() == TokenTag::EOF
        {
            let dot_span = previous_token_span(token_stream);
            return Err(calls_data::invalid_field_access(
                InvalidFieldAccessReason::ExpectedNameAfterDot,
                None,
                None,
                Vec::new(),
                dot_span,
            )
            .into());
        }

        let member_name = parse_member_name_typed(token_stream, &mut *string_table)?;
        let receiver_type_id = receiver_node_type_id(&receiver_node, expression_builder)?;
        let member_span = Some(token_stream.current_span());
        let member_context = MemberStepContext {
            receiver_node: &receiver_node,
            receiver_type_id,
            member_name,
            member_span,
            receiver_access_mode,
            authored_marker_span,
            scope_context: context,
        };

        if let Some(field_access) = parse_field_member_access_typed(
            token_stream,
            member_context.to_owned(),
            type_interner,
            expression_builder,
            const_values,
            path_fork,
        )? {
            receiver_node = field_access;
            continue;
        }

        if let Some(collection_builtin_call) = parse_collection_builtin_member_typed(
            token_stream,
            member_context.to_owned(),
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        )? {
            match collection_builtin_call {
                ParsedBuiltinMemberCall::Completed(node) => {
                    receiver_node = node;
                    encountered_receiver_call = true;
                    continue;
                }
                // The operation and its result slots are already resolved, so every authored step
                // that follows resolves against known receiver structure and travels with the
                // operand, exactly as a retained receiver method does.
                ParsedBuiltinMemberCall::RetainedOperand(item) => {
                    let item = continue_retained_operand(
                        item,
                        token_stream,
                        context,
                        chain_access,
                        type_interner,
                        expression_builder,
                        const_values,
                        string_table,
                        path_fork,
                    )?;
                    return Ok(ParsedPostfixChain::RetainedOperand(item));
                }
            }
        }

        if let Some(map_builtin_call) = parse_map_builtin_member_typed(
            token_stream,
            member_context.to_owned(),
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        )? {
            match map_builtin_call {
                ParsedBuiltinMemberCall::Completed(node) => {
                    receiver_node = node;
                    encountered_receiver_call = true;
                    continue;
                }
                ParsedBuiltinMemberCall::RetainedOperand(item) => {
                    let item = continue_retained_operand(
                        item,
                        token_stream,
                        context,
                        chain_access,
                        type_interner,
                        expression_builder,
                        const_values,
                        string_table,
                        path_fork,
                    )?;
                    return Ok(ParsedPostfixChain::RetainedOperand(item));
                }
            }
        }

        if let Some(receiver_method_call) = parse_receiver_method_call_typed(
            token_stream,
            member_context,
            allow_boundary_catch,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        )? {
            match receiver_method_call {
                ParsedReceiverMethodCall::Value(node) => {
                    receiver_node = node;
                    encountered_receiver_call = true;
                    continue;
                }
                // The call's declared success slots are already concrete, so every authored step
                // that follows resolves against known receiver structure and travels with the
                // operand; only a receiver with no known result shape keeps the ordinary member
                // diagnostic at its authored dot.
                ParsedReceiverMethodCall::RetainedOperand(item) => {
                    let item = continue_retained_operand(
                        item,
                        token_stream,
                        context,
                        chain_access,
                        type_interner,
                        expression_builder,
                        const_values,
                        string_table,
                        path_fork,
                    )?;
                    return Ok(ParsedPostfixChain::RetainedOperand(item));
                }
            }
        }

        // No handler matched. Preserve the user-facing distinction between
        // deferred choice payload access, opaque externals, and ordinary
        let next_token = next_token_tag(token_stream);
        let reason = if type_interner
            .environment()
            .variants_for(receiver_type_id)
            .is_some()
            && next_token != Some(TokenTag::OPEN_PARENTHESIS)
        {
            if next_token_is_assignment_operator(next_token) {
                InvalidFieldAccessReason::ChoicePayloadMutation
            } else {
                InvalidFieldAccessReason::ChoicePayloadDeferred
            }
        } else if type_id_is_external(receiver_type_id, type_interner) {
            InvalidFieldAccessReason::UnknownExternalMember
        } else {
            InvalidFieldAccessReason::UnknownMember
        };

        let known_fields = collect_known_member_names(receiver_type_id, type_interner, path_fork);
        return Err(calls_data::invalid_field_access(
            reason,
            Some(member_name),
            Some(receiver_type_id),
            known_fields,
            member_span,
        )
        .into());
    }

    // ----------------------------
    //  Validate assignment receiver is a place
    // ----------------------------
    if token_stream.current_tag().is_assignment_operator() {
        let receiver_expression = expression_from_postfix_node(&receiver_node)?;
        if matches!(
            classify_expression_source_state(
                expression_builder.view(receiver_expression)?,
                path_fork
            ),
            ReceiverSourceState::Temporary
        ) {
            expression_builder
                .view(receiver_expression)?
                .require_single(ValueReceiverContext::ReceiverBase)?;
            let diagnostic = names_declarations::invalid_assignment_target(
                InvalidAssignmentTargetReason::TemporaryNotAssignable,
                None,
                None,
                None,
                receiver_node.span,
            );
            return Err(diagnostic.into());
        }
    }

    if receiver_access_mode == ReceiverAccessMode::Mutable && !encountered_receiver_call {
        // The authored `~` marker is the source the author must change when no receiver call
        // followed it. Fall back to the receiver boundary only when the marker was not threaded
        // through this chain entry.
        let marker_span = authored_marker_span.or(receiver_node.span);
        // When the chain is followed by an assignment operator, the author wrote `~place = ...`
        // and intended an assignment target, not a receiver call.
        if token_stream.current_tag().is_assignment_operator() {
            return Err(names_declarations::invalid_assignment_target(
                InvalidAssignmentTargetReason::MutableMarkerOnAssignmentTarget,
                None,
                None,
                None,
                marker_span,
            )
            .into());
        }
        return Err(calls_data::invalid_receiver_call(
            InvalidReceiverCallReason::MutableMarkerOnNonReceiverCall,
            None,
            None,
            None,
            None,
            marker_span,
        )
        .into());
    }

    Ok(ParsedPostfixChain::Node(receiver_node))
}

/// Continue a retained operand with the authored member steps that follow it.
///
/// WHAT: when the cursor sits on a dot, collects the authored steps through the retained-operand
///       collector and returns the wrapper for that extended operand.
/// WHY: every retained call — receiver method or compiler-owned builtin — keeps the same
///      continuation policy at the chain boundary, so the operand's authored steps stay attached to
///      the position whose value they resolve against.
#[allow(
    clippy::too_many_arguments,
    reason = "retained operand continuation keeps the item, token stream, scope, chain access, type interner, expression builder, const store, string table, and path fork as separate borrows"
)]
fn continue_retained_operand(
    item: ExpressionRpnItem,
    token_stream: &mut AstCursor<'_>,
    context: &ScopeContext,
    chain_access: PostfixChainAccess,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ExpressionRpnItem, ExpressionParseError> {
    if token_stream.position() >= token_stream.length()
        || token_stream.current_tag() != TokenTag::DOT
    {
        return Ok(item);
    }

    let mut expression = vec![item];
    super::retained_postfix::retain_pending_operand_member_access(
        token_stream,
        &mut expression,
        super::retained_postfix::RetainedOperandInput {
            chain_access,
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
    Ok(expression
        .pop()
        .expect("a retained operand keeps its authored position"))
}

/// Parses a postfix/member chain rooted at one resolved declaration.
///
/// WHAT: returns the completed expression, or the pending receiver-call item the chain ended in.
/// WHY: a call whose numeric-dependent argument was retained has no value yet; the statement and
///      mutation owners decide from the exclusive outcome whether they have a value, a mutation
///      target, or one deferred expression-statement position to record.
#[allow(
    clippy::too_many_arguments,
    reason = "field-access parsing keeps the token stream, base declaration, scope, type interner, expression builder, const store, string table, and path fork as separate borrows at the shared postfix entry point"
)]
pub fn parse_field_access(
    token_stream: &mut AstCursor<'_>,
    base_arg: &Declaration,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedPostfixExpression, ExpressionParseError> {
    // Operand errors stay pending until the receiving chain has completed. Assignment targets
    // still leave their operator for the mutation owner instead of entering expression completion.
    let deferred_context = (!context.defer_typed_error_validation)
        .then(|| context.new_pending_typed_error_expression_context());
    let parse_context = deferred_context.as_ref().unwrap_or(context);

    let postfix = parse_field_access_with_receiver_access(
        token_stream,
        base_arg,
        parse_context,
        PostfixChainAccess::shared(),
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )?;

    let postfix_node = match postfix {
        ParsedPostfixChain::Node(node) => node,
        ParsedPostfixChain::RetainedOperand(item) => {
            return Ok(ParsedPostfixExpression::RetainedOperand(item));
        }
    };

    let expression = expression_builder.own(expression_from_postfix_node(&postfix_node)?);
    if token_stream.current_tag().is_assignment_operator() {
        return Ok(ParsedPostfixExpression::Value(expression));
    }
    complete_pending_expression(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        expression,
        true,
        string_table,
        path_fork,
    )
    .map(ParsedPostfixExpression::Value)
}

#[allow(
    clippy::too_many_arguments,
    reason = "receiver access chain parsing keeps the token stream, base declaration, scope, chain access, type interner, expression builder, const store, string table, and path fork as separate borrows"
)]
fn parse_field_access_with_receiver_access(
    token_stream: &mut AstCursor<'_>,
    base_arg: &Declaration,
    context: &ScopeContext,
    chain_access: PostfixChainAccess,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedPostfixChain, ExpressionParseError> {
    let base_span = previous_token_span(token_stream).or_else(|| Some(token_stream.current_span()));

    // `parse_field_access` completes the chain at the expression-statement boundary, which allows
    // an authored boundary catch there.
    parse_postfix_chain_typed(
        token_stream,
        receiver_reference_node(
            base_arg,
            context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            base_span,
        )?,
        chain_access,
        true,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "entry keeps the token stream, base declaration, scope, chain access, boundary catch permission, and mutable interner/string/path state as separate borrows"
)]
pub(crate) fn parse_field_access_expression_with_receiver_access(
    token_stream: &mut AstCursor<'_>,
    base_arg: &Declaration,
    context: &ScopeContext,
    chain_access: PostfixChainAccess,
    allow_boundary_catch: bool,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedPostfixExpression, ExpressionParseError> {
    let base_span = previous_token_span(token_stream).or_else(|| Some(token_stream.current_span()));

    let receiver_expression = reference_expression_from_declaration(
        base_arg,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        base_span,
    )?;

    parse_postfix_chain_expression(
        token_stream,
        receiver_expression,
        base_span,
        chain_access,
        allow_boundary_catch,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )
}

/// Report the ordinary member diagnostic for a pending numeric receiver.
///
/// WHAT: consumes the authored member name after the dot and returns the ordinary
///       unknown-member diagnostic with no receiver type.
/// WHY: a pending numeric binding's selected type is always a scalar, so no member can exist on
///      it, and member resolution must never search fields, methods or trait surfaces against a
///      type that does not exist yet. The chain driver owns member-name parsing and the ordinary
///      member diagnostic, so both stay here instead of becoming a second member-name convention
///      in the expression files.
///
/// PRECONDITION: the cursor is positioned on the authored dot.
pub(crate) fn pending_numeric_member_access_error(
    token_stream: &mut AstCursor<'_>,
    string_table: &mut StringTable,
) -> ExpressionParseError {
    if token_stream.position() < token_stream.length()
        && token_stream.current_tag() == TokenTag::DOT
    {
        token_stream.advance();

        // An access that ends immediately after the authored dot has no member name. The dot is
        // the owning boundary, exactly like the concrete-receiver path above.
        if token_stream.position() >= token_stream.length()
            || token_stream.current_tag() == TokenTag::EOF
        {
            let dot_span = previous_token_span(token_stream);
            return calls_data::invalid_field_access(
                InvalidFieldAccessReason::ExpectedNameAfterDot,
                None,
                None,
                Vec::new(),
                dot_span,
            )
            .into();
        }

        let member_span = Some(token_stream.current_span());
        return match parse_member_name_typed(token_stream, string_table) {
            Ok(member_name) => calls_data::invalid_field_access(
                InvalidFieldAccessReason::UnknownMember,
                Some(member_name),
                None,
                Vec::new(),
                member_span,
            )
            .into(),
            Err(error) => error,
        };
    }

    crate::compiler_bug!(
        CompilerStage::AstConstruction,
        "a pending numeric receiver entered member parsing without an authored dot",
    )
}

/// Collect known struct field names and choice variant names for a receiver type.
///
/// WHAT: gathers the member names that are valid on the receiver so the diagnostic renderer
/// can offer a "did you mean?" suggestion when a name is misspelled.
/// WHY: the most helpful thing a compiler can do for a typo'd field access is suggest the
/// correct field name. The type environment already has this information.
pub(super) fn collect_known_member_names(
    receiver_type_id: TypeId,
    type_interner: &AstTypeInterner<'_>,
    path_fork: &PathInternerFork,
) -> Vec<StringId> {
    let environment = type_interner.environment();
    let mut names = Vec::new();

    // Struct field names are stored as PathId; the field name is the last
    // path component, not the first (which may be a source-file prefix).
    if let Some(fields) = environment.fields_for(receiver_type_id) {
        for field in fields {
            if let Some(name) = path_fork.component(field.name) {
                names.push(name);
            }
        }
    }

    // Choice variant names are already plain StringId values.
    if let Some(variants) = environment.variants_for(receiver_type_id) {
        for variant in variants {
            names.push(variant.name);
        }
    }

    names
}
