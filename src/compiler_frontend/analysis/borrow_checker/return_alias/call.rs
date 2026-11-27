//! Instantiate a callee's authored lane in the caller's current allocation graph.

use super::graph::{Conditions, Node, Pairs, pair};
use super::{BorrowCheckError, Cell, FunctionEvidence, Graph, Origin, Site, Spec};
use crate::compiler_frontend::builtins::CollectionBuiltinOp;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::{
    CallTarget, ExternalAccessKind, ExternalReturnAlias,
};
use crate::compiler_frontend::hir::expression_store::HirValueRange;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirMapOp, ValueKind};
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::statements::HirLocalDestination;
use crate::compiler_frontend::public_call_summary::{
    CallAllocationSource, CallField, CallLaneSummary, CallOutputPath, CallParameterPath,
    CallSharingCondition, FunctionReturnAliasSummary,
};
use std::collections::{BTreeMap, BTreeSet};

impl FunctionEvidence<'_, '_> {
    pub(super) fn call(
        &self,
        state: &mut Graph,
        site: Site,
        target: &CallTarget,
        args: HirValueRange,
        error: bool,
        results: &[HirLocalDestination],
    ) -> Result<bool, BorrowCheckError> {
        let values = self.checker.module.expressions.values(args);
        if let CallTarget::External(id) = target {
            if !self.index.tracks_values {
                return Ok(true);
            }
            if let Some(operation) = CollectionBuiltinOp::from_external_function_id(*id) {
                return self.collection(state, site, operation, values, error, results);
            }
            let function = self
                .checker
                .external_package_registry
                .get_function_by_id(*id)
                .ok_or_else(|| {
                    self.checker
                        .diagnostics
                        .internal_error("External call has no registered allocation contract")
                })?;
            let actuals = self.capture_actuals(state, values)?;
            let sources = function.hir_return_sources();
            let mut reached = true;
            for (slot, &destination) in results.iter().enumerate() {
                let allowed = if error {
                    None
                } else {
                    sources.get(slot).map(|source| match &source.alias {
                        ExternalReturnAlias::Fresh => [].as_slice(),
                        ExternalReturnAlias::AliasArgs(args) => args.as_slice(),
                    })
                };
                let value = self.opaque_result(
                    state,
                    site,
                    self.ty(destination.local()),
                    &actuals,
                    allowed,
                )?;
                reached &= self.bind(state, destination, &value);
            }
            self.opaque_stores(state, site, &actuals, |index| {
                function
                    .parameters
                    .get(index)
                    .is_some_and(|parameter| parameter.access_kind == ExternalAccessKind::Mutable)
            })?;
            return Ok(reached);
        }

        let summary = self.call_summary(target).ok_or_else(|| {
            self.checker.diagnostics.internal_error(format!(
                "No allocation summary for HIR call target {target:?}"
            ))
        })?;
        match &summary.return_alias {
            FunctionReturnAliasSummary::Pending => Ok(false),
            FunctionReturnAliasSummary::Known(facts) => {
                let facts = if error { &facts.error } else { &facts.success };
                if !facts.reachable {
                    return Ok(false);
                }
                if !self.index.tracks_values {
                    return Ok(true);
                }
                if facts.sources.is_empty() && facts.stores.is_empty() {
                    // No allocation reaches any result yet. The destinations still take the
                    // (bottom) results; keeping their previous contents would let a later
                    // widening of this lane strongly replace facts it never published.
                    let mut reached = true;
                    for &destination in results {
                        let result = state.temporary();
                        reached &= self.bind(state, destination, &result);
                    }
                    return Ok(reached);
                }
                let actuals = self.capture_actuals(state, values)?;
                self.instantiate_lane(state, site, facts, &actuals, results)
            }
        }
    }

    fn capture_actuals(
        &self,
        state: &mut Graph,
        values: &[HirValueId],
    ) -> Result<Vec<Actual>, BorrowCheckError> {
        values
            .iter()
            .map(|&value| {
                let ty = self.checker.module.expressions.expression(value).ty;
                Ok(Actual {
                    expression: value,
                    value: self.value(state, value)?,
                    ty,
                })
            })
            .collect()
    }

    fn instantiate_lane(
        &self,
        state: &mut Graph,
        site: Site,
        lane: &CallLaneSummary,
        actuals: &[Actual],
        destinations: &[HirLocalDestination],
    ) -> Result<bool, BorrowCheckError> {
        let results = destinations
            .iter()
            .map(|_| state.temporary())
            .collect::<Vec<_>>();
        let mut stores = BTreeMap::new();
        let mut endpoints = BTreeMap::new();
        let mut rows = Vec::new();
        let mut present = BTreeSet::new();
        for row in lane.sources.iter().chain(&lane.stores) {
            let (value, ty, prefix, index) = match &row.output {
                CallOutputPath::Result(result) => (
                    &results[result.slot],
                    self.ty(destinations[result.slot].local()),
                    &result.path,
                    result.slot,
                ),
                CallOutputPath::Store { target, path } => {
                    let next_index = results.len() + stores.len();
                    let store = match stores.entry(target) {
                        std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            let actual = &actuals[target.index];
                            let destination =
                                if let Some((&last, prefix)) = target.path.split_last() {
                                    Some(state.capture(&actual.value.at(prefix)).at(&[last]))
                                } else {
                                    self.capture_actual_destination(state, actual.expression)?
                                };
                            entry.insert(Store {
                                destination,
                                value: state.temporary(),
                                ty: self.checker.call_path_type(actual.ty, &target.path),
                                index: next_index,
                            })
                        }
                    };
                    let ty = store.ty.ok_or_else(|| {
                        self.checker
                            .diagnostics
                            .internal_error("Call store target has an incompatible layout")
                    })?;
                    (&store.value, ty, path, store.index)
                }
            };
            let ty = self.checker.call_path_type(ty, prefix).ok_or_else(|| {
                self.checker
                    .diagnostics
                    .internal_error("Invalid allocation summary output path")
            })?;
            let cell = value.at(prefix);
            endpoints.insert(&row.output, cell.clone());
            if let Some((_, parent)) = prefix.split_last()
                && !present.contains(&value.at(parent))
            {
                continue;
            }
            let start = rows.len();
            match &row.source {
                CallAllocationSource::Parameter(parameter) => {
                    let source = actual_path(actuals, parameter);
                    if let Some(node) = state.nodes.get(&source) {
                        rows.push(Spec {
                            cell,
                            node: node.clone(),
                            refs: vec![source],
                            births: Vec::new(),
                            detached: false,
                        });
                    }
                }
                CallAllocationSource::Fresh => {
                    // The origin names the full output route. A collapsed selector on it, the
                    // store target's included, means the atom may stand for many allocations.
                    let route = match &row.output {
                        CallOutputPath::Store { target, .. } => {
                            target.path.iter().chain(prefix).copied().collect()
                        }
                        CallOutputPath::Result(_) => prefix.clone(),
                    };
                    rows.push(Spec::born(cell, ty, Origin::Fresh(site, index, route)));
                }
                CallAllocationSource::OpaqueFresh => {
                    // A known lane carries its own simultaneous pair relation, so these
                    // witnesses are endpoint-local unless the lane names a pair explicitly.
                    rows.push(Spec::born(
                        cell,
                        ty,
                        Origin::Opaque(site, ty, Some((index, prefix.clone()))),
                    ));
                }
            }
            if let Some(row) = rows.get(start) {
                present.insert(row.cell.clone());
            }
        }
        let mut sharing = Pairs::new();
        for relation in &lane.sharing {
            let left = &endpoints[&relation.left];
            let right = &endpoints[&relation.right];
            if !present.contains(left) || !present.contains(right) {
                continue;
            }
            let conditions = match &relation.condition {
                CallSharingCondition::Always => Conditions::from([CallSharingCondition::Always]),
                CallSharingCondition::Parameters(a, b) => {
                    state.conditions(&actual_path(actuals, a), &actual_path(actuals, b))
                }
            };
            if !conditions.is_empty() {
                sharing
                    .entry(pair(left, right))
                    .or_default()
                    .extend(conditions);
            }
        }
        state.install(rows, &[], Some(sharing));
        // A store describes a possible write. Do not kill the prior contents: another
        // execution of this lane may not write, and a summary may gain new stores later.
        for store in stores.values() {
            if let Some(destination) = &store.destination {
                let _ = self.assign(state, destination, &store.value, true);
            }
        }
        let mut reached = true;
        for (&destination, result) in destinations.iter().zip(&results) {
            reached &= self.bind(state, destination, result);
        }
        Ok(reached)
    }

    /// Capture the cell separately from its entry value. A projected argument keeps the
    /// containing allocation it named before any lane writes replace an enclosing slot.
    fn capture_actual_destination(
        &self,
        state: &mut Graph,
        actual: HirValueId,
    ) -> Result<Option<Cell>, BorrowCheckError> {
        let expression = self.checker.module.expressions.expression(actual);
        let HirExpressionKind::Load(place) = &expression.kind else {
            return Ok(None);
        };
        if expression.value_kind != ValueKind::Place {
            return Ok(None);
        }
        let target = self.place(place)?;
        if let Some((&key, prefix)) = target.path.split_last() {
            let holder = Cell::root(target.owner.clone()).at(prefix);
            Ok(Some(state.capture(&holder).at(&[key])))
        } else {
            Ok(Some(target))
        }
    }

    fn opaque_result(
        &self,
        state: &mut Graph,
        site: Site,
        ty: TypeId,
        actuals: &[Actual],
        allowed: Option<&[usize]>,
    ) -> Result<Cell, BorrowCheckError> {
        let result = state.temporary();
        let mut rows = Vec::new();
        self.typed_paths(ty, |path, ty| {
            rows.push(opaque_spec(
                state,
                site,
                result.at(path),
                ty,
                actuals,
                allowed,
            ));
        })?;
        state.install(rows, &[], None);
        Ok(result)
    }

    /// An opaque callee may store into every mutable argument path. Each store is a may-store,
    /// so a bottom holder (an absent optional payload, say) contributes nothing.
    fn opaque_stores(
        &self,
        state: &mut Graph,
        site: Site,
        actuals: &[Actual],
        is_mutable: impl Fn(usize) -> bool,
    ) -> Result<(), BorrowCheckError> {
        let mut stores = Vec::new();
        for (index, actual) in actuals.iter().enumerate() {
            if !is_mutable(index) {
                continue;
            }
            let mut paths = Vec::new();
            self.typed_paths(actual.ty, |path, ty| {
                if !path.is_empty() {
                    paths.push((path.to_vec(), ty));
                }
            })?;
            for (path, ty) in paths {
                let (&key, prefix) = path.split_last().unwrap();
                let holder = state.capture(&actual.value.at(prefix));
                let value = self.opaque_result(state, site, ty, actuals, None)?;
                stores.push((holder, key, value));
            }
        }
        for (holder, key, value) in stores {
            let _ = self.store(state, &holder, key, &value, true);
        }
        Ok(())
    }

    fn collection(
        &self,
        state: &mut Graph,
        site: Site,
        op: CollectionBuiltinOp,
        args: &[HirValueId],
        error: bool,
        results: &[HirLocalDestination],
    ) -> Result<bool, BorrowCheckError> {
        if error {
            let mut reached = true;
            for (slot, &destination) in results.iter().enumerate() {
                let value =
                    self.fresh_builtin_error(state, site, slot, self.ty(destination.local()))?;
                reached &= self.bind(state, destination, &value);
            }
            return Ok(reached);
        }
        Ok(match op {
            CollectionBuiltinOp::PushGrowable
            | CollectionBuiltinOp::PushFixed
            | CollectionBuiltinOp::Set => {
                let slot = if op == CollectionBuiltinOp::Set { 2 } else { 1 };
                let holder = self.value(state, args[0])?;
                let value = self.value(state, args[slot])?;
                self.store(state, &holder, CallField::Elem, &value, true)
            }
            CollectionBuiltinOp::Get | CollectionBuiltinOp::Remove => {
                let holder = self.value(state, args[0])?;
                let reached = self.element_result(state, &holder, results);
                reached
                    && (op != CollectionBuiltinOp::Remove
                        || state.erase(
                            &holder,
                            &[CallField::Elem],
                            Some(|origin: &Origin| self.literal_singleton(origin)),
                        ))
            }
            CollectionBuiltinOp::Length => true,
        })
    }

    pub(super) fn map(
        &self,
        state: &mut Graph,
        _site: Site,
        op: HirMapOp,
        receiver: HirValueId,
        args: HirValueRange,
        results: &[HirLocalDestination],
    ) -> Result<bool, BorrowCheckError> {
        let holder = self.value(state, receiver)?;
        let args = self.checker.module.expressions.values(args);
        Ok(match op {
            HirMapOp::Set => {
                let key = self.value(state, args[0])?;
                let value = self.value(state, args[1])?;
                self.store(state, &holder, CallField::MapKey, &key, true)
                    & self.store(state, &holder, CallField::Elem, &value, true)
            }
            HirMapOp::Get | HirMapOp::Remove => {
                let reached = self.element_result(state, &holder, results);
                reached
                    && (op != HirMapOp::Remove
                        || state.erase(
                            &holder,
                            &[CallField::MapKey, CallField::Elem],
                            Some(|origin: &Origin| self.literal_singleton(origin)),
                        ))
            }
            HirMapOp::Clear => state.erase(
                &holder,
                &[CallField::MapKey, CallField::Elem],
                None::<fn(&Origin) -> bool>,
            ),
            HirMapOp::Contains | HirMapOp::Length => true,
        })
    }

    #[must_use]
    fn element_result(
        &self,
        state: &mut Graph,
        holder: &Cell,
        results: &[HirLocalDestination],
    ) -> bool {
        let Some(&destination) = results.first() else {
            return true;
        };
        let result = state.capture(&holder.at(&[CallField::Elem]));
        self.bind(state, destination, &result)
    }

    fn literal_singleton(&self, origin: &Origin) -> bool {
        let Origin::Fresh(Site::Value(value), _, path) = origin else {
            return false;
        };
        if !path.is_empty() {
            return false;
        }
        match self
            .checker
            .module
            .expressions
            .expression(HirValueId(*value))
            .kind
        {
            HirExpressionKind::Collection(values) => values.len() <= 1,
            HirExpressionKind::MapLiteral(values) => values.len() <= 1,
            _ => false,
        }
    }
}

struct Actual {
    expression: HirValueId,
    value: Cell,
    ty: TypeId,
}

struct Store {
    destination: Option<Cell>,
    value: Cell,
    ty: Option<TypeId>,
    index: usize,
}

fn actual_path(actuals: &[Actual], path: &CallParameterPath) -> Cell {
    actuals[path.index].value.at(&path.path)
}

fn opaque_spec(
    state: &Graph,
    site: Site,
    cell: Cell,
    ty: TypeId,
    actuals: &[Actual],
    allowed: Option<&[usize]>,
) -> Spec {
    let mut row = Spec::born(cell, ty, Origin::Opaque(site, ty, None));
    let Node { sources, .. } = &mut row.node;
    for (index, actual) in actuals.iter().enumerate() {
        if allowed.is_some_and(|allowed| !allowed.contains(&index)) {
            continue;
        }
        // An actual's cells form one ordered range, as in `Graph::snapshot`.
        let subtree = state
            .nodes
            .range(actual.value.clone()..)
            .take_while(|(cell, _)| cell.below(&actual.value).is_some());
        for (cell, node) in subtree.filter(|(_, node)| node.ty == ty) {
            row.refs.push(cell.clone());
            sources.extend(node.sources.iter().cloned());
        }
    }
    row
}
