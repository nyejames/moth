//! Static-if specialisation traversal stack-depth regression tests.
//!
//! WHAT: checks that specialisation preserves a deeply nested Collection chain's root.
//! WHY: expression depth must consume heap work space rather than one native frame per child.

use super::super::collection_chain_fixture::collection_chain;
use super::*;

#[test]
fn deep_static_specialisation_uses_one_mib_stack() {
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let (mut builder, mut values, mut root) = collection_chain();
            let original = root;
            let mut strings = StringTable::default();
            let types = TypeEnvironment::default();
            let resolver = ConstValueResolver::new(
                &mut strings,
                &mut values,
                &mut builder,
                Rc::new(RefCell::new(TemplateIrStore::new())),
                &types,
            );
            let mut specializer = StaticIfSpecializer {
                resolver,
                inactive_generic_requests: Vec::new(),
                selection_count: 0,
                current_function_path: None,
                function_provenance: FxHashMap::default(),
                expression_pending: Vec::new(),
                expression_results: Vec::new(),
            };
            specializer
                .specialize_expression(&mut root, &mut ConstValueEnvironment::default())
                .expect("specialise chain");
            assert_eq!(root, original);
        })
        .expect("spawn specialisation thread")
        .join()
        .expect("specialisation completes");
}
