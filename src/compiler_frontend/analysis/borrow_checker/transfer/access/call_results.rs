//! Instantiate validated return shapes, or conservatively connect compatible opaque paths.

use super::super::layer::{
    CapturedValue, LayerValueContext, capture_copy, resolve_place, stored_allocation_values,
    stored_values,
};
use super::{CallArgumentTransfer, CallTransferContext};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AllocId, Coverage, DependencySet, FieldKey, PlaceResolution, ResolvedPlace, RootSet,
    StoredValue, TypedStore, ValueAlt, ValueAlternatives, ValueKind as PayloadKind,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::call_semantics::CallReturns;
use crate::compiler_frontend::analysis::borrow_checker::transfer::{
    BorrowTransferContext, InvokeLane, InvokeLayerOp,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::{ExternalReturnAlias, ExternalReturnSlot};
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;
use crate::compiler_frontend::hir::statements::HirLocalDestination;
use crate::compiler_frontend::public_call_summary::{
    CallAllocationSource, CallField, CallFieldHir, CallLaneSummary, CallOutputPath, CallPathSource,
    CallSharingCondition, FunctionReturnAliasSummary,
};
use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Clone, Copy)]
pub(super) enum CallLane<'a> {
    Unreachable,
    Known(&'a CallLaneSummary),
    Lookup,
    External(&'a [ExternalReturnSlot]),
    Opaque(Inputs<'a>),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Inputs<'a> {
    Fresh,
    Args(&'a [usize]),
    Any,
}

impl<'a> CallLane<'a> {
    pub(super) fn new(returns: CallReturns<'a>, lane: InvokeLane) -> Self {
        match (returns, lane) {
            (CallReturns::Summary(FunctionReturnAliasSummary::Pending), _) => {
                Self::Opaque(Inputs::Any)
            }
            (CallReturns::Summary(FunctionReturnAliasSummary::Known(facts)), lane) => {
                let facts = match lane {
                    InvokeLane::Success => &facts.success,
                    InvokeLane::Error => &facts.error,
                };
                if facts.reachable {
                    Self::Known(facts)
                } else if facts.can_return() {
                    Self::Opaque(Inputs::Any)
                } else {
                    Self::Unreachable
                }
            }
            (CallReturns::External(slots), InvokeLane::Success) => Self::External(slots),
            (CallReturns::External(_), InvokeLane::Error) => Self::Opaque(Inputs::Any),
            (CallReturns::Fresh, _) => Self::Opaque(Inputs::Fresh),
        }
    }

    pub(super) fn unreachable(self) -> bool {
        matches!(self, Self::Unreachable)
    }

    fn opaque_inputs(self, slot: usize) -> Inputs<'a> {
        match self {
            Self::Opaque(inputs) => inputs,
            Self::External(slots) => match slots.get(slot).map(|slot| &slot.alias) {
                Some(ExternalReturnAlias::AliasArgs(indices)) => Inputs::Args(indices),
                Some(ExternalReturnAlias::Fresh) => Inputs::Fresh,
                None => Inputs::Any,
            },
            Self::Lookup | Self::Known(_) | Self::Unreachable => {
                unreachable!("expected an opaque lane")
            }
        }
    }

    pub(super) fn retains_result(self) -> bool {
        match self {
            Self::Unreachable | Self::Opaque(Inputs::Fresh) => false,
            Self::Lookup => true,
            Self::Known(facts) => facts.sources.iter().any(|row| {
                matches!(row.source, CallAllocationSource::Parameter(_))
            }),
            Self::External(slots) => slots.iter().any(|slot| {
                matches!(&slot.alias, ExternalReturnAlias::AliasArgs(indices) if !indices.is_empty())
            }),
            Self::Opaque(_) => true,
        }
    }

    pub(super) fn may_store(self, argument: usize) -> bool {
        matches!(self, Self::Known(facts) if facts.stores.iter().any(|row| match &row.source {
            CallAllocationSource::Parameter(path) => path.index == argument,
            CallAllocationSource::OpaqueFresh | CallAllocationSource::Fresh => false,
        }))
    }

    pub(super) fn may_retain(self, argument: usize, roots: &[RootSet]) -> bool {
        let opaque = || roots.get(argument).is_some_and(|root| !root.is_empty());
        match self {
            Self::Lookup => argument == 0,
            Self::Known(facts) => facts.sources.iter().any(|row| match &row.source {
                CallAllocationSource::Parameter(path) => path.index == argument,
                CallAllocationSource::OpaqueFresh | CallAllocationSource::Fresh => false,
            }),
            Self::External(slots) => slots.iter().any(|slot| {
                matches!(&slot.alias, ExternalReturnAlias::AliasArgs(indices) if indices.contains(&argument))
            }),
            Self::Opaque(Inputs::Args(indices)) => indices.contains(&argument),
            Self::Opaque(Inputs::Any) => opaque(),
            Self::Opaque(Inputs::Fresh) | Self::Unreachable => false,
        }
    }

    /// Coarse value roots name only sources of this slot's whole allocation, not its children.
    pub(super) fn root_sources<'b>(
        self,
        slot: usize,
        sources: &'b mut Vec<usize>,
    ) -> Option<&'b [usize]>
    where
        'a: 'b,
    {
        match self {
            Self::Lookup => Some(if slot == 0 { &[0] } else { &[] }),
            Self::Known(facts) => {
                let rows = result_root_rows(&facts.sources, slot);
                if let [row] = rows
                    && let CallAllocationSource::Parameter(path) = &row.source
                    && path.path.is_empty()
                {
                    return Some(std::slice::from_ref(&path.index));
                }
                sources.clear();
                for row in rows {
                    if let CallAllocationSource::Parameter(path) = &row.source
                        && path.path.is_empty()
                    {
                        sources.push(path.index);
                    }
                }
                sources.sort_unstable();
                sources.dedup();
                Some(sources)
            }
            Self::Unreachable => Some(&[]),
            _ => match self.opaque_inputs(slot) {
                Inputs::Fresh => Some(&[]),
                Inputs::Args(indices) => Some(indices),
                Inputs::Any => None,
            },
        }
    }

    /// Unlike alias roots, a derivation is an exemption only if every possible root value
    /// derives from that same writer. A projected parameter source still carries its ledger.
    pub(super) fn derive_result(
        self,
        slot: usize,
        actuals: &CallActuals<'_>,
        local_count: usize,
    ) -> RootSet {
        let empty = || RootSet::empty(local_count);
        if matches!(self, Self::Lookup) {
            return if slot == 0 {
                actuals.derivations.first().cloned().unwrap_or_else(empty)
            } else {
                empty()
            };
        }
        let Self::Known(facts) = self else {
            // An opaque result may be fresh even when it is allowed to alias an actual.
            return empty();
        };
        let mut rows = result_root_rows(&facts.sources, slot).iter();
        let origin = |row: &CallPathSource| {
            if let CallAllocationSource::Parameter(path) = &row.source {
                actuals.derivations.get(path.index)
            } else {
                None
            }
        };
        let Some(first) = rows.next().and_then(origin) else {
            return empty();
        };
        let mut derived = first.clone();
        for row in rows {
            let Some(other) = origin(row) else {
                return empty();
            };
            derived.intersect_with(other);
        }
        derived
    }
}

fn result_root_rows(sources: &[CallPathSource], slot: usize) -> &[CallPathSource] {
    let start = sources.partition_point(
        |row| matches!(&row.output, CallOutputPath::Result(result) if result.slot < slot),
    );
    let remaining = &sources[start..];
    let end = remaining.partition_point(|row| {
        matches!(&row.output, CallOutputPath::Result(result) if result.slot == slot && result.path.is_empty())
    });
    &remaining[..end]
}

pub(super) fn call_result_ops(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    results: impl Iterator<Item = HirLocalDestination> + Clone,
    lane: CallLane<'_>,
    outcome: InvokeLane,
    allow_plain_roots: bool,
) -> Result<Vec<InvokeLayerOp>, BorrowCheckError> {
    let mut ops = Vec::new();
    let mut opaque = FxHashMap::default();
    match lane {
        CallLane::Unreachable => return Ok(ops),
        CallLane::Known(facts) => {
            let mut ops = if facts.stores.is_empty()
                && facts.sharing.is_empty()
                && let Some(ops) =
                    direct_result_ops(input, actuals, results.clone(), facts, allow_plain_roots)?
            {
                ops
            } else {
                instantiate_lane(input, actuals, results.clone(), facts, outcome)?
            };
            if input.state.has_value_alternatives() {
                append_scalar_results(input, actuals, results, facts, &mut ops)?;
                append_uncertain_scalar_stores(input, actuals, facts, &mut ops)?;
            }
            return Ok(ops);
        }
        _ => {}
    }
    let multiple = results.clone().nth(1).is_some();
    for (slot, dest) in results.enumerate() {
        let Some(ty) = result_type(input, dest)? else {
            if input.state.has_value_alternatives() {
                let inputs = lane.opaque_inputs(slot);
                let opaque = opaque_inputs(&mut opaque, input, actuals, inputs)?;
                let mut captured = CapturedValue::default();
                opaque.add_uncertain_dependencies(&mut captured, true);
                bind_result(&mut ops, dest, captured);
            }
            continue;
        };
        let inputs = lane.opaque_inputs(slot);
        let opaque = opaque_inputs(&mut opaque, input, actuals, inputs)?;
        let mut values = opaque.values(ty);
        if opaque.has_inputs()
            || multiple
            || matches!(dest, HirLocalDestination::Update(_))
            || !heap_children(input.context, ty)?.is_empty()
        {
            values.extend(opaque.add_result_type(input, ty, outcome, &mut ops)?);
        }
        let mut captured = CapturedValue::new(values, Vec::new());
        opaque.add_uncertain_dependencies(&mut captured, false);
        bind_result(&mut ops, dest, captured);
    }
    Ok(ops)
}

fn append_scalar_results(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    results: impl Iterator<Item = HirLocalDestination>,
    facts: &CallLaneSummary,
    ops: &mut Vec<InvokeLayerOp>,
) -> Result<(), BorrowCheckError> {
    let mut captures = PathCaptures::default();
    let mut possible = None;
    for (slot, dest) in results.enumerate() {
        if result_type(input, dest)?.is_some() {
            continue;
        }
        let mut alternatives = Vec::new();
        let rows = result_root_rows(&facts.sources, slot);
        let mut unresolved = rows.is_empty();
        for row in rows {
            if let CallAllocationSource::Parameter(source) = &row.source {
                let value = captures.get(input, actuals, source.index, &source.path)?;
                if value.alternatives.is_empty() {
                    alternatives.push(ValueAlt::plain(PayloadKind::Scalar));
                } else {
                    alternatives.extend_from_slice(&value.alternatives);
                }
            } else {
                unresolved = true;
            }
        }
        if unresolved {
            if possible.is_none() {
                possible = Some(possible_dependencies(input, actuals, Inputs::Any)?);
            }
            let deps = possible
                .as_ref()
                .expect("unresolved output captures possible sources");
            if !deps.is_empty() {
                alternatives.push(ValueAlt::plain(PayloadKind::Scalar));
                alternatives.push(ValueAlt {
                    kind: PayloadKind::Unknown,
                    deps: deps.clone(),
                });
            }
        }
        if alternatives.iter().any(|alt| !alt.deps.is_empty()) {
            bind_result(
                ops,
                dest,
                CapturedValue {
                    alternatives,
                    ..CapturedValue::default()
                }
                .into_scalar(),
            );
        }
    }
    Ok(())
}

fn unresolved_scalar(deps: &DependencySet) -> CapturedValue {
    CapturedValue {
        alternatives: vec![ValueAlt {
            kind: PayloadKind::Scalar,
            deps: deps.clone(),
        }],
        ..Default::default()
    }
}

/// The allocation summary can describe a store without recording scalar writes. Replay the
/// omitted scalar destinations as may writes; an absent allocation row cannot kill anything.
fn append_uncertain_scalar_stores(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    facts: &CallLaneSummary,
    ops: &mut Vec<InvokeLayerOp>,
) -> Result<(), BorrowCheckError> {
    if !actuals.args.iter().any(CallArgumentTransfer::may_write) {
        return Ok(());
    }
    let deps = possible_dependencies(input, actuals, Inputs::Any)?;
    if deps.is_empty() {
        return Ok(());
    }
    for index in 0..actuals.args.len() {
        if !actuals.args[index].may_write() {
            continue;
        }
        let ty = actuals.ty(input, index)?;
        if !heap_type(input.context, ty)? {
            if !has_scalar_store(facts, index, &[])
                && let Some(place) = actuals.entry_place(input, index)?
                && !place.address.cells().is_empty()
            {
                ops.push(InvokeLayerOp::Store(TypedStore::write_cells(
                    place.address.clone(),
                    unresolved_scalar(&deps),
                )));
            }
            continue;
        }
        let root = ResolvedPlace::from_capture(actuals.root(input, index)?);
        visit_uncertain_scalar_stores(input, facts, index, ty, root, &mut Vec::new(), &deps, ops)?;
    }
    Ok(())
}

fn has_scalar_store(facts: &CallLaneSummary, index: usize, fields: &[CallField]) -> bool {
    facts.stores.iter().any(|row| {
        matches!(&row.output,
        CallOutputPath::Store { target, path } if target.index == index
            && target.path.iter().chain(path).copied().eq(fields.iter().copied())
            && matches!(row.source, CallAllocationSource::Parameter(_)))
    })
}

#[allow(clippy::too_many_arguments)]
fn visit_uncertain_scalar_stores(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    facts: &CallLaneSummary,
    index: usize,
    ty: TypeId,
    holders: ResolvedPlace,
    path: &mut Vec<CallField>,
    deps: &DependencySet,
    ops: &mut Vec<InvokeLayerOp>,
) -> Result<(), BorrowCheckError> {
    if holders.ids.is_empty() {
        return Ok(());
    }
    CallField::for_each_child(ty, input.context.type_environment, |selector, _| {
        let (key, child) = typed_child(input.context, ty, selector)?;
        path.push(selector);
        if heap_type(input.context, child)? {
            let mut nested = holders.clone();
            step_graph(input, &mut nested, key);
            visit_uncertain_scalar_stores(input, facts, index, child, nested, path, deps, ops)?;
        } else if !has_scalar_store(facts, index, path) {
            ops.push(InvokeLayerOp::Store(TypedStore::write_projected(
                holders.ids.iter().copied(),
                key,
                Coverage::Partial,
                unresolved_scalar(deps),
                false,
            )));
        }
        path.pop();
        Ok(())
    })
}

/// The identity/forwarding boundary does not require a synthetic graph, even when an
/// immutable actual needs an explicit allocation capture to retain its permission.
fn direct_result_ops(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    mut results: impl Iterator<Item = HirLocalDestination>,
    facts: &CallLaneSummary,
    allow_plain_roots: bool,
) -> Result<Option<Vec<InvokeLayerOp>>, BorrowCheckError> {
    let Some(dest) = results.next() else {
        return Ok(None);
    };
    let Some(first) = facts.sources.first() else {
        return Ok(None);
    };
    if results.next().is_some()
        || !matches!(&first.output, CallOutputPath::Result(path) if path.slot == 0 && path.path.is_empty())
    {
        return Ok(None);
    }
    let Some(ty) = result_type(input, dest)? else {
        return Ok(Some(Vec::new()));
    };
    if matches!(first.source, CallAllocationSource::Fresh) && input.state.has_value_alternatives() {
        return Ok(None);
    }
    let CallAllocationSource::Parameter(param) = &first.source else {
        return Ok((facts.sources.len() == 1
            && matches!(first.source, CallAllocationSource::Fresh)
            && matches!(dest, HirLocalDestination::Define(_)))
        .then(Vec::new));
    };
    if !param.path.is_empty()
        || facts.sources[1..].iter().any(|row| {
            !matches!((&row.output, &row.source),
                (CallOutputPath::Result(result), CallAllocationSource::Parameter(source))
                    if result.slot == 0 && !result.path.is_empty()
                        && source.index == param.index && source.path == result.path)
        })
    {
        return Ok(None);
    }
    if actuals.ty(input, param.index)? != ty {
        return Err(input
            .context
            .diagnostics
            .internal_error("Argument and output allocation types differ at call site"));
    }
    let index = input
        .view
        .index_of(dest.local())
        .expect("result type lookup checked layout");
    if allow_plain_roots
        && !input
            .state
            .binding_mode(index)
            .contains(super::LocalMode::ALIAS)
        && preserves_plain_roots(input, actuals, index, param.index)?
        && (!input.state.has_value_alternatives()
            || actuals.root(input, param.index)?.alternatives.is_empty())
    {
        return Ok(Some(Vec::new()));
    }
    let captured = actuals.root(input, param.index)?;
    let mut ops = Vec::new();
    bind_result(&mut ops, dest, captured.clone());
    Ok(Some(ops))
}

fn result_type(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    dest: HirLocalDestination,
) -> Result<Option<TypeId>, BorrowCheckError> {
    let local = dest.local();
    let index = input.view.index_of(local).ok_or_else(|| {
        input
            .context
            .diagnostics
            .internal_error("Missing HIR call result local")
    })?;
    if !input.view.local_has_in_place_mutation_surface[index] {
        return Ok(None);
    }
    input
        .context
        .local_types
        .get(&local)
        .copied()
        .map(Some)
        .ok_or_else(|| {
            input
                .context
                .diagnostics
                .internal_error("Missing HIR call result type")
        })
}

pub(super) fn bind_result(
    ops: &mut Vec<InvokeLayerOp>,
    dest: HirLocalDestination,
    mut value: CapturedValue,
) {
    if !value.is_empty() {
        // Allocation permission is a meet over the possible result values.
        let readonly = value.readonly();
        value.restrict_permission(readonly);
        ops.push(InvokeLayerOp::Bind {
            local: dest.local(),
            value,
        });
    }
}

/// A direct same-root return needs no override when existing value roots already carry its
/// allocation and permission. Projected paths, alias destinations and rebinding a source
/// need the explicit capture instead.
fn preserves_plain_roots(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    destination: usize,
    index: usize,
) -> Result<bool, BorrowCheckError> {
    let argument = actuals.argument(input, index)?.argument;
    let expression = input.context.expressions.expression(argument);
    let HirExpressionKind::Load(place) = &expression.kind else {
        return Ok(false);
    };
    let Some(source) = input.view.index_of(place.root) else {
        return Ok(false);
    };
    if !place.projections.is_empty()
        || expression.value_kind == ValueKind::Const
        || !input.view.local_mutable[source]
    {
        return Ok(false);
    }
    let Some(place) = actuals.entry_place(input, index)? else {
        return Ok(false);
    };
    input
        .state
        .load_place_content(place, input.view.local_mutable, false);
    let content = place
        .content
        .as_ref()
        .expect("forwarded place requests content");
    Ok(!content.layer_only
        && !content.readonly
        && content.ids.iter().all(|&id| {
            input
                .state
                .alloc_origin_local(id)
                .is_none_or(|origin| origin != destination && input.view.local_mutable[origin])
        }))
}

fn heap_type(context: &BorrowTransferContext<'_>, ty: TypeId) -> Result<bool, BorrowCheckError> {
    Ok(classify_hir_type(ty, context.type_environment)?.has_in_place_mutation_surface())
}

/// Resolve arguments on demand and share their entry values across both Invoke lanes.
pub(super) struct CallActuals<'a> {
    args: &'a [CallArgumentTransfer],
    entries: Vec<Option<CallActual>>,
    derivations: Vec<RootSet>,
}

struct CallActual {
    place: Option<PlaceResolution>,
    is_copy: bool,
    value: Option<CapturedValue>,
}

impl<'a> CallActuals<'a> {
    pub(super) fn new(args: &'a [CallArgumentTransfer]) -> Self {
        Self {
            args,
            entries: (0..args.len()).map(|_| None).collect(),
            derivations: Vec::new(),
        }
    }

    pub(super) fn capture_derivations(
        &mut self,
        input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    ) {
        self.derivations.extend(self.args.iter().map(|arg| {
            super::alias_derivation_roots_from_expression(
                input.view,
                input.state,
                input.context.expressions,
                arg.argument,
            )
        }));
    }

    fn argument(
        &self,
        input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
        index: usize,
    ) -> Result<&CallArgumentTransfer, BorrowCheckError> {
        self.args.get(index).ok_or_else(|| {
            input
                .context
                .diagnostics
                .internal_error("Allocation summary names a missing argument")
        })
    }

    fn entry(
        &mut self,
        input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
        index: usize,
    ) -> Result<&mut CallActual, BorrowCheckError> {
        let argument = self.argument(input, index)?.argument;
        Ok(self.entries[index].get_or_insert_with(|| {
            let expressions = input.context.expressions;
            let resolve = |place| resolve_place(input.view, input.state, expressions, place);
            let (place, is_copy) = match &expressions.expression(argument).kind {
                HirExpressionKind::Load(place) => (resolve(place), false),
                HirExpressionKind::Copy(place) => (resolve(place), true),
                _ => (None, false),
            };
            CallActual {
                place,
                is_copy,
                value: None,
            }
        }))
    }

    pub(super) fn entry_place(
        &mut self,
        input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
        index: usize,
    ) -> Result<Option<&mut PlaceResolution>, BorrowCheckError> {
        let entry = self.entry(input, index)?;
        Ok(entry.place.as_mut().filter(|_| !entry.is_copy))
    }

    pub(super) fn entry_copy_source(
        &mut self,
        input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
        index: usize,
    ) -> Result<Option<&mut PlaceResolution>, BorrowCheckError> {
        let entry = self.entry(input, index)?;
        Ok(entry.place.as_mut().filter(|_| entry.is_copy))
    }

    fn root_entry(
        &mut self,
        input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
        index: usize,
    ) -> Result<&CallActual, BorrowCheckError> {
        let argument = self.argument(input, index)?.argument;
        let entry = self.entry(input, index)?;
        if entry.value.is_none() {
            let expression = input.context.expressions.expression(argument);
            let heap = heap_type(input.context, expression.ty)?;
            let value = if !heap && !input.state.has_value_alternatives() {
                CapturedValue::default()
            } else if expression.value_kind != ValueKind::Const
                && !entry.is_copy
                && let Some(place) = entry.place.as_mut()
            {
                input
                    .state
                    .load_place_content(place, input.view.local_mutable, !heap);
                let content = place
                    .content
                    .as_ref()
                    .expect("captured place requests content");
                let captured = CapturedValue {
                    values: stored_allocation_values(
                        input.view.local_mutable,
                        input.state,
                        content,
                        place.permission.wrapper_readonly_source.is_none(),
                    ),
                    views: content.views.clone(),
                    alternatives: content.alternatives.clone(),
                };
                if heap {
                    captured
                } else {
                    captured.into_scalar()
                }
            } else if heap
                && entry.is_copy
                && let Some(source) = entry.place.as_mut()
            {
                let source = if input.state.has_alloc_edges() {
                    input
                        .state
                        .load_place_content(source, input.view.local_mutable, false);
                    source.content.as_ref()
                } else {
                    None
                };
                capture_copy(
                    &LayerValueContext {
                        context: input.context,
                        view: input.view,
                        block_id: input.block_id,
                    },
                    input.state,
                    argument,
                    source,
                    true,
                )
            } else {
                stored_values(
                    &LayerValueContext {
                        context: input.context,
                        view: input.view,
                        block_id: input.block_id,
                    },
                    input.state,
                    argument,
                    true,
                )?
            };
            entry.value = Some(value);
        }
        Ok(entry)
    }

    pub(super) fn root(
        &mut self,
        input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
        index: usize,
    ) -> Result<&CapturedValue, BorrowCheckError> {
        let actual = self.root_entry(input, index)?;
        Ok(actual
            .value
            .as_ref()
            .expect("demanded entry value was captured"))
    }

    /// A replacement writes argument storage. A raw rvalue has only an entry value.
    fn capture_cell_store(
        &mut self,
        input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
        index: usize,
        node: &LaneNode<'_>,
    ) -> Result<Option<InvokeLayerOp>, BorrowCheckError> {
        let argument = self.argument(input, index)?.argument;
        let expression = input.context.expressions.expression(argument);
        if expression.value_kind != ValueKind::Place
            || !matches!(&expression.kind, HirExpressionKind::Load(_))
        {
            return Ok(None);
        }
        let Some(place) = self.entry_place(input, index)? else {
            return Ok(None);
        };
        let addresses = &place.address;
        Ok((!addresses.cells().is_empty()).then(|| {
            InvokeLayerOp::Store(TypedStore::write_cells(addresses.clone(), node.capture()))
        }))
    }

    fn ty(
        &self,
        input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
        index: usize,
    ) -> Result<TypeId, BorrowCheckError> {
        let argument = self.argument(input, index)?.argument;
        Ok(input.context.expressions.expression(argument).ty)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ActualPath<'a> {
    index: usize,
    path: &'a [CallField],
}

#[derive(Default)]
struct PathCaptures<'a>(FxHashMap<ActualPath<'a>, CapturedValue>);

impl<'a> PathCaptures<'a> {
    fn get<'b>(
        &'b mut self,
        input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
        actuals: &'b mut CallActuals<'_>,
        index: usize,
        path: &'a [CallField],
    ) -> Result<&'b CapturedValue, BorrowCheckError> {
        if path.is_empty() {
            return actuals.root(input, index);
        }
        let key = ActualPath { index, path };
        match self.0.entry(key) {
            std::collections::hash_map::Entry::Occupied(entry) => Ok(entry.into_mut()),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let mut ty = actuals.ty(input, index)?;
                let actual = actuals.root_entry(input, index)?;
                let value = actual
                    .value
                    .as_ref()
                    .expect("demanded entry value was captured");
                let mut graph = ResolvedPlace::from_capture(value);
                if !actual.is_copy
                    && let Some(content) = actual
                        .place
                        .as_ref()
                        .and_then(|place| place.content.as_ref())
                {
                    graph.coverage = content.coverage;
                    graph.may_be_absent = content.may_be_absent;
                }
                for (index, &field) in path.iter().enumerate() {
                    let (key, child) = typed_child(input.context, ty, field)?;
                    if index + 1 == path.len()
                        && input.state.has_value_alternatives()
                        && !heap_type(input.context, child)?
                    {
                        return Ok(entry.insert(input.state.capture_scalar_projection(&graph, key)));
                    }
                    step_graph(input, &mut graph, key);
                    ty = child;
                }
                Ok(entry.insert(capture_graph_values(graph)))
            }
        }
    }
}

fn typed_child(
    context: &BorrowTransferContext<'_>,
    ty: TypeId,
    step: CallField,
) -> Result<(FieldKey, TypeId), BorrowCheckError> {
    let Some((step, ty)) = step.hir_child(
        ty,
        context.type_environment,
        context.structs,
        context.choices,
    ) else {
        return Err(context
            .diagnostics
            .internal_error("Allocation summary path does not match caller type"));
    };
    Ok((field_key(step), ty))
}

fn field_key(step: CallFieldHir) -> FieldKey {
    match step {
        CallFieldHir::Field(id) => FieldKey::Field(id),
        CallFieldHir::UnmaterializedField(ordinal) => FieldKey::UnmaterializedField(ordinal),
        CallFieldHir::Payload { variant, field } => FieldKey::Payload { variant, field },
        CallFieldHir::Elem => FieldKey::Elem,
        CallFieldHir::MapKey => FieldKey::MapKey,
    }
}

fn typed_path(
    context: &BorrowTransferContext<'_>,
    mut ty: TypeId,
    path: &[CallField],
) -> Result<TypeId, BorrowCheckError> {
    for &step in path {
        ty = typed_child(context, ty, step)?.1;
    }
    Ok(ty)
}

struct LaneNode<'a> {
    route: &'a CallOutputPath,
    ty: TypeId,
    values: Vec<StoredValue>,
    alternatives: ValueAlternatives,
    inherited: Vec<AllocId>,
    view: Option<AllocId>,
    nonsingular: bool,
    /// A projected parent that came from a parameter already has its own graph.
    /// Only these call-owned alternatives receive synthetic child edges.
    fresh: Vec<AllocId>,
}

impl LaneNode<'_> {
    fn capture(&self) -> CapturedValue {
        CapturedValue {
            values: self.values.clone(),
            views: output_views(self),
            alternatives: self.alternatives.clone(),
        }
    }

    fn contribute(&mut self, value: &CapturedValue) {
        if self.alternatives.is_empty() && !value.alternatives.is_empty() {
            self.alternatives
                .extend(self.values.iter().copied().map(allocation_alt));
        }
        if !self.alternatives.is_empty() || !value.alternatives.is_empty() {
            if value.alternatives.is_empty() {
                self.alternatives
                    .extend(value.values.iter().copied().map(allocation_alt));
            } else {
                self.alternatives.extend_from_slice(&value.alternatives);
            }
        }
        self.values.extend_from_slice(&value.values);
    }

    fn add_fresh(&mut self, id: AllocId, writable: bool) {
        let value = StoredValue { id, writable };
        self.fresh.push(id);
        self.values.push(value);
        self.inherited.push(id);
        if !self.alternatives.is_empty() {
            self.alternatives.push(allocation_alt(value));
        }
    }
}

fn allocation_alt(value: StoredValue) -> ValueAlt {
    ValueAlt::plain(PayloadKind::Allocation(value))
}

fn instantiate_lane(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    results: impl Iterator<Item = HirLocalDestination> + Clone,
    facts: &CallLaneSummary,
    outcome: InvokeLane,
) -> Result<Vec<InvokeLayerOp>, BorrowCheckError> {
    if facts.sources.is_empty() && facts.stores.is_empty() {
        return Ok(Vec::new());
    }
    let mut captures = PathCaptures::default();
    let mut nodes: Vec<LaneNode<'_>> = Vec::new();
    let mut ops = Vec::new();
    for row in facts.sources.iter().chain(&facts.stores) {
        let output = &row.output;
        if !nodes.last().is_some_and(|node| node.route == output) {
            let (root_ty, path) = match output {
                CallOutputPath::Result(result) => {
                    let dest = results.clone().nth(result.slot).ok_or_else(|| {
                        input
                            .context
                            .diagnostics
                            .internal_error("Allocation summary names a missing result")
                    })?;
                    let Some(ty) = result_type(input, dest)? else {
                        continue;
                    };
                    (ty, result.path.as_slice())
                }
                CallOutputPath::Store { target, path } => {
                    let ty = actuals.ty(input, target.index)?;
                    (
                        typed_path(input.context, ty, &target.path)?,
                        path.as_slice(),
                    )
                }
            };
            nodes.push(LaneNode {
                route: output,
                ty: typed_path(input.context, root_ty, path)?,
                values: Vec::new(),
                alternatives: Vec::new(),
                fresh: Vec::new(),
                inherited: Vec::new(),
                view: None,
                nonsingular: output.has_collapsed_selector(),
            });
        }
        let site = nodes.len() as u32 - 1;
        let node = nodes.last_mut().expect("source endpoint was just inserted");
        match &row.source {
            CallAllocationSource::Parameter(path) => {
                let actual_ty = actuals.ty(input, path.index)?;
                if typed_path(input.context, actual_ty, &path.path)? != node.ty {
                    return Err(input.context.diagnostics.internal_error(
                        "Argument and output allocation types differ at call site",
                    ));
                }
                let values = captures.get(input, actuals, path.index, &path.path)?;
                node.inherited.extend(captured_occurrences(values));
                node.contribute(values);
            }
            CallAllocationSource::Fresh | CallAllocationSource::OpaqueFresh => {
                let opaque = matches!(row.source, CallAllocationSource::OpaqueFresh);
                let nonsingular = node.nonsingular || opaque;
                node.nonsingular = nonsingular;
                let site = if opaque {
                    site + (facts.sources.len() + facts.stores.len() + facts.sharing.len()) as u32
                } else {
                    site
                };
                let id = input.state.call_site_id(
                    input.current_order,
                    matches!(outcome, InvokeLane::Error),
                    site,
                    nonsingular,
                    input.block_id,
                    input.view.block_successors,
                );
                node.add_fresh(id, true);
            }
        }
    }
    let find = |nodes: &[LaneNode<'_>], endpoint: &CallOutputPath| {
        nodes.binary_search_by(|node| node.route.cmp(endpoint)).ok()
    };
    let mut permitted = FxHashSet::default();
    for (ordinal, pair) in facts.sharing.iter().enumerate() {
        let (Some(left), Some(right)) = (find(&nodes, &pair.left), find(&nodes, &pair.right))
        else {
            continue;
        };
        let may_share = match &pair.condition {
            CallSharingCondition::Always => true,
            CallSharingCondition::Parameters(a, b) => {
                let first = captures.get(input, actuals, a.index, &a.path)?.clone();
                let second = captures.get(input, actuals, b.index, &b.path)?;
                let left = first
                    .values
                    .iter()
                    .map(|value| value.id)
                    .collect::<Vec<_>>();
                let right = second
                    .values
                    .iter()
                    .map(|value| value.id)
                    .collect::<Vec<_>>();
                input
                    .state
                    .alloc_paths_may_share(&left, &first.views, &right, &second.views)
            }
        };
        if !may_share || !permitted.insert((left, right)) {
            continue;
        }
        if !nodes[left].fresh.is_empty() && !nodes[right].fresh.is_empty() {
            let id = input.state.call_site_id(
                input.current_order,
                matches!(outcome, InvokeLane::Error),
                (facts.sources.len() + facts.stores.len() + ordinal) as u32,
                nodes[left].nonsingular || nodes[right].nonsingular,
                input.block_id,
                input.view.block_successors,
            );
            for index in [left, right] {
                let node = &mut nodes[index];
                let writable = node.values.iter().all(|value| value.writable);
                node.add_fresh(id, writable);
            }
        }
    }

    // A repeated site's marginal allocations may refer to different evaluations.
    let correlate = nodes.len() > 1
        && !input
            .state
            .block_may_repeat(input.block_id, input.view.block_successors);
    let mut inverse = FxHashMap::<AllocId, Vec<usize>>::default();
    for (index, node) in nodes.iter_mut().enumerate() {
        node.values
            .sort_unstable_by_key(|value| (value.id, value.writable));
        node.values.dedup_by_key(|value| value.id);
        node.inherited.sort_unstable();
        node.inherited.dedup();
        if correlate {
            for value in &node.values {
                inverse.entry(value.id).or_default().push(index);
            }
        }
    }
    let mut restricted = FxHashSet::default();
    for endpoints in inverse.values() {
        for (at, &left) in endpoints.iter().enumerate() {
            for &right in &endpoints[at + 1..] {
                if !permitted.contains(&(left, right)) {
                    restricted.insert((left, right));
                }
            }
        }
    }
    if !restricted.is_empty() {
        // Concrete sets keep caller aliases for uncopied reachability. A separate view
        // names the outcome occurrence when paths with overlapping sets do not share.
        for (ordinal, node) in nodes.iter_mut().enumerate() {
            if node.values.is_empty() {
                continue;
            }
            let view = input.state.call_view_id(
                input.current_order,
                matches!(outcome, InvokeLane::Error),
                ordinal as u32,
            );
            node.view = Some(view);
            for value in &node.values {
                ops.push(InvokeLayerOp::ViewMember { view, id: value.id });
            }
        }
        let mut pairs: Vec<_> = restricted
            .into_iter()
            .map(|pair| (pair, false))
            .chain(permitted.into_iter().map(|pair| (pair, true)))
            .collect();
        pairs.sort_unstable();
        for ((left, right), may_share) in pairs {
            if let (Some(left), Some(right)) = (nodes[left].view, nodes[right].view) {
                ops.push(InvokeLayerOp::ViewPair {
                    left,
                    right,
                    may_share,
                });
            }
        }
    }

    for node in &nodes {
        let (parent, last) = match node.route {
            CallOutputPath::Result(result) => {
                let Some((&last, parent)) = result.path.split_last() else {
                    continue;
                };
                (
                    CallOutputPath::Result(
                        crate::compiler_frontend::public_call_summary::CallResultPath {
                            slot: result.slot,
                            path: parent.to_vec(),
                        },
                    ),
                    last,
                )
            }
            CallOutputPath::Store { target, path } => {
                let Some((&last, parent)) = path.split_last() else {
                    let Some((&key, holders)) = target.path.split_last() else {
                        ops.extend(actuals.capture_cell_store(input, target.index, node)?);
                        continue;
                    };
                    let ty = actuals.ty(input, target.index)?;
                    let ty = typed_path(input.context, ty, holders)?;
                    let (key, _) = typed_child(input.context, ty, key)?;
                    let holders = captures.get(input, actuals, target.index, holders)?;
                    ops.push(InvokeLayerOp::Store(TypedStore::write_projected(
                        holders.values.iter().map(|holder| holder.id),
                        key,
                        Coverage::Partial,
                        node.capture(),
                        false,
                    )));
                    continue;
                };
                (
                    CallOutputPath::Store {
                        target: target.clone(),
                        path: parent.to_vec(),
                    },
                    last,
                )
            }
        };
        if let Some(parent) = find(&nodes, &parent).map(|index| &nodes[index]) {
            let (key, _) = typed_child(input.context, parent.ty, last)?;
            let views = output_views(node);
            if let Some(holder) = parent.view {
                for &view in &views {
                    ops.push(InvokeLayerOp::ViewField { holder, key, view });
                }
            }
            for &holder in &parent.fresh {
                if parent.view.is_none() && !views.is_empty() || !node.alternatives.is_empty() {
                    // Rewrapping a projected actual carries its occurrence with the edge.
                    ops.push(InvokeLayerOp::ConstructField {
                        holder,
                        key,
                        value: CapturedValue {
                            values: node.values.clone(),
                            views: if parent.view.is_none() {
                                views.clone()
                            } else {
                                Vec::new()
                            },
                            alternatives: node.alternatives.clone(),
                        },
                    });
                } else {
                    for &value in &node.values {
                        ops.push(InvokeLayerOp::Contribute { holder, key, value });
                    }
                }
            }
        }
    }
    if input.state.has_value_alternatives() && nodes.iter().any(|node| !node.fresh.is_empty()) {
        let deps = possible_dependencies(input, actuals, Inputs::Any)?;
        if !deps.is_empty() {
            for node in &nodes {
                if node.fresh.is_empty() || !heap_type(input.context, node.ty)? {
                    continue;
                }
                append_fresh_scalar_fields(
                    input,
                    &nodes,
                    node.route,
                    node.ty,
                    &node.fresh,
                    &deps,
                    &mut ops,
                )?;
            }
        }
    }
    for node in nodes {
        let CallOutputPath::Result(result) = node.route else {
            continue;
        };
        if !result.path.is_empty() {
            continue;
        }
        let dest = results
            .clone()
            .nth(result.slot)
            .expect("allocation summary result slot must name a destination");
        // The direct-root fast path above handles elision. A shaped return keeps its
        // pre-call allocation captures even when a second result also retires their source.
        let views = output_views(&node);
        bind_result(
            &mut ops,
            dest,
            CapturedValue {
                values: node.values,
                views,
                alternatives: node.alternatives,
            },
        );
    }
    Ok(ops)
}

/// Only synthesised allocations lack field captures. Parameter-derived alternatives keep their
/// own graphs; an explicitly supplied child is handled at its own allocation-summary node.
fn append_fresh_scalar_fields(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    nodes: &[LaneNode<'_>],
    parent: &CallOutputPath,
    ty: TypeId,
    holders: &[AllocId],
    deps: &DependencySet,
    ops: &mut Vec<InvokeLayerOp>,
) -> Result<(), BorrowCheckError> {
    CallField::for_each_child(ty, input.context.type_environment, |selector, _| {
        let (key, child) = typed_child(input.context, ty, selector)?;
        let mut route = parent.clone();
        match &mut route {
            CallOutputPath::Result(result) => result.path.push(selector),
            CallOutputPath::Store { path, .. } => path.push(selector),
        }
        if nodes.iter().any(|node| *node.route == route) {
            return Ok(());
        }
        if heap_type(input.context, child)? {
            let nested: Vec<_> = holders
                .iter()
                .map(|&holder| input.state.alloc_child(holder, key))
                .collect();
            append_fresh_scalar_fields(input, nodes, &route, child, &nested, deps, ops)?;
        } else {
            ops.extend(holders.iter().map(|&holder| InvokeLayerOp::ConstructField {
                holder,
                key,
                value: unresolved_scalar(deps),
            }));
        }
        Ok(())
    })
}

fn captured_occurrences(value: &CapturedValue) -> impl Iterator<Item = AllocId> + '_ {
    value.views.iter().copied().chain(
        value
            .values
            .iter()
            .filter(|_| value.views.is_empty())
            .map(|value| value.id),
    )
}

fn output_views(node: &LaneNode<'_>) -> Vec<AllocId> {
    if let Some(view) = node.view {
        vec![view]
    } else if node
        .inherited
        .iter()
        .any(|id| !node.values.iter().any(|value| value.id == *id))
    {
        node.inherited.clone()
    } else {
        Vec::new()
    }
}

fn step(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    values: &CapturedValue,
    key: FieldKey,
) -> CapturedValue {
    let mut place = ResolvedPlace::from_capture(values);
    step_graph(input, &mut place, key);
    capture_graph_values(place)
}

fn step_graph(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    graph: &mut ResolvedPlace,
    key: FieldKey,
) {
    input
        .state
        .step_resolved(graph, key, input.view.local_mutable);
    input
        .state
        .mark_alloc_provenance_readonly(input.view.local_mutable, graph);
}

pub(super) fn capture_graph_values(place: ResolvedPlace) -> CapturedValue {
    CapturedValue {
        values: place
            .ids
            .into_iter()
            .map(|id| StoredValue {
                id,
                writable: !place.readonly,
            })
            .collect(),
        views: place.views,
        alternatives: place.alternatives,
    }
}

fn opaque_inputs<'a, 'b>(
    cache: &'b mut FxHashMap<Inputs<'a>, OpaqueInputs>,
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    sources: Inputs<'a>,
) -> Result<&'b mut OpaqueInputs, BorrowCheckError> {
    let scope = cache.len() as u32 + 1;
    match cache.entry(sources) {
        std::collections::hash_map::Entry::Occupied(entry) => Ok(entry.into_mut()),
        std::collections::hash_map::Entry::Vacant(entry) => {
            Ok(entry.insert(OpaqueInputs::new(input, actuals, sources, scope)?))
        }
    }
}

fn possible_dependencies(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    sources: Inputs<'_>,
) -> Result<DependencySet, BorrowCheckError> {
    let mut dependencies = DependencySet::new();
    if input.state.has_value_alternatives() {
        for index in 0..actuals.args.len() {
            match sources {
                Inputs::Fresh => break,
                Inputs::Args(indices) if !indices.contains(&index) => continue,
                _ => {}
            }
            let value = actuals.root(input, index)?;
            input
                .state
                .extend_possible_dependencies(value, &mut dependencies);
        }
        dependencies.sort_unstable();
        dependencies.dedup();
    }
    Ok(dependencies)
}

/// Without a validated shape, a result path may contain any compatible input path or an
/// unknown allocation. Both are separate from allocations of other types. The latter is one
/// nonsingular node per call site and type; opaque results in that class may share it.
#[derive(Default)]
struct OpaqueInputs {
    inputs: FxHashMap<TypeId, Vec<StoredValue>>,
    dependencies: DependencySet,
    built: FxHashSet<TypeId>,
    scope: u32,
}

impl OpaqueInputs {
    fn new(
        input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
        actuals: &mut CallActuals<'_>,
        sources: Inputs<'_>,
        scope: u32,
    ) -> Result<Self, BorrowCheckError> {
        let mut this = Self {
            scope,
            dependencies: possible_dependencies(input, actuals, sources)?,
            ..Self::default()
        };
        let mut active = FxHashSet::default();
        for index in 0..actuals.args.len() {
            match sources {
                Inputs::Fresh => break,
                Inputs::Args(indices) if !indices.contains(&index) => continue,
                _ => {}
            }
            let ty = actuals.ty(input, index)?;
            if heap_type(input.context, ty)? {
                let values = actuals.root(input, index)?.clone();
                this.visit_input(input, ty, values, &mut active)?;
            }
        }
        Ok(this)
    }

    fn add_uncertain_dependencies(&self, value: &mut CapturedValue, scalar: bool) {
        if !self.dependencies.is_empty() {
            value
                .alternatives
                .extend(value.values.iter().copied().map(allocation_alt));
            if scalar {
                value
                    .alternatives
                    .push(ValueAlt::plain(PayloadKind::Scalar));
            }
            value.alternatives.push(ValueAlt {
                kind: PayloadKind::Unknown,
                deps: self.dependencies.clone(),
            });
        }
    }

    fn has_inputs(&self) -> bool {
        !self.inputs.is_empty()
    }

    fn values(&self, ty: TypeId) -> Vec<StoredValue> {
        self.inputs.get(&ty).cloned().unwrap_or_default()
    }

    fn visit_input(
        &mut self,
        input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
        ty: TypeId,
        mut values: CapturedValue,
        active: &mut FxHashSet<TypeId>,
    ) -> Result<(), BorrowCheckError> {
        let seen = self.inputs.entry(ty).or_default();
        values.values.retain(|value| {
            if seen.contains(value) {
                return false;
            }
            seen.push(*value);
            true
        });
        if values.values.is_empty() {
            return Ok(());
        }
        // Validated source/HIR does not have recursive value layouts. Don't produce an
        // unbounded sequence of lazy allocation IDs if that invariant is broken upstream.
        if !active.insert(ty) {
            return Err(input
                .context
                .diagnostics
                .internal_error("Recursive runtime layout reached opaque call provenance"));
        }
        for (key, child) in heap_children(input.context, ty)? {
            self.visit_input(input, child, step(input, &values, key), active)?;
        }
        active.remove(&ty);
        Ok(())
    }

    fn add_result_type(
        &mut self,
        input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
        ty: TypeId,
        outcome: InvokeLane,
        ops: &mut Vec<InvokeLayerOp>,
    ) -> Result<Vec<StoredValue>, BorrowCheckError> {
        let error = matches!(outcome, InvokeLane::Error);
        let detached = input
            .state
            .opaque_call_id(input.current_order, error, 0, ty.0);
        let connected = input
            .state
            .opaque_call_id(input.current_order, error, self.scope, ty.0);
        if self.built.insert(ty) {
            for (key, child) in heap_children(input.context, ty)? {
                let mut unknown = self
                    .add_result_type(input, child, outcome, ops)?
                    .into_iter();
                let fresh = unknown
                    .next()
                    .expect("every opaque type has a fresh alternative");
                ops.push(InvokeLayerOp::Contribute {
                    holder: detached,
                    key,
                    value: fresh,
                });
                if self.has_inputs() {
                    for value in self.values(child).into_iter().chain([fresh]).chain(unknown) {
                        ops.push(InvokeLayerOp::Contribute {
                            holder: connected,
                            key,
                            value,
                        });
                    }
                }
            }
        }
        let mut values = vec![StoredValue {
            id: detached,
            writable: true,
        }];
        if self.has_inputs() {
            values.push(StoredValue {
                id: connected,
                writable: true,
            });
        }
        Ok(values)
    }
}

fn heap_children(
    context: &BorrowTransferContext<'_>,
    ty: TypeId,
) -> Result<Vec<(FieldKey, TypeId)>, BorrowCheckError> {
    let mut fields = Vec::new();
    let structure = context.type_environment.fields_for(ty).and_then(|_| {
        context
            .structs
            .iter()
            .find(|layout| layout.frontend_type_id == ty)
    });
    let choice = context.type_environment.variants_for(ty).and_then(|_| {
        context
            .choices
            .iter()
            .find(|layout| layout.frontend_type_id == ty)
    });
    // The HIR layout supplies the caller's field identity and lowered child type.
    CallField::for_each_child::<BorrowCheckError>(ty, context.type_environment, |selector, _| {
        let Some((key, child)) =
            selector.hir_child_with_layout(ty, context.type_environment, structure, choice)
        else {
            return Err(context
                .diagnostics
                .internal_error("Opaque call selector does not match caller type"));
        };
        if heap_type(context, child)? {
            fields.push((field_key(key), child));
        }
        Ok(())
    })?;
    Ok(fields)
}
