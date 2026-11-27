//! Allocation-layer transfer hooks.
//!
//! WHAT: turns HIR places and values into allocation ids, records the edges constructions,
//!       field writes and collection inserts create, and checks mutations against bindings that
//!       reach the mutated allocation through those edges.
//! WHY: root provenance cannot follow a value once it is stored inside another value. These
//!      hooks run beside the root rules and only add rejections the roots cannot see, plus the
//!      precision the layer's strong updates allow.

use super::conflicts::{
    LayerConflictAccess, PlaceAccess, check_layer_conflict, check_layer_shared_access,
    check_mutable_access_with_targets, check_shared_access_with_place,
};
use super::*;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AllocId, FieldKey, MutationTarget, ResolvedPlace, StoredValue,
};
use crate::compiler_frontend::hir::expression_store::HirProjection;
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;

pub(super) fn projection_key(projection: &HirProjection) -> FieldKey {
    match projection {
        HirProjection::Field(field) => FieldKey::Field(field.0),
        HirProjection::Index(_) => FieldKey::Elem,
    }
}

/// Allocations `place` reaches, or `None` when its root is outside the layout.
pub(super) fn resolve_place_allocs(
    layout: &FunctionLayout,
    state: &BorrowState,
    expressions: &HirExpressionStore,
    place: &HirPlace,
) -> Option<ResolvedPlace> {
    let root = layout.index_of(place.root)?;
    let mut resolved = resolve_path(
        layout,
        state,
        root,
        expressions.projections(place.projections),
    );
    mark_alloc_provenance_readonly(state, &layout.local_mutable, &mut resolved);
    Some(resolved)
}

/// The path into the value and the value's own origin both constrain mutation.
pub(super) fn mark_alloc_provenance_readonly(
    state: &BorrowState,
    local_mutable: &[bool],
    resolved: &mut ResolvedPlace,
) {
    if resolved.readonly_source.is_none()
        && let Some(origin) = resolved
            .ids
            .iter()
            .filter_map(|&id| state.alloc_origin_local(id))
            .find(|&origin| !local_mutable[origin])
    {
        resolved.readonly = true;
        resolved.readonly_source = Some(origin);
    }
}

/// Retain the permission of each containing allocation. Callers either check all terminal
/// origins for mutation or record the terminal permission separately for each stored value.
fn resolve_path(
    layout: &FunctionLayout,
    state: &BorrowState,
    root: usize,
    projections: &[HirProjection],
) -> ResolvedPlace {
    let mut resolved = state.resolve_local_allocs(root);
    for projection in projections {
        mark_alloc_provenance_readonly(state, &layout.local_mutable, &mut resolved);
        state.step_resolved(
            &mut resolved,
            projection_key(projection),
            &layout.local_mutable,
        );
    }
    resolved
}

pub(super) use crate::compiler_frontend::analysis::borrow_checker::state::CapturedValue;

impl CapturedValue {
    pub(super) fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    fn fresh(id: AllocId) -> Self {
        Self {
            values: vec![StoredValue { id, writable: true }],
            views: Vec::new(),
        }
    }
}

pub(super) fn stored_allocations(
    layout: &FunctionLayout,
    state: &BorrowState,
    resolved: ResolvedPlace,
    source_writable: bool,
) -> CapturedValue {
    let values = resolved
        .ids
        .into_iter()
        .map(|id| StoredValue {
            id,
            writable: source_writable
                && !resolved.readonly
                && state
                    .alloc_origin_local(id)
                    .is_none_or(|origin| layout.local_mutable[origin]),
        })
        .collect();
    CapturedValue {
        values,
        views: resolved.views,
    }
}

/// The holders of `place`'s last field and that field, or `None` for a whole binding.
fn resolve_place_cell(
    layout: &FunctionLayout,
    state: &BorrowState,
    expressions: &HirExpressionStore,
    place: &HirPlace,
) -> Option<(ResolvedPlace, FieldKey)> {
    let root = layout.index_of(place.root)?;
    let (last, prefix) = expressions.projections(place.projections).split_last()?;
    let mut holders = resolve_path(layout, state, root, prefix);
    mark_alloc_provenance_readonly(state, &layout.local_mutable, &mut holders);
    Some((holders, projection_key(last)))
}

fn projected_addresses(state: &BorrowState, holders: &ResolvedPlace, key: FieldKey) -> AddressSet {
    let addresses = AddressSet::from_cells(
        holders
            .ids
            .iter()
            .map(|&holder| CellId::Projected(holder, key)),
        if holders.complete && !holders.ids.is_empty() {
            Coverage::Complete
        } else {
            Coverage::Partial
        },
    );
    if matches!(holders.ids.as_slice(), [holder] if state.alloc_singular(*holder)) {
        addresses.with_singleton_proof()
    } else {
        addresses
    }
}

pub(super) fn capture_projected_binding(
    check: &mut AccessCheckContext<'_, '_>,
    place: &HirPlace,
    source: usize,
    writable: bool,
    roots: &RootSet,
    capture_payload: bool,
) -> Result<(BindingSpec, Option<CapturedValue>), BorrowCheckError> {
    let (mut holders, key) =
        resolve_place_cell(check.layout, check.state, check.context.expressions, place)
            .expect("a projected source has a holder and final selector");
    let addresses = projected_addresses(check.state, &holders, key);
    let isolated = check.state.binding_mode(source) == LocalMode::SLOT
        && check.state.binding(source).value_roots().is_empty()
        && !check.state.has_layer_binding(source)
        && addresses.definite().is_some()
        && matches!(holders.ids.as_slice(), [holder] if
            check.state.alloc_origin_local(*holder) == Some(source));
    let readonly_source = check
        .state
        .resolve_cells_with_permission(source, false, &check.layout.local_mutable)
        .1
        .or_else(|| {
            holders
                .readonly
                .then_some(holders.readonly_source.unwrap_or(source))
        });
    let refine_overlap = writable && !check.state.layer_is_empty();
    if writable && let Some(origin) = readonly_source {
        return Err(immutable_cell_error(check, origin));
    }
    if capture_payload || refine_overlap {
        check
            .state
            .step_resolved(&mut holders, key, &check.layout.local_mutable);
    }
    if writable {
        check_alias_acquisition_overlap(
            check,
            source,
            &addresses,
            roots,
            refine_overlap.then_some(&holders),
        )?;
    }
    let mut route_roots = check.state.binding(source).overlap_roots();
    route_roots.union_with(&check.state.effective_roots(source));
    let payload = capture_payload.then(|| {
        stored_allocations(
            check.layout,
            check.state,
            holders,
            readonly_source.is_none(),
        )
    });
    Ok((
        BindingSpec::Projected {
            addresses,
            origin: WrapperId::current(source),
            writable: writable && readonly_source.is_none(),
            isolated,
            route_roots,
        },
        payload,
    ))
}

pub(super) fn check_alias_acquisition_overlap(
    check: &mut AccessCheckContext<'_, '_>,
    source: usize,
    addresses: &AddressSet,
    roots: &RootSet,
    payload: Option<&ResolvedPlace>,
) -> Result<(), BorrowCheckError> {
    let policy = MutableAccessPolicy {
        allow_prior_shared: true,
        require_root_mutable: false,
        strict_move_exclusivity: false,
        alias_exclusivity: AliasExclusivity::All,
    };
    if check.state.layer_is_empty() {
        return check_mutable_access(check, roots, policy);
    }
    let cells: Vec<_> = addresses
        .cells()
        .iter()
        .filter_map(|&cell| match cell {
            CellId::Projected(holder, key) => Some((holder, key)),
            CellId::Slot(_) => None,
        })
        .collect();
    // Previous contents only refine overlap, never the cell's permission.
    let targets: Vec<_> = payload
        .into_iter()
        .flat_map(|payload| {
            payload.ids.iter().map(|&id| MutationTarget {
                id,
                layer_reached: payload.layer_only,
                deep: false,
            })
        })
        .collect();
    let access = PlaceAccess {
        cells: &cells,
        values: &targets,
    };
    check_mutable_access_with_targets(
        check,
        roots,
        policy,
        (addresses.coverage == Coverage::Complete).then_some(access),
    )?;
    check_layer_conflict(check, source, access, LayerConflictAccess::ObservedValue)
}

/// A projected place keeps its address even when its value has no allocation.
fn place_cells(
    check: &AccessCheckContext<'_, '_>,
    place: &HirPlace,
    actor: usize,
) -> (Vec<(AllocId, FieldKey)>, bool) {
    let Some((holders, key)) =
        resolve_place_cell(check.layout, check.state, check.context.expressions, place)
    else {
        let binding = check.state.binding(actor);
        return (
            binding.projected_cells().collect(),
            binding.projected_coverage() == Coverage::Complete,
        );
    };
    let complete = holders.complete && !holders.ids.is_empty();
    (
        holders
            .ids
            .into_iter()
            .map(|holder| (holder, key))
            .collect(),
        complete,
    )
}

pub(super) fn check_shared_place_access(
    check: &mut AccessCheckContext<'_, '_>,
    place: &HirPlace,
    roots: &RootSet,
) -> Result<(), BorrowCheckError> {
    if check.state.layer_is_empty() {
        return check_shared_access(check, roots);
    }
    let Some(actor) = check.actor_index_hint else {
        return check_shared_access(check, roots);
    };
    let Some(resolved) =
        resolve_place_allocs(check.layout, check.state, check.context.expressions, place)
    else {
        return check_shared_access(check, roots);
    };
    let mut targets = resolved
        .ids
        .iter()
        .map(|&id| MutationTarget {
            id,
            layer_reached: resolved.layer_only,
            deep: true,
        })
        .collect::<Vec<_>>();
    targets.extend(
        check
            .state
            .alloc_readable_closure(&resolved.ids)
            .into_iter()
            .map(|id| MutationTarget {
                id,
                layer_reached: true,
                deep: true,
            }),
    );
    let (cells, complete) = place_cells(check, place, actor);
    let access = PlaceAccess {
        cells: &cells,
        values: &targets,
    };
    check_shared_access_with_place(
        check,
        roots,
        (complete && resolved.complete).then_some(access),
    )?;
    check_layer_shared_access(check, actor, access)
}

pub(super) fn is_heap_value(
    context: &BorrowTransferContext<'_>,
    expression_id: HirValueId,
) -> Result<bool, BorrowCheckError> {
    let expression = context.expressions.expression(expression_id);
    Ok(classify_hir_type(expression.ty, context.type_environment)?.has_in_place_mutation_surface())
}

/// Each child of a construction with the field it fills.
fn construction_children(
    expressions: &HirExpressionStore,
    expression_id: HirValueId,
) -> Vec<(FieldKey, HirValueId)> {
    match &expressions.expression(expression_id).kind {
        HirExpressionKind::StructConstruct { fields, .. } => expressions
            .struct_fields(*fields)
            .iter()
            .map(|(field, value)| (FieldKey::Field(field.0), *value))
            .collect(),
        HirExpressionKind::Collection(values) => expressions
            .values(*values)
            .iter()
            .map(|value| (FieldKey::Elem, *value))
            .collect(),
        HirExpressionKind::VariantConstruct {
            variant_index,
            fields,
            ..
        }
        | HirExpressionKind::RouteConstruct {
            variant_index,
            fields,
            ..
        } => expressions
            .variant_fields(*fields)
            .iter()
            .enumerate()
            .map(|(index, field)| (payload_key(*variant_index, index), field.value))
            .collect(),
        HirExpressionKind::MapLiteral(entries) => expressions
            .map_entries(*entries)
            .iter()
            .flat_map(|entry| [(FieldKey::MapKey, entry.key), (FieldKey::Elem, entry.value)])
            .collect(),
        _ => Vec::new(),
    }
}

fn payload_key(variant: usize, field: usize) -> FieldKey {
    FieldKey::Payload {
        variant: variant as u32,
        field: field as u32,
    }
}

fn is_collection_like(kind: &HirExpressionKind) -> bool {
    matches!(
        kind,
        HirExpressionKind::Collection(_) | HirExpressionKind::MapLiteral(_)
    )
}

/// Whether `expression_id` may contribute stored relationships to its containing graph.
fn needs_allocation_facts(
    context: &BorrowTransferContext<'_>,
    state: &BorrowState,
    expression_id: HirValueId,
) -> Result<bool, BorrowCheckError> {
    let expression = context.expressions.expression(expression_id);
    match &expression.kind {
        HirExpressionKind::Load(_) => {
            Ok(expression.value_kind != ValueKind::Const && is_heap_value(context, expression_id)?)
        }
        HirExpressionKind::VariantPayloadGet { source, .. } => {
            Ok(is_heap_value(context, expression_id)?
                && needs_allocation_facts(context, state, *source)?)
        }
        kind if kind.is_aggregate_construction() => {
            for (_, child) in construction_children(context.expressions, expression_id) {
                if needs_allocation_facts(context, state, child)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        HirExpressionKind::Copy(_) => Ok(state.has_alloc_edges()),
        _ => Ok(false),
    }
}

/// Shared inputs for building allocation ids of values.
pub(super) struct LayerValueContext<'a, 'module> {
    pub context: &'a BorrowTransferContext<'module>,
    pub layout: &'a FunctionLayout,
    pub block_id: BlockId,
}

/// Allocations `expression_id` contributes when stored into a holder field.
///
/// WHAT: a heap place load contributes what its place reaches, keeping the source binding's
///       permission. A construction becomes a site allocation holding its own children. A fresh
///       value contributes a site only when `explicit_fresh` asks for one.
pub(super) fn stored_values(
    values: &LayerValueContext<'_, '_>,
    state: &mut BorrowState,
    expression_id: HirValueId,
    explicit_fresh: bool,
) -> Result<CapturedValue, BorrowCheckError> {
    let context = values.context;
    if !is_heap_value(context, expression_id)? {
        return Ok(CapturedValue::default());
    }
    let expression = context.expressions.expression(expression_id);
    match &expression.kind {
        HirExpressionKind::Load(place) if expression.value_kind != ValueKind::Const => {
            Ok(stored_place(values, state, place))
        }
        HirExpressionKind::VariantPayloadGet {
            source,
            variant_index,
            field_index,
            ..
        } => {
            let sources = stored_values(values, state, *source, false)?;
            let key = payload_key(*variant_index, *field_index);
            let mut resolved = ResolvedPlace {
                ids: sources.values.iter().map(|value| value.id).collect(),
                views: sources.views,
                layer_only: false,
                readonly: sources.values.iter().any(|value| !value.writable),
                readonly_source: None,
                complete: true,
            };
            if resolved.ids.is_empty() {
                return Ok(CapturedValue::default());
            }
            state.step_resolved(&mut resolved, key, &values.layout.local_mutable);
            Ok(stored_allocations(values.layout, state, resolved, true))
        }
        HirExpressionKind::Copy(place) if state.has_alloc_edges() => {
            let Some(source) =
                resolve_place_allocs(values.layout, state, context.expressions, place)
            else {
                return Ok(CapturedValue::default());
            };
            let site = state.site_id(
                expression_id.0,
                values.block_id,
                &values.layout.block_successors,
            );
            let copied = state.copy_alloc_graph(&source, site);
            if copied.is_empty() && explicit_fresh {
                return Ok(CapturedValue::fresh(site));
            }
            Ok(CapturedValue {
                values: copied
                    .into_iter()
                    .map(|id| StoredValue { id, writable: true })
                    .collect(),
                views: Vec::new(),
            })
        }
        kind if kind.is_aggregate_construction() => {
            if !explicit_fresh && !needs_allocation_facts(context, state, expression_id)? {
                return Ok(CapturedValue::default());
            }
            let site = state.site_id(
                expression_id.0,
                values.block_id,
                &values.layout.block_successors,
            );
            capture_construction(values, state, expression_id)?.apply(state, site);
            Ok(CapturedValue::fresh(site))
        }
        _ if explicit_fresh => Ok(CapturedValue::fresh(state.site_id(
            expression_id.0,
            values.block_id,
            &values.layout.block_successors,
        ))),
        _ => Ok(CapturedValue::default()),
    }
}

fn stored_place(
    values: &LayerValueContext<'_, '_>,
    state: &BorrowState,
    place: &HirPlace,
) -> CapturedValue {
    let Some(root) = values.layout.index_of(place.root) else {
        return CapturedValue::default();
    };
    let projections = values.context.expressions.projections(place.projections);
    let (resolved, readonly_source) = if projections.is_empty() {
        let (addresses, readonly_source) =
            state.resolve_cells_with_permission(root, false, &values.layout.local_mutable);
        // The route permission applies to the whole capture; payload evidence is independent.
        (
            state.resolve_local_allocs_from_cells(root, &addresses, None),
            readonly_source,
        )
    } else {
        let resolved = resolve_path(values.layout, state, root, projections);
        let (_, readonly_source) =
            state.resolve_cells_with_permission(root, false, &values.layout.local_mutable);
        (resolved, readonly_source)
    };
    stored_allocations(values.layout, state, resolved, readonly_source.is_none())
}

/// Edges to install after the destination binding has received its new allocation. Nested
/// constructions stay lazy under the new holder; leaves name what the RHS read before the write.
#[derive(Default)]
pub(super) struct Construction(Vec<ConstructionEdge>);

enum ConstructionEdge {
    Nested(FieldKey, Construction),
    Stored(FieldKey, CapturedValue),
}

impl Construction {
    fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        for edge in &mut self.0 {
            match edge {
                ConstructionEdge::Nested(_, nested) => nested.remap(state, retirement),
                ConstructionEdge::Stored(_, value) => value.remap(state, retirement),
            }
        }
    }

    fn apply(&self, state: &mut BorrowState, holder: AllocId) {
        for edge in &self.0 {
            match edge {
                ConstructionEdge::Nested(key, nested) => {
                    let child = state.alloc_child(holder, *key);
                    nested.apply(state, child);
                }
                ConstructionEdge::Stored(key, captured) => {
                    state.record_constructed_field(
                        holder,
                        *key,
                        &captured.values,
                        &captured.views,
                        true,
                    );
                }
            }
        }
    }
}

fn capture_construction(
    values: &LayerValueContext<'_, '_>,
    state: &mut BorrowState,
    expression_id: HirValueId,
) -> Result<Construction, BorrowCheckError> {
    let context = values.context;
    let mut captured = Construction::default();
    if !needs_allocation_facts(context, state, expression_id)? {
        return Ok(captured);
    }
    // Once a collection lists references, list fresh elements too so removal can count them.
    let list_elements = is_collection_like(&context.expressions.expression(expression_id).kind);
    for (key, child) in construction_children(context.expressions, expression_id) {
        let child_kind = &context.expressions.expression(child).kind;
        if child_kind.is_aggregate_construction() && !list_elements {
            let nested = capture_construction(values, state, child)?;
            if !nested.0.is_empty() {
                captured.0.push(ConstructionEdge::Nested(key, nested));
            }
        } else {
            let captured_value = stored_values(values, state, child, list_elements)?;
            if !captured_value.is_empty() {
                captured
                    .0
                    .push(ConstructionEdge::Stored(key, captured_value));
            }
        }
    }
    Ok(captured)
}

/// Sparse value facts captured before a whole-slot replacement. Locations are planned separately.
#[derive(Default)]
pub(super) struct SlotCapture {
    value: CapturedValue,
    explicit_payload: bool,
    construction: Construction,
    legacy: CapturedLocalFacts,
}

impl SlotCapture {
    pub(super) fn has_facts(&self) -> bool {
        self.explicit_payload
            || !self.value.is_empty()
            || !self.value.views.is_empty()
            || !self.construction.0.is_empty()
            || !self.legacy.detached.is_empty()
            || !self.legacy.stability.is_empty()
    }

    pub(super) fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        self.value.remap(state, retirement);
        self.construction.remap(state, retirement);
        self.legacy.remap(state, retirement);
    }
}

pub(super) fn publish_slot_capture(
    state: &mut BorrowState,
    local: usize,
    observer: Option<usize>,
    capture: SlotCapture,
    roots: RootSet,
    order: OrderKey,
) {
    let addresses = AddressSet::from_cells(
        [CellId::Slot(WrapperId::current(local))],
        Coverage::Complete,
    )
    .with_singleton_proof();
    let mut store = TypedStore::replace_cells(addresses, capture.value).with_slot_roots(roots);
    if capture.explicit_payload {
        store = store.with_explicit_slot_payload();
    }
    if let Some(observer) = observer {
        store = store.with_actor_observation(observer);
    }
    state.publish_store(&store, order, None);
    if !capture.construction.0.is_empty() {
        let (holders, _) = state.local_alloc_ids(local);
        for holder in holders {
            capture.construction.apply(state, holder);
        }
    }
    capture.legacy.apply(state, local);
}

/// Detachments and receiver stability associated with the loaded local.
#[derive(Default)]
pub(super) struct CapturedLocalFacts {
    detached: Vec<AllocId>,
    stability: Vec<AllocId>,
}

impl CapturedLocalFacts {
    pub(super) fn from_load(
        values: &LayerValueContext<'_, '_>,
        state: &BorrowState,
        value: HirValueId,
    ) -> Self {
        let expression = values.context.expressions.expression(value);
        let HirExpressionKind::Load(place) = &expression.kind else {
            return Self::default();
        };
        if state.layer_is_empty() || expression.value_kind == ValueKind::Const {
            return Self::default();
        }
        let Some(source) = values.layout.index_of(place.root) else {
            return Self::default();
        };
        Self {
            detached: state.detached_holders(source).collect(),
            stability: state.stability_holders(source).collect(),
        }
    }

    pub(super) fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        for holder in self.detached.iter_mut().chain(&mut self.stability) {
            retirement.remap_id(state, holder);
        }
    }

    pub(super) fn apply(self, state: &mut BorrowState, local: usize) {
        if !self.detached.is_empty() {
            state.bind_detached_holders(local, self.detached);
        }
        state.add_stability_loans(local, self.stability);
    }
}

pub(super) fn capture_slot_value(
    values: &LayerValueContext<'_, '_>,
    state: &mut BorrowState,
    value: HirValueId,
) -> Result<SlotCapture, BorrowCheckError> {
    let expression = values.context.expressions.expression(value);
    let mut capture = SlotCapture::default();
    match &expression.kind {
        kind if kind.is_aggregate_construction() => {
            capture.construction = capture_construction(values, state, value)?;
        }
        HirExpressionKind::Load(place) if expression.value_kind != ValueKind::Const => {
            if is_heap_value(values.context, value)? {
                capture.value = stored_place(values, state, place);
                capture.explicit_payload = true;
            }
            capture.legacy = CapturedLocalFacts::from_load(values, state, value);
        }
        HirExpressionKind::Copy(_) | HirExpressionKind::VariantPayloadGet { .. }
            if !state.layer_is_empty() =>
        {
            capture.value = stored_values(values, state, value, false)?;
        }
        _ => {}
    }
    Ok(capture)
}

fn readonly_mutation(
    values: &LayerValueContext<'_, '_>,
    place: &HirPlace,
    resolved: &ResolvedPlace,
    span: Option<SourceSpan>,
) -> BorrowCheckError {
    let diagnostics = &values.context.diagnostics;
    let local = resolved
        .readonly_source
        .map(|index| values.layout.local_ids[index])
        .unwrap_or(place.root);
    diagnostics.invalid_mutable_access(
        diagnostics.local_place(local),
        InvalidMutableAccessReason::ImmutablePlace,
        None,
        None,
        span,
    )
}

/// Check alias acquisition and call borrowing with the same projected overlap evidence.
pub(super) fn check_mutable_place_access(
    check: &mut AccessCheckContext<'_, '_>,
    place: Option<&HirPlace>,
    roots: &RootSet,
    policy: MutableAccessPolicy,
    reaches_children: bool,
    borrow_location: bool,
) -> Result<(), BorrowCheckError> {
    let Some(place) = place else {
        return check_mutable_access(check, roots, policy);
    };
    let Some(actor) = check.layout.index_of(place.root) else {
        return check_mutable_access(check, roots, policy);
    };
    let (cells, complete) = place_cells(check, place, actor);
    let targets = layer_mutation_targets(check, place, reaches_children, borrow_location)?;
    let access = PlaceAccess {
        cells: &cells,
        values: &targets,
    };
    check_mutable_access_with_targets(check, roots, policy, complete.then_some(access))?;
    check_layer_conflict(check, actor, access, LayerConflictAccess::ObservedValue)?;
    Ok(())
}

/// Check a mutation of what `place` reaches against bindings that reach it through the layer.
///
/// WHAT: rejects a path through a value stored from an immutable binding, then any other active
///       binding that reaches a mutated allocation. `reaches_children` adds every allocation the
///       mutated values hold through writable edges, for callees that may write through fields.
pub(super) fn check_layer_place_mutation(
    check: &mut AccessCheckContext<'_, '_>,
    place: &HirPlace,
    reaches_children: bool,
    borrow_location: bool,
) -> Result<(), BorrowCheckError> {
    let Some(actor) = check.layout.index_of(place.root) else {
        return Ok(());
    };
    let targets = layer_mutation_targets(check, place, reaches_children, borrow_location)?;
    let (cells, _) = place_cells(check, place, actor);
    check_layer_conflict(
        check,
        actor,
        PlaceAccess {
            cells: &cells,
            values: &targets,
        },
        LayerConflictAccess::ObservedValue,
    )
}

fn layer_mutation_targets(
    check: &AccessCheckContext<'_, '_>,
    place: &HirPlace,
    reaches_children: bool,
    borrow_location: bool,
) -> Result<Vec<MutationTarget>, BorrowCheckError> {
    let Some(actor) = check.layout.index_of(place.root) else {
        return Ok(Vec::new());
    };
    if borrow_location
        && let Some(source) = check
            .state
            .resolve_cells_with_permission(actor, false, &check.layout.local_mutable)
            .1
    {
        return Err(immutable_cell_error(check, source));
    }
    if place.projections.is_empty() && !check.layout.local_has_in_place_mutation_surface[actor] {
        return Ok(Vec::new());
    }
    let Some(resolved) =
        resolve_place_allocs(check.layout, check.state, check.context.expressions, place)
    else {
        return Ok(Vec::new());
    };
    if resolved.readonly {
        let values = LayerValueContext {
            context: check.context,
            layout: check.layout,
            block_id: check.block_id,
        };
        return Err(readonly_mutation(
            &values,
            place,
            &resolved,
            check.span.or(check.location),
        ));
    }
    if check.state.layer_is_empty() {
        return Ok(Vec::new());
    }
    let mut targets = resolved
        .ids
        .iter()
        .map(|id| MutationTarget {
            id: *id,
            layer_reached: resolved.layer_only,
            deep: reaches_children,
        })
        .collect::<Vec<_>>();
    if reaches_children {
        targets.extend(
            check
                .state
                .alloc_writable_closure(&resolved.ids)
                .into_iter()
                .map(|id| MutationTarget {
                    id,
                    layer_reached: true,
                    deep: true,
                }),
        );
    }
    Ok(targets)
}

/// Holder targets come first; the remaining allocations only refine coarse root overlap.
fn projected_write_targets(
    state: &BorrowState,
    holders: &ResolvedPlace,
    key: FieldKey,
    field_path: bool,
) -> Vec<MutationTarget> {
    let (old_values, old_children) = if field_path {
        // An observer may name the old field's contents rather than its cell. Replacing
        // the cell does not mutate those allocations, but they can rule out disjointness.
        let old_values = state.alloc_field_ids(&holders.ids, key);
        let old_children = state.alloc_readable_closure(&old_values);
        (old_values, old_children)
    } else {
        (Vec::new(), Vec::new())
    };
    holders
        .ids
        .iter()
        .map(|&id| MutationTarget {
            id,
            layer_reached: holders.layer_only,
            deep: false,
        })
        .chain(
            old_values
                .into_iter()
                .chain(old_children)
                .map(|id| MutationTarget {
                    id,
                    layer_reached: true,
                    deep: true,
                }),
        )
        .collect()
}

/// Check a projected replacement, retaining the final cell and not its old payload.
pub(super) fn check_layer_field_write(
    check: &mut AccessCheckContext<'_, '_>,
    actor: usize,
    place: &HirPlace,
    roots: &RootSet,
) -> Result<AddressSet, BorrowCheckError> {
    if let Some(origin) = check
        .state
        .resolve_cells_with_permission(actor, false, &check.layout.local_mutable)
        .1
    {
        return Err(immutable_cell_error(check, origin));
    }
    let (holders, key) =
        resolve_place_cell(check.layout, check.state, check.context.expressions, place)
            .expect("projected assignment has a holder and final selector");
    let has_layer = !check.state.layer_is_empty();
    let field_path = has_layer
        && check
            .context
            .expressions
            .projections(place.projections)
            .iter()
            .all(|projection| matches!(projection, HirProjection::Field(_)));
    let targets = if has_layer {
        projected_write_targets(check.state, &holders, key, field_path)
    } else {
        Vec::new()
    };
    let cells: Vec<_> = if has_layer {
        holders.ids.iter().map(|&holder| (holder, key)).collect()
    } else {
        Vec::new()
    };
    check_mutable_access_with_targets(
        check,
        roots,
        MutableAccessPolicy {
            allow_prior_shared: true,
            require_root_mutable: false,
            strict_move_exclusivity: false,
            alias_exclusivity: AliasExclusivity::All,
        },
        (field_path && holders.complete && !cells.is_empty()).then(|| PlaceAccess {
            cells: &cells,
            values: &targets[holders.ids.len()..],
        }),
    )?;
    if holders.readonly {
        let values = LayerValueContext {
            context: check.context,
            layout: check.layout,
            block_id: check.block_id,
        };
        return Err(readonly_mutation(
            &values,
            place,
            &holders,
            check.span.or(check.location),
        ));
    }
    let addresses = projected_addresses(check.state, &holders, key);
    if has_layer {
        check_layer_conflict(
            check,
            actor,
            PlaceAccess {
                cells: &cells,
                values: &targets[..holders.ids.len()],
            },
            LayerConflictAccess::CapturedProjection(&addresses),
        )?;
    }
    Ok(addresses)
}

/// Store `value` into every holder's `key` through a collection operation.
///
/// WHY: a fresh value needs no edge while the holder's group is still lazy, but once a group
///      lists its elements every inserted value must appear so a removal can count them.
pub(super) fn layer_store_values(
    values: &LayerValueContext<'_, '_>,
    state: &mut BorrowState,
    holders: &[AllocId],
    key: FieldKey,
    value: HirValueId,
) -> Result<Option<CapturedValue>, BorrowCheckError> {
    let explicit = holders
        .iter()
        .any(|holder| state.alloc_group_is_explicit(*holder, key));
    if !explicit && !needs_allocation_facts(values.context, state, value)? {
        return Ok(None);
    }
    Ok(Some(stored_values(values, state, value, true)?))
}

/// A local store through a retained slot or projection checks the actual captured cells.
pub(super) fn check_captured_cell_write(
    check: &mut AccessCheckContext<'_, '_>,
    actor: usize,
    addresses: &AddressSet,
) -> Result<(), BorrowCheckError> {
    let targets: Vec<_> = addresses
        .cells()
        .iter()
        .filter_map(|&cell| match cell {
            CellId::Projected(holder, _) => Some(MutationTarget {
                id: holder,
                layer_reached: true,
                deep: false,
            }),
            CellId::Slot(_) => None,
        })
        .collect();
    check_layer_conflict(
        check,
        actor,
        PlaceAccess {
            cells: &[],
            values: &targets,
        },
        LayerConflictAccess::CapturedBinding(addresses),
    )
}
