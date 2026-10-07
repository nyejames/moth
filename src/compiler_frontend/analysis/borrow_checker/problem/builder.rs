//! Validated-HIR to normalized BorrowProblem extraction.
//!
//! WHAT: owns the one function-local traversal that linearises HIR evaluation into normalized
//! points, places, origins, accesses, calls and control-flow events.
//! WHY: Boracle consumes one explicit problem and must not rediscover HIR meaning in its solver.
//!
//! This builder is deliberately not called by the alpha checker. Its inputs are validated HIR
//! plus existing call summaries and external access metadata; it does not parse source, mutate
//! HIR, or decide borrow legality, last use, lifetime topology or backend ownership.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalPackageRegistry};
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expression_store::HirProjection;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirMapOp, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{BlockId as HirBlockId, FunctionId, HirValueId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirLocalDestination, HirWriteTarget};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::utils::{collect_reachable_blocks, terminator_targets};
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use rustc_hash::FxHashMap;
use std::collections::{BTreeMap, BTreeSet};

use super::bindings::Binding;
use super::control_flow::{CfgBlock, CfgEdge, ProgramPoint};
use super::events::{
    AccessKind, AggregateField, BindingDestination, Call, CallArgument, CallEffect, CallResult,
    Event, EventKind, EventSource, JumpArgument, TerminatorEventKind, Use, UseKind,
};
use super::ids::{BindingId, BlockId, CallId, EventId, PlaceId, PointId, ValueOriginId};
use super::origins::{CallResultProvenance, CallResultUnknownReason, OriginKind, ValueOrigin};
use super::places::{Place, ProjectionElem};
use super::{BorrowProblem, BorrowProblemParts};

#[path = "call_effects.rs"]
mod call_effects;

/// Build one normalized problem for one validated HIR function.
pub(crate) fn from_hir(
    module: &HirModule,
    function: &HirFunction,
    local_summaries: Option<&FxHashMap<FunctionId, PublicCallSummary>>,
    external_registry: Option<&ExternalPackageRegistry>,
) -> Result<BorrowProblem, CompilerError> {
    let builder =
        FunctionProblemBuilder::new(module, function, local_summaries, external_registry)?;
    builder.build()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PlaceKey {
    root: BindingId,
    projections: Vec<ProjectionElem>,
}

#[derive(Debug, Clone, Copy)]
struct ValueRef {
    place: Option<PlaceId>,
}

struct CallEffectSpec<'a> {
    label: String,
    arguments: Vec<PlaceId>,
    argument_sources: Vec<EventSource>,
    accesses: Vec<AccessKind>,
    provenance: CallResultProvenance,
    result: Option<HirLocalDestination>,
    source: &'a EventSource,
}

struct FunctionProblemBuilder<'a> {
    module: &'a HirModule,
    function: &'a HirFunction,
    local_summaries: Option<&'a FxHashMap<FunctionId, PublicCallSummary>>,
    external_registry: Option<&'a ExternalPackageRegistry>,
    block_by_id: BTreeMap<u32, &'a HirBlock>,
    reachable_blocks: Vec<HirBlockId>,
    problem_block_by_hir: BTreeMap<u32, BlockId>,
    binding_by_local: BTreeMap<u32, BindingId>,
    synthetic_binding_by_value: BTreeMap<u32, BindingId>,
    bindings: Vec<Binding>,
    place_by_key: BTreeMap<PlaceKey, PlaceId>,
    places: Vec<Place>,
    origins: Vec<ValueOrigin>,
    unknown_origin: Option<ValueOriginId>,
    calls: Vec<Call>,
    uses: Vec<Use>,
    events: Vec<Event>,
    points: Vec<ProgramPoint>,
    blocks: Vec<CfgBlock>,
    edges: Vec<CfgEdge>,
    exits: Vec<BlockId>,
    current_problem_block: Option<BlockId>,
    next_problem_block_id: u32,
}

#[derive(Clone, Copy)]
enum ExpressionPlan {
    Load(HirPlace),
    Copy(HirPlace),
    Literal,
    TupleGet {
        tuple: HirValueId,
        index: usize,
    },
    VariantPayloadGet {
        source: HirValueId,
        field_index: usize,
    },
    FallibleUnwrap(HirValueId),
    Aggregate,
    Other,
}

impl<'a> FunctionProblemBuilder<'a> {
    fn new(
        module: &'a HirModule,
        function: &'a HirFunction,
        local_summaries: Option<&'a FxHashMap<FunctionId, PublicCallSummary>>,
        external_registry: Option<&'a ExternalPackageRegistry>,
    ) -> Result<Self, CompilerError> {
        let mut block_by_id = BTreeMap::new();
        for block in &module.blocks {
            if block_by_id.insert(block.id.0, block).is_some() {
                return Err(compiler_error(format!(
                    "Boracle problem extraction found duplicate HIR block {:?}",
                    block.id
                )));
            }
        }
        if !block_by_id.contains_key(&function.entry.0) {
            return Err(compiler_error(format!(
                "Boracle problem extraction cannot find function entry block {:?}",
                function.entry
            )));
        }

        let reachable_blocks = collect_reachable_blocks(function.entry, |block_id| {
            let block = block_by_id.get(&block_id.0).ok_or_else(|| {
                compiler_error(format!(
                    "Boracle problem extraction reached missing HIR block {:?}",
                    block_id
                ))
            })?;
            Ok::<_, CompilerError>(terminator_targets(&block.terminator))
        })?;

        let mut builder = Self {
            module,
            function,
            local_summaries,
            external_registry,
            block_by_id,
            reachable_blocks,
            problem_block_by_hir: BTreeMap::new(),
            binding_by_local: BTreeMap::new(),
            synthetic_binding_by_value: BTreeMap::new(),
            bindings: Vec::new(),
            place_by_key: BTreeMap::new(),
            places: Vec::new(),
            origins: Vec::new(),
            unknown_origin: None,
            calls: Vec::new(),
            uses: Vec::new(),
            events: Vec::new(),
            points: Vec::new(),
            blocks: Vec::new(),
            edges: Vec::new(),
            exits: Vec::new(),
            current_problem_block: None,
            next_problem_block_id: 0,
        };
        builder.assign_problem_block_ids()?;
        builder.collect_bindings()?;
        Ok(builder)
    }

    fn build(mut self) -> Result<BorrowProblem, CompilerError> {
        for hir_block_id in self.reachable_blocks.clone() {
            self.build_block(hir_block_id)?;
        }

        self.blocks.sort_by_key(|block| block.id);

        let entry = self.problem_block(self.function.entry)?;
        let mut problem_exits = self.exits.clone();
        problem_exits.sort_by_key(|block| block.raw());
        problem_exits.dedup();

        let problem = BorrowProblem::new(BorrowProblemParts {
            bindings: self.bindings,
            points: self.points,
            blocks: self.blocks,
            edges: self.edges,
            entry,
            exits: problem_exits,
            places: self.places,
            origins: self.origins,
            loans: Vec::new(),
            uses: self.uses,
            calls: self.calls,
            events: self.events,
        })?;

        Ok(problem)
    }

    fn assign_problem_block_ids(&mut self) -> Result<(), CompilerError> {
        for (index, hir_block) in self.reachable_blocks.iter().enumerate() {
            let id = dense_id(index, "normalized CFG block")?;
            self.problem_block_by_hir.insert(hir_block.0, id);
        }
        self.next_problem_block_id =
            dense_u32(self.reachable_blocks.len(), "normalized CFG block")?;
        Ok(())
    }

    fn collect_bindings(&mut self) -> Result<(), CompilerError> {
        let mut locals = BTreeMap::<u32, HirLocal>::new();
        for block_id in self.reachable_blocks.clone() {
            let block = self.hir_block(block_id)?.clone();
            for local in block.locals {
                let local_id = local.id;
                if locals.insert(local_id.0, local).is_some() {
                    return Err(compiler_error(format!(
                        "Boracle problem extraction found duplicate HIR local {:?}",
                        local_id
                    )));
                }
            }
        }

        for parameter in &self.function.params {
            if !locals.contains_key(&parameter.0) {
                return Err(compiler_error(format!(
                    "Boracle problem extraction cannot map parameter local {:?}",
                    parameter
                )));
            }
        }

        for (index, (local_id, local)) in locals.into_iter().enumerate() {
            let binding_id = BindingId::new(dense_u32(index, "normalized binding")?);
            self.binding_by_local.insert(local_id, binding_id);
            self.bindings.push(Binding::new(
                binding_id,
                Some(local.id),
                Some(local.region),
                local.mutable,
                matches!(
                    self.module.side_table.local_origin_kind(local.id),
                    Some(
                        HirLocalOriginKind::CompilerTemp
                            | HirLocalOriginKind::CompilerFreshMutableArg
                    )
                ),
                EventSource {
                    hir_node: None,
                    span: local.span,
                },
            ));
        }
        Ok(())
    }

    fn build_block(&mut self, hir_block_id: HirBlockId) -> Result<(), CompilerError> {
        let block = self.hir_block(hir_block_id)?.clone();
        let problem_block_id = self.problem_block(hir_block_id)?;
        self.current_problem_block = Some(problem_block_id);
        let block_source = self.block_source(hir_block_id);
        let entry = self.new_point(problem_block_id, block_source);
        let mut event_ids = Vec::new();

        if hir_block_id == self.function.entry {
            self.emit_parameter_origins(&mut event_ids)?;
        }

        for statement in &block.statements {
            self.lower_statement(statement, &mut event_ids)?;
        }

        let mut successors = terminator_targets(&block.terminator);
        successors.sort_by_key(|successor| successor.0);
        successors.dedup();
        // A terminator's operands are evaluated while the block's bindings are still live, so the
        // return value is read before scope retirement and the terminator event stays last.
        let terminator_source = self.terminator_source(hir_block_id);
        let kind = self.lower_terminator_kind(
            &block.terminator,
            hir_block_id,
            &terminator_source,
            &mut event_ids,
        )?;
        if successors.is_empty() {
            self.emit_scope_exit_events(&block, &mut event_ids, &block_source)?;
        }
        self.emit_event(
            &mut event_ids,
            terminator_source,
            EventKind::Terminator { kind },
        );

        let exit = self.new_point(problem_block_id, block_source);
        self.blocks
            .push(CfgBlock::new(problem_block_id, entry, exit, event_ids));

        if successors.is_empty() {
            self.exits.push(problem_block_id);
        }
        let mut target_blocks = BTreeMap::new();
        for successor in successors {
            let from = problem_block_id;
            let original_to = self.problem_block(successor)?;
            let bindings = self.scope_exit_bindings(&block, successor)?;
            let to = if bindings.is_empty() {
                original_to
            } else {
                let edge_block = self.new_edge_block(bindings, original_to, block_source)?;
                self.edges.push(CfgEdge::new(edge_block, original_to));
                edge_block
            };
            target_blocks.insert(original_to, to);
            if !self
                .edges
                .iter()
                .any(|edge| edge.from == from && edge.to == to)
            {
                self.edges.push(CfgEdge::new(from, to));
            }
        }
        if !target_blocks.is_empty() {
            self.remap_terminator_targets(problem_block_id, &target_blocks)?;
        }

        Ok(())
    }

    fn emit_parameter_origins(
        &mut self,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        for (index, local) in self.function.params.iter().copied().enumerate() {
            let binding = self.binding_for_local(local)?;
            let source = self
                .bindings
                .get(binding.index())
                .map(|binding| binding.source)
                .unwrap_or_else(EventSource::none);
            let place = self.local_place(local, &source)?;
            let origin = self.new_origin(OriginKind::Parameter {
                index: u32::try_from(index).map_err(|_| {
                    compiler_error("Boracle parameter origin index exceeds u32::MAX")
                })?,
            });
            self.emit_event(
                event_ids,
                source,
                EventKind::Fresh {
                    destination: BindingDestination::Define(place),
                    origin,
                },
            );
        }
        Ok(())
    }

    fn lower_statement(
        &mut self,
        statement: &HirStatement,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let source = EventSource {
            hir_node: Some(statement.id),
            span: statement.span,
        };
        match &statement.kind {
            HirStatementKind::Write { target, value } => {
                let destination = match target {
                    HirWriteTarget::DefineLocal(local) => {
                        BindingDestination::Define(self.local_place(*local, &source)?)
                    }
                    HirWriteTarget::AssignPlace(place) => {
                        BindingDestination::Update(self.lower_place(place, &source, event_ids)?)
                    }
                };
                self.lower_assignment(destination, *value, &source, event_ids)?;
            }
            HirStatementKind::Call {
                target,
                args,
                result,
            } => {
                let args = self.module.expressions.values(*args).to_vec();
                self.lower_call(target, &args, *result, &source, event_ids)?;
            }
            HirStatementKind::Expr(expression)
            | HirStatementKind::PushRuntimeFragment {
                value: expression, ..
            } => {
                self.lower_expression(*expression, &source, event_ids)?;
            }
            HirStatementKind::MapOp {
                op,
                receiver,
                args,
                result,
            } => {
                let args = self.module.expressions.values(*args).to_vec();
                self.lower_map_call(*op, *receiver, &args, *result, &source, event_ids)?;
            }
            HirStatementKind::Drop(_) => {}
            HirStatementKind::CastOp {
                source: value,
                result,
                ..
            } => {
                let target = result
                    .map(|destination| {
                        self.local_place(destination.local(), &source)
                            .map(|place| binding_destination(destination, place))
                    })
                    .transpose()?;
                self.lower_fresh_value(target, *value, &source, event_ids)?;
            }
            HirStatementKind::FormatFloat {
                source: value,
                result,
                ..
            }
            | HirStatementKind::ValidateFloat {
                source: value,
                result,
                ..
            } => {
                let target = Some(binding_destination(
                    *result,
                    self.local_place(result.local(), &source)?,
                ));
                self.lower_fresh_value(target, *value, &source, event_ids)?;
            }
            HirStatementKind::NumericOp {
                operands, result, ..
            } => {
                self.lower_numeric_operands(operands, &source, event_ids)?;
                let target =
                    binding_destination(*result, self.local_place(result.local(), &source)?);
                self.emit_fresh_write(target, &source, event_ids)?;
            }
            HirStatementKind::RangeStepFailure { result, .. } => {
                let target =
                    binding_destination(*result, self.local_place(result.local(), &source)?);
                self.emit_fresh_write(target, &source, event_ids)?;
            }
            HirStatementKind::FloatRangeCandidate {
                current,
                step,
                end,
                ascending,
                candidate_result,
                in_range_result,
                ..
            } => {
                self.lower_expression(*current, &source, event_ids)?;
                self.lower_expression(*step, &source, event_ids)?;
                self.lower_expression(*end, &source, event_ids)?;
                self.lower_expression(*ascending, &source, event_ids)?;

                let candidate_target = binding_destination(
                    *candidate_result,
                    self.local_place(candidate_result.local(), &source)?,
                );
                self.emit_fresh_write(candidate_target, &source, event_ids)?;
                let in_range_target = binding_destination(
                    *in_range_result,
                    self.local_place(in_range_result.local(), &source)?,
                );
                self.emit_fresh_write(in_range_target, &source, event_ids)?;
            }
        }
        Ok(())
    }

    fn lower_load_assignment(
        &mut self,
        target: BindingDestination,
        place: &HirPlace,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let source_place = self.lower_place(place, source, event_ids)?;
        self.emit_read(source_place, source, event_ids)?;
        let is_exact_root_self_update = matches!(
            target,
            BindingDestination::Update(destination) if destination == source_place
        ) && self
            .places
            .get(source_place.index())
            .is_some_and(|place| place.projections.is_empty());

        if is_exact_root_self_update {
            // Re-reading the same root preserves its current binding generation and capabilities.
            self.emit_access_with_definition(source_place, UseKind::Write, source, event_ids, false)
        } else {
            self.emit_alias_write(target, source_place, source, event_ids)
        }
    }

    fn lower_assignment(
        &mut self,
        target: BindingDestination,
        expression_id: HirValueId,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        match self.expression_plan(expression_id) {
            ExpressionPlan::Load(place)
                if self.module.expressions.expression(expression_id).value_kind
                    == ValueKind::Place =>
            {
                self.lower_load_assignment(target, &place, source, event_ids)?;
            }
            ExpressionPlan::Load(place)
                if self.module.expressions.expression(expression_id).value_kind
                    == ValueKind::RValue =>
            {
                let source_place = self.lower_place(&place, source, event_ids)?;
                self.emit_read(source_place, source, event_ids)?;
                self.emit_rebind_write(
                    target,
                    super::events::RebindValue::AliasFromPlace(source_place),
                    source,
                    event_ids,
                )?;
            }
            ExpressionPlan::Load(_) => self.emit_fresh_write(target, source, event_ids)?,
            ExpressionPlan::Copy(place) => {
                let source_place = self.lower_place(&place, source, event_ids)?;
                self.emit_read(source_place, source, event_ids)?;
                let origin = self.new_copy_origin();
                self.emit_binding_write(target, source, event_ids)?;
                self.emit_event(
                    event_ids,
                    *source,
                    EventKind::Copy {
                        source: source_place,
                        destination: target,
                        origin,
                    },
                );
            }
            ExpressionPlan::TupleGet { tuple, index } => {
                let value_ref = self.lower_expression(tuple, source, event_ids)?;
                self.emit_projection_write(
                    target,
                    value_ref,
                    ProjectionElem::FixedIndex(index as u32),
                    source,
                    event_ids,
                )?;
            }
            ExpressionPlan::VariantPayloadGet {
                source: value,
                field_index,
            } => {
                let value_ref = self.lower_expression(value, source, event_ids)?;
                self.emit_projection_write(
                    target,
                    value_ref,
                    ProjectionElem::FixedIndex(field_index as u32),
                    source,
                    event_ids,
                )?;
            }
            ExpressionPlan::FallibleUnwrap(result) => {
                let value_ref = self.lower_expression(result, source, event_ids)?;
                self.emit_projection_write(
                    target,
                    value_ref,
                    ProjectionElem::DynamicIndex,
                    source,
                    event_ids,
                )?;
            }
            ExpressionPlan::Aggregate => {
                self.lower_aggregate_into(target, expression_id, source, event_ids)?;
            }
            ExpressionPlan::Literal | ExpressionPlan::Other => {
                self.lower_fresh_value(Some(target), expression_id, source, event_ids)?;
            }
        }
        Ok(())
    }

    fn lower_fresh_value(
        &mut self,
        target: Option<BindingDestination>,
        expression_id: HirValueId,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let Some(target) = target else {
            let _ = self.lower_expression(expression_id, source, event_ids)?;
            return Ok(());
        };
        if matches!(
            self.expression_plan(expression_id),
            ExpressionPlan::Aggregate
        ) {
            return self.lower_aggregate_into(target, expression_id, source, event_ids);
        }
        match self.expression_plan(expression_id) {
            ExpressionPlan::Load(_) | ExpressionPlan::Copy(_) => {
                let _ = self.lower_expression(expression_id, source, event_ids)?;
            }
            _ => {
                let _ = self.lower_expression_children(expression_id, source, event_ids)?;
            }
        }
        let origin = self.new_fresh_origin();
        self.emit_binding_write(target, source, event_ids)?;
        self.emit_event(
            event_ids,
            *source,
            EventKind::Fresh {
                destination: target,
                origin,
            },
        );
        Ok(())
    }

    fn lower_aggregate_into(
        &mut self,
        target: BindingDestination,
        expression_id: HirValueId,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let fields = self.lower_aggregate_children(expression_id, source, event_ids)?;
        for field in &fields {
            self.project_place(target.place(), field.projection)?;
        }
        let origin = self.new_fresh_origin();
        self.emit_binding_write(target, source, event_ids)?;
        self.emit_event(
            event_ids,
            *source,
            EventKind::Aggregate {
                destination: target,
                origin,
                fields: fields.into_boxed_slice(),
            },
        );
        Ok(())
    }

    fn lower_aggregate_children(
        &mut self,
        expression_id: HirValueId,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<Vec<AggregateField>, CompilerError> {
        let mut fields = Vec::new();
        for (projection, child_id) in self.aggregate_expression_children(expression_id) {
            let value_ref = self.lower_expression(child_id, source, event_ids)?;
            if let Some(source) = value_ref.place {
                fields.push(AggregateField { projection, source });
            }
        }
        Ok(fields)
    }

    fn lower_expression_children(
        &mut self,
        expression_id: HirValueId,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<Vec<ValueRef>, CompilerError> {
        let child_ids = self.expression_children(expression_id);
        let mut values = Vec::with_capacity(child_ids.len());
        for child_id in child_ids {
            values.push(self.lower_expression(child_id, source, event_ids)?);
        }
        Ok(values)
    }

    fn expression_plan(&self, expression_id: HirValueId) -> ExpressionPlan {
        match &self.module.expressions.expression(expression_id).kind {
            HirExpressionKind::Load(place) => ExpressionPlan::Load(*place),
            HirExpressionKind::Copy(place) => ExpressionPlan::Copy(*place),
            HirExpressionKind::TupleGet { tuple, index } => ExpressionPlan::TupleGet {
                tuple: *tuple,
                index: *index,
            },
            HirExpressionKind::VariantPayloadGet {
                source,
                field_index,
                ..
            } => ExpressionPlan::VariantPayloadGet {
                source: *source,
                field_index: *field_index,
            },
            HirExpressionKind::FallibleUnwrapSuccess { result }
            | HirExpressionKind::FallibleUnwrapError { result } => {
                ExpressionPlan::FallibleUnwrap(*result)
            }
            HirExpressionKind::StructConstruct { .. }
            | HirExpressionKind::Collection(_)
            | HirExpressionKind::Range { .. }
            | HirExpressionKind::TupleConstruct { .. }
            | HirExpressionKind::VariantConstruct { .. }
            | HirExpressionKind::MapLiteral(_) => ExpressionPlan::Aggregate,
            HirExpressionKind::Number(_)
            | HirExpressionKind::Uint(_)
            | HirExpressionKind::Int(_)
            | HirExpressionKind::Float(_)
            | HirExpressionKind::FixedScalar(_)
            | HirExpressionKind::Bool(_)
            | HirExpressionKind::Char(_)
            | HirExpressionKind::StringLiteral(_)
            | HirExpressionKind::StructuralString { .. } => ExpressionPlan::Literal,
            _ => ExpressionPlan::Other,
        }
    }

    fn expression_children(&self, expression_id: HirValueId) -> Vec<HirValueId> {
        match &self.module.expressions.expression(expression_id).kind {
            HirExpressionKind::BinOp { left, right, .. } => vec![*left, *right],
            HirExpressionKind::UnaryOp { operand, .. }
            | HirExpressionKind::Cast {
                source: operand, ..
            } => vec![*operand],
            HirExpressionKind::StructConstruct { fields, .. } => self
                .module
                .expressions
                .struct_fields(*fields)
                .iter()
                .map(|(_, value)| *value)
                .collect(),
            HirExpressionKind::Collection(elements)
            | HirExpressionKind::TupleConstruct { elements } => {
                self.module.expressions.values(*elements).to_vec()
            }
            HirExpressionKind::Range { start, end } => vec![*start, *end],
            HirExpressionKind::TupleGet { tuple, .. }
            | HirExpressionKind::FallibleUnwrapSuccess { result: tuple }
            | HirExpressionKind::FallibleUnwrapError { result: tuple }
            | HirExpressionKind::VariantPayloadGet { source: tuple, .. } => vec![*tuple],
            HirExpressionKind::VariantConstruct { fields, .. } => self
                .module
                .expressions
                .variant_fields(*fields)
                .iter()
                .map(|field| field.value)
                .collect(),
            HirExpressionKind::MapLiteral(entries) => self
                .module
                .expressions
                .map_entries(*entries)
                .iter()
                .flat_map(|entry| [entry.key, entry.value])
                .collect(),
            HirExpressionKind::Load(_)
            | HirExpressionKind::Copy(_)
            | HirExpressionKind::Number(_)
            | HirExpressionKind::Uint(_)
            | HirExpressionKind::Int(_)
            | HirExpressionKind::Float(_)
            | HirExpressionKind::FixedScalar(_)
            | HirExpressionKind::Bool(_)
            | HirExpressionKind::Char(_)
            | HirExpressionKind::StringLiteral(_)
            | HirExpressionKind::StructuralString { .. } => Vec::new(),
        }
    }

    fn aggregate_expression_children(
        &self,
        expression_id: HirValueId,
    ) -> Vec<(ProjectionElem, HirValueId)> {
        match &self.module.expressions.expression(expression_id).kind {
            HirExpressionKind::StructConstruct { fields, .. } => self
                .module
                .expressions
                .struct_fields(*fields)
                .iter()
                .map(|(field, value)| (ProjectionElem::Field(field.0), *value))
                .collect(),
            HirExpressionKind::Collection(elements)
            | HirExpressionKind::TupleConstruct { elements } => self
                .module
                .expressions
                .values(*elements)
                .iter()
                .enumerate()
                .map(|(index, value)| (ProjectionElem::FixedIndex(index as u32), *value))
                .collect(),
            HirExpressionKind::Range { start, end } => vec![
                (ProjectionElem::FixedIndex(0), *start),
                (ProjectionElem::FixedIndex(1), *end),
            ],
            HirExpressionKind::VariantConstruct { fields, .. } => self
                .module
                .expressions
                .variant_fields(*fields)
                .iter()
                .enumerate()
                .map(|(index, field)| (ProjectionElem::FixedIndex(index as u32), field.value))
                .collect(),
            HirExpressionKind::MapLiteral(entries) => self
                .module
                .expressions
                .map_entries(*entries)
                .iter()
                .flat_map(|entry| {
                    [
                        (ProjectionElem::MapEntry, entry.key),
                        (ProjectionElem::MapEntry, entry.value),
                    ]
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn lower_expression(
        &mut self,
        expression_id: HirValueId,
        fallback_source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<ValueRef, CompilerError> {
        let source = self.value_source(expression_id, fallback_source);
        match self.expression_plan(expression_id) {
            ExpressionPlan::Load(place) => {
                let place = self.lower_place(&place, &source, event_ids)?;
                let value_kind = self.module.expressions.expression(expression_id).value_kind;
                if value_kind != ValueKind::Const {
                    self.emit_read(place, &source, event_ids)?;
                }
                match value_kind {
                    ValueKind::Place => Ok(ValueRef { place: Some(place) }),
                    ValueKind::RValue => {
                        let destination = self.synthetic_place(expression_id.0)?;
                        self.emit_rebind_write(
                            BindingDestination::Define(destination),
                            super::events::RebindValue::AliasFromPlace(place),
                            &source,
                            event_ids,
                        )?;
                        Ok(ValueRef {
                            place: Some(destination),
                        })
                    }
                    ValueKind::Const => {
                        let destination = self.synthetic_place(expression_id.0)?;
                        let origin = self.new_fresh_origin();
                        self.emit_write(destination, &source, event_ids)?;
                        self.emit_event(
                            event_ids,
                            source,
                            EventKind::Fresh {
                                destination: BindingDestination::Define(destination),
                                origin,
                            },
                        );
                        Ok(ValueRef {
                            place: Some(destination),
                        })
                    }
                }
            }
            ExpressionPlan::Copy(place) => {
                let source_place = self.lower_place(&place, &source, event_ids)?;
                self.emit_read(source_place, &source, event_ids)?;
                let destination = self.synthetic_place(expression_id.0)?;
                let origin = self.new_copy_origin();
                self.emit_write(destination, &source, event_ids)?;
                self.emit_event(
                    event_ids,
                    source,
                    EventKind::Copy {
                        source: source_place,
                        destination: BindingDestination::Define(destination),
                        origin,
                    },
                );
                Ok(ValueRef {
                    place: Some(destination),
                })
            }
            ExpressionPlan::Literal => {
                if self.module.expressions.expression(expression_id).value_kind == ValueKind::Const
                {
                    return Ok(ValueRef { place: None });
                }
                let destination = self.synthetic_place(expression_id.0)?;
                let origin = self.new_fresh_origin();
                self.emit_write(destination, &source, event_ids)?;
                self.emit_event(
                    event_ids,
                    source,
                    EventKind::Fresh {
                        destination: BindingDestination::Define(destination),
                        origin,
                    },
                );
                Ok(ValueRef {
                    place: Some(destination),
                })
            }
            ExpressionPlan::TupleGet { tuple, index } => {
                let source_ref = self.lower_expression(tuple, &source, event_ids)?;
                self.expression_projection(
                    expression_id,
                    source_ref,
                    ProjectionElem::FixedIndex(index as u32),
                    &source,
                    event_ids,
                )
            }
            ExpressionPlan::VariantPayloadGet {
                source: value,
                field_index,
            } => {
                let source_ref = self.lower_expression(value, &source, event_ids)?;
                self.expression_projection(
                    expression_id,
                    source_ref,
                    ProjectionElem::FixedIndex(field_index as u32),
                    &source,
                    event_ids,
                )
            }
            ExpressionPlan::FallibleUnwrap(result) => {
                let source_ref = self.lower_expression(result, &source, event_ids)?;
                self.expression_projection(
                    expression_id,
                    source_ref,
                    ProjectionElem::DynamicIndex,
                    &source,
                    event_ids,
                )
            }
            ExpressionPlan::Aggregate => {
                let destination = self.synthetic_place(expression_id.0)?;
                self.lower_aggregate_into(
                    BindingDestination::Define(destination),
                    expression_id,
                    &source,
                    event_ids,
                )?;
                Ok(ValueRef {
                    place: Some(destination),
                })
            }
            _ => {
                let _ = self.lower_expression_children(expression_id, &source, event_ids)?;
                let destination = self.synthetic_place(expression_id.0)?;
                let origin = self.new_fresh_origin();
                self.emit_write(destination, &source, event_ids)?;
                self.emit_event(
                    event_ids,
                    source,
                    EventKind::Fresh {
                        destination: BindingDestination::Define(destination),
                        origin,
                    },
                );
                Ok(ValueRef {
                    place: Some(destination),
                })
            }
        }
    }

    fn expression_projection(
        &mut self,
        expression_id: HirValueId,
        source_ref: ValueRef,
        projection: ProjectionElem,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<ValueRef, CompilerError> {
        let Some(source_place) = source_ref.place else {
            return Ok(ValueRef { place: None });
        };
        let projected_source = self.project_place(source_place, projection)?;
        let destination = self.synthetic_place(expression_id.0)?;
        let origin = self.new_projection_origin(projection);
        self.emit_read(projected_source, source, event_ids)?;
        self.emit_binding_write(BindingDestination::Define(destination), source, event_ids)?;
        self.emit_event(
            event_ids,
            *source,
            EventKind::Projection {
                source: source_place,
                destination: BindingDestination::Define(destination),
                origin,
            },
        );
        Ok(ValueRef {
            place: Some(destination),
        })
    }

    fn emit_projection_write(
        &mut self,
        target: BindingDestination,
        source_ref: ValueRef,
        projection: ProjectionElem,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let Some(source_place) = source_ref.place else {
            return self.emit_fresh_write(target, source, event_ids);
        };
        let origin = self.new_projection_origin(projection);
        self.emit_binding_write(target, source, event_ids)?;
        self.emit_event(
            event_ids,
            *source,
            EventKind::Projection {
                source: source_place,
                destination: target,
                origin,
            },
        );
        Ok(())
    }

    fn lower_numeric_operands(
        &mut self,
        operands: &crate::compiler_frontend::hir::numeric::HirNumericOperands,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        match operands {
            crate::compiler_frontend::hir::numeric::HirNumericOperands::Unary { operand } => {
                self.lower_expression(*operand, source, event_ids)?;
            }
            crate::compiler_frontend::hir::numeric::HirNumericOperands::Binary { left, right } => {
                self.lower_expression(*left, source, event_ids)?;
                self.lower_expression(*right, source, event_ids)?;
            }
        }
        Ok(())
    }

    fn lower_call(
        &mut self,
        target: &CallTarget,
        args: &[HirValueId],
        result: Option<HirLocalDestination>,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let arguments = self.lower_call_arguments(args, source, event_ids)?;
        let argument_sources = args
            .iter()
            .map(|argument| self.value_source(*argument, source))
            .collect();
        let (accesses, provenance) = self.call_effect(target, args.len())?;
        self.emit_call_effect_with_label(
            CallEffectSpec {
                label: format!("{target:?}"),
                arguments,
                argument_sources,
                accesses,
                provenance,
                result,
                source,
            },
            event_ids,
        )
    }

    fn lower_map_call(
        &mut self,
        op: HirMapOp,
        receiver: HirValueId,
        args: &[HirValueId],
        result: Option<HirLocalDestination>,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let mut expression_ids = Vec::with_capacity(args.len() + 1);
        expression_ids.push(receiver);
        expression_ids.extend_from_slice(args);
        let arguments = self.lower_call_arguments(&expression_ids, source, event_ids)?;
        let argument_sources = expression_ids
            .iter()
            .map(|argument| self.value_source(*argument, source))
            .collect();
        let receiver_access = if op.requires_mutable_receiver() {
            AccessKind::Exclusive
        } else {
            AccessKind::Shared
        };
        let mut accesses = vec![receiver_access];
        accesses.extend(std::iter::repeat_n(AccessKind::Shared, args.len()));
        let provenance = match op {
            HirMapOp::Get => CallResultProvenance::AliasParams(vec![0].into_boxed_slice()),
            HirMapOp::Remove => {
                CallResultProvenance::Unknown(CallResultUnknownReason::OpaqueExternal)
            }
            HirMapOp::Contains | HirMapOp::Set | HirMapOp::Clear | HirMapOp::Length => {
                CallResultProvenance::Fresh
            }
        };
        let label = format!("map::{op:?}");
        self.emit_call_effect_with_label(
            CallEffectSpec {
                label,
                arguments,
                argument_sources,
                accesses,
                provenance,
                result,
                source,
            },
            event_ids,
        )
    }

    fn lower_call_arguments(
        &mut self,
        args: &[HirValueId],
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<Vec<PlaceId>, CompilerError> {
        args.iter()
            .map(|argument_id| {
                let argument_source = self.value_source(*argument_id, source);
                let value = match self.expression_plan(*argument_id) {
                    ExpressionPlan::Load(place)
                        if self.module.expressions.expression(*argument_id).value_kind
                            == ValueKind::Place =>
                    {
                        ValueRef {
                            place: Some(self.lower_place(&place, &argument_source, event_ids)?),
                        }
                    }
                    _ => self.lower_expression(*argument_id, source, event_ids)?,
                };
                self.materialize_value_place(value, *argument_id, &argument_source, event_ids)
            })
            .collect()
    }

    fn emit_call_effect_with_label(
        &mut self,
        spec: CallEffectSpec<'_>,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        if spec.arguments.len() != spec.accesses.len()
            || spec.arguments.len() != spec.argument_sources.len()
        {
            return Err(compiler_error(
                "Boracle problem extraction produced mismatched call argument metadata",
            ));
        }
        let CallEffectSpec {
            label,
            arguments,
            argument_sources,
            accesses,
            provenance,
            result: result_local,
            source,
        } = spec;
        let call_id = self.next_call_id()?;
        self.calls.push(Call { id: call_id, label });
        let mut call_arguments = Vec::with_capacity(arguments.len());
        for (index, ((place, access), argument_source)) in arguments
            .into_iter()
            .zip(accesses)
            .zip(argument_sources)
            .enumerate()
        {
            let point = self.new_point(self.current_problem_block()?, argument_source);
            let use_id = self.next_use_id()?;
            self.uses.push(Use {
                id: use_id,
                point,
                place,
                kind: if access == AccessKind::Exclusive {
                    UseKind::Write
                } else {
                    UseKind::Read
                },
                definition: false,
            });
            call_arguments.push(CallArgument {
                place,
                access,
                use_id,
            });
            let event_id = self.next_event_id()?;
            self.events.push(Event::new(
                event_id,
                point,
                EventKind::CallArgument {
                    call: call_id,
                    index: u32::try_from(index).map_err(|_| {
                        compiler_error("Boracle call argument index exceeds u32::MAX")
                    })?,
                    argument: call_arguments
                        .last()
                        .cloned()
                        .expect("call argument was just appended"),
                },
                argument_source,
            ));
            event_ids.push(event_id);
        }
        let result = result_local
            .map(|destination| {
                self.local_place(destination.local(), source)
                    .map(|place| (binding_destination(destination, place), place))
            })
            .transpose()?;
        let call_result = result.map(|(destination, _place)| {
            let origin = self.new_origin(OriginKind::CallResult {
                call: call_id,
                provenance,
            });
            CallResult {
                destination,
                origin,
            }
        });
        let event_id = self.next_event_id()?;
        let point = self.new_point(self.current_problem_block()?, *source);
        self.events.push(Event::new(
            event_id,
            point,
            EventKind::CallEffect(CallEffect {
                call: call_id,
                arguments: call_arguments.into_boxed_slice(),
                result: call_result,
            }),
            *source,
        ));
        event_ids.push(event_id);
        if let Some((destination, result)) = result {
            let point = self.new_point(self.current_problem_block()?, *source);
            let use_id = self.next_use_id()?;
            self.uses.push(Use {
                id: use_id,
                point,
                place: result,
                kind: UseKind::BindingWrite(destination),
                definition: true,
            });
            let event_id = self.next_event_id()?;
            self.events.push(Event::new(
                event_id,
                point,
                EventKind::Access { use_id },
                *source,
            ));
            event_ids.push(event_id);
        }
        Ok(())
    }

    /// Lower a terminator's operand accesses and return its event kind.
    ///
    /// operand accesses and the terminator itself.
    fn lower_terminator_kind(
        &mut self,
        terminator: &HirTerminator,
        block_id: HirBlockId,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<TerminatorEventKind, CompilerError> {
        let kind = match terminator {
            HirTerminator::Jump { target, args } => {
                let mut destinations = BTreeSet::new();
                let mut captured_arguments = Vec::with_capacity(args.len());
                for argument in args {
                    if !destinations.insert(argument.destination.0) {
                        return Err(compiler_error(format!(
                            "Boracle problem extraction found duplicate jump destination {:?} into block {:?}",
                            argument.destination, target
                        )));
                    }
                    let source_place = self.local_place(argument.source, source)?;
                    let destination_place = self.local_place(argument.destination, source)?;
                    self.emit_read(source_place, source, event_ids)?;
                    captured_arguments.push((source_place, destination_place));
                }
                let mut jump_arguments = Vec::with_capacity(captured_arguments.len());
                for (source_place, destination_place) in captured_arguments {
                    self.emit_binding_write(
                        BindingDestination::Define(destination_place),
                        source,
                        event_ids,
                    )?;
                    jump_arguments.push(JumpArgument {
                        source: source_place,
                        destination: BindingDestination::Define(destination_place),
                    });
                }
                TerminatorEventKind::Jump {
                    target: self.problem_block(*target)?,
                    arguments: jump_arguments.into_boxed_slice(),
                }
            }
            HirTerminator::If {
                condition,
                then_block,
                else_block,
            } => {
                self.lower_expression(*condition, source, event_ids)?;
                TerminatorEventKind::Branch {
                    targets: self.sorted_problem_targets([*then_block, *else_block].into_iter())?,
                }
            }
            HirTerminator::FallibleBranch {
                result,
                success_block,
                error_block,
            } => {
                self.lower_expression(*result, source, event_ids)?;
                TerminatorEventKind::Branch {
                    targets: self
                        .sorted_problem_targets([*success_block, *error_block].into_iter())?,
                }
            }
            HirTerminator::Match { scrutinee, arms } => {
                self.lower_expression(*scrutinee, source, event_ids)?;
                for arm in arms {
                    self.lower_match_arm(arm, source, event_ids)?;
                }
                TerminatorEventKind::Branch {
                    targets: self.sorted_problem_targets(arms.iter().map(|arm| arm.body))?,
                }
            }
            HirTerminator::Break { target } => TerminatorEventKind::Break {
                target: self.problem_block(*target)?,
            },
            HirTerminator::Continue { target } => TerminatorEventKind::Continue {
                target: self.problem_block(*target)?,
            },
            HirTerminator::Return(value) => {
                self.lower_expression(*value, source, event_ids)?;
                TerminatorEventKind::Return
            }
            HirTerminator::ReturnSuccess(value) => {
                self.lower_expression(*value, source, event_ids)?;
                TerminatorEventKind::ReturnSuccess
            }
            HirTerminator::ReturnError(value) => {
                self.lower_expression(*value, source, event_ids)?;
                TerminatorEventKind::ReturnError
            }
            HirTerminator::RuntimeFailure { .. } => TerminatorEventKind::RuntimeFailure,
            HirTerminator::AssertFailure { message, .. } => {
                self.lower_expression(*message, source, event_ids)?;
                TerminatorEventKind::AssertFailure
            }
            HirTerminator::Uninitialized => {
                return Err(compiler_error(format!(
                    "Boracle problem extraction reached uninitialized HIR terminator in block {:?}",
                    block_id
                )));
            }
        };
        Ok(kind)
    }

    fn lower_match_arm(
        &mut self,
        arm: &HirMatchArm,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        match &arm.pattern {
            HirPattern::Literal(value)
            | HirPattern::OptionValue { value }
            | HirPattern::OptionRelational { value, .. }
            | HirPattern::Relational { value, .. } => {
                self.lower_expression(*value, source, event_ids)?;
            }
            HirPattern::OptionNone
            | HirPattern::OptionPresent
            | HirPattern::Wildcard
            | HirPattern::ChoiceVariant { .. } => {}
        }
        if let Some(guard) = arm.guard {
            self.lower_expression(guard, source, event_ids)?;
        }
        Ok(())
    }

    fn emit_scope_exit_events(
        &mut self,
        block: &HirBlock,
        event_ids: &mut Vec<EventId>,
        source: &EventSource,
    ) -> Result<(), CompilerError> {
        let mut bindings = self.visible_bindings(block.region);
        bindings.sort_by_key(|binding| binding.raw());
        bindings.dedup();
        if !bindings.is_empty() {
            self.emit_event(
                event_ids,
                *source,
                EventKind::ScopeExit {
                    bindings: bindings.into_boxed_slice(),
                },
            );
        }
        Ok(())
    }

    fn scope_exit_bindings(
        &self,
        block: &HirBlock,
        successor: HirBlockId,
    ) -> Result<Box<[BindingId]>, CompilerError> {
        let next = self.hir_block(successor)?;
        let next_visible = self
            .visible_bindings(next.region)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let mut bindings = self
            .visible_bindings(block.region)
            .into_iter()
            .filter(|binding| !next_visible.contains(binding))
            .collect::<Vec<_>>();
        bindings.sort_by_key(|binding| binding.raw());
        bindings.dedup();
        Ok(bindings.into_boxed_slice())
    }

    fn visible_bindings(
        &self,
        region: crate::compiler_frontend::hir::ids::RegionId,
    ) -> Vec<BindingId> {
        self.bindings
            .iter()
            .filter_map(|binding| {
                binding
                    .region
                    .filter(|binding_region| self.region_contains(*binding_region, region))
                    .map(|_| binding.id)
            })
            .collect()
    }

    fn new_edge_block(
        &mut self,
        bindings: Box<[BindingId]>,
        target: BlockId,
        source: EventSource,
    ) -> Result<BlockId, CompilerError> {
        let id = BlockId::new(self.next_problem_block_id);
        self.next_problem_block_id = self
            .next_problem_block_id
            .checked_add(1)
            .ok_or_else(|| compiler_error("normalized CFG block table is larger than u32::MAX"))?;
        let entry = self.new_point(id, source);
        let event_id = self.next_event_id()?;
        self.events.push(Event::new(
            event_id,
            entry,
            EventKind::ScopeExit { bindings },
            source,
        ));
        let jump_point = self.new_point(id, source);
        let jump_event_id = self.next_event_id()?;
        self.events.push(Event::new(
            jump_event_id,
            jump_point,
            EventKind::Terminator {
                kind: TerminatorEventKind::Jump {
                    target,
                    arguments: Box::new([]),
                },
            },
            source,
        ));
        let exit = self.new_point(id, source);
        self.blocks.push(CfgBlock::new(
            id,
            entry,
            exit,
            vec![event_id, jump_event_id],
        ));
        Ok(id)
    }

    fn remap_terminator_targets(
        &mut self,
        block: BlockId,
        target_blocks: &BTreeMap<BlockId, BlockId>,
    ) -> Result<(), CompilerError> {
        let event_id = self
            .blocks
            .iter()
            .find(|candidate| candidate.id == block)
            .and_then(|candidate| candidate.events.last().copied())
            .ok_or_else(|| {
                compiler_error(format!("normalized CFG block {block:?} has no terminator"))
            })?;
        let event = self.events.get_mut(event_id.index()).ok_or_else(|| {
            compiler_error(format!("unknown normalized terminator event {event_id:?}"))
        })?;
        let EventKind::Terminator { kind } = &mut event.kind else {
            return Err(compiler_error(format!(
                "normalized CFG block {block:?} does not end in a terminator event"
            )));
        };
        let remap = |target: &mut BlockId| {
            if let Some(replacement) = target_blocks.get(target) {
                *target = *replacement;
            }
        };
        match kind {
            TerminatorEventKind::Jump { target, .. }
            | TerminatorEventKind::Break { target }
            | TerminatorEventKind::Continue { target } => remap(target),
            TerminatorEventKind::Branch { targets } => {
                for target in targets.iter_mut() {
                    remap(target);
                }
            }
            TerminatorEventKind::Return
            | TerminatorEventKind::ReturnSuccess
            | TerminatorEventKind::ReturnError
            | TerminatorEventKind::RuntimeFailure
            | TerminatorEventKind::AssertFailure => {}
        }
        Ok(())
    }

    fn lower_place(
        &mut self,
        place: &HirPlace,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<PlaceId, CompilerError> {
        let mut place_id = self.local_place(place.root, source)?;
        let projections = self
            .module
            .expressions
            .projections(place.projections)
            .to_vec();
        for projection in projections {
            let projection = match projection {
                HirProjection::Field(field) => ProjectionElem::Field(field.0),
                HirProjection::Index(index_id) => {
                    let fixed_index = match &self.module.expressions.expression(index_id).kind {
                        HirExpressionKind::Int(value) => Some(*value),
                        _ => None,
                    };
                    match fixed_index {
                        // Fixed projections need a 32-bit table id, so only values that
                        // fit `u32` stay fixed; negatives and oversized values use the
                        // dynamic path.
                        Some(value) => u32::try_from(value)
                            .map(ProjectionElem::FixedIndex)
                            .unwrap_or(ProjectionElem::DynamicIndex),
                        None => {
                            self.lower_expression(index_id, source, event_ids)?;
                            ProjectionElem::DynamicIndex
                        }
                    }
                }
            };
            place_id = self.project_place(place_id, projection)?;
        }
        Ok(place_id)
    }

    fn local_place(
        &mut self,
        local: LocalId,
        _source: &EventSource,
    ) -> Result<PlaceId, CompilerError> {
        let binding = self.binding_for_local(local)?;
        self.intern_place(binding, Vec::new())
    }

    fn project_place(
        &mut self,
        base: PlaceId,
        projection: ProjectionElem,
    ) -> Result<PlaceId, CompilerError> {
        let base_place = self
            .places
            .get(base.index())
            .ok_or_else(|| compiler_error(format!("unknown normalized base place {base:?}")))?;
        let mut projections = base_place.projections.to_vec();
        projections.push(projection);
        self.intern_place(base_place.root, projections)
    }

    fn synthetic_place(&mut self, value_id: u32) -> Result<PlaceId, CompilerError> {
        let binding = if let Some(binding) = self.synthetic_binding_by_value.get(&value_id) {
            *binding
        } else {
            let binding = BindingId::new(dense_u32(
                self.bindings.len(),
                "synthetic normalized binding",
            )?);
            self.bindings.push(Binding::new(
                binding,
                None,
                None,
                false,
                false,
                EventSource::none(),
            ));
            self.synthetic_binding_by_value.insert(value_id, binding);
            binding
        };
        self.intern_place(binding, Vec::new())
    }

    fn intern_place(
        &mut self,
        root: BindingId,
        projections: Vec<ProjectionElem>,
    ) -> Result<PlaceId, CompilerError> {
        let key = PlaceKey {
            root,
            projections: projections.clone(),
        };
        if let Some(place) = self.place_by_key.get(&key) {
            return Ok(*place);
        }
        let id = PlaceId::new(dense_u32(self.places.len(), "normalized place")?);
        self.places.push(Place::new(id, root, projections));
        self.place_by_key.insert(key, id);
        Ok(id)
    }

    fn materialize_value_place(
        &mut self,
        value: ValueRef,
        expression_id: HirValueId,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<PlaceId, CompilerError> {
        if let Some(place) = value.place {
            return Ok(place);
        }
        let place = self.synthetic_place(expression_id.0)?;
        let origin = self.new_fresh_origin();
        self.emit_write(place, source, event_ids)?;
        self.emit_event(
            event_ids,
            *source,
            EventKind::Fresh {
                destination: BindingDestination::Define(place),
                origin,
            },
        );
        Ok(place)
    }

    fn emit_alias_write(
        &mut self,
        destination: BindingDestination,
        source_place: PlaceId,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let destination_place = destination.place();
        self.emit_binding_write(destination, source, event_ids)?;
        let destination_binding = self
            .places
            .get(destination_place.index())
            .ok_or_else(|| {
                compiler_error(format!("unknown alias destination {destination_place:?}"))
            })?
            .root;
        let destination_access = self.alias_access_kind(destination_binding);
        let event = if destination_access == AccessKind::Exclusive {
            EventKind::ExclusiveAliasFromPlace {
                source: source_place,
                destination,
            }
        } else {
            EventKind::AliasFromPlace {
                source: source_place,
                destination,
            }
        };
        self.emit_event(event_ids, *source, event);
        Ok(())
    }

    fn alias_access_kind(&self, destination_binding: BindingId) -> AccessKind {
        let Some(binding) = self.bindings.get(destination_binding.index()) else {
            return AccessKind::Shared;
        };
        match binding
            .hir_local
            .and_then(|local| self.module.side_table.local_origin_kind(local))
        {
            Some(HirLocalOriginKind::CompilerFreshMutableArg) => AccessKind::Exclusive,
            Some(HirLocalOriginKind::CompilerTemp) => AccessKind::Shared,
            Some(HirLocalOriginKind::User) | None => {
                if binding.mutable {
                    AccessKind::Exclusive
                } else {
                    AccessKind::Shared
                }
            }
        }
    }

    fn emit_fresh_write(
        &mut self,
        destination: BindingDestination,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let origin = self.new_fresh_origin();
        self.emit_binding_write(destination, source, event_ids)?;
        let event = EventKind::Fresh {
            destination,
            origin,
        };
        self.emit_event(event_ids, *source, event);
        Ok(())
    }

    fn emit_rebind_write(
        &mut self,
        destination: BindingDestination,
        value: super::events::RebindValue,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        self.emit_binding_write(destination, source, event_ids)?;
        self.emit_event(event_ids, *source, EventKind::Rebind { destination, value });
        Ok(())
    }

    fn emit_write(
        &mut self,
        place: PlaceId,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        self.emit_access(place, UseKind::Write, source, event_ids)
    }

    fn emit_binding_write(
        &mut self,
        destination: BindingDestination,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        self.emit_access(
            destination.place(),
            UseKind::BindingWrite(destination),
            source,
            event_ids,
        )
    }

    fn emit_read(
        &mut self,
        place: PlaceId,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        self.emit_access(place, UseKind::Read, source, event_ids)
    }

    fn emit_access(
        &mut self,
        place: PlaceId,
        kind: UseKind,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let definition = kind.is_write()
            && self
                .places
                .get(place.index())
                .is_some_and(|place| place.projections.is_empty());
        self.emit_access_with_definition(place, kind, source, event_ids, definition)
    }

    fn emit_access_with_definition(
        &mut self,
        place: PlaceId,
        kind: UseKind,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
        definition: bool,
    ) -> Result<(), CompilerError> {
        let point = self.new_point(self.current_problem_block()?, *source);
        let use_id = self.next_use_id()?;
        self.uses.push(Use {
            id: use_id,
            point,
            place,
            kind,
            definition,
        });
        let event_id = self.next_event_id()?;
        self.events.push(Event::new(
            event_id,
            point,
            EventKind::Access { use_id },
            *source,
        ));
        event_ids.push(event_id);
        Ok(())
    }

    fn emit_event(
        &mut self,
        event_ids: &mut Vec<EventId>,
        source: EventSource,
        kind: EventKind,
    ) -> EventId {
        let point = self.new_point(
            self.current_problem_block
                .expect("Boracle event emission requires an active CFG block"),
            source,
        );
        let event_id = EventId::new(self.events.len() as u32);
        self.events.push(Event::new(event_id, point, kind, source));
        event_ids.push(event_id);
        event_id
    }

    fn local_parameter_accesses(
        &self,
        function_id: FunctionId,
        argument_count: usize,
    ) -> Result<Option<Vec<AccessKind>>, CompilerError> {
        let Some(function) = self
            .module
            .functions
            .iter()
            .find(|function| function.id == function_id)
        else {
            return Ok(None);
        };
        if function.params.len() != argument_count {
            return Err(compiler_error(format!(
                "Boracle local call {:?} has {} arguments but its HIR function has {} parameters",
                function_id,
                argument_count,
                function.params.len()
            )));
        }

        let mut accesses = Vec::with_capacity(function.params.len());
        for parameter in &function.params {
            let local = self
                .module
                .blocks
                .iter()
                .flat_map(|block| block.locals.iter())
                .find(|local| local.id == *parameter)
                .ok_or_else(|| {
                    compiler_error(format!(
                        "Boracle local call {:?} cannot resolve parameter local {:?}",
                        function_id, parameter
                    ))
                })?;
            accesses.push(if local.mutable {
                AccessKind::Exclusive
            } else {
                AccessKind::Shared
            });
        }
        Ok(Some(accesses))
    }

    fn new_fresh_origin(&mut self) -> ValueOriginId {
        self.new_origin(OriginKind::Fresh)
    }

    fn new_copy_origin(&mut self) -> ValueOriginId {
        let unknown = self.ensure_unknown_origin();
        self.new_origin(OriginKind::Copy(vec![unknown].into_boxed_slice()))
    }

    fn new_projection_origin(&mut self, projection: ProjectionElem) -> ValueOriginId {
        let unknown = self.ensure_unknown_origin();
        self.new_origin(OriginKind::Projection {
            source: unknown,
            projection,
        })
    }

    fn ensure_unknown_origin(&mut self) -> ValueOriginId {
        if let Some(origin) = self.unknown_origin {
            return origin;
        }
        let origin = ValueOriginId::new(self.origins.len() as u32);
        self.origins.push(ValueOrigin::unknown(origin));
        self.unknown_origin = Some(origin);
        origin
    }

    fn new_origin(&mut self, kind: OriginKind) -> ValueOriginId {
        let id = ValueOriginId::new(self.origins.len() as u32);
        self.origins.push(ValueOrigin::new(id, kind));
        id
    }

    fn new_point(&mut self, block: BlockId, source: EventSource) -> PointId {
        let id = PointId::new(self.points.len() as u32);
        self.points.push(ProgramPoint::with_source(
            id,
            block,
            self.next_block_ordinal(block),
            source,
        ));
        id
    }

    fn next_block_ordinal(&self, block: BlockId) -> u32 {
        self.points
            .iter()
            .filter(|point| point.block == block)
            .count() as u32
    }

    fn next_use_id(&self) -> Result<super::ids::UseId, CompilerError> {
        Ok(super::ids::UseId::new(dense_u32(
            self.uses.len(),
            "normalized use",
        )?))
    }

    fn next_event_id(&self) -> Result<EventId, CompilerError> {
        Ok(EventId::new(dense_u32(
            self.events.len(),
            "normalized event",
        )?))
    }

    fn next_call_id(&self) -> Result<CallId, CompilerError> {
        Ok(CallId::new(dense_u32(self.calls.len(), "normalized call")?))
    }

    fn current_problem_block(&self) -> Result<BlockId, CompilerError> {
        self.current_problem_block
            .ok_or_else(|| compiler_error("Boracle problem extraction has no current CFG block"))
    }

    fn problem_block(&self, block: HirBlockId) -> Result<BlockId, CompilerError> {
        self.problem_block_by_hir
            .get(&block.0)
            .copied()
            .ok_or_else(|| {
                compiler_error(format!("unknown normalized target for HIR block {block:?}"))
            })
    }

    fn hir_block(&self, block: HirBlockId) -> Result<&HirBlock, CompilerError> {
        self.block_by_id
            .get(&block.0)
            .copied()
            .ok_or_else(|| compiler_error(format!("unknown HIR block {block:?}")))
    }

    fn binding_for_local(&self, local: LocalId) -> Result<BindingId, CompilerError> {
        self.binding_by_local
            .get(&local.0)
            .copied()
            .ok_or_else(|| compiler_error(format!("unknown HIR local {local:?}")))
    }

    fn block_source(&self, block: HirBlockId) -> EventSource {
        EventSource {
            hir_node: None,
            span: self
                .module
                .side_table
                .hir_source_span_for_hir(
                    crate::compiler_frontend::hir::hir_side_table::HirLocation::Block(block),
                )
                .or_else(|| {
                    self.module.side_table.ast_span_for_hir(
                        crate::compiler_frontend::hir::hir_side_table::HirLocation::Block(block),
                    )
                }),
        }
    }

    fn terminator_source(&self, block: HirBlockId) -> EventSource {
        EventSource {
            hir_node: None,
            span: self.module.side_table.terminator_span(block).copied(),
        }
    }

    fn value_source(&self, expression_id: HirValueId, fallback: &EventSource) -> EventSource {
        let expression = self.module.expressions.expression(expression_id);
        EventSource {
            hir_node: None,
            span: self
                .module
                .side_table
                .value_source_span(expression_id)
                .or(expression.span)
                .or(fallback.span),
        }
    }

    fn region_contains(
        &self,
        ancestor: crate::compiler_frontend::hir::ids::RegionId,
        candidate: crate::compiler_frontend::hir::ids::RegionId,
    ) -> bool {
        let mut current = Some(candidate);
        let mut seen = BTreeSet::new();
        while let Some(region) = current {
            if !seen.insert(region.0) {
                return false;
            }
            if region == ancestor {
                return true;
            }
            current = self
                .module
                .regions
                .iter()
                .find(|entry| entry.id() == region)
                .and_then(|entry| entry.parent());
        }
        false
    }

    fn sorted_problem_targets(
        &self,
        targets: impl Iterator<Item = HirBlockId>,
    ) -> Result<Box<[BlockId]>, CompilerError> {
        let mut targets = targets
            .map(|target| self.problem_block(target))
            .collect::<Result<Vec<_>, _>>()?;
        targets.sort_by_key(|target| target.raw());
        targets.dedup();
        Ok(targets.into_boxed_slice())
    }
}

fn binding_destination(destination: HirLocalDestination, place: PlaceId) -> BindingDestination {
    match destination {
        HirLocalDestination::Define(_) => BindingDestination::Define(place),
        HirLocalDestination::Update(_) => BindingDestination::Update(place),
    }
}

fn dense_id(index: usize, owner: &str) -> Result<BlockId, CompilerError> {
    Ok(BlockId::new(dense_u32(index, owner)?))
}

fn dense_u32(index: usize, owner: &str) -> Result<u32, CompilerError> {
    u32::try_from(index)
        .map_err(|_| compiler_error(format!("{owner} table is larger than u32::MAX rows")))
}

fn compiler_error(message: impl Into<String>) -> CompilerError {
    CompilerError::compiler_error(message)
}
