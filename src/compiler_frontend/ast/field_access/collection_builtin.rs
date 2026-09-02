//! Collection builtin receiver-member parsing.
//!
//! WHAT: parses compiler-owned collection members (`get/set/push/remove/length`). Source `push`
//! resolves to the growable or fixed identity from the receiver's canonical collection shape.
//! WHY: collection builtin policy should stay separate from user field/method dispatch.

use super::MemberStepContext;
use super::builtin_call_args::parse_builtin_method_args_typed;
use super::parse_chain::expression_from_postfix_node;
use super::receiver_access::{
    ReceiverAccessDiagnostic, ReceiverAccessRequirement, validate_receiver_access,
};
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::call_argument::normalize_call_arguments;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::fallible_handling::token_stream_starts_fallible_handling_suffix;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::builtins::error_type::{
    ResolvedBuiltinType, resolve_builtin_error_type_typed,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidAssignmentTargetReason, InvalidBuiltinCallReason,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

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

fn collection_builtin_method_name(
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

fn fallible_collection_result(
    ok_type_id: TypeId,
    error_type: ResolvedBuiltinType,
    type_interner: &mut AstTypeInterner<'_>,
) -> Vec<TypeId> {
    // Temporary carrier bridge: collection fallibility is public `Error!` control flow, not a
    // first-class `Result` value. HIR consumes this carrier immediately while lowering the
    // handled call into explicit success/error edges.
    vec![type_interner.intern_fallible_carrier(ok_type_id, error_type.type_id)]
}

// --------------------------
//  Main parser
// --------------------------

pub(super) fn parse_collection_builtin_member_typed(
    token_stream: &mut FileTokens,
    context: MemberStepContext<'_>,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<Option<AstNode>, ExpressionParseError> {
    let MemberStepContext {
        receiver_node,
        receiver_type_id,
        member_name,
        member_location,
        receiver_access_mode,
        authored_marker_location,
        scope_context,
    } = context;

    // One canonical shape query classifies the receiver: element type for arguments/results and
    // fixed capacity for the push identity. Transparent aliases resolve to the same canonical
    // shape, so they follow the same classification.
    let Some(collection_shape) = type_interner
        .environment()
        .collection_shape(receiver_type_id)
    else {
        return Ok(None);
    };
    let element_type_id = collection_shape.element_type;
    let fixed_capacity = collection_shape.fixed_capacity;

    let Some(builtin) = collection_builtin_method_name(member_name, string_table, fixed_capacity)
    else {
        return Ok(None);
    };
    let int_type_id = type_interner.builtins().int;
    let none_type_id = type_interner.builtins().none;
    let member_name_text = string_table.resolve(member_name).to_owned();

    if token_stream.peek_next_token() != Some(&TokenKind::OpenParenthesis) {
        return Err(CompilerDiagnostic::invalid_builtin_call(
            InvalidBuiltinCallReason::MissingParentheses,
            Some(member_name),
            member_location,
        )
        .into());
    }

    let mutating_receiver_required = builtin.requires_mutable_receiver();

    validate_receiver_access(
        receiver_node,
        receiver_access_mode,
        &member_location,
        authored_marker_location.as_ref(),
        ReceiverAccessRequirement {
            requires_mutable: mutating_receiver_required,
            diagnostic: ReceiverAccessDiagnostic::CollectionBuiltin {
                method_name: member_name,
            },
        },
    )?;

    token_stream.advance();

    let (args, result_type_ids) = match builtin {
        CollectionBuiltinOp::Get => {
            let expected_type_ids = [int_type_id];
            let args = parse_builtin_method_args_typed(
                token_stream,
                &member_name_text,
                &expected_type_ids,
                scope_context,
                type_interner,
                &member_location,
                string_table,
            )?;
            let error_type =
                resolve_builtin_error_type_typed(scope_context, &member_location, string_table)?;
            let result_type_ids =
                fallible_collection_result(element_type_id, error_type, type_interner);
            (args, result_type_ids)
        }

        CollectionBuiltinOp::Set => {
            let expected_type_ids = [int_type_id, element_type_id];
            let args = parse_builtin_method_args_typed(
                token_stream,
                &member_name_text,
                &expected_type_ids,
                scope_context,
                type_interner,
                &member_location,
                string_table,
            )?;
            let error_type =
                resolve_builtin_error_type_typed(scope_context, &member_location, string_table)?;
            let result_type_ids =
                fallible_collection_result(none_type_id, error_type, type_interner);
            (args, result_type_ids)
        }

        CollectionBuiltinOp::PushGrowable => {
            let expected_type_ids = [element_type_id];
            let args = parse_builtin_method_args_typed(
                token_stream,
                &member_name_text,
                &expected_type_ids,
                scope_context,
                type_interner,
                &member_location,
                string_table,
            )?;
            // Growable push is infallible: no error resolution and no fallible carrier, so the
            // call produces no result. A stray `catch`/`!` suffix is rejected after the postfix
            // chain by the shared non-fallible-handler diagnostics.
            (args, vec![])
        }

        CollectionBuiltinOp::PushFixed => {
            let expected_type_ids = [element_type_id];
            let args = parse_builtin_method_args_typed(
                token_stream,
                &member_name_text,
                &expected_type_ids,
                scope_context,
                type_interner,
                &member_location,
                string_table,
            )?;
            // Fixed push can exceed capacity, so it keeps the existing fallible carrier.
            let error_type =
                resolve_builtin_error_type_typed(scope_context, &member_location, string_table)?;
            let result_type_ids =
                fallible_collection_result(none_type_id, error_type, type_interner);
            (args, result_type_ids)
        }

        CollectionBuiltinOp::Remove => {
            let expected_type_ids = [int_type_id];
            let args = parse_builtin_method_args_typed(
                token_stream,
                &member_name_text,
                &expected_type_ids,
                scope_context,
                type_interner,
                &member_location,
                string_table,
            )?;
            let error_type =
                resolve_builtin_error_type_typed(scope_context, &member_location, string_table)?;
            let result_type_ids =
                fallible_collection_result(element_type_id, error_type, type_interner);
            (args, result_type_ids)
        }

        CollectionBuiltinOp::Length => {
            let args = parse_builtin_method_args_typed(
                token_stream,
                &member_name_text,
                &[],
                scope_context,
                type_interner,
                &member_location,
                string_table,
            )?;
            (args, vec![int_type_id])
        }
    };

    if matches!(builtin, CollectionBuiltinOp::Get)
        && token_stream.current_token_kind().is_assignment_operator()
    {
        return Err(CompilerDiagnostic::invalid_assignment_target(
            InvalidAssignmentTargetReason::CollectionGetTargetNotWritable,
            None,
            Some(element_type_id),
            None,
            None,
            None,
            token_stream.current_location(),
        )
        .into());
    }

    // Collection `get`, `set`, fixed `push`, and `remove` produce fallible carriers, so the parser
    // rejects raw values before HIR can mistake them for ordinary runtime data. Growable push is
    // infallible, so handling suffixes on it never reach this check.
    if builtin.is_fallible() && !token_stream_starts_fallible_handling_suffix(token_stream) {
        return Err(CompilerDiagnostic::invalid_builtin_call(
            InvalidBuiltinCallReason::UnhandledFallibleCall,
            Some(member_name),
            token_stream.current_location(),
        )
        .into());
    }

    increment_ast_counter(AstCounter::PostfixReceiverNodesCopied);

    let receiver_expression = expression_from_postfix_node(receiver_node)?;
    let builtin_expression = Expression::collection_builtin_call_with_typed_arguments(
        receiver_expression,
        builtin,
        normalize_call_arguments(&args),
        result_type_ids,
        type_interner.environment_mut_for_derived_types(),
        member_location.clone(),
    );

    Ok(Some(AstNode {
        kind: NodeKind::ExpressionStatement(builtin_expression),
        scope: scope_context.scope.to_owned(),
        location: member_location,
    }))
}
