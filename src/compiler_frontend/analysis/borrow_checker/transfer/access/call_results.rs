//! Instantiate validated return shapes, or conservatively connect compatible opaque paths.

use super::super::layer::resolve_place_allocs;
use super::super::layer::{LayerValueContext, mark_alloc_provenance_readonly, stored_values};
use super::{CallArgumentTransfer, CallTransferContext};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::metadata::CallResultFacts;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    FieldKey, ResolvedPlace, StoredValue,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::{
    BorrowTransferContext, InvokeLayerOp,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::expressions::OPTION_SOME_VARIANT_INDEX;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::hir_datatypes::classify_hir_type;
use crate::compiler_frontend::hir::statements::HirLocalDestination;
use crate::compiler_frontend::public_call_summary::FunctionReturnAliasSummary;
use rustc_hash::{FxHashMap, FxHashSet};

pub(super) fn call_result_ops(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    args: &[CallArgumentTransfer],
    results: impl Iterator<Item = HirLocalDestination> + Clone,
    coarse: &FunctionReturnAliasSummary,
    evidence: &[Option<CallResultFacts>],
) -> Result<Vec<InvokeLayerOp>, BorrowCheckError> {
    let mut ops = Vec::new();
    let mut next_site = 0;
    let mut opaque = None;
    let multiple = results.clone().nth(1).is_some();
    for (slot, dest) in results.enumerate() {
        let local = dest.local();
        let Some(index) = input.layout.index_of(local) else {
            return Err(input
                .context
                .diagnostics
                .internal_error("Missing HIR call result local"));
        };
        if !input.layout.local_has_in_place_mutation_surface[index] {
            continue;
        }
        let Some(&ty) = input.context.local_types.get(&local) else {
            return Err(input
                .context
                .diagnostics
                .internal_error("Missing HIR call result type"));
        };
        let values = if let Some(Some(facts)) = evidence.get(slot) {
            if let CallResultFacts::Parameter {
                index: argument,
                path,
            } = facts
                && path.is_empty()
                && !input
                    .state
                    .local_state(index)
                    .mode
                    .contains(super::LocalMode::ALIAS)
                && let Some(arg) = args.get(*argument)
                && preserves_plain_roots(input, index, arg)
            {
                continue;
            }
            instantiate(input, args, facts, &mut next_site, &mut ops)?
        } else {
            let opaque = match &mut opaque {
                Some(opaque) => opaque,
                None => opaque.insert(OpaqueInputs::new(input, args, coarse)?),
            };
            let mut values = opaque.values(ty);
            if opaque.has_inputs()
                || multiple
                || matches!(dest, HirLocalDestination::Update(_))
                || !heap_children(input.context, ty)?.is_empty()
            {
                values.push(opaque.add_result_type(input, ty, &mut ops)?);
            }
            values
        };
        if !values.is_empty() {
            ops.push(InvokeLayerOp::Bind {
                local,
                readonly: values.iter().any(|value| !value.writable),
                ids: values.into_iter().map(|value| value.id).collect(),
            });
        }
    }
    Ok(ops)
}

/// A direct same-root return needs no override when existing value roots already carry its
/// allocation and permission. Projected paths, alias destinations and rebinding a source
/// need the explicit capture instead.
fn preserves_plain_roots(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    destination: usize,
    arg: &CallArgumentTransfer,
) -> bool {
    let expression = input.context.expressions.expression(arg.argument);
    let HirExpressionKind::Load(place) = &expression.kind else {
        return false;
    };
    let Some(source) = input.layout.index_of(place.root) else {
        return false;
    };
    place.projections.is_empty()
        && expression.value_kind != ValueKind::Const
        && input.layout.local_mutable[source]
        && resolve_place_allocs(input.layout, input.state, input.context.expressions, place)
            .is_some_and(|resolved| {
                !resolved.layer_only
                    && !resolved.readonly
                    && resolved
                        .ids
                        .iter()
                        .all(|&id| input.state.alloc_origin_local(id) != Some(destination))
            })
}

fn heap_type(context: &BorrowTransferContext<'_>, ty: TypeId) -> Result<bool, BorrowCheckError> {
    Ok(classify_hir_type(ty, context.type_environment)?.has_in_place_mutation_surface())
}

fn instantiate(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    args: &[CallArgumentTransfer],
    facts: &CallResultFacts,
    next_site: &mut u32,
    ops: &mut Vec<InvokeLayerOp>,
) -> Result<Vec<StoredValue>, BorrowCheckError> {
    match facts {
        CallResultFacts::Parameter { index, path } => {
            let Some(arg) = args.get(*index) else {
                return Err(input
                    .context
                    .diagnostics
                    .internal_error("Typed call result names a missing argument"));
            };
            let mut values = argument_values(input, arg)?;
            for key in path {
                values = step(input, values, *key);
            }
            if values.is_empty() {
                return Err(input
                    .context
                    .diagnostics
                    .internal_error("Typed retained path has no allocation on the success lane"));
            }
            Ok(values)
        }
        CallResultFacts::Fresh { children } => {
            let id = input.state.call_site_id(
                input.current_order,
                *next_site,
                input.block_id,
                &input.layout.block_successors,
            );
            *next_site += 1;
            for (key, child) in children {
                for value in instantiate(input, args, child, next_site, ops)? {
                    ops.push(InvokeLayerOp::Contribute {
                        holder: id,
                        key: *key,
                        value,
                    });
                }
            }
            Ok(vec![StoredValue { id, writable: true }])
        }
    }
}

fn argument_values(
    input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
    arg: &CallArgumentTransfer,
) -> Result<Vec<StoredValue>, BorrowCheckError> {
    stored_values(
        &LayerValueContext {
            context: input.context,
            layout: input.layout,
            block_id: input.block_id,
        },
        input.state,
        arg.argument,
        true,
    )
}

fn step(
    input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
    values: Vec<StoredValue>,
    key: FieldKey,
) -> Vec<StoredValue> {
    let mut place = ResolvedPlace {
        readonly: values.iter().any(|value| !value.writable),
        ids: values.into_iter().map(|value| value.id).collect(),
        layer_only: false,
        readonly_source: None,
    };
    input
        .state
        .step_resolved(&mut place, key, &input.layout.local_mutable);
    mark_alloc_provenance_readonly(input.state, &input.layout.local_mutable, &mut place);
    place
        .ids
        .into_iter()
        .map(|id| StoredValue {
            id,
            writable: !place.readonly,
        })
        .collect()
}

/// Without a validated shape, a result path may contain any compatible input path or an
/// unknown allocation. Both are separate from allocations of other types. The latter is one
/// nonsingular node per call site and type; opaque results in that class may share it.
#[derive(Default)]
struct OpaqueInputs {
    inputs: FxHashMap<TypeId, Vec<StoredValue>>,
    built: FxHashSet<TypeId>,
}

impl OpaqueInputs {
    fn new(
        input: &mut CallTransferContext<'_, '_, '_, '_, '_, '_>,
        args: &[CallArgumentTransfer],
        coarse: &FunctionReturnAliasSummary,
    ) -> Result<Self, BorrowCheckError> {
        let mut this = Self::default();
        let mut active = FxHashSet::default();
        for (index, arg) in args.iter().enumerate() {
            match coarse {
                FunctionReturnAliasSummary::Fresh => break,
                FunctionReturnAliasSummary::AliasParams(indices) if !indices.contains(&index) => {
                    continue;
                }
                _ => {}
            }
            let ty = input.context.expressions.expression(arg.argument).ty;
            if heap_type(input.context, ty)? {
                let values = argument_values(input, arg)?;
                this.visit_input(input, ty, values, &mut active)?;
            }
        }
        Ok(this)
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
        mut values: Vec<StoredValue>,
        active: &mut FxHashSet<TypeId>,
    ) -> Result<(), BorrowCheckError> {
        let seen = self.inputs.entry(ty).or_default();
        values.retain(|value| {
            if seen.contains(value) {
                return false;
            }
            seen.push(*value);
            true
        });
        if values.is_empty() {
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
            self.visit_input(input, child, step(input, values.clone(), key), active)?;
        }
        active.remove(&ty);
        Ok(())
    }

    fn add_result_type(
        &mut self,
        input: &CallTransferContext<'_, '_, '_, '_, '_, '_>,
        ty: TypeId,
        ops: &mut Vec<InvokeLayerOp>,
    ) -> Result<StoredValue, BorrowCheckError> {
        let id = input.state.opaque_call_id(input.current_order, ty.0);
        if self.built.insert(ty) {
            for (key, child) in heap_children(input.context, ty)? {
                let unknown = self.add_result_type(input, child, ops)?;
                for value in self.values(child).into_iter().chain([unknown]) {
                    ops.push(InvokeLayerOp::Contribute {
                        holder: id,
                        key,
                        value,
                    });
                }
            }
        }
        Ok(StoredValue { id, writable: true })
    }
}

fn heap_children(
    context: &BorrowTransferContext<'_>,
    ty: TypeId,
) -> Result<Vec<(FieldKey, TypeId)>, BorrowCheckError> {
    let types = context.type_environment;
    let mut fields = Vec::new();
    let mut child = |key, ty| -> Result<(), BorrowCheckError> {
        if heap_type(context, ty)? {
            fields.push((key, ty));
        }
        Ok(())
    };
    if let Some(element) = types.collection_element_type(ty) {
        child(FieldKey::Elem, element)?;
    } else if let Some(map) = types.map_shape(ty) {
        child(FieldKey::MapKey, map.key_type)?;
        child(FieldKey::Elem, map.value_type)?;
    } else if let Some(inner) = types.option_inner_type(ty) {
        child(
            FieldKey::Payload {
                variant: OPTION_SOME_VARIANT_INDEX as u32,
                field: 0,
            },
            inner,
        )?;
    } else if let Some(layout) = context
        .structs
        .iter()
        .find(|layout| layout.frontend_type_id == ty)
    {
        for field in &layout.fields {
            child(FieldKey::Field(field.id.0), field.ty)?;
        }
    } else if let Some(layout) = context
        .choices
        .iter()
        .find(|layout| layout.frontend_type_id == ty)
    {
        for (variant, layout) in layout.variants.iter().enumerate() {
            for (field, definition) in layout.fields.iter().enumerate() {
                child(
                    FieldKey::Payload {
                        variant: variant as u32,
                        field: field as u32,
                    },
                    definition.ty,
                )?;
            }
        }
    }
    Ok(fields)
}
