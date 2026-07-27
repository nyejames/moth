//! Borrow-checker Reactivity V1 fact tests.
//!
//! WHAT: verifies that borrow validation records conservative invalidation facts for reactive
//! sources while keeping subscriptions out of active borrow state.
//! WHY: later backend phases need source-level dirtying facts, but ordinary borrow/exclusivity
//! rules remain the only mutation authority.

use super::super::types::{
    LocalMode, ReactiveInvalidationFact, ReactiveInvalidationKind, ReactivePlaceWriteKind,
    ValueAccessClassification,
};
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ReactiveSource, ReactiveSourceKind, ReactiveTemplateMetadata,
};
use crate::compiler_frontend::ast::statements::functions::{
    FunctionReturn, FunctionSignature, ReturnChannel, ReturnSlot,
};
use crate::compiler_frontend::ast::templates::template::ReactiveSubscription;
use crate::compiler_frontend::compiler_messages::InvalidMutableAccessReason;
use crate::compiler_frontend::datatypes::{DataType, builtin_type_ids};
use crate::compiler_frontend::hir::expressions::{
    HirExpression, HirExpressionKind, HirMapOp, ValueKind,
};
use crate::compiler_frontend::hir::ids::{FieldId, HirNodeId, HirValueId};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallParameterAccess, PublicCallReactiveEffect,
    PublicCallTransferEffect, PublicCallTransferEligibility,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    assignment_target, function_node, make_test_variable, node, reference_expr, symbol,
    test_location,
};
use crate::compiler_frontend::tests::borrow_fixture_support::assert_invalid_mutable_access_reason;
use crate::compiler_frontend::tests::borrow_fixture_support::run_borrow_checker;
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::{build_ast, entry_and_start, lower_hir};
use crate::compiler_frontend::tests::type_id_fixture_support::param_with_type_id;
use crate::compiler_frontend::value_mode::ValueMode;
use crate::compiler_frontend::{external_packages::CallTarget, hir::reactivity::ReactiveSourceId};

#[test]
fn reactive_assignment_records_invalidation_after_initialization() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let count_path = symbol("count", &mut string_table);
    let source = reactive_source(count_path.clone(), ReactiveSourceKind::Declaration);

    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    count_path.clone(),
                    Expression::int(1, test_location(1), ValueMode::MutableOwned)
                        .with_reactive_source(source),
                )),
                test_location(1),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        count_path.clone(),
                        DataType::Int,
                        builtin_type_ids::INT,
                        test_location(2),
                    ),
                    value: Expression::int(2, test_location(2), ValueMode::ImmutableOwned),
                },
                test_location(2),
            ),
        ],
        test_location(1),
    );

    let hir = lower_hir(build_ast(vec![start], entry_path), &mut string_table);
    let source_id = reactive_source_id_for_path(&hir, &count_path);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("reactive reassignment should follow ordinary mutable assignment rules");

    let facts = all_reactive_invalidations(&report);
    assert_eq!(
        facts.len(),
        1,
        "declaration initialization should not emit an invalidation fact"
    );
    assert_eq!(facts[0].source, source_id);
    assert!(matches!(
        facts[0].kind,
        ReactiveInvalidationKind::Assignment
    ));
}

#[test]
fn reactive_parameter_summary_retains_subscription_without_transfer() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let render_name = symbol("render_reactive_parameter", &mut string_table);
    let render_name_for_lookup = render_name.clone();
    let parameter_path = symbol("source", &mut string_table);
    let view_path = symbol("view", &mut string_table);
    let mut parameter = param_with_type_id(
        parameter_path.clone(),
        builtin_type_ids::INT,
        false,
        test_location(1),
    );
    parameter.value.reactive_source = Some(reactive_source(
        parameter_path.clone(),
        ReactiveSourceKind::Parameter,
    ));

    let render = function_node(
        render_name,
        FunctionSignature {
            parameters: vec![parameter],
            returns: vec![ReturnSlot {
                value: FunctionReturn::Value(DataType::StringSlice),
                type_id: Some(builtin_type_ids::STRING),
                reactive_template: None,
                channel: ReturnChannel::Success,
            }],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    view_path.clone(),
                    Expression::string_slice(
                        string_table.intern("reactive"),
                        test_location(2),
                        ValueMode::ImmutableOwned,
                    ),
                )),
                test_location(2),
            ),
            node(
                NodeKind::Return(vec![
                    reference_expr(
                        view_path,
                        DataType::StringSlice,
                        builtin_type_ids::STRING,
                        test_location(2),
                    )
                    .with_reactive_template_metadata(
                        metadata_with_subscription(
                            reactive_source(parameter_path, ReactiveSourceKind::Parameter),
                            test_location(2),
                        ),
                    ),
                ]),
                test_location(2),
            ),
        ],
        test_location(1),
    );

    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_location(3),
    );

    let hir = lower_hir(
        build_ast(vec![render, start], entry_path),
        &mut string_table,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("reactive parameter summary should validate");
    let render_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path.name() == render_name_for_lookup.name())
        })
        .expect("reactive render function should lower to HIR")
        .id;
    let summary = report
        .analysis
        .public_call_summaries
        .get(&render_id)
        .expect("reactive render function should have a retained summary");

    assert_eq!(summary.parameters.len(), 1);
    assert_eq!(
        summary.parameters[0].access,
        PublicCallParameterAccess::Reactive
    );
    assert_eq!(
        summary.parameters[0].transfer_eligibility,
        PublicCallTransferEligibility::Ineligible
    );
    assert_eq!(
        summary.parameters[0].transfer_effect,
        PublicCallTransferEffect::NeverConsumes
    );
    assert_eq!(
        summary.parameters[0].reactive_effect,
        PublicCallReactiveEffect::Subscribes
    );
    assert_eq!(summary.return_alias, FunctionReturnAliasSummary::Fresh);
}

#[test]
fn reactive_subscription_followed_by_mutation_is_valid_and_dirtying() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let count_path = symbol("count", &mut string_table);
    let view_path = symbol("view", &mut string_table);
    let source = reactive_source(count_path.clone(), ReactiveSourceKind::Declaration);
    let template_metadata = metadata_with_subscription(source.clone(), test_location(3));

    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    count_path.clone(),
                    Expression::int(1, test_location(1), ValueMode::MutableOwned)
                        .with_reactive_source(source),
                )),
                test_location(1),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    view_path.clone(),
                    Expression::string_slice(
                        string_table.intern("<p>count</p>"),
                        test_location(2),
                        ValueMode::ImmutableOwned,
                    ),
                )),
                test_location(2),
            ),
            node(
                NodeKind::PushStartRuntimeFragment(
                    reference_expr(
                        view_path,
                        DataType::StringSlice,
                        builtin_type_ids::STRING,
                        test_location(3),
                    )
                    .with_reactive_template_metadata(template_metadata),
                ),
                test_location(3),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        count_path.clone(),
                        DataType::Int,
                        builtin_type_ids::INT,
                        test_location(4),
                    ),
                    value: Expression::int(2, test_location(4), ValueMode::ImmutableOwned),
                },
                test_location(4),
            ),
        ],
        test_location(1),
    );

    let hir = lower_hir(build_ast(vec![start], entry_path), &mut string_table);
    let source_id = reactive_source_id_for_path(&hir, &count_path);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("subscriptions should not create active borrow lifetimes");

    let facts = all_reactive_invalidations(&report);
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].source, source_id);
    assert!(matches!(
        facts[0].kind,
        ReactiveInvalidationKind::Assignment
    ));
}

#[test]
fn mutable_call_argument_records_reactive_invalidation() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let mutate_path = symbol("mutate", &mut string_table);
    let value_path = symbol("value", &mut string_table);
    let count_path = symbol("count", &mut string_table);
    let source = reactive_source(count_path.clone(), ReactiveSourceKind::Declaration);

    let callee = function_node(
        mutate_path.clone(),
        FunctionSignature {
            parameters: vec![param_with_type_id(
                value_path,
                builtin_type_ids::INT,
                true,
                test_location(1),
            )],
            returns: vec![],
        },
        vec![],
        test_location(1),
    );

    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    count_path.clone(),
                    Expression::int(1, test_location(2), ValueMode::MutableOwned)
                        .with_reactive_source(source),
                )),
                test_location(2),
            ),
            node(
                NodeKind::ExpressionStatement(Expression::function_call_with_arguments(
                    mutate_path,
                    vec![CallArgument::positional(
                        reference_expr(
                            count_path.clone(),
                            DataType::Int,
                            builtin_type_ids::INT,
                            test_location(3),
                        ),
                        CallAccessMode::Shared,
                        test_location(3),
                    )],
                    vec![],
                    test_location(3),
                )),
                test_location(3),
            ),
        ],
        test_location(2),
    );

    let hir = lower_hir(
        build_ast(vec![callee, start], entry_path),
        &mut string_table,
    );
    let source_id = reactive_source_id_for_path(&hir, &count_path);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("reactive sources passed to mutable parameters use ordinary mutable rules");

    let facts = all_reactive_invalidations(&report);
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].source, source_id);
    assert!(matches!(
        &facts[0].kind,
        ReactiveInvalidationKind::MutableCallArgument {
            target: CallTarget::Local(_),
            argument_index: 0,
        }
    ));

    let source_local = hir
        .side_table
        .reactive_source(source_id)
        .expect("reactive source should resolve")
        .local_id;
    let call_block = hir
        .blocks
        .iter()
        .find(|block| {
            block
                .statements
                .iter()
                .any(|statement| statement.id == facts[0].statement_id)
        })
        .expect("mutable reactive call should belong to a reachable block");
    let exit_snapshot = report
        .analysis
        .block_exit_states
        .get(&call_block.id)
        .expect("mutable reactive call should retain an exit snapshot");
    let source_snapshot = exit_snapshot
        .locals
        .iter()
        .find(|snapshot| snapshot.local == source_local)
        .expect("mutable reactive source should remain in the exit snapshot");
    assert!(
        !source_snapshot.mode.is_definitely_uninit(),
        "mutable reactive access must not consume the stable source root"
    );
}

#[test]
fn reactive_source_shared_optional_transfer_falls_back_to_read() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let inspect_path = symbol("inspect", &mut string_table);
    let parameter_path = symbol("value", &mut string_table);
    let count_path = symbol("count", &mut string_table);
    let source = reactive_source(count_path.clone(), ReactiveSourceKind::Declaration);

    let inspect = function_node(
        inspect_path.clone(),
        FunctionSignature {
            parameters: vec![param_with_type_id(
                parameter_path,
                builtin_type_ids::INT,
                false,
                test_location(1),
            )],
            returns: vec![],
        },
        vec![],
        test_location(1),
    );

    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    count_path.clone(),
                    Expression::int(1, test_location(2), ValueMode::MutableOwned)
                        .with_reactive_source(source),
                )),
                test_location(2),
            ),
            node(
                NodeKind::ExpressionStatement(Expression::function_call_with_arguments(
                    inspect_path,
                    vec![CallArgument::positional(
                        reference_expr(
                            count_path.clone(),
                            DataType::Int,
                            builtin_type_ids::INT,
                            test_location(3),
                        ),
                        CallAccessMode::Shared,
                        test_location(3),
                    )],
                    vec![],
                    test_location(3),
                )),
                test_location(3),
            ),
        ],
        test_location(2),
    );

    let hir = lower_hir(
        build_ast(vec![inspect, start], entry_path),
        &mut string_table,
    );
    let source_id = reactive_source_id_for_path(&hir, &count_path);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("shared reactive source access should remain a read at final use");

    assert!(
        all_reactive_invalidations(&report).is_empty(),
        "shared optional transfer must not create a reactive invalidation"
    );

    let call = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Call { args, .. } => args.first(),
            _ => None,
        })
        .expect("shared reactive call should lower to a call statement");
    let value_fact = report
        .analysis
        .value_fact(call.id)
        .expect("shared reactive call argument should have a value fact");
    assert_eq!(
        value_fact.classification,
        ValueAccessClassification::SharedRead
    );

    let source_local = hir
        .side_table
        .reactive_source(source_id)
        .expect("reactive source should resolve")
        .local_id;
    let source_remains_initialized = report.analysis.block_exit_states.values().any(|snapshot| {
        snapshot
            .locals
            .iter()
            .any(|local| local.local == source_local && !local.mode.contains(LocalMode::UNINIT))
    });
    assert!(
        source_remains_initialized,
        "shared optional transfer must not consume the stable source root"
    );
}

#[test]
fn field_write_records_reactive_invalidation() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let source_path = symbol("state", &mut string_table);
    let source = reactive_source(source_path.clone(), ReactiveSourceKind::Declaration);

    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::VariableDeclaration(make_test_variable(
                source_path.clone(),
                Expression::int(1, test_location(1), ValueMode::MutableOwned)
                    .with_reactive_source(source),
            )),
            test_location(1),
        )],
        test_location(1),
    );

    let mut hir = lower_hir(build_ast(vec![start], entry_path), &mut string_table);
    let source_id = reactive_source_id_for_path(&hir, &source_path);
    let statement_id = append_synthetic_field_write(&mut hir, source_id, test_location(2));

    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("field write fact collection should preserve ordinary borrow rules");
    let facts = report
        .analysis
        .reactive_invalidations_for_statement(statement_id)
        .expect("synthetic field write should record invalidation facts");

    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].source, source_id);
    assert!(matches!(
        facts[0].kind,
        ReactiveInvalidationKind::PlaceWrite(ReactivePlaceWriteKind::Field)
    ));
}

#[test]
fn reactive_parameter_does_not_grant_mutation_permission() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let param_path = symbol("source", &mut string_table);
    let mut parameter = param_with_type_id(
        param_path.clone(),
        builtin_type_ids::INT,
        false,
        test_location(1),
    );
    parameter.value.reactive_source = Some(reactive_source(
        param_path.clone(),
        ReactiveSourceKind::Parameter,
    ));

    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![parameter],
            returns: vec![],
        },
        vec![node(
            NodeKind::Assignment {
                target: assignment_target(
                    param_path,
                    DataType::Int,
                    builtin_type_ids::INT,
                    test_location(2),
                ),
                value: Expression::int(2, test_location(2), ValueMode::ImmutableOwned),
            },
            test_location(2),
        )],
        test_location(1),
    );

    let hir = lower_hir(build_ast(vec![start], entry_path), &mut string_table);
    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect_err("reactive parameter metadata must not make an immutable parameter mutable");

    assert_invalid_mutable_access_reason(&error, InvalidMutableAccessReason::ImmutablePlace);
}

#[test]
fn map_mutation_records_reactive_invalidation() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let map_path = symbol("scores", &mut string_table);
    let source = reactive_source(map_path.clone(), ReactiveSourceKind::Declaration);

    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::VariableDeclaration(make_test_variable(
                map_path.clone(),
                Expression::int(1, test_location(1), ValueMode::MutableOwned)
                    .with_reactive_source(source),
            )),
            test_location(1),
        )],
        test_location(1),
    );

    let mut hir = lower_hir(build_ast(vec![start], entry_path), &mut string_table);
    let source_id = reactive_source_id_for_path(&hir, &map_path);
    let (statement_id, value_id) =
        append_synthetic_map_clear(&mut hir, source_id, test_location(2));

    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("map mutation fact collection should preserve ordinary borrow rules");
    let facts = report
        .analysis
        .reactive_invalidations_for_statement(statement_id)
        .expect("synthetic map mutation should record invalidation facts");

    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].source, source_id);
    assert!(matches!(
        facts[0].kind,
        ReactiveInvalidationKind::MapMutation(HirMapOp::Clear)
    ));
    assert!(
        report.analysis.value_fact(value_id).is_some(),
        "synthetic receiver should still receive an ordinary value fact"
    );
}

fn reactive_source(path: InternedPath, kind: ReactiveSourceKind) -> ReactiveSource {
    ReactiveSource { path, kind }
}

fn metadata_with_subscription(
    source: ReactiveSource,
    location: crate::compiler_frontend::ast::ast_nodes::SourceLocation,
) -> ReactiveTemplateMetadata {
    let mut metadata = ReactiveTemplateMetadata::template_backed();
    metadata.push_subscription(ReactiveSubscription {
        source,
        type_id: builtin_type_ids::INT,
        location,
    });
    metadata
}

fn reactive_source_id_for_path(
    hir: &crate::compiler_frontend::hir::module::HirModule,
    path: &InternedPath,
) -> ReactiveSourceId {
    hir.side_table
        .reactive_source_id_for_path(path)
        .expect("reactive source path should resolve")
}

fn all_reactive_invalidations(
    report: &crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport,
) -> Vec<&ReactiveInvalidationFact> {
    let mut facts = report
        .analysis
        .reactive_invalidations
        .values()
        .flatten()
        .collect::<Vec<_>>();
    facts.sort_by_key(|fact| fact.statement_id.0);
    facts
}

fn append_synthetic_map_clear(
    hir: &mut crate::compiler_frontend::hir::module::HirModule,
    source_id: ReactiveSourceId,
    location: crate::compiler_frontend::ast::ast_nodes::SourceLocation,
) -> (HirNodeId, HirValueId) {
    let source = hir
        .side_table
        .reactive_source(source_id)
        .expect("reactive source should resolve");
    let local_id = source.local_id;
    let local = hir
        .blocks
        .iter()
        .flat_map(|block| block.locals.iter())
        .find(|local| local.id == local_id)
        .expect("reactive source local should exist");

    let (statement_id, value_id) = next_statement_and_value_id(hir);

    let receiver = HirExpression {
        id: value_id,
        kind: HirExpressionKind::Load(HirPlace::Local(local_id)),
        ty: local.ty,
        value_kind: ValueKind::Place,
        region: local.region,
    };
    let statement = HirStatement {
        id: statement_id,
        kind: HirStatementKind::MapOp {
            op: HirMapOp::Clear,
            receiver,
            args: vec![],
            result: None,
        },
        location,
    };

    let start_function = hir
        .start_function
        .expect("normal test module should have start");
    let entry_block = hir
        .functions
        .iter()
        .find(|function| function.id == start_function)
        .expect("start function should exist")
        .entry;
    let block = hir
        .blocks
        .iter_mut()
        .find(|block| block.id == entry_block)
        .expect("start entry block should exist");
    block.statements.push(statement);

    (statement_id, value_id)
}

fn append_synthetic_field_write(
    hir: &mut crate::compiler_frontend::hir::module::HirModule,
    source_id: ReactiveSourceId,
    location: crate::compiler_frontend::ast::ast_nodes::SourceLocation,
) -> HirNodeId {
    let source = hir
        .side_table
        .reactive_source(source_id)
        .expect("reactive source should resolve");
    let local_id = source.local_id;
    let local = hir
        .blocks
        .iter()
        .flat_map(|block| block.locals.iter())
        .find(|local| local.id == local_id)
        .expect("reactive source local should exist");

    let (statement_id, value_id) = next_statement_and_value_id(hir);
    let value = HirExpression {
        id: value_id,
        kind: HirExpressionKind::Int(2),
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::RValue,
        region: local.region,
    };
    let statement = HirStatement {
        id: statement_id,
        kind: HirStatementKind::Assign {
            target: HirPlace::Field {
                base: Box::new(HirPlace::Local(local_id)),
                field: FieldId(0),
            },
            value,
        },
        location,
    };

    let start_function = hir
        .start_function
        .expect("normal test module should have start");
    let entry_block = hir
        .functions
        .iter()
        .find(|function| function.id == start_function)
        .expect("start function should exist")
        .entry;
    let block = hir
        .blocks
        .iter_mut()
        .find(|block| block.id == entry_block)
        .expect("start entry block should exist");
    block.statements.push(statement);

    statement_id
}

fn next_statement_and_value_id(
    hir: &crate::compiler_frontend::hir::module::HirModule,
) -> (HirNodeId, HirValueId) {
    let statement_id = HirNodeId(
        hir.blocks
            .iter()
            .flat_map(|block| block.statements.iter())
            .map(|statement| statement.id.0)
            .max()
            .unwrap_or(0)
            + 1,
    );
    let value_id = HirValueId(
        // Synthetic values in these focused tests only need a deterministic ID outside the
        // builder-allocated range for the small fixture module.
        statement_id.0 + 10_000,
    );

    (statement_id, value_id)
}
