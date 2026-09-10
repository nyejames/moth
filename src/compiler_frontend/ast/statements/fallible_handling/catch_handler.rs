//! Catch fallible-handler parsing helpers.
//!
//! WHAT: parses `catch:` no-binding handlers and `catch |err|:` binding handlers, preserving
//! `then` value-production statements inside value-producing handler bodies.
//! WHY: call and expression fallible handling share identical catch-handler syntax and validation,
//! so this module removes duplicated parser paths.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression_types::CatchErrorBinding;
use crate::compiler_frontend::ast::function_body_to_ast;
use crate::compiler_frontend::ast::statements::value_production::{
    ActiveValueProductionTarget, ProducedValues, ProducedValuesParseInput, ValueReceiverKind,
    is_missing_produced_value_boundary, parse_produced_values_typed,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::ids::TypeId;

use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::value_mode::ValueMode;

use super::validation::{
    validate_catch_fallible_handler_binding, validate_catch_fallible_handler_conflict,
    validate_catch_fallible_handler_value_requirement,
};

pub(crate) struct CatchFallibleHandler {
    pub(crate) error: Option<CatchErrorBinding>,
    pub(crate) body: Vec<AstNode>,
}

pub(crate) struct CatchFallibleHandlerSite<'a> {
    pub(crate) success_result_type_ids: &'a [TypeId],
    pub(crate) error_return_type_id: TypeId,
    pub(crate) value_required: bool,
    pub(crate) compilation_stage: &'a str,
    pub(crate) value_required_span: Option<SourceSpan>,
}
struct ParsedCatchErrorBinding {
    binding: CatchErrorBinding,
    span: Option<SourceSpan>,
}

/// Parses a `catch |err|:` handler with an explicit error binding.
///
/// WHAT: consumes the `|identifier|` binding syntax and the subsequent handler body,
/// validates the binding name, and produces a typed `CatchFallibleHandler`.
/// WHY: expression and call fallible handling both use this entrypoint when the user
/// supplies an error variable.
pub(crate) fn parse_catch_fallible_handler_typed(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: CatchFallibleHandlerSite<'_>,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    string_table: &mut StringTable,
) -> Result<CatchFallibleHandler, ExpressionParseError> {
    let mut local_handler_warnings: Vec<CompilerDiagnostic> = Vec::new();
    let warnings = match warnings {
        Some(warnings) => warnings,
        None => &mut local_handler_warnings,
    };

    let error_binding =
        parse_catch_error_binding(token_stream, context, &site, warnings, string_table)?;

    parse_catch_fallible_handler_body(
        token_stream,
        context,
        type_interner,
        site,
        Some(error_binding),
        warnings,
        string_table,
    )
}

/// Parses a `catch:` handler without an error binding.
///
/// WHAT: skips error-variable parsing and proceeds directly to the handler body.
/// WHY: used when the catch handler does not need to reference the error value.
pub(crate) fn parse_catch_without_error_binding_typed(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: CatchFallibleHandlerSite<'_>,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    string_table: &mut StringTable,
) -> Result<CatchFallibleHandler, ExpressionParseError> {
    let mut local_handler_warnings: Vec<CompilerDiagnostic> = Vec::new();
    let warnings = match warnings {
        Some(warnings) => warnings,
        None => &mut local_handler_warnings,
    };

    parse_catch_fallible_handler_body(
        token_stream,
        context,
        type_interner,
        site,
        None,
        warnings,
        string_table,
    )
}

/// Parses the `|identifier|` error binding inside a catch handler.
///
/// WHAT: validates bracket tokens, extracts the handler identifier, runs naming and
/// scope-conflict checks, and returns a `CatchErrorBinding`.
/// WHY: both block and inline catch handlers share the same binding shape, but the
/// token after the closing pipe decides whether the handler body is `:` or `then`.
fn parse_catch_error_binding(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    site: &CatchFallibleHandlerSite<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> Result<ParsedCatchErrorBinding, ExpressionParseError> {
    if token_stream.current_token_kind() != &TokenKind::TypeParameterBracket {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchHandlerOpeningPipe,
            Some(token_stream.current_span()),
        )
        .into());
    }

    token_stream.advance();

    if token_stream.current_token_kind() == &TokenKind::TypeParameterBracket {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::EmptyCatchHandlerBinding,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let TokenKind::Symbol(handler_name) = token_stream.current_token_kind().to_owned() else {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchHandlerIdentifier,
            Some(token_stream.current_span()),
        )
        .into());
    };

    let handler_name_span = Some(token_stream.current_span());
    validate_catch_fallible_handler_binding(
        handler_name,
        handler_name_span,
        site.compilation_stage,
        warnings,
        string_table,
    )?;

    validate_catch_fallible_handler_conflict(context, handler_name, handler_name_span)?;

    token_stream.advance();

    if token_stream.current_token_kind() == &TokenKind::Comma {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::MultipleCatchHandlerBindings,
            Some(token_stream.current_span()),
        )
        .into());
    }

    if token_stream.current_token_kind() != &TokenKind::TypeParameterBracket {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchHandlerClosingPipe,
            Some(token_stream.current_span()),
        )
        .into());
    }

    token_stream.advance();

    Ok(ParsedCatchErrorBinding {
        binding: CatchErrorBinding {
            error_binding: context.scope.append(handler_name),
        },
        span: handler_name_span,
    })
}

/// Parses the colon and statement body of a catch handler.
///
/// WHAT: validates the leading colon, sets up a child control-flow context with the
/// error variable bound, parses the handler body, and validates value-production requirements.
/// WHY: this is the shared backend for both binding and no-binding catch handlers.
fn parse_catch_fallible_handler_body(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: CatchFallibleHandlerSite<'_>,
    error: Option<ParsedCatchErrorBinding>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> Result<CatchFallibleHandler, ExpressionParseError> {
    if token_stream.current_token_kind() != &TokenKind::Colon {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchHandlerColon,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let mut handler_context =
        context.new_child_control_flow(ContextKind::CatchHandler, string_table);
    if site.value_required {
        handler_context.active_value_target = Some(ActiveValueProductionTarget::known(
            site.success_result_type_ids.to_vec(),
            ValueReceiverKind::CatchHandler,
        ));
    }

    if let Some(error_binding) = &error {
        let error_data_type =
            diagnostic_type_spelling(site.error_return_type_id, type_interner.environment());
        let error_binding_span = error_binding.span;
        handler_context.add_var(
            Declaration {
                id: error_binding.binding.error_binding.to_owned(),
                value: Expression::no_value_with_type_id(
                    error_binding_span,
                    error_data_type,
                    site.error_return_type_id,
                    ValueMode::ImmutableOwned,
                ),
                binding_span: error_binding_span,
                config_qualifier: None,
            },
            error_binding_span,
        );
    }

    token_stream.advance();

    let handler_body = function_body_to_ast(
        token_stream,
        handler_context,
        type_interner,
        warnings,
        string_table,
    )?;

    validate_catch_fallible_handler_value_requirement(
        site.value_required,
        site.success_result_type_ids,
        &handler_body,
        site.value_required_span,
    )?;

    Ok(CatchFallibleHandler {
        error: error.map(|parsed| parsed.binding),
        body: handler_body,
    })
}

/// Parses inline `catch then ...` recovery without an error binding.
///
/// WHAT: builds the same one-node handler body as block-form `catch: then ... ;`.
/// WHY: inline catch is only sugar at receiving sites; keeping the body as `ThenValue`
/// lets AST/HIR reuse the value-block catch path without catch-specific fallback fields.
pub(super) fn parse_inline_catch_without_error_binding_typed(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: CatchFallibleHandlerSite<'_>,
    string_table: &mut StringTable,
) -> Result<CatchFallibleHandler, ExpressionParseError> {
    parse_inline_catch_handler_body(
        token_stream,
        context,
        type_interner,
        site,
        None,
        string_table,
    )
}

/// Parses inline `catch |err| then ...` recovery with an error binding.
///
/// WHAT: validates and binds `err`, then parses the single inline produced-value list.
/// WHY: the inline shorthand must preserve the same scoping and conflict rules as
/// block-form catch handlers.
pub(super) fn parse_inline_catch_fallible_handler_typed(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: CatchFallibleHandlerSite<'_>,
    warnings: Option<&mut Vec<CompilerDiagnostic>>,
    string_table: &mut StringTable,
) -> Result<CatchFallibleHandler, ExpressionParseError> {
    let mut local_handler_warnings: Vec<CompilerDiagnostic> = Vec::new();
    let warnings = match warnings {
        Some(warnings) => warnings,
        None => &mut local_handler_warnings,
    };
    let error_binding =
        parse_catch_error_binding(token_stream, context, &site, warnings, string_table)?;

    parse_inline_catch_handler_body(
        token_stream,
        context,
        type_interner,
        site,
        Some(error_binding),
        string_table,
    )
}

fn parse_inline_catch_handler_body(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    site: CatchFallibleHandlerSite<'_>,
    error: Option<ParsedCatchErrorBinding>,
    string_table: &mut StringTable,
) -> Result<CatchFallibleHandler, ExpressionParseError> {
    if token_stream.current_token_kind() == &TokenKind::Newline {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::InlineCatchMultiline,
            Some(token_stream.current_span()),
        )
        .into());
    }

    if token_stream.current_token_kind() != &TokenKind::Then {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchBlockOrHandler,
            Some(token_stream.current_span()),
        )
        .into());
    }

    if !site.value_required || site.success_result_type_ids.is_empty() {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::FallbackValuesForErrorOnlyResult,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let then_span = Some(token_stream.current_span());
    token_stream.advance();

    // A retained newline proves a multiline form. Other empty boundaries use the
    // shared missing-value diagnostic before expression evaluation.
    if token_stream.current_token_kind() == &TokenKind::Newline {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::InlineCatchMultiline,
            Some(token_stream.current_span()),
        )
        .into());
    }

    if is_missing_produced_value_boundary(token_stream.current_token_kind()) {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ThenRequiresValues,
            Some(token_stream.current_span()),
        )
        .into());
    }

    let mut handler_context =
        context.new_child_control_flow(ContextKind::CatchHandler, string_table);
    let active_target = ActiveValueProductionTarget::known(
        site.success_result_type_ids.to_vec(),
        ValueReceiverKind::CatchHandler,
    );
    handler_context.active_value_target = Some(active_target.clone());

    if let Some(error_binding) = &error {
        let error_data_type =
            diagnostic_type_spelling(site.error_return_type_id, type_interner.environment());
        let error_binding_span = error_binding.span;
        handler_context.add_var(
            Declaration {
                id: error_binding.binding.error_binding.to_owned(),
                value: Expression::no_value_with_type_id(
                    error_binding_span,
                    error_data_type,
                    site.error_return_type_id,
                    ValueMode::ImmutableOwned,
                ),
                binding_span: error_binding_span,
                config_qualifier: None,
            },
            error_binding_span,
        );
    }

    let produced_values = parse_produced_values_typed(ProducedValuesParseInput {
        token_stream,
        context: &handler_context,
        type_interner,
        target: &active_target,
        label: "inline catch fallback values",
        string_table,
    })?;

    if token_stream.current_token_kind() == &TokenKind::Catch {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::ExpectedCatchBlockOrHandler,
            Some(token_stream.current_span()),
        )
        .into());
    }
    let body = vec![AstNode {
        kind: NodeKind::ThenValue(ProducedValues {
            expressions: produced_values,
            span: then_span,
        }),
        span: then_span,
        scope: handler_context.scope.clone(),
    }];

    Ok(CatchFallibleHandler {
        error: error.map(|parsed| parsed.binding),
        body,
    })
}
