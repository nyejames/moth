//! HIR function-origin tracking tests.
//!
//! WHAT: verifies how HIR records entry and normal function origins.
//! WHY: backends rely on stable function-origin metadata when deciding which functions to emit.
//!
//! The old FileStart and RuntimeTemplate origins are removed.
//! Only EntryStart (for the entry-file implicit start) and Normal (for all other functions)
//! remain after Phase 1.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind, SourceLocation};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::hir::functions::{
    FunctionOriginSeed, HirFunctionOrigin, HirFunctionOriginLookup,
};
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, OriginFunctionId, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;

fn location(line: i32) -> SourceLocation {
    crate::compiler_frontend::hir::tests::hir_expression_lowering_tests::location(line)
}

fn node(kind: NodeKind, location: SourceLocation) -> AstNode {
    AstNode {
        kind,
        location,
        scope: InternedPath::new(),
    }
}

fn function_node(name: InternedPath, location: SourceLocation) -> AstNode {
    node(
        NodeKind::Function(
            name,
            FunctionSignature {
                parameters: vec![],
                returns: vec![],
            },
            vec![node(NodeKind::Return(vec![]), location.clone())],
        ),
        location,
    )
}

use crate::compiler_frontend::hir::hir_builder::build_ast;

fn find_function_id_by_path(
    module: &crate::compiler_frontend::hir::module::HirModule,
    target_path: &InternedPath,
) -> Option<FunctionId> {
    module.functions.iter().find_map(|function| {
        let path = module.side_table.function_name_path(function.id)?;
        (path == target_path).then_some(function.id)
    })
}

#[test]
fn classifies_entry_start_and_normal_functions() {
    let mut string_table = StringTable::new();

    let entry_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let entry_start = entry_path.join_str(IMPLICIT_START_FUNC_NAME, &mut string_table);
    let normal_fn = entry_path.join_str("helper", &mut string_table);

    let ast = build_ast(
        vec![
            function_node(entry_start, location(1)),
            function_node(normal_fn.clone(), location(2)),
        ],
        entry_path,
    );

    let lowering = HirBuilder::new(
        &mut string_table,
        PathStringFormatConfig::default(),
        crate::compiler_frontend::datatypes::environment::TypeEnvironment::new(),
        crate::compiler_frontend::hir::functions::HirFunctionOriginLookup::default(),
    )
    .build_hir_module(ast)
    .expect("HIR lowering should succeed");
    let module = lowering.hir_module;

    let normal_id =
        find_function_id_by_path(&module, &normal_fn).expect("normal function should be present");

    assert_eq!(
        module.function_origins.get(&module.start_function),
        Some(&HirFunctionOrigin::EntryStart)
    );
    assert_eq!(
        module.function_origins.get(&normal_id),
        Some(&HirFunctionOrigin::Normal)
    );
    // Every function has exactly one origin tag.
    assert_eq!(module.function_origins.len(), module.functions.len());
}

#[test]
fn lowers_exact_stable_origin_to_local_function_id() {
    let mut string_table = StringTable::new();

    let entry_path = InternedPath::from_single_str("main.moth", &mut string_table);
    let entry_start = entry_path.join_str(IMPLICIT_START_FUNC_NAME, &mut string_table);
    let normal_fn = entry_path.join_str("helper", &mut string_table);
    let stable_module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "shapes".to_owned(),
        ModuleRootRole::Normal,
    );
    let stable_function_origin =
        OriginFunctionId::new_free(stable_module_origin, "helper".to_owned());

    let ast = build_ast(
        vec![
            function_node(entry_start, location(1)),
            function_node(normal_fn.clone(), location(2)),
        ],
        entry_path,
    );

    let lookup = HirFunctionOriginLookup::from_seeds(vec![FunctionOriginSeed {
        path: normal_fn.clone(),
        origin: stable_function_origin.clone(),
    }])
    .expect("exact function-origin path should be unique");
    let module = HirBuilder::new(
        &mut string_table,
        PathStringFormatConfig::default(),
        crate::compiler_frontend::datatypes::environment::TypeEnvironment::new(),
        lookup,
    )
    .build_hir_module(ast)
    .expect("HIR lowering should retain the stable origin mapping")
    .hir_module;

    let normal_id =
        find_function_id_by_path(&module, &normal_fn).expect("normal function should be present");
    assert_eq!(
        module.function_ids_by_origin.get(&stable_function_origin),
        Some(&normal_id)
    );
}

#[test]
fn rejects_duplicate_stable_origins_before_lookup_insertion() {
    let mut string_table = StringTable::new();
    let first_path = InternedPath::from_single_str("first", &mut string_table);
    let second_path = InternedPath::from_single_str("second", &mut string_table);
    let stable_module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("test-project"),
        "shapes".to_owned(),
        ModuleRootRole::Normal,
    );
    let stable_function_origin =
        OriginFunctionId::new_free(stable_module_origin, "helper".to_owned());

    let result = HirFunctionOriginLookup::from_seeds(vec![
        FunctionOriginSeed {
            path: first_path,
            origin: stable_function_origin.clone(),
        },
        FunctionOriginSeed {
            path: second_path,
            origin: stable_function_origin,
        },
    ]);

    assert!(result.is_err(), "duplicate stable origins must be rejected");
}
