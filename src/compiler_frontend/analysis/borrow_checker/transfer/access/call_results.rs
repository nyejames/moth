//! Instantiate validated return shapes, or conservatively connect compatible opaque paths.

use super::super::layer::{
    CapturedValue, LayerValueContext, capture_copy, resolve_place, stored_allocation_values,
    stored_values,
};
use super::{CallArgumentTransfer, CallTransferContext};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AddressSet, AllocId, AllocationRetirement, BindingAlt, BindingTarget, BorrowState, CellId,
    Coverage, DependencySet, FamilyMember, FieldKey, OrderKey, Origin, PlaceResolution,
    PublicationCarriers, ReceiverDependency, Replacement, ResolvedPlace, RootSet, SmallSet,
    StoredValue, TypedStore, ValueAlt, ValueAlternatives, ValueKind as PayloadKind, WrapperAge,
    WrapperId,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::call_semantics::CallReturns;
use crate::compiler_frontend::analysis::borrow_checker::transfer::{
    BorrowTransferContext, InvokeLane, InvokeLayerOp,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::{ExternalReturnAlias, ExternalReturnSlot};
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;
use crate::compiler_frontend::hir::statements::HirLocalDestination;
use crate::compiler_frontend::public_call_summary::{
    CallAllocationSource, CallField, CallFieldHir, CallLaneSummary, CallOutputPath, CallPathSource,
    CallSharingCondition, FunctionReturnAliasSummary,
};
use crate::compiler_frontend::public_call_summary::{allocation_error, typed};
use rustc_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;

/// Canonical predecessor observations. A wrapper, its terminal cell and its
/// value are independent observations, even when they share one local spelling.
#[allow(dead_code)]
#[derive(Clone)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct EntryCapture {
    pub wrappers: SmallSet<WrapperId>,
    pub wrapper_coverage: Option<Coverage>,
    pub role: Option<typed::Role>,
    pub writable: Option<bool>,
    pub address: Option<AddressSet>,
    pub value: CapturedValue,
    pub allocation_coverage: Option<Coverage>,
    pub derivation: Option<WrapperId>,
    pub origin: Option<Origin>,
    pub roots: Option<RootSet>,
    pub ty: Option<TypeId>,
}

/// Resolve declared selectors from the actual's captured local, not from a
/// receiving name or an ordinal in the caller's local layout.
#[allow(dead_code)]
pub(in crate::compiler_frontend::analysis::borrow_checker) fn capture_local_value_ports(
    state: &BorrowState,
    mutable: &[bool],
    ports: &[typed::EntryPort],
    actuals: &[(usize, TypeId)],
    field: impl Fn(TypeId, CallField) -> Option<(FieldKey, TypeId)>,
    allocation: impl Fn(TypeId) -> bool,
) -> Result<Vec<EntryCapture>, InfrastructureFailure> {
    ports
        .iter()
        .map(|port| {
            let typed::EntryPort::Value { parameter, path } = port else {
                return Err(allocation_error(
                    "value port capture requires a declared value path",
                ));
            };
            let &(local, mut ty) = required(actuals.get(*parameter))?;
            let mut selectors = Vec::with_capacity(path.len());
            for &selector in path {
                let (key, child) = required(field(ty, selector))?;
                selectors.push(key);
                ty = child;
            }
            let mut place = state.resolve_place_path(local, mutable, selectors);
            let heap = allocation(ty);
            state.load_place_content(&mut place, mutable, !heap);
            let writable = place.permission.wrapper_readonly_source.is_none()
                && !place.permission.holder_readonly;
            let origin = place.captured_origin();
            let content = required(place.content)?;
            let coverage = content.holder_coverage();
            let value = if heap {
                capture_graph_values(content)
            } else {
                CapturedValue {
                    alternatives: content.alternatives,
                    ..Default::default()
                }
            };
            Ok(EntryCapture {
                ty: Some(ty),
                address: Some(place.address),
                value,
                allocation_coverage: heap.then_some(coverage),
                origin,
                writable: Some(writable),
                ..Default::default()
            })
        })
        .collect()
}

impl Default for EntryCapture {
    fn default() -> Self {
        Self {
            wrappers: SmallSet::Empty,
            wrapper_coverage: None,
            role: None,
            writable: None,
            address: None,
            value: CapturedValue::default(),
            allocation_coverage: None,
            derivation: None,
            origin: None,
            roots: None,
            ty: None,
        }
    }
}

#[allow(dead_code)]
pub(in crate::compiler_frontend::analysis::borrow_checker) trait MatchInput {
    fn state(&self) -> &BorrowState;
    fn entry(&self, port: typed::PortId) -> Option<&EntryCapture>;
    fn retained(&self, object: &typed::ObjectRef) -> Option<&EntryCapture>;
    fn field(&self, holder: TypeId, selector: CallField) -> Option<FieldKey>;
    fn entry_count(&self) -> usize;
    fn opaque_source_ports(&self) -> Option<&[typed::PortId]> {
        None
    }

    fn uncertain_effects(&self, _footprint: &typed::Footprint) -> Option<&CapturedUncertainty> {
        None
    }
    fn reach_layout(&self) -> Option<&Rc<ReachLayout>> {
        None
    }
    fn field_value_type(&self, _holder: TypeId, _selector: CallField) -> Option<TypeId> {
        None
    }
    fn result_type(&self, _slot: usize) -> Option<TypeId> {
        None
    }
    fn allocation_type(&self, _value: AllocId) -> Option<TypeId> {
        None
    }
}

#[cfg(test)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct ExplicitMatchInput<'a> {
    pub state: &'a BorrowState,
    pub entries: &'a [EntryCapture],
    pub retained: &'a [(typed::ObjectRef, EntryCapture)],
    pub uncertainty: &'a [(typed::Footprint, CapturedUncertainty)],
}

#[cfg(test)]
impl MatchInput for ExplicitMatchInput<'_> {
    fn state(&self) -> &BorrowState {
        self.state
    }

    fn entry(&self, port: typed::PortId) -> Option<&EntryCapture> {
        self.entries.get(port.0 as usize)
    }

    fn entry_count(&self) -> usize {
        self.entries.len()
    }

    fn retained(&self, object: &typed::ObjectRef) -> Option<&EntryCapture> {
        self.retained
            .iter()
            .find_map(|(key, value)| (key == object).then_some(value))
    }

    fn field(&self, _: TypeId, selector: CallField) -> Option<FieldKey> {
        Some(match selector {
            CallField::Field(ordinal) => FieldKey::Field(ordinal),
            CallField::Payload { variant, field } => FieldKey::Payload { variant, field },
            CallField::Elem => FieldKey::Elem,
            CallField::MapKey => FieldKey::MapKey,
        })
    }

    fn uncertain_effects(&self, footprint: &typed::Footprint) -> Option<&CapturedUncertainty> {
        self.uncertainty
            .iter()
            .find_map(|(key, value)| (key == footprint).then_some(value))
    }
}

#[allow(dead_code)]
#[derive(Clone)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct CapturedBinding {
    pub target: WrapperId,
    pub update: BindingEdit,
    pub replace: Replacement,
    pub roots: Option<RootSet>,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(in crate::compiler_frontend::analysis::borrow_checker) enum BindingEdit {
    Slot,
    Link {
        targets: SmallSet<WrapperId>,
        writable: bool,
        coverage: Coverage,
        origin: Option<Origin>,
        routes: Option<AddressSet>,
    },
    Projected {
        addresses: AddressSet,
        writable: bool,
        origin: Option<Origin>,
    },
}

#[allow(dead_code)]
#[derive(Clone)]
pub(in crate::compiler_frontend::analysis::borrow_checker) enum CapturedEdit {
    Cell(TypedStore),
    Family {
        receivers: Vec<AllocId>,
        coverage: Coverage,
        key: FieldKey,
        replace: Replacement,
        members: Vec<FamilyMember>,
    },
}

#[allow(dead_code)]
#[derive(Clone, Default)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct CapturedUncertainty {
    pub bindings: Vec<CapturedBinding>,
    pub edits: Vec<CapturedEdit>,
    pub results: Vec<(usize, CapturedValue)>,
    pub reach: Option<CapturedReach>,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct ReachType {
    pub identity: crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity,
    pub allocation: bool,
    pub children: Vec<(FieldKey, TypeId)>,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct ReachLayout {
    pub types: FxHashMap<TypeId, ReachType>,
    pub local_mutable: Vec<bool>,
    pub error: bool,
    pub scope: u32,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct CapturedReach {
    bound: typed::ReachBound,
    layout: Rc<ReachLayout>,
    roots: Vec<EntryCapture>,
    sources: Vec<(TypeId, CapturedValue)>,
    locations: Vec<(TypeId, AddressSet)>,
    results: Vec<(usize, TypeId)>,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct CapturedResult {
    pub slot: usize,
    pub value: CapturedValue,
    pub roots: Option<RootSet>,
    pub derivation: Option<CapturedDerivation>,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(in crate::compiler_frontend::analysis::borrow_checker) enum CapturedDerivation {
    Wrapper(WrapperId),
    Roots(RootSet),
}

#[allow(dead_code)]
#[derive(Default)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct CapturedPost {
    pub bindings: Vec<CapturedBinding>,
    pub edits: Vec<CapturedEdit>,
    pub results: Vec<CapturedResult>,
    pub uncertainty: Option<CapturedUncertainty>,
}

/// Keep independent supports separate; callers need not enumerate their product.
#[allow(dead_code)]
pub(in crate::compiler_frontend::analysis::borrow_checker) type MatchedLane = Vec<MatchedStratum>;

#[allow(dead_code)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct MatchedStratum {
    pub guard: Vec<typed::Literal>,
    pub restrictions: FxHashMap<typed::PortId, EntryCapture>,
    pub components: Vec<MatchedComponent>,
}

impl EntryCapture {
    pub fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        self.wrappers = SmallSet::from_iter(
            self.wrappers
                .as_slice()
                .iter()
                .copied()
                .map(|wrapper| retirement.remap_wrapper(wrapper)),
        );
        if let Some(address) = &mut self.address {
            address.remap_allocations(state, retirement);
        }
        self.value.remap(state, retirement);
        self.derivation = self
            .derivation
            .filter(|&wrapper| retirement.remap_wrapper(wrapper) == wrapper);
        remap_origin(&mut self.origin, retirement);
    }
}

#[allow(dead_code)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct MatchedComponent {
    pub support: typed::SupportId,
    pub alternatives: Vec<CapturedPost>,
}

#[allow(dead_code)]
pub(in crate::compiler_frontend::analysis::borrow_checker) fn match_typed_lane(
    input: &impl MatchInput,
    lane: &typed::Lane,
) -> Result<MatchedLane, InfrastructureFailure> {
    let mut strata = Vec::new();
    lane.visit_compatible(
        |predicate| observe_typed_guard(input, predicate),
        |path, product| {
            if let Some(presentation) = guard_presentation(input, path)? {
                let mut components = capture_product(&presentation, product)?;
                weaken_product_collisions(input.state(), &mut components);
                strata.push(MatchedStratum {
                    guard: path.to_vec(),
                    restrictions: presentation.entries,
                    components,
                });
            }
            Ok(())
        },
    )?;
    Ok(strata)
}

fn capture_product(
    input: &impl MatchInput,
    product: &typed::Product,
) -> Result<Vec<MatchedComponent>, InfrastructureFailure> {
    let mut components = Vec::with_capacity(product.components().len());
    let mut owners = FxHashMap::default();
    for component in product.components() {
        let mut alternatives = Vec::with_capacity(component.alternatives.len());
        for post in &component.alternatives {
            let mut locations = FxHashSet::default();
            alternatives.push(capture_typed_post(input, post, |location, prefix| {
                if location.is_occurrence(input.state())
                    && ((prefix && !locations.insert(location))
                        || owners
                            .insert(location, component.support)
                            .is_some_and(|old| old != component.support))
                {
                    return Err(allocation_error(
                        "typed call updates collide after actual substitution",
                    ));
                }
                Ok(())
            })?);
        }
        components.push(MatchedComponent {
            support: component.support,
            alternatives,
        });
    }
    Ok(components)
}

fn weaken_product_collisions(state: &BorrowState, components: &mut [MatchedComponent]) {
    let mut writes = Vec::new();
    for (index, component) in components.iter_mut().enumerate() {
        for post in &mut component.alternatives {
            post.weaken_collisions(state);
            writes.extend(post.write_footprints(true).map(|write| (index, write)));
        }
    }
    for index in overlapping_owners(state, &writes, |left, right| left != right) {
        for post in &mut components[index].alternatives {
            post.weaken_writes();
        }
    }
}

fn overlapping_owners(
    state: &BorrowState,
    writes: &[(usize, super::super::super::facts::AccessFootprint)],
    compare: impl Fn(usize, usize) -> bool,
) -> FxHashSet<usize> {
    use super::super::conflicts::{OperandAccessIndex, Overlap, overlap};
    let mut overlapping = FxHashSet::default();
    if writes.len() < 2 {
        return overlapping;
    }
    let mut index = OperandAccessIndex::new(
        writes.len(),
        writes
            .iter()
            .enumerate()
            .map(|(position, (_, access))| (position, access)),
    );
    for (position, (owner, access)) in writes.iter().enumerate() {
        index.any_later(state, position, access, |later| {
            if compare(*owner, writes[later].0)
                && overlap(state, access, &writes[later].1) != Overlap::No
            {
                overlapping.insert(*owner);
                overlapping.insert(writes[later].0);
            }
            false
        });
    }
    overlapping
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Candidate {
    Wrapper(WrapperId),
    Cell(CellId),
    Allocation(AllocId),
}

struct GuardPresentation<'a, I> {
    input: &'a I,
    entries: FxHashMap<typed::PortId, EntryCapture>,
}

impl<I: MatchInput> MatchInput for GuardPresentation<'_, I> {
    fn state(&self) -> &BorrowState {
        self.input.state()
    }
    fn entry(&self, port: typed::PortId) -> Option<&EntryCapture> {
        self.entries.get(&port).or_else(|| self.input.entry(port))
    }
    fn entry_count(&self) -> usize {
        self.input.entry_count()
    }
    fn opaque_source_ports(&self) -> Option<&[typed::PortId]> {
        self.input.opaque_source_ports()
    }
    fn retained(&self, object: &typed::ObjectRef) -> Option<&EntryCapture> {
        self.input.retained(object)
    }
    fn field(&self, holder: TypeId, selector: CallField) -> Option<FieldKey> {
        self.input.field(holder, selector)
    }
    fn uncertain_effects(&self, footprint: &typed::Footprint) -> Option<&CapturedUncertainty> {
        self.input.uncertain_effects(footprint)
    }
    fn reach_layout(&self) -> Option<&Rc<ReachLayout>> {
        self.input.reach_layout()
    }
    fn field_value_type(&self, holder: TypeId, selector: CallField) -> Option<TypeId> {
        self.input.field_value_type(holder, selector)
    }
    fn result_type(&self, slot: usize) -> Option<TypeId> {
        self.input.result_type(slot)
    }
    fn allocation_type(&self, value: AllocId) -> Option<TypeId> {
        self.input.allocation_type(value)
    }
}

fn observed_candidates(entry: &EntryCapture, kind: typed::IdentityKind) -> (Vec<Candidate>, bool) {
    use typed::IdentityKind;
    match kind {
        IdentityKind::Wrapper => (
            entry
                .wrappers
                .as_slice()
                .iter()
                .copied()
                .map(Candidate::Wrapper)
                .collect(),
            entry.wrapper_coverage == Some(Coverage::Complete),
        ),
        IdentityKind::Cell => entry
            .address
            .as_ref()
            .map_or((Vec::new(), false), |address| {
                (
                    address
                        .cells()
                        .iter()
                        .copied()
                        .map(Candidate::Cell)
                        .collect(),
                    address.coverage == Coverage::Complete,
                )
            }),
        IdentityKind::Allocation => (
            entry
                .value
                .values
                .iter()
                .map(|value| Candidate::Allocation(value.id))
                .collect(),
            entry.allocation_coverage == Some(Coverage::Complete),
        ),
    }
}

fn same_candidate(state: &BorrowState, left: Candidate, right: Candidate) -> Option<bool> {
    match (left, right) {
        (Candidate::Wrapper(a), Candidate::Wrapper(b))
        | (Candidate::Cell(CellId::Slot(a)), Candidate::Cell(CellId::Slot(b))) => {
            if a != b {
                Some(false)
            } else {
                (a.age == WrapperAge::Current).then_some(true)
            }
        }
        (Candidate::Cell(CellId::Projected(a, x)), Candidate::Cell(CellId::Projected(b, y))) => {
            if x != y {
                Some(false)
            } else {
                let identity =
                    same_candidate(state, Candidate::Allocation(a), Candidate::Allocation(b));
                if matches!(x, FieldKey::Elem | FieldKey::MapKey) && identity == Some(true) {
                    None
                } else {
                    identity
                }
            }
        }
        (Candidate::Allocation(a), Candidate::Allocation(b)) => {
            if a == b && state.alloc_singular(a) {
                Some(true)
            } else if !state.alloc_paths_may_share(&[a], &[], &[b], &[]) {
                Some(false)
            } else {
                None
            }
        }
        _ => Some(false),
    }
}

type IdentityPort = (typed::IdentityKind, typed::PortId);

fn guard_presentation<'a, I: MatchInput>(
    input: &'a I,
    path: &[typed::Literal],
) -> Result<Option<GuardPresentation<'a, I>>, InfrastructureFailure> {
    use typed::Predicate;
    let mut indices = FxHashMap::default();
    let mut domains = Vec::new();
    let mut equalities = Vec::new();
    let mut unequal = FxHashSet::default();
    for &(predicate, truth) in path {
        let Predicate::Equal(kind, left, right) = predicate else {
            continue;
        };
        let a = (kind, left);
        let b = (kind, right);
        for key in [a, b] {
            if let std::collections::hash_map::Entry::Vacant(index) = indices.entry(key) {
                let (values, complete) = observed_candidates(required(input.entry(key.1))?, key.0);
                let mut options: Vec<_> = values.into_iter().map(Some).collect();
                if !complete {
                    options.push(None);
                }
                index.insert(domains.len());
                domains.push((key, options));
            }
        }
        if truth {
            equalities.push((a, b));
        } else {
            unequal.insert((a.min(b), a.max(b)));
        }
    }
    fn leader(parent: &mut [usize], mut index: usize) -> usize {
        while parent[index] != index {
            parent[index] = parent[parent[index]];
            index = parent[index];
        }
        index
    }
    let mut parents: Vec<_> = (0..domains.len()).collect();
    let mut adjacent = vec![Vec::new(); domains.len()];
    for (a, b) in equalities {
        let (a, b) = (indices[&a], indices[&b]);
        let x = leader(&mut parents, a);
        let y = leader(&mut parents, b);
        parents[y] = x;
        adjacent[a].push(b);
        adjacent[b].push(a);
    }
    for &(a, b) in &unequal {
        let (a, b) = (indices[&a], indices[&b]);
        adjacent[a].push(b);
        adjacent[b].push(a);
    }
    let classes: Vec<_> = (0..domains.len())
        .map(|i| leader(&mut parents, i))
        .collect();
    if unequal
        .iter()
        .any(|&(a, b)| classes[indices[&a]] == classes[indices[&b]])
    {
        return Ok(None);
    }
    let mut groups = FxHashMap::<usize, Vec<IdentityPort>>::default();
    for (at, (key, _)) in domains.iter().enumerate() {
        groups.entry(classes[at]).or_default().push(*key);
    }
    // Ask whether a candidate extends to a feasible presentation, rather than
    // materialising the product of choices from independent identity classes.
    for group in groups.values().filter(|group| group.len() > 1) {
        let witnesses: Vec<_> = group
            .iter()
            .flat_map(|key| domains[indices[key]].1.iter().copied().flatten())
            .collect();
        for key in group {
            let values = &mut domains[indices[key]].1;
            if values.contains(&None) {
                for &witness in &witnesses {
                    if !values.contains(&Some(witness)) {
                        values.push(Some(witness));
                    }
                }
            }
        }
    }
    let compatible = |a: IdentityPort, left, b: IdentityPort, right| {
        let equal = classes[indices[&a]] == classes[indices[&b]];
        let distinct = unequal.contains(&(a.min(b), a.max(b)));
        match (left, right) {
            (Some(left), Some(right)) => {
                let same = same_candidate(input.state(), left, right);
                !(equal && same == Some(false) || distinct && same == Some(true))
            }
            (Some(candidate), None) | (None, Some(candidate)) => {
                !equal || same_candidate(input.state(), candidate, candidate) != Some(true)
            }
            (None, None) => true,
        }
    };
    fn feasible(
        domains: &[(IdentityPort, Vec<Option<Candidate>>)],
        remaining: &[usize],
        chosen: &mut Vec<(IdentityPort, Option<Candidate>)>,
        compatible: &impl Fn(IdentityPort, Option<Candidate>, IdentityPort, Option<Candidate>) -> bool,
    ) -> bool {
        let Some((&index, rest)) = remaining.split_first() else {
            return true;
        };
        let (port, values) = &domains[index];
        values.iter().copied().any(|candidate| {
            if !chosen
                .iter()
                .all(|&(peer, value)| compatible(*port, candidate, peer, value))
            {
                return false;
            }
            chosen.push((*port, candidate));
            let found = feasible(domains, rest, chosen, compatible);
            chosen.pop();
            found
        })
    }
    let mut entries = FxHashMap::default();
    let mut included = vec![false; domains.len()];
    let mut chosen = Vec::new();
    let mut peers = Vec::new();
    for start in 0..domains.len() {
        if std::mem::replace(&mut included[start], true) {
            continue;
        }
        let mut component = vec![start];
        let mut next = 0;
        while let Some(&index) = component.get(next) {
            for &peer in &adjacent[index] {
                if !std::mem::replace(&mut included[peer], true) {
                    component.push(peer);
                }
            }
            next += 1;
        }
        if !feasible(&domains, &component, &mut chosen, &compatible) {
            return Ok(None);
        }
        for &index in &component {
            let ((kind, port), candidates) = &domains[index];
            peers.clear();
            peers.extend(component.iter().copied().filter(|&peer| peer != index));
            let allowed: Vec<_> = candidates
                .iter()
                .copied()
                .filter(|&candidate| {
                    chosen.push(((*kind, *port), candidate));
                    let found = feasible(&domains, &peers, &mut chosen, &compatible);
                    chosen.pop();
                    found
                })
                .collect();
            let complete = !allowed.contains(&None);
            let entry = entries
                .entry(*port)
                .or_insert_with(|| input.entry(*port).unwrap().clone());
            match kind {
                typed::IdentityKind::Wrapper => {
                    entry.wrappers =
                        SmallSet::from_iter(allowed.iter().filter_map(|value| match value {
                            Some(Candidate::Wrapper(wrapper)) => Some(*wrapper),
                            _ => None,
                        }));
                    entry.wrapper_coverage = Some(if complete {
                        Coverage::Complete
                    } else {
                        Coverage::Partial
                    });
                }
                typed::IdentityKind::Cell => {
                    let address = entry
                        .address
                        .get_or_insert_with(|| AddressSet::from_cells([], Coverage::Partial));
                    address.restrict_cells(
                        allowed.iter().filter_map(|value| match value {
                            Some(Candidate::Cell(cell)) => Some(*cell),
                            _ => None,
                        }),
                        complete,
                        input.state(),
                    );
                }
                typed::IdentityKind::Allocation => {
                    let ids: FxHashSet<_> = allowed
                        .iter()
                        .filter_map(|value| match value {
                            Some(Candidate::Allocation(id)) => Some(*id),
                            _ => None,
                        })
                        .collect();
                    let permitted = entry.writable == Some(true);
                    entry.value.values.retain(|value| ids.contains(&value.id));
                    entry.value.views.retain(|&view| {
                        ids.iter().any(|&id| {
                            input
                                .state()
                                .alloc_paths_may_share(&[id], &[], &[view], &[])
                        })
                    });
                    entry
                        .value
                        .alternatives
                        .retain(|alternative| match alternative.kind {
                            PayloadKind::Allocation(value) => ids.contains(&value.id),
                            PayloadKind::Unknown => !complete,
                            _ => false,
                        });
                    for id in ids {
                        if !entry.value.values.iter().any(|value| value.id == id) {
                            entry.value.values.push(StoredValue {
                                id,
                                writable: permitted,
                            });
                        }
                    }
                    entry.allocation_coverage = Some(if complete {
                        Coverage::Complete
                    } else {
                        Coverage::Partial
                    });
                    if !complete
                        && !entry
                            .value
                            .alternatives
                            .iter()
                            .any(|alt| alt.kind == PayloadKind::Unknown)
                    {
                        entry
                            .value
                            .alternatives
                            .extend(entry.value.values.iter().copied().map(allocation_alt));
                        entry
                            .value
                            .alternatives
                            .push(ValueAlt::plain(PayloadKind::Unknown));
                        canonicalise_capture(&mut entry.value);
                    }
                }
            }
        }
    }
    // Equal case-split operands can have different representatives in the
    // caller's allocation quotient. Retain both representatives, without
    // manufacturing a singleton or deriving permission from the other route.
    for group in groups.values().filter(|group| group.len() > 1) {
        if group[0].0 != typed::IdentityKind::Allocation {
            continue;
        }
        let ids: FxHashSet<_> = group
            .iter()
            .flat_map(|&(_, port)| entries[&port].value.values.iter().map(|value| value.id))
            .collect();
        if ids.len() < 2 {
            continue;
        }
        for &(_, port) in group {
            let entry = entries.get_mut(&port).unwrap();
            let carried = entry.value.values.len();
            for &id in &ids {
                if !entry.value.values.iter().any(|value| value.id == id) {
                    entry.value.values.push(StoredValue {
                        id,
                        writable: entry.writable == Some(true),
                    });
                }
            }
            if entry.value.values.len() != carried {
                entry.allocation_coverage = Some(Coverage::Partial);
                entry
                    .value
                    .alternatives
                    .extend(entry.value.values.iter().copied().map(allocation_alt));
                canonicalise_capture(&mut entry.value);
            }
        }
    }
    for &(predicate, truth) in path {
        let port = match predicate {
            Predicate::Role(port, _)
            | Predicate::Writable(port)
            | Predicate::DerivedFrom(port, _) => port,
            _ => continue,
        };
        let holder = match predicate {
            Predicate::DerivedFrom(_, holder) => entries
                .get(&holder)
                .or_else(|| input.entry(holder))
                .and_then(|entry| match entry.wrappers.as_slice() {
                    [wrapper]
                        if entry.wrapper_coverage == Some(Coverage::Complete)
                            && wrapper.age == WrapperAge::Current =>
                    {
                        Some(*wrapper)
                    }
                    _ => None,
                }),
            _ => None,
        };
        let entry = entries
            .entry(port)
            .or_insert_with(|| input.entry(port).unwrap().clone());
        match predicate {
            Predicate::Role(_, role) if truth => entry.role = Some(role),
            Predicate::Role(..) => {
                let mut possible = [
                    typed::Role::Slot,
                    typed::Role::WholeAlias,
                    typed::Role::ProjectedAlias,
                ]
                .into_iter()
                .filter(|&role| !path.contains(&(Predicate::Role(port, role), false)));
                if let Some(role) = possible.next()
                    && possible.next().is_none()
                {
                    entry.role = Some(role);
                }
            }
            Predicate::Writable(_) => {
                entry.writable = Some(truth);
                if !truth {
                    entry.value.restrict_permission(true);
                } else if entry.value.values.iter().any(|value| value.writable) {
                    entry.value.values.retain(|value| value.writable);
                    entry.value.alternatives.retain(|alt| {
                        !matches!(alt.kind, PayloadKind::Allocation(value) if !value.writable)
                    });
                }
            }
            Predicate::DerivedFrom(..) => {
                if let Some(wrapper) = holder {
                    if truth {
                        entry.derivation = Some(wrapper);
                    } else if entry.derivation == Some(wrapper) {
                        entry.derivation = None;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(Some(GuardPresentation { input, entries }))
}

fn required<T>(capture: Option<T>) -> Result<T, InfrastructureFailure> {
    capture.ok_or_else(|| allocation_error("typed call capture is missing a required observation"))
}

#[allow(dead_code)]
pub(in crate::compiler_frontend::analysis::borrow_checker) fn observe_typed_guard(
    input: &impl MatchInput,
    predicate: typed::Predicate,
) -> Result<Option<bool>, InfrastructureFailure> {
    use typed::{IdentityKind, Predicate};
    let entry = |port| required(input.entry(port));
    let meaning = match predicate {
        Predicate::Role(port, role) => entry(port)?.role.map(|actual| actual == role),
        Predicate::Writable(port) => entry(port)?.writable,
        Predicate::DerivedFrom(value, holder) => {
            let value = entry(value)?;
            let holder = entry(holder)?;
            match holder.wrappers.as_slice() {
                [wrapper]
                    if holder.wrapper_coverage == Some(Coverage::Complete)
                        && wrapper.age == WrapperAge::Current =>
                {
                    value.derivation.map(|source| source == *wrapper)
                }
                _ => None,
            }
        }
        Predicate::Equal(kind, left, right) => {
            let (left, right) = (entry(left)?, entry(right)?);
            if left
                .ty
                .zip(right.ty)
                .is_some_and(|(left, right)| left != right)
            {
                return Ok(Some(false));
            }
            match kind {
                IdentityKind::Wrapper => {
                    if left.wrapper_coverage != Some(Coverage::Complete)
                        || right.wrapper_coverage != Some(Coverage::Complete)
                    {
                        return Ok(None);
                    }
                    match (left.wrappers.as_slice(), right.wrappers.as_slice()) {
                        ([a], [b])
                            if a.age == WrapperAge::Current && b.age == WrapperAge::Current =>
                        {
                            Some(a == b)
                        }
                        (a, b) if !a.iter().any(|id| b.contains(id)) => Some(false),
                        _ => None,
                    }
                }
                IdentityKind::Cell => match (
                    left.address.as_ref().and_then(AddressSet::definite),
                    right.address.as_ref().and_then(AddressSet::definite),
                ) {
                    (Some(a), Some(b)) if a == b => Some(true),
                    (Some(CellId::Slot(_)), Some(_)) | (Some(_), Some(CellId::Slot(_))) => {
                        Some(false)
                    }
                    (Some(CellId::Projected(a, ka)), Some(CellId::Projected(b, kb))) => (ka != kb
                        || !input.state().alloc_paths_may_share(&[a], &[], &[b], &[]))
                    .then_some(false),
                    _ => None,
                },
                IdentityKind::Allocation => {
                    if left.allocation_coverage != Some(Coverage::Complete)
                        || right.allocation_coverage != Some(Coverage::Complete)
                    {
                        None
                    } else {
                        let a: Vec<_> = left.value.values.iter().map(|value| value.id).collect();
                        let b: Vec<_> = right.value.values.iter().map(|value| value.id).collect();
                        if matches!((&a[..], &b[..]), ([a], [b]) if a == b && input.state().alloc_singular(*a))
                        {
                            Some(true)
                        } else if !a.is_empty()
                            && !b.is_empty()
                            && !input.state().alloc_paths_may_share(
                                &a,
                                &left.value.views,
                                &b,
                                &right.value.views,
                            )
                        {
                            Some(false)
                        } else {
                            None
                        }
                    }
                }
            }
        }
    };
    Ok(meaning)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ExactLocation {
    Binding(WrapperId),
    Cell(CellId),
    Result(usize),
}

impl ExactLocation {
    fn is_occurrence(self, state: &BorrowState) -> bool {
        match self {
            Self::Binding(wrapper) | Self::Cell(CellId::Slot(wrapper)) => {
                wrapper.age == WrapperAge::Current
            }
            Self::Cell(CellId::Projected(holder, selector)) => {
                state.projected_cell_is_exact(holder, selector)
            }
            Self::Result(_) => true,
        }
    }
}

fn capture_typed_post(
    input: &impl MatchInput,
    post: &typed::Post,
    mut observe_write: impl FnMut(ExactLocation, bool) -> Result<(), InfrastructureFailure>,
) -> Result<CapturedPost, InfrastructureFailure> {
    use typed::{BindingPost, CellPost, FamilyPost};
    let mut capture = CapturedPost::default();
    for (target, edit) in &post.bindings {
        let target = resolve_object(input, target)?;
        let mut roots = None;
        let update = match edit {
            BindingPost::OwnSlot => BindingEdit::Slot,
            BindingPost::Link(source, route) => {
                let source = resolve_object(input, source)?;
                roots.clone_from(&source.roots);
                let dependencies = capture_dependencies(input, &route.dependencies)?;
                let mut routes = source.address.clone();
                if let Some(addresses) = &mut routes {
                    addresses.add_retained_dependencies(&dependencies);
                } else if !dependencies.is_empty() {
                    return Err(allocation_error(
                        "retained link dependency has no captured route",
                    ));
                }
                BindingEdit::Link {
                    targets: source.wrappers.clone(),
                    coverage: source.wrapper_coverage.unwrap_or(Coverage::Partial),
                    origin: capture_origin(input, route.access.derived_from.as_ref())?,
                    writable: source.writable == Some(true)
                        && route.access.restriction != Some(typed::Permission::ReadOnly),
                    routes,
                }
            }
            BindingPost::Projected(cell, route) => {
                let (mut addresses, writable) = resolve_cell(input, cell)?;
                let projected: Vec<_> = addresses
                    .cells()
                    .iter()
                    .copied()
                    .filter(|cell| matches!(cell, CellId::Projected(..)))
                    .collect();
                if projected.len() != addresses.cells().len() {
                    if projected.is_empty() && addresses.coverage == Coverage::Complete {
                        return Err(allocation_error("a projected binding has only owned slots"));
                    }
                    addresses.restrict_cells(projected, false, input.state());
                }
                let dependencies = capture_dependencies(input, &route.dependencies)?;
                addresses.add_retained_dependencies(&dependencies);
                BindingEdit::Projected {
                    addresses,
                    writable: writable
                        && route.access.restriction != Some(typed::Permission::ReadOnly),
                    origin: capture_origin(input, route.access.derived_from.as_ref())?,
                }
            }
        };
        let Some((&last, earlier)) = target.wrappers.as_slice().split_last() else {
            continue;
        };
        let unique = target.wrapper_coverage == Some(Coverage::Complete) && earlier.is_empty();
        let mut capture_binding = |wrapper: WrapperId, update, roots| {
            let exact = unique && wrapper.age == WrapperAge::Current;
            observe_write(ExactLocation::Binding(wrapper), true)?;
            capture.bindings.push(CapturedBinding {
                target: wrapper,
                update,
                replace: if exact {
                    Replacement::Must
                } else {
                    Replacement::May
                },
                roots,
            });
            Ok::<(), InfrastructureFailure>(())
        };
        for &wrapper in earlier {
            capture_binding(wrapper, update.clone(), roots.clone())?;
        }
        capture_binding(last, update, roots)?;
    }
    for (cell, edit) in &post.cells {
        let address = resolve_writable_cell(input, cell)?;
        for &cell in address.cells() {
            observe_write(ExactLocation::Cell(cell), true)?;
        }
        let (CellPost::Replace(values) | CellPost::WeakJoin(values)) = edit;
        let mut value = CapturedValue::default();
        let mut roots = None;
        for alternative in values {
            if let Some(alternative) = alternative {
                let source = capture_typed_value(input, alternative)?;
                union_optional_roots(&mut roots, source.roots);
                merge_capture(&mut value, source.value);
            } else {
                value
                    .alternatives
                    .push(ValueAlt::plain(PayloadKind::Absent));
            }
        }
        canonicalise_capture(&mut value);
        let store = match edit {
            CellPost::Replace(_) => TypedStore::replace_cells(address, value),
            CellPost::WeakJoin(_) => TypedStore::write_cells(address, value),
        }
        .with_explicit_slot_payload();
        let store = if let Some(roots) = roots {
            store.with_slot_roots(roots)
        } else {
            store
        };
        capture.edits.push(CapturedEdit::Cell(store));
    }
    for (family, edit) in &post.families {
        let holder = resolve_object(input, &family.receiver)?;
        let key = required(input.field(required(holder.ty)?, family.selector))?;
        let mut receivers: Vec<_> = holder
            .value
            .values
            .iter()
            .filter(|value| value.writable)
            .map(|value| value.id)
            .collect();
        if holder.value.values.is_empty() && holder.writable != Some(false) {
            receivers.extend_from_slice(&holder.value.views);
        }
        receivers.sort_unstable();
        receivers.dedup();
        let coverage = holder.allocation_coverage.unwrap_or(Coverage::Partial);
        for &receiver in &receivers {
            observe_write(ExactLocation::Cell(CellId::Projected(receiver, key)), true)?;
        }
        let (FamilyPost::Extend(members) | FamilyPost::ReplaceAll(members)) = edit;
        let replace = match edit {
            FamilyPost::Extend(_) => Replacement::May,
            FamilyPost::ReplaceAll(_) => Replacement::Must,
        };
        let mut captured = Vec::with_capacity(members.len());
        for (member, occurrence) in members {
            captured.push(FamilyMember {
                value: capture_typed_value(input, member)?.value,
                at_most_once: replace == Replacement::Must
                    && *occurrence == typed::Occurrence::AtMostOnce,
            });
        }
        capture.edits.push(CapturedEdit::Family {
            receivers,
            coverage,
            key,
            replace,
            members: captured,
        });
    }
    for (slot, descriptor) in &post.results {
        observe_write(ExactLocation::Result(*slot), true)?;
        let mut result = capture_typed_value(input, descriptor)?;
        result.slot = *slot;
        capture.results.push(result);
    }
    if let typed::Coverage::Unresolved(footprint) = &post.coverage {
        let unknown = capture_uncertainty(input, footprint)?;
        for binding in &unknown.bindings {
            observe_write(ExactLocation::Binding(binding.target), false)?;
        }
        for edit in &unknown.edits {
            match edit {
                CapturedEdit::Cell(store) => {
                    for &cell in required(store.captured_cells())? {
                        observe_write(ExactLocation::Cell(cell), false)?;
                    }
                }
                CapturedEdit::Family { receivers, key, .. } => {
                    for &holder in receivers {
                        observe_write(ExactLocation::Cell(CellId::Projected(holder, *key)), false)?;
                    }
                }
            }
        }
        let bounded = unknown.reach.as_ref();
        for (_, address) in bounded.into_iter().flat_map(|reach| &reach.locations) {
            for &cell in address.cells() {
                observe_write(ExactLocation::Cell(cell), false)?;
            }
        }
        for (slot, _) in unknown.results.iter().map(|(slot, _)| (slot, ())).chain(
            bounded
                .into_iter()
                .flat_map(|reach| reach.results.iter().map(|(slot, _)| (slot, ()))),
        ) {
            if !capture.results.iter().any(|result| result.slot == *slot) {
                return Err(allocation_error(
                    "unresolved result has no prefix descriptor",
                ));
            }
            observe_write(ExactLocation::Result(*slot), false)?;
        }
        capture.uncertainty = Some(unknown);
    }
    Ok(capture)
}

fn capture_uncertainty(
    input: &impl MatchInput,
    footprint: &typed::Footprint,
) -> Result<CapturedUncertainty, InfrastructureFailure> {
    match footprint {
        typed::Footprint::Locations(locations) => match input.uncertain_effects(footprint) {
            Some(capture) => {
                if capture.reach.is_some()
                    || !UncertainLocations::capture(input, locations)?.matches(input, capture)
                {
                    return Err(unresolved_footprint_error());
                }
                Ok(capture.clone())
            }
            None => capture_opaque_locations(input, locations),
        },
        typed::Footprint::WritableReach(bound) => {
            if input.uncertain_effects(footprint).is_some() {
                return Err(allocation_error(
                    "writable reach cannot accept pre-lowered writes",
                ));
            }
            let reach = capture_writable_reach(input, bound)?;
            let bindings = if bound.region == typed::ReachKind::EntryStructure {
                capture_reach_bindings(input, &reach)?
            } else {
                Vec::new()
            };
            Ok(CapturedUncertainty {
                bindings,
                reach: Some(reach),
                ..Default::default()
            })
        }
    }
}

fn unresolved_footprint_error() -> InfrastructureFailure {
    allocation_error("unresolved call effect exceeds its declared footprint")
}

#[derive(Default)]
struct UncertainLocations {
    bindings: FxHashMap<WrapperId, Option<TypeId>>,
    cells: FxHashMap<CellId, Option<TypeId>>,
    families: FxHashMap<(AllocId, FieldKey), Option<TypeId>>,
    results: FxHashMap<usize, Option<TypeId>>,
}

impl UncertainLocations {
    fn capture(
        input: &impl MatchInput,
        locations: &[typed::OutputPort],
    ) -> Result<Self, InfrastructureFailure> {
        use typed::OutputPort;
        let mut expected = Self::default();
        for location in locations {
            match location {
                OutputPort::Binding(wrapper) => {
                    let wrapper = resolve_object(input, wrapper)?;
                    if wrapper.writable == Some(false) {
                        return Err(unresolved_footprint_error());
                    }
                    expected.bindings.extend(
                        wrapper
                            .wrappers
                            .as_slice()
                            .iter()
                            .map(|&target| (target, wrapper.ty)),
                    );
                }
                OutputPort::Cell(cell) => {
                    let address = resolve_writable_cell(input, cell)?;
                    if !cell_may_write(input, cell)? {
                        return Err(unresolved_footprint_error());
                    }
                    let ty = cell_value_type(input, cell)?;
                    expected
                        .cells
                        .extend(address.cells().iter().map(|&cell| (cell, ty)));
                }
                OutputPort::Family(family) => {
                    let receiver = resolve_object(input, &family.receiver)?;
                    if receiver.writable == Some(false)
                        || !receiver.value.values.is_empty()
                            && receiver.value.values.iter().all(|value| !value.writable)
                    {
                        return Err(unresolved_footprint_error());
                    }
                    let holder_type = required(receiver.ty)?;
                    let field = required(input.field(holder_type, family.selector))?;
                    expected.families.extend(
                        receiver
                            .value
                            .values
                            .iter()
                            .filter(|holder| holder.writable)
                            .map(|holder| {
                                let ty = input.field_value_type(holder_type, family.selector);
                                ((holder.id, field), ty)
                            }),
                    );
                }
                OutputPort::Result(slot) => {
                    expected.results.insert(*slot, input.result_type(*slot));
                }
            }
        }
        Ok(expected)
    }

    fn matches(mut self, input: &impl MatchInput, capture: &CapturedUncertainty) -> bool {
        // Consume each declared location once so missing and duplicate edits both fail.
        let bindings_match = capture.bindings.iter().all(|binding| {
            self.bindings
                .remove(&binding.target)
                .is_some_and(|ty| suffix_binding_matches(input, ty, &binding.update))
        });
        if !bindings_match {
            return false;
        }
        let edits_match = capture.edits.iter().all(|edit| match edit {
            CapturedEdit::Cell(store) => store
                .captured_cells()
                .zip(store.captured_value())
                .is_some_and(|(targets, value)| {
                    targets.iter().all(|cell| {
                        self.cells
                            .remove(cell)
                            .is_some_and(|ty| suffix_value_matches(input, ty, value))
                    })
                }),
            CapturedEdit::Family {
                receivers,
                key,
                members,
                ..
            } => {
                !members.is_empty()
                    && receivers.iter().all(|holder| {
                        self.families.remove(&(*holder, *key)).is_some_and(|ty| {
                            members
                                .iter()
                                .all(|member| suffix_value_matches(input, ty, &member.value))
                        })
                    })
            }
        });
        if !edits_match {
            return false;
        }
        capture.results.iter().all(|(slot, value)| {
            self.results
                .remove(slot)
                .is_some_and(|ty| suffix_value_matches(input, ty, value))
        }) && self.bindings.is_empty()
            && self.cells.is_empty()
            && self.families.is_empty()
            && self.results.is_empty()
    }
}

fn cell_value_type(
    input: &impl MatchInput,
    cell: &typed::CellPort,
) -> Result<Option<TypeId>, InfrastructureFailure> {
    Ok(match cell {
        typed::CellPort::Slot(wrapper) => resolve_object(input, wrapper)?.ty,
        typed::CellPort::Terminal(port) => required(input.entry(*port))?.ty,
        typed::CellPort::Edge(holder, selector) => {
            input.field_value_type(required(resolve_object(input, holder)?.ty)?, *selector)
        }
    })
}

fn suffix_value_matches(
    input: &impl MatchInput,
    ty: Option<TypeId>,
    value: &CapturedValue,
) -> bool {
    if value.is_empty() {
        return false;
    }
    let expected = |id| ty.is_some_and(|expected| input.allocation_type(id) == Some(expected));
    let allocation = ty
        .and_then(|ty| input.reach_layout()?.types.get(&ty))
        .map(|descriptor| descriptor.allocation);
    let permitted = |value: &StoredValue| {
        expected(value.id)
            && (!value.writable
                || (0..input.entry_count()).any(|index| {
                    input
                        .entry(typed::PortId(index as u32))
                        .is_some_and(|entry| {
                            entry.ty == ty
                                && entry.writable != Some(false)
                                && entry
                                    .value
                                    .values
                                    .iter()
                                    .any(|source| source.id == value.id && source.writable)
                        })
                }))
    };
    if allocation == Some(true)
        && value.values.is_empty()
        && !value
            .alternatives
            .iter()
            .any(|alt| matches!(alt.kind, PayloadKind::Allocation(_) | PayloadKind::Absent))
    {
        return false;
    }
    value.values.iter().all(permitted)
        && value.views.iter().copied().all(expected)
        && value.alternatives.iter().all(|alt| match alt.kind {
            PayloadKind::Allocation(value) => allocation != Some(false) && permitted(&value),
            PayloadKind::Scalar => allocation != Some(true),
            _ => true,
        })
}

fn cell_may_write(
    input: &impl MatchInput,
    cell: &typed::CellPort,
) -> Result<bool, InfrastructureFailure> {
    Ok(match cell {
        typed::CellPort::Terminal(port) => required(input.entry(*port))?.writable != Some(false),
        typed::CellPort::Slot(wrapper) => resolve_object(input, wrapper)?.writable != Some(false),
        typed::CellPort::Edge(holder, _) => {
            let holder = resolve_object(input, holder)?;
            holder.writable != Some(false)
                && (holder.value.values.is_empty()
                    || holder.value.values.iter().any(|value| value.writable))
        }
    })
}

fn resolve_writable_cell(
    input: &impl MatchInput,
    cell: &typed::CellPort,
) -> Result<AddressSet, InfrastructureFailure> {
    let mut address = resolve_cell(input, cell)?.0;
    if let typed::CellPort::Edge(holder, _) = cell {
        let entry = resolve_object(input, holder)?;
        if entry.value.values.iter().any(|value| !value.writable) {
            let permitted: Vec<_> = address.cells().iter().copied().filter(|candidate| {
                matches!(candidate, CellId::Projected(id, _) if entry.value.values.iter().any(|value| value.id == *id && value.writable))
            }).collect();
            address.restrict_cells(permitted, false, input.state());
        }
    }
    Ok(address)
}

fn suffix_binding_matches(input: &impl MatchInput, ty: Option<TypeId>, edit: &BindingEdit) -> bool {
    let compatible = |test: &dyn Fn(&EntryCapture) -> bool| {
        (0..input.entry_count()).any(|index| {
            input
                .entry(typed::PortId(index as u32))
                .is_some_and(|source| ty.is_some() && source.ty == ty && test(source))
        })
    };
    match edit {
        BindingEdit::Slot => true,
        BindingEdit::Link {
            targets,
            writable,
            routes,
            origin,
            ..
        } => {
            !targets.as_slice().is_empty()
                && targets.as_slice().iter().all(|target| {
                    compatible(&|source| {
                        source.wrappers.as_slice().contains(target)
                            && (!*writable || source.writable == Some(true))
                            && routes
                                .as_ref()
                                .is_none_or(|routes| source.address.as_ref() == Some(routes))
                            && origin.is_none_or(|origin| source.origin == Some(origin))
                    })
                })
        }
        BindingEdit::Projected {
            addresses,
            writable,
            origin,
        } => {
            !addresses.cells().is_empty()
                && addresses
                    .cells()
                    .iter()
                    .all(|cell| matches!(cell, CellId::Projected(..)))
                && compatible(&|source| {
                    source.address.as_ref() == Some(addresses)
                        && (!*writable || source.writable == Some(true))
                        && origin.is_none_or(|origin| source.origin == Some(origin))
                })
        }
    }
}

fn opaque_sources(
    input: &impl MatchInput,
    layout: &ReachLayout,
) -> Result<Vec<(TypeId, CapturedValue)>, InfrastructureFailure> {
    let mut sources = Vec::new();
    for index in 0..input.entry_count() {
        let port = typed::PortId(index as u32);
        if input
            .opaque_source_ports()
            .is_some_and(|ports| !ports.contains(&port))
        {
            continue;
        }
        let Some(entry) = input.entry(port) else {
            continue;
        };
        if !entry.value.is_empty() {
            let ty = required(entry.ty)?;
            required(layout.types.get(&ty))?;
            sources.push((ty, entry.value.clone()));
        }
    }
    Ok(sources)
}

fn capture_opaque_locations(
    input: &impl MatchInput,
    locations: &[typed::OutputPort],
) -> Result<CapturedUncertainty, InfrastructureFailure> {
    let layout = required(input.reach_layout())?.clone();
    let mut reach = CapturedReach {
        bound: typed::ReachBound {
            roots: Vec::new(),
            region: typed::ReachKind::Cell,
            types: Vec::new(),
        },
        sources: opaque_sources(input, &layout)?,
        layout,
        roots: Vec::new(),
        locations: Vec::new(),
        results: Vec::new(),
    };
    for location in locations {
        use typed::{CellPort, OutputPort};
        match location {
            OutputPort::Binding(object) => {
                let root = resolve_object(input, object)?;
                if root.writable == Some(false) {
                    return Err(allocation_error("bounded binding target is read-only"));
                }
                let ty = required(root.ty)?;
                reach
                    .bound
                    .types
                    .push(required(reach.layout.types.get(&ty))?.identity.clone());
                reach.roots.push(root.clone());
            }
            OutputPort::Cell(cell) => {
                if !cell_may_write(input, cell)? {
                    return Err(allocation_error("bounded cell target is read-only"));
                }
                let ty = required(cell_value_type(input, cell)?)?;
                required(reach.layout.types.get(&ty))?;
                let address = resolve_writable_cell(input, cell)?;
                reach.locations.push((ty, address));
            }
            OutputPort::Family(family) => {
                let cell = CellPort::Edge(family.receiver.clone(), family.selector);
                if !cell_may_write(input, &cell)? {
                    return Err(allocation_error("bounded family target is read-only"));
                }
                let receiver = resolve_object(input, &family.receiver)?;
                let ty = required(input.field_value_type(required(receiver.ty)?, family.selector))?;
                required(reach.layout.types.get(&ty))?;
                reach
                    .locations
                    .push((ty, resolve_writable_cell(input, &cell)?));
            }
            OutputPort::Result(slot) => {
                let ty = required(input.result_type(*slot))?;
                required(reach.layout.types.get(&ty))?;
                reach.results.push((*slot, ty));
            }
        }
    }
    reach.bound.types.sort_unstable();
    reach.bound.types.dedup();
    let bindings = capture_reach_bindings(input, &reach)?;
    reach.roots.clear();
    Ok(CapturedUncertainty {
        bindings,
        reach: Some(reach),
        ..Default::default()
    })
}

fn capture_writable_reach(
    input: &impl MatchInput,
    bound: &typed::ReachBound,
) -> Result<CapturedReach, InfrastructureFailure> {
    let layout = required(input.reach_layout())?.clone();
    if layout.types.values().any(|descriptor| {
        descriptor
            .children
            .iter()
            .any(|&(_, ty)| !layout.types.contains_key(&ty))
    }) {
        return Err(allocation_error(
            "writable reach layout is missing a child endpoint",
        ));
    }
    let mut roots = Vec::with_capacity(bound.roots.len());
    let mut types = FxHashSet::default();
    let mut pending = Vec::new();
    for &root in &bound.roots {
        let entry = required(input.entry(root))?;
        let ty = required(entry.ty)?;
        let descriptor = required(layout.types.get(&ty))?;
        let cell = entry.address.as_ref();
        if entry.writable == Some(false)
            || bound.region == typed::ReachKind::Cell && cell.is_none()
            || bound.region == typed::ReachKind::Referent && !descriptor.allocation
        {
            return Err(allocation_error(
                "writable reach has no permitted typed root",
            ));
        }
        pending.push(ty);
        roots.push(entry.clone());
    }
    while let Some(ty) = pending.pop() {
        if types.insert(ty) {
            let descriptor = required(layout.types.get(&ty))?;
            if bound.region != typed::ReachKind::Cell {
                pending.extend(descriptor.children.iter().map(|&(_, child)| child));
            }
        }
    }
    if bound.types.iter().any(|expected| {
        !types
            .iter()
            .any(|ty| layout.types[ty].identity == *expected)
    }) {
        return Err(allocation_error(
            "writable reach contains a type outside its root layout",
        ));
    }
    let sources = opaque_sources(input, &layout)?;
    Ok(CapturedReach {
        bound: bound.clone(),
        layout,
        roots,
        sources,
        locations: Vec::new(),
        results: Vec::new(),
    })
}

fn capture_reach_bindings(
    input: &impl MatchInput,
    reach: &CapturedReach,
) -> Result<Vec<CapturedBinding>, InfrastructureFailure> {
    let mut edits = Vec::new();
    for root in &reach.roots {
        let ty = required(root.ty)?;
        if root.wrappers.as_slice().is_empty()
            || reach
                .bound
                .types
                .binary_search(&reach.layout.types[&ty].identity)
                .is_err()
        {
            continue;
        }
        for &target in root.wrappers.as_slice() {
            let edit = |update, roots| CapturedBinding {
                target,
                update,
                roots,
                replace: Replacement::May,
            };
            edits.push(edit(BindingEdit::Slot, None));
            for index in 0..input.entry_count() {
                let Some(source) = input.entry(typed::PortId(index as u32)) else {
                    continue;
                };
                if source.ty != Some(ty) {
                    continue;
                }
                let targets = SmallSet::from_iter(
                    source
                        .wrappers
                        .as_slice()
                        .iter()
                        .copied()
                        .filter(|&wrapper| wrapper != target),
                );
                if !targets.as_slice().is_empty() {
                    edits.push(edit(
                        BindingEdit::Link {
                            targets,
                            writable: false,
                            coverage: source.wrapper_coverage.unwrap_or(Coverage::Partial),
                            origin: source.origin,
                            routes: source.address.clone(),
                        },
                        source.roots.clone(),
                    ));
                }
                if let Some(addresses) = &source.address
                    && !addresses.cells().is_empty()
                    && addresses
                        .cells()
                        .iter()
                        .all(|cell| matches!(cell, CellId::Projected(..)))
                {
                    edits.push(edit(
                        BindingEdit::Projected {
                            addresses: addresses.clone(),
                            writable: false,
                            origin: source.origin,
                        },
                        source.roots.clone(),
                    ));
                }
            }
        }
    }
    Ok(edits)
}

fn visit_typed_values(
    state: &BorrowState,
    layout: &ReachLayout,
    roots: Vec<(TypeId, CapturedValue)>,
    deep: bool,
    mut visit: impl FnMut(TypeId, &CapturedValue),
) {
    let mut pending: Vec<_> = roots
        .into_iter()
        .map(|(ty, value)| (ty, value, Vec::new()))
        .collect();
    let mut seen = FxHashSet::default();
    while let Some((ty, mut value, mut ancestry)) = pending.pop() {
        visit(ty, &value);
        if !deep {
            continue;
        }
        value.values.retain(|stored| seen.insert((ty, *stored)));
        if value.values.is_empty() {
            continue;
        }
        let Some(descriptor) = layout.types.get(&ty) else {
            continue;
        };
        let recursive = ancestry.contains(&ty);
        if !recursive {
            ancestry.push(ty);
        }
        for &(key, child) in &descriptor.children {
            let Some(endpoint) = layout.types.get(&child) else {
                continue;
            };
            let filtered;
            let capture = if recursive {
                filtered = CapturedValue::new(
                    value
                        .values
                        .iter()
                        .copied()
                        .filter(|value| state.has_captured_projection(value.id, key))
                        .collect(),
                    Vec::new(),
                );
                if filtered.values.is_empty() {
                    continue;
                }
                &filtered
            } else {
                &value
            };
            let mut graph = ResolvedPlace::from_capture(capture);
            if endpoint.allocation {
                // Preserve the observed source permissions while collecting possible values.
                state.step_resolved(&mut graph, key, &layout.local_mutable);
                pending.push((child, capture_graph_values(graph), ancestry.clone()));
            } else {
                let scalar = state.capture_scalar_projection(&graph, key);
                visit(child, &scalar);
            }
        }
    }
}

fn visit_bound_values(
    input: &impl MatchInput,
    bound: &typed::ReachBound,
    mut visit: impl FnMut(&CapturedValue),
) -> Result<(), InfrastructureFailure> {
    let layout = required(input.reach_layout())?;
    let roots = bound
        .roots
        .iter()
        .map(|&port| {
            let entry = required(input.entry(port))?;
            let ty = required(entry.ty)?;
            required(layout.types.get(&ty))?;
            Ok((ty, entry.value.clone()))
        })
        .collect::<Result<_, InfrastructureFailure>>()?;
    visit_typed_values(
        input.state(),
        layout,
        roots,
        bound.region != typed::ReachKind::Cell,
        |ty, value| {
            if bound
                .types
                .binary_search(&layout.types[&ty].identity)
                .is_ok()
            {
                visit(value);
            }
        },
    );
    Ok(())
}

impl CapturedReach {
    fn apply(
        self,
        state: &mut BorrowState,
        order: OrderKey,
    ) -> (PublicationCarriers, Vec<(usize, CapturedValue)>) {
        let mut opaque = OpaqueInputs {
            scope: self.layout.scope,
            ..Default::default()
        };
        visit_typed_values(state, &self.layout, self.sources, true, |ty, value| {
            state.extend_possible_dependencies(value, &mut opaque.dependencies);
            opaque
                .inputs
                .entry(ty)
                .or_default()
                .extend(value.values.iter().copied());
        });
        for values in opaque.inputs.values_mut() {
            values.sort_unstable();
            values.dedup();
        }
        opaque.dependencies.sort_unstable();
        opaque.dependencies.dedup();

        let includes = |ty| {
            self.bound
                .types
                .binary_search(&self.layout.types[&ty].identity)
                .is_ok()
        };
        let mut targets = FxHashMap::<TypeId, Vec<CellId>>::default();
        for (ty, address) in self.locations {
            targets
                .entry(ty)
                .or_default()
                .extend_from_slice(address.cells());
        }
        let mut pending = Vec::new();
        for root in self.roots {
            let ty = root.ty.unwrap();
            if self.bound.region != typed::ReachKind::Referent
                && includes(ty)
                && let Some(address) = &root.address
            {
                targets
                    .entry(ty)
                    .or_default()
                    .extend_from_slice(address.cells());
            }
            if self.bound.region != typed::ReachKind::Cell && self.layout.types[&ty].allocation {
                let mut value = if self.bound.region == typed::ReachKind::EntryStructure
                    && let Some(address) = &root.address
                {
                    let mut resolved = state.resolve_captured_allocs(address, None);
                    state.mark_alloc_provenance_readonly(&self.layout.local_mutable, &mut resolved);
                    capture_graph_values(resolved)
                } else {
                    root.value
                };
                if value.values.is_empty() {
                    value
                        .values
                        .extend(value.views.iter().map(|&id| StoredValue {
                            id,
                            writable: root.writable != Some(false),
                        }));
                }
                pending.push((ty, value, Vec::new()));
            }
        }
        let mut visited = FxHashSet::default();
        while let Some((ty, capture, mut ancestry)) = pending.pop() {
            let recursive = ancestry.contains(&ty);
            if !recursive {
                ancestry.push(ty);
            }
            for stored in capture.values.into_iter().filter(|value| value.writable) {
                if !visited.insert((ty, stored.id)) {
                    continue;
                }
                for &(key, child) in &self.layout.types[&ty].children {
                    if includes(child) {
                        targets
                            .entry(child)
                            .or_default()
                            .push(CellId::Projected(stored.id, key));
                    }
                    if self.layout.types[&child].allocation
                        && (!recursive || state.has_captured_projection(stored.id, key))
                    {
                        let mut graph = ResolvedPlace::from_values(&[stored], Vec::new());
                        step_graph(state, &self.layout.local_mutable, &mut graph, key);
                        pending.push((child, capture_graph_values(graph), ancestry.clone()));
                    }
                }
            }
        }

        let mut receipt = PublicationCarriers::default();
        let mut targets: Vec<_> = targets.into_iter().collect();
        targets.sort_unstable_by_key(|(ty, _)| *ty);
        for (ty, mut cells) in targets {
            if cells.is_empty() {
                continue;
            }
            cells.sort_unstable();
            cells.dedup();
            let value =
                Self::opaque_value(&self.layout, &mut opaque, state, order, ty, &mut receipt);
            for selector in [FieldKey::Elem, FieldKey::MapKey] {
                let holders: Vec<_> = cells
                    .extract_if(.., |cell| {
                        matches!(cell,
                            CellId::Projected(_, key) if *key == selector
                        )
                    })
                    .map(|cell| match cell {
                        CellId::Projected(holder, _) => holder,
                        _ => unreachable!(),
                    })
                    .collect();
                if !holders.is_empty() {
                    let mut member = value.clone();
                    member
                        .alternatives
                        .push(ValueAlt::plain(PayloadKind::Absent));
                    receipt.append(state.publish_family_members(
                        &holders,
                        Coverage::Partial,
                        selector,
                        Replacement::May,
                        &[FamilyMember {
                            value: member,
                            at_most_once: false,
                        }],
                    ));
                    receipt.append(state.publish_family_members(
                        &holders,
                        Coverage::Partial,
                        selector,
                        Replacement::Must,
                        &[],
                    ));
                }
            }
            if !cells.is_empty() {
                receipt.append(
                    state.publish_captured_store(
                        &TypedStore::write_cells(
                            AddressSet::from_cells(cells, Coverage::Partial),
                            value,
                        )
                        .with_explicit_slot_payload(),
                        order,
                    ),
                );
            }
        }
        let results = self
            .results
            .into_iter()
            .map(|(slot, ty)| {
                let value =
                    Self::opaque_value(&self.layout, &mut opaque, state, order, ty, &mut receipt);
                (slot, value)
            })
            .collect();
        (receipt, results)
    }

    fn opaque_value(
        layout: &ReachLayout,
        opaque: &mut OpaqueInputs,
        state: &mut BorrowState,
        order: OrderKey,
        ty: TypeId,
        receipt: &mut PublicationCarriers,
    ) -> CapturedValue {
        let descriptor = &layout.types[&ty];
        let mut value = if descriptor.allocation {
            let fresh = state.opaque_call_id(order, layout.error, opaque.scope, ty.0);
            let mut values = opaque.values(ty);
            values.push(StoredValue {
                id: fresh,
                writable: true,
            });
            if opaque.built.insert(ty) {
                for &(key, child) in &descriptor.children {
                    let child = Self::opaque_value(layout, opaque, state, order, child, receipt);
                    receipt.append(state.publish_captured_store(
                        &TypedStore::write_projected([fresh], key, Coverage::Partial, child, false),
                        order,
                    ));
                }
            }
            CapturedValue::new(values, Vec::new())
        } else {
            CapturedValue {
                alternatives: vec![ValueAlt::plain(PayloadKind::Unknown)],
                ..Default::default()
            }
        };
        opaque.add_uncertain_dependencies(&mut value, !descriptor.allocation);
        canonicalise_capture(&mut value);
        value
    }
}

fn resolve_object<'a>(
    input: &'a impl MatchInput,
    object: &typed::ObjectRef,
) -> Result<&'a EntryCapture, InfrastructureFailure> {
    required(match object {
        typed::ObjectRef::Entry(port) => input.entry(*port),
        _ => input.retained(object),
    })
}

fn resolve_cell(
    input: &impl MatchInput,
    cell: &typed::CellPort,
) -> Result<(AddressSet, bool), InfrastructureFailure> {
    use typed::CellPort;
    match cell {
        CellPort::Terminal(port) => {
            let entry = required(input.entry(*port))?;
            Ok((
                required(entry.address.as_ref())?.clone(),
                entry.writable == Some(true),
            ))
        }
        CellPort::Slot(wrapper) => {
            let entry = resolve_object(input, wrapper)?;
            Ok((
                AddressSet::from_cells(
                    entry.wrappers.as_slice().iter().copied().map(CellId::Slot),
                    entry.wrapper_coverage.unwrap_or(Coverage::Partial),
                )
                .with_singleton_proof(),
                entry.writable == Some(true),
            ))
        }
        CellPort::Edge(holder, selector) => {
            let entry = resolve_object(input, holder)?;
            let field = required(input.field(required(entry.ty)?, *selector))?;
            let cells = entry
                .value
                .values
                .iter()
                .map(|value| value.id)
                .chain(
                    entry
                        .value
                        .views
                        .iter()
                        .copied()
                        .filter(|_| entry.value.values.is_empty()),
                )
                .map(|id| CellId::Projected(id, field));
            let mut addresses = AddressSet::from_cells(
                cells,
                entry.allocation_coverage.unwrap_or(Coverage::Partial),
            );
            if matches!(addresses.cells(), [CellId::Projected(holder, _)] if input.state().alloc_singular(*holder))
            {
                addresses = addresses.with_singleton_proof();
            }
            Ok((
                addresses,
                entry.writable == Some(true) && !entry.value.readonly(),
            ))
        }
    }
}

fn capture_derivation(
    input: &impl MatchInput,
    derivation: Option<&typed::Derivation>,
) -> Result<Option<WrapperId>, InfrastructureFailure> {
    Ok(match derivation {
        None => None,
        Some(typed::Derivation::Entry(port)) => required(input.entry(*port))?.derivation,
        Some(typed::Derivation::Retained(object)) => {
            let holder = resolve_object(input, object)?;
            match holder.wrappers.as_slice() {
                [wrapper]
                    if holder.wrapper_coverage == Some(Coverage::Complete)
                        && wrapper.age == WrapperAge::Current =>
                {
                    Some(*wrapper)
                }
                _ => None,
            }
        }
    })
}

fn capture_origin(
    input: &impl MatchInput,
    derivation: Option<&typed::Derivation>,
) -> Result<Option<Origin>, InfrastructureFailure> {
    Ok(match derivation {
        None => None,
        Some(typed::Derivation::Entry(port)) => required(input.entry(*port))?.origin,
        Some(typed::Derivation::Retained(object)) => {
            let holder = resolve_object(input, object)?;
            let wrapper = capture_derivation(input, derivation)?;
            holder
                .origin
                .filter(|origin| Some(origin.wrapper) == wrapper)
        }
    })
}

fn capture_dependencies(
    input: &impl MatchInput,
    dependencies: &[typed::ReceiverDependency],
) -> Result<DependencySet, InfrastructureFailure> {
    let mut lowered = Vec::with_capacity(dependencies.len());
    for dependency in dependencies {
        let mut coverage = Coverage::Complete;
        let mut receivers = Vec::new();
        match dependency {
            typed::ReceiverDependency::Known(objects) => {
                for object in objects {
                    let entry = resolve_object(input, object)?;
                    if entry.allocation_coverage != Some(Coverage::Complete) {
                        coverage = Coverage::Partial;
                    }
                    receivers.extend(entry.value.values.iter().map(|value| value.id));
                }
            }
            typed::ReceiverDependency::Unresolved(bound) => {
                coverage = Coverage::Partial;
                visit_bound_values(input, bound, |capture| {
                    receivers.extend(capture.values.iter().map(|value| value.id));
                })?;
            }
        }
        lowered.push(ReceiverDependency {
            entries: SmallSet::from_iter(receivers),
            coverage,
        });
    }
    lowered.sort_unstable();
    lowered.dedup();
    Ok(lowered)
}

fn capture_typed_value(
    input: &impl MatchInput,
    descriptor: &typed::ValueRef,
) -> Result<CapturedResult, InfrastructureFailure> {
    use typed::{ScalarSource, ScalarSourceBound, ValueSource};
    let mut transported = None;
    let mut value = match &descriptor.source {
        ValueSource::Allocation(object) => {
            let source = resolve_object(input, object)?;
            transported = Some(source);
            source.value.clone()
        }
        ValueSource::Scalar(ScalarSource::EntryValue(port)) => {
            let source = required(input.entry(*port))?;
            transported = Some(source);
            CapturedValue {
                alternatives: source.value.alternatives.clone(),
                ..Default::default()
            }
            .into_scalar()
        }
        ValueSource::Scalar(ScalarSource::Independent) => captured_scalar(Vec::new()),
        ValueSource::Scalar(ScalarSource::Unresolved(bound)) => {
            let mut dependencies = Vec::new();
            match bound {
                ScalarSourceBound::Values(ports) => {
                    for &port in ports {
                        input.state().extend_possible_dependencies(
                            &required(input.entry(port))?.value,
                            &mut dependencies,
                        );
                    }
                }
                ScalarSourceBound::Reach(bound) => {
                    visit_bound_values(input, bound, |value| {
                        input
                            .state()
                            .extend_possible_dependencies(value, &mut dependencies);
                    })?;
                }
            }
            dependencies.sort_unstable();
            dependencies.dedup();
            captured_scalar(dependencies)
        }
    };
    if matches!(descriptor.source, ValueSource::Scalar(_)) && value.alternatives.is_empty() {
        value
            .alternatives
            .push(ValueAlt::plain(PayloadKind::Scalar));
    }
    let dependencies = capture_dependencies(input, &descriptor.dependencies)?;
    if !dependencies.is_empty() {
        if value.alternatives.is_empty() {
            value
                .alternatives
                .extend(value.values.iter().copied().map(allocation_alt));
            if value.alternatives.is_empty() {
                value
                    .alternatives
                    .push(ValueAlt::plain(PayloadKind::Unknown));
            }
        }
        for alternative in &mut value.alternatives {
            if alternative.kind != PayloadKind::Absent {
                alternative.deps.extend_from_slice(&dependencies);
                alternative.deps.sort_unstable();
                alternative.deps.dedup();
            }
        }
    }
    value.restrict_permission(descriptor.access.restriction == Some(typed::Permission::ReadOnly));
    canonicalise_capture(&mut value);
    let derivation = match descriptor.access.derived_from.as_ref() {
        Some(derivation) => capture_derivation(input, Some(derivation))?,
        None => transported.and_then(|source| source.derivation),
    };
    Ok(CapturedResult {
        slot: 0,
        value,
        roots: transported.and_then(|source| source.roots.clone()),
        derivation: derivation.map(CapturedDerivation::Wrapper),
    })
}

fn merge_capture(target: &mut CapturedValue, mut source: CapturedValue) {
    if target.views.is_empty() && !source.views.is_empty() {
        target
            .views
            .extend(target.values.iter().map(|value| value.id));
    } else if !target.views.is_empty() && source.views.is_empty() {
        source
            .views
            .extend(source.values.iter().map(|value| value.id));
    }
    if source.alternatives.is_empty() {
        target
            .alternatives
            .extend(source.values.iter().copied().map(allocation_alt));
    } else {
        target.alternatives.extend(source.alternatives);
    }
    target.values.extend(source.values);
    target.views.extend(source.views);
}

fn canonicalise_capture(value: &mut CapturedValue) {
    value.values.sort_unstable();
    value.values.dedup();
    value.views.sort_unstable();
    value.views.dedup();
    value.alternatives.sort_unstable();
    value.alternatives.dedup();
}

impl CapturedBinding {
    fn names_wrapper(&self, wrapper: WrapperId) -> bool {
        self.target == wrapper
            || match &self.update {
                BindingEdit::Slot => false,
                BindingEdit::Link {
                    targets,
                    routes,
                    origin,
                    ..
                } => {
                    targets.as_slice().contains(&wrapper)
                        || routes
                            .as_ref()
                            .is_some_and(|route| route.cells().contains(&CellId::Slot(wrapper)))
                        || origin.is_some_and(|origin| {
                            origin.wrapper == wrapper || origin.owner == Some(wrapper)
                        })
                }
                BindingEdit::Projected {
                    addresses, origin, ..
                } => {
                    origin.is_some_and(|origin| {
                        origin.wrapper == wrapper || origin.owner == Some(wrapper)
                    }) || addresses.cells().contains(&CellId::Slot(wrapper))
                }
            }
    }

    fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        self.target = retirement.remap_wrapper(self.target);
        match &mut self.update {
            BindingEdit::Slot => {}
            BindingEdit::Link {
                targets,
                routes,
                origin,
                ..
            } => {
                *targets = SmallSet::from_iter(
                    targets
                        .as_slice()
                        .iter()
                        .copied()
                        .map(|wrapper| retirement.remap_wrapper(wrapper)),
                );
                if let Some(routes) = routes {
                    routes.remap_allocations(state, retirement);
                }
                remap_origin(origin, retirement);
            }
            BindingEdit::Projected {
                addresses, origin, ..
            } => {
                addresses.remap_allocations(state, retirement);
                remap_origin(origin, retirement);
            }
        }
    }

    fn apply(self, state: &mut BorrowState, possible: bool) -> PublicationCarriers {
        let (alts, coverage, routes) = match self.update {
            BindingEdit::Slot => (
                SmallSet::One(BindingAlt {
                    target: BindingTarget::OwnSlot,
                    origin: None,
                    writable: true,
                }),
                Coverage::Complete,
                Vec::new(),
            ),
            BindingEdit::Link {
                targets,
                writable,
                coverage,
                origin,
                routes,
            } => (
                SmallSet::from_iter(targets.as_slice().iter().copied().map(|source| BindingAlt {
                    target: BindingTarget::Link(source),
                    origin,
                    writable,
                })),
                if routes
                    .as_ref()
                    .is_some_and(|addresses| addresses.coverage == Coverage::Partial)
                {
                    Coverage::Partial
                } else {
                    coverage
                },
                routes.map_or_else(Vec::new, AddressSet::into_retained_routes),
            ),
            BindingEdit::Projected {
                addresses,
                writable,
                origin,
            } => (
                SmallSet::from_iter(addresses.cells().iter().copied().map(|cell| BindingAlt {
                    target: match cell {
                        CellId::Projected(holder, key) => BindingTarget::Projected(holder, key),
                        CellId::Slot(_) => {
                            unreachable!("projected bindings capture projected cells")
                        }
                    },
                    origin,
                    writable,
                })),
                addresses.coverage,
                addresses.into_retained_routes(),
            ),
        };
        state.publish_captured_binding(
            self.target,
            alts,
            coverage,
            routes,
            if possible {
                Replacement::May
            } else {
                self.replace
            },
            self.roots,
        )
    }
}

fn remap_origin(origin: &mut Option<Origin>, retirement: AllocationRetirement) {
    if let Some(origin) = origin {
        origin.wrapper = retirement.remap_wrapper(origin.wrapper);
        origin.owner = origin.owner.map(|owner| retirement.remap_wrapper(owner));
    }
}

impl CapturedEdit {
    fn names_wrapper(&self, wrapper: WrapperId) -> bool {
        matches!(self, Self::Cell(store) if wrapper.age == WrapperAge::Current && store.captures_slot(wrapper.local))
    }

    fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        match self {
            Self::Cell(store) => store.remap(state, retirement),
            Self::Family {
                receivers, members, ..
            } => {
                for receiver in receivers {
                    retirement.remap_id(state, receiver);
                }
                for member in members {
                    member.value.remap(state, retirement);
                }
            }
        }
    }

    fn apply(
        self,
        state: &mut BorrowState,
        order: OrderKey,
        possible: bool,
    ) -> PublicationCarriers {
        match self {
            Self::Cell(store) => {
                let store = if possible {
                    store.without_kill()
                } else {
                    store
                };
                state.publish_captured_store(&store, order)
            }
            Self::Family {
                mut receivers,
                coverage,
                key,
                replace,
                members,
            } => {
                receivers.sort_unstable();
                receivers.dedup();
                state.publish_family_members(
                    &receivers,
                    if possible {
                        Coverage::Partial
                    } else {
                        coverage
                    },
                    key,
                    replace,
                    &members,
                )
            }
        }
    }
}

impl CapturedResult {
    pub(in crate::compiler_frontend::analysis::borrow_checker) fn remap(
        &mut self,
        state: &BorrowState,
        retirement: AllocationRetirement,
    ) {
        self.value.remap(state, retirement);
        match &mut self.derivation {
            Some(CapturedDerivation::Wrapper(wrapper))
                if retirement.remap_wrapper(*wrapper) != *wrapper =>
            {
                self.derivation = None;
            }
            Some(CapturedDerivation::Roots(roots)) => {
                let retired = roots
                    .iter_ones()
                    .find(|&root| retirement.retires_local(root));
                if let Some(root) = retired {
                    roots.remove(root);
                }
            }
            _ => {}
        }
    }
}

impl CapturedPost {
    fn write_footprints(
        &self,
        include_suffix: bool,
    ) -> impl Iterator<Item = super::super::super::facts::AccessFootprint> + '_ {
        use super::super::super::facts::{AccessFootprint, PlaceUse};
        let suffix = self.uncertainty.as_ref().filter(|_| include_suffix);
        let exact = self
            .edits
            .iter()
            .chain(suffix.into_iter().flat_map(|suffix| &suffix.edits))
            .filter_map(|edit| {
                let address = match edit {
                    CapturedEdit::Cell(store) => store.captured_address()?.clone(),
                    CapturedEdit::Family {
                        receivers,
                        key,
                        coverage,
                        ..
                    } => AddressSet::from_cells(
                        receivers
                            .iter()
                            .map(|&holder| CellId::Projected(holder, *key)),
                        *coverage,
                    ),
                };
                Some(AccessFootprint::cells(address, PlaceUse::Replace))
            });
        let reach = suffix
            .and_then(|suffix| suffix.reach.as_ref())
            .into_iter()
            .flat_map(|reach| {
                reach.roots.iter().map(|root| {
                    let direct = reach.bound.region != typed::ReachKind::Referent;
                    let deep = reach.bound.region != typed::ReachKind::Cell;
                    let mut access = AccessFootprint::cells(
                        root.address
                            .as_ref()
                            .filter(|_| direct)
                            .cloned()
                            .unwrap_or_else(|| AddressSet::from_cells([], Coverage::Complete)),
                        if deep {
                            PlaceUse::Mutate { deep }
                        } else {
                            PlaceUse::Replace
                        },
                    );
                    if deep && !root.value.is_empty() {
                        access.content = Some(ResolvedPlace::from_capture(&root.value));
                    }
                    access
                })
            });
        let direct_suffix = suffix
            .and_then(|suffix| suffix.reach.as_ref())
            .into_iter()
            .flat_map(|reach| {
                reach
                    .locations
                    .iter()
                    .map(|(_, address)| AccessFootprint::cells(address.clone(), PlaceUse::Replace))
            });
        let prefix_reachable = suffix
            .and_then(|suffix| suffix.reach.as_ref())
            .filter(|reach| reach.bound.region != typed::ReachKind::Cell)
            .into_iter()
            .flat_map(|_| {
                self.edits.iter().flat_map(|edit| {
                    let (cell, members) = match edit {
                        CapturedEdit::Cell(store) => (store.captured_value(), &[][..]),
                        CapturedEdit::Family { members, .. } => (None, members.as_slice()),
                    };
                    cell.into_iter()
                        .chain(members.iter().map(|member| &member.value))
                })
            })
            .filter(|value| !value.values.is_empty())
            .map(|value| {
                let mut footprint = AccessFootprint::cells(
                    AddressSet::from_cells([], Coverage::Complete),
                    PlaceUse::Mutate { deep: true },
                );
                footprint.content = Some(ResolvedPlace::from_capture(value));
                footprint
            });
        exact
            .chain(reach)
            .chain(direct_suffix)
            .chain(prefix_reachable)
    }

    pub(in crate::compiler_frontend::analysis::borrow_checker) fn weaken_collisions(
        &mut self,
        state: &BorrowState,
    ) {
        let mut bindings = FxHashSet::default();
        let duplicate = self
            .bindings
            .iter()
            .any(|binding| !bindings.insert(binding.target));
        if duplicate {
            self.weaken_writes();
            return;
        }
        let writes: Vec<_> = self
            .write_footprints(false)
            .map(|write| (0, write))
            .collect();
        if !overlapping_owners(state, &writes, |_, _| true).is_empty() {
            self.weaken_writes();
        }
    }

    fn weaken_writes(&mut self) {
        for binding in &mut self.bindings {
            binding.replace = Replacement::May;
        }
        for edit in &mut self.edits {
            match edit {
                CapturedEdit::Cell(store) => store.weaken(),
                CapturedEdit::Family {
                    replace, members, ..
                } => {
                    *replace = Replacement::May;
                    for member in members {
                        member.at_most_once = false;
                    }
                }
            }
        }
    }

    pub(in crate::compiler_frontend::analysis::borrow_checker) fn names_wrapper(
        &self,
        wrapper: WrapperId,
    ) -> bool {
        self.bindings
            .iter()
            .any(|binding| binding.names_wrapper(wrapper))
            || self.edits.iter().any(|edit| edit.names_wrapper(wrapper))
            || self.uncertainty.as_ref().is_some_and(|unknown| {
                unknown
                    .bindings
                    .iter()
                    .any(|binding| binding.names_wrapper(wrapper))
                    || unknown.edits.iter().any(|edit| edit.names_wrapper(wrapper))
                    || unknown.reach.as_ref().is_some_and(|reach| {
                        reach.roots.iter().any(|root| {
                            root.wrappers.as_slice().contains(&wrapper)
                                || root.address.as_ref().is_some_and(|address| {
                                    address.cells().contains(&CellId::Slot(wrapper))
                                })
                        }) || reach
                            .locations
                            .iter()
                            .any(|(_, address)| address.cells().contains(&CellId::Slot(wrapper)))
                    })
            })
    }

    pub(in crate::compiler_frontend::analysis::borrow_checker) fn remap(
        &mut self,
        state: &BorrowState,
        retirement: AllocationRetirement,
    ) {
        for binding in &mut self.bindings {
            binding.remap(state, retirement);
        }
        for edit in &mut self.edits {
            edit.remap(state, retirement);
        }
        for result in &mut self.results {
            result.remap(state, retirement);
        }
        if let Some(unknown) = &mut self.uncertainty {
            for binding in &mut unknown.bindings {
                binding.remap(state, retirement);
            }
            for edit in &mut unknown.edits {
                edit.remap(state, retirement);
            }
            for (_, value) in &mut unknown.results {
                value.remap(state, retirement);
            }
            if let Some(reach) = &mut unknown.reach {
                for root in &mut reach.roots {
                    root.remap(state, retirement);
                }
                for (_, value) in &mut reach.sources {
                    value.remap(state, retirement);
                }
                for (_, address) in &mut reach.locations {
                    address.remap_allocations(state, retirement);
                }
            }
        }
    }

    pub(in crate::compiler_frontend::analysis::borrow_checker) fn apply(
        self,
        state: &mut BorrowState,
        order: OrderKey,
    ) -> (Vec<CapturedResult>, PublicationCarriers) {
        let Self {
            bindings,
            edits,
            mut results,
            uncertainty,
        } = self;
        let mut receipt = PublicationCarriers::default();
        for edit in bindings {
            receipt.append(edit.apply(state, false));
        }
        for edit in edits {
            receipt.append(edit.apply(state, order, false));
        }
        if let Some(unknown) = uncertainty {
            // Unknown suffixes operate on the prefix's state, and cannot
            // establish a definite replacement of their own.
            for edit in unknown.bindings {
                receipt.append(edit.apply(state, true));
            }
            for edit in unknown.edits {
                receipt.append(edit.apply(state, order, true));
            }
            let mut suffix_results = unknown.results;
            if let Some(reach) = unknown.reach {
                let (suffix_receipt, bounded_results) = reach.apply(state, order);
                receipt.append(suffix_receipt);
                suffix_results.extend(bounded_results);
            }
            for (slot, value) in suffix_results {
                let result = results
                    .iter_mut()
                    .find(|result| result.slot == slot)
                    .expect("validated prefix has a result descriptor for the affected slot");
                merge_capture(&mut result.value, value);
                canonicalise_capture(&mut result.value);
                result.derivation = None;
            }
        }
        (results, receipt)
    }
}

fn union_optional_roots(target: &mut Option<RootSet>, source: Option<RootSet>) {
    if let Some(source) = source {
        match target {
            Some(target) => {
                target.union_with(&source);
            }
            None => *target = Some(source),
        }
    }
}

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

fn captured_scalar(deps: DependencySet) -> CapturedValue {
    CapturedValue {
        alternatives: vec![ValueAlt {
            kind: PayloadKind::Scalar,
            deps,
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
                    captured_scalar(deps.clone()),
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
            step_graph(input.state, input.view.local_mutable, &mut nested, key);
            visit_uncertain_scalar_stores(input, facts, index, child, nested, path, deps, ops)?;
        } else if !has_scalar_store(facts, index, path) {
            ops.push(InvokeLayerOp::Store(TypedStore::write_projected(
                holders.ids.iter().copied(),
                key,
                Coverage::Partial,
                captured_scalar(deps.clone()),
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

    pub(super) fn arguments(&self) -> &'a [CallArgumentTransfer] {
        self.args
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

    #[allow(dead_code)]
    pub(super) fn expression(
        &self,
        index: usize,
    ) -> Option<crate::compiler_frontend::hir::ids::HirValueId> {
        self.args.get(index).map(|arg| arg.argument)
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
                    step_graph(input.state, input.view.local_mutable, &mut graph, key);
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
                value: captured_scalar(deps.clone()),
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
    step_graph(input.state, input.view.local_mutable, &mut place, key);
    capture_graph_values(place)
}

fn step_graph(state: &BorrowState, mutable: &[bool], graph: &mut ResolvedPlace, key: FieldKey) {
    state.step_resolved(graph, key, mutable);
    state.mark_alloc_provenance_readonly(mutable, graph);
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
