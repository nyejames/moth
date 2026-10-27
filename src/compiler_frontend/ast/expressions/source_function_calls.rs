//! Source function and generic free-function call parsing.
//!
//! WHAT: handles calls to source-defined functions, including generic templates,
//! and pushes the resulting expression-owned call payloads into expression RPN.
//! WHY: identifier and namespace parsing both route source callable members here
//! so generic and non-generic call behavior stays consistent.

use super::error::ExpressionParseError;
use super::expression_rpn::ExpressionRpnItem;
use super::function_calls::{
    FunctionCallParseInput, ParsedFunctionCallExpression, parse_function_call_expression,
};
use super::parse_expression_dispatch::push_expression_operand;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::generic_functions::{
    GenericCallExpectedContext, GenericFunctionCallParseInput, GenericFunctionTemplate,
    parse_generic_function_call_expression, validate_generic_function_template_call_expression,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::InvalidGenericInstantiationReason;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Input bundle for source callable member parsing.
///
/// WHAT: carries everything needed to parse a call to a source-defined function,
/// whether generic or non-generic.
/// WHY: avoids threading a long argument list through both bare-identifier and
/// namespace-member call sites.
pub(super) struct SourceCallableMemberInput<'a, 'env, 'tokens> {
    pub(super) token_stream: &'a mut AstCursor<'tokens>,
    pub(super) function_path: &'a PathId,
    pub(super) signature: &'a FunctionSignature,
    pub(super) generic_template: Option<&'a GenericFunctionTemplate>,
    pub(super) visible_name: StringId,
    pub(super) call_span: Option<SourceSpan>,
    pub(super) context: &'a ScopeContext,
    pub(super) expression: &'a mut Vec<ExpressionRpnItem>,
    /// Whether the innermost fragment frame had no items before this call.
    pub(super) expression_is_boundary_leading: bool,
    pub(super) allow_boundary_catch: bool,
    pub(super) expected_result_evidence_allowed: bool,
    pub(super) type_interner: &'a mut AstTypeInterner<'env>,
    pub(super) expression_builder: &'a mut AstExpressionBuilder,
    pub(super) const_values: &'a mut ConstValueStore,
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
    input: SourceCallableMemberInput<'_, '_, '_>,
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
        expression_is_boundary_leading,
        allow_boundary_catch,
        expected_result_evidence_allowed,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    } = input;

    // ------------------------
    //  Generic source call
    // ------------------------
    if let Some(template) = generic_template {
        match token_stream.peek_next_tag() {
            // Explicit call-site type arguments are not part of the Alpha surface.
            // Reject the known foreign spellings before they can be interpreted as
            // generic function values, comparisons, or templates.
            Some(TokenTag::OF | TokenTag::LESS_THAN | TokenTag::TEMPLATE_HEAD) => {
                // Pure lookahead: read the offending follower span through the cursor view.
                let follower_span = token_stream.span_at(token_stream.position().saturating_add(1));
                let explicit_syntax_span = follower_span.or(call_span);

                return Err(explicit_generic_call_type_arguments_error(
                    visible_name,
                    explicit_syntax_span,
                )
                .into());
            }

            // Generic functions must be called; using them as first-class values is
            // deferred for Alpha. Require an immediate `(` to route into the call parser.
            Some(TokenTag::OPEN_PARENTHESIS) => {}

            _ => {
                return Err(generics_traits_api::invalid_generic_instantiation(
                    Some(visible_name),
                    InvalidGenericInstantiationReason::GenericFunctionValueDeferred,
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
            allow_boundary_catch,
            call_span,
            warnings: None,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        };

        let parsed_call = if context.generic_template_validation {
            validate_generic_function_template_call_expression(generic_call_input)
        } else {
            parse_generic_function_call_expression(generic_call_input)
        }?;

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
            // Retained generic calls take their authored operand position exactly like retained
            // non-generic calls; see the non-generic branch below.
            ParsedFunctionCallExpression::RetainedOperand(item) => {
                expression.push(item);
            }
        }

        return Ok(());
    }

    // ------------------------
    //  Non-generic source call
    // ------------------------
    token_stream.advance();

    let parsed_call = parse_function_call_expression(FunctionCallParseInput {
        token_stream,
        id: function_path,
        call_span,
        context,
        signature,
        value_required: true,
        allow_boundary_catch,
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
        // parsed (when its own receiving boundary can) or defers to the body root with this item
        // in place, so the call resumes exactly once at its original position.
        ParsedFunctionCallExpression::RetainedOperand(item) => {
            expression.push(item);
        }
    }

    Ok(())
}

fn explicit_generic_call_type_arguments_error(
    function_name: StringId,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    generics_traits_api::invalid_generic_instantiation(
        Some(function_name),
        InvalidGenericInstantiationReason::ExplicitCallTypeArgumentsUnsupported,
        span,
    )
}
