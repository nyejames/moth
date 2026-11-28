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

use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, CallPassingMode,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    ArgumentListView, ArgumentView, CallView, ExpressionStore, ExpressionView,
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
use crate::compiler_frontend::hir::expression_store::{HirConstructionFailure, HirProjection};
use crate::compiler_frontend::hir::expressions::{
    HirExpressionKind, HirMapOp, HirVariantCarrier, HirVariantField, OPTION_SOME_VARIANT_INDEX,
    ValueKind,
};
use crate::compiler_frontend::hir::functions::HirFunctionReturnContract;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;

use super::{LoweredExpression, LoweredResults};

pub(super) type CallReceiver<'a> = (ExpressionView<'a>, CallAccessMode);

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
        receiver: Option<CallReceiver<'_>>,
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
        } = self.lower_call_operands(
            args,
            Some((receiver, receiver_access(receiver_requires_mutable))),
            span,
        )?;
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
        receiver: Option<CallReceiver<'_>>,
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
        receiver: Option<CallReceiver<'_>>,
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
        let receiver = call
            .receiver
            .map(|(id, access)| expressions.view(id).map(|view| (view, access)))
            .transpose()?;
        let args = call
            .arguments
            .iter()
            .map(|arg| CallOperand::Never(expressions, arg));
        self.lower_operands_in_authored_order(args, receiver, span)
    }

    fn lower_operands_in_authored_order<'source>(
        &mut self,
        args: impl ExactSizeIterator<Item = CallOperand<'source>> + Clone,
        receiver: Option<CallReceiver<'source>>,
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
        receiver: Option<CallReceiver<'source>>,
        span: &Option<SourceSpan>,
    ) -> Result<LoweredResults, HirConstructionFailure> {
        let offset = usize::from(receiver.is_some());
        let mut values = Vec::with_capacity(args.len() + offset);
        let mut modes = Vec::with_capacity(args.len() + offset);
        let mut prelude = Vec::new();
        if let Some((receiver, access)) = receiver {
            values.push(self.lower_child_expression_for_parent(&mut prelude, receiver)?);
            modes.push(match access {
                CallAccessMode::Shared => CallPassingMode::Shared,
                CallAccessMode::Mutable => CallPassingMode::MutablePlace,
            });
        }

        let mut pending = 0;
        for (slot, argument) in args {
            let (expression, mode, source_span) = argument.parts()?;
            let mut value = if self.expression_needs_current_block_lowering(expression) {
                self.capture_earlier_operands(
                    &mut values[pending..],
                    &modes[pending..],
                    &mut prelude,
                )?;
                pending = values.len();
                self.flush_pending_call_prelude(&mut prelude)?;
                self.lower_expression_value_to_current_block(expression)?
            } else {
                let lowered = self.lower_expression(expression)?;
                if !lowered.prelude.is_empty() || mode == CallPassingMode::FreshMutableValue {
                    self.capture_earlier_operands(
                        &mut values[pending..],
                        &modes[pending..],
                        &mut prelude,
                    )?;
                    pending = values.len();
                }
                prelude.extend(lowered.prelude);
                lowered.value
            };

            if mode == CallPassingMode::FreshMutableValue {
                let ty = self.module.expressions.expression(value).ty;
                let local =
                    self.allocate_fresh_mutable_call_arg_local(ty, None, *span, slot + offset)?;
                value = self.define_call_operand(
                    &mut prelude,
                    value,
                    local,
                    None,
                    source_span,
                    ValueKind::RValue,
                )?;
            }
            values.push(value);
            modes.push(mode);
        }
        Ok(LoweredResults { prelude, values })
    }

    /// Pins earlier operands before a later operand's prelude runs, so each operand observes
    /// storage as it was when the operand was written.
    fn capture_earlier_operands(
        &mut self,
        operands: &mut [HirValueId],
        modes: &[CallPassingMode],
        prelude: &mut Vec<HirStatement>,
    ) -> Result<(), HirConstructionFailure> {
        for (operand, &mode) in operands.iter_mut().zip(modes) {
            // A copy reads source storage now and produces independent data. Don't introduce a
            // live borrow by materialising a plain place.
            if self.is_copy_operand(*operand) {
                let value = self.module.expressions.expression(*operand);
                let (ty, span) = (value.ty, value.span);
                let local = self.allocate_temp_local(ty, span)?;
                *operand = self.define_call_operand(
                    prelude,
                    *operand,
                    local,
                    span,
                    span,
                    ValueKind::RValue,
                )?;
            } else if let Some((load, place)) = self.place_needing_capture(*operand, mode) {
                // Define an ordinary alias before the later prelude. This keeps the captured
                // location and leaves conflicting accesses visible to borrow validation.
                // For shared `some`, bind only the load so the alias remains visible.
                let value = self.module.expressions.expression(load);
                let (ty, span) = (value.ty, value.span);
                let display = self.place_display_path(place)?;
                let mutable = mode == CallPassingMode::MutablePlace;
                let local = self.allocate_operand_capture_local(ty, span, display, mutable)?;
                // Passing an RValue load would read through the capture and give the callee
                // a fresh slot. Pass the alias itself for mutable write-through.
                let kind = if mutable {
                    ValueKind::Place
                } else {
                    ValueKind::RValue
                };
                let captured = self.define_call_operand(prelude, load, local, span, span, kind)?;
                *operand = self.rewrap_option_some(*operand, load, captured)?;
            }
        }
        Ok(())
    }

    fn place_needing_capture(
        &self,
        operand: HirValueId,
        mode: CallPassingMode,
    ) -> Option<(HirValueId, HirPlace)> {
        let load = if mode == CallPassingMode::Shared {
            self.unwrap_option_some(operand)
        } else {
            operand
        };
        let HirExpressionKind::Load(place) = self.module.expressions.expression(load).kind else {
            return None;
        };
        let needs_capture = match mode {
            CallPassingMode::Shared => self
                .side_table
                .local_origin_kind(place.root)
                .is_none_or(|kind| kind == HirLocalOriginKind::User),
            // Projected references already enter a mutable parameter as aliases. A whole
            // local names a stable binding: wrapping it would change slot acquisition to
            // write-through in the callee.
            CallPassingMode::MutablePlace => !place.projections.is_empty(),
            CallPassingMode::FreshMutableValue => false,
        };
        needs_capture.then_some((load, place))
    }

    /// Rebuilds `operand` around `captured` when `payload` sat under an implicit `some`.
    fn rewrap_option_some(
        &mut self,
        operand: HirValueId,
        payload: HirValueId,
        captured: HirValueId,
    ) -> Result<HirValueId, HirConstructionFailure> {
        if operand == payload {
            return Ok(captured);
        }
        let wrapper = self.module.expressions.expression(operand);
        let (ty, span, region) = (wrapper.ty, wrapper.span, wrapper.region);
        let HirExpressionKind::VariantConstruct { fields, .. } = wrapper.kind else {
            return Err(InfrastructureFailure::hir_transformation(
                "captured call operand payload lost its option wrapper",
            )
            .into());
        };
        let name = self.module.expressions.variant_fields(fields)[0].name;
        let fields = self.module.expressions.append_variant_fields(
            &[HirVariantField {
                name,
                value: captured,
            }],
            span,
        )?;
        self.make_expression(
            &span,
            HirExpressionKind::VariantConstruct {
                carrier: HirVariantCarrier::Option,
                variant_index: OPTION_SOME_VARIANT_INDEX,
                fields,
            },
            ty,
            ValueKind::RValue,
            region,
        )
    }

    /// Interns `root.field.field` for diagnostics. The text stops at the first index, whose
    /// operand has no authored spelling here, so the source root is still named.
    fn place_display_path(&mut self, place: HirPlace) -> Result<PathId, HirConstructionFailure> {
        let side_table = &self.side_table;
        let component = |path: Option<PathId>| path.and_then(|path| self.path_fork.component(path));
        let mut text = component(side_table.local_name_path(place.root))
            .map(|name| self.string_table.resolve(name).to_owned())
            .unwrap_or_default();
        for projection in self.module.expressions.projections(place.projections) {
            let HirProjection::Field(field) = projection else {
                break;
            };
            if let Some(name) = component(side_table.field_name_path(*field)) {
                text.push('.');
                text.push_str(self.string_table.resolve(name));
            }
        }
        let text = self.string_table.intern(&text);
        self.path_fork
            .try_intern_child(PathId::ROOT, text)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::HirConstruction,
                    "path table exhausted while naming a call-operand capture",
                )
                .into()
            })
    }

    fn is_copy_operand(&self, operand: HirValueId) -> bool {
        let operand = self.unwrap_option_some(operand);
        matches!(
            self.module.expressions.expression(operand).kind,
            HirExpressionKind::Copy(_)
        )
    }

    /// The value under the `some` that argument validation inserts for an option parameter.
    /// The wrapper is a value tree, so its payload is still read only where the call runs.
    fn unwrap_option_some(&self, operand: HirValueId) -> HirValueId {
        let expressions = &self.module.expressions;
        match expressions.expression(operand).kind {
            HirExpressionKind::VariantConstruct {
                carrier: HirVariantCarrier::Option,
                variant_index: OPTION_SOME_VARIANT_INDEX,
                fields,
            } => match expressions.variant_fields(fields) {
                [field] => field.value,
                _ => operand,
            },
            _ => operand,
        }
    }

    fn define_call_operand(
        &mut self,
        prelude: &mut Vec<HirStatement>,
        value: HirValueId,
        local: LocalId,
        span: Option<SourceSpan>,
        ast_span: Option<SourceSpan>,
        value_kind: ValueKind,
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

        self.make_expression(
            &span,
            HirExpressionKind::Load(HirPlace::local(local)),
            self.module.expressions.expression(value).ty,
            value_kind,
            self.current_region_or_error()?,
        )
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

pub(super) fn method_receiver<'a>(
    receiver: ExpressionView<'a>,
    call: CallView<'a>,
) -> Result<CallReceiver<'a>, HirConstructionFailure> {
    let access = call.metadata().receiver_access.ok_or_else(|| {
        InfrastructureFailure::hir_transformation("Method receiver has no resolved access mode")
    })?;
    Ok((receiver, access))
}

pub(super) fn receiver_access(requires_mutable: bool) -> CallAccessMode {
    if requires_mutable {
        CallAccessMode::Mutable
    } else {
        CallAccessMode::Shared
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
