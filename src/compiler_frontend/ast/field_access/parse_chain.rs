//! Postfix/member chain parsing implementation.
//!
//! WHAT: drives chained postfix parsing and dispatches each member step to focused handlers.
//! WHY: field access, receiver methods, and compiler-owned builtin members evolve independently,
//! so the chain driver should stay thin while policy lives in dedicated modules.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::parse_expression_places::place_expression_from_expression;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidAssignmentTargetReason, InvalidFieldAccessReason,
    InvalidReceiverCallReason,
};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::ids::TypeId;

use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};
use crate::compiler_frontend::value_mode::ValueMode;

use super::collection_builtin::parse_collection_builtin_member_typed;
use super::field_member::{parse_field_member_access_typed, parse_member_name_typed};
use super::map_builtin::parse_map_builtin_member_typed;
use super::receiver_calls::parse_receiver_method_call_typed;
use super::{MemberStepContext, PostfixChainAccess, ReceiverAccessMode};

/// Builds the expression payload for a declaration reference without choosing an AST node shape.
///
/// WHAT: preserves constant-context inlining, placeholder typing, value shape, and reactive
/// metadata exactly as the field-access receiver path expects.
/// WHY: expression parsing can push plain references as narrow operands while member/place
/// parsing can still wrap the same payload as a temporary `AstNode` during migration.
pub(crate) fn reference_expression_from_declaration(
    reference_arg: &Declaration,
    context: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
    base_location: SourceLocation,
) -> Expression {
    if context.kind.is_constant_context() {
        if reference_arg.is_unresolved_constant_placeholder() {
            let placeholder_type = context
                .expected_result_type_ids
                .first()
                .map(|type_id| diagnostic_type_spelling(*type_id, type_interner.environment()))
                .unwrap_or_else(|| reference_arg.value.diagnostic_type.to_owned());
            let placeholder_type_id = context
                .expected_result_type_ids
                .first()
                .copied()
                .unwrap_or(reference_arg.value.type_id);
            return Expression::reference_with_type_id(
                reference_arg.id.to_owned(),
                placeholder_type,
                placeholder_type_id,
                base_location,
                ValueMode::ImmutableOwned,
                reference_arg.value.const_record_state,
            );
        }

        let mut inlined_expression = reference_arg.value.to_owned();
        inlined_expression.value_mode = ValueMode::ImmutableOwned;
        inlined_expression
    } else {
        let mut ref_expr = Expression::reference_with_type_id(
            reference_arg.id.to_owned(),
            reference_arg.value.diagnostic_type.to_owned(),
            reference_arg.value.type_id,
            base_location,
            reference_arg.value.value_mode.to_owned(),
            reference_arg.value.const_record_state,
        );
        if let Some(source) = reference_arg.value.reactive_source.clone() {
            ref_expr = ref_expr.with_reactive_source(source);
        }
        if let Some(template_metadata) = reference_arg.value.reactive_template.clone() {
            ref_expr = ref_expr.with_reactive_template_metadata(template_metadata);
        }

        ref_expr
    }
}

fn receiver_reference_node(
    reference_arg: &Declaration,
    context: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
    base_location: SourceLocation,
) -> AstNode {
    AstNode {
        kind: NodeKind::ExpressionStatement(reference_expression_from_declaration(
            reference_arg,
            context,
            type_interner,
            base_location.clone(),
        )),
        scope: context.scope.to_owned(),
        location: base_location,
    }
}

fn receiver_node_type_id(node: &AstNode) -> Result<TypeId, CompilerError> {
    node.expression_type_id()
}

fn type_id_is_external(type_id: TypeId, type_interner: &AstTypeInterner<'_>) -> bool {
    matches!(
        type_interner.environment().get(type_id),
        Some(TypeDefinition::External(..))
    )
}

pub(crate) fn parse_postfix_chain_expression(
    token_stream: &mut FileTokens,
    receiver_expression: Expression,
    receiver_location: SourceLocation,
    chain_access: PostfixChainAccess,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    let receiver_node = AstNode {
        kind: NodeKind::ExpressionStatement(receiver_expression),
        scope: context.scope.to_owned(),
        location: receiver_location,
    };

    let postfix_node = parse_postfix_chain_typed(
        token_stream,
        receiver_node,
        chain_access,
        context,
        type_interner,
        string_table,
    )?;

    expression_from_postfix_node(&postfix_node)
}

pub(crate) fn expression_from_postfix_node(
    postfix_node: &AstNode,
) -> Result<Expression, ExpressionParseError> {
    match &postfix_node.kind {
        NodeKind::ExpressionStatement(expression) => Ok(expression.to_owned()),

        unexpected_kind => Err(CompilerError::compiler_error(format!(
            "Expected postfix expression node, found {unexpected_kind:?}"
        ))
        .into()),
    }
}

fn parse_postfix_chain_typed(
    token_stream: &mut FileTokens,
    mut receiver_node: AstNode,
    chain_access: PostfixChainAccess,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<AstNode, ExpressionParseError> {
    let receiver_access_mode = chain_access.mode;
    let authored_marker_location = chain_access.authored_marker_location;
    let mut encountered_receiver_call = false;

    // ----------------------------
    //  Walk postfix member-access chain
    // ----------------------------
    while token_stream.index < token_stream.length
        && token_stream.current_token_kind() == &TokenKind::Dot
    {
        token_stream.advance();

        // An access that ends immediately after the authored dot has no member name. The dot
        // we just consumed is the owning boundary: point at it directly so EOF (or a stream
        // with no trailing Eof token) never falls back to the receiver start or a default
        // location. Non-EOF missing-member boundaries keep their offending-token location
        // through `parse_member_name_typed` below.
        if token_stream.index >= token_stream.length
            || matches!(token_stream.current_token_kind(), TokenKind::Eof)
        {
            let dot_location = token_stream.tokens[token_stream.index - 1].location.clone();
            return Err(CompilerDiagnostic::invalid_field_access(
                InvalidFieldAccessReason::ExpectedNameAfterDot,
                None,
                None,
                Vec::new(),
                dot_location,
            )
            .into());
        }

        let member_name = parse_member_name_typed(token_stream, string_table)?;
        let receiver_type_id = receiver_node_type_id(&receiver_node)?;
        let member_location = token_stream.current_location();
        let member_context = MemberStepContext {
            receiver_node: &receiver_node,
            receiver_type_id,
            member_name,
            member_location: member_location.clone(),
            receiver_access_mode,
            authored_marker_location: authored_marker_location.clone(),
            scope_context: context,
        };

        if let Some(field_access) =
            parse_field_member_access_typed(token_stream, member_context.to_owned(), type_interner)?
        {
            receiver_node = field_access;
            continue;
        }

        if let Some(collection_builtin_call) = parse_collection_builtin_member_typed(
            token_stream,
            member_context.to_owned(),
            type_interner,
            string_table,
        )? {
            receiver_node = collection_builtin_call;
            encountered_receiver_call = true;
            continue;
        }

        if let Some(map_builtin_call) = parse_map_builtin_member_typed(
            token_stream,
            member_context.to_owned(),
            type_interner,
            string_table,
        )? {
            receiver_node = map_builtin_call;
            encountered_receiver_call = true;
            continue;
        }

        if let Some(receiver_method_call) = parse_receiver_method_call_typed(
            token_stream,
            member_context,
            type_interner,
            string_table,
        )? {
            receiver_node = receiver_method_call;
            encountered_receiver_call = true;
            continue;
        }

        // No handler matched. Preserve the user-facing distinction between
        // deferred choice payload access, opaque externals, and ordinary
        // missing members while routing all cases through one typed diagnostic.
        let reason = if type_interner
            .environment()
            .variants_for(receiver_type_id)
            .is_some()
            && token_stream.peek_next_token() != Some(&TokenKind::OpenParenthesis)
        {
            let next_token = token_stream.peek_next_token();
            if next_token
                .map(|t| t.is_assignment_operator())
                .unwrap_or(false)
            {
                InvalidFieldAccessReason::ChoicePayloadMutation
            } else {
                InvalidFieldAccessReason::ChoicePayloadDeferred
            }
        } else if type_id_is_external(receiver_type_id, type_interner) {
            InvalidFieldAccessReason::UnknownExternalMember
        } else {
            InvalidFieldAccessReason::UnknownMember
        };

        // Collect known field/method names for "did you mean?" suggestions on UnknownMember.
        let known_fields = collect_known_member_names(receiver_type_id, type_interner);

        return Err(CompilerDiagnostic::invalid_field_access(
            reason,
            Some(member_name),
            Some(receiver_type_id),
            known_fields,
            member_location,
        )
        .into());
    }

    // ----------------------------
    //  Validate assignment receiver is a place
    // ----------------------------
    if token_stream.current_token_kind().is_assignment_operator() {
        let receiver_expression = expression_from_postfix_node(&receiver_node)?;
        if place_expression_from_expression(&receiver_expression).is_none() {
            let diagnostic = CompilerDiagnostic::invalid_assignment_target(
                InvalidAssignmentTargetReason::TemporaryNotAssignable,
                None,
                Some(receiver_expression.type_id),
                None,
                None,
                None,
                receiver_node.location.clone(),
            );
            return Err(diagnostic.into());
        }
    }

    if receiver_access_mode == ReceiverAccessMode::Mutable && !encountered_receiver_call {
        // The authored `~` marker is the source the author must change when no receiver call
        // followed it. Fall back to the receiver boundary only when the marker was not threaded
        // through this chain entry.
        let marker_location = authored_marker_location
            .clone()
            .unwrap_or_else(|| receiver_node.location.clone());
        // When the chain is followed by an assignment operator, the author wrote `~place = ...`
        // and intended an assignment target, not a receiver call.
        if token_stream.current_token_kind().is_assignment_operator() {
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

    Ok(receiver_node)
}

pub fn parse_field_access(
    token_stream: &mut FileTokens,
    base_arg: &Declaration,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    let postfix_node = parse_field_access_with_receiver_access(
        token_stream,
        base_arg,
        context,
        PostfixChainAccess::shared(),
        type_interner,
        string_table,
    )?;

    expression_from_postfix_node(&postfix_node)
}

fn parse_field_access_with_receiver_access(
    token_stream: &mut FileTokens,
    base_arg: &Declaration,
    context: &ScopeContext,
    chain_access: PostfixChainAccess,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<AstNode, ExpressionParseError> {
    let base_location = if token_stream.index > 0 {
        token_stream.tokens[token_stream.index - 1].location.clone()
    } else {
        token_stream.current_location()
    };

    parse_postfix_chain_typed(
        token_stream,
        receiver_reference_node(base_arg, context, type_interner, base_location),
        chain_access,
        context,
        type_interner,
        string_table,
    )
}

pub(crate) fn parse_field_access_expression_with_receiver_access(
    token_stream: &mut FileTokens,
    base_arg: &Declaration,
    context: &ScopeContext,
    chain_access: PostfixChainAccess,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    let base_location = if token_stream.index > 0 {
        token_stream.tokens[token_stream.index - 1].location.clone()
    } else {
        token_stream.current_location()
    };

    let receiver_expression = reference_expression_from_declaration(
        base_arg,
        context,
        type_interner,
        base_location.clone(),
    );

    parse_postfix_chain_expression(
        token_stream,
        receiver_expression,
        base_location,
        chain_access,
        context,
        type_interner,
        string_table,
    )
}

/// Collect known struct field names and choice variant names for a receiver type.
///
/// WHAT: gathers the member names that are valid on the receiver so the diagnostic renderer
/// can offer a "did you mean?" suggestion when a name is misspelled.
/// WHY: the most helpful thing a compiler can do for a typo'd field access is suggest the
/// correct field name. The type environment already has this information.
fn collect_known_member_names(
    receiver_type_id: TypeId,
    type_interner: &AstTypeInterner<'_>,
) -> Vec<StringId> {
    let environment = type_interner.environment();
    let mut names = Vec::new();

    // Struct field names are stored as InternedPath; the field name is the last
    // path component, not the first (which may be a source-file prefix).
    if let Some(fields) = environment.fields_for(receiver_type_id) {
        for field in fields {
            if let Some(name) = field.name.name() {
                names.push(name);
            }
        }
    }

    // Choice variant names are already plain StringId values.
    if let Some(variants) = environment.variants_for(receiver_type_id) {
        for variant in variants {
            names.push(variant.name);
        }
    }

    names
}
