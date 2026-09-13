//! Fallible suffix parsing implementation.
//!
//! WHAT: parses postfix `!` propagation and `catch` recovery suffixes for fallible calls and
//! expressions without exposing raw Result values as ordinary user data.
//!
//! WHY: fallible handling has dedicated control-flow rules (error-type compatibility, catch-body
//! value production, and boundary restrictions) that would make the general expression parser too
//! large and too coupled to function bodies.
//!
//! STAGE BOUNDARY: this is pure AST frontend parsing. Result handling is attached to
//! expression-owned call payloads; handler bodies are carried only by `ValueBlock::Catch`.

use crate::compiler_frontend::ast::ContextKind;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleExpressionHandling, FallibleHandling,
    HandledFallibleHostFunctionCallInput,
};
use crate::compiler_frontend::ast::statements::value_production::types::{
    ValueBlock, ValueCatchBlock,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::type_coercion::compatibility::is_postfix_error_compatible;

use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;

use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

use super::catch_handler::{
    CatchFallibleHandler, CatchFallibleHandlerSite, parse_catch_fallible_handler_typed,
    parse_catch_without_error_binding_typed, parse_inline_catch_fallible_handler_typed,
    parse_inline_catch_without_error_binding_typed,
};
use super::success_types::fallible_success_type_ids;
use super::{EXPRESSION_STAGE, FUNCTION_CALL_STAGE};

pub(crate) struct HandledFallibleCall {
    pub(crate) name: PathId,
    pub(crate) args: Vec<CallArgument>,
    pub(crate) result_type_ids: Vec<TypeId>,
    pub(crate) call_span: Option<SourceSpan>,
}

pub(crate) struct FallibleCallSite {
    pub(crate) call: HandledFallibleCall,
    pub(crate) error_return_type_id: TypeId,
    pub(crate) value_required: bool,
    pub(crate) allow_boundary_catch: bool,
}

pub(crate) struct HandledFallibleHostCall {
    pub(crate) name: ExternalFunctionId,
    pub(crate) args: Vec<CallArgument>,
    pub(crate) result_type_ids: Vec<TypeId>,
    pub(crate) error_type_id: TypeId,
    pub(crate) call_span: Option<SourceSpan>,
}

pub(crate) struct FallibleHostCallSite {
    pub(crate) call: HandledFallibleHostCall,
    pub(crate) value_required: bool,
    pub(crate) allow_boundary_catch: bool,
}

struct FallibleHandlingSite<'a> {
    success_result_type_ids: &'a [TypeId],
    error_return_type_id: TypeId,
    value_required: bool,
    value_required_span: Option<SourceSpan>,
    compilation_stage: &'a str,
    allow_boundary_catch: bool,
}

impl HandledFallibleHostCall {
    pub(crate) fn into_expression(
        self,
        handling: FallibleHandling,
        propagation_span: Option<SourceSpan>,
        type_environment: &mut TypeEnvironment,
    ) -> Expression {
        let expression_handling = match &handling {
            FallibleHandling::Propagate => FallibleExpressionHandling::Propagate,
            FallibleHandling::Handler { .. } => FallibleExpressionHandling::Recover,
        };
        let result_type_ids = self.result_type_ids.clone();
        let function_call_expression =
            Expression::handled_fallible_host_function_call_with_typed_arguments(
                HandledFallibleHostFunctionCallInput {
                    id: self.name,
                    args: self.args,
                    result_type_ids: self.result_type_ids,
                    error_type_id: self.error_type_id,
                    handling: expression_handling,
                    span: self.call_span,
                },
                type_environment,
            );
        let function_call_expression = match propagation_span {
            Some(span) => function_call_expression.with_propagation_span(Some(span)),
            None => function_call_expression,
        };

        match handling {
            FallibleHandling::Propagate => function_call_expression,
            FallibleHandling::Handler { .. } => {
                wrap_catch_expression(function_call_expression, handling, result_type_ids)
            }
        }
    }
}

impl HandledFallibleCall {
    pub(crate) fn into_plain_expression(
        self,
        type_environment: &mut TypeEnvironment,
    ) -> Expression {
        Expression::function_call_with_typed_arguments(
            self.name,
            self.args,
            self.result_type_ids,
            type_environment,
            self.call_span,
        )
    }

    pub(crate) fn into_expression(
        self,
        handling: FallibleHandling,
        propagation_span: Option<SourceSpan>,
        type_environment: &mut TypeEnvironment,
    ) -> Expression {
        let expression_handling = match &handling {
            FallibleHandling::Propagate => FallibleExpressionHandling::Propagate,
            FallibleHandling::Handler { .. } => FallibleExpressionHandling::Recover,
        };
        let result_type_ids = self.result_type_ids.clone();
        let function_call_expression =
            Expression::handled_fallible_function_call_with_typed_arguments(
                self.name,
                self.args,
                self.result_type_ids,
                expression_handling,
                type_environment,
                self.call_span,
            );
        let function_call_expression = match propagation_span {
            Some(span) => function_call_expression.with_propagation_span(Some(span)),
            None => function_call_expression,
        };

        match handling {
            FallibleHandling::Propagate => function_call_expression,
            FallibleHandling::Handler { .. } => {
                wrap_catch_expression(function_call_expression, handling, result_type_ids)
            }
        }
    }
}

pub(crate) fn parse_fallible_handling_suffix_for_expression(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression: Expression,
    value_required: bool,
    allow_boundary_catch: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Expression, ExpressionParseError> {
    let expression_type_id = expression.type_id;
    let type_environment = type_interner.environment();

    // Extract the error-return type from the expression's fallible carrier.
    // The success-slot type IDs are computed separately because they drive the
    // catch-handler's value-production target, not the propagation check.
    let Some((_success_type_ids, error_return_type_id)) =
        type_environment.fallible_carrier_slots(expression_type_id)
    else {
        let operand_is_optional = type_environment.is_option(expression_type_id);
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            super::non_fallible_handler_reason(
                token_stream.current_token_kind(),
                operand_is_optional,
            ),
            Some(token_stream.current_span()),
        )
        .into());
    };

    let success_result_type_ids = fallible_success_type_ids(expression_type_id, type_environment);

    // Collapse single or zero success types into a concrete type for the
    // resulting handled expression. Multi-value successes become a tuple.
    let handled_type_id = match success_result_type_ids.as_slice() {
        [] => type_environment.builtins().none,
        [single] => *single,
        multiple => type_interner
            .environment_mut_for_derived_types()
            .intern_tuple(multiple.to_vec()),
    };

    let success_type_diagnostic_spelling =
        diagnostic_type_spelling(handled_type_id, type_interner.environment());

    let propagation_span = (token_stream.current_token_kind() == &TokenKind::Bang)
        .then(|| token_stream.current_postfix_operator_span());

    if let Some(handling) = parse_fallible_handling_suffix(
        token_stream,
        context,
        type_interner,
        FallibleHandlingSite {
            success_result_type_ids: &success_result_type_ids,
            error_return_type_id,
            value_required,
            value_required_span: expression.span,
            compilation_stage: EXPRESSION_STAGE,
            allow_boundary_catch,
        },
        None,
        string_table,
        path_fork,
    )? {
        let expression_span = expression.span;

        return Ok(match handling {
            FallibleHandling::Propagate => Expression::handled_result_with_type_id(
                expression,
                FallibleExpressionHandling::Propagate,
                handled_type_id,
                success_type_diagnostic_spelling,
                expression_span,
            )
            .with_propagation_span(Some(
                propagation_span.expect("propagation handling must have a postfix span"),
            )),

            FallibleHandling::Handler { .. } => {
                let handled_expression = Expression::handled_result_with_type_id(
                    expression,
                    FallibleExpressionHandling::Recover,
                    handled_type_id,
                    success_type_diagnostic_spelling,
                    expression_span,
                );

                wrap_catch_expression(handled_expression, handling, success_result_type_ids)
            }
        });
    }

    Ok(expression)
}

/// Returns whether `catch` handlers are syntactically permitted in the given scope context.
/// Returns whether `catch` handlers are syntactically permitted in the given scope context.
///
/// WHY: catch introduces a statement-like body block, so it is forbidden inside expression-only
/// contexts (conditions, templates, constants) where statements are not allowed.
pub(crate) fn fallible_catch_allowed_in_context(context: &ScopeContext) -> bool {
    !matches!(
        context.kind,
        ContextKind::Expression
            | ContextKind::Condition
            | ContextKind::Template
            | ContextKind::Constant
            | ContextKind::ConstantHeader
    )
}

fn parse_fallible_handling_suffix(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: FallibleHandlingSite<'_>,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Option<FallibleHandling>, ExpressionParseError> {
    match token_stream.current_token_kind() {
        TokenKind::Bang => {
            parse_postfix_propagation(token_stream, context, site, type_interner.environment())
                .map(Some)
        }
        TokenKind::Catch => parse_catch_handling_suffix(
            token_stream,
            context,
            type_interner,
            site,
            warnings,
            string_table,
            path_fork,
        )
        .map(Some),
        _ => Ok(None),
    }
}

fn parse_postfix_propagation(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    site: FallibleHandlingSite<'_>,
    type_environment: &TypeEnvironment,
) -> Result<FallibleHandling, ExpressionParseError> {
    let propagation_span = token_stream.current_postfix_operator_span();
    token_stream.advance();

    let Some(expected_error_type_id) = context.expected_error_type else {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::FunctionHasNoErrorSlot,
            Some(propagation_span),
        )
        .into());
    };

    if !is_postfix_error_compatible(
        expected_error_type_id,
        site.error_return_type_id,
        type_environment,
    ) {
        return Err(CompilerDiagnostic::type_mismatch(
            expected_error_type_id,
            site.error_return_type_id,
            TypeMismatchContext::ErrorReturn,
            Some(propagation_span),
        )
        .into());
    }

    Ok(FallibleHandling::Propagate)
}

/// Wraps value-producing catch recovery in the shared `ValueBlock` expression shape.
///
/// WHAT: only `catch` handlers become value blocks; postfix propagation stays an ordinary
/// handled fallible expression because it leaves the current function instead of recovering.
/// WHY: this keeps catch recovery on the same AST/HIR model as value `if` and match blocks
/// without changing statement-only catch behavior.
pub(crate) fn wrap_catch_expression(
    handled_expression: Expression,
    handler: FallibleHandling,
    result_type_ids: Vec<TypeId>,
) -> Expression {
    debug_assert!(matches!(handler, FallibleHandling::Handler { .. }));

    let span = handled_expression.span;
    let result_type_id = handled_expression.type_id;
    let diagnostic_type = handled_expression.diagnostic_type.to_owned();

    Expression::new(
        ExpressionKind::ValueBlock {
            block: Box::new(ValueBlock::Catch(ValueCatchBlock {
                handled_value: Box::new(handled_expression),
                handler,
                result_type_ids,
            })),
        },
        span,
        result_type_id,
        diagnostic_type,
        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
    )
}

fn parse_catch_handling_suffix(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: FallibleHandlingSite<'_>,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<FallibleHandling, ExpressionParseError> {
    if !site.allow_boundary_catch {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::CatchOutsideBoundary,
            Some(token_stream.current_span()),
        )
        .into());
    }

    token_stream.advance();
    let catch_context = context.activate_pending_catch_assignment_targets();

    match token_stream.current_token_kind() {
        // `catch then ...` — inline value-producing fallback with no error binding.
        TokenKind::Then => parse_inline_catch_without_error_binding(
            token_stream,
            &catch_context,
            type_interner,
            site,
            string_table,
            path_fork,
        ),

        // `catch:` — no error binding, just a fallback body.
        TokenKind::Colon => parse_catch_without_error_binding(
            token_stream,
            &catch_context,
            type_interner,
            site,
            warnings,
            string_table,
            path_fork,
        ),

        // `catch |err|:` or `catch |err| then ...` — bind the error value before recovery.
        TokenKind::TypeParameterBracket => parse_catch_handler(
            token_stream,
            &catch_context,
            type_interner,
            site,
            warnings,
        string_table,
        path_fork,
    ),

        _ => Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchBlockOrHandler,
            Some(token_stream.current_span()),
        )
        .into()),
    }
}

fn parse_inline_catch_without_error_binding(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: FallibleHandlingSite<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<FallibleHandling, ExpressionParseError> {
    let CatchFallibleHandler { error, body } = parse_inline_catch_without_error_binding_typed(
        token_stream,
        context,
        type_interner,
        CatchFallibleHandlerSite {
            success_result_type_ids: site.success_result_type_ids,
            error_return_type_id: site.error_return_type_id,
            value_required: site.value_required,
            compilation_stage: site.compilation_stage,
            value_required_span: site.value_required_span,
        },
        string_table,
        path_fork,
    )?;

    Ok(FallibleHandling::Handler { error, body })
}

/// Delegates to `parse_catch_without_error_binding_typed` for `catch:` (no error binding).
fn parse_catch_without_error_binding(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: FallibleHandlingSite<'_>,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<FallibleHandling, ExpressionParseError> {
    let CatchFallibleHandler { error, body } = parse_catch_without_error_binding_typed(
        token_stream,
        context,
        type_interner,
        CatchFallibleHandlerSite {
            success_result_type_ids: site.success_result_type_ids,
            error_return_type_id: site.error_return_type_id,
            value_required: site.value_required,
            compilation_stage: site.compilation_stage,
            value_required_span: site.value_required_span,
        },
        warnings,
        string_table,
        path_fork,
    )?;

    Ok(FallibleHandling::Handler { error, body })
}

/// Delegates to the block or inline binding parser for `catch |err|`.
fn parse_catch_handler(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: FallibleHandlingSite<'_>,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<FallibleHandling, ExpressionParseError> {
    let handler_site = CatchFallibleHandlerSite {
        success_result_type_ids: site.success_result_type_ids,
        error_return_type_id: site.error_return_type_id,
        value_required: site.value_required,
        compilation_stage: site.compilation_stage,
        value_required_span: site.value_required_span,
    };

    let CatchFallibleHandler { error, body } = if next_catch_binding_is_inline(token_stream) {
        parse_inline_catch_fallible_handler_typed(
            token_stream,
            context,
            type_interner,
            handler_site,
            warnings,
            string_table,
            path_fork,
        )?
    } else {
        parse_catch_fallible_handler_typed(
            token_stream,
            context,
            type_interner,
            handler_site,
            warnings,
            string_table,
            path_fork,
        )?
    };

    Ok(FallibleHandling::Handler { error, body })
}

fn next_catch_binding_is_inline(token_stream: &FileTokens) -> bool {
    let mut index = token_stream.index + 1;

    while index < token_stream.length {
        match &token_stream.tokens[index].kind {
            TokenKind::TypeParameterBracket => {
                return token_stream
                    .tokens
                    .iter()
                    .skip(index + 1)
                    .find(|token| token.kind != TokenKind::Newline)
                    .is_some_and(|token| token.kind == TokenKind::Then);
            }

            TokenKind::Newline | TokenKind::End | TokenKind::Eof => return false,

            _ => index += 1,
        }
    }

    false
}

// --------------------------
//  Catch handler call parsing
// --------------------------

pub(crate) fn parse_fallible_handling_suffix_for_call_expression(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    handler_call: FallibleCallSite,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Expression, ExpressionParseError> {
    let FallibleCallSite {
        call,
        error_return_type_id,
        value_required,
        allow_boundary_catch,
    } = handler_call;

    let propagation_span = (token_stream.current_token_kind() == &TokenKind::Bang)
        .then(|| token_stream.current_postfix_operator_span());

    let handling = parse_fallible_handling_suffix(
        token_stream,
        context,
        type_interner,
        FallibleHandlingSite {
            success_result_type_ids: &call.result_type_ids,
            error_return_type_id,
            value_required,
            value_required_span: call.call_span,
            compilation_stage: FUNCTION_CALL_STAGE,
            allow_boundary_catch,
        },
        warnings,
        string_table,
        path_fork,
    )?;

    let Some(handling) = handling else {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchBlockOrHandler,
            Some(token_stream.current_span()),
        )
        .into());
    };

    Ok(call.into_expression(
        handling,
        propagation_span,
        type_interner.environment_mut_for_derived_types(),
    ))
}

pub(crate) fn parse_fallible_handling_suffix_for_host_call_expression(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    handler_call: FallibleHostCallSite,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Expression, ExpressionParseError> {
    let FallibleHostCallSite {
        call,
        value_required,
        allow_boundary_catch,
    } = handler_call;

    let propagation_span = (token_stream.current_token_kind() == &TokenKind::Bang)
        .then(|| token_stream.current_postfix_operator_span());

    let handling = parse_fallible_handling_suffix(
        token_stream,
        context,
        type_interner,
        FallibleHandlingSite {
            success_result_type_ids: &call.result_type_ids,
            error_return_type_id: call.error_type_id,
            value_required,
            value_required_span: call.call_span,
            compilation_stage: FUNCTION_CALL_STAGE,
            allow_boundary_catch,
        },
        warnings,
        string_table,
        path_fork,
    )?;

    let Some(handling) = handling else {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchBlockOrHandler,
            Some(token_stream.current_span()),
        )
        .into());
    };

    Ok(call.into_expression(
        handling,
        propagation_span,
        type_interner.environment_mut_for_derived_types(),
    ))
}

/// Input bundle for `parse_cast_catch_handling_suffix`.
///
/// WHAT: carries the success type and boundary permission for a `cast ... catch:` handler.
/// WHY: cast failure is not a `Result`-typed value, so the shared fallible-handling parser
///      needs a small, cast-specific site description.
pub(crate) struct CastCatchSite {
    pub(crate) success_type_id: TypeId,
    pub(crate) error_type_id: TypeId,
    pub(crate) value_required_span: Option<SourceSpan>,
    pub(crate) allow_boundary_catch: bool,
}

/// Parses a `catch` recovery suffix for a fallible `cast` expression.
///
/// WHAT: reuses the shared catch-handler parser with a single success slot (the cast target)
///      and the cast failure error type.
/// WHY: cast recovery uses the same surface syntax as fallible calls, but the error value is
///      supplied by the selected cast evidence rather than a Result carrier.
pub(crate) fn parse_cast_catch_handling_suffix(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: CastCatchSite,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<FallibleHandling, ExpressionParseError> {
    let success_type_ids = [site.success_type_id];
    parse_catch_handling_suffix(
        token_stream,
        context,
        type_interner,
        FallibleHandlingSite {
            success_result_type_ids: &success_type_ids,
            error_return_type_id: site.error_type_id,
            value_required: true,
            value_required_span: site.value_required_span,
            compilation_stage: EXPRESSION_STAGE,
            allow_boundary_catch: site.allow_boundary_catch,
        },
        None,
        string_table,
        path_fork,
    )
}
