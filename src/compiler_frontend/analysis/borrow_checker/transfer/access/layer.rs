//! Allocation-layer transfer hooks.
//!
//! WHAT: turns HIR places and values into allocation ids, records the edges constructions,
//!       field writes and collection inserts create, and checks mutations against bindings that
//!       reach the mutated allocation through those edges.
//! WHY: root provenance cannot follow a value once it is stored inside another value. These
//!      hooks run beside the root rules and only add rejections the roots cannot see, plus the
//!      precision the layer's strong updates allow.

use super::conflicts::{check_access, footprint};
use super::*;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AllocId, FieldKey, PlaceResolution, ResolvedPlace, StoreReceipt, StoredValue, ValueAlt,
    ValueKind as PayloadKind,
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
    view: OperationView<'_>,
    state: &BorrowState,
    expressions: &HirExpressionStore,
    place: &HirPlace,
) -> Option<ResolvedPlace> {
    let mut resolution = resolve_place(view, state, expressions, place)?;
    state.load_place_content(&mut resolution, view.local_mutable, false);
    let mut content = resolution.content?;
    state.mark_alloc_provenance_readonly(view.local_mutable, &mut content);
    Some(content)
}

pub(super) fn resolve_place(
    view: OperationView<'_>,
    state: &BorrowState,
    expressions: &HirExpressionStore,
    place: &HirPlace,
) -> Option<PlaceResolution> {
    Some(
        state.resolve_place_path(
            view.index_of(place.root)?,
            view.local_mutable,
            expressions
                .projections(place.projections)
                .iter()
                .map(projection_key),
        ),
    )
}

pub(super) use crate::compiler_frontend::analysis::borrow_checker::state::CapturedValue;

impl CapturedValue {
    fn fresh(id: AllocId) -> Self {
        Self::new(vec![StoredValue { id, writable: true }], Vec::new())
    }
}

pub(super) fn stored_allocations(
    local_mutable: &[bool],
    state: &BorrowState,
    resolved: ResolvedPlace,
    source_writable: bool,
) -> CapturedValue {
    let readonly = !source_writable || resolved.readonly;
    let mut value = CapturedValue {
        values: stored_allocation_values(local_mutable, state, &resolved, source_writable),
        views: resolved.views,
        alternatives: resolved.alternatives,
    };
    value.restrict_permission(readonly);
    value
}

pub(super) fn stored_allocation_values(
    local_mutable: &[bool],
    state: &BorrowState,
    resolved: &ResolvedPlace,
    source_writable: bool,
) -> Vec<StoredValue> {
    resolved
        .ids
        .iter()
        .map(|&id| StoredValue {
            id,
            writable: source_writable
                && !resolved.readonly
                && state
                    .alloc_origin_local(id)
                    .is_none_or(|origin| local_mutable[origin]),
        })
        .collect()
}

pub(super) fn isolated_projected_binding(state: &BorrowState, resolved: &PlaceResolution) -> bool {
    let (holders, _) = resolved.final_projection().expect("projected source");
    let source = resolved.route.local;
    state.binding_mode(source) == LocalMode::SLOT
        && state.binding(source).value_roots().is_empty()
        && !state.has_layer_binding(source)
        && resolved.address.definite().is_some()
        && matches!(holders.ids.as_slice(), [holder] if
            state.alloc_origin_local(*holder) == Some(source))
}

pub(super) fn capture_projected_binding(
    state: &BorrowState,
    local_mutable: &[bool],
    mut resolved: PlaceResolution,
    writable: bool,
    capture_payload: bool,
    isolated: bool,
) -> (BindingSpec, Option<CapturedValue>) {
    let source = resolved.route.local;
    let readonly_source = readonly_cell_origin(&resolved);
    let mut route_roots = state.binding(source).attribution_roots();
    route_roots.union_with(&state.effective_roots(source));
    let owner = resolved.projection_owner();
    let payload = capture_payload.then(|| {
        stored_allocations(
            local_mutable,
            state,
            resolved
                .content
                .take()
                .expect("projected capture requests content"),
            readonly_source.is_none(),
        )
    });
    (
        BindingSpec::Projected {
            addresses: resolved.address,
            origin: WrapperId::current(source),
            owner,
            writable: writable && readonly_source.is_none(),
            isolated,
            route_roots,
        },
        payload,
    )
}

pub(super) fn check_shared_place_access(
    check: &mut AccessCheckContext<'_, '_>,
    place: &HirPlace,
    roots: &mut RootSet,
    heap: bool,
) -> Result<(), BorrowCheckError> {
    let mut resolution = resolve_place(
        OperationView::from_layout(check.layout),
        check.state,
        check.context.expressions,
        place,
    );
    if let Some(resolution) = &resolution {
        report_place_cells(check.state, &resolution.address, roots);
    }
    if let Some(resolution) = resolution.as_mut().filter(|_| heap) {
        check
            .state
            .load_place_content(resolution, &check.layout.local_mutable, false);
    }
    let access = resolution.as_ref().map_or_else(
        || {
            AccessFootprint::cells(
                AddressSet::from_cells([], Coverage::Partial),
                PlaceUse::Read,
            )
        },
        |place| footprint(place, PlaceUse::Read),
    );
    check_access(check, roots, access, resolution.as_ref(), BORROW_ACCESS)
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
        HirExpressionKind::Load(_) => Ok(expression.value_kind != ValueKind::Const
            && (state.has_value_alternatives() || is_heap_value(context, expression_id)?)),
        HirExpressionKind::VariantPayloadGet { source, .. } => Ok((state.has_value_alternatives()
            || is_heap_value(context, expression_id)?)
            && needs_allocation_facts(context, state, *source)?),
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
    pub view: OperationView<'a>,
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
    let heap = is_heap_value(context, expression_id)?;
    if !heap && !state.has_value_alternatives() {
        return Ok(CapturedValue::default());
    }
    let expression = context.expressions.expression(expression_id);
    match &expression.kind {
        HirExpressionKind::Load(place) if expression.value_kind != ValueKind::Const => {
            Ok(stored_place(values, state, place, !heap))
        }
        HirExpressionKind::VariantPayloadGet {
            source,
            variant_index,
            field_index,
            ..
        } => {
            let sources = stored_values(values, state, *source, false)?;
            let key = payload_key(*variant_index, *field_index);
            let mut resolved = ResolvedPlace::from_capture(&sources);
            if !heap {
                return Ok(state.capture_scalar_projection(&resolved, key));
            }
            if resolved.ids.is_empty() {
                return Ok(CapturedValue::default());
            }
            state.step_resolved(&mut resolved, key, values.view.local_mutable);
            Ok(stored_allocations(
                values.view.local_mutable,
                state,
                resolved,
                true,
            ))
        }
        HirExpressionKind::Copy(place) if heap && state.has_alloc_edges() => {
            let Some(source) = resolve_place_allocs(values.view, state, context.expressions, place)
            else {
                return Ok(CapturedValue::default());
            };
            Ok(capture_copy(
                values,
                state,
                expression_id,
                Some(&source),
                explicit_fresh,
            ))
        }
        kind if kind.is_aggregate_construction() => {
            if !explicit_fresh && !needs_allocation_facts(context, state, expression_id)? {
                return Ok(CapturedValue::default());
            }
            let site = state.site_id(
                expression_id.0,
                values.block_id,
                values.view.block_successors,
            );
            capture_construction(values, state, expression_id)?.apply(state, site);
            Ok(CapturedValue::fresh(site))
        }
        _ if heap && explicit_fresh => Ok(CapturedValue::fresh(state.site_id(
            expression_id.0,
            values.block_id,
            values.view.block_successors,
        ))),
        _ => Ok(CapturedValue::default()),
    }
}

pub(super) fn capture_copy(
    values: &LayerValueContext<'_, '_>,
    state: &mut BorrowState,
    expression: HirValueId,
    source: Option<&ResolvedPlace>,
    explicit_fresh: bool,
) -> CapturedValue {
    let site = state.site_id(expression.0, values.block_id, values.view.block_successors);
    let copied = source.map_or_else(Vec::new, |source| state.copy_alloc_graph(source, site));
    if copied.is_empty() && explicit_fresh {
        return CapturedValue::fresh(site);
    }
    CapturedValue::new(
        copied
            .into_iter()
            .map(|id| StoredValue { id, writable: true })
            .collect(),
        Vec::new(),
    )
}

fn stored_place(
    values: &LayerValueContext<'_, '_>,
    state: &BorrowState,
    place: &HirPlace,
    scalar: bool,
) -> CapturedValue {
    let Some(mut resolved) = resolve_place(values.view, state, values.context.expressions, place)
    else {
        return CapturedValue::default();
    };
    state.load_place_content(&mut resolved, values.view.local_mutable, scalar);
    let captured = stored_allocations(
        values.view.local_mutable,
        state,
        resolved.content.expect("stored place requests content"),
        resolved.permission.wrapper_readonly_source.is_none(),
    );
    if scalar {
        captured.into_scalar()
    } else {
        captured
    }
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
    fn names_allocation(&self, names: &impl Fn(AllocId) -> bool) -> bool {
        self.0.iter().any(|edge| match edge {
            ConstructionEdge::Nested(_, nested) => nested.names_allocation(names),
            ConstructionEdge::Stored(_, value) => value.names_allocation(names),
        })
    }

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
                    state.record_constructed_value(holder, *key, captured, true);
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
    let track_scalars = list_elements && state.has_value_alternatives();
    for (key, child) in construction_children(context.expressions, expression_id) {
        let child_kind = &context.expressions.expression(child).kind;
        if child_kind.is_aggregate_construction() && !list_elements {
            let nested = capture_construction(values, state, child)?;
            if !nested.0.is_empty() {
                captured.0.push(ConstructionEdge::Nested(key, nested));
            }
        } else {
            let captured_value = stored_values(values, state, child, list_elements)?;
            if !captured_value.is_empty() || track_scalars && !is_heap_value(context, child)? {
                captured
                    .0
                    .push(ConstructionEdge::Stored(key, captured_value));
            }
        }
    }
    if track_scalars {
        let is_dependent = |key| {
            captured.0.iter().any(|edge| {
                matches!(edge, ConstructionEdge::Stored(field, value)
                if *field == key && !value.alternatives.is_empty())
            })
        };
        let track_keys = is_dependent(FieldKey::MapKey);
        let track_elements = is_dependent(FieldKey::Elem);
        captured.0.retain_mut(|edge| match edge {
            ConstructionEdge::Stored(key, value) if value.is_empty() => {
                let track = match key {
                    FieldKey::Elem => track_elements,
                    FieldKey::MapKey => track_keys,
                    _ => false,
                };
                if track {
                    value
                        .alternatives
                        .push(ValueAlt::plain(PayloadKind::Scalar));
                }
                track
            }
            _ => true,
        });
    }
    Ok(captured)
}

/// Sparse value facts captured before a whole-slot replacement. Locations are planned separately.
#[derive(Default)]
pub(super) struct SlotCapture {
    value: CapturedValue,
    explicit_payload: bool,
    construction: Construction,
    detachments: CapturedDetachments,
}

impl SlotCapture {
    pub(super) fn has_facts(&self) -> bool {
        self.explicit_payload
            || !self.value.is_empty()
            || !self.value.views.is_empty()
            || !self.construction.0.is_empty()
            || !self.detachments.0.is_empty()
    }

    pub(super) fn names_allocation(&self, names: &impl Fn(AllocId) -> bool) -> bool {
        self.value.names_allocation(names)
            || self.construction.names_allocation(names)
            || self.detachments.names_allocation(names)
    }

    pub(super) fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        self.value.remap(state, retirement);
        self.construction.remap(state, retirement);
        self.detachments.remap(state, retirement);
    }
}

pub(super) fn publish_slot_capture(
    state: &mut BorrowState,
    local: usize,
    observer: Option<usize>,
    capture: SlotCapture,
    roots: RootSet,
    order: OrderKey,
) -> StoreReceipt {
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
    let receipt = state.publish_store(&store, order, None);
    if !capture.construction.0.is_empty() {
        let (holders, _) = state.local_alloc_ids(local);
        for holder in holders {
            capture.construction.apply(state, holder);
        }
    }
    capture.detachments.apply(state, local);
    receipt
}

/// Detachments associated with the loaded local.
#[derive(Default)]
pub(super) struct CapturedDetachments(Vec<AllocId>);

impl CapturedDetachments {
    pub(super) fn names_allocation(&self, names: &impl Fn(AllocId) -> bool) -> bool {
        self.0.iter().copied().any(names)
    }

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
        let Some(source) = values.view.index_of(place.root) else {
            return Self::default();
        };
        Self(state.detached_holders(source).collect())
    }

    pub(super) fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        for holder in &mut self.0 {
            retirement.remap_id(state, holder);
        }
    }

    pub(super) fn apply(self, state: &mut BorrowState, local: usize) {
        if !self.0.is_empty() {
            state.bind_detached_holders(local, self.0);
        }
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
            let heap = is_heap_value(values.context, value)?;
            if heap || state.has_value_alternatives() {
                capture.value = stored_place(values, state, place, !heap);
                capture.explicit_payload = heap;
            }
            capture.detachments = CapturedDetachments::from_load(values, state, value);
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

fn readonly_cell_origin(resolved: &PlaceResolution) -> Option<usize> {
    let permission = resolved.permission;
    permission.wrapper_readonly_source.or_else(|| {
        permission.holder_readonly.then_some(
            permission
                .holder_readonly_source
                .unwrap_or(resolved.route.local),
        )
    })
}

pub(super) fn check_cell_permission(
    check: &AccessCheckContext<'_, '_>,
    resolved: &PlaceResolution,
) -> Result<(), BorrowCheckError> {
    if let Some(origin) = readonly_cell_origin(resolved) {
        return Err(immutable_cell_error(check, origin));
    }
    Ok(())
}

/// A replacement writes the cell, not the allocation it used to hold.
pub(super) fn check_place_replacement(
    check: &mut AccessCheckContext<'_, '_>,
    resolved: &PlaceResolution,
    roots: &RootSet,
) -> Result<(), BorrowCheckError> {
    check_cell_permission(check, resolved)?;
    check_access(
        check,
        roots,
        footprint(resolved, PlaceUse::Replace),
        Some(resolved),
        STORE_ACCESS,
    )
}

/// Store `value` into every holder's `key` through a collection operation.
///
/// WHY: a fresh value needs no edge while the holder's group is still lazy, but once a group
///      lists its elements every inserted value must appear so a removal can count them.
pub(super) fn needs_layer_store_capture(
    context: &BorrowTransferContext<'_>,
    state: &BorrowState,
    holders: &[AllocId],
    key: FieldKey,
    value: HirValueId,
) -> Result<bool, BorrowCheckError> {
    Ok(holders
        .iter()
        .any(|holder| state.alloc_group_is_explicit(*holder, key))
        || needs_allocation_facts(context, state, value)?)
}
