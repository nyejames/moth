//! Assignment parsing and completed compound-value construction.
//!
//! WHAT:
//!   - Parses simple assignment (`=`) and compound assignment
//!     (`+=`, `-=`, `*=`, `/=`, `//=`, `%=`, `^=`) after a typed place
//!     has been resolved.
//!   - Validates mutability and type compatibility for the target.
//!   - Parses compound RHSs with numeric-literal peer hints for fixed scalars and non-power
//!     `Dec` targets, evaluates `target op rhs` in its promoted domain and converts incompatible
//!     numeric results back through builtin cast evidence.
//!
//! WHY: assignment statements retain a PlaceId and an OwnedExpr, while numeric-dependent
//! values stay in the body's one-shot continuation owner. Keeping their construction
//! here leaves expression dispatch free of assignment-specific rules.
//!
//! Use `handle_mutation` when the statement parser has only a variable
//! reference, or `handle_mutation_target` when the caller has already
//! resolved the place target (e.g. after field-access parsing). Both return
//! [`MutationOutcome`]: a finished node, or the ordered continuation the body
//! root resumes when the right-hand side still holds numeric origins.
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
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::eval_expression::evaluate_expression;
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, CastFacts, OwnedExpr, PlaceId, PlaceProjectionView,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_with_trailing_newline_policy;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::expressions::parse_expression_places::{
    expression_from_place_expression, place_expression_from_expression,
    place_expression_is_mutable, root_binding_name_of_place,
};
use crate::compiler_frontend::ast::field_access::{ParsedPostfixExpression, parse_field_access};
use crate::compiler_frontend::ast::numeric_construction::{
    AssignmentContinuation, CompoundAssignmentContinuation, DeferredAssignmentTarget,
    NumericContinuation, NumericContinuationId, StatementExpression, deferred_numeric_expression,
};
use crate::compiler_frontend::ast::statements::value_production::receiver::try_parse_value_block_at_receiver;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ParsedReceiverValue, ValueReceiverKind,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::builtins::casts::evidence::lookup_builtin_evidence;
use crate::compiler_frontend::builtins::casts::targets::builtin_cast_target_for_type;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets;

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;
use crate::compiler_frontend::compiler_messages::{
    InvalidAssignmentTargetReason, InvalidFallibleHandlingReason, TypeMismatchContext,
    ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::compatibility::is_type_compatible;
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_explicit_type_boundary;
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedType, cast_target_context_for_type_id,
    is_numeric_literal_destination_type_id, parse_expectation_for_type_id,
};
use crate::compiler_frontend::value_mode::ValueMode;

/// One parsed mutation: a finished node, or a continuation waiting for its retained value.
///
/// WHAT: reports whether the mutation completed at parse time or must resume at the body root.
/// WHY: a right-hand side that still holds numeric origins cannot be evaluated at the receiving
///      site, but the target place and the statement's authored position are already known, so
///      the statement owner records one ordered continuation instead of a fabricated node.
// `Ready` is a one-shot statement node pushed straight into the draft, so it stays inline;
// boxing it would add a heap allocation to every parsed mutation.
#[allow(clippy::large_enum_variant)]
pub(crate) enum MutationOutcome {
    Ready(AstNode),
    /// The statement waits for its retained right-hand side at the body root.
    Retained(NumericContinuationId),
}

/// Build the existing assignment mismatch diagnostic for a compound store.
///
/// WHAT: reports the same incompatibility as a simple assignment when a promoted operation
///       result has no permitted receiving conversion.
/// WHY: receiving conversion stays owned by builtin cast evidence; the promoted arithmetic has
///       already proved the result numeric, so any remaining mismatch is a non-castable pair and
///       retains the assignment boundary's established diagnostic.
fn assignment_type_mismatch(
    expected_type_id: TypeId,
    actual_type_id: TypeId,
    span: Option<SourceSpan>,
) -> ExpressionParseError {
    types_borrows_targets::type_mismatch(
        expected_type_id,
        actual_type_id,
        TypeMismatchContext::Assignment,
        span,
    )
    .into()
}

/// Map a canonical compound-assignment tag to its arithmetic operator and label.
///
/// WHAT: converts `+=`, `-=`, `*=`, `/=`, `//=`, `%=`, `^=` into the corresponding
///       `Operator` variant and a human-readable label used in diagnostics.
/// WHY: compound assignments are desugared into `target = target op rhs`;
///      the stable `TokenTag` taxonomy is the canonical operator authority, so
///      the cursor classifies the operator tag directly. Also consumed by the
///      statement parser so pending-target mutations record the same operator.
pub(crate) fn compound_assignment_operator_for_tag(
    tag: TokenTag,
) -> Option<(Operator, &'static str)> {
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
///       is used to build the desugared RHS expression.
/// WHY: compound assignment evaluation needs the same state as simple
///      assignment plus the operator; bundling keeps the signature readable.
struct CompoundAssignmentInput<'a> {
    /// Declaration of the variable being assigned to.
    variable_declaration: &'a Declaration,
    target: PlaceId,
    target_type_id: TypeId,
    operator: Operator,
}

/// Input bundle for `evaluate_compound_assignment_with_ready_rhs`.
///
/// WHAT: carries the target declaration, the resolved place target and an already-evaluated
///       right-hand operand.
/// WHY: numeric-dependent statements retain their compound RHS until the body root has selected
///      the target's single static type; resuming must not need the token stream that produced it.
pub(crate) struct CompoundAssignmentReadyInput<'a, 'env> {
    /// The authored declaration's value mode: every compound tag is an arithmetic operator, so
    /// the promoted evaluation runs in the declared binding's mode.
    pub(crate) declaration_value_mode: &'a ValueMode,
    pub(crate) target: PlaceId,
    pub(crate) target_type_id: TypeId,
    pub(crate) operator: Operator,
    pub(crate) rhs: OwnedExpr,
    pub(crate) context: &'a ScopeContext,
    pub(crate) type_interner: &'a mut AstTypeInterner<'env>,
    pub(crate) expression_builder: &'a mut AstExpressionBuilder,
    pub(crate) const_values: &'a mut ConstValueStore,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a mut PathInternerFork,
}

/// Complete a compound assignment whose right-hand operand is already evaluated.
///
/// WHAT: evaluates `target op rhs` in the promoted domain, then applies the established store
///       conversion back to the target's exact type.
/// WHY: the promoted arithmetic and the receiving conversion are the same rules whether the RHS
///      arrives straight from the cursor or from body-root numeric completion.
pub(crate) fn evaluate_compound_assignment_with_ready_rhs(
    input: CompoundAssignmentReadyInput<'_, '_>,
) -> Result<OwnedExpr, ExpressionParseError> {
    let CompoundAssignmentReadyInput {
        declaration_value_mode,
        target,
        target_type_id,
        operator,
        rhs,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    } = input;

    let target_span = expression_builder.place(target)?.metadata().span;
    let target_expression = expression_from_place_expression(expression_builder, target)?;
    let operator_item = ExpressionRpnItem::Operator {
        operator,
        span: target_span,
    };
    let mut inferred = ExpectedType::Infer;
    let value = evaluate_expression(
        context,
        vec![
            ExpressionRpnItem::Operand(target_expression.id()),
            ExpressionRpnItem::Operand(rhs.id()),
            operator_item,
        ],
        type_interner,
        expression_builder,
        const_values,
        &mut inferred,
        declaration_value_mode,
        string_table,
        path_fork,
    )?;
    let source_type_id = expression_builder.view_owned(value)?.value_type_id()?;

    if is_type_compatible(target_type_id, source_type_id, type_interner.environment()) {
        return Ok(value);
    }

    let type_environment = type_interner.environment();
    // The promoted arithmetic result is always numeric: every compound tag is an arithmetic
    // operator and `+` never concatenates strings, so the arithmetic evaluation already proved
    // the source supports a cast classification. Let the builtin pair evidence owner decide the
    // destination conversion, including exact `Dec` scales.
    let source_target =
        builtin_cast_target_for_type(source_type_id, type_environment, string_table, path_fork);
    let destination_target =
        builtin_cast_target_for_type(target_type_id, type_environment, string_table, path_fork);
    let cast_targets = source_target.zip(destination_target);
    let evidence =
        cast_targets.and_then(|(source, target)| lookup_builtin_evidence(source, target));
    let Some(evidence) = evidence else {
        return Err(assignment_type_mismatch(
            target_type_id,
            source_type_id,
            expression_builder.view_owned(value)?.span(),
        ));
    };

    let cast_span = expression_builder.view_owned(value)?.span();
    let cast = CastFacts {
        source_type_id,
        target_type_id,
        requires_optional_wrap_after_cast: false,
        evidence: ResolvedCastEvidence::Builtin {
            policy: evidence.policy,
        },
        fallibility: evidence.fallibility,
        handling: CastHandling::StoreConversion,
        span: cast_span,
    };
    let cast_value = expression_builder.cast(value, cast, target_type_id)?;
    let mut inferred = ExpectedType::Infer;
    let converted_value = evaluate_expression(
        context,
        vec![ExpressionRpnItem::Operand(cast_value.id())],
        type_interner,
        expression_builder,
        const_values,
        &mut inferred,
        declaration_value_mode,
        string_table,
        path_fork,
    )?;

    Ok(converted_value)
}

/// WHAT: gives fixed-scalar and non-power `Dec` targets a numeric literal hint, then evaluates
///       `target op rhs` in the promoted domain.
/// WHY: `Dec ^ I32` keeps the exponent in the `I32` domain even when the compound assignment's
///      receiving type is a Dec scale, while unsigned fixed-scalar powers keep the exponent in
///      their unsigned peer domain.
#[allow(
    clippy::too_many_arguments,
    reason = "compound assignment evaluation keeps the token stream, scope, input, type interner, expression builder, const store, string table, and path fork as separate borrows"
)]
fn evaluate_compound_assignment_value(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    input: CompoundAssignmentInput<'_>,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<OwnedExpr, ExpressionParseError> {
    let CompoundAssignmentInput {
        variable_declaration,
        target,
        target_type_id,
        operator,
    } = input;
    let type_environment = type_interner.environment();
    let target_is_number = type_environment.number_scale(target_type_id).is_some();
    // Dec power keeps a pending compound RHS in I32 instead of its Dec peer.
    let number_power_exponent = target_is_number && matches!(operator, Operator::Exponent);
    let peer_numeric_literal =
        is_numeric_literal_destination_type_id(target_type_id, type_environment)
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
    // The compound right-hand side is this statement's whole authored operand, so a deferral
    // raised here is a complete fragment the statement owner can retain.
    let mut cast_target_context = CastTargetContext::None;
    let rhs_input = ExpressionParseInput::ordinary(
        ExpressionParseResources {
            token_stream,
            scope_context: &rhs_context,
            type_interner,
            expression_builder,
            const_values,
            expected_type: &mut expr_type,
            cast_target_context: &mut cast_target_context,
            value_mode: &variable_declaration.value_mode,
            string_table,
            path_fork,
        },
        false,
    )
    .with_numeric_deferral_owner();
    let rhs = create_expression_with_trailing_newline_policy(rhs_input)?;

    // -------------------------------------------
    //  Evaluate `target op rhs` in its promoted
    //  domain before applying the store conversion
    // -------------------------------------------
    evaluate_compound_assignment_with_ready_rhs(CompoundAssignmentReadyInput {
        declaration_value_mode: &variable_declaration.value_mode,
        target,
        target_type_id,
        operator,
        rhs,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    })
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
    target: PlaceId,
    declaration_span: Option<SourceSpan>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<MutationOutcome, ExpressionParseError> {
    let span = Some(token_stream.current_span());
    let target_type_id = expression_builder.place(target)?.metadata().type_id;

    ast_log!(
        "Handling mutation for ",
        #variable_declaration.value_mode, " ",
        Blue path_fork.render_portable(variable_declaration.id, string_table, &mut Vec::new())
    );

    // -----------------------
    //  Validate mutability
    // -----------------------

    let (is_mutable, field_name) = {
        let target_view = expression_builder.place(target)?;
        let is_mutable = place_expression_is_mutable(target_view);
        let field_name = target_view
            .projections()
            .filter_map(|projection| match projection {
                PlaceProjectionView::Field { field, .. } => Some(field),
                PlaceProjectionView::Index { .. } => None,
            })
            .last();
        (is_mutable, field_name)
    };
    if !is_mutable {
        let root_binding_name = if field_name.is_some() {
            let target_view = expression_builder.place(target)?;
            root_binding_name_of_place(target_view, path_fork)
        } else {
            None
        };
        let reason = if field_name.is_some() {
            InvalidAssignmentTargetReason::ImmutableFieldRoot
        } else {
            InvalidAssignmentTargetReason::ImmutableBinding
        };
        let diagnostic = names_declarations::invalid_assignment_target(
            reason,
            path_fork.component(variable_declaration.id),
            field_name,
            root_binding_name,
            span,
        );
        let diagnostic = if let Some(declaration_span) = declaration_span {
            diagnostic.with_label(
                Some(declaration_span),
                Some(LabelDraftMessage::ImmutableBindingDeclaration),
            )
        } else {
            diagnostic
        };
        return Err(diagnostic.into());
    }

    // -----------------------
    //  Determine mutation kind
    // -----------------------

    let value = match token_stream.current_tag() {
        TokenTag::ASSIGN => {
            // Simple assignment keeps the receiver hint for context-sensitive values. Compound
            // RHSs peer numeric literals for fixed scalars and non-power Dec targets before
            // promoted evaluation.
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

            let rhs_result = if let Some(value_block_result) = try_parse_value_block_at_receiver(
                token_stream,
                &rhs_context,
                type_interner,
                expression_builder,
                const_values,
                &[target_type_id],
                ValueReceiverKind::Assignment,
                string_table,
                path_fork,
            ) {
                match value_block_result {
                    Ok(ParsedReceiverValue::Complete(expression)) => Ok(expression),
                    Ok(ParsedReceiverValue::Retained(retained)) => {
                        Err(retained.into_deferral(&variable_declaration.value_mode))
                    }
                    Ok(ParsedReceiverValue::NeedsSlotInference { .. }) => crate::compiler_bug!(
                        CompilerStage::AstConstruction,
                        "an assignment receiver knows its target type; a value block cannot ask for slot inference"
                    ),
                    Err(error) => Err(error),
                }
            } else {
                let input = ExpressionParseInput::ordinary(
                    ExpressionParseResources {
                        token_stream,
                        scope_context: &rhs_context,
                        type_interner,
                        expression_builder,
                        const_values,
                        expected_type: &mut expr_type,
                        cast_target_context: &mut cast_target_context,
                        value_mode: &variable_declaration.value_mode,
                        string_table,
                        path_fork,
                    },
                    false,
                )
                .with_numeric_deferral_owner();
                create_expression_with_trailing_newline_policy(input)
            };
            let rhs = match rhs_result {
                Ok(rhs) => rhs,
                Err(error) => {
                    return retain_ready_target_mutation(
                        error,
                        target,
                        variable_declaration,
                        declaration_span,
                        None,
                        span,
                        context,
                    );
                }
            };
            expression_builder
                .view_owned(rhs)?
                .require_single(ValueReceiverContext::Assignment)?;

            // Direct option fallback is rejected at each closed receiver so the
            // later statement parser does not report the trailing `else` as an
            // unrelated branch error.
            token_stream.skip_newlines();
            if token_stream.current_tag() == TokenTag::ELSE
                && type_interner
                    .environment()
                    .option_inner_type(expression_builder.view_owned(rhs)?.value_type_id()?)
                    .is_some()
            {
                return Err(choices_control_flow::invalid_fallible_handling(
                    InvalidFallibleHandlingReason::DirectOptionFallbackSyntax,
                    Some(token_stream.current_span()),
                )
                .into());
            }

            coerce_expression_to_explicit_type_boundary(
                rhs,
                target_type_id,
                type_interner.environment(),
                TypeMismatchContext::Assignment,
                expression_builder,
            )?
        }

        compound_tag => {
            let Some((operator, _label)) = compound_assignment_operator_for_tag(compound_tag)
            else {
                return Err(names_declarations::invalid_assignment_target(
                    InvalidAssignmentTargetReason::ExpectedAssignmentOperator,
                    path_fork.component(variable_declaration.id),
                    None,
                    None,
                    span,
                )
                .into());
            };

            token_stream.advance();

            match evaluate_compound_assignment_value(
                token_stream,
                context,
                CompoundAssignmentInput {
                    variable_declaration,
                    target,
                    target_type_id,
                    operator,
                },
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            ) {
                Ok(value) => value,
                Err(error) => {
                    return retain_ready_target_mutation(
                        error,
                        target,
                        variable_declaration,
                        declaration_span,
                        Some(operator),
                        span,
                        context,
                    );
                }
            }
        }
    };

    Ok(MutationOutcome::Ready(AstNode {
        kind: NodeKind::Assignment {
            target,
            value: value.id(),
        },
        span,
        scope: context.scope,
    }))
}

/// Retain a mutation whose ready target's right-hand side still needs numeric resolution.
///
/// WHAT: converts a raised numeric deferral into the ordered continuation the body root resumes,
///       carrying the authored target place, the declaration facts the store boundary needs and
///       the retained right-hand side.
/// WHY: the target place already exists, so only the value is numeric-dependent; the statement
///      must resume in its authored position instead of fabricating a node or refusing legal
///      source, and the store boundary keeps the authored declaration's value mode and span
///      rather than a synthetic declaration.
fn retain_ready_target_mutation(
    error: ExpressionParseError,
    target: PlaceId,
    variable_declaration: &Declaration,
    declaration_span: Option<SourceSpan>,
    operator: Option<Operator>,
    span: Option<SourceSpan>,
    context: &ScopeContext,
) -> Result<MutationOutcome, ExpressionParseError> {
    let deferral = error.take_numeric_deferral("the right-hand side of an assignment")?;
    let value = StatementExpression::Retained(deferred_numeric_expression(
        deferral.items,
        deferral.expected,
        deferral.value_mode,
        deferral.span,
        deferral.suffix,
    ));
    let assignment_target = DeferredAssignmentTarget::Ready {
        target,
        declaration_value_mode: variable_declaration.value_mode.to_owned(),
        declaration_span,
    };
    let continuation = match operator {
        Some(operator) => NumericContinuation::CompoundAssignment(CompoundAssignmentContinuation {
            target: assignment_target,
            operator,
            value,
            span,
            scope: context.scope,
        }),
        None => NumericContinuation::Assignment(AssignmentContinuation {
            target: assignment_target,
            value,
            span,
            scope: context.scope,
        }),
    };
    let continuation = context.register_numeric_continuation(continuation)?;
    Ok(MutationOutcome::Retained(continuation))
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
    target: PlaceId,
    declaration_span: Option<SourceSpan>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<MutationOutcome, ExpressionParseError> {
    build_mutation_from_target(
        token_stream,
        variable_declaration,
        target,
        declaration_span,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )
}

/// Handle mutation of an existing mutable variable.
///
/// WHAT: parses field-access chains on the variable, then builds the mutation
///       node through `build_mutation_from_target`.
#[allow(
    clippy::too_many_arguments,
    reason = "mutation handling keeps the token stream, declaration, span, scope, type interner, expression builder, const store, string table, and path fork as separate borrows at the shared mutation entry point"
)]
pub fn handle_mutation(
    token_stream: &mut AstCursor,
    variable_declaration: &Declaration,
    declaration_span: Option<SourceSpan>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<MutationOutcome, ExpressionParseError> {
    let target_expression = parse_field_access(
        token_stream,
        variable_declaration,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )?;
    let target_expression = match target_expression {
        ParsedPostfixExpression::Value(expression) => expression,
        // A pending receiver call has no value or place yet and can never be a mutation target; the
        // authored operator is the source boundary that reports it.
        ParsedPostfixExpression::RetainedOperand(_) => {
            return Err(names_declarations::invalid_assignment_target(
                InvalidAssignmentTargetReason::TemporaryNotAssignable,
                None,
                None,
                None,
                Some(token_stream.current_span()),
            )
            .into());
        }
    };

    let Some(target) = place_expression_from_expression(expression_builder, target_expression)?
    else {
        return Err(names_declarations::invalid_assignment_target(
            InvalidAssignmentTargetReason::TemporaryNotAssignable,
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
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )
}

#[cfg(test)]
#[path = "tests/mutation_tests.rs"]
mod mutation_tests;
