//! Compiler-generated lexical-scope HIR lowering tests.
//!
//! WHAT: verifies the internal scope wrapper creates a child lexical region and rejoins its parent.
//! WHY: static Bool specialization removes runtime control flow but must retain authored scope.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::hir::blocks::HirLocal;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_node, make_test_variable, node, test_source_location,
};

use crate::compiler_frontend::value_mode::ValueMode;

use crate::compiler_frontend::hir::hir_builder::{
    assert_no_placeholder_terminators, build_ast_with_registered_types, lower_ast,
};

fn local_by_name<'a>(
    module: &'a HirModule,
    string_table: &StringTable,
    name: &str,
) -> &'a HirLocal {
    module
        .blocks
        .iter()
        .flat_map(|block| block.locals.iter())
        .find(|local| module.side_table.resolve_local_name(local.id, string_table) == Some(name))
        .unwrap_or_else(|| panic!("expected local '{name}'"))
}

#[test]
fn compiler_generated_scope_lowers_through_child_region_and_rejoins_parent() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let inner = super::symbol("inner", &mut string_table);
    let after = super::symbol("after", &mut string_table);

    let lexical_scope = node(
        NodeKind::LexicalScope {
            body: vec![node(
                NodeKind::VariableDeclaration(make_test_variable(
                    inner,
                    Expression::int(1, test_source_location(2), ValueMode::ImmutableOwned),
                )),
                test_source_location(2),
            )],
        },
        test_source_location(1),
    );
    let after_declaration = node(
        NodeKind::VariableDeclaration(make_test_variable(
            after,
            Expression::int(2, test_source_location(4), ValueMode::ImmutableOwned),
        )),
        test_source_location(4),
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![lexical_scope, after_declaration],
        test_source_location(1),
    );

    let ast = build_ast_with_registered_types(vec![start_function], entry_path);
    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");
    assert_no_placeholder_terminators(&module);

    let start_function = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let parent_region = module.blocks[start_function.entry.0 as usize].region;
    let inner_local = local_by_name(&module, &string_table, "inner");
    let after_local = local_by_name(&module, &string_table, "after");
    let inner_region = module
        .regions
        .iter()
        .find(|region| region.id() == inner_local.region)
        .expect("inner local region should exist");

    assert_ne!(inner_local.region, parent_region);
    assert_eq!(inner_region.parent(), Some(parent_region));
    assert_eq!(after_local.region, parent_region);
}
