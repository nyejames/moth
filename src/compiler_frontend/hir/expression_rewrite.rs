//! Generic HIR expression rewriting helpers.
//!
//! WHAT: centralizes recursive expression traversal for local expression rewrites.
//! WHY: small transforms such as match-guard capture substitution should not each
//! duplicate a full `HirExpressionKind` walker.

use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirMapEntry, HirVariantField,
};
use crate::compiler_frontend::hir::places::HirPlace;

/// Rewrite an expression tree after first rewriting all child expressions.
pub(crate) fn rewrite_expression_bottom_up(
    expression: &HirExpression,
    rewrite: &mut impl FnMut(&HirExpression) -> Option<HirExpression>,
) -> HirExpression {
    let kind = match &expression.kind {
        HirExpressionKind::Int(_)
        | HirExpressionKind::Float(_)
        | HirExpressionKind::Bool(_)
        | HirExpressionKind::Char(_)
        | HirExpressionKind::StringLiteral(_)
        | HirExpressionKind::StructuralString { .. } => expression.kind.clone(),

        HirExpressionKind::Load(place) => {
            HirExpressionKind::Load(rewrite_place_expressions_bottom_up(place, rewrite))
        }
        HirExpressionKind::Copy(place) => {
            HirExpressionKind::Copy(rewrite_place_expressions_bottom_up(place, rewrite))
        }

        HirExpressionKind::BinOp { left, op, right } => HirExpressionKind::BinOp {
            left: Box::new(rewrite_expression_bottom_up(left, rewrite)),
            op: *op,
            right: Box::new(rewrite_expression_bottom_up(right, rewrite)),
        },

        HirExpressionKind::UnaryOp { op, operand } => HirExpressionKind::UnaryOp {
            op: *op,
            operand: Box::new(rewrite_expression_bottom_up(operand, rewrite)),
        },

        HirExpressionKind::StructConstruct { struct_id, fields } => {
            HirExpressionKind::StructConstruct {
                struct_id: *struct_id,
                fields: fields
                    .iter()
                    .map(|(field_id, value)| {
                        (*field_id, rewrite_expression_bottom_up(value, rewrite))
                    })
                    .collect(),
            }
        }

        HirExpressionKind::Collection(elements) => HirExpressionKind::Collection(
            elements
                .iter()
                .map(|element| rewrite_expression_bottom_up(element, rewrite))
                .collect(),
        ),

        HirExpressionKind::MapLiteral(entries) => HirExpressionKind::MapLiteral(
            entries
                .iter()
                .map(|entry| HirMapEntry {
                    key: rewrite_expression_bottom_up(&entry.key, rewrite),
                    value: rewrite_expression_bottom_up(&entry.value, rewrite),
                })
                .collect(),
        ),

        HirExpressionKind::Range { start, end } => HirExpressionKind::Range {
            start: Box::new(rewrite_expression_bottom_up(start, rewrite)),
            end: Box::new(rewrite_expression_bottom_up(end, rewrite)),
        },

        HirExpressionKind::TupleConstruct { elements } => HirExpressionKind::TupleConstruct {
            elements: elements
                .iter()
                .map(|element| rewrite_expression_bottom_up(element, rewrite))
                .collect(),
        },

        HirExpressionKind::TupleGet { tuple, index } => HirExpressionKind::TupleGet {
            tuple: Box::new(rewrite_expression_bottom_up(tuple, rewrite)),
            index: *index,
        },

        HirExpressionKind::FallibleUnwrapSuccess { result } => {
            HirExpressionKind::FallibleUnwrapSuccess {
                result: Box::new(rewrite_expression_bottom_up(result, rewrite)),
            }
        }

        HirExpressionKind::FallibleUnwrapError { result } => {
            HirExpressionKind::FallibleUnwrapError {
                result: Box::new(rewrite_expression_bottom_up(result, rewrite)),
            }
        }

        HirExpressionKind::Cast { source, policy } => HirExpressionKind::Cast {
            source: Box::new(rewrite_expression_bottom_up(source, rewrite)),
            policy: *policy,
        },

        HirExpressionKind::VariantConstruct {
            carrier,
            variant_index,
            fields,
        } => HirExpressionKind::VariantConstruct {
            carrier: carrier.clone(),
            variant_index: *variant_index,
            fields: fields
                .iter()
                .map(|field| HirVariantField {
                    name: field.name,
                    value: rewrite_expression_bottom_up(&field.value, rewrite),
                })
                .collect(),
        },

        HirExpressionKind::VariantPayloadGet {
            carrier,
            source,
            variant_index,
            field_index,
        } => HirExpressionKind::VariantPayloadGet {
            carrier: carrier.clone(),
            source: Box::new(rewrite_expression_bottom_up(source, rewrite)),
            variant_index: *variant_index,
            field_index: *field_index,
        },
    };

    let rewritten = HirExpression {
        id: expression.id,
        kind,
        ty: expression.ty,
        value_kind: expression.value_kind,
        region: expression.region,
    };

    rewrite(&rewritten).unwrap_or(rewritten)
}

fn rewrite_place_expressions_bottom_up(
    place: &HirPlace,
    rewrite: &mut impl FnMut(&HirExpression) -> Option<HirExpression>,
) -> HirPlace {
    match place {
        HirPlace::Local(_) => place.clone(),
        HirPlace::Field { base, field } => HirPlace::Field {
            base: Box::new(rewrite_place_expressions_bottom_up(base, rewrite)),
            field: *field,
        },
        HirPlace::Index { base, index } => HirPlace::Index {
            base: Box::new(rewrite_place_expressions_bottom_up(base, rewrite)),
            index: Box::new(rewrite_expression_bottom_up(index, rewrite)),
        },
    }
}
