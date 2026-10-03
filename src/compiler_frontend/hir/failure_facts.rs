//! Immutable function failure facts projected once from typed AST bodies.
//!
//! Summary convergence reads direct unhandled producers and resolved call targets here, without
//! retaining expression trees or changing the existing runtime HIR failure delivery.

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

#[derive(Debug, Clone)]
pub(crate) enum HirBuiltinFailureSource {
    NumericOperation,
    Call(CallTarget),
}
