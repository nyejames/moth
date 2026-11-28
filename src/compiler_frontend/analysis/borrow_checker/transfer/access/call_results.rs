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
    StoredValue, TypedStore, ValueAlt, ValueKind as PayloadKind, WrapperAge, WrapperId,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::call_semantics::CallReturns;
use crate::compiler_frontend::analysis::borrow_checker::transfer::{
    BorrowTransferContext, InlineVec, InvokeLane, InvokeLayerOp,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTypeIdentity, project_type_id_to_canonical_identity,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::{ExternalReturnAlias, ExternalReturnSlot};
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;
use crate::compiler_frontend::hir::statements::HirLocalDestination;
use crate::compiler_frontend::public_call_summary::{CallField, CallFieldHir};
use crate::compiler_frontend::public_call_summary::{allocation_error, typed};
use rustc_hash::{FxHashMap, FxHashSet};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

pub(in crate::compiler_frontend::analysis::borrow_checker) fn capture_boundary_value(
    context: &BorrowTransferContext<'_>,
    view: super::super::super::OperationView<'_>,
    block: crate::compiler_frontend::hir::ids::BlockId,
    state: &mut BorrowState,
    value: crate::compiler_frontend::hir::ids::HirValueId,
) -> Result<CapturedValue, BorrowCheckError> {
    stored_values(
        &LayerValueContext {
            context,
            view,
            block_id: block,
            materialise_contents: true,
        },
        state,
        value,
        true,
    )
}

/// Canonical predecessor observations. A wrapper, its terminal cell and its
/// value are independent observations, even when they share one local spelling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct EntryCapture {
    pub wrappers: SmallSet<WrapperId>,
    pub wrapper_coverage: Option<Coverage>,
    pub role: Option<typed::Role>,
    pub writable: Option<bool>,
    pub address: Option<AddressSet>,
    pub value: CapturedValue,
    pub allocation_coverage: Option<Coverage>,
    pub derivation: Option<WrapperId>,
    pub derivation_roots: Option<RootSet>,
    pub origin: Option<Origin>,
    pub roots: Option<RootSet>,
    pub ty: Option<TypeId>,
}

/// Resolve declared selectors from the actual's captured local, not from a
/// receiving name or an ordinal in the caller's local layout.
#[cfg(test)]
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
            capture_local_value_port(
                state,
                mutable,
                *required(actuals.get(*parameter))?,
                path,
                &field,
                &allocation,
            )
        })
        .collect()
}

pub(in crate::compiler_frontend::analysis::borrow_checker) fn capture_local_value_port(
    state: &BorrowState,
    mutable: &[bool],
    (local, mut ty): (usize, TypeId),
    path: &[CallField],
    field: impl Fn(TypeId, CallField) -> Option<(FieldKey, TypeId)>,
    allocation: impl Fn(TypeId) -> bool,
) -> Result<EntryCapture, InfrastructureFailure> {
    let mut selectors = Vec::with_capacity(path.len());
    for &selector in path {
        let (key, child) = required(field(ty, selector))?;
        selectors.push(key);
        ty = child;
    }
    let mut place = state.resolve_place_path(local, mutable, selectors);
    let heap = allocation(ty);
    state.load_place_content(&mut place, mutable, !heap);
    let writable =
        place.permission.wrapper_readonly_source.is_none() && !place.permission.holder_readonly;
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
        derivation_roots: Some(state.derived_roots_from_source(local)),
        writable: Some(writable),
        ..Default::default()
    })
}

pub(super) fn capture_typed_actuals(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    ports: &[typed::EntryPort],
) -> Result<Vec<EntryCapture>, BorrowCheckError> {
    use typed::EntryPort;
    let mut captured = vec![EntryCapture::default(); ports.len()];
    for (at, port) in ports.iter().enumerate() {
        let (parameter, selectors) = match port {
            EntryPort::Wrapper(index)
            | EntryPort::Immediate(index)
            | EntryPort::Terminal(index) => (*index, &[][..]),
            EntryPort::Value { parameter, path } => (*parameter, path.as_ref()),
            EntryPort::Edge(..) => continue,
        };
        let ty = actuals.ty(input, parameter)?;
        let derivation_roots = matches!(port, EntryPort::Value { .. }).then(|| {
            super::alias_derivation_roots_from_expression(
                input.view,
                input.state,
                input.context.expressions,
                actuals.arguments()[parameter].argument,
            )
        });
        let mut entry = {
            let actual = actuals.root_entry(input, parameter)?;
            let place = actual.place.as_ref().filter(|_| !actual.is_copy);
            let mut capture = EntryCapture {
                ty: Some(ty),
                value: actual.value.as_ref().unwrap().clone(),
                derivation_roots,
                ..Default::default()
            };
            if let Some(place) = place {
                capture.address = Some(place.address.clone());
                capture.origin = place.captured_origin();
                capture.derivation = capture
                    .origin
                    .map(|origin| origin.wrapper)
                    .or_else(|| place.projection_owner());
                capture.writable = Some(
                    place.permission.wrapper_readonly_source.is_none()
                        && !place.permission.holder_readonly,
                );
                if !place.route.projected {
                    let wrapper = WrapperId::current(place.route.local);
                    if !matches!(port, EntryPort::Immediate(_)) {
                        capture.wrappers = SmallSet::One(wrapper);
                        capture.wrapper_coverage = Some(Coverage::Complete);
                    }
                    let binding = input.state.boundary_binding(wrapper);
                    let classify_role = |alt: &BindingAlt| match alt.target {
                        BindingTarget::OwnSlot => typed::Role::Slot,
                        BindingTarget::Link(_) => typed::Role::WholeAlias,
                        BindingTarget::Projected(..) => typed::Role::ProjectedAlias,
                    };
                    if binding.coverage == Coverage::Complete {
                        capture.role =
                            binding
                                .alternatives
                                .first()
                                .map(classify_role)
                                .filter(|role| {
                                    binding
                                        .alternatives
                                        .iter()
                                        .all(|alt| classify_role(alt) == *role)
                                });
                    }
                    if matches!(port, EntryPort::Immediate(_)) {
                        capture.wrappers =
                            SmallSet::from_iter(binding.alternatives.iter().filter_map(|alt| {
                                match alt.target {
                                    BindingTarget::OwnSlot => Some(wrapper),
                                    BindingTarget::Link(immediate) => Some(immediate),
                                    BindingTarget::Projected(..) => None,
                                }
                            }));
                        capture.wrapper_coverage = Some(binding.coverage);
                    }
                } else {
                    capture.role = Some(typed::Role::ProjectedAlias);
                }
            } else {
                capture.role = Some(typed::Role::Slot);
            }
            if heap_type(input.context, ty)? {
                capture.allocation_coverage =
                    Some(place.and_then(|place| place.content.as_ref()).map_or_else(
                        || ResolvedPlace::from_capture(&capture.value).holder_coverage(),
                        ResolvedPlace::holder_coverage,
                    ));
            }
            capture
        };
        if matches!(port, EntryPort::Terminal(_)) && entry.address.is_none() {
            // An evaluated rvalue has no caller-visible terminal to replace.
            entry.address = Some(AddressSet::from_cells([], Coverage::Complete));
        }
        for &selector in selectors {
            entry = capture_entry_edge(
                input.context,
                input.state,
                input.view.local_mutable,
                &entry,
                selector,
            )?;
        }
        captured[at] = entry;
    }
    for (at, port) in ports.iter().enumerate() {
        if let EntryPort::Edge(holder, selector) = port {
            captured[at] = capture_entry_edge(
                input.context,
                input.state,
                input.view.local_mutable,
                &captured[holder.0 as usize],
                *selector,
            )?;
        }
    }
    Ok(captured)
}

fn capture_entry_edge(
    context: &BorrowTransferContext<'_>,
    state: &BorrowState,
    mutable: &[bool],
    entry: &EntryCapture,
    selector: CallField,
) -> Result<EntryCapture, BorrowCheckError> {
    let (key, ty) = boundary_child(context, required(entry.ty)?, selector)?;
    let mut graph = ResolvedPlace::from_capture(&entry.value);
    if let Some(coverage) = entry.allocation_coverage {
        graph.coverage = coverage;
    }
    let mut address = AddressSet::from_cells(
        graph
            .ids
            .iter()
            .chain(&graph.views)
            .map(|&id| CellId::Projected(id, key)),
        graph.holder_coverage(),
    );
    if matches!(address.cells(), [CellId::Projected(holder, _)] if state.alloc_singular(*holder)) {
        address = address.with_singleton_proof();
    }
    let allocation = heap_type(context, ty)?;
    let (value, coverage) = if allocation {
        step_graph(state, mutable, &mut graph, key);
        let coverage = graph.holder_coverage();
        (capture_graph_values(graph), Some(coverage))
    } else {
        (state.capture_scalar_projection(&graph, key), None)
    };
    Ok(EntryCapture {
        address: Some(address),
        value,
        ty: Some(ty),
        allocation_coverage: coverage,
        writable: entry.writable,
        origin: entry.origin,
        derivation: entry.derivation,
        derivation_roots: entry.derivation_roots.clone(),
        ..Default::default()
    })
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
            derivation_roots: None,
            origin: None,
            roots: None,
            ty: None,
        }
    }
}

type ReachTypes = FxHashMap<TypeId, ReachType>;
type CallSite = (crate::compiler_frontend::hir::ids::BlockId, OrderKey, bool);
type RetainedTypes = BTreeMap<typed::ObjectRef, (typed::StorageKind, Option<TypeId>)>;

#[derive(Default)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct TypedCallCache {
    types: RefCell<FxHashMap<InlineVec<TypeId>, Rc<ReachTypes>>>,
    lanes: RefCell<FxHashMap<CallSite, InlineVec<PreparedLane>>>,
}

struct PreparedLane {
    layout: Rc<ReachLayout>,
    results: InlineVec<TypeId>,
    entry_types: InlineVec<TypeId>,
    parameters: InlineVec<TypeId>,
    selection: Option<Vec<typed::SupportId>>,
    projected: Option<typed::Lane>,
    retained: RetainedTypes,
}

struct CallerMatch<'a, 'hir> {
    state: &'a BorrowState,
    context: &'a BorrowTransferContext<'hir>,
    actuals: &'a [EntryCapture],
    retained: BTreeMap<&'a typed::ObjectRef, EntryCapture>,
    result_types: &'a [TypeId],
    local_mutable: &'a [bool],
    reach: Rc<ReachLayout>,
}

impl MatchInput for CallerMatch<'_, '_> {
    fn state(&self) -> &BorrowState {
        self.state
    }

    fn entry(&self, port: typed::PortId) -> Option<&EntryCapture> {
        self.actuals.get(port.0 as usize)
    }

    fn retained(&self, object: &typed::ObjectRef) -> Option<&EntryCapture> {
        self.retained.get(object)
    }

    fn field(&self, holder: TypeId, selector: CallField) -> Option<FieldKey> {
        boundary_child(self.context, holder, selector)
            .ok()
            .map(|(key, _)| key)
    }

    fn entry_count(&self) -> usize {
        self.actuals.len()
    }

    fn reach_layout(&self) -> Option<&Rc<ReachLayout>> {
        Some(&self.reach)
    }

    fn local_mutable(&self) -> &[bool] {
        self.local_mutable
    }

    fn field_value_type(&self, holder: TypeId, selector: CallField) -> Option<TypeId> {
        boundary_child(self.context, holder, selector)
            .ok()
            .map(|(_, ty)| ty)
    }

    fn result_type(&self, slot: usize) -> Option<TypeId> {
        self.result_types.get(slot).copied()
    }
}

pub(super) fn match_call_actuals(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    entries: &[EntryCapture],
    parameters: &[TypeId],
    result_types: &[TypeId],
    lane: &typed::Lane,
    error: bool,
) -> Result<MatchedLane, BorrowCheckError> {
    let context = input.context;
    let selected = input.call_components;
    let site = (input.block_id, input.current_order, error);
    let mut cache = context.typed_call_cache.lanes.borrow_mut();
    let shapes = cache.entry(site).or_default();
    let matching = shapes.iter().position(|shape| {
        shape
            .entry_types
            .iter()
            .copied()
            .eq(entries.iter().filter_map(|entry| entry.ty))
            && &*shape.results == result_types
            && &*shape.parameters == parameters
            && shape.selection.as_deref() == selected
    });
    let shape = if let Some(index) = matching {
        &shapes[index]
    } else {
        let types = reachable_call_types(context, entries, parameters, result_types)?;
        // Guard collapse affects feasibility and correlated result rows. Keep
        // canonical projection for guarded lanes, once per stable call shape.
        let projected = selected.and_then(|supports| {
            matches!(lane, typed::Lane::Decision(_)).then(|| lane.project_supports(supports))
        });
        let effective_lane = projected.as_ref().unwrap_or(lane);
        let selection = selected.filter(|_| projected.is_none());
        let mut retained = RetainedTypes::new();
        effective_lane.visit_compatible(
            |_| Ok::<_, BorrowCheckError>(None),
            |_, product| {
                for post in selected_components(product, selection)
                    .flat_map(|component| &component.alternatives)
                {
                    for (object, kind, identity) in &post.retained {
                        if !retained.contains_key(object) {
                            retained.insert(
                                object.clone(),
                                (
                                    *kind,
                                    retained_type(
                                        context,
                                        parameters,
                                        result_types,
                                        object,
                                        identity,
                                        &types,
                                    ),
                                ),
                            );
                        }
                    }
                }
                Ok(())
            },
        )?;
        shapes.push(PreparedLane {
            layout: Rc::new(ReachLayout {
                types,
                error,
                scope: 1,
            }),
            results: result_types.iter().copied().collect(),
            entry_types: entries.iter().filter_map(|entry| entry.ty).collect(),
            parameters: parameters.iter().copied().collect(),
            selection: selected.map(<[_]>::to_vec),
            projected,
            retained,
        });
        shapes.last().expect("installed call shape")
    };
    let lane = shape.projected.as_ref().unwrap_or(lane);
    let selection = selected.filter(|_| shape.projected.is_none());
    // No retained rows means no retained IDs to resolve.
    let retained = shape
        .retained
        .iter()
        .filter_map(|(object, &(kind, ty))| {
            let ty = ty?;
            let summarized = matches!(object, typed::ObjectRef::Family(..));
            let node = input.state.call_export_coordinate(object);
            let capture = match kind {
                typed::StorageKind::Allocation => {
                    let id = input.state.call_site_id(
                        input.current_order,
                        error,
                        node,
                        summarized,
                        input.block_id,
                        input.view.block_successors,
                    );
                    EntryCapture {
                        value: CapturedValue::new(
                            vec![StoredValue { id, writable: true }],
                            Vec::new(),
                        ),
                        allocation_coverage: Some(Coverage::Complete),
                        writable: Some(true),
                        ty: Some(ty),
                        ..Default::default()
                    }
                }
                typed::StorageKind::Wrapper => {
                    let id = input.state.call_wrapper_id(
                        input.current_order,
                        error,
                        node,
                        summarized,
                        input.block_id,
                        input.view.block_successors,
                    );
                    EntryCapture {
                        wrappers: SmallSet::One(id),
                        wrapper_coverage: Some(Coverage::Complete),
                        address: Some(AddressSet::from_cells(
                            [CellId::Slot(id)],
                            Coverage::Complete,
                        )),
                        origin: Some(Origin {
                            wrapper: id,
                            owner: None,
                            isolated: false,
                        }),
                        writable: Some(true),
                        ty: Some(ty),
                        ..Default::default()
                    }
                }
            };
            Some((object, capture))
        })
        .collect();
    match_selected_lane(
        &CallerMatch {
            state: input.state,
            context,
            actuals: entries,
            retained,
            result_types,
            local_mutable: input.view.local_mutable,
            reach: Rc::clone(&shape.layout),
        },
        lane,
        selection,
    )
    .map_err(Into::into)
}

fn reachable_call_types(
    context: &BorrowTransferContext<'_>,
    entries: &[EntryCapture],
    parameters: &[TypeId],
    results: &[TypeId],
) -> Result<Rc<ReachTypes>, BorrowCheckError> {
    let roots: InlineVec<_> = entries
        .iter()
        .filter_map(|entry| entry.ty)
        .chain(parameters.iter().chain(results).copied())
        .collect();
    let mut cache = context.typed_call_cache.types.borrow_mut();
    if let Some(layout) = cache.get(&roots) {
        return Ok(Rc::clone(layout));
    }
    let types = context.type_environment;
    let mut reachable = roots.clone();
    let mut layout = FxHashMap::default();
    while let Some(ty) = reachable.pop() {
        if layout.contains_key(&ty) {
            continue;
        }
        let allocation = heap_type(context, ty)?;
        let mut children = Vec::new();
        if allocation {
            CallField::for_each_child(ty, types, |selector, child| {
                let (key, _) = boundary_child(context, ty, selector)?;
                children.push((key, child));
                reachable.push(child);
                Ok::<(), BorrowCheckError>(())
            })?;
        }
        let identity =
            project_type_id_to_canonical_identity(ty, types, &context.type_projection).ok();
        layout.insert(
            ty,
            ReachType {
                identity,
                allocation,
                children,
            },
        );
    }
    let layout = Rc::new(layout);
    cache.insert(roots, Rc::clone(&layout));
    Ok(layout)
}

fn retained_type(
    context: &BorrowTransferContext<'_>,
    parameters: &[TypeId],
    results: &[TypeId],
    object: &typed::ObjectRef,
    identity: &CanonicalTypeIdentity,
    layout: &ReachTypes,
) -> Option<TypeId> {
    use typed::{ExportPort, ObjectRef};
    let endpoint = match object {
        ObjectRef::Exact(ExportPort::Result(path)) => results
            .get(path.slot)
            .copied()
            .map(|root| (root, path.path.as_slice())),
        ObjectRef::Exact(ExportPort::Cell(path)) => parameters
            .get(path.index)
            .copied()
            .map(|root| (root, path.path.as_slice())),
        ObjectRef::Exact(ExportPort::Binding(index) | ExportPort::PrivateTerminal(index)) => {
            parameters.get(*index).copied().map(|root| (root, &[][..]))
        }
        _ => None,
    };
    if let Some((mut ty, path)) = endpoint {
        for &selector in path {
            ty = selector.type_child(ty, context.type_environment)?;
        }
        Some(ty)
    } else {
        layout
            .iter()
            .find_map(|(&ty, shape)| (shape.identity.as_ref() == Some(identity)).then_some(ty))
    }
}

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
    fn local_mutable(&self) -> &[bool] {
        &[]
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct CapturedBinding {
    pub target: WrapperId,
    pub update: BindingEdit,
    pub replace: Replacement,
    pub roots: Option<RootSet>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct CapturedUncertainty {
    pub bindings: Vec<CapturedBinding>,
    pub edits: Vec<CapturedEdit>,
    pub results: Vec<(usize, CapturedValue)>,
    pub reach: Option<CapturedReach>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct ReachType {
    pub identity: Option<crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity>,
    pub allocation: bool,
    pub children: Vec<(FieldKey, TypeId)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct ReachLayout {
    pub types: Rc<ReachTypes>,
    pub error: bool,
    pub scope: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct CapturedReach {
    bound: typed::ReachBound,
    layout: Rc<ReachLayout>,
    roots: Vec<EntryCapture>,
    sources: Vec<(TypeId, CapturedValue)>,
    locations: Vec<(TypeId, AddressSet)>,
    results: Vec<(usize, TypeId)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct CapturedResult {
    pub slot: usize,
    pub value: CapturedValue,
    pub roots: Option<RootSet>,
    pub derivation: Option<CapturedDerivation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::compiler_frontend::analysis::borrow_checker) enum CapturedDerivation {
    Wrapper(WrapperId),
    Roots(RootSet),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct CapturedPost {
    pub bindings: Vec<CapturedBinding>,
    pub edits: Vec<CapturedEdit>,
    pub results: Vec<CapturedResult>,
    pub uncertainty: Option<CapturedUncertainty>,
    pub result_views: Option<CorrelatedResultViews>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct CorrelatedResultViews {
    layout: Rc<ReachLayout>,
    row: u32,
    support: typed::SupportId,
    results: Vec<(usize, TypeId)>,
}

impl CorrelatedResultViews {
    fn install(
        &self,
        state: &mut BorrowState,
        mutable: &[bool],
        order: OrderKey,
        results: &mut [CapturedResult],
    ) {
        let root = state.call_row_view_id(order, self.layout.error, self.support.0, self.row);
        let mut pending = Vec::new();
        let mut seen = FxHashSet::default();
        for &(slot, ty) in &self.results {
            let Some(result) = results.iter_mut().find(|result| result.slot == slot) else {
                continue;
            };
            let view = state.alloc_child(root, FieldKey::Field(slot as u32));
            for value in &result.value.values {
                state.add_view_member(view, value.id);
            }
            pending.push((ty, view, result.value.values.clone()));
            result.value.views.push(view);
        }
        while let Some((ty, view, values)) = pending.pop() {
            for &(key, child) in &self.layout.types[&ty].children {
                if !self.layout.types[&child].allocation {
                    continue;
                }
                let mut child_graph = ResolvedPlace::from_values(&values, Vec::new());
                step_graph(state, mutable, &mut child_graph, key);
                if !child_graph.views.is_empty() {
                    for child_view in child_graph.views {
                        state.link_view(view, key, child_view);
                    }
                    continue;
                }
                // An omitted field still denotes the lazy child of this row,
                // not a field installed by another alternative after the join.
                let next_view = state.alloc_child(view, key);
                state.link_view(view, key, next_view);
                let mut children = Vec::new();
                for id in child_graph.ids {
                    state.add_view_member(next_view, id);
                    let explicit = self.layout.types[&child]
                        .children
                        .iter()
                        .any(|&(field, _)| {
                            state.has_captured_projection(id, field)
                                || state.retained_lazy_child(id, field).is_some()
                        });
                    if explicit && seen.insert((child, id)) {
                        children.push(StoredValue {
                            id,
                            writable: !child_graph.readonly,
                        });
                    }
                }
                if !children.is_empty() {
                    pending.push((child, next_view, children));
                }
            }
        }
    }
}

/// Keep independent supports separate; callers need not enumerate their product.
pub(in crate::compiler_frontend::analysis::borrow_checker) type MatchedLane = Vec<MatchedStratum>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct MatchedStratum {
    #[cfg(test)]
    pub guard: Vec<typed::Literal>,
    pub components: Vec<MatchedComponent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::compiler_frontend::analysis::borrow_checker) struct MatchedComponent {
    pub support: typed::SupportId,
    pub alternatives: Vec<CapturedPost>,
}

pub(in crate::compiler_frontend::analysis::borrow_checker) trait CapturedPosts:
    Clone
{
    fn posts(&self) -> impl Iterator<Item = &CapturedPost>;
    fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement);
    fn apply_capture(
        post: Cow<'_, Self>,
        state: &mut BorrowState,
        mutable: &[bool],
        order: OrderKey,
    ) -> (Vec<CapturedResult>, PublicationCarriers);
}

impl CapturedPosts for CapturedPost {
    fn posts(&self) -> impl Iterator<Item = &CapturedPost> {
        std::iter::once(self)
    }

    fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        CapturedPost::remap(self, state, retirement);
    }

    fn apply_capture(
        mut post: Cow<'_, Self>,
        state: &mut BorrowState,
        mutable: &[bool],
        order: OrderKey,
    ) -> (Vec<CapturedResult>, PublicationCarriers) {
        if post.has_collisions(state) {
            post.to_mut().weaken_writes();
        }
        Self::apply_post(post, state, mutable, order)
    }
}

impl CapturedPosts for MatchedLane {
    fn posts(&self) -> impl Iterator<Item = &CapturedPost> {
        self.iter()
            .flat_map(|stratum| &stratum.components)
            .flat_map(|component| &component.alternatives)
    }

    fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        for stratum in self {
            for post in stratum
                .components
                .iter_mut()
                .flat_map(|component| &mut component.alternatives)
            {
                post.remap(state, retirement);
            }
        }
    }

    fn apply_capture(
        lane: Cow<'_, Self>,
        state: &mut BorrowState,
        mutable: &[bool],
        order: OrderKey,
    ) -> (Vec<CapturedResult>, PublicationCarriers) {
        if let Cow::Borrowed(lane) = &lane
            && let [stratum] = lane.as_slice()
            && let [component] = stratum.components.as_slice()
            && let [post] = component.alternatives.as_slice()
        {
            let (mut results, receipt) =
                CapturedPost::apply_capture(Cow::Borrowed(post), state, mutable, order);
            if results.windows(2).any(|pair| pair[0].slot >= pair[1].slot) {
                // Same slot order and last-write rule as the product merge.
                results = results
                    .into_iter()
                    .map(|row| (row.slot, row))
                    .collect::<BTreeMap<_, _>>()
                    .into_values()
                    .collect();
            }
            return (results, receipt);
        }
        join_captured_alternatives(state, lane.into_owned(), |mut stratum, state| {
            weaken_product_collisions(state, &mut stratum.components);
            let mut results = BTreeMap::new();
            let mut receipt = PublicationCarriers::default();
            // The caller joins only alternatives of one support at a time.
            // Applying these disjoint edits does not require their product.
            for component in stratum.components {
                let (component_results, changes) =
                    join_captured_alternatives(state, component.alternatives, |post, state| {
                        CapturedPost::apply_post(Cow::Owned(post), state, mutable, order)
                    });
                for result in component_results {
                    results.insert(result.slot, result);
                }
                receipt.append(changes);
            }
            (results.into_values().collect(), receipt)
        })
    }
}

fn join_captured_alternatives<T>(
    state: &mut BorrowState,
    alternatives: Vec<T>,
    mut apply: impl FnMut(T, &mut BorrowState) -> (Vec<CapturedResult>, PublicationCarriers),
) -> (Vec<CapturedResult>, PublicationCarriers) {
    let mut alternatives = alternatives.into_iter();
    let Some(first) = alternatives.next() else {
        return (Vec::new(), PublicationCarriers::default());
    };
    if alternatives.len() == 0 {
        return apply(first, state);
    }
    let base = state.clone();
    let (mut results, mut receipt) = apply(first, state);
    for alternative in alternatives {
        let mut branch = base.clone();
        let (incoming, changes) = apply(alternative, &mut branch);
        state.join_in_place(&branch);
        receipt.intersect(&changes);
        for source in incoming {
            match results.binary_search_by_key(&source.slot, |result| result.slot) {
                Ok(index) => {
                    let target = &mut results[index];
                    merge_capture(&mut target.value, source.value);
                    union_optional_roots(&mut target.roots, source.roots);
                    if !same_derivation(target.derivation.as_ref(), source.derivation.as_ref()) {
                        target.derivation = None;
                    }
                }
                Err(index) => results.insert(index, source),
            }
        }
    }
    (results, receipt)
}

fn same_derivation(left: Option<&CapturedDerivation>, right: Option<&CapturedDerivation>) -> bool {
    match (left, right) {
        (Some(CapturedDerivation::Wrapper(a)), Some(CapturedDerivation::Wrapper(b))) => a == b,
        (Some(CapturedDerivation::Roots(a)), Some(CapturedDerivation::Roots(b))) => a == b,
        (None, None) => true,
        _ => false,
    }
}

#[cfg(test)]
pub(in crate::compiler_frontend::analysis::borrow_checker) fn match_typed_lane(
    input: &impl MatchInput,
    lane: &typed::Lane,
) -> Result<MatchedLane, InfrastructureFailure> {
    match_selected_lane(input, lane, None)
}

fn selected_components<'a>(
    product: &'a typed::Product,
    selection: Option<&'a [typed::SupportId]>,
) -> impl Iterator<Item = &'a typed::Component> {
    product.components().iter().filter(move |component| {
        selection.is_none_or(|supports| supports.contains(&component.support))
    })
}

fn match_selected_lane(
    input: &impl MatchInput,
    lane: &typed::Lane,
    selection: Option<&[typed::SupportId]>,
) -> Result<MatchedLane, InfrastructureFailure> {
    let mut strata = Vec::new();
    lane.visit_compatible(
        |predicate| observe_typed_guard(input, predicate),
        |path, product| {
            if let Some(presentation) = guard_presentation(input, path, &[])? {
                let mut components = capture_product(&presentation, path, product, selection)?;
                weaken_product_collisions(input.state(), &mut components);
                strata.push(MatchedStratum {
                    #[cfg(test)]
                    guard: path.to_vec(),
                    components,
                });
            }
            Ok(())
        },
    )?;
    Ok(strata)
}

fn capture_product(
    input: &GuardPresentation<'_, impl MatchInput>,
    guard: &[typed::Literal],
    product: &typed::Product,
    selection: Option<&[typed::SupportId]>,
) -> Result<Vec<MatchedComponent>, InfrastructureFailure> {
    let mut components = Vec::new();
    let mut owners = FxHashMap::default();
    for component in selected_components(product, selection) {
        let mut alternatives = Vec::with_capacity(component.alternatives.len());
        for post in &component.alternatives {
            let mut capture = |presentation: &GuardPresentation<'_, _>| {
                let mut locations = FxHashSet::default();
                let mut captured = capture_typed_post(presentation, post, |location, definite| {
                    if definite
                        && location.is_occurrence(input.state())
                        && (!locations.insert(location)
                            || owners
                                .insert(location, component.support)
                                .is_some_and(|old| old != component.support))
                    {
                        return Err(allocation_error(
                            "typed call updates collide after actual substitution",
                        ));
                    }
                    Ok(())
                })?;
                if (component.alternatives.len() > 1 || !guard.is_empty())
                    && post.coverage == typed::Coverage::Complete
                    && let Some(layout) = presentation.reach_layout()
                {
                    let results: Vec<_> = post
                        .results
                        .iter()
                        .filter_map(|(slot, value)| {
                            if !matches!(
                                &value.source,
                                typed::ValueSource::Allocation(typed::ObjectRef::Exact(_))
                            ) {
                                return None;
                            }
                            presentation.result_type(*slot).map(|ty| (*slot, ty))
                        })
                        .collect();
                    if !results.is_empty() {
                        captured.result_views = Some(CorrelatedResultViews {
                            results,
                            row: input.state().call_view_row(post),
                            support: component.support,
                            layout: Rc::clone(layout),
                        });
                    }
                }
                alternatives.push(captured);
                Ok(())
            };
            let mut keys = Vec::new();
            post.visit_entry_ports(true, &mut |port| {
                for &(test, _) in guard {
                    if let typed::Predicate::Equal(kind, left, right) = test
                        && (left == port || right == port)
                    {
                        keys.push((kind, port));
                    }
                }
            });
            keys.sort_unstable();
            keys.dedup();
            if keys.len() < 2 {
                capture(input)?;
                continue;
            }
            let choices: Vec<_> = keys
                .into_iter()
                .filter_map(|key @ (kind, port)| {
                    let (candidates, complete) = observed_candidates(input.entry(port)?, kind);
                    (complete && candidates.len() > 1).then_some((key, candidates))
                })
                .collect();
            capture_joint_post(input, guard, &choices, &mut Vec::new(), &mut capture)?;
        }
        components.push(MatchedComponent {
            support: component.support,
            alternatives,
        });
    }
    Ok(components)
}

fn capture_joint_post<I: MatchInput>(
    marginal: &GuardPresentation<'_, I>,
    guard: &[typed::Literal],
    choices: &[(IdentityPort, Vec<Candidate>)],
    fixed: &mut Vec<(IdentityPort, Candidate)>,
    capture: &mut impl FnMut(&GuardPresentation<'_, I>) -> Result<(), InfrastructureFailure>,
) -> Result<(), InfrastructureFailure> {
    let Some(((key, candidates), remaining)) = choices.split_first() else {
        return if fixed.is_empty() {
            capture(marginal)
        } else if let Some(joint) = guard_presentation(marginal.input, guard, fixed)? {
            capture(&joint)
        } else {
            Ok(())
        };
    };
    // Enumerate only identities read by this post, never the choices of an
    // independent support. The guard still checks the unobserved identities.
    for &candidate in candidates {
        fixed.push((*key, candidate));
        capture_joint_post(marginal, guard, remaining, fixed, capture)?;
        fixed.pop();
    }
    Ok(())
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
    fn local_mutable(&self) -> &[bool] {
        self.input.local_mutable()
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
                a.is_singleton().then_some(true)
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
    fixed: &[(IdentityPort, Candidate)],
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
    for &(key, candidate) in fixed {
        domains[indices[&key]]
            .1
            .retain(|value| *value == Some(candidate));
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
                            && wrapper.is_singleton() =>
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
                    if wrapper.age == WrapperAge::Current
                        && let Some(roots) = &mut entry.derivation_roots
                    {
                        if truth {
                            roots.insert(wrapper.local);
                        } else {
                            roots.remove(wrapper.local);
                        }
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
                        && wrapper.is_singleton() =>
                {
                    if wrapper.age == WrapperAge::Current
                        && value
                            .derivation_roots
                            .as_ref()
                            .is_some_and(|roots| roots.contains(wrapper.local))
                    {
                        Some(true)
                    } else {
                        value.derivation.map(|source| source == *wrapper)
                    }
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
                        ([a], [b]) if a.is_singleton() && b.is_singleton() => Some(a == b),
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
            Self::Binding(wrapper) | Self::Cell(CellId::Slot(wrapper)) => wrapper.is_singleton(),
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
        let Some((&last, earlier)) = target.wrappers.as_slice().split_last() else {
            // A direct projection or rvalue has no caller binding to rebind.
            continue;
        };
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
                let origin = capture_origin(input, route.access.derived_from.as_ref())?;
                let writable = source.writable == Some(true)
                    && route.access.restriction != Some(typed::Permission::ReadOnly);
                if source.wrappers.as_slice().is_empty()
                    && source.role == Some(typed::Role::ProjectedAlias)
                {
                    BindingEdit::Projected {
                        addresses: required(routes)?,
                        writable,
                        origin,
                    }
                } else {
                    BindingEdit::Link {
                        targets: source.wrappers.clone(),
                        coverage: source.wrapper_coverage.unwrap_or(Coverage::Partial),
                        origin,
                        writable,
                        routes,
                    }
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
        let unique = target.wrapper_coverage == Some(Coverage::Complete) && earlier.is_empty();
        let mut capture_binding = |wrapper: WrapperId, update, roots| {
            let exact = unique && wrapper.is_singleton();
            observe_write(ExactLocation::Binding(wrapper), exact)?;
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
            observe_write(ExactLocation::Cell(cell), address.definite() == Some(cell))?;
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
        let (receivers, coverage) = writable_holders(input, holder)?;
        for &receiver in &receivers {
            observe_write(
                ExactLocation::Cell(CellId::Projected(receiver, key)),
                coverage == Coverage::Complete && receivers.len() == 1,
            )?;
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
                at_most_once: *occurrence == typed::Occurrence::AtMostOnce,
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
                capture_reach_bindings(input, &reach, true)?
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
fn writable_holders(
    input: &impl MatchInput,
    entry: &EntryCapture,
) -> Result<(Vec<AllocId>, Coverage), InfrastructureFailure> {
    let coverage = entry.allocation_coverage.unwrap_or(Coverage::Partial);
    if entry.writable == Some(false) {
        return Ok((Vec::new(), coverage));
    }
    let mut holders: Vec<_> = entry
        .value
        .values
        .iter()
        .filter(|value| value.writable)
        .map(|value| value.id)
        .collect();
    if entry.value.values.is_empty() {
        holders.extend_from_slice(&entry.value.views);
        if holders.is_empty()
            && coverage == Coverage::Partial
            && let Some(address) = &entry.address
        {
            let captured =
                capture_graph_values(input.state().resolve_captured_allocs(address, None));
            holders.extend(
                captured
                    .values
                    .into_iter()
                    .filter_map(|value| value.writable.then_some(value.id)),
            );
            holders.extend(captured.views);
        }
        if holders.is_empty() && coverage == Coverage::Partial {
            return Err(unresolved_footprint_error());
        }
    }
    holders.sort_unstable();
    holders.dedup();
    Ok((holders, coverage))
}

fn resolve_writable_cell(
    input: &impl MatchInput,
    cell: &typed::CellPort,
) -> Result<AddressSet, InfrastructureFailure> {
    if let typed::CellPort::Edge(holder, selector) = cell {
        let entry = resolve_object(input, holder)?;
        let field = required(input.field(required(entry.ty)?, *selector))?;
        let (holders, coverage) = writable_holders(input, entry)?;
        let exact = matches!(&holders[..], [holder] if input.state().alloc_singular(*holder))
            && entry.value.values.iter().all(|value| value.writable);
        let address = AddressSet::from_cells(
            holders.into_iter().map(|id| CellId::Projected(id, field)),
            coverage,
        );
        return Ok(if exact {
            address.with_singleton_proof()
        } else {
            address
        });
    }
    let (address, _) = resolve_cell(input, cell)?;
    if address.cells().is_empty()
        && address.may_coverage() == Coverage::Partial
        && cell_may_write(input, cell)?
    {
        return Err(unresolved_footprint_error());
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
                required(reach.layout.types.get(&required(root.ty)?))?;
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
    let bindings = capture_reach_bindings(input, &reach, false)?;
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
            .any(|ty| layout.types[ty].identity.as_ref() == Some(expected))
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
    restrict_types: bool,
) -> Result<Vec<CapturedBinding>, InfrastructureFailure> {
    let mut edits = Vec::new();
    for root in &reach.roots {
        let ty = required(root.ty)?;
        if root.wrappers.as_slice().is_empty()
            || restrict_types
                && !reach.layout.types[&ty]
                    .identity
                    .as_ref()
                    .is_some_and(|identity| reach.bound.types.binary_search(identity).is_ok())
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
    mutable: &[bool],
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
                state.step_resolved(&mut graph, key, mutable);
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
        input.local_mutable(),
        roots,
        bound.region != typed::ReachKind::Cell,
        |ty, value| {
            if layout.types[&ty]
                .identity
                .as_ref()
                .is_some_and(|identity| bound.types.binary_search(identity).is_ok())
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
        mutable: &[bool],
        order: OrderKey,
    ) -> (PublicationCarriers, Vec<(usize, CapturedValue)>) {
        let mut opaque = OpaqueInputs {
            scope: self.layout.scope,
            ..Default::default()
        };
        visit_typed_values(
            state,
            &self.layout,
            mutable,
            self.sources,
            true,
            |ty, value| {
                state.extend_possible_dependencies(value, &mut opaque.dependencies);
                opaque
                    .inputs
                    .entry(ty)
                    .or_default()
                    .extend(value.values.iter().copied());
            },
        );
        for values in opaque.inputs.values_mut() {
            values.sort_unstable();
            values.dedup();
        }
        opaque.dependencies.sort_unstable();
        opaque.dependencies.dedup();

        let includes = |ty| {
            self.layout.types[&ty]
                .identity
                .as_ref()
                .is_some_and(|identity| self.bound.types.binary_search(identity).is_ok())
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
                    state.mark_alloc_provenance_readonly(mutable, &mut resolved);
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
                        step_graph(state, mutable, &mut graph, key);
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
                    state.publish_store(
                        &TypedStore::write_cells(
                            AddressSet::from_cells(cells, Coverage::Partial),
                            value,
                        )
                        .with_explicit_slot_payload(),
                        order,
                        None,
                    ),
                );
            }
        }
        let results = self
            .results
            .into_iter()
            .map(|(slot, ty)| {
                let mut value =
                    Self::opaque_value(&self.layout, &mut opaque, state, order, ty, &mut receipt);
                if self.layout.types[&ty].allocation && value.values.len() > 1 {
                    // A loaded opaque result is one occurrence even when its
                    // predecessor may supply any of several allocations.
                    let view = state.call_view_id(order, self.layout.error, slot as u32);
                    for member in &value.values {
                        state.add_view_member(view, member.id);
                    }
                    value.views.push(view);
                }
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
                    receipt.append(state.publish_store(
                        &TypedStore::write_projected([fresh], key, Coverage::Partial, child, false),
                        order,
                        None,
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
                        && wrapper.is_singleton() =>
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
        Some(derivation) => {
            capture_derivation(input, Some(derivation))?.map(CapturedDerivation::Wrapper)
        }
        None => transported.and_then(|source| {
            source
                .derivation_roots
                .clone()
                .map(CapturedDerivation::Roots)
                .or_else(|| source.derivation.map(CapturedDerivation::Wrapper))
        }),
    };
    Ok(CapturedResult {
        slot: 0,
        value,
        roots: transported.and_then(|source| source.roots.clone()),
        derivation,
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
        mut edit: Cow<'_, Self>,
        state: &mut BorrowState,
        order: OrderKey,
        possible: bool,
    ) -> PublicationCarriers {
        let owned = matches!(&edit, Cow::Owned(_));
        if let Cow::Owned(Self::Family { receivers, .. }) = &mut edit {
            receivers.sort_unstable();
            receivers.dedup();
        }
        match edit.as_ref() {
            Self::Cell(store) if !possible => state.publish_store(store, order, None),
            Self::Cell(_) => {
                let Self::Cell(store) = edit.into_owned() else {
                    unreachable!()
                };
                state.publish_store(&store.without_kill(), order, None)
            }
            Self::Family {
                receivers,
                coverage,
                key,
                replace,
                members,
            } => {
                let mut canonical;
                let receivers = if !owned && receivers.windows(2).any(|pair| pair[0] >= pair[1]) {
                    canonical = receivers.clone();
                    canonical.sort_unstable();
                    canonical.dedup();
                    &canonical
                } else {
                    receivers
                };
                state.publish_family_members(
                    receivers,
                    if possible {
                        Coverage::Partial
                    } else {
                        *coverage
                    },
                    *key,
                    *replace,
                    members,
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
        if self.has_collisions(state) {
            self.weaken_writes();
        }
    }

    fn has_collisions(&self, state: &BorrowState) -> bool {
        let mut bindings = FxHashSet::default();
        let duplicate = self
            .bindings
            .iter()
            .any(|binding| !bindings.insert(binding.target));
        if duplicate {
            return true;
        }
        let writes: Vec<_> = self
            .write_footprints(false)
            .map(|write| (0, write))
            .collect();
        !overlapping_owners(state, &writes, |_, _| true).is_empty()
    }

    fn weaken_writes(&mut self) {
        for binding in &mut self.bindings {
            binding.replace = Replacement::May;
        }
        for edit in &mut self.edits {
            match edit {
                CapturedEdit::Cell(store) => store.weaken(),
                CapturedEdit::Family {
                    coverage, members, ..
                } => {
                    *coverage = Coverage::Partial;
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
                    if let Some(address) = &mut root.address {
                        address.remap_allocations(state, retirement);
                    }
                    root.value.remap(state, retirement);
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

    #[cfg(test)]
    pub(in crate::compiler_frontend::analysis::borrow_checker) fn apply(
        self,
        state: &mut BorrowState,
        mutable: &[bool],
        order: OrderKey,
    ) -> (Vec<CapturedResult>, PublicationCarriers) {
        Self::apply_post(Cow::Owned(self), state, mutable, order)
    }

    fn apply_post(
        post: Cow<'_, Self>,
        state: &mut BorrowState,
        mutable: &[bool],
        order: OrderKey,
    ) -> (Vec<CapturedResult>, PublicationCarriers) {
        let (bindings, edits, mut results, uncertainty, result_views) = match post {
            Cow::Owned(post) => (
                Cow::Owned(post.bindings),
                Cow::Owned(post.edits),
                post.results,
                post.uncertainty.map(Cow::Owned),
                post.result_views.map(Cow::Owned),
            ),
            Cow::Borrowed(post) => (
                Cow::Borrowed(post.bindings.as_slice()),
                Cow::Borrowed(post.edits.as_slice()),
                post.results.clone(),
                post.uncertainty.as_ref().map(Cow::Borrowed),
                post.result_views.as_ref().map(Cow::Borrowed),
            ),
        };
        let mut receipt = PublicationCarriers::default();
        let links = SmallSet::from_iter(bindings.iter().filter_map(|binding| {
            (binding.replace == Replacement::Must
                && matches!(&binding.update, BindingEdit::Link { .. }))
            .then_some(binding.target)
        }));
        for edit in capture_rows(bindings) {
            receipt.append(edit.into_owned().apply(state, false));
        }
        for &wrapper in links.as_slice() {
            state.refresh_captured_link_derivation(wrapper);
        }
        for edit in capture_rows(edits) {
            receipt.append(CapturedEdit::apply(edit, state, order, false));
        }
        if let Some(unknown) = uncertainty {
            // Unknown suffixes operate on the prefix's state, and cannot
            // establish a definite replacement of their own.
            let (bindings, edits, mut suffix_results, reach) = match unknown {
                Cow::Owned(unknown) => (
                    Cow::Owned(unknown.bindings),
                    Cow::Owned(unknown.edits),
                    unknown.results,
                    unknown.reach,
                ),
                Cow::Borrowed(unknown) => (
                    Cow::Borrowed(unknown.bindings.as_slice()),
                    Cow::Borrowed(unknown.edits.as_slice()),
                    unknown.results.clone(),
                    unknown.reach.clone(),
                ),
            };
            for edit in capture_rows(bindings) {
                receipt.append(edit.into_owned().apply(state, true));
            }
            for edit in capture_rows(edits) {
                receipt.append(CapturedEdit::apply(edit, state, order, true));
            }
            if let Some(reach) = reach {
                let (suffix_receipt, bounded_results) = reach.apply(state, mutable, order);
                receipt.append(suffix_receipt);
                suffix_results.extend(bounded_results);
            }
            for (slot, value) in suffix_results {
                if let Some(result) = results.iter_mut().find(|result| result.slot == slot) {
                    merge_capture(&mut result.value, value);
                    canonicalise_capture(&mut result.value);
                    result.derivation = None;
                } else {
                    results.push(CapturedResult {
                        slot,
                        value,
                        roots: None,
                        derivation: None,
                    });
                }
            }
            results.sort_unstable_by_key(|result| result.slot);
        }
        if let Some(views) = result_views {
            views.install(state, mutable, order, &mut results);
        }
        (results, receipt)
    }
}

fn capture_rows<T: Clone>(rows: Cow<'_, [T]>) -> impl Iterator<Item = Cow<'_, T>> {
    let (borrowed, owned) = match rows {
        Cow::Borrowed(rows) => (rows, Vec::new()),
        Cow::Owned(rows) => (&[][..], rows),
    };
    borrowed
        .iter()
        .map(Cow::Borrowed)
        .chain(owned.into_iter().map(Cow::Owned))
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
            (CallReturns::External(slots), InvokeLane::Success) => Self::External(slots),
            (CallReturns::External(_), InvokeLane::Error) => Self::Opaque(Inputs::Any),
            (CallReturns::Fresh, _) => Self::Opaque(Inputs::Fresh),
            (CallReturns::Summary(_), _) => unreachable!("source calls use typed continuations"),
        }
    }

    fn opaque_inputs(self, slot: usize) -> Inputs<'a> {
        match self {
            Self::Opaque(inputs) => inputs,
            Self::External(slots) => match slots.get(slot).map(|slot| &slot.alias) {
                Some(ExternalReturnAlias::AliasArgs(indices)) => Inputs::Args(indices),
                Some(ExternalReturnAlias::Fresh) => Inputs::Fresh,
                None => Inputs::Any,
            },
            Self::Lookup => unreachable!("lookups capture their member directly"),
        }
    }

    pub(super) fn retains_result(self) -> bool {
        match self {
            Self::Opaque(Inputs::Fresh) => false,
            Self::Lookup => true,
            Self::External(slots) => slots.iter().any(|slot| {
                matches!(&slot.alias, ExternalReturnAlias::AliasArgs(indices) if !indices.is_empty())
            }),
            Self::Opaque(_) => true,
        }
    }

    pub(super) fn may_retain(self, argument: usize, roots: &[RootSet]) -> bool {
        match self {
            Self::Lookup => argument == 0,
            Self::External(slots) => slots.iter().any(|slot| {
                matches!(&slot.alias, ExternalReturnAlias::AliasArgs(indices) if indices.contains(&argument))
            }),
            Self::Opaque(Inputs::Args(indices)) => indices.contains(&argument),
            Self::Opaque(Inputs::Any) => roots.get(argument).is_some_and(|root| !root.is_empty()),
            Self::Opaque(Inputs::Fresh) => false,
        }
    }

    pub(super) fn root_sources(self, slot: usize) -> Option<&'a [usize]> {
        match self {
            Self::Lookup => Some(if slot == 0 { &[0] } else { &[] }),
            _ => match self.opaque_inputs(slot) {
                Inputs::Fresh => Some(&[]),
                Inputs::Args(indices) => Some(indices),
                Inputs::Any => None,
            },
        }
    }

    pub(super) fn derive_result(
        self,
        slot: usize,
        actuals: &CallActuals<'_>,
        local_count: usize,
    ) -> RootSet {
        if matches!(self, Self::Lookup)
            && slot == 0
            && let Some(roots) = actuals.derivations.first()
        {
            return roots.clone();
        }
        RootSet::empty(local_count)
    }
}

pub(super) fn call_result_ops(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    actuals: &mut CallActuals<'_>,
    results: impl Iterator<Item = HirLocalDestination> + Clone,
    lane: CallLane<'_>,
    outcome: InvokeLane,
) -> Result<Vec<InvokeLayerOp>, BorrowCheckError> {
    let mut ops = Vec::new();
    let mut opaque = FxHashMap::default();
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

fn captured_scalar(deps: DependencySet) -> CapturedValue {
    CapturedValue {
        alternatives: vec![ValueAlt {
            kind: PayloadKind::Scalar,
            deps,
        }],
        ..Default::default()
    }
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

fn heap_type(context: &BorrowTransferContext<'_>, ty: TypeId) -> Result<bool, BorrowCheckError> {
    Ok(classify_hir_type(ty, context.type_environment)?.has_in_place_mutation_surface())
}

/// Resolve arguments on demand and share their entry values across both Invoke lanes.
pub(super) struct CallActuals<'a> {
    args: &'a [CallArgumentTransfer],
    entries: InlineVec<Option<CallActual>>,
    derivations: InlineVec<RootSet>,
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
            derivations: InlineVec::Empty,
        }
    }

    pub(super) fn arguments(&self) -> &'a [CallArgumentTransfer] {
        self.args
    }

    pub(super) fn capture_derivations(
        &mut self,
        input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    ) {
        self.derivations = self
            .args
            .iter()
            .map(|arg| {
                super::alias_derivation_roots_from_expression(
                    input.view,
                    input.state,
                    input.context.expressions,
                    arg.argument,
                )
            })
            .collect();
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
                        materialise_contents: input.stats.sparse,
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
                        materialise_contents: input.stats.sparse,
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

    fn ty(
        &self,
        input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
        index: usize,
    ) -> Result<TypeId, BorrowCheckError> {
        let argument = self.argument(input, index)?.argument;
        Ok(input.context.expressions.expression(argument).ty)
    }
}

pub(in crate::compiler_frontend::analysis::borrow_checker) fn boundary_child(
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

fn allocation_alt(value: StoredValue) -> ValueAlt {
    ValueAlt::plain(PayloadKind::Allocation(value))
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
