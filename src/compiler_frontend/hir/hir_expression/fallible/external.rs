//! Native fallible external call emission, retaining boundary float-validation facts.

use super::EmittedFallibleResults;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ArgumentListView, ExpressionView,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalFunctionId};
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::terminators::HirFallibleOperation;
use crate::compiler_frontend::source::SourceSpan;

impl<'a> HirBuilder<'a> {
    #[allow(
        clippy::too_many_arguments,
        reason = "external result call keeps the mutable builder, callee, arguments, receiver, result types, float-validation flag, error type and span as separate inputs"
    )]
    pub(in crate::compiler_frontend::hir) fn emit_external_result_call_to_current_block(
        &mut self,
        id: ExternalFunctionId,
        args: ArgumentListView<'_>,
        receiver: Option<ExpressionView<'_>>,
        result_type_ids: &[TypeId],
        requires_external_float_validation: bool,
        error_type_id: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<EmittedFallibleResults, HirConstructionFailure> {
        let args = self.lower_invoke_arguments(args, receiver, span)?;
        let types = self.lower_call_result_types(result_type_ids, span)?;
        let error = self.lower_type_id(error_type_id, span)?;
        let mut results = self.emit_fallible_operation(
            HirFallibleOperation::Call {
                target: CallTarget::External(id),
                args,
            },
            &types,
            error,
            span,
        )?;
        results.validate_float_success = requires_external_float_validation;
        Ok(results)
    }
}
