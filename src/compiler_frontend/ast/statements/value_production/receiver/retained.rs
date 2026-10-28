//! Retained value-producing receiver constructs.
//!
//! WHAT: holds one value block whose nested statement bodies still contain numeric continuations,
//!       together with the exact constructor routing that built it, and rebuilds that block once
//!       the enclosing body root resumes those bodies.
//! WHY: a value block is an expression over statement bodies. Its bodies share the enclosing
//!      function's inference boundary, so the block cannot be built at its receiving site; the
//!      retained record keeps the authored inputs and the block's own form so completion re-runs
//!      the existing constructor, slot inference and closure checks exactly once at the root.

use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::error::{ExpressionParseError, NumericDeferral};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, single_operand,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, OwnedExpr,
};
use crate::compiler_frontend::ast::generic_functions::IfGenericRequestRanges;
use crate::compiler_frontend::ast::numeric_construction::{
    BodyEntry, DeferredArmPattern, DeferredBody, DeferredConstruct, DeferredExpression,
    DeferredExpressionCompletion, DeferredExpressionSuffix, DeferredIf, DeferredMatch,
    MultiBindProducerFact, NumericConstructionState, NumericContinuation, PendingMultiBindProducer,
    StatementExpression, pending_multi_bind_producer_kind, retained_known_shape,
};
use crate::compiler_frontend::ast::statements::branching::{
    DeferredMatchBodyReceiver, resume_deferred_match,
};
use crate::compiler_frontend::ast::statements::match_patterns::{
    ensure_pattern_compatible_with_subject, finish_pending_choice_pattern,
    finish_pending_option_pattern, finish_retained_pattern,
};
use crate::compiler_frontend::ast::statements::value_production::completeness::{
    analyze_branch_exits, visit_reachable_then_values,
};
use crate::compiler_frontend::ast::statements::value_production::multi_bind::finalize_inferred_value_block;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, BranchExitSummary, ParsedReceiverValue, ProducedValues,
    ThenValueSlot,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::ValueReceiverContext;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

use super::block_body::ValueBlockBranchBody;
use super::block_if::{BlockValueIfFinishInput, finish_block_value_if};
use super::block_match::{BlockValueMatchFinishInput, finish_block_value_match};
use super::full_match::{ValueMatchFinishInput, finish_full_value_match};

/// Which value-producing constructor owns one retained block.
///
/// WHAT: names the authored form together with every input the form's constructor needs.
/// WHY: each form runs a different closure check, result-slot inference and expression builder;
///      keeping the form explicit lets completion call the same constructor the parse path used.
#[derive(Debug, Clone)]
// The form lives inside the already-boxed `DeferredValueBlock` (`DeferredExpression::ValueBlock`),
// so its variant size is paid once per retained block; boxing the large variants would add a
// second allocation per block.
#[allow(clippy::large_enum_variant)]
pub(crate) enum DeferredValueBlockForm {
    /// Block bool `if condition: <then> else <else>`.
    If {
        condition: StatementExpression,
        then_body: ValueBlockBranchBody,
        else_body: ValueBlockBranchBody,
        generic_request_ranges: IfGenericRequestRanges,
    },
    /// Block single-predicate match (`if <scrutinee> is <pattern>: <then> else <else>`).
    SinglePredicate {
        scrutinee: StatementExpression,
        /// The committed pattern: ready, or retained until the subject type is selected.
        pattern: DeferredArmPattern,
        then_body: ValueBlockBranchBody,
        else_body: ValueBlockBranchBody,
    },
    /// Full value match (`if <subject> is: <arms> else => ...`) at a closed receiver.
    ///
    /// WHY: a pending subject, pattern, guard or arm body keeps the whole match until the body
    ///      root has selected every numeric origin's type; the value finish then runs once through
    ///      `finish_full_value_match`, sharing completeness and slot inference with the parse path.
    FullMatch { match_block: DeferredMatch },
}

/// One retained value-producing block.
///
/// WHAT: the authored form inputs plus the receiving routing that must survive until completion.
/// WHY: result-slot inference, arity and receiver-aware coercion are authored decisions, so the
///      completion session must re-run them with the same target instead of inventing slots.
#[derive(Debug, Clone)]
pub(crate) struct DeferredValueBlock {
    pub(crate) form: DeferredValueBlockForm,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) target: ActiveValueProductionTarget,
    /// The receiving slots a resumed multi-bind completed for this inferred block.
    ///
    /// WHAT: `None` until a multi-bind resumes the retained block, which then records the type each
    ///       target resolved to.
    /// WHY: a dead `then` group must materialise at its resolved slot. Inference never reads these
    ///      slots and still uses only the reachable producers in `target`.
    pub(crate) resolved_slot_types: Option<Vec<TypeId>>,
}

impl DeferredValueBlock {
    /// Read real producer facts for each slot before a retained multi-bind block completes.
    ///
    /// WHAT: collects ready expression types and retained `then` recipe/ID routes from every
    ///       reachable producing branch, including nested deferred `if` and match bodies.
    /// WHY: the multi-bind target must be visible at its authored position, while the value block
    ///      and every producer expression still complete once through their existing owners.
    pub(crate) fn multi_bind_producer_facts(
        &self,
        state: &NumericConstructionState,
        expressions: &AstExpressionBuilder,
        constants: &ConstValueStore,
    ) -> Result<Vec<Vec<MultiBindProducerFact>>, ExpressionParseError> {
        let mut slots = (0..self.target.expected_value_count())
            .map(|_| Vec::new())
            .collect::<Vec<_>>();
        match &self.form {
            DeferredValueBlockForm::If {
                then_body,
                else_body,
                ..
            }
            | DeferredValueBlockForm::SinglePredicate {
                then_body,
                else_body,
                ..
            } => {
                collect_branch_producers(then_body, state, expressions, constants, &mut slots)?;
                collect_branch_producers(else_body, state, expressions, constants, &mut slots)?;
            }
            DeferredValueBlockForm::FullMatch { match_block } => {
                collect_match_producers(match_block, state, expressions, constants, &mut slots)?;
            }
        }
        Ok(slots)
    }
}

fn collect_branch_producers(
    body: &ValueBlockBranchBody,
    state: &NumericConstructionState,
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    slots: &mut [Vec<MultiBindProducerFact>],
) -> Result<(), ExpressionParseError> {
    match body {
        ValueBlockBranchBody::Nodes { nodes, .. } => visit_reachable_then_values(
            nodes,
            &mut |values| collect_ready_producers(values, expressions, slots),
            expressions,
            constants,
        ),
        ValueBlockBranchBody::Draft(body) => {
            collect_deferred_body_producers(body, state, expressions, constants, slots).map(|_| ())
        }
    }
}

fn collect_ready_producers(
    values: &ProducedValues,
    expressions: &AstExpressionBuilder,
    slots: &mut [Vec<MultiBindProducerFact>],
) -> Result<(), ExpressionParseError> {
    for (index, expression) in values.expressions.iter().enumerate() {
        if let Some(slot) = slots.get_mut(index) {
            slot.push(MultiBindProducerFact::Known(
                expressions
                    .view(*expression)?
                    .require_single(ValueReceiverContext::Then)?,
            ));
        }
    }
    Ok(())
}

fn collect_retained_then_producers(
    continuation_id: crate::compiler_frontend::ast::numeric_construction::NumericContinuationId,
    continuation: &crate::compiler_frontend::ast::statements::value_production::ThenValueContinuation,
    expressions: &AstExpressionBuilder,
    slots: &mut [Vec<MultiBindProducerFact>],
) -> Result<(), ExpressionParseError> {
    for (index, value) in continuation.values.iter().enumerate() {
        let Some(slot) = slots.get_mut(index) else {
            continue;
        };
        match value {
            ThenValueSlot::Pending(StatementExpression::Built(expression))
            | ThenValueSlot::Completed(expression) => {
                slot.push(MultiBindProducerFact::Known(
                    expressions
                        .view(*expression)?
                        .require_single(ValueReceiverContext::Then)?,
                ));
            }
            ThenValueSlot::Pending(StatementExpression::Retained(deferred)) => {
                if let Some(type_id) = retained_single_result_type(&deferred.items, expressions)? {
                    slot.push(MultiBindProducerFact::Known(type_id));
                } else {
                    slot.push(MultiBindProducerFact::Pending(PendingMultiBindProducer {
                        continuation: continuation_id,
                        value_index: index,
                        kind: pending_multi_bind_producer_kind(&deferred.items),
                        span: deferred.span,
                        shape: retained_known_shape(&deferred.items),
                    }));
                }
            }
            ThenValueSlot::Taken => {
                return Err(InfrastructureFailure::internal(CompilerStage::AstConstruction,
                    "multi-bind producer discovery reached a `then` value already owned by completion",
                )
                .into());
            }
        }
    }
    Ok(())
}

fn retained_single_result_type(
    items: &[ExpressionRpnItem],
    expressions: &AstExpressionBuilder,
) -> Result<Option<TypeId>, ExpressionParseError> {
    let result = match single_operand(items) {
        Some(ExpressionRpnItem::Operand(expression)) => Some(
            expressions
                .view(*expression)?
                .require_single(ValueReceiverContext::Then)?,
        ),
        Some(ExpressionRpnItem::PendingNumericCall { call, .. }) => {
            call.single_success_return_type_id()
        }
        Some(ExpressionRpnItem::PendingNumericCast { target_type_id, .. }) => Some(*target_type_id),
        _ => None,
    };
    Ok(result)
}

fn collect_deferred_body_producers(
    body: &DeferredBody,
    state: &NumericConstructionState,
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    slots: &mut [Vec<MultiBindProducerFact>],
) -> Result<BranchExitSummary, ExpressionParseError> {
    let mut exits = BranchExitSummary::FALLS_THROUGH;
    for entry in &body.draft.entries {
        if !exits.can_fall_through {
            break;
        }
        let statement_exits = match entry {
            BodyEntry::Node(node) => {
                visit_reachable_then_values(
                    std::slice::from_ref(node),
                    &mut |values| collect_ready_producers(values, expressions, slots),
                    expressions,
                    constants,
                )?;
                analyze_branch_exits(std::slice::from_ref(node), expressions, constants)
            }
            BodyEntry::Pending(id) => {
                collect_pending_continuation_producers(*id, state, expressions, constants, slots)?
            }
        };
        exits = exits.then_sequence(statement_exits);
    }
    Ok(exits)
}

fn collect_pending_continuation_producers(
    id: crate::compiler_frontend::ast::numeric_construction::NumericContinuationId,
    state: &NumericConstructionState,
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    slots: &mut [Vec<MultiBindProducerFact>],
) -> Result<BranchExitSummary, ExpressionParseError> {
    if let Some(continuation) = state.continuation_then_values(id) {
        collect_retained_then_producers(id, continuation, expressions, slots)?;
        return Ok(BranchExitSummary::PRODUCES);
    }
    let continuation = state.continuation(id).ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "multi-bind producer discovery addressed a continuation outside its body domain",
        )
    })?;
    match continuation {
        NumericContinuation::ThenValue(_) => Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "a live `then` continuation was not exposed through its producer-slot accessor",
        )
        .into()),
        NumericContinuation::Return(_) => Ok(BranchExitSummary::TERMINATES),
        NumericContinuation::Construct(construct) => {
            collect_construct_producers(construct, state, expressions, constants, slots)
        }
        _ => Ok(BranchExitSummary::FALLS_THROUGH),
    }
}

fn collect_construct_producers(
    construct: &DeferredConstruct,
    state: &NumericConstructionState,
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    slots: &mut [Vec<MultiBindProducerFact>],
) -> Result<BranchExitSummary, ExpressionParseError> {
    match construct {
        DeferredConstruct::If(deferred) => {
            collect_if_producers(deferred, state, expressions, constants, slots)
        }
        DeferredConstruct::Match(deferred) => {
            collect_match_exits_and_producers(deferred, state, expressions, constants, slots)
        }
        DeferredConstruct::Loop(_) => Ok(BranchExitSummary::FALLS_THROUGH),
    }
}

fn collect_if_producers(
    deferred: &DeferredIf,
    state: &NumericConstructionState,
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    slots: &mut [Vec<MultiBindProducerFact>],
) -> Result<BranchExitSummary, ExpressionParseError> {
    let then_exits =
        collect_deferred_body_producers(&deferred.then_body, state, expressions, constants, slots)?;
    let else_exits = match &deferred.else_body {
        Some(body) => collect_deferred_body_producers(body, state, expressions, constants, slots)?,
        None => BranchExitSummary::FALLS_THROUGH,
    };
    Ok(then_exits.union(else_exits))
}

fn collect_match_exits_and_producers(
    deferred: &DeferredMatch,
    state: &NumericConstructionState,
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    slots: &mut [Vec<MultiBindProducerFact>],
) -> Result<BranchExitSummary, ExpressionParseError> {
    let mut exits = None;
    for arm in &deferred.arms {
        let arm_exits =
            collect_deferred_body_producers(&arm.body, state, expressions, constants, slots)?;
        exits = Some(exits.map_or(arm_exits, |existing: BranchExitSummary| {
            existing.union(arm_exits)
        }));
    }
    if let Some(default) = &deferred.default {
        let default_exits =
            collect_deferred_body_producers(default, state, expressions, constants, slots)?;
        exits = Some(exits.map_or(default_exits, |existing| existing.union(default_exits)));
    }
    Ok(exits.unwrap_or(BranchExitSummary::FALLS_THROUGH))
}

fn collect_match_producers(
    deferred: &DeferredMatch,
    state: &NumericConstructionState,
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    slots: &mut [Vec<MultiBindProducerFact>],
) -> Result<(), ExpressionParseError> {
    for arm in &deferred.arms {
        collect_deferred_body_producers(&arm.body, state, expressions, constants, slots)?;
    }
    if let Some(default) = &deferred.default {
        collect_deferred_body_producers(default, state, expressions, constants, slots)?;
    }
    Ok(())
}

/// The operand item and authored span a retained value block contributes to its position.
///
/// WHAT: a receiving site splices this as the one retained item of its own position fragment.
/// WHY: the site owns the position's expectation and value mode, so it must be able to wrap a
///      retained block into its own lane without the receiver guessing that context.
#[derive(Debug, Clone)]
pub(crate) struct RetainedValueBlock {
    pub(crate) item: ExpressionRpnItem,
    pub(crate) span: Option<SourceSpan>,
}

impl RetainedValueBlock {
    /// Wrap one retained value block as the operand item of its receiving position.
    pub(crate) fn value_block(block: DeferredValueBlock, span: Option<SourceSpan>) -> Self {
        Self {
            item: ExpressionRpnItem::PendingNumericExpression {
                expression: Box::new(DeferredExpression::ValueBlock(Box::new(block))),
                span,
            },
            span,
        }
    }

    /// Raise the deferral that hands this retained block to the enclosing body root.
    ///
    /// WHAT: splices the block's single retained item into the receiving position's deferral with
    ///       the position's authored value mode.
    /// WHY: the block's bodies hold the numeric continuations, and the block is the position's
    ///      whole authored syntax, so the position hands it to the root verbatim instead of
    ///      guessing a typed value — one splice shape shared by every receiving lane.
    pub(crate) fn into_deferral(self, value_mode: &ValueMode) -> ExpressionParseError {
        ExpressionParseError::NumericDeferral(Box::new(NumericDeferral {
            items: vec![self.item],
            expected: ExpectedType::Infer,
            value_mode: value_mode.to_owned(),
            span: self.span,
            suffix: DeferredExpressionSuffix::None,
            complete_fragment: true,
        }))
    }
}

/// Rebuild one retained value block at the enclosing body root.
///
/// WHAT: resolves the form's retained predicates and bodies, re-runs the form's constructor with
///       the authored receiving target, finishes mixed-slot inference when the receiver asked for
///       it, and returns the finished expression.
/// WHY: the block's bodies and produced values only become concrete once the body root has
///      selected every numeric origin's type; rebuilding through the same constructor keeps one
///      inference and one closure check for both the parse-time and the resumed path.
pub(crate) fn complete_deferred_value_block<'env>(
    block: Box<DeferredValueBlock>,
    completion: &mut impl DeferredExpressionCompletion<'env>,
) -> Result<OwnedExpr, ExpressionParseError> {
    let DeferredValueBlock {
        form,
        span,
        target,
        resolved_slot_types,
    } = *block;

    let receiver_kind = target.receiver_kind;
    let needs_slot_inference = target.needs_slot_inference();
    let expected_result_type_ids = target.result_type_ids;

    // Known slots win; a resumed inferred multi-bind supplies the slots it completed. Inference
    // never reads these, so a dead `then` group materialises without contributing a type.
    let own_contract = (!expected_result_type_ids.is_empty())
        .then_some(expected_result_type_ids.as_slice())
        .or(resolved_slot_types.as_deref());

    let finished = match form {
        DeferredValueBlockForm::If {
            condition,
            then_body,
            else_body,
            generic_request_ranges,
        } => {
            let condition = completion.resolve_statement_expression(condition)?;
            let then_scope = then_body.scope();
            let else_scope = else_body.scope();
            let then_body = resolve_branch_body(then_body, own_contract, completion)?;
            let else_body = resolve_branch_body(else_body, own_contract, completion)?;
            let services = completion.session_services();
            finish_block_value_if(BlockValueIfFinishInput {
                condition: condition.id(),
                then_body,
                else_body,
                then_scope,
                else_scope,
                span,
                generic_request_ranges,
                expected_result_type_ids,
                receiver_kind,
                needs_slot_inference,
                expression_builder: services.expression_builder,
                const_values: services.const_values,
            })?
        }
        DeferredValueBlockForm::SinglePredicate {
            scrutinee,
            pattern,
            then_body,
            else_body,
        } => {
            let scrutinee = completion.resolve_statement_expression(scrutinee)?;
            let subject_type_id = completion
                .session_services()
                .expression_builder
                .view_owned(scrutinee)?
                .require_single(ValueReceiverContext::MatchScrutinee)?;

            // The pattern materialises at the selected subject type: a retained draft resolves its
            // fragment there, a ready pattern was parsed before a pending subject's type existed,
            // and a pending choice or option pattern takes its capture types from the selected
            // subject.
            // A numeric literal hint is not a strict expectation, so both shapes are checked
            // against the resolved subject exactly as the parse path checks a ready one.
            let pattern = match pattern {
                DeferredArmPattern::Ready(pattern) => pattern,
                DeferredArmPattern::PendingChoice(pending) => {
                    let services = completion.session_services();
                    finish_pending_choice_pattern(
                        pending,
                        subject_type_id,
                        services.type_interner.environment(),
                    )?
                }
                DeferredArmPattern::PendingOption(pending) => {
                    finish_pending_option_pattern(pending, subject_type_id, completion)?
                }
                DeferredArmPattern::Retained(draft) => {
                    let value = completion.resolve_fragment(
                        draft.value.items,
                        ExpectedType::NumericLiteral(subject_type_id),
                        &draft.value.value_mode,
                    )?;
                    let services = completion.session_services();
                    finish_retained_pattern(
                        draft.relational,
                        services.expression_builder.view_owned(value)?,
                    )?
                }
            };

            let services = completion.session_services();
            ensure_pattern_compatible_with_subject(
                &pattern,
                subject_type_id,
                services.type_interner.environment(),
            )?;

            let then_body = resolve_branch_body(then_body, own_contract, completion)?;
            let else_body = resolve_branch_body(else_body, own_contract, completion)?;
            let services = completion.session_services();
            finish_block_value_match(BlockValueMatchFinishInput {
                scrutinee: scrutinee.id(),
                pattern,
                then_body,
                else_body,
                span,
                expected_result_type_ids,
                receiver_kind,
                needs_slot_inference,
                expression_builder: services.expression_builder,
                const_values: services.const_values,
            })?
        }
        DeferredValueBlockForm::FullMatch { match_block } => {
            let resumed = resume_deferred_match(
                match_block,
                DeferredMatchBodyReceiver::Own(own_contract),
                completion,
            )?;
            let services = completion.session_services();
            finish_full_value_match(ValueMatchFinishInput {
                scrutinee: resumed.scrutinee,
                arms: resumed.arms,
                default: resumed.default,
                exhaustiveness: resumed.exhaustiveness,
                span: span.or(resumed.span),
                expected_result_type_ids,
                receiver_kind,
                needs_slot_inference,
                expression_builder: services.expression_builder,
                const_values: services.const_values,
            })?
        }
    };

    match finished {
        ParsedReceiverValue::Complete(expression) => Ok(expression),
        ParsedReceiverValue::NeedsSlotInference { block, span } => {
            let target_count = target.expected_arity.ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "a resumed value block asked for slot inference without an authored arity",
                )
            })?;
            let services = completion.session_services();
            finalize_inferred_value_block(
                block,
                span,
                &target.known_slot_types,
                target_count,
                services.type_interner,
                services.expression_builder,
                services.const_values,
            )
        }
        ParsedReceiverValue::Retained(_) => Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "a resumed value block retained itself instead of completing",
        )
        .into()),
    }
}

/// Resolve one retained value-block branch into its authored statement nodes.
///
/// WHAT: publishes a branch that completed at its parse position directly, and asks the
///       completion session to splice a branch that retained numeric continuations.
/// WHY: a concrete branch must not be re-wrapped or re-resolved, while a retained branch must
///      resume through the session's single construction state — under the receiving contract of
///      the block that owns the branch, never an enclosing site's override.
fn resolve_branch_body<'env>(
    body: ValueBlockBranchBody,
    contract: Option<&[TypeId]>,
    completion: &mut impl DeferredExpressionCompletion<'env>,
) -> Result<Vec<AstNode>, ExpressionParseError> {
    match body {
        ValueBlockBranchBody::Nodes { nodes, .. } => Ok(nodes),
        ValueBlockBranchBody::Draft(body) => {
            completion.resolve_deferred_body_with_expectations(body, contract)
        }
    }
}
