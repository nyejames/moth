//! Function-call parsing and result-handling suffix integration.
//!
//! WHAT: resolves user/host call signatures and applies the postfix `!` propagation and `catch`
//!       recovery forms that can follow a call expression.
//! WHY: function-call completion owns result handling after the shared `call_arguments` parser
//!      has produced retained, source-located argument metadata.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallArgument, normalize_call_arguments,
};
use crate::compiler_frontend::ast::expressions::call_arguments::{
    NamedArgumentSyntax, parse_call_arguments_typed_with_expectations,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext, expectations_from_host_function,
    expectations_from_user_parameters, resolve_call_arguments,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::fallible_handling::{
    FallibleCallSite, FallibleHostCallSite, HandledFallibleCall, HandledFallibleHostCall,
    call_success_is_optional, non_fallible_handler_reason,
    parse_fallible_handling_suffix_for_call_expression,
    parse_fallible_handling_suffix_for_host_call_expression,
    token_stream_starts_fallible_handling_suffix,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::builtins::error_type::resolve_builtin_error_type_typed;
use crate::compiler_frontend::compiler_errors::{CompilerError, SourceLocation};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::{
    ExternalFunctionDef, ExternalFunctionId, ExternalSignatureType,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

/// Input bundle for `parse_function_call` to avoid long argument lists.
pub struct FunctionCallParseInput<'a, 'b> {
    pub token_stream: &'a mut FileTokens,
    pub id: &'a InternedPath,
    pub context: &'a ScopeContext,
    pub signature: &'a FunctionSignature,
    pub value_required: bool,
    pub allow_boundary_catch: bool,
    pub warnings: Option<&'a mut Vec<CompilerDiagnostic>>,
    pub type_interner: &'a mut AstTypeInterner<'b>,
    pub string_table: &'a mut StringTable,
}

/// Input bundle for external function calls.
pub struct ExternalFunctionCallParseInput<'a, 'b> {
    pub token_stream: &'a mut FileTokens,
    pub external_function_id: ExternalFunctionId,
    pub external_function: &'a ExternalFunctionDef,
    pub context: &'a ScopeContext,
    pub value_required: bool,
    pub allow_boundary_catch: bool,
    pub warnings: Option<&'a mut Vec<CompilerDiagnostic>>,
    pub type_interner: &'a mut AstTypeInterner<'b>,
    pub string_table: &'a mut StringTable,
}

struct ParsedExternalFunctionCall {
    id: ExternalFunctionId,
    args: Vec<CallArgument>,
    result_type_ids: Vec<TypeId>,
    error_return_type_id: Option<TypeId>,
    location: SourceLocation,
}

struct CallFinishContext<'a, 'b> {
    token_stream: &'a mut FileTokens,
    context: &'a ScopeContext,
    value_required: bool,
    allow_boundary_catch: bool,
    warnings: Option<&'a mut Vec<CompilerDiagnostic>>,
    type_interner: &'a mut AstTypeInterner<'b>,
    string_table: &'a mut StringTable,
}

/// Parses a source-function call for expression position.
///
/// WHAT: resolves a source or visible external function call and returns the
/// expression-owned call payload directly.
/// WHY: call validation belongs here, while callers should consume the same
/// expression contract in statement and expression positions.
pub(crate) fn parse_function_call_expression(
    input: FunctionCallParseInput<'_, '_>,
) -> Result<Expression, ExpressionParseError> {
    let FunctionCallParseInput {
        token_stream,
        id,
        context,
        signature,
        value_required,
        allow_boundary_catch,
        warnings,
        type_interner,
        string_table,
    } = input;

    // ------------------------
    //  Route to external call
    // ------------------------
    // External calls share the same argument parser, but they reject named targets until
    // external metadata carries stable public parameter names.
    if let Some((function_id, host_function)) = id
        .name()
        .and_then(|name| context.lookup_visible_external_function(name))
    {
        return parse_external_function_call_expression(ExternalFunctionCallParseInput {
            token_stream,
            external_function_id: function_id,
            external_function: host_function,
            context,
            value_required,
            allow_boundary_catch,
            warnings,
            type_interner,
            string_table,
        });
    }

    // ------------------------
    //  Parse and resolve arguments
    // ------------------------
    let parameter_expectations = expectations_from_user_parameters(&signature.parameters);
    let raw_args = parse_call_arguments_typed_with_expectations(
        token_stream,
        context,
        type_interner,
        string_table,
        &parameter_expectations,
        NamedArgumentSyntax::Supported {
            callee_name: id.name(),
        },
    )?;
    let args = resolve_user_function_call_arguments(
        id,
        &raw_args,
        &signature.parameters,
        token_stream.current_location(),
        string_table,
        type_interner,
        Some(context),
    )?;

    let call = HandledFallibleCall {
        name: id.to_owned(),
        result_type_ids: signature.success_return_type_ids(),
        args,
        call_location: token_stream.current_location(),
    };

    finish_function_call_expression(
        call,
        signature.error_return_type_id(),
        CallFinishContext {
            token_stream,
            context,
            value_required,
            allow_boundary_catch,
            warnings,
            type_interner,
            string_table,
        },
    )
}

fn finish_function_call_expression(
    call: HandledFallibleCall,
    error_return_type_id: Option<TypeId>,
    finish: CallFinishContext<'_, '_>,
) -> Result<Expression, ExpressionParseError> {
    let CallFinishContext {
        token_stream,
        context,
        value_required,
        allow_boundary_catch,
        warnings,
        type_interner,
        string_table,
    } = finish;

    let Some(error_return_type_id) = error_return_type_id else {
        if matches!(
            token_stream.current_token_kind(),
            TokenKind::Bang | TokenKind::Catch
        ) {
            let operand_is_optional = call_success_is_optional(
                call.result_type_ids.as_slice(),
                type_interner.environment(),
            );
            return Err(CompilerDiagnostic::invalid_fallible_handling(
                non_fallible_handler_reason(token_stream.current_token_kind(), operand_is_optional),
                token_stream.current_location(),
            )
            .into());
        }

        return Ok(call.into_plain_expression(type_interner.environment_mut_for_derived_types()));
    };

    if token_stream_starts_fallible_handling_suffix(token_stream) {
        return parse_fallible_handling_suffix_for_call_expression(
            token_stream,
            context,
            FallibleCallSite {
                call,
                error_return_type_id,
                value_required,
                allow_boundary_catch,
            },
            warnings,
            type_interner,
            string_table,
        );
    }

    Err(CompilerDiagnostic::invalid_fallible_handling(
        InvalidFallibleHandlingReason::UnhandledErrorReturn,
        token_stream.current_location(),
    )
    .into())
}

fn resolve_user_function_call_arguments(
    function_name: &InternedPath,
    raw_args: &[CallArgument],
    parameters: &[Declaration],
    location: SourceLocation,
    string_table: &mut StringTable,
    type_interner: &mut AstTypeInterner<'_>,
    _scope_context: Option<&ScopeContext>,
) -> Result<Vec<CallArgument>, ExpressionParseError> {
    let callee_name = function_name
        .name_str(string_table)
        .map(|name| name.to_owned())
        .unwrap_or_else(|| String::from("<unknown>"));
    let expectations = expectations_from_user_parameters(parameters);
    let type_check_context = type_interner.type_check_context();

    resolve_call_arguments(
        CallDiagnosticContext::function(&callee_name),
        raw_args,
        &expectations,
        location,
        CallArgumentResolutionContext {
            string_table,
            type_environment: type_check_context.type_environment,
            compatibility_cache: type_check_context.compatibility_cache,
        },
    )
    .map_err(ExpressionParseError::from)
}

/// Parses an external-function call for expression position.
///
/// WHAT: returns the narrowed expression call contract after shared external-call
/// argument and signature validation.
/// WHY: statement and expression parsing both consume the expression-owned call
/// payload, avoiding a statement-shaped call-node detour for host functions.
pub(crate) fn parse_external_function_call_expression(
    input: ExternalFunctionCallParseInput<'_, '_>,
) -> Result<Expression, ExpressionParseError> {
    let ExternalFunctionCallParseInput {
        token_stream,
        external_function_id,
        external_function,
        context,
        value_required,
        allow_boundary_catch,
        warnings,
        type_interner,
        string_table,
    } = input;

    let parsed_call = parse_external_function_call_parts(
        token_stream,
        external_function_id,
        external_function,
        context,
        type_interner,
        string_table,
    )?;

    finish_external_function_call_expression(
        parsed_call,
        CallFinishContext {
            token_stream,
            context,
            value_required,
            allow_boundary_catch,
            warnings,
            type_interner,
            string_table,
        },
    )
}

fn parse_external_function_call_parts(
    token_stream: &mut FileTokens,
    external_function_id: ExternalFunctionId,
    external_function: &ExternalFunctionDef,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<ParsedExternalFunctionCall, ExpressionParseError> {
    let location = token_stream.current_location();

    // ------------------------
    //  Parse raw arguments
    // ------------------------
    // External metadata does not expose public parameter names yet, so named arguments remain
    // intentionally unsupported.
    let expectations = {
        let type_environment = type_interner.environment_mut_for_derived_types();
        expectations_from_host_function(external_function, type_environment)
    };
    let callee_name = string_table.intern(&external_function.name);
    let raw_args = parse_call_arguments_typed_with_expectations(
        token_stream,
        context,
        type_interner,
        string_table,
        &expectations,
        NamedArgumentSyntax::UnsupportedCall {
            callee_name: Some(callee_name),
        },
    )?;

    // ------------------------
    //  Resolve and validate arguments
    // ------------------------
    let type_check_context = type_interner.type_check_context();
    let args = resolve_call_arguments(
        CallDiagnosticContext::host_function(&external_function.name),
        &raw_args,
        &expectations,
        location.clone(),
        CallArgumentResolutionContext {
            string_table,
            type_environment: type_check_context.type_environment,
            compatibility_cache: type_check_context.compatibility_cache,
        },
    )
    .map_err(ExpressionParseError::from)?;
    // ------------------------
    //  Validate signature and returns
    // ------------------------
    let builtin_error_type = resolve_builtin_error_type_typed(context, &location, string_table)?;
    validate_external_signature_types_are_registered(external_function, context, location.clone())?;
    let diagnostic_result_types = external_function.success_return_data_types();
    let result_type_ids = external_function.success_return_type_ids(
        type_interner.environment_mut_for_derived_types(),
        builtin_error_type.type_id,
    );
    validate_external_return_slots_are_visible(
        external_function,
        &diagnostic_result_types,
        &result_type_ids,
        location.clone(),
    )?;

    let error_return_type_id = external_function.error_return_type_id(
        type_interner.environment_mut_for_derived_types(),
        builtin_error_type.type_id,
    );

    let error_return_type_id = if external_function.is_fallible() {
        let Some(error_return_type_id) = error_return_type_id else {
            return Err(CompilerError::compiler_error(format!(
                "Fallible external function '{}' has no frontend-visible concrete error slot.",
                external_function.name
            ))
            .into());
        };

        Some(error_return_type_id)
    } else {
        None
    };

    Ok(ParsedExternalFunctionCall {
        id: external_function_id,
        args,
        result_type_ids,
        error_return_type_id,
        location,
    })
}

fn finish_external_function_call_expression(
    parsed_call: ParsedExternalFunctionCall,
    finish: CallFinishContext<'_, '_>,
) -> Result<Expression, ExpressionParseError> {
    let CallFinishContext {
        token_stream,
        context,
        value_required,
        allow_boundary_catch,
        warnings,
        type_interner,
        string_table,
    } = finish;

    let ParsedExternalFunctionCall {
        id,
        args,
        result_type_ids,
        error_return_type_id,
        location,
    } = parsed_call;

    if let Some(error_type_id) = error_return_type_id {
        let call = HandledFallibleHostCall {
            name: id,
            args,
            result_type_ids,
            error_type_id,
            call_location: location,
        };

        if token_stream_starts_fallible_handling_suffix(token_stream) {
            return parse_fallible_handling_suffix_for_host_call_expression(
                token_stream,
                context,
                FallibleHostCallSite {
                    call,
                    value_required,
                    allow_boundary_catch,
                },
                warnings,
                type_interner,
                string_table,
            );
        }

        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::UnhandledErrorReturn,
            token_stream.current_location(),
        )
        .into());
    }

    if matches!(
        token_stream.current_token_kind(),
        TokenKind::Bang | TokenKind::Catch
    ) {
        let operand_is_optional =
            call_success_is_optional(result_type_ids.as_slice(), type_interner.environment());
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            non_fallible_handler_reason(token_stream.current_token_kind(), operand_is_optional),
            token_stream.current_location(),
        )
        .into());
    }

    let normalized_args = normalize_call_arguments(&args);
    Ok(Expression::host_function_call_with_typed_arguments(
        id,
        normalized_args,
        result_type_ids,
        type_interner.environment_mut_for_derived_types(),
        location,
    ))
}

/// Verifies that every declared return slot has a corresponding frontend-visible type.
fn validate_external_return_slots_are_visible(
    external_function: &ExternalFunctionDef,
    diagnostic_result_types: &[DataType],
    result_type_ids: &[TypeId],
    location: SourceLocation,
) -> Result<(), ExpressionParseError> {
    if external_function.returns.len() != diagnostic_result_types.len()
        || external_function.returns.len() != result_type_ids.len()
    {
        return Err(CompilerError::compiler_error(format!(
            "External function '{}' declares a return slot that is not frontend-visible at {:?}.",
            external_function.name, location
        ))
        .into());
    }

    Ok(())
}

/// Ensures every type referenced in an external function signature is registered
/// in the external package registry.
fn validate_external_signature_types_are_registered(
    external_function: &ExternalFunctionDef,
    context: &ScopeContext,
    location: SourceLocation,
) -> Result<(), ExpressionParseError> {
    for parameter in &external_function.parameters {
        validate_external_signature_type_is_registered(
            external_function,
            &parameter.language_type,
            context,
            location.clone(),
        )?;
    }

    for slot in &external_function.returns {
        validate_external_signature_type_is_registered(
            external_function,
            &slot.value_type,
            context,
            location.clone(),
        )?;
    }

    if let Some(error_type) = &external_function.error_return_type {
        validate_external_signature_type_is_registered(
            external_function,
            error_type,
            context,
            location,
        )?;
    }

    Ok(())
}

/// Checks that a single external signature type is known to the frontend.
fn validate_external_signature_type_is_registered(
    external_function: &ExternalFunctionDef,
    signature_type: &ExternalSignatureType,
    context: &ScopeContext,
    location: SourceLocation,
) -> Result<(), ExpressionParseError> {
    match signature_type {
        ExternalSignatureType::Abi(_)
        | ExternalSignatureType::BuiltinError
        | ExternalSignatureType::StringContent => Ok(()),
        ExternalSignatureType::External(type_id) => {
            if context
                .external_package_registry
                .get_type_by_id(*type_id)
                .is_some()
            {
                return Ok(());
            }

            Err(CompilerError::compiler_error(format!(
                "External function '{}' references unknown external type {:?} at {:?}.",
                external_function.name, type_id, location
            ))
            .into())
        }
        ExternalSignatureType::Optional(inner) => validate_external_signature_type_is_registered(
            external_function,
            inner,
            context,
            location,
        ),
    }
}

#[cfg(test)]
#[path = "tests/cast_boundary_tests.rs"]
mod cast_boundary_tests;
