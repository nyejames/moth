//! Place-sensitive expression parsing helpers.
//!
//! WHAT: parses place-sensitive expression forms such as `copy` and mutable receiver syntax.
//! WHY: place rules differ from general expression parsing and benefit from one focused module.

use super::error::ExpressionParseError;
use super::expression_rpn::ExpressionRpnItem;
use super::parse_expression_dispatch::push_expression_operand;
use crate::compiler_frontend::ast::ScopeContext;
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
    reserved_trait_keyword_error, reserved_trait_keyword_or_dispatch_mismatch,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidAssignmentTargetReason, InvalidCopyTargetReason,
    InvalidReceiverCallReason, NameNamespace,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

pub(super) struct ParsedCopyPlace {
    pub(super) place: PlaceExpression,
    pub(super) diagnostic_type: DataType,
    pub(super) type_id: TypeId,
}

// WHAT: parses a `~name.<chain>` receiver expression.
// WHY: mutable receiver syntax is a distinct place expression that must resolve to a field-access
//      chain so the backend can pass the receiver by mutable reference.
pub(super) fn parse_mutable_receiver_expression(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    string_table: &mut StringTable,
) -> Result<(), ExpressionParseError> {
    let marker_location = token_stream.current_location();
    token_stream.advance();

    let TokenKind::Symbol(symbol_id) = token_stream.current_token_kind().to_owned() else {
        return Err(CompilerDiagnostic::unexpected_token(
            token_stream.current_token_kind().to_owned(),
            marker_location,
        )
        .into());
    };

    let Some(receiver_declaration) = context.get_reference(&symbol_id) else {
        if context.is_visible_type_alias_name(symbol_id) {
            return Err(CompilerDiagnostic::namespace_misuse(
                symbol_id,
                NameNamespace::Value,
                NameNamespace::Type,
                token_stream.current_location(),
            )
            .into());
        }
        return Err(CompilerDiagnostic::unknown_value_name(
            symbol_id,
            token_stream.current_location(),
        )
        .into());
    };

    // The mutable marker must be followed by a field-access chain; bare `~name` is not valid.
    // When the author wrote `~name = ...`, the intent was an assignment target, not a receiver
    // call, so report the assignment-target reason instead of the receiver-call reason.
    if token_stream.peek_next_token() != Some(&TokenKind::Dot) {
        if token_stream
            .peek_next_token()
            .is_some_and(|token| token.is_assignment_operator())
        {
            return Err(CompilerDiagnostic::invalid_assignment_target(
                InvalidAssignmentTargetReason::MutableMarkerOnAssignmentTarget,
                None,
                None,
                None,
                None,
                None,
                marker_location,
            )
            .into());
        }
        return Err(CompilerDiagnostic::invalid_receiver_call(
            InvalidReceiverCallReason::MutableMarkerOnNonReceiverCall,
            None,
            None,
            None,
            None,
            marker_location,
        )
        .into());
    }

    token_stream.advance();
    let receiver_expression = parse_field_access_expression_with_receiver_access(
        token_stream,
        receiver_declaration.as_declaration(),
        context,
        PostfixChainAccess::mutable_marker(marker_location),
        type_interner,
        string_table,
    )?;

    push_expression_operand(
        token_stream,
        context,
        type_interner,
        string_table,
        expression,
        allow_boundary_catch,
        receiver_expression,
    )
}

// WHAT: parses the operand of a `copy` expression, which must resolve to a place.
// WHY: `copy` clones the current stored value at a place; arbitrary expressions do not have
//      stable storage, so the parser restricts this to names and parenthesized places.
pub(super) fn parse_copy_place_expression(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<ParsedCopyPlace, ExpressionParseError> {
    parse_copy_place_payload(token_stream, context, type_interner, string_table)
}

fn parse_copy_place_payload(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<ParsedCopyPlace, ExpressionParseError> {
    match token_stream.current_token_kind() {
        // Parenthesized places are allowed for grouping; the outer `(` location is preserved
        // so diagnostics point at the `copy(` call site rather than the inner name.
        TokenKind::OpenParenthesis => {
            let open_location = token_stream.current_location();
            token_stream.advance();

            let mut parsed_place =
                parse_copy_place_payload(token_stream, context, type_interner, string_table)?;

            if token_stream.current_token_kind() != &TokenKind::CloseParenthesis {
                return Err(CompilerDiagnostic::expected_token(
                    TokenKind::CloseParenthesis,
                    Some(token_stream.current_token_kind().to_owned()),
                    token_stream.current_location(),
                )
                .into());
            }

            token_stream.advance();
            parsed_place.place.location = open_location;
            Ok(parsed_place)
        }

        // Named places: resolve the symbol and verify it denotes a place-capable value, not a function.
        TokenKind::Symbol(symbol_id) => {
            let Some(place_declaration) = context.get_reference(symbol_id) else {
                if context.is_visible_type_alias_name(*symbol_id) {
                    return Err(CompilerDiagnostic::namespace_misuse(
                        *symbol_id,
                        NameNamespace::Value,
                        NameNamespace::Type,
                        token_stream.current_location(),
                    )
                    .into());
                }
                return Err(CompilerDiagnostic::unknown_value_name(
                    *symbol_id,
                    token_stream.current_location(),
                )
                .into());
            };

            if context
                .source_callable_signature(place_declaration.as_declaration())
                .is_some()
            {
                // A bare function name is not a copyable value; a call returns a value that
                // must be received in a binding first. Distinguish the two by checking whether
                // `(` follows the name, since `token_stream` is still positioned at the symbol.
                let reason = if token_stream.peek_next_token() == Some(&TokenKind::OpenParenthesis)
                {
                    InvalidCopyTargetReason::FunctionCall
                } else {
                    InvalidCopyTargetReason::FunctionName
                };
                Err(CompilerDiagnostic::invalid_copy_target(
                    reason,
                    token_stream.current_location(),
                )
                .into())
            } else {
                let reference_location = token_stream.current_location();
                let reference_expression = reference_expression_from_declaration(
                    place_declaration.as_declaration(),
                    context,
                    type_interner,
                    reference_location.clone(),
                );
                token_stream.advance();

                let copied_expression = if token_stream.index < token_stream.length
                    && token_stream.current_token_kind() == &TokenKind::Dot
                {
                    parse_postfix_chain_expression(
                        token_stream,
                        reference_expression,
                        reference_location,
                        PostfixChainAccess::shared(),
                        context,
                        type_interner,
                        string_table,
                    )?
                } else {
                    reference_expression
                };

                let Some(place) = place_expression_from_expression(&copied_expression) else {
                    return Err(CompilerDiagnostic::invalid_copy_target(
                        InvalidCopyTargetReason::NonPlace,
                        copied_expression.location,
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

        // Reserved trait keywords are not valid place expressions.
        TokenKind::Must | TokenKind::TraitThis => {
            let keyword = reserved_trait_keyword_or_dispatch_mismatch(
                token_stream.current_token_kind(),
                token_stream.current_location(),
                "Expression Parsing",
                "copy-place parsing",
            )?;

            Err(reserved_trait_keyword_error(keyword, token_stream.current_location()).into())
        }

        // `copy` does not take the `~` mutable-access marker. When `~` precedes
        // an otherwise valid binding or field projection, the marker is just
        // unnecessary. When the operand is itself invalid (literal, call, etc.),
        // the recursive call returns the existing factual diagnostic, since
        // removing `~` would not make it copyable.
        TokenKind::Mutable => {
            let marker_location = token_stream.current_location();
            token_stream.advance();

            match parse_copy_place_payload(token_stream, context, type_interner, string_table) {
                Ok(_) => Err(CompilerDiagnostic::invalid_copy_target(
                    InvalidCopyTargetReason::MutableMarkerNotAllowed,
                    marker_location,
                )
                .into()),
                Err(existing_error) => Err(existing_error),
            }
        }

        // Any other token cannot begin a place expression.
        //
        // `copy` requires a binding or field projection, not a literal, call,
        // or computed expression. We emit a targeted diagnostic so the user
        // understands *why* `copy` rejected this token, not just that it was
        // unexpected.
        _ => Err(CompilerDiagnostic::invalid_copy_target(
            InvalidCopyTargetReason::NonPlace,
            token_stream.current_location(),
        )
        .into()),
    }
}

pub(crate) fn place_expression_from_expression(expression: &Expression) -> Option<PlaceExpression> {
    match &expression.kind {
        ExpressionKind::Reference(path) => Some(PlaceExpression {
            kind: PlaceExpressionKind::Local(path.clone()),
            type_id: expression.type_id,
            diagnostic_type: expression.diagnostic_type.clone(),
            value_mode: expression.value_mode.clone(),
            location: expression.location.clone(),
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
                location: expression.location.clone(),
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
pub(crate) fn root_binding_name_of_place(place: &PlaceExpression) -> Option<StringId> {
    match &place.kind {
        PlaceExpressionKind::Local(path) => path.name(),
        PlaceExpressionKind::Field { base, .. } => root_binding_name_of_place(base),
    }
}

/// Reconstructs an expression payload from a narrow place expression.
///
/// WHAT: compound assignment desugars `target op rhs` by reading the target place as a value.
/// WHY: places do not carry enough metadata to be evaluated directly, so this builds the equivalent
///      expression tree (local reference or field access) that the evaluator already understands.
pub(crate) fn expression_from_place_expression(place: &PlaceExpression) -> Expression {
    let kind = match &place.kind {
        PlaceExpressionKind::Local(path) => ExpressionKind::Reference(path.clone()),

        PlaceExpressionKind::Field { base, field } => ExpressionKind::FieldAccess {
            base: Box::new(expression_from_place_expression(base)),
            field: *field,
        },
    };

    let mut expression = Expression::new(
        kind,
        place.location.clone(),
        place.type_id,
        place.diagnostic_type.clone(),
        place.value_mode.clone(),
    );
    expression.const_record_state = ConstRecordState::RuntimeValue;
    expression
}
