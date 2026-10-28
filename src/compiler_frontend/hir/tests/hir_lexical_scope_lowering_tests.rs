//! Compiler-generated lexical-scope HIR lowering tests.
//!
//! WHAT: verifies the internal scope wrapper creates a child lexical region and rejoins its parent.
//! WHY: static Bool specialization removes runtime control flow but must retain authored scope.

use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::hir::blocks::HirLocal;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_node, make_test_variable, node,
};

use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::tests::type_id_fixture_support::{
    build_ast_with_registered_types, lower_ast,
};

/// One exact `I32` literal expression.
///
/// WHY: these fixtures pin the exact `I32` width explicitly at the scalar carrier.
fn i32_literal(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: i64,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
) -> ExprId {
    builder
        .fixed_scalar(
            constants,
            FixedScalarValue::signed(FixedScalar::I32, value).expect("test literal fits I32"),
            span,
            value_mode,
        )
        .expect("test literal fits expression store")
        .id()
}

fn local_by_name<'a>(
    module: &'a HirModule,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> &'a HirLocal {
    module
        .blocks
        .iter()
        .flat_map(|block| block.locals.iter())
        .find(|local| {
            module
                .side_table
                .resolve_local_name(local.id, path_fork, string_table)
                == Some(name)
        })
        .unwrap_or_else(|| panic!("expected local '{name}'"))
}

#[test]
fn compiler_generated_scope_lowers_through_child_region_and_rejoins_parent() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let inner = super::symbol("inner", &mut path_fork, &mut string_table);
    let after = super::symbol("after", &mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();

    let inner_value = i32_literal(
        &mut builder,
        &mut constants,
        1,
        None,
        ValueMode::ImmutableOwned,
    );
    let lexical_scope = node(
        NodeKind::LexicalScope {
            body: vec![node(
                NodeKind::VariableDeclaration(make_test_variable(inner, inner_value, &builder)),
                None,
            )],
        },
        None,
    );
    let after_value = i32_literal(
        &mut builder,
        &mut constants,
        2,
        None,
        ValueMode::ImmutableOwned,
    );
    let after_declaration = node(
        NodeKind::VariableDeclaration(make_test_variable(after, after_value, &builder)),
        None,
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![lexical_scope, after_declaration],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_function],
        entry_path,
        builder,
        constants,
        TypeEnvironment::new(),
    );
    let (module, _type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("HIR lowering should succeed");
    let _inner_local = local_by_name(&module, &path_fork, &string_table, "inner");
    let _after_local = local_by_name(&module, &path_fork, &string_table, "after");
    let start_function = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let parent_region = module.blocks[start_function.entry.0 as usize].region;
    let inner_local = local_by_name(&module, &path_fork, &string_table, "inner");
    let after_local = local_by_name(&module, &path_fork, &string_table, "after");
    let inner_region = module
        .regions
        .iter()
        .find(|region| region.id() == inner_local.region)
        .expect("inner local region should exist");

    assert_ne!(inner_local.region, parent_region);
    assert_eq!(inner_region.parent(), Some(parent_region));
    assert_eq!(after_local.region, parent_region);
}
