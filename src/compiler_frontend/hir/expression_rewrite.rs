//! Bottom-up rewriting of module-owned HIR expression graphs.
//!
//! Unchanged roots and side ranges stay shared. A changed edge creates a new row and, when
//! necessary, a new side range, so match-capture substitution cannot mutate another root.

use crate::compiler_frontend::hir::expression_store::{
    HirConstructionFailure, HirExpressionStore, HirProjection, HirValueRange,
};
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind};
use crate::compiler_frontend::hir::hir_side_table::HirSideTable;
use crate::compiler_frontend::hir::ids::HirValueId;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::source::SourceSpan;

/// Rewrite children before selecting an existing replacement or appending a changed parent.
///
/// The callback's replacement belongs to this same store. Returning `None` preserves the
/// original row when its children are unchanged. Literal content is never copied for that path.
pub(crate) fn rewrite_expression_bottom_up(
    store: &mut HirExpressionStore,
    side_table: &mut HirSideTable,
    expression: HirValueId,
    rewrite: &mut impl FnMut(&HirExpression) -> Option<HirValueId>,
) -> Result<HirValueId, HirConstructionFailure> {
    let row = store.expression(expression);
    if matches!(
        row.kind,
        HirExpressionKind::Uint(_)
            | HirExpressionKind::Int(_)
            | HirExpressionKind::Float(_)
            | HirExpressionKind::FixedScalar(_)
            | HirExpressionKind::Number(_)
            | HirExpressionKind::Bool(_)
            | HirExpressionKind::Char(_)
            | HirExpressionKind::StringLiteral(_)
            | HirExpressionKind::StructuralString { .. }
    ) {
        return Ok(rewrite(row).unwrap_or(expression));
    }

    // Non-literal rows contain only fixed graph descriptors, never owned child graphs.
    let mut row = row.clone();
    let changed = match &mut row.kind {
        HirExpressionKind::Load(place) | HirExpressionKind::Copy(place) => {
            rewrite_place_expressions(store, side_table, place, row.span, rewrite)?
        }
        HirExpressionKind::BinOp { left, right, .. }
        | HirExpressionKind::Range {
            start: left,
            end: right,
        } => {
            let left_changed = rewrite_edge(store, side_table, left, rewrite)?;
            let right_changed = rewrite_edge(store, side_table, right, rewrite)?;
            left_changed || right_changed
        }
        HirExpressionKind::UnaryOp { operand, .. }
        | HirExpressionKind::TupleGet { tuple: operand, .. }
        | HirExpressionKind::FallibleUnwrapSuccess { result: operand }
        | HirExpressionKind::FallibleUnwrapError { result: operand }
        | HirExpressionKind::Cast {
            source: operand, ..
        }
        | HirExpressionKind::VariantPayloadGet {
            source: operand, ..
        } => rewrite_edge(store, side_table, operand, rewrite)?,
        HirExpressionKind::Collection(elements)
        | HirExpressionKind::TupleConstruct { elements } => {
            rewrite_values(store, side_table, elements, row.span, rewrite)?
        }
        HirExpressionKind::StructConstruct { fields, .. } => {
            let original = *fields;
            let mut changed_fields = None;
            for index in 0..original.len() {
                let value = store.struct_fields(original)[index].1;
                let replacement = rewrite_expression_bottom_up(store, side_table, value, rewrite)?;
                if replacement != value {
                    let fields = changed_fields
                        .get_or_insert_with(|| store.struct_fields(original).to_vec());
                    fields[index].1 = replacement;
                }
            }
            if let Some(changed_fields) = changed_fields {
                *fields = store.append_struct_fields(&changed_fields, row.span)?;
                true
            } else {
                false
            }
        }
        HirExpressionKind::VariantConstruct { fields, .. } => {
            let original = *fields;
            let mut changed_fields = None;
            for index in 0..original.len() {
                let value = store.variant_fields(original)[index].value;
                let replacement = rewrite_expression_bottom_up(store, side_table, value, rewrite)?;
                if replacement != value {
                    let fields = changed_fields
                        .get_or_insert_with(|| store.variant_fields(original).to_vec());
                    fields[index].value = replacement;
                }
            }
            if let Some(changed_fields) = changed_fields {
                *fields = store.append_variant_fields(&changed_fields, row.span)?;
                true
            } else {
                false
            }
        }
        HirExpressionKind::MapLiteral(entries) => {
            let original = *entries;
            let mut changed_entries = None;
            for index in 0..original.len() {
                let entry = store.map_entries(original)[index];
                let key = rewrite_expression_bottom_up(store, side_table, entry.key, rewrite)?;
                let value = rewrite_expression_bottom_up(store, side_table, entry.value, rewrite)?;
                if key != entry.key || value != entry.value {
                    let entries =
                        changed_entries.get_or_insert_with(|| store.map_entries(original).to_vec());
                    entries[index].key = key;
                    entries[index].value = value;
                }
            }
            if let Some(changed_entries) = changed_entries {
                *entries = store.append_map_entries(&changed_entries, row.span)?;
                true
            } else {
                false
            }
        }
        // These leaves returned before cloning their potentially allocated value content.
        HirExpressionKind::Uint(_)
        | HirExpressionKind::Int(_)
        | HirExpressionKind::Float(_)
        | HirExpressionKind::FixedScalar(_)
        | HirExpressionKind::Number(_)
        | HirExpressionKind::Bool(_)
        | HirExpressionKind::Char(_)
        | HirExpressionKind::StringLiteral(_)
        | HirExpressionKind::StructuralString { .. } => false,
    };

    if let Some(replacement) = rewrite(&row) {
        return Ok(replacement);
    }
    if changed {
        // Replacements keep the occurrence's reversible source mapping under their new ID.
        let ast_span = side_table.value_ast_span(expression).or(row.span);
        let source_span = side_table.value_source_span(expression).or(row.span);
        let replacement = store.append_expression(row)?;
        side_table.map_value(ast_span, replacement, source_span);
        Ok(replacement)
    } else {
        Ok(expression)
    }
}

fn rewrite_edge(
    store: &mut HirExpressionStore,
    side_table: &mut HirSideTable,
    edge: &mut HirValueId,
    rewrite: &mut impl FnMut(&HirExpression) -> Option<HirValueId>,
) -> Result<bool, HirConstructionFailure> {
    let replacement = rewrite_expression_bottom_up(store, side_table, *edge, rewrite)?;
    let changed = replacement != *edge;
    *edge = replacement;
    Ok(changed)
}

fn rewrite_values(
    store: &mut HirExpressionStore,
    side_table: &mut HirSideTable,
    range: &mut HirValueRange,
    span: Option<SourceSpan>,
    rewrite: &mut impl FnMut(&HirExpression) -> Option<HirValueId>,
) -> Result<bool, HirConstructionFailure> {
    let original = *range;
    let mut changed_values = None;
    for index in 0..original.len() {
        let value = store.values(original)[index];
        let replacement = rewrite_expression_bottom_up(store, side_table, value, rewrite)?;
        if replacement != value {
            let values = changed_values.get_or_insert_with(|| store.values(original).to_vec());
            values[index] = replacement;
        }
    }
    if let Some(changed_values) = changed_values {
        *range = store.append_values(&changed_values, span)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn rewrite_place_expressions(
    store: &mut HirExpressionStore,
    side_table: &mut HirSideTable,
    place: &mut HirPlace,
    span: Option<SourceSpan>,
    rewrite: &mut impl FnMut(&HirExpression) -> Option<HirValueId>,
) -> Result<bool, HirConstructionFailure> {
    let original = place.projections;
    let mut changed_projections = None;
    for index in 0..original.len() {
        if let HirProjection::Index(value) = store.projections(original)[index] {
            let replacement = rewrite_expression_bottom_up(store, side_table, value, rewrite)?;
            if replacement != value {
                let projections =
                    changed_projections.get_or_insert_with(|| store.projections(original).to_vec());
                projections[index] = HirProjection::Index(replacement);
            }
        }
    }
    if let Some(changed_projections) = changed_projections {
        place.projections = store.append_projections(&changed_projections, span)?;
        Ok(true)
    } else {
        Ok(false)
    }
}
