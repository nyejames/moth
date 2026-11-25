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

use std::num::NonZeroU32;

use crate::compiler_frontend::ast::expressions::call_argument::{CallArgument, CallPassingMode};
use crate::compiler_frontend::ast::expressions::expression_store::{
    ArgumentListView, ArgumentView, ExpressionStore, ExpressionView,
};
use crate::compiler_frontend::ast::statements::never_call::NeverCall;
use crate::compiler_frontend::builtins::maps::MapBuiltinOp;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::datatypes::CallableReturnShape;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::ids::TypeId as FrontendTypeId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::hir::expression_store::HirConstructionFailure;
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirMapOp, HirVariantCarrier, OPTION_SOME_VARIANT_INDEX,
};
use crate::compiler_frontend::hir::functions::HirFunctionReturnContract;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::source::SourceSpan;

use super::{LoweredExpression, LoweredResults};

#[derive(Clone, Copy)]
enum CallOperand<'a> {
    Stored(ArgumentView<'a>),
    Never(&'a ExpressionStore, &'a CallArgument),
}

impl<'a> CallOperand<'a> {
    fn authored_ordinal(self) -> Option<NonZeroU32> {
        match self {
            Self::Stored(argument) => argument.metadata().authored_ordinal,
            Self::Never(_, argument) => argument.authored_ordinal,
        }
    }

    fn parts(
        self,
    ) -> Result<(ExpressionView<'a>, CallPassingMode, Option<SourceSpan>), HirConstructionFailure>
    {
        Ok(match self {
            Self::Stored(argument) => {
                let metadata = argument.metadata();
                (argument.value(), metadata.passing_mode, metadata.span)
            }
            Self::Never(store, argument) => (
                store.view(argument.value)?,
                argument.passing_mode,
                argument.span,
            ),
        })
    }
}

impl<'a> HirBuilder<'a> {
    pub(crate) fn call_target_requires_result_projection(&self, target: &CallTarget) -> bool {
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
        let LoweredResults {
            mut prelude,
            values,
        } = self.lower_call_operands(args, receiver, span)?;
        self.flush_pending_call_prelude(&mut prelude)?;

        let result_type = self.lower_call_result_type(result_type_ids)?;
        let result_local = self.allocate_temp_local(result_type, None)?;
        let args = self.module.expressions.append_values(&values, *span)?;

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
        let hir_op = hir_map_op_from_builtin(op);
        if receiver_requires_mutable != hir_op.requires_mutable_receiver() {
            crate::compiler_bug!(
                CompilerStage::HirConstruction,
                "Map builtin {:?} reached HIR lowering with inconsistent receiver mutability",
                op,
            );
        }

        let LoweredResults {
            mut prelude,
            values,
        } = self.lower_call_operands(args, Some(receiver), span)?;
        let (&lowered_receiver, lowered_args) = values.split_first().ok_or_else(|| {
            InfrastructureFailure::hir_transformation("Map call lost its receiver")
        })?;

        let statement_id = self.allocate_node_id();
        let region = self.current_region_or_error()?;
        let result_type = self.lower_call_result_type(result_type_ids)?;

        let result = if result_type == self.type_environment.builtins().none {
            None
        } else {
            Some(self.allocate_temp_local(result_type, None)?)
        };

        let args = self.module.expressions.append_values(lowered_args, *span)?;
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
        let LoweredResults { prelude, values } = self.lower_call_operands(args, receiver, span)?;
        self.lower_call_results_from_values(target, &values, prelude, result_type_ids, span)
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
                HirFunctionReturnContract::returns(CallableReturnShape::from_slots(
                    types.iter().copied(),
                    None,
                )),
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

    pub(super) fn lower_call_operands(
        &mut self,
        args: ArgumentListView<'_>,
        receiver: Option<ExpressionView<'_>>,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        self.lower_operands_in_authored_order(args.iter().map(CallOperand::Stored), receiver, span)
    }

    pub(in crate::compiler_frontend::hir) fn lower_never_call_operands(
        &mut self,
        expressions: &ExpressionStore,
        call: &NeverCall,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let receiver = call.receiver.map(|id| expressions.view(id)).transpose()?;
        let args = call
            .arguments
            .iter()
            .map(|arg| CallOperand::Never(expressions, arg));
        self.lower_operands_in_authored_order(args, receiver, span)
    }

    fn lower_operands_in_authored_order<'source>(
        &mut self,
        args: impl ExactSizeIterator<Item = CallOperand<'source>> + Clone,
        receiver: Option<ExpressionView<'source>>,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        if args
            .clone()
            .filter_map(CallOperand::authored_ordinal)
            .is_sorted()
        {
            return self.lower_operand_sequence(args.enumerate(), receiver, span);
        }

        // Typed method calls can already carry their receiver as an unmarked
        // leading argument. Leave that prefix first. Remaining unmarked
        // operands are compile-time defaults, not authored evaluations.
        let prefix = args
            .clone()
            .take_while(|arg| arg.authored_ordinal().is_none())
            .count();
        let mut ordered: Vec<_> = args.enumerate().collect();
        ordered.sort_unstable_by_key(|(slot, arg)| match arg.authored_ordinal() {
            Some(ordinal) => (1, ordinal.get() as usize),
            None if *slot < prefix => (0, *slot),
            None => (2, *slot),
        });

        let mut lowered = self.lower_operand_sequence(ordered.iter().copied(), receiver, span)?;
        let offset = usize::from(receiver.is_some());
        // Return to ABI slot order without evaluating any operand again.
        for position in 0..ordered.len() {
            while ordered[position].0 != position {
                let slot = ordered[position].0;
                ordered.swap(position, slot);
                lowered.values.swap(position + offset, slot + offset);
            }
        }
        Ok(lowered)
    }

    fn lower_operand_sequence<'source>(
        &mut self,
        args: impl ExactSizeIterator<Item = (usize, CallOperand<'source>)>,
        receiver: Option<ExpressionView<'source>>,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let offset = usize::from(receiver.is_some());
        let mut values = Vec::with_capacity(args.len() + offset);
        let mut prelude = Vec::new();
        if let Some(receiver) = receiver {
            values.push(self.lower_child_expression_for_parent(&mut prelude, receiver)?);
        }

        let mut pending = 0;
        for (slot, argument) in args {
            let (expression, mode, source_span) = argument.parts()?;
            let mut value = if self.expression_needs_current_block_lowering(expression) {
                self.commit_earlier_copies(&mut values[pending..], &mut prelude)?;
                pending = values.len();
                self.flush_pending_call_prelude(&mut prelude)?;
                self.lower_expression_value_to_current_block(expression)?
            } else {
                let lowered = self.lower_expression(expression)?;
                if !lowered.prelude.is_empty() || mode == CallPassingMode::FreshMutableValue {
                    self.commit_earlier_copies(&mut values[pending..], &mut prelude)?;
                    pending = values.len();
                }
                prelude.extend(lowered.prelude);
                lowered.value
            };

            if mode == CallPassingMode::FreshMutableValue {
                let ty = self.module.expressions.expression(value).ty;
                let local =
                    self.allocate_fresh_mutable_call_arg_local(ty, None, *span, slot + offset)?;
                value = self.define_call_operand(&mut prelude, value, local, None, source_span)?;
            }
            values.push(value);
        }
        Ok(LoweredResults { prelude, values })
    }

    fn commit_earlier_copies(
        &mut self,
        operands: &mut [HirValueId],
        prelude: &mut Vec<HirStatement>,
    ) -> Result<(), HirConstructionFailure> {
        for operand in operands {
            // A copy reads source storage now and produces independent data.
            // Don't introduce a live borrow by materialising a plain place.
            if self.is_copy_operand(*operand) {
                let value = self.module.expressions.expression(*operand);
                let (ty, span) = (value.ty, value.span);
                let local = self.allocate_temp_local(ty, span)?;
                *operand = self.define_call_operand(prelude, *operand, local, span, span)?;
            }
        }
        Ok(())
    }

    /// An explicit copy, possibly wrapped by the implicit `some` that argument
    /// validation inserts for an option parameter.
    fn is_copy_operand(&self, operand: HirValueId) -> bool {
        let expressions = &self.module.expressions;
        match expressions.expression(operand).kind {
            HirExpressionKind::Copy(_) => true,
            HirExpressionKind::VariantConstruct {
                carrier: HirVariantCarrier::Option,
                variant_index: OPTION_SOME_VARIANT_INDEX,
                fields,
            } => matches!(
                expressions.variant_fields(fields),
                [field] if matches!(expressions.expression(field.value).kind, HirExpressionKind::Copy(_))
            ),
            _ => false,
        }
    }

    fn define_call_operand(
        &mut self,
        prelude: &mut Vec<HirStatement>,
        value: HirValueId,
        local: LocalId,
        span: Option<SourceSpan>,
        ast_span: Option<SourceSpan>,
    ) -> Result<HirValueId, HirConstructionFailure> {
        let definition = HirStatement {
            id: self.allocate_node_id(),
            kind: HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                value,
            },
            span,
        };
        self.side_table.map_statement(ast_span, &definition);
        prelude.push(definition);

        let ty = self.module.expressions.expression(value).ty;
        self.make_local_load_expression(local, ty, &span, self.current_region_or_error()?)
    }

    pub(crate) fn flush_pending_call_prelude(
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
