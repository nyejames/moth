//! Dense HIR expression-store invariants.
//!
//! WHAT: checks row identity, typed side ranges, growth, freezing and scalar remapping.
//! WHY: every durable expression edge and variable payload must stay in one module-owned store.

use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, HirCapacityResource};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::expression_rewrite::rewrite_expression_bottom_up;
use crate::compiler_frontend::hir::expression_store::{
    HirConstructionFailure, HirExpressionStore, HirExpressionStoreTestLimits, HirMapEntryRange,
    HirProjection, HirProjectionRange, HirStringPieceRange, HirStructFieldRange, HirValueRange,
    HirVariantFieldRange,
};
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirMapEntry, HirVariantField, ValueKind,
};
use crate::compiler_frontend::hir::hir_side_table::{HirLocation, HirSideTable};
use crate::compiler_frontend::hir::ids::{FieldId, HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::source::{LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

fn row(kind: HirExpressionKind) -> HirExpression {
    HirExpression {
        kind,
        ty: TypeId(0),
        value_kind: ValueKind::RValue,
        region: RegionId(0),
        span: None,
    }
}

fn assert_capacity_diagnostic<T>(
    result: Result<T, HirConstructionFailure>,
    resource: HirCapacityResource,
    span: SourceSpan,
) {
    let diagnostic = match result {
        Err(HirConstructionFailure::Diagnosed(diagnostic)) => diagnostic,
        Err(HirConstructionFailure::Infrastructure(error)) => {
            panic!("expected HIR capacity diagnostic, got infrastructure error: {error:?}")
        }
        Ok(_) => panic!("expected HIR capacity diagnostic"),
    };
    assert_eq!(diagnostic.kind.code(), "MOTH-SYNTAX-0037");
    assert_eq!(diagnostic.primary_span, Some(span));
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::CompilerCapacityExceeded { resource: actual } if actual == resource
    ));
}

fn nonempty_test_span() -> SourceSpan {
    test_span(1, 9)
}

fn test_span(source_index: usize, start: u32) -> SourceSpan {
    let mut extended = crate::compiler_frontend::source::ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(start, 4, &mut extended).expect("short test span should fit");
    SourceSpan::new(SourceId::from_index(source_index), local)
}

fn is_infrastructure<T>(result: Result<T, HirConstructionFailure>) -> bool {
    matches!(result, Err(HirConstructionFailure::Infrastructure(_)))
}

#[test]
fn module_store_issues_row_positions_including_zero() {
    let mut module = HirModule::new();
    let first = module
        .expressions
        .append_expression(row(HirExpressionKind::Int(12)))
        .expect("first row should fit");
    let second = module
        .expressions
        .append_expression(row(HirExpressionKind::Bool(true)))
        .expect("second row should fit");

    assert_eq!(first, HirValueId(0));
    assert_eq!(second, HirValueId(1));
    assert!(matches!(
        &module.expressions.expression(first).kind,
        HirExpressionKind::Int(12)
    ));
    assert!(matches!(
        &module.expressions.expression(second).kind,
        HirExpressionKind::Bool(true)
    ));
}

#[test]
fn typed_ranges_read_empty_and_nonempty_payloads() {
    let mut store = HirExpressionStore::default();
    let child = store
        .append_expression(row(HirExpressionKind::Int(7)))
        .expect("expression row should fit");

    assert!(store.values(HirValueRange::empty()).is_empty());
    let empty_struct_fields = store.append_struct_fields(&[], None).unwrap();
    assert!(store.struct_fields(empty_struct_fields).is_empty());
    assert!(
        store
            .variant_fields(HirVariantFieldRange::empty())
            .is_empty()
    );
    let empty_map_entries = store.append_map_entries(&[], None).unwrap();
    assert!(store.map_entries(empty_map_entries).is_empty());
    let empty_string_pieces = store.append_string_pieces(&[], None).unwrap();
    assert!(store.string_pieces(empty_string_pieces).is_empty());
    assert!(store.projections(HirProjectionRange::empty()).is_empty());

    let value_range = store
        .append_values(&[child], None)
        .expect("value range should fit");
    let field_range = store
        .append_struct_fields(&[(FieldId(3), child)], None)
        .expect("field range should fit");
    let variant_range = store
        .append_variant_fields(
            &[HirVariantField {
                name: Some(StringId::from_index(4)),
                value: child,
            }],
            None,
        )
        .expect("variant range should fit");
    let entry_range = store
        .append_map_entries(
            &[HirMapEntry {
                key: child,
                value: child,
            }],
            None,
        )
        .expect("map entry range should fit");
    let piece_range = store
        .append_string_pieces(
            &[
                ConstStringPiece::Text(StringId::from_index(5)),
                ConstStringPiece::SiteRoot,
            ],
            None,
        )
        .expect("string piece range should fit");
    let projection_range = store
        .append_projections(
            &[
                HirProjection::Field(FieldId(6)),
                HirProjection::Index(child),
            ],
            None,
        )
        .expect("projection range should fit");

    assert_eq!(store.values(value_range), &[child]);
    assert_eq!(store.struct_fields(field_range), &[(FieldId(3), child)]);
    assert_eq!(store.variant_fields(variant_range)[0].value, child);
    assert_eq!(store.map_entries(entry_range)[0].key, child);
    assert_eq!(store.map_entries(entry_range)[0].value, child);
    assert_eq!(
        store.string_pieces(piece_range),
        &[
            ConstStringPiece::Text(StringId::from_index(5)),
            ConstStringPiece::SiteRoot
        ]
    );
    assert_eq!(
        store.projections(projection_range),
        &[
            HirProjection::Field(FieldId(6)),
            HirProjection::Index(child),
        ]
    );
}

#[test]
fn growth_preserves_row_ids_and_previous_ranges() {
    let mut store = HirExpressionStore::with_capacity(1, 1);
    let first = store
        .append_expression(row(HirExpressionKind::Int(1)))
        .expect("first row should fit");
    let first_values = store
        .append_values(&[first], None)
        .expect("first value range should fit");
    let first_fields = store
        .append_struct_fields(&[(FieldId(1), first)], None)
        .expect("first struct field range should fit");
    let first_variant_fields = store
        .append_variant_fields(
            &[HirVariantField {
                name: Some(StringId::from_index(1)),
                value: first,
            }],
            None,
        )
        .expect("first variant field range should fit");
    let first_map_entries = store
        .append_map_entries(
            &[HirMapEntry {
                key: first,
                value: first,
            }],
            None,
        )
        .expect("first map entry range should fit");
    let first_string_pieces = store
        .append_string_pieces(&[ConstStringPiece::Text(StringId::from_index(1))], None)
        .expect("first string piece range should fit");
    let first_projections = store
        .append_projections(&[HirProjection::Index(first)], None)
        .expect("first projection range should fit");

    for value in 2..=32 {
        let value_id = store
            .append_expression(row(HirExpressionKind::Int(value)))
            .expect("growth should preserve valid row IDs");
        store
            .append_values(&[value_id], None)
            .expect("growth should preserve valid value ranges");
        store
            .append_struct_fields(&[(FieldId(value as u32), value_id)], None)
            .expect("growth should preserve valid struct field ranges");
        store
            .append_variant_fields(
                &[HirVariantField {
                    name: Some(StringId::from_index(value as u32)),
                    value: value_id,
                }],
                None,
            )
            .expect("growth should preserve valid variant field ranges");
        store
            .append_map_entries(
                &[HirMapEntry {
                    key: value_id,
                    value: value_id,
                }],
                None,
            )
            .expect("growth should preserve valid map entry ranges");
        store
            .append_string_pieces(
                &[ConstStringPiece::Text(StringId::from_index(value as u32))],
                None,
            )
            .expect("growth should preserve valid string piece ranges");
        store
            .append_projections(&[HirProjection::Index(value_id)], None)
            .expect("growth should preserve valid projection ranges");
    }

    assert!(matches!(
        &store.expression(first).kind,
        HirExpressionKind::Int(1)
    ));
    assert_eq!(store.values(first_values), &[first]);
    assert_eq!(store.struct_fields(first_fields), &[(FieldId(1), first)]);
    assert_eq!(store.variant_fields(first_variant_fields)[0].value, first);
    assert_eq!(store.map_entries(first_map_entries)[0].key, first);
    assert_eq!(
        store.string_pieces(first_string_pieces),
        &[ConstStringPiece::Text(StringId::from_index(1))]
    );
    assert_eq!(
        store.projections(first_projections),
        &[HirProjection::Index(first)]
    );
    let measurements = store.measurements();
    assert!(measurements.rows.growth_events > 0);
    assert!(measurements.values.growth_events > 0);
    assert!(measurements.struct_fields.growth_events > 0);
    assert!(measurements.variant_fields.growth_events > 0);
    assert!(measurements.map_entries.growth_events > 0);
    assert!(measurements.string_pieces.growth_events > 0);
    assert!(measurements.projections.growth_events > 0);
}

#[test]
fn test_limits_reject_each_append_without_mutating_the_store() {
    let mut store = HirExpressionStore::with_test_limits(HirExpressionStoreTestLimits {
        rows: 1,
        values: 1,
        struct_fields: 1,
        variant_fields: 1,
        map_entries: 1,
        string_pieces: 1,
        projections: 1,
    });
    let child = HirValueId(0);

    let row_id = store
        .append_expression(row(HirExpressionKind::Int(0)))
        .expect("first row should fit below the test limit");
    assert_eq!(row_id, child);
    store
        .append_values(&[child], None)
        .expect("first value range should fit below the test limit");
    store
        .append_struct_fields(&[(FieldId(1), child)], None)
        .expect("first field range should fit below the test limit");
    store
        .append_variant_fields(
            &[HirVariantField {
                name: None,
                value: child,
            }],
            None,
        )
        .expect("first variant range should fit below the test limit");
    store
        .append_map_entries(
            &[HirMapEntry {
                key: child,
                value: child,
            }],
            None,
        )
        .expect("first map entry should fit below the test limit");
    store
        .append_string_pieces(&[ConstStringPiece::SiteRoot], None)
        .expect("first string piece should fit below the test limit");
    store
        .append_projections(&[HirProjection::Field(FieldId(1))], None)
        .expect("first projection should fit below the test limit");
    let before_rejected_appends = store.measurements();
    let span = nonempty_test_span();

    let mut rejected_row = row(HirExpressionKind::Int(1));
    rejected_row.span = Some(span);
    assert_capacity_diagnostic(
        store.append_expression(rejected_row),
        HirCapacityResource::ExpressionRows,
        span,
    );
    assert_capacity_diagnostic(
        store.append_values(&[child], Some(span)),
        HirCapacityResource::ValueEdges,
        span,
    );
    assert_capacity_diagnostic(
        store.append_struct_fields(&[(FieldId(1), child)], Some(span)),
        HirCapacityResource::StructFields,
        span,
    );
    assert_capacity_diagnostic(
        store.append_variant_fields(
            &[HirVariantField {
                name: None,
                value: child,
            }],
            Some(span),
        ),
        HirCapacityResource::VariantFields,
        span,
    );
    assert_capacity_diagnostic(
        store.append_map_entries(
            &[HirMapEntry {
                key: child,
                value: child,
            }],
            Some(span),
        ),
        HirCapacityResource::MapEntries,
        span,
    );
    assert_capacity_diagnostic(
        store.append_string_pieces(&[ConstStringPiece::SiteRoot], Some(span)),
        HirCapacityResource::StringPieces,
        span,
    );
    assert_capacity_diagnostic(
        store.append_projections(&[HirProjection::Field(FieldId(1))], Some(span)),
        HirCapacityResource::PlaceProjections,
        span,
    );

    assert_eq!(store.measurements(), before_rejected_appends);
}

#[test]
fn checked_side_ranges_accept_exact_u32_bounds_and_reject_overflow() {
    let store = HirExpressionStore::default();
    let span = nonempty_test_span();

    let empty_at_maximum_start: HirValueRange = store
        .checked_side_range(
            u32::MAX as usize,
            0,
            HirCapacityResource::ValueEdges,
            None,
            None,
        )
        .expect("an empty range may start at the maximum u32 coordinate");
    assert_eq!(empty_at_maximum_start.start, u32::MAX);
    assert_eq!(empty_at_maximum_start.length, 0);

    let maximum_length: HirValueRange = store
        .checked_side_range(
            0,
            u32::MAX as usize,
            HirCapacityResource::ValueEdges,
            None,
            None,
        )
        .expect("a half-open range may end exactly at the maximum u32 coordinate");
    assert_eq!(maximum_length.start, 0);
    assert_eq!(maximum_length.length, u32::MAX);

    let resource = HirCapacityResource::ValueEdges;
    assert_capacity_diagnostic(
        store.checked_side_range::<HirValueRange>(u32::MAX as usize, 1, resource, Some(span), None),
        resource,
        span,
    );
    assert_capacity_diagnostic(
        store.checked_side_range::<HirValueRange>(usize::MAX, 1, resource, Some(span), None),
        resource,
        span,
    );
}

#[test]
fn checked_range_accessors_reject_out_of_bounds_and_overflowing_ranges() {
    let mut store = HirExpressionStore::default();
    let value = store
        .append_expression(row(HirExpressionKind::Int(1)))
        .expect("expression row should fit");
    let values = store
        .append_values(&[value], None)
        .expect("value range should fit");
    let struct_fields = store
        .append_struct_fields(&[(FieldId(1), value)], None)
        .expect("struct field range should fit");
    let variant_fields = store
        .append_variant_fields(&[HirVariantField { name: None, value }], None)
        .expect("variant field range should fit");
    let map_entries = store
        .append_map_entries(&[HirMapEntry { key: value, value }], None)
        .expect("map entry range should fit");
    let string_pieces = store
        .append_string_pieces(&[ConstStringPiece::SiteRoot], None)
        .expect("string piece range should fit");
    let projections = store
        .append_projections(&[HirProjection::Field(FieldId(1))], None)
        .expect("projection range should fit");

    assert!(store.get_values(values).is_some());
    assert!(store.get_struct_fields(struct_fields).is_some());
    assert!(store.get_variant_fields(variant_fields).is_some());
    assert!(store.get_map_entries(map_entries).is_some());
    assert!(store.get_string_pieces(string_pieces).is_some());
    assert!(store.get_projections(projections).is_some());

    assert!(
        store
            .get_values(HirValueRange::from_checked_parts(1, 1))
            .is_none()
    );
    assert!(
        store
            .get_struct_fields(HirStructFieldRange::from_checked_parts(1, 1))
            .is_none()
    );
    assert!(
        store
            .get_variant_fields(HirVariantFieldRange::from_checked_parts(1, 1))
            .is_none()
    );
    assert!(
        store
            .get_map_entries(HirMapEntryRange::from_checked_parts(1, 1))
            .is_none()
    );
    assert!(
        store
            .get_string_pieces(HirStringPieceRange::from_checked_parts(1, 1))
            .is_none()
    );
    assert!(
        store
            .get_projections(HirProjectionRange::from_checked_parts(1, 1))
            .is_none()
    );

    assert!(
        store
            .get_values(HirValueRange::from_checked_parts(u32::MAX, 1))
            .is_none()
    );
    assert!(
        store
            .get_struct_fields(HirStructFieldRange::from_checked_parts(u32::MAX, 1))
            .is_none()
    );
    assert!(
        store
            .get_variant_fields(HirVariantFieldRange::from_checked_parts(u32::MAX, 1))
            .is_none()
    );
    assert!(
        store
            .get_map_entries(HirMapEntryRange::from_checked_parts(u32::MAX, 1))
            .is_none()
    );
    assert!(
        store
            .get_string_pieces(HirStringPieceRange::from_checked_parts(u32::MAX, 1))
            .is_none()
    );
    assert!(
        store
            .get_projections(HirProjectionRange::from_checked_parts(u32::MAX, 1))
            .is_none()
    );
}

#[test]
fn places_keep_flat_root_outward_field_and_index_order() {
    let mut store = HirExpressionStore::default();
    let index = store
        .append_expression(row(HirExpressionKind::Int(2)))
        .expect("index expression should fit");
    let place = HirPlace::local(LocalId(9))
        .with_field(FieldId(2), &mut store, None)
        .expect("field projection should fit")
        .with_index(index, &mut store, None)
        .expect("index projection should fit");
    let extended_place = place
        .with_field(FieldId(8), &mut store, None)
        .expect("extended field projection should fit");

    assert_eq!(
        store.projections(extended_place.projections),
        &[
            HirProjection::Field(FieldId(2)),
            HirProjection::Index(index),
            HirProjection::Field(FieldId(8)),
        ]
    );
    assert_eq!(extended_place.root, LocalId(9));
    assert_eq!(store.projections(place.projections).len(), 2);
}

#[test]
fn freeze_keeps_exact_capacity_and_rejects_every_append_family() {
    let mut store = HirExpressionStore::with_test_limits(HirExpressionStoreTestLimits {
        rows: 1,
        values: 1,
        struct_fields: 1,
        variant_fields: 1,
        map_entries: 1,
        string_pieces: 1,
        projections: 1,
    });
    let value = store
        .append_expression(row(HirExpressionKind::Int(1)))
        .expect("expression row should fit");
    let value_range = store
        .append_values(&[value], None)
        .expect("value range should fit");
    let field_range = store
        .append_struct_fields(&[(FieldId(2), value)], None)
        .expect("field range should fit");
    let variant_range = store
        .append_variant_fields(
            &[HirVariantField {
                name: Some(StringId::from_index(3)),
                value,
            }],
            None,
        )
        .expect("variant range should fit");
    let entry_range = store
        .append_map_entries(&[HirMapEntry { key: value, value }], None)
        .expect("map entry range should fit");
    let piece_range = store
        .append_string_pieces(&[ConstStringPiece::SiteRoot], None)
        .expect("string piece range should fit");
    let projection_range = store
        .append_projections(&[HirProjection::Index(value)], None)
        .expect("projection range should fit");

    store.freeze();
    assert!(store.is_frozen());
    let frozen = store.measurements();
    for buffer in [
        frozen.rows,
        frozen.values,
        frozen.struct_fields,
        frozen.variant_fields,
        frozen.map_entries,
        frozen.string_pieces,
        frozen.projections,
    ] {
        assert_eq!(buffer.retained_capacity, buffer.len);
    }
    assert_eq!(store.values(value_range), &[value]);
    assert_eq!(store.struct_fields(field_range), &[(FieldId(2), value)]);
    assert_eq!(store.variant_fields(variant_range)[0].value, value);
    assert_eq!(store.map_entries(entry_range)[0].value, value);
    assert_eq!(
        store.string_pieces(piece_range),
        &[ConstStringPiece::SiteRoot]
    );
    assert_eq!(
        store.projections(projection_range),
        &[HirProjection::Index(value)]
    );

    assert!(is_infrastructure(
        store.append_expression(row(HirExpressionKind::Bool(true)))
    ));
    assert!(is_infrastructure(store.append_values(&[value], None)));
    assert!(is_infrastructure(
        store.append_struct_fields(&[(FieldId(4), value)], None)
    ));
    assert!(is_infrastructure(store.append_variant_fields(
        &[HirVariantField { name: None, value }],
        None
    )));
    assert!(is_infrastructure(
        store.append_map_entries(&[HirMapEntry { key: value, value }], None)
    ));
    assert!(is_infrastructure(
        store.append_string_pieces(&[ConstStringPiece::SiteRoot], None)
    ));
    assert!(is_infrastructure(
        store.append_projections(&[HirProjection::Field(FieldId(4))], None)
    ));
    assert_eq!(store.measurements(), frozen);
}

#[test]
fn scalar_remapping_preserves_ids_and_ranges_before_and_after_freeze() {
    for freeze_before_remap in [false, true] {
        let mut source = StringTable::new();
        let old_name = source.intern("old-field-name");
        let mut destination = StringTable::new();
        destination.intern("destination-prefix");
        let remap = destination.merge_from(&source);
        let remapped_name = remap.get(old_name);
        assert_ne!(old_name, remapped_name);

        let mut store = HirExpressionStore::default();
        let value = store
            .append_expression(row(HirExpressionKind::Int(1)))
            .expect("expression row should fit");
        let variant_range = store
            .append_variant_fields(
                &[HirVariantField {
                    name: Some(old_name),
                    value,
                }],
                None,
            )
            .expect("variant range should fit");
        let piece_range = store
            .append_string_pieces(&[ConstStringPiece::Text(old_name)], None)
            .expect("string piece range should fit");

        if freeze_before_remap {
            store.freeze();
        }
        store.remap_string_ids(&remap);

        assert!(matches!(
            &store.expression(value).kind,
            HirExpressionKind::Int(1)
        ));
        assert_eq!(
            store.variant_fields(variant_range)[0].name,
            Some(remapped_name)
        );
        assert_eq!(
            store.string_pieces(piece_range),
            &[ConstStringPiece::Text(remapped_name)]
        );
    }
}

#[test]
fn expression_rewrites_reuse_unchanged_rows_and_allocate_changed_parents() {
    let mut store = HirExpressionStore::default();
    let mut side_table = HirSideTable::default();
    let changed_child = store
        .append_expression(row(HirExpressionKind::Int(1)))
        .expect("child row should fit");
    let unchanged_child = store
        .append_expression(row(HirExpressionKind::Int(2)))
        .expect("unchanged child should fit");
    let replacement = store
        .append_expression(row(HirExpressionKind::Int(3)))
        .expect("replacement row should fit");
    let old_elements = store
        .append_values(&[changed_child, unchanged_child], None)
        .expect("tuple children should fit");
    let ast_span = test_span(7, 3);
    let source_span = test_span(8, 21);
    assert_ne!(ast_span, source_span);
    let mut parent_row = row(HirExpressionKind::TupleConstruct {
        elements: old_elements,
    });
    parent_row.span = Some(source_span);
    let old_parent = store
        .append_expression(parent_row)
        .expect("changed parent row should fit");
    side_table.map_value(Some(ast_span), old_parent, Some(source_span));

    let before_unchanged_rewrite = store.measurements();
    let unchanged_parent =
        rewrite_expression_bottom_up(&mut store, &mut side_table, old_parent, &mut |_| None)
            .expect("unchanged rewrite should succeed");
    assert_eq!(unchanged_parent, old_parent);
    assert_eq!(store.measurements(), before_unchanged_rewrite);

    let rewritten_parent =
        rewrite_expression_bottom_up(&mut store, &mut side_table, old_parent, &mut |expression| {
            matches!(&expression.kind, HirExpressionKind::Int(1)).then_some(replacement)
        })
        .expect("changed rewrite should succeed");
    assert_ne!(old_parent, rewritten_parent);

    let HirExpressionKind::TupleConstruct { elements } = &store.expression(old_parent).kind else {
        panic!("original parent should remain unchanged");
    };
    assert_eq!(store.values(*elements), &[changed_child, unchanged_child]);
    assert_eq!(store.expression(old_parent).span, Some(source_span));

    let HirExpressionKind::TupleConstruct { elements } = &store.expression(rewritten_parent).kind
    else {
        panic!("rewritten parent should keep its typed child range");
    };
    assert_eq!(store.values(*elements), &[replacement, unchanged_child]);
    assert_eq!(store.expression(rewritten_parent).span, Some(source_span));

    for value in [old_parent, rewritten_parent] {
        assert_eq!(side_table.value_ast_span(value), Some(ast_span));
        assert_eq!(side_table.value_source_span(value), Some(source_span));
        assert!(
            side_table
                .hir_locations_for_ast(ast_span)
                .contains(&HirLocation::Value(value))
        );
    }
}

#[test]
fn metadata_replacements_reuse_unchanged_roots_and_isolate_changed_rows() {
    let mut store = HirExpressionStore::with_test_limits(HirExpressionStoreTestLimits {
        rows: 3,
        values: 1,
        ..HirExpressionStoreTestLimits::default()
    });
    let child = store
        .append_expression(row(HirExpressionKind::Int(7)))
        .unwrap();
    let elements = store.append_values(&[child], None).unwrap();
    let original_span = nonempty_test_span();
    let replacement_span = test_span(2, 17);
    let mut original_row = row(HirExpressionKind::TupleConstruct { elements });
    original_row.span = Some(original_span);
    original_row.value_kind = ValueKind::Const;
    let original = store.append_expression(original_row).unwrap();
    let before = store.measurements();

    let unchanged = store
        .copy_expression_with_metadata(original, Some(original_span), TypeId(0), RegionId(0))
        .unwrap();
    assert_eq!(unchanged, original);
    assert_eq!(store.measurements(), before);

    let replacement = store
        .copy_expression_with_metadata(original, Some(replacement_span), TypeId(1), RegionId(2))
        .unwrap();
    assert_ne!(replacement, original);
    for value in [original, replacement] {
        let expression = store.expression(value);
        assert_eq!(expression.value_kind, ValueKind::Const);
        let HirExpressionKind::TupleConstruct { elements: actual } = expression.kind else {
            panic!("metadata replacement must preserve the operation");
        };
        assert_eq!(actual, elements);
        assert_eq!(store.values(actual), &[child]);
    }
    let original_row = store.expression(original);
    assert_eq!(original_row.span, Some(original_span));
    assert_eq!(original_row.ty, TypeId(0));
    assert_eq!(original_row.region, RegionId(0));
    let replacement_row = store.expression(replacement);
    assert_eq!(replacement_row.span, Some(replacement_span));
    assert_eq!(replacement_row.ty, TypeId(1));
    assert_eq!(replacement_row.region, RegionId(2));

    let before_rejection = store.measurements();
    assert_capacity_diagnostic(
        store.copy_expression_with_metadata(
            original,
            Some(replacement_span),
            TypeId(2),
            RegionId(3),
        ),
        HirCapacityResource::ExpressionRows,
        replacement_span,
    );
    assert_eq!(store.measurements(), before_rejection);

    store.freeze();
    assert_eq!(
        store
            .copy_expression_with_metadata(original, Some(original_span), TypeId(0), RegionId(0),)
            .unwrap(),
        original
    );
    assert!(is_infrastructure(store.copy_expression_with_metadata(
        original,
        Some(replacement_span),
        TypeId(1),
        RegionId(2),
    )));
}
