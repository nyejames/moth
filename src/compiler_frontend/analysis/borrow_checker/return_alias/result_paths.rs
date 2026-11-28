//! Index demanded call-boundary supports and export their canonical transfer states.

use super::super::{
    collect_statement_defined_locals, collect_statement_loaded_locals,
    collect_statement_written_locals, collect_terminator_loaded_locals,
};
use super::{BorrowCheckError, BorrowChecker};
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AllocId, BindingSpec, BorrowState, CapturedValue, Coverage, OrderKey, RootSet, TypedStore,
    UNKNOWN_ORDER_KEY, ValueAlt, ValueKind as PayloadKind, cyclic_blocks,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::call_semantics::{
    BorrowedCall, CallReturns,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::typed_call::{
    EntryCapture, boundary_child, capture_boundary_value, capture_local_value_port,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::{
    OperationView, SparseBlockView,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::{
    call_member_result, call_member_writes,
};
use crate::compiler_frontend::analysis::borrow_checker::{state, transfer};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalReturnAlias};
use crate::compiler_frontend::hir::expression_store::{HirExpressionStore, HirProjection};
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirMapOp, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;
use crate::compiler_frontend::hir::ids::{
    BlockId, FieldId, FunctionId, HirValueId, LocalId, RegionId,
};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::hir::utils::for_each_terminator_target;
use crate::compiler_frontend::instrumentation::frontend_counters::{
    FrontendCounter, add_frontend_counter,
};
use crate::compiler_frontend::public_call_summary::typed::{
    self, EntryPort, IdentityKind, PortId, Predicate, ReachKind, Role, SupportId,
};
use crate::compiler_frontend::public_call_summary::{
    CallField, CallParameterPath, CallResultPath, PublicCallParameterAccess,
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::Bound::{Included, Unbounded};
use std::rc::Rc;

#[cfg(test)]
#[path = "result_paths_tests.rs"]
mod result_paths_tests;

/// A HIR revision owns these coordinates. Published contracts use the typed
/// ports, not the local, operation or block IDs of the producer.
pub(in crate::compiler_frontend::analysis::borrow_checker) struct SparseFunction {
    pub(super) ports: Vec<EntryPort>,
    pub(super) port_supports: Vec<SupportId>,
    pub(super) support_count: usize,
    pub callees: Vec<FunctionId>,
    result_supports: BTreeMap<(bool, usize), SupportId>,
    ports_by_support: Vec<Vec<PortId>>,
    pub(super) guards: Vec<Predicate>,
    pub(super) compatibility: Vec<CompatibilityConstraint>,
    #[cfg(test)]
    pub(super) operations: BTreeMap<IndexedSite, Box<[SupportId]>>,
    operations_by_support: BTreeMap<(SupportId, BlockId), Box<[usize]>>,
    call_components: BTreeMap<(SupportId, BlockId), BTreeMap<usize, Vec<SupportId>>>,
    #[cfg(test)]
    pub(super) conditional_supports: BTreeSet<SupportId>,
    #[cfg(test)]
    pub(super) conditional_arm_count: usize,
    pub(super) counts: IndexCounts,
    transfer: Rc<SparseTransferIndex>,
    envelope: BTreeMap<(usize, Box<[CallField]>, DemandKind), SupportId>,
    result_sources: BTreeMap<(bool, usize), ResultSources>,
    total: bool,
}

/// Transfer coordinates are unchanged when a callee's structural envelope widens.
struct SparseTransferIndex {
    local_ids: Vec<LocalId>,
    local_indices: FxHashMap<LocalId, usize>,
    parameters: FxHashMap<LocalId, usize>,
    mutable: Vec<bool>,
    heap: Vec<bool>,
    first_write: Vec<i32>,
    last_match_read: Vec<usize>,
    successors: FxHashMap<BlockId, Vec<BlockId>>,
    predecessors: FxHashMap<BlockId, Vec<BlockId>>,
    local_regions: Vec<RegionId>,
    exits_by_region: FxHashMap<RegionId, Vec<BlockId>>,
    recurring: Rc<FxHashSet<BlockId>>,
    ranks: Rc<FxHashMap<BlockId, usize>>,
    postdom_jumps: Vec<FxHashMap<BlockId, BlockId>>,
    structural_barriers: BTreeSet<usize>,
    acyclic_entry: Option<BlockId>,
    blocks: FxHashMap<BlockId, SparseBlockOrders>,
}

struct SparseBlockOrders {
    statements: Box<[i32]>,
    operands: Box<[Box<[usize]>]>,
    terminator: i32,
    structural_calls: Box<[(usize, usize)]>,
    structural_terminator: Option<usize>,
}

impl SparseTransferIndex {
    fn new(checker: &BorrowChecker<'_>, snapshot: &Snapshot<'_>) -> Result<Self, BorrowCheckError> {
        let blocks = || {
            snapshot
                .blocks
                .iter()
                .map(|id| &checker.module.blocks[checker.block_index_by_id[id]])
        };
        let mut locals: Vec<_> = blocks().flat_map(|block| &block.locals).collect();
        locals.sort_unstable_by_key(|local| local.id.0);
        let mut result = Self {
            local_ids: locals.iter().map(|local| local.id).collect(),
            local_indices: locals
                .iter()
                .enumerate()
                .map(|(i, local)| (local.id, i))
                .collect(),
            parameters: snapshot
                .function
                .params
                .iter()
                .copied()
                .enumerate()
                .map(|(index, local)| (local, index))
                .collect(),
            mutable: locals.iter().map(|local| local.mutable).collect(),
            heap: locals
                .iter()
                .map(|local| is_heap(checker, local.ty))
                .collect::<Result<_, _>>()?,
            first_write: vec![UNKNOWN_ORDER_KEY; locals.len()],
            last_match_read: vec![usize::MAX; locals.len()],
            successors: FxHashMap::default(),
            predecessors: FxHashMap::default(),
            local_regions: locals.iter().map(|local| local.region).collect(),
            exits_by_region: FxHashMap::default(),
            recurring: Rc::default(),
            acyclic_entry: Some(snapshot.function.entry),
            blocks: FxHashMap::default(),
            ranks: Rc::default(),
            postdom_jumps: Vec::new(),
            structural_barriers: BTreeSet::new(),
        };
        let mut prior_statements = 0;
        for (block_index, block) in blocks().enumerate() {
            // The preceding blocks each contribute one terminator in addition to their statements.
            let first_order = (prior_statements + block_index) as OrderKey;
            prior_statements += block.statements.len();
            let order = (prior_statements + block_index) as OrderKey;
            let mut first_write = |local, order| {
                let slot = &mut result.first_write[result.local_indices[&local]];
                if *slot == UNKNOWN_ORDER_KEY {
                    *slot = order;
                }
            };
            let mut operands = Vec::with_capacity(block.statements.len() + 1);
            let mut structural_calls = Vec::new();
            let statements = block
                .statements
                .iter()
                .enumerate()
                .map(|(position, statement)| {
                    let at = first_order + position as OrderKey;
                    if let HirStatementKind::Call { args, .. } = &statement.kind {
                        structural_calls
                            .push((position, checker.module.expressions.values(*args).len()));
                    }
                    collect_statement_written_locals(statement, &mut |local| {
                        first_write(local, at)
                    });
                    let mut slots = Vec::new();
                    let mut push = |local| slots.push(result.local_indices[&local]);
                    collect_statement_loaded_locals(
                        &checker.module.expressions,
                        statement,
                        &mut push,
                    );
                    collect_statement_defined_locals(statement, &mut push);
                    collect_statement_written_locals(statement, &mut push);
                    slots.sort_unstable();
                    slots.dedup();
                    operands.push(slots.into_boxed_slice());
                    at
                })
                .collect();
            let mut slots = Vec::new();
            let mut push = |local| slots.push(result.local_indices[&local]);
            collect_terminator_loaded_locals(
                &checker.module.expressions,
                &block.terminator,
                &mut push,
            );
            match &block.terminator {
                HirTerminator::Jump { args, .. } => {
                    for argument in args {
                        first_write(argument.destination, order);
                        push(argument.source);
                        push(argument.destination);
                    }
                }
                HirTerminator::Invoke {
                    success_results,
                    error_result,
                    ..
                } => {
                    for &local in success_results.iter().chain(std::iter::once(error_result)) {
                        first_write(local, order);
                        push(local);
                    }
                }
                _ => {}
            }
            slots.sort_unstable();
            slots.dedup();
            operands.push(slots.into_boxed_slice());
            let successors = result.successors.entry(block.id).or_default();
            for_each_terminator_target(&block.terminator, |next| successors.push(next));
            for &next in successors.iter() {
                result.predecessors.entry(next).or_default().push(block.id);
                let target_region = checker.module.blocks[checker.block_index_by_id[&next]].region;
                let mut source_region = block.region;
                while !checker.is_region_ancestor_of(source_region, target_region, next)? {
                    result
                        .exits_by_region
                        .entry(source_region)
                        .or_default()
                        .push(next);
                    let Some(parent) = checker.region_parent_by_id[&source_region] else {
                        break;
                    };
                    source_region = parent;
                }
            }
            if successors.contains(&snapshot.function.entry) {
                result.acyclic_entry = None;
            }
            result.blocks.insert(
                block.id,
                SparseBlockOrders {
                    statements,
                    operands: operands.into_boxed_slice(),
                    terminator: order,
                    structural_calls: structural_calls.into_boxed_slice(),
                    structural_terminator: match &block.terminator {
                        HirTerminator::NeverCall { args, .. }
                        | HirTerminator::Invoke {
                            operation: HirFallibleOperation::Call { args, .. },
                            ..
                        } => Some(checker.module.expressions.values(*args).len()),
                        _ => None,
                    },
                },
            );
        }
        for exits in result.exits_by_region.values_mut() {
            exits.sort_unstable_by_key(|block| block.0);
            exits.dedup();
        }
        result.ranks = Rc::new(
            crate::compiler_frontend::analysis::borrow_checker::engine::reverse_postorder_ranks(
                snapshot.function.entry,
                &result.successors,
            ),
        );
        for block in blocks() {
            let orders = &result.blocks[&block.id];
            if let HirTerminator::Match { .. } = block.terminator
                && let Some(&rank) = result.ranks.get(&block.id)
            {
                for &local in orders.operands.last().expect("terminator operands") {
                    let last = &mut result.last_match_read[local];
                    *last = (*last).min(rank);
                }
            }
            if matches!(block.terminator, HirTerminator::Uninitialized)
                || !orders.structural_calls.is_empty()
                || orders.structural_terminator.is_some()
            {
                result.structural_barriers.insert(result.ranks[&block.id]);
            }
        }
        let cfg = SparseCfg::new(checker, snapshot, |_| false, false, &mut 0);
        result.recurring = Rc::new(cyclic_blocks(&result.successors));
        if result.recurring.is_empty() {
            let parents = cfg.postdominators(&mut 0);
            let immediate: FxHashMap<_, _> = snapshot
                .blocks
                .iter()
                .copied()
                .enumerate()
                .filter_map(|(i, block)| snapshot.blocks.get(parents[i]).map(|&next| (block, next)))
                .collect();
            result.postdom_jumps.push(immediate);
            // Jump along the postdominator chain without walking each identity diamond.
            while let Some(previous) = result.postdom_jumps.last() {
                let next: FxHashMap<_, _> = previous
                    .iter()
                    .filter_map(|(&from, through)| previous.get(through).map(|&to| (from, to)))
                    .collect();
                if next.is_empty() {
                    break;
                }
                result.postdom_jumps.push(next);
            }
        }
        Ok(result)
    }
}

pub(in crate::compiler_frontend::analysis::borrow_checker) struct ProjectedSupport<'a> {
    function: &'a SparseFunction,
    support: SupportId,
    barriers: BTreeSet<usize>,
    visibility: FxHashMap<BlockId, RootSet>,
    last_use: Vec<usize>,
    replay_retirements: Vec<usize>,
    entry_roots: Option<RootSet>,
    #[cfg(test)]
    compress: bool,
    local_ids: Vec<LocalId>,
    local_indices: FxHashMap<LocalId, usize>,
    mutable: Vec<bool>,
    heap: Vec<bool>,
    first_write: Vec<i32>,
}

struct SupportSeed<'a> {
    projected: ProjectedSupport<'a>,
    entry: BorrowState,
    entries: Vec<(PortId, EntryCapture)>,
}

#[derive(Clone, Copy)]
enum LoanValue {
    Result(usize),
    Cell(usize, usize),
    Family(usize, usize),
}

#[derive(Clone)]
struct BoundaryExport<'a, 'hir> {
    uncovered: BTreeSet<typed::OutputPort>,
    checker: &'a BorrowChecker<'hir>,
    transfer: &'a transfer::BorrowTransferContext<'hir>,
    state: &'a BorrowState,
    acyclic: bool,
    entries: &'a [(PortId, EntryCapture)],
    entry_ports: &'a [EntryPort],
    entry_allocations: &'a FxHashMap<AllocId, PortId>,
    retained: FxHashMap<AllocId, typed::ObjectRef>,
    loans: Vec<(LoanValue, state::DependencySet)>,
    post: typed::Post,
}

impl BoundaryExport<'_, '_> {
    fn result(&mut self, slot: usize, ty: TypeId, value: &ValueAlt) {
        let location = typed::ExportPort::Result(CallResultPath {
            slot,
            path: Vec::new(),
        });
        if let Some(descriptor) = self.value(value, ty, Some(location)) {
            if !value.deps.is_empty() {
                self.loans.push((
                    LoanValue::Result(self.post.results.len()),
                    value.deps.clone(),
                ));
            }
            self.post.results.push((slot, descriptor));
        } else {
            self.uncovered.insert(typed::OutputPort::Result(slot));
        }
    }

    fn bound(
        &self,
        region: ReachKind,
        root_types: impl IntoIterator<Item = (PortId, TypeId)>,
        target: TypeId,
    ) -> Option<typed::ReachBound> {
        let identity = self.checker.project_type(target)?;
        let env = self.checker.type_environment;
        let mut roots = Vec::new();
        let mut seen = FxHashSet::default();
        let mut pending = Vec::new();
        for (root, ty) in root_types {
            seen.clear();
            pending.push(ty);
            while let Some(ty) = pending.pop() {
                if ty == target {
                    roots.push(root);
                    pending.clear();
                    break;
                }
                if region != ReachKind::Cell && seen.insert(ty) {
                    let _: Result<(), std::convert::Infallible> =
                        CallField::for_each_child(ty, env, |_, child| {
                            pending.push(child);
                            Ok(())
                        });
                }
            }
        }
        roots.sort_unstable();
        roots.dedup();
        (!roots.is_empty()).then_some(typed::ReachBound {
            roots,
            region,
            types: vec![identity],
        })
    }

    fn value_bound(&self, ty: TypeId) -> Option<typed::ReachBound> {
        self.bound(
            ReachKind::EntryStructure,
            self.entries
                .iter()
                .filter_map(|(port, entry)| Some((*port, entry.ty?))),
            ty,
        )
    }

    fn value(
        &mut self,
        value: &ValueAlt,
        ty: TypeId,
        exact: Option<typed::ExportPort>,
    ) -> Option<typed::ValueRef> {
        use typed::{
            ObjectRef, PathAccess, Permission, ScalarSource, ScalarSourceBound, ValueSource,
        };
        // Native operation edges can use an allocation-shaped carrier for a
        // scalar member. Its endpoint determines the exported storage kind.
        let scalar_endpoint = is_heap(self.checker, ty).is_ok_and(|heap| !heap);
        let (source, restriction) = match value.kind {
            PayloadKind::Unknown if !scalar_endpoint => return None,
            kind if scalar_endpoint => {
                if kind == PayloadKind::Absent {
                    return None;
                }
                let mut scalar_sources: Vec<_> = value
                    .deps
                    .iter()
                    .flat_map(|dep| dep.entries.as_slice())
                    .filter_map(|&id| self.state.entry_scalar_source(id).map(PortId))
                    .collect();
                if let PayloadKind::Allocation(value) = kind
                    && let Some(port) = self.state.entry_scalar_source(value.id)
                {
                    scalar_sources.push(PortId(port));
                }
                // A computed scalar of another type is not the predecessor's
                // stored value, even if an operation used it as a contributor.
                scalar_sources.retain(|port| {
                    self.entries
                        .iter()
                        .any(|(id, entry)| id == port && entry.ty == Some(ty))
                });
                scalar_sources.sort_unstable();
                scalar_sources.dedup();
                let scalar = match scalar_sources.as_slice() {
                    [] if kind != PayloadKind::Unknown => ScalarSource::Independent,
                    [] => ScalarSource::Unresolved(ScalarSourceBound::Reach(self.value_bound(ty)?)),
                    [port] if kind != PayloadKind::Unknown => ScalarSource::EntryValue(*port),
                    _ => ScalarSource::Unresolved(ScalarSourceBound::Values(scalar_sources)),
                };
                (ValueSource::Scalar(scalar), None)
            }
            PayloadKind::Allocation(value) => {
                let raw_result =
                    matches!(&exact, Some(typed::ExportPort::Result(path)) if path.path.is_empty());
                let object = if let Some(&entry) = self.entry_allocations.get(&value.id) {
                    ObjectRef::Entry(entry)
                } else {
                    self.allocation(value.id, ty, exact)?
                };
                // A callee-local name is not an immutable caller binding.
                // Raw transport keeps the actual's permission, but a new
                // edge through a shared formal keeps its own restriction.
                let restricted =
                    matches!(&object, ObjectRef::Entry(_)) && !raw_result && !value.writable;
                (
                    ValueSource::Allocation(object),
                    restricted.then_some(Permission::ReadOnly),
                )
            }
            PayloadKind::Absent | PayloadKind::Scalar | PayloadKind::Unknown => return None,
        };
        Some(typed::ValueRef {
            source,
            access: PathAccess {
                restriction,
                derived_from: None,
            },
            dependencies: Vec::new(),
        })
    }

    fn receiver_loans(&self, loans: &state::DependencySet) -> Vec<typed::ReceiverDependency> {
        use typed::ObjectRef;
        let mut dependencies = Vec::new();
        for dep in loans {
            let mut receivers = Vec::new();
            for &id in dep.entries.as_slice() {
                if self.state.entry_scalar_source(id).is_some() {
                    continue;
                }
                if let Some(&port) = self.entry_allocations.get(&id) {
                    receivers.push(ObjectRef::Entry(port));
                } else if let Some(receiver) = self.retained.get(&id) {
                    receivers.push(receiver.clone());
                } else if let Some(ty) = self.entries.iter().find_map(|(_, entry)| {
                    entry
                        .value
                        .values
                        .iter()
                        .any(|value| {
                            self.state
                                .alloc_paths_may_share(&[value.id], &[], &[id], &[])
                        })
                        .then_some(entry.ty)
                        .flatten()
                }) && let Some(bound) = self.value_bound(ty)
                {
                    dependencies.push(typed::ReceiverDependency::Unresolved(bound));
                }
            }
            receivers.sort_unstable();
            receivers.dedup();
            if !receivers.is_empty() {
                dependencies.push(typed::ReceiverDependency::Known(receivers));
            }
        }
        dependencies.sort_unstable();
        dependencies.dedup();
        dependencies
    }

    fn allocation(
        &mut self,
        id: AllocId,
        ty: TypeId,
        exact: Option<typed::ExportPort>,
    ) -> Option<typed::ObjectRef> {
        use typed::{CellPort, FamilyPort, ObjectRef, StorageKind};
        if let Some(object) = self.retained.get(&id) {
            return Some(object.clone());
        }
        let identity = self.checker.project_type(ty)?;
        let kind = StorageKind::Allocation;
        let object = exact
            .filter(|_| self.state.alloc_exact_at_boundary(id, self.acyclic))
            .map_or_else(
                || ObjectRef::Family(kind, identity.clone()),
                ObjectRef::Exact,
            );
        self.retained.insert(id, object.clone());
        self.post.retained.push((object.clone(), kind, identity));
        let mut children = Vec::new();
        let _: Result<(), std::convert::Infallible> =
            CallField::for_each_child(ty, self.checker.type_environment, |selector, child| {
                if let Ok((key, _)) = boundary_child(self.transfer, ty, selector) {
                    children.push((selector, key, child));
                }
                Ok(())
            });
        let state = self.state;
        for (selector, key, child) in children {
            let cell = state::CellId::Projected(id, key);
            let values = state.value_cell(cell);
            let group = state.retained_edge_group(id, key);
            if selector.is_collapsed() {
                let family = FamilyPort {
                    receiver: object.clone(),
                    selector,
                };
                let mut members = Vec::new();
                let mut loans = Vec::new();
                for (value, once) in state.retained_family_values(id, key) {
                    if let Some(descriptor) = self.value(&value, child, None) {
                        if !value.deps.is_empty() {
                            loans.push((members.len(), value.deps.clone()));
                        }
                        let occurrence = if once {
                            typed::Occurrence::AtMostOnce
                        } else {
                            typed::Occurrence::MaybeMany
                        };
                        members.push((descriptor, occurrence));
                    } else {
                        self.uncovered
                            .insert(typed::OutputPort::Family(family.clone()));
                    }
                }
                self.record_family_loans(loans);
                if matches!(object, ObjectRef::Family(..)) {
                    if !members.is_empty() {
                        self.post
                            .families
                            .push((family, typed::FamilyPost::Extend(members)));
                    }
                } else if !group.is_empty() || !values.is_empty() || state.family_was_reset(id, key)
                {
                    self.post
                        .families
                        .push((family, typed::FamilyPost::ReplaceAll(members)));
                }
            } else {
                let target = CellPort::Edge(object.clone(), selector);
                let mut fallback;
                let write_values = if values.is_empty() {
                    fallback = group
                        .iter()
                        .map(|edge| edge.value_alternative())
                        .collect::<Vec<_>>();
                    if fallback.is_empty()
                        && is_heap(self.checker, child).is_ok_and(|heap| heap)
                        && let Some(id) = state.retained_lazy_child(id, key)
                    {
                        fallback.push(ValueAlt::plain(PayloadKind::Allocation(
                            state::StoredValue { id, writable: true },
                        )));
                    }
                    fallback.as_slice()
                } else {
                    values
                };
                if !write_values.is_empty() {
                    let projected = extend_export(&object, selector);
                    let mut captured = Vec::with_capacity(write_values.len());
                    let mut loans = Vec::new();
                    for value in write_values {
                        let output = self.value(value, child, projected.clone());
                        if output.is_none() && value.kind != PayloadKind::Absent {
                            self.uncovered
                                .insert(typed::OutputPort::Cell(target.clone()));
                        } else {
                            if output.is_some() && !value.deps.is_empty() {
                                loans.push((captured.len(), value.deps.clone()));
                            }
                            captured.push(output);
                        }
                    }
                    if !captured.is_empty() {
                        let post = if matches!(object, ObjectRef::Family(..)) {
                            typed::CellPost::WeakJoin(captured)
                        } else {
                            typed::CellPost::Replace(captured)
                        };
                        self.record_cell_loans(loans);
                        self.post.cells.push((target, post));
                    }
                }
            }
        }
        Some(object)
    }

    fn export_entry_family(
        &mut self,
        entry_state: &BorrowState,
        entry: &EntryCapture,
        holder: PortId,
        selector: CallField,
        field: state::FieldKey,
        ty: TypeId,
    ) {
        use typed::{FamilyPort, FamilyPost, ObjectRef, Occurrence, OutputPort};
        let state = self.state;
        let target = FamilyPort {
            receiver: ObjectRef::Entry(holder),
            selector,
        };
        let mut members = Vec::new();
        let mut loans = Vec::new();
        let mut replace = entry.value.values.len() == 1;
        for value in &entry.value.values {
            if !value.writable {
                replace = false;
                continue;
            }
            let reset = state.family_was_reset(value.id, field);
            replace &= reset;
            let previous = if reset {
                FxHashSet::default()
            } else {
                entry_state
                    .retained_family_values(value.id, field)
                    .collect()
            };
            for member in state.retained_family_values(value.id, field) {
                if previous.contains(&member) {
                    continue;
                }
                let (alt, once) = member;
                if let Some(descriptor) = self.value(&alt, ty, None) {
                    if !alt.deps.is_empty() {
                        loans.push((members.len(), alt.deps.clone()));
                    }
                    members.push((
                        descriptor,
                        if once {
                            Occurrence::AtMostOnce
                        } else {
                            Occurrence::MaybeMany
                        },
                    ));
                } else {
                    self.uncovered.insert(OutputPort::Family(target.clone()));
                }
            }
        }
        self.record_family_loans(loans);
        if replace {
            self.post
                .families
                .push((target, FamilyPost::ReplaceAll(members)));
        } else if !members.is_empty() {
            self.post
                .families
                .push((target, FamilyPost::Extend(members)));
        }
    }

    fn record_cell_loans(&mut self, loans: Vec<(usize, state::DependencySet)>) {
        let cell = self.post.cells.len();
        self.loans.extend(
            loans
                .into_iter()
                .map(|(value, deps)| (LoanValue::Cell(cell, value), deps)),
        );
    }

    fn record_family_loans(&mut self, loans: Vec<(usize, state::DependencySet)>) {
        let family = self.post.families.len();
        self.loans.extend(
            loans
                .into_iter()
                .map(|(member, deps)| (LoanValue::Family(family, member), deps)),
        );
    }

    fn finish(mut self) -> typed::Post {
        // A scalar can precede the output that gives its receiver a name.
        // Resolve loans against the complete exported graph before sorting rows.
        for (at, loans) in std::mem::take(&mut self.loans) {
            let dependencies = self.receiver_loans(&loans);
            let value = match at {
                LoanValue::Result(index) => &mut self.post.results[index].1,
                LoanValue::Cell(row, index) => {
                    let (typed::CellPost::Replace(values) | typed::CellPost::WeakJoin(values)) =
                        &mut self.post.cells[row].1;
                    values[index].as_mut().expect("loans name a present value")
                }
                LoanValue::Family(row, index) => {
                    let (typed::FamilyPost::ReplaceAll(members)
                    | typed::FamilyPost::Extend(members)) = &mut self.post.families[row].1;
                    &mut members[index].0
                }
            };
            value.dependencies = dependencies;
        }
        self.post.retained.sort_unstable();
        self.post.retained.dedup();
        self.post
            .bindings
            .sort_unstable_by(|left, right| left.0.cmp(&right.0));
        self.post
            .cells
            .sort_unstable_by(|left, right| left.0.cmp(&right.0));
        self.post
            .cells
            .dedup_by(|(target, edit), (previous_target, previous)| {
                if target != previous_target {
                    return false;
                }
                let (typed::CellPost::Replace(incoming) | typed::CellPost::WeakJoin(incoming)) =
                    edit;
                let (typed::CellPost::Replace(values) | typed::CellPost::WeakJoin(values)) =
                    previous;
                values.append(incoming);
                true
            });
        for (_, edit) in &mut self.post.cells {
            let (typed::CellPost::Replace(values) | typed::CellPost::WeakJoin(values)) = edit;
            values.sort_unstable();
            values.dedup();
        }
        self.post
            .families
            .sort_unstable_by(|left, right| left.0.cmp(&right.0));
        self.post.families.dedup_by(|(target, edit), (previous_target, previous)| {
            if target != previous_target {
                return false;
            }
            let (typed::FamilyPost::ReplaceAll(incoming) | typed::FamilyPost::Extend(incoming)) = edit;
            let (typed::FamilyPost::ReplaceAll(members) | typed::FamilyPost::Extend(members)) = previous;
            members.append(incoming);
            true
        });
        for (_, edit) in &mut self.post.families {
            let (typed::FamilyPost::ReplaceAll(values) | typed::FamilyPost::Extend(values)) = edit;
            values.sort_unstable();
            values.dedup_by(|(member, _), (previous, previous_occurrence)| {
                if member != previous {
                    return false;
                }
                // Distinct live insertions can quotient to the same member.
                *previous_occurrence = typed::Occurrence::MaybeMany;
                true
            });
        }
        self.post.results.sort_unstable_by_key(|(slot, _)| *slot);
        if !self.uncovered.is_empty() {
            let reach = if self.uncovered.len() == 1
                && let Some(typed::OutputPort::Cell(typed::CellPort::Terminal(port))) =
                    self.uncovered.first()
                && let Some(EntryPort::Terminal(parameter)) = self.entry_ports.get(port.0 as usize)
                && let Some(ty) = self.entries.iter().find_map(|(port, entry)| {
                    match self.entry_ports.get(port.0 as usize)? {
                        EntryPort::Value {
                            parameter: source,
                            path,
                        } if source == parameter && path.is_empty() => entry.ty,
                        _ => None,
                    }
                }) {
                self.bound(ReachKind::Cell, [(*port, ty)], ty)
            } else {
                None
            };
            self.post.coverage = typed::Coverage::Unresolved(if let Some(reach) = reach {
                typed::Footprint::WritableReach(reach)
            } else {
                typed::Footprint::Locations(self.uncovered.into_iter().collect())
            });
        }
        self.post
    }
}

fn extend_export(object: &typed::ObjectRef, selector: CallField) -> Option<typed::ExportPort> {
    let typed::ObjectRef::Exact(export) = object else {
        return None;
    };
    let mut export = export.clone();
    match &mut export {
        typed::ExportPort::Cell(path) => path.path.push(selector),
        typed::ExportPort::Result(path) => path.path.push(selector),
        _ => return None,
    }
    Some(export)
}

fn boundary_value_alternatives(captured: CapturedValue, scalar: bool) -> Vec<ValueAlt> {
    if !captured.alternatives.is_empty() {
        captured.alternatives
    } else if scalar {
        vec![ValueAlt::plain(PayloadKind::Scalar)]
    } else {
        captured
            .values
            .into_iter()
            .map(|value| ValueAlt::plain(PayloadKind::Allocation(value)))
            .collect()
    }
}

fn boundary_cell_alternatives(
    state: &BorrowState,
    cell: state::CellId,
    scalar: bool,
) -> Vec<ValueAlt> {
    use state::CellId;
    let explicit = state.value_cell(cell);
    if !explicit.is_empty() {
        return explicit.to_vec();
    }
    match cell {
        CellId::Slot(wrapper) => {
            if let Some(values) = state.retained_slot_payload(wrapper) {
                return values.map(ValueAlt::plain).collect();
            }
        }
        CellId::Projected(holder, selector) => {
            if let Some(edges) = state
                .retained_edge_groups(holder)
                .find(|edges| edges[0].key == selector)
            {
                return edges.iter().map(|edge| edge.value_alternative()).collect();
            }
        }
    }
    if scalar {
        vec![ValueAlt::plain(PayloadKind::Scalar)]
    } else {
        Vec::new()
    }
}

fn visit_entry_roles<E>(
    remaining: &[(PortId, usize)],
    roles: &mut Vec<(PortId, usize, Role)>,
    visit: &mut impl FnMut(&[(PortId, usize, Role)]) -> Result<(), E>,
) -> Result<(), E> {
    let Some((&(port, parameter), remaining)) = remaining.split_first() else {
        return visit(roles);
    };
    for role in [Role::Slot, Role::WholeAlias, Role::ProjectedAlias] {
        roles.push((port, parameter, role));
        visit_entry_roles(remaining, roles, visit)?;
        roles.pop();
    }
    Ok(())
}

fn visit_entry_identities<E>(
    remaining: &[Predicate],
    literals: &mut Vec<typed::Literal>,
    visit: &mut impl FnMut(&[typed::Literal], typed::Guard) -> Result<(), E>,
) -> Result<(), E> {
    let Some((&predicate, remaining)) = remaining.split_first() else {
        return match typed::Guard::new(literals.iter().copied()) {
            Some(guard) => visit(literals, guard),
            None => Ok(()),
        };
    };
    if typed::Guard::implied_by(literals, predicate).is_some() {
        return visit_entry_identities(remaining, literals, visit);
    }
    for truth in [false, true] {
        literals.push((predicate, truth));
        visit_entry_identities(remaining, literals, visit)?;
        literals.pop();
    }
    Ok(())
}

fn allocation_classes(presentation: &[typed::Literal]) -> BTreeMap<PortId, PortId> {
    fn root(classes: &BTreeMap<PortId, PortId>, mut port: PortId) -> PortId {
        while let Some(&parent) = classes.get(&port) {
            if parent == port {
                break;
            }
            port = parent;
        }
        port
    }
    let mut classes = BTreeMap::new();
    for &(predicate, truth) in presentation {
        if let (Predicate::Equal(IdentityKind::Allocation, left, right), true) = (predicate, truth)
        {
            let left = root(&classes, left);
            let right = root(&classes, right);
            let leader = left.min(right);
            classes.insert(leader, leader);
            classes.insert(left.max(right), leader);
        }
    }
    classes
        .keys()
        .map(|&port| (port, root(&classes, port)))
        .collect()
}

impl SparseFunction {
    pub(super) fn evaluate_canonical(
        &self,
        checker: &BorrowChecker<'_>,
        function: &HirFunction,
    ) -> Result<typed::TypedCallAllocationFacts, BorrowCheckError> {
        let context = checker.transfer_context();
        let mut lanes = [Vec::new(), Vec::new()];
        let mut may_retain = BTreeSet::new();
        let mut roles = Vec::new();
        let mut roles_by_support = vec![Vec::new(); self.support_count];
        let mut equalities_by_support = vec![Vec::new(); self.support_count];
        let mut tested_wrappers = BTreeSet::new();
        for &predicate in &self.guards {
            match predicate {
                Predicate::Role(port, _) => {
                    tested_wrappers.insert(port);
                }
                Predicate::Equal(IdentityKind::Allocation, left, right) => {
                    let support = self.port_supports[left.0 as usize];
                    debug_assert_eq!(support, self.port_supports[right.0 as usize]);
                    equalities_by_support[support.0 as usize].push(predicate);
                }
                _ => {}
            }
        }
        for constraint in &self.compatibility {
            if let CompatibilityConstraint::Equality(IdentityKind::Wrapper, group) = constraint {
                tested_wrappers.extend(group.iter().copied());
            }
        }
        for port in tested_wrappers {
            let EntryPort::Wrapper(parameter) = self.ports[port.0 as usize] else {
                continue;
            };
            roles_by_support[self.port_supports[port.0 as usize].0 as usize]
                .push((port, parameter));
        }
        let mut demanded = vec![false; self.support_count];
        for (id, ports) in self.ports_by_support.iter().enumerate() {
            demanded[id] = !ports.is_empty();
        }
        for &support in self.result_supports.values() {
            demanded[support.0 as usize] = true;
        }
        let prune =
            demanded.contains(&false) && self.has_structural_call_reach(checker, function)?;
        let supports = if prune && !demanded.contains(&true) {
            // No boundary observation remains. One empty projection records
            // lane reach without replaying any of the discarded local state.
            self.support_count..self.support_count + 1
        } else {
            0..self.support_count.max(1)
        };
        for id in supports {
            if prune && demanded.get(id) == Some(&false) {
                continue;
            }
            let support = SupportId(id as u32);
            let wrappers = roles_by_support.get(id).map_or(&[][..], Vec::as_slice);
            let identities = equalities_by_support.get(id).map_or(&[][..], Vec::as_slice);
            let mut support_lanes = [typed::Lane::Bottom, typed::Lane::Bottom];
            visit_entry_roles(wrappers, &mut roles, &mut |roles| {
                let mut literals = roles
                    .iter()
                    .map(|&(port, _, role)| (Predicate::Role(port, role), true))
                    .collect();
                visit_entry_identities(identities, &mut literals, &mut |presentation, guard| {
                    let SupportSeed {
                        projected,
                        entry,
                        entries,
                    } = self.seed_support(checker, function, support, roles, presentation)?;
                    let mut entry_allocations = FxHashMap::default();
                    for (port, entry) in &entries {
                        for value in &entry.value.values {
                            entry_allocations.entry(value.id).or_insert(*port);
                        }
                    }
                    let evaluated =
                        checker.evaluate_sparse_support(function, entry.clone(), &projected)?;
                    may_retain.extend(evaluated.retained.into_iter().filter_map(|index| {
                        self.transfer
                            .parameters
                            .get(&projected.local_ids[index])
                            .copied()
                    }));
                    let mut alternatives = [Vec::new(), Vec::new()];
                    for (block, exit) in evaluated.exits.into_iter().flat_map(|(block, states)| {
                        states.into_iter().map(move |state| (block, state))
                    }) {
                        let terminator =
                            &checker.block_by_id_or_error(block, function.id)?.terminator;
                        let (error, values) = match terminator {
                            HirTerminator::Return(values) => {
                                (false, checker.module.expressions.values(*values))
                            }
                            HirTerminator::ReturnError(value) => {
                                (true, std::slice::from_ref(value))
                            }
                            _ => continue,
                        };
                        let mut state =
                            Rc::try_unwrap(exit).unwrap_or_else(|state| state.as_ref().clone());
                        let view = projected.block_view(block).operations;
                        let mut result = Vec::new();
                        for (slot, &value) in values.iter().enumerate() {
                            if self.result_supports.get(&(error, slot)) == Some(&support) {
                                result.push((
                                    slot,
                                    checker.module.expressions.expression(value).ty,
                                    capture_boundary_value(
                                        &context, view, block, &mut state, value,
                                    )?,
                                ));
                            }
                        }
                        let mut export = BoundaryExport {
                            checker,
                            transfer: &context,
                            state: &state,
                            acyclic: self.transfer.recurring.is_empty(),
                            entries: &entries,
                            entry_ports: &self.ports,
                            entry_allocations: &entry_allocations,
                            retained: FxHashMap::default(),
                            loans: Vec::new(),
                            uncovered: BTreeSet::new(),
                            post: typed::Post::default(),
                        };
                        let mut choice = None;
                        for (slot, ty, captured) in result {
                            let values =
                                boundary_value_alternatives(captured, !is_heap(checker, ty)?);
                            match values.as_slice() {
                                [value] => export.result(slot, ty, value),
                                [_, _, ..] if choice.is_none() => choice = Some((slot, ty, values)),
                                _ => {
                                    // This state has no evidence relating two
                                    // independently joined result cells.
                                    export.uncovered.insert(typed::OutputPort::Result(slot));
                                }
                            }
                        }
                        let selected = &mut alternatives[usize::from(error)];
                        if let Some((slot, ty, values)) = choice {
                            for value in values {
                                let mut row = export.clone();
                                row.result(slot, ty, &value);
                                self.export_entry_cells(
                                    function, &projected, &entry, &entries, &mut row,
                                )?;
                                selected.push(row.finish());
                            }
                        } else {
                            self.export_entry_cells(
                                function,
                                &projected,
                                &entry,
                                &entries,
                                &mut export,
                            )?;
                            selected.push(export.finish());
                        }
                    }
                    for (lane, alternatives) in support_lanes.iter_mut().zip(alternatives) {
                        if !alternatives.is_empty() {
                            let product = typed::Product::new(vec![typed::Component {
                                support,
                                alternatives,
                            }])
                            .expect("one projected support");
                            *lane = lane.union(&typed::Lane::guarded(guard.clone(), product));
                        }
                    }
                    Ok::<(), BorrowCheckError>(())
                })
            })?;
            for (lane, projected) in lanes.iter_mut().zip(support_lanes) {
                lane.push(projected);
            }
        }
        let [success, error] = lanes.map(typed::Lane::compose_supports);
        Ok(typed::TypedCallAllocationFacts {
            ports: self.ports.clone(),
            support_count: self.support_count,
            success,
            error,
            may_retain: may_retain.into_iter().collect(),
        })
    }
    fn has_structural_call_reach(
        &self,
        checker: &BorrowChecker<'_>,
        function: &HirFunction,
    ) -> Result<bool, BorrowCheckError> {
        let signatures = checker.call_signatures();
        let structural = |target, argc, both_lanes: bool| -> Result<_, BorrowCheckError> {
            Ok(
                match signatures
                    .resolve(target, argc, &checker.diagnostics)?
                    .returns()
                {
                    CallReturns::Summary(summary) => summary.facts().is_none_or(|facts| {
                        facts.success.structural_reach().is_some()
                            && (!both_lanes || facts.error.structural_reach().is_some())
                    }),
                    _ => true,
                },
            )
        };
        for (&id, orders) in &self.transfer.blocks {
            let block = checker.block_by_id_or_error(id, function.id)?;
            for &(position, argc) in &orders.structural_calls {
                let HirStatementKind::Call { target, .. } = &block.statements[position].kind else {
                    unreachable!("indexed statement call");
                };
                if !structural(target, argc, false)? {
                    return Ok(false);
                }
            }
            if let Some(argc) = orders.structural_terminator
                && let HirTerminator::Invoke {
                    operation: HirFallibleOperation::Call { target, .. },
                    ..
                } = &block.terminator
                && !structural(target, argc, true)?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn export_entry_binding(
        &self,
        parameter: usize,
        port: PortId,
        function: &HirFunction,
        support: &ProjectedSupport<'_>,
        entry: &BorrowState,
        export: &mut BoundaryExport<'_, '_>,
    ) {
        use state::{BindingTarget, CellId, WrapperId};
        use typed::{
            BindingPost, CellPort, Derivation, ObjectRef, OutputPort, PathAccess, Permission,
            RetainedRoute,
        };
        let index = support.local_indices[&function.params[parameter]];
        if !support.mutable[index] {
            return;
        }
        let wrapper = WrapperId::current(index);
        let before = entry.boundary_binding(wrapper);
        let after = export.state.boundary_binding(wrapper);
        if before.alternatives == after.alternatives && before.routes == after.routes {
            return;
        }
        let owner = ObjectRef::Entry(port);
        let source_wrapper = |wrapper| {
            let direct = function
                .params
                .iter()
                .enumerate()
                .find_map(|(parameter, local)| {
                    support
                        .local_indices
                        .get(local)
                        .is_some_and(|&i| WrapperId::current(i) == wrapper)
                        .then_some(parameter)
                });
            let mut source = direct.map(EntryPort::Wrapper);
            if source.is_none() {
                source = function
                    .params
                    .iter()
                    .enumerate()
                    .find_map(|(parameter, local)| {
                        let index = *support.local_indices.get(local)?;
                        entry
                            .boundary_binding(WrapperId::current(index))
                            .alternatives
                            .iter()
                            .any(|alt| alt.target == BindingTarget::Link(wrapper))
                            .then_some(EntryPort::Immediate(parameter))
                    });
            }
            self.ports
                .iter()
                .position(|p| Some(p) == source.as_ref())
                .map(|i| PortId(i as u32))
        };
        let edge = |export: &BoundaryExport<'_, '_>, holder, key| {
            let source = *export.entry_allocations.get(&holder)?;
            let EntryPort::Value { parameter, path } = self.ports.get(source.0 as usize)? else {
                return None;
            };
            let mut ty = export.checker.local_types[&function.params[*parameter]];
            for &selector in path {
                ty = selector.type_child(ty, export.checker.type_environment)?;
            }
            let mut field = None;
            let _: Result<(), std::convert::Infallible> =
                CallField::for_each_child(ty, export.checker.type_environment, |selector, _| {
                    if boundary_child(export.transfer, ty, selector).is_ok_and(|(k, _)| k == key)
                        && !selector.is_collapsed()
                    {
                        field = Some(selector);
                    }
                    Ok(())
                });
            Some(CellPort::Edge(ObjectRef::Entry(source), field?))
        };
        let edit = (|| {
            let [after] = after.alternatives else {
                return None;
            };
            let route = RetainedRoute {
                access: PathAccess {
                    restriction: Some(if after.writable {
                        Permission::Writable
                    } else {
                        Permission::ReadOnly
                    }),
                    derived_from: after
                        .origin
                        .and_then(|origin| source_wrapper(origin.wrapper))
                        .map(Derivation::Entry),
                },
                dependencies: Vec::new(),
            };
            Some(match after.target {
                BindingTarget::OwnSlot => BindingPost::OwnSlot,
                BindingTarget::Link(source) => {
                    let mut route = route;
                    let source = if let Some(port) = source_wrapper(source) {
                        ObjectRef::Entry(port)
                    } else {
                        let [binding] = export.state.boundary_binding(source).alternatives else {
                            return None;
                        };
                        if binding.target != BindingTarget::OwnSlot {
                            return None;
                        }
                        let ty = export.checker.local_types[&function.params[parameter]];
                        let identity = export.checker.project_type(ty)?;
                        let export_port = if source.is_singleton()
                            && source == WrapperId::current(source.local)
                        {
                            typed::ExportPort::Binding(parameter)
                        } else {
                            typed::ExportPort::PrivateTerminal(parameter)
                        };
                        let source_ref = ObjectRef::Exact(export_port);
                        let values = boundary_cell_alternatives(
                            export.state,
                            CellId::Slot(source),
                            !is_heap(export.checker, ty).ok()?,
                        );
                        let mut contents = Vec::new();
                        let mut loans = Vec::new();
                        for value in &values {
                            if let Some(descriptor) = export.value(value, ty, None) {
                                if !value.deps.is_empty() {
                                    loans.push((contents.len(), value.deps.clone()));
                                }
                                contents.push(Some(descriptor));
                            } else if value.kind != PayloadKind::Absent {
                                return None;
                            }
                        }
                        export.post.retained.push((
                            source_ref.clone(),
                            typed::StorageKind::Wrapper,
                            identity,
                        ));
                        export
                            .post
                            .bindings
                            .push((source_ref.clone(), BindingPost::OwnSlot));
                        if !contents.is_empty() {
                            export.record_cell_loans(loans);
                            export.post.cells.push((
                                CellPort::Slot(source_ref.clone()),
                                typed::CellPost::Replace(contents),
                            ));
                        }
                        route.access.derived_from = Some(Derivation::Retained(source_ref.clone()));
                        source_ref
                    };
                    BindingPost::Link(source, route)
                }
                BindingTarget::Projected(holder, key) => {
                    let cell = edge(export, holder, key).or_else(|| {
                        function
                            .params
                            .iter()
                            .enumerate()
                            .find_map(|(parameter, local)| {
                                let index = *support.local_indices.get(local)?;
                                entry
                                    .resolve_cells(index)
                                    .cells()
                                    .contains(&CellId::Projected(holder, key))
                                    .then_some(())?;
                                self.ports
                                    .iter()
                                    .position(|port| *port == EntryPort::Terminal(parameter))
                                    .map(|port| CellPort::Terminal(PortId(port as u32)))
                            })
                    })?;
                    BindingPost::Projected(cell, route)
                }
            })
        })();
        if let Some(edit) = edit {
            export.post.bindings.push((owner, edit));
        } else {
            export.uncovered.insert(OutputPort::Binding(owner));
        }
    }

    fn export_entry_cells(
        &self,
        function: &HirFunction,
        support: &ProjectedSupport<'_>,
        entry_state: &BorrowState,
        entries: &[(PortId, EntryCapture)],
        export: &mut BoundaryExport<'_, '_>,
    ) -> Result<(), BorrowCheckError> {
        use state::CellId;
        use typed::{CellPort, ExportPort, ObjectRef, OutputPort};
        let ports = self
            .ports_by_support
            .get(support.support.0 as usize)
            .map_or(&[][..], Vec::as_slice);
        let mut emitted = BTreeSet::new();
        for &port in ports {
            let (ty, target, cells, path) = match self.ports[port.0 as usize] {
                EntryPort::Wrapper(parameter) => {
                    self.export_entry_binding(
                        parameter,
                        port,
                        function,
                        support,
                        entry_state,
                        export,
                    );
                    continue;
                }
                EntryPort::Terminal(parameter) => {
                    let local = function.params[parameter];
                    let index = support.local_indices[&local];
                    if !support.mutable[index] {
                        continue;
                    }
                    (
                        export.checker.local_types[&local],
                        CellPort::Terminal(port),
                        entry_state.resolve_cells(index).cells().to_vec(),
                        Some(ExportPort::Cell(CallParameterPath {
                            index: parameter,
                            path: Vec::new(),
                        })),
                    )
                }
                EntryPort::Edge(holder, selector) => {
                    let Some(entry) = entries
                        .binary_search_by_key(&holder, |(port, _)| *port)
                        .ok()
                        .map(|i| &entries[i].1)
                    else {
                        continue;
                    };
                    if entry.writable == Some(false) || entry.value.values.is_empty() {
                        continue;
                    }
                    let (field, ty) = boundary_child(export.transfer, entry.ty.unwrap(), selector)?;
                    if selector.is_collapsed() {
                        if let [member] = entry.value.values.as_slice()
                            && !emitted.insert(CellId::Projected(member.id, field))
                        {
                            continue;
                        }
                        export.export_entry_family(entry_state, entry, holder, selector, field, ty);
                        continue;
                    }
                    let cells = entry
                        .value
                        .values
                        .iter()
                        .map(|value| CellId::Projected(value.id, field))
                        .collect();
                    let path = match &self.ports[holder.0 as usize] {
                        EntryPort::Value { parameter, path } => {
                            Some(ExportPort::Cell(CallParameterPath {
                                index: *parameter,
                                path: path.iter().copied().chain([selector]).collect(),
                            }))
                        }
                        _ => None,
                    };
                    (
                        ty,
                        CellPort::Edge(ObjectRef::Entry(holder), selector),
                        cells,
                        path,
                    )
                }
                _ => continue,
            };
            let scalar = !is_heap(export.checker, ty)?;
            let mut changed = false;
            let mut output = Vec::new();
            for &cell in &cells {
                let previous = boundary_cell_alternatives(entry_state, cell, scalar);
                let next = boundary_cell_alternatives(export.state, cell, scalar);
                changed |= next != previous;
                output.extend(next);
            }
            if !changed {
                continue;
            }
            if let [cell] = cells.as_slice()
                && !emitted.insert(*cell)
            {
                continue;
            }
            let mut values = Vec::with_capacity(output.len());
            let mut loans = Vec::new();
            for value in &output {
                let descriptor = export.value(value, ty, path.clone());
                if descriptor.is_none() && value.kind != PayloadKind::Absent {
                    export.uncovered.insert(OutputPort::Cell(target.clone()));
                } else {
                    if descriptor.is_some() && !value.deps.is_empty() {
                        loans.push((values.len(), value.deps.clone()));
                    }
                    values.push(descriptor);
                }
            }
            if !values.is_empty() {
                let edit = if cells.len() == 1 {
                    typed::CellPost::Replace(values)
                } else {
                    typed::CellPost::WeakJoin(values)
                };
                export.record_cell_loans(loans);
                export.post.cells.push((target, edit));
            }
        }
        Ok(())
    }

    fn seed_support(
        &self,
        checker: &BorrowChecker<'_>,
        function: &HirFunction,
        support: SupportId,
        roles: &[(PortId, usize, Role)],
        presentation: &[typed::Literal],
    ) -> Result<SupportSeed<'_>, BorrowCheckError> {
        let ports = self
            .ports_by_support
            .get(support.0 as usize)
            .map_or(&[][..], Vec::as_slice);
        let anchors = ports
            .iter()
            .map(|&port| function.params[port.parameter(&self.ports)]);
        let mut projected = self.project_support(support, anchors, []);
        let mut alias_anchors = Vec::new();
        for &(_, parameter, role) in roles {
            if role == Role::Slot {
                continue;
            }
            let local = function.params[parameter];
            let index = projected.local_indices[&local];
            let anchor = projected.local_ids.len();
            projected.local_ids.push(local);
            projected.mutable.push(projected.mutable[index]);
            projected.heap.push(projected.heap[index]);
            projected.first_write.push(UNKNOWN_ORDER_KEY);
            alias_anchors.push((index, state::WrapperId::current(anchor), role));
        }
        let mut entry_roots = RootSet::empty(projected.local_ids.len());
        for (index, local) in projected.local_ids.iter().enumerate() {
            if self.transfer.parameters.contains_key(local) {
                entry_roots.insert(index);
            }
        }
        projected.entry_roots = Some(entry_roots);
        projected.add_lexical_visibility(checker)?;
        projected.add_replay_lifetimes(function);
        let mut state = projected.seed_entry(function.params.iter().copied().filter_map(|local| {
            projected
                .local_indices
                .contains_key(&local)
                .then_some((local, BindingSpec::Slot))
        }));
        for (index, anchor, role) in alias_anchors {
            use state::{AddressSet, CellId, FieldKey, WrapperId};
            state.initialize_entry_binding(anchor.local, BindingSpec::Slot);
            let writable = projected.mutable[index];
            let spec = match role {
                Role::WholeAlias => BindingSpec::Link {
                    source: anchor,
                    writable,
                },
                Role::ProjectedAlias => {
                    let holder = state.local_alloc_ids(anchor.local).0[0];
                    let key = FieldKey::Field(0);
                    let addresses = AddressSet::from_cells(
                        [CellId::Projected(holder, key)],
                        Coverage::Complete,
                    );
                    BindingSpec::Projected {
                        addresses,
                        origin: WrapperId::current(index),
                        owner: Some(anchor),
                        writable,
                        isolated: false,
                        route_roots: RootSet::empty(projected.local_ids.len()),
                    }
                }
                Role::Slot => unreachable!("slot does not need a private entry anchor"),
            };
            state.initialize_entry_binding(index, spec);
        }
        state.track_family_resets();
        let context = checker.transfer_context();
        let capture = |state: &BorrowState, parameter: usize, path: &[CallField]| {
            let local = function.params[parameter];
            capture_local_value_port(
                state,
                &projected.mutable,
                (projected.local_indices[&local], checker.local_types[&local]),
                path,
                |ty, selector| boundary_child(&context, ty, selector).ok(),
                |ty| {
                    classify_hir_type(ty, checker.type_environment)
                        .is_ok_and(|shape| shape.has_in_place_mutation_surface())
                },
            )
        };
        let classes = allocation_classes(presentation);
        let mut equal_values = classes
            .into_iter()
            .map(|(port, leader)| {
                let EntryPort::Value { parameter, path } = &self.ports[port.0 as usize] else {
                    unreachable!("allocation identity requires entry values");
                };
                (path.as_ref(), *parameter, leader)
            })
            .collect::<Vec<_>>();
        equal_values
            .sort_unstable_by_key(|(path, parameter, leader)| (path.len(), *parameter, *leader));
        for (path, parameter, leader) in equal_values {
            let entry = capture(&state, parameter, path)?;
            let address = entry
                .address
                .expect("an entry value has its predecessor storage");
            let value = CapturedValue::fresh(state.entry_allocation_id(leader.0));
            state.publish_store(
                &TypedStore::replace_cells(address, value).with_explicit_slot_payload(),
                UNKNOWN_ORDER_KEY,
                None,
            );
        }
        for &port in ports {
            let EntryPort::Value { parameter, path } = &self.ports[port.0 as usize] else {
                continue;
            };
            let entry = capture(&state, *parameter, path)?;
            if entry.allocation_coverage.is_some() {
                continue;
            }
            let address = entry
                .address
                .expect("local path resolves storage before its value");
            state.initialize_entry_scalar(&address, port.0);
        }
        let mut entries = Vec::with_capacity(ports.len());
        for &port in ports {
            if let EntryPort::Value { parameter, path } = &self.ports[port.0 as usize] {
                entries.push((port, capture(&state, *parameter, path)?));
            }
        }
        Ok(SupportSeed {
            projected,
            entry: state,
            entries,
        })
    }

    fn project_support(
        &self,
        support: SupportId,
        anchors: impl IntoIterator<Item = LocalId>,
        visibility: impl IntoIterator<Item = (BlockId, RootSet)>,
    ) -> ProjectedSupport<'_> {
        let coordinates = &self.transfer;
        let mut barriers = coordinates.structural_barriers.clone();
        let mut locals: BTreeMap<_, _> = anchors
            .into_iter()
            .map(|local| (coordinates.local_indices[&local], 0))
            .collect();
        for (&(_, block), positions) in self
            .operations_by_support
            .range((support, BlockId(u32::MIN))..=(support, BlockId(u32::MAX)))
        {
            let rank = coordinates.ranks[&block];
            barriers.insert(rank);
            for &position in positions.iter() {
                for &local in &coordinates.blocks[&block].operands[position] {
                    let last = locals.entry(local).or_insert(rank);
                    *last = (*last).min(rank);
                }
            }
        }
        let mut projected = ProjectedSupport {
            function: self,
            support,
            barriers,
            visibility: FxHashMap::default(),
            last_use: Vec::with_capacity(locals.len()),
            replay_retirements: Vec::new(),
            entry_roots: None,
            #[cfg(test)]
            compress: true,
            local_ids: Vec::with_capacity(locals.len()),
            local_indices: FxHashMap::default(),
            mutable: Vec::with_capacity(locals.len()),
            heap: Vec::with_capacity(locals.len()),
            first_write: Vec::with_capacity(locals.len()),
        };
        projected.local_indices.reserve(locals.len());
        for (i, last) in locals {
            let local = coordinates.local_ids[i];
            projected
                .local_indices
                .insert(local, projected.local_ids.len());
            projected.local_ids.push(local);
            projected.mutable.push(coordinates.mutable[i]);
            projected.heap.push(coordinates.heap[i]);
            projected.first_write.push(coordinates.first_write[i]);
            // Edge feasibility may inspect an already tracked scrutinee even
            // when this support does not transfer the match itself.
            projected
                .last_use
                .push(last.min(coordinates.last_match_read[i]));
        }
        for (target, mask) in visibility {
            assert_eq!(mask.bit_len(), coordinates.local_ids.len());
            let mut visible = RootSet::empty(projected.local_ids.len());
            for (index, local) in projected.local_ids.iter().enumerate() {
                if mask.contains(coordinates.local_indices[local]) {
                    visible.insert(index);
                }
            }
            if visible.covers_all(projected.local_ids.len()) {
                continue;
            }
            projected.visibility.insert(target, visible);
            projected.barriers.insert(coordinates.ranks[&target]);
            // Narrowing does not distribute over a join. Keep the authored edge
            // source so all its incoming facts are joined before the mask applies.
            if let Some(predecessors) = coordinates.predecessors.get(&target) {
                projected
                    .barriers
                    .extend(predecessors.iter().map(|block| coordinates.ranks[block]));
            }
        }
        projected
    }
}

impl ProjectedSupport<'_> {
    fn add_lexical_visibility(
        &mut self,
        checker: &BorrowChecker<'_>,
    ) -> Result<(), BorrowCheckError> {
        let coordinates = &self.function.transfer;
        let exits: FxHashSet<_> = self
            .local_ids
            .iter()
            .flat_map(|local| {
                coordinates
                    .exits_by_region
                    .get(&coordinates.local_regions[coordinates.local_indices[local]])
                    .into_iter()
                    .flatten()
                    .copied()
            })
            .collect();
        for target in exits {
            let target_region = checker.module.blocks[checker.block_index_by_id[&target]].region;
            let mut visible = RootSet::empty(self.local_ids.len());
            for (index, local) in self.local_ids.iter().enumerate() {
                let region = coordinates.local_regions[coordinates.local_indices[local]];
                if checker.is_region_ancestor_of(region, target_region, target)? {
                    visible.insert(index);
                }
            }
            // Compression must not skip a scope exit and revive an old local
            // when a later edge enters the region again.
            self.visibility.insert(target, visible);
            self.barriers.insert(coordinates.ranks[&target]);
            if let Some(predecessors) = coordinates.predecessors.get(&target) {
                self.barriers
                    .extend(predecessors.iter().map(|block| coordinates.ranks[block]));
            }
        }
        Ok(())
    }
    fn add_replay_lifetimes(&mut self, function: &HirFunction) {
        // Acyclic RPO ranks decrease on every traversed edge. After the last
        // selected operation using a local, its storage can remain reachable
        // through other live names but its spelling cannot be observed again.
        if !self.function.transfer.recurring.is_empty() {
            return;
        }
        self.last_use.resize(self.local_ids.len(), 0);
        for formal in &function.params {
            if let Some(&local) = self.local_indices.get(formal) {
                self.last_use[local] = 0;
            }
        }
        self.replay_retirements
            .extend(self.last_use.iter().copied().filter(|&rank| rank > 0));
        self.replay_retirements.sort_unstable();
        self.replay_retirements.dedup();
    }

    pub(in crate::compiler_frontend::analysis::borrow_checker) fn replay_visibility(
        &self,
        source: BlockId,
        target: BlockId,
    ) -> Option<RootSet> {
        let ranks = &self.function.transfer.ranks;
        let rank = ranks[&target];
        let index = self
            .replay_retirements
            .partition_point(|&last| last <= rank);
        let last = *self.replay_retirements.get(index)?;
        if last > ranks[&source] {
            return None;
        }
        let mut visible = RootSet::empty(self.local_ids.len());
        for (local, &last) in self.last_use.iter().enumerate() {
            if last <= rank {
                visible.insert(local);
            }
        }
        Some(visible)
    }

    pub(in crate::compiler_frontend::analysis::borrow_checker) fn continuation(
        &self,
        mut block: BlockId,
    ) -> BlockId {
        #[cfg(test)]
        if !self.compress {
            return block;
        }
        let transfer = &self.function.transfer;
        let rank = transfer.ranks[&block];
        let barrier = self
            .barriers
            .range(..=rank)
            .next_back()
            .copied()
            .unwrap_or(0);
        // In a DAG an interior block lies strictly between an origin and its
        // postdominator in RPO. Jump only when that whole interval is identity.
        for jumps in transfer.postdom_jumps.iter().rev() {
            if let Some(&next) = jumps.get(&block)
                && transfer.ranks[&next] >= barrier
            {
                block = next;
            }
        }
        block
    }

    pub(in crate::compiler_frontend::analysis::borrow_checker) fn ranks(
        &self,
    ) -> Rc<FxHashMap<BlockId, usize>> {
        Rc::clone(&self.function.transfer.ranks)
    }

    pub(in crate::compiler_frontend::analysis::borrow_checker) fn recurring_blocks(
        &self,
    ) -> Rc<FxHashSet<BlockId>> {
        Rc::clone(&self.function.transfer.recurring)
    }

    pub(in crate::compiler_frontend::analysis::borrow_checker) fn visibility(
        &self,
        block: BlockId,
    ) -> Option<&RootSet> {
        self.visibility.get(&block)
    }

    pub(in crate::compiler_frontend::analysis::borrow_checker) fn block_view(
        &self,
        block: BlockId,
    ) -> SparseBlockView<'_> {
        let coordinates = &self.function.transfer;
        let orders = &coordinates.blocks[&block];
        let positions = self
            .function
            .operations_by_support
            .get(&(self.support, block))
            .map(Box::as_ref)
            .unwrap_or_default();
        let count = orders.statements.len();
        let transfer_terminator = positions.last() == Some(&count);
        SparseBlockView {
            operations: OperationView {
                local_ids: &self.local_ids,
                local_indices: &self.local_indices,
                local_mutable: &self.mutable,
                local_has_in_place_mutation_surface: &self.heap,
                block_successors: &coordinates.successors,
                local_first_write_order: &self.first_write,
                acyclic_entry_block: coordinates.acyclic_entry,
                entry_roots: self.entry_roots.as_ref(),
            },
            statement_orders: &orders.statements,
            statement_positions: if transfer_terminator {
                &positions[..positions.len() - 1]
            } else {
                positions
            },
            terminator_order: orders.terminator,
            transfer_terminator,
            structural_calls: &orders.structural_calls,
            structural_terminator: orders.structural_terminator,
            call_components: self.function.call_components.get(&(self.support, block)),
        }
    }

    fn seed_entry(
        &self,
        bindings: impl IntoIterator<Item = (LocalId, BindingSpec)>,
    ) -> BorrowState {
        let mut entry = BorrowState::new_uninitialized(self.local_ids.len());
        for (local, binding) in bindings {
            entry.initialize_entry_binding(self.local_indices[&local], binding);
        }
        entry
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct ResultSource {
    parameter: usize,
    input: Box<[CallField]>,
}

#[derive(Default, PartialEq, Eq)]
struct ResultSources(BTreeMap<Box<[CallField]>, BTreeSet<ResultSource>>);

impl ResultSources {
    // A demand can select above a retained source or continue inside it.
    // Probe ancestors and then the contiguous range beneath the demand.
    fn select<'a>(
        &'a self,
        tail: &'a [CallField],
    ) -> impl Iterator<Item = (&'a ResultSource, &'a [CallField], &'a [CallField])> + 'a {
        let ancestors = (0..tail.len()).filter_map(move |length| {
            self.0
                .get(&tail[..length])
                .map(|sources| (sources, &[][..], &tail[length..]))
        });
        let descendants = self
            .0
            .range::<[CallField], _>((Included(tail), Unbounded))
            .take_while(move |(path, _)| path.starts_with(tail))
            .map(move |(path, sources)| (sources, &path[tail.len()..], &[][..]));
        ancestors
            .chain(descendants)
            .flat_map(|(sources, output, input)| {
                sources.iter().map(move |source| (source, output, input))
            })
    }

    fn from_settled_call(
        summary: &crate::compiler_frontend::public_call_summary::FunctionReturnAliasSummary,
        slot: Option<usize>,
    ) -> Self {
        use typed::{ObjectRef, ScalarSource, ScalarSourceBound, ValueSource};
        let mut sources = Self::default();
        let Some(facts) = summary.facts() else {
            return sources;
        };
        let (error, slot) = outcome_slot(slot);
        let _: Result<(), std::convert::Infallible> = facts.lane(error).visit_compatible(
            |_| Ok(None),
            |_, product| {
                for post in product
                    .components()
                    .iter()
                    .flat_map(|component| &component.alternatives)
                {
                    for (_, result) in post.results.iter().filter(|(output, _)| *output == slot) {
                        let mut retain =
                            |port: PortId| {
                                if let EntryPort::Value { parameter, path } =
                                    &facts.ports[port.0 as usize]
                                {
                                    sources.0.entry(Box::default()).or_default().insert(
                                        ResultSource {
                                            parameter: *parameter,
                                            input: path.clone(),
                                        },
                                    );
                                }
                            };
                        match &result.source {
                            ValueSource::Allocation(ObjectRef::Entry(port))
                            | ValueSource::Scalar(ScalarSource::EntryValue(port)) => retain(*port),
                            ValueSource::Scalar(ScalarSource::Unresolved(
                                ScalarSourceBound::Values(ports),
                            )) => {
                                ports.iter().copied().for_each(retain);
                            }
                            _ => {}
                        }
                    }
                }
                Ok(())
            },
        );
        sources
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum ResultDemand {
    Value(HirValueId, Box<[CallField]>),
    Place(IndexedPlace),
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum CompatibilityConstraint {
    Equality(IdentityKind, Vec<PortId>),
    Reach(Vec<PortId>),
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct IndexCounts {
    pub(super) blocks: usize,
    pub(super) index_builds: usize,
    pub(super) statements: usize,
    /// HIR operands counted once per snapshot, not probes on every build.
    pub(super) arguments: usize,
    pub(super) access_incidences: usize,
    pub(super) selector_incidences: usize,
    pub(super) reach_incidences: usize,
    pub(super) compatibility_candidates: usize,
    pub(super) support_candidates: usize,
    pub(super) reverse_visits: usize,
}

impl std::ops::AddAssign for IndexCounts {
    fn add_assign(&mut self, other: Self) {
        self.blocks += other.blocks;
        self.index_builds += other.index_builds;
        self.statements += other.statements;
        self.arguments += other.arguments;
        self.access_incidences += other.access_incidences;
        self.selector_incidences += other.selector_incidences;
        self.reach_incidences += other.reach_incidences;
        self.compatibility_candidates += other.compatibility_candidates;
        self.support_candidates += other.support_candidates;
        self.reverse_visits += other.reverse_visits;
    }
}

impl IndexCounts {
    fn record(self, entry_ports: usize) {
        use FrontendCounter::*;
        for (counter, count) in [
            (BorrowIndexEntryPorts, entry_ports),
            (BorrowIndexAccessIncidences, self.access_incidences),
            (BorrowIndexSelectorIncidences, self.selector_incidences),
            (BorrowIndexReachIncidences, self.reach_incidences),
            (
                BorrowIndexCompatibilityCandidates,
                self.compatibility_candidates,
            ),
            (BorrowIndexSupportCandidates, self.support_candidates),
            (BorrowIndexReverseVisits, self.reverse_visits),
        ] {
            add_frontend_counter(counter, count);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum DemandKind {
    Value,
    Cell,
    Retarget,
    Reach(ReachKind),
}

impl DemandKind {
    fn writes(self) -> bool {
        matches!(
            self,
            Self::Cell | Self::Retarget | Self::Reach(ReachKind::EntryStructure)
        )
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct IndexedPlace {
    local: LocalId,
    path: Box<[CallField]>,
}

#[derive(Clone, Copy)]
enum Input {
    Value(HirValueId),
    Jump(LocalId),
    Member(HirValueId),
    CallResult(usize, Option<usize>),
}

// HIR IDs deliberately have no global ordering. The block ordinal is only a
// stable order within the owning revision.
pub(super) type IndexedSite = (u32, usize);

struct IndexedCall<'a> {
    site: IndexedSite,
    target: &'a CallTarget,
    args: &'a [HirValueId],
    continuation: CallContinuation,
}

#[derive(Clone, Copy)]
enum CallContinuation {
    Statement,
    Invoke,
    Never,
}

#[derive(Clone, Copy)]
enum Observation {
    Place(HirPlace, DemandKind),
    Value(HirValueId),
    Member(HirValueId, DemandKind),
}

struct Snapshot<'a> {
    function: &'a HirFunction,
    blocks: Vec<BlockId>,
    writes: Vec<(IndexedSite, HirPlace, Option<HirValueId>)>,
    returns: Vec<(IndexedSite, bool, usize, HirValueId)>,
    maps: Vec<(IndexedSite, HirMapOp, HirValueId, &'a [HirValueId])>,
    definitions: FxHashMap<LocalId, Vec<(IndexedSite, Input)>>,
    aliases: FxHashMap<LocalId, Vec<HirPlace>>,
    calls: Vec<IndexedCall<'a>>,
    counts: IndexCounts,
}

impl<'a> Snapshot<'a> {
    fn call(
        &mut self,
        site: IndexedSite,
        target: &'a CallTarget,
        args: &'a [HirValueId],
        continuation: CallContinuation,
        results: impl IntoIterator<Item = (LocalId, Option<usize>)>,
    ) {
        self.counts.arguments += args.len();
        let index = self.calls.len();
        self.calls.push(IndexedCall {
            site,
            target,
            args,
            continuation,
        });
        for (local, slot) in results {
            self.definitions
                .entry(local)
                .or_default()
                .push((site, Input::CallResult(index, slot)));
        }
    }

    fn map(
        &mut self,
        site: IndexedSite,
        op: HirMapOp,
        receiver: HirValueId,
        args: &'a [HirValueId],
        result: Option<LocalId>,
    ) {
        self.counts.arguments += 1 + args.len();
        self.maps.push((site, op, receiver, args));
        if matches!(op, HirMapOp::Get | HirMapOp::Remove)
            && let Some(local) = result
        {
            self.definitions
                .entry(local)
                .or_default()
                .push((site, Input::Member(receiver)));
        }
    }

    fn collect(
        checker: &BorrowChecker<'a>,
        function: &'a HirFunction,
    ) -> Result<Self, BorrowCheckError> {
        let blocks = checker.collect_reachable_blocks(function)?;
        let mut result = Self {
            function,
            blocks: Vec::new(),
            writes: Vec::new(),
            returns: Vec::new(),
            maps: Vec::new(),
            definitions: FxHashMap::default(),
            aliases: FxHashMap::default(),
            calls: Vec::new(),
            counts: IndexCounts::default(),
        };
        for &id in &blocks {
            let block = checker
                .module
                .blocks
                .get(checker.block_index_by_id[&id])
                .ok_or_else(|| {
                    checker
                        .diagnostics
                        .internal_error("Missing block in sparse borrow index")
                })?;
            let expressions = &checker.module.expressions;
            result.counts.blocks += 1;
            result.counts.statements += block.statements.len();
            for (position, statement) in block.statements.iter().enumerate() {
                result.collect_statement((id.0, position), &statement.kind, expressions);
            }
            result.collect_terminator(
                (id.0, block.statements.len()),
                &block.terminator,
                expressions,
            );
        }
        result.blocks = blocks;
        Ok(result)
    }

    fn collect_statement(
        &mut self,
        site: IndexedSite,
        statement: &'a HirStatementKind,
        expressions: &'a HirExpressionStore,
    ) {
        match statement {
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                value,
            } => self.define_value(site, *local, *value, None, expressions),
            HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(place),
                value,
            } => {
                self.writes.push((site, *place, Some(*value)));
                if place.projections.is_empty() {
                    self.define_value(site, place.root, *value, Some(*place), expressions);
                }
            }
            HirStatementKind::Call {
                target,
                args,
                results,
            } => {
                self.call(
                    site,
                    target,
                    expressions.values(*args),
                    CallContinuation::Statement,
                    results
                        .iter()
                        .enumerate()
                        .map(|(slot, dest)| (dest.local(), Some(slot))),
                );
                for &destination in results {
                    self.collect_update(site, destination);
                }
            }
            HirStatementKind::MapOp {
                op,
                receiver,
                args,
                result,
            } => self.map(
                site,
                *op,
                *receiver,
                expressions.values(*args),
                result.map(HirLocalDestination::local),
            ),
            HirStatementKind::NumericOp { result, .. }
            | HirStatementKind::FastStoreConversion { result, .. }
            | HirStatementKind::LoopStepFailure { result, .. }
            | HirStatementKind::FormatFloat { result, .. }
            | HirStatementKind::ValidateFloat { result, .. } => {
                self.collect_update(site, *result);
            }
            HirStatementKind::RangeCandidate {
                candidate_result,
                in_range_result,
                ..
            } => {
                self.collect_update(site, *candidate_result);
                self.collect_update(site, *in_range_result);
            }
            _ => {}
        }
    }

    fn collect_update(&mut self, site: IndexedSite, destination: HirLocalDestination) {
        if let HirLocalDestination::Update(local) = destination {
            self.writes.push((site, HirPlace::local(local), None));
        }
    }

    fn define_value(
        &mut self,
        site: IndexedSite,
        local: LocalId,
        value: HirValueId,
        update_target: Option<HirPlace>,
        expressions: &HirExpressionStore,
    ) {
        self.definitions
            .entry(local)
            .or_default()
            .push((site, Input::Value(value)));
        let expression = expressions.expression(value);
        if expression.value_kind == ValueKind::Place
            && let HirExpressionKind::Load(source) = expression.kind
            && Some(source) != update_target
        {
            self.aliases.entry(local).or_default().push(source);
        }
    }

    fn collect_terminator(
        &mut self,
        site: IndexedSite,
        terminator: &'a HirTerminator,
        expressions: &'a HirExpressionStore,
    ) {
        match terminator {
            HirTerminator::Jump { args, .. } => {
                self.counts.arguments += args.len();
                for argument in args {
                    self.definitions
                        .entry(argument.destination)
                        .or_default()
                        .push((site, Input::Jump(argument.source)));
                }
            }
            HirTerminator::Invoke {
                operation: HirFallibleOperation::Call { target, args },
                success_results,
                error_result,
                ..
            } => self.call(
                site,
                target,
                expressions.values(*args),
                CallContinuation::Invoke,
                success_results
                    .iter()
                    .enumerate()
                    .map(|(slot, &local)| (local, Some(slot)))
                    .chain([(*error_result, None)]),
            ),
            HirTerminator::Invoke {
                operation: HirFallibleOperation::MapOp { op, receiver, args },
                success_results,
                ..
            } => self.map(
                site,
                *op,
                *receiver,
                expressions.values(*args),
                success_results.first().copied(),
            ),
            HirTerminator::NeverCall { target, args } => self.call(
                site,
                target,
                expressions.values(*args),
                CallContinuation::Never,
                [],
            ),
            HirTerminator::Return(values) => {
                self.returns.extend(
                    expressions
                        .values(*values)
                        .iter()
                        .enumerate()
                        .map(|(slot, &value)| (site, false, slot, value)),
                );
            }
            HirTerminator::ReturnError(value) => self.returns.push((site, true, 0, *value)),
            _ => {}
        }
    }
}

#[derive(Default)]
struct Supports(Vec<usize>);

impl Supports {
    fn insert(&mut self) -> usize {
        let id = self.0.len();
        self.0.push(id);
        id
    }

    fn root(&mut self, mut node: usize) -> usize {
        while self.0[node] != node {
            self.0[node] = self.0[self.0[node]];
            node = self.0[node];
        }
        node
    }

    fn couple(&mut self, left: usize, right: usize) {
        let left = self.root(left);
        let right = self.root(right);
        self.0[left.max(right)] = left.min(right);
    }

    fn connect_equality(
        &mut self,
        kind: IdentityKind,
        ports: &BTreeSet<PortId>,
        nodes: &[usize],
        counts: &mut IndexCounts,
        guards: &mut BTreeSet<Predicate>,
        broad: &mut BTreeSet<CompatibilityConstraint>,
    ) {
        let Some(&first) = ports.first() else {
            return;
        };
        for &other in ports.iter().skip(1) {
            counts.compatibility_candidates += 1;
            counts.support_candidates += 1;
            self.couple(nodes[first.0 as usize], nodes[other.0 as usize]);
        }
        match ports.len() {
            2 => {
                let last = *ports.last().expect("two indexed equality candidates");
                guards.insert(Predicate::Equal(kind, first, last));
            }
            3.. => {
                broad.insert(CompatibilityConstraint::Equality(
                    kind,
                    ports.iter().copied().collect(),
                ));
            }
            _ => {}
        }
    }
}

struct DemandBuilder<'a, 'module> {
    checker: &'a BorrowChecker<'module>,
    call_contracts: Vec<BorrowedCall<'a>>,
    snapshot: &'a Snapshot<'module>,
    callees: &'a FxHashMap<FunctionId, SparseFunction>,
    field_ordinals: &'a FxHashMap<FieldId, CallField>,
    parameter_indices: FxHashMap<LocalId, usize>,
    counts: IndexCounts,
    supports: Supports,
    sites: BTreeMap<IndexedSite, Vec<usize>>,
    call_components: Vec<(IndexedSite, SupportId, usize)>,
    result_nodes: BTreeMap<(bool, usize), usize>,
    local_nodes: FxHashMap<(IndexedPlace, DemandKind), usize>,
    ports: Vec<EntryPort>,
    port_index: FxHashMap<EntryPort, PortId>,
    port_nodes: Vec<usize>,
    // Match locations by the containing allocation's type and the selector,
    // not by the type of what was previously stored there.
    selector_index: BTreeMap<(TypeId, CallField), BTreeMap<PortId, Vec<usize>>>,
    written_selectors: BTreeSet<(TypeId, CallField)>,
    holders_by_type: BTreeMap<TypeId, BTreeMap<PortId, Vec<usize>>>,
    wide_reach: BTreeMap<TypeId, BTreeSet<PortId>>,
    terminals_by_type: BTreeMap<TypeId, BTreeSet<PortId>>,
    edges_by_value_type: BTreeMap<TypeId, BTreeMap<PortId, Vec<usize>>>,
    wrappers_by_type: BTreeMap<TypeId, BTreeSet<PortId>>,
    demanded_roles: BTreeSet<Predicate>,
    constructed_fields: FxHashMap<HirValueId, FxHashMap<CallField, HirValueId>>,
    envelope: BTreeMap<(usize, Box<[CallField]>, DemandKind), usize>,
    work: VecDeque<(IndexedPlace, DemandKind, usize)>,
    value_work: VecDeque<(HirValueId, Box<[CallField]>, DemandKind, usize)>,
    value_nodes: FxHashMap<(HirValueId, Box<[CallField]>, DemandKind), usize>,
    path_ancestors: Vec<TypeId>,
}

impl<'a, 'module> DemandBuilder<'a, 'module> {
    fn new(
        checker: &'a BorrowChecker<'module>,
        snapshot: &'a Snapshot<'module>,
        field_ordinals: &'a FxHashMap<FieldId, CallField>,
        callees: &'a FxHashMap<FunctionId, SparseFunction>,
    ) -> Self {
        Self {
            checker,
            callees,
            call_contracts: Vec::with_capacity(snapshot.calls.len()),
            snapshot,
            field_ordinals,
            parameter_indices: snapshot
                .function
                .params
                .iter()
                .enumerate()
                .map(|(index, &local)| (local, index))
                .collect(),
            counts: IndexCounts {
                index_builds: 1,
                ..IndexCounts::default()
            },
            supports: Supports::default(),
            sites: BTreeMap::new(),
            call_components: Vec::new(),
            result_nodes: BTreeMap::new(),
            local_nodes: FxHashMap::default(),
            ports: Vec::new(),
            port_index: FxHashMap::default(),
            port_nodes: Vec::new(),
            terminals_by_type: BTreeMap::new(),
            edges_by_value_type: BTreeMap::new(),
            wrappers_by_type: BTreeMap::new(),
            demanded_roles: BTreeSet::new(),
            constructed_fields: FxHashMap::default(),
            selector_index: BTreeMap::new(),
            written_selectors: BTreeSet::new(),
            holders_by_type: BTreeMap::new(),
            wide_reach: BTreeMap::new(),
            envelope: BTreeMap::new(),
            work: VecDeque::new(),
            value_work: VecDeque::new(),
            value_nodes: FxHashMap::default(),
            path_ancestors: Vec::new(),
        }
    }

    fn couple(&mut self, left: usize, right: usize) {
        self.counts.support_candidates += 1;
        self.supports.couple(left, right);
    }

    fn port(&mut self, port: EntryPort, dependent: Option<usize>) -> PortId {
        let id = match self.port_index.entry(port) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let id = PortId(u32::try_from(self.ports.len()).expect("HIR demand port capacity"));
                self.ports.push(entry.key().clone());
                entry.insert(id);
                self.port_nodes.push(self.supports.insert());
                id
            }
        };
        if let Some(node) = dependent {
            self.couple(node, self.port_nodes[id.0 as usize]);
        }
        id
    }

    fn site(&mut self, site: IndexedSite) -> usize {
        let node = self.supports.insert();
        self.sites.entry(site).or_default().push(node);
        node
    }

    fn place_path(&self, place: HirPlace, tail: &[CallField]) -> IndexedPlace {
        let projections = self
            .checker
            .module
            .expressions
            .projections(place.projections);
        let path = projections
            .iter()
            .map(|projection| match projection {
                HirProjection::Field(field) => self.field_ordinals[field],
                HirProjection::Index(_) => CallField::Elem,
            })
            .chain(tail.iter().copied())
            .collect();
        IndexedPlace {
            local: place.root,
            path,
        }
    }

    fn demand_place(&mut self, mut place: IndexedPlace, mut kind: DemandKind, node: usize) {
        self.counts.access_incidences += 1;
        let prefix = self.finite_prefix(self.checker.local_types[&place.local], &place.path);
        if prefix != place.path.len() {
            // A recursive layout tail is a typed reach family, not a
            // new coordinate each time this path crosses a call or loop.
            place.path = place.path[..prefix].into();
            kind = DemandKind::Reach(if kind.writes() {
                ReachKind::EntryStructure
            } else {
                ReachKind::Referent
            });
        }
        match self.local_nodes.entry((place, kind)) {
            Entry::Occupied(entry) => {
                let first = *entry.get();
                self.couple(first, node);
            }
            Entry::Vacant(entry) => {
                self.work.push_back((entry.key().0.clone(), kind, node));
                entry.insert(node);
            }
        }
    }

    fn demand_value(
        &mut self,
        value: HirValueId,
        path: &[CallField],
        kind: DemandKind,
        node: usize,
    ) {
        self.demand_value_path(value, path.into(), kind, node);
    }

    fn demand_value_path(
        &mut self,
        value: HirValueId,
        path: Box<[CallField]>,
        kind: DemandKind,
        node: usize,
    ) {
        match self.value_nodes.entry((value, path, kind)) {
            Entry::Occupied(entry) => {
                let first = *entry.get();
                self.couple(first, node);
            }
            Entry::Vacant(entry) => {
                self.value_work
                    .push_back((value, entry.key().1.clone(), kind, node));
                entry.insert(node);
            }
        }
    }

    fn entry(&mut self, param: usize, place: &IndexedPlace, kind: DemandKind, node: usize) {
        let path = place.path.as_ref();
        let mut ty = self.checker.local_types[&place.local];
        let access =
            self.checker.public_call_summaries[&self.snapshot.function.id].parameters[param].access;
        let writable = access == PublicCallParameterAccess::Mutable;
        if kind.writes() && !writable {
            return;
        }
        self.envelope
            .insert((param, place.path.clone(), kind), node);
        if path.is_empty()
            && matches!(
                kind,
                DemandKind::Cell | DemandKind::Retarget | DemandKind::Reach(ReachKind::Cell)
            )
        {
            let terminal = self.port(EntryPort::Terminal(param), Some(node));
            self.terminals_by_type
                .entry(ty)
                .or_default()
                .insert(terminal);
            if kind != DemandKind::Cell {
                let wrapper = self.port(EntryPort::Wrapper(param), Some(node));
                self.wrappers_by_type.entry(ty).or_default().insert(wrapper);
                self.demanded_roles
                    .insert(Predicate::Role(wrapper, Role::Slot));
                if kind == DemandKind::Reach(ReachKind::Cell) {
                    self.demanded_roles
                        .insert(Predicate::Role(wrapper, Role::WholeAlias));
                }
                self.port(EntryPort::Immediate(param), Some(node));
            }
            return;
        }
        for (index, &selector) in path.iter().enumerate() {
            self.counts.selector_incidences += 1;
            let final_cell =
                kind.writes() && index + 1 == path.len() && !matches!(kind, DemandKind::Reach(_));
            let holder = self.port(
                EntryPort::Value {
                    parameter: param,
                    path: path[..index].into(),
                },
                None,
            );
            self.holders_by_type
                .entry(ty)
                .or_default()
                .entry(holder)
                .or_default()
                .push(node);
            let edge = self.port(
                EntryPort::Edge(holder, selector),
                final_cell.then_some(node),
            );
            self.selector_index
                .entry((ty, selector))
                .or_default()
                .entry(holder)
                .or_default()
                .push(node);
            if final_cell {
                self.written_selectors.insert((ty, selector));
            }
            ty = self
                .checker
                .call_child_type(ty, selector)
                .expect("checked entry path");
            if !selector.is_collapsed() {
                self.edges_by_value_type
                    .entry(ty)
                    .or_default()
                    .entry(edge)
                    .or_default()
                    .push(node);
            }
        }
        if matches!(kind, DemandKind::Value | DemandKind::Reach(_)) {
            let port = self.port(
                EntryPort::Value {
                    parameter: param,
                    path: path.into(),
                },
                Some(node),
            );
            if path.is_empty()
                && let Some(&wrapper) = self.port_index.get(&EntryPort::Wrapper(param))
            {
                self.demanded_roles
                    .insert(Predicate::DerivedFrom(port, wrapper));
            }
            if matches!(kind, DemandKind::Reach(_))
                && classify_hir_type(ty, self.checker.type_environment)
                    .is_ok_and(|shape| shape.has_in_place_mutation_surface())
            {
                self.counts.reach_incidences += 1;
                self.wide_reach.entry(ty).or_default().insert(port);
            }
        }
    }

    fn visit_place(&mut self, place: IndexedPlace, kind: DemandKind, node: usize) {
        if let Some(&param) = self.parameter_indices.get(&place.local) {
            self.entry(param, &place, kind, node);
        }
        let definitions = self.snapshot.definitions.get(&place.local);
        if let Some(definitions) = definitions {
            for &(site, _) in definitions {
                self.counts.reverse_visits += 1;
                self.sites.entry(site).or_default().push(node);
            }
        }
        if kind.writes() {
            if let Some(sources) = self.snapshot.aliases.get(&place.local) {
                for &source in sources {
                    let next = self.place_path(source, &place.path);
                    let kind = if next.path.is_empty() {
                        kind
                    } else if kind == DemandKind::Retarget {
                        DemandKind::Cell
                    } else {
                        kind
                    };
                    self.demand_place(next, kind, node);
                }
            }
            if place.path.is_empty() && !matches!(kind, DemandKind::Reach(_)) {
                return;
            }
        }
        if let Some(definitions) = definitions {
            for &(_, source) in definitions {
                self.counts.reverse_visits += 1;
                match source {
                    Input::Value(value) => self.demand_value(value, &place.path, kind, node),
                    Input::Jump(local) => self.demand_place(
                        IndexedPlace {
                            local,
                            path: place.path.clone(),
                        },
                        kind,
                        node,
                    ),
                    Input::Member(receiver) => {
                        self.demand_member(receiver, &place.path, kind, node);
                    }
                    Input::CallResult(call_index, slot) => {
                        let call = &self.snapshot.calls[call_index];
                        if let Some(effects) = self.sites.get(&call.site) {
                            for &effect in effects {
                                self.counts.support_candidates += 1;
                                self.supports.couple(node, effect);
                            }
                        }
                        if let CallTarget::Local(target) = call.target
                            && let Some(&callee_support) = self
                                .callees
                                .get(target)
                                .and_then(|callee| callee.result_supports.get(&outcome_slot(slot)))
                        {
                            let site = self.site(call.site);
                            self.couple(node, site);
                            self.call_components.push((call.site, callee_support, node));
                        }
                        if slot == Some(0)
                            && let Some(receiver) = call_member_result(call.target)
                        {
                            self.demand_member(call.args[receiver], &place.path, kind, node);
                            continue;
                        }
                        let reach = if kind.writes() {
                            ReachKind::EntryStructure
                        } else {
                            ReachKind::Referent
                        };
                        match self.call_contracts[call_index].returns() {
                            CallReturns::External(slots) => {
                                if let Some(slot) = slot.and_then(|index| slots.get(index))
                                    && let ExternalReturnAlias::AliasArgs(indices) = &slot.alias
                                {
                                    for &index in indices {
                                        let actual = call.args[index];
                                        self.demand_opaque_result(
                                            actual, &place, kind, reach, node,
                                        );
                                    }
                                }
                            }
                            CallReturns::Summary(summary) => {
                                if let CallTarget::Local(target) = call.target {
                                    for &actual in call.args {
                                        self.demand_value(
                                            actual,
                                            &[],
                                            DemandKind::Reach(reach),
                                            node,
                                        );
                                    }
                                    if let Some(sources) =
                                        self.callees.get(target).and_then(|callee| {
                                            callee.result_sources.get(&outcome_slot(slot))
                                        })
                                    {
                                        self.counts.reverse_visits += place.path.len() + 1;
                                        for (source, _, tail) in sources.select(&place.path) {
                                            self.counts.reverse_visits += 1;
                                            if kind.writes()
                                                && !matches!(kind, DemandKind::Reach(_))
                                                && tail.is_empty()
                                            {
                                                // A cell allocated by the callee is not a cell of its actual.
                                                continue;
                                            }
                                            let path = join_selectors(&source.input, tail);
                                            self.demand_value_path(
                                                call.args[source.parameter],
                                                path,
                                                kind,
                                                node,
                                            );
                                        }
                                    }
                                } else {
                                    for &actual in call.args {
                                        self.demand_opaque_result(
                                            actual, &place, kind, reach, node,
                                        );
                                    }
                                    let sources = ResultSources::from_settled_call(summary, slot);
                                    for (source, _, tail) in sources.select(&place.path) {
                                        if kind.writes()
                                            && !matches!(kind, DemandKind::Reach(_))
                                            && tail.is_empty()
                                        {
                                            continue;
                                        }
                                        self.demand_value_path(
                                            call.args[source.parameter],
                                            join_selectors(&source.input, tail),
                                            kind,
                                            node,
                                        );
                                    }
                                }
                            }
                            CallReturns::Fresh => {}
                        }
                    }
                }
            }
        }
    }

    fn demand_opaque_result(
        &mut self,
        actual: HirValueId,
        result: &IndexedPlace,
        kind: DemandKind,
        reach: ReachKind,
        node: usize,
    ) {
        self.demand_value(actual, &[], DemandKind::Reach(reach), node);
        // An opaque whole-argument alias provides no construction route.
        // Its selectors can only be refined when the result has the same type.
        if !result.path.is_empty()
            && self.checker.module.expressions.expression(actual).ty
                == self.checker.local_types[&result.local]
        {
            self.demand_value(actual, &result.path, kind, node);
        }
    }

    fn demand_member(
        &mut self,
        receiver: HirValueId,
        tail: &[CallField],
        kind: DemandKind,
        node: usize,
    ) {
        let path = join_selectors(&[CallField::Elem], tail);
        self.demand_value_path(receiver, path, kind, node);
        self.demand_value(receiver, &[], DemandKind::Reach(ReachKind::Referent), node);
    }

    fn constructed_field(&mut self, value: HirValueId, selector: CallField) -> Option<HirValueId> {
        let fields = self.constructed_fields.entry(value).or_insert_with(|| {
            let expressions = &self.checker.module.expressions;
            let HirExpressionKind::StructConstruct { fields, .. } =
                expressions.expression(value).kind
            else {
                unreachable!("only struct constructions have indexed fields");
            };
            let fields = expressions.struct_fields(fields);
            self.counts.reach_incidences += fields.len();
            fields
                .iter()
                .map(|&(field, value)| (self.field_ordinals[&field], value))
                .collect()
        });
        fields.get(&selector).copied()
    }

    fn visit_value(
        &mut self,
        value: HirValueId,
        tail: &[CallField],
        kind: DemandKind,
        node: usize,
    ) {
        let checker = self.checker;
        let expressions = &checker.module.expressions;
        match &expressions.expression(value).kind {
            HirExpressionKind::Load(place) => {
                let place = self.place_path(*place, tail);
                self.demand_place(place, kind, node);
            }
            HirExpressionKind::Copy(place) => {
                let source = self.place_path(*place, tail);
                self.demand_place(source, DemandKind::Value, node);
            }
            HirExpressionKind::VariantPayloadGet {
                source,
                variant_index,
                field_index,
                ..
            } => {
                let selector = payload(*variant_index, *field_index);
                let path = join_selectors(&[selector], tail);
                self.demand_value_path(*source, path, kind, node);
            }
            kind_expression if kind_expression.is_aggregate_construction() => {
                let selected = tail.split_first();
                if kind.writes() && !matches!(kind, DemandKind::Reach(_)) && tail.len() == 1 {
                    // Replacing a fresh aggregate's own cell cannot write
                    // through the object previously stored in that cell.
                    return;
                }
                match (kind_expression, selected) {
                    (
                        HirExpressionKind::StructConstruct { .. },
                        Some((selector @ CallField::Field(_), tail)),
                    ) => {
                        if let Some(selected) = self.constructed_field(value, *selector) {
                            self.demand_value(selected, tail, kind, node);
                        }
                    }
                    (
                        HirExpressionKind::VariantConstruct {
                            fields,
                            variant_index,
                            ..
                        },
                        Some((CallField::Payload { variant, field }, tail)),
                    ) => {
                        if *variant as usize == *variant_index
                            && let Some(field) =
                                expressions.variant_fields(*fields).get(*field as usize)
                        {
                            self.demand_value(field.value, tail, kind, node);
                        }
                    }
                    (HirExpressionKind::Collection(values), Some((CallField::Elem, tail))) => {
                        for &value in expressions.values(*values) {
                            self.demand_value(value, tail, kind, node);
                        }
                    }
                    (
                        HirExpressionKind::MapLiteral(entries),
                        Some((selector @ (CallField::Elem | CallField::MapKey), tail)),
                    ) => {
                        for entry in expressions.map_entries(*entries) {
                            let value = if *selector == CallField::Elem {
                                entry.value
                            } else {
                                entry.key
                            };
                            self.demand_value(value, tail, kind, node);
                        }
                    }
                    _ => {
                        for child in expressions.child_values(value) {
                            self.demand_value(child, &[], DemandKind::Value, node);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn drain_demands(&mut self) {
        loop {
            while let Some((place, kind, node)) = self.work.pop_front() {
                self.counts.reverse_visits += 1;
                self.visit_place(place, kind, node);
            }
            let Some((value, path, kind, node)) = self.value_work.pop_front() else {
                break;
            };
            self.counts.reverse_visits += 1;
            self.visit_value(value, &path, kind, node);
        }
    }

    fn collect_roots(&mut self) {
        let expressions = &self.checker.module.expressions;
        for &(site, place, value) in &self.snapshot.writes {
            let node = self.site(site);
            let retargets = place.projections.is_empty()
                && value.is_some_and(|value| {
                    let expression = expressions.expression(value);
                    expression.value_kind == ValueKind::Place
                        && !HirWriteTarget::AssignPlace(place).is_direct_self_update_of(expression)
                });
            let kind = if retargets {
                DemandKind::Retarget
            } else {
                DemandKind::Cell
            };
            self.observe(Observation::Place(place, kind), node);
            if let Some(value) = value {
                self.observe(Observation::Value(value), node);
                if retargets
                    && let HirExpressionKind::Load(source) = expressions.expression(value).kind
                {
                    self.observe(
                        Observation::Place(source, DemandKind::Reach(ReachKind::Cell)),
                        node,
                    );
                }
            }
        }

        for &(site, op, receiver, args) in &self.snapshot.maps {
            if op.requires_mutable_receiver() || op == HirMapOp::Get {
                let node = self.site(site);
                let kind = if op.requires_mutable_receiver() {
                    ReachKind::EntryStructure
                } else {
                    ReachKind::Referent
                };
                self.observe(Observation::Member(receiver, DemandKind::Reach(kind)), node);
                if op.requires_mutable_receiver() {
                    self.demand_value(receiver, &[CallField::Elem], DemandKind::Cell, node);
                    self.demand_value(receiver, &[CallField::MapKey], DemandKind::Cell, node);
                }
                if op == HirMapOp::Set {
                    for &argument in args {
                        self.observe(Observation::Value(argument), node);
                    }
                }
            }
        }

        for &(site, error, slot, value) in &self.snapshot.returns {
            let node = self.site(site);
            if let Some(&previous) = self.result_nodes.get(&(error, slot)) {
                self.couple(previous, node);
            } else {
                self.result_nodes.insert((error, slot), node);
            }
            self.observe(Observation::Value(value), node);
        }
    }

    fn collect_call_roots(&mut self) -> Result<(), BorrowCheckError> {
        let checker = self.checker;
        let expressions = &checker.module.expressions;
        let signatures = checker.call_signatures();
        for call in &self.snapshot.calls {
            self.call_contracts.push(signatures.resolve(
                call.target,
                call.args.len(),
                &checker.diagnostics,
            )?);
        }
        for (position, call) in self.snapshot.calls.iter().enumerate() {
            if let CallTarget::Local(target) = call.target {
                if let Some(callee) = self.callees.get(target) {
                    // Project an entire callee component into an actual support.
                    // Independently certified callee effects are not all joined
                    // just because they occur at one call site.
                    let mut nodes = FxHashMap::default();
                    for ((parameter, path, kind), &support) in &callee.envelope {
                        let Some(&actual) = call.args.get(*parameter) else {
                            return Err(self
                                .checker
                                .diagnostics
                                .internal_error("Sparse call argument is missing"));
                        };
                        let node = *nodes.entry(support).or_insert_with(|| {
                            let node = self.site(call.site);
                            self.call_components.push((call.site, support, node));
                            node
                        });
                        self.demand_value(actual, path, *kind, node);
                    }
                }
                continue;
            }

            let contract = self.call_contracts[position];
            let is_mutable = |index| contract.is_mutable(index);
            if (0..call.args.len()).any(is_mutable) {
                let node = self.site(call.site);
                let member_writes = call_member_writes(call.target);
                let source_summary = matches!(contract.returns(), CallReturns::Summary(_));
                if let Some(&receiver) = call.args.first() {
                    for &field in member_writes {
                        self.demand_value(receiver, &[field], DemandKind::Cell, node);
                    }
                }
                for (index, &actual) in call.args.iter().enumerate() {
                    if is_mutable(index) {
                        if member_writes.is_empty() {
                            self.demand_value(actual, &[], DemandKind::Retarget, node);
                        }
                        if is_heap(self.checker, expressions.expression(actual).ty)? {
                            self.demand_value(
                                actual,
                                &[],
                                DemandKind::Reach(ReachKind::EntryStructure),
                                node,
                            );
                            if source_summary {
                                self.demand_non_local_cells(
                                    actual,
                                    expressions.expression(actual).ty,
                                    node,
                                );
                            }
                        }
                    }
                    self.demand_value(actual, &[], DemandKind::Value, node);
                }
            }
        }
        Ok(())
    }

    fn demand_non_local_cells(&mut self, actual: HirValueId, ty: TypeId, node: usize) {
        // A generated callee may settle after this support is built. Use its
        // signature's writable selectors, not the first delivered set of edits.
        let env = self.checker.type_environment;
        let mut pending = vec![(ty, Vec::new())];
        while let Some((ty, path)) = pending.pop() {
            let _: Result<(), std::convert::Infallible> =
                CallField::for_each_child(ty, env, |field, child| {
                    let mut child_path = path.clone();
                    child_path.push(field);
                    self.demand_value(actual, &child_path, DemandKind::Cell, node);
                    if !field.is_collapsed() {
                        // A family owns everything below its collapsed selector.
                        pending.push((child, child_path));
                    }
                    Ok(())
                });
        }
    }

    fn observe(&mut self, observation: Observation, node: usize) {
        match observation {
            Observation::Place(place, kind) => {
                let place = self.place_path(place, &[]);
                self.demand_place(place, kind, node);
            }
            Observation::Value(value) => self.demand_value(value, &[], DemandKind::Value, node),
            Observation::Member(receiver, kind) => self.demand_value(receiver, &[], kind, node),
        }
    }

    // These incidences conservatively couple supports. Admitting a resolved
    // presentation uses the transfer's typed footprint overlap, not these buckets.
    fn connect_compatibility(&mut self) -> (Vec<Predicate>, Vec<CompatibilityConstraint>) {
        let mut guards = std::mem::take(&mut self.demanded_roles);
        let mut broad = BTreeSet::new();
        for (ty, mut terminals) in std::mem::take(&mut self.terminals_by_type) {
            if let Some(edges) = self.edges_by_value_type.get(&ty) {
                for (&edge, incidences) in edges {
                    terminals.insert(edge);
                    for &site in incidences {
                        self.counts.support_candidates += 1;
                        self.supports.couple(self.port_nodes[edge.0 as usize], site);
                    }
                }
            }
            self.supports.connect_equality(
                IdentityKind::Cell,
                &terminals,
                &self.port_nodes,
                &mut self.counts,
                &mut guards,
                &mut broad,
            );
        }
        for wrappers in self.wrappers_by_type.values() {
            self.supports.connect_equality(
                IdentityKind::Wrapper,
                wrappers,
                &self.port_nodes,
                &mut self.counts,
                &mut guards,
                &mut broad,
            );
        }
        for key in &self.written_selectors {
            let holders = &self.selector_index[key];
            let mut candidates = holders.iter();
            let Some((&first, first_sites)) = candidates.next() else {
                continue;
            };
            let first_site = first_sites[0];
            // An exact selector shared by reads and writes is also a
            // dependency when there is just one observed holder.
            for &site in &first_sites[1..] {
                self.counts.support_candidates += 1;
                self.supports.couple(first_site, site);
            }
            if holders.len() == 1 {
                continue;
            }
            let first_holder = self.port_nodes[first.0 as usize];
            self.counts.support_candidates += 1;
            self.supports.couple(first_holder, first_site);
            for (&port, sites) in candidates {
                self.counts.compatibility_candidates += 1;
                let holder_node = self.port_nodes[port.0 as usize];
                self.counts.support_candidates += 1;
                self.supports.couple(first_holder, holder_node);
                for &site in sites {
                    self.counts.support_candidates += 1;
                    self.supports.couple(holder_node, site);
                }
            }
            if holders.len() == 2 {
                let last = *holders.last_key_value().expect("two indexed holders").0;
                guards.insert(Predicate::Equal(IdentityKind::Allocation, first, last));
            } else {
                broad.insert(CompatibilityConstraint::Equality(
                    IdentityKind::Allocation,
                    holders.keys().copied().collect(),
                ));
            }
        }
        let mut remaining = Vec::new();
        for (ty, mut compatible) in std::mem::take(&mut self.wide_reach) {
            // An unknown write can affect a reached child as well as its root.
            // One constraint represents the compatible set without pairing
            // the roots or building a concrete occurrence graph.
            let mut seen = FxHashSet::default();
            remaining.push(ty);
            while let Some(ty) = remaining.pop() {
                self.counts.reach_incidences += 1;
                if !seen.insert(ty) {
                    continue;
                }
                if let Some(holders) = self.holders_by_type.get(&ty) {
                    self.counts.reach_incidences += holders.len();
                    for (&holder, sites) in holders {
                        compatible.insert(holder);
                        for &site in sites {
                            self.counts.support_candidates += 1;
                            self.supports
                                .couple(self.port_nodes[holder.0 as usize], site);
                        }
                    }
                }
                let result: Result<(), std::convert::Infallible> =
                    CallField::for_each_child(ty, self.checker.type_environment, |_, child| {
                        remaining.push(child);
                        Ok(())
                    });
                match result {
                    Ok(()) => {}
                    Err(never) => match never {},
                }
            }
            if let Some(first) = compatible.first() {
                for port in compatible.iter().skip(1) {
                    self.counts.compatibility_candidates += 1;
                    self.counts.support_candidates += 1;
                    self.supports.couple(
                        self.port_nodes[first.0 as usize],
                        self.port_nodes[port.0 as usize],
                    );
                }
                if compatible.len() > 1 {
                    broad.insert(CompatibilityConstraint::Reach(
                        compatible.into_iter().collect(),
                    ));
                }
            }
        }
        (guards.into_iter().collect(), broad.into_iter().collect())
    }

    fn connect_local_accesses(&mut self) {
        // A local can carry a fresh component that has no entry port. A
        // whole-value observation must still be coupled to writes beneath it.
        for ((place, demand), &node) in &self.local_nodes {
            let related = if demand.writes() {
                [
                    DemandKind::Value,
                    DemandKind::Reach(ReachKind::Referent),
                    DemandKind::Reach(ReachKind::EntryStructure),
                ]
            } else {
                [
                    DemandKind::Cell,
                    DemandKind::Retarget,
                    DemandKind::Reach(ReachKind::EntryStructure),
                ]
            };
            for end in 0..=place.path.len() {
                let mut key = (
                    IndexedPlace {
                        local: place.local,
                        path: place.path[..end].into(),
                    },
                    *demand,
                );
                for demand in related {
                    key.1 = demand;
                    self.counts.access_incidences += 1;
                    if let Some(&dependent) = self.local_nodes.get(&key) {
                        self.counts.support_candidates += 1;
                        self.supports.couple(node, dependent);
                    }
                }
            }
        }
    }

    fn collect_result_sources(&mut self) -> BTreeMap<(bool, usize), ResultSources> {
        let checker = self.checker;
        let expressions = &checker.module.expressions;
        let mut sources = BTreeMap::<_, ResultSources>::new();
        let mut visited = FxHashSet::default();
        let mut work = Vec::new();
        for &(_, error, slot, returned) in &self.snapshot.returns {
            let result_type = expressions.expression(returned).ty;
            work.push((
                ResultDemand::Value(returned, Box::new([])),
                Box::<[CallField]>::default(),
            ));
            while let Some((demand, output)) = work.pop() {
                self.counts.reverse_visits += 1;
                let (input_type, input) = match &demand {
                    ResultDemand::Value(value, input) => (expressions.expression(*value).ty, input),
                    ResultDemand::Place(place) => {
                        (self.checker.local_types[&place.local], &place.path)
                    }
                };
                if self.finite_prefix(input_type, input) != input.len()
                    || self.finite_prefix(result_type, &output) != output.len()
                    || !visited.insert(((error, slot), demand.clone(), output.clone()))
                {
                    continue;
                }
                match demand {
                    ResultDemand::Place(place) => {
                        if let Some(&parameter) = self.parameter_indices.get(&place.local) {
                            sources
                                .entry((error, slot))
                                .or_default()
                                .0
                                .entry(output.clone())
                                .or_default()
                                .insert(ResultSource {
                                    parameter,
                                    input: place.path.clone(),
                                });
                        }
                        for &(_, input) in self
                            .snapshot
                            .definitions
                            .get(&place.local)
                            .into_iter()
                            .flatten()
                        {
                            self.counts.reverse_visits += 1;
                            let actual = match input {
                                Input::Value(value) => {
                                    ResultDemand::Value(value, place.path.clone())
                                }
                                Input::Jump(local) => ResultDemand::Place(IndexedPlace {
                                    local,
                                    path: place.path.clone(),
                                }),
                                Input::Member(receiver) => ResultDemand::Value(
                                    receiver,
                                    join_selectors(&[CallField::Elem], &place.path),
                                ),
                                Input::CallResult(position, slot) => {
                                    let call = &self.snapshot.calls[position];
                                    if slot == Some(0)
                                        && let Some(receiver) = call_member_result(call.target)
                                    {
                                        work.push((
                                            ResultDemand::Value(
                                                call.args[receiver],
                                                join_selectors(&[CallField::Elem], &place.path),
                                            ),
                                            output.clone(),
                                        ));
                                    } else if let CallTarget::Local(target) = call.target
                                        && let Some(rows) =
                                            self.callees.get(target).and_then(|callee| {
                                                callee.result_sources.get(&outcome_slot(slot))
                                            })
                                    {
                                        self.counts.reverse_visits += place.path.len() + 1;
                                        for (source, output_tail, input_tail) in
                                            rows.select(&place.path)
                                        {
                                            self.counts.reverse_visits += 1;
                                            let actual = ResultDemand::Value(
                                                call.args[source.parameter],
                                                join_selectors(&source.input, input_tail),
                                            );
                                            work.push((
                                                actual,
                                                join_selectors(&output, output_tail),
                                            ));
                                        }
                                    } else if let CallReturns::Summary(summary) =
                                        self.call_contracts[position].returns()
                                    {
                                        for (source, output_tail, input_tail) in
                                            ResultSources::from_settled_call(summary, slot)
                                                .select(&place.path)
                                        {
                                            work.push((
                                                ResultDemand::Value(
                                                    call.args[source.parameter],
                                                    join_selectors(&source.input, input_tail),
                                                ),
                                                join_selectors(&output, output_tail),
                                            ));
                                        }
                                    }
                                    continue;
                                }
                            };
                            work.push((actual, output.clone()));
                        }
                    }
                    ResultDemand::Value(value, input) => {
                        let mut field = |selector, child| {
                            if let Some((&selected, tail)) = input.split_first() {
                                if selected == selector {
                                    work.push((
                                        ResultDemand::Value(child, tail.into()),
                                        output.clone(),
                                    ));
                                }
                            } else {
                                let output = join_selectors(&output, &[selector]);
                                work.push((ResultDemand::Value(child, Box::new([])), output));
                            }
                        };
                        match &expressions.expression(value).kind {
                            HirExpressionKind::Load(place) => {
                                work.push((
                                    ResultDemand::Place(self.place_path(*place, &input)),
                                    output,
                                ));
                            }
                            HirExpressionKind::VariantPayloadGet {
                                source,
                                variant_index,
                                field_index,
                                ..
                            } => {
                                let selector = payload(*variant_index, *field_index);
                                work.push((
                                    ResultDemand::Value(
                                        *source,
                                        join_selectors(&[selector], &input),
                                    ),
                                    output,
                                ));
                            }
                            HirExpressionKind::StructConstruct { fields, .. } => {
                                if let Some((&selector, _)) = input.split_first() {
                                    if let Some(child) = self.constructed_field(value, selector) {
                                        field(selector, child);
                                    }
                                } else {
                                    let fields = expressions.struct_fields(*fields);
                                    self.counts.selector_incidences += fields.len();
                                    for &(id, child) in fields {
                                        field(self.field_ordinals[&id], child);
                                    }
                                }
                            }
                            HirExpressionKind::VariantConstruct {
                                fields,
                                variant_index,
                                ..
                            } => {
                                for (index, member) in
                                    expressions.variant_fields(*fields).iter().enumerate()
                                {
                                    field(payload(*variant_index, index), member.value);
                                }
                            }
                            HirExpressionKind::Collection(values) => {
                                for &value in expressions.values(*values) {
                                    field(CallField::Elem, value);
                                }
                            }
                            HirExpressionKind::MapLiteral(entries) => {
                                for entry in expressions.map_entries(*entries) {
                                    field(CallField::MapKey, entry.key);
                                    field(CallField::Elem, entry.value);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        sources
    }

    fn finite_prefix(&mut self, mut ty: TypeId, path: &[CallField]) -> usize {
        self.path_ancestors.clear();
        for (index, &selector) in path.iter().enumerate() {
            if self.path_ancestors.contains(&ty) {
                return index;
            }
            let Some(child) = selector.type_child(ty, self.checker.type_environment) else {
                return index;
            };
            if index + 1 != path.len() {
                self.path_ancestors.push(ty);
            }
            ty = child;
        }
        path.len()
    }

    fn seal(mut self, transfer: Rc<SparseTransferIndex>) -> SparseFunction {
        let result_sources = self.collect_result_sources();
        self.drain_demands();
        let (_conditional, _conditional_arm_count) = self.connect_control();
        self.drain_demands();
        self.connect_local_accesses();
        let (guards, compatibility) = self.connect_compatibility();
        let mut support_indices = BTreeMap::new();
        let mut support_of = |node| {
            let root = self.supports.root(node);
            let next =
                SupportId(u32::try_from(support_indices.len()).expect("HIR support capacity"));
            *support_indices.entry(root).or_insert(next)
        };
        let envelope = self
            .envelope
            .into_iter()
            .map(|(key, node)| (key, support_of(node)))
            .collect();
        let port_supports: Vec<_> = self
            .port_nodes
            .iter()
            .copied()
            .map(&mut support_of)
            .collect();
        let mut support_positions = BTreeMap::<_, Vec<_>>::new();
        #[cfg(test)]
        let mut operations = BTreeMap::new();
        for (site, nodes) in self.sites {
            let mut supports: Vec<_> = nodes.into_iter().map(&mut support_of).collect();
            supports.sort_unstable();
            supports.dedup();
            for &support in &supports {
                support_positions
                    .entry((support, BlockId(site.0)))
                    .or_default()
                    .push(site.1);
            }
            #[cfg(test)]
            operations.insert(site, supports.into_boxed_slice());
        }
        let conditional_supports = _conditional.into_iter().map(&mut support_of);
        #[cfg(test)]
        let conditional_supports = conditional_supports.collect();
        #[cfg(not(test))]
        conditional_supports.for_each(drop);
        let result_supports = self
            .result_nodes
            .into_iter()
            .map(|(slot, node)| (slot, support_of(node)))
            .collect();
        let mut call_components = BTreeMap::<_, BTreeMap<usize, Vec<SupportId>>>::new();
        for ((block, position), callee, node) in self.call_components {
            call_components
                .entry((support_of(node), BlockId(block)))
                .or_default()
                .entry(position)
                .or_default()
                .push(callee);
        }
        for components in call_components.values_mut().flat_map(BTreeMap::values_mut) {
            components.sort_unstable();
            components.dedup();
        }
        let support_count = support_indices.len();
        let mut ports_by_support = vec![Vec::new(); support_count];
        for (port, &support) in port_supports.iter().enumerate() {
            ports_by_support[support.0 as usize].push(PortId(port as u32));
        }
        // An observed edge needs its holder in the predecessor even when two
        // independent supports only share a read of that holder.
        for (port, &support) in self.ports.iter().zip(&port_supports) {
            if let EntryPort::Edge(holder, _) = *port {
                ports_by_support[support.0 as usize].push(holder);
            }
        }
        for ports in &mut ports_by_support {
            ports.sort_unstable();
            ports.dedup();
        }
        let mut callees: Vec<_> = self
            .snapshot
            .calls
            .iter()
            .filter_map(|call| match call.target {
                CallTarget::Local(function) => Some(*function),
                _ => None,
            })
            .collect();
        callees.sort_unstable_by_key(|function| function.0);
        callees.dedup();

        let total = transfer.recurring.is_empty()
            && self.snapshot.blocks.iter().all(|block| {
                !transfer.successors[block].is_empty()
                    || matches!(
                        self.checker.module.blocks[self.checker.block_index_by_id[block]]
                            .terminator,
                        HirTerminator::Return(_) | HirTerminator::ReturnError(_)
                    )
            })
            && self.snapshot.calls.iter().all(|call| match call.target {
                CallTarget::External(_) => true,
                CallTarget::Local(callee) => {
                    self.callees.get(callee).is_some_and(|callee| callee.total)
                }
                _ => false,
            });
        SparseFunction {
            ports: self.ports,
            port_supports,
            support_count,
            callees,
            result_supports,
            ports_by_support,
            guards,
            compatibility,
            #[cfg(test)]
            operations,
            operations_by_support: support_positions
                .into_iter()
                .map(|(key, positions)| (key, positions.into_boxed_slice()))
                .collect(),
            call_components,
            #[cfg(test)]
            conditional_supports,
            #[cfg(test)]
            conditional_arm_count: _conditional_arm_count,
            counts: self.counts,
            transfer,
            envelope,
            result_sources,
            total,
        }
    }

    fn connect_control(&mut self) -> (Vec<usize>, usize) {
        let cfg = SparseCfg::new(
            self.checker,
            self.snapshot,
            |target| match target {
                CallTarget::External(_) => true,
                CallTarget::Local(callee) => {
                    self.callees.get(callee).is_some_and(|callee| callee.total)
                }
                _ => false,
            },
            true,
            &mut self.counts.reverse_visits,
        );
        if cfg.may_recur(&mut self.counts.reverse_visits) {
            // Iterations can share a trip count or skip a reconvergence.
            // Couple their state until there is a local recurrence proof.
            let mut sites = self.sites.values().flatten().copied();
            if let Some(first) = sites.next() {
                for site in sites {
                    self.counts.support_candidates += 1;
                    self.supports.couple(first, site);
                }
                return (vec![first], 0);
            }
            return (Vec::new(), 0);
        }
        let joins = cfg.postdominators(&mut self.counts.reverse_visits);
        let mut conditional = Vec::new();
        let mut arms = 0;
        let mut reuse = FxHashMap::default();
        let mut seen = vec![0; cfg.successors.len()];
        let mut joined_to_exit = None;
        let mut work = Vec::new();
        let mut affected = Vec::new();
        for (branch, successors) in cfg.successors.iter().enumerate() {
            if successors.len() < 2 && !cfg.unproved_continuation[branch] {
                continue;
            }
            let join = joins[branch];
            let block = self.snapshot.blocks[branch];
            let branch_block = &self.checker.module.blocks[self.checker.block_index_by_id[&block]];
            let term = &branch_block.terminator;
            affected.clear();
            let stamp = branch + 1;
            if join == cfg.exit {
                // No reconvergence is proved for the continuing alternatives.
                if let Some(first) = joined_to_exit {
                    affected.push(first);
                } else {
                    affected.extend(self.sites.values().flatten().copied());
                    joined_to_exit = affected.first().copied();
                }
            } else {
                if let Some(nodes) = self.sites.get(&(block.0, branch_block.statements.len())) {
                    affected.extend(nodes);
                }
                work.extend_from_slice(successors);
                while let Some(next) = work.pop() {
                    self.counts.reverse_visits += 1;
                    if next == join
                        || next == cfg.exit
                        || std::mem::replace(&mut seen[next], stamp) == stamp
                    {
                        continue;
                    }
                    let block = self.snapshot.blocks[next];
                    affected.extend(
                        self.sites
                            .range((block.0, 0)..=(block.0, usize::MAX))
                            .flat_map(|(_, nodes)| nodes.iter().copied()),
                    );
                    work.extend_from_slice(&cfg.successors[next]);
                }
            }
            let Some(&first) = affected.first() else {
                continue;
            };
            conditional.push(first);
            arms += successors.len();
            for &node in &affected[1..] {
                self.couple(first, node);
            }
            if let HirTerminator::If { condition, .. } = term {
                self.demand_value(*condition, &[], DemandKind::Value, first);
                let expression = self.checker.module.expressions.expression(*condition);
                if let HirExpressionKind::Load(place) = expression.kind {
                    let key = self.place_path(place, &[]);
                    if let Some(previous) = reuse.insert(key, first) {
                        self.couple(previous, first);
                    }
                }
            } else {
                let mut operands = Vec::new();
                collect_terminator_loaded_locals(
                    &self.checker.module.expressions,
                    term,
                    &mut |local| operands.push(local),
                );
                for local in operands {
                    self.demand_place(
                        IndexedPlace {
                            local,
                            path: Box::default(),
                        },
                        DemandKind::Value,
                        first,
                    );
                }
            }
        }
        (conditional, arms)
    }
}

fn outcome_slot(slot: Option<usize>) -> (bool, usize) {
    (slot.is_none(), slot.unwrap_or(0))
}

fn join_selectors(prefix: &[CallField], tail: &[CallField]) -> Box<[CallField]> {
    [prefix, tail].concat().into_boxed_slice()
}

struct SparseCfg {
    successors: Vec<Vec<usize>>,
    predecessors: Vec<Vec<usize>>,
    unproved_continuation: Vec<bool>,
    exit: usize,
}

impl SparseCfg {
    fn new(
        checker: &BorrowChecker<'_>,
        snapshot: &Snapshot<'_>,
        call_total: impl Fn(&CallTarget) -> bool,
        continuing_only: bool,
        visits: &mut usize,
    ) -> Self {
        let blocks = &snapshot.blocks;
        let indices: FxHashMap<_, _> = blocks.iter().enumerate().map(|(i, &id)| (id, i)).collect();
        let exit = blocks.len();
        let mut successors = vec![Vec::new(); exit + 1];
        let mut predecessors = vec![Vec::new(); exit + 1];
        let mut unproved_continuation = vec![false; exit + 1];
        for call in &snapshot.calls {
            *visits += 1;
            if !call_total(call.target)
                && matches!(
                    call.continuation,
                    CallContinuation::Statement | CallContinuation::Invoke
                )
            {
                // The HIR continuation is executable only if the call returns.
                // Keep the stop edge structural, independent of delivered summaries.
                unproved_continuation[indices[&BlockId(call.site.0)]] = true;
            }
        }
        let mut returning = Vec::new();
        for (i, &id) in blocks.iter().enumerate() {
            let block = &checker.module.blocks[checker.block_index_by_id[&id]];
            for_each_terminator_target(&block.terminator, |target| {
                successors[i].push(indices[&target])
            });
            if continuing_only {
                if matches!(
                    block.terminator,
                    HirTerminator::Return(_) | HirTerminator::ReturnError(_)
                ) {
                    returning.push(i);
                }
                for &target in &successors[i] {
                    predecessors[target].push(i);
                }
            }
        }
        if continuing_only {
            // An aborting arm has executed effects but cannot choose the output
            // of a sibling continuation. Only the support partition erases it.
            let mut reaches_return = vec![false; exit];
            while let Some(block) = returning.pop() {
                *visits += 1;
                if !std::mem::replace(&mut reaches_return[block], true) {
                    returning.extend_from_slice(&predecessors[block]);
                }
            }
            for (i, targets) in successors[..exit].iter_mut().enumerate() {
                if reaches_return[i] {
                    targets.retain(|&target| reaches_return[target]);
                }
            }
            predecessors.iter_mut().for_each(Vec::clear);
        }
        for i in 0..exit {
            if successors[i].is_empty() || unproved_continuation[i] {
                successors[i].push(exit);
            }
            for &j in &successors[i] {
                predecessors[j].push(i);
            }
        }
        let mut reaches_exit = vec![false; exit + 1];
        let mut work = vec![exit];
        while let Some(block) = work.pop() {
            *visits += 1;
            if !std::mem::replace(&mut reaches_exit[block], true) {
                work.extend_from_slice(&predecessors[block]);
            }
        }
        // A closed recurrent region is another possible stop, not proof that
        // all paths through it reach an unrelated continuation.
        for i in 0..exit {
            if !reaches_exit[i] {
                successors[i].push(exit);
                predecessors[exit].push(i);
            }
        }
        Self {
            successors,
            predecessors,
            unproved_continuation,
            exit,
        }
    }

    fn may_recur(&self, visits: &mut usize) -> bool {
        let mut inbound: Vec<_> = self.predecessors.iter().map(Vec::len).collect();
        let mut ready: Vec<_> = inbound
            .iter()
            .enumerate()
            .filter_map(|(node, &count)| (count == 0).then_some(node))
            .collect();
        let mut visited = 0;
        while let Some(node) = ready.pop() {
            *visits += 1;
            visited += 1;
            for &next in &self.successors[node] {
                *visits += 1;
                inbound[next] -= 1;
                if inbound[next] == 0 {
                    ready.push(next);
                }
            }
        }
        visited != self.successors.len()
    }

    fn postdominators(&self, visits: &mut usize) -> Vec<usize> {
        let mut visited = vec![false; self.exit + 1];
        let mut order = Vec::new();
        let mut stack = vec![(self.exit, false)];
        while let Some((block, finished)) = stack.pop() {
            *visits += 1;
            if finished {
                order.push(block);
            } else if !std::mem::replace(&mut visited[block], true) {
                stack.push((block, true));
                *visits += self.predecessors[block].len();
                stack.extend(
                    self.predecessors[block]
                        .iter()
                        .rev()
                        .map(|&predecessor| (predecessor, false)),
                );
            }
        }
        let mut ranks = vec![0; self.exit + 1];
        for (rank, &block) in order.iter().rev().enumerate() {
            ranks[block] = rank;
        }
        let mut parent = vec![usize::MAX; self.exit + 1];
        parent[self.exit] = self.exit;
        loop {
            let mut changed = false;
            for &block in order.iter().rev().skip(1) {
                *visits += 1;
                *visits += self.successors[block].len();
                let mut known = self.successors[block]
                    .iter()
                    .copied()
                    .filter(|&next| parent[next] != usize::MAX);
                let Some(mut common) = known.next() else {
                    continue;
                };
                for mut other in known {
                    while common != other {
                        *visits += 1;
                        if ranks[common] > ranks[other] {
                            common = parent[common];
                        } else {
                            other = parent[other];
                        }
                    }
                }
                changed |= parent[block] != common;
                parent[block] = common;
            }
            if !changed {
                return parent;
            }
        }
    }
}

impl SparseFunction {
    pub(super) fn build_sparse(
        checker: &BorrowChecker<'_>,
    ) -> Result<FxHashMap<FunctionId, SparseFunction>, BorrowCheckError> {
        let mut fields = FxHashMap::default();
        for layout in &checker.module.structs {
            for (position, field) in layout.fields.iter().enumerate() {
                fields.insert(field.id, CallField::Field(position as u32));
            }
        }
        let snapshots = checker
            .module
            .functions
            .iter()
            .map(|function| Snapshot::collect(checker, function))
            .collect::<Result<Vec<_>, _>>()?;
        let transfer_indices = snapshots
            .iter()
            .map(|snapshot| SparseTransferIndex::new(checker, snapshot).map(Rc::new))
            .collect::<Result<Vec<_>, _>>()?;
        let mut callers = FxHashMap::<FunctionId, Vec<usize>>::default();
        for (caller, snapshot) in snapshots.iter().enumerate() {
            for call in &snapshot.calls {
                if let CallTarget::Local(target) = call.target {
                    callers.entry(*target).or_default().push(caller);
                }
            }
        }
        // Acyclic callees supply their structural envelope before any caller
        // reads it. Only cycles need provisional rows and revisits.
        let positions: FxHashSet<_> = snapshots
            .iter()
            .map(|snapshot| snapshot.function.id)
            .collect();
        let mut unscheduled_dependencies = vec![0; snapshots.len()];
        for (&callee, edges) in &callers {
            if positions.contains(&callee) {
                for &caller in edges {
                    unscheduled_dependencies[caller] += 1;
                }
            }
        }
        let mut ready: VecDeque<_> = unscheduled_dependencies
            .iter()
            .enumerate()
            .filter_map(|(position, &count)| (count == 0).then_some(position))
            .collect();
        let mut work = VecDeque::with_capacity(snapshots.len());
        while let Some(position) = ready.pop_front() {
            work.push_back(position);
            if let Some(caller_edges) = callers.get(&snapshots[position].function.id) {
                for &caller in caller_edges {
                    unscheduled_dependencies[caller] -= 1;
                    if unscheduled_dependencies[caller] == 0 {
                        ready.push_back(caller);
                    }
                }
            }
        }
        work.extend(
            unscheduled_dependencies
                .iter()
                .enumerate()
                .filter_map(|(position, &count)| (count != 0).then_some(position)),
        );
        let mut pending = vec![true; snapshots.len()];
        let mut indices = FxHashMap::<FunctionId, SparseFunction>::default();
        while let Some(position) = work.pop_front() {
            pending[position] = false;
            let snapshot = &snapshots[position];
            let mut builder = DemandBuilder::new(checker, snapshot, &fields, &indices);
            builder.collect_roots();
            builder.collect_call_roots()?;
            let mut index = builder.seal(Rc::clone(&transfer_indices[position]));
            index.counts.record(index.ports.len());
            let previous = indices.get(&snapshot.function.id);
            if let Some(old) = previous {
                // Work evidence includes the previous visits, not just the
                // final reduced graph.
                index.counts += old.counts;
            } else {
                index.counts += snapshot.counts;
            }
            if previous.is_none_or(|old| {
                old.envelope != index.envelope
                    || old.result_sources != index.result_sources
                    || old.total != index.total
            }) && let Some(callers) = callers.get(&snapshot.function.id)
            {
                for &caller in callers {
                    if !std::mem::replace(&mut pending[caller], true) {
                        work.push_back(caller);
                    }
                }
            }
            indices.insert(snapshot.function.id, index);
        }
        Ok(indices)
    }
}

#[cfg(test)]
fn build_sparse_index_for_test(
    checker: &mut BorrowChecker<'_>,
) -> Result<FxHashMap<FunctionId, SparseFunction>, BorrowCheckError> {
    checker.public_call_summaries =
        BorrowChecker::seed_public_call_summaries(checker.module, &checker.diagnostics)?;
    SparseFunction::build_sparse(checker)
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

#[test]
fn result_without_registered_nominal_identity_keeps_its_opaque_footprint() {
    use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
    use crate::compiler_frontend::tests::hir_fixture_support::lower_hir;
    use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;

    let (ast, mut paths, mut strings) = parse_single_file_ast(
        "Link = |items {I32}|\nwrap |items {I32}| -> Link:\n    return Link(items)\n;\n",
    );
    let packages = default_external_package_registry(&mut strings);
    let hir = lower_hir(ast, &mut strings, &mut paths, &packages);
    let mut checker = BorrowChecker::new(&hir, &hir.type_environment, &packages, &paths, &strings);
    let wrap = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .resolve_function_name(function.id, &paths, &strings)
                == Some("wrap")
        })
        .unwrap();
    let result = wrap.return_contract.ordinary().unwrap().success()[0];
    assert!(checker.project_type(result).is_none());
    checker.build_public_call_summaries().unwrap();
    let facts = checker.public_call_summaries[&wrap.id]
        .return_alias
        .facts()
        .unwrap();
    let typed::Lane::Returning(product) = &facts.success else {
        panic!(
            "the unresolved result must not erase its continuation: {:?}",
            facts.success
        );
    };
    assert!(product.components().iter().flat_map(|component| &component.alternatives).any(|post| {
        matches!(&post.coverage, typed::Coverage::Unresolved(typed::Footprint::Locations(locations))
            if locations.as_slice() == [typed::OutputPort::Result(0)])
    }));
    assert!(facts.ports.iter().any(|port| {
        matches!(port, EntryPort::Value { parameter: 0, path } if path.is_empty())
    }));
}
