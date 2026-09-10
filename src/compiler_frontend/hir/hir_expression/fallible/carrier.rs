//! Fallible carrier creation and branch helpers.
//!
//! WHAT: emits the temporary backend-boundary carrier, creates success/error CFG branches from it,
//! unwraps payloads, and wraps error payloads into option shapes when required.
//! WHY: all fallible lowering paths need the same carrier construction and branch emission logic.
//! Keeping these helpers together prevents drift between propagation, catch, and direct-return paths.

use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirVariantCarrier, HirVariantField, ValueKind,
};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::{BlockId, LocalId};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::type_coercion::compatibility::is_postfix_error_compatible;
use crate::return_hir_transformation_error;

/// Localized fallible carrier metadata after a fallible value has been emitted.
///
/// WHAT: stores the temporary local holding the backend-boundary carrier plus its semantic slots.
/// WHY: propagation, catch, and direct-return lowering all branch from the same carrier shape.
pub(crate) struct EmittedFallibleCarrier {
    pub(crate) result_local: LocalId,
    pub(crate) carrier_type: TypeId,
    pub(crate) ok_type: TypeId,
    pub(crate) err_type: TypeId,
    /// True when the success payload is a `Float` entering from an external/backend boundary
    /// and must be validated before ordinary Moth code observes it.
    pub(crate) validate_float_success: bool,
}

/// Success/error blocks created from one fallible carrier branch.
///
/// WHY: statement propagation and direct-return propagation both split the current block on the
/// same carrier shape before deciding what the success edge does.
pub(super) struct FallibleCarrierBranch {
    pub(super) success_block: BlockId,
    pub(super) error_block: BlockId,
}

impl<'a> HirBuilder<'a> {
    /// Returns true when the given type is the builtin `Float` type.
    ///
    /// WHAT: compares the type id against the registered builtin `Float` id.
    /// WHY: external/backend boundary Float success values need explicit validation before use.
    pub(crate) fn type_id_is_float(&self, type_id: TypeId) -> bool {
        type_id == self.type_environment.builtins().float
    }

    /// Emits a plain call to the current block and stores its result in a temporary local.
    pub(super) fn emit_result_call_to_current_block(
        &mut self,
        target: CallTarget,
        args: &[CallArgument],
        carrier_type: TypeId,
        call_span: &Option<SourceSpan>,
    ) -> Result<LocalId, CompilerError> {
        let mut lowered_args = Vec::with_capacity(args.len());

        for (arg_index, argument) in args.iter().enumerate() {
            let lowered = self.lower_call_argument_value(argument, call_span, arg_index)?;
            for prelude in lowered.prelude {
                self.emit_statement_to_current_block(prelude, call_span)?;
            }
            lowered_args.push(lowered.value);
        }

        let result_local = self.allocate_temp_local(carrier_type, None)?;
        let call_statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::Call {
                target,
                args: lowered_args,
                result: Some(result_local),
            },
            span: *call_span,
        };

        self.side_table.map_statement(*call_span, &call_statement);
        self.emit_statement_to_current_block(call_statement, call_span)?;

        Ok(result_local)
    }

    /// Lowers a fallible carrier into its success payload after emitting the error return edge.
    pub(crate) fn lower_fallible_carrier_to_success_value(
        &mut self,
        result_carrier: EmittedFallibleCarrier,
        span: &Option<SourceSpan>,
    ) -> Result<HirExpression, CompilerError> {
        let current_function_id = self.current_function_id_or_error(span)?;
        let current_return_type = self
            .function_by_id_or_error(current_function_id, span)?
            .return_type;
        let Some((_, current_error_type)) = self
            .type_environment
            .fallible_carrier_slots(current_return_type)
        else {
            return_hir_transformation_error!(
                "Value fallible propagation reached HIR outside a fallible function",
                self.hir_error_location(span)
            );
        };

        if !is_postfix_error_compatible(
            current_error_type,
            result_carrier.err_type,
            &self.type_environment,
        ) {
            return_hir_transformation_error!(
                "Value fallible propagation error type does not match the enclosing function",
                self.hir_error_location(span)
            );
        }

        let branch = self.emit_result_carrier_branch(
            result_carrier.result_local,
            result_carrier.carrier_type,
            span,
            *span,
            "propagate-value-ok",
            "propagate-value-err",
        )?;
        self.emit_result_carrier_error_return(
            branch.error_block,
            &result_carrier,
            current_error_type,
            span,
            *span,
        )?;

        self.set_current_block(branch.success_block, span)?;
        let success_region = self.current_region_or_error(span)?;
        let success_result = self.make_local_load_expression(
            result_carrier.result_local,
            result_carrier.carrier_type,
            &None,
            success_region,
        );
        let mut success_payload = self.make_expression(
            &None,
            HirExpressionKind::FallibleUnwrapSuccess {
                result: Box::new(success_result),
            },
            result_carrier.ok_type,
            ValueKind::RValue,
            success_region,
        );

        if result_carrier.validate_float_success {
            success_payload = self.emit_validated_float_value(success_payload, span)?;
        }

        Ok(success_payload)
    }

    pub(super) fn emit_result_carrier_branch(
        &mut self,
        result_local: LocalId,
        carrier_type: TypeId,
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
        success_label: &str,
        error_label: &str,
    ) -> Result<FallibleCarrierBranch, CompilerError> {
        let branch_block = self.current_block_id_or_error(span)?;
        let branch_region = self.current_region_or_error(span)?;
        let result_for_branch =
            self.make_local_load_expression(result_local, carrier_type, &None, branch_region);
        let success_block = self.create_block(branch_region, span, success_label)?;
        let error_block = self.create_block(branch_region, span, error_label)?;

        self.emit_terminator_with_span(
            branch_block,
            HirTerminator::FallibleBranch {
                result: result_for_branch,
                success_block,
                error_block,
            },
            span,
            authored_span,
        )?;

        Ok(FallibleCarrierBranch {
            success_block,
            error_block,
        })
    }
    pub(super) fn emit_result_carrier_error_return(
        &mut self,
        error_block: BlockId,
        result_carrier: &EmittedFallibleCarrier,
        expected_error_type: TypeId,
        span: &Option<SourceSpan>,
        authored_span: Option<SourceSpan>,
    ) -> Result<(), CompilerError> {
        self.set_current_block(error_block, span)?;
        let error_region = self.current_region_or_error(span)?;
        let error_result = self.make_local_load_expression(
            result_carrier.result_local,
            result_carrier.carrier_type,
            &None,
            error_region,
        );
        let error_payload = self.make_expression(
            &None,
            HirExpressionKind::FallibleUnwrapError {
                result: Box::new(error_result),
            },
            result_carrier.err_type,
            ValueKind::RValue,
            error_region,
        );
        let error_payload =
            self.coerce_postfix_error_payload(error_payload, expected_error_type, span)?;

        self.emit_terminator_with_span(
            error_block,
            HirTerminator::ReturnError(error_payload),
            span,
            authored_span,
        )
    }
    pub(super) fn coerce_postfix_error_payload(
        &mut self,
        error_payload: HirExpression,
        expected_error_type: TypeId,
        span: &Option<SourceSpan>,
    ) -> Result<HirExpression, CompilerError> {
        if error_payload.ty == expected_error_type {
            return Ok(error_payload);
        }

        if self.type_environment.option_inner_type(expected_error_type) == Some(error_payload.ty) {
            let value_name = self.string_table.intern("value");
            let region = error_payload.region;
            return Ok(self.make_expression(
                &None,
                HirExpressionKind::VariantConstruct {
                    carrier: HirVariantCarrier::Option,
                    variant_index: 1,
                    fields: vec![HirVariantField {
                        name: Some(value_name),
                        value: error_payload,
                    }],
                },
                expected_error_type,
                ValueKind::RValue,
                region,
            ));
        }

        return_hir_transformation_error!(
            "Postfix propagation reached HIR with an incompatible error payload",
            self.hir_error_location(span)
        );
    }

    pub(crate) fn result_call_carrier_slots(
        &self,
        target: &CallTarget,
        span: &Option<SourceSpan>,
    ) -> Result<(TypeId, TypeId, TypeId), CompilerError> {
        match target {
            CallTarget::Local(function_id) => {
                let Some(function_index) = self.function_index_by_id.get(function_id).copied()
                else {
                    return_hir_transformation_error!(
                        format!("Function {:?} is not registered in HIR module", function_id),
                        self.hir_error_location(span)
                    );
                };

                let function_return_type = self.module.functions[function_index].return_type;
                match self
                    .type_environment
                    .fallible_carrier_slots(function_return_type)
                {
                    Some((ok, err)) => Ok((function_return_type, ok, err)),
                    None => {
                        return_hir_transformation_error!(
                            "Fallible-handled call targeted a function without an internal carrier return type",
                            self.hir_error_location(span)
                        );
                    }
                }
            }

            CallTarget::CrossModule(origin) => {
                let carrier_type_id = self
                    .imported_fallible_carriers_by_origin
                    .get(origin)
                    .copied()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "Fallible imported call target {origin:?} has no projected carrier type"
                        ))
                    })?;
                match self
                    .type_environment
                    .fallible_carrier_slots(carrier_type_id)
                {
                    Some((success, error)) => Ok((carrier_type_id, success, error)),
                    None => {
                        return_hir_transformation_error!(
                            format!(
                                "Fallible imported call target {origin:?} has an invalid projected carrier type"
                            ),
                            self.hir_error_location(span)
                        );
                    }
                }
            }

            CallTarget::ModulePrivate(identity) => {
                let carrier_type_id = self
                    .module_private_fallible_carriers_by_identity
                    .get(identity)
                    .copied()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "Fallible module-private call target {identity:?} has no projected carrier type"
                        ))
                    })?;
                match self
                    .type_environment
                    .fallible_carrier_slots(carrier_type_id)
                {
                    Some((success, error)) => Ok((carrier_type_id, success, error)),
                    None => {
                        return_hir_transformation_error!(
                            format!(
                                "Fallible module-private call target {identity:?} has an invalid projected carrier type"
                            ),
                            self.hir_error_location(span)
                        );
                    }
                }
            }

            CallTarget::Generated(identity) => {
                let carrier_type_id = self
                    .generated_fallible_carriers_by_identity
                    .get(identity)
                    .copied()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "Fallible generated call target {identity:?} has no projected carrier type"
                        ))
                    })?;
                match self
                    .type_environment
                    .fallible_carrier_slots(carrier_type_id)
                {
                    Some((success, error)) => Ok((carrier_type_id, success, error)),
                    None => {
                        return_hir_transformation_error!(
                            format!(
                                "Fallible generated call target {identity:?} has an invalid projected carrier type"
                            ),
                            self.hir_error_location(span)
                        );
                    }
                }
            }

            CallTarget::External(_) => {
                return_hir_transformation_error!(
                    "Fallible-handled call targeted a host function",
                    self.hir_error_location(span)
                );
            }
        }
    }
}
