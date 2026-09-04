//! Map builtin receiver-member parsing.
//!
//! WHAT: parses compiler-owned map members (`get/contains/set/remove/clear/length`).
//! WHY: map builtin policy should stay separate from user field/method dispatch.

use super::MemberStepContext;
use super::builtin_call_args::parse_builtin_method_args_typed;
use super::parse_chain::expression_from_postfix_node;
use super::receiver_access::{
    ReceiverAccessDiagnostic, ReceiverAccessRequirement, validate_receiver_access,
};
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::fallible_handling::token_stream_starts_fallible_handling_suffix;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::builtins::error_type::{
    ResolvedBuiltinType, resolve_builtin_error_type_typed,
};
use crate::compiler_frontend::builtins::maps::MapBuiltinOp;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidAssignmentTargetReason, InvalidBuiltinCallReason,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

// --------------------------
//  Helpers
// --------------------------

/// Temporary carrier bridge: map fallibility is public `Error!` control flow,
/// not a first-class `Result` value. HIR consumes this carrier immediately
/// while lowering the handled call into explicit success/error edges.
fn fallible_map_result(
    ok_type_id: TypeId,
    error_type: ResolvedBuiltinType,
    type_interner: &mut AstTypeInterner<'_>,
) -> Vec<TypeId> {
    vec![type_interner.intern_fallible_carrier(ok_type_id, error_type.type_id)]
}

// --------------------------
//  Main parser
// --------------------------

pub(super) fn parse_map_builtin_member_typed(
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

    let Some(map_shape) = type_interner.environment().map_shape(receiver_type_id) else {
        return Ok(None);
    };

    let Some(builtin) = MapBuiltinOp::from_source_name(string_table.resolve(member_name)) else {
        return Ok(None);
    };

    let bool_type_id = type_interner.builtins().bool;
    let int_type_id = type_interner.builtins().int;
    let none_type_id = type_interner.builtins().none;

    let key_type_id = map_shape.key_type;
    let value_type_id = map_shape.value_type;
    let member_name_text = builtin.source_name().to_owned();

    // Property-style `length` requires no parentheses.
    if builtin.is_property() {
        if token_stream.peek_next_token() == Some(&TokenKind::OpenParenthesis) {
            return Err(CompilerDiagnostic::invalid_builtin_call(
                InvalidBuiltinCallReason::MapLengthIsProperty,
                Some(member_name),
                member_location,
            )
            .into());
        }

        validate_receiver_access(
            receiver_node,
            receiver_access_mode,
            &member_location,
            authored_marker_location.as_ref(),
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
        if token_stream.current_token_kind().is_assignment_operator() {
            return Err(CompilerDiagnostic::invalid_assignment_target(
                InvalidAssignmentTargetReason::ReadOnlyMapProperty,
                None,
                Some(receiver_type_id),
                None,
                None,
                None,
                token_stream.current_location(),
            )
            .into());
        }

        increment_ast_counter(AstCounter::PostfixReceiverNodesCopied);

        let receiver_expression = expression_from_postfix_node(receiver_node)?;
        let builtin_expression = Expression::map_builtin_call_with_typed_arguments(
            receiver_expression,
            builtin,
            false,
            Vec::new(),
            vec![int_type_id],
            type_interner.environment_mut_for_derived_types(),
            member_location.clone(),
        );

        return Ok(Some(AstNode {
            kind: NodeKind::ExpressionStatement(builtin_expression),
            scope: scope_context.scope.to_owned(),
            location: member_location,
        }));
    }

    // All other map builtins require parentheses.
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
            diagnostic: ReceiverAccessDiagnostic::MapBuiltin {
                method_name: member_name,
            },
        },
    )?;

    token_stream.advance();

    // Parse arguments and compute result types for each builtin variant.
    let (args, result_type_ids) = match builtin {
        MapBuiltinOp::Get => {
            let expected_type_ids = [key_type_id];
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
            let result_type_ids = fallible_map_result(value_type_id, error_type, type_interner);
            (args, result_type_ids)
        }

        MapBuiltinOp::Contains => {
            let expected_type_ids = [key_type_id];
            let args = parse_builtin_method_args_typed(
                token_stream,
                &member_name_text,
                &expected_type_ids,
                scope_context,
                type_interner,
                &member_location,
                string_table,
            )?;
            (args, vec![bool_type_id])
        }

        MapBuiltinOp::Set => {
            let expected_type_ids = [key_type_id, value_type_id];
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
            let result_type_ids = fallible_map_result(none_type_id, error_type, type_interner);
            (args, result_type_ids)
        }

        MapBuiltinOp::Remove => {
            let expected_type_ids = [key_type_id];
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
            let result_type_ids = fallible_map_result(value_type_id, error_type, type_interner);
            (args, result_type_ids)
        }

        MapBuiltinOp::Clear => {
            let args = parse_builtin_method_args_typed(
                token_stream,
                &member_name_text,
                &[],
                scope_context,
                type_interner,
                &member_location,
                string_table,
            )?;
            (args, vec![none_type_id])
        }

        MapBuiltinOp::Length => {
            // Defensive: `length` is parsed as a property above.
            return Err(CompilerDiagnostic::invalid_builtin_call(
                InvalidBuiltinCallReason::MapLengthIsProperty,
                Some(member_name),
                member_location,
            )
            .into());
        }
    };

    // Reject assignment through `map.get(...)`.
    if matches!(builtin, MapBuiltinOp::Get)
        && token_stream.current_token_kind().is_assignment_operator()
    {
        return Err(CompilerDiagnostic::invalid_assignment_target(
            InvalidAssignmentTargetReason::MapGetTargetNotWritable,
            None,
            Some(value_type_id),
            None,
            None,
            None,
            token_stream.current_location(),
        )
        .into());
    }

    // Map `get`, `set`, and `remove` produce fallible carriers, so the parser
    // rejects raw values before HIR can mistake them for ordinary runtime data.
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
    let builtin_expression = Expression::map_builtin_call_with_typed_arguments(
        receiver_expression,
        builtin,
        mutating_receiver_required,
        args,
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
