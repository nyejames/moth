//! External namespace member parsing.
//!
//! WHAT: handles function and constant members exposed by external package namespace records.
//! WHY: external package calls use registry IDs and backend metadata, which should stay separate
//! from source namespace handling.

use super::error::ExpressionParseError;
use super::expression_rpn::ExpressionRpnItem;
use super::expression_store::{AstExpressionBuilder, ExpressionConstructionFailure, OwnedExpr};
use super::function_calls::{
    ExternalFunctionCallParseInput, ParsedFunctionCallExpression,
    parse_external_function_call_expression,
};
use super::parse_expression_dispatch::push_expression_operand;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::statements::fallible_handling::fallible_catch_allowed_in_context;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic,
};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalConstantDef, ExternalConstantId, ExternalConstantRawValue,
    ExternalConstantValue, ExternalFunctionId, ExternalSignatureType,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use moth_lexical::numeric::parse::NumberLiteralErrorReason;

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
    pub(super) expression_builder: &'a mut AstExpressionBuilder,
    pub(super) const_values: &'a mut ConstValueStore,
    pub(super) expression: &'a mut Vec<ExpressionRpnItem>,
    pub(super) allow_boundary_catch: bool,
    pub(super) string_table: &'a mut StringTable,
    pub(super) path_fork: &'a mut PathInternerFork,
}

/// Parse a call to an external package function accessed through a namespace record.
///
/// WHAT: locates registered function metadata, parses the arguments and pushes the resulting call.
/// WHY: namespace and bare calls share metadata validation but have distinct syntactic entry paths.
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
        expression_builder,
        const_values,
        expression,
        allow_boundary_catch,
        string_table,
        path_fork,
    } = input;

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
    let parsed_call = parse_external_function_call_expression(ExternalFunctionCallParseInput {
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
        expression_builder,
        const_values,
        string_table,
        path_fork,
    })?;

    match parsed_call {
        ParsedFunctionCallExpression::Value(function_call_expression) => {
            push_expression_operand(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                expression,
                allow_boundary_catch,
                function_call_expression,
                path_fork,
            )?;
        }
        // A call whose numeric-dependent arguments were retained takes its authored operand
        // position as one pending item; the enclosing fragment either completes where it was
        // parsed or defers to the body root with this item in place, so the call resumes exactly
        // once at its original position.
        ParsedFunctionCallExpression::RetainedOperand(item) => {
            expression.push(item);
        }
    }

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
    pub(super) expression_builder: &'a mut AstExpressionBuilder,
    pub(super) const_values: &'a mut ConstValueStore,
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
        expression_builder,
        const_values,
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
        )
        .into());
    }

    // External constants are always immutable owned values.
    let value_mode = ValueMode::ImmutableOwned;

    let constant_expression = project_external_constant(
        constant_definition,
        member_name,
        member_span,
        value_mode,
        string_table,
        expression_builder,
        const_values,
    )?;

    push_expression_operand(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
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
/// WHAT: fixed foreign scalars keep their declared identity and exact payload. Raw host `U32`
///       and `F64` registrations are range-checked and checked for finiteness at their source use.
/// WHY: constants share the same ABI-versus-language distinction as external function slots, and
///      no default alias may stand in for the declared exact type.
pub(super) fn project_external_constant(
    constant_definition: &ExternalConstantDef,
    constant_name: StringId,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
    string_table: &mut StringTable,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> Result<OwnedExpr, ExpressionConstructionFailure> {
    match (&constant_definition.data_type, constant_definition.value) {
        (
            ExternalSignatureType::Abi(ExternalAbiType::Fixed(scalar)),
            ExternalConstantValue::Fixed(value),
        ) if value.scalar() == *scalar => {
            expression_builder.fixed_scalar(const_values, value, span, value_mode)
        }
        (
            ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::F64)),
            ExternalConstantValue::Registered {
                scalar: FixedScalar::F64,
                raw: ExternalConstantRawValue::BinaryFloat(value),
            },
        ) => expression_builder.float_from_external_constant(
            const_values,
            value,
            constant_name,
            span,
            value_mode,
        ),
        (
            ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::U32)),
            ExternalConstantValue::Registered {
                scalar: FixedScalar::U32,
                raw: ExternalConstantRawValue::Unsigned(value),
            },
        ) => {
            // Keep all host u64 bits at registration, but only materialise valid U32 values.
            let Some(materialised) = FixedScalarValue::unsigned(FixedScalar::U32, value) else {
                let literal_text = string_table.get_or_intern(value.to_string());
                return Err(CompilerDiagnostic::invalid_number_literal(
                    literal_text,
                    NumberLiteralErrorReason::OutsideFixedScalarRange(FixedScalar::U32),
                    span,
                )
                .into());
            };
            expression_builder.fixed_scalar(const_values, materialised, span, value_mode)
        }
        (
            ExternalSignatureType::Abi(ExternalAbiType::Utf8Str),
            ExternalConstantValue::StringSlice(value),
        ) => {
            let string_id = string_table.intern(value);
            expression_builder.string_slice(const_values, string_id, span, value_mode)
        }
        (ExternalSignatureType::Abi(ExternalAbiType::Bool), ExternalConstantValue::Bool(value)) => {
            expression_builder.bool(const_values, value, span, value_mode)
        }
        _ => unreachable!("registered external constant has mismatched signature and payload"),
    }
}
