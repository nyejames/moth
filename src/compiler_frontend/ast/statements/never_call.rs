//! Resolved standalone `-> !` call statements.
//!
//! WHAT: owns the executable AST payload for one call whose published contract is `Never`, and
//! the placement rule that accepts it only as a standalone terminating statement.
//! WHY: a Never call has no result value or result type. Keeping it out of the expression store
//! makes every value consumer reject it through one choke point instead of fabricating an empty
//! or unit result, while statement sequencing still sees a real terminating statement.
//!
//! The payload keeps the call target and routed argument IDs exactly as call construction
//! validated them. Receiver methods retain the receiver separately from declared arguments.
//! HIR lowers this payload into the terminating call statement.

use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::expressions::parse_expression::validate_completed_typed_errors;
use crate::compiler_frontend::compiler_messages::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::InvalidCallShapeReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data;
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;

/// The resolved callee identity of one Never call.
///
/// WHAT: distinguishes a source-resolved path from a binding-backed external function.
/// WHY: the AST keeps the stable target it resolved at the call site; HIR lowers it into the
///      matching `CallTarget` without re-resolving names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum NeverCallTarget {
    /// A source free function, method or generated instance path.
    Source(PathId),
    /// A binding-backed external function.
    External(ExternalFunctionId),
}

/// One resolved call that never returns to its caller.
#[derive(Clone, Debug)]
pub(crate) struct NeverCall {
    /// Stable function or receiver-method path the call dispatches to, or the external binding.
    pub(crate) target: NeverCallTarget,
    /// The resolved receiver of a receiver-method call, absent for a free call.
    ///
    /// WHAT: keeps the receiver separate from the routed declared-parameter arguments.
    /// WHY: the hidden receiver is not an authored argument slot, and HIR owns the ordinary
    ///      receiver-to-slot-zero projection when it lowers the call.
    pub(crate) receiver: Option<ExprId>,
    /// Declared-parameter arguments in declaration order.
    pub(crate) arguments: Vec<CallArgument<ExprId>>,
    /// Visible callee name for placement diagnostics.
    pub(crate) name: StringId,
    /// Authored call span.
    pub(crate) span: Option<SourceSpan>,
}

impl NeverCall {
    /// Builds the shared "Never call used as a value" diagnostic.
    ///
    /// WHAT: one owner for every value-position rejection, whether the call was built at its
    ///       authored site or lifted from a retained numeric-dependent call.
    /// WHY: statement position is the only legal placement; every other consumer must report the
    ///      same targeted message instead of inventing an empty or unit result.
    pub(crate) fn used_as_value_diagnostic(&self) -> DiagnosticDraft {
        Self::used_as_value_diagnostic_for(self.name, self.span)
    }

    /// Builds the same value-position diagnostic from authored call facts alone.
    ///
    /// WHAT: one constructor shared by a completed call and by a retained call that is rejected
    ///       before its arguments complete.
    /// WHY: every value-position rejection must report the identical reason and wording, whether
    ///      the payload exists or only its callee name and span are known.
    pub(crate) fn used_as_value_diagnostic_for(
        name: StringId,
        span: Option<SourceSpan>,
    ) -> DiagnosticDraft {
        calls_data::invalid_call_shape(
            InvalidCallShapeReason::NeverCallUsedAsValue,
            Some(name),
            span,
        )
    }

    /// Validates the receiver and declared arguments at their receiving boundary.
    ///
    /// WHAT: reports the first unhandled typed producer in the receiver or any argument.
    /// WHY: a Never call builds no value expression, so the ordinary whole-call boundary check
    ///      never sees its operands. Statement parsing defers that check while operands are
    ///      pending, so every owner that lifts a `NeverCall` must close it explicitly; otherwise a
    ///      fallible argument either reaches HIR as an unhandled propagation or is silently
    ///      accepted inside an error-returning caller.
    pub(crate) fn validate_typed_errors(
        &self,
        expression_builder: &AstExpressionBuilder,
    ) -> Result<(), ExpressionParseError> {
        if let Some(receiver) = self.receiver {
            validate_completed_typed_errors(expression_builder.view(receiver)?)?;
        }
        for argument in &self.arguments {
            validate_completed_typed_errors(expression_builder.view(argument.value)?)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/never_call_tests.rs"]
mod never_call_tests;
