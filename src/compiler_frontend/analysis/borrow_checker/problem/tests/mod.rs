//! Hand-authored normalized BorrowProblem fixtures and invariant tests.

mod fixtures;

use super::{
    AccessKind, BindingId, BlockId, BorrowProblem, Call, CallArgument, CallEffect, CallId,
    CallResult, CallResultProvenance, CallResultUnknownReason, Event, EventId, EventKind,
    EventSource, OriginKind, PlaceId, PlaceOverlap, PointId, ProgramPoint, ProjectionElem,
    RebindValue, TerminatorEventKind, Use, UseId, UseKind, ValueOrigin, ValueOriginId, from_hir,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
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
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModulePrivateExecutableCategory,
    ModulePrivateExecutableIdentity, ModuleRootRole, OriginFunctionId, StableModuleOriginIdentity,
    StablePackageIdentity,
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
fn borrow_problem_rejects_projected_binding_definitions() {
    let mut parts = field_accesses();
    parts.uses[0].kind = UseKind::Write;
    parts.uses[0].definition = true;

    let error = BorrowProblem::new(parts).expect_err("projected definitions must be rejected");
    assert!(format!("{error:?}").contains("projected place"));
}

#[test]
fn borrow_problem_same_statement_access_fixture_preserves_event_order_at_one_point() {
    let problem = BorrowProblem::new(same_statement_access_order())
        .expect("same-statement access fixture should validate");

    assert_eq!(problem.events()[0].point, problem.events()[1].point);
    assert_eq!(
        problem.control_flow().blocks[0].events.as_ref(),
        [EventId::new(0), EventId::new(1), EventId::new(2),]
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
        EventId::new(1),
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
fn borrow_problem_rejects_exit_block_without_a_terminator() {
    let mut parts = empty();
    parts.events.clear();
    parts.blocks[0].events = Vec::new().into_boxed_slice();

    let error = BorrowProblem::new(parts).expect_err("exit blocks need terminal events");

    assert!(error.msg.contains("must end in a terminator event"));
}

#[test]
fn borrow_problem_rejects_nonterminal_terminator_on_an_exit_block() {
    let mut parts = empty();
    parts.events[0].kind = EventKind::Terminator {
        kind: TerminatorEventKind::Jump {
            target: BlockId::new(0),
        },
    };

    let error = BorrowProblem::new(parts).expect_err("exit blocks need terminal terminators");

    assert!(error.msg.contains("must end in a terminal terminator"));
}

#[test]
fn borrow_problem_rejects_terminator_edges_that_disagree_with_the_cfg() {
    let mut parts = branch_join();
    if let EventKind::Terminator {
        kind: TerminatorEventKind::Branch { targets },
    } = &mut parts.events[4].kind
    {
        *targets = vec![BlockId::new(1)].into_boxed_slice();
    }

    let error = BorrowProblem::new(parts).expect_err("terminator edges must match the CFG");

    assert!(error.msg.contains("do not match its outgoing edges"));
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
fn borrow_problem_rejects_call_effect_without_granular_arguments() {
    let mut parts = granular_call_parts();
    parts.events.remove(0);
    parts.events[0].id = EventId::new(0);
    parts.events[1].id = EventId::new(1);
    parts.blocks[0].events = vec![EventId::new(0), EventId::new(1)].into();

    let error = BorrowProblem::new(parts).expect_err("non-empty calls need argument events");

    assert!(error.msg.contains("no granular argument events"));
}

#[test]
fn borrow_problem_rejects_granular_argument_after_call_effect() {
    let mut parts = granular_call_parts();
    parts.events[0].point = PointId::new(2);
    parts.events[1].point = PointId::new(1);
    parts.uses[0].point = PointId::new(2);
    parts.blocks[0].events = vec![EventId::new(1), EventId::new(0), EventId::new(2)].into();

    let error = BorrowProblem::new(parts).expect_err("argument events must precede the effect");

    assert!(error.msg.contains("must precede its CallEffect"));
}

#[test]
fn borrow_problem_rejects_call_argument_with_inconsistent_access_kind() {
    let mut parts = granular_call_parts();
    if let EventKind::CallArgument { argument, .. } = &mut parts.events[0].kind {
        argument.access = AccessKind::Exclusive;
    }
    if let EventKind::CallEffect(effect) = &mut parts.events[1].kind {
        effect.arguments[0].access = AccessKind::Exclusive;
    }

    let error = BorrowProblem::new(parts).expect_err("call argument access must match its use");

    assert!(error.msg.contains("access kind inconsistent"));
}

#[test]
fn borrow_problem_rejects_granular_argument_without_call_effect() {
    let mut parts = granular_call_parts();
    parts.events.remove(1);
    parts.events[1].id = EventId::new(1);
    parts.blocks[0].events = vec![EventId::new(0), EventId::new(1)].into();

    let error = BorrowProblem::new(parts).expect_err("argument events need a call effect");

    assert!(error.msg.contains("no CallEffect event"));
}

#[test]
fn borrow_problem_rejects_granular_arguments_in_declared_order_only() {
    let mut parts = two_argument_call_parts();
    parts.uses[0].point = PointId::new(2);
    parts.uses[1].point = PointId::new(1);
    let first_argument = match &parts.events[0].kind {
        EventKind::CallArgument { argument, .. } => argument.clone(),
        _ => panic!("first event should be a call argument"),
    };
    let second_argument = match &parts.events[1].kind {
        EventKind::CallArgument { argument, .. } => argument.clone(),
        _ => panic!("second event should be a call argument"),
    };
    parts.events[0].kind = EventKind::CallArgument {
        call: CallId::new(0),
        index: 1,
        argument: second_argument,
    };
    parts.events[1].kind = EventKind::CallArgument {
        call: CallId::new(0),
        index: 0,
        argument: first_argument,
    };

    let error = BorrowProblem::new(parts).expect_err("CFG argument order must be semantic order");

    assert!(error.msg.contains("do not exactly match its CallEffect"));
}

#[test]
fn borrow_problem_rejects_call_result_alias_parameter_out_of_range() {
    let mut parts = granular_call_parts();
    parts.origins.push(ValueOrigin::new(
        ValueOriginId::new(1),
        OriginKind::CallResult {
            call: CallId::new(0),
            provenance: CallResultProvenance::AliasParams(vec![1].into_boxed_slice()),
        },
    ));
    if let EventKind::CallEffect(effect) = &mut parts.events[1].kind {
        effect.result = Some(CallResult {
            place: PlaceId::new(0),
            origin: ValueOriginId::new(1),
        });
    }

    let error = BorrowProblem::new(parts).expect_err("call result parameter index must be valid");

    assert!(error.msg.contains("outside call"));
}

#[test]
fn borrow_problem_rejects_empty_alias_params() {
    let mut parts = granular_call_parts();
    parts.origins.push(ValueOrigin::new(
        ValueOriginId::new(1),
        OriginKind::CallResult {
            call: CallId::new(0),
            provenance: CallResultProvenance::AliasParams(Vec::new().into_boxed_slice()),
        },
    ));
    if let EventKind::CallEffect(effect) = &mut parts.events[1].kind {
        effect.result = Some(CallResult {
            place: PlaceId::new(0),
            origin: ValueOriginId::new(1),
        });
    }

    // WHAT: an empty AliasParams index list claims an argument derivation without naming
    // any argument.
    // WHY: that is malformed normalized input. Accepting it let the solver publish the
    // call-result origin as a fresh independent generation; the invariant must fail at
    // validation with a CompilerError naming the origin, call, event and result place.
    let error = BorrowProblem::new(parts).expect_err("empty AliasParams is malformed input");

    assert!(
        error.msg.contains("empty AliasParams")
            && error.msg.contains("CallEffect event")
            && error.msg.contains("result place")
            && error.msg.contains("EventId(1)")
            && error.msg.contains("PlaceId(0)"),
        "expected the rejection to name origin, call, event and result place, got: {error:?}"
    );
}

#[test]
fn borrow_problem_rejects_detached_call_result_origin() {
    let mut parts = granular_call_parts();
    parts.origins.push(ValueOrigin::new(
        ValueOriginId::new(1),
        OriginKind::CallResult {
            call: CallId::new(0),
            provenance: CallResultProvenance::AliasParams(vec![0].into_boxed_slice()),
        },
    ));

    let error = BorrowProblem::new(parts).expect_err("call result origins need an owning effect");

    assert!(error.msg.contains("not attached to a CallEffect"));
}

#[test]
fn borrow_problem_rejects_call_result_origin_from_another_call() {
    let mut parts = granular_call_parts();
    parts.calls.push(Call {
        id: CallId::new(1),
        label: "other-call".to_owned(),
    });
    parts.origins.push(ValueOrigin::new(
        ValueOriginId::new(1),
        OriginKind::CallResult {
            call: CallId::new(1),
            provenance: CallResultProvenance::Fresh,
        },
    ));
    if let EventKind::CallEffect(effect) = &mut parts.events[1].kind {
        effect.result = Some(CallResult {
            place: PlaceId::new(0),
            origin: ValueOriginId::new(1),
        });
    }

    let error = BorrowProblem::new(parts).expect_err("result origin must belong to its call");

    assert!(error.msg.contains("inconsistent call ownership"));
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
fn borrow_problem_hir_extractor_seeds_parameter_origins() {
    let local = LocalId(0);
    let region = RegionId(0);
    let module = module_with_block(HirBlock {
        id: HirBlockId(0),
        region,
        locals: vec![hir_local(local, region)],
        statements: Vec::new(),
        terminator: HirTerminator::Return(load_expression(0, local, region)),
    });
    let function = HirFunction {
        id: FunctionId(0),
        entry: HirBlockId(0),
        params: vec![local],
        return_type: builtin_type_ids::INT,
    };
    let problem = from_hir(&module, &function, None, None).expect("parameter HIR should extract");

    assert!(
        problem
            .origins()
            .iter()
            .any(|origin| matches!(origin.kind, OriginKind::Parameter { index: 0 }))
    );
    assert!(problem.events().iter().any(|event| matches!(
        event.kind,
        EventKind::Fresh { destination, .. } if destination.raw() == 0
    )));
}

#[test]
fn borrow_problem_hir_extractor_places_scope_exit_on_the_losing_edge() {
    let parent = RegionId(0);
    let child = RegionId(1);
    let local = LocalId(0);
    let condition = HirExpression {
        id: HirValueId(0),
        kind: HirExpressionKind::Bool(true),
        ty: builtin_type_ids::BOOL,
        value_kind: ValueKind::RValue,
        region: child,
    };
    let module = HirModule {
        blocks: vec![
            HirBlock {
                id: HirBlockId(0),
                region: child,
                locals: vec![hir_local(local, child)],
                statements: Vec::new(),
                terminator: HirTerminator::If {
                    condition,
                    then_block: HirBlockId(1),
                    else_block: HirBlockId(2),
                },
            },
            HirBlock {
                id: HirBlockId(1),
                region: child,
                locals: Vec::new(),
                statements: Vec::new(),
                terminator: HirTerminator::Return(int_expression(1, 1, child)),
            },
            HirBlock {
                id: HirBlockId(2),
                region: parent,
                locals: Vec::new(),
                statements: Vec::new(),
                terminator: HirTerminator::Return(int_expression(2, 2, parent)),
            },
        ],
        regions: vec![
            HirRegion::lexical(parent, None),
            HirRegion::lexical(child, Some(parent)),
        ],
        ..HirModule::new()
    };
    let function = function_for(HirBlockId(0));
    let problem = from_hir(&module, &function, None, None).expect("edge scope HIR should extract");

    let scope_exit_blocks = problem
        .control_flow()
        .blocks
        .iter()
        .filter(|block| {
            block.events.iter().any(|event_id| {
                problem.events()[event_id.index()]
                    .kind
                    .eq(&EventKind::ScopeExit {
                        bindings: vec![BindingId::new(0)].into_boxed_slice(),
                    })
            })
        })
        .map(|block| block.id)
        .collect::<Vec<_>>();
    assert_eq!(scope_exit_blocks.len(), 2);
    let edge_block = scope_exit_blocks
        .iter()
        .copied()
        .find(|block_id| {
            problem.control_flow().blocks[block_id.index()]
                .events
                .iter()
                .any(|event_id| {
                    matches!(
                        problem.events()[event_id.index()].kind,
                        EventKind::Terminator {
                            kind: TerminatorEventKind::Jump { .. }
                        }
                    )
                })
        })
        .expect("losing edge should have a synthetic jump block");
    let incoming_edges = problem
        .control_flow()
        .edges
        .iter()
        .filter(|edge| edge.to == edge_block)
        .collect::<Vec<_>>();
    let outgoing_edges = problem
        .control_flow()
        .edges
        .iter()
        .filter(|edge| edge.from == edge_block)
        .collect::<Vec<_>>();
    assert_eq!(incoming_edges.len(), 1);
    assert_eq!(outgoing_edges.len(), 1);

    let edge_events = &problem.control_flow().blocks[edge_block.index()].events;
    assert_eq!(edge_events.len(), 2);
    assert!(matches!(
        &problem.events()[edge_events[0].index()].kind,
        EventKind::ScopeExit { bindings }
            if bindings.as_ref() == [BindingId::new(0)]
    ));
    assert!(matches!(
        &problem.events()[edge_events[1].index()].kind,
        EventKind::Terminator {
            kind: TerminatorEventKind::Jump { target }
        } if *target == outgoing_edges[0].to
    ));
}

#[test]
fn borrow_problem_hir_extractor_carries_ancestor_scope_exit_to_later_blocks() {
    let parent = RegionId(0);
    let child = RegionId(1);
    let local = LocalId(0);
    let module = HirModule {
        blocks: vec![
            HirBlock {
                id: HirBlockId(0),
                region: child,
                locals: vec![hir_local(local, child)],
                statements: Vec::new(),
                terminator: HirTerminator::Jump {
                    target: HirBlockId(1),
                    args: Vec::new(),
                },
            },
            HirBlock {
                id: HirBlockId(1),
                region: child,
                locals: Vec::new(),
                statements: Vec::new(),
                terminator: HirTerminator::If {
                    condition: HirExpression {
                        id: HirValueId(1),
                        kind: HirExpressionKind::Bool(true),
                        ty: builtin_type_ids::BOOL,
                        value_kind: ValueKind::RValue,
                        region: child,
                    },
                    then_block: HirBlockId(2),
                    else_block: HirBlockId(3),
                },
            },
            HirBlock {
                id: HirBlockId(2),
                region: child,
                locals: Vec::new(),
                statements: Vec::new(),
                terminator: HirTerminator::Return(int_expression(2, 2, child)),
            },
            HirBlock {
                id: HirBlockId(3),
                region: parent,
                locals: Vec::new(),
                statements: Vec::new(),
                terminator: HirTerminator::Return(int_expression(3, 3, parent)),
            },
        ],
        regions: vec![
            HirRegion::lexical(parent, None),
            HirRegion::lexical(child, Some(parent)),
        ],
        ..HirModule::new()
    };
    let function = function_for(HirBlockId(0));
    let problem = from_hir(&module, &function, None, None)
        .expect("ancestor scope should survive through the intermediate block");

    let scope_exit_blocks = problem
        .control_flow()
        .blocks
        .iter()
        .filter(|block| {
            block.events.iter().any(|event_id| {
                matches!(
                    &problem.events()[event_id.index()].kind,
                    EventKind::ScopeExit { bindings }
                        if bindings.as_ref() == [BindingId::new(0)]
                )
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(scope_exit_blocks.len(), 2);
    assert!(scope_exit_blocks.iter().any(|block| {
        matches!(
            problem.events()[block.events.last().expect("edge block has jump").index()].kind,
            EventKind::Terminator {
                kind: TerminatorEventKind::Jump { .. }
            }
        )
    }));
    assert!(scope_exit_blocks.iter().any(|block| {
        matches!(
            problem.events()[block
                .events
                .last()
                .expect("return block has terminator")
                .index()]
            .kind,
            EventKind::Terminator {
                kind: TerminatorEventKind::Return
            }
        )
    }));
}

#[test]
fn borrow_problem_hir_extractor_deduplicates_repeated_scope_exit_successors() {
    let parent = RegionId(0);
    let child = RegionId(1);
    let local = LocalId(0);
    let module = HirModule {
        blocks: vec![
            HirBlock {
                id: HirBlockId(0),
                region: child,
                locals: vec![hir_local(local, child)],
                statements: Vec::new(),
                terminator: HirTerminator::If {
                    condition: HirExpression {
                        id: HirValueId(0),
                        kind: HirExpressionKind::Bool(true),
                        ty: builtin_type_ids::BOOL,
                        value_kind: ValueKind::RValue,
                        region: child,
                    },
                    then_block: HirBlockId(1),
                    else_block: HirBlockId(1),
                },
            },
            HirBlock {
                id: HirBlockId(1),
                region: parent,
                locals: Vec::new(),
                statements: Vec::new(),
                terminator: HirTerminator::Return(int_expression(1, 1, parent)),
            },
        ],
        regions: vec![
            HirRegion::lexical(parent, None),
            HirRegion::lexical(child, Some(parent)),
        ],
        ..HirModule::new()
    };
    let function = function_for(HirBlockId(0));
    let problem = from_hir(&module, &function, None, None)
        .expect("repeated branch targets should share one edge block");

    let branch_targets = problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Terminator {
                kind: TerminatorEventKind::Branch { targets },
            } => Some(targets),
            _ => None,
        })
        .expect("HIR branch should be normalized");
    assert_eq!(branch_targets.len(), 1);
    assert_eq!(
        problem
            .control_flow()
            .edges
            .iter()
            .filter(|edge| edge.from.raw() == 0)
            .count(),
        1
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

    let (destination, fields) = problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Aggregate {
                destination,
                fields,
                ..
            } if fields.len() == 2 => Some((*destination, fields)),
            _ => None,
        })
        .expect("aggregate event should retain its children");
    let destination_place = &problem.places()[destination.index()];
    for field in fields {
        assert!(problem.places().iter().any(|place| {
            place.root == destination_place.root && place.projections.as_ref() == [field.projection]
        }));
    }
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
fn borrow_problem_hir_extractor_uses_conservative_generated_fallback_without_summary() {
    let region = RegionId(0);
    let source_local = LocalId(0);
    let result_local = LocalId(1);
    let generated_identity = GeneratedFunctionIdentity::new(
        GeneratedDeclarationIdentity::ModulePrivate(ModulePrivateExecutableIdentity::new(
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local("boracle-generated-fallback"),
                "main".to_owned(),
                ModuleRootRole::Normal,
            ),
            "@page.moth".to_owned(),
            ModulePrivateExecutableCategory::GenericFunction,
            "identity".to_owned(),
            None,
        )),
        Box::new([CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int)]),
        Box::new([]),
    );
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
                    target: CallTarget::Generated(generated_identity),
                    args: vec![load_expression(2, source_local, region)],
                    result: Some(result_local),
                },
                location: SourceLocation::default(),
            },
        ],
        terminator: HirTerminator::Return(load_expression(3, result_local, region)),
    });
    let function = function_for(HirBlockId(0));
    let problem = from_hir(&module, &function, None, None)
        .expect("generated call without a summary should extract conservatively");
    let effect = problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::CallEffect(effect) => Some(effect),
            _ => None,
        })
        .expect("generated call effect should be present");

    assert!(
        problem.calls()[effect.call.index()]
            .label
            .starts_with("Generated(")
    );
    assert!(matches!(
        effect.arguments.as_ref(),
        [argument] if argument.access == AccessKind::Exclusive
    ));
    let result = effect
        .result
        .as_ref()
        .expect("generated call should retain its result row");
    assert!(matches!(
        &problem.origins()[result.origin.index()].kind,
        OriginKind::CallResult {
            provenance: CallResultProvenance::Unknown(CallResultUnknownReason::MissingSummary),
            ..
        }
    ));
}

#[test]
fn borrow_problem_hir_extractor_rejects_missing_imported_summary() {
    let region = RegionId(0);
    let source_local = LocalId(0);
    let result_local = LocalId(1);
    let imported_identity = OriginFunctionId::new_free(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("boracle-imported-summary"),
            "provider".to_owned(),
            ModuleRootRole::Normal,
        ),
        "missing".to_owned(),
    );
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
                    target: CallTarget::CrossModule(imported_identity),
                    args: vec![load_expression(2, source_local, region)],
                    result: Some(result_local),
                },
                location: SourceLocation::default(),
            },
        ],
        terminator: HirTerminator::Return(load_expression(3, result_local, region)),
    });
    let function = function_for(HirBlockId(0));
    let error = from_hir(&module, &function, None, None)
        .expect_err("missing imported call summaries must fail extraction");

    assert!(format!("{error:?}").contains("provider call summary"));
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

fn granular_call_parts() -> super::BorrowProblemParts {
    let argument = CallArgument {
        place: PlaceId::new(0),
        access: AccessKind::Shared,
        use_id: UseId::new(0),
    };
    super::BorrowProblemParts {
        bindings: vec![super::Binding::synthetic(BindingId::new(0))],
        points: vec![
            ProgramPoint::new(PointId::new(0), BlockId::new(0), 0),
            ProgramPoint::new(PointId::new(1), BlockId::new(0), 1),
            ProgramPoint::new(PointId::new(2), BlockId::new(0), 2),
            ProgramPoint::new(PointId::new(3), BlockId::new(0), 3),
        ],
        blocks: vec![super::CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(3),
            vec![EventId::new(0), EventId::new(1), EventId::new(2)],
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![super::Place::new(
            PlaceId::new(0),
            BindingId::new(0),
            Vec::new(),
        )],
        origins: vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        uses: vec![Use {
            id: UseId::new(0),
            point: PointId::new(1),
            place: PlaceId::new(0),
            kind: UseKind::Read,
            definition: false,
        }],
        calls: vec![Call {
            id: super::CallId::new(0),
            label: "malformed-call".to_owned(),
        }],
        events: vec![
            Event::new(
                EventId::new(0),
                PointId::new(1),
                EventKind::CallArgument {
                    call: super::CallId::new(0),
                    index: 0,
                    argument: argument.clone(),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(1),
                PointId::new(2),
                EventKind::CallEffect(CallEffect {
                    call: super::CallId::new(0),
                    arguments: vec![argument].into_boxed_slice(),
                    result: None,
                }),
                EventSource::none(),
            ),
            Event::new(
                EventId::new(2),
                PointId::new(3),
                EventKind::Terminator {
                    kind: TerminatorEventKind::Return,
                },
                EventSource::none(),
            ),
        ],
        ..super::BorrowProblemParts::default()
    }
}

fn two_argument_call_parts() -> super::BorrowProblemParts {
    let first_argument = CallArgument {
        place: PlaceId::new(0),
        access: AccessKind::Shared,
        use_id: UseId::new(0),
    };
    let second_argument = CallArgument {
        place: PlaceId::new(1),
        access: AccessKind::Shared,
        use_id: UseId::new(1),
    };
    super::BorrowProblemParts {
        bindings: vec![
            super::Binding::synthetic(BindingId::new(0)),
            super::Binding::synthetic(BindingId::new(1)),
        ],
        points: vec![
            ProgramPoint::new(PointId::new(0), BlockId::new(0), 0),
            ProgramPoint::new(PointId::new(1), BlockId::new(0), 1),
            ProgramPoint::new(PointId::new(2), BlockId::new(0), 2),
            ProgramPoint::new(PointId::new(3), BlockId::new(0), 3),
            ProgramPoint::new(PointId::new(4), BlockId::new(0), 4),
        ],
        blocks: vec![super::CfgBlock::new(
            BlockId::new(0),
            PointId::new(0),
            PointId::new(4),
            vec![
                EventId::new(0),
                EventId::new(1),
                EventId::new(2),
                EventId::new(3),
            ],
        )],
        entry: BlockId::new(0),
        exits: vec![BlockId::new(0)],
        places: vec![
            super::Place::new(PlaceId::new(0), BindingId::new(0), Vec::new()),
            super::Place::new(PlaceId::new(1), BindingId::new(1), Vec::new()),
        ],
        origins: vec![ValueOrigin::fresh(ValueOriginId::new(0))],
        uses: vec![
            Use {
                id: UseId::new(0),
                point: PointId::new(1),
                place: PlaceId::new(0),
                kind: UseKind::Read,
                definition: false,
            },
            Use {
                id: UseId::new(1),
                point: PointId::new(2),
                place: PlaceId::new(1),
                kind: UseKind::Read,
                definition: false,
            },
        ],
        calls: vec![Call {
            id: CallId::new(0),
            label: "two-argument-call".to_owned(),
        }],
        events: vec![
            Event::new(
                EventId::new(0),
                PointId::new(1),
                EventKind::CallArgument {
                    call: CallId::new(0),
                    index: 0,
                    argument: first_argument.clone(),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(1),
                PointId::new(2),
                EventKind::CallArgument {
                    call: CallId::new(0),
                    index: 1,
                    argument: second_argument.clone(),
                },
                EventSource::none(),
            ),
            Event::new(
                EventId::new(2),
                PointId::new(3),
                EventKind::CallEffect(CallEffect {
                    call: CallId::new(0),
                    arguments: vec![first_argument, second_argument].into_boxed_slice(),
                    result: None,
                }),
                EventSource::none(),
            ),
            Event::new(
                EventId::new(3),
                PointId::new(4),
                EventKind::Terminator {
                    kind: TerminatorEventKind::Return,
                },
                EventSource::none(),
            ),
        ],
        ..super::BorrowProblemParts::default()
    }
}
