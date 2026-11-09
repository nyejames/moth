//! Standalone terminating call lowering.
//!
//! WHAT: lowers the AST `NeverCall` payload into one `HirTerminator::NeverCall`.
//! WHY: a declared `-> !` call has no value, result local or continuation block. It still
//! evaluates its receiver and arguments exactly once in ordinary left-to-right order, so the
//! terminating path reuses the ordinary call target and argument lowering rather than inventing
//! a second call form.
//!
//! ## Diagnostic boundary
//!
//! `InfrastructureFailure::hir_transformation` in this module means an internal lowering
//! invariant failure. The AST owns Never call placement and contract compatibility; HIR
//! validation rejects malformed terminators.

use crate::compiler_frontend::ast::expressions::expression_store::ExpressionStore;
use crate::compiler_frontend::ast::statements::never_call::{NeverCall, NeverCallTarget};
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::functions::HirFunctionReturnContract;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;

impl<'a> HirBuilder<'a> {
    /// Lower one standalone call whose published contract is `Never`.
    ///
    /// WHAT: resolves the target, evaluates the receiver and declared arguments once in order,
    ///       and terminates the current block with a no-successor `NeverCall`.
    /// WHY: later statements on the current path are dead. The sequence owner stops after this
    ///       terminator, and no result local or continuation block is created.
    pub(super) fn lower_never_call_statement(
        &mut self,
        expressions: &ExpressionStore,
        call: &NeverCall,
        statement_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        let call_span = call.span.or(statement_span);
        let target = match call.target {
            NeverCallTarget::Source(path) => self.resolve_call_target_or_error(&path)?,
            NeverCallTarget::External(function_id) => CallTarget::External(function_id),
        };

        // A nonlocal target's explicit contract lives beside the imported callable. Recording the
        // Never fact lets validation reject an ordinary call to the same target without
        // rescanning source or sidecar metadata.
        if !matches!(target, CallTarget::Local(_) | CallTarget::External(_))
            && self.call_target_requires_result_projection(&target)
        {
            self.module
                .call_result_contracts
                .insert(target.clone(), HirFunctionReturnContract::Never);
        }

        let mut lowered_args =
            Vec::with_capacity(call.arguments.len() + usize::from(call.receiver.is_some()));
        if let Some(receiver) = call.receiver {
            let mut prelude = Vec::new();
            let value =
                self.lower_child_expression_for_parent(&mut prelude, expressions.view(receiver)?)?;
            self.flush_pending_call_prelude(&mut prelude)?;
            lowered_args.push(value);
        }

        for (arg_index, argument) in call.arguments.iter().enumerate() {
            let value = self.lower_call_argument_value_parts(
                expressions.view(argument.value)?,
                argument.passing_mode,
                argument.span,
                &call_span,
                arg_index + usize::from(call.receiver.is_some()),
            )?;
            let mut prelude = value.prelude;
            self.flush_pending_call_prelude(&mut prelude)?;
            lowered_args.push(value.value);
        }

        let args = self
            .module
            .expressions
            .append_values(&lowered_args, call_span)?;
        let block = self.current_block_id_or_error()?;
        self.emit_terminator_with_span(
            block,
            HirTerminator::NeverCall { target, args },
            &call_span,
            call_span,
        )
    }
}
