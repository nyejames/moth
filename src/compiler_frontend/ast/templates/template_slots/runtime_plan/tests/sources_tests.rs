//! Runtime contribution source planning tests.
//!
//! WHAT: pins routed-presence handling when a slot target has no authored nodes.
//! WHY: an explicitly empty contribution must keep suppressing the occurrence
//!      default on the runtime path, which is source-driven.

use super::super::super::RuntimeSlotPlanningServices;
use super::super::types::RuntimeSlotContributionSourceDraft;
use super::build_tir_native_contribution_sources;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::templates::template::SlotKey;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrNodeKind, TemplateIrStore, TirCopyState, TirSlotContributions, TirSlotSchema,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn routed_empty_named_target_still_plans_one_presence_source() {
    let mut string_table = StringTable::new();
    let name = string_table.intern("title");
    let mut store = TemplateIrStore::new();
    let expressions = AstExpressionBuilder::new().expect("expression builder");
    let const_values = ConstValueStore::default();
    let mut copy_state = TirCopyState::new();
    let schema = TirSlotSchema {
        named_slots: [name].into_iter().collect(),
        ..TirSlotSchema::default()
    };

    // `[$in title:]` records routing presence without producing a node.
    let mut contributions = TirSlotContributions::default();
    contributions.extend_named_nodes(name, Vec::new());

    let services = RuntimeSlotPlanningServices {
        string_table: &string_table,
        external_package_registry: None,
    };
    let sources: Vec<RuntimeSlotContributionSourceDraft> = build_tir_native_contribution_sources(
        &schema,
        &contributions,
        &services,
        &mut store,
        &expressions,
        &const_values,
        &mut copy_state,
    )
    .expect("presence source planning should succeed");

    assert_eq!(sources.len(), 1, "routing presence must produce a source");
    assert_eq!(sources[0].source.target, SlotKey::Named(name));
    assert!(
        sources[0].source.renders_wrapper_unconditionally,
        "an empty presence source is constant work, not runtime-conditional content"
    );
    assert!(
        !sources[0].control_flow,
        "an empty presence source cannot be a control-flow root"
    );

    let render_root = store
        .get_node(sources[0].source.render_root)
        .expect("presence render root exists");
    let TemplateIrNodeKind::Sequence { children } = &render_root.kind else {
        panic!("presence source must use a neutral empty sequence render root");
    };
    assert!(children.is_empty());
}
