//! Call-expression lowering helpers.
//!
//! WHAT: lowers resolved user, external, receiver, and collection calls into explicit HIR call
//! statements and values.
//! WHY: ordinary calls emit preludes, ordered result bindings and fresh mutable argument
//! materialisation. Native error continuations and recovery joins live in `fallible/`.
//!
//! ## Diagnostic boundary
//!
//! `InfrastructureFailure::hir_transformation` in this module means an internal
//! HIR transformation or lowering invariant failure only.

use crate::compiler_frontend::ast::expressions::call_argument::CallPassingMode;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ArgumentListView, ArgumentView, ExpressionView,
};
use crate::compiler_frontend::builtins::maps::MapBuiltinOp;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::TypeId as FrontendTypeId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirMapOp, ValueKind};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::source::SourceSpan;

use super::{LoweredExpression, LoweredResults};

impl<'a> HirBuilder<'a> {
    pub(super) fn call_target_requires_result_projection(&self, target: &CallTarget) -> bool {
        match target {
            CallTarget::Local(_) | CallTarget::External(_) => false,
            CallTarget::CrossModule(origin) => {
                !self.module.function_ids_by_origin.contains_key(origin)
            }
            CallTarget::ModulePrivate(identity) => !self
                .module
                .function_ids_by_private_origin
                .contains_key(identity),
            CallTarget::Generated(identity) => !self
                .module
                .function_ids_by_generated
                .contains_key(identity.as_ref()),
        }
    }

    /// Lowers an external function call whose success return is exactly one binary float, emitting
    /// the raw call and a `ValidateFloat` boundary check directly into the current block.
    ///
    /// WHAT: evaluates arguments, emits the external `Call` to the current block, then emits
    ///       `ValidateFloat` on the raw result and returns the validated finite value at its
    ///       resolved exact type.
    /// WHY: `ValidateFloat` can branch, so it must not be hidden inside a passive expression prelude.
    ///      This helper is used only when the producer's `requires_external_float_validation` fact has
    ///      already opted the call into current-block lowering, and the raw/validated locals must use
    ///      the resolved result type rather than re-deriving a default precision.
    pub(crate) fn lower_validated_external_call_expression(
        &mut self,
        id: ExternalFunctionId,
        args: ArgumentListView<'_>,
        receiver: Option<ExpressionView<'_>>,
        result_type_ids: &[FrontendTypeId],
        span: &Option<SourceSpan>,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let mut lowered_args = Vec::with_capacity(args.len() + usize::from(receiver.is_some()));
        if let Some(receiver) = receiver {
            lowered_args.push(self.lower_expression_value_to_current_block(receiver)?);
        }

        for (arg_index, argument) in args.iter().enumerate() {
            let lowered = self.lower_call_argument_value(
                argument,
                span,
                arg_index + usize::from(receiver.is_some()),
            )?;
            for prelude_statement in lowered.prelude {
                self.emit_statement_to_current_block(prelude_statement)?;
            }
            lowered_args.push(lowered.value);
        }

        let result_type = self.lower_call_result_type(result_type_ids)?;
        let result_local = self.allocate_temp_local(result_type, None)?;
        let args = self
            .module
            .expressions
            .append_values(&lowered_args, *span)?;

        let call_statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::Call {
                target: CallTarget::External(id),
                args,
                results: vec![HirLocalDestination::Define(result_local)],
            },
            span: *span,
        };
        self.side_table.map_statement(*span, &call_statement);
        self.emit_statement_to_current_block(call_statement)?;

        let region = self.current_region_or_error()?;
        let no_span = None;
        let raw_value =
            self.make_local_load_expression(result_local, result_type, &no_span, region)?;
        let validated_value = self.emit_validated_float_value(raw_value, span)?;

        Ok(LoweredExpression {
            prelude: vec![],
            value: validated_value,
        })
    }

    /// Lowers a compiler-owned map builtin call into a dedicated `MapOp` HIR statement.
    ///
    /// WHAT: converts `map.get`, `map.contains`, `~map.set`, `~map.remove`, `~map.clear`, and
    ///       `map.length` into explicit HIR map operations.
    /// WHY: map builtins are language constructs, not external calls. This keeps lowering,
    ///      borrow validation, and reachability explicit and backend-neutral.
    pub(crate) fn lower_map_builtin_call_results(
        &mut self,
        op: MapBuiltinOp,
        receiver: ExpressionView<'_>,
        receiver_requires_mutable: bool,
        args: ArgumentListView<'_>,
        result_type_ids: &[FrontendTypeId],
        span: &Option<SourceSpan>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let mut prelude = Vec::new();
        let hir_op = hir_map_op_from_builtin(op);
        if receiver_requires_mutable != hir_op.requires_mutable_receiver() {
            crate::compiler_bug!(
                CompilerStage::HirConstruction,
                "Map builtin {:?} reached HIR lowering with inconsistent receiver mutability",
                op,
            );
        }

        let lowered_receiver = self.lower_child_expression_for_parent(&mut prelude, receiver)?;

        let mut lowered_args = Vec::with_capacity(args.len());
        for (arg_index, argument) in args.iter().enumerate() {
            if self.expression_needs_current_block_lowering(argument.value()) {
                self.flush_pending_call_prelude(&mut prelude)?;
            }
            let lowered = self.lower_call_argument_value(argument, span, arg_index)?;
            prelude.extend(lowered.prelude);
            lowered_args.push(lowered.value);
        }

        let statement_id = self.allocate_node_id();
        let region = self.current_region_or_error()?;
        let result_type = self.lower_call_result_type(result_type_ids)?;

        let result = if result_type == self.type_environment.builtins().none {
            None
        } else {
            Some(self.allocate_temp_local(result_type, None)?)
        };

        let args = self
            .module
            .expressions
            .append_values(&lowered_args, *span)?;
        let statement = HirStatement {
            id: statement_id,
            kind: HirStatementKind::MapOp {
                op: hir_op,
                receiver: lowered_receiver,
                args,
                result: result.map(HirLocalDestination::Define),
            },
            span: *span,
        };

        self.side_table.map_statement(*span, &statement);
        prelude.push(statement);

        let no_span = None;
        let values = if let Some(result_local) = result {
            let value =
                self.make_local_load_expression(result_local, result_type, &no_span, region)?;
            self.log_call_result_binding(span, result, value);
            vec![value]
        } else {
            vec![]
        };
        Ok(LoweredResults { prelude, values })
    }

    pub(crate) fn lower_call_results(
        &mut self,
        target: CallTarget,
        args: ArgumentListView<'_>,
        receiver: Option<ExpressionView<'_>>,
        result_type_ids: &[FrontendTypeId],
        span: &Option<SourceSpan>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let mut prelude = Vec::new();
        let mut lowered_args = Vec::with_capacity(args.len() + usize::from(receiver.is_some()));
        if let Some(receiver) = receiver {
            let value = self.lower_child_expression_for_parent(&mut prelude, receiver)?;
            lowered_args.push(value);
        }
        for (arg_index, argument) in args.iter().enumerate() {
            if self.expression_needs_current_block_lowering(argument.value()) {
                self.flush_pending_call_prelude(&mut prelude)?;
            }
            let lowered = self.lower_call_argument_value(
                argument,
                span,
                arg_index + usize::from(receiver.is_some()),
            )?;
            prelude.extend(lowered.prelude);
            lowered_args.push(lowered.value);
        }
        self.lower_call_results_from_values(target, &lowered_args, prelude, result_type_ids, span)
    }

    pub(super) fn lower_call_results_from_values(
        &mut self,
        target: CallTarget,
        lowered_args: &[HirValueId],
        mut prelude: Vec<HirStatement>,
        result_type_ids: &[FrontendTypeId],
        span: &Option<SourceSpan>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let statement_id = self.allocate_node_id();
        let region = self.current_region_or_error()?;
        let is_external = matches!(target, CallTarget::External(_));
        let types = self.lower_call_result_types(result_type_ids)?;
        if self.call_target_requires_result_projection(&target) {
            self.module.call_result_contracts.insert(
                target.clone(),
                crate::compiler_frontend::hir::module::HirCallResultContract {
                    returns: types.as_slice().into(),
                    error_return: None,
                },
            );
        }
        let mut results = Vec::with_capacity(types.len());
        let mut values = Vec::with_capacity(types.len());
        for ty in types {
            let local = self.allocate_temp_local(ty, None)?;
            results.push(HirLocalDestination::Define(local));
            values.push(self.make_local_load_expression(local, ty, &None, region)?);
        }
        let args = self.module.expressions.append_values(lowered_args, *span)?;
        let statement = HirStatement {
            id: statement_id,
            kind: HirStatementKind::Call {
                target,
                args,
                results,
            },
            span: *span,
        };
        self.side_table.map_statement(*span, &statement);
        if !is_external {
            self.record_catch_protected_call(statement_id);
        }
        prelude.push(statement);
        Ok(LoweredResults { prelude, values })
    }

    pub(crate) fn lower_call_result_types(
        &mut self,
        result_type_ids: &[FrontendTypeId],
    ) -> Result<Vec<TypeId>, HirConstructionFailure> {
        result_type_ids
            .iter()
            .map(|ty| self.lower_type_id(*ty))
            .collect()
    }

    pub(super) fn lower_call_result_type(
        &mut self,
        result_type_ids: &[FrontendTypeId],
    ) -> Result<TypeId, HirConstructionFailure> {
        match result_type_ids {
            [] => Ok(self.type_environment.builtins().none),
            [ty] => self.lower_type_id(*ty),
            _ => Err(InfrastructureFailure::hir_transformation(
                "Multiple slots reached scalar operation",
            )
            .into()),
        }
    }

    pub(super) fn lower_call_argument_value(
        &mut self,
        argument: ArgumentView<'_>,
        call_span: &Option<SourceSpan>,
        argument_index: usize,
    ) -> Result<LoweredExpression, HirConstructionFailure> {
        let value = argument.value();
        let metadata = argument.metadata();
        let lowered_value = if self.expression_needs_current_block_lowering(value) {
            LoweredExpression {
                prelude: vec![],
                value: self.lower_expression_value_to_current_block(value)?,
            }
        } else {
            self.lower_expression(value)?
        };

        if metadata.passing_mode != CallPassingMode::FreshMutableValue {
            return Ok(lowered_value);
        }

        let mut prelude = lowered_value.prelude;
        let value = lowered_value.value;
        let value_type = self.module.expressions.expression(value).ty;
        let temp_local = self.allocate_fresh_mutable_call_arg_local(
            value_type,
            None,
            *call_span,
            argument_index,
        )?;

        let write_statement = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(temp_local),
                value,
            },
            span: None,
        };
        self.side_table
            .map_statement(metadata.span, &write_statement);
        prelude.push(write_statement);

        let region = self.current_region_or_error()?;
        let no_span = None;
        let value = self.make_expression(
            &no_span,
            HirExpressionKind::Load(HirPlace::local(temp_local)),
            value_type,
            ValueKind::RValue,
            region,
        )?;

        Ok(LoweredExpression { prelude, value })
    }

    fn flush_pending_call_prelude(
        &mut self,
        prelude: &mut Vec<HirStatement>,
    ) -> Result<(), HirConstructionFailure> {
        for statement in prelude.drain(..) {
            self.emit_statement_to_current_block(statement)?;
        }

        Ok(())
    }
}

pub(super) fn hir_map_op_from_builtin(op: MapBuiltinOp) -> HirMapOp {
    match op {
        MapBuiltinOp::Get => HirMapOp::Get,
        MapBuiltinOp::Contains => HirMapOp::Contains,
        MapBuiltinOp::Set => HirMapOp::Set,
        MapBuiltinOp::Remove => HirMapOp::Remove,
        MapBuiltinOp::Clear => HirMapOp::Clear,
        MapBuiltinOp::Length => HirMapOp::Length,
    }
}
