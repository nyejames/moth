use super::super::node::{
    TemplateIr, TemplateIrBranch, TemplateIrNodeKind, TemplateLoopHeaderExpressionSites,
};
use super::super::store::TemplateIrStore;
use super::super::summary::TemplateIrSummary;
use super::builder::TemplateIrBuilder;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateBranchSelector;
use crate::compiler_frontend::ast::templates::tir::ids::{
    ChildTemplateOccurrenceId, ExpressionSiteId, SlotOccurrenceId, TemplateIrNodeId,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

fn slot_occurrence_id(store: &TemplateIrStore, node_id: TemplateIrNodeId) -> SlotOccurrenceId {
    match &store
        .get_node(node_id)
        .expect("slot node should exist")
        .kind
    {
        TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
        other => panic!("expected Slot node, got {other:?}"),
    }
}

fn child_template_occurrence_id(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> ChildTemplateOccurrenceId {
    match &store
        .get_node(node_id)
        .expect("child-template node should exist")
        .kind
    {
        TemplateIrNodeKind::ChildTemplate { occurrence_id, .. } => *occurrence_id,
        other => panic!("expected ChildTemplate node, got {other:?}"),
    }
}

fn dynamic_expression_site_id(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> ExpressionSiteId {
    match &store
        .get_node(node_id)
        .expect("dynamic-expression node should exist")
        .kind
    {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected DynamicExpression node, got {other:?}"),
    }
}

fn branch_selector_site_id(store: &TemplateIrStore, node_id: TemplateIrNodeId) -> ExpressionSiteId {
    match &store
        .get_node(node_id)
        .expect("branch-chain node should exist")
        .kind
    {
        TemplateIrNodeKind::BranchChain { branches, .. } => branches[0].selector_site_id,
        other => panic!("expected BranchChain node, got {other:?}"),
    }
}

fn conditional_loop_site_id(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> ExpressionSiteId {
    match &store
        .get_node(node_id)
        .expect("loop node should exist")
        .kind
    {
        TemplateIrNodeKind::Loop { header_sites, .. } => match header_sites {
            TemplateLoopHeaderExpressionSites::Conditional { condition } => *condition,
            other => panic!("expected Conditional header sites, got {other:?}"),
        },
        other => panic!("expected Loop node, got {other:?}"),
    }
}

fn range_loop_site_ids(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> (ExpressionSiteId, ExpressionSiteId, Option<ExpressionSiteId>) {
    match &store
        .get_node(node_id)
        .expect("loop node should exist")
        .kind
    {
        TemplateIrNodeKind::Loop { header_sites, .. } => match header_sites {
            TemplateLoopHeaderExpressionSites::Range { start, end, step } => (*start, *end, *step),
            other => panic!("expected Range header sites, got {other:?}"),
        },
        other => panic!("expected Loop node, got {other:?}"),
    }
}

#[test]
fn push_text_node_stores_text_payload() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let text_id = string_table.intern("payload");
    let node_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_text_node(
            text_id,
            7,
            TemplateSegmentOrigin::Head,
            SourceLocation::default(),
        )
    };

    let node = store.get_node(node_id).expect("node should exist");
    let stored_text_id = match &node.kind {
        TemplateIrNodeKind::Text {
            text,
            byte_len,
            origin,
        } => {
            assert_eq!(*byte_len, 7);
            assert_eq!(*origin, TemplateSegmentOrigin::Head);
            *text
        }
        other => panic!("expected Text node, got {other:?}"),
    };
    assert_eq!(stored_text_id, text_id);
}

#[test]
fn push_sequence_node_stores_children() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let (child_a, child_b, sequence_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);

        let child_a = builder.push_text_node(
            string_table.intern("a"),
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let child_b = builder.push_text_node(
            string_table.intern("b"),
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let sequence_id =
            builder.push_sequence_node(vec![child_a, child_b], SourceLocation::default());

        (child_a, child_b, sequence_id)
    };

    let node = store
        .get_node(sequence_id)
        .expect("sequence node should exist");
    let children = match &node.kind {
        TemplateIrNodeKind::Sequence { children } => children,
        other => panic!("expected Sequence node, got {other:?}"),
    };
    assert_eq!(children.len(), 2);
    assert_eq!(children[0], child_a);
    assert_eq!(children[1], child_b);
}

#[test]
fn push_child_template_node_stores_child_id() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let (child_template_id, child_node_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);

        let child_root = builder.push_text_node(
            string_table.intern("child"),
            5,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let child_template_id = builder.finish_template(
            child_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );
        let child_node_id =
            builder.push_child_template_node(child_template_id, SourceLocation::default());

        (child_template_id, child_node_id)
    };

    let node = store
        .get_node(child_node_id)
        .expect("child template node should exist");
    match &node.kind {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            assert_eq!(reference.root, child_template_id);
        }
        other => panic!("expected ChildTemplate node, got {other:?}"),
    }
}

#[test]
fn finish_template_stores_metadata() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let (root, template_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);

        let root = builder.push_text_node(
            string_table.intern("root"),
            4,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );

        (root, template_id)
    };

    let template = store
        .get_template(template_id)
        .expect("template should exist");
    assert_eq!(template.root, root);
    assert_eq!(template.kind, TemplateType::String);
}

#[test]
fn builder_does_not_expose_mutable_store_vectors() {
    // The builder only exposes construction methods. This test exercises that
    // the store is still owned externally and remains inspectable after the
    // builder is dropped.
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();

    let template_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let root = builder.push_text_node(
            string_table.intern("x"),
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        )
    };

    let template = store
        .get_template(template_id)
        .expect("template should exist");
    assert!(matches!(
        store.get_node(template.root).map(|n| &n.kind),
        Some(TemplateIrNodeKind::Text { .. })
    ));
}

#[test]
fn push_slot_node_stores_placeholder_payload() {
    let mut store = TemplateIrStore::new();
    let node_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_slot_node(SlotKey::Default, SourceLocation::default())
    };

    let node = store.get_node(node_id).expect("slot node should exist");
    match &node.kind {
        TemplateIrNodeKind::Slot {
            placeholder: stored_slot,
        } => {
            assert_eq!(stored_slot.key, SlotKey::Default);
            assert!(stored_slot.child_wrapper_set.is_none());
        }
        other => panic!("expected Slot node, got {other:?}"),
    }
}

#[test]
fn push_insert_contribution_node_stores_child_template_id() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let (contribution_template_id, contribution_node_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);

        let contribution_root = builder.push_text_node(
            string_table.intern("contribution"),
            12,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let contribution_template_id = builder.finish_template(
            contribution_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );
        let contribution_node_id = builder
            .push_insert_contribution_node(contribution_template_id, SourceLocation::default());

        (contribution_template_id, contribution_node_id)
    };

    let node = store
        .get_node(contribution_node_id)
        .expect("insert contribution node should exist");
    match &node.kind {
        TemplateIrNodeKind::InsertContribution { template } => {
            assert_eq!(*template, contribution_template_id);
        }
        other => panic!("expected InsertContribution node, got {other:?}"),
    }
}

#[test]
fn push_dynamic_expression_node_stores_expression_payload() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let expression = Expression::string_slice(
        string_table.intern("expr"),
        SourceLocation::default(),
        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
    );

    let node_id = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        builder.push_dynamic_expression_node(
            expression.clone(),
            TemplateSegmentOrigin::Head,
            None,
            SourceLocation::default(),
        )
    };

    let node = store
        .get_node(node_id)
        .expect("dynamic expression node should exist");
    match &node.kind {
        TemplateIrNodeKind::DynamicExpression {
            expression, origin, ..
        } => {
            assert_eq!(*origin, TemplateSegmentOrigin::Head);
            assert!(matches!(
                expression.kind,
                crate::compiler_frontend::ast::expressions::expression_kind::ExpressionKind::StringSlice(_)
            ));
        }
        other => panic!("expected DynamicExpression node, got {other:?}"),
    }
}

// -------------------------
//  Occurrence / Site ID Tests
// -------------------------

#[test]
fn slot_occurrence_ids_assigned_in_document_order() {
    let mut store = TemplateIrStore::new();
    let (id_a, id_b, id_c) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let id_a = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
        let id_b = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
        let id_c = builder.push_slot_node(SlotKey::Default, SourceLocation::default());
        (id_a, id_b, id_c)
    };

    let occurrence_a = match &store.get_node(id_a).expect("slot a").kind {
        TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
        other => panic!("expected Slot node, got {other:?}"),
    };
    let occurrence_b = match &store.get_node(id_b).expect("slot b").kind {
        TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
        other => panic!("expected Slot node, got {other:?}"),
    };
    let occurrence_c = match &store.get_node(id_c).expect("slot c").kind {
        TemplateIrNodeKind::Slot { placeholder } => placeholder.occurrence_id,
        other => panic!("expected Slot node, got {other:?}"),
    };

    assert_eq!(occurrence_a, SlotOccurrenceId::new(0));
    assert_eq!(occurrence_b, SlotOccurrenceId::new(1));
    assert_eq!(occurrence_c, SlotOccurrenceId::new(2));
}

#[test]
fn child_template_occurrence_ids_assigned_in_document_order() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let (id_a, id_b) = {
        let mut builder = TemplateIrBuilder::new(&mut store);

        let root_a = builder.push_text_node(
            string_table.intern("a"),
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let template_a = builder.finish_template(
            root_a,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );

        let root_b = builder.push_text_node(
            string_table.intern("b"),
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let template_b = builder.finish_template(
            root_b,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );

        let id_a = builder.push_child_template_node(template_a, SourceLocation::default());
        let id_b = builder.push_child_template_node(template_b, SourceLocation::default());
        (id_a, id_b)
    };

    let occurrence_a = match &store.get_node(id_a).expect("child a").kind {
        TemplateIrNodeKind::ChildTemplate { occurrence_id, .. } => *occurrence_id,
        other => panic!("expected ChildTemplate node, got {other:?}"),
    };
    let occurrence_b = match &store.get_node(id_b).expect("child b").kind {
        TemplateIrNodeKind::ChildTemplate { occurrence_id, .. } => *occurrence_id,
        other => panic!("expected ChildTemplate node, got {other:?}"),
    };

    assert_eq!(occurrence_a, ChildTemplateOccurrenceId::new(0));
    assert_eq!(occurrence_b, ChildTemplateOccurrenceId::new(1));
}

#[test]
fn expression_site_ids_assigned_in_document_order() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let (id_a, id_b, id_c) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let id_a = builder.push_dynamic_expression_node(
            Expression::string_slice(
                string_table.intern("a"),
                SourceLocation::default(),
                crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
            ),
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );
        let id_b = builder.push_dynamic_expression_node(
            Expression::string_slice(
                string_table.intern("b"),
                SourceLocation::default(),
                crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
            ),
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );
        let id_c = builder.push_dynamic_expression_node(
            Expression::string_slice(
                string_table.intern("c"),
                SourceLocation::default(),
                crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
            ),
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );
        (id_a, id_b, id_c)
    };

    let site_a = match &store.get_node(id_a).expect("expr a").kind {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected DynamicExpression node, got {other:?}"),
    };
    let site_b = match &store.get_node(id_b).expect("expr b").kind {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected DynamicExpression node, got {other:?}"),
    };
    let site_c = match &store.get_node(id_c).expect("expr c").kind {
        TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
        other => panic!("expected DynamicExpression node, got {other:?}"),
    };

    assert_eq!(site_a, ExpressionSiteId::new(0));
    assert_eq!(site_b, ExpressionSiteId::new(1));
    assert_eq!(site_c, ExpressionSiteId::new(2));
}

// -------------------------
//  Derived-root preservation and fresh-ID continuation
// -------------------------

/// A derived template root that structurally shares an existing root node
/// preserves the occurrence/site IDs already embedded in those shared nodes.
///
/// WHAT: pushes a second `TemplateIr` entry that references the same root node
/// as the first template. Because TIR nodes are store-owned and looked up by
/// ID (not cloned per template), the occurrence/site IDs inside those nodes
/// are inherently preserved: no re-allocation happens.
/// WHY: the final TIR architecture uses append-only derived roots. Overlay and
/// slot-resolution phases rely on shared nodes keeping their original IDs so
/// contributions and wrappers remain unambiguous across derived roots.
#[test]
fn derived_root_preserves_existing_occurrence_and_site_ids() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();

    // Build a sequence root with one slot, one child template, and one
    // dynamic expression so all three occurrence/site ID families are
    // represented in the shared subtree.
    let (root, first_template_id, slot_id, child_id, expr_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);

        let slot_id = builder.push_slot_node(SlotKey::Default, SourceLocation::default());

        let child_root = builder.push_text_node(
            string_table.intern("child"),
            5,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let child_template = builder.finish_template(
            child_root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );
        let child_id = builder.push_child_template_node(child_template, SourceLocation::default());

        let expr_id = builder.push_dynamic_expression_node(
            Expression::string_slice(
                string_table.intern("expr"),
                SourceLocation::default(),
                crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
            ),
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );

        let root =
            builder.push_sequence_node(vec![slot_id, child_id, expr_id], SourceLocation::default());

        let first_template_id = builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );

        (root, first_template_id, slot_id, child_id, expr_id)
    };

    // Capture the original occurrence/site IDs before creating the derived root.
    let original_slot_occurrence = slot_occurrence_id(&store, slot_id);
    let original_child_occurrence = child_template_occurrence_id(&store, child_id);
    let original_expression_site = dynamic_expression_site_id(&store, expr_id);

    // Push a second template that structurally shares the same root node.
    let derived_template_id = store.push_template(TemplateIr::new(
        root,
        Style::default(),
        TemplateType::String,
        TemplateIrSummary::default(),
        SourceLocation::default(),
    ));

    // Both templates point to the same root node: the derived root is shared,
    // not copied.
    let first_template = store
        .get_template(first_template_id)
        .expect("first template");
    let derived_template = store
        .get_template(derived_template_id)
        .expect("derived template");
    assert_eq!(first_template.root, root);
    assert_eq!(derived_template.root, root);
    assert_eq!(first_template.root, derived_template.root);

    // The occurrence/site IDs on the shared nodes are unchanged.
    assert_eq!(
        slot_occurrence_id(&store, slot_id),
        original_slot_occurrence
    );
    assert_eq!(
        child_template_occurrence_id(&store, child_id),
        original_child_occurrence
    );
    assert_eq!(
        dynamic_expression_site_id(&store, expr_id),
        original_expression_site
    );
}

/// Newly created structural nodes receive fresh occurrence/site IDs that
/// continue from existing counter values, covering all five ID families.
///
/// WHAT: builds a first round of slot, child-template, dynamic-expression,
/// branch-selector, and loop-header nodes, then builds a second round and
/// asserts the new IDs are exactly one past the first-round values.
/// WHY: the per-store counters must be monotonic: a derived root or
/// re-pushed template must never cause the counter to reset or re-issue an
/// already-assigned ID.
#[test]
fn newly_created_nodes_receive_fresh_ids_after_existing_allocations() {
    use crate::compiler_frontend::ast::ast_nodes::{LoopBindings, RangeLoopSpec};
    use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;

    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();

    let (
        slot_occ_first,
        child_occ_first,
        expr_site_first,
        branch_site_first,
        loop_conditional_site_first,
        slot_occ_second,
        child_occ_second,
        expr_site_second,
        branch_site_second,
        range_start_second,
        range_end_second,
        range_step_second,
    ) = {
        let mut builder = TemplateIrBuilder::new(&mut store);

        // ---- Round 1: one of each structural node family ----

        let slot_first = builder.push_slot_node(SlotKey::Default, SourceLocation::default());

        let child_root_a = builder.push_text_node(
            string_table.intern("a"),
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let child_template_a = builder.finish_template(
            child_root_a,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );
        let child_first =
            builder.push_child_template_node(child_template_a, SourceLocation::default());

        let expr_first = builder.push_dynamic_expression_node(
            Expression::string_slice(
                string_table.intern("e1"),
                SourceLocation::default(),
                crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
            ),
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );

        let branch_body_a = builder.push_text_node(
            string_table.intern("ba"),
            2,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let branch_first = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(Expression::bool(
                true,
                SourceLocation::default(),
                crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
            )),
            branch_body_a,
            SourceLocation::default(),
            builder.store.next_expression_site_id(),
        );
        let chain_first =
            builder.push_branch_chain_node(vec![branch_first], None, SourceLocation::default());

        let loop_body_a = builder.push_text_node(
            string_table.intern("la"),
            2,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let loop_first = builder.push_loop_node(
            TemplateLoopHeader::Conditional {
                condition: Box::new(Expression::bool(
                    true,
                    SourceLocation::default(),
                    crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
                )),
            },
            loop_body_a,
            None,
            SourceLocation::default(),
        );

        // ---- Round 2: one of each again, continuing from round-1 counters ----

        let slot_second = builder.push_slot_node(SlotKey::Default, SourceLocation::default());

        let child_root_b = builder.push_text_node(
            string_table.intern("b"),
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let child_template_b = builder.finish_template(
            child_root_b,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            SourceLocation::default(),
        );
        let child_second =
            builder.push_child_template_node(child_template_b, SourceLocation::default());

        let expr_second = builder.push_dynamic_expression_node(
            Expression::string_slice(
                string_table.intern("e2"),
                SourceLocation::default(),
                crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
            ),
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );

        let branch_body_b = builder.push_text_node(
            string_table.intern("bb"),
            2,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let branch_second = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(Expression::bool(
                false,
                SourceLocation::default(),
                crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
            )),
            branch_body_b,
            SourceLocation::default(),
            builder.store.next_expression_site_id(),
        );
        let chain_second =
            builder.push_branch_chain_node(vec![branch_second], None, SourceLocation::default());

        let loop_body_b = builder.push_text_node(
            string_table.intern("lb"),
            2,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let loop_second = builder.push_loop_node(
            TemplateLoopHeader::Range {
                bindings: Box::new(LoopBindings {
                    item: None,
                    index: None,
                }),
                range: Box::new(RangeLoopSpec {
                    start: Expression::int(
                        0,
                        SourceLocation::default(),
                        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
                    ),
                    end: Expression::int(
                        10,
                        SourceLocation::default(),
                        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
                    ),
                    end_kind: crate::compiler_frontend::ast::ast_nodes::RangeEndKind::Exclusive,
                    step: Some(Expression::int(
                        1,
                        SourceLocation::default(),
                        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
                    )),
                }),
            },
            loop_body_b,
            None,
            SourceLocation::default(),
        );

        let (range_start_second, range_end_second, range_step_second) =
            range_loop_site_ids(&store, loop_second);

        (
            slot_occurrence_id(&store, slot_first),
            child_template_occurrence_id(&store, child_first),
            dynamic_expression_site_id(&store, expr_first),
            branch_selector_site_id(&store, chain_first),
            conditional_loop_site_id(&store, loop_first),
            slot_occurrence_id(&store, slot_second),
            child_template_occurrence_id(&store, child_second),
            dynamic_expression_site_id(&store, expr_second),
            branch_selector_site_id(&store, chain_second),
            range_start_second,
            range_end_second,
            range_step_second,
        )
    };

    // Round 1 IDs start from zero.
    assert_eq!(slot_occ_first, SlotOccurrenceId::new(0));
    assert_eq!(child_occ_first, ChildTemplateOccurrenceId::new(0));
    assert_eq!(expr_site_first, ExpressionSiteId::new(0));
    assert_eq!(branch_site_first, ExpressionSiteId::new(1));
    assert_eq!(loop_conditional_site_first, ExpressionSiteId::new(2));

    // Round 2 IDs continue from the round-1 counters: no reset, no re-issue.
    assert_eq!(slot_occ_second, SlotOccurrenceId::new(1));
    assert_eq!(child_occ_second, ChildTemplateOccurrenceId::new(1));
    assert_eq!(expr_site_second, ExpressionSiteId::new(3));
    assert_eq!(branch_site_second, ExpressionSiteId::new(4));
    assert_eq!(range_start_second, ExpressionSiteId::new(5));
    assert_eq!(range_end_second, ExpressionSiteId::new(6));
    assert_eq!(range_step_second, Some(ExpressionSiteId::new(7)));
}

// -------------------------
//  Branch selector and loop-header expression-site IDs
// -------------------------

#[test]
fn branch_selector_site_ids_assigned_in_document_order() {
    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let (branch_a_site, branch_b_site) = {
        let mut builder = TemplateIrBuilder::new(&mut store);

        let body_a = builder.push_text_node(
            string_table.intern("a"),
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let body_b = builder.push_text_node(
            string_table.intern("b"),
            1,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );

        let branch_a = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(Expression::bool(
                true,
                SourceLocation::default(),
                crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
            )),
            body_a,
            SourceLocation::default(),
            builder.store.next_expression_site_id(),
        );
        let branch_b = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(Expression::bool(
                false,
                SourceLocation::default(),
                crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
            )),
            body_b,
            SourceLocation::default(),
            builder.store.next_expression_site_id(),
        );

        let chain_id = builder.push_branch_chain_node(
            vec![branch_a, branch_b],
            None,
            SourceLocation::default(),
        );

        let chain = store.get_node(chain_id).expect("chain node");
        let branches = match &chain.kind {
            TemplateIrNodeKind::BranchChain { branches, .. } => branches,
            other => panic!("expected BranchChain, got {other:?}"),
        };
        (branches[0].selector_site_id, branches[1].selector_site_id)
    };

    assert_eq!(branch_a_site, ExpressionSiteId::new(0));
    assert_eq!(branch_b_site, ExpressionSiteId::new(1));
}

#[test]
fn loop_conditional_and_collection_headers_each_assign_one_expression_site() {
    use crate::compiler_frontend::ast::ast_nodes::LoopBindings;
    use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;

    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let (conditional_node_id, collection_node_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);

        let conditional_body = builder.push_text_node(
            string_table.intern("cond-body"),
            10,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let conditional_node_id = builder.push_loop_node(
            TemplateLoopHeader::Conditional {
                condition: Box::new(Expression::bool(
                    true,
                    SourceLocation::default(),
                    crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
                )),
            },
            conditional_body,
            None,
            SourceLocation::default(),
        );

        let collection_body = builder.push_text_node(
            string_table.intern("coll-body"),
            10,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let collection_node_id = builder.push_loop_node(
            TemplateLoopHeader::Collection {
                bindings: Box::new(LoopBindings {
                    item: None,
                    index: None,
                }),
                iterable: Box::new(Expression::string_slice(
                    string_table.intern("items"),
                    SourceLocation::default(),
                    crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
                )),
            },
            collection_body,
            None,
            SourceLocation::default(),
        );

        (conditional_node_id, collection_node_id)
    };

    // Conditional and collection loop headers each allocate exactly one
    // expression site, in document order.
    let conditional_loop = store
        .get_node(conditional_node_id)
        .expect("conditional loop node");
    match &conditional_loop.kind {
        TemplateIrNodeKind::Loop { header_sites, .. } => match header_sites {
            TemplateLoopHeaderExpressionSites::Conditional { condition } => {
                assert_eq!(*condition, ExpressionSiteId::new(0));
            }
            other => panic!("expected Conditional header sites, got {other:?}"),
        },
        other => panic!("expected Loop node, got {other:?}"),
    }

    let collection_loop = store
        .get_node(collection_node_id)
        .expect("collection loop node");
    match &collection_loop.kind {
        TemplateIrNodeKind::Loop { header_sites, .. } => match header_sites {
            TemplateLoopHeaderExpressionSites::Collection { iterable } => {
                assert_eq!(*iterable, ExpressionSiteId::new(1));
            }
            other => panic!("expected Collection header sites, got {other:?}"),
        },
        other => panic!("expected Loop node, got {other:?}"),
    }
}

#[test]
fn loop_range_header_assigns_start_end_and_optional_step_sites() {
    use crate::compiler_frontend::ast::ast_nodes::{LoopBindings, RangeLoopSpec};
    use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;

    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();
    let (with_step_id, without_step_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);

        // Range with an explicit step allocates start, end, then the step site.
        let with_step_body = builder.push_text_node(
            string_table.intern("step-body"),
            9,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let with_step_id = builder.push_loop_node(
            TemplateLoopHeader::Range {
                bindings: Box::new(LoopBindings {
                    item: None,
                    index: None,
                }),
                range: Box::new(RangeLoopSpec {
                    start: Expression::int(
                        0,
                        SourceLocation::default(),
                        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
                    ),
                    end: Expression::int(
                        10,
                        SourceLocation::default(),
                        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
                    ),
                    end_kind: crate::compiler_frontend::ast::ast_nodes::RangeEndKind::Exclusive,
                    step: Some(Expression::int(
                        2,
                        SourceLocation::default(),
                        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
                    )),
                }),
            },
            with_step_body,
            None,
            SourceLocation::default(),
        );

        // Range without a step allocates start and end only; no step site.
        let without_step_body = builder.push_text_node(
            string_table.intern("no-step-body"),
            11,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let without_step_id = builder.push_loop_node(
            TemplateLoopHeader::Range {
                bindings: Box::new(LoopBindings {
                    item: None,
                    index: None,
                }),
                range: Box::new(RangeLoopSpec {
                    start: Expression::int(
                        0,
                        SourceLocation::default(),
                        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
                    ),
                    end: Expression::int(
                        10,
                        SourceLocation::default(),
                        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
                    ),
                    end_kind: crate::compiler_frontend::ast::ast_nodes::RangeEndKind::Exclusive,
                    step: None,
                }),
            },
            without_step_body,
            None,
            SourceLocation::default(),
        );

        (with_step_id, without_step_id)
    };

    // With step: start, end, then the step site, in document order.
    let with_step = store.get_node(with_step_id).expect("range with step node");
    match &with_step.kind {
        TemplateIrNodeKind::Loop { header_sites, .. } => match header_sites {
            TemplateLoopHeaderExpressionSites::Range { start, end, step } => {
                assert_eq!(*start, ExpressionSiteId::new(0));
                assert_eq!(*end, ExpressionSiteId::new(1));
                assert_eq!(*step, Some(ExpressionSiteId::new(2)));
            }
            other => panic!("expected Range header sites, got {other:?}"),
        },
        other => panic!("expected Loop node, got {other:?}"),
    }

    // Without step: start and end continue the document order; no step site.
    let without_step = store
        .get_node(without_step_id)
        .expect("range without step node");
    match &without_step.kind {
        TemplateIrNodeKind::Loop { header_sites, .. } => match header_sites {
            TemplateLoopHeaderExpressionSites::Range { start, end, step } => {
                assert_eq!(*start, ExpressionSiteId::new(3));
                assert_eq!(*end, ExpressionSiteId::new(4));
                assert!(step.is_none());
            }
            other => panic!("expected Range header sites, got {other:?}"),
        },
        other => panic!("expected Loop node, got {other:?}"),
    }
}

#[test]
fn expression_sites_share_one_document_order_counter() {
    use crate::compiler_frontend::ast::ast_nodes::{LoopBindings, RangeLoopSpec};
    use crate::compiler_frontend::ast::templates::template_control_flow::TemplateLoopHeader;

    let mut store = TemplateIrStore::new();
    let mut string_table = StringTable::new();

    let (expr_site, branch_site, range_start, range_end, range_step) = {
        let mut builder = TemplateIrBuilder::new(&mut store);

        // First: a dynamic expression splice (site 0).
        let expr_node = builder.push_dynamic_expression_node(
            Expression::string_slice(
                string_table.intern("expr"),
                SourceLocation::default(),
                crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
            ),
            TemplateSegmentOrigin::Body,
            None,
            SourceLocation::default(),
        );

        // Second: a branch chain with one branch selector (site 1).
        let branch_body = builder.push_text_node(
            string_table.intern("branch"),
            6,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let branch = TemplateIrBranch::new(
            TemplateBranchSelector::Bool(Expression::bool(
                true,
                SourceLocation::default(),
                crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
            )),
            branch_body,
            SourceLocation::default(),
            builder.store.next_expression_site_id(),
        );
        let chain_node =
            builder.push_branch_chain_node(vec![branch], None, SourceLocation::default());

        // Third: a range loop with start (site 2), end (site 3), step (site 4).
        let loop_body = builder.push_text_node(
            string_table.intern("loop"),
            4,
            TemplateSegmentOrigin::Body,
            SourceLocation::default(),
        );
        let loop_node = builder.push_loop_node(
            TemplateLoopHeader::Range {
                bindings: Box::new(LoopBindings {
                    item: None,
                    index: None,
                }),
                range: Box::new(RangeLoopSpec {
                    start: Expression::int(
                        0,
                        SourceLocation::default(),
                        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
                    ),
                    end: Expression::int(
                        10,
                        SourceLocation::default(),
                        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
                    ),
                    end_kind: crate::compiler_frontend::ast::ast_nodes::RangeEndKind::Exclusive,
                    step: Some(Expression::int(
                        1,
                        SourceLocation::default(),
                        crate::compiler_frontend::value_mode::ValueMode::ImmutableOwned,
                    )),
                }),
            },
            loop_body,
            None,
            SourceLocation::default(),
        );

        // Read back the site IDs.
        let expr_site = match &store.get_node(expr_node).expect("expr").kind {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            other => panic!("expected DynamicExpression, got {other:?}"),
        };

        let branch_site = match &store.get_node(chain_node).expect("chain").kind {
            TemplateIrNodeKind::BranchChain { branches, .. } => branches[0].selector_site_id,
            other => panic!("expected BranchChain, got {other:?}"),
        };

        let (range_start, range_end, range_step) =
            match &store.get_node(loop_node).expect("loop").kind {
                TemplateIrNodeKind::Loop { header_sites, .. } => match header_sites {
                    TemplateLoopHeaderExpressionSites::Range { start, end, step } => {
                        (*start, *end, *step)
                    }
                    other => panic!("expected Range header sites, got {other:?}"),
                },
                other => panic!("expected Loop node, got {other:?}"),
            };

        (expr_site, branch_site, range_start, range_end, range_step)
    };

    // All sites share one document-order counter.
    assert_eq!(expr_site, ExpressionSiteId::new(0));
    assert_eq!(branch_site, ExpressionSiteId::new(1));
    assert_eq!(range_start, ExpressionSiteId::new(2));
    assert_eq!(range_end, ExpressionSiteId::new(3));
    assert_eq!(range_step, Some(ExpressionSiteId::new(4)));
}
