//! Borrow-checker call-summary regression tests.
//!
//! WHAT: verifies how call return aliases and host-call access summaries affect borrow facts.
//! WHY: call boundaries are where alias metadata is easiest to get wrong and hardest to debug.

use crate::compiler_frontend::analysis::borrow_checker::{LocalMode, OptionalTransferStatus};
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, CallPassingMode,
};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, FallibleExpressionHandling, HandledFallibleHostFunctionCallInput, Operator,
    ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpn, ExpressionRpnItem,
};
use crate::compiler_frontend::ast::statements::functions::{
    FunctionSignature, ReturnChannel, ReturnSlot,
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::compiler_messages::{
    BorrowAccessKind, BorrowDiagnosticKind, DiagnosticPayload, InvalidMutableAccessReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::{DataType, builtin_type_ids};
use crate::compiler_frontend::external_packages::test_support::{
    TestExternalAbiType as ExternalAbiType, TestExternalAccessKind as ExternalAccessKind,
    TestExternalReturnAlias as ExternalReturnAlias,
};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType as RegistryAbiType, ExternalFunctionDef, ExternalFunctionId,
    ExternalFunctionLowerings, ExternalReturnSlot, ExternalSignatureType,
};
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    assignment_target, fresh_success_returns, function_node, immutable_reference_expr,
    make_test_variable, node, param, symbol, test_source_location,
};
use crate::compiler_frontend::tests::borrow_fixture_support::{
    assert_borrow_error_kind, assert_infrastructure_error_contains,
    assert_invalid_mutable_access_reason, run_borrow_checker,
};
use crate::compiler_frontend::tests::external_package_support::{
    default_external_package_registry, register_external_function,
};
use crate::compiler_frontend::tests::hir_fixture_support::{entry_and_start, lower_hir};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::tests::type_id_fixture_support::build_ast_with_registered_types;
use crate::compiler_frontend::tests::type_id_fixture_support::param_with_type_id;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use crate::compiler_frontend::value_mode::ValueMode;
use std::sync::Arc;

use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallReactiveEffect, PublicCallTransferEffect, PublicCallTransferEligibility,
};

fn function_call_node(
    name: InternedPath,
    args: Vec<CallArgument>,
    result_type_ids: Vec<TypeId>,
    location: SourceLocation,
) -> NodeKind {
    NodeKind::ExpressionStatement(Expression::function_call_with_arguments(
        name,
        args,
        result_type_ids,
        location,
    ))
}

fn host_function_call_node(
    id: ExternalFunctionId,
    args: Vec<CallArgument>,
    result_type_ids: Vec<TypeId>,
    location: SourceLocation,
) -> NodeKind {
    NodeKind::ExpressionStatement(Expression::host_function_call_with_arguments(
        id,
        args,
        result_type_ids,
        location,
    ))
}

#[test]
fn public_call_summaries_cover_zero_parameter_and_parameter_effects() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let summary_target = symbol("summary_target", &mut string_table);
    let summary_target_name = summary_target.clone();
    let shared_parameter = symbol("shared_parameter", &mut string_table);
    let mutable_parameter = symbol("mutable_parameter", &mut string_table);
    let untouched_mutable_parameter = symbol("untouched_mutable_parameter", &mut string_table);
    let reactive_parameter = symbol("reactive_parameter", &mut string_table);
    let mut reactive_parameter_declaration = param_with_type_id(
        reactive_parameter.clone(),
        builtin_type_ids::INT,
        false,
        test_source_location(1),
    );
    reactive_parameter_declaration.value.reactive_source = Some(ReactiveSource {
        path: reactive_parameter.clone(),
        kind: ReactiveSourceKind::Parameter,
    });

    let target = function_node(
        summary_target,
        FunctionSignature {
            parameters: vec![
                param(
                    shared_parameter,
                    DataType::Int,
                    builtin_type_ids::INT,
                    false,
                    test_source_location(1),
                ),
                param(
                    mutable_parameter.clone(),
                    DataType::Int,
                    builtin_type_ids::INT,
                    true,
                    test_source_location(1),
                ),
                param(
                    untouched_mutable_parameter,
                    DataType::Int,
                    builtin_type_ids::INT,
                    true,
                    test_source_location(1),
                ),
                reactive_parameter_declaration,
            ],
            returns: vec![],
        },
        vec![node(
            NodeKind::Assignment {
                target: assignment_target(
                    mutable_parameter,
                    DataType::Int,
                    builtin_type_ids::INT,
                    test_source_location(2),
                ),
                value: Expression::int(2, test_source_location(2), ValueMode::ImmutableOwned),
            },
            test_source_location(2),
        )],
        test_source_location(1),
    );

    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_source_location(3),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![target, start], entry_path),
        &mut string_table,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("public call summary construction should succeed");

    assert_eq!(
        report.analysis.public_call_summaries.len(),
        hir.functions.len()
    );

    let target_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path.name() == summary_target_name.name())
        })
        .expect("summary target should lower to HIR")
        .id;
    let target_summary = report
        .analysis
        .public_call_summaries
        .get(&target_id)
        .expect("every local function should have one retained summary");

    assert_eq!(target_summary.parameters.len(), 4);
    assert_eq!(
        target_summary.parameters[0].access,
        PublicCallParameterAccess::Shared
    );
    assert_eq!(
        target_summary.parameters[0].mutation,
        PublicCallMutationEffect::NoWrite
    );
    assert_eq!(
        target_summary.parameters[0].transfer_eligibility,
        PublicCallTransferEligibility::Eligible
    );
    assert_eq!(
        target_summary.parameters[0].transfer_effect,
        PublicCallTransferEffect::MayConsume
    );

    assert_eq!(
        target_summary.parameters[1].access,
        PublicCallParameterAccess::Mutable
    );
    assert_eq!(
        target_summary.parameters[1].mutation,
        PublicCallMutationEffect::Writes
    );
    assert_eq!(
        target_summary.parameters[1].transfer_effect,
        PublicCallTransferEffect::MayConsume
    );

    assert_eq!(
        target_summary.parameters[2].access,
        PublicCallParameterAccess::Mutable
    );
    assert_eq!(
        target_summary.parameters[2].mutation,
        PublicCallMutationEffect::NoWrite
    );

    assert_eq!(
        target_summary.parameters[3].access,
        PublicCallParameterAccess::Reactive
    );
    assert_eq!(
        target_summary.parameters[3].transfer_eligibility,
        PublicCallTransferEligibility::Ineligible
    );
    assert_eq!(
        target_summary.parameters[3].transfer_effect,
        PublicCallTransferEffect::NeverConsumes
    );
    assert_eq!(
        target_summary.parameters[3].reactive_effect,
        PublicCallReactiveEffect::None
    );

    let start_id = hir
        .start_function
        .expect("normal test module should have start");
    let start_summary = report
        .analysis
        .public_call_summaries
        .get(&start_id)
        .expect("the zero-parameter start function should have a summary");
    assert!(start_summary.parameters.is_empty());
    assert_eq!(
        start_summary.return_alias,
        FunctionReturnAliasSummary::Fresh
    );

    let rendered = format!("{target_summary:?}");
    assert!(!rendered.contains("LocalId"));
    assert!(!rendered.contains("BlockId"));
    assert!(!rendered.contains("SourceLocation"));
    assert!(!rendered.contains("InternedPath"));
}

#[test]
fn incomplete_public_call_summary_metadata_uses_infrastructure_lane() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );

    let mut hir = lower_hir(
        build_ast_with_registered_types(vec![start], entry_path),
        &mut string_table,
    );
    hir.functions[0].params.push(LocalId(999_999));

    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect_err("partial parameter metadata should fail summary construction");
    assert_infrastructure_error_contains(
        &error,
        ErrorType::Compiler,
        &["could not resolve mutability for parameter local"],
    );
}

#[test]
fn public_call_summary_keeps_mutable_but_unwritten_call_path_read_only() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let mutator_name = symbol("mutator", &mut string_table);
    let wrapper_name = symbol("wrapper", &mut string_table);
    let wrapper_name_for_lookup = wrapper_name.clone();
    let parameter_name = symbol("value", &mut string_table);

    let mutator = function_node(
        mutator_name.clone(),
        FunctionSignature {
            parameters: vec![param(
                symbol("input", &mut string_table),
                DataType::Int,
                builtin_type_ids::INT,
                true,
                test_source_location(1),
            )],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );
    let wrapper = function_node(
        wrapper_name,
        FunctionSignature {
            parameters: vec![param(
                parameter_name.clone(),
                DataType::Int,
                builtin_type_ids::INT,
                true,
                test_source_location(2),
            )],
            returns: vec![],
        },
        vec![node(
            function_call_node(
                mutator_name,
                vec![CallArgument::positional(
                    immutable_reference_expr(
                        parameter_name,
                        DataType::Int,
                        builtin_type_ids::INT,
                        test_source_location(3),
                    ),
                    CallAccessMode::Mutable,
                    test_source_location(3),
                )],
                vec![],
                test_source_location(3),
            ),
            test_source_location(3),
        )],
        test_source_location(2),
    );
    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_source_location(4),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![mutator, wrapper, start], entry_path),
        &mut string_table,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("mutable parameter call path should validate");
    let wrapper_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path.name() == wrapper_name_for_lookup.name())
        })
        .expect("wrapper function should lower to HIR")
        .id;
    let summary = report
        .analysis
        .public_call_summaries
        .get(&wrapper_id)
        .expect("wrapper should have a retained call summary");
    assert_eq!(
        summary.parameters[0].mutation,
        PublicCallMutationEffect::NoWrite
    );
}

#[test]
fn public_call_summary_tracks_mixed_argument_access_by_position() {
    let source = r#"mutator |shared Int, mutable ~Int|:
;

wrapper |shared Int, mutable ~Int|:
    mutator(shared, ~mutable)
;"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let wrapper_name = symbol("wrapper", &mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("mixed shared and mutable call arguments should validate");

    let wrapper_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path.name() == wrapper_name.name())
        })
        .expect("wrapper function should lower to HIR")
        .id;
    let summary = report
        .analysis
        .public_call_summaries
        .get(&wrapper_id)
        .expect("wrapper should have a retained call summary");

    assert_eq!(
        summary.parameters[0].mutation,
        PublicCallMutationEffect::NoWrite
    );
    assert_eq!(
        summary.parameters[1].mutation,
        PublicCallMutationEffect::NoWrite
    );
}

fn local_call_mutation_summary_in_order(
    callee_body: &str,
    callee_declared_first: bool,
) -> (PublicCallMutationEffect, PublicCallMutationEffect) {
    let callee = format!("mutator |input ~Int|:\n{callee_body}\n;\n");
    let wrapper = "wrapper |value ~Int|:\n    mutator(~value)\n;\n";
    let source = if callee_declared_first {
        format!("{callee}\n{wrapper}")
    } else {
        format!("{wrapper}\n{callee}")
    };
    let (ast, mut string_table) = parse_single_file_ast(&source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let mutator_name = symbol("mutator", &mut string_table);
    let wrapper_name = symbol("wrapper", &mut string_table);
    let mutator_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path.name() == mutator_name.name())
        })
        .expect("mutator function should lower to HIR")
        .id;
    let wrapper_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path.name() == wrapper_name.name())
        })
        .expect("wrapper function should lower to HIR")
        .id;
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("local mutation summary fixture should validate");

    let mutator_mutation = report
        .analysis
        .public_call_summaries
        .get(&mutator_id)
        .expect("mutator should have a retained call summary")
        .parameters[0]
        .mutation;
    let wrapper_mutation = report
        .analysis
        .public_call_summaries
        .get(&wrapper_id)
        .expect("wrapper should have a retained call summary")
        .parameters[0]
        .mutation;

    (mutator_mutation, wrapper_mutation)
}

#[test]
fn local_mutation_summary_keeps_unwritten_mutable_callee_order_independent() {
    for callee_declared_first in [true, false] {
        let (mutator_mutation, wrapper_mutation) =
            local_call_mutation_summary_in_order("", callee_declared_first);
        assert_eq!(mutator_mutation, PublicCallMutationEffect::NoWrite);
        assert_eq!(wrapper_mutation, PublicCallMutationEffect::NoWrite);
    }
}

#[test]
fn local_mutation_summary_propagates_writes_order_independently() {
    for callee_declared_first in [true, false] {
        let (mutator_mutation, wrapper_mutation) =
            local_call_mutation_summary_in_order("    input = 1", callee_declared_first);
        assert_eq!(mutator_mutation, PublicCallMutationEffect::Writes);
        assert_eq!(wrapper_mutation, PublicCallMutationEffect::Writes);
    }
}

#[test]
fn public_call_summary_map_set_only_writes_receiver_parameter() {
    let source = r#"mutate |scores ~{String = String}, key String, value String|:
    ~scores.set(key, value) catch:
    ;
;"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let mutate_name = symbol("mutate", &mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("map set with mutable receiver should validate");

    let function_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path.name() == mutate_name.name())
        })
        .expect("map mutator function should lower to HIR")
        .id;
    let summary = report
        .analysis
        .public_call_summaries
        .get(&function_id)
        .expect("map mutator should have a retained call summary");

    assert_eq!(
        summary.parameters[0].mutation,
        PublicCallMutationEffect::Writes
    );
    assert_eq!(
        summary.parameters[1].mutation,
        PublicCallMutationEffect::NoWrite
    );
    assert_eq!(
        summary.parameters[2].mutation,
        PublicCallMutationEffect::NoWrite
    );
}

#[test]
fn immutable_shared_parameter_optional_transfer_remains_legal() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let reader_name = symbol("reader", &mut string_table);
    let value_name = symbol("value", &mut string_table);

    let reader = function_node(
        reader_name.clone(),
        FunctionSignature {
            parameters: vec![param(
                symbol("input", &mut string_table),
                DataType::Int,
                builtin_type_ids::INT,
                false,
                test_source_location(1),
            )],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
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
                    value_name.clone(),
                    Expression::int(1, test_source_location(2), ValueMode::ImmutableOwned),
                )),
                test_source_location(2),
            ),
            node(
                function_call_node(
                    reader_name,
                    vec![CallArgument::positional(
                        immutable_reference_expr(
                            value_name,
                            DataType::Int,
                            builtin_type_ids::INT,
                            test_source_location(3),
                        ),
                        CallAccessMode::Shared,
                        test_source_location(3),
                    )],
                    vec![],
                    test_source_location(3),
                ),
                test_source_location(3),
            ),
        ],
        test_source_location(2),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![reader, start], entry_path),
        &mut string_table,
    );
    run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("optional transfer of an immutable shared parameter should fall back to borrowing");
}

#[test]
fn path_dependent_optional_call_records_transfer_or_borrow_without_invalidating_source() {
    let source = r#"inspect |value String|:
;

optional_transfer |flag Bool, op_name String|:
    if flag:
        inspect(op_name)
    else
        inspect(op_name)
        retained = op_name
    ;
    sentinel = 0
;
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("path-dependent optional transfer should fall back to borrowing");

    let op_name_local = hir
        .blocks
        .iter()
        .flat_map(|block| block.locals.iter())
        .find(|local| hir.side_table.resolve_local_name(local.id, &string_table) == Some("op_name"))
        .map(|local| local.id)
        .expect("should locate the path-dependent parameter");

    assert!(
        report.analysis.value_facts.values().any(|fact| {
            fact.optional_transfer == OptionalTransferStatus::Transfer
                && fact.roots.contains(&op_name_local)
        }),
        "the final-use branch should record advisory transfer"
    );
    assert!(
        report.analysis.value_facts.values().any(|fact| {
            fact.optional_transfer == OptionalTransferStatus::Borrow
                && fact.roots.contains(&op_name_local)
        }),
        "the retaining branch should record a borrow fallback"
    );

    let sentinel_statement = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Assign {
                    target: HirPlace::Local(local),
                    ..
                } if hir.side_table.resolve_local_name(*local, &string_table) == Some("sentinel")
            )
        })
        .expect("should locate the post-join sentinel assignment");
    let op_name_snapshot = report
        .analysis
        .statement_entry_states
        .get(&sentinel_statement.id)
        .and_then(|state| {
            state
                .locals
                .iter()
                .find(|local| local.local == op_name_local)
        })
        .expect("post-join state should include the parameter");
    assert!(
        !op_name_snapshot.mode.is_definitely_uninit(),
        "optional transfer must not make the parameter unavailable at the join"
    );
    assert!(
        op_name_snapshot.mode.contains(LocalMode::SLOT),
        "post-join source state should retain its initialized slot"
    );
}

#[test]
fn unused_alias_before_final_shared_call_falls_back_to_borrow() {
    let source = r#"
inspect |value String|:
;

check |source String|:
    alias = source
    inspect(source)
;
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("an unused alias must not make a final shared call invalid");

    let source_local = hir
        .blocks
        .iter()
        .flat_map(|block| block.locals.iter())
        .find(|local| hir.side_table.resolve_local_name(local.id, &string_table) == Some("source"))
        .map(|local| local.id)
        .expect("should locate the source parameter");
    let call_argument = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| {
            let HirStatementKind::Call { args, .. } = &statement.kind else {
                return None;
            };
            let argument = args.first()?;
            matches!(
                &argument.kind,
                crate::compiler_frontend::hir::expressions::HirExpressionKind::Load(
                    HirPlace::Local(local)
                ) if *local == source_local
            )
            .then_some(argument.id)
        })
        .expect("should locate the final shared call argument");

    assert_eq!(
        report
            .analysis
            .value_fact(call_argument)
            .expect("the call argument should have a borrow fact")
            .optional_transfer,
        OptionalTransferStatus::Borrow,
        "failed transfer exclusivity must fall back to an ordinary shared borrow"
    );
}

#[test]
fn user_function_returning_param_alias_allows_caller_rebinding() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let alias_fn = symbol("alias_fn", &mut string_table);
    let p = symbol("p", &mut string_table);
    let x = symbol("x", &mut string_table);
    let y = symbol("y", &mut string_table);

    let callee = function_node(
        alias_fn.clone(),
        FunctionSignature {
            parameters: vec![param(
                p.clone(),
                DataType::Int,
                builtin_type_ids::INT,
                false,
                test_source_location(1),
            )],
            returns: vec![ReturnSlot {
                value: DataType::Int,
                type_id: Some(builtin_type_ids::INT),
                reactive_template: None,
                channel: ReturnChannel::Success,
            }],
        },
        vec![node(
            NodeKind::Return(vec![immutable_reference_expr(
                p,
                DataType::Int,
                builtin_type_ids::INT,
                test_source_location(2),
            )]),
            test_source_location(2),
        )],
        test_source_location(1),
    );

    let caller = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x.clone(),
                    Expression::int(1, test_source_location(10), ValueMode::MutableOwned),
                )),
                test_source_location(10),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    y,
                    Expression::function_call(
                        alias_fn,
                        vec![immutable_reference_expr(
                            x.clone(),
                            DataType::Int,
                            builtin_type_ids::INT,
                            test_source_location(11),
                        )],
                        vec![builtin_type_ids::INT],
                        test_source_location(11),
                    ),
                )),
                test_source_location(11),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        x,
                        DataType::Int,
                        builtin_type_ids::INT,
                        test_source_location(12),
                    ),
                    value: Expression::int(2, test_source_location(12), ValueMode::ImmutableOwned),
                },
                test_source_location(12),
            ),
        ],
        test_source_location(10),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![callee, caller], entry_path),
        &mut string_table,
    );
    run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("reassigning the source binding should detach it from the returned alias");
}

#[test]
fn slot_backed_alias_result_rebinding_to_another_alias_detaches_old_root() {
    let source = r#"Point = |
    x Int,
|

identity |value Point| -> Point:
    return value
;

original ~= Point(1)
returned ~= identity(original)
other ~= Point(2)
other_returned ~= identity(other)

returned = other_returned
original.x = 4
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("slot-backed result rebinding should detach its previous allocation root");
}

#[test]
fn fallible_alias_return_propagation_validates_success_alias_metadata() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let source_fn = symbol("source_fn", &mut string_table);
    let source_param = symbol("source_param", &mut string_table);
    let source_unused = symbol("source_unused", &mut string_table);
    let forward_fn = symbol("forward_fn", &mut string_table);
    let forward_param = symbol("forward_param", &mut string_table);
    let source_fn_name_for_lookup = source_fn.clone();
    let forward_fn_name_for_lookup = forward_fn.clone();

    let source = function_node(
        source_fn.clone(),
        FunctionSignature {
            parameters: vec![
                param(
                    source_param,
                    DataType::StringSlice,
                    builtin_type_ids::STRING,
                    false,
                    test_source_location(20),
                ),
                param(
                    source_unused.clone(),
                    DataType::StringSlice,
                    builtin_type_ids::STRING,
                    false,
                    test_source_location(20),
                ),
            ],
            returns: vec![
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    reactive_template: None,
                    channel: ReturnChannel::Success,
                },
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    reactive_template: None,
                    channel: ReturnChannel::Error,
                },
            ],
        },
        vec![node(
            NodeKind::Return(vec![immutable_reference_expr(
                source_unused,
                DataType::StringSlice,
                builtin_type_ids::STRING,
                test_source_location(21),
            )]),
            test_source_location(21),
        )],
        test_source_location(20),
    );

    let mut expression_types = TypeEnvironment::new();
    let propagated_call = Expression::handled_fallible_function_call_with_typed_arguments(
        source_fn,
        vec![
            CallArgument::positional(
                Expression::string_slice(
                    string_table.intern("unused"),
                    test_source_location(30),
                    ValueMode::ImmutableOwned,
                ),
                CallAccessMode::Shared,
                test_source_location(30),
            ),
            CallArgument::positional(
                immutable_reference_expr(
                    forward_param.clone(),
                    DataType::StringSlice,
                    builtin_type_ids::STRING,
                    test_source_location(30),
                ),
                CallAccessMode::Shared,
                test_source_location(30),
            ),
        ],
        vec![builtin_type_ids::STRING],
        FallibleExpressionHandling::Propagate,
        &mut expression_types,
        test_source_location(30),
    );

    let forward = function_node(
        forward_fn,
        FunctionSignature {
            parameters: vec![param(
                forward_param,
                DataType::StringSlice,
                builtin_type_ids::STRING,
                false,
                test_source_location(29),
            )],
            returns: vec![
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    reactive_template: None,
                    channel: ReturnChannel::Success,
                },
                ReturnSlot {
                    value: DataType::StringSlice,
                    type_id: Some(builtin_type_ids::STRING),
                    reactive_template: None,
                    channel: ReturnChannel::Error,
                },
            ],
        },
        vec![node(
            NodeKind::Return(vec![propagated_call]),
            test_source_location(30),
        )],
        test_source_location(29),
    );

    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), test_source_location(40))],
        test_source_location(40),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![source, forward, start], entry_path),
        &mut string_table,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("fallible alias forwarding should validate declared success alias metadata");

    for (function_name, expected_alias) in [
        (
            &source_fn_name_for_lookup,
            FunctionReturnAliasSummary::AliasParams(vec![1]),
        ),
        (
            &forward_fn_name_for_lookup,
            FunctionReturnAliasSummary::AliasParams(vec![0]),
        ),
    ] {
        let function_id = hir
            .functions
            .iter()
            .find(|function| {
                hir.side_table
                    .function_name_path(function.id)
                    .is_some_and(|path| path.name() == function_name.name())
            })
            .expect("fallible alias function should lower to HIR")
            .id;
        let summary = report
            .analysis
            .public_call_summaries
            .get(&function_id)
            .expect("fallible alias function should have a retained call summary");
        assert_eq!(summary.return_alias, expected_alias);
    }
}

#[test]
fn fresh_user_return_does_not_alias_caller_roots() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let fresh_fn = symbol("fresh_fn", &mut string_table);
    let fresh_fn_name_for_lookup = fresh_fn.clone();
    let p = symbol("p", &mut string_table);
    let x = symbol("x", &mut string_table);
    let y = symbol("y", &mut string_table);

    let callee = function_node(
        fresh_fn.clone(),
        FunctionSignature {
            parameters: vec![param(
                p,
                DataType::Int,
                builtin_type_ids::INT,
                false,
                test_source_location(1),
            )],
            returns: fresh_success_returns(vec![builtin_type_ids::INT]),
        },
        vec![node(
            NodeKind::Return(vec![Expression::int(
                42,
                test_source_location(2),
                ValueMode::ImmutableOwned,
            )]),
            test_source_location(2),
        )],
        test_source_location(1),
    );

    let caller = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x.clone(),
                    Expression::int(1, test_source_location(10), ValueMode::MutableOwned),
                )),
                test_source_location(10),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    y,
                    Expression::function_call(
                        fresh_fn,
                        vec![immutable_reference_expr(
                            x.clone(),
                            DataType::Int,
                            builtin_type_ids::INT,
                            test_source_location(11),
                        )],
                        vec![builtin_type_ids::INT],
                        test_source_location(11),
                    ),
                )),
                test_source_location(11),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        x,
                        DataType::Int,
                        builtin_type_ids::INT,
                        test_source_location(12),
                    ),
                    value: Expression::int(2, test_source_location(12), ValueMode::ImmutableOwned),
                },
                test_source_location(12),
            ),
        ],
        test_source_location(10),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![callee, caller], entry_path),
        &mut string_table,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("fresh callee returns should not alias caller roots");
    let fresh_function_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path.name() == fresh_fn_name_for_lookup.name())
        })
        .expect("fresh function should lower to HIR")
        .id;
    let fresh_summary = report
        .analysis
        .public_call_summaries
        .get(&fresh_function_id)
        .expect("fresh function should have a retained call summary");
    assert_eq!(
        fresh_summary.return_alias,
        FunctionReturnAliasSummary::Fresh
    );
}

#[test]
fn multi_return_fallible_external_retains_unknown_alias_summary() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let mut external_package_registry = default_external_package_registry(&mut string_table);
    let external_id = Arc::make_mut(&mut external_package_registry)
        .register_function(ExternalFunctionDef {
            name: "imprecise_external".to_owned(),
            parameters: vec![],
            returns: vec![
                ExternalReturnSlot::fresh(RegistryAbiType::I32),
                ExternalReturnSlot::fresh(RegistryAbiType::I32),
            ],
            error_return_type: Some(ExternalSignatureType::Abi(RegistryAbiType::I32)),
            lowerings: ExternalFunctionLowerings::default(),
        })
        .expect("fallible external fixture registration should succeed");

    let imprecise_return_name = symbol("imprecise_return", &mut string_table);
    let mut expression_types = TypeEnvironment::new();
    let fallible_external_call =
        Expression::handled_fallible_host_function_call_with_typed_arguments(
            HandledFallibleHostFunctionCallInput {
                id: external_id,
                args: vec![],
                result_type_ids: vec![builtin_type_ids::INT, builtin_type_ids::INT],
                error_type_id: builtin_type_ids::INT,
                handling: FallibleExpressionHandling::Propagate,
                location: test_source_location(2),
            },
            &mut expression_types,
        );
    let imprecise_return = function_node(
        imprecise_return_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                ReturnSlot {
                    value: DataType::Int,
                    type_id: Some(builtin_type_ids::INT),
                    reactive_template: None,
                    channel: ReturnChannel::Success,
                },
                ReturnSlot {
                    value: DataType::Int,
                    type_id: Some(builtin_type_ids::INT),
                    reactive_template: None,
                    channel: ReturnChannel::Success,
                },
                ReturnSlot {
                    value: DataType::Int,
                    type_id: Some(builtin_type_ids::INT),
                    reactive_template: None,
                    channel: ReturnChannel::Error,
                },
            ],
        },
        vec![node(
            NodeKind::Return(vec![fallible_external_call]),
            test_source_location(2),
        )],
        test_source_location(1),
    );
    let forward_name = symbol("forward_imprecise_return", &mut string_table);
    let mut forward_expression_types = TypeEnvironment::new();
    let forwarded_call = Expression::handled_fallible_function_call_with_typed_arguments(
        imprecise_return_name.clone(),
        vec![],
        vec![builtin_type_ids::INT, builtin_type_ids::INT],
        FallibleExpressionHandling::Propagate,
        &mut forward_expression_types,
        test_source_location(3),
    );
    let forward = function_node(
        forward_name.clone(),
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                ReturnSlot {
                    value: DataType::Int,
                    type_id: Some(builtin_type_ids::INT),
                    reactive_template: None,
                    channel: ReturnChannel::Success,
                },
                ReturnSlot {
                    value: DataType::Int,
                    type_id: Some(builtin_type_ids::INT),
                    reactive_template: None,
                    channel: ReturnChannel::Success,
                },
                ReturnSlot {
                    value: DataType::Int,
                    type_id: Some(builtin_type_ids::INT),
                    reactive_template: None,
                    channel: ReturnChannel::Error,
                },
            ],
        },
        vec![node(
            NodeKind::Return(vec![forwarded_call]),
            test_source_location(3),
        )],
        test_source_location(3),
    );
    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![],
        test_source_location(3),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![imprecise_return, forward, start], entry_path),
        &mut string_table,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("fallible external return should validate");
    let mut reversed_hir = hir.clone();
    reversed_hir.functions.reverse();
    let reversed_report =
        run_borrow_checker(&reversed_hir, &external_package_registry, &string_table)
            .expect("reversed fallible external return should validate");

    for analysis in [&report.analysis, &reversed_report.analysis] {
        for function_name in [&imprecise_return_name, &forward_name] {
            let function_id = hir
                .functions
                .iter()
                .find(|function| {
                    hir.side_table
                        .function_name_path(function.id)
                        .is_some_and(|path| path.name() == function_name.name())
                })
                .expect("imprecise return function should lower to HIR")
                .id;
            let summary = analysis
                .public_call_summaries
                .get(&function_id)
                .expect("forwarded imprecise function should have a retained call summary");
            assert_eq!(summary.return_alias, FunctionReturnAliasSummary::Unknown);
        }
    }
}

#[test]
fn checked_numeric_return_local_retains_fresh_alias_summary() {
    let source = r#"
increment |input Int| -> Int, Error!:
    return input + 1
;
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let increment_name = symbol("increment", &mut string_table);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("checked numeric return should validate");

    let function_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path.name() == increment_name.name())
        })
        .expect("numeric return function should lower to HIR")
        .id;
    let summary = report
        .analysis
        .public_call_summaries
        .get(&function_id)
        .expect("numeric return function should have a call summary");

    assert_eq!(summary.return_alias, FunctionReturnAliasSummary::Fresh);
}

#[test]
fn map_remove_return_local_retains_fresh_alias_summary() {
    let source = r#"
remove_value |scores ~{String = String}| -> String, Error!:
    return ~scores.remove("key")!
;
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let remove_value_name = symbol("remove_value", &mut string_table);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("map remove return should validate");

    let function_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path.name() == remove_value_name.name())
        })
        .expect("map return function should lower to HIR")
        .id;
    let summary = report
        .analysis
        .public_call_summaries
        .get(&function_id)
        .expect("map return function should have a call summary");

    assert_eq!(summary.return_alias, FunctionReturnAliasSummary::Fresh);
}

#[test]
fn multi_return_alias_summary_conservatively_unions_returned_elements() {
    let source = r#"
Point = |
    x Int,
|

pair |left Point, right Point| -> Point, Point:
    return left, right
;

duplicate |value Point| -> Point, Point:
    return value, value
;

copy_one |value Point| -> Point, Point:
    return value, copy value
;
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("multi-return alias summaries should validate");

    let summary_for = |name: &str| {
        let function = hir
            .functions
            .iter()
            .find(|function| {
                hir.side_table
                    .function_name_path(function.id)
                    .is_some_and(|path| path.name_str(&string_table) == Some(name))
            })
            .expect("multi-return function should lower to HIR");
        report
            .analysis
            .public_call_summaries
            .get(&function.id)
            .expect("multi-return function should have a call summary")
            .return_alias
            .clone()
    };

    assert_eq!(
        summary_for("pair"),
        FunctionReturnAliasSummary::AliasParams(vec![0, 1])
    );
    assert_eq!(
        summary_for("duplicate"),
        FunctionReturnAliasSummary::AliasParams(vec![0])
    );
    assert_eq!(
        summary_for("copy_one"),
        FunctionReturnAliasSummary::AliasParams(vec![0])
    );
}

#[test]
fn recursive_return_summary_stays_unknown() {
    let source = r#"
recursive |value Int| -> Int:
    return recursive(value)
;
"#;
    let (ast, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("recursive summary cycle should remain a valid conservative analysis");
    let recursive_id = hir
        .functions
        .iter()
        .find(|function| {
            hir.side_table
                .function_name_path(function.id)
                .is_some_and(|path| path.name_str(&string_table) == Some("recursive"))
        })
        .expect("recursive function should lower to HIR")
        .id;

    assert_eq!(
        report
            .analysis
            .public_call_summaries
            .get(&recursive_id)
            .expect("recursive function should have a call summary")
            .return_alias,
        FunctionReturnAliasSummary::Unknown
    );
}

#[test]
fn inferred_alias_return_from_parameter_reference_allows_caller_rebinding() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let unknown_fn = symbol("unknown_fn", &mut string_table);
    let p = symbol("p", &mut string_table);
    let q = symbol("q", &mut string_table);
    let x = symbol("x", &mut string_table);
    let y = symbol("y", &mut string_table);

    let callee = function_node(
        unknown_fn.clone(),
        FunctionSignature {
            parameters: vec![param(
                p.clone(),
                DataType::Int,
                builtin_type_ids::INT,
                false,
                test_source_location(1),
            )],
            returns: fresh_success_returns(vec![builtin_type_ids::INT]),
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    q.clone(),
                    immutable_reference_expr(
                        p,
                        DataType::Int,
                        builtin_type_ids::INT,
                        test_source_location(2),
                    ),
                )),
                test_source_location(2),
            ),
            node(
                NodeKind::Return(vec![immutable_reference_expr(
                    q,
                    DataType::Int,
                    builtin_type_ids::INT,
                    test_source_location(3),
                )]),
                test_source_location(3),
            ),
        ],
        test_source_location(1),
    );

    let caller = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x.clone(),
                    Expression::int(1, test_source_location(10), ValueMode::MutableOwned),
                )),
                test_source_location(10),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    y,
                    Expression::function_call(
                        unknown_fn,
                        vec![immutable_reference_expr(
                            x.clone(),
                            DataType::Int,
                            builtin_type_ids::INT,
                            test_source_location(11),
                        )],
                        vec![builtin_type_ids::INT],
                        test_source_location(11),
                    ),
                )),
                test_source_location(11),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        x,
                        DataType::Int,
                        builtin_type_ids::INT,
                        test_source_location(12),
                    ),
                    value: Expression::int(2, test_source_location(12), ValueMode::ImmutableOwned),
                },
                test_source_location(12),
            ),
        ],
        test_source_location(10),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![callee, caller], entry_path),
        &mut string_table,
    );
    run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("reassigning the source binding should preserve the returned allocation alias");
}

#[test]
fn mutable_user_argument_is_accepted_without_false_shared_conflict() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let mut_sink = symbol("mut_sink", &mut string_table);
    let p = symbol("p", &mut string_table);
    let x = symbol("x", &mut string_table);

    let callee = function_node(
        mut_sink.clone(),
        FunctionSignature {
            parameters: vec![param(
                p,
                DataType::Int,
                builtin_type_ids::INT,
                true,
                test_source_location(1),
            )],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );

    let caller = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x.clone(),
                    Expression::int(1, test_source_location(10), ValueMode::MutableOwned),
                )),
                test_source_location(10),
            ),
            node(
                function_call_node(
                    mut_sink,
                    vec![CallArgument::positional(
                        immutable_reference_expr(
                            x,
                            DataType::Int,
                            builtin_type_ids::INT,
                            test_source_location(11),
                        ),
                        CallAccessMode::Shared,
                        test_source_location(11),
                    )],
                    vec![],
                    test_source_location(11),
                ),
                test_source_location(11),
            ),
        ],
        test_source_location(10),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![callee, caller], entry_path),
        &mut string_table,
    );
    run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("single mutable argument call should be accepted");
}

#[test]
fn mutable_user_call_with_fresh_mutable_arg_does_not_alias_existing_place_argument() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let mut2 = symbol("mut2", &mut string_table);
    let a = symbol("a", &mut string_table);
    let b = symbol("b", &mut string_table);
    let x = symbol("x", &mut string_table);

    let callee = function_node(
        mut2.clone(),
        FunctionSignature {
            parameters: vec![
                param(
                    a,
                    DataType::Int,
                    builtin_type_ids::INT,
                    true,
                    test_source_location(1),
                ),
                param(
                    b,
                    DataType::Int,
                    builtin_type_ids::INT,
                    true,
                    test_source_location(1),
                ),
            ],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );

    let caller = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x.clone(),
                    Expression::int(1, test_source_location(10), ValueMode::MutableOwned),
                )),
                test_source_location(10),
            ),
            node(
                function_call_node(
                    mut2,
                    vec![
                        CallArgument::positional(
                            immutable_reference_expr(
                                x,
                                DataType::Int,
                                builtin_type_ids::INT,
                                test_source_location(11),
                            ),
                            CallAccessMode::Shared,
                            test_source_location(11),
                        ),
                        CallArgument::positional(
                            Expression::int(2, test_source_location(11), ValueMode::ImmutableOwned),
                            CallAccessMode::Shared,
                            test_source_location(11),
                        )
                        .with_passing_mode(CallPassingMode::FreshMutableValue),
                    ],
                    vec![],
                    test_source_location(11),
                ),
                test_source_location(11),
            ),
        ],
        test_source_location(10),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![callee, caller], entry_path),
        &mut string_table,
    );
    run_borrow_checker(&hir, &external_package_registry, &string_table).expect(
        "fresh mutable call args should be treated as independent locals, not aliases of other args",
    );
}

#[test]
fn host_mutable_parameter_requires_mutable_access() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let mut external_package_registry = default_external_package_registry(&mut string_table);
    let host_fn = register_external_function(
        &mut external_package_registry,
        "host_mut",
        vec![ExternalAccessKind::Mutable],
        ExternalReturnAlias::Fresh,
        ExternalAbiType::Void,
    );
    let x = symbol("x", &mut string_table);

    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x.clone(),
                    Expression::int(1, test_source_location(1), ValueMode::ImmutableOwned),
                )),
                test_source_location(1),
            ),
            node(
                host_function_call_node(
                    host_fn,
                    vec![CallArgument::positional(
                        immutable_reference_expr(
                            x,
                            DataType::Int,
                            builtin_type_ids::INT,
                            test_source_location(2),
                        ),
                        CallAccessMode::Shared,
                        test_source_location(2),
                    )],
                    vec![],
                    test_source_location(2),
                ),
                test_source_location(2),
            ),
        ],
        test_source_location(1),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start], entry_path),
        &mut string_table,
    );
    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect_err("mutable host parameter should enforce mutable access");
    assert_invalid_mutable_access_reason(&error, InvalidMutableAccessReason::ImmutablePlace);
}

#[test]
fn host_mutable_parameter_accepts_mutable_local_argument() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let mut external_package_registry = default_external_package_registry(&mut string_table);
    let host_fn = register_external_function(
        &mut external_package_registry,
        "host_mut_ok",
        vec![ExternalAccessKind::Mutable],
        ExternalReturnAlias::Fresh,
        ExternalAbiType::Void,
    );
    let x = symbol("x", &mut string_table);

    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x.clone(),
                    Expression::int(1, test_source_location(1), ValueMode::MutableOwned),
                )),
                test_source_location(1),
            ),
            node(
                host_function_call_node(
                    host_fn,
                    vec![CallArgument::positional(
                        immutable_reference_expr(
                            x,
                            DataType::Int,
                            builtin_type_ids::INT,
                            test_source_location(2),
                        ),
                        CallAccessMode::Shared,
                        test_source_location(2),
                    )],
                    vec![],
                    test_source_location(2),
                ),
                test_source_location(2),
            ),
        ],
        test_source_location(1),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start], entry_path),
        &mut string_table,
    );
    run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("mutable host parameter should accept mutable local argument");
}

#[test]
fn host_shared_parameter_is_shared_only() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let mut external_package_registry = default_external_package_registry(&mut string_table);
    let host_fn = register_external_function(
        &mut external_package_registry,
        "host_shared",
        vec![ExternalAccessKind::Shared],
        ExternalReturnAlias::Fresh,
        ExternalAbiType::Void,
    );
    let x = symbol("x", &mut string_table);

    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x.clone(),
                    Expression::int(1, test_source_location(1), ValueMode::ImmutableOwned),
                )),
                test_source_location(1),
            ),
            node(
                host_function_call_node(
                    host_fn,
                    vec![CallArgument::positional(
                        immutable_reference_expr(
                            x,
                            DataType::Int,
                            builtin_type_ids::INT,
                            test_source_location(2),
                        ),
                        CallAccessMode::Shared,
                        test_source_location(2),
                    )],
                    vec![],
                    test_source_location(2),
                ),
                test_source_location(2),
            ),
        ],
        test_source_location(1),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start], entry_path),
        &mut string_table,
    );
    run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("shared host parameter should not force mutable access");
}

#[test]
fn two_mutable_args_to_same_root_are_rejected() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let mut2 = symbol("mut2", &mut string_table);
    let a = symbol("a", &mut string_table);
    let b = symbol("b", &mut string_table);
    let x = symbol("x", &mut string_table);
    let y = symbol("y", &mut string_table);

    let callee = function_node(
        mut2.clone(),
        FunctionSignature {
            parameters: vec![
                param(
                    a,
                    DataType::Int,
                    builtin_type_ids::INT,
                    true,
                    test_source_location(1),
                ),
                param(
                    b,
                    DataType::Int,
                    builtin_type_ids::INT,
                    true,
                    test_source_location(1),
                ),
            ],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );

    let caller = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x.clone(),
                    Expression::int(1, test_source_location(10), ValueMode::MutableOwned),
                )),
                test_source_location(10),
            ),
            node(
                function_call_node(
                    mut2,
                    vec![
                        CallArgument::positional(
                            immutable_reference_expr(
                                x.clone(),
                                DataType::Int,
                                builtin_type_ids::INT,
                                test_source_location(11),
                            ),
                            CallAccessMode::Shared,
                            test_source_location(11),
                        ),
                        CallArgument::positional(
                            immutable_reference_expr(
                                x.clone(),
                                DataType::Int,
                                builtin_type_ids::INT,
                                test_source_location(11),
                            ),
                            CallAccessMode::Shared,
                            test_source_location(11),
                        ),
                    ],
                    vec![],
                    test_source_location(11),
                ),
                test_source_location(11),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    y,
                    immutable_reference_expr(
                        x,
                        DataType::Int,
                        builtin_type_ids::INT,
                        test_source_location(12),
                    ),
                )),
                test_source_location(12),
            ),
        ],
        test_source_location(10),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![callee, caller], entry_path),
        &mut string_table,
    );
    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect_err("two mutable args to the same root should fail");
    assert_borrow_error_kind(&error, BorrowDiagnosticKind::MultipleMutableBorrows);
}

#[test]
fn shared_then_mutable_args_to_same_root_are_rejected() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let read_then_mut = symbol("read_then_mut", &mut string_table);
    let read = symbol("read", &mut string_table);
    let mutate = symbol("mutate", &mut string_table);
    let x = symbol("x", &mut string_table);

    let callee = function_node(
        read_then_mut.clone(),
        FunctionSignature {
            parameters: vec![
                param(
                    read,
                    DataType::Int,
                    builtin_type_ids::INT,
                    false,
                    test_source_location(1),
                ),
                param(
                    mutate,
                    DataType::Int,
                    builtin_type_ids::INT,
                    true,
                    test_source_location(1),
                ),
            ],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );

    let caller = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x.clone(),
                    Expression::int(1, test_source_location(10), ValueMode::MutableOwned),
                )),
                test_source_location(10),
            ),
            node(
                function_call_node(
                    read_then_mut,
                    vec![
                        CallArgument::positional(
                            immutable_reference_expr(
                                x.clone(),
                                DataType::Int,
                                builtin_type_ids::INT,
                                test_source_location(11),
                            ),
                            CallAccessMode::Shared,
                            test_source_location(11),
                        ),
                        CallArgument::positional(
                            immutable_reference_expr(
                                x,
                                DataType::Int,
                                builtin_type_ids::INT,
                                test_source_location(11),
                            ),
                            CallAccessMode::Shared,
                            test_source_location(11),
                        ),
                    ],
                    vec![],
                    test_source_location(11),
                ),
                test_source_location(11),
            ),
        ],
        test_source_location(10),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![callee, caller], entry_path),
        &mut string_table,
    );
    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect_err("shared then mutable access to same root in a call must fail");
    let payload = assert_borrow_error_kind(&error, BorrowDiagnosticKind::SharedMutableConflict);
    let DiagnosticPayload::SharedMutableConflict {
        existing_access,
        requested_access,
        ..
    } = payload
    else {
        panic!("expected shared/mutable conflict payload, found {payload:?}");
    };
    assert_eq!(
        (*existing_access, *requested_access),
        (BorrowAccessKind::Shared, BorrowAccessKind::Mutable)
    );
}

#[test]
fn external_alias_args_result_stays_slot_backed_after_rebinding() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let mut external_package_registry = default_external_package_registry(&mut string_table);
    let host_alias = register_external_function(
        &mut external_package_registry,
        "alias_arg_result",
        vec![ExternalAccessKind::Shared],
        ExternalReturnAlias::AliasArgs(vec![0]),
        ExternalAbiType::I32,
    );
    let original = symbol("original", &mut string_table);
    let returned = symbol("returned", &mut string_table);

    let returned_call = Expression::host_function_call_with_arguments(
        host_alias,
        vec![CallArgument::positional(
            immutable_reference_expr(
                original.clone(),
                DataType::Int,
                builtin_type_ids::INT,
                test_source_location(11),
            ),
            CallAccessMode::Shared,
            test_source_location(11),
        )],
        vec![builtin_type_ids::INT],
        test_source_location(11),
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
                    original.clone(),
                    Expression::int(1, test_source_location(10), ValueMode::MutableOwned),
                )),
                test_source_location(10),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(returned.clone(), returned_call)),
                test_source_location(11),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        returned,
                        DataType::Int,
                        builtin_type_ids::INT,
                        test_source_location(12),
                    ),
                    value: Expression::int(2, test_source_location(12), ValueMode::ImmutableOwned),
                },
                test_source_location(12),
            ),
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        original,
                        DataType::Int,
                        builtin_type_ids::INT,
                        test_source_location(13),
                    ),
                    value: Expression::int(3, test_source_location(13), ValueMode::ImmutableOwned),
                },
                test_source_location(13),
            ),
        ],
        test_source_location(10),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start], entry_path),
        &mut string_table,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("external AliasArgs result should remain rebindable as a caller slot");
    let original_local = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Assign {
                target: HirPlace::Local(local),
                ..
            } if hir.side_table.resolve_local_name(*local, &string_table) == Some("original") => {
                Some(*local)
            }
            _ => None,
        })
        .expect("should locate the original binding");
    let returned_local = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Assign {
                target: HirPlace::Local(local),
                ..
            } if hir.side_table.resolve_local_name(*local, &string_table) == Some("returned") => {
                Some(*local)
            }
            _ => None,
        })
        .expect("should locate the returned binding");
    let returned_rebinding = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .rfind(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Assign {
                    target: HirPlace::Local(local),
                    ..
                } if *local == returned_local
            )
        })
        .expect("should locate the result rebinding statement");
    let returned_before_rebinding = report
        .analysis
        .statement_entry_states
        .get(&returned_rebinding.id)
        .expect("result rebinding should have an entry snapshot")
        .locals
        .iter()
        .find(|local| local.local == returned_local)
        .expect("result rebinding snapshot should include the external result");

    assert!(
        returned_before_rebinding.mode.contains(LocalMode::SLOT),
        "external AliasArgs result should use a caller slot before rebinding, got mode {:?} with roots {:?}",
        returned_before_rebinding.mode,
        returned_before_rebinding.alias_roots
    );
    assert!(!returned_before_rebinding.mode.contains(LocalMode::ALIAS));
    assert!(
        returned_before_rebinding
            .alias_roots
            .contains(&original_local),
        "external AliasArgs result should retain the argument root before rebinding, got mode {:?} with roots {:?}",
        returned_before_rebinding.mode,
        returned_before_rebinding.alias_roots
    );

    let original_assignment = hir
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .rfind(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Assign {
                    target: HirPlace::Local(local),
                    ..
                } if *local == original_local
            )
        })
        .expect("should locate the source rebinding statement");
    let snapshot = report
        .analysis
        .statement_entry_states
        .get(&original_assignment.id)
        .expect("source rebinding should have an entry snapshot");
    let returned_snapshot = snapshot
        .locals
        .iter()
        .find(|local| local.local == returned_local)
        .expect("source rebinding snapshot should include the external result");

    assert!(returned_snapshot.mode.contains(LocalMode::SLOT));
    assert!(!returned_snapshot.mode.contains(LocalMode::ALIAS));
    assert!(returned_snapshot.alias_roots.is_empty());
}

#[test]
fn unresolved_or_mismatched_host_signature_errors() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let mut external_package_registry = default_external_package_registry(&mut string_table);
    let one_arg = register_external_function(
        &mut external_package_registry,
        "one_arg",
        vec![ExternalAccessKind::Shared],
        ExternalReturnAlias::Fresh,
        ExternalAbiType::Void,
    );

    let missing_host =
        crate::compiler_frontend::external_packages::ExternalFunctionId::Synthetic(9999);

    let start_missing = function_node(
        symbol("start_missing", &mut string_table),
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            host_function_call_node(missing_host, vec![], vec![], test_source_location(1)),
            test_source_location(1),
        )],
        test_source_location(1),
    );

    let start_mismatch = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            host_function_call_node(one_arg, vec![], vec![], test_source_location(2)),
            test_source_location(2),
        )],
        test_source_location(2),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start_missing, start_mismatch], entry_path),
        &mut string_table,
    );

    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect_err("missing host or mismatched signature should fail");
    assert_infrastructure_error_contains(
        &error,
        ErrorType::Compiler,
        &["host call target", "argument count mismatch"],
    );
}

#[test]
fn mutable_user_parameter_rejects_immutable_argument_reused_after_call() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let mut_user = symbol("mut_user", &mut string_table);
    let p = symbol("p", &mut string_table);
    let x = symbol("x", &mut string_table);
    let y = symbol("y", &mut string_table);

    let callee = function_node(
        mut_user.clone(),
        FunctionSignature {
            parameters: vec![param(
                p,
                DataType::Int,
                builtin_type_ids::INT,
                true,
                test_source_location(1),
            )],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );

    let caller = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x.clone(),
                    Expression::int(1, test_source_location(10), ValueMode::ImmutableOwned),
                )),
                test_source_location(10),
            ),
            node(
                function_call_node(
                    mut_user,
                    vec![CallArgument::positional(
                        immutable_reference_expr(
                            x.clone(),
                            DataType::Int,
                            builtin_type_ids::INT,
                            test_source_location(11),
                        ),
                        CallAccessMode::Shared,
                        test_source_location(11),
                    )],
                    vec![],
                    test_source_location(11),
                ),
                test_source_location(11),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    y,
                    immutable_reference_expr(
                        x,
                        DataType::Int,
                        builtin_type_ids::INT,
                        test_source_location(12),
                    ),
                )),
                test_source_location(12),
            ),
        ],
        test_source_location(10),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![callee, caller], entry_path),
        &mut string_table,
    );
    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect_err("immutable argument passed to mutable user param and reused must fail");
    assert_invalid_mutable_access_reason(&error, InvalidMutableAccessReason::ImmutablePlace);
}

#[test]
fn out_of_range_return_alias_metadata_is_reported_at_call_site() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let mut external_package_registry = default_external_package_registry(&mut string_table);
    let bad_alias_host = register_external_function(
        &mut external_package_registry,
        "bad_alias_host",
        vec![ExternalAccessKind::Shared],
        ExternalReturnAlias::AliasArgs(vec![1]),
        ExternalAbiType::I32,
    );
    let x = symbol("x", &mut string_table);

    let start = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x.clone(),
                    Expression::int(1, test_source_location(10), ValueMode::MutableOwned),
                )),
                test_source_location(10),
            ),
            node(
                host_function_call_node(
                    bad_alias_host,
                    vec![CallArgument::positional(
                        immutable_reference_expr(
                            x,
                            DataType::Int,
                            builtin_type_ids::INT,
                            test_source_location(11),
                        ),
                        CallAccessMode::Shared,
                        test_source_location(11),
                    )],
                    vec![],
                    test_source_location(11),
                ),
                test_source_location(11),
            ),
        ],
        test_source_location(10),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start], entry_path),
        &mut string_table,
    );

    let error = run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect_err("out-of-range return alias metadata should fail at call site");
    assert_infrastructure_error_contains(
        &error,
        ErrorType::Compiler,
        &["out-of-range return-alias index"],
    );
}

#[test]
fn same_line_mutable_call_then_reuse_uses_order_keys() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let mut_user = symbol("mut_user", &mut string_table);
    let p = symbol("p", &mut string_table);
    let x = symbol("x", &mut string_table);
    let y = symbol("y", &mut string_table);

    let callee = function_node(
        mut_user.clone(),
        FunctionSignature {
            parameters: vec![param(
                p,
                DataType::Int,
                builtin_type_ids::INT,
                true,
                test_source_location(1),
            )],
            returns: vec![],
        },
        vec![],
        test_source_location(1),
    );

    // WHAT: both statements intentionally share one source line.
    // WHY: validates that borrow/move classification uses statement order keys, not line numbers.
    let same_line = test_source_location(20);
    let caller = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x.clone(),
                    Expression::int(1, test_source_location(10), ValueMode::MutableOwned),
                )),
                test_source_location(10),
            ),
            node(
                function_call_node(
                    mut_user,
                    vec![CallArgument::positional(
                        immutable_reference_expr(
                            x.clone(),
                            DataType::Int,
                            builtin_type_ids::INT,
                            same_line.clone(),
                        ),
                        CallAccessMode::Shared,
                        same_line.clone(),
                    )],
                    vec![],
                    same_line.clone(),
                ),
                same_line.clone(),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    y,
                    immutable_reference_expr(
                        x,
                        DataType::Int,
                        builtin_type_ids::INT,
                        same_line.clone(),
                    ),
                )),
                same_line,
            ),
        ],
        test_source_location(10),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![callee, caller], entry_path),
        &mut string_table,
    );
    run_borrow_checker(&hir, &external_package_registry, &string_table).expect(
        "same-line mutable call + later reuse should borrow (not move) when ordered by statement",
    );
}

#[test]
fn short_circuit_rhs_mutable_call_with_later_merge_use_borrows_instead_of_moving() {
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let rhs_name = symbol("rhs_short", &mut string_table);
    let calls = symbol("calls", &mut string_table);
    let lhs = symbol("lhs", &mut string_table);
    let value = symbol("value", &mut string_table);
    let sink = symbol("sink", &mut string_table);
    let param_calls = symbol("param_calls", &mut string_table);

    let rhs_function = function_node(
        rhs_name.clone(),
        FunctionSignature {
            parameters: vec![param(
                param_calls.clone(),
                DataType::Int,
                builtin_type_ids::INT,
                true,
                test_source_location(1),
            )],
            returns: fresh_success_returns(vec![builtin_type_ids::BOOL]),
        },
        vec![
            node(
                NodeKind::Assignment {
                    target: assignment_target(
                        param_calls.clone(),
                        DataType::Int,
                        builtin_type_ids::INT,
                        test_source_location(2),
                    ),
                    value: Expression::runtime(
                        ExpressionRpn {
                            items: vec![
                                ExpressionRpnItem::Operand(Expression::reference_with_type_id(
                                    param_calls,
                                    DataType::Int,
                                    builtin_type_ids::INT,
                                    test_source_location(2),
                                    ValueMode::MutableOwned,
                                    crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
                                )),
                                ExpressionRpnItem::Operand(Expression::int(
                                    1,
                                    test_source_location(2),
                                    ValueMode::ImmutableOwned,
                                )),
                                ExpressionRpnItem::Operator {
                                    operator: Operator::Add,
                                    location: test_source_location(2),
                                },
                            ],
                        },
                        DataType::Int,
                        test_source_location(2),
                        ValueMode::MutableOwned,
                    ),
                },
                test_source_location(2),
            ),
            node(
                NodeKind::Return(vec![Expression::bool(
                    true,
                    test_source_location(3),
                    ValueMode::ImmutableOwned,
                )]),
                test_source_location(3),
            ),
        ],
        test_source_location(1),
    );

    let short_circuit_value = Expression::runtime(
        ExpressionRpn {
            items: vec![
                ExpressionRpnItem::Operand(Expression::reference_with_type_id(
                    lhs.clone(),
                    DataType::Bool,
                    builtin_type_ids::BOOL,
                    test_source_location(11),
                    ValueMode::ImmutableOwned,
                    crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
                )),
                ExpressionRpnItem::Operand(Expression::function_call(
                    rhs_name,
                    vec![Expression::reference_with_type_id(
                        calls.clone(),
                        DataType::Int,
                        builtin_type_ids::INT,
                        test_source_location(11),
                        ValueMode::MutableOwned,
                        crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState::RuntimeValue,
                    )],
                    vec![builtin_type_ids::BOOL],
                    test_source_location(11),
                )),
                ExpressionRpnItem::Operator {
                    operator: Operator::And,
                    location: test_source_location(11),
                },
            ],
        },
        DataType::Bool,
        test_source_location(11),
        ValueMode::ImmutableOwned,
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    lhs,
                    Expression::bool(false, test_source_location(10), ValueMode::ImmutableOwned),
                )),
                test_source_location(10),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    calls.clone(),
                    Expression::int(0, test_source_location(10), ValueMode::MutableOwned),
                )),
                test_source_location(10),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(value, short_circuit_value)),
                test_source_location(11),
            ),
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    sink,
                    Expression::reference(
                        calls,
                        DataType::Int,
                        test_source_location(12),
                        ValueMode::ImmutableReference,
                    ),
                )),
                test_source_location(12),
            ),
        ],
        test_source_location(10),
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![rhs_function, start_function], entry_path),
        &mut string_table,
    );
    run_borrow_checker(&hir, &external_package_registry, &string_table)
        .expect("rhs-only mutable short-circuit call with later merge use should stay borrowed");
}
