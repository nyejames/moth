//! External namespace member parsing.
//!
//! WHAT: handles function and constant members exposed by external package namespace records.
//! WHY: external package calls use registry IDs and backend metadata, which should stay separate
//! from source namespace handling.

use super::error::ExpressionParseError;
use super::expression::Expression;
use super::expression_rpn::ExpressionRpnItem;
use super::function_calls::{
    ExternalFunctionCallParseInput, parse_external_function_call_expression,
};
use super::parse_expression_dispatch::push_expression_operand;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::statements::fallible_handling::fallible_catch_allowed_in_context;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic,
};
use crate::compiler_frontend::external_packages::{
    ExternalConstantId, ExternalConstantValue, ExternalFunctionId,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};
use crate::compiler_frontend::value_mode::ValueMode;

/// Input bundle for external namespace function member parsing.
///
/// WHAT: carries everything needed to parse a call to an external package function
/// accessed through a namespace record.
/// WHY: avoids threading a long argument list through the namespace member dispatch path.
pub(super) struct ExternalNamespaceFunctionMemberInput<'a, 'env> {
    pub(super) function_id: ExternalFunctionId,
    pub(super) member_name: StringId,
    pub(super) member_location: SourceLocation,
    pub(super) token_stream: &'a mut FileTokens,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'env>,
    pub(super) expression: &'a mut Vec<ExpressionRpnItem>,
    pub(super) allow_boundary_catch: bool,
    pub(super) string_table: &'a mut StringTable,
}

/// Parse a call to an external package function accessed through a namespace record.
///
/// WHAT: validates the call is not in a constant context, locates the external function
/// metadata, parses the call arguments, and pushes the resulting expression node.
/// WHY: external namespace function calls share backend metadata with bare external calls
/// but are reached through a different syntactic path (namespace.member).
pub(super) fn parse_external_namespace_function_member(
    input: ExternalNamespaceFunctionMemberInput<'_, '_>,
) -> Result<(), ExpressionParseError> {
    let ExternalNamespaceFunctionMemberInput {
        function_id,
        member_name,
        member_location,
        token_stream,
        context,
        type_interner,
        expression,
        allow_boundary_catch,
        string_table,
    } = input;

    // External function calls are not permitted in constant evaluation contexts.
    if context.kind.is_constant_context() {
        return Err(CompilerDiagnostic::compile_time_evaluation_error(
            CompileTimeEvaluationErrorReason::ExternalFunctionCallInConstantContext,
            Some(member_name),
            member_location,
        )
        .into());
    }

    // Namespace function members must be followed by an argument list.
    if token_stream.peek_next_token() != Some(&TokenKind::OpenParenthesis) {
        return Err(CompilerDiagnostic::unknown_value_name(member_name, member_location).into());
    }

    // Verify the external function metadata is still registered.
    let Some(external_function) = context
        .external_package_registry
        .get_function_by_id(function_id)
    else {
        return Err(CompilerDiagnostic::unknown_value_name(member_name, member_location).into());
    };

    // Advance from the member name to the opening parenthesis so the shared
    // external call parser sees the expected token stream position.
    token_stream.advance();

    // The shared call parser consumes external-call result handling itself, so
    // it needs the effective boundary-catch flag before argument parsing starts.
    let function_call_expression =
        parse_external_function_call_expression(ExternalFunctionCallParseInput {
            token_stream,
            external_function_id: function_id,
            external_function,
            call_location: member_location.clone(),
            context,
            value_required: true,
            allow_boundary_catch: allow_boundary_catch
                && expression.is_empty()
                && fallible_catch_allowed_in_context(context),
            warnings: None,
            type_interner,
            string_table,
        })?;

    push_expression_operand(
        token_stream,
        context,
        type_interner,
        string_table,
        expression,
        allow_boundary_catch,
        function_call_expression,
    )?;

    Ok(())
}

/// Input bundle for external namespace constant member parsing.
///
/// WHAT: carries everything needed to resolve an external package constant accessed
/// through a namespace record.
/// WHY: avoids threading a long argument list through the namespace member dispatch path.
pub(super) struct ExternalNamespaceConstantMemberInput<'a, 'env> {
    pub(super) constant_id: ExternalConstantId,
    pub(super) member_name: StringId,
    pub(super) member_location: SourceLocation,
    pub(super) token_stream: &'a mut FileTokens,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'env>,
    pub(super) expression: &'a mut Vec<ExpressionRpnItem>,
    pub(super) allow_boundary_catch: bool,
    pub(super) string_table: &'a mut StringTable,
}

/// Parse an external package constant accessed through a namespace record.
///
/// WHAT: locates the constant metadata, validates constant-context restrictions,
/// and pushes the resulting expression node.
/// WHY: external namespace constants are reached through namespace.member syntax
/// and need the same constant-context scalar restriction as bare external constants.
pub(super) fn parse_external_namespace_constant_member(
    input: ExternalNamespaceConstantMemberInput<'_, '_>,
) -> Result<(), ExpressionParseError> {
    let ExternalNamespaceConstantMemberInput {
        constant_id,
        member_name,
        member_location,
        token_stream,
        context,
        type_interner,
        expression,
        allow_boundary_catch,
        string_table,
    } = input;

    // Verify the external constant metadata is still registered.
    let Some(constant_definition) = context
        .external_package_registry
        .get_constant_by_id(constant_id)
    else {
        return Err(CompilerDiagnostic::unknown_value_name(member_name, member_location).into());
    };

    // Advance past the member name token so the caller resumes at the next token.
    token_stream.advance();

    // Non-scalar external constants cannot be used inside constant evaluations.
    if context.kind.is_constant_context() && !constant_definition.value.is_scalar() {
        return Err(CompilerDiagnostic::compile_time_evaluation_error(
            CompileTimeEvaluationErrorReason::ExternalNonScalarConstantInConstantContext,
            Some(member_name),
            member_location,
        )
        .into());
    }

    // External constants are always immutable owned values.
    let value_mode = ValueMode::ImmutableOwned;

    let constant_expression = match constant_definition.value {
        ExternalConstantValue::Float(value) => {
            Expression::float(value, member_location, value_mode)
        }

        ExternalConstantValue::Int(value) => Expression::int(value, member_location, value_mode),

        ExternalConstantValue::StringSlice(value) => {
            let string_id = string_table.intern(value);
            Expression::string_slice(string_id, member_location, value_mode)
        }

        ExternalConstantValue::Bool(value) => Expression::bool(value, member_location, value_mode),
    };

    push_expression_operand(
        token_stream,
        context,
        type_interner,
        string_table,
        expression,
        allow_boundary_catch,
        constant_expression,
    )?;

    Ok(())
}
