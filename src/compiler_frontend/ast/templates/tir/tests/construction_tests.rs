use super::super::construction_context::TemplateConstructionContext;
use super::super::node::TemplateIrNodeKind;
use super::super::store::TemplateIrStore;
use super::super::view::TemplateTirPhase;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::template::{Style, TemplateType};
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateBranchSelector;
use crate::compiler_frontend::ast::templates::tir::node::TemplateIrBranch;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn finish_consumes_the_construction_context_and_records_real_depth() {
    let store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let mut string_table = StringTable::new();
    let mut construction =
        TemplateConstructionContext::new(Rc::clone(&store), SourceLocation::default());

    let body_text = string_table.intern("leaf");
    construction.record_text(body_text, 4, SourceLocation::default());
    let body_id = *construction
        .root_children()
        .first()
        .expect("recorded text child");

    let selector = TemplateBranchSelector::Bool(Expression::bool(
        true,
        SourceLocation::default(),
        ValueMode::ImmutableOwned,
    ));
    let selector_site_id = construction.next_expression_site_id();
    construction.record_branch_chain(
        vec![TemplateIrBranch::new(
            selector,
            body_id,
            SourceLocation::default(),
            selector_site_id,
        )],
        None,
        SourceLocation::default(),
    );

    assert!(construction.control_flow_node_id().is_some());

    let reference = construction
        .finish(
            Style::default(),
            TemplateType::String,
            TemplateTirPhase::Parsed,
        )
        .expect("parser-emitted control-flow TIR is finite");
    let store = store.borrow();
    let template = store
        .get_template(reference.root)
        .expect("finished template");
    assert!(template.summary.has_control_flow);
    assert!(
        template.summary.max_depth >= 1,
        "branch body should contribute depth, got {}",
        template.summary.max_depth
    );

    match &store.get_node(template.root).expect("root exists").kind {
        TemplateIrNodeKind::BranchChain { .. } => {}
        other => panic!("expected a direct branch-chain root, got {other:?}"),
    }

    let recomputed = super::super::summary::summarize_existing_root(&store, template.root)
        .expect("published control-flow root is finite");
    let mut published = template.summary.clone();
    published.head_node_count = recomputed.head_node_count;
    assert_eq!(
        published, recomputed,
        "parser control-flow publication must match recomputation"
    );
}
