//! Hand-authored normalized BorrowProblem fixtures and invariant tests.

mod fixtures;

use super::{
    AccessKind, BlockId, BorrowProblem, CallResultProvenance, Event, EventId, EventKind,
    EventSource, OriginKind, PlaceId, PlaceOverlap, PointId, ProgramPoint, ProjectionElem,
    RebindValue, TerminatorEventKind, UseId, ValueOriginId, from_hir,
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{
    BlockId as HirBlockId, FunctionId, HirNodeId, HirValueId, LocalId, RegionId,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallMutationEffect, PublicCallParameterAccess,
    PublicCallParameterSummary, PublicCallReactiveEffect, PublicCallSummary,
    PublicCallTransferEffect, PublicCallTransferEligibility,
};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use fixtures::{
    branch_join, copy, empty, field_accesses, loop_with_rebind, old_alias_after_rebind,
    same_statement_access_order,
};

#[test]
fn borrow_problem_copy_fixture_validates_and_keeps_source_and_result_origins_distinct() {
    let problem = BorrowProblem::new(copy()).expect("copy fixture should validate");

    assert_eq!(problem.origins().len(), 2);
    assert!(matches!(
        &problem.events()[1].kind,
        EventKind::Copy {
            origin,
            ..
        } if *origin == ValueOriginId::new(1)
    ));
    assert_eq!(problem.events()[2].id.raw(), 2);
}

#[test]
fn borrow_problem_fresh_rebind_fixture_preserves_old_alias_after_new_definition() {
    let problem = BorrowProblem::new(old_alias_after_rebind())
        .expect("old alias after rebind fixture should validate");

    assert!(matches!(
        &problem.events()[1].kind,
        EventKind::Alias {
            origins,
            ..
        } if origins.as_ref() == [ValueOriginId::new(0)]
    ));
    assert!(matches!(
        &problem.events()[2].kind,
        EventKind::Rebind {
            value: RebindValue::Fresh(origin),
            ..
        } if *origin == ValueOriginId::new(1)
    ));
}

#[test]
fn borrow_problem_branch_join_fixture_retains_explicit_join_origin() {
    let problem = BorrowProblem::new(branch_join()).expect("branch fixture should validate");

    assert_eq!(problem.control_flow().edges.len(), 4);
    assert!(matches!(
        &problem.origins()[3].kind,
        OriginKind::Join(origins)
            if origins.as_ref() == [ValueOriginId::new(1), ValueOriginId::new(2)]
    ));
}

#[test]
fn borrow_problem_loop_fixture_accepts_a_back_edge_without_flattening_the_cfg() {
    let problem = BorrowProblem::new(loop_with_rebind()).expect("loop fixture should validate");

    assert!(
        problem
            .control_flow()
            .edges
            .iter()
            .any(|edge| edge.from.raw() == 1 && edge.to.raw() == 1)
    );
}

#[test]
fn borrow_problem_field_fixture_keeps_disjoint_fields_and_base_overlap_explicit() {
    let problem = BorrowProblem::new(field_accesses()).expect("field fixture should validate");
    let base = &problem.places()[0];
    let left = &problem.places()[1];
    let right = &problem.places()[2];

    assert_eq!(left.overlap(right), PlaceOverlap::Disjoint);
    assert_eq!(base.overlap(left), PlaceOverlap::Overlap);
    assert_eq!(
        problem.places()[1].projections.as_ref(),
        [ProjectionElem::Field(0)]
    );
}

#[test]
fn borrow_problem_same_statement_access_fixture_preserves_event_order_at_one_point() {
    let problem = BorrowProblem::new(same_statement_access_order())
        .expect("same-statement access fixture should validate");

    assert_eq!(problem.events()[0].point, problem.events()[1].point);
    assert_eq!(
        problem.control_flow().blocks[0].events.as_ref(),
        [EventId::new(0), EventId::new(1),]
    );
    assert!(matches!(
        &problem.events()[0].kind,
        EventKind::Access { use_id } if *use_id == UseId::new(0)
    ));
    assert!(matches!(
        &problem.events()[1].kind,
        EventKind::Access { use_id } if *use_id == UseId::new(1)
    ));
}

#[test]
fn borrow_problem_malformed_dense_ids_fail_through_the_internal_compiler_error_lane() {
    let mut parts = empty();
    parts.points[0].id = PointId::new(1);

    let error = BorrowProblem::new(parts).expect_err("non-dense point IDs must be rejected");

    assert!(matches!(error.error_type, ErrorType::Compiler));
    assert!(error.msg.contains("program-point IDs must be dense"));
}

#[test]
fn borrow_problem_malformed_unowned_event_fails_atomic_construction() {
    let mut parts = empty();
    parts.events.push(Event::new(
        EventId::new(0),
        PointId::new(1),
        EventKind::Fresh {
            destination: PlaceId::new(0),
            origin: ValueOriginId::new(0),
        },
        EventSource::none(),
    ));

    let error = BorrowProblem::new(parts).expect_err("every event must belong to one block");

    assert!(matches!(error.error_type, ErrorType::Compiler));
    assert!(error.msg.contains("every normalized event"));
}

#[test]
fn borrow_problem_rejects_points_outside_their_block_range() {
    let mut parts = empty();
    parts
        .points
        .push(ProgramPoint::new(PointId::new(2), BlockId::new(0), 2));

    let error = BorrowProblem::new(parts).expect_err("point range must be coherent");

    assert!(matches!(error.error_type, ErrorType::Compiler));
    assert!(error.msg.contains("outside the entry/exit range"));
}

#[test]
fn borrow_problem_deterministic_debug_dump_is_stable_for_equal_problems() {
    let first = BorrowProblem::new(copy()).expect("copy fixture should validate");
    let second = BorrowProblem::new(copy()).expect("copy fixture should validate");

    assert_eq!(first.debug_dump(), second.debug_dump());
}

#[test]
fn borrow_problem_hir_extractor_preserves_ordered_events_and_scope_exit() {
    let (module, function) = hir_literal_fixture();
    let first = from_hir(&module, &function, None, None).expect("HIR fixture should extract");
    let second = from_hir(&module, &function, None, None).expect("HIR fixture should re-extract");

    assert_eq!(first.debug_dump(), second.debug_dump());
    assert_eq!(first.bindings().len(), 1);
    assert_eq!(first.places().len(), 1);
    assert!(
        first
            .events()
            .iter()
            .any(|event| matches!(event.kind, EventKind::Fresh { .. }))
    );
    assert!(
        first
            .events()
            .iter()
            .any(|event| matches!(event.kind, EventKind::Access { .. }))
    );
    assert!(
        first
            .events()
            .iter()
            .any(|event| matches!(event.kind, EventKind::Terminator { .. }))
    );
    assert!(
        first
            .events()
            .iter()
            .any(|event| matches!(event.kind, EventKind::ScopeExit { .. }))
    );
    assert!(
        first
            .events()
            .iter()
            .any(|event| event.source.hir_node == Some(HirNodeId(0)))
    );
    assert!(
        first
            .points()
            .windows(2)
            .all(|points| points[0].id.raw() < points[1].id.raw())
    );
}

#[test]
fn borrow_problem_hir_extractor_emits_aggregate_storage_events() {
    let region = RegionId(0);
    let target = LocalId(0);
    let expression = HirExpression {
        id: HirValueId(0),
        kind: HirExpressionKind::Collection(vec![
            int_expression(1, 1, region),
            int_expression(2, 2, region),
        ]),
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::RValue,
        region,
    };
    let module = module_with_block(HirBlock {
        id: HirBlockId(0),
        region,
        locals: vec![hir_local(target, region)],
        statements: vec![HirStatement {
            id: HirNodeId(0),
            kind: HirStatementKind::Assign {
                target: HirPlace::Local(target),
                value: expression,
            },
            location: SourceLocation::default(),
        }],
        terminator: HirTerminator::Return(load_expression(3, target, region)),
    });
    let function = function_for(HirBlockId(0));
    let problem = from_hir(&module, &function, None, None).expect("aggregate HIR should extract");

    assert!(problem.events().iter().any(|event| matches!(
        &event.kind,
        EventKind::Aggregate { fields, .. } if fields.len() == 2
    )));
}

#[test]
fn borrow_problem_hir_extractor_imports_call_access_and_result_alias_facts() {
    let region = RegionId(0);
    let source_local = LocalId(0);
    let result_local = LocalId(1);
    let module = module_with_block(HirBlock {
        id: HirBlockId(0),
        region,
        locals: vec![
            hir_local(source_local, region),
            hir_local(result_local, region),
        ],
        statements: vec![
            HirStatement {
                id: HirNodeId(0),
                kind: HirStatementKind::Assign {
                    target: HirPlace::Local(source_local),
                    value: int_expression(0, 1, region),
                },
                location: SourceLocation::default(),
            },
            HirStatement {
                id: HirNodeId(1),
                kind: HirStatementKind::Call {
                    target: CallTarget::Local(FunctionId(9)),
                    args: vec![load_expression(2, source_local, region)],
                    result: Some(result_local),
                },
                location: SourceLocation::default(),
            },
        ],
        terminator: HirTerminator::Return(load_expression(3, result_local, region)),
    });
    let function = function_for(HirBlockId(0));
    let mut summaries = rustc_hash::FxHashMap::default();
    summaries.insert(
        FunctionId(9),
        PublicCallSummary {
            parameters: vec![PublicCallParameterSummary {
                access: PublicCallParameterAccess::Mutable,
                mutation: PublicCallMutationEffect::Writes,
                transfer_eligibility: PublicCallTransferEligibility::Ineligible,
                transfer_effect: PublicCallTransferEffect::NeverConsumes,
                reactive_effect: PublicCallReactiveEffect::None,
            }],
            return_alias: FunctionReturnAliasSummary::AliasParams(vec![0]),
        },
    );
    let problem =
        from_hir(&module, &function, Some(&summaries), None).expect("call HIR should extract");

    let call = problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::CallEffect(effect) => Some(effect),
            _ => None,
        })
        .expect("call effect should be present");
    assert_eq!(call.arguments.len(), 1);
    assert_eq!(call.arguments[0].access, AccessKind::Exclusive);
    assert!(problem.origins().iter().any(|origin| matches!(
        &origin.kind,
        OriginKind::CallResult {
            provenance: CallResultProvenance::AliasParams(indices),
            ..
        } if indices.as_ref() == [0]
    )));
}

#[test]
fn borrow_problem_hir_extractor_preserves_fallible_success_and_error_edges() {
    let region = RegionId(0);
    let branch = HirBlock {
        id: HirBlockId(0),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::FallibleBranch {
            result: int_expression(0, 0, region),
            success_block: HirBlockId(1),
            error_block: HirBlockId(2),
        },
    };
    let success = HirBlock {
        id: HirBlockId(1),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::ReturnSuccess(int_expression(1, 1, region)),
    };
    let error = HirBlock {
        id: HirBlockId(2),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::ReturnError(int_expression(2, 2, region)),
    };
    let module = HirModule {
        blocks: vec![branch, success, error],
        regions: vec![HirRegion::lexical(region, None)],
        ..HirModule::new()
    };
    let function = function_for(HirBlockId(0));
    let problem =
        from_hir(&module, &function, None, None).expect("fallible control-flow HIR should extract");

    assert!(problem.events().iter().any(|event| matches!(
        event.kind,
        EventKind::Terminator {
            kind: TerminatorEventKind::Branch { .. }
        }
    )));
    assert!(problem.events().iter().any(|event| matches!(
        event.kind,
        EventKind::Terminator {
            kind: TerminatorEventKind::ReturnSuccess
        }
    )));
    assert!(problem.events().iter().any(|event| matches!(
        event.kind,
        EventKind::Terminator {
            kind: TerminatorEventKind::ReturnError
        }
    )));
    assert_eq!(problem.control_flow().edges.len(), 2);
}

fn hir_literal_fixture() -> (HirModule, HirFunction) {
    let local = LocalId(0);
    let region = RegionId(0);
    let literal = HirExpression {
        id: HirValueId(0),
        kind: HirExpressionKind::Int(7),
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::RValue,
        region,
    };
    let load = HirExpression {
        id: HirValueId(1),
        kind: HirExpressionKind::Load(HirPlace::Local(local)),
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::Place,
        region,
    };
    let module = HirModule {
        blocks: vec![HirBlock {
            id: HirBlockId(0),
            region,
            locals: vec![HirLocal {
                id: local,
                ty: builtin_type_ids::INT,
                mutable: true,
                region,
                source_info: None,
            }],
            statements: vec![HirStatement {
                id: HirNodeId(0),
                kind: HirStatementKind::Assign {
                    target: HirPlace::Local(local),
                    value: literal,
                },
                location: SourceLocation::default(),
            }],
            terminator: HirTerminator::Return(load),
        }],
        functions: vec![],
        regions: vec![HirRegion::lexical(region, None)],
        ..HirModule::new()
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: HirBlockId(0),
        params: vec![],
        return_type: builtin_type_ids::INT,
    };
    (module, function)
}

fn module_with_block(block: HirBlock) -> HirModule {
    let region = block.region;
    HirModule {
        blocks: vec![block],
        regions: vec![HirRegion::lexical(region, None)],
        ..HirModule::new()
    }
}

fn function_for(entry: HirBlockId) -> HirFunction {
    HirFunction {
        id: FunctionId(0),
        entry,
        params: vec![],
        return_type: builtin_type_ids::INT,
    }
}

fn hir_local(id: LocalId, region: RegionId) -> HirLocal {
    HirLocal {
        id,
        ty: builtin_type_ids::INT,
        mutable: true,
        region,
        source_info: None,
    }
}

fn int_expression(id: u32, value: i32, region: RegionId) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind: HirExpressionKind::Int(value),
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::RValue,
        region,
    }
}

fn load_expression(id: u32, local: LocalId, region: RegionId) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind: HirExpressionKind::Load(HirPlace::Local(local)),
        ty: builtin_type_ids::INT,
        value_kind: ValueKind::Place,
        region,
    }
}
