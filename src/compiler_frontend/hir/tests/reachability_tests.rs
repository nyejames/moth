//! HIR reachability regression tests.
//!
//! WHAT: exercises the backend-neutral HIR reachability helper against hand-built CFGs.
//! WHY: runtime metadata consumers need deterministic function/block/external-call facts without
//! coupling these tests to AST lowering or backend emission.

use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalFunctionId};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, HirNodeId, HirValueId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::NumericFailureMode;
use crate::compiler_frontend::hir::patterns::{HirMatchArm, HirPattern};
use crate::compiler_frontend::hir::reachability::{
    HirReachability, ReachableFloatStatementKind, ReachableMapUseKind,
    collect_module_function_link_facts, collect_reachability_from_function_link_facts,
};
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::{HirAssertionMessageEvaluation, HirTerminator};
use crate::compiler_frontend::hir::{
    expressions::HirExpression, expressions::HirExpressionKind, expressions::HirMapEntry,
    expressions::HirMapOp,
};
use crate::compiler_frontend::hir::{expressions::ValueKind, ids::LocalId};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};
use std::path::Path;

#[test]
fn start_reachability_ignores_unreachable_function_external_calls() {
    let reachable_external_function = ExternalFunctionId::Synthetic(99);
    let unreachable_external_function = ExternalFunctionId::Synthetic(100);
    let reachable_location = location_at(10, 4);
    let unreachable_location = location_at(20, 8);
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
        ],
        vec![
            block(
                BlockId(0),
                vec![call_statement_at(
                    0,
                    CallTarget::External(reachable_external_function),
                    reachable_location.clone(),
                )],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![call_statement_at(
                    0,
                    CallTarget::External(unreachable_external_function),
                    unreachable_location,
                )],
                HirTerminator::Return(unit_expression(1)),
            ),
        ],
    );

    let reachability = collect_test_reachability(
        &module,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect("reachability should collect from start");

    assert_reachability(&reachability, &[0], &[0], &[reachable_external_function]);
    assert_reachable_external_calls(
        &reachability,
        &[(
            reachable_external_function,
            HirNodeId(0),
            reachable_location,
        )],
    );
}

#[test]
fn reachable_collection_push_ids_remain_distinct() {
    let growable_push = ExternalFunctionId::CollectionPushGrowable;
    let fixed_push = ExternalFunctionId::CollectionPushFixed;
    let growable_location = location_at(10, 4);
    let fixed_location = location_at(10, 8);
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![
                call_statement_at(
                    0,
                    CallTarget::External(growable_push),
                    growable_location.clone(),
                ),
                call_statement_at(1, CallTarget::External(fixed_push), fixed_location.clone()),
            ],
            HirTerminator::Return(unit_expression(0)),
        )],
    );

    let reachability = collect_test_reachability(
        &module,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect("reachability should collect both collection push identities");

    assert_reachability(&reachability, &[0], &[0], &[growable_push, fixed_push]);
    assert_reachable_external_calls(
        &reachability,
        &[
            (growable_push, HirNodeId(0), growable_location),
            (fixed_push, HirNodeId(1), fixed_location),
        ],
    );
}

#[test]
fn user_function_calls_make_transitive_functions_and_external_calls_reachable() {
    let external_function = ExternalFunctionId::Synthetic(200);
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(2)),
        ],
        vec![
            block(
                BlockId(0),
                vec![call_statement(0, CallTarget::Local(FunctionId(1)))],
                HirTerminator::Jump {
                    target: BlockId(1),
                    args: vec![],
                },
            ),
            block(
                BlockId(1),
                vec![],
                HirTerminator::Return(unit_expression(1)),
            ),
            block(
                BlockId(2),
                vec![call_statement(1, CallTarget::External(external_function))],
                HirTerminator::Return(unit_expression(2)),
            ),
        ],
    );

    let reachability = collect_test_reachability(
        &module,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect("reachability should follow call graph");

    assert_reachability(&reachability, &[0, 1], &[0, 1, 2], &[external_function]);
}

#[test]
fn per_function_facts_build_the_exact_reachable_union() {
    let reachable_external_function = ExternalFunctionId::Synthetic(210);
    let unreachable_external_function = ExternalFunctionId::Synthetic(211);
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(2), BlockId(2)),
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
        ],
        vec![
            block(
                BlockId(0),
                vec![call_statement(0, CallTarget::Local(FunctionId(1)))],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![call_statement(
                    1,
                    CallTarget::External(reachable_external_function),
                )],
                HirTerminator::Return(unit_expression(1)),
            ),
            block(
                BlockId(2),
                vec![call_statement(
                    2,
                    CallTarget::External(unreachable_external_function),
                )],
                HirTerminator::Return(unit_expression(2)),
            ),
        ],
    );

    let function_facts = collect_module_function_link_facts(&module)
        .expect("per-function link facts should be collected");
    let reachability = collect_reachability_from_function_link_facts(
        &function_facts,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect("build-owned reachability should succeed");

    assert_reachability(
        &reachability,
        &[0, 1],
        &[0, 1],
        &[reachable_external_function],
    );
    assert!(
        !reachability
            .backend_selection()
            .contains_function(FunctionId(2))
    );
    assert!(
        !reachability
            .reachable_external_functions
            .contains(&unreachable_external_function)
    );
}

#[test]
fn retained_block_facts_preserve_cross_function_breadth_first_diagnostic_order() {
    let external_from_first_callee = ExternalFunctionId::Synthetic(220);
    let external_from_second_callee = ExternalFunctionId::Synthetic(221);
    let second_callee_location = location_at(30, 2);
    let first_callee_successor_location = location_at(40, 2);
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
            function(FunctionId(2), BlockId(2)),
        ],
        vec![
            block(
                BlockId(0),
                vec![
                    call_statement(0, CallTarget::Local(FunctionId(1))),
                    call_statement(1, CallTarget::Local(FunctionId(2))),
                ],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![],
                HirTerminator::Jump {
                    target: BlockId(3),
                    args: vec![],
                },
            ),
            block(
                BlockId(2),
                vec![
                    call_statement_at(
                        2,
                        CallTarget::External(external_from_second_callee),
                        second_callee_location.clone(),
                    ),
                    map_statement_at(4, HirMapOp::Contains, second_callee_location.clone()),
                ],
                HirTerminator::Return(unit_expression(2)),
            ),
            block(
                BlockId(3),
                vec![
                    call_statement_at(
                        3,
                        CallTarget::External(external_from_first_callee),
                        first_callee_successor_location.clone(),
                    ),
                    map_statement_at(5, HirMapOp::Clear, first_callee_successor_location.clone()),
                ],
                HirTerminator::Return(unit_expression(3)),
            ),
        ],
    );

    let reachability = collect_test_reachability(
        &module,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect("retained facts should preserve breadth-first ordering");

    assert_reachable_external_calls(
        &reachability,
        &[
            (
                external_from_second_callee,
                HirNodeId(2),
                second_callee_location.clone(),
            ),
            (
                external_from_first_callee,
                HirNodeId(3),
                first_callee_successor_location.clone(),
            ),
        ],
    );
    assert_eq!(
        reachability
            .reachable_map_uses
            .iter()
            .map(|map_use| map_use.location.clone())
            .collect::<Vec<_>>(),
        vec![second_callee_location, first_callee_successor_location]
    );
}

#[test]
fn backend_selection_rejects_a_detached_cfg_with_colliding_ids() {
    let source_module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![],
            HirTerminator::Return(unit_expression(0)),
        )],
    );
    let source_reachability = collect_test_reachability(&source_module, &[FunctionId(0)])
        .expect("source selection should be valid");
    let target_module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![
            block(
                BlockId(0),
                vec![],
                HirTerminator::Jump {
                    target: BlockId(1),
                    args: vec![],
                },
            ),
            block(
                BlockId(1),
                vec![],
                HirTerminator::Return(unit_expression(1)),
            ),
        ],
    );

    let error = source_reachability
        .backend_selection()
        .validate_for_hir(&target_module)
        .expect_err("selection from another CFG must be rejected");

    assert!(error.msg.contains("does not match the CFG"));
}

#[test]
fn backend_selection_rejects_a_detached_call_graph_with_colliding_ids() {
    let source_module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![],
            HirTerminator::Return(unit_expression(0)),
        )],
    );
    let source_reachability = collect_test_reachability(&source_module, &[FunctionId(0)])
        .expect("source selection should be valid");
    let target_module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
        ],
        vec![
            block(
                BlockId(0),
                vec![call_statement(0, CallTarget::Local(FunctionId(1)))],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![],
                HirTerminator::Return(unit_expression(1)),
            ),
        ],
    );

    let error = source_reachability
        .backend_selection()
        .validate_for_hir(&target_module)
        .expect_err("selection with an omitted callee must be rejected");

    assert!(error.msg.contains("omits callee"));
}

#[test]
fn cfg_successors_cover_branch_match_break_continue_and_terminal_edges() {
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![
            block(
                BlockId(0),
                vec![],
                HirTerminator::If {
                    condition: bool_expression(0),
                    then_block: BlockId(1),
                    else_block: BlockId(2),
                },
            ),
            block(
                BlockId(1),
                vec![],
                HirTerminator::Jump {
                    target: BlockId(3),
                    args: vec![],
                },
            ),
            block(
                BlockId(2),
                vec![],
                HirTerminator::FallibleBranch {
                    result: bool_expression(1),
                    success_block: BlockId(4),
                    error_block: BlockId(5),
                },
            ),
            block(
                BlockId(3),
                vec![],
                HirTerminator::Match {
                    scrutinee: int_expression(2),
                    arms: vec![match_arm(BlockId(6)), match_arm(BlockId(7))],
                },
            ),
            block(
                BlockId(4),
                vec![],
                HirTerminator::Break { target: BlockId(8) },
            ),
            block(
                BlockId(5),
                vec![],
                HirTerminator::Continue { target: BlockId(9) },
            ),
            block(
                BlockId(6),
                vec![],
                HirTerminator::ReturnSuccess(unit_expression(3)),
            ),
            block(
                BlockId(7),
                vec![],
                HirTerminator::ReturnError(unit_expression(4)),
            ),
            block(
                BlockId(8),
                vec![],
                HirTerminator::AssertFailure {
                    message: unit_expression(8),
                    message_evaluation: HirAssertionMessageEvaluation::Default,
                },
            ),
            block(
                BlockId(9),
                vec![],
                HirTerminator::RuntimeFailure {
                    message: "stop".to_owned(),
                },
            ),
        ],
    );

    let reachability = collect_test_reachability(
        &module,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect("reachability should follow CFG edges");

    assert_reachability(&reachability, &[0], &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9], &[]);
    assert_eq!(reachability.reachable_assertion_messages.len(), 1);
    assert_eq!(
        reachability.reachable_assertion_messages[0].evaluation,
        HirAssertionMessageEvaluation::Default
    );
}

#[test]
fn custom_roots_are_supported_without_using_module_start() {
    let external_function = ExternalFunctionId::Synthetic(300);
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
        ],
        vec![
            block(
                BlockId(0),
                vec![],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![call_statement(0, CallTarget::External(external_function))],
                HirTerminator::Return(unit_expression(1)),
            ),
        ],
    );

    let reachability = collect_test_reachability(&module, &[FunctionId(1)])
        .expect("reachability should collect from explicit roots");

    assert_reachability(&reachability, &[1], &[1], &[external_function]);
    assert_reachable_external_calls(
        &reachability,
        &[(external_function, HirNodeId(0), SourceLocation::default())],
    );
}

#[test]
fn reachability_records_reachable_map_uses_only() {
    let literal_location = location_at(40, 2);
    let operation_location = location_at(41, 4);
    let unreachable_location = location_at(50, 6);
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
        ],
        vec![
            block(
                BlockId(0),
                vec![
                    HirStatement {
                        id: HirNodeId(10),
                        kind: HirStatementKind::Expr(map_literal_expression(10)),
                        location: literal_location.clone(),
                    },
                    map_statement_at(11, HirMapOp::Contains, operation_location.clone()),
                ],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![
                    HirStatement {
                        id: HirNodeId(12),
                        kind: HirStatementKind::Expr(map_literal_expression(12)),
                        location: unreachable_location.clone(),
                    },
                    map_statement_at(13, HirMapOp::Clear, unreachable_location),
                ],
                HirTerminator::Return(unit_expression(1)),
            ),
        ],
    );

    let reachability = collect_test_reachability(
        &module,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect("reachability should collect map uses");

    assert_reachability(&reachability, &[0], &[0], &[]);
    assert_eq!(
        reachable_map_use_summaries(&reachability),
        vec![
            ("literal".to_owned(), 40, 2),
            ("contains".to_owned(), 41, 4)
        ],
        "only map uses in reachable blocks should be reported"
    );
}

#[test]
fn reachability_records_ordered_resource_and_site_root_uses_per_owner() {
    let mut resource_table = ModuleResourceTable::new();
    let resource_id = resource_table.intern_origin(
        StableResourceOriginId::module_owned(
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local("reachability-tests"),
                String::new(),
                ModuleRootRole::Normal,
            ),
            PortableResourcePath::from_relative_logical_path(Path::new("assets/logo.svg"))
                .expect("fixture resource path should be portable"),
        ),
        SourceLocation::default(),
    );
    let first_location = location_at(40, 2);
    let second_location = location_at(41, 4);
    let unreachable_location = location_at(50, 6);
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
        ],
        vec![
            block(
                BlockId(0),
                vec![
                    structural_string_statement(
                        10,
                        vec![
                            ConstStringPiece::Resource(resource_id),
                            ConstStringPiece::SiteRoot,
                        ],
                        first_location.clone(),
                    ),
                    structural_string_statement(
                        11,
                        vec![ConstStringPiece::Resource(resource_id)],
                        second_location.clone(),
                    ),
                ],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![structural_string_statement(
                    12,
                    vec![ConstStringPiece::Resource(resource_id)],
                    unreachable_location,
                )],
                HirTerminator::Return(unit_expression(1)),
            ),
        ],
    );

    let reachability = collect_test_reachability(
        &module,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect("reachability should collect structural uses");

    assert_eq!(
        reachability
            .reachable_resource_uses
            .iter()
            .map(|resource_use| (
                resource_use.resource_id,
                resource_use.owner,
                resource_use.location.start_pos.line_number,
                resource_use.location.start_pos.char_column,
            ))
            .collect::<Vec<_>>(),
        vec![
            (resource_id, FunctionId(0), 40, 2),
            (resource_id, FunctionId(0), 41, 4),
        ],
        "repeated origins retain each authored use in order"
    );
    assert_eq!(
        reachability
            .reachable_site_root_uses
            .iter()
            .map(|site_root_use| (
                site_root_use.owner,
                site_root_use.location.start_pos.line_number,
                site_root_use.location.start_pos.char_column,
            ))
            .collect::<Vec<_>>(),
        vec![(FunctionId(0), 40, 2)],
        "site-root use keeps its exact executable owner"
    );
}

#[test]
fn reachability_records_ordered_resource_and_site_root_uses_across_blocks() {
    let mut resource_table = ModuleResourceTable::new();
    let resource_id = resource_table.intern_origin(
        StableResourceOriginId::module_owned(
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local("reachability-tests"),
                String::new(),
                ModuleRootRole::Normal,
            ),
            PortableResourcePath::from_relative_logical_path(Path::new("assets/logo.svg"))
                .expect("fixture resource path should be portable"),
        ),
        SourceLocation::default(),
    );
    let outer_location = location_at(40, 2);
    let nested_branch_location = location_at(41, 4);
    let sibling_branch_location = location_at(42, 6);
    let after_branch_location = location_at(43, 8);
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![
            block(
                BlockId(0),
                vec![structural_string_statement(
                    10,
                    vec![
                        ConstStringPiece::Resource(resource_id),
                        ConstStringPiece::SiteRoot,
                        ConstStringPiece::Resource(resource_id),
                    ],
                    outer_location.clone(),
                )],
                HirTerminator::If {
                    condition: bool_expression(0),
                    then_block: BlockId(2),
                    else_block: BlockId(1),
                },
            ),
            block(
                BlockId(1),
                vec![structural_string_statement(
                    11,
                    vec![
                        ConstStringPiece::Resource(resource_id),
                        ConstStringPiece::SiteRoot,
                    ],
                    sibling_branch_location.clone(),
                )],
                HirTerminator::Jump {
                    target: BlockId(4),
                    args: vec![],
                },
            ),
            block(
                BlockId(2),
                vec![structural_string_statement(
                    12,
                    vec![
                        ConstStringPiece::Resource(resource_id),
                        ConstStringPiece::SiteRoot,
                        ConstStringPiece::Resource(resource_id),
                    ],
                    nested_branch_location.clone(),
                )],
                HirTerminator::Jump {
                    target: BlockId(4),
                    args: vec![],
                },
            ),
            block(
                BlockId(4),
                vec![structural_string_statement(
                    13,
                    vec![
                        ConstStringPiece::Resource(resource_id),
                        ConstStringPiece::SiteRoot,
                        ConstStringPiece::Resource(resource_id),
                    ],
                    after_branch_location.clone(),
                )],
                HirTerminator::Return(unit_expression(4)),
            ),
        ],
    );

    let reachability = collect_test_reachability(
        &module,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect("reachability should collect structural uses across CFG blocks");

    assert_eq!(
        reachability
            .reachable_resource_uses
            .iter()
            .map(|resource_use| (
                resource_use.resource_id,
                resource_use.owner,
                resource_use.location.start_pos.line_number,
                resource_use.location.start_pos.char_column,
            ))
            .collect::<Vec<_>>(),
        vec![
            (resource_id, FunctionId(0), 40, 2),
            (resource_id, FunctionId(0), 40, 2),
            (resource_id, FunctionId(0), 41, 4),
            (resource_id, FunctionId(0), 41, 4),
            (resource_id, FunctionId(0), 42, 6),
            (resource_id, FunctionId(0), 43, 8),
            (resource_id, FunctionId(0), 43, 8),
        ],
        "resource uses must follow breadth-first block order and retain repeats",
    );
    assert_eq!(
        reachability
            .reachable_site_root_uses
            .iter()
            .map(|site_root_use| (
                site_root_use.owner,
                site_root_use.location.start_pos.line_number,
                site_root_use.location.start_pos.char_column,
            ))
            .collect::<Vec<_>>(),
        vec![
            (FunctionId(0), 40, 2),
            (FunctionId(0), 41, 4),
            (FunctionId(0), 42, 6),
            (FunctionId(0), 43, 8),
        ],
        "site-root uses must follow breadth-first block order",
    );
}

#[test]
fn missing_function_references_are_internal_hir_errors() {
    let module = hir_module(FunctionId(0), vec![], vec![]);

    let error = collect_test_reachability(&module, &[FunctionId(99)])
        .expect_err("missing root function should fail");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Function link facts are missing"));
}

#[test]
fn missing_block_references_are_internal_hir_errors() {
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(
            BlockId(0),
            vec![],
            HirTerminator::Jump {
                target: BlockId(99),
                args: vec![],
            },
        )],
    );

    let error = collect_test_reachability(
        &module,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect_err("missing target block should fail");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Unknown HIR block id"));
}

#[test]
fn uninitialized_terminators_are_internal_hir_errors() {
    let module = hir_module(
        FunctionId(0),
        vec![function(FunctionId(0), BlockId(0))],
        vec![block(BlockId(0), vec![], HirTerminator::Uninitialized)],
    );

    let error = collect_test_reachability(
        &module,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect_err("uninitialized terminator should fail");

    assert_eq!(error.error_type, ErrorType::HirTransformation);
    assert!(error.msg.contains("Uninitialized HIR terminator"));
}

fn hir_module(
    start_function: FunctionId,
    functions: Vec<HirFunction>,
    blocks: Vec<HirBlock>,
) -> HirModule {
    let mut module = HirModule::new();
    module.start_function = Some(start_function);
    module.functions = functions;
    module.blocks = blocks;
    module
}

fn collect_test_reachability(
    module: &HirModule,
    roots: &[FunctionId],
) -> Result<HirReachability, crate::compiler_frontend::compiler_errors::CompilerError> {
    let function_facts = collect_module_function_link_facts(module)?;
    collect_reachability_from_function_link_facts(&function_facts, roots)
}

fn function(id: FunctionId, entry: BlockId) -> HirFunction {
    HirFunction {
        id,
        entry,
        params: vec![],
        return_type: builtin_type_ids::NONE,
    }
}

fn block(id: BlockId, statements: Vec<HirStatement>, terminator: HirTerminator) -> HirBlock {
    HirBlock {
        id,
        region: RegionId(0),
        locals: vec![],
        statements,
        terminator,
    }
}

fn call_statement(id: u32, target: CallTarget) -> HirStatement {
    call_statement_at(id, target, SourceLocation::default())
}

fn call_statement_at(id: u32, target: CallTarget, location: SourceLocation) -> HirStatement {
    HirStatement {
        id: HirNodeId(id),
        kind: HirStatementKind::Call {
            target,
            args: vec![],
            result: None::<LocalId>,
        },
        location,
    }
}

fn map_statement_at(id: u32, op: HirMapOp, location: SourceLocation) -> HirStatement {
    HirStatement {
        id: HirNodeId(id),
        kind: HirStatementKind::MapOp {
            op,
            receiver: int_expression(id + 100),
            args: vec![int_expression(id + 200)],
            result: None::<LocalId>,
        },
        location,
    }
}

fn match_arm(body: BlockId) -> HirMatchArm {
    HirMatchArm {
        pattern: HirPattern::Wildcard,
        guard: None,
        body,
    }
}

fn structural_string_statement(
    id: u32,
    pieces: Vec<ConstStringPiece>,
    location: SourceLocation,
) -> HirStatement {
    HirStatement {
        id: HirNodeId(id),
        kind: HirStatementKind::Expr(HirExpression {
            id: HirValueId(id),
            kind: HirExpressionKind::StructuralString { pieces },
            ty: builtin_type_ids::STRING,
            value_kind: ValueKind::RValue,
            region: RegionId(0),
        }),
        location,
    }
}

fn unit_expression(id: u32) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind: HirExpressionKind::TupleConstruct { elements: vec![] },
        ty: builtin_type_ids::NONE,
        value_kind: ValueKind::RValue,
        region: RegionId(0),
    }
}

fn bool_expression(id: u32) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind: HirExpressionKind::Bool(true),
        ty: builtin_type_ids::BOOL,
        value_kind: ValueKind::Const,
        region: RegionId(0),
    }
}

fn int_expression(id: u32) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind: HirExpressionKind::Int(1),
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::Const,
        region: RegionId(0),
    }
}

fn float_expression(id: u32) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind: HirExpressionKind::Float(1.5),
        ty: builtin_type_ids::FLOAT,
        value_kind: ValueKind::Const,
        region: RegionId(0),
    }
}

fn float_statement(
    id: u32,
    kind: ReachableFloatStatementKind,
    location: SourceLocation,
) -> HirStatement {
    let failure_mode = NumericFailureMode::Trap;
    let source = float_expression(id + 100);
    let result = LocalId(9000);

    HirStatement {
        id: HirNodeId(id),
        kind: match kind {
            ReachableFloatStatementKind::FormatFloat => HirStatementKind::FormatFloat {
                source,
                failure_mode,
                result,
            },
            ReachableFloatStatementKind::ValidateFloat => HirStatementKind::ValidateFloat {
                source,
                failure_mode,
                result,
            },
        },
        location,
    }
}

fn cast_expression(id: u32) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind: HirExpressionKind::Cast {
            source: Box::new(int_expression(id + 1)),
            policy: BuiltinCastPolicyId::IntToString,
        },
        ty: builtin_type_ids::STRING,
        value_kind: ValueKind::RValue,
        region: RegionId(0),
    }
}

#[test]
fn reachability_records_reachable_runtime_casts_only() {
    let reachable_location = location_at(30, 2);
    let unreachable_location = location_at(50, 4);
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
        ],
        vec![
            block(
                BlockId(0),
                vec![HirStatement {
                    id: HirNodeId(10),
                    kind: HirStatementKind::Expr(cast_expression(10)),
                    location: reachable_location.clone(),
                }],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![HirStatement {
                    id: HirNodeId(11),
                    kind: HirStatementKind::Expr(cast_expression(11)),
                    location: unreachable_location.clone(),
                }],
                HirTerminator::Return(unit_expression(1)),
            ),
        ],
    );

    let reachability = collect_test_reachability(
        &module,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect("reachability should collect casts");

    assert_eq!(
        reachability.reachable_runtime_casts.len(),
        1,
        "only casts in reachable blocks should be reported"
    );
    assert_eq!(
        reachability.reachable_runtime_casts[0]
            .location
            .start_pos
            .line_number,
        30
    );
    assert_eq!(
        reachability.reachable_runtime_casts[0]
            .location
            .start_pos
            .char_column,
        2
    );
}

#[test]
fn reachability_records_reachable_float_statements_only() {
    let reachable_location = location_at(30, 2);
    let unreachable_location = location_at(50, 4);
    let module = hir_module(
        FunctionId(0),
        vec![
            function(FunctionId(0), BlockId(0)),
            function(FunctionId(1), BlockId(1)),
        ],
        vec![
            block(
                BlockId(0),
                vec![float_statement(
                    10,
                    ReachableFloatStatementKind::FormatFloat,
                    reachable_location.clone(),
                )],
                HirTerminator::Return(unit_expression(0)),
            ),
            block(
                BlockId(1),
                vec![float_statement(
                    11,
                    ReachableFloatStatementKind::ValidateFloat,
                    unreachable_location.clone(),
                )],
                HirTerminator::Return(unit_expression(1)),
            ),
        ],
    );

    let reachability = collect_test_reachability(
        &module,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect("reachability should collect float statements");

    assert_eq!(
        reachability.reachable_float_statements.len(),
        1,
        "only float statements in reachable blocks should be reported"
    );
    assert_eq!(
        reachability.reachable_float_statements[0].kind,
        ReachableFloatStatementKind::FormatFloat
    );
    assert_eq!(
        reachability.reachable_float_statements[0]
            .location
            .start_pos
            .line_number,
        30
    );
    assert_eq!(
        reachability.reachable_float_statements[0]
            .location
            .start_pos
            .char_column,
        2
    );
}

fn map_literal_expression(id: u32) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind: HirExpressionKind::MapLiteral(vec![HirMapEntry {
            key: int_expression(id + 1),
            value: int_expression(id + 2),
        }]),
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::RValue,
        region: RegionId(0),
    }
}

fn assert_reachability(
    reachability: &HirReachability,
    function_ids: &[u32],
    block_ids: &[u32],
    external_function_ids: &[ExternalFunctionId],
) {
    assert_eq!(
        sorted_function_ids(reachability),
        function_ids,
        "reachable functions differ"
    );
    assert_eq!(
        sorted_block_ids(reachability),
        block_ids,
        "reachable blocks differ"
    );
    assert_eq!(
        sorted_external_function_ids(reachability),
        sorted_external_ids(external_function_ids),
        "reachable external functions differ"
    );
}

fn assert_reachable_external_calls(
    reachability: &HirReachability,
    expected_calls: &[(ExternalFunctionId, HirNodeId, SourceLocation)],
) {
    let actual_calls = reachability
        .reachable_external_calls
        .iter()
        .map(|call| {
            (
                external_id_sort_key(&call.function_id),
                call.statement_id.0,
                call.location.start_pos.line_number,
                call.location.start_pos.char_column,
            )
        })
        .collect::<Vec<_>>();

    let expected_calls = expected_calls
        .iter()
        .map(|(function_id, statement_id, location)| {
            (
                external_id_sort_key(function_id),
                statement_id.0,
                location.start_pos.line_number,
                location.start_pos.char_column,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        actual_calls, expected_calls,
        "reachable external call sites differ"
    );
}

fn reachable_map_use_summaries(reachability: &HirReachability) -> Vec<(String, i32, i32)> {
    reachability
        .reachable_map_uses
        .iter()
        .map(|map_use| {
            (
                match &map_use.kind {
                    ReachableMapUseKind::Literal => "literal".to_owned(),
                    ReachableMapUseKind::Operation(op) => op.source_name().to_owned(),
                },
                map_use.location.start_pos.line_number,
                map_use.location.start_pos.char_column,
            )
        })
        .collect()
}

fn location_at(line_number: i32, char_column: i32) -> SourceLocation {
    SourceLocation {
        start_pos: CharPosition {
            line_number,
            char_column,
        },
        end_pos: CharPosition {
            line_number,
            char_column: char_column + 1,
        },
        ..SourceLocation::default()
    }
}

fn sorted_function_ids(reachability: &HirReachability) -> Vec<u32> {
    let mut ids = reachability
        .backend_selection()
        .functions()
        .iter()
        .map(|function_id| function_id.0)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

fn sorted_block_ids(reachability: &HirReachability) -> Vec<u32> {
    let mut ids = reachability
        .backend_selection()
        .blocks()
        .iter()
        .map(|block_id| block_id.0)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

fn sorted_external_function_ids(reachability: &HirReachability) -> Vec<String> {
    let mut ids = reachability
        .reachable_external_functions
        .iter()
        .map(external_id_sort_key)
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn sorted_external_ids(ids: &[ExternalFunctionId]) -> Vec<String> {
    let mut ids = ids.iter().map(external_id_sort_key).collect::<Vec<_>>();
    ids.sort();
    ids
}

fn external_id_sort_key(id: &ExternalFunctionId) -> String {
    match id {
        ExternalFunctionId::IoPrint => "builtin:print".to_owned(),
        ExternalFunctionId::IoLine => "builtin:line".to_owned(),
        ExternalFunctionId::IoDebug => "builtin:debug".to_owned(),
        ExternalFunctionId::IoWarn => "builtin:warn".to_owned(),
        ExternalFunctionId::IoError => "builtin:error".to_owned(),
        ExternalFunctionId::IoInputNew => "builtin:io_input_new".to_owned(),
        ExternalFunctionId::IoInputUpdate => "builtin:io_input_update".to_owned(),
        ExternalFunctionId::IoInputClose => "builtin:io_input_close".to_owned(),
        ExternalFunctionId::IoInputKeyDown => "builtin:io_input_key_down".to_owned(),
        ExternalFunctionId::IoInputKeyPressed => "builtin:io_input_key_pressed".to_owned(),
        ExternalFunctionId::IoInputKeyReleased => "builtin:io_input_key_released".to_owned(),
        ExternalFunctionId::IoInputPointerX => "builtin:io_input_pointer_x".to_owned(),
        ExternalFunctionId::IoInputPointerY => "builtin:io_input_pointer_y".to_owned(),
        ExternalFunctionId::IoInputPointerDown => "builtin:io_input_pointer_down".to_owned(),
        ExternalFunctionId::IoInputPointerPressed => "builtin:io_input_pointer_pressed".to_owned(),
        ExternalFunctionId::IoInputPointerReleased => {
            "builtin:io_input_pointer_released".to_owned()
        }
        ExternalFunctionId::IoInputLastKeyPressed => "builtin:io_input_last_key_pressed".to_owned(),
        ExternalFunctionId::IoInputLastKeyReleased => {
            "builtin:io_input_last_key_released".to_owned()
        }
        ExternalFunctionId::IoInputLastPointerPressed => {
            "builtin:io_input_last_pointer_pressed".to_owned()
        }
        ExternalFunctionId::IoInputLastPointerReleased => {
            "builtin:io_input_last_pointer_released".to_owned()
        }
        ExternalFunctionId::CollectionGet => "builtin:collection_get".to_owned(),
        ExternalFunctionId::CollectionSet => "builtin:collection_set".to_owned(),
        ExternalFunctionId::CollectionPushGrowable => "builtin:collection_push_growable".to_owned(),
        ExternalFunctionId::CollectionPushFixed => "builtin:collection_push_fixed".to_owned(),
        ExternalFunctionId::CollectionRemove => "builtin:collection_remove".to_owned(),
        ExternalFunctionId::CollectionLength => "builtin:collection_length".to_owned(),
        ExternalFunctionId::Synthetic(id) => format!("synthetic:{id}"),
    }
}
