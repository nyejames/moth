//! Allocation-layer transfer hooks.
//!
//! WHAT: turns HIR places and values into allocation ids, records the edges constructions,
//!       field writes and collection inserts create, and checks mutations against bindings that
//!       reach the mutated allocation through those edges.
//! WHY: root provenance cannot follow a value once it is stored inside another value. These
//!      hooks run beside the root rules and only add rejections the roots cannot see, plus the
//!      precision the layer's strong updates allow.

use super::conflicts::{check_layer_conflict, check_layer_shared_access};
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
    if !resolved.readonly
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

fn stored_allocations(
    layout: &FunctionLayout,
    state: &BorrowState,
    resolved: ResolvedPlace,
    source_writable: bool,
) -> Vec<StoredValue> {
    resolved
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
        .collect()
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

pub(super) fn check_layer_place_read(
    check: &AccessCheckContext<'_, '_>,
    place: &HirPlace,
) -> Result<(), BorrowCheckError> {
    if check.state.layer_is_empty() {
        return Ok(());
    }
    let Some(actor) = check.actor_index_hint else {
        return Ok(());
    };
    let Some(resolved) =
        resolve_place_allocs(check.layout, check.state, check.context.expressions, place)
    else {
        return Ok(());
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
    check_layer_shared_access(check, actor, &targets)
}

fn is_heap_value(
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
) -> Result<Vec<StoredValue>, BorrowCheckError> {
    let context = values.context;
    if !is_heap_value(context, expression_id)? {
        return Ok(Vec::new());
    }
    let expression = context.expressions.expression(expression_id);
    match &expression.kind {
        HirExpressionKind::Load(place) if expression.value_kind != ValueKind::Const => {
            let Some(root) = values.layout.index_of(place.root) else {
                return Ok(Vec::new());
            };
            let resolved = resolve_path(
                values.layout,
                state,
                root,
                context.expressions.projections(place.projections),
            );
            Ok(stored_allocations(
                values.layout,
                state,
                resolved,
                values.layout.local_mutable[root],
            ))
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
                ids: sources.iter().map(|value| value.id).collect(),
                layer_only: false,
                readonly: sources.iter().any(|value| !value.writable),
                readonly_source: None,
            };
            if resolved.ids.is_empty() {
                return Ok(Vec::new());
            }
            state.step_resolved(&mut resolved, key, &values.layout.local_mutable);
            Ok(stored_allocations(values.layout, state, resolved, true))
        }
        HirExpressionKind::Copy(place) if state.has_alloc_edges() => {
            let Some(source) =
                resolve_place_allocs(values.layout, state, context.expressions, place)
            else {
                return Ok(Vec::new());
            };
            let site = state.site_id(
                expression_id.0,
                values.block_id,
                &values.layout.block_successors,
            );
            let copied = state.copy_alloc_graph(&source.ids, site);
            if copied.is_empty() && explicit_fresh {
                return Ok(vec![StoredValue {
                    id: site,
                    writable: true,
                }]);
            }
            Ok(copied
                .into_iter()
                .map(|id| StoredValue { id, writable: true })
                .collect())
        }
        kind if kind.is_aggregate_construction() => {
            if !explicit_fresh && !needs_allocation_facts(context, state, expression_id)? {
                return Ok(Vec::new());
            }
            let site = state.site_id(
                expression_id.0,
                values.block_id,
                &values.layout.block_successors,
            );
            capture_construction(values, state, expression_id)?.apply(state, site);
            Ok(vec![StoredValue {
                id: site,
                writable: true,
            }])
        }
        _ if explicit_fresh => Ok(vec![StoredValue {
            id: state.site_id(
                expression_id.0,
                values.block_id,
                &values.layout.block_successors,
            ),
            writable: true,
        }]),
        _ => Ok(Vec::new()),
    }
}

/// Edges to install after the destination binding has received its new allocation. Nested
/// constructions stay lazy under the new holder; leaves name what the RHS read before the write.
#[derive(Default)]
pub(super) struct Construction(Vec<ConstructionEdge>);

enum ConstructionEdge {
    Nested(FieldKey, Construction),
    Stored(FieldKey, StoredValue),
}

impl Construction {
    fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        for edge in &mut self.0 {
            match edge {
                ConstructionEdge::Nested(_, nested) => nested.remap(state, retirement),
                ConstructionEdge::Stored(_, value) => retirement.remap_value(state, value),
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
                ConstructionEdge::Stored(key, value) => state.add_alloc_edge(holder, *key, *value),
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
            captured.0.extend(
                stored_values(values, state, child, list_elements)?
                    .into_iter()
                    .map(|value| ConstructionEdge::Stored(key, value)),
            );
        }
    }
    Ok(captured)
}

/// The allocation facts for a whole-binding replacement, captured before its old value retires.
pub(super) enum LocalLayerValue {
    None,
    Constructed(Construction),
    Bound {
        ids: Vec<AllocId>,
        readonly: bool,
        override_roots: bool,
        cells: Vec<(AllocId, FieldKey)>,
        complete: bool,
        detached: Vec<AllocId>,
    },
}

impl LocalLayerValue {
    pub(super) fn has_facts(&self) -> bool {
        !matches!(self, Self::None)
    }

    pub(super) fn remap(&mut self, state: &BorrowState, retirement: AllocationRetirement) {
        match self {
            Self::None => {}
            Self::Constructed(captured) => captured.remap(state, retirement),
            Self::Bound {
                ids,
                override_roots,
                cells,
                detached,
                ..
            } => {
                for id in ids {
                    let old = *id;
                    retirement.remap_id(state, id);
                    *override_roots |= old != *id;
                }
                for (holder, _) in cells {
                    retirement.remap_id(state, holder);
                }
                for holder in detached {
                    retirement.remap_id(state, holder);
                }
            }
        }
    }

    fn apply_cells(&self, state: &mut BorrowState, local: usize, strong: bool) {
        if let Self::Bound {
            cells, complete, ..
        } = self
            && !cells.is_empty()
        {
            state.bind_alias_cells(local, cells, strong, *complete);
        }
    }

    pub(super) fn apply(self, state: &mut BorrowState, local: usize) {
        self.apply_cells(state, local, true);
        match self {
            Self::None => {}
            Self::Constructed(captured) => {
                let (holders, _) = state.local_alloc_ids(local);
                debug_assert!(
                    matches!(holders.as_slice(), [holder] if state.alloc_singular(*holder))
                );
                for holder in holders {
                    captured.apply(state, holder);
                }
            }
            Self::Bound {
                ids,
                readonly,
                override_roots,
                detached,
                ..
            } => {
                if override_roots {
                    state.bind_alloc_ids(local, &ids, readonly);
                }
                if !detached.is_empty() {
                    state.bind_detached_holders(local, detached);
                }
            }
        }
    }
}

pub(super) fn capture_local_value(
    values: &LayerValueContext<'_, '_>,
    state: &mut BorrowState,
    value: HirValueId,
    mutable_alias: bool,
    span: Option<SourceSpan>,
) -> Result<LocalLayerValue, BorrowCheckError> {
    let context = values.context;
    let expression = context.expressions.expression(value);
    Ok(match &expression.kind {
        kind if kind.is_aggregate_construction() => {
            let captured = capture_construction(values, state, value)?;
            if captured.0.is_empty() {
                LocalLayerValue::None
            } else {
                LocalLayerValue::Constructed(captured)
            }
        }
        HirExpressionKind::Load(place) if expression.value_kind != ValueKind::Const => {
            if (state.layer_is_empty() && place.projections.is_empty())
                || !is_heap_value(context, value)?
            {
                return Ok(LocalLayerValue::None);
            }
            let Some(resolved) =
                resolve_place_allocs(values.layout, state, context.expressions, place)
            else {
                return Ok(LocalLayerValue::None);
            };
            if mutable_alias && resolved.readonly {
                return Err(readonly_mutation(values, place, &resolved, span));
            }
            let (cells, complete) = if mutable_alias {
                if let Some((holders, key)) =
                    resolve_place_cell(values.layout, state, context.expressions, place)
                {
                    (
                        holders
                            .ids
                            .into_iter()
                            .map(|holder| (holder, key))
                            .collect(),
                        true,
                    )
                } else if let Some(source) = values.layout.index_of(place.root) {
                    let cells = state.alias_cells(source);
                    let complete = state.alias_cell_is_definite(source, &cells);
                    (cells, complete)
                } else {
                    (Vec::new(), false)
                }
            } else {
                (Vec::new(), false)
            };
            let detached = values
                .layout
                .index_of(place.root)
                .into_iter()
                .flat_map(|source| state.detached_holders(source))
                .collect();
            LocalLayerValue::Bound {
                ids: resolved.ids,
                readonly: resolved.readonly,
                override_roots: resolved.layer_only || !place.projections.is_empty(),
                cells,
                complete,
                detached,
            }
        }
        HirExpressionKind::Copy(_) | HirExpressionKind::VariantPayloadGet { .. } => {
            if state.layer_is_empty() {
                return Ok(LocalLayerValue::None);
            }
            let stored = stored_values(values, state, value, false)?;
            if stored.is_empty() {
                LocalLayerValue::None
            } else {
                let readonly = stored.iter().any(|value| !value.writable);
                LocalLayerValue::Bound {
                    ids: stored.into_iter().map(|value| value.id).collect(),
                    readonly,
                    override_roots: true,
                    cells: Vec::new(),
                    complete: false,
                    detached: Vec::new(),
                }
            }
        }
        _ => LocalLayerValue::None,
    })
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

/// Check a mutation of what `place` reaches against bindings that reach it through the layer.
///
/// WHAT: rejects a path through a value stored from an immutable binding, then any other active
///       binding that reaches a mutated allocation. `reaches_children` adds every allocation the
///       mutated values hold through writable edges, for callees that may write through fields.
pub(super) fn check_layer_place_mutation(
    check: &mut AccessCheckContext<'_, '_>,
    place: &HirPlace,
    reaches_children: bool,
) -> Result<(), BorrowCheckError> {
    let Some(actor) = check.layout.index_of(place.root) else {
        return Ok(());
    };
    if place.projections.is_empty() && !check.layout.local_has_in_place_mutation_surface[actor] {
        return Ok(());
    }
    let Some(resolved) =
        resolve_place_allocs(check.layout, check.state, check.context.expressions, place)
    else {
        return Ok(());
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
        return Ok(());
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
    check_layer_conflict(check, actor, &targets, true)
}

/// Check a write into `key` of `holders` made through `actor`.
pub(super) fn check_layer_cell_write(
    check: &mut AccessCheckContext<'_, '_>,
    actor: usize,
    holders: &ResolvedPlace,
    place: &HirPlace,
) -> Result<(), BorrowCheckError> {
    if holders.readonly {
        let values = LayerValueContext {
            context: check.context,
            layout: check.layout,
            block_id: check.block_id,
        };
        return Err(readonly_mutation(
            &values,
            place,
            holders,
            check.span.or(check.location),
        ));
    }
    if check.state.layer_is_empty() {
        return Ok(());
    }
    let targets = holders
        .ids
        .iter()
        .map(|id| MutationTarget {
            id: *id,
            layer_reached: holders.layer_only,
            deep: false,
        })
        .collect::<Vec<_>>();
    check_layer_conflict(check, actor, &targets, true)
}

/// Apply a projected write `place = value` to the layer after its access checks.
pub(super) fn transfer_layer_field_write(
    check: &mut AccessCheckContext<'_, '_>,
    place: &HirPlace,
    value: HirValueId,
) -> Result<Option<LayerFieldWrite>, BorrowCheckError> {
    let Some(actor) = check.layout.index_of(place.root) else {
        return Ok(None);
    };
    let Some((holders, key)) =
        resolve_place_cell(check.layout, check.state, check.context.expressions, place)
    else {
        return Ok(None);
    };
    check_layer_cell_write(check, actor, &holders, place)?;
    if check.state.layer_is_empty() && !needs_allocation_facts(check.context, check.state, value)? {
        return Ok(None);
    }
    Ok(Some(LayerFieldWrite { holders, key }))
}

/// The cell a projected write stores into, resolved before the write.
pub(super) struct LayerFieldWrite {
    holders: ResolvedPlace,
    key: FieldKey,
}

impl LayerFieldWrite {
    pub(super) fn apply(
        self,
        values: &LayerValueContext<'_, '_>,
        state: &mut BorrowState,
        value: HirValueId,
    ) -> Result<(), BorrowCheckError> {
        let stored = stored_values(values, state, value, true)?;
        let strong = self.key != FieldKey::Elem
            && matches!(self.holders.ids.as_slice(), [holder] if state.alloc_singular(*holder));
        state.store_alloc_field(&self.holders.ids, self.key, &stored, strong);
        Ok(())
    }
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
) -> Result<Option<Vec<StoredValue>>, BorrowCheckError> {
    let explicit = holders
        .iter()
        .any(|holder| state.alloc_group_is_explicit(*holder, key));
    if !explicit && !needs_allocation_facts(values.context, state, value)? {
        return Ok(None);
    }
    Ok(Some(stored_values(values, state, value, true)?))
}

/// Check a write through a mutable alias of a projected place against the layer.
///
/// WHY: the write replaces a field of the holder the alias was formed from, so a binding that
///      reaches that holder through the layer observes it.
pub(super) fn transfer_layer_alias_write(
    check: &mut AccessCheckContext<'_, '_>,
    local_index: usize,
) -> Result<(), BorrowCheckError> {
    let cells = check.state.alias_cells(local_index);
    if let Some(origin) = cells
        .iter()
        .filter_map(|&(holder, _)| check.state.alloc_origin_local(holder))
        .find(|&origin| !check.layout.local_mutable[origin])
    {
        let diagnostics = &check.context.diagnostics;
        return Err(diagnostics.invalid_mutable_access(
            diagnostics.local_place(check.layout.local_ids[origin]),
            InvalidMutableAccessReason::ImmutablePlace,
            None,
            None,
            check.span.or(check.location),
        ));
    }
    let targets = cells
        .into_iter()
        .map(|(holder, _)| MutationTarget {
            id: holder,
            layer_reached: true,
            deep: false,
        })
        .collect::<Vec<_>>();
    check_layer_conflict(check, local_index, &targets, false)
}

/// A through-alias write either replaces one whole binding or stores into its possible cells.
pub(super) enum LayerAliasWrite {
    None,
    Whole(usize, LocalLayerValue),
    Cells {
        cells: Vec<(AllocId, FieldKey)>,
        exact: bool,
        stored: Vec<StoredValue>,
        incoming_alias: LocalLayerValue,
    },
}

impl LayerAliasWrite {
    pub(super) fn has_facts(&self) -> bool {
        match self {
            Self::None => false,
            Self::Whole(_, value) => value.has_facts(),
            Self::Cells { .. } => true,
        }
    }

    pub(super) fn publishes_weak_whole_ids(&self) -> bool {
        matches!(self, Self::Cells { exact: false, stored, .. } if !stored.is_empty())
    }

    pub(super) fn apply(
        self,
        state: &mut BorrowState,
        local_index: usize,
        alias_state: &LocalState,
        retirement: Option<AllocationRetirement>,
    ) {
        match self {
            Self::None => {}
            Self::Whole(cell, mut value) => {
                if let Some(retirement) = retirement {
                    value.remap(state, retirement);
                }
                value.apply(state, cell);
                if state.layer_is_empty() {
                    return;
                }
                let (ids, bound) = state.local_alloc_ids(cell);
                let readonly = state.alloc_binding_readonly(cell);
                state.bind_alloc_ids(local_index, if bound { &ids } else { &[] }, readonly);
                let detached = state.detached_holders(cell).collect::<Vec<_>>();
                if !detached.is_empty() {
                    state.bind_detached_holders(local_index, detached);
                }
            }
            Self::Cells {
                cells,
                exact,
                stored,
                incoming_alias,
            } => {
                // A joined alias may target several projected or whole cells. None of those
                // targets can drop its old contents unless this is one exact projected write.
                for (holder, key) in cells {
                    state.store_alloc_field(&[holder], key, &stored, exact);
                }
                let ids = stored.iter().map(|value| value.id).collect::<Vec<_>>();
                let readonly = stored.iter().any(|value| !value.writable);
                // Every path replaces what the writer itself observes, even when none of its
                // possible referents can be strongly updated.
                state.bind_alloc_ids(local_index, &ids, readonly);
                if !exact && !ids.is_empty() {
                    for cell in alias_state.direct_alias_roots.iter_ones() {
                        if cell != local_index
                            && state.local_state(cell).mode.contains(LocalMode::SLOT)
                        {
                            state.extend_alloc_ids(cell, &ids, readonly);
                        }
                    }
                }
                incoming_alias.apply_cells(state, local_index, false);
            }
        }
    }
}

pub(super) fn capture_layer_alias_write(
    values: &LayerValueContext<'_, '_>,
    state: &mut BorrowState,
    local_index: usize,
    alias_state: &LocalState,
    value: HirValueId,
    incoming_mutable_alias: bool,
    span: Option<SourceSpan>,
) -> Result<LayerAliasWrite, BorrowCheckError> {
    if let Some(cell) = definite_slot_referent(state, local_index, alias_state) {
        let captured = capture_local_value(values, state, value, false, span)?;
        return Ok(LayerAliasWrite::Whole(cell, captured));
    }
    let cells = state.alias_cells(local_index);
    if cells.is_empty()
        && state.layer_is_empty()
        && !needs_allocation_facts(values.context, state, value)?
    {
        return Ok(LayerAliasWrite::None);
    }
    let exact = state.exact_projected_cell(local_index, &cells);
    let stored = stored_values(values, state, value, true)?;
    let incoming_alias = if incoming_mutable_alias {
        capture_local_value(values, state, value, true, span)?
    } else {
        LocalLayerValue::None
    };
    Ok(LayerAliasWrite::Cells {
        cells,
        exact,
        stored,
        incoming_alias,
    })
}
