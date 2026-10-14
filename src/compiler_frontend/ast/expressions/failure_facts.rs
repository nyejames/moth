//! Origin-owned failure witnesses and compact expression compatibility summaries.
//!
//! Ancestors retain only the first diagnostic witnesses and eligibility metadata. The semantic
//! classifier collects origin witnesses once for function projection; folding discharges only
//! the checked work it owns. Catch disposition protects work, never its handler.

use super::expression_store::{ExpressionOperationView, ExpressionView};
use super::expression_types::{CastHandling, FallibleExpressionHandling, ResolvedCastEvidence};
use crate::compiler_frontend::builtins::casts::policies::builtin_cast_failure_codes;
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId,
};
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::compiler_messages::BuiltinFailureWitness;
use crate::compiler_frontend::datatypes::ids::TypeId;
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
    /// An authored arithmetic operation remains eligible after folding proves it safe.
    pub(crate) checked_numeric_operation: bool,
    /// An authored cast remains eligible even when it is infallible or folded away.
    pub(crate) authored_cast: bool,
    /// Saturated count of calls at this expression's selected evaluation level.
    pub(crate) safety_call_count: u8,
    /// Explicit `!` and `?` leave the function, never target an enclosing catch.
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
    /// A source-authored fallible cast; it delivers builtin `Error` through the ordinary path.
    AuthoredCastConversion,
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
///
/// HIR selects eagerly so every contributor's summary and code set is validated. The AST and
/// both witness walkers apply the same policy lazily through `WitnessCandidates`.
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

/// Lazily yields one body's active contributors in witness-policy order.
///
/// WHAT: yields active calls in evaluation order, then at most one terminal numeric origin, after
///       which the cursor is exhausted. Arithmetic yields to a later active write-back of its own
///       compound update, matching `select_failure_witness`.
/// WHY: every consumer stops at the first numeric origin and skips only calls, so nothing ever
///      resumes after a terminal origin. Selecting on demand examines each skipped call once
///      instead of ordering every contributor of the body before the first one is tried.
pub(crate) struct WitnessCandidates<'contributors, T> {
    contributors: &'contributors [T],
    next: usize,
}

impl<'contributors, T> WitnessCandidates<'contributors, T> {
    pub(crate) fn new(contributors: &'contributors [T]) -> Self {
        Self {
            contributors,
            next: 0,
        }
    }

    pub(crate) fn next_candidate<E>(
        &mut self,
        mut is_active: impl FnMut(&T) -> Result<bool, E>,
        site: impl Fn(&T) -> FailureWitnessSite,
    ) -> Result<Option<&'contributors T>, E> {
        while let Some(contributor) = self.contributors.get(self.next) {
            self.next += 1;
            if !is_active(contributor)? {
                continue;
            }

            let arithmetic_span = match site(contributor) {
                FailureWitnessSite::Call => return Ok(Some(contributor)),
                FailureWitnessSite::Arithmetic(span) => span,
                FailureWitnessSite::WriteBack(_) => None,
            };

            // A numeric origin is terminal: exhaust the cursor before yielding it.
            let remaining = &self.contributors[self.next..];
            self.next = self.contributors.len();
            let Some(arithmetic_span) = arithmetic_span else {
                return Ok(Some(contributor));
            };
            for candidate in remaining {
                if let FailureWitnessSite::WriteBack(Some(span)) = site(candidate)
                    && span == arithmetic_span
                    && is_active(candidate)?
                {
                    return Ok(Some(candidate));
                }
            }
            return Ok(Some(contributor));
        }
        Ok(None)
    }
}

/// Keep at most three private call hops on a failure witness, counting the rest as elided.
///
/// WHAT: stores the first three path hops as labels and records how many further hops the
///       bounded rendering omits.
/// WHY: both witness walkers share one visible-hop bound, so the search renders the final path
///      in exactly one place instead of adjusting labels while exploring.
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
            ImplicitFailureSource::NumericOperation
            | ImplicitFailureSource::AuthoredCastConversion => {
                FailureWitnessSite::Arithmetic(self.span)
            }
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

    pub(crate) fn record_implicit(&mut self, contributor: ImplicitFailureContributor) {
        let witness = Some(ImplicitFailureContributorSummary {
            span: contributor.span,
        });
        self.first_implicit = self.first_implicit.or(witness);
        match contributor.source {
            ImplicitFailureSource::NumericOperation
            | ImplicitFailureSource::AuthoredCastConversion
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
        self.authored_cast |= other.authored_cast;
        self.summary.merge_from(&other.summary);
    }

    /// Authored eligibility that a folded constant keeps from the operation it replaced.
    ///
    /// WHAT: copies only the pending `checked_numeric_operation`, `authored_cast` and saturated
    ///       safety call count; origins, witnesses and deferred checks stay with their producers.
    /// WHY: a folded root cannot fail at runtime, so it owns no origins, but a `catch` on the
    ///      folded value must still see the authored checked operation or cast it replaced.
    pub(crate) fn folded_constant_eligibility(&self) -> Self {
        if self.disposition != FailureDisposition::Pending {
            return Self::default();
        }
        Self {
            checked_numeric_operation: self.checked_numeric_operation,
            authored_cast: self.authored_cast,
            safety_call_count: self.safety_call_count,
            ..Self::default()
        }
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

    pub(crate) fn has_authored_safety_catch_eligibility(&self) -> bool {
        self.checked_numeric_operation
            || self.authored_cast
            || self.safety_call_count >= 2
            || self.summary.first_implicit.is_some()
            || self.summary.first_typed.is_some()
    }

    pub(crate) fn is_statically_infallible_catch(&self) -> bool {
        self.has_authored_safety_catch_eligibility()
            && self.summary.first_implicit.is_none()
            && self.summary.first_typed.is_none()
            && self.summary.first_private_call.is_none()
    }

    fn add_call_count(&mut self, count: u8) {
        self.safety_call_count = self.safety_call_count.saturating_add(count).min(2);
    }

    /// Summaries join evaluated children in authored order; witnesses stay at their origin row.
    pub(crate) fn from_operation(
        operation: ExpressionOperationView<'_>,
        span: Option<SourceSpan>,
    ) -> Self {
        use ExpressionOperationView as V;
        let mut facts = Self::default();
        match operation {
            V::NumericUnary {
                operand,
                facts: operation,
            } => {
                facts.merge_child(operand, true);
                facts.checked_numeric_operation = true;
                if let Some(contributor) = operation.failure {
                    facts.record_implicit(contributor);
                }
            }
            V::NumericBinary {
                left,
                right,
                facts: operation,
            }
            | V::Compare {
                left,
                right,
                facts: operation,
            }
            | V::Range {
                start: left,
                end: right,
                facts: operation,
            } => {
                facts.merge_child(left, true);
                facts.merge_child(right, true);
                facts.checked_numeric_operation |= operation.operator.numeric_operator().is_some();
                if let Some(contributor) = operation.failure {
                    facts.record_implicit(contributor);
                }
            }
            V::LogicalNot { operand } => facts.merge_child(operand, true),
            V::LogicalBinary { left, right, .. } => {
                facts.merge_child(left, true);
                facts.merge_child(right, true);
            }
            V::Call(call) => {
                facts.safety_call_count = 1;
                for argument in call.arguments().iter() {
                    facts.merge_child(argument.value(), false);
                }
                if let Some(handling) = call.handling()
                    && handling.handling == FallibleExpressionHandling::Propagate
                {
                    facts.postfix_exit_span = handling.propagation_span.or(span);
                }
            }
            V::MethodCall { receiver, call } => {
                facts.merge_child(receiver, true);
                facts.add_call_count(1);
                for argument in call.arguments().iter() {
                    facts.merge_child(argument.value(), false);
                }
                if let Some(handling) = call.handling()
                    && handling.handling == FallibleExpressionHandling::Propagate
                {
                    facts.postfix_exit_span = facts
                        .postfix_exit_span
                        .or(handling.propagation_span.or(span));
                }
            }
            V::CollectionBuiltin {
                receiver,
                arguments,
                ..
            }
            | V::MapBuiltin {
                receiver,
                arguments,
                ..
            } => {
                facts.merge_child(receiver, true);
                facts.add_call_count(1);
                for argument in arguments.iter() {
                    facts.merge_child(argument.value(), false);
                }
            }
            V::FieldAccess { base, .. } | V::Coerce { value: base, .. } => {
                facts.merge_child(base, true)
            }
            V::PropagateOption { value } => {
                facts.merge_child(value, true);
                facts.postfix_exit_span = facts.postfix_exit_span.or(span);
            }
            V::HandleFallible { value, handling } => {
                facts.merge_child(value, true);
                if handling.handling == FallibleExpressionHandling::Propagate {
                    // Only this producer's channel is consumed. Argument failures remain pending.
                    if let Some(producer) =
                        value.failure_facts().and_then(|child| child.typed_error)
                    {
                        if facts.summary.first_typed == Some(producer) {
                            facts.summary.first_typed = facts.summary.conflicting_typed.take();
                        } else if facts.summary.conflicting_typed == Some(producer) {
                            facts.summary.conflicting_typed = None;
                        }
                    }
                    facts.postfix_exit_span = facts
                        .postfix_exit_span
                        .or(handling.propagation_span.or(span));
                }
            }
            V::Cast {
                source,
                facts: cast,
            } => {
                let store_conversion = cast.handling == CastHandling::StoreConversion;
                facts.merge_child(source, true);
                facts.authored_cast |= !store_conversion;
                if cast.fallibility == BuiltinCastFallibility::Fallible {
                    let codes = match &cast.evidence {
                        ResolvedCastEvidence::Builtin { policy } => {
                            builtin_cast_failure_codes(*policy, cast.fallibility)
                        }
                        _ => &[],
                    };
                    if !store_conversion {
                        facts.record_implicit(ImplicitFailureContributor {
                            span,
                            codes,
                            source: ImplicitFailureSource::AuthoredCastConversion,
                        });
                    } else if matches!(
                        cast.evidence,
                        ResolvedCastEvidence::Builtin {
                            policy: BuiltinCastPolicyId::NumericConversion { .. }
                        }
                    ) && !codes.is_empty()
                    {
                        let arithmetic_span = match source.operation() {
                            V::NumericUnary { facts, .. } | V::NumericBinary { facts, .. } => {
                                facts.failure.and_then(|origin| origin.span)
                            }
                            _ => source
                                .failure_facts()
                                .and_then(|child| {
                                    child.implicit.iter().rev().find(|origin| {
                                        origin.source == ImplicitFailureSource::NumericOperation
                                    })
                                })
                                .and_then(|origin| origin.span),
                        };
                        facts.record_implicit(ImplicitFailureContributor {
                            span,
                            codes,
                            source: ImplicitFailureSource::CompoundWriteBack {
                                target: cast.target_type_id,
                                arithmetic_span,
                            },
                        });
                    }
                }
            }
            V::Collection(values) => {
                for value in values.iter() {
                    facts.merge_child(value, true);
                }
            }
            V::Map(entries) => {
                for entry in entries.iter() {
                    facts.merge_child(entry.key(), true);
                    facts.merge_child(entry.value(), true);
                }
            }
            V::Struct { fields } | V::Choice { fields, .. } => {
                for field in fields.iter() {
                    facts.merge_child(field.value(), false);
                }
            }
            // These payloads have store-aware completion walkers. Constant/reference rows do
            // not inherit the eligibility or witnesses of their declaration's initializer.
            V::Constant(_)
            | V::Reference(_)
            | V::Copy(_)
            | V::ValueBlock(_)
            | V::Template(_)
            | V::RuntimeTemplate(_)
            | V::RuntimeSlotApplication(_) => {}
        }
        facts
    }

    fn merge_child(&mut self, child: ExpressionView<'_>, count_calls: bool) {
        if let Some(other) = child.failure_facts() {
            self.merge_pending_from(other);
            if count_calls {
                self.add_call_count(other.safety_call_count);
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/failure_facts_tests.rs"]
mod tests;
