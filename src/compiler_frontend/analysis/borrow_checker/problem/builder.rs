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
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalAccessKind, ExternalPackageRegistry, ExternalReturnAlias,
};
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirMapOp, ValueKind,
};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_side_table::HirLocation;
use crate::compiler_frontend::hir::ids::{BlockId as HirBlockId, FunctionId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::utils::{collect_reachable_blocks, terminator_targets};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallParameterAccess, PublicCallSummary,
};
use rustc_hash::FxHashMap;
use std::collections::{BTreeMap, BTreeSet};

use super::bindings::Binding;
use super::control_flow::{CfgBlock, CfgEdge, ProgramPoint};
use super::events::{
    AccessKind, AggregateField, Call, CallArgument, CallEffect, CallResult, Event, EventKind,
    EventSource, RebindValue, TerminatorEventKind, Use, UseKind,
};
use super::ids::{BindingId, BlockId, CallId, EventId, PlaceId, PointId, ValueOriginId};
use super::origins::{CallResultProvenance, OriginKind, ValueOrigin};
use super::places::{Place, ProjectionElem};
use super::{BorrowProblem, BorrowProblemParts};

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
    accesses: Vec<AccessKind>,
    provenance: CallResultProvenance,
    result: Option<LocalId>,
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
    written_places: BTreeSet<PlaceId>,
    current_problem_block: Option<BlockId>,
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
            written_places: BTreeSet::new(),
            current_problem_block: None,
        };
        builder.assign_problem_block_ids()?;
        builder.collect_bindings()?;
        Ok(builder)
    }

    fn build(mut self) -> Result<BorrowProblem, CompilerError> {
        for hir_block_id in self.reachable_blocks.clone() {
            self.build_block(hir_block_id)?;
        }

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
                EventSource {
                    hir_node: None,
                    location: local.source_info.clone(),
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
        let entry = self.new_point(problem_block_id, block_source.clone());
        let mut event_ids = Vec::new();

        for statement in &block.statements {
            self.lower_statement(statement, &mut event_ids)?;
        }

        let successors = terminator_targets(&block.terminator);
        self.lower_terminator(&block.terminator, hir_block_id, &mut event_ids)?;
        self.emit_scope_exit_events(&block, &successors, &mut event_ids, &block_source)?;

        let exit = self.new_point(problem_block_id, block_source.clone());
        self.blocks
            .push(CfgBlock::new(problem_block_id, entry, exit, event_ids));

        if successors.is_empty() {
            self.exits.push(problem_block_id);
        }
        for successor in successors {
            let from = problem_block_id;
            let to = self.problem_block(successor)?;
            if !self
                .edges
                .iter()
                .any(|edge| edge.from == from && edge.to == to)
            {
                self.edges.push(CfgEdge::new(from, to));
            }
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
            location: Some(statement.location.clone()),
        };
        match &statement.kind {
            HirStatementKind::Assign { target, value } => {
                let target = self.lower_place(target, &source, event_ids)?;
                self.lower_assignment(target, value, &source, event_ids)?;
            }
            HirStatementKind::Call {
                target,
                args,
                result,
            } => self.lower_call(target, args, *result, &source, event_ids)?,
            HirStatementKind::Expr(expression)
            | HirStatementKind::PushRuntimeFragment {
                value: expression, ..
            } => {
                self.lower_expression(expression, &source, event_ids)?;
            }
            HirStatementKind::MapOp {
                op,
                receiver,
                args,
                result,
            } => self.lower_map_call(*op, receiver, args, *result, &source, event_ids)?,
            HirStatementKind::Drop(_) => {}
            HirStatementKind::CastOp {
                source: value,
                result,
                ..
            } => {
                let target = result
                    .map(|local| self.local_place(local, &source))
                    .transpose()?;
                self.lower_fresh_value(target, value, &source, event_ids)?;
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
                let target = Some(self.local_place(*result, &source)?);
                self.lower_fresh_value(target, value, &source, event_ids)?;
            }
            HirStatementKind::NumericOp {
                operands, result, ..
            } => {
                self.lower_numeric_operands(operands, &source, event_ids)?;
                let target = self.local_place(*result, &source)?;
                self.emit_fresh_write(target, &source, event_ids)?;
            }
        }
        Ok(())
    }

    fn lower_assignment(
        &mut self,
        target: PlaceId,
        value: &HirExpression,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        match &value.kind {
            HirExpressionKind::Load(place) => {
                let source_place = self.lower_place(place, source, event_ids)?;
                self.emit_read(source_place, source, event_ids)?;
                self.emit_alias_write(target, source_place, source, event_ids)?;
            }
            HirExpressionKind::Copy(place) => {
                let source_place = self.lower_place(place, source, event_ids)?;
                self.emit_read(source_place, source, event_ids)?;
                let origin = self.new_copy_origin();
                self.emit_write(target, source, event_ids)?;
                self.emit_event(
                    event_ids,
                    source.clone(),
                    EventKind::Copy {
                        source: source_place,
                        destination: target,
                        origin,
                    },
                );
            }
            HirExpressionKind::TupleGet { tuple, index } => {
                let value_ref = self.lower_expression(tuple, source, event_ids)?;
                self.emit_projection_write(
                    target,
                    value_ref,
                    ProjectionElem::FixedIndex(*index as u32),
                    source,
                    event_ids,
                )?;
            }
            HirExpressionKind::VariantPayloadGet {
                source: value,
                field_index,
                ..
            } => {
                let value_ref = self.lower_expression(value, source, event_ids)?;
                self.emit_projection_write(
                    target,
                    value_ref,
                    ProjectionElem::FixedIndex(*field_index as u32),
                    source,
                    event_ids,
                )?;
            }
            HirExpressionKind::FallibleUnwrapSuccess { result }
            | HirExpressionKind::FallibleUnwrapError { result } => {
                let value_ref = self.lower_expression(result, source, event_ids)?;
                self.emit_projection_write(
                    target,
                    value_ref,
                    ProjectionElem::DynamicIndex,
                    source,
                    event_ids,
                )?;
            }
            HirExpressionKind::StructConstruct { .. }
            | HirExpressionKind::Collection(_)
            | HirExpressionKind::Range { .. }
            | HirExpressionKind::TupleConstruct { .. }
            | HirExpressionKind::VariantConstruct { .. }
            | HirExpressionKind::MapLiteral(_) => {
                self.lower_aggregate_into(target, value, source, event_ids)?;
            }
            _ => self.lower_fresh_value(Some(target), value, source, event_ids)?,
        }
        Ok(())
    }

    fn lower_fresh_value(
        &mut self,
        target: Option<PlaceId>,
        expression: &HirExpression,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let Some(target) = target else {
            let _ = self.lower_expression(expression, source, event_ids)?;
            return Ok(());
        };
        if is_aggregate_expression(expression) {
            return self.lower_aggregate_into(target, expression, source, event_ids);
        }
        match &expression.kind {
            HirExpressionKind::Load(_) | HirExpressionKind::Copy(_) => {
                let _ = self.lower_expression(expression, source, event_ids)?;
            }
            _ => {
                let _ = self.lower_expression_children(expression, source, event_ids)?;
            }
        }
        let origin = self.new_fresh_origin();
        self.emit_write(target, source, event_ids)?;
        if self.written_places.insert(target) {
            self.emit_event(
                event_ids,
                source.clone(),
                EventKind::Fresh {
                    destination: target,
                    origin,
                },
            );
        } else {
            self.emit_event(
                event_ids,
                source.clone(),
                EventKind::Rebind {
                    destination: target,
                    value: RebindValue::Fresh(origin),
                },
            );
        }
        Ok(())
    }

    fn lower_aggregate_into(
        &mut self,
        target: PlaceId,
        expression: &HirExpression,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let fields = self.lower_aggregate_children(expression, source, event_ids)?;
        let origin = self.new_fresh_origin();
        self.emit_write(target, source, event_ids)?;
        if self.written_places.insert(target) {
            self.emit_event(
                event_ids,
                source.clone(),
                EventKind::Aggregate {
                    destination: target,
                    origin,
                    fields: fields.into_boxed_slice(),
                },
            );
        } else {
            self.emit_event(
                event_ids,
                source.clone(),
                EventKind::Rebind {
                    destination: target,
                    value: RebindValue::Fresh(origin),
                },
            );
        }
        Ok(())
    }

    fn lower_aggregate_children(
        &mut self,
        expression: &HirExpression,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<Vec<AggregateField>, CompilerError> {
        let mut fields = Vec::new();
        match &expression.kind {
            HirExpressionKind::StructConstruct { fields: values, .. } => {
                for (field, value) in values {
                    let value_ref = self.lower_expression(value, source, event_ids)?;
                    if let Some(source) = value_ref.place {
                        fields.push(AggregateField {
                            projection: ProjectionElem::Field(field.0),
                            source,
                        });
                    }
                }
            }
            HirExpressionKind::Collection(values)
            | HirExpressionKind::TupleConstruct { elements: values } => {
                for (index, value) in values.iter().enumerate() {
                    let value_ref = self.lower_expression(value, source, event_ids)?;
                    if let Some(source) = value_ref.place {
                        fields.push(AggregateField {
                            projection: ProjectionElem::FixedIndex(index as u32),
                            source,
                        });
                    }
                }
            }
            HirExpressionKind::Range { start, end } => {
                for (index, value) in [start.as_ref(), end.as_ref()].into_iter().enumerate() {
                    let value_ref = self.lower_expression(value, source, event_ids)?;
                    if let Some(source) = value_ref.place {
                        fields.push(AggregateField {
                            projection: ProjectionElem::FixedIndex(index as u32),
                            source,
                        });
                    }
                }
            }
            HirExpressionKind::VariantConstruct { fields: values, .. } => {
                for (index, value) in values.iter().enumerate() {
                    let value_ref = self.lower_expression(&value.value, source, event_ids)?;
                    if let Some(source) = value_ref.place {
                        fields.push(AggregateField {
                            projection: ProjectionElem::FixedIndex(index as u32),
                            source,
                        });
                    }
                }
            }
            HirExpressionKind::MapLiteral(entries) => {
                for entry in entries {
                    let key = self.lower_expression(&entry.key, source, event_ids)?;
                    if let Some(source) = key.place {
                        fields.push(AggregateField {
                            projection: ProjectionElem::MapEntry,
                            source,
                        });
                    }
                    let value = self.lower_expression(&entry.value, source, event_ids)?;
                    if let Some(source) = value.place {
                        fields.push(AggregateField {
                            projection: ProjectionElem::MapEntry,
                            source,
                        });
                    }
                }
            }
            _ => {}
        }
        Ok(fields)
    }

    fn lower_expression_children(
        &mut self,
        expression: &HirExpression,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<Vec<ValueRef>, CompilerError> {
        let mut values = Vec::new();
        match &expression.kind {
            HirExpressionKind::BinOp { left, right, .. } => {
                values.push(self.lower_expression(left, source, event_ids)?);
                values.push(self.lower_expression(right, source, event_ids)?);
            }
            HirExpressionKind::UnaryOp { operand, .. }
            | HirExpressionKind::Cast {
                source: operand, ..
            } => {
                values.push(self.lower_expression(operand, source, event_ids)?);
            }
            HirExpressionKind::StructConstruct { fields, .. } => {
                for (_, value) in fields {
                    values.push(self.lower_expression(value, source, event_ids)?);
                }
            }
            HirExpressionKind::Collection(elements)
            | HirExpressionKind::TupleConstruct { elements } => {
                for element in elements {
                    values.push(self.lower_expression(element, source, event_ids)?);
                }
            }
            HirExpressionKind::Range { start, end } => {
                values.push(self.lower_expression(start, source, event_ids)?);
                values.push(self.lower_expression(end, source, event_ids)?);
            }
            HirExpressionKind::TupleGet { tuple, .. }
            | HirExpressionKind::FallibleUnwrapSuccess { result: tuple }
            | HirExpressionKind::FallibleUnwrapError { result: tuple }
            | HirExpressionKind::VariantPayloadGet { source: tuple, .. } => {
                values.push(self.lower_expression(tuple, source, event_ids)?);
            }
            HirExpressionKind::VariantConstruct { fields, .. } => {
                for field in fields {
                    values.push(self.lower_expression(&field.value, source, event_ids)?);
                }
            }
            HirExpressionKind::MapLiteral(entries) => {
                for entry in entries {
                    values.push(self.lower_expression(&entry.key, source, event_ids)?);
                    values.push(self.lower_expression(&entry.value, source, event_ids)?);
                }
            }
            HirExpressionKind::Load(_) | HirExpressionKind::Copy(_) => {}
            HirExpressionKind::Int(_)
            | HirExpressionKind::Float(_)
            | HirExpressionKind::Bool(_)
            | HirExpressionKind::Char(_)
            | HirExpressionKind::StringLiteral(_) => {}
        }
        Ok(values)
    }

    fn lower_expression(
        &mut self,
        expression: &HirExpression,
        fallback_source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<ValueRef, CompilerError> {
        let source = self.value_source(expression, fallback_source);
        self.emit_reactive_observations(expression, &source, event_ids)?;
        match &expression.kind {
            HirExpressionKind::Load(place) => {
                let place = self.lower_place(place, &source, event_ids)?;
                self.emit_read(place, &source, event_ids)?;
                Ok(ValueRef { place: Some(place) })
            }
            HirExpressionKind::Copy(place) => {
                let source_place = self.lower_place(place, &source, event_ids)?;
                self.emit_read(source_place, &source, event_ids)?;
                let destination = self.synthetic_place(expression.id.0, &source)?;
                let origin = self.new_copy_origin();
                self.emit_write(destination, &source, event_ids)?;
                self.emit_event(
                    event_ids,
                    source,
                    EventKind::Copy {
                        source: source_place,
                        destination,
                        origin,
                    },
                );
                Ok(ValueRef {
                    place: Some(destination),
                })
            }
            HirExpressionKind::Int(_)
            | HirExpressionKind::Float(_)
            | HirExpressionKind::Bool(_)
            | HirExpressionKind::Char(_)
            | HirExpressionKind::StringLiteral(_) => {
                if expression.value_kind == ValueKind::Const {
                    return Ok(ValueRef { place: None });
                }
                let destination = self.synthetic_place(expression.id.0, &source)?;
                let origin = self.new_fresh_origin();
                self.emit_write(destination, &source, event_ids)?;
                self.emit_event(
                    event_ids,
                    source,
                    EventKind::Fresh {
                        destination,
                        origin,
                    },
                );
                Ok(ValueRef {
                    place: Some(destination),
                })
            }
            HirExpressionKind::TupleGet { tuple, index } => {
                let source_ref = self.lower_expression(tuple, &source, event_ids)?;
                self.expression_projection(
                    source_ref,
                    ProjectionElem::FixedIndex(*index as u32),
                    &source,
                    event_ids,
                )
            }
            HirExpressionKind::VariantPayloadGet {
                source: value,
                field_index,
                ..
            } => {
                let source_ref = self.lower_expression(value, &source, event_ids)?;
                self.expression_projection(
                    source_ref,
                    ProjectionElem::FixedIndex(*field_index as u32),
                    &source,
                    event_ids,
                )
            }
            HirExpressionKind::FallibleUnwrapSuccess { result }
            | HirExpressionKind::FallibleUnwrapError { result } => {
                let source_ref = self.lower_expression(result, &source, event_ids)?;
                self.expression_projection(
                    source_ref,
                    ProjectionElem::DynamicIndex,
                    &source,
                    event_ids,
                )
            }
            HirExpressionKind::StructConstruct { .. }
            | HirExpressionKind::Collection(_)
            | HirExpressionKind::Range { .. }
            | HirExpressionKind::TupleConstruct { .. }
            | HirExpressionKind::VariantConstruct { .. }
            | HirExpressionKind::MapLiteral(_) => {
                let destination = self.synthetic_place(expression.id.0, &source)?;
                self.lower_aggregate_into(destination, expression, &source, event_ids)?;
                Ok(ValueRef {
                    place: Some(destination),
                })
            }
            _ => {
                let _ = self.lower_expression_children(expression, &source, event_ids)?;
                let destination = self.synthetic_place(expression.id.0, &source)?;
                let origin = self.new_fresh_origin();
                self.emit_write(destination, &source, event_ids)?;
                self.emit_event(
                    event_ids,
                    source,
                    EventKind::Fresh {
                        destination,
                        origin,
                    },
                );
                Ok(ValueRef {
                    place: Some(destination),
                })
            }
        }
    }

    fn emit_reactive_observations(
        &mut self,
        expression: &HirExpression,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let Some(template) = self
            .module
            .side_table
            .reactive_template_for_value(expression.id)
        else {
            return Ok(());
        };
        if !template.has_runtime_reactive_dependency() {
            return Ok(());
        }

        let mut locals = BTreeSet::new();
        for dependency in &template.dependencies {
            let reactive_source = self
                .module
                .side_table
                .reactive_source(dependency.source)
                .ok_or_else(|| {
                    compiler_error(format!(
                        "Boracle problem extraction cannot resolve reactive source {:?}",
                        dependency.source
                    ))
                })?;
            locals.insert(reactive_source.local_id.0);
        }
        locals.extend(
            template
                .template_value_parameters
                .iter()
                .map(|dependency| dependency.parameter.0),
        );

        for local in locals {
            let place = self.local_place(LocalId(local), source)?;
            self.emit_event(
                event_ids,
                source.clone(),
                EventKind::ReactiveObserve { place },
            );
        }
        Ok(())
    }

    fn expression_projection(
        &mut self,
        source_ref: ValueRef,
        projection: ProjectionElem,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<ValueRef, CompilerError> {
        let Some(source_place) = source_ref.place else {
            return Ok(ValueRef { place: None });
        };
        let destination = self.project_place(source_place, projection)?;
        let origin = self.new_projection_origin(projection);
        self.emit_write(destination, source, event_ids)?;
        self.emit_event(
            event_ids,
            source.clone(),
            EventKind::Projection {
                source: source_place,
                destination,
                origin,
            },
        );
        Ok(ValueRef {
            place: Some(destination),
        })
    }

    fn emit_projection_write(
        &mut self,
        target: PlaceId,
        source_ref: ValueRef,
        projection: ProjectionElem,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let Some(source_place) = source_ref.place else {
            return self.emit_fresh_write(target, source, event_ids);
        };
        let origin = self.new_projection_origin(projection);
        self.emit_write(target, source, event_ids)?;
        self.emit_event(
            event_ids,
            source.clone(),
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
                self.lower_expression(operand, source, event_ids)?;
            }
            crate::compiler_frontend::hir::numeric::HirNumericOperands::Binary { left, right } => {
                self.lower_expression(left, source, event_ids)?;
                self.lower_expression(right, source, event_ids)?;
            }
        }
        Ok(())
    }

    fn lower_call(
        &mut self,
        target: &CallTarget,
        args: &[HirExpression],
        result: Option<LocalId>,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let arguments = self.lower_call_arguments(args, source, event_ids)?;
        let (accesses, provenance) = self.call_effect(target, args.len())?;
        self.emit_call_effect_with_label(
            CallEffectSpec {
                label: format!("{target:?}"),
                arguments,
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
        receiver: &HirExpression,
        args: &[HirExpression],
        result: Option<LocalId>,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let mut expressions = Vec::with_capacity(args.len() + 1);
        expressions.push(receiver.clone());
        expressions.extend(args.iter().cloned());
        let arguments = self.lower_call_arguments(&expressions, source, event_ids)?;
        let receiver_access = if op.requires_mutable_receiver() {
            AccessKind::Exclusive
        } else {
            AccessKind::Shared
        };
        let mut accesses = vec![receiver_access];
        accesses.extend(std::iter::repeat_n(AccessKind::Shared, args.len()));
        let provenance = match op {
            HirMapOp::Get => CallResultProvenance::AliasParams(vec![0].into_boxed_slice()),
            HirMapOp::Remove => CallResultProvenance::Fresh,
            HirMapOp::Contains | HirMapOp::Set | HirMapOp::Clear | HirMapOp::Length => {
                CallResultProvenance::Fresh
            }
        };
        let label = format!("map::{op:?}");
        self.emit_call_effect_with_label(
            CallEffectSpec {
                label,
                arguments,
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
        args: &[HirExpression],
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<Vec<PlaceId>, CompilerError> {
        args.iter()
            .map(|argument| {
                let value = match &argument.kind {
                    HirExpressionKind::Load(place) => ValueRef {
                        place: Some(self.lower_place(place, source, event_ids)?),
                    },
                    _ => self.lower_expression(argument, source, event_ids)?,
                };
                self.materialize_value_place(value, argument, source, event_ids)
            })
            .collect()
    }

    fn emit_call_effect_with_label(
        &mut self,
        spec: CallEffectSpec<'_>,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        if spec.arguments.len() != spec.accesses.len() {
            return Err(compiler_error(
                "Boracle problem extraction produced mismatched call argument metadata",
            ));
        }
        let CallEffectSpec {
            label,
            arguments,
            accesses,
            provenance,
            result: result_local,
            source,
        } = spec;
        let call_id = self.next_call_id()?;
        self.calls.push(Call { id: call_id, label });
        let point = self.new_point(self.current_problem_block()?, source.clone());
        let mut call_arguments = Vec::with_capacity(arguments.len());
        for (place, access) in arguments.into_iter().zip(accesses) {
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
            });
            call_arguments.push(CallArgument {
                place,
                access,
                use_id,
            });
        }
        let result = result_local
            .map(|local| self.local_place(local, source))
            .transpose()?;
        let call_result = result.map(|place| {
            let origin = self.new_origin(OriginKind::CallResult {
                call: call_id,
                provenance,
            });
            CallResult { place, origin }
        });
        let event_id = self.next_event_id()?;
        self.events.push(Event::new(
            event_id,
            point,
            EventKind::CallEffect(CallEffect {
                call: call_id,
                arguments: call_arguments.into_boxed_slice(),
                result: call_result,
            }),
            source.clone(),
        ));
        event_ids.push(event_id);
        if let Some(result) = result {
            let use_id = self.next_use_id()?;
            self.uses.push(Use {
                id: use_id,
                point,
                place: result,
                kind: UseKind::Write,
            });
            let event_id = self.next_event_id()?;
            self.events.push(Event::new(
                event_id,
                point,
                EventKind::Access { use_id },
                spec.source.clone(),
            ));
            event_ids.push(event_id);
        }
        Ok(())
    }

    fn lower_terminator(
        &mut self,
        terminator: &HirTerminator,
        block_id: HirBlockId,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let source = self.terminator_source(block_id);
        let kind = match terminator {
            HirTerminator::Jump { target, .. } => TerminatorEventKind::Jump {
                target: self.problem_block(*target)?,
            },
            HirTerminator::If {
                condition,
                then_block,
                else_block,
            } => {
                self.lower_expression(condition, &source, event_ids)?;
                TerminatorEventKind::Branch {
                    targets: self.sorted_problem_targets([*then_block, *else_block].into_iter())?,
                }
            }
            HirTerminator::FallibleBranch {
                result,
                success_block,
                error_block,
            } => {
                self.lower_expression(result, &source, event_ids)?;
                TerminatorEventKind::Branch {
                    targets: self
                        .sorted_problem_targets([*success_block, *error_block].into_iter())?,
                }
            }
            HirTerminator::Match { scrutinee, arms } => {
                self.lower_expression(scrutinee, &source, event_ids)?;
                for arm in arms {
                    self.lower_match_arm(arm, &source, event_ids)?;
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
                self.lower_expression(value, &source, event_ids)?;
                TerminatorEventKind::Return
            }
            HirTerminator::ReturnSuccess(value) => {
                self.lower_expression(value, &source, event_ids)?;
                TerminatorEventKind::ReturnSuccess
            }
            HirTerminator::ReturnError(value) => {
                self.lower_expression(value, &source, event_ids)?;
                TerminatorEventKind::ReturnError
            }
            HirTerminator::RuntimeFailure { .. } => TerminatorEventKind::RuntimeFailure,
            HirTerminator::AssertFailure { message, .. } => {
                self.lower_expression(message, &source, event_ids)?;
                TerminatorEventKind::AssertFailure
            }
            HirTerminator::Uninitialized => {
                return Err(compiler_error(format!(
                    "Boracle problem extraction reached uninitialized HIR terminator in block {:?}",
                    block_id
                )));
            }
        };
        self.emit_event(event_ids, source, EventKind::Terminator { kind });
        Ok(())
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
                self.lower_expression(value, source, event_ids)?;
            }
            HirPattern::OptionNone
            | HirPattern::OptionPresent
            | HirPattern::Wildcard
            | HirPattern::ChoiceVariant { .. } => {}
        }
        if let Some(guard) = &arm.guard {
            self.lower_expression(guard, source, event_ids)?;
        }
        Ok(())
    }

    fn emit_scope_exit_events(
        &mut self,
        block: &HirBlock,
        successors: &[HirBlockId],
        event_ids: &mut Vec<EventId>,
        source: &EventSource,
    ) -> Result<(), CompilerError> {
        let mut bindings = Vec::new();
        for local in &block.locals {
            let binding = self.binding_for_local(local.id)?;
            let survives_successor = successors.iter().any(|successor| {
                self.block_by_id
                    .get(&successor.0)
                    .is_some_and(|next| self.region_contains(local.region, next.region))
            });
            if successors.is_empty() || !survives_successor {
                bindings.push(binding);
            }
        }
        bindings.sort_by_key(|binding| binding.raw());
        bindings.dedup();
        if !bindings.is_empty() {
            self.emit_event(
                event_ids,
                source.clone(),
                EventKind::ScopeExit {
                    bindings: bindings.into_boxed_slice(),
                },
            );
        }
        Ok(())
    }

    fn lower_place(
        &mut self,
        place: &HirPlace,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<PlaceId, CompilerError> {
        match place {
            HirPlace::Local(local) => self.local_place(*local, source),
            HirPlace::Field { base, field } => {
                let base = self.lower_place(base, source, event_ids)?;
                self.project_place(base, ProjectionElem::Field(field.0))
            }
            HirPlace::Index { base, index } => {
                let base = self.lower_place(base, source, event_ids)?;
                let projection = match index.kind {
                    HirExpressionKind::Int(value) if value >= 0 => {
                        ProjectionElem::FixedIndex(value as u32)
                    }
                    HirExpressionKind::Int(_) => ProjectionElem::DynamicIndex,
                    _ => {
                        self.lower_expression(index, source, event_ids)?;
                        ProjectionElem::DynamicIndex
                    }
                };
                self.project_place(base, projection)
            }
        }
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

    fn synthetic_place(
        &mut self,
        value_id: u32,
        source: &EventSource,
    ) -> Result<PlaceId, CompilerError> {
        let binding = if let Some(binding) = self.synthetic_binding_by_value.get(&value_id) {
            *binding
        } else {
            let binding = BindingId::new(dense_u32(
                self.bindings.len(),
                "synthetic normalized binding",
            )?);
            self.bindings
                .push(Binding::new(binding, None, None, source.clone()));
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
        expression: &HirExpression,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<PlaceId, CompilerError> {
        if let Some(place) = value.place {
            return Ok(place);
        }
        let place = self.synthetic_place(expression.id.0, source)?;
        let origin = self.new_fresh_origin();
        self.emit_write(place, source, event_ids)?;
        self.emit_event(
            event_ids,
            source.clone(),
            EventKind::Fresh {
                destination: place,
                origin,
            },
        );
        Ok(place)
    }

    fn emit_alias_write(
        &mut self,
        destination: PlaceId,
        source_place: PlaceId,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        self.emit_write(destination, source, event_ids)?;
        let event = if self.written_places.insert(destination) {
            EventKind::AliasFromPlace {
                source: source_place,
                destination,
            }
        } else {
            EventKind::Rebind {
                destination,
                value: RebindValue::AliasFromPlace(source_place),
            }
        };
        self.emit_event(event_ids, source.clone(), event);
        Ok(())
    }

    fn emit_fresh_write(
        &mut self,
        destination: PlaceId,
        source: &EventSource,
        event_ids: &mut Vec<EventId>,
    ) -> Result<(), CompilerError> {
        let origin = self.new_fresh_origin();
        self.emit_write(destination, source, event_ids)?;
        let event = if self.written_places.insert(destination) {
            EventKind::Fresh {
                destination,
                origin,
            }
        } else {
            EventKind::Rebind {
                destination,
                value: RebindValue::Fresh(origin),
            }
        };
        self.emit_event(event_ids, source.clone(), event);
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
        let point = self.new_point(self.current_problem_block()?, source.clone());
        let use_id = self.next_use_id()?;
        self.uses.push(Use {
            id: use_id,
            point,
            place,
            kind,
        });
        let event_id = self.next_event_id()?;
        self.events.push(Event::new(
            event_id,
            point,
            EventKind::Access { use_id },
            source.clone(),
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
            source.clone(),
        );
        let event_id = EventId::new(self.events.len() as u32);
        self.events.push(Event::new(event_id, point, kind, source));
        event_ids.push(event_id);
        event_id
    }

    fn call_effect(
        &self,
        target: &CallTarget,
        argument_count: usize,
    ) -> Result<(Vec<AccessKind>, CallResultProvenance), CompilerError> {
        if let CallTarget::External(id) = target {
            let Some(registry) = self.external_registry else {
                return Ok((
                    vec![AccessKind::Shared; argument_count],
                    CallResultProvenance::Unknown,
                ));
            };
            let Some(definition) = registry.get_function_by_id(*id) else {
                return Err(compiler_error(format!(
                    "Boracle problem extraction cannot resolve external call {:?}",
                    id
                )));
            };
            let accesses = definition
                .parameters
                .iter()
                .map(|parameter| match parameter.access_kind {
                    ExternalAccessKind::Shared => AccessKind::Shared,
                    ExternalAccessKind::Mutable => AccessKind::Exclusive,
                })
                .collect::<Vec<_>>();
            if accesses.len() != argument_count {
                return Err(compiler_error(format!(
                    "Boracle external call {:?} has {} arguments but metadata has {} parameters",
                    id,
                    argument_count,
                    accesses.len()
                )));
            }
            let provenance = match definition.hir_return_alias() {
                ExternalReturnAlias::Fresh => CallResultProvenance::Fresh,
                ExternalReturnAlias::AliasArgs(indices) => CallResultProvenance::AliasParams(
                    indices.into_iter().collect::<Vec<_>>().into_boxed_slice(),
                ),
            };
            return Ok((accesses, provenance));
        }

        let summary = match target {
            CallTarget::Local(id) => self.local_summaries.and_then(|summaries| summaries.get(id)),
            CallTarget::CrossModule(id) => self.module.imported_call_summaries.get(id),
            CallTarget::ModulePrivate(id) => self.module.module_private_call_summaries.get(id),
            CallTarget::Generated(id) => self.module.generated_call_summaries.get(id),
            CallTarget::External(_) => None,
        };
        let Some(summary) = summary else {
            return Ok((
                vec![AccessKind::Shared; argument_count],
                CallResultProvenance::Unknown,
            ));
        };
        if summary.parameters.len() != argument_count {
            return Err(compiler_error(format!(
                "Boracle call summary for {target:?} has {} parameters but call has {argument_count} arguments",
                summary.parameters.len()
            )));
        }
        let accesses = summary
            .parameters
            .iter()
            .map(|parameter| match parameter.access {
                PublicCallParameterAccess::Mutable => AccessKind::Exclusive,
                PublicCallParameterAccess::Shared | PublicCallParameterAccess::Reactive => {
                    AccessKind::Shared
                }
            })
            .collect();
        let provenance = match &summary.return_alias {
            FunctionReturnAliasSummary::Fresh => CallResultProvenance::Fresh,
            FunctionReturnAliasSummary::AliasParams(indices) => {
                CallResultProvenance::AliasParams(indices.iter().copied().collect())
            }
            FunctionReturnAliasSummary::Unknown => CallResultProvenance::Unknown,
        };
        Ok((accesses, provenance))
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
            location: self
                .module
                .side_table
                .hir_source_location_for_hir(HirLocation::Block(block))
                .cloned(),
        }
    }

    fn terminator_source(&self, block: HirBlockId) -> EventSource {
        EventSource {
            hir_node: None,
            location: self
                .module
                .side_table
                .hir_source_location_for_hir(HirLocation::Terminator(block))
                .cloned(),
        }
    }

    fn value_source(&self, expression: &HirExpression, fallback: &EventSource) -> EventSource {
        EventSource {
            hir_node: None,
            location: self
                .module
                .side_table
                .value_source_location(expression.id)
                .cloned()
                .or_else(|| fallback.location.clone()),
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

fn dense_id(index: usize, owner: &str) -> Result<BlockId, CompilerError> {
    Ok(BlockId::new(dense_u32(index, owner)?))
}

fn dense_u32(index: usize, owner: &str) -> Result<u32, CompilerError> {
    u32::try_from(index)
        .map_err(|_| compiler_error(format!("{owner} table is larger than u32::MAX rows")))
}

fn is_aggregate_expression(expression: &HirExpression) -> bool {
    matches!(
        expression.kind,
        HirExpressionKind::StructConstruct { .. }
            | HirExpressionKind::Collection(_)
            | HirExpressionKind::Range { .. }
            | HirExpressionKind::TupleConstruct { .. }
            | HirExpressionKind::VariantConstruct { .. }
            | HirExpressionKind::MapLiteral(_)
    )
}

fn compiler_error(message: impl Into<String>) -> CompilerError {
    CompilerError::compiler_error(message)
}
