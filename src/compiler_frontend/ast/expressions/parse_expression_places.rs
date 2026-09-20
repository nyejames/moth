//! Place-sensitive expression parsing helpers.
//!
//! WHAT: parses place-sensitive expression forms such as `copy` and mutable receiver syntax.
//! WHY: place rules differ from general expression parsing and benefit from one focused module.

use super::error::ExpressionParseError;
use super::expression_rpn::ExpressionRpnItem;
use super::parse_expression_dispatch::push_expression_operand;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression::{
    ConstRecordState, Expression, ExpressionKind,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    PlaceExpression, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::field_access::{
    PostfixChainAccess, parse_field_access_expression_with_receiver_access,
    parse_postfix_chain_expression, reference_expression_from_declaration,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_or_dispatch_mismatch_for_tag,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticToken, InvalidAssignmentTargetReason, InvalidCopyTargetReason,
    InvalidReceiverCallReason, NameNamespace,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

pub(super) struct ParsedCopyPlace {
    pub(super) place: PlaceExpression,
    pub(super) diagnostic_type: DataType,
    pub(super) type_id: TypeId,
}

// WHAT: parses a `~name.<chain>` receiver expression.
// WHY: mutable receiver syntax is a distinct place expression that must resolve to a field-access
//      chain so the backend can pass the receiver by mutable reference.
pub(super) fn parse_mutable_receiver_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    let marker_span = Some(token_stream.current_postfix_operator_span());
    token_stream.advance();

    if token_stream.current_tag() != TokenTag::SYMBOL {
        let found = token_stream
            .current_diagnostic_token(string_table)
            .map_err(|error| {
                crate::compiler_frontend::compiler_messages::CompilerDiagnostic::token_view_invariant_error(
                    error,
                    "mutable-receiver symbol diagnostic",
                )
            })?
            .unwrap_or_else(|| DiagnosticToken::from_static_tag(token_stream.current_tag()));
        return Err(CompilerDiagnostic::unexpected_token_from_tag(found, marker_span).into());
    }
    let symbol_id = token_stream
        .current_string_id_in(string_table)?
        .ok_or_else(|| {
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                "mutable receiver symbol had no string payload",
            )
        })?;

    let Some(receiver_declaration) = context.get_reference(&symbol_id) else {
        if context.is_visible_type_alias_name(symbol_id) {
            return Err(CompilerDiagnostic::namespace_misuse(
                symbol_id,
                NameNamespace::Value,
                NameNamespace::Type,
                current_span(token_stream),
            )
            .into());
        }
        return Err(
            CompilerDiagnostic::unknown_value_name(symbol_id, current_span(token_stream)).into(),
        );
    };

    // The mutable marker must be followed by a field-access chain; bare `~name` is not valid.
    // When the author wrote `~name = ...`, the intent was an assignment target, not a receiver
    // call, so report the assignment-target reason instead of the receiver-call reason.
    if token_stream.peek_next_tag() != Some(TokenTag::DOT) {
        if token_stream
            .peek_next_tag()
            .is_some_and(TokenTag::is_assignment_operator)
        {
            return Err(CompilerDiagnostic::invalid_assignment_target(
                InvalidAssignmentTargetReason::MutableMarkerOnAssignmentTarget,
                None,
                None,
                None,
                None,
                None,
                marker_span,
            )
            .into());
        }
        return Err(CompilerDiagnostic::invalid_receiver_call(
            InvalidReceiverCallReason::MutableMarkerOnNonReceiverCall,
            None,
            None,
            None,
            None,
            marker_span,
        )
        .into());
    }

    token_stream.advance();
    let receiver_expression = parse_field_access_expression_with_receiver_access(
        token_stream,
        receiver_declaration.as_declaration(),
        context,
        PostfixChainAccess::mutable_marker(marker_span),
        type_interner,
        string_table,
        path_fork,
    )?;

    push_expression_operand(
        token_stream,
        context,
        type_interner,
        string_table,
        expression,
        allow_boundary_catch,
        receiver_expression,
        path_fork,
    )
}

// WHAT: parses the operand of a `copy` expression, which must resolve to a place.
// WHY: `copy` clones the current stored value at a place; arbitrary expressions do not have
//      stable storage, so the parser restricts this to names and parenthesized places.
pub(super) fn parse_copy_place_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedCopyPlace, ExpressionParseError> {
    parse_copy_place_payload(
        token_stream,
        context,
        type_interner,
        string_table,
        path_fork,
    )
}

fn parse_copy_place_payload(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedCopyPlace, ExpressionParseError> {
    match token_stream.current_tag() {
        TokenTag::OPEN_PARENTHESIS => {
            let open_span = current_span(token_stream);
            token_stream.advance();

            let mut parsed_place = parse_copy_place_payload(
                token_stream,
                context,
                type_interner,
                string_table,
                path_fork,
            )?;

            if token_stream.current_tag() != TokenTag::CLOSE_PARENTHESIS {
                let found = token_stream
                    .current_diagnostic_token(string_table)
                    .map_err(|error| {
                        CompilerDiagnostic::token_view_invariant_error(
                            error,
                            "copy-place closing-delimiter diagnostic",
                        )
                    })?;
                return Err(CompilerDiagnostic::expected_token_from_tags(
                    TokenTag::CLOSE_PARENTHESIS,
                    found,
                    current_span(token_stream),
                )
                .into());
            }

            token_stream.advance();
            parsed_place.place.span = open_span;
            Ok(parsed_place)
        }

        TokenTag::SYMBOL => {
            let symbol_id = token_stream
                .current_string_id_in(string_table)?
                .ok_or_else(|| {
                    crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                        "copy-place symbol had no string payload",
                    )
                })?;
            let Some(place_declaration) = context.get_reference(&symbol_id) else {
                if context.is_visible_type_alias_name(symbol_id) {
                    return Err(CompilerDiagnostic::namespace_misuse(
                        symbol_id,
                        NameNamespace::Value,
                        NameNamespace::Type,
                        current_span(token_stream),
                    )
                    .into());
                }
                return Err(CompilerDiagnostic::unknown_value_name(
                    symbol_id,
                    current_span(token_stream),
                )
                .into());
            };

            if context
                .source_callable_signature(place_declaration.as_declaration())
                .is_some()
            {
                let reason = if token_stream.peek_next_tag() == Some(TokenTag::OPEN_PARENTHESIS) {
                    InvalidCopyTargetReason::FunctionCall
                } else {
                    InvalidCopyTargetReason::FunctionName
                };
                Err(
                    CompilerDiagnostic::invalid_copy_target(reason, current_span(token_stream))
                        .into(),
                )
            } else {
                let reference_span = current_span(token_stream);
                let reference_expression = reference_expression_from_declaration(
                    place_declaration.as_declaration(),
                    context,
                    type_interner,
                    reference_span,
                );
                token_stream.advance();

                let copied_expression = if token_stream.position() < token_stream.length()
                    && token_stream.current_tag() == TokenTag::DOT
                {
                    parse_postfix_chain_expression(
                        token_stream,
                        reference_expression,
                        reference_span,
                        PostfixChainAccess::shared(),
                        context,
                        type_interner,
                        string_table,
                        path_fork,
                    )?
                } else {
                    reference_expression
                };

                let Some(place) = place_expression_from_expression(&copied_expression) else {
                    return Err(CompilerDiagnostic::invalid_copy_target(
                        InvalidCopyTargetReason::NonPlace,
                        copied_expression.span,
                    )
                    .into());
                };

                Ok(ParsedCopyPlace {
                    place,
                    diagnostic_type: copied_expression.diagnostic_type,
                    type_id: copied_expression.type_id,
                })
            }
        }

        TokenTag::MUST | TokenTag::TRAIT_THIS => {
            let keyword = reserved_trait_keyword_or_dispatch_mismatch_for_tag(
                token_stream.current_tag(),
                current_span(token_stream),
                "Expression Parsing",
                "copy-place parsing",
            )?;

            Err(reserved_trait_keyword_error(keyword, current_span(token_stream)).into())
        }

        TokenTag::MUTABLE => {
            let marker_span = current_span(token_stream);
            token_stream.advance();

            match parse_copy_place_payload(
                token_stream,
                context,
                type_interner,
                string_table,
                path_fork,
            ) {
                Ok(_) => Err(CompilerDiagnostic::invalid_copy_target(
                    InvalidCopyTargetReason::MutableMarkerNotAllowed,
                    marker_span,
                )
                .into()),
                Err(existing_error) => Err(existing_error),
            }
        }

        _ => Err(CompilerDiagnostic::invalid_copy_target(
            InvalidCopyTargetReason::NonPlace,
            current_span(token_stream),
        )
        .into()),
    }
}

pub(crate) fn place_expression_from_expression(expression: &Expression) -> Option<PlaceExpression> {
    match &expression.kind {
        ExpressionKind::Reference(path) => Some(PlaceExpression {
            kind: PlaceExpressionKind::Local(*path),
            type_id: expression.type_id,
            diagnostic_type: expression.diagnostic_type.clone(),
            value_mode: expression.value_mode.clone(),
            span: expression.span,
        }),

        ExpressionKind::FieldAccess { base, field } => {
            let base_place = place_expression_from_expression(base)?;
            Some(PlaceExpression {
                kind: PlaceExpressionKind::Field {
                    base: Box::new(base_place),
                    field: *field,
                },
                type_id: expression.type_id,
                diagnostic_type: expression.diagnostic_type.clone(),
                value_mode: expression.value_mode.clone(),
                span: expression.span,
            })
        }

        _ => None,
    }
}

/// Returns true when the place expression resolves to a mutable root place.
///
/// WHAT: a local place is mutable when its value mode says so; a field place is mutable when
///       the base place it projects from is mutable.
/// WHY: mutability for field projections is inherited from the root local/receiver, matching the
///      language rule that `~obj.field.method()` requires `obj` to be mutable.
pub(crate) fn place_expression_is_mutable(place: &PlaceExpression) -> bool {
    match &place.kind {
        PlaceExpressionKind::Local(_) => place.value_mode.is_mutable(),

        PlaceExpressionKind::Field { base, .. } => place_expression_is_mutable(base),
    }
}

/// Walks a place expression to its root local and returns the binding name.
///
/// WHAT: follows field-projection bases down to the underlying local, then returns its name.
/// WHY: immutable field-write diagnostics name the root binding that must be made mutable.
pub(crate) fn root_binding_name_of_place(
    place: &PlaceExpression,
    path_fork: &PathInternerFork,
) -> Option<StringId> {
    match &place.kind {
        PlaceExpressionKind::Local(path) => path_fork.component(*path),
        PlaceExpressionKind::Field { base, .. } => root_binding_name_of_place(base, path_fork),
    }
}

/// Reconstructs an expression payload from a narrow place expression.
///
/// WHAT: compound assignment desugars `target op rhs` by reading the target place as a value.
/// WHY: places do not carry enough metadata to be evaluated directly, so this builds the equivalent
///      expression tree (local reference or field access) that the evaluator already understands.
pub(crate) fn expression_from_place_expression(place: &PlaceExpression) -> Expression {
    let kind = match &place.kind {
        PlaceExpressionKind::Local(path) => ExpressionKind::Reference(*path),

        PlaceExpressionKind::Field { base, field } => ExpressionKind::FieldAccess {
            base: Box::new(expression_from_place_expression(base)),
            field: *field,
        },
    };

    let mut expression = Expression::new(
        kind,
        place.span,
        place.type_id,
        place.diagnostic_type.clone(),
        place.value_mode.clone(),
    );
    expression.const_record_state = ConstRecordState::RuntimeValue;
    expression
}

fn current_span(token_stream: &AstCursor) -> Option<SourceSpan> {
    Some(token_stream.current_span())
}
