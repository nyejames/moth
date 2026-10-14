//! Constant-fact traversal stack-depth regression tests.
//!
//! WHAT: checks that constant-fact collection reaches a deeply nested Collection leaf.
//! WHY: expression depth must consume heap work space rather than one native frame per child.

use super::super::collection_chain_fixture::collection_chain;
use super::*;

#[test]
fn deep_const_fact_walk_uses_one_mib_stack() {
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let (mut builder, mut values, root) = collection_chain();
            let mut strings = StringTable::default();
            let types = TypeEnvironment::default();
            let mut collector = ConstFactCollector::new(
                &mut strings,
                &mut values,
                &mut builder,
                Rc::new(RefCell::new(TemplateIrStore::new())),
                &types,
            );
            collector
                .walk_expression_for_body_local(&root, &mut ConstValueEnvironment::default())
                .expect("collect facts");
        })
        .expect("spawn fact walk thread")
        .join()
        .expect("fact walk completes");
}
