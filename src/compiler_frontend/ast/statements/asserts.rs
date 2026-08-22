//! Assert statement parsing.
//!
//! WHAT: parses the reserved `assert` statement through the shared call-argument contract.
//! WHY: assertion placement, unrecoverable failure, and message control-flow policy are special;
//!      parentheses, separators, named routing, defaults, access and type validation are not.
//!
//! The message-effect classifier lives in `ast::expressions::assertion_message_effects`, where
//! its AST/TIR/runtime representation boundary and message-evaluation control-flow rules have a
//! truthful owner. This module remains responsible for the assertion statement contract only.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::assertion_message_effects::assert_message_escape_diagnostic;
use crate::compiler_frontend::ast::expressions::assertion_message_effects::assertion_condition_is_statically_true;
use crate::compiler_frontend::ast::expressions::call_arguments::{
    CallArgumentSyntax, parse_call_arguments_typed_with_expectations,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext, ExpectedAccessMode,
    ExpectedParameterType, ParameterExpectation, resolve_call_arguments,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

pub(crate) fn parse_assert_statement(
    token_stream: &mut FileTokens,
    ast: &mut Vec<AstNode>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<(), ExpressionParseError> {
    let assert_location = token_stream.current_location();
    let assert_name = string_table.intern("assert");
    let condition_name = string_table.intern("condition");
    let message_name = string_table.intern("message");
    let generic_request_checkpoint = context.generic_request_checkpoint();

    token_stream.advance(); // past `assert`

    let bool_type_id = type_interner.environment().builtins().bool;
    let string_type_id = type_interner.environment().builtins().string;
    let default_message = Expression::option_none_with_type_id(
        string_type_id,
        DataType::StringSlice,
        type_interner.environment_mut_for_derived_types(),
        assert_location.clone(),
    );
    let message_type_id = default_message.type_id;

    let expectations = [
        ParameterExpectation {
            name: Some(condition_name),
            expected_type: ExpectedParameterType::Known(bool_type_id),
            access_mode: ExpectedAccessMode::Shared,
            requires_reactive_source: false,
            default_value: None,
        },
        ParameterExpectation {
            name: Some(message_name),
            expected_type: ExpectedParameterType::Known(message_type_id),
            access_mode: ExpectedAccessMode::Shared,
            requires_reactive_source: false,
            default_value: Some(default_message),
        },
    ];

    let raw_arguments = parse_call_arguments_typed_with_expectations(
        token_stream,
        context,
        type_interner,
        string_table,
        &expectations,
        CallArgumentSyntax::Supported {
            callee_name: Some(assert_name),
        },
    )?;

    let resolved_arguments = {
        let type_check_context = type_interner.type_check_context();
        resolve_call_arguments(
            CallDiagnosticContext::assertion("assert"),
            &raw_arguments,
            &expectations,
            assert_location.clone(),
            CallArgumentResolutionContext {
                string_table,
                type_environment: type_check_context.type_environment,
                compatibility_cache: type_check_context.compatibility_cache,
            },
        )?
    };

    let mut resolved_arguments = resolved_arguments.into_iter();
    let condition = resolved_arguments
        .next()
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Assertion call resolution did not produce a condition argument",
            )
        })?
        .value;
    let message = resolved_arguments
        .next()
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Assertion call resolution did not produce a message argument",
            )
        })?
        .value;

    if assertion_condition_is_statically_true(&condition) {
        // The message has been parsed, inferred, and evidence-checked above. It is inactive at
        // this compiler-owned boundary, so provisional generic requests must not leak into AST
        // finalization, HIR, linking, or backend request facts.
        context.discard_generic_requests_since(generic_request_checkpoint);
    }

    if let Some(diagnostic) =
        assert_message_escape_diagnostic(&message, &context.template_ir_store.borrow())?
    {
        return Err(diagnostic.into());
    }

    // Reject `assert(...)!` — assert is not a fallible expression.
    if token_stream.current_token_kind() == &TokenKind::Bang {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::BangOnNonFallible,
            token_stream.current_location(),
        )
        .into());
    }

    // Reject `assert(...) catch ...` — assert is not a fallible expression.
    if token_stream.current_token_kind() == &TokenKind::Catch {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::CatchOnNonFallible,
            token_stream.current_location(),
        )
        .into());
    }

    ast.push(AstNode {
        kind: NodeKind::Assert { condition, message },
        location: assert_location,
        scope: context.scope.clone(),
    });

    Ok(())
}

#[cfg(test)]
#[path = "tests/assertion_message_tests.rs"]
mod assertion_message_tests;
