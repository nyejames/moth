//! Expression-owned failure contributors, independent of the expression's success type.
//!
//! AST typing records checked work before folding. Folding discharges runtime contributors but
//! preserves catch eligibility. Catch disposition applies to protected work, never its handler.

use super::expression::ExpressionKind;
use super::expression_rpn::ExpressionRpnItem;
use super::expression_types::{
    CastHandling, FallibleExpressionHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExpressionFailureFacts {
    pub(crate) implicit: Vec<ImplicitFailureContributor>,
    pub(crate) typed_errors: Vec<TypedErrorProducer>,
    /// Custom catch compatibility waits for exact private-callee failure summaries.
    pub(crate) deferred_custom_catches: Vec<DeferredCustomCatchCheck>,
    /// A checked operation remains catch-eligible even when constant folding discharges it.
    pub(crate) checked_numeric_operation: bool,
    /// Explicit `!`, `cast!` and `?` leave the function, never target an enclosing catch.
    pub(crate) postfix_exit_span: Option<SourceSpan>,
    pub(crate) disposition: FailureDisposition,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImplicitFailureContributor {
    pub(crate) span: Option<SourceSpan>,
    pub(crate) codes: Vec<BuiltinErrorCode>,
    pub(crate) source: ImplicitFailureSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImplicitFailureSource {
    NumericOperation,
    PrivateCall(PathId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TypedErrorProducer {
    pub(crate) span: Option<SourceSpan>,
    pub(crate) error_type_id: TypeId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeferredCustomCatchCheck {
    pub(crate) catch_span: Option<SourceSpan>,
    pub(crate) error_type_id: TypeId,
    pub(crate) typed_producer_span: Option<SourceSpan>,
    pub(crate) candidates: Vec<ImplicitFailureContributor>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FailureDisposition {
    #[default]
    Pending,
    HandledByCatch { error_type_id: TypeId },
}

impl ExpressionFailureFacts {
    pub(crate) fn has_pending_implicit(&self) -> bool {
        self.disposition == FailureDisposition::Pending && !self.implicit.is_empty()
    }

    pub(crate) fn merge_pending_from(&mut self, other: &Self) {
        self.merge_deferred_catches_from(other);
        self.postfix_exit_span = self.postfix_exit_span.or(other.postfix_exit_span);
        if other.disposition != FailureDisposition::Pending {
            return;
        }

        self.checked_numeric_operation |= other.checked_numeric_operation;
        for contributor in &other.implicit {
            if !self.implicit.contains(contributor) {
                self.implicit.push(contributor.clone());
            }
        }
        for producer in &other.typed_errors {
            if !self.typed_errors.contains(producer) {
                self.typed_errors.push(producer.clone());
            }
        }
    }

    pub(crate) fn merge_deferred_catches_from(&mut self, other: &Self) {
        for check in &other.deferred_custom_catches {
            if !self.deferred_custom_catches.contains(check) {
                self.deferred_custom_catches.push(check.clone());
            }
        }
    }

    pub(crate) fn is_folded_numeric_catch_success(&self) -> bool {
        self.checked_numeric_operation && self.implicit.is_empty() && self.typed_errors.is_empty()
    }

    pub(crate) fn record_numeric_operation(
        &mut self,
        operator: NumericOperator,
        domain: NumericScalar,
        span: Option<SourceSpan>,
    ) {
        let codes = numeric_failure_codes(operator, domain);
        if codes.is_empty() {
            return;
        }

        self.checked_numeric_operation = true;
        self.implicit.push(ImplicitFailureContributor {
            span,
            codes,
            source: ImplicitFailureSource::NumericOperation,
        });
    }

    /// Aggregate actual evaluated children, not references to their declaration initialisers.
    pub(crate) fn from_expression_kind(
        kind: &ExpressionKind,
        span: Option<SourceSpan>,
    ) -> Self {
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
                facts.merge_pending_from(&cast.source.failure_facts);
                if matches!(cast.handling, CastHandling::StoreConversion)
                    && let ResolvedCastEvidence::Builtin {
                        policy: BuiltinCastPolicyId::NumericConversion { target, .. },
                    } = &cast.evidence
                {
                    // Compound stores check the numeric destination domain. Explicit casts keep
                    // their distinct typed Error producer rather than entering this lane.
                    let codes = if target.is_binary_float() {
                        vec![BuiltinErrorCode::FloatNonFinite]
                    } else {
                        vec![BuiltinErrorCode::IntOverflow]
                    };
                    facts.implicit.push(ImplicitFailureContributor {
                        span: cast.span,
                        codes,
                        source: ImplicitFailureSource::NumericOperation,
                    });
                }
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

fn numeric_failure_codes(operator: NumericOperator, domain: NumericScalar) -> Vec<BuiltinErrorCode> {
    let mut codes = Vec::new();
    if matches!(
        operator,
        NumericOperator::Divide | NumericOperator::IntegerDivide | NumericOperator::Remainder
    ) {
        codes.push(BuiltinErrorCode::DivideByZero);
    }
    if domain.is_integer()
        && !matches!(operator, NumericOperator::Divide | NumericOperator::Remainder)
    {
        codes.push(BuiltinErrorCode::IntOverflow);
    }
    if operator == NumericOperator::Power && !domain.is_binary_float() {
        codes.push(BuiltinErrorCode::InvalidExponent);
    }
    if domain.is_binary_float() && operator != NumericOperator::Negate {
        codes.push(BuiltinErrorCode::FloatNonFinite);
    }
    debug_assert!(codes.iter().all(|code| code.is_implicit_failure()));
    codes
}

#[cfg(test)]
#[path = "tests/failure_facts_tests.rs"]
mod tests;
