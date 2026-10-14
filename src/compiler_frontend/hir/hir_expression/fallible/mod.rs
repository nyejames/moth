//! Native success/error continuation lowering for propagation and catch recovery.

use super::LoweredResults;
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionStore;
use crate::compiler_frontend::ast::expressions::expression_types::{
    FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

mod catch;
mod continuation;
mod external;
mod propagation;
pub(crate) use continuation::EmittedFallibleResults;

pub(crate) struct FallibleBranchingContext<'a> {
    pub(crate) expressions: &'a ExpressionStore,
    pub(crate) result_type_ids: &'a [TypeId],
    pub(crate) handling: &'a FallibleHandling,
    pub(crate) err_type: TypeId,
    pub(crate) span: &'a Option<SourceSpan>,
}

impl<'a> HirBuilder<'a> {
    pub(super) fn finish_handled_fallible_results(
        &mut self,
        results: EmittedFallibleResults,
        handling: &FallibleExpressionHandling,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let values = if self.active_catch_handler.is_some() {
            self.lower_results_to_active_catch_success_values(results, span)?
        } else if matches!(handling, FallibleExpressionHandling::Propagate) {
            self.lower_fallible_results_to_success_values(results, span)?
        } else {
            return_hir_transformation_error!(
                "Recovering producer outside a catch",
                self.hir_error_location(span)
            );
        };
        Ok(LoweredResults {
            prelude: vec![],
            values,
        })
    }
}
