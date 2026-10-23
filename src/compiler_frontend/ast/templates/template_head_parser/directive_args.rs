//! Centralized template-head directive argument parsing.
//!
//! WHAT:
//! - Shared helpers for parsing parenthesized arguments after `$directive` tokens.
//! - Directive-level validation for empty arguments and single-argument arity.
//!
//! WHY:
//! - The shared call-argument owner handles parentheses, separators, newlines,
//!   named-entry recognition, access markers, and expression boundaries.
//! - Directive modules retain semantic validation: type checking, compile-time
//!   restrictions, and normalization into template/style state.
//!
//! ## Ownership boundary
//!
//! - **This module** owns directive-level token lookahead and the one-argument
//!   arity/category adapter.
//! - **The shared call-argument owner** owns the parenthesized list syntax.
//! - **Slot modules** own slot schema and composition; they do not parse tokens.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::const_values::store::{ConstStringValue, ConstValuePayload};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::call_argument::CallAccessMode;
use crate::compiler_frontend::ast::expressions::call_arguments::{
    CallArgumentDiagnosticContext, CallArgumentNamingPolicy, CallArgumentReceivingContext,
    CallArgumentSyntax, CallArgumentSyntaxContext, CallArgumentValuePolicy,
    parse_call_arguments_with_receiving_context,
};
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExprId, ExpressionOperationView,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    templates, token_lexical,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticToken, InvalidTemplateDirectiveReason,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use moth_lexical::numeric::fixed_scalar::{FixedScalarClass, FixedScalarValue};

/// Typed result shared by directive-argument parsing helpers.
type DirectiveArgsResult<T> = Result<T, TemplateError>;
#[derive(Debug)]
pub(crate) enum RequiredDirectiveArgumentError {
    EmptyArguments(Option<SourceSpan>),
    Other(TemplateError),
}

impl RequiredDirectiveArgumentError {
    pub(crate) fn into_template_error(
        self,
        directive_name: StringId,
        empty_reason: InvalidTemplateDirectiveReason,
    ) -> TemplateError {
        match self {
            Self::EmptyArguments(span) => {
                let is_children_argument = matches!(
                    empty_reason,
                    InvalidTemplateDirectiveReason::InvalidChildrenArgument
                );
                let diagnostic =
                    templates::invalid_template_directive(Some(directive_name), empty_reason, span);
                if is_children_argument {
                    TemplateError::Diagnostic(diagnostic)
                } else {
                    TemplateError::EmptyArguments(diagnostic)
                }
            }
            Self::Other(TemplateError::EmptyArguments(diagnostic))
                if matches!(
                    empty_reason,
                    InvalidTemplateDirectiveReason::InvalidChildrenArgument
                ) =>
            {
                TemplateError::Diagnostic(templates::invalid_template_directive(
                    Some(directive_name),
                    InvalidTemplateDirectiveReason::InvalidChildrenArgument,
                    diagnostic.primary_span(),
                ))
            }
            Self::Other(error) => error,
        }
    }
}

impl From<TemplateError> for RequiredDirectiveArgumentError {
    fn from(error: TemplateError) -> Self {
        Self::Other(error)
    }
}

impl From<DiagnosticDraft> for RequiredDirectiveArgumentError {
    fn from(diagnostic: DiagnosticDraft) -> Self {
        Self::Other(diagnostic.into())
    }
}

/// Returns true if the next token after the current directive is `(`.
///
/// WHAT: pure read-only `(` lookahead on the canonical cursor view.
/// WHY: directive dispatch must not advance before committing to the paren
/// path; the cached cursor lookahead keeps that fact read-only.
pub(crate) fn directive_has_arguments(token_stream: &AstCursor) -> bool {
    token_stream.peek_next_tag() == Some(TokenTag::OPEN_PARENTHESIS)
}

/// Rejects parenthesized arguments for directives that do not accept them.
pub(crate) fn reject_unexpected_directive_arguments(
    directive_name: StringId,
    token_stream: &AstCursor,
) -> DirectiveArgsResult<()> {
    if directive_has_arguments(token_stream) {
        return Err(with_current_token_span(
            token_stream,
            templates::invalid_template_directive(
                Some(directive_name),
                InvalidTemplateDirectiveReason::UnexpectedArguments,
                None,
            ),
        )
        .into());
    }
    Ok(())
}

/// Identifies template boundaries that end a directive argument before its expression begins.
/// This keeps empty arguments with the directive parser while header balancing retains true EOF.
fn ends_directive_argument_without_expression(tag: TokenTag) -> bool {
    matches!(
        tag,
        TokenTag::TEMPLATE_CLOSE
            | TokenTag::START_TEMPLATE_BODY
            | TokenTag::COLON
            | TokenTag::CLOSE_CURLY
            | TokenTag::ELSE
            | TokenTag::END
    )
}
fn first_significant_argument_offset(token_stream: &AstCursor) -> usize {
    let mut offset = 1;
    while token_stream
        .token_ref_at_offset(offset)
        .is_some_and(|token| token.tag() == TokenTag::NEWLINE)
    {
        offset += 1;
    }
    offset
}

/// Parses one implemented-directive argument list through the shared call
/// argument owner, then applies the directive's positional single-argument
/// contract.
///
/// The caller stays on the directive token. The shared owner consumes the
/// opening and closing parentheses, so a successful return leaves the cursor
/// on the token after `)`.
#[allow(
    clippy::too_many_arguments,
    reason = "the shared owner keeps the directive name and parser borrows as separate inputs"
)]
fn parse_single_directive_argument(
    directive_name: StringId,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ExprId, RequiredDirectiveArgumentError> {
    // Move from the directive token to the opening delimiter. The shared
    // parser receives this exact cursor shape and consumes both delimiters.
    token_stream.advance();
    let first_argument_offset = first_significant_argument_offset(token_stream);
    let first_argument_tag = token_stream
        .token_ref_at_offset(first_argument_offset)
        .map(|token| token.tag())
        .unwrap_or(TokenTag::EOF);
    let first_argument_span = token_stream.span_at(first_argument_offset);

    // Preserve directive-owned empty and template-boundary diagnostics without
    // advancing and rewinding the cursor around the shared parser.
    if first_argument_tag == TokenTag::CLOSE_PARENTHESIS {
        return Err(RequiredDirectiveArgumentError::EmptyArguments(
            first_argument_span,
        ));
    }
    if ends_directive_argument_without_expression(first_argument_tag) {
        return Err(RequiredDirectiveArgumentError::EmptyArguments(
            first_argument_span,
        ));
    }
    let syntax = CallArgumentSyntax::Supported {
        callee_name: Some(directive_name),
    };
    let receiving_context = CallArgumentReceivingContext::with_policies(
        CallArgumentDiagnosticContext::from_syntax(syntax),
        CallArgumentNamingPolicy::PositionalOnly,
        CallArgumentValuePolicy::Ordinary,
    );
    let arguments = parse_call_arguments_with_receiving_context(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        receiving_context,
        None,
        CallArgumentSyntaxContext::Ordinary,
        path_fork,
    )
    .map_err(|error| {
        let error = TemplateError::from(error);
        let error = if token_stream.current_tag() == TokenTag::EOF {
            error.map_diagnostic(|diagnostic| {
                if diagnostic.code() == "MOTH-SYNTAX-0002" {
                    return with_current_token_span(
                        token_stream,
                        token_lexical::expected_token_from_projections(
                            DiagnosticToken::from_static_tag(TokenTag::CLOSE_PARENTHESIS),
                            Some(DiagnosticToken::from_static_tag(TokenTag::EOF)),
                            None,
                        ),
                    );
                }
                diagnostic
            })
        } else {
            error
        };
        RequiredDirectiveArgumentError::Other(error)
    })?;

    let Some(argument) = arguments.first() else {
        return Err(RequiredDirectiveArgumentError::EmptyArguments(Some(
            token_stream.current_span(),
        )));
    };

    if argument.access_mode != CallAccessMode::Shared {
        return Err(templates::invalid_template_directive(
            Some(directive_name),
            InvalidTemplateDirectiveReason::invalid_argument(),
            argument.marker_span.or(argument.span),
        )
        .into());
    }

    if arguments.len() > 1 {
        let diagnostic = token_lexical::unexpected_token_from_tag(
            DiagnosticToken::from_static_tag(TokenTag::COMMA),
            argument.following_separator_span.or(argument.span),
        );
        return Err(diagnostic.into());
    }

    Ok(arguments
        .into_iter()
        .next()
        .expect("validated directive argument list is non-empty")
        .value
        .id())
}

/// Parses an optional parenthesized compile-time expression after a directive.
///
/// Returns `Ok(None)` if no `(` follows the directive.
/// Returns `Ok(Some(expression))` if a single expression was parsed.
#[allow(
    clippy::too_many_arguments,
    reason = "optional parsing keeps the directive name and parser borrows as separate inputs"
)]
pub(crate) fn parse_optional_parenthesized_expression(
    directive_name: StringId,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DirectiveArgsResult<Option<ExprId>> {
    if !directive_has_arguments(token_stream) {
        return Ok(None);
    }

    parse_single_directive_argument(
        directive_name,
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )
    .map(Some)
    .map_err(|error| {
        error.into_template_error(
            directive_name,
            InvalidTemplateDirectiveReason::EmptyArguments,
        )
    })
}

/// Parses a required parenthesized compile-time expression after a directive.
///
/// Returns an error if no `(` follows the directive.
#[allow(
    clippy::too_many_arguments,
    reason = "required parsing keeps the directive name and parser borrows as separate inputs"
)]
pub(crate) fn parse_required_parenthesized_expression(
    directive_name: StringId,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ExprId, RequiredDirectiveArgumentError> {
    if !directive_has_arguments(token_stream) {
        let found = token_stream
            .current_diagnostic_token(string_table)
            .map_err(|error| {
                TemplateError::from(CompilerError::compiler_error(format!(
                    "template directive opening-paren diagnostic token payload was malformed: {error:?}"
                )))
            })?

            .or_else(|| Some(DiagnosticToken::from_static_tag(token_stream.current_tag())));
        return Err(with_current_token_span(
            token_stream,
            token_lexical::expected_token_from_projections(
                DiagnosticToken::from_static_tag(TokenTag::OPEN_PARENTHESIS),
                found,
                None,
            ),
        )
        .into());
    }

    parse_single_directive_argument(
        directive_name,
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )
}

// ----------------------------------------------------------------------------
// Slot-target argument parsers (moved from template_slots.rs)
// ----------------------------------------------------------------------------

/// Parses the optional argument to `$slot`: default (no parens), named string,
/// or positive positional integer.
///
/// The caller stays on the directive token. The shared owner consumes the
/// opening and closing parentheses, so a successful return leaves the cursor
/// on the token after `)`.
#[allow(
    clippy::too_many_arguments,
    reason = "optional slot targets keep the directive name and parser borrows as separate inputs"
)]
pub(crate) fn parse_optional_slot_target_argument(
    directive_name: StringId,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DirectiveArgsResult<SlotKey> {
    if !directive_has_arguments(token_stream) {
        return Ok(SlotKey::Default);
    }

    let expression = parse_single_directive_argument(
        directive_name,
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )
    .map_err(|error| {
        error.into_template_error(
            directive_name,
            InvalidTemplateDirectiveReason::EmptyArguments,
        )
    })?;

    let expression = expression_builder.view(expression)?;
    let payload = match expression.operation() {
        ExpressionOperationView::Constant(value) => {
            Some(const_values.payload(value).ok_or_else(|| {
                CompilerError::compiler_error(
                    "template expression references an absent constant value",
                )
            })?)
        }
        _ => None,
    };
    match payload {
        Some(ConstValuePayload::String(ConstStringValue::Text(name))) => Ok(SlotKey::Named(*name)),
        Some(ConstValuePayload::FixedScalar(value)) => match fixed_slot_position(*value) {
            Some(position) => Ok(SlotKey::Positional(position)),
            None => Err(with_current_token_span(
                token_stream,
                templates::invalid_template_directive(
                    Some(directive_name),
                    InvalidTemplateDirectiveReason::InvalidSlotTarget,
                    expression.span(),
                ),
            )
            .into()),
        },
        _ => Err(with_current_token_span(
            token_stream,
            templates::invalid_template_directive(
                Some(directive_name),
                InvalidTemplateDirectiveReason::InvalidSlotTarget,
                expression.span(),
            ),
        )
        .into()),
    }
}

/// Converts one positional slot-target literal into its positive position.
///
/// WHY: positions are authored as exact fixed-integer literals, so any integer width is
///      accepted while binary floats, `Byte` and non-positive values keep the ordinary
///      slot-target diagnostic.
fn fixed_slot_position(value: FixedScalarValue) -> Option<usize> {
    let position = match value.scalar().class() {
        FixedScalarClass::SignedInteger => i128::from(value.as_i64()?),
        FixedScalarClass::UnsignedInteger => i128::from(value.as_u64()?),
        FixedScalarClass::BinaryFloat | FixedScalarClass::Octet => return None,
    };
    usize::try_from(position)
        .ok()
        .filter(|position| *position > 0)
}

/// Parses the required named target argument to `$insert("name")`.
///
/// The caller stays on the directive token. The shared owner consumes the
/// opening and closing parentheses, so a successful return leaves the cursor
/// on the token after `)`.
#[allow(
    clippy::too_many_arguments,
    reason = "required slot names keep the directive name and parser borrows as separate inputs"
)]
pub(crate) fn parse_required_slot_name_argument(
    directive_name: StringId,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> DirectiveArgsResult<StringId> {
    if !directive_has_arguments(token_stream) {
        let found = token_stream
            .current_diagnostic_token(string_table)
            .map_err(|error| {
                CompilerError::compiler_error(format!(
                    "template insert opening-paren diagnostic token payload was malformed: {error:?}"
                ))
            })?
            .or_else(|| Some(DiagnosticToken::from_static_tag(token_stream.current_tag())));
        return Err(with_current_token_span(
            token_stream,
            token_lexical::expected_token_from_projections(
                DiagnosticToken::from_static_tag(TokenTag::OPEN_PARENTHESIS),
                found,
                None,
            ),
        )
        .into());
    }

    let expression = parse_single_directive_argument(
        directive_name,
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )
    .map_err(|error| {
        error.into_template_error(
            directive_name,
            InvalidTemplateDirectiveReason::EmptyArguments,
        )
    })?;

    let expression = expression_builder.view(expression)?;
    let payload = match expression.operation() {
        ExpressionOperationView::Constant(value) => {
            Some(const_values.payload(value).ok_or_else(|| {
                CompilerError::compiler_error(
                    "template expression references an absent constant value",
                )
            })?)
        }
        _ => None,
    };
    match payload {
        Some(ConstValuePayload::String(ConstStringValue::Text(name))) => Ok(*name),
        _ => Err(with_current_token_span(
            token_stream,
            templates::invalid_template_directive(
                Some(directive_name),
                InvalidTemplateDirectiveReason::InvalidInsertTarget,
                expression.span(),
            ),
        )
        .into()),
    }
}

/// Attach the authored span of the token that owns a directive syntax diagnostic.
///
/// Directive argument validation keeps the exact global span of the token
/// owning a syntax diagnostic.
fn with_current_token_span(
    token_stream: &AstCursor,
    mut diagnostic: DiagnosticDraft,
) -> DiagnosticDraft {
    if diagnostic.primary_span().is_none() {
        diagnostic.set_primary_span(Some(token_stream.current_span()));
    }
    diagnostic
}
