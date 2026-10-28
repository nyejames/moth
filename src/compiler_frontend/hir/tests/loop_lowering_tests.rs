//! Loop lowering regression tests.
//!
//! WHAT: validates range/collection loop lowering into explicit HIR CFG blocks.
//! WHY: loop header refactors must preserve control-flow semantics and loop-target routing.

use crate::compiler_frontend::ast::ast_nodes::{
    LoopBindings, NodeKind, RangeEndKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::ids::BlockId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, NumericFailureMode, RangeStepFailureCause,
};
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_node, make_test_variable, node, reference_expr_with_type_id, test_if_branch_metadata,
};
use crate::compiler_frontend::tests::type_id_fixture_support::{
    assert_no_placeholder_terminators, build_ast_with_registered_types,
    loop_binding_with_type_id as loop_binding, lower_ast,
};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

/// Builds one range-loop spec for fixtures with exact `I32` operands.
///
/// WHY: the loop header's selected iteration domain is part of the spec, and these fixtures
///      author exact `I32` bounds and steps, so their selected domain is `I32`.
fn range_loop_spec(
    start: ExprId,
    end: ExprId,
    end_kind: RangeEndKind,
    step: Option<ExprId>,
) -> Box<RangeLoopSpec> {
    range_loop_spec_with_domain(
        start,
        end,
        end_kind,
        step,
        NumericScalar::Fixed(FixedScalar::I32),
    )
}

/// Builds one range-loop spec with an explicit header-selected iteration domain.
///
/// WHY: the domain is a header fact, so a fixture can pin it independently of the operand
///      result types that normally select it.
fn range_loop_spec_with_domain(
    start: ExprId,
    end: ExprId,
    end_kind: RangeEndKind,
    step: Option<ExprId>,
    domain: NumericScalar,
) -> Box<RangeLoopSpec> {
    Box::new(RangeLoopSpec {
        start,
        end,
        end_kind,
        step,
        domain,
    })
}

fn range_loop_cfg_blocks(module: &HirModule) -> (BlockId, BlockId, BlockId) {
    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];
    let step_zero_check_block = match entry_block.terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected entry jump to range step-zero-check"),
    };

    let step_abs_check_block = match module.blocks[step_zero_check_block.0 as usize].terminator {
        HirTerminator::If { else_block, .. } => else_block,
        _ => panic!("expected zero-check branch"),
    };

    let header_selector_block = match module.blocks[step_abs_check_block.0 as usize].terminator {
        HirTerminator::If { else_block, .. } => else_block,
        _ => panic!("expected step magnitude branch"),
    };

    let header_ascending_block = match module.blocks[header_selector_block.0 as usize].terminator {
        HirTerminator::If { then_block, .. } => then_block,
        _ => panic!("expected header selector branch"),
    };

    (
        step_zero_check_block,
        header_selector_block,
        header_ascending_block,
    )
}

fn collection_literal(
    builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    type_environment: &mut TypeEnvironment,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> ExprId {
    let items = vec![
        i32_literal(builder, const_values, 1, span, ValueMode::ImmutableOwned),
        i32_literal(builder, const_values, 2, span, ValueMode::ImmutableOwned),
        i32_literal(builder, const_values, 3, span, ValueMode::ImmutableOwned),
    ];
    crate::compiler_frontend::tests::type_id_fixture_support::collection_expr(
        builder,
        items,
        span,
        ValueMode::ImmutableOwned,
        type_environment,
    )
}

/// One exact `I32` literal expression.
///
/// WHY: these fixtures pin the exact `I32` width explicitly at the scalar carrier.
fn i32_literal(
    builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    value: i64,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
    value_mode: ValueMode,
) -> ExprId {
    builder
        .fixed_scalar(
            const_values,
            FixedScalarValue::signed(FixedScalar::I32, value).expect("test literal fits I32"),
            span,
            value_mode,
        )
        .expect("fixture literal fits store")
        .id()
}

#[test]
fn lowers_range_loop_with_new_syntax() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    let range_loop = node(
        NodeKind::RangeLoop {
            bindings: Box::new(LoopBindings {
                item: Some(loop_binding(
                    PathId::ROOT,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
                index: None,
            }),
            range: range_loop_spec(
                i32_literal(
                    &mut expressions,
                    &mut const_values,
                    0,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                i32_literal(
                    &mut expressions,
                    &mut const_values,
                    3,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                RangeEndKind::Exclusive,
                None,
            ),
            body: vec![],
        },
        location,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![range_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("range loop lowering should succeed");

    let (_, _, header_ascending_block) = range_loop_cfg_blocks(&module);

    let (body_block, exit_block) = match module.blocks[header_ascending_block.0 as usize].terminator
    {
        HirTerminator::If {
            then_block,
            else_block,
            ..
        } => (then_block, else_block),
        _ => panic!("expected ascending header branch"),
    };

    let step_block = match module.blocks[body_block.0 as usize].terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected body jump to step block"),
    };

    assert!(matches!(
        module.blocks[step_block.0 as usize].terminator,
        HirTerminator::If { .. }
    ));
    assert!(matches!(
        module.blocks[exit_block.0 as usize].terminator,
        HirTerminator::Return(_)
    ));
    assert_no_placeholder_terminators(&module);
}

/// A range header's selected iteration domain drives lowering, not the operand result types.
///
/// WHAT: pins an `I64` header domain over exact `I32` operands on a header with no item binding,
///       so `range.domain` is the only source of `I64`, and asserts lowering converts the bounds
///       to `I64`, initialises the counter state as `I64` and generates its checked arithmetic
///       in `I64`.
/// WHY: the header selected that domain when its operands still had canonical types. Re-deriving
///      a domain from the operands during lowering would build `I32` state here and silently
///      ignore the selected header fact.
#[test]
fn range_loop_lowering_uses_the_header_selected_domain() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let selected_domain = NumericScalar::Fixed(FixedScalar::I64);
    let range_loop = node(
        NodeKind::RangeLoop {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: range_loop_spec_with_domain(
                i32_literal(
                    &mut expressions,
                    &mut const_values,
                    0,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                i32_literal(
                    &mut expressions,
                    &mut const_values,
                    4,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                RangeEndKind::Exclusive,
                None,
                selected_domain,
            ),
            body: vec![],
        },
        location,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![range_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("range loop lowering should succeed");

    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let i64_type = builtin_type_ids::fixed_scalar(FixedScalar::I64);
    let local_type = |local| {
        module
            .blocks
            .iter()
            .flat_map(|block| block.locals.iter())
            .find_map(|candidate| (candidate.id == local).then_some(candidate.ty))
    };

    // The exact-width bound operands must be converted to the header's selected domain, and the
    // counter state locals that conversion initialises must carry that same domain.
    let bound_state_locals: Vec<_> = module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .filter_map(|statement| {
            let HirStatementKind::Write { value, target } = &statement.kind else {
                return None;
            };
            let HirWriteTarget::DefineLocal(local) = target else {
                return None;
            };
            matches!(
                &module.expressions.expression(*value).kind,
                HirExpressionKind::Cast {
                    policy: BuiltinCastPolicyId::NumericConversion {
                        source: NumericScalar::Fixed(FixedScalar::I32),
                        target: NumericScalar::Fixed(FixedScalar::I64),
                    },
                    ..
                }
            )
            .then_some(*local)
        })
        .collect();
    assert!(
        !bound_state_locals.is_empty(),
        "range bounds must convert from their own I32 type to the header's I64 domain"
    );
    assert!(
        bound_state_locals
            .iter()
            .all(|local| local_type(*local) == Some(i64_type)),
        "counter state initialised from a converted bound must use the header's I64 domain"
    );

    // Every generated checked operation on the counter belongs to the header's selected domain.
    // The automatically supplied iteration index is the one exception: it always increments in the
    // language's exact `I32` positional-index identity, never in the counter's domain.
    let counter_domains: Vec<NumericScalar> = module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .filter_map(|statement| {
            let HirStatementKind::NumericOp { op, result, .. } = &statement.kind else {
                return None;
            };
            let destination = match result {
                HirLocalDestination::Define(local) | HirLocalDestination::Update(local) => *local,
            };
            (local_type(destination) != Some(i32_type)).then_some(op.domain)
        })
        .collect();
    assert!(
        !counter_domains.is_empty(),
        "range loop lowering should generate checked counter arithmetic"
    );
    assert!(
        counter_domains
            .iter()
            .all(|domain| *domain == selected_domain),
        "generated range-loop counter arithmetic must use the header domain, got {counter_domains:?}"
    );
}

#[test]
fn lowers_range_loop_without_user_bindings() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let range_loop = node(
        NodeKind::RangeLoop {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: range_loop_spec(
                i32_literal(
                    &mut expressions,
                    &mut const_values,
                    0,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                i32_literal(
                    &mut expressions,
                    &mut const_values,
                    3,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                RangeEndKind::Exclusive,
                None,
            ),
            body: vec![],
        },
        location,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![range_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("range loop lowering without user bindings should succeed");

    let (_, _, header_ascending_block) = range_loop_cfg_blocks(&module);
    let body_block = match module.blocks[header_ascending_block.0 as usize].terminator {
        HirTerminator::If { then_block, .. } => then_block,
        _ => panic!("expected ascending header branch"),
    };

    let body_locals = &module.blocks[body_block.0 as usize].locals;
    assert!(
        body_locals.is_empty(),
        "binding-less range loops should not allocate user binding locals"
    );
}

#[test]
fn lowers_range_loop_with_index_binding() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    let item_symbol = super::symbol("item", &mut path_fork, &mut string_table);
    let index_symbol = super::symbol("index", &mut path_fork, &mut string_table);
    let range_loop = node(
        NodeKind::RangeLoop {
            bindings: Box::new(LoopBindings {
                item: Some(loop_binding(
                    item_symbol,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
                index: Some(loop_binding(
                    index_symbol,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
            }),
            range: range_loop_spec(
                i32_literal(
                    &mut expressions,
                    &mut const_values,
                    0,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                i32_literal(
                    &mut expressions,
                    &mut const_values,
                    4,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                RangeEndKind::Exclusive,
                None,
            ),
            body: vec![],
        },
        location,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![range_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("range loop lowering with index should succeed");

    let (_, _, header_ascending_block) = range_loop_cfg_blocks(&module);

    let body_block = match module.blocks[header_ascending_block.0 as usize].terminator {
        HirTerminator::If { then_block, .. } => then_block,
        _ => panic!("expected ascending header branch"),
    };

    let body_statements = &module.blocks[body_block.0 as usize].statements;
    let binding_definition_count = body_statements
        .iter()
        .filter(|statement| {
            matches!(
                statement.kind,
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(_),
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        binding_definition_count, 2,
        "expected definitions for the value and index bindings"
    );

    let has_index_increment = module.blocks.iter().any(|block| {
        block
            .statements
            .iter()
            .any(|statement| match &statement.kind {
                HirStatementKind::NumericOp {
                    op:
                        HirNumericOp {
                            operator: NumericOperator::Add,
                            domain: NumericScalar::Fixed(FixedScalar::I32),
                        },
                    operands: HirNumericOperands::Binary { right, .. },
                    result: HirLocalDestination::Update(_),
                    ..
                } => matches!(
                    &module.expressions.expression(*right).kind,
                    HirExpressionKind::FixedScalar(literal)
                        if *literal
                            == FixedScalarValue::signed(FixedScalar::I32, 1)
                                .expect("I32 literal in range")
                ),
                _ => false,
            })
    });

    assert!(
        has_index_increment,
        "expected explicit zero-based index increment in range step block"
    );
}

#[test]
fn preserves_runtime_zero_step_guard_for_dynamic_step() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    let step_symbol = super::symbol("step", &mut path_fork, &mut string_table);
    let step_value = i32_literal(
        &mut expressions,
        &mut const_values,
        2,
        location,
        ValueMode::ImmutableOwned,
    );
    let step_decl = node(
        NodeKind::VariableDeclaration(make_test_variable(step_symbol, step_value, &expressions)),
        location,
    );

    let range_loop = node(
        NodeKind::RangeLoop {
            bindings: Box::new(LoopBindings {
                item: Some(loop_binding(
                    PathId::ROOT,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
                index: None,
            }),
            range: range_loop_spec(
                i32_literal(
                    &mut expressions,
                    &mut const_values,
                    0,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                i32_literal(
                    &mut expressions,
                    &mut const_values,
                    10,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                RangeEndKind::Exclusive,
                Some(reference_expr_with_type_id(
                    &mut expressions,
                    step_symbol,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                    location,
                    ValueMode::ImmutableReference,
                )),
            ),
            body: vec![],
        },
        location,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![step_decl, range_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("dynamic-step range loop lowering should succeed");

    let (step_zero_check_block, _, _) = range_loop_cfg_blocks(&module);

    let panic_block = match module.blocks[step_zero_check_block.0 as usize].terminator {
        HirTerminator::If { then_block, .. } => then_block,
        _ => panic!("expected runtime zero-check branch"),
    };

    assert!(
        module.blocks[panic_block.0 as usize]
            .statements
            .iter()
            .any(|statement| matches!(
                statement.kind,
                HirStatementKind::RangeStepFailure {
                    cause: RangeStepFailureCause::ZeroStep,
                    failure_mode: NumericFailureMode::Trap,
                    ..
                }
            )),
        "a function without Error! or a private lane keeps a typed fatal guard"
    );
}

#[test]
fn lowers_collection_loop_to_explicit_cfg() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();

    let collection_loop = node(
        NodeKind::CollectionLoop {
            bindings: Box::new(LoopBindings {
                item: Some(loop_binding(
                    PathId::ROOT,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
                index: None,
            }),
            iterable: collection_literal(
                &mut expressions,
                &mut const_values,
                &mut type_environment,
                location,
            ),
            body: vec![],
        },
        location,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![collection_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            type_environment,
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("collection loop lowering should succeed");

    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];
    let header_block = match entry_block.terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected jump to collection header"),
    };

    let (body_block, exit_block) = match module.blocks[header_block.0 as usize].terminator {
        HirTerminator::If {
            then_block,
            else_block,
            ..
        } => (then_block, else_block),
        _ => panic!("expected collection loop header conditional"),
    };

    let step_block = match module.blocks[body_block.0 as usize].terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected collection body jump to step block"),
    };

    assert!(matches!(
        module.blocks[step_block.0 as usize].terminator,
        HirTerminator::Jump { target, .. } if target == header_block
    ));
    assert!(matches!(
        module.blocks[exit_block.0 as usize].terminator,
        HirTerminator::Return(_)
    ));
}

#[test]
fn lowers_collection_loop_without_user_bindings() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();

    let collection_loop = node(
        NodeKind::CollectionLoop {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            iterable: collection_literal(
                &mut expressions,
                &mut const_values,
                &mut type_environment,
                location,
            ),
            body: vec![],
        },
        location,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![collection_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            type_environment,
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("collection loop lowering without user bindings should succeed");

    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];
    let header_block = match entry_block.terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected jump to collection header"),
    };
    let body_block = match module.blocks[header_block.0 as usize].terminator {
        HirTerminator::If { then_block, .. } => then_block,
        _ => panic!("expected collection loop header conditional"),
    };

    assert!(
        module.blocks[body_block.0 as usize].locals.is_empty(),
        "binding-less collection loops should not allocate user binding locals"
    );
}

#[test]
fn lowers_collection_loop_item_binding_from_indexed_place() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();

    let collection_loop = node(
        NodeKind::CollectionLoop {
            bindings: Box::new(LoopBindings {
                item: Some(loop_binding(
                    PathId::ROOT,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
                index: None,
            }),
            iterable: collection_literal(
                &mut expressions,
                &mut const_values,
                &mut type_environment,
                location,
            ),
            body: vec![],
        },
        location,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![collection_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            type_environment,
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("collection loop lowering should succeed");

    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];
    let header_block = match entry_block.terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected jump to collection header"),
    };
    let body_block = match module.blocks[header_block.0 as usize].terminator {
        HirTerminator::If { then_block, .. } => then_block,
        _ => panic!("expected collection loop header conditional"),
    };

    let has_indexed_item_assign =
        module.blocks[body_block.0 as usize]
            .statements
            .iter()
            .any(|statement| {
                super::write_value(&module, statement).is_some_and(|value| {
                    matches!(
                        &value.kind,
                        HirExpressionKind::Load(place)
                            if super::place_outermost_projection_is_index(&module, place)
                    )
                })
            });

    assert!(
        has_indexed_item_assign,
        "expected collection item binding to load from indexed place"
    );
}

#[test]
fn lowers_collection_loop_optional_index_binding() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();

    let item_symbol = super::symbol("item", &mut path_fork, &mut string_table);
    let index_symbol = super::symbol("index", &mut path_fork, &mut string_table);
    let collection_loop = node(
        NodeKind::CollectionLoop {
            bindings: Box::new(LoopBindings {
                item: Some(loop_binding(
                    item_symbol,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
                index: Some(loop_binding(
                    index_symbol,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
            }),
            iterable: collection_literal(
                &mut expressions,
                &mut const_values,
                &mut type_environment,
                location,
            ),
            body: vec![],
        },
        location,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![collection_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            type_environment,
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("collection loop lowering with index should succeed");

    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];
    let header_block = match entry_block.terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected jump to collection header"),
    };
    let body_block = match module.blocks[header_block.0 as usize].terminator {
        HirTerminator::If { then_block, .. } => then_block,
        _ => panic!("expected collection loop header conditional"),
    };

    let statements = &module.blocks[body_block.0 as usize].statements;
    let has_item_assign = statements.iter().any(|statement| {
        super::write_value(&module, statement).is_some_and(|value| {
            matches!(
                &value.kind,
                HirExpressionKind::Load(place)
                    if super::place_outermost_projection_is_index(&module, place)
            )
        })
    });
    let has_index_assign = statements.iter().any(|statement| {
        super::write_value(&module, statement).is_some_and(|value| {
            matches!(
                &value.kind,
                HirExpressionKind::Load(place) if super::is_local_place(place)
            )
        })
    });

    assert!(has_item_assign, "expected indexed item assignment");
    assert!(has_index_assign, "expected explicit user index assignment");
}

#[test]
fn lowers_range_loop_user_bindings_as_immutable_locals() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let item_symbol = super::symbol("item", &mut path_fork, &mut string_table);
    let index_symbol = super::symbol("index", &mut path_fork, &mut string_table);
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let range_loop = node(
        NodeKind::RangeLoop {
            bindings: Box::new(LoopBindings {
                item: Some(loop_binding(
                    item_symbol,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
                index: Some(loop_binding(
                    index_symbol,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
            }),
            range: range_loop_spec(
                i32_literal(
                    &mut expressions,
                    &mut const_values,
                    0,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                i32_literal(
                    &mut expressions,
                    &mut const_values,
                    4,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                RangeEndKind::Exclusive,
                None,
            ),
            body: vec![],
        },
        location,
    );
    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![range_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("range loop lowering should succeed");

    let (_, _, header_ascending_block) = range_loop_cfg_blocks(&module);
    let body_block = match module.blocks[header_ascending_block.0 as usize].terminator {
        HirTerminator::If { then_block, .. } => then_block,
        _ => panic!("expected ascending header branch"),
    };

    let body_locals = &module.blocks[body_block.0 as usize].locals;
    assert!(
        !body_locals.is_empty() && body_locals.iter().all(|local| !local.mutable),
        "range loop user binding locals should be immutable"
    );
}

#[test]
fn lowers_collection_loop_user_bindings_as_immutable_locals() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();

    let item_symbol = super::symbol("item", &mut path_fork, &mut string_table);
    let index_symbol = super::symbol("index", &mut path_fork, &mut string_table);
    let collection_loop = node(
        NodeKind::CollectionLoop {
            bindings: Box::new(LoopBindings {
                item: Some(loop_binding(
                    item_symbol,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
                index: Some(loop_binding(
                    index_symbol,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
            }),
            iterable: collection_literal(
                &mut expressions,
                &mut const_values,
                &mut type_environment,
                location,
            ),
            body: vec![],
        },
        location,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![collection_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            type_environment,
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("collection loop lowering should succeed");

    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];
    let header_block = match entry_block.terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected jump to collection header"),
    };
    let body_block = match module.blocks[header_block.0 as usize].terminator {
        HirTerminator::If { then_block, .. } => then_block,
        _ => panic!("expected collection loop header conditional"),
    };

    let body_locals = &module.blocks[body_block.0 as usize].locals;
    assert!(
        !body_locals.is_empty() && body_locals.iter().all(|local| !local.mutable),
        "collection loop user binding locals should be immutable"
    );
}

#[test]
fn break_targets_exit_block_in_collection_loop() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let flag = super::symbol("flag", &mut path_fork, &mut string_table);
    let flag_value = expressions
        .bool(&mut const_values, true, location, ValueMode::ImmutableOwned)
        .expect("fixture literal fits store")
        .id();
    let flag_decl = node(
        NodeKind::VariableDeclaration(make_test_variable(flag, flag_value, &expressions)),
        location,
    );
    let condition = reference_expr_with_type_id(
        &mut expressions,
        flag,
        builtin_type_ids::BOOL,
        location,
        ValueMode::ImmutableReference,
    );

    let collection_loop = node(
        NodeKind::CollectionLoop {
            bindings: Box::new(LoopBindings {
                item: Some(loop_binding(
                    PathId::ROOT,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
                index: None,
            }),
            iterable: collection_literal(
                &mut expressions,
                &mut const_values,
                &mut type_environment,
                location,
            ),
            body: vec![node(
                NodeKind::If(
                    condition,
                    vec![node(NodeKind::Break, location)],
                    None,
                    test_if_branch_metadata(false),
                ),
                location,
            )],
        },
        location,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![flag_decl, collection_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            type_environment,
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("collection loop lowering should succeed");

    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];
    let header_block = match entry_block.terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected jump to collection header"),
    };

    let (_, exit_block) = match module.blocks[header_block.0 as usize].terminator {
        HirTerminator::If {
            then_block,
            else_block,
            ..
        } => (then_block, else_block),
        _ => panic!("expected collection loop header conditional"),
    };

    let break_targets_exit = module.blocks.iter().any(|block| {
        matches!(
            block.terminator,
            HirTerminator::Break { target } if target == exit_block
        )
    });

    assert!(
        break_targets_exit,
        "expected break terminator to target collection loop exit block"
    );
}

#[test]
fn direct_break_in_collection_loop_does_not_leave_unreachable_step_block() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();

    let collection_loop = node(
        NodeKind::CollectionLoop {
            bindings: Box::new(LoopBindings {
                item: Some(loop_binding(
                    PathId::ROOT,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
                index: None,
            }),
            iterable: collection_literal(
                &mut expressions,
                &mut const_values,
                &mut type_environment,
                location,
            ),
            body: vec![node(NodeKind::Break, location)],
        },
        location,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![collection_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            type_environment,
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("direct break collection loop lowering should succeed");

    assert_no_placeholder_terminators(&module);
}

#[test]
fn direct_break_in_range_loop_does_not_leave_unreachable_step_block() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    let range_loop = node(
        NodeKind::RangeLoop {
            bindings: Box::new(LoopBindings {
                item: Some(loop_binding(
                    PathId::ROOT,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
                index: None,
            }),
            range: range_loop_spec(
                i32_literal(
                    &mut expressions,
                    &mut const_values,
                    0,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                i32_literal(
                    &mut expressions,
                    &mut const_values,
                    3,
                    location,
                    ValueMode::ImmutableOwned,
                ),
                RangeEndKind::Exclusive,
                None,
            ),
            body: vec![node(NodeKind::Break, location)],
        },
        location,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![range_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("direct break range loop lowering should succeed");

    assert_no_placeholder_terminators(&module);
}

#[test]
fn continue_targets_step_block_in_collection_loop() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();

    let collection_loop = node(
        NodeKind::CollectionLoop {
            bindings: Box::new(LoopBindings {
                item: Some(loop_binding(
                    PathId::ROOT,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
                index: None,
            }),
            iterable: collection_literal(
                &mut expressions,
                &mut const_values,
                &mut type_environment,
                location,
            ),
            body: vec![node(NodeKind::Continue, location)],
        },
        location,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![collection_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            type_environment,
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("collection loop lowering should succeed");

    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];
    let header_block = match entry_block.terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected jump to collection header"),
    };

    let body_block = match module.blocks[header_block.0 as usize].terminator {
        HirTerminator::If { then_block, .. } => then_block,
        _ => panic!("expected collection loop header conditional"),
    };

    let step_block = match module.blocks[body_block.0 as usize].terminator {
        HirTerminator::Continue { target } => target,
        _ => panic!("expected continue terminator in collection body"),
    };

    assert!(matches!(
        module.blocks[step_block.0 as usize].terminator,
        HirTerminator::Jump { target, .. } if target == header_block
    ));
}

#[test]
fn nested_loop_targets_remain_correct() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();

    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let inner_item_symbol = super::symbol("inner_item", &mut path_fork, &mut string_table);
    let inner_loop = node(
        NodeKind::CollectionLoop {
            bindings: Box::new(LoopBindings {
                item: Some(loop_binding(
                    inner_item_symbol,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
                index: None,
            }),
            iterable: collection_literal(
                &mut expressions,
                &mut const_values,
                &mut type_environment,
                location,
            ),
            body: vec![node(NodeKind::Continue, location)],
        },
        location,
    );

    let outer_item_symbol = super::symbol("outer_item", &mut path_fork, &mut string_table);
    let outer_loop = node(
        NodeKind::CollectionLoop {
            bindings: Box::new(LoopBindings {
                item: Some(loop_binding(
                    outer_item_symbol,
                    builtin_type_ids::fixed_scalar(FixedScalar::I32),
                )),
                index: None,
            }),
            iterable: collection_literal(
                &mut expressions,
                &mut const_values,
                &mut type_environment,
                location,
            ),
            body: vec![inner_loop, node(NodeKind::Continue, location)],
        },
        location,
    );

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![outer_loop],
        None,
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            entry_path,
            expressions,
            const_values,
            type_environment,
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("nested collection loop lowering should succeed");

    let continue_targets = module
        .blocks
        .iter()
        .filter_map(|block| match block.terminator {
            HirTerminator::Continue { target } => Some(target),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(
        continue_targets.len() >= 2,
        "expected at least one continue for each nested loop"
    );

    let unique_targets = continue_targets
        .iter()
        .map(|target| target.0)
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        unique_targets.len() >= 2,
        "nested loops should continue to distinct step blocks"
    );

    for target in continue_targets {
        assert!(matches!(
            module.blocks[target.0 as usize].terminator,
            HirTerminator::Jump { .. }
        ));
    }
}
