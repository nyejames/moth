//! Immutable function failure facts projected once from typed AST bodies.
//!
//! Summary convergence reads direct unhandled producers and resolved call targets here, without
//! retaining expression trees or changing the existing runtime HIR failure delivery.

use crate::compiler_frontend::ast::expressions::failure_facts::FailureWitnessSite;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::source::SourceSpan;

#[derive(Debug, Clone)]
pub(crate) struct HirFunctionFailureFacts {
    pub(crate) span: Option<SourceSpan>,
    pub(crate) boundary: HirBuiltinFailureBoundary,
    pub(crate) contributors: Vec<HirBuiltinFailureContributor>,
    /// Deferred assertion-message checks are not escaping function producers.
    pub(crate) assertion_message_calls: Vec<HirBuiltinFailureContributor>,
    /// Catch eligibility and compatibility wait for private-call summaries to converge.
    pub(crate) deferred_custom_catches: Vec<HirDeferredCustomCatchCheck>,
}

#[derive(Debug, Clone)]
pub(crate) struct HirDeferredCustomCatchCheck {
    pub(crate) catch_span: Option<SourceSpan>,
    pub(crate) error_type_id: TypeId,
    pub(crate) typed_producer_span: Option<SourceSpan>,
    pub(crate) candidates: Vec<HirBuiltinFailureContributor>,
    pub(crate) eligibility_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HirBuiltinFailureBoundary {
    InferPrivate,
    BuiltinErrorSlot,
    CustomErrorSlot(TypeId),
    ExportedNoSlot,
}

#[derive(Debug, Clone)]
pub(crate) struct HirBuiltinFailureContributor {
    pub(crate) source: HirBuiltinFailureSource,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) codes: Vec<BuiltinErrorCode>,
}

#[allow(
    clippy::large_enum_variant,
    reason = "contributors are matched by reference during convergence; boxing would allocate per call contributor"
)]
#[derive(Debug, Clone)]
pub(crate) enum HirBuiltinFailureSource {
    NumericOperation,
    AuthoredCastConversion,
    /// Checked compound-assignment write-back with its canonical target type.
    ///
    /// WHAT: mirrors the AST write-back contributor through projection so the
    ///       witness names the write-back target instead of earlier handled work.
    /// WHY: convergence matches contributors by reference; the target travels
    ///      in the source rather than a parallel lookup.
    CompoundWriteBack {
        target: TypeId,
        arithmetic_span: Option<SourceSpan>,
    },
    Call(CallTarget),
}

impl HirBuiltinFailureContributor {
    pub(crate) fn witness_site(&self) -> FailureWitnessSite {
        match self.source {
            HirBuiltinFailureSource::NumericOperation
            | HirBuiltinFailureSource::AuthoredCastConversion => {
                FailureWitnessSite::Arithmetic(self.span)
            }
            HirBuiltinFailureSource::CompoundWriteBack {
                arithmetic_span, ..
            } => FailureWitnessSite::WriteBack(arithmetic_span),
            HirBuiltinFailureSource::Call(_) => FailureWitnessSite::Call,
        }
    }
}
