//! Mutation expression parsing for assignment and compound assignment.
//!
//! WHAT:
//!   - Parses simple assignment (`=`) and compound assignment
//!     (`+=`, `-=`, `*=`, `/=`, `//=`, `%=`, `^=`) after a place
//!     expression has been resolved.
//!   - Validates mutability and type compatibility for the target.
//!   - Desugars compound operators into `target = target op rhs` for
//!     uniform type checking and constant folding.
//!
//! WHY:  Mutation is a distinct expression kind in the AST; centralising
//!       the parsing, validation, and compound-operator expansion here
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
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    PlaceExpression, PlaceExpressionKind,
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

use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidAssignmentTargetReason, InvalidFallibleHandlingReason,
    TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::type_coercion::compatibility::is_declaration_compatible;
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_declared_type;
use crate::compiler_frontend::type_coercion::parse_context::{
    ExpectedType, cast_target_context_for_type_id, parse_expectation_for_type_id,
};

/// Check that an expression's type is compatible with the assignment target.
///
/// WHAT: compares the RHS expression type against the target type using the
///       declaration-compatibility relation.
/// WHY: assignments are rejected when the value type cannot be coerced to the
///      target type; this helper emits the appropriate diagnostic.
fn validate_assignment_value_type(
    expected_type_id: TypeId,
    actual_value: &Expression,
    type_environment: &TypeEnvironment,
) -> Result<(), ExpressionParseError> {
    if is_declaration_compatible(expected_type_id, actual_value.type_id, type_environment) {
        return Ok(());
    }

    Err(CompilerDiagnostic::type_mismatch(
        expected_type_id,
        actual_value.type_id,
        TypeMismatchContext::Assignment,
        actual_value.span,
    )
    .into())
}

/// Map a compound-assignment token to its arithmetic operator and diagnostic label.
///
/// WHAT: converts `+=`, `-=`, `*=`, `/=`, `//=`, `%=`, `^=` into the corresponding
///       `Operator` variant and a human-readable label used in diagnostics.
/// WHY: compound assignments are desugared into `target = target op rhs`;
///      this mapping is needed both for the desugaring and for error messages.
fn compound_assignment_operator(token_kind: &TokenKind) -> Option<(Operator, &'static str)> {
    match token_kind {
        TokenKind::AddAssign => Some((Operator::Add, "Compound assignment '+='")),
        TokenKind::SubtractAssign => Some((Operator::Subtract, "Compound assignment '-='")),
        TokenKind::MultiplyAssign => Some((Operator::Multiply, "Compound assignment '*='")),
        TokenKind::DivideAssign => Some((Operator::Divide, "Compound assignment '/='")),
        TokenKind::IntDivideAssign => Some((Operator::IntDivide, "Compound assignment '//='")),
        TokenKind::ModulusAssign => Some((Operator::Modulus, "Compound assignment '%='")),
        TokenKind::ExponentAssign => Some((Operator::Exponent, "Compound assignment '^='")),
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

/// Build the RHS value for a compound assignment by evaluating `target op rhs`.
///
/// WHAT: parses the RHS expression, then evaluates `target op rhs` through the
///       normal expression evaluator so that type checking and constant folding
///       apply to the desugared arithmetic.
/// WHY: compound assignments must behave exactly as the equivalent binary
///      expression for type rules and for constant propagation.
fn evaluate_compound_assignment_value(
    token_stream: &mut FileTokens,
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
    let mut expr_type = ExpectedType::Infer;

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
    //  Build `target op rhs` and validate the
    //  result type against the declared type
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

    validate_assignment_value_type(target_type_id, &value, type_interner.environment())?;

    Ok(value)
}

/// Parse and validate a mutation given an already-resolved place target.
///
/// WHAT: checks that `target` is a mutable place, then parses the assignment
///       operator and RHS, validates type compatibility, and returns an
///       `Assignment` AST node.
/// WHY: this is the core mutation logic shared by `handle_mutation` (which
///      parses field access first) and `handle_mutation_target` (which receives
///      an already-built target node from the caller).
fn build_mutation_from_target(
    token_stream: &mut FileTokens,
    variable_declaration: &Declaration,
    target: PlaceExpression,
    declaration_span: Option<SourceSpan>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<AstNode, ExpressionParseError> {
    let span = Some(SourceSpan::new(
        token_stream.file_id,
        token_stream.current_token().span,
    ));
    let target_type_id = target.type_id;

    ast_log!(
        "Handling mutation for ",
        #variable_declaration.value.value_mode, " ",
        Blue variable_declaration.id.to_string(string_table)
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

    let value = match token_stream.current_token_kind() {
        TokenKind::Assign => {
            // Simple mutation: variable = new_value. Parse-time context is
            // preserved only for context-sensitive literals. Compound
            // assignments below use Inferred because that context does not
            // apply to arithmetic operators.
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
            if token_stream.current_token_kind() == &TokenKind::Else
                && type_interner
                    .environment()
                    .option_inner_type(rhs.type_id)
                    .is_some()
            {
                return Err(CompilerDiagnostic::invalid_fallible_handling(
                    InvalidFallibleHandlingReason::DirectOptionFallbackSyntax,
                    Some(SourceSpan::new(
                        token_stream.file_id,
                        token_stream.current_token().span,
                    )),
                )
                .into());
            }

            validate_assignment_value_type(target_type_id, &rhs, type_interner.environment())?;

            coerce_expression_to_declared_type(rhs, target_type_id, type_interner.environment())
        }

        compound_token => {
            let Some((operator, _label)) = compound_assignment_operator(compound_token) else {
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
        scope: context.scope.clone(),
    })
}

/// Parse a mutation when the place target has already been parsed.
///
/// WHAT: thin wrapper around `build_mutation_from_target` for callers that
///       have already resolved the left-hand side (e.g. after field-access parsing).
/// WHY: keeps the public surface small; callers that already own the target
///      node do not need to re-parse it.
pub(crate) fn handle_mutation_target(
    token_stream: &mut FileTokens,
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
    token_stream: &mut FileTokens,
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
            Some(SourceSpan::new(
                token_stream.file_id,
                token_stream.current_token().span,
            )),
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
