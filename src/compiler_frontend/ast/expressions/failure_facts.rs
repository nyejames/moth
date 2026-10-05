//! Origin-owned failure witnesses and compact expression compatibility summaries.
//!
//! Ancestors retain only the first diagnostic witnesses and eligibility metadata. The semantic
//! classifier collects origin witnesses once for function projection; folding discharges only
//! the checked work it owns. Catch disposition protects work, never its handler.

use super::expression::ExpressionKind;
use super::expression_rpn::ExpressionRpnItem;
use super::expression_types::{CastHandling, FallibleExpressionHandling, ResolvedCastEvidence};
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::compiler_messages::BuiltinFailureWitness;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericOperator, numeric_failure_codes, numeric_operation_cannot_fail,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExpressionFailureFacts {
    pub(crate) summary: FailureSummary,
    /// Witnesses introduced at this expression, never copies of descendants' witnesses.
    pub(crate) implicit: Vec<ImplicitFailureContributor>,
    pub(crate) typed_error: Option<TypedErrorProducer>,
    /// Stored at the protected expression; candidates are collected there during projection.
    pub(crate) deferred_custom_catch: Option<DeferredCustomCatchCheck>,
    /// A checked operation remains catch-eligible even when constant folding discharges it.
    pub(crate) checked_numeric_operation: bool,
    /// Explicit `!`, `cast!` and `?` leave the function, never target an enclosing catch.
    pub(crate) postfix_exit_span: Option<SourceSpan>,
    pub(crate) disposition: FailureDisposition,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FailureSummary {
    pub(crate) first_implicit: Option<ImplicitFailureContributorSummary>,
    pub(crate) first_numeric: Option<ImplicitFailureContributorSummary>,
    pub(crate) first_private_call: Option<ImplicitFailureContributorSummary>,
    pub(crate) first_typed: Option<TypedErrorProducer>,
    pub(crate) conflicting_typed: Option<TypedErrorProducer>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ImplicitFailureContributorSummary {
    pub(crate) span: Option<SourceSpan>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ImplicitFailureContributor {
    pub(crate) span: Option<SourceSpan>,
    pub(crate) codes: &'static [BuiltinErrorCode],
    pub(crate) source: ImplicitFailureSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImplicitFailureSource {
    NumericOperation,
    /// Checked conversion back into a compound-assignment target.
    ///
    /// WHAT: carries the canonical destination type of a compiler-inserted
    ///       `StoreConversion` cast. The write-back runs after the compound
    ///       arithmetic on already-handled operands, so the diagnostic names
    ///       the target instead of blaming that earlier work.
    /// WHY: recording the target at the contributor keeps the AST-to-HIR
    ///      projection and both witness builders lossless without a new
    ///      diagnostic family.
    CompoundWriteBack {
        target: TypeId,
        /// The compound operator's exact identity, never an unrelated RHS or prior statement.
        arithmetic_span: Option<SourceSpan>,
    },
    PrivateCall(PathId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TypedErrorProducer {
    pub(crate) span: Option<SourceSpan>,
    pub(crate) error_type_id: TypeId,
    /// Compiler-owned member identity retained for deferred builtin-call diagnostics.
    pub(crate) builtin_name: Option<StringId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DeferredCustomCatchCheck {
    pub(crate) catch_span: Option<SourceSpan>,
    pub(crate) error_type_id: TypeId,
    pub(crate) typed_producer_span: Option<SourceSpan>,
}

/// Shared AST/HIR witness policy. Inputs are active contributors in evaluation order.
/// Only the arithmetic of the same compound update yields priority to its write-back.
pub(crate) fn select_failure_witness<'a, T: 'a, E>(
    contributors: impl Iterator<Item = Result<&'a T, E>>,
    site: impl Fn(&T) -> FailureWitnessSite,
) -> Result<Option<&'a T>, E> {
    let mut first = None;
    for contributor in contributors {
        let contributor = contributor?;
        let Some(selected) = first else {
            first = Some(contributor);
            continue;
        };
        if let (
            FailureWitnessSite::Arithmetic(Some(span)),
            FailureWitnessSite::WriteBack(Some(arithmetic_span)),
        ) = (site(selected), site(contributor))
            && span == arithmetic_span
        {
            first = Some(contributor);
        }
    }
    Ok(first)
}

/// One witness-search step over a call contributor.
///
/// WHAT: distinguishes the terminal producer or unavailable leaf from a skippable recursive
///       back-edge. Only `SkippedBackEdge` permits trying the next contributor; a terminal
///       leaf keeps its recorded path with no codes or origin.
/// WHY: an unavailable hop is already the visible boundary, so later contributors must not
///      supply a connected origin across it, while a back-edge carries no origin of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WitnessSearchOutcome {
    FoundOrigin,
    SkippedBackEdge,
    TerminalLeaf,
}

/// Deterministic candidate order for one function body's active contributors.
///
/// WHAT: repeatedly applies the shared witness policy to the contributors not yet ordered,
///       so each matching write-back stays ahead of its own arithmetic wherever the
///       remaining sequence starts.
/// WHY: after skipping a recursive back-edge the search resumes mid-sequence; reapplying
///      the selection (instead of appending the rest in raw order) keeps same-compound
///      write-back precedence after any skipped prefix.
pub(crate) fn order_failure_witness_candidates<'candidates, T>(
    active: Vec<&'candidates T>,
    site: impl Copy + Fn(&T) -> FailureWitnessSite,
) -> Vec<&'candidates T> {
    let mut remaining: Vec<&'candidates T> = active;
    let mut ordered = Vec::with_capacity(remaining.len());
    while !remaining.is_empty() {
        let winner = select_failure_witness::<_, std::convert::Infallible>(
            remaining.iter().copied().map(Ok),
            site,
        )
        .unwrap_or_else(|never| match never {});
        let Some(winner) = winner else {
            break;
        };
        let position = remaining
            .iter()
            .position(|candidate| std::ptr::eq(*candidate, winner))
            .expect("the policy winner comes from the remaining contributors");
        ordered.push(remaining.remove(position));
    }
    ordered
}

/// Keep at most three private call hops on a failure witness, counting the rest as elided.
///
/// WHAT: stores the first three path hops as labels and records how many further hops the
///       bounded rendering omits.
/// WHY: both witness walkers share one visible-hop bound, so backtracking search renders the
///      final path in exactly one place instead of adjusting labels while exploring.
pub(crate) fn retain_bounded_witness_hops(
    witness: &mut BuiltinFailureWitness,
    hops: Vec<Option<SourceSpan>>,
) {
    const MAX_CALL_HOPS: usize = 3;

    witness.elided_call_hops = hops.len().saturating_sub(MAX_CALL_HOPS) as u32;
    witness.call_spans = hops.into_iter().take(MAX_CALL_HOPS).collect();
}

#[derive(Clone, Copy)]
pub(crate) enum FailureWitnessSite {
    Arithmetic(Option<SourceSpan>),
    WriteBack(Option<SourceSpan>),
    Call,
}

impl ImplicitFailureContributor {
    pub(crate) fn witness_site(&self) -> FailureWitnessSite {
        match self.source {
            ImplicitFailureSource::NumericOperation => FailureWitnessSite::Arithmetic(self.span),
            ImplicitFailureSource::CompoundWriteBack {
                arithmetic_span, ..
            } => FailureWitnessSite::WriteBack(arithmetic_span),
            ImplicitFailureSource::PrivateCall(_) => FailureWitnessSite::Call,
        }
    }
}

impl FailureSummary {
    pub(crate) fn merge_from(&mut self, other: &Self) {
        self.first_implicit = self.first_implicit.or(other.first_implicit);
        self.first_numeric = self.first_numeric.or(other.first_numeric);
        self.first_private_call = self.first_private_call.or(other.first_private_call);
        if let Some(first) = self.first_typed {
            if self.conflicting_typed.is_none() {
                self.conflicting_typed = match other.first_typed {
                    Some(producer) if producer.error_type_id != first.error_type_id => {
                        Some(producer)
                    }
                    _ => other.conflicting_typed,
                };
            }
        } else {
            self.first_typed = other.first_typed;
            self.conflicting_typed = other.conflicting_typed;
        }
    }

    fn record_implicit(&mut self, contributor: ImplicitFailureContributor) {
        let witness = Some(ImplicitFailureContributorSummary {
            span: contributor.span,
        });
        self.first_implicit = self.first_implicit.or(witness);
        match contributor.source {
            ImplicitFailureSource::NumericOperation
            | ImplicitFailureSource::CompoundWriteBack { .. } => {
                self.first_numeric = self.first_numeric.or(witness);
            }
            ImplicitFailureSource::PrivateCall(_) => {
                self.first_private_call = self.first_private_call.or(witness);
            }
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FailureDisposition {
    #[default]
    Pending,
    HandledByCatch {
        error_type_id: TypeId,
    },
}

impl ExpressionFailureFacts {
    pub(crate) fn has_pending_implicit(&self) -> bool {
        self.disposition == FailureDisposition::Pending && self.summary.first_implicit.is_some()
    }

    /// Constant-cost metadata join. Origin witnesses and deferred checks never leave their owner.
    pub(crate) fn merge_pending_from(&mut self, other: &Self) {
        self.postfix_exit_span = self.postfix_exit_span.or(other.postfix_exit_span);
        if other.disposition != FailureDisposition::Pending {
            return;
        }
        self.checked_numeric_operation |= other.checked_numeric_operation;
        self.summary.merge_from(&other.summary);
    }

    pub(crate) fn record_implicit(&mut self, contributor: ImplicitFailureContributor) {
        self.summary.record_implicit(contributor);
        self.implicit.push(contributor);
    }

    pub(crate) fn record_typed_error(&mut self, producer: TypedErrorProducer) {
        self.summary.merge_from(&FailureSummary {
            first_typed: Some(producer),
            ..FailureSummary::default()
        });
        self.typed_error = Some(producer);
    }

    /// Folding transfers surviving operator witnesses to the reduced expression by move.
    pub(crate) fn take_origin_work_from(&mut self, mut other: Self) {
        self.checked_numeric_operation |= other.checked_numeric_operation;
        for contributor in other.implicit.drain(..) {
            self.record_implicit(contributor);
        }
    }

    /// The caller retains eligibility while discarding folded operator witnesses.
    pub(crate) fn refresh_origin_summary(&mut self) {
        self.summary = FailureSummary::default();
        for contributor in &self.implicit {
            self.summary.record_implicit(*contributor);
        }
        if let Some(producer) = self.typed_error {
            self.summary.first_typed = Some(producer);
        }
    }

    pub(crate) fn is_folded_numeric_catch_success(&self) -> bool {
        self.checked_numeric_operation
            && self.summary.first_implicit.is_none()
            && self.summary.first_typed.is_none()
    }

    pub(crate) fn record_numeric_operation(
        &mut self,
        operator: NumericOperator,
        left: NumericScalar,
        right: Option<NumericScalar>,
        domain: NumericScalar,
        span: Option<SourceSpan>,
    ) {
        // Semantically proven-safe operations stay catch-eligible (an eligible handler does not
        // become invalid because proof removes its runtime path) but record no contributor, so
        // no failure edge or boundary diagnostic follows.
        self.checked_numeric_operation = true;
        if numeric_operation_cannot_fail(operator, left, right, domain) {
            return;
        }
        let codes = numeric_failure_codes(operator, domain);
        if codes.is_empty() {
            return;
        }
        self.record_implicit(ImplicitFailureContributor {
            span,
            codes,
            source: ImplicitFailureSource::NumericOperation,
        });
    }

    /// Aggregate actual evaluated children, not references to their declaration initialisers.
    pub(crate) fn from_expression_kind(kind: &ExpressionKind, span: Option<SourceSpan>) -> Self {
        let mut facts = Self::default();
        match kind {
            ExpressionKind::Runtime(rpn) => {
                for item in &rpn.items {
                    if let ExpressionRpnItem::Operand(value) = item {
                        facts.merge_pending_from(&value.failure_facts);
                    }
                }
            }
            ExpressionKind::FunctionCall { args, .. }
            | ExpressionKind::HostFunctionCall { args, .. }
            | ExpressionKind::HandledFallibleFunctionCall { args, .. }
            | ExpressionKind::HandledFallibleHostFunctionCall { args, .. } => {
                for argument in args {
                    facts.merge_pending_from(&argument.value.failure_facts);
                }
            }
            ExpressionKind::MethodCall { receiver, args, .. }
            | ExpressionKind::CollectionBuiltinCall { receiver, args, .. }
            | ExpressionKind::MapBuiltinCall { receiver, args, .. } => {
                facts.merge_pending_from(&receiver.failure_facts);
                for argument in args {
                    facts.merge_pending_from(&argument.value.failure_facts);
                }
            }
            ExpressionKind::FieldAccess { base, .. }
            | ExpressionKind::Coerced { value: base, .. }
            | ExpressionKind::OptionPropagation { value: base } => {
                facts.merge_pending_from(&base.failure_facts);
            }
            ExpressionKind::HandledFallibleExpression { value, .. } => {
                facts.merge_pending_from(&value.failure_facts);
            }
            ExpressionKind::Cast(cast) => {
                // Retain the compound operator identity, so both witness builders prefer this
                // write-back only to arithmetic from the same update, not earlier statements.
                if matches!(cast.handling, CastHandling::StoreConversion)
                    && let ResolvedCastEvidence::Builtin {
                        policy: BuiltinCastPolicyId::NumericConversion { target, .. },
                    } = &cast.evidence
                {
                    // Compound stores check the numeric destination domain after the compound
                    // arithmetic. The contributor keeps the canonical target so the witness
                    // names the write-back instead of the already-handled RHS work. Explicit
                    // casts keep their distinct typed Error producer rather than this lane.
                    let codes = if target.is_binary_float() {
                        &[BuiltinErrorCode::FloatNonFinite][..]
                    } else {
                        &[BuiltinErrorCode::IntOverflow][..]
                    };
                    facts.record_implicit(ImplicitFailureContributor {
                        span,
                        codes,
                        source: ImplicitFailureSource::CompoundWriteBack {
                            target: cast.target_type_id,
                            arithmetic_span: cast
                                .source
                                .failure_facts
                                .implicit
                                .iter()
                                .rev()
                                .find(|contributor| {
                                    matches!(
                                        contributor.source,
                                        ImplicitFailureSource::NumericOperation
                                    )
                                })
                                .and_then(|contributor| contributor.span),
                        },
                    });
                }
                facts.merge_pending_from(&cast.source.failure_facts);
            }
            ExpressionKind::Collection(values) => {
                for value in values {
                    facts.merge_pending_from(&value.failure_facts);
                }
            }
            ExpressionKind::MapLiteral(entries) => {
                for entry in entries {
                    facts.merge_pending_from(&entry.key.failure_facts);
                    facts.merge_pending_from(&entry.value.failure_facts);
                }
            }
            ExpressionKind::StructInstance(fields)
            | ExpressionKind::AnonymousConstRecord { fields }
            | ExpressionKind::ChoiceConstruct { fields, .. } => {
                for field in fields {
                    facts.merge_pending_from(&field.value.failure_facts);
                }
            }
            ExpressionKind::Range(start, end) => {
                facts.merge_pending_from(&start.failure_facts);
                facts.merge_pending_from(&end.failure_facts);
            }
            #[cfg(test)]
            ExpressionKind::FallibleCarrierConstruct { value, .. } => {
                facts.merge_pending_from(&value.failure_facts);
            }
            // Template and value-block payloads need their owning store-aware completion walker.
            _ => {}
        }
        let own_exit_span = match kind {
            ExpressionKind::HandledFallibleFunctionCall {
                handling: FallibleExpressionHandling::Propagate,
                propagation_span,
                ..
            }
            | ExpressionKind::HandledFallibleHostFunctionCall {
                handling: FallibleExpressionHandling::Propagate,
                propagation_span,
                ..
            }
            | ExpressionKind::HandledFallibleExpression {
                handling: FallibleExpressionHandling::Propagate,
                propagation_span,
                ..
            } => propagation_span.or(span),
            ExpressionKind::OptionPropagation { .. } => span,
            ExpressionKind::Cast(cast) if matches!(cast.handling, CastHandling::Propagate) => {
                cast.span.or(span)
            }
            _ => None,
        };
        facts.postfix_exit_span = facts.postfix_exit_span.or(own_exit_span);
        facts
    }
}

#[cfg(test)]
#[path = "tests/failure_facts_tests.rs"]
mod tests;
