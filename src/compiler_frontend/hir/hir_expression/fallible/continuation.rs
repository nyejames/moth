//! Native fallible producer continuations and error propagation.
//!
//! Invoke defines separate edge-local success slots and an error local. This owner
//! connects those edges to propagation or recovery without manufacturing value carriers.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirVariantCarrier, HirVariantField, ValueKind,
};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::{BlockId, HirValueId, LocalId};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::type_coercion::compatibility::is_postfix_error_compatible;
use crate::return_hir_transformation_error;

pub(crate) struct EmittedFallibleResults {
    pub(crate) success_block: BlockId,
    pub(crate) error_block: BlockId,
    pub(crate) success_locals: Vec<LocalId>,
    pub(crate) success_types: Vec<TypeId>,
    pub(crate) error_local: LocalId,
    pub(crate) err_type: TypeId,
    pub(crate) validate_float_success: bool,
}

impl<'a> HirBuilder<'a> {
    pub(crate) fn type_id_is_exact_binary_float(&self, type_id: TypeId) -> bool {
        NumericScalar::from_type_id(type_id, &self.type_environment)
            .is_some_and(NumericScalar::is_binary_float)
    }

    pub(crate) fn emit_fallible_operation(
        &mut self,
        operation: HirFallibleOperation,
        success_types: &[TypeId],
        err_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<EmittedFallibleResults, HirConstructionFailure> {
        if let HirFallibleOperation::Call { target, .. } = &operation
            && self.call_target_requires_result_projection(target)
        {
            self.module.call_result_contracts.insert(
                target.clone(),
                crate::compiler_frontend::hir::module::HirCallResultContract {
                    returns: success_types.into(),
                    error_return: Some(err_type),
                },
            );
        }
        let block = self.current_block_id_or_error(span)?;
        let region = self.current_region_or_error(span)?;
        let success_block = self.create_block(region, span, "invoke-success")?;
        let error_block = self.create_block(region, span, "invoke-error")?;
        // Invoke destinations are edge-defined locals, owned by their selected edge blocks.
        self.set_current_block(success_block, span)?;
        let mut success_locals = Vec::with_capacity(success_types.len());
        for ty in success_types {
            success_locals.push(self.allocate_temp_local(*ty, None)?);
        }
        self.set_current_block(error_block, span)?;
        let error_local = self.allocate_temp_local(err_type, None)?;
        self.set_current_block(block, span)?;
        let id = self.allocate_node_id();
        self.emit_terminator_with_span(
            block,
            HirTerminator::Invoke {
                id,
                span: *span,
                operation,
                success: success_block,
                success_results: success_locals.clone(),
                failure: error_block,
                error_result: error_local,
            },
            span,
            *span,
        )?;
        Ok(EmittedFallibleResults {
            success_block,
            error_block,
            success_locals,
            success_types: success_types.to_vec(),
            error_local,
            err_type,
            validate_float_success: false,
        })
    }

    pub(crate) fn fallible_success_values(
        &mut self,
        results: &EmittedFallibleResults,
        span: &Option<SourceSpan>,
    ) -> Result<Vec<HirValueId>, HirConstructionFailure> {
        self.set_current_block(results.success_block, span)?;
        let region = self.current_region_or_error(span)?;
        let mut values = Vec::with_capacity(results.success_locals.len());
        for (local, ty) in results.success_locals.iter().zip(&results.success_types) {
            let mut value = self.make_local_load_expression(*local, *ty, &None, region)?;
            if results.validate_float_success && self.type_id_is_exact_binary_float(*ty) {
                value = self.emit_validated_float_value(value, span)?;
            }
            values.push(value);
        }
        Ok(values)
    }

    pub(crate) fn lower_fallible_results_to_success_values(
        &mut self,
        results: EmittedFallibleResults,
        span: &Option<SourceSpan>,
    ) -> Result<Vec<HirValueId>, HirConstructionFailure> {
        let function_id = self.current_function_id_or_error(span)?;
        let Some(error_type) = self
            .function_by_id_or_error(function_id, span)?
            .error_return
        else {
            return_hir_transformation_error!(
                "Fallible propagation outside an error-returning function",
                self.hir_error_location(span)
            );
        };
        if !is_postfix_error_compatible(error_type, results.err_type, &self.type_environment) {
            return_hir_transformation_error!(
                "Fallible propagation error type mismatch",
                self.hir_error_location(span)
            );
        }
        self.emit_fallible_error_return(&results, error_type, span, *span)?;
        self.fallible_success_values(&results, span)
    }

    pub(crate) fn lower_fallible_results_to_success_value(
        &mut self,
        results: EmittedFallibleResults,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let values = self.lower_fallible_results_to_success_values(results, span)?;
        self.single_result_value(values, span)
    }

    pub(crate) fn single_result_value(
        &mut self,
        values: Vec<HirValueId>,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        match values.as_slice() {
            [value] => Ok(*value),
            _ => return_hir_transformation_error!(
                "Scalar receiving site requires exactly one success slot",
                self.hir_error_location(span)
            ),
        }
    }

    pub(super) fn emit_fallible_error_return(
        &mut self,
        results: &EmittedFallibleResults,
        expected_error_type: TypeId,
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), HirConstructionFailure> {
        self.set_current_block(results.error_block, span)?;
        let region = self.current_region_or_error(span)?;
        let error =
            self.make_local_load_expression(results.error_local, results.err_type, &None, region)?;
        let error = self.coerce_postfix_error_payload(error, expected_error_type, span)?;
        self.emit_terminator_with_span(
            results.error_block,
            HirTerminator::ReturnError(error),
            span,
            authored_span,
        )
    }

    pub(super) fn coerce_postfix_error_payload(
        &mut self,
        error_payload: HirValueId,
        expected_error_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let row = self.module.expressions.expression(error_payload);
        let (payload_type, payload_region) = (row.ty, row.region);
        if payload_type == expected_error_type {
            return Ok(error_payload);
        }
        if self.type_environment.option_inner_type(expected_error_type) == Some(payload_type) {
            let name = self.string_table.intern("value");
            let fields = self.module.expressions.append_variant_fields(
                &[HirVariantField {
                    name: Some(name),
                    value: error_payload,
                }],
                *span,
            )?;
            return self.make_expression(
                &None,
                HirExpressionKind::VariantConstruct {
                    carrier: HirVariantCarrier::Option,
                    variant_index: 1,
                    fields,
                },
                expected_error_type,
                ValueKind::RValue,
                payload_region,
            );
        }
        return_hir_transformation_error!(
            "Postfix propagation has incompatible error payload",
            self.hir_error_location(span)
        );
    }

    pub(crate) fn result_call_slots(
        &self,
        target: &CallTarget,
        span: &Option<SourceSpan>,
    ) -> Result<(Vec<TypeId>, TypeId), HirConstructionFailure> {
        let slots = match target {
            CallTarget::Local(id) => {
                let function = self.function_by_id_or_error(*id, span)?;
                return function
                    .error_return
                    .map(|error| (function.returns.to_vec(), error))
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Fallible call targets an infallible signature",
                        )
                        .into()
                    });
            }
            CallTarget::CrossModule(origin) => self.imported_fallible_results_by_origin.get(origin),
            CallTarget::ModulePrivate(identity) => self
                .module_private_fallible_results_by_identity
                .get(identity),
            CallTarget::Generated(identity) => self
                .generated_fallible_results_by_identity
                .get(identity.as_ref()),
            CallTarget::External(_) => None,
        };
        slots
            .map(|(returns, error)| (returns.to_vec(), *error))
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Fallible call has no projected native result signature",
                )
                .into()
            })
    }
}
