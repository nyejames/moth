//! HIR result-handling lowering regression tests.
//!
//! WHAT: checks how result propagation, multi-bind fallbacks, and error handlers lower into HIR
//!       control-flow and local bindings.
//! WHY: fallible handling spans expression and statement boundaries; targeted tests prevent
//!      regressions in error-path routing and fallback binding.

use crate::compiler_frontend::ast::ast_nodes::{MultiBindTargetKind, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::expressions::expression_types::{
    CatchErrorBinding, FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::failure_facts::FailureDisposition;
use crate::compiler_frontend::ast::statements::fallible_handling::wrap_catch_expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::value_production::{
    ProducedValues,
    types::{ValueBlock, ValueIfBlock},
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalPackageRegistry};
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::hir_builder::{build_ast_with_registered_types, lower_ast};
use crate::compiler_frontend::hir::numeric::HirNumericOp;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    assignment_target, fresh_success_returns, function_node, make_test_variable, node,
    param_with_type_id, reference_expr_with_type_id, success_return_slot,
};
use crate::compiler_frontend::tests::type_id_fixture_support::{
    error_return_slot, multi_bind_target, runtime_expr, runtime_handled_function_call_item,
    runtime_operand_item, runtime_operator_item,
};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

/// One exact `I32` literal expression.
///
/// WHY: these fixtures pin the exact `I32` width explicitly at the scalar carrier.
fn i32_literal(
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    value: i64,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
    value_mode: ValueMode,
) -> ExprId {
    let value = FixedScalarValue::signed(FixedScalar::I32, value).expect("test literal fits I32");
    builder
        .fixed_scalar(constants, value, span, value_mode)
        .expect("fixture scalar")
        .id()
}

/// Follow the per-slot snapshot that prevents earlier multi-bind writes changing later inputs.
fn staged_result_local(
    module: &crate::compiler_frontend::hir::module::HirModule,
    block: &crate::compiler_frontend::hir::blocks::HirBlock,
    result: crate::compiler_frontend::hir::ids::LocalId,
) -> crate::compiler_frontend::hir::ids::LocalId {
    let snapshots = block
        .statements
        .iter()
        .filter_map(|statement| {
            let HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                value,
            } = &statement.kind
            else {
                return None;
            };
            matches!(&module.expressions.expression(*value).kind,
            HirExpressionKind::Load(place)
                if place.root == result && super::is_local_place(place))
            .then_some(*local)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        snapshots.len(),
        1,
        "each native result slot should be staged once"
    );
    snapshots[0]
}

#[test]
fn statement_result_propagation_with_unit_success_lowers_to_explicit_error_edge() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let can_fail_name = super::symbol("can_fail", &mut path_fork, &mut string_table);
    let location = None;

    let boom = builder
        .string_slice(
            &mut constants,
            string_table.intern("boom"),
            location,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string fits expression store")
        .id();
    let can_fail_function = function_node(
        can_fail_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![error_return_slot(builtin_type_ids::STRING)],
        },
        vec![node(NodeKind::ReturnError(boom), location)],
        location,
    );

    let propagated_call = builder
        .handled_fallible_function_call_with_typed_arguments(
            can_fail_name,
            vec![],
            vec![],
            builtin_type_ids::STRING,
            FallibleExpressionHandling::Propagate,
            location,
        )
        .expect("fixture handled call fits expression store")
        .id();
    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![error_return_slot(builtin_type_ids::STRING)],
        },
        vec![node(
            NodeKind::ExpressionStatement(propagated_call),
            location,
        )],
        location,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![can_fail_function, start_function],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("statement propagation lowering should succeed");

    let start_function = module
        .functions
        .iter()
        .find(|function| Some(function.id) == module.start_function)
        .expect("start function should exist");
    let start_entry = module
        .blocks
        .iter()
        .find(|block| block.id == start_function.entry)
        .expect("start entry block should exist");

    let HirTerminator::Invoke {
        success_results,
        failure: error_block,
        ..
    } = &start_entry.terminator
    else {
        panic!("zero-success statement propagation should lower to Invoke");
    };
    assert!(success_results.is_empty());

    let error_block = module
        .blocks
        .iter()
        .find(|block| block.id == *error_block)
        .expect("statement propagation should create an error block");
    assert!(
        matches!(error_block.terminator, HirTerminator::ReturnError(_)),
        "statement propagation error edge should return through the enclosing error slot"
    );
}

#[test]
fn direct_return_result_propagation_lowers_to_explicit_success_and_error_edges() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let can_fail_name = super::symbol("can_fail", &mut path_fork, &mut string_table);
    let forward_name = super::symbol("forward", &mut path_fork, &mut string_table);
    let location = None;

    let ok_value = builder
        .string_slice(
            &mut constants,
            string_table.intern("ok"),
            location,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string fits expression store")
        .id();
    let can_fail_function = function_node(
        can_fail_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(NodeKind::Return(vec![ok_value]), location)],
        location,
    );

    let propagated_call = builder
        .handled_fallible_function_call_with_typed_arguments(
            can_fail_name,
            vec![],
            vec![builtin_type_ids::STRING],
            builtin_type_ids::STRING,
            FallibleExpressionHandling::Propagate,
            location,
        )
        .expect("fixture handled call fits expression store")
        .id();

    let forward_function = function_node(
        forward_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(NodeKind::Return(vec![propagated_call]), location)],
        location,
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location)],
        location,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![can_fail_function, forward_function, start_function],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("direct return propagation lowering should succeed");

    let forward_function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == forward_name)
        })
        .expect("forward function should exist");
    let forward_entry = module
        .blocks
        .iter()
        .find(|block| block.id == forward_function.entry)
        .expect("forward entry block should exist");

    let HirTerminator::Invoke {
        success: success_block,
        success_results,
        failure: error_block,
        error_result,
        ..
    } = &forward_entry.terminator
    else {
        panic!("direct return propagation should lower to Invoke");
    };
    assert_eq!(success_results.len(), 1);
    assert!(!success_results.contains(error_result));

    let success_block = module
        .blocks
        .iter()
        .find(|block| block.id == *success_block)
        .expect("direct return propagation should create a success block");
    let HirTerminator::Return(success_values) = &success_block.terminator else {
        panic!("success edge should return its ordered values");
    };
    let success_values = module.expressions.values(*success_values);
    assert_eq!(success_values.len(), 1);
    assert!(
        matches!(
            &module.expressions.expression(success_values[0]).kind,
            HirExpressionKind::Load(place)
                if place.root == success_results[0] && super::is_local_place(place)
        ),
        "success edge should forward the native success local"
    );

    let error_block = module
        .blocks
        .iter()
        .find(|block| block.id == *error_block)
        .expect("direct return propagation should create an error block");
    let HirTerminator::ReturnError(error_value) = &error_block.terminator else {
        panic!("error edge should return through ReturnError");
    };
    assert!(
        matches!(
            &module.expressions.expression(*error_value).kind,
            HirExpressionKind::Load(place)
                if place.root == *error_result && super::is_local_place(place)
        ),
        "error edge should forward the native error local"
    );
}

#[test]
fn direct_return_result_propagation_allows_alias_success_return() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let constants = ConstValueStore::default();
    let source_name = super::symbol("source", &mut path_fork, &mut string_table);
    let source_input = super::symbol("input", &mut path_fork, &mut string_table);
    let forward_name = super::symbol("forward", &mut path_fork, &mut string_table);
    let forward_input = super::symbol("input", &mut path_fork, &mut string_table);
    let location = None;

    let source_input_ref = reference_expr_with_type_id(
        &mut builder,
        source_input,
        builtin_type_ids::STRING,
        location,
        ValueMode::ImmutableReference,
    );
    let source_function = function_node(
        source_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                source_input,
                builtin_type_ids::STRING,
                false,
                location,
            )],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(NodeKind::Return(vec![source_input_ref]), location)],
        location,
    );

    let forward_input_ref = reference_expr_with_type_id(
        &mut builder,
        forward_input,
        builtin_type_ids::STRING,
        location,
        ValueMode::ImmutableReference,
    );
    let propagated_call = builder
        .handled_fallible_function_call_with_typed_arguments(
            source_name,
            vec![CallArgument::positional(
                builder.own(forward_input_ref),
                CallAccessMode::Shared,
                location,
            )],
            vec![builtin_type_ids::STRING],
            builtin_type_ids::STRING,
            FallibleExpressionHandling::Propagate,
            location,
        )
        .expect("fixture handled call fits expression store")
        .id();

    let forward_function = function_node(
        forward_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                forward_input,
                builtin_type_ids::STRING,
                false,
                location,
            )],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(NodeKind::Return(vec![propagated_call]), location)],
        location,
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location)],
        location,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![source_function, forward_function, start_function],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("direct alias-return propagation lowering should succeed");

    let forward_function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == forward_name)
        })
        .expect("forward function should exist");

    let forward_entry = module
        .blocks
        .iter()
        .find(|block| block.id == forward_function.entry)
        .expect("forward entry block should exist");
    let HirTerminator::Invoke {
        success: success_block,
        success_results,
        ..
    } = &forward_entry.terminator
    else {
        panic!("alias-return propagation should lower to Invoke");
    };
    assert_eq!(success_results.len(), 1);

    let success_block = module
        .blocks
        .iter()
        .find(|block| block.id == *success_block)
        .expect("alias-return propagation should create a success block");
    let HirTerminator::Return(success_values) = &success_block.terminator else {
        panic!("alias success edge should return ordered values");
    };
    let success_values = module.expressions.values(*success_values);
    assert_eq!(success_values.len(), 1);
    assert!(
        matches!(
            &module.expressions.expression(success_values[0]).kind,
            HirExpressionKind::Load(place)
                if place.root == success_results[0] && super::is_local_place(place)
        ),
        "alias success edge should forward its native result binding without copying"
    );
}

#[test]
fn declaration_result_propagation_assigns_native_success_on_success_edge() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let can_fail_name = super::symbol("can_fail", &mut path_fork, &mut string_table);
    let forward_name = super::symbol("forward", &mut path_fork, &mut string_table);
    let value_name = path_fork
        .try_intern_child(forward_name, string_table.intern("value"))
        .expect("test path fits");
    let location = None;

    let ok_value = builder
        .string_slice(
            &mut constants,
            string_table.intern("ok"),
            location,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string fits expression store")
        .id();
    let can_fail_function = function_node(
        can_fail_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(NodeKind::Return(vec![ok_value]), location)],
        location,
    );

    let propagated_call = builder
        .handled_fallible_function_call_with_typed_arguments(
            can_fail_name,
            vec![],
            vec![builtin_type_ids::STRING],
            builtin_type_ids::STRING,
            FallibleExpressionHandling::Propagate,
            location,
        )
        .expect("fixture handled call fits expression store")
        .id();

    let declared_value = make_test_variable(value_name, propagated_call, &builder);
    let forward_value_ref = reference_expr_with_type_id(
        &mut builder,
        value_name,
        builtin_type_ids::STRING,
        location,
        ValueMode::ImmutableReference,
    );
    let forward_function = function_node(
        forward_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![
            node(NodeKind::VariableDeclaration(declared_value), location),
            node(NodeKind::Return(vec![forward_value_ref]), location),
        ],
        location,
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location)],
        location,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![can_fail_function, forward_function, start_function],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("declaration result propagation lowering should succeed");

    let forward_function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == forward_name)
        })
        .expect("forward function should exist");
    let forward_entry = module
        .blocks
        .iter()
        .find(|block| block.id == forward_function.entry)
        .expect("forward entry block should exist");

    let HirTerminator::Invoke {
        success: success_block,
        success_results,
        failure: error_block,
        ..
    } = &forward_entry.terminator
    else {
        panic!("declaration propagation should lower to Invoke");
    };
    assert_eq!(success_results.len(), 1);

    let success_block = module
        .blocks
        .iter()
        .find(|block| block.id == *success_block)
        .expect("declaration propagation should create a success block");
    assert!(
        success_block.statements.iter().any(|statement| {
            super::write_value(&module, statement).is_some_and(|value| {
                matches!(&value.kind, HirExpressionKind::Load(place)
                    if place.root == success_results[0] && super::is_local_place(place))
            })
        }),
        "success edge should assign the native success local to the declaration local"
    );

    let error_block = module
        .blocks
        .iter()
        .find(|block| block.id == *error_block)
        .expect("declaration propagation should create an error block");
    assert!(
        matches!(error_block.terminator, HirTerminator::ReturnError(_)),
        "declaration propagation error edge should return through the enclosing error slot"
    );
}

#[test]
fn multi_bind_result_propagation_binds_ordered_slots_on_success_edge() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let pair_name = super::symbol("pair", &mut path_fork, &mut string_table);
    let forward_name = super::symbol("forward", &mut path_fork, &mut string_table);
    let first_id = path_fork
        .try_intern_child(forward_name, string_table.intern("first"))
        .expect("test path fits");
    let count_id = path_fork
        .try_intern_child(forward_name, string_table.intern("count"))
        .expect("test path fits");
    let location = None;

    let ok_value = builder
        .string_slice(
            &mut constants,
            string_table.intern("ok"),
            location,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string fits expression store")
        .id();
    let two = i32_literal(
        &mut builder,
        &mut constants,
        2,
        location,
        ValueMode::ImmutableOwned,
    );
    let pair_function = function_node(
        pair_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                success_return_slot(builtin_type_ids::fixed_scalar(FixedScalar::I32)),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(NodeKind::Return(vec![ok_value, two]), location)],
        location,
    );

    let propagated_call = builder
        .handled_fallible_function_call_with_typed_arguments(
            pair_name,
            vec![],
            vec![
                builtin_type_ids::STRING,
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
            ],
            builtin_type_ids::STRING,
            FallibleExpressionHandling::Propagate,
            location,
        )
        .expect("fixture handled call fits expression store")
        .id();

    let first_ref = reference_expr_with_type_id(
        &mut builder,
        first_id,
        builtin_type_ids::STRING,
        location,
        ValueMode::ImmutableReference,
    );
    let count_ref = reference_expr_with_type_id(
        &mut builder,
        count_id,
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        location,
        ValueMode::ImmutableReference,
    );
    let forward_function = function_node(
        forward_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                success_return_slot(builtin_type_ids::fixed_scalar(FixedScalar::I32)),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![
            node(
                NodeKind::MultiBind {
                    targets: vec![
                        multi_bind_target(
                            first_id,
                            builtin_type_ids::STRING,
                            ValueMode::ImmutableOwned,
                            MultiBindTargetKind::Declaration,
                            location,
                        ),
                        multi_bind_target(
                            count_id,
                            builtin_type_ids::fixed_scalar(FixedScalar::I32),
                            ValueMode::ImmutableOwned,
                            MultiBindTargetKind::Declaration,
                            location,
                        ),
                    ],
                    value: propagated_call,
                },
                location,
            ),
            node(NodeKind::Return(vec![first_ref, count_ref]), location),
        ],
        location,
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location)],
        location,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![pair_function, forward_function, start_function],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("multi-bind result propagation lowering should succeed");

    let forward_function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == forward_name)
        })
        .expect("forward function should exist");
    let forward_entry = module
        .blocks
        .iter()
        .find(|block| block.id == forward_function.entry)
        .expect("forward entry block should exist");

    let HirTerminator::Invoke {
        success: success_block,
        success_results,
        ..
    } = &forward_entry.terminator
    else {
        panic!("multi-bind propagation should lower to Invoke");
    };
    assert_eq!(success_results.len(), 2);

    let success_block = module
        .blocks
        .iter()
        .find(|block| block.id == *success_block)
        .expect("multi-bind propagation should create a success block");
    for (name, result) in ["first", "count"].iter().zip(success_results) {
        let staged = staged_result_local(&module, success_block, *result);
        let definitions = success_block
            .statements
            .iter()
            .filter(|statement| {
                let HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(local),
                    value,
                } = &statement.kind
                else {
                    return false;
                };
                module
                    .side_table
                    .resolve_local_name(*local, &path_fork, &string_table)
                    == Some(*name)
                    && matches!(&module.expressions.expression(*value).kind,
                    HirExpressionKind::Load(place)
                        if place.root == staged && super::is_local_place(place))
            })
            .count();
        assert_eq!(
            definitions, 1,
            "success slot should define {name} once in order"
        );
    }
}

#[test]
fn call_argument_result_propagation_lowers_before_outer_call() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let can_fail_name = super::symbol("can_fail", &mut path_fork, &mut string_table);
    let consume_name = super::symbol("consume", &mut path_fork, &mut string_table);
    let consume_input = path_fork
        .try_intern_child(consume_name, string_table.intern("input"))
        .expect("test path fits");
    let forward_name = super::symbol("forward", &mut path_fork, &mut string_table);
    let value_name = path_fork
        .try_intern_child(forward_name, string_table.intern("value"))
        .expect("test path fits");
    let location = None;

    let ok_value = builder
        .string_slice(
            &mut constants,
            string_table.intern("ok"),
            location,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string fits expression store")
        .id();
    let can_fail_function = function_node(
        can_fail_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(NodeKind::Return(vec![ok_value]), location)],
        location,
    );

    let consume_input_ref = reference_expr_with_type_id(
        &mut builder,
        consume_input,
        builtin_type_ids::STRING,
        location,
        ValueMode::ImmutableReference,
    );
    let consume_function = function_node(
        consume_name,
        FunctionSignature {
            parameters: vec![param_with_type_id(
                consume_input,
                builtin_type_ids::STRING,
                false,
                location,
            )],
            returns: vec![success_return_slot(builtin_type_ids::STRING)],
        },
        vec![node(NodeKind::Return(vec![consume_input_ref]), location)],
        location,
    );

    let propagated_call = builder
        .handled_fallible_function_call_with_typed_arguments(
            can_fail_name,
            vec![],
            vec![builtin_type_ids::STRING],
            builtin_type_ids::STRING,
            FallibleExpressionHandling::Propagate,
            location,
        )
        .expect("fixture handled call fits expression store")
        .id();
    let outer_call = builder
        .function_call_with_typed_arguments(
            consume_name,
            vec![CallArgument::positional(
                builder.own(propagated_call),
                CallAccessMode::Shared,
                location,
            )],
            vec![builtin_type_ids::STRING],
            location,
        )
        .expect("fixture outer call fits expression store")
        .id();

    let declared_value = make_test_variable(value_name, outer_call, &builder);
    let forward_value_ref = reference_expr_with_type_id(
        &mut builder,
        value_name,
        builtin_type_ids::STRING,
        location,
        ValueMode::ImmutableReference,
    );
    let forward_function = function_node(
        forward_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![
            node(NodeKind::VariableDeclaration(declared_value), location),
            node(NodeKind::Return(vec![forward_value_ref]), location),
        ],
        location,
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location)],
        location,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![
                can_fail_function,
                consume_function,
                forward_function,
                start_function,
            ],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("call-argument propagation lowering should succeed");

    let forward_function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == forward_name)
        })
        .expect("forward function should exist");
    let forward_entry = module
        .blocks
        .iter()
        .find(|block| block.id == forward_function.entry)
        .expect("forward entry block should exist");
    let HirTerminator::Invoke {
        success: success_block,
        ..
    } = &forward_entry.terminator
    else {
        panic!("call argument propagation should invoke before the outer call");
    };
    let success_block = module
        .blocks
        .iter()
        .find(|block| block.id == *success_block)
        .expect("call argument propagation should create a success block");

    let entry_calls_consume = forward_entry.statements.iter().any(|statement| {
        let HirStatementKind::Call {
            target: CallTarget::Local(function_id),
            ..
        } = statement.kind
        else {
            return false;
        };

        module
            .side_table
            .function_name_path(function_id)
            .is_some_and(|path| path == consume_name)
    });
    let success_calls_consume = success_block.statements.iter().any(|statement| {
        let HirStatementKind::Call {
            target: CallTarget::Local(function_id),
            ..
        } = statement.kind
        else {
            return false;
        };

        module
            .side_table
            .function_name_path(function_id)
            .is_some_and(|path| path == consume_name)
    });

    assert!(
        !entry_calls_consume && success_calls_consume,
        "the outer call must lower only after fallible argument propagation succeeds"
    );
}

#[test]
fn runtime_binary_result_propagation_lowers_before_operator() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let can_fail_name = super::symbol("can_fail", &mut path_fork, &mut string_table);
    let forward_name = super::symbol("forward", &mut path_fork, &mut string_table);
    let value_name = path_fork
        .try_intern_child(forward_name, string_table.intern("value"))
        .expect("test path fits");
    let location = None;

    let forty_one = i32_literal(
        &mut builder,
        &mut constants,
        41,
        location,
        ValueMode::ImmutableOwned,
    );
    let can_fail_function = function_node(
        can_fail_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::fixed_scalar(FixedScalar::I32)),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(NodeKind::Return(vec![forty_one]), location)],
        location,
    );

    let call_item = runtime_handled_function_call_item(
        &mut builder,
        can_fail_name,
        vec![builtin_type_ids::fixed_scalar(FixedScalar::I32)],
        builtin_type_ids::STRING,
        FallibleHandling::Propagate,
        location,
    );
    let one = i32_literal(
        &mut builder,
        &mut constants,
        1,
        location,
        ValueMode::ImmutableOwned,
    );
    let runtime_value = runtime_expr(
        &mut builder,
        &mut constants,
        vec![
            call_item,
            runtime_operand_item(one),
            runtime_operator_item(Operator::Add, location),
        ],
        builtin_type_ids::fixed_scalar(FixedScalar::I32),
        location,
        ValueMode::MutableOwned,
        &TypeEnvironment::new(),
    );

    let forward_function = function_node(
        forward_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::fixed_scalar(FixedScalar::I32)),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    value_name,
                    runtime_value,
                    &builder,
                )),
                location,
            ),
            node(
                NodeKind::Return(vec![reference_expr_with_type_id(
                    &mut builder,
                    value_name,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    location,
                    ValueMode::ImmutableReference,
                )]),
                location,
            ),
        ],
        location,
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location)],
        location,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![can_fail_function, forward_function, start_function],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("runtime binary propagation lowering should succeed");

    let forward_function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == forward_name)
        })
        .expect("forward function should exist");
    let forward_entry = module
        .blocks
        .iter()
        .find(|block| block.id == forward_function.entry)
        .expect("forward entry block should exist");
    let HirTerminator::Invoke {
        success: success_block,
        ..
    } = &forward_entry.terminator
    else {
        panic!("runtime binary propagation should invoke before the numeric operator");
    };
    let success_block = module
        .blocks
        .iter()
        .find(|block| block.id == *success_block)
        .expect("runtime binary propagation should create a success block");

    let entry_has_numeric_op = forward_entry
        .statements
        .iter()
        .any(|statement| matches!(statement.kind, HirStatementKind::NumericOp { .. }));
    let success_has_add = matches!(
        &success_block.terminator,
        HirTerminator::Invoke {
            operation: HirFallibleOperation::NumericOp {
                op: HirNumericOp {
                    operator: NumericOperator::Add,
                    domain: NumericScalar::Fixed(FixedScalar::I32),
                },
                ..
            },
            ..
        }
    ) || success_block.statements.iter().any(|statement| {
        matches!(
            &statement.kind,
            HirStatementKind::NumericOp {
                op: HirNumericOp {
                    operator: NumericOperator::Add,
                    domain: NumericScalar::Fixed(FixedScalar::I32),
                },
                ..
            }
        )
    });

    assert!(
        !entry_has_numeric_op && success_has_add,
        "the numeric operator must lower only after fallible operand propagation succeeds"
    );
}

#[test]
fn return_bang_lowers_to_explicit_error_terminator() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let can_fail_name = super::symbol("can_fail", &mut path_fork, &mut string_table);
    let location = None;

    let boom = builder
        .string_slice(
            &mut constants,
            string_table.intern("boom"),
            location,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string fits expression store")
        .id();
    let can_fail_function = function_node(
        can_fail_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![error_return_slot(builtin_type_ids::STRING)],
        },
        vec![node(NodeKind::ReturnError(boom), location)],
        location,
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location)],
        location,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![can_fail_function, start_function],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("return! lowering should succeed");

    let can_fail_function = module
        .functions
        .iter()
        .find(|function| Some(function.id) != module.start_function)
        .expect("fallible function should exist");
    let can_fail_entry = module
        .blocks
        .iter()
        .find(|block| block.id == can_fail_function.entry)
        .expect("fallible function entry block should exist");

    assert!(
        matches!(can_fail_entry.terminator, HirTerminator::ReturnError(_)),
        "return! should lower to a dedicated HIR error-return terminator"
    );
}

#[test]
fn fallible_success_return_preserves_success_slots_and_error_signature() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let can_succeed_name = super::symbol("can_succeed", &mut path_fork, &mut string_table);
    let location = None;

    let ok_value = builder
        .string_slice(
            &mut constants,
            string_table.intern("ok"),
            location,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string fits expression store")
        .id();
    let can_succeed_function = function_node(
        can_succeed_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![
                success_return_slot(builtin_type_ids::STRING),
                error_return_slot(builtin_type_ids::STRING),
            ],
        },
        vec![node(NodeKind::Return(vec![ok_value]), location)],
        location,
    );

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), location)],
        location,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![can_succeed_function, start_function],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("fallible success return lowering should succeed");

    let can_succeed_function = module
        .functions
        .iter()
        .find(|function| Some(function.id) != module.start_function)
        .expect("fallible function should exist");
    let can_succeed_entry = module
        .blocks
        .iter()
        .find(|block| block.id == can_succeed_function.entry)
        .expect("fallible function entry block should exist");

    assert_eq!(
        can_succeed_function.returns.as_ref(),
        &[builtin_type_ids::STRING]
    );
    assert_eq!(
        can_succeed_function.error_return,
        Some(builtin_type_ids::STRING)
    );
    assert!(
        matches!(&can_succeed_entry.terminator, HirTerminator::Return(values) if values.len() == 1),
        "success return in a fallible function should preserve its ordinary ordered success slot"
    );
}

#[test]
fn statement_catch_handler_lowering_builds_explicit_result_branching() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let can_fail_name = super::symbol("can_fail", &mut path_fork, &mut string_table);
    let location = None;
    let error_binding = path_fork
        .try_intern_child(start_name, string_table.intern("err"))
        .expect("test path fits");

    let boom = builder
        .string_slice(
            &mut constants,
            string_table.intern("boom"),
            location,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture string fits expression store")
        .id();
    let can_fail_function = function_node(
        can_fail_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![error_return_slot(builtin_type_ids::STRING)],
        },
        vec![node(NodeKind::ReturnError(boom), location)],
        location,
    );

    let caught_call = builder
        .handled_fallible_function_call_with_typed_arguments(
            can_fail_name,
            vec![],
            vec![],
            builtin_type_ids::STRING,
            FallibleExpressionHandling::Recover,
            location,
        )
        .expect("fixture handled call fits expression store")
        .id();
    // Parser-produced catches carry a resolved HandledByCatch disposition on the protected
    // node; synthetic fixtures set it explicitly to match.
    let mut caught_facts = builder
        .view(caught_call)
        .expect("fixture handled call belongs to builder")
        .failure_facts()
        .cloned()
        .unwrap_or_default();
    caught_facts.disposition = FailureDisposition::HandledByCatch {
        error_type_id: builtin_type_ids::STRING,
    };
    let caught_call = builder
        .with_failure_facts(builder.own(caught_call), caught_facts)
        .expect("fixture handled call accepts catch disposition")
        .id();
    let caught_call = builder.own(caught_call);
    let catch_expression = wrap_catch_expression(
        &mut builder,
        caught_call,
        FallibleHandling::Handler {
            error: Some(CatchErrorBinding { error_binding }),
            body: vec![],
        },
        vec![],
    )
    .expect("fixture catch fits expression store");

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::ExpressionStatement(catch_expression.id()),
            location,
        )],
        location,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![can_fail_function, start_function],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("catch-handler statement lowering should succeed");

    let start = &module.functions[module.start_function.expect("start function").0 as usize];
    let entry = &module.blocks[start.entry.0 as usize];
    let HirTerminator::Invoke {
        failure,
        error_result,
        success_results,
        ..
    } = &entry.terminator
    else {
        panic!("catch handler should branch with an explicit Invoke");
    };
    assert!(success_results.is_empty());
    let failure_block = &module.blocks[failure.0 as usize];
    let HirTerminator::Jump {
        target: handler,
        args,
    } = &failure_block.terminator
    else {
        panic!("failure edge should transfer its native error to the catch handler");
    };
    let [error_transfer] = args.as_slice() else {
        panic!("failure edge should transfer exactly one error slot");
    };
    assert_eq!(error_transfer.source, *error_result);
    assert!(
        failure_block
            .locals
            .iter()
            .any(|local| local.id == *error_result),
        "the selected failure edge owns its error result"
    );
    let handler_block = &module.blocks[handler.0 as usize];
    assert!(
        handler_block
            .locals
            .iter()
            .any(|local| local.id == error_transfer.destination),
        "the handler owns its transferred error destination"
    );
    let error_bindings = handler_block
        .statements
        .iter()
        .filter(|statement| {
            let HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                value,
            } = &statement.kind
            else {
                return false;
            };
            module
                .side_table
                .resolve_local_name(*local, &path_fork, &string_table)
                == Some("err")
                && matches!(&module.expressions.expression(*value).kind,
                HirExpressionKind::Load(place)
                    if place.root == error_transfer.destination && super::is_local_place(place))
        })
        .count();
    assert_eq!(
        error_bindings, 1,
        "catch should bind the transferred native error once in its handler"
    );
}

#[test]
fn multi_bind_lowering_binds_ordered_slots_from_single_rhs_call() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let pair_name = super::symbol("pair", &mut path_fork, &mut string_table);
    let location = None;

    let pair_function = function_node(
        pair_name,
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![
                builtin_type_ids::fixed_scalar(FixedScalar::I32),
                builtin_type_ids::STRING,
            ]),
        },
        vec![node(
            NodeKind::Return(vec![
                i32_literal(
                    &mut builder,
                    &mut constants,
                    1,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                builder
                    .string_slice(
                        &mut constants,
                        string_table.intern("value"),
                        location,
                        ValueMode::ImmutableOwned,
                    )
                    .expect("fixture string fits expression store")
                    .id(),
            ]),
            location,
        )],
        location,
    );

    let left_id = path_fork
        .try_intern_child(start_name, string_table.intern("left"))
        .expect("test path fits");
    let right_id = path_fork
        .try_intern_child(start_name, string_table.intern("right"))
        .expect("test path fits");

    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::MultiBind {
                targets: vec![
                    multi_bind_target(
                        left_id,
                        builtin_type_ids::fixed_scalar(FixedScalar::I32),
                        ValueMode::ImmutableOwned,
                        MultiBindTargetKind::Declaration,
                        location,
                    ),
                    multi_bind_target(
                        right_id,
                        builtin_type_ids::STRING,
                        ValueMode::ImmutableOwned,
                        MultiBindTargetKind::Declaration,
                        location,
                    ),
                ],
                value: builder
                    .function_call_with_typed_arguments(
                        pair_name,
                        vec![],
                        vec![
                            builtin_type_ids::fixed_scalar(FixedScalar::I32),
                            builtin_type_ids::STRING,
                        ],
                        location,
                    )
                    .expect("fixture multi-bind call fits expression store")
                    .id(),
            },
            location,
        )],
        location,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![pair_function, start_function],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("multi-bind lowering should succeed");

    let call_count = module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .filter(|statement| matches!(statement.kind, HirStatementKind::Call { .. }))
        .count();
    assert_eq!(
        call_count, 1,
        "multi-bind RHS call should lower exactly once"
    );

    let call_results = module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Call { results, .. } => Some(results),
            _ => None,
        })
        .expect("multi-bind should emit its RHS call");
    assert_eq!(call_results.len(), 2);
    let result_locals = call_results
        .iter()
        .map(|result| match result {
            HirLocalDestination::Define(local) => *local,
            _ => panic!("multi-bind RHS should define fresh result bindings"),
        })
        .collect::<Vec<_>>();

    let start = &module.functions[module.start_function.expect("start function").0 as usize];
    let entry = &module.blocks[start.entry.0 as usize];
    for (name, result) in ["left", "right"].iter().zip(result_locals) {
        let staged = staged_result_local(&module, entry, result);
        let local = entry
            .locals
            .iter()
            .find(|local| {
                module
                    .side_table
                    .resolve_local_name(local.id, &path_fork, &string_table)
                    == Some(*name)
            })
            .expect("multi-bind declaration should own its authored local");
        let definitions = entry
            .statements
            .iter()
            .filter(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Write {
                        target: HirWriteTarget::DefineLocal(target),
                        value,
                    } if *target == local.id && matches!(
                        &module.expressions.expression(*value).kind,
                        HirExpressionKind::Load(place)
                            if place.root == staged && super::is_local_place(place)
                    )
                )
            })
            .count();
        assert_eq!(
            definitions, 1,
            "multi-bind declaration {name} should define once"
        );
    }
}

#[test]
fn multi_value_then_snapshots_earlier_scalar_before_later_mutation() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let mut builder = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let count = super::symbol("count", &mut path_fork, &mut string_table);
    let bump = super::symbol("bump", &mut path_fork, &mut string_table);
    let input = path_fork
        .try_intern_child(bump, string_table.intern("input"))
        .expect("test parameter path should fit");
    let scalar_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let bump_target = assignment_target(
        &mut builder,
        input,
        DataType::FixedScalar(FixedScalar::I32),
        scalar_type,
        None,
    );
    let bump_value = i32_literal(
        &mut builder,
        &mut constants,
        2,
        None,
        ValueMode::ImmutableOwned,
    );
    let bump_return = reference_expr_with_type_id(
        &mut builder,
        input,
        scalar_type,
        None,
        ValueMode::ImmutableReference,
    );
    let bump_function = function_node(
        bump,
        FunctionSignature {
            parameters: vec![param_with_type_id(input, scalar_type, true, None)],
            returns: fresh_success_returns(vec![scalar_type]),
        },
        vec![
            node(
                NodeKind::Assignment {
                    target: bump_target,
                    value: bump_value,
                },
                None,
            ),
            node(NodeKind::Return(vec![bump_return]), None),
        ],
        None,
    );
    let count_snapshot = reference_expr_with_type_id(
        &mut builder,
        count,
        scalar_type,
        None,
        ValueMode::ImmutableReference,
    );
    let bump_argument = reference_expr_with_type_id(
        &mut builder,
        count,
        scalar_type,
        None,
        ValueMode::MutableReference,
    );
    let bump_call = builder
        .function_call_with_typed_arguments(
            bump,
            vec![CallArgument::positional(
                builder.own(bump_argument),
                CallAccessMode::Mutable,
                None,
            )],
            vec![scalar_type],
            None,
        )
        .expect("fixture call fits expression store")
        .id();
    let produced = ProducedValues {
        expressions: vec![count_snapshot, bump_call],
        span: None,
    };
    let flag = super::symbol("flag", &mut path_fork, &mut string_table);
    let flag_value = builder
        .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
        .expect("fixture bool fits expression store")
        .id();
    let flag_decl = node(
        NodeKind::VariableDeclaration(make_test_variable(flag, flag_value, &builder)),
        None,
    );
    let condition = reference_expr_with_type_id(
        &mut builder,
        flag,
        builtin_type_ids::BOOL,
        None,
        ValueMode::ImmutableReference,
    );
    let else_first = i32_literal(
        &mut builder,
        &mut constants,
        0,
        None,
        ValueMode::ImmutableOwned,
    );
    let else_second = i32_literal(
        &mut builder,
        &mut constants,
        0,
        None,
        ValueMode::ImmutableOwned,
    );
    let value_if_success = builder
        .success_from_slots(vec![scalar_type, scalar_type], None)
        .expect("fixture value-if success fits store");
    let value_if = builder
        .value_block(
            ValueBlock::If(ValueIfBlock {
                condition,
                then_body: vec![node(NodeKind::ThenValue(produced), None)],
                else_body: vec![node(
                    NodeKind::ThenValue(ProducedValues {
                        expressions: vec![else_first, else_second],
                        span: None,
                    }),
                    None,
                )],
                then_scope: entry_path,
                else_scope: entry_path,
                span: None,
                generic_request_ranges: Default::default(),
                result_type_ids: vec![scalar_type, scalar_type],
            }),
            None,
            value_if_success,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture value block fits expression store")
        .id();
    let start_initial = i32_literal(
        &mut builder,
        &mut constants,
        1,
        None,
        ValueMode::MutableOwned,
    );
    let start_function = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: fresh_success_returns(vec![scalar_type, scalar_type]),
        },
        vec![
            flag_decl,
            node(
                NodeKind::VariableDeclaration(make_test_variable(count, start_initial, &builder)),
                None,
            ),
            node(NodeKind::Return(vec![value_if]), None),
        ],
        None,
    );
    let (module, _) = lower_ast(
        build_ast_with_registered_types(
            vec![bump_function, start_function],
            entry_path,
            builder,
            constants,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("multi-value then with later mutation should lower");
    let start = &module.functions[module.start_function.expect("start function").0 as usize];
    let entry = &module.blocks[start.entry.0 as usize];
    let count_local = entry
        .locals
        .iter()
        .find(|local| {
            module
                .side_table
                .resolve_local_name(local.id, &path_fork, &string_table)
                == Some("count")
        })
        .expect("count should be an authored mutable local")
        .id;
    let branch = module
        .blocks
        .iter()
        .find(|block| {
            block.statements.iter().any(|statement| {
                matches!(&statement.kind,
            HirStatementKind::Call { target: CallTarget::Local(function), .. }
                if module.side_table.function_name_path(*function) == Some(bump))
            })
        })
        .expect("then branch should execute bump");
    let (call_position, call_results) = branch
        .statements
        .iter()
        .enumerate()
        .find_map(|(index, statement)| match &statement.kind {
            HirStatementKind::Call { results, .. } => Some((index, results)),
            _ => None,
        })
        .expect("then branch should contain bump call");
    let [HirLocalDestination::Define(second_result)] = call_results.as_slice() else {
        panic!("bump should define one result");
    };
    let HirTerminator::Jump { target, args } = &branch.terminator else {
        panic!("then should transfer ordered slots to its join");
    };
    assert_eq!(args.len(), 2);
    let first_snapshot = branch.statements.iter().position(|statement| matches!(
        &statement.kind,
        HirStatementKind::Write { target: HirWriteTarget::DefineLocal(local), value }
            if *local == args[0].source && matches!(&module.expressions.expression(*value).kind,
                HirExpressionKind::Load(place)
                    if place.root == count_local && super::is_local_place(place))
    )).expect("first slot should snapshot count");
    assert!(
        first_snapshot < call_position,
        "the first scalar value must be read before the second producer mutates count"
    );
    assert!(
        branch
            .statements
            .iter()
            .enumerate()
            .any(|(index, statement)| {
                index > call_position
                    && matches!(&statement.kind,
            HirStatementKind::Write { target: HirWriteTarget::DefineLocal(local), value }
                if *local == args[1].source && matches!(&module.expressions.expression(*value).kind,
                    HirExpressionKind::Load(place)
                        if place.root == *second_result && super::is_local_place(place)))
            }),
        "second slot should snapshot the later call result"
    );
    let join = &module.blocks[target.0 as usize];
    for argument in args {
        assert!(
            join.locals
                .iter()
                .any(|local| local.id == argument.destination),
            "join destinations must be owned by the target block"
        );
        assert!(
            !branch
                .locals
                .iter()
                .any(|local| local.id == argument.destination)
        );
    }
    let HirTerminator::Return(return_values) = &join.terminator else {
        panic!("join should return the ordered success slots");
    };
    let return_values = module.expressions.values(*return_values);
    assert_eq!(return_values.len(), args.len());
    for (value, argument) in return_values.iter().zip(args) {
        assert!(matches!(&module.expressions.expression(*value).kind,
            HirExpressionKind::Load(place)
                if place.root == argument.destination && super::is_local_place(place)));
    }
}
