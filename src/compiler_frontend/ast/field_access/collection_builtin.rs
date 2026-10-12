//! Collection builtin receiver-member parsing.
//!
//! WHAT: parses compiler-owned collection members (`get/set/push/remove/length`). Source `push`
//! resolves to the growable or fixed identity from the receiver's canonical collection shape.
//! WHY: collection builtin policy should stay separate from user field/method dispatch.

use super::MemberStepContext;
use super::ParsedBuiltinMemberCall;
use super::builtin_call_args::{
    BuiltinCallSite, builtin_argument_expectations, builtin_argument_slots,
    builtin_result_type_ids, concrete_shape_type, intern_concrete_shape,
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
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::call_arguments::{
    CallArgumentSyntax, ParsedCallArguments, parse_call_arguments_typed_with_numeric_retention,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::numeric_construction::DeferredCallHandling;
use crate::compiler_frontend::ast::numeric_known_shape::{KnownShape, KnownShapeId};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::builtins::error_type::resolve_builtin_error_type_typed;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidAssignmentTargetReason, InvalidBuiltinCallReason,
};
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

// --------------------------
//  Constants
// --------------------------

const COLLECTION_GET_NAME: &str = "get";
const COLLECTION_SET_NAME: &str = "set";
const COLLECTION_PUSH_NAME: &str = "push";
const COLLECTION_REMOVE_NAME: &str = "remove";
const COLLECTION_LENGTH_NAME: &str = "length";

// --------------------------
//  Helpers
// --------------------------

fn resolve_collection_builtin_op(
    member_name: StringId,
    string_table: &StringTable,
    fixed_capacity: Option<usize>,
) -> Option<CollectionBuiltinOp> {
    match string_table.resolve(member_name) {
        COLLECTION_GET_NAME => Some(CollectionBuiltinOp::Get),
        COLLECTION_SET_NAME => Some(CollectionBuiltinOp::Set),
        // One source spelling `push`: the canonical receiver shape picks the identity.
        COLLECTION_PUSH_NAME if fixed_capacity.is_some() => Some(CollectionBuiltinOp::PushFixed),
        COLLECTION_PUSH_NAME => Some(CollectionBuiltinOp::PushGrowable),
        COLLECTION_REMOVE_NAME => Some(CollectionBuiltinOp::Remove),
        COLLECTION_LENGTH_NAME => Some(CollectionBuiltinOp::Length),
        _ => None,
    }
}

/// The canonical source name of one resolved collection operation.
///
/// WHAT: the same constant `resolve_collection_builtin_op` matched, so the name owner stays one.
/// WHY: the builtin-member diagnostic surface takes a `&str`; the resolved operation already fixes
///      the recognized spelling (source `push` picks one identity), so neither the authored-name
///      resolution nor an allocation is needed.
pub(super) fn collection_builtin_op_source_name(op: CollectionBuiltinOp) -> &'static str {
    match op {
        CollectionBuiltinOp::Get => COLLECTION_GET_NAME,
        CollectionBuiltinOp::Set => COLLECTION_SET_NAME,
        CollectionBuiltinOp::PushGrowable | CollectionBuiltinOp::PushFixed => COLLECTION_PUSH_NAME,
        CollectionBuiltinOp::Remove => COLLECTION_REMOVE_NAME,
        CollectionBuiltinOp::Length => COLLECTION_LENGTH_NAME,
    }
}

/// One collection builtin member's argument slots and success slot.
///
/// WHAT: the argument slots the shared argument owner validates against and the success slot an
///       authored `!` propagates, both as construction shapes, plus the declared error.
/// WHY: a retained receiver can hold an open numeric element, so the slots stay shapes until
///      body-root completion materialises them; the completed and retained lanes resolve the same
///      member through this one rule.
pub(super) struct CollectionBuiltinMemberRule {
    /// The authored argument slots in order.
    pub(super) expected_argument_shapes: Vec<KnownShapeId>,
    /// The success slot an authored `!` propagates, before the carrier wrap. `None` when the
    /// operation produces no value.
    pub(super) success_shape: Option<KnownShapeId>,
    /// The declared error an authored `!` unwraps; `None` for an infallible operation.
    pub(super) error_return_type_id: Option<TypeId>,
}

/// One resolved collection builtin member, independent of where its receiver value comes from.
pub(super) struct CollectionBuiltinMember {
    pub(super) op: CollectionBuiltinOp,
    /// The receiver's element slot, shared by the operation's argument and success slots.
    pub(super) element_shape: KnownShapeId,
    pub(super) rule: CollectionBuiltinMemberRule,
}

/// The receiver a collection builtin resolves against.
///
/// WHAT: a completed receiver is resolved by its canonical type; a retained receiver is resolved
///       by its construction shape, whose element may still be an open numeric leaf.
/// WHY: one resolver owns the per-operation rule for both lanes, so no lane resolves a member
///      through a placeholder type.
pub(super) enum CollectionBuiltinReceiver {
    Concrete(TypeId),
    Shape(KnownShapeId),
}

/// Resolve one collection builtin member against a receiver without consuming any tokens.
///
/// WHAT: classifies the receiver, resolves the authored member name and computes the operation's
///       argument slots and success slot. Non-collection receivers resolve to `None` and intern
///       nothing.
/// WHY: a retained chain has no receiver value, so it resolves the exact member the completed
///      chain would, then parses the same argument syntax through the shared owner.
pub(super) fn resolve_collection_builtin_member(
    receiver: CollectionBuiltinReceiver,
    member_name: StringId,
    member_span: Option<SourceSpan>,
    scope_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<Option<CollectionBuiltinMember>, ExpressionParseError> {
    let receiver_shape = match receiver {
        CollectionBuiltinReceiver::Concrete(type_id) => KnownShape::Concrete {
            type_id,
            span: None,
        },
        CollectionBuiltinReceiver::Shape(shape) => scope_context.numeric_state().shape(shape),
    };
    let (element_shape, fixed_capacity) = match receiver_shape {
        KnownShape::Collection {
            element,
            fixed_capacity,
        } => (element, fixed_capacity),
        KnownShape::Concrete { type_id, .. } => {
            let Some(collection_shape) = type_interner.environment().collection_shape(type_id)
            else {
                return Ok(None);
            };
            let element_shape =
                intern_concrete_shape(scope_context, collection_shape.element_type)?;
            (element_shape, collection_shape.fixed_capacity)
        }
        KnownShape::Numeric(_)
        | KnownShape::Map { .. }
        | KnownShape::Optional { .. }
        | KnownShape::DeclaredSubstitution { .. } => return Ok(None),
    };

    let Some(op) = resolve_collection_builtin_op(member_name, string_table, fixed_capacity) else {
        return Ok(None);
    };

    let int_shape = intern_concrete_shape(
        scope_context,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
    )?;

    let rule = match op {
        CollectionBuiltinOp::Get => {
            let error_type =
                resolve_builtin_error_type_typed(scope_context, member_span, string_table)?;
            CollectionBuiltinMemberRule {
                expected_argument_shapes: vec![int_shape],
                success_shape: Some(element_shape),
                error_return_type_id: Some(error_type.type_id),
            }
        }

        CollectionBuiltinOp::Set => {
            let error_type =
                resolve_builtin_error_type_typed(scope_context, member_span, string_table)?;
            CollectionBuiltinMemberRule {
                expected_argument_shapes: vec![int_shape, element_shape],
                success_shape: None,
                error_return_type_id: Some(error_type.type_id),
            }
        }

        // Growable push has no recoverable source-visible `Error!` path: it needs no error
        // resolution or fallible carrier and produces no result. A stray `catch`/`!` suffix is
        // rejected after the postfix chain by the shared non-fallible handling diagnostics.
        CollectionBuiltinOp::PushGrowable => CollectionBuiltinMemberRule {
            expected_argument_shapes: vec![element_shape],
            success_shape: None,
            error_return_type_id: None,
        },

        // Fixed push can fail recoverably at capacity, so it keeps the fallible carrier bridge.
        CollectionBuiltinOp::PushFixed => {
            let error_type =
                resolve_builtin_error_type_typed(scope_context, member_span, string_table)?;
            CollectionBuiltinMemberRule {
                expected_argument_shapes: vec![element_shape],
                success_shape: None,
                error_return_type_id: Some(error_type.type_id),
            }
        }

        CollectionBuiltinOp::Remove => {
            let error_type =
                resolve_builtin_error_type_typed(scope_context, member_span, string_table)?;
            CollectionBuiltinMemberRule {
                expected_argument_shapes: vec![int_shape],
                success_shape: Some(element_shape),
                error_return_type_id: Some(error_type.type_id),
            }
        }

        CollectionBuiltinOp::Length => CollectionBuiltinMemberRule {
            expected_argument_shapes: Vec::new(),
            success_shape: Some(int_shape),
            error_return_type_id: None,
        },
    };

    Ok(Some(CollectionBuiltinMember {
        op,
        element_shape,
        rule,
    }))
}

/// Parse one collection builtin member step on a completed receiver node.
///
/// WHAT: validates receiver access, parses the operation's arguments through the shared argument
///       owner and either builds the builtin call or keeps the member as one retained step.
/// WHY: the operation, the receiver shape and the result slots are already resolved when an
///      argument turns out to still hold a numeric origin, so the member step must travel with the
///      retained argument and resume through this same constructor instead of ending the chain.
pub(super) fn parse_collection_builtin_member_typed(
    token_stream: &mut AstCursor<'_>,
    context: MemberStepContext<'_>,
    type_interner: &mut AstTypeInterner<'_>,
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

    let Some(member) = resolve_collection_builtin_member(
        CollectionBuiltinReceiver::Concrete(receiver_type_id),
        member_name,
        member_span,
        scope_context,
        type_interner,
        string_table,
    )?
    else {
        return Ok(None);
    };
    // A completed receiver's element is always a canonical type, so the diagnostic keeps it.
    let element_type_id = concrete_shape_type(scope_context, member.element_shape);

    if token_stream.peek_next_tag() != Some(TokenTag::OPEN_PARENTHESIS) {
        return Err(CompilerDiagnostic::invalid_builtin_call(
            InvalidBuiltinCallReason::MissingParentheses,
            Some(member_name),
            member_span,
        )
        .into());
    }

    let mutating_receiver_required = member.op.requires_mutable_receiver();

    validate_receiver_access(
        receiver_node,
        path_fork,
        receiver_access_mode,
        member_span,
        authored_marker_span,
        ReceiverAccessRequirement {
            requires_mutable: mutating_receiver_required,
            diagnostic: ReceiverAccessDiagnostic::CollectionBuiltin {
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
        string_table,
        &expectations,
        CallArgumentSyntax::UnsupportedBuiltinMember {
            member_name: Some(member_name),
            takes_no_arguments: member.rule.expected_argument_shapes.is_empty(),
        },
        path_fork,
    )?;

    // The completed lane validates the parsed arguments through the same shared owner the
    // retained lane resumes at completion, so no lane skips the operation's arity, access-mode or
    // type-compatibility contract, and the validation keeps its authored order before the
    // assignment-target guard below.
    let parsed_arguments = match parsed_arguments {
        ParsedCallArguments::Ready(arguments) => {
            ParsedCallArguments::Ready(resolve_builtin_member_arguments(
                BuiltinCallSite {
                    member_name: collection_builtin_op_source_name(member.op),
                    member_span,
                },
                &arguments,
                &expectations,
                scope_context,
                type_interner,
                string_table,
                path_fork,
            )?)
        }
        retained => retained,
    };

    if matches!(member.op, CollectionBuiltinOp::Get)
        && token_stream.current_tag().is_assignment_operator()
    {
        return Err(CompilerDiagnostic::invalid_assignment_target(
            InvalidAssignmentTargetReason::CollectionGetTargetNotWritable,
            None,
            element_type_id,
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
            increment_ast_counter(AstCounter::PostfixReceiverNodesCopied);
            return Ok(Some(ParsedBuiltinMemberCall::RetainedOperand(
                retained_builtin_operand(
                    expression_from_postfix_node(receiver_node)?,
                    RetainedBuiltinStep {
                        kind: RetainedBuiltinKind::Collection(member.op),
                        member_name,
                        member_span,
                        arguments: ParsedCallArguments::Retained(arguments),
                        expected_argument_shapes: member.rule.expected_argument_shapes,
                        success_shape: member.rule.success_shape,
                        error_return_type_id: member.rule.error_return_type_id,
                        authored_handling: DeferredCallHandling::Unhandled,
                    },
                ),
            )));
        }
    };

    increment_ast_counter(AstCounter::PostfixReceiverNodesCopied);

    let receiver_expression = expression_from_postfix_node(receiver_node)?;
    let result_type_ids = builtin_result_type_ids(
        member.rule.success_shape,
        member.rule.error_return_type_id,
        scope_context,
        type_interner,
    )?;
    let builtin_expression = Expression::collection_builtin_call_with_typed_arguments(
        receiver_expression,
        member.op,
        arguments,
        result_type_ids,
        type_interner.environment_mut_for_derived_types(),
        member_span,
    );
    let builtin_expression = finish_pending_receiver_call_expression(
        builtin_expression,
        member_name,
        token_stream,
        type_interner,
    );

    Ok(Some(ParsedBuiltinMemberCall::Completed(AstNode {
        kind: NodeKind::ExpressionStatement(builtin_expression),
        scope: scope_context.scope.to_owned(),
        span: member_span,
    })))
}
