//! The summary evaluator keeps projected provenance and simultaneous sharing separate.
//! A block has one joined input; loops join finite typed paths and site identities, rather
//! than expanding the set of execution paths or comparing independently joined exits.

use super::super::{
    collect_statement_defined_locals, collect_statement_loaded_locals,
    collect_statement_written_locals, collect_terminator_loaded_locals,
};
use super::{BorrowCheckError, BorrowChecker};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::expression_store::HirProjection;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{BlockId, FieldId, FunctionId, HirValueId, LocalId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::hir::utils::for_each_terminator_target;
use crate::compiler_frontend::public_call_summary::{
    CallAllocationFacts, CallField, CallLaneSummary, CallOutputPath, CallParameterPath,
    CallPathSource, CallResultPath, CallSharing, PublicCallParameterAccess, PublicCallSummary,
};
use rustc_hash::FxHashMap;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[path = "call.rs"]
mod call;
#[path = "graph.rs"]
mod graph;
#[cfg(test)]
#[path = "result_paths_tests.rs"]
mod result_paths_tests;

use graph::{Cell, Graph, Origin, Owner, Path, Site, Spec};

#[derive(Clone, Copy)]
struct Local {
    mutable: bool,
    parameter: bool,
    authored: bool,
}

pub(in super::super) struct IndexedFunction {
    locals: FxHashMap<LocalId, Local>,
    retirements: FxHashMap<BlockId, Vec<(usize, LocalId)>>,
    pub callees: Vec<FunctionId>,
    tracks_values: bool,
}

impl IndexedFunction {
    pub(in super::super) fn new(
        checker: &BorrowChecker<'_>,
        function: &HirFunction,
        collect_callees: bool,
    ) -> Result<Self, BorrowCheckError> {
        let mut locals = FxHashMap::default();
        let mut callees = Vec::new();
        let mut retirements = FxHashMap::<_, Vec<_>>::default();
        let Some(shape) = function.return_contract.ordinary() else {
            return Ok(Self {
                locals,
                retirements,
                callees,
                tracks_values: false,
            });
        };
        let mut tracks_values = false;
        for ty in shape.success().iter().copied().chain(shape.error()) {
            tracks_values |= is_heap(checker, ty)?;
        }
        let access = &checker.public_call_summaries[&function.id].parameters;
        for (&local, parameter) in function.params.iter().zip(access) {
            if !tracks_values && parameter.access == PublicCallParameterAccess::Mutable {
                CallField::for_each_child(
                    checker.local_types[&local],
                    checker.type_environment,
                    |_, ty| {
                        tracks_values |= is_heap(checker, ty)?;
                        Ok::<_, BorrowCheckError>(())
                    },
                )?;
            }
        }
        let expressions = &checker.module.expressions;
        let mut uses = FxHashMap::<LocalId, LocalUses>::default();
        for block in checker.collect_reachable_blocks(function)? {
            let block = checker.block_by_id_or_error(block, function.id)?;
            if tracks_values {
                locals.extend(block.locals.iter().map(|local| {
                    let authored = checker
                        .module
                        .side_table
                        .local_origin_kind(local.id)
                        .is_none_or(|origin| origin == HirLocalOriginKind::User);
                    (
                        local.id,
                        Local {
                            mutable: local.mutable,
                            parameter: false,
                            authored,
                        },
                    )
                }));
            }
            for (index, statement) in block.statements.iter().enumerate() {
                if collect_callees
                    && let HirStatementKind::Call {
                        target: CallTarget::Local(callee),
                        ..
                    } = &statement.kind
                {
                    callees.push(*callee);
                }
                if !tracks_values {
                    continue;
                }
                collect_statement_loaded_locals(expressions, statement, &mut |local| {
                    LocalUses::record(&mut uses, local, block.id, index);
                });
                collect_statement_defined_locals(statement, &mut |local| {
                    uses.entry(local).or_insert(LocalUses {
                        defining_block: Some(block.id),
                        last: index,
                    });
                });
                collect_statement_written_locals(statement, &mut |local| {
                    LocalUses::record(&mut uses, local, block.id, index);
                });
            }
            if tracks_values {
                let mut use_in_terminator = |local| {
                    LocalUses::record(&mut uses, local, block.id, block.statements.len());
                };
                // Plumbing is not a user read, but the extractor still needs to capture it.
                if let HirTerminator::Jump { args, .. } = &block.terminator {
                    for arg in args {
                        use_in_terminator(arg.source);
                        use_in_terminator(arg.destination);
                    }
                } else {
                    collect_terminator_loaded_locals(
                        expressions,
                        &block.terminator,
                        &mut use_in_terminator,
                    );
                }
            }
            if collect_callees
                && let HirTerminator::Invoke {
                    operation:
                        HirFallibleOperation::Call {
                            target: CallTarget::Local(callee),
                            ..
                        },
                    ..
                } = &block.terminator
            {
                callees.push(*callee);
            }
        }
        if tracks_values {
            for parameter in &function.params {
                let local = locals.get_mut(parameter).ok_or_else(|| {
                    checker
                        .diagnostics
                        .internal_error("Missing summary parameter local")
                })?;
                local.parameter = true;
            }
        }
        for (local, uses) in uses {
            if let Some(block) = uses.defining_block {
                retirements
                    .entry(block)
                    .or_default()
                    .push((uses.last, local));
            }
        }
        for events in retirements.values_mut() {
            events.sort_unstable_by_key(|&(index, local)| (index, local.0));
        }
        callees.sort_unstable_by_key(|callee| callee.0);
        callees.dedup();
        Ok(Self {
            locals,
            retirements,
            callees,
            tracks_values,
        })
    }
}

// Only retire named bindings proven dead without a CFG liveness walk. Every reference must
// follow a definition in the same block, so a backedge also redefines before the next use.
struct LocalUses {
    defining_block: Option<BlockId>,
    last: usize,
}

impl LocalUses {
    fn record(uses: &mut FxHashMap<LocalId, Self>, local: LocalId, block: BlockId, index: usize) {
        let uses = uses.entry(local).or_insert(Self {
            defining_block: None,
            last: index,
        });
        if uses.defining_block == Some(block) {
            uses.last = index;
        } else {
            uses.defining_block = None;
        }
    }
}

pub(super) struct FunctionEvidence<'a, 'module> {
    pub(super) checker: &'a BorrowChecker<'module>,
    pub(super) function: &'a HirFunction,
    pub(super) index: &'a IndexedFunction,
}

impl FunctionEvidence<'_, '_> {
    pub(super) fn evaluate(&self) -> Result<CallAllocationFacts, BorrowCheckError> {
        if self.function.return_contract.is_never() {
            return Ok(CallAllocationFacts::default());
        }
        let entry = self.function.entry;
        let mut initial = Graph::default();
        let mut rows = Vec::new();
        if self.index.tracks_values {
            for (index, &local) in self.function.params.iter().enumerate() {
                self.typed_paths(self.ty(local), |path, ty| {
                    let parameter = CallParameterPath {
                        index,
                        path: path.to_vec(),
                    };
                    rows.push(Spec::born(
                        Self::local(local).at(path),
                        ty,
                        Origin::Parameter(parameter),
                    ));
                })?;
            }
        }
        initial.install(rows, &[], None);
        let mut inputs = FxHashMap::default();
        inputs.insert(entry, initial);
        let mut queue = VecDeque::from([entry]);
        let mut queued = rustc_hash::FxHashSet::default();
        queued.insert(entry);
        let mut success = Lane::default();
        let mut error = Lane::default();
        'worklist: while let Some(block_id) = queue.pop_front() {
            queued.remove(&block_id);
            let block = self
                .checker
                .block_by_id_or_error(block_id, self.function.id)?;
            let mut state = inputs[&block_id].clone();
            let mut retiring = self
                .index
                .retirements
                .get(&block_id)
                .into_iter()
                .flatten()
                .peekable();
            let mut protected = Vec::new();
            for (index, statement) in block.statements.iter().enumerate() {
                if !self.statement(
                    &mut state,
                    Site::Operation(block_id.0, index),
                    &statement.kind,
                )? {
                    continue 'worklist;
                }
                state.retire_temporaries();
                while let Some(&(_, local)) = retiring.next_if(|&&(last, _)| last == index) {
                    if !state.retire_local(local.0) {
                        protected.push(local.0);
                    }
                }
                protected.retain(|&local| !state.retire_local(local));
            }
            let mut edge = |target, mut state: Graph| {
                state.retire_temporaries();
                use std::collections::hash_map::Entry;
                let changed = match inputs.entry(target) {
                    Entry::Occupied(mut input) => input.get_mut().join(state),
                    Entry::Vacant(input) => {
                        input.insert(state);
                        true
                    }
                };
                if changed && queued.insert(target) {
                    queue.push_back(target);
                }
            };
            match &block.terminator {
                HirTerminator::Return(values) => {
                    let values = self.checker.module.expressions.values(*values);
                    let results = values
                        .iter()
                        .map(|&value| self.value(&mut state, value))
                        .collect::<Result<Vec<_>, _>>()?;
                    success.extend(&state, &results);
                }
                HirTerminator::ReturnError(value) => {
                    let result = self.value(&mut state, *value)?;
                    error.extend(&state, &[result]);
                }
                HirTerminator::Jump { target, args } => {
                    let mut rows = Vec::new();
                    let mut clear = Vec::with_capacity(args.len());
                    for arg in args {
                        let destination = Self::local(arg.destination);
                        state.snapshot(&destination, &Self::local(arg.source), &mut rows);
                        clear.push(destination);
                        state.aliases.remove(&arg.destination.0);
                    }
                    state.install(rows, &clear, None);
                    edge(*target, state);
                }
                HirTerminator::Invoke {
                    operation,
                    success,
                    success_results,
                    failure,
                    error_result,
                    ..
                } => {
                    let site = Site::Operation(block_id.0, block.statements.len());
                    let mut normal = state.clone();
                    // Evaluate operands against the same predecessor. Retire the unselected
                    // slots after capture; selected definitions replace their own prior bindings.
                    if self.invoke(&mut normal, site, operation, false, success_results)? {
                        self.kill_local(&mut normal, *error_result);
                        edge(*success, normal);
                    }
                    if self.invoke(
                        &mut state,
                        site,
                        operation,
                        true,
                        std::slice::from_ref(error_result),
                    )? {
                        for &local in success_results {
                            self.kill_local(&mut state, local);
                        }
                        edge(*failure, state);
                    }
                }
                HirTerminator::Break { target } | HirTerminator::Continue { target } => {
                    edge(*target, state)
                }
                terminator => {
                    for_each_terminator_target(terminator, |target| edge(target, state.clone()))
                }
            }
        }
        Ok(CallAllocationFacts {
            success: success.finish(),
            error: error.finish(),
        })
    }

    fn statement(
        &self,
        state: &mut Graph,
        site: Site,
        statement: &HirStatementKind,
    ) -> Result<bool, BorrowCheckError> {
        if !self.index.tracks_values {
            return match statement {
                HirStatementKind::Call {
                    target,
                    args,
                    results,
                } => self.call(state, site, target, *args, false, results),
                _ => Ok(true),
            };
        }
        match statement {
            HirStatementKind::Write { target, value } => match target {
                HirWriteTarget::DefineLocal(local) => {
                    let alias = self.acquired_alias(*local, *value)?;
                    let captured = self.value(state, *value)?;
                    self.define(state, *local, &captured, alias.map(|alias| (site, alias)));
                }
                HirWriteTarget::AssignPlace(place) => {
                    // A direct self-update adds no alias and keeps the binding role, as in live
                    // transfer.
                    if target.is_direct_self_update_of(
                        self.checker.module.expressions.expression(*value),
                    ) {
                        return Ok(true);
                    }
                    let target = self.place(place)?;
                    let alias = if target.path.is_empty() {
                        self.acquired_alias(place.root, *value)?
                    } else {
                        None
                    };
                    let value = self.value(state, *value)?;
                    let Some(alias) = alias else {
                        return Ok(self.assign(state, &target, &value, false));
                    };
                    // The plain-slot alternative acquires the place as its alias, as at a
                    // definition; each existing alias alternative is written through instead.
                    let Some(alternatives) = state.aliases.get(&place.root.0) else {
                        self.define(state, place.root, &value, Some((site, alias)));
                        return Ok(true);
                    };
                    if !alternatives.contains(&target) {
                        return Ok(self.assign(state, &target, &value, false));
                    }
                    let mut aliased = state.clone();
                    if let Some(alternatives) = aliased.aliases.get_mut(&place.root.0) {
                        alternatives.remove(&target);
                    }
                    // The alternatives are joined possibilities: one whose holder no allocation
                    // reaches yet adds nothing, and widening that resolves it only adds facts.
                    let aliased = self
                        .assign(&mut aliased, &target, &value, false)
                        .then_some(aliased);
                    self.define(state, place.root, &value, Some((site, alias)));
                    if let Some(aliased) = aliased {
                        state.join(aliased);
                    }
                }
            },
            HirStatementKind::Call {
                target,
                args,
                results,
            } => {
                return self.call(state, site, target, *args, false, results);
            }
            HirStatementKind::MapOp {
                op,
                receiver,
                args,
                result,
            } => {
                return self.map(state, site, *op, *receiver, *args, result.as_slice());
            }
            HirStatementKind::FormatFloat { result, .. } => {
                let value = self.fresh_operation(state, site, 0, self.ty(result.local()))?;
                return Ok(self.bind(state, *result, &value));
            }
            HirStatementKind::PushRuntimeFragment { vec_local, value } => {
                let value = self.value(state, *value)?;
                return Ok(self.store(
                    state,
                    &Self::local(*vec_local),
                    CallField::Elem,
                    &value,
                    true,
                ));
            }
            HirStatementKind::NumericOp { .. }
            | HirStatementKind::FastStoreConversion { .. }
            | HirStatementKind::LoopStepFailure { .. }
            | HirStatementKind::RangeCandidate { .. }
            | HirStatementKind::ValidateFloat { .. }
            | HirStatementKind::Expr(_) => {}
        }
        Ok(true)
    }

    fn invoke(
        &self,
        state: &mut Graph,
        site: Site,
        op: &HirFallibleOperation,
        error: bool,
        locals: &[LocalId],
    ) -> Result<bool, BorrowCheckError> {
        if !self.index.tracks_values {
            return match op {
                HirFallibleOperation::Call { target, args } => {
                    self.call(state, site, target, *args, error, &[])
                }
                _ => Ok(true),
            };
        }
        let destinations = locals
            .iter()
            .copied()
            .map(HirLocalDestination::Define)
            .collect::<Vec<_>>();
        match op {
            HirFallibleOperation::Call { target, args } => {
                self.call(state, site, target, *args, error, &destinations)
            }
            HirFallibleOperation::MapOp { op, receiver, args } if !error => {
                self.map(state, site, *op, *receiver, *args, &destinations)
            }
            _ => {
                let mut reached = true;
                for (slot, &destination) in destinations.iter().enumerate() {
                    let ty = self.ty(destination.local());
                    let value = if error {
                        self.fresh_builtin_error(state, site, slot, ty)?
                    } else {
                        self.fresh_operation(state, site, slot, ty)?
                    };
                    reached &= self.bind(state, destination, &value);
                }
                Ok(reached)
            }
        }
    }

    fn value(&self, state: &mut Graph, value: HirValueId) -> Result<Cell, BorrowCheckError> {
        let expressions = &self.checker.module.expressions;
        let expression = expressions.expression(value);
        if !self.heap(expression.ty)? {
            return Ok(state.temporary());
        }
        let site = Site::Value(value.0);
        match &expression.kind {
            HirExpressionKind::Load(place) if expression.value_kind != ValueKind::Const => {
                let source = self.place(place)?;
                Ok(state.capture(&source))
            }
            HirExpressionKind::VariantPayloadGet {
                source,
                variant_index,
                field_index,
                ..
            } => {
                let source = self.value(state, *source)?;
                Ok(source.at(&[payload(*variant_index, *field_index)]))
            }
            HirExpressionKind::StructConstruct { fields, .. } => {
                let children = expressions
                    .struct_fields(*fields)
                    .iter()
                    .map(|&(field, child)| Ok((self.field(expression.ty, field)?, child)));
                self.construct(state, value, expression.ty, children)
            }
            HirExpressionKind::VariantConstruct {
                variant_index,
                fields,
                ..
            }
            | HirExpressionKind::RouteConstruct {
                variant_index,
                fields,
                ..
            } => self.construct(
                state,
                value,
                expression.ty,
                expressions
                    .variant_fields(*fields)
                    .iter()
                    .enumerate()
                    .map(|(index, field)| Ok((payload(*variant_index, index), field.value))),
            ),
            HirExpressionKind::Collection(values) => self.construct(
                state,
                value,
                expression.ty,
                expressions
                    .values(*values)
                    .iter()
                    .map(|&child| Ok((CallField::Elem, child))),
            ),
            HirExpressionKind::MapLiteral(entries) => self.construct(
                state,
                value,
                expression.ty,
                expressions.map_entries(*entries).iter().flat_map(|entry| {
                    [
                        Ok((CallField::MapKey, entry.key)),
                        Ok((CallField::Elem, entry.value)),
                    ]
                }),
            ),
            HirExpressionKind::Copy(place) => {
                let source = self.place(place)?;
                let target = state.temporary();
                let mut rows = Vec::new();
                state.snapshot(&target, &source, &mut rows);
                for row in &mut rows {
                    let detached = Spec::born(
                        row.cell.clone(),
                        row.node.ty,
                        Origin::Fresh(site, 0, row.cell.path.clone()),
                    );
                    row.node = detached.node;
                    row.births = detached.births;
                    row.detached = true;
                }
                state.install(rows, &[], None);
                Ok(target)
            }
            HirExpressionKind::Load(_) => self.opaque_value(state, site, 0, expression.ty),
            _ => self.fresh_operation(state, site, 0, expression.ty),
        }
    }

    fn construct(
        &self,
        state: &mut Graph,
        value: HirValueId,
        ty: TypeId,
        children: impl Iterator<Item = Result<(CallField, HirValueId), BorrowCheckError>>,
    ) -> Result<Cell, BorrowCheckError> {
        let result = state.temporary();
        let mut rows = vec![Spec::born(
            result.clone(),
            ty,
            Origin::Fresh(Site::Value(value.0), 0, Vec::new()),
        )];
        for child in children {
            let (field, value) = child?;
            if !self.heap(self.checker.module.expressions.expression(value).ty)? {
                continue;
            }
            let value = self.value(state, value)?;
            state.snapshot(&result.at(&[field]), &value, &mut rows);
        }
        state.install(rows, &[], None);
        Ok(result)
    }

    fn fresh_operation(
        &self,
        state: &mut Graph,
        site: Site,
        slot: usize,
        ty: TypeId,
    ) -> Result<Cell, BorrowCheckError> {
        let result = state.temporary();
        if self.heap(ty)? {
            state.install(
                vec![Spec::born(
                    result.clone(),
                    ty,
                    Origin::Fresh(site, slot, Vec::new()),
                )],
                &[],
                None,
            );
        }
        Ok(result)
    }

    /// Compiler-produced builtin errors initialise their message as well as the record.
    fn fresh_builtin_error(
        &self,
        state: &mut Graph,
        site: Site,
        slot: usize,
        ty: TypeId,
    ) -> Result<Cell, BorrowCheckError> {
        let result = state.temporary();
        let mut rows = Vec::new();
        self.typed_paths(ty, |path, ty| {
            rows.push(Spec::born(
                result.at(path),
                ty,
                Origin::Fresh(site, slot, path.to_vec()),
            ));
        })?;
        state.install(rows, &[], None);
        Ok(result)
    }

    fn opaque_value(
        &self,
        state: &mut Graph,
        site: Site,
        _slot: usize,
        ty: TypeId,
    ) -> Result<Cell, BorrowCheckError> {
        let result = state.temporary();
        let mut specs = Vec::new();
        self.typed_paths(ty, |path, ty| {
            specs.push(Spec::born(
                result.at(path),
                ty,
                Origin::Opaque(site, ty, None),
            ))
        })?;
        state.install(specs, &[], None);
        Ok(result)
    }

    /// The place a mutable authored local starts aliasing when it receives a place load.
    fn acquired_alias(
        &self,
        local: LocalId,
        value: HirValueId,
    ) -> Result<Option<Cell>, BorrowCheckError> {
        let binding = self.index.locals[&local];
        let expression = self.checker.module.expressions.expression(value);
        if binding.mutable
            && binding.authored
            && expression.value_kind == ValueKind::Place
            && let HirExpressionKind::Load(place) = &expression.kind
        {
            return Ok(Some(self.place(place)?));
        }
        Ok(None)
    }

    /// Bind a definite new value to the local's slot. A place load also makes the local that
    /// place's alias; a projected one captures its holder under the acquisition site.
    fn define(&self, state: &mut Graph, local: LocalId, value: &Cell, alias: Option<(Site, Cell)>) {
        let holder = alias.as_ref().and_then(|(_, alias)| {
            let (_, prefix) = alias.path.split_last()?;
            let holder = Cell::root(alias.owner.clone()).at(prefix);
            // A place that runs through the dying slot is captured before the slot is cleared.
            let through_slot = alias.owner == Owner::Local(local.0);
            Some(if through_slot {
                state.capture(&holder)
            } else {
                holder
            })
        });
        self.kill_local(state, local);
        self.rebind(state, Self::local(local), value, false);
        let Some((site, alias)) = alias else { return };
        let targets = match holder {
            Some(holder) => {
                let owner = Cell::root(Owner::Alias(local.0, site));
                self.rebind(state, owner.clone(), &holder, false);
                BTreeSet::from([owner.at(&alias.path[alias.path.len() - 1..])])
            }
            None => state
                .aliases
                .get(&place_local(&alias))
                .cloned()
                .unwrap_or_else(|| BTreeSet::from([alias])),
        };
        state.aliases.insert(local.0, targets);
    }

    /// Returns `false` when an update reaches a definite holder that no allocation reaches yet.
    #[must_use]
    fn bind(&self, state: &mut Graph, destination: HirLocalDestination, value: &Cell) -> bool {
        let local = destination.local();
        if matches!(destination, HirLocalDestination::Update(_)) {
            return self.assign(state, &Self::local(local), value, false);
        }
        self.define(state, local, value, None);
        true
    }

    fn kill_local(&self, state: &mut Graph, local: LocalId) {
        state.aliases.remove(&local.0);
        state.clear_local(local.0);
    }

    fn rebind(&self, state: &mut Graph, target: Cell, value: &Cell, weak: bool) {
        let mut rows = Vec::new();
        state.snapshot(&target, value, &mut rows);
        let clear = if weak {
            &[]
        } else {
            std::slice::from_ref(&target)
        };
        state.install(rows, clear, None);
    }

    /// Write every destination of one assignment. Returns `false` when a definite destination's
    /// holder is still bottom: that path stays incomplete until a widening input resolves the
    /// holder, instead of dropping a write that the resolved state would make strong.
    #[must_use]
    fn assign(&self, state: &mut Graph, target: &Cell, value: &Cell, weak: bool) -> bool {
        let mut seen = BTreeSet::new();
        let mut pending = vec![(target.clone(), weak)];
        let mut reached = true;
        while let Some((target, weak)) = pending.pop() {
            if !seen.insert(target.clone()) {
                continue;
            }
            if let Some((&key, prefix)) = target.path.split_last() {
                // An ambiguous alias target is only a may-destination: a bottom alternative adds
                // nothing, and another alternative carries the write.
                reached &= self.store(
                    state,
                    &Cell::root(target.owner.clone()).at(prefix),
                    key,
                    value,
                    weak,
                ) || weak;
            } else {
                let aliases = state.aliases.get(&place_local(&target));
                let weak = weak || aliases.is_some_and(|aliases| aliases.len() != 1);
                if let Some(aliases) = aliases {
                    pending.extend(aliases.iter().cloned().map(|alias| (alias, weak)));
                }
                self.rebind(state, target, value, weak);
            }
        }
        reached
    }

    /// Returns `false` when the holder is bottom. `weak` only stops the write replacing existing
    /// contents; whether a bottom holder may be skipped is the caller's decision, because a
    /// definite operation on a weakly updated element path still needs its receiver to exist.
    #[must_use]
    fn store(
        &self,
        state: &mut Graph,
        holder: &Cell,
        key: CallField,
        value: &Cell,
        weak: bool,
    ) -> bool {
        let allowed = |index: usize| self.index.locals[&self.function.params[index]].mutable;
        state.write(holder, key, value, weak, allowed)
    }

    fn place(&self, place: &HirPlace) -> Result<Cell, BorrowCheckError> {
        let mut cell = Self::local(place.root);
        let mut ty = self.ty(place.root);
        for projection in self
            .checker
            .module
            .expressions
            .projections(place.projections)
        {
            let field = match projection {
                HirProjection::Field(field) => self.field(ty, *field)?,
                HirProjection::Index(_) => CallField::Elem,
            };
            ty = self.checker.call_child_type(ty, field).ok_or_else(|| {
                self.checker
                    .diagnostics
                    .internal_error("Invalid projection in allocation summary")
            })?;
            cell.path.push(field);
        }
        Ok(cell)
    }

    fn field(&self, ty: TypeId, field: FieldId) -> Result<CallField, BorrowCheckError> {
        self.checker
            .module
            .structs
            .iter()
            .find(|layout| layout.frontend_type_id == ty)
            .and_then(|layout| {
                layout
                    .fields
                    .iter()
                    .position(|candidate| candidate.id == field)
            })
            .map(|ordinal| CallField::Field(ordinal as u32))
            .ok_or_else(|| {
                self.checker
                    .diagnostics
                    .internal_error("Missing field ordinal in allocation summary")
            })
    }

    fn heap(&self, ty: TypeId) -> Result<bool, BorrowCheckError> {
        is_heap(self.checker, ty)
    }

    fn ty(&self, local: LocalId) -> TypeId {
        self.checker.local_types[&local]
    }

    fn local(local: LocalId) -> Cell {
        Cell::root(Owner::Local(local.0))
    }

    fn typed_paths(
        &self,
        ty: TypeId,
        mut visit: impl FnMut(&[CallField], TypeId),
    ) -> Result<(), BorrowCheckError> {
        fn walk(
            context: &FunctionEvidence<'_, '_>,
            ty: TypeId,
            path: &mut Path,
            stack: &mut Vec<TypeId>,
            visit: &mut impl FnMut(&[CallField], TypeId),
        ) -> Result<(), BorrowCheckError> {
            if !context.heap(ty)? {
                return Ok(());
            }
            if stack.contains(&ty) {
                return Err(context
                    .checker
                    .diagnostics
                    .internal_error("Recursive runtime layout reached allocation summary"));
            }
            visit(path, ty);
            stack.push(ty);
            CallField::for_each_child::<BorrowCheckError>(
                ty,
                context.checker.type_environment,
                |field, child| {
                    path.push(field);
                    walk(context, child, path, stack, visit)?;
                    path.pop();
                    Ok(())
                },
            )?;
            stack.pop();
            Ok(())
        }
        walk(self, ty, &mut Vec::new(), &mut Vec::new(), &mut visit)
    }

    fn call_summary(&self, target: &CallTarget) -> Option<&PublicCallSummary> {
        match target {
            CallTarget::Local(id) => self.checker.public_call_summaries.get(id),
            CallTarget::CrossModule(id) => self.checker.module.imported_call_summaries.get(id),
            CallTarget::ModulePrivate(id) => {
                self.checker.module.module_private_call_summaries.get(id)
            }
            CallTarget::Generated(id) => self
                .checker
                .module
                .generated_call_summaries
                .get(id.as_ref()),
            CallTarget::External(_) => None,
        }
    }
}

fn is_heap(checker: &BorrowChecker<'_>, ty: TypeId) -> Result<bool, BorrowCheckError> {
    Ok(classify_hir_type(ty, checker.type_environment)?.has_in_place_mutation_surface())
}

fn payload(variant: usize, field: usize) -> CallField {
    CallField::Payload {
        variant: variant as u32,
        field: field as u32,
    }
}

fn place_local(cell: &Cell) -> u32 {
    match cell.owner {
        Owner::Local(local) => local,
        _ => unreachable!("a HIR place must have a local root"),
    }
}

#[derive(Default)]
struct Lane {
    reachable: bool,
    sources: BTreeSet<CallPathSource>,
    stores: BTreeSet<CallPathSource>,
    sharing: BTreeSet<CallSharing>,
}

impl Lane {
    fn extend(&mut self, state: &Graph, results: &[Cell]) {
        self.reachable = true;
        let mut endpoints = BTreeMap::<&Cell, Vec<CallOutputPath>>::new();
        for (cell, node) in &state.nodes {
            let mut outputs = Vec::new();
            for (slot, result) in results.iter().enumerate() {
                if let Some(path) = cell.below(result) {
                    let output = CallOutputPath::Result(CallResultPath {
                        slot,
                        path: path.to_vec(),
                    });
                    self.sources
                        .extend(node.sources.iter().cloned().map(|origin| CallPathSource {
                            output: output.clone(),
                            source: origin.into_source(),
                        }));
                    outputs.push(output);
                }
            }
            if let Owner::Store(target) = &cell.owner {
                let output = CallOutputPath::Store {
                    target: target.clone(),
                    path: cell.path.clone(),
                };
                self.stores
                    .extend(node.sources.iter().cloned().map(|origin| CallPathSource {
                        output: output.clone(),
                        source: origin.into_source(),
                    }));
                outputs.push(output);
            }
            if !outputs.is_empty() {
                endpoints.insert(cell, outputs);
            }
        }
        for ((left, right), conditions) in &state.pairs {
            let (Some(left), Some(right)) = (endpoints.get(left), endpoints.get(right)) else {
                continue;
            };
            for a in left {
                for b in right {
                    let (left, right) = if a <= b { (a, b) } else { (b, a) };
                    if left != right {
                        self.sharing
                            .extend(conditions.iter().cloned().map(|condition| CallSharing {
                                left: left.clone(),
                                right: right.clone(),
                                condition,
                            }));
                    }
                }
            }
        }
    }

    fn finish(self) -> CallLaneSummary {
        CallLaneSummary {
            reachable: self.reachable,
            sources: self.sources.into_iter().collect(),
            stores: self.stores.into_iter().collect(),
            sharing: self.sharing.into_iter().collect(),
        }
    }
}
