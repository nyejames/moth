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
use crate::compiler_frontend::builtins::error_type::{
    builtin_error_type_path, register_builtin_error_types,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::datatypes::definitions::{FieldDefinition, StructTypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::datatypes::{CallableReturnShape, FunctionReturnContract};
use crate::compiler_frontend::external_packages::{CallTarget, ExternalFunctionId};
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::ids::{BlockId, HirValueId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::{
    HirNumericOp, HirNumericOperands, LoopStepFailureCause, NumericFailureMode,
};
use crate::compiler_frontend::hir::operators::HirBinOp;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    make_test_variable, node, reference_expr_with_type_id, test_if_branch_metadata,
};
use crate::compiler_frontend::tests::type_id_fixture_support::{
    assert_no_placeholder_terminators, build_ast_with_registered_types, error_return_slot,
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

/// The stable destination blocks of one lowered range loop.
struct RangeLoopCfg {
    step_zero_check: BlockId,
    header_selector: BlockId,
    body: BlockId,
    step: BlockId,
    exit: BlockId,
}

/// Walks the range-loop CFG from the entry block.
///
/// WHY: the zero guard selects the direction-recording header selector, which then branches by
///      `step > 0`. Reading the shape through the terminators keeps the test independent of block
///      allocation order while still pinning the edges that own loop semantics.
fn range_loop_cfg_blocks(module: &HirModule) -> RangeLoopCfg {
    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry_block = &module.blocks[start.entry.0 as usize];
    let step_zero_check = match entry_block.terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected entry jump to range step-zero-check"),
    };

    let header_selector = match module.blocks[step_zero_check.0 as usize].terminator {
        HirTerminator::If { else_block, .. } => else_block,
        _ => panic!("expected zero-check branch"),
    };

    let header_ascending = match module.blocks[header_selector.0 as usize].terminator {
        HirTerminator::If { then_block, .. } => then_block,
        _ => panic!("expected header direction branch"),
    };

    let (body, exit) = match module.blocks[header_ascending.0 as usize].terminator {
        HirTerminator::If {
            then_block,
            else_block,
            ..
        } => (then_block, else_block),
        _ => panic!("expected ascending header branch"),
    };

    let step = match module.blocks[body.0 as usize].terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected body jump to step block"),
    };

    RangeLoopCfg {
        step_zero_check,
        header_selector,
        body,
        step,
        exit,
    }
}

fn local_type(
    module: &HirModule,
    local: crate::compiler_frontend::hir::ids::LocalId,
) -> Option<TypeId> {
    module
        .blocks
        .iter()
        .flat_map(|block| block.locals.iter())
        .find_map(|candidate| (candidate.id == local).then_some(candidate.ty))
}

fn fixed_literal(
    builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    value: FixedScalarValue,
    span: Option<crate::compiler_frontend::source::SourceSpan>,
) -> ExprId {
    builder
        .fixed_scalar(const_values, value, span, ValueMode::ImmutableOwned)
        .expect("fixture literal fits store")
        .id()
}

fn lower_range_loop_fixture(
    domain: NumericScalar,
    start: FixedScalarValue,
    end: FixedScalarValue,
    end_kind: RangeEndKind,
    step: Option<FixedScalarValue>,
    step_span: Option<SourceSpan>,
) -> HirModule {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();

    let start = fixed_literal(&mut expressions, &mut const_values, start, location);
    let end = fixed_literal(&mut expressions, &mut const_values, end, location);
    let step = step.map(|step| fixed_literal(&mut expressions, &mut const_values, step, step_span));
    let range_loop = node(
        NodeKind::RangeLoop {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: range_loop_spec_with_domain(start, end, end_kind, step, domain),
            body: vec![],
        },
        location,
    );

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![range_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            expressions,
            const_values,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("range loop lowering should succeed");

    module
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

/// Whether the stepped fixture function keeps trap-mode or builtin-`Error!` loop failures.
#[derive(Clone, Copy)]
enum CollectionLoopFixtureFailure {
    Trap,
    BuiltinError,
}

/// Lowers one collection loop with an explicit signed step and both user bindings.
fn lower_collection_loop_with_step_fixture(
    step: FixedScalarValue,
    step_span: Option<SourceSpan>,
    failure: CollectionLoopFixtureFailure,
    body: Vec<crate::compiler_frontend::ast::ast_nodes::AstNode>,
) -> HirModule {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let location = None;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut const_values = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();

    let item_symbol = super::symbol("item", &mut path_fork, &mut string_table);
    let index_symbol = super::symbol("index", &mut path_fork, &mut string_table);
    let step_expression = fixed_literal(&mut expressions, &mut const_values, step, step_span);
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
            step: Some(step_expression),
            body,
        },
        location,
    );

    let returns = match failure {
        CollectionLoopFixtureFailure::Trap => CallableReturnShape::from_slots([], None),
        CollectionLoopFixtureFailure::BuiltinError => {
            let error_path = builtin_error_type_path(&mut path_fork, &mut string_table);
            let manifest = register_builtin_error_types(&mut path_fork, &mut string_table);
            let fields = manifest
                .resolved_struct_fields_by_path
                .get(&error_path)
                .expect("builtin Error should expose its canonical fields")
                .iter()
                .map(|field| FieldDefinition {
                    name: field.id,
                    type_id: field.type_id,
                    span: field.binding_span,
                })
                .collect::<Vec<_>>();
            let definition = StructTypeDefinition {
                id: NominalTypeId(0),
                path: error_path,
                fields: fields.into_boxed_slice(),
                generic_parameters: None,
                const_record: false,
            };
            let (_, error_type_id) = type_environment.register_nominal_struct(definition);
            type_environment
                .register_canonical_identity(
                    CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error),
                    error_type_id,
                )
                .expect("the fixture registers the canonical Error identity once");
            error_return_slot(error_type_id)
        }
    };

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(returns),
        },
        vec![collection_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            expressions,
            const_values,
            type_environment,
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("stepped collection loop lowering should succeed");

    module
}

fn collection_step_test_span() -> SourceSpan {
    let mut extended = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(9, 4, &mut extended).expect("short test span should fit");
    SourceSpan::new(SourceId::from_index(1), local)
}

/// The stable destination blocks and captured length of one stepped collection loop.
struct SteppedCollectionLoopCfg {
    entry: BlockId,
    length_local: LocalId,
    zero_check: BlockId,
    zero_failure: BlockId,
    empty_check: BlockId,
    seed_selector: BlockId,
    seed_positive: BlockId,
    seed_negative: BlockId,
    body: BlockId,
    step: BlockId,
    step_candidate: BlockId,
    step_commit: BlockId,
}

fn jump_target(module: &HirModule, block: BlockId, context: &str) -> BlockId {
    match module.blocks[block.0 as usize].terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("expected jump from {context}"),
    }
}

fn if_branches(module: &HirModule, block: BlockId, context: &str) -> (BlockId, BlockId) {
    match module.blocks[block.0 as usize].terminator {
        HirTerminator::If {
            then_block,
            else_block,
            ..
        } => (then_block, else_block),
        _ => panic!("expected conditional in {context}"),
    }
}

fn if_condition(module: &HirModule, block: BlockId, context: &str) -> HirValueId {
    match module.blocks[block.0 as usize].terminator {
        HirTerminator::If { condition, .. } => condition,
        _ => panic!("expected conditional in {context}"),
    }
}

fn loaded_local(module: &HirModule, value: HirValueId) -> Option<LocalId> {
    match &module.expressions.expression(value).kind {
        HirExpressionKind::Load(place) => Some(place.root),
        _ => None,
    }
}

fn copied_local(module: &HirModule, value: HirValueId) -> Option<LocalId> {
    match &module.expressions.expression(value).kind {
        HirExpressionKind::Copy(place) => Some(place.root),
        _ => None,
    }
}

fn value_is_zero(module: &HirModule, value: HirValueId, scalar: FixedScalar) -> bool {
    matches!(
        &module.expressions.expression(value).kind,
        HirExpressionKind::FixedScalar(literal)
            if *literal == FixedScalarValue::signed(scalar, 0).expect("zero fits the scalar")
    )
}

/// Walks the stepped collection CFG from the entry block.
///
/// WHY: reading the shape through terminators pins the semantic edges without depending on block
///      allocation order, and the length local comes from the one length call the loop emits.
fn stepped_collection_loop_cfg(module: &HirModule) -> SteppedCollectionLoopCfg {
    let start = &module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize];
    let entry = start.entry;
    let length_local = module.blocks[entry.0 as usize]
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Call {
                target: CallTarget::External(ExternalFunctionId::CollectionLength),
                results,
                ..
            } => match results.as_slice() {
                [HirLocalDestination::Define(local)] => Some(*local),
                _ => None,
            },
            _ => None,
        })
        .expect("stepped collection loop should capture its length once");

    let zero_check = jump_target(module, entry, "collection entry");
    let (zero_failure, empty_check) = if_branches(module, zero_check, "step zero check");
    let (empty_exit, seed_selector) = if_branches(module, empty_check, "empty check");
    let (seed_positive, seed_negative) = if_branches(module, seed_selector, "seed selector");
    let body = jump_target(module, seed_positive, "positive seed");
    assert_eq!(
        jump_target(module, seed_negative, "negative seed"),
        body,
        "both sign-selected seeds must enter the same body"
    );
    // A falling-through body jumps to the step; an authored `continue` targets it directly.
    let step = match module.blocks[body.0 as usize].terminator {
        HirTerminator::Jump { target, .. } | HirTerminator::Continue { target } => target,
        _ => panic!("expected the collection body to reach the step block"),
    };
    let (step_exit, step_candidate) = if_branches(module, step, "collection step");
    assert_eq!(
        step_exit, empty_exit,
        "the inclusive final-index check must target the loop exit"
    );
    let (step_commit, candidate_exit) = if_branches(module, step_candidate, "collection candidate");
    assert_eq!(
        candidate_exit, empty_exit,
        "an out-of-range candidate must target the loop exit"
    );
    assert_eq!(
        jump_target(module, step_commit, "collection step commit"),
        body,
        "an in-range candidate must commit the index and re-enter the body"
    );

    SteppedCollectionLoopCfg {
        entry,
        length_local,
        zero_check,
        zero_failure,
        empty_check,
        seed_selector,
        seed_positive,
        seed_negative,
        body,
        step,
        step_candidate,
        step_commit,
    }
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

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![range_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            expressions,
            const_values,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("range loop lowering should succeed");

    // The CFG walk pins the zero guard, direction selector and body edges. The loop is the last
    // statement in the start function, so its exit returns; candidate production and spans are
    // owned by the dedicated range tests.
    let cfg = range_loop_cfg_blocks(&module);
    assert!(matches!(
        module.blocks[cfg.exit.0 as usize].terminator,
        HirTerminator::Return(_)
    ));
    assert_no_placeholder_terminators(&module);
}

/// Bounds convert to the selected counter domain while the step keeps its own type.
///
/// WHAT: pins a `U8` counter domain over `I32` bounds and an `I32` `by -1` step, then asserts the
///       bounds gain counter-domain casts, the step operand on `RangeCandidate` stays `I32`, and
///       the only checked numeric operation left is the `I32` ordinal increment.
/// WHY: the header selects the counter from its bounds, but a signed step is independent of that
///      counter. Re-deriving or coercing the step to the counter would silently lose direction and
///      magnitude, and any checked counter arithmetic would reintroduce the out-of-domain
///      successor problem the candidate statement exists to solve.
#[test]
fn range_loop_lowering_converts_bounds_but_keeps_the_step_type() {
    let step_value = FixedScalarValue::signed(FixedScalar::I32, -1).expect("negative one fits I32");
    let module = lower_range_loop_fixture(
        NumericScalar::Fixed(FixedScalar::U8),
        FixedScalarValue::signed(FixedScalar::I32, 0).expect("zero fits I32"),
        FixedScalarValue::signed(FixedScalar::I32, 4).expect("four fits I32"),
        RangeEndKind::Exclusive,
        Some(step_value),
        None,
    );

    let u8_type = builtin_type_ids::fixed_scalar(FixedScalar::U8);
    let i32_type = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    // Both authored bounds convert from their own `I32` type to the `U8` counter domain.
    let converted_bound_locals: Vec<_> = module
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
                        target: NumericScalar::Fixed(FixedScalar::U8),
                    },
                    ..
                }
            )
            .then_some(*local)
        })
        .collect();
    assert_eq!(
        converted_bound_locals.len(),
        2,
        "both range bounds must convert from I32 to the U8 counter domain"
    );
    assert!(
        converted_bound_locals
            .iter()
            .all(|local| local_type(&module, *local) == Some(u8_type)),
        "counter state initialised from a converted bound must use the U8 counter domain"
    );

    // The step seed keeps its own `I32` type; no cast converts it to the counter.
    let step_local = module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| {
            let HirStatementKind::Write { value, target } = &statement.kind else {
                return None;
            };
            let HirWriteTarget::DefineLocal(local) = target else {
                return None;
            };
            matches!(
                &module.expressions.expression(*value).kind,
                HirExpressionKind::FixedScalar(literal) if *literal == step_value
            )
            .then_some(*local)
        })
        .expect("range lowering must seed the step local from the authored step");
    assert_eq!(
        local_type(&module, step_local),
        Some(i32_type),
        "an integer range step keeps its own signed I32 type"
    );

    let (candidate_step, candidate_domain) = module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            HirStatementKind::RangeCandidate { step, domain, .. } => Some((*step, *domain)),
            _ => None,
        })
        .expect("range lowering must emit one RangeCandidate");
    assert_eq!(candidate_domain, NumericScalar::Fixed(FixedScalar::U8));
    let candidate_step_row = module.expressions.expression(candidate_step);
    assert_eq!(
        candidate_step_row.ty, i32_type,
        "the RangeCandidate step operand must stay I32, not the counter domain"
    );
    match &candidate_step_row.kind {
        HirExpressionKind::Load(place) => assert_eq!(
            place.root, step_local,
            "the candidate must read the independently typed step local"
        ),
        other => panic!("expected a step local load, got {other:?}"),
    }

    // The advance is the ordinal `I32` add. Nothing else may remain: the step keeps its own type,
    // so an injected signed-abs `Negate` or `Subtract` on it would also be `I32` and must be
    // rejected by exact operator identity, not merely by domain.
    let ordinal_increment = HirNumericOp {
        operator: NumericOperator::Add,
        domain: NumericScalar::Fixed(FixedScalar::I32),
    };
    let numeric_ops: Vec<HirNumericOp> = module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .filter_map(|statement| match &statement.kind {
            HirStatementKind::NumericOp { op, .. } => Some(*op),
            _ => None,
        })
        .collect();
    assert!(
        numeric_ops.contains(&ordinal_increment),
        "the I32 ordinal increment must remain"
    );
    assert!(
        numeric_ops.iter().all(|op| *op == ordinal_increment),
        "only the I32 ordinal increment may remain, got {numeric_ops:?}"
    );

    // The zero guard runs first and tests the step, never the bounds.
    let cfg = range_loop_cfg_blocks(&module);
    let zero_condition = match module.blocks[cfg.step_zero_check.0 as usize].terminator {
        HirTerminator::If { condition, .. } => condition,
        _ => panic!("expected the zero-step guard"),
    };
    assert!(
        matches!(
            &module.expressions.expression(zero_condition).kind,
            HirExpressionKind::BinOp {
                op: HirBinOp::Eq,
                ..
            }
        ),
        "the zero guard must compare the step with zero"
    );

    // Direction is recorded after the guard from `step > 0`.
    let selector = &module.blocks[cfg.header_selector.0 as usize];
    let ascending_from_step = selector.statements.iter().any(|statement| {
        let HirStatementKind::Write {
            target: HirWriteTarget::DefineLocal(_),
            value,
        } = &statement.kind
        else {
            return false;
        };
        let HirExpressionKind::BinOp {
            left,
            op: HirBinOp::Gt,
            right,
        } = &module.expressions.expression(*value).kind
        else {
            return false;
        };
        module.expressions.expression(*left).ty == i32_type
            && module.expressions.expression(*right).ty == i32_type
    });
    assert!(
        ascending_from_step,
        "the header selector must record ascending from the I32 step sign"
    );
}

/// An inclusive endpoint exits before any candidate is produced.
///
/// WHY: the endpoint value is the last visited item, so the step block must branch to the exit on
///      `current == end` before the candidate could compute an out-of-range successor.
#[test]
fn inclusive_range_loop_exits_before_the_candidate() {
    let module = lower_range_loop_fixture(
        NumericScalar::Fixed(FixedScalar::I32),
        FixedScalarValue::signed(FixedScalar::I32, 0).expect("zero fits I32"),
        FixedScalarValue::signed(FixedScalar::I32, 3).expect("three fits I32"),
        RangeEndKind::Inclusive,
        None,
        None,
    );

    let cfg = range_loop_cfg_blocks(&module);
    let (endpoint_exit, candidate_block) = match module.blocks[cfg.step.0 as usize].terminator {
        HirTerminator::If {
            then_block,
            else_block,
            ..
        } => (then_block, else_block),
        _ => panic!("expected the inclusive endpoint branch in the step block"),
    };
    assert_eq!(
        endpoint_exit, cfg.exit,
        "the inclusive endpoint must exit the loop before the candidate"
    );
    assert!(
        module.blocks[candidate_block.0 as usize]
            .statements
            .iter()
            .any(|statement| matches!(
                statement.kind,
                HirStatementKind::RangeCandidate {
                    inclusive: true,
                    ..
                }
            )),
        "the candidate after the inclusive endpoint check must mark the bound inclusive"
    );
}

/// Binary-float ranges keep a no-progress guard; integer ranges never need one.
///
/// WHY: float candidate rounding can return the current value, which is a recoverable failure.
///      Integer addition is exact, so a nonzero step always advances and no guard is emitted.
#[test]
fn float_range_loops_keep_a_no_progress_guard_but_integer_ranges_do_not() {
    let float_module = lower_range_loop_fixture(
        NumericScalar::Fixed(FixedScalar::F64),
        FixedScalarValue::binary_float(FixedScalar::F64, 0.0).expect("zero fits F64"),
        FixedScalarValue::binary_float(FixedScalar::F64, 1.0).expect("one fits F64"),
        RangeEndKind::Inclusive,
        Some(FixedScalarValue::binary_float(FixedScalar::F64, 0.5).expect("half fits F64")),
        None,
    );
    let has_no_progress = float_module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .any(|statement| {
            matches!(
                statement.kind,
                HirStatementKind::LoopStepFailure {
                    cause: LoopStepFailureCause::NoProgress,
                    ..
                }
            )
        });
    assert!(
        has_no_progress,
        "a binary-float range candidate needs the rounded-result no-progress failure"
    );

    let integer_module = lower_range_loop_fixture(
        NumericScalar::Fixed(FixedScalar::I32),
        FixedScalarValue::signed(FixedScalar::I32, 0).expect("zero fits I32"),
        FixedScalarValue::signed(FixedScalar::I32, 3).expect("three fits I32"),
        RangeEndKind::Exclusive,
        None,
        None,
    );
    let has_no_progress = integer_module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .any(|statement| {
            matches!(
                statement.kind,
                HirStatementKind::LoopStepFailure {
                    cause: LoopStepFailureCause::NoProgress,
                    ..
                }
            )
        });
    assert!(
        !has_no_progress,
        "an exact integer range step never stalls, so no no-progress guard is emitted"
    );
}

#[test]
fn lowers_range_loop_without_user_bindings() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
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

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![range_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            expressions,
            const_values,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("range loop lowering without user bindings should succeed");

    let cfg = range_loop_cfg_blocks(&module);
    let body_locals = &module.blocks[cfg.body.0 as usize].locals;
    assert!(
        body_locals.is_empty(),
        "binding-less range loops should not allocate user binding locals"
    );
}

#[test]
fn lowers_range_loop_with_index_binding() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
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

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![range_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            expressions,
            const_values,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("range loop lowering with index should succeed");

    let cfg = range_loop_cfg_blocks(&module);
    let body_statements = &module.blocks[cfg.body.0 as usize].statements;
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

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![step_decl, range_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            expressions,
            const_values,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("dynamic-step range loop lowering should succeed");

    let cfg = range_loop_cfg_blocks(&module);

    let panic_block = match module.blocks[cfg.step_zero_check.0 as usize].terminator {
        HirTerminator::If { then_block, .. } => then_block,
        _ => panic!("expected runtime zero-check branch"),
    };

    assert!(
        module.blocks[panic_block.0 as usize]
            .statements
            .iter()
            .any(|statement| matches!(
                statement.kind,
                HirStatementKind::LoopStepFailure {
                    cause: LoopStepFailureCause::ZeroStep,
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
            step: None,
            body: vec![],
        },
        location,
    );

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![collection_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
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
fn stepped_collection_loop_captures_length_before_step_and_guards_zero_before_emptiness() {
    let module = lower_collection_loop_with_step_fixture(
        FixedScalarValue::signed(FixedScalar::I64, 2).expect("step fits I64"),
        None,
        CollectionLoopFixtureFailure::Trap,
        vec![],
    );
    let cfg = stepped_collection_loop_cfg(&module);
    let entry_statements = &module.blocks[cfg.entry.0 as usize].statements;

    let length_position = entry_statements
        .iter()
        .position(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Call {
                    target: CallTarget::External(ExternalFunctionId::CollectionLength),
                    ..
                }
            )
        })
        .expect("stepped collection loop should capture the source length");
    let step_local = entry_statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                ..
            } if local_type(&module, *local)
                == Some(builtin_type_ids::fixed_scalar(FixedScalar::I64)) =>
            {
                Some(*local)
            }
            _ => None,
        })
        .expect("stepped collection loop should evaluate one I64 step temporary");
    let step_position = entry_statements
        .iter()
        .position(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(local),
                    ..
                } if *local == step_local
            )
        })
        .expect("the evaluated step should be defined in the entry block");

    assert!(
        length_position < step_position,
        "the source and its length must be captured before the step expression is evaluated"
    );
    assert_eq!(
        module
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter(|statement| matches!(
                &statement.kind,
                HirStatementKind::Write {
                    target: HirWriteTarget::DefineLocal(local),
                    ..
                } if *local == step_local
            ))
            .count(),
        1,
        "the step expression must be evaluated exactly once"
    );

    let zero_condition = if_condition(&module, cfg.zero_check, "step zero check");
    let HirExpressionKind::BinOp {
        left,
        op: HirBinOp::Eq,
        right,
    } = &module.expressions.expression(zero_condition).kind
    else {
        panic!("step zero check should compare the step with zero");
    };
    assert_eq!(loaded_local(&module, *left), Some(step_local));
    assert!(
        value_is_zero(&module, *right, FixedScalar::I64),
        "the zero guard must use the step's own I64 type"
    );
    assert!(
        module.blocks[cfg.zero_failure.0 as usize]
            .statements
            .iter()
            .any(|statement| matches!(
                &statement.kind,
                HirStatementKind::LoopStepFailure {
                    cause: LoopStepFailureCause::ZeroStep,
                    ..
                }
            )),
        "a dynamic zero collection step should emit the shared loop-step failure"
    );

    let empty_condition = if_condition(&module, cfg.empty_check, "empty check");
    let HirExpressionKind::BinOp {
        left,
        op: HirBinOp::Eq,
        right,
    } = &module.expressions.expression(empty_condition).kind
    else {
        panic!("empty check should compare the captured length with zero");
    };
    assert_eq!(loaded_local(&module, *left), Some(cfg.length_local));
    assert!(
        value_is_zero(&module, *right, FixedScalar::I32),
        "empty check should compare the exact I32 captured length with zero"
    );
    assert_eq!(
        local_type(&module, cfg.length_local),
        Some(builtin_type_ids::fixed_scalar(FixedScalar::I32)),
        "collection lengths keep the exact I32 identity"
    );
}

#[test]
fn stepped_collection_loop_seeds_by_sign_and_advances_with_one_range_candidate() {
    let module = lower_collection_loop_with_step_fixture(
        FixedScalarValue::signed(FixedScalar::I64, 2).expect("step fits I64"),
        None,
        CollectionLoopFixtureFailure::Trap,
        vec![],
    );
    let cfg = stepped_collection_loop_cfg(&module);

    let zero_condition = if_condition(&module, cfg.zero_check, "step zero check");
    let HirExpressionKind::BinOp { left, .. } = &module.expressions.expression(zero_condition).kind
    else {
        panic!("step zero check should compare the step with zero");
    };
    let step_local = loaded_local(&module, *left).expect("zero check should load the step");

    // The inclusive final index is captured once as `length - 1` after the emptiness check.
    let (last_local, subtraction) = module.blocks[cfg.seed_selector.0 as usize]
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::NumericOp {
                op:
                    HirNumericOp {
                        operator: NumericOperator::Subtract,
                        domain: NumericScalar::Fixed(FixedScalar::I32),
                    },
                operands: HirNumericOperands::Binary { left, right },
                result: HirLocalDestination::Define(last),
                ..
            } => Some((*last, (*left, *right))),
            _ => None,
        })
        .expect("the seed selector should capture the inclusive final index");
    assert_eq!(loaded_local(&module, subtraction.0), Some(cfg.length_local));
    assert_eq!(
        signed_literal_value(&module, subtraction.1),
        Some(1),
        "the inclusive final index is the captured length minus one"
    );

    let selector_condition = if_condition(&module, cfg.seed_selector, "seed selector");
    let HirExpressionKind::BinOp {
        left,
        op: HirBinOp::Gt,
        right,
    } = &module.expressions.expression(selector_condition).kind
    else {
        panic!("the seed selector should test the step sign");
    };
    assert_eq!(loaded_local(&module, *left), Some(step_local));
    assert!(value_is_zero(&module, *right, FixedScalar::I64));

    let index_local = module.blocks[cfg.seed_positive.0 as usize]
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                ..
            } => Some(*local),
            _ => None,
        })
        .expect("the positive seed should define the index");
    assert!(
        module.blocks[cfg.seed_positive.0 as usize]
            .statements
            .iter()
            .any(|statement| matches!(
                &statement.kind,
                HirStatementKind::Write { value, .. } if value_is_zero(&module, *value, FixedScalar::I32)
            )),
        "a positive step starts at source index zero"
    );

    let negative_statements = &module.blocks[cfg.seed_negative.0 as usize].statements;
    let negative_index = negative_statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Write {
                target: HirWriteTarget::DefineLocal(local),
                value,
            } if *local == index_local => Some(*value),
            _ => None,
        })
        .expect("the negative seed should define the index");
    assert_eq!(
        copied_local(&module, negative_index),
        Some(last_local),
        "a negative step starts at the captured final source index"
    );
    assert!(
        negative_statements.iter().any(|statement| matches!(
            &statement.kind,
            HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(place),
                value,
            } if place.root == last_local && value_is_zero(&module, *value, FixedScalar::I32)
        )),
        "a negative step resets the inclusive final index to zero"
    );

    let step_condition = if_condition(&module, cfg.step, "collection step");
    let HirExpressionKind::BinOp {
        left,
        op: HirBinOp::Eq,
        right,
    } = &module.expressions.expression(step_condition).kind
    else {
        panic!("the step block should compare the cursor with the final index");
    };
    assert_eq!(loaded_local(&module, *left), Some(index_local));
    assert_eq!(loaded_local(&module, *right), Some(last_local));

    let candidate = module.blocks[cfg.step_candidate.0 as usize]
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::RangeCandidate {
                current,
                step,
                end,
                inclusive,
                domain,
                candidate_result,
                in_range_result,
            } => Some((
                *current,
                *step,
                *end,
                *inclusive,
                *domain,
                *candidate_result,
                *in_range_result,
            )),
            _ => None,
        })
        .expect("the stepped collection loop should produce one range candidate per advance");

    assert!(candidate.3, "the final source index is inclusive");
    assert_eq!(
        candidate.4,
        NumericScalar::Fixed(FixedScalar::I32),
        "the collection counter domain is the exact I32 source index"
    );
    assert_eq!(loaded_local(&module, candidate.0), Some(index_local));
    assert_eq!(loaded_local(&module, candidate.1), Some(step_local));
    assert_eq!(loaded_local(&module, candidate.2), Some(last_local));
    assert_eq!(
        module.expressions.expression(candidate.0).ty,
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert_eq!(
        module.expressions.expression(candidate.2).ty,
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
    assert_eq!(
        module.expressions.expression(candidate.1).ty,
        builtin_type_ids::fixed_scalar(FixedScalar::I64),
        "the wider signed step keeps its own type against the I32 index"
    );

    let candidate_condition = if_condition(&module, cfg.step_candidate, "candidate branch");
    let HirExpressionKind::Load(place) = &module.expressions.expression(candidate_condition).kind
    else {
        panic!("the candidate branch should test the Bool result");
    };
    assert_eq!(place.root, candidate.6.local());

    let commit_value = module.blocks[cfg.step_commit.0 as usize]
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(place),
                value,
            } if place.root == index_local => Some(*value),
            _ => None,
        })
        .expect("an in-range candidate should commit the cursor");
    assert_eq!(
        copied_local(&module, commit_value),
        Some(candidate.5.local())
    );

    // A negative step keeps its narrow I8 identity instead of being widened or coerced.
    let narrow = lower_collection_loop_with_step_fixture(
        FixedScalarValue::signed(FixedScalar::I8, -2).expect("step fits I8"),
        None,
        CollectionLoopFixtureFailure::Trap,
        vec![],
    );
    let narrow_candidate = narrow
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            HirStatementKind::RangeCandidate { step, domain, .. } => Some((*step, *domain)),
            _ => None,
        })
        .expect("a negative I8 step should still advance through a range candidate");
    assert_eq!(
        narrow.expressions.expression(narrow_candidate.0).ty,
        builtin_type_ids::fixed_scalar(FixedScalar::I8),
        "the negative I8 step must not be widened to the I32 index"
    );
    assert_eq!(
        narrow_candidate.1,
        NumericScalar::Fixed(FixedScalar::I32),
        "the counter domain stays the exact I32 source index"
    );
}

#[test]
fn collection_zero_step_failure_carries_the_authored_step_span() {
    let step_span = collection_step_test_span();
    let module = lower_collection_loop_with_step_fixture(
        FixedScalarValue::signed(FixedScalar::I64, 2).expect("step fits I64"),
        Some(step_span),
        CollectionLoopFixtureFailure::Trap,
        vec![],
    );
    let failure_span = module
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            HirStatementKind::LoopStepFailure {
                cause: LoopStepFailureCause::ZeroStep,
                ..
            } => Some(statement.span),
            _ => None,
        })
        .expect("trap-mode stepped loops should emit the shared zero-step failure");
    assert_eq!(
        failure_span,
        Some(step_span),
        "the zero-step failure must carry the authored step span"
    );
}

/// Range and collection candidates carry the authored step position.
///
/// WHY: backend target validation rejects a candidate domain at the candidate's statement span,
///      and an `F16` range always has an authored step. A spanless candidate would report a
///      target capability limit with no location to repair.
#[test]
fn range_candidates_carry_the_authored_step_span() {
    let step_span = collection_step_test_span();
    let range = lower_range_loop_fixture(
        NumericScalar::Fixed(FixedScalar::F64),
        FixedScalarValue::binary_float(FixedScalar::F64, 0.0).expect("zero fits F64"),
        FixedScalarValue::binary_float(FixedScalar::F64, 1.0).expect("one fits F64"),
        RangeEndKind::Exclusive,
        Some(FixedScalarValue::binary_float(FixedScalar::F64, 0.5).expect("half fits F64")),
        Some(step_span),
    );
    let range_candidate_span = range
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match statement.kind {
            HirStatementKind::RangeCandidate { .. } => Some(statement.span),
            _ => None,
        })
        .expect("a range loop should advance through a range candidate");
    assert_eq!(
        range_candidate_span,
        Some(step_span),
        "a range candidate must carry the authored step span"
    );

    let collection = lower_collection_loop_with_step_fixture(
        FixedScalarValue::signed(FixedScalar::I64, 2).expect("step fits I64"),
        Some(step_span),
        CollectionLoopFixtureFailure::Trap,
        vec![],
    );
    let collection_candidate_span = collection
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match statement.kind {
            HirStatementKind::RangeCandidate { .. } => Some(statement.span),
            _ => None,
        })
        .expect("a stepped collection loop should advance through a range candidate");
    assert_eq!(
        collection_candidate_span,
        Some(step_span),
        "a collection candidate must carry the authored step span"
    );
}

#[test]
fn recoverable_stepped_collection_loop_has_zero_step_edge_without_seed_overflow_edge() {
    let module = lower_collection_loop_with_step_fixture(
        FixedScalarValue::signed(FixedScalar::I64, 2).expect("step fits I64"),
        None,
        CollectionLoopFixtureFailure::BuiltinError,
        vec![],
    );

    let zero_step_invokes = module
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                &block.terminator,
                HirTerminator::Invoke {
                    operation: HirFallibleOperation::LoopStepFailure {
                        cause: LoopStepFailureCause::ZeroStep,
                    },
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        zero_step_invokes, 1,
        "a builtin Error! function should emit one recoverable zero-step failure edge"
    );

    let seed_subtract_invokes = module
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                &block.terminator,
                HirTerminator::Invoke {
                    operation: HirFallibleOperation::NumericOp {
                        op: HirNumericOp {
                            operator: NumericOperator::Subtract,
                            domain: NumericScalar::Fixed(FixedScalar::I32),
                        },
                        ..
                    },
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        seed_subtract_invokes, 0,
        "the proven-safe seed subtraction must not add an IntOverflow failure edge"
    );

    let seed_subtract_mode = module
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            HirStatementKind::NumericOp {
                op:
                    HirNumericOp {
                        operator: NumericOperator::Subtract,
                        domain: NumericScalar::Fixed(FixedScalar::I32),
                    },
                failure_mode,
                ..
            } => Some(*failure_mode),
            _ => None,
        })
        .expect("the seed should still compute its inclusive final index");
    assert_eq!(
        seed_subtract_mode,
        NumericFailureMode::Infallible,
        "the empty check proves the seed subtraction cannot overflow"
    );
}

fn signed_literal_value(module: &HirModule, value: HirValueId) -> Option<i64> {
    match &module.expressions.expression(value).kind {
        HirExpressionKind::FixedScalar(literal) => literal.as_i64(),
        _ => None,
    }
}

#[test]
fn continue_targets_step_block_in_stepped_collection_loop() {
    let location = None;
    let module = lower_collection_loop_with_step_fixture(
        FixedScalarValue::signed(FixedScalar::I64, 2).expect("step fits I64"),
        None,
        CollectionLoopFixtureFailure::Trap,
        vec![node(NodeKind::Continue, location)],
    );
    let cfg = stepped_collection_loop_cfg(&module);

    match module.blocks[cfg.body.0 as usize].terminator {
        HirTerminator::Continue { target } => assert_eq!(
            target, cfg.step,
            "continue must advance by the signed step rather than exit"
        ),
        _ => panic!("expected continue terminator in stepped collection body"),
    }
}

#[test]
fn direct_break_in_stepped_collection_loop_discards_unreachable_candidate_blocks() {
    let location = None;
    let module = lower_collection_loop_with_step_fixture(
        FixedScalarValue::signed(FixedScalar::I64, 2).expect("step fits I64"),
        None,
        CollectionLoopFixtureFailure::Trap,
        vec![node(NodeKind::Break, location)],
    );

    assert_no_placeholder_terminators(&module);
    assert!(
        module
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .all(|statement| !matches!(statement.kind, HirStatementKind::RangeCandidate { .. })),
        "a body that always breaks should discard the unreachable candidate and commit blocks"
    );

    let break_target = module
        .blocks
        .iter()
        .find_map(|block| match block.terminator {
            HirTerminator::Break { target } => Some(target),
            _ => None,
        })
        .expect("the stepped body should retain its authored break");
    assert!(
        matches!(
            module.blocks[break_target.0 as usize].terminator,
            HirTerminator::Return(_)
        ),
        "break must leave the loop at the exit block rather than advancing the step"
    );
}

#[test]
fn lowers_collection_loop_without_user_bindings() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
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
            step: None,
            body: vec![],
        },
        location,
    );

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![collection_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
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
            step: None,
            body: vec![],
        },
        location,
    );

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![collection_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
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
            step: None,
            body: vec![],
        },
        location,
    );

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![collection_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
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
    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![range_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
            expressions,
            const_values,
            TypeEnvironment::new(),
        ),
        &mut string_table,
        &mut path_fork,
        &super::ExternalPackageRegistry::new(),
    )
    .expect("range loop lowering should succeed");

    let cfg = range_loop_cfg_blocks(&module);
    let body_locals = &module.blocks[cfg.body.0 as usize].locals;
    assert!(
        !body_locals.is_empty() && body_locals.iter().all(|local| !local.mutable),
        "range loop user binding locals should be immutable"
    );
}

#[test]
fn lowers_collection_loop_user_bindings_as_immutable_locals() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
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
            step: None,
            body: vec![],
        },
        location,
    );

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![collection_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
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
            step: None,
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

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![flag_decl, collection_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
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
            step: None,
            body: vec![node(NodeKind::Break, location)],
        },
        location,
    );

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![collection_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
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

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![range_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
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
            step: None,
            body: vec![node(NodeKind::Continue, location)],
        },
        location,
    );

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![collection_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
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
            step: None,
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
            step: None,
            body: vec![inner_loop, node(NodeKind::Continue, location)],
        },
        location,
    );

    let start_fn = super::entry_start_node(
        FunctionSignature {
            parameters: vec![],
            return_contract: FunctionReturnContract::Returns(CallableReturnShape::from_slots(
                [],
                None,
            )),
        },
        vec![outer_loop],
    );

    let (module, _type_environment) = lower_ast(
        build_ast_with_registered_types(
            vec![start_fn],
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
