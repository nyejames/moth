//! HIR function provenance association and validation tests.
//!
//! WHAT: verifies that direct synthetic-interface provenance from AST value metadata is retained
//!      as one immutable fact per local HIR function, that empty facts are explicit, and that HIR
//!      validation rejects missing, extra or out-of-range coverage.
//! WHY: the per-function link-fact lane needs exact AST-to-HIR function association through the
//!      existing path-to-FunctionId lowering owner. These are hidden invariants that integration
//!      output cannot inspect.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::hir::hir_builder::{build_ast_with_registered_types, lower_ast};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_node, make_test_variable, node, test_source_location,
};
use crate::compiler_frontend::value_mode::ValueMode;

fn provenance_member(interface: &str, member_name: &str) -> SyntheticInterfaceMemberIdentity {
    SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        interface,
        member_name,
    )
}

fn int_with_provenance(value: i32, provenance: SyntheticInterfaceProvenance) -> Expression {
    Expression::int(value, test_source_location(2), ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(provenance)
}

#[test]
fn retains_direct_provenance_per_function() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);
    let helper_name = entry_path.join_str("helper", &mut string_table);
    let var_name = super::symbol("result", &mut string_table);

    let member_a = provenance_member("render", "html");
    let member_b = provenance_member("render", "wasm");
    let injected =
        SyntheticInterfaceProvenance::from_members(vec![member_a.clone(), member_b.clone()]);

    let start_body = vec![node(NodeKind::Return(vec![]), test_source_location(1))];
    let helper_body = vec![
        node(
            NodeKind::VariableDeclaration(make_test_variable(
                var_name,
                int_with_provenance(42, injected),
            )),
            test_source_location(2),
        ),
        node(NodeKind::Return(vec![]), test_source_location(3)),
    ];

    let ast = build_ast_with_registered_types(
        vec![
            function_node(
                start_name,
                FunctionSignature {
                    parameters: vec![],
                    returns: vec![],
                },
                start_body,
                test_source_location(1),
            ),
            function_node(
                helper_name.clone(),
                FunctionSignature {
                    parameters: vec![],
                    returns: vec![],
                },
                helper_body,
                test_source_location(2),
            ),
        ],
        entry_path,
    );

    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    // Every function has exactly one provenance fact.
    assert_eq!(module.function_provenance.len(), module.functions.len());

    // The start function has an explicit empty (portable) fact.
    let start_provenance = module
        .function_provenance
        .get(
            &module
                .start_function
                .expect("normal test module should have start"),
        )
        .expect("start function should have a provenance fact");
    assert!(start_provenance.is_empty());

    // The helper function carries the injected member-granular dependencies.
    let helper_id = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == &helper_name)
        })
        .map(|function| function.id)
        .expect("helper function should be present");

    let helper_provenance = module
        .function_provenance
        .get(&helper_id)
        .expect("helper function should have a provenance fact");
    assert_eq!(
        helper_provenance.members(),
        &[
            provenance_member("render", "html"),
            provenance_member("render", "wasm"),
        ]
    );
}

#[test]
fn empty_function_has_explicit_empty_provenance() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let start_body = vec![node(NodeKind::Return(vec![]), test_source_location(1))];

    let ast = build_ast_with_registered_types(
        vec![function_node(
            start_name,
            FunctionSignature {
                parameters: vec![],
                returns: vec![],
            },
            start_body,
            test_source_location(1),
        )],
        entry_path,
    );

    let (module, _type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    assert_eq!(module.function_provenance.len(), module.functions.len());
    let provenance = module
        .function_provenance
        .get(
            &module
                .start_function
                .expect("normal test module should have start"),
        )
        .expect("start function should have a provenance fact");
    assert!(provenance.is_empty());
}

#[test]
fn validation_rejects_missing_provenance_coverage() {
    use crate::compiler_frontend::hir::validation::validate_hir_module;

    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let ast = build_ast_with_registered_types(
        vec![function_node(
            start_name,
            FunctionSignature {
                parameters: vec![],
                returns: vec![],
            },
            vec![node(NodeKind::Return(vec![]), test_source_location(1))],
            test_source_location(1),
        )],
        entry_path,
    );

    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    // Remove the provenance fact to simulate missing coverage.
    module.function_provenance.clear();

    let result = validate_hir_module(&module, &type_environment);
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(
        error.msg.contains("function_provenance"),
        "error should mention function_provenance, got: {}",
        error.msg
    );
}

#[test]
fn validation_rejects_extra_provenance_entry() {
    use crate::compiler_frontend::hir::ids::FunctionId;
    use crate::compiler_frontend::hir::validation::validate_hir_module;

    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let ast = build_ast_with_registered_types(
        vec![function_node(
            start_name,
            FunctionSignature {
                parameters: vec![],
                returns: vec![],
            },
            vec![node(NodeKind::Return(vec![]), test_source_location(1))],
            test_source_location(1),
        )],
        entry_path,
    );

    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    // Add an extra provenance entry for a non-existent function.
    module
        .function_provenance
        .insert(FunctionId(999), SyntheticInterfaceProvenance::empty());

    let result = validate_hir_module(&module, &type_environment);
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(
        error.msg.contains("function_provenance"),
        "error should mention function_provenance, got: {}",
        error.msg
    );
}

#[test]
fn validation_rejects_replaced_out_of_range_provenance_key() {
    use crate::compiler_frontend::hir::ids::FunctionId;
    use crate::compiler_frontend::hir::validation::validate_hir_module;

    let mut string_table = StringTable::new();
    let (entry_path, start_name) = super::entry_path_and_start_name(&mut string_table);

    let ast = build_ast_with_registered_types(
        vec![function_node(
            start_name,
            FunctionSignature {
                parameters: vec![],
                returns: vec![],
            },
            vec![node(NodeKind::Return(vec![]), test_source_location(1))],
            test_source_location(1),
        )],
        entry_path,
    );

    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table).expect("HIR lowering should succeed");

    let start_function = module
        .start_function
        .expect("normal test module should have start");
    module.function_provenance.remove(&start_function);
    module
        .function_provenance
        .insert(FunctionId(999), SyntheticInterfaceProvenance::empty());

    let result = validate_hir_module(&module, &type_environment);
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(
        error.msg.contains("function_provenance"),
        "error should mention function_provenance, got: {}",
        error.msg
    );
}
