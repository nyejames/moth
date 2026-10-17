//! Template normalisation and assertion-discard traversal stack-depth regression tests.
//!
//! WHAT: checks that both traversals preserve the root of a deeply nested Collection chain.
//! WHY: expression depth must consume heap work space rather than one native frame per child.

use super::super::collection_chain_fixture::collection_chain;
use super::*;

#[test]
fn deep_template_normalisation_uses_one_mib_stack() {
    check_deep_chain(false);
}

#[test]
fn deep_assertion_discard_uses_one_mib_stack() {
    check_deep_chain(true);
}

fn check_deep_chain(discard: bool) {
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            let (mut builder, mut values, mut root) = collection_chain();
            let original = root;
            let mut strings = StringTable::default();
            let mut context = TemplateNormalizationContext {
                template_const_loop_iteration_limit: 1000,
                expression_builder: &mut builder,
                const_values: &mut values,
                string_table: &mut strings,
                template_ir_store: Rc::new(RefCell::new(TemplateIrStore::new())),
                external_package_registry: None,
                module_resources: None,
                expression_pending: Vec::new(),
                expression_results: Vec::new(),
            };
            if discard {
                discard_inactive_assertion_messages_in_expression(
                    &mut root,
                    context.expression_builder,
                    context.const_values,
                    &mut AssertionDiscardScratch::default(),
                )
                .expect("discard messages");
            } else {
                normalize_expression_templates(&mut root, &mut context).expect("normalise chain");
            }
            assert_eq!(root, original);
        })
        .expect("spawn normalisation thread")
        .join()
        .expect("normalisation completes");
}
