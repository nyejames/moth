//! Source function and generic free-function call parsing.
//!
//! WHAT: handles calls to source-defined functions, including generic templates,
//! and pushes the resulting expression-owned call payloads into expression RPN.
//! WHY: identifier and namespace parsing both route source callable members here
//! so generic and non-generic call behavior stays consistent.

use super::error::ExpressionParseError;
use super::expression_rpn::ExpressionRpnItem;
use super::function_calls::{FunctionCallParseInput, parse_function_call_expression};
use super::parse_expression_dispatch::push_expression_operand;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::generic_functions::{
    GenericCallExpectedContext, GenericFunctionCallParseInput, GenericFunctionTemplate,
    parse_generic_function_call_expression, validate_generic_function_template_call_expression,
    with_generic_primary_span,
};
use crate::compiler_frontend::ast::statements::fallible_handling::fallible_catch_allowed_in_context;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidGenericInstantiationReason,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

/// Input bundle for source callable member parsing.
///
/// WHAT: carries everything needed to parse a call to a source-defined function,
/// whether generic or non-generic.
/// WHY: avoids threading a long argument list through both bare-identifier and
/// namespace-member call sites.
pub(super) struct SourceCallableMemberInput<'a, 'env> {
    pub(super) token_stream: &'a mut FileTokens,
    pub(super) function_path: &'a PathId,
    pub(super) signature: &'a FunctionSignature,
    pub(super) generic_template: Option<&'a GenericFunctionTemplate>,
    pub(super) visible_name: StringId,
    pub(super) call_span: Option<SourceSpan>,
    pub(super) context: &'a ScopeContext,
    pub(super) expression: &'a mut Vec<ExpressionRpnItem>,
    pub(super) allow_boundary_catch: bool,
    pub(super) expected_result_evidence_allowed: bool,
    pub(super) type_interner: &'a mut AstTypeInterner<'env>,
    pub(super) string_table: &'a mut StringTable,
    pub(super) path_fork: &'a mut PathInternerFork,
}

/// Parse a call to a source-defined function (generic or non-generic) and push
/// the resulting expression node onto the expression buffer.
///
/// WHAT: resolves generic vs non-generic dispatch, enforces the "generic function
/// values are deferred" rule, and pushes the expression-owned call payload into
/// the active RPN buffer.
/// WHY: both bare identifier and namespace member access share this path, so
/// keeping it in one place guarantees consistent behavior.
pub(super) fn parse_source_callable_member(
    input: SourceCallableMemberInput<'_, '_>,
) -> Result<(), ExpressionParseError> {
    let SourceCallableMemberInput {
        token_stream,
        function_path,
        signature,
        generic_template,
        visible_name,
        call_span,
        context,
        expression,
        allow_boundary_catch,
        expected_result_evidence_allowed,
        type_interner,
        string_table,
        path_fork,
    } = input;

    let expression_is_boundary_leading = expression.is_empty();
    let allow_call_boundary_catch = allow_boundary_catch
        && expression_is_boundary_leading
        && fallible_catch_allowed_in_context(context);

    // ------------------------
    //  Generic source call
    // ------------------------
    if let Some(template) = generic_template {
        match token_stream.peek_next_token() {
            // Explicit call-site type arguments are not part of the Alpha surface.
            // Reject the known foreign spellings before they can be interpreted as
            // generic function values, comparisons, or templates.
            Some(TokenKind::Of | TokenKind::LessThan | TokenKind::TemplateHead) => {
                let explicit_syntax_span = token_stream
                    .tokens
                    .get(token_stream.index + 1)
                    .map(|token| SourceSpan::new(token_stream.file_id, token.span))
                    .or(call_span);

                return Err(with_generic_primary_span(
                    explicit_generic_call_type_arguments_error(visible_name, explicit_syntax_span),
                    explicit_syntax_span,
                )
                .into());
            }

            // Generic functions must be called; using them as first-class values is
            // deferred for Alpha. Require an immediate `(` to route into the call parser.
            Some(TokenKind::OpenParenthesis) => {}

            _ => {
                return Err(with_generic_primary_span(
                    CompilerDiagnostic::invalid_generic_instantiation(
                        Some(visible_name),
                        InvalidGenericInstantiationReason::GenericFunctionValueDeferred,
                        call_span,
                    ),
                    call_span,
                )
                .into());
            }
        }

        // Move from the visible generic function name to the `(` consumed by the shared call parser.
        token_stream.advance();

        let expected_context = if expected_result_evidence_allowed
            && expression_is_boundary_leading
            && !context.expected_result_type_ids.is_empty()
        {
            GenericCallExpectedContext::ImmediateResult(context.expected_result_type_ids.as_slice())
        } else {
            GenericCallExpectedContext::None
        };

        let generic_call_input = GenericFunctionCallParseInput {
            token_stream,
            template,
            context,
            expected_context,
            value_required: true,
            allow_boundary_catch: allow_call_boundary_catch,
            call_span,
            warnings: None,
            type_interner,
            string_table,
            path_fork,
        };

        let function_call_expression = if context.generic_template_validation {
            validate_generic_function_template_call_expression(generic_call_input)
        } else {
            parse_generic_function_call_expression(generic_call_input)
        }?;

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

        return Ok(());
    }

    // ------------------------
    //  Non-generic source call
    // ------------------------
    token_stream.advance();

    let function_call_expression = parse_function_call_expression(FunctionCallParseInput {
        token_stream,
        id: function_path,
        call_span,
        context,
        signature,
        value_required: true,
        allow_boundary_catch: allow_call_boundary_catch,
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

fn explicit_generic_call_type_arguments_error(
    function_name: StringId,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_generic_instantiation(
        Some(function_name),
        InvalidGenericInstantiationReason::ExplicitCallTypeArgumentsUnsupported,
        span,
    )
}
