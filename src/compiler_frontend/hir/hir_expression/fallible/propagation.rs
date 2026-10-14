//! Native fallible call emission and postfix propagation.

use super::EmittedFallibleResults;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ArgumentListView, CallView, ExpressionOperationView, ExpressionView, ResolvedCallableTarget,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expression_store::{HirConstructionFailure, HirValueRange};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::terminators::HirFallibleOperation;
use crate::compiler_frontend::source::SourceSpan;
use crate::return_hir_transformation_error;

impl<'a> HirBuilder<'a> {
    pub(super) fn lower_invoke_arguments(
        &mut self,
        args: ArgumentListView<'_>,
        receiver: Option<ExpressionView<'_>>,
        span: &Option<SourceSpan>,
    ) -> Result<HirValueRange, HirConstructionFailure> {
        let mut values = Vec::with_capacity(args.len() + usize::from(receiver.is_some()));
        if let Some(receiver) = receiver {
            values.push(self.lower_expression_value_to_current_block(receiver)?);
        }
        for (index, argument) in args.iter().enumerate() {
            let lowered = self.lower_call_argument_value(
                argument,
                span,
                index + usize::from(receiver.is_some()),
            )?;
            for statement in lowered.prelude {
                self.emit_statement_to_current_block(statement, span)?;
            }
            values.push(lowered.value);
        }
        self.module.expressions.append_values(&values, *span)
    }

    pub(in crate::compiler_frontend::hir) fn emit_result_call_to_current_block(
        &mut self,
        target: CallTarget,
        args: ArgumentListView<'_>,
        receiver: Option<ExpressionView<'_>>,
        result_type_ids: &[TypeId],
        span: &Option<SourceSpan>,
    ) -> Result<EmittedFallibleResults, HirConstructionFailure> {
        let (types, error) = self.result_call_slots(&target, span)?;
        if types != result_type_ids {
            return_hir_transformation_error!(
                "Fallible call success slots mismatch",
                self.hir_error_location(span)
            );
        }
        let args = self.lower_invoke_arguments(args, receiver, span)?;
        self.emit_fallible_operation(
            HirFallibleOperation::Call { target, args },
            &types,
            error,
            span,
        )
    }

    pub(in crate::compiler_frontend::hir::hir_expression) fn emit_result_expression_to_current_block(
        &mut self,
        value: ExpressionView<'_>,
    ) -> Result<EmittedFallibleResults, HirConstructionFailure> {
        let span = value.span();
        match value.operation() {
            ExpressionOperationView::Call(call) => {
                self.emit_resolved_result_call(value, call, None)
            }
            ExpressionOperationView::MethodCall { receiver, call } => {
                self.emit_resolved_result_call(value, call, Some(receiver))
            }
            ExpressionOperationView::MapBuiltin { receiver, facts, arguments } => {
                let op = super::super::calls::hir_map_op_from_builtin(facts.operation);
                if facts.receiver_requires_mutable != op.requires_mutable_receiver() {
                    return_hir_transformation_error!(
                        "Fallible map producer has inconsistent receiver access",
                        self.hir_error_location(&span)
                    );
                }
                let Some(error_type) = value.error_type() else {
                    return_hir_transformation_error!(
                        "Fallible map producer lacks an error channel",
                        self.hir_error_location(&span)
                    );
                };
                // Commit receiver work before arguments can enter a failure continuation.
                let receiver = self.lower_expression_value_to_current_block(receiver)?;
                let args = self.lower_invoke_arguments(arguments, None, &span)?;
                let types = self.lower_call_result_types(value.success_types(), &span)?;
                let error = self.lower_type_id(error_type, &span)?;
                self.emit_fallible_operation(
                    HirFallibleOperation::MapOp { op, receiver, args }, &types, error, &span,
                )
            }
            ExpressionOperationView::CollectionBuiltin { receiver, facts, arguments } => {
                let Some(error) = value.error_type() else {
                    return_hir_transformation_error!(
                        "Fallible collection producer lacks error channel",
                        self.hir_error_location(&span)
                    );
                };
                self.emit_external_result_call_to_current_block(
                    facts.operation.external_function_id(), arguments, Some(receiver),
                    value.success_types(), false, error, &span,
                )
            }
            ExpressionOperationView::Cast { source, facts } => match &facts.evidence {
                crate::compiler_frontend::ast::expressions::expression_types::ResolvedCastEvidence::Builtin { policy } =>
                    self.emit_builtin_cast_results(source, facts, *policy, &span),
                crate::compiler_frontend::ast::expressions::expression_types::ResolvedCastEvidence::UserDefined { method_path, .. } => {
                    let target = self.resolve_call_target_or_error(method_path, &span)?;
                    self.emit_user_defined_cast_call_results(target, source, &span)
                }
                _ => return_hir_transformation_error!(
                    "Unresolved fallible cast evidence", self.hir_error_location(&span)
                ),
            },
            _ => return_hir_transformation_error!(
                "Unsupported raw fallible producer", self.hir_error_location(&span)
            ),
        }
    }

    fn emit_resolved_result_call(
        &mut self,
        value: ExpressionView<'_>,
        call: CallView<'_>,
        receiver: Option<ExpressionView<'_>>,
    ) -> Result<EmittedFallibleResults, HirConstructionFailure> {
        let metadata = call.metadata();
        let span = value.span();
        match metadata.target {
            ResolvedCallableTarget::Function(name) => {
                let target = self.resolve_call_target_or_error(&name, &span)?;
                self.emit_result_call_to_current_block(
                    target,
                    call.arguments(),
                    receiver,
                    value.success_types(),
                    &span,
                )
            }
            ResolvedCallableTarget::Host(id) => {
                let Some(error) = metadata.error_type.or(value.error_type()) else {
                    return_hir_transformation_error!(
                        "Fallible host producer lacks error channel",
                        self.hir_error_location(&span)
                    );
                };
                self.emit_external_result_call_to_current_block(
                    id,
                    call.arguments(),
                    receiver,
                    value.success_types(),
                    metadata.requires_external_float_validation,
                    error,
                    &span,
                )
            }
        }
    }
}
