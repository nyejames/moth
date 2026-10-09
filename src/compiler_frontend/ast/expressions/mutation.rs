//! Mutation expression parsing for assignment and compound assignment.
//!
//! WHAT:
//!   - Parses simple assignment (`=`) and compound assignment
//!     (`+=`, `-=`, `*=`, `/=`, `//=`, `%=`, `^=`) after a place
//!     expression has been resolved.
//!   - Validates mutability and type compatibility for the target.
//!   - Parses compound RHSs with numeric-literal peer hints for fixed scalars, bare `Float`,
//!     non-power `Dec` and every `Uint` target, evaluates `target op rhs` in its promoted
//!     domain and converts incompatible numeric results back through builtin cast evidence.
//!
//! WHY:  Mutation is a distinct expression kind in the AST; centralising
//!       the parsing, validation, and compound-value construction here
//!       keeps the main expression dispatch logic free of assignment-
//!       specific rules.
//!
//! Use `handle_mutation` when the statement parser has only a variable
//! reference, or `handle_mutation_target` when the caller has already
//! resolved the place target (e.g. after field-access parsing).
//!
//! DOES NOT OWN:
//!   - Value-block / receiver-block mutation. Those live in
//!     `statements/value_production/`.
//!   - Field-access chain parsing. That lives in
//!     `expressions/parse_expression_places.rs` and `field_access/`.
//!   - Statement-level mutation orchestration. That lives in
//!     `statements/`.
use crate::ast_log;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::eval_expression::evaluate_expression;
use crate::compiler_frontend::ast::expressions::expression::{Expression, Operator};
use crate::compiler_frontend::ast::expressions::expression_kind::ResolvedCastExpression;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    PlaceExpression, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::ast::expressions::parse_expression::{
    create_expression, create_expression_with_trailing_newline_policy,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::expressions::parse_expression_places::{
    expression_from_place_expression, place_expression_from_expression,
    place_expression_is_mutable, root_binding_name_of_place,
};
use crate::compiler_frontend::ast::field_access::parse_field_access;
use crate::compiler_frontend::ast::statements::value_production::receiver::try_parse_value_block_at_receiver;
use crate::compiler_frontend::ast::statements::value_production::types::ValueReceiverKind;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::builtins::casts::evidence::lookup_builtin_evidence;
use crate::compiler_frontend::builtins::casts::targets::builtin_cast_target_for_type;

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidAssignmentTargetReason, InvalidFallibleHandlingReason,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::compatibility::is_declaration_compatible;
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_explicit_type_boundary;
use crate::compiler_frontend::type_coercion::parse_context::{
    ExpectedType, cast_target_context_for_type_id, is_numeric_literal_destination_type_id,
    parse_expectation_for_type_id,
};

/// Build the existing assignment mismatch diagnostic for a compound store.
///
/// WHAT: reports the same incompatibility as a simple assignment when a promoted operation
///       result has no permitted receiving conversion.
/// WHY: receiving conversion stays owned by builtin cast evidence; the promoted arithmetic has
///       already proved the result numeric, so any remaining mismatch is a non-castable pair and
///       retains the assignment boundary's established diagnostic.
fn assignment_type_mismatch(
    expected_type_id: TypeId,
    actual_value: &Expression,
) -> ExpressionParseError {
    CompilerDiagnostic::type_mismatch(
        expected_type_id,
        actual_value.type_id,
        TypeMismatchContext::Assignment,
        actual_value.span,
    )
    .into()
}

/// Map a canonical compound-assignment tag to its arithmetic operator and label.
///
/// WHAT: converts `+=`, `-=`, `*=`, `/=`, `//=`, `%=`, `^=` into the corresponding
///       `Operator` variant and a human-readable label used in diagnostics.
/// WHY: compound assignments are desugared into `target = target op rhs`;
///      the stable `TokenTag` taxonomy is the canonical operator authority, so
///      the cursor classifies the operator tag directly.
fn compound_assignment_operator_for_tag(tag: TokenTag) -> Option<(Operator, &'static str)> {
    match tag {
        TokenTag::ADD_ASSIGN => Some((Operator::Add, "Compound assignment '+='")),
        TokenTag::SUBTRACT_ASSIGN => Some((Operator::Subtract, "Compound assignment '-='")),
        TokenTag::MULTIPLY_ASSIGN => Some((Operator::Multiply, "Compound assignment '*='")),
        TokenTag::DIVIDE_ASSIGN => Some((Operator::Divide, "Compound assignment '/='")),
        TokenTag::INT_DIVIDE_ASSIGN => Some((Operator::IntDivide, "Compound assignment '//='")),
        TokenTag::MODULUS_ASSIGN => Some((Operator::Modulus, "Compound assignment '%='")),
        TokenTag::EXPONENT_ASSIGN => Some((Operator::Exponent, "Compound assignment '^='")),
        _ => None,
    }
}

/// Input bundle for `evaluate_compound_assignment_value` to avoid a long parameter list.
///
/// WHAT: carries the place target, its resolved type, and the operator that
///       will be used to build the desugared RHS expression.
/// WHY: compound assignment evaluation needs the same state as simple
///      assignment plus the operator; bundling keeps the signature readable.
struct CompoundAssignmentInput<'a> {
    /// Declaration of the variable being assigned to.
    variable_declaration: &'a Declaration,
    target: &'a PlaceExpression,
    target_type_id: TypeId,
    operator: Operator,
}

/// WHAT: gives existing fixed-scalar targets, bare `Float` targets, non-power `Dec` targets
///       and every `Uint` target a numeric literal hint, then evaluates `target op rhs` in the
///       promoted domain.
/// WHY: `Dec ^ Int` must keep the exponent in the profile `Int` domain even when the compound
///      assignment's receiving type is a Dec scale, while `Uint ^ Uint` keeps its exponent in
///      the `Uint` domain.
fn evaluate_compound_assignment_value(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    input: CompoundAssignmentInput<'_>,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Expression, ExpressionParseError> {
    let CompoundAssignmentInput {
        variable_declaration,
        target,
        target_type_id,
        operator,
    } = input;
    let type_environment = type_interner.environment();
    let target_is_number = type_environment.number_scale(target_type_id).is_some();
    // Dec power alone keeps a pending compound RHS in profile Int instead of its Dec peer.
    let number_power_exponent = target_is_number && matches!(operator, Operator::Exponent);
    let peer_numeric_literal =
        is_numeric_literal_destination_type_id(target_type_id, type_environment)
            && (type_environment.fixed_scalar(target_type_id).is_some()
                || target_type_id == type_environment.builtins().float
                || target_type_id == type_environment.builtins().uint
                || target_is_number)
            && !number_power_exponent;
    let mut expr_type = if peer_numeric_literal {
        ExpectedType::NumericLiteral(target_type_id)
    } else {
        ExpectedType::Infer
    };

    // -----------------------
    //  Parse the RHS operand
    // -----------------------

    let rhs_context = path_fork
        .component(variable_declaration.id)
        .map(|target_name| context.with_pending_catch_assignment_targets(&[target_name]))
        .unwrap_or_else(|| context.clone());
    let rhs = create_expression(
        token_stream,
        &rhs_context,
        type_interner,
        &mut expr_type,
        &variable_declaration.value.value_mode,
        false,
        string_table,
        path_fork,
    )?;

    // -------------------------------------------
    //  Evaluate `target op rhs` in its promoted
    //  domain before applying the store conversion
    // -------------------------------------------
    let target_expression = expression_from_place_expression(target);

    let operator_item = ExpressionRpnItem::Operator {
        operator,
        span: target.span,
    };
    let mut inferred = ExpectedType::Infer;
    let value = evaluate_expression(
        context,
        vec![
            ExpressionRpnItem::Operand(target_expression),
            ExpressionRpnItem::Operand(rhs),
            operator_item,
        ],
        type_interner,
        &mut inferred,
        &variable_declaration.value.value_mode,
        string_table,
        path_fork,
    )?;

    if is_declaration_compatible(target_type_id, value.type_id, type_interner.environment()) {
        return Ok(value);
    }

    let type_environment = type_interner.environment();
    // The promoted arithmetic result is always numeric: every compound tag is an arithmetic
    // operator and `+` never concatenates strings, so the arithmetic evaluation already proved
    // the source supports a cast classification. Let the builtin pair evidence owner decide the
    // destination conversion, including exact `Dec` scales.
    let source_target =
        builtin_cast_target_for_type(value.type_id, type_environment, string_table, path_fork);
    let destination_target =
        builtin_cast_target_for_type(target_type_id, type_environment, string_table, path_fork);
    let cast_targets = source_target.zip(destination_target);
    let evidence = cast_targets.and_then(|(source, target)| {
        lookup_builtin_evidence(source, target, context.numeric_profile)
            .map(|evidence| (target, evidence))
    });
    let Some((target, evidence)) = evidence else {
        return Err(assignment_type_mismatch(target_type_id, &value));
    };

    let source_type_id = value.type_id;
    let cast_span = value.span;
    let cast = ResolvedCastExpression {
        source: Box::new(value),
        source_type_id,
        target_type_id,
        target,
        requires_optional_wrap_after_cast: false,
        evidence: ResolvedCastEvidence::Builtin {
            policy: evidence.policy,
        },
        fallibility: evidence.fallibility,
        handling: CastHandling::StoreConversion,
        span: cast_span,
    };
    let cast_value = Expression::cast(cast, target_type_id, type_interner.environment());
    let mut inferred = ExpectedType::Infer;
    let converted_value = evaluate_expression(
        context,
        vec![ExpressionRpnItem::Operand(cast_value)],
        type_interner,
        &mut inferred,
        &variable_declaration.value.value_mode,
        string_table,
        path_fork,
    )?;

    Ok(converted_value)
}

/// Parse and validate a mutation given an already-resolved place target.
///
/// WHAT: checks that `target` is a mutable place, then parses the assignment
///       operator and RHS, validates type compatibility, and returns an
///       `Assignment` AST node.
/// WHY: this is the core mutation logic shared by `handle_mutation` (which
///      parses field access first) and `handle_mutation_target` (which receives
///      an already-built target node from the caller).
#[allow(
    clippy::too_many_arguments,
    reason = "mutation building keeps the token stream, declaration, place target, span, scope, and mutable interner/string/path state as separate borrows"
)]
fn build_mutation_from_target(
    token_stream: &mut AstCursor,
    variable_declaration: &Declaration,
    target: PlaceExpression,
    declaration_span: Option<SourceSpan>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<AstNode, ExpressionParseError> {
    let span = Some(token_stream.current_span());
    let target_type_id = target.type_id;

    ast_log!(
        "Handling mutation for ",
        #variable_declaration.value.value_mode, " ",
        Blue path_fork.render_portable(variable_declaration.id, string_table, &mut Vec::new())
    );

    // -----------------------
    //  Validate mutability
    // -----------------------

    if !place_expression_is_mutable(&target) {
        let (reason, field_name, root_binding_name) = match &target.kind {
            PlaceExpressionKind::Field { field, base } => {
                let root = root_binding_name_of_place(base, path_fork);
                (
                    InvalidAssignmentTargetReason::ImmutableFieldRoot,
                    Some(*field),
                    root,
                )
            }
            PlaceExpressionKind::Local(_) => {
                (InvalidAssignmentTargetReason::ImmutableBinding, None, None)
            }
        };

        let diagnostic = CompilerDiagnostic::invalid_assignment_target(
            reason,
            path_fork.component(variable_declaration.id),
            Some(target_type_id),
            field_name,
            root_binding_name,
            declaration_span,
            span,
        );
        return Err(diagnostic.into());
    }

    // -----------------------
    //  Determine mutation kind
    // -----------------------

    let value = match token_stream.current_tag() {
        TokenTag::ASSIGN => {
            // Simple assignment keeps the receiver hint for context-sensitive values. Compound
            // RHSs peer numeric literals for fixed scalars, bare Float and Uint before
            // promoted evaluation; Dec and Uint arithmetic resolve through their receiving
            // and peer contexts.
            token_stream.advance();

            let mut expr_type =
                parse_expectation_for_type_id(target_type_id, type_interner.environment());
            let mut cast_target_context = cast_target_context_for_type_id(
                target_type_id,
                type_interner.environment(),
                string_table,
                &*path_fork,
            );
            let rhs_context = path_fork
                .component(variable_declaration.id)
                .map(|target_name| context.with_pending_catch_assignment_targets(&[target_name]))
                .unwrap_or_else(|| context.clone());

            let rhs = if let Some(value_block_result) = try_parse_value_block_at_receiver(
                token_stream,
                &rhs_context,
                type_interner,
                &[target_type_id],
                ValueReceiverKind::Assignment,
                string_table,
                path_fork,
            ) {
                value_block_result?
            } else {
                let input = ExpressionParseInput::ordinary(
                    ExpressionParseResources {
                        token_stream,
                        scope_context: &rhs_context,
                        type_interner,
                        expected_type: &mut expr_type,
                        cast_target_context: &mut cast_target_context,
                        value_mode: &variable_declaration.value.value_mode,
                        string_table,
                        path_fork,
                    },
                    false,
                );
                create_expression_with_trailing_newline_policy(input)?
            };

            // Direct option fallback is rejected at each closed receiver so the
            // later statement parser does not report the trailing `else` as an
            // unrelated branch error.
            token_stream.skip_newlines();
            if token_stream.current_tag() == TokenTag::ELSE
                && type_interner
                    .environment()
                    .option_inner_type(rhs.type_id)
                    .is_some()
            {
                return Err(CompilerDiagnostic::invalid_fallible_handling(
                    InvalidFallibleHandlingReason::DirectOptionFallbackSyntax,
                    Some(token_stream.current_span()),
                )
                .into());
            }

            coerce_expression_to_explicit_type_boundary(
                rhs,
                target_type_id,
                type_interner.environment(),
                context.numeric_profile.float_precision,
                TypeMismatchContext::Assignment,
            )?
        }

        compound_tag => {
            let Some((operator, _label)) = compound_assignment_operator_for_tag(compound_tag)
            else {
                return Err(CompilerDiagnostic::invalid_assignment_target(
                    InvalidAssignmentTargetReason::ExpectedAssignmentOperator,
                    path_fork.component(variable_declaration.id),
                    Some(target_type_id),
                    None,
                    None,
                    None,
                    span,
                )
                .into());
            };

            token_stream.advance();

            evaluate_compound_assignment_value(
                token_stream,
                context,
                CompoundAssignmentInput {
                    variable_declaration,
                    target: &target,
                    target_type_id,
                    operator,
                },
                type_interner,
                string_table,
                path_fork,
            )?
        }
    };

    Ok(AstNode {
        kind: NodeKind::Assignment { target, value },
        span,
        scope: context.scope,
    })
}

/// Parse a mutation when the place target has already been parsed.
///
/// WHAT: thin wrapper around `build_mutation_from_target` for callers that
///       have already resolved the left-hand side (e.g. after field-access parsing).
/// WHY: keeps the public surface small; callers that already own the target
///      node do not need to re-parse it.
#[allow(
    clippy::too_many_arguments,
    reason = "mutation handling keeps the token stream, declaration, place target, span, scope, and mutable interner/string/path state as separate borrows"
)]
pub(crate) fn handle_mutation_target(
    token_stream: &mut AstCursor,
    variable_declaration: &Declaration,
    target: PlaceExpression,
    declaration_span: Option<SourceSpan>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<AstNode, ExpressionParseError> {
    build_mutation_from_target(
        token_stream,
        variable_declaration,
        target,
        declaration_span,
        context,
        type_interner,
        string_table,
        path_fork,
    )
}

/// Handle mutation of an existing mutable variable.
///
/// WHAT: parses field-access chains on the variable, then builds the mutation
///       node through `build_mutation_from_target`.
pub fn handle_mutation(
    token_stream: &mut AstCursor,
    variable_declaration: &Declaration,
    declaration_span: Option<SourceSpan>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<AstNode, ExpressionParseError> {
    let target_expression = parse_field_access(
        token_stream,
        variable_declaration,
        context,
        type_interner,
        string_table,
        path_fork,
    )?;

    let Some(target) = place_expression_from_expression(&target_expression) else {
        return Err(CompilerDiagnostic::invalid_assignment_target(
            InvalidAssignmentTargetReason::TemporaryNotAssignable,
            None,
            Some(target_expression.type_id),
            None,
            None,
            None,
            Some(token_stream.current_span()),
        )
        .into());
    };

    build_mutation_from_target(
        token_stream,
        variable_declaration,
        target,
        declaration_span,
        context,
        type_interner,
        string_table,
        path_fork,
    )
}

#[cfg(test)]
#[path = "tests/mutation_tests.rs"]
mod mutation_tests;
