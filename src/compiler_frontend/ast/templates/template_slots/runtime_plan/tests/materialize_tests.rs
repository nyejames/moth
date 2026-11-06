//! Fast-path regression for runtime slot plan materialization.
//!
//! WHAT: bounds marker retention for an application without inherited wrapper contexts.
//! WHY: extra plan markers are dead retained nodes and indicate an unnecessary
//!      wrapper-context walk.

use super::super::RuntimeSlotPlanningServices;
use super::materialize_tir_native_runtime_slot_plan;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrBuilder, TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore, TemplateIrSummary,
    TemplateViewContext, TirSlotContributions, collect_tir_slot_layout_with_context,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn undecorated_named_application_keeps_only_site_markers() {
    let mut string_table = StringTable::new();
    let title = string_table.intern("title");
    let mut store = TemplateIrStore::new();

    let mut builder = TemplateIrBuilder::new(&mut store);
    let slot = builder.push_slot_node(SlotKey::Named(title), None);
    let root = builder.push_sequence_node(vec![slot], None);
    let wrapper = builder.finish_template(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary {
            slot_count: 1,
            ..TemplateIrSummary::empty()
        },
        None,
    );
    let contribution = builder.push_text_node(
        string_table.intern("hello"),
        "hello".len(),
        TemplateSegmentOrigin::Body,
        None,
    );

    let layout =
        collect_tir_slot_layout_with_context(&store, wrapper, TemplateViewContext::default())
            .expect("the undecorated wrapper layout should collect");
    assert!(
        !layout.schema.has_inherited_wrapper_contexts,
        "a wrapper without fixed-child decorators has no inherited wrapper context"
    );

    let mut contributions = TirSlotContributions::default();
    contributions.extend_named_nodes(title, vec![contribution]);

    let expressions = AstExpressionBuilder::new().expect("expression builder");
    let const_values = ConstValueStore::default();
    let services = RuntimeSlotPlanningServices {
        string_table: &string_table,
        external_package_registry: None,
    };

    materialize_tir_native_runtime_slot_plan(
        &mut store,
        &expressions,
        &const_values,
        wrapper,
        TemplateViewContext::default(),
        &layout.schema,
        &contributions,
        &services,
    )
    .expect("an undecorated runtime named-slot application should materialize");

    // Site planning needs at most one marker. A second marker would be an
    // unreachable plan-level node retained by an unnecessary context fill.
    let marker_count = (0..store.node_count())
        .filter_map(|index| store.get_node(TemplateIrNodeId::new(index)))
        .filter(|node| {
            matches!(
                &node.kind,
                TemplateIrNodeKind::RuntimeSlotContributionSource { .. }
            )
        })
        .count();
    assert!(
        marker_count <= 1,
        "an application without inherited wrapper contexts must not retain extra markers"
    );
}
