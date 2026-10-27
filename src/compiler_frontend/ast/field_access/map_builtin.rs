//! Map builtin receiver-member parsing.
//!
//! WHAT: parses compiler-owned map members (`get/contains/set/remove/clear/length`).
//! WHY: map builtin policy should stay separate from user field/method dispatch.

use super::MemberStepContext;
use super::ParsedBuiltinMemberCall;
use super::builtin_call_args::{
    BuiltinCallSite, builtin_argument_expectations, builtin_argument_shapes,
    builtin_argument_slots, builtin_result_type_ids, intern_concrete_shape,
    resolve_builtin_member_arguments,
};
use super::parse_chain::expression_from_postfix_node;
use super::receiver_access::{
    ReceiverAccessDiagnostic, ReceiverAccessRequirement, validate_receiver_access,
};
use super::receiver_calls::finish_pending_receiver_call_expression;
use super::retained_postfix::{RetainedBuiltinKind, RetainedBuiltinStep, retained_builtin_operand};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::call_arguments::{
    CallArgumentSyntax, ParsedCallArguments, parse_call_arguments_typed_with_numeric_retention,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::numeric_construction::DeferredCallHandling;
use crate::compiler_frontend::ast::numeric_known_shape::{KnownShape, KnownShapeId};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::builtins::error_type::resolve_builtin_error_type_typed;
use crate::compiler_frontend::builtins::maps::MapBuiltinOp;
use crate::compiler_frontend::compiler_messages::InvalidAssignmentTargetReason;
use crate::compiler_frontend::compiler_messages::InvalidBuiltinCallReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

// --------------------------
//  Main parser
// --------------------------

/// One map builtin member's argument slots, success slot and declared error.
///
/// WHAT: the authored argument slots the shared argument owner validates against and the success
///       slot an authored `!` propagates, both as construction shapes, plus the declared error.
/// WHY: a retained receiver can hold an open numeric key or value, so the slots stay shapes until
///      body-root completion materialises them; the completed and retained lanes resolve the same
///      member through this one rule.
pub(super) struct MapBuiltinMemberRule {
    /// The authored argument slots in order: the key, then the value for `set`.
    pub(super) expected_argument_shapes: Vec<KnownShapeId>,
    /// The separate success slot an authored `!` propagates. `None` when the operation
    /// produces no value.
    pub(super) success_shape: Option<KnownShapeId>,
    /// The separate declared error channel an authored `!` propagates; `None` if infallible.
    pub(super) error_return_type_id: Option<TypeId>,
}

/// One resolved map builtin member, independent of where its receiver value comes from.
pub(super) struct MapBuiltinMember {
    pub(super) op: MapBuiltinOp,
    pub(super) rule: MapBuiltinMemberRule,
}

/// The receiver a map builtin resolves against.
///
/// WHAT: a completed receiver is resolved by its canonical type; a retained receiver is resolved
///       by its construction shape, whose key or value may still be an open numeric leaf.
/// WHY: one resolver owns the per-operation rule for both lanes, so no lane resolves a member
///      through a placeholder type.
pub(super) enum MapBuiltinReceiver {
    Concrete(TypeId),
    Shape(KnownShapeId),
}

/// Resolve one map builtin member against a receiver without consuming any tokens.
///
/// WHAT: classifies the receiver, resolves the authored member name and computes the operation's
///       argument slots and success slot. Non-map receivers resolve to `None` and intern nothing.
/// WHY: a retained chain has no receiver value, so it resolves the exact member the completed
///      chain would, then parses the same argument syntax through the shared owner.
pub(super) fn resolve_map_builtin_member(
    receiver: MapBuiltinReceiver,
    member_name: StringId,
    member_span: Option<SourceSpan>,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<Option<MapBuiltinMember>, ExpressionParseError> {
    let receiver_shape = match receiver {
        MapBuiltinReceiver::Concrete(type_id) => KnownShape::Concrete {
            type_id,
            span: None,
        },
        MapBuiltinReceiver::Shape(shape) => scope_context.numeric_state().shape(shape),
    };
    let (key_shape, value_shape) = match receiver_shape {
        KnownShape::Map { key, value } => (key, value),
        KnownShape::Concrete { type_id, .. } => {
            let Some(map_shape) = type_interner.environment().map_shape(type_id) else {
                return Ok(None);
            };
            let key_shape = intern_concrete_shape(scope_context, map_shape.key_type)?;
            let value_shape = intern_concrete_shape(scope_context, map_shape.value_type)?;
            (key_shape, value_shape)
        }
        KnownShape::Numeric(_)
        | KnownShape::Collection { .. }
        | KnownShape::Optional { .. }
        | KnownShape::DeclaredSubstitution { .. } => return Ok(None),
    };

    let Some(op) = MapBuiltinOp::from_source_name(string_table.resolve(member_name)) else {
        return Ok(None);
    };

    let bool_shape = intern_concrete_shape(scope_context, type_interner.builtins().bool)?;
    let int_shape = intern_concrete_shape(
        scope_context,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    )?;

    let semantic_rule = op.rule(key_shape, value_shape, bool_shape, int_shape);
    let error_return_type_id = if semantic_rule.fallible {
        Some(resolve_builtin_error_type_typed(scope_context, member_span, string_table)?.type_id)
    } else {
        None
    };
    let rule = MapBuiltinMemberRule {
        expected_argument_shapes: builtin_argument_shapes(semantic_rule.arguments),
        success_shape: semantic_rule.success,
        error_return_type_id,
    };

    Ok(Some(MapBuiltinMember { op, rule }))
}

/// Parse one map builtin member step on a completed receiver node.
///
/// WHAT: validates receiver access, parses the operation's arguments through the shared argument
///       owner and either builds the builtin call or keeps the member as one retained step.
/// WHY: the operation, the receiver shape and the result slots are already resolved when an
///      argument turns out to still hold a numeric origin, so the member step must travel with the
///      retained argument and resume through this same constructor instead of ending the chain.
pub(super) fn parse_map_builtin_member_typed(
    token_stream: &mut AstCursor<'_>,
    context: MemberStepContext<'_>,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Option<ParsedBuiltinMemberCall>, ExpressionParseError> {
    let MemberStepContext {
        receiver_node,
        receiver_type_id,
        member_name,
        member_span,
        receiver_access_mode,
        authored_marker_span,
        scope_context,
    } = context;

    let Some(member) = resolve_map_builtin_member(
        MapBuiltinReceiver::Concrete(receiver_type_id),
        member_name,
        member_span,
        scope_context,
        type_interner,
        string_table,
    )?
    else {
        return Ok(None);
    };

    // Property-style `length` requires no parentheses.
    if member.op.is_property() {
        if token_stream.peek_next_tag() == Some(TokenTag::OPEN_PARENTHESIS) {
            return Err(calls_data::invalid_builtin_call(
                InvalidBuiltinCallReason::MapLengthIsProperty,
                Some(member_name),
                member_span,
            )
            .into());
        }

        validate_receiver_access(
            receiver_node,
            expression_builder,
            path_fork,
            receiver_access_mode,
            member_span,
            authored_marker_span,
            ReceiverAccessRequirement {
                requires_mutable: false,
                diagnostic: ReceiverAccessDiagnostic::MapBuiltin {
                    method_name: member_name,
                },
            },
        )?;

        token_stream.advance();

        // `length` is a property, not a call, so there are no arguments to parse.
        // Reject assignment through `map.length`.
        if token_stream.current_tag().is_assignment_operator() {
            return Err(names_declarations::invalid_assignment_target(
                InvalidAssignmentTargetReason::ReadOnlyMapProperty,
                None,
                None,
                None,
                Some(token_stream.current_span()),
            )
            .into());
        }

        let result_type_ids =
            builtin_result_type_ids(member.rule.success_shape, scope_context, type_interner)?;
        let receiver_expression = expression_from_postfix_node(receiver_node)?;
        let builtin_expression = expression_builder.map_builtin_call_with_typed_arguments(
            expression_builder.own(receiver_expression),
            member.op,
            false,
            Vec::new(),
            result_type_ids,
            member_span,
        )?;

        return Ok(Some(ParsedBuiltinMemberCall::Completed(AstNode {
            kind: NodeKind::ExpressionStatement(builtin_expression.id()),
            scope: scope_context.scope.to_owned(),
            span: member_span,
        })));
    }

    // All other map builtins require parentheses.
    if token_stream.peek_next_tag() != Some(TokenTag::OPEN_PARENTHESIS) {
        return Err(calls_data::invalid_builtin_call(
            InvalidBuiltinCallReason::MissingParentheses,
            Some(member_name),
            member_span,
        )
        .into());
    }

    let mutating_receiver_required = member.op.requires_mutable_receiver();

    validate_receiver_access(
        receiver_node,
        expression_builder,
        path_fork,
        receiver_access_mode,
        member_span,
        authored_marker_span,
        ReceiverAccessRequirement {
            requires_mutable: mutating_receiver_required,
            diagnostic: ReceiverAccessDiagnostic::MapBuiltin {
                method_name: member_name,
            },
        },
    )?;

    token_stream.advance();

    let expectations = builtin_argument_expectations(builtin_argument_slots(
        scope_context,
        &member.rule.expected_argument_shapes,
    ));
    let parsed_arguments = parse_call_arguments_typed_with_numeric_retention(
        token_stream,
        scope_context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        &expectations,
        CallArgumentSyntax::UnsupportedBuiltinMember {
            member_name: Some(member_name),
            takes_no_arguments: member.rule.expected_argument_shapes.is_empty(),
        },
        path_fork,
    )?;

    // Reject assignment through `map.get(...)`.
    // The completed lane validates the parsed arguments through the same shared owner the
    // retained lane resumes at completion, so no lane skips the operation's arity, access-mode or
    // type-compatibility contract, and the validation keeps its authored order before the
    // assignment-target guard below.
    let parsed_arguments = match parsed_arguments {
        ParsedCallArguments::Ready(arguments) => {
            ParsedCallArguments::Ready(resolve_builtin_member_arguments(
                BuiltinCallSite {
                    member_name: member.op.source_name(),
                    member_span,
                },
                arguments,
                &expectations,
                scope_context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            )?)
        }
        retained => retained,
    };

    if matches!(member.op, MapBuiltinOp::Get) && token_stream.current_tag().is_assignment_operator()
    {
        return Err(names_declarations::invalid_assignment_target(
            InvalidAssignmentTargetReason::MapGetTargetNotWritable,
            None,
            None,
            None,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let arguments = match parsed_arguments {
        ParsedCallArguments::Ready(arguments) => arguments,
        // The operation and its result slots are already resolved, so the member step travels with
        // the retained argument and resumes through this same constructor once it completes.
        ParsedCallArguments::Retained(arguments) => {
            return Ok(Some(ParsedBuiltinMemberCall::RetainedOperand(
                retained_builtin_operand(
                    expression_from_postfix_node(receiver_node)?,
                    RetainedBuiltinStep {
                        kind: RetainedBuiltinKind::Map(member.op),
                        member_name,
                        member_span,
                        arguments: ParsedCallArguments::Retained(arguments),
                        expected_argument_shapes: member.rule.expected_argument_shapes,
                        success_shape: member.rule.success_shape,
                        error_return_type_id: member.rule.error_return_type_id,
                        authored_handling: DeferredCallHandling::Unhandled,
                    },
                    expression_builder,
                )?,
            )));
        }
    };

    let receiver_expression = expression_from_postfix_node(receiver_node)?;
    let result_type_ids =
        builtin_result_type_ids(member.rule.success_shape, scope_context, type_interner)?;
    let builtin_expression = expression_builder.map_builtin_call_with_typed_arguments(
        expression_builder.own(receiver_expression),
        member.op,
        mutating_receiver_required,
        arguments,
        result_type_ids,
        member_span,
    )?;
    let builtin_expression = match member.rule.error_return_type_id {
        Some(error_type) => expression_builder.with_typed_error_producer(
            builtin_expression,
            error_type,
            Some(member_name),
        )?,
        None => builtin_expression,
    };
    let builtin_expression = finish_pending_receiver_call_expression(
        builtin_expression,
        member_name,
        token_stream,
        type_interner,
        expression_builder,
    )?;

    Ok(Some(ParsedBuiltinMemberCall::Completed(AstNode {
        kind: NodeKind::ExpressionStatement(builtin_expression.id()),
        scope: scope_context.scope.to_owned(),
        span: member_span,
    })))
}
