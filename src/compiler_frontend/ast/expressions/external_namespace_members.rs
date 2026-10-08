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
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::statements::fallible_handling::fallible_catch_allowed_in_context;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic,
};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalConstantDef, ExternalConstantId, ExternalConstantValue,
    ExternalFunctionId, ExternalSignatureType,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::parse::NumberLiteralErrorReason;
use moth_lexical::numeric::profile::NumericProfile;

/// Input bundle for external namespace function member parsing.
///
/// WHAT: carries everything needed to parse a call to an external package function
/// accessed through a namespace record.
/// WHY: avoids threading a long argument list through the namespace member dispatch path.
pub(super) struct ExternalNamespaceFunctionMemberInput<'a, 'env, 'tokens> {
    pub(super) function_id: ExternalFunctionId,
    pub(super) member_name: StringId,
    pub(super) member_span: Option<SourceSpan>,
    pub(super) token_stream: &'a mut AstCursor<'tokens>,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'env>,
    pub(super) expression: &'a mut Vec<ExpressionRpnItem>,
    pub(super) allow_boundary_catch: bool,
    pub(super) string_table: &'a mut StringTable,
    pub(super) path_fork: &'a mut PathInternerFork,
}

/// Parse a call to an external package function accessed through a namespace record.
///
/// WHAT: validates the call is not in a constant context, locates the external function
/// metadata, parses the call arguments, and pushes the resulting expression node.
/// WHY: external namespace function calls share backend metadata with bare external calls
/// but are reached through a different syntactic path (namespace.member).
pub(super) fn parse_external_namespace_function_member(
    input: ExternalNamespaceFunctionMemberInput<'_, '_, '_>,
) -> Result<(), ExpressionParseError> {
    let ExternalNamespaceFunctionMemberInput {
        function_id,
        member_name,
        member_span,
        token_stream,
        context,
        type_interner,
        expression,
        allow_boundary_catch,
        string_table,
        path_fork,
    } = input;

    // External function calls are not permitted in constant evaluation contexts.
    if context.kind.is_constant_context() {
        return Err(CompilerDiagnostic::compile_time_evaluation_error(
            CompileTimeEvaluationErrorReason::ExternalFunctionCallInConstantContext,
            Some(member_name),
            member_span,
            None,
        )
        .into());
    }

    // Namespace function members must be followed by an argument list.
    if token_stream.peek_next_tag() != Some(TokenTag::OPEN_PARENTHESIS) {
        return Err(CompilerDiagnostic::unknown_value_name(member_name, member_span).into());
    }

    // Verify the external function metadata is still registered.
    let Some(external_function) = context
        .external_package_registry
        .get_function_by_id(function_id)
    else {
        return Err(CompilerDiagnostic::unknown_value_name(member_name, member_span).into());
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
            call_span: member_span,
            context,
            value_required: true,
            allow_boundary_catch: allow_boundary_catch
                && expression.is_empty()
                && fallible_catch_allowed_in_context(context),
            warnings: None,
            type_interner,
            string_table,
            path_fork,
        })?;

    push_expression_operand(
        token_stream,
        context,
        type_interner,
        string_table,
        expression,
        allow_boundary_catch,
        function_call_expression,
        path_fork,
    )?;

    Ok(())
}

/// Input bundle for external namespace constant member parsing.
///
/// WHAT: carries everything needed to resolve an external package constant accessed
/// through a namespace record.
/// WHY: avoids threading a long argument list through the namespace member dispatch path.
pub(super) struct ExternalNamespaceConstantMemberInput<'a, 'env, 'tokens> {
    pub(super) constant_id: ExternalConstantId,
    pub(super) member_name: StringId,
    pub(super) member_span: Option<SourceSpan>,
    pub(super) token_stream: &'a mut AstCursor<'tokens>,
    pub(super) context: &'a ScopeContext,
    pub(super) type_interner: &'a mut AstTypeInterner<'env>,
    pub(super) expression: &'a mut Vec<ExpressionRpnItem>,
    pub(super) allow_boundary_catch: bool,
    pub(super) string_table: &'a mut StringTable,
    pub(super) path_fork: &'a mut PathInternerFork,
}

/// Parse an external package constant accessed through a namespace record.
///
/// WHAT: locates the external constant metadata, validates constant-context restrictions,
/// and pushes the resulting expression node.
/// WHY: external namespace constants are reached through namespace.member syntax and need the same
/// constant-context scalar restriction as bare external constants.
pub(super) fn parse_external_namespace_constant_member(
    input: ExternalNamespaceConstantMemberInput<'_, '_, '_>,
) -> Result<(), ExpressionParseError> {
    let ExternalNamespaceConstantMemberInput {
        constant_id,
        member_name,
        member_span,
        token_stream,
        context,
        type_interner,
        expression,
        allow_boundary_catch,
        string_table,
        path_fork,
    } = input;
    // Verify the external constant metadata is still registered.
    let Some(constant_definition) = context
        .external_package_registry
        .get_constant_by_id(constant_id)
    else {
        return Err(CompilerDiagnostic::unknown_value_name(member_name, member_span).into());
    };

    // Advance past the member name token so the caller resumes at the next token.
    token_stream.advance();

    // Non-scalar external constants cannot be used inside constant evaluations.
    if context.kind.is_constant_context() && !constant_definition.value.is_scalar() {
        return Err(CompilerDiagnostic::compile_time_evaluation_error(
            CompileTimeEvaluationErrorReason::ExternalNonScalarConstantInConstantContext,
            Some(member_name),
            member_span,
            None,
        )
        .into());
    }

    // External constants are always immutable owned values.
    let value_mode = ValueMode::ImmutableOwned;

    let constant_expression = project_external_constant(
        constant_definition,
        member_name,
        member_span,
        context.numeric_profile,
        value_mode,
        string_table,
    )?;

    push_expression_operand(
        token_stream,
        context,
        type_interner,
        string_table,
        expression,
        allow_boundary_catch,
        constant_expression,
        path_fork,
    )?;

    Ok(())
}

/// Projects an external constant using its semantic signature type.
///
/// WHAT: fixed foreign scalars project their already materialised exact value, while native Moth
///       Int/Uint/Float values use their profile-selected literal constructors.
/// WHY: constants share the same ABI-versus-language distinction as external function slots.
pub(super) fn project_external_constant(
    constant_definition: &ExternalConstantDef,
    constant_name: StringId,
    span: Option<SourceSpan>,
    numeric_profile: NumericProfile,
    value_mode: ValueMode,
    string_table: &mut StringTable,
) -> Result<Expression, CompilerDiagnostic> {
    match (&constant_definition.data_type, constant_definition.value) {
        (ExternalSignatureType::NativeFloat, ExternalConstantValue::Float(value)) => {
            Expression::float_from_external_constant(
                value,
                numeric_profile,
                constant_name,
                span,
                value_mode,
            )
        }
        (
            ExternalSignatureType::Abi(ExternalAbiType::Fixed(_)),
            ExternalConstantValue::Fixed(value),
        ) => Ok(Expression::fixed_scalar(value, span, value_mode)),
        (ExternalSignatureType::NativeInt, ExternalConstantValue::Int(value)) => {
            Ok(Expression::int(i64::from(value), span, value_mode))
        }
        (ExternalSignatureType::NativeUint, ExternalConstantValue::Uint(value)) => {
            // Compiler-registered payloads are exact `u64`, so the selected Uint width
            // gates them exactly like a source literal: Uint32 rejects 4294967296 and
            // above with the source-level range diagnostic instead of an internal
            // backend error at emission.
            if value > numeric_profile.int_width.unsigned_max_value() {
                let literal_text = string_table.intern(&value.to_string());
                return Err(CompilerDiagnostic::invalid_number_literal(
                    literal_text,
                    NumberLiteralErrorReason::OutsideUintRange(numeric_profile.int_width),
                    span,
                ));
            }
            Ok(Expression::uint(value, span, value_mode))
        }
        (
            ExternalSignatureType::Abi(ExternalAbiType::Utf8Str),
            ExternalConstantValue::StringSlice(value),
        ) => {
            let string_id = string_table.intern(value);
            Ok(Expression::string_slice(string_id, span, value_mode))
        }
        (ExternalSignatureType::Abi(ExternalAbiType::Bool), ExternalConstantValue::Bool(value)) => {
            Ok(Expression::bool(value, span, value_mode))
        }
        _ => unreachable!("registered external constant has mismatched signature and payload"),
    }
}
