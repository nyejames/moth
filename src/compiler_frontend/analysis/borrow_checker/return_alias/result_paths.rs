//! Unconditional heap result paths proven by reachable local HIR.
//!
//! WHY: a constructor owns its root but can retain an argument at a field. The older
//! root-only call summary cannot name both identities without conflating their layouts.

use super::{BorrowCheckError, BorrowChecker};
use crate::compiler_frontend::analysis::borrow_checker::state::FieldKey;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalAccessKind};
use crate::compiler_frontend::hir::expression_store::{HirProjection, HirValueRange};
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;
use crate::compiler_frontend::hir::ids::{FunctionId, HirValueId, LocalId};
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::public_call_summary::PublicCallParameterAccess;
use rustc_hash::{FxHashMap, FxHashSet};

#[cfg(test)]
#[path = "result_paths_tests.rs"]
mod result_paths_tests;

/// One success result's unconditional allocation identity, in authored slot order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::compiler_frontend::analysis::borrow_checker) enum CallResultFacts {
    Parameter {
        index: usize,
        path: Vec<FieldKey>,
    },
    Fresh {
        children: Vec<(FieldKey, CallResultFacts)>,
    },
}

#[derive(Clone, Copy)]
enum Writer {
    Value(HirValueId),
    Transport(LocalId),
    Unproven,
}

/// Pending projections, borrowed from the validated loads that contributed them.
#[derive(Clone, Copy)]
enum Path<'a> {
    End,
    Place(&'a [HirProjection], &'a Self),
    Payload(FieldKey, &'a Self),
}

impl<'a> Path<'a> {
    fn step(self) -> Option<(FieldKey, Self)> {
        match self {
            Self::End => None,
            Self::Place([HirProjection::Field(field), remaining @ ..], tail) => Some((
                FieldKey::Field(field.0),
                if remaining.is_empty() {
                    *tail
                } else {
                    Self::Place(remaining, tail)
                },
            )),
            Self::Place(..) => None, // An index is not one exact allocation path.
            Self::Payload(key, tail) => Some((key, *tail)),
        }
    }
}

#[derive(Default)]
struct ExitEvidence {
    visiting: FxHashSet<LocalId>,
    fresh: FxHashSet<HirValueId>,
    repeated_fresh: bool,
}

struct FunctionEvidence<'a, 'module> {
    checker: &'a BorrowChecker<'module>,
    locals: FxHashMap<LocalId, (TypeId, bool)>,
    writers: FxHashMap<LocalId, Writer>,
    params: FxHashMap<LocalId, usize>,
}

impl BorrowChecker<'_> {
    pub(in crate::compiler_frontend::analysis::borrow_checker) fn build_typed_local_results(
        &self,
    ) -> Result<FxHashMap<FunctionId, Vec<Option<CallResultFacts>>>, BorrowCheckError> {
        self.module
            .functions
            .iter()
            .map(|function| Ok((function.id, self.typed_function_results(function)?)))
            .collect()
    }

    fn typed_function_results(
        &self,
        function: &HirFunction,
    ) -> Result<Vec<Option<CallResultFacts>>, BorrowCheckError> {
        let Some(shape) = function.return_contract.ordinary() else {
            return Ok(Vec::new());
        };
        let mut facts = vec![None; shape.success().len()];
        let mut heap_slots = Vec::new();
        for (slot, &ty) in shape.success().iter().enumerate() {
            if self.result_type_is_heap(ty)? {
                heap_slots.push(slot);
            }
        }
        if heap_slots.is_empty() {
            return Ok(facts);
        }

        let blocks = self.collect_reachable_blocks(function)?;
        let mut evidence = FunctionEvidence {
            checker: self,
            locals: FxHashMap::default(),
            writers: FxHashMap::default(),
            params: function
                .params
                .iter()
                .copied()
                .enumerate()
                .map(|(index, local)| (local, index))
                .collect(),
        };
        for &block_id in &blocks {
            let block = self.block_by_id_or_error(block_id, function.id)?;
            evidence.locals.extend(
                block
                    .locals
                    .iter()
                    .map(|local| (local.id, (local.ty, local.mutable))),
            );
        }
        let mut returns = Vec::new();
        for block_id in blocks {
            let block = self.block_by_id_or_error(block_id, function.id)?;
            for statement in &block.statements {
                if !evidence.record_statement(&statement.kind)? {
                    return Ok(facts);
                }
            }
            match &block.terminator {
                HirTerminator::Return(values) => returns.push(*values),
                HirTerminator::Jump { args, .. } => {
                    for arg in args {
                        evidence.record_writer(arg.destination, Writer::Transport(arg.source));
                    }
                }
                HirTerminator::Invoke {
                    operation,
                    success_results,
                    error_result,
                    ..
                } => {
                    match operation {
                        HirFallibleOperation::Call { target, args }
                            if self.call_may_change_paths(target, *args)? =>
                        {
                            return Ok(facts);
                        }
                        HirFallibleOperation::MapOp { op, .. }
                            if op.requires_mutable_receiver() =>
                        {
                            return Ok(facts);
                        }
                        _ => {}
                    }
                    for &result in success_results.iter().chain(std::iter::once(error_result)) {
                        evidence.record_writer(result, Writer::Unproven);
                    }
                }
                _ => {}
            }
        }

        let mut exit = ExitEvidence::default();
        let mut repeated_fresh = false;
        for (return_index, values) in returns.into_iter().enumerate() {
            exit.fresh.clear();
            exit.repeated_fresh = false;
            let values = self.module.expressions.values(values);
            // HIR validation guarantees the shape, types, and definite definitions of these reads.
            for &slot in &heap_slots {
                if return_index != 0 && facts[slot].is_none() {
                    continue;
                }
                let candidate = if let Some(&value) = values.get(slot)
                    && self.module.expressions.expression(value).ty == shape.success()[slot]
                {
                    evidence.value(value, Path::End, false, &mut exit)?
                } else {
                    None
                };
                if return_index == 0 {
                    facts[slot] = candidate;
                } else if facts[slot] != candidate {
                    facts[slot] = None;
                }
            }
            repeated_fresh |= exit.repeated_fresh;
        }

        // The tree format names each Fresh node independently. Never split one allocation
        // that appears twice, or assume it is disjoint from an unproven sibling result.
        if repeated_fresh || heap_slots.iter().any(|&slot| facts[slot].is_none()) {
            for slot in &mut facts {
                if matches!(slot, Some(CallResultFacts::Fresh { .. })) {
                    *slot = None;
                }
            }
        }
        Ok(facts)
    }

    fn result_type_is_heap(&self, ty: TypeId) -> Result<bool, BorrowCheckError> {
        Ok(classify_hir_type(ty, self.type_environment)?.has_in_place_mutation_surface())
    }

    fn call_may_change_paths(
        &self,
        target: &CallTarget,
        args: HirValueRange,
    ) -> Result<bool, BorrowCheckError> {
        let summary = match target {
            CallTarget::Local(id) => self.public_call_summaries.get(id),
            CallTarget::CrossModule(id) => self.module.imported_call_summaries.get(id),
            CallTarget::ModulePrivate(id) => self.module.module_private_call_summaries.get(id),
            CallTarget::Generated(id) => self.module.generated_call_summaries.get(id.as_ref()),
            CallTarget::External(_) => None,
        };
        let external = match target {
            CallTarget::External(id) => self.external_package_registry.get_function_by_id(*id),
            _ => None,
        };
        for (index, &argument) in self.module.expressions.values(args).iter().enumerate() {
            let expression = self.module.expressions.expression(argument);
            let shared = summary
                .and_then(|summary| summary.parameters.get(index))
                .is_some_and(|param| param.access == PublicCallParameterAccess::Shared)
                || external
                    .and_then(|def| def.parameters.get(index))
                    .is_some_and(|param| param.access_kind == ExternalAccessKind::Shared);
            if !shared
                && (self.result_type_is_heap(expression.ty)?
                    || matches!(&expression.kind, HirExpressionKind::Load(place) if !place.projections.is_empty()))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Copy detaches from its source, but can preserve sharing *inside* the copied graph.
    /// Only copy evidence with no heap selectors can be represented without that topology.
    fn copy_has_no_heap_children(&self, ty: TypeId) -> Result<bool, BorrowCheckError> {
        if let Some(element) = self.type_environment.collection_element_type(ty) {
            return Ok(!self.result_type_is_heap(element)?);
        }
        if let Some(map) = self.type_environment.map_shape(ty) {
            return self.all_scalar([map.key_type, map.value_type]);
        }
        if let Some(layout) = self
            .module
            .structs
            .iter()
            .find(|layout| layout.frontend_type_id == ty)
        {
            return self.all_scalar(layout.fields.iter().map(|field| field.ty));
        }
        Ok(false)
    }

    fn all_scalar(
        &self,
        types: impl IntoIterator<Item = TypeId>,
    ) -> Result<bool, BorrowCheckError> {
        for ty in types {
            if self.result_type_is_heap(ty)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl FunctionEvidence<'_, '_> {
    fn record_writer(&mut self, local: LocalId, writer: Writer) {
        self.writers
            .entry(local)
            .and_modify(|writer| *writer = Writer::Unproven)
            .or_insert(writer);
    }

    fn record_destination(
        &mut self,
        destination: HirLocalDestination,
    ) -> Result<bool, BorrowCheckError> {
        let local = destination.local();
        if matches!(destination, HirLocalDestination::Update(_)) && self.may_write_heap(local)? {
            return Ok(false);
        }
        self.record_writer(local, Writer::Unproven);
        Ok(true)
    }

    fn may_write_heap(&self, local: LocalId) -> Result<bool, BorrowCheckError> {
        let Some(&(ty, _)) = self.locals.get(&local) else {
            return Ok(true);
        };
        self.checker.result_type_is_heap(ty)
    }

    /// Reject the function's candidate paths if a write could change their storage through an
    /// alias. Tracking which paths survive such a write would require a store-effect analysis.
    fn record_statement(&mut self, statement: &HirStatementKind) -> Result<bool, BorrowCheckError> {
        match statement {
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                value,
            } => {
                self.record_writer(*local, Writer::Value(*value));
            }
            HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(place),
                ..
            } => {
                if !place.projections.is_empty() || self.may_write_heap(place.root)? {
                    return Ok(false);
                }
                self.record_writer(place.root, Writer::Unproven);
            }
            HirStatementKind::Call {
                target,
                args,
                results,
            } => {
                if self.checker.call_may_change_paths(target, *args)? {
                    return Ok(false);
                }
                for &result in results {
                    if !self.record_destination(result)? {
                        return Ok(false);
                    }
                }
            }
            HirStatementKind::MapOp { op, result, .. } => {
                if op.requires_mutable_receiver() {
                    return Ok(false);
                }
                if let Some(result) = result {
                    return self.record_destination(*result);
                }
            }
            HirStatementKind::NumericOp { result, .. }
            | HirStatementKind::FastStoreConversion { result, .. }
            | HirStatementKind::LoopStepFailure { result, .. }
            | HirStatementKind::FormatFloat { result, .. }
            | HirStatementKind::ValidateFloat { result, .. } => {
                return self.record_destination(*result);
            }
            HirStatementKind::RangeCandidate {
                candidate_result,
                in_range_result,
                ..
            } => {
                return Ok(self.record_destination(*candidate_result)?
                    && self.record_destination(*in_range_result)?);
            }
            HirStatementKind::PushRuntimeFragment { .. } => return Ok(false),
            HirStatementKind::Expr(_) => {}
        }
        Ok(true)
    }

    fn local(
        &self,
        local: LocalId,
        mut path: Path<'_>,
        retained: bool,
        exit: &mut ExitEvidence,
    ) -> Result<Option<CallResultFacts>, BorrowCheckError> {
        let writer = self.writers.get(&local);
        if let Some(&index) = self.params.get(&local) {
            if writer.is_some() {
                return Ok(None);
            }
            let mut keys = Vec::new();
            while !matches!(path, Path::End) {
                let Some((key, tail)) = path.step() else {
                    return Ok(None);
                };
                keys.push(key);
                path = tail;
            }
            return Ok(Some(CallResultFacts::Parameter { index, path: keys }));
        }
        // A retained load through a shared implementation local may narrow access beyond the
        // caller's actual parameter or the fresh origin. This tree has no permission override.
        if (retained && !self.locals.get(&local).is_some_and(|&(_, mutable)| mutable))
            || !exit.visiting.insert(local)
        {
            return Ok(None);
        }
        let result = match writer {
            Some(Writer::Value(value)) => self.value(*value, path, retained, exit),
            Some(Writer::Transport(source)) => self.local(*source, path, retained, exit),
            _ => Ok(None),
        };
        exit.visiting.remove(&local);
        result
    }

    fn value(
        &self,
        value: HirValueId,
        path: Path<'_>,
        retained: bool,
        exit: &mut ExitEvidence,
    ) -> Result<Option<CallResultFacts>, BorrowCheckError> {
        let expressions = &self.checker.module.expressions;
        let expression = expressions.expression(value);
        match &expression.kind {
            HirExpressionKind::Load(place) if expression.value_kind != ValueKind::Const => {
                let projections = expressions.projections(place.projections);
                let path = if projections.is_empty() {
                    path
                } else {
                    Path::Place(projections, &path)
                };
                self.local(place.root, path, retained, exit)
            }
            HirExpressionKind::VariantPayloadGet {
                source,
                variant_index,
                field_index,
                ..
            } => {
                let (Ok(variant), Ok(field)) =
                    (u32::try_from(*variant_index), u32::try_from(*field_index))
                else {
                    return Ok(None);
                };
                let key = FieldKey::Payload { variant, field };
                self.value(*source, Path::Payload(key, &path), retained, exit)
            }
            HirExpressionKind::StructConstruct { fields, .. } => self.construct(
                value,
                path,
                exit,
                expressions
                    .struct_fields(*fields)
                    .iter()
                    .map(|(field, value)| (FieldKey::Field(field.0), *value)),
            ),
            HirExpressionKind::VariantConstruct {
                variant_index,
                fields,
                ..
            }
            | HirExpressionKind::RouteConstruct {
                variant_index,
                fields,
                ..
            } => {
                let fields = expressions.variant_fields(*fields);
                let Ok(variant) = u32::try_from(*variant_index) else {
                    return Ok(None);
                };
                if u32::try_from(fields.len()).is_err() {
                    return Ok(None);
                }
                self.construct(
                    value,
                    path,
                    exit,
                    fields.iter().enumerate().map(|(index, field)| {
                        (
                            FieldKey::Payload {
                                variant,
                                field: index as u32,
                            },
                            field.value,
                        )
                    }),
                )
            }
            // Collection entries share a summary key, not one exact field. An explicit
            // construction is enough evidence only if it retains no heap entries.
            HirExpressionKind::Collection(values)
                if matches!(path, Path::End)
                    && self.checker.all_scalar(
                        expressions
                            .values(*values)
                            .iter()
                            .map(|id| expressions.expression(*id).ty),
                    )? =>
            {
                Ok(Some(fresh(value, Vec::new(), exit)))
            }
            HirExpressionKind::MapLiteral(entries)
                if matches!(path, Path::End)
                    && self.checker.all_scalar(
                        expressions.map_entries(*entries).iter().flat_map(|entry| {
                            [
                                expressions.expression(entry.key).ty,
                                expressions.expression(entry.value).ty,
                            ]
                        }),
                    )? =>
            {
                Ok(Some(fresh(value, Vec::new(), exit)))
            }
            HirExpressionKind::Copy(_)
                if matches!(path, Path::End)
                    && self.checker.copy_has_no_heap_children(expression.ty)? =>
            {
                Ok(Some(fresh(value, Vec::new(), exit)))
            }
            _ => Ok(None),
        }
    }

    fn construct(
        &self,
        value: HirValueId,
        path: Path<'_>,
        exit: &mut ExitEvidence,
        mut fields: impl Iterator<Item = (FieldKey, HirValueId)>,
    ) -> Result<Option<CallResultFacts>, BorrowCheckError> {
        if !matches!(path, Path::End) {
            let Some((key, remaining)) = path.step() else {
                return Ok(None);
            };
            let Some((_, child)) = fields.find(|(field, _)| *field == key) else {
                return Ok(None);
            };
            return self.value(child, remaining, true, exit);
        }
        let mut children = Vec::new();
        for (field, child) in fields {
            let ty = self.checker.module.expressions.expression(child).ty;
            if !self.checker.result_type_is_heap(ty)? {
                continue;
            }
            let Some(facts) = self.value(child, Path::End, true, exit)? else {
                return Ok(None);
            };
            children.push((field, facts));
        }
        Ok(Some(fresh(value, children, exit)))
    }
}

fn fresh(
    origin: HirValueId,
    children: Vec<(FieldKey, CallResultFacts)>,
    exit: &mut ExitEvidence,
) -> CallResultFacts {
    exit.repeated_fresh |= !exit.fresh.insert(origin);
    CallResultFacts::Fresh { children }
}
