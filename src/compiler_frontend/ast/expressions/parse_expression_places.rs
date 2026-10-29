//! Place-sensitive expression parsing helpers.
//!
//! WHAT: parses place-sensitive forms with named or reserved receiver roots, including `copy`
//! and mutable receiver syntax.
//! WHY: place rules differ from general expression parsing, and each form keeps its own postfix
//! access policy after shared root resolution.

use super::error::ExpressionParseError;
use super::expression_rpn::ExpressionRpnItem;
use super::parse_expression_dispatch::{push_expression_operand, push_pending_numeric_binding};
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionConstructionFailure, ExpressionOp, ExpressionOperationView,
    OwnedExpr, PlaceId, PlaceProjection, PlaceProjectionView, PlaceRecord, PlaceView,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::field_access::{
    ParsedPostfixExpression, PostfixChainAccess, RetainedOperandInput,
    parse_field_access_expression_with_receiver_access, parse_postfix_chain_expression,
    reference_expression_from_declaration, retain_pending_operand_member_access,
};
use crate::compiler_frontend::ast::module_ast::scope_context::ValueDeclarationRef;
use crate::compiler_frontend::ast::numeric_construction::NumericBindingId;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_or_dispatch_mismatch_for_tag,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticToken, InvalidCopyTargetReason, InvalidReceiverCallReason, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

/// One parsed `copy` operand.
///
/// WHAT: either a ready place payload with its concrete type, or an eligible body-local numeric
///       binding whose single static type is not selected yet.
/// WHY: `copy` must retain the authored shape (a value copy of a place) until the body root
///      selects the binding's type; a place payload would require fabricating a `TypeId` and a
///      default value that the language has not selected.
pub(super) enum ParsedCopyPlace {
    /// A ready place payload with its concrete type.
    Ready { place: PlaceId, type_id: TypeId },
    /// A pending numeric binding reference whose copy completes at the body root.
    PendingNumeric {
        binding: NumericBindingId,
        span: Option<SourceSpan>,
    },
}

// WHAT: parses a mutable receiver expression rooted at a name or `this`.
// WHY: mutable receiver syntax is a distinct place expression that must resolve to a field-access
//      chain so the backend can pass the receiver by mutable reference.
#[allow(
    clippy::too_many_arguments,
    reason = "mutable receiver parsing keeps the token stream, scope, type interner, expression builder, const store, expression output, catch policy, string table, and path fork as separate borrows"
)]
pub(super) fn parse_mutable_receiver_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    let marker_span = Some(token_stream.current_span());
    token_stream.advance();

    if !matches!(
        token_stream.current_tag(),
        TokenTag::SYMBOL | TokenTag::THIS
    ) {
        return Err(calls_data::invalid_receiver_call(
            InvalidReceiverCallReason::MutableMarkerOnNonReceiverCall,
            None,
            None,
            None,
            None,
            marker_span,
        )
        .into());
    }
    let receiver_declaration = resolve_place_root(token_stream, context, string_table)?;

    // In expression position the marker still denotes a mutable receiver call.
    if token_stream.peek_next_tag() != Some(TokenTag::DOT) {
        return Err(calls_data::invalid_receiver_call(
            InvalidReceiverCallReason::MutableMarkerOnNonReceiverCall,
            None,
            None,
            None,
            None,
            marker_span,
        )
        .into());
    }

    // A pending numeric binding has no declaration or selected type yet: retain the authored
    // receiver so the expression entry raises the body-root deferral for the whole fragment.
    // Member access on it is reported by the ordinary member diagnostic, never by a field,
    // method or trait search against a type that does not exist yet.
    if let Some(pending_binding) = receiver_declaration.pending_binding() {
        let receiver_span = Some(token_stream.current_span());
        token_stream.advance();
        push_pending_numeric_binding(context, expression, pending_binding, receiver_span, false)?;
        // The authored `~` opened a mutable receiver chain over a real place: the retained member
        // steps keep that access and the operand completes as a mutable reference, so the existing
        // receiver-access policy validates them against the binding instead of a temporary.
        if token_stream.current_tag() == TokenTag::DOT {
            retain_pending_operand_member_access(
                token_stream,
                expression,
                RetainedOperandInput {
                    chain_access: PostfixChainAccess::mutable_marker(marker_span),
                    expected: ExpectedType::Infer,
                    value_mode: &ValueMode::MutableReference,
                },
                context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            )?;
        }
        return Ok(());
    }

    // A ready root needs its declaration for the field-access chain below; the pending case
    // above is the only shape without one.
    let Some(receiver) = receiver_declaration.ready_declaration() else {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "a mutable receiver root carried neither a ready declaration nor a pending numeric \
             binding",
        )
        .into());
    };

    token_stream.advance();
    let receiver_expression = parse_field_access_expression_with_receiver_access(
        token_stream,
        receiver,
        context,
        PostfixChainAccess::mutable_marker(marker_span),
        allow_boundary_catch,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )?;

    // A retained receiver call keeps its authored operand position: the enclosing expression
    // boundary decides whether the fragment completes here or defers to the body root.
    let receiver_expression = match receiver_expression {
        ParsedPostfixExpression::Value(value) => value,
        ParsedPostfixExpression::RetainedOperand(item) => {
            expression.push(item);
            return Ok(());
        }
    };

    push_expression_operand(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        expression,
        allow_boundary_catch,
        receiver_expression,
        path_fork,
    )
}

// WHAT: parses a named or `this`-rooted operand of a `copy` expression, which must resolve to a
//      place.
// WHY: `copy` clones the current stored value at a place; arbitrary expressions do not have
//      stable storage, so the parser restricts this to names and parenthesized places.
pub(super) fn parse_copy_place_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedCopyPlace, ExpressionParseError> {
    match token_stream.current_tag() {
        TokenTag::OPEN_PARENTHESIS => {
            let open_span = Some(token_stream.current_span());
            token_stream.advance();

            let mut parsed_place = parse_copy_place_expression(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            )?;

            if token_stream.current_tag() != TokenTag::CLOSE_PARENTHESIS {
                let found = token_stream
                    .current_diagnostic_token(string_table)
                    .map_err(|error| {
                        InfrastructureFailure::malformed_token_projection(
                            "copy-place closing-delimiter diagnostic",
                            error,
                        )
                    })?;
                return Err(token_lexical::expected_token_from_projections(
                    DiagnosticToken::from_static_tag(TokenTag::CLOSE_PARENTHESIS),
                    found,
                    Some(token_stream.current_span()),
                )
                .into());
            }

            token_stream.advance();
            // The authored place expression began at the opening parenthesis; both shapes keep
            // that boundary so diagnostics point at the whole place.
            match &mut parsed_place {
                ParsedCopyPlace::Ready { place, .. } => {
                    let mut record = expression_builder.place(*place)?.metadata().clone();
                    record.span = open_span;
                    *place = expression_builder.append_place(record, open_span)?;
                }
                ParsedCopyPlace::PendingNumeric { span, .. } => *span = open_span,
            }
            Ok(parsed_place)
        }

        TokenTag::SYMBOL | TokenTag::THIS => {
            let place_declaration = resolve_place_root(token_stream, context, string_table)?;

            // An eligible body-local numeric binding has no declaration, place payload or
            // `TypeId` yet: retain the authored `copy` of the reference so the expression entry
            // raises the body-root deferral for the whole fragment.
            if let Some(pending_binding) = place_declaration.pending_binding() {
                let reference_span = Some(token_stream.current_span());
                token_stream.advance();
                return Ok(ParsedCopyPlace::PendingNumeric {
                    binding: pending_binding,
                    span: reference_span,
                });
            }

            // A ready root needs its declaration for the place payload below; the pending case
            // above is the only shape without one.
            let Some(place_declaration) = place_declaration.ready_declaration() else {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "a copy-place root carried neither a ready declaration nor a pending numeric \
                     binding",
                )
                .into());
            };

            if context
                .source_callable_signature(place_declaration)
                .is_some()
            {
                let reason = if token_stream.peek_next_tag() == Some(TokenTag::OPEN_PARENTHESIS) {
                    InvalidCopyTargetReason::FunctionCall
                } else {
                    InvalidCopyTargetReason::FunctionName
                };
                Err(
                    calls_data::invalid_copy_target(reason, Some(token_stream.current_span()))
                        .into(),
                )
            } else {
                let reference_span = Some(token_stream.current_span());
                let reference_expression = reference_expression_from_declaration(
                    place_declaration,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    reference_span,
                )?;
                token_stream.advance();

                let copied_expression = if token_stream.position() < token_stream.length()
                    && token_stream.current_tag() == TokenTag::DOT
                {
                    match parse_postfix_chain_expression(
                        token_stream,
                        reference_expression,
                        reference_span,
                        PostfixChainAccess::shared(),
                        // A `copy` operand is a place: an authored boundary catch never attaches to
                        // a retained receiver call here.
                        false,
                        context,
                        type_interner,
                        expression_builder,
                        const_values,
                        string_table,
                        path_fork,
                    )? {
                        ParsedPostfixExpression::Value(value) => value,
                        // A retained receiver call has no value and can never be a place.
                        ParsedPostfixExpression::RetainedOperand(item) => {
                            return Err(calls_data::invalid_copy_target(
                                InvalidCopyTargetReason::NonPlace,
                                item.source_span(expression_builder)?,
                            )
                            .into());
                        }
                    }
                } else {
                    reference_expression
                };

                let copied_view = expression_builder.view_owned(copied_expression)?;
                let type_id = copied_view.require_single(ValueReceiverContext::ReceiverBase)?;
                let copied_span = copied_view.span();
                let Some(place) =
                    place_expression_from_expression(expression_builder, copied_expression)?
                else {
                    return Err(calls_data::invalid_copy_target(
                        InvalidCopyTargetReason::NonPlace,
                        copied_span,
                    )
                    .into());
                };

                Ok(ParsedCopyPlace::Ready { place, type_id })
            }
        }

        TokenTag::MUST | TokenTag::TRAIT_THIS => {
            let keyword = reserved_trait_keyword_or_dispatch_mismatch_for_tag(
                token_stream.current_tag(),
                CompilerStage::AstConstruction,
                "copy-place parsing",
            )?;

            Err(reserved_trait_keyword_error(keyword, Some(token_stream.current_span())).into())
        }

        TokenTag::MUTABLE => {
            let marker_span = Some(token_stream.current_span());
            token_stream.advance();

            match parse_copy_place_expression(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            ) {
                Ok(_) => Err(calls_data::invalid_copy_target(
                    InvalidCopyTargetReason::MutableMarkerNotAllowed,
                    marker_span,
                )
                .into()),
                Err(existing_error) => Err(existing_error),
            }
        }

        _ => Err(calls_data::invalid_copy_target(
            InvalidCopyTargetReason::NonPlace,
            Some(token_stream.current_span()),
        )
        .into()),
    }
}

fn resolve_place_root<'a>(
    token_stream: &mut AstCursor,
    context: &'a ScopeContext,
    string_table: &mut StringTable,
) -> Result<ValueDeclarationRef<'a>, ExpressionParseError> {
    let span = token_stream.current_span();
    let declaration = match token_stream.current_tag() {
        TokenTag::THIS => context.resolve_this_reference(string_table.intern("this"), span)?,
        TokenTag::SYMBOL => {
            let symbol_id = token_stream
                .current_string_id_in(string_table)?
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "place root symbol had no string payload",
                    )
                })?;
            context.resolve_value_reference(symbol_id, span)?
        }
        _ => {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "place root resolution received a token other than a name or `this`",
            )
            .into());
        }
    };
    Ok(declaration)
}

/// Record a completed place using its resolved root and ordered field projections.
pub(crate) fn place_expression_from_expression(
    expressions: &mut AstExpressionBuilder,
    id: OwnedExpr,
) -> Result<Option<PlaceId>, ExpressionConstructionFailure> {
    let expression = expressions.view_owned(id)?;
    let type_id = expression.value_type_id()?;
    let span = expression.span();
    let mut root = expression;
    let mut projections = Vec::new();
    let path = loop {
        match root.operation() {
            ExpressionOperationView::Reference(path) => break path,
            ExpressionOperationView::FieldAccess { base, field } => {
                projections.push(PlaceProjection::Field {
                    field,
                    result_type: root.value_type_id()?,
                });
                root = base;
            }
            _ => return Ok(None),
        }
    };
    let root_type_id = root.value_type_id()?;
    let value_mode = root.value_mode();
    projections.reverse();
    let projections = expressions.append_place_projections(projections, span)?;
    let place = expressions.append_place(
        PlaceRecord {
            root: path,
            root_type_id,
            projections,
            type_id,
            value_mode,
            span,
        },
        span,
    )?;
    Ok(Some(place))
}

pub(crate) fn place_expression_is_mutable(place: PlaceView<'_>) -> bool {
    place.metadata().value_mode.is_mutable()
}

pub(crate) fn root_binding_name_of_place(
    place: PlaceView<'_>,
    path_fork: &PathInternerFork,
) -> Option<StringId> {
    path_fork.component(place.metadata().root)
}

/// Emit the typed read operations used by compound assignment.
///
/// Place records retain canonical types at every projection boundary, so this does not
/// re-resolve field meaning or rebuild a recursive compatibility expression.
pub(crate) fn expression_from_place_expression(
    expressions: &mut AstExpressionBuilder,
    id: PlaceId,
) -> Result<OwnedExpr, ExpressionConstructionFailure> {
    let place = expressions.place(id)?;
    let record = place.metadata();
    let root = record.root;
    let root_type_id = record.root_type_id;
    let span = record.span;
    let value_mode = record.value_mode;
    let mut fields = Vec::new();
    for projection in place.projections() {
        match projection {
            PlaceProjectionView::Field { field, result_type } => fields.push((field, result_type)),
            PlaceProjectionView::Index { .. } => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "compound read received an unsupported indexed place",
                )
                .into());
            }
        }
    }
    let mut value = expressions.reference_with_type_id(
        root,
        root_type_id,
        span,
        value_mode,
        ConstRecordState::RuntimeValue,
    )?;
    for (field, type_id) in fields {
        value = expressions.append_expression(
            ExpressionOp::FieldAccess { base: value, field },
            span,
            type_id,
            value_mode,
        )?;
    }
    Ok(value)
}
