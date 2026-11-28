//! Private failure lane producer selection and exact transfer-input receipts.

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::numeric_operators::{NumericOperator, NumericSemantics};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::numeric::{LoopStepFailureCause, NumericFailureMode};
use crate::compiler_frontend::hir::private_failure_lane::{
    PrivateFailureLaneReceipt, install_private_failure_lanes,
};
use crate::compiler_frontend::hir::statements::{HirLocalDestination, HirStatementKind};
use crate::compiler_frontend::hir::terminators::{
    HirFallibleOperation, HirTerminator, RuntimeFailureCause,
};
use crate::compiler_frontend::hir::utils::collect_structural_blocks;
use crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::borrow_fixture_support::run_prepared_borrow_checker;
use crate::compiler_frontend::tests::hir_fixture_support::PreparedHirFixture;
use rustc_hash::FxHashSet;
use std::sync::Arc;

fn function_id_by_name(
    module: &HirModule,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> FunctionId {
    module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .and_then(|path| path_fork.component(path))
                .is_some_and(|component| string_table.resolve(component) == name)
        })
        .expect("authored function must lower")
        .id
}

fn numeric_op_modes(module: &HirModule, operator: NumericOperator) -> Vec<NumericFailureMode> {
    module
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match &statement.kind {
            HirStatementKind::NumericOp {
                op, failure_mode, ..
            } if op.operator == operator => Some(*failure_mode),
            _ => None,
        })
        .collect()
}

struct LaneFixture {
    hir: HirModule,
    type_environment: TypeEnvironment,
    report: BorrowCheckReport,
    registry: Arc<ExternalPackageRegistry>,
    path_fork: PathInternerFork,
    string_table: StringTable,
}

impl LaneFixture {
    fn new(source: &str) -> Self {
        let prepared = PreparedHirFixture::new(source);
        let mut report = run_prepared_borrow_checker(&prepared)
            .expect("lane fixtures must pass borrow checking");
        let hir = prepared.hir.hir_module;
        infer_builtin_failure_summaries(&hir, &mut report);
        Self {
            hir,
            type_environment: prepared.hir.type_environment,
            report,
            registry: prepared.packages,
            path_fork: prepared.paths,
            string_table: prepared.strings,
        }
    }

    fn function_id(&self, name: &str) -> FunctionId {
        function_id_by_name(&self.hir, &self.path_fork, &self.string_table, name)
    }

    fn entry(&self, id: FunctionId) -> BlockId {
        self.hir
            .functions
            .iter()
            .find(|function| function.id == id)
            .expect("fixture function must exist")
            .entry
    }

    fn install_with_receipt(&mut self) -> PrivateFailureLaneReceipt {
        install_private_failure_lanes(
            &mut self.hir,
            &self.report,
            &mut self.type_environment,
            &self.registry,
        )
        .expect("lane installation must succeed")
    }
}

fn install_lanes(
    source: &str,
) -> (
    HirModule,
    TypeEnvironment,
    BorrowCheckReport,
    PathInternerFork,
    StringTable,
) {
    let LaneFixture {
        mut hir,
        mut type_environment,
        report,
        registry,
        path_fork,
        string_table,
    } = LaneFixture::new(source);
    let numeric_producers = hir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match &statement.kind {
            HirStatementKind::NumericOp {
                op,
                failure_mode: NumericFailureMode::Trap,
                result: HirLocalDestination::Define(local),
                ..
            } if op.operator == NumericOperator::Multiply => {
                Some((statement.id, statement.span, *local))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    install_private_failure_lanes(&mut hir, &report, &mut type_environment, &registry)
        .expect("lane installation must succeed");
    for (statement, span, destination) in numeric_producers {
        assert!(
            hir.blocks.iter().any(|block| matches!(
                &block.terminator,
                HirTerminator::Invoke { id, span: operation_span, success_results, .. }
                    if *id == statement && *operation_span == span
                        && success_results == &[destination]
            )),
            "installation preserves numeric producer identity, span and success destination"
        );
    }
    (hir, type_environment, report, path_fork, string_table)
}

#[test]
fn lane_installation_splits_code_carrying_traps_but_not_codeless_float_negation() {
    // `negate` becomes a lane function through its escaping `failer` call. Its `I32`
    // multiplication (code-carrying) must split onto the inferred lane, while its
    // `F64` negation (empty shared code set) must keep its lowering-time trap.
    let (hir, _, report, path_fork, string_table) = install_lanes(
        "failer |value I32| -> I32:\n    return value * 2\n;\n\
         negate |value F64, seed I32| -> F64:\n    bumped = failer(seed)\n    return -value\n;\n",
    );
    let negate = function_id_by_name(&hir, &path_fork, &string_table, "negate");
    assert!(
        report.analysis.public_call_summaries[&negate].escapes_builtin_failure,
        "the escaping call must make the helper a lane function"
    );
    assert!(
        hir.blocks.iter().any(|block| matches!(
            &block.terminator,
            HirTerminator::Invoke {
                operation: HirFallibleOperation::NumericOp { op, .. }, ..
            } if op.operator == NumericOperator::Multiply
        )),
        "code-carrying arithmetic in a lane function must join the inferred lane"
    );
    assert_eq!(
        numeric_op_modes(&hir, NumericOperator::Negate),
        vec![NumericFailureMode::Trap],
        "codeless F64 negation must keep its trap instead of gaining a dead error edge"
    );
    assert!(
        hir.blocks
            .iter()
            .any(|block| matches!(block.terminator, HirTerminator::ReturnError(_))),
        "the lane function must still return its genuine failure"
    );
}

#[test]
fn start_narrowing_drops_only_a_non_escaping_error_channel() {
    for (source, escapes) in [
        ("value = \"plain\"\n~[:[value]]\n", false),
        (
            "load || -> String, Error!:\n    return! Error(\"failed\")\n;\n\
             value = load()!\n~[:[value]]\n",
            true,
        ),
        (
            "load || -> String, Error!:\n    return! Error(\"failed\")\n;\n\
             value = load() catch then \"recovered\"\n~[:[value]]\n",
            false,
        ),
    ] {
        let LaneFixture {
            mut hir,
            mut type_environment,
            report,
            registry,
            ..
        } = LaneFixture::new(source);
        let start = hir.start_function.expect("normal roots must have a start");
        let function = hir
            .functions
            .iter()
            .find(|function| function.id == start)
            .expect("start must have a function record");
        let success_slots = function
            .return_contract
            .ordinary()
            .expect("start fixture is ordinary")
            .success()
            .to_vec();
        let error_slot = function
            .return_contract
            .ordinary()
            .expect("start fixture is ordinary")
            .error();
        assert!(
            error_slot.is_some(),
            "lowering retains the AST builtin Error slot"
        );
        install_private_failure_lanes(&mut hir, &report, &mut type_environment, &registry)
            .expect("start narrowing must leave valid native HIR");
        let function = hir
            .functions
            .iter()
            .find(|function| function.id == start)
            .expect("installation must retain start");
        assert_eq!(
            function
                .return_contract
                .ordinary()
                .expect("start fixture is ordinary")
                .success(),
            success_slots.as_slice(),
            "success slots remain unchanged"
        );
        assert_eq!(
            function
                .return_contract
                .ordinary()
                .expect("start fixture is ordinary")
                .error(),
            if escapes { error_slot } else { None }
        );
    }
}

#[test]
fn lane_receipt_reports_outcome_only_entry_narrowing() {
    let mut fixture = LaneFixture::new("value = \"plain\"\n~[:[value]]\n");
    let start = fixture
        .hir
        .start_function
        .expect("fixture must have an entry");

    let receipt = fixture.install_with_receipt();
    assert_eq!(receipt.outcome_changes, FxHashSet::from_iter([start]));
    assert!(receipt.producer_splits.is_empty());
    assert!(receipt.newly_executable_handlers.is_empty());
    assert!(receipt.implicit_failure_retargets.is_empty());
    assert!(!receipt.block_remap.renumbered());
    for block in &fixture.hir.blocks {
        assert_eq!(receipt.block_remap.get(block.id), Some(block.id));
    }
    assert_eq!(
        receipt
            .block_remap
            .get(BlockId(fixture.hir.blocks.len() as u32)),
        None
    );
}

#[test]
fn lane_receipt_reports_splits_but_only_newly_executable_handlers() {
    let mut fixture = LaneFixture::new(
        "double |value I32| -> I32:\n    return value * 2\n;\n\
         recover |value I32| -> I32:\n    return double(value) catch then -1\n;\n\
         shared |value I32| -> I32:\n\
             return double(value) + value * 1000000 catch then -1\n;\n",
    );
    let double = fixture.function_id("double");
    let recover = fixture.function_id("recover");
    let shared = fixture.function_id("shared");
    let start = fixture
        .hir
        .start_function
        .expect("fixture must have an entry");
    let handler_for = |owner| {
        fixture
            .hir
            .catch_handlers
            .iter()
            .find(|record| record.owner == owner)
            .expect("fixture must retain the catch record")
            .handler
            .block
    };
    let new_handler = handler_for(recover);
    let old_handler = handler_for(shared);
    assert!(
        !collect_structural_blocks(&fixture.hir, fixture.entry(recover)).contains(&new_handler)
    );
    assert!(collect_structural_blocks(&fixture.hir, fixture.entry(shared)).contains(&old_handler));

    let mut producers: FxHashSet<_> = fixture
        .hir
        .catch_protected_calls
        .iter()
        .map(|record| (record.owner, record.statement))
        .collect();
    assert_eq!(producers.len(), 2);
    let multiply = fixture.hir.blocks[fixture.entry(double).0 as usize]
        .statements
        .iter()
        .find(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::NumericOp { op, .. } if op.operator == NumericOperator::Multiply
            )
        })
        .expect("double must lower to a multiply producer");
    producers.insert((double, multiply.id));

    let receipt = fixture.install_with_receipt();
    assert_eq!(
        receipt.outcome_changes,
        FxHashSet::from_iter([start, double])
    );
    assert_eq!(receipt.producer_splits, producers);
    assert_eq!(
        receipt.newly_executable_handlers,
        FxHashSet::from_iter([(recover, new_handler)])
    );
    assert!(receipt.implicit_failure_retargets.is_empty());
    let installed_handler = receipt
        .block_remap
        .get(new_handler)
        .expect("handler survives");
    assert!(
        collect_structural_blocks(&fixture.hir, fixture.entry(recover))
            .contains(&installed_handler)
    );
}

#[test]
fn lane_receipt_reports_implicit_conversion_failure_retargets() {
    let mut fixture = LaneFixture::new(
        "parse_count |text String| -> I32:\n    return cast text\n;\n\
         adjust |extra I8| -> U8:\n\
             total ~U8 = 250\n    ~total += extra\n    return total\n;\n",
    );
    let parse = fixture.function_id("parse_count");
    let adjust = fixture.function_id("adjust");
    let start = fixture
        .hir
        .start_function
        .expect("fixture must have an entry");
    let retargets: FxHashSet<_> = [parse, adjust]
        .into_iter()
        .map(|owner| {
            let block = collect_structural_blocks(&fixture.hir, fixture.entry(owner))
                .into_iter()
                .find(|id| {
                    matches!(
                        fixture.hir.blocks[id.0 as usize].terminator,
                        HirTerminator::RuntimeFailure {
                            cause: Some(RuntimeFailureCause::AuthoredCastConversion { .. }),
                            ..
                        } if owner == parse
                    ) || matches!(
                        fixture.hir.blocks[id.0 as usize].terminator,
                        HirTerminator::RuntimeFailure {
                            cause: Some(RuntimeFailureCause::StoreConversion { .. }),
                            ..
                        } if owner == adjust
                    )
                })
                .expect("fixture must lower to an implicit conversion failure");
            (owner, block)
        })
        .collect();

    let receipt = fixture.install_with_receipt();
    assert_eq!(
        receipt.outcome_changes,
        FxHashSet::from_iter([start, parse, adjust])
    );
    // Mixed U8 + I8 widens before the checked store; neither conversion is an installer split.
    assert!(receipt.producer_splits.is_empty());
    assert!(receipt.newly_executable_handlers.is_empty());
    assert_eq!(receipt.implicit_failure_retargets, retargets);
    for (_, block) in retargets {
        let installed = receipt
            .block_remap
            .get(block)
            .expect("failure block survives");
        assert!(matches!(
            fixture.hir.blocks[installed.0 as usize].terminator,
            HirTerminator::ReturnError(_)
        ));
    }
}

#[test]
fn lane_receipt_reports_prune_only_compaction_without_dirtying_transfer_inputs() {
    // The forward calls leave a provisional handler; inference proves both calls infallible.
    // The typed escaping call keeps the entry's Error slot, so installation only prunes.
    let mut fixture = LaneFixture::new(
        "mixed |left I32, right I32| -> Bool:\n\
             return calm(left) < calm(right) catch then true\n;\n\
         plain |value I32| -> I32:\n    return value\n;\n\
         calm |value I32| -> I32:\n    return value\n;\n\
         load || -> String, Error!:\n    return! Error(\"failed\")\n;\n\
         value = load()!\n~[:[value]]\n",
    );
    let mixed = fixture.function_id("mixed");
    let handler = fixture
        .hir
        .catch_handlers
        .iter()
        .find(|record| record.owner == mixed)
        .expect("fixture must retain the provisional handler")
        .handler
        .block;
    let reachable: FxHashSet<_> = fixture
        .hir
        .functions
        .iter()
        .flat_map(|function| collect_structural_blocks(&fixture.hir, function.entry))
        .collect();
    assert!(!reachable.contains(&handler));
    let old_count = fixture.hir.blocks.len();

    let receipt = fixture.install_with_receipt();
    assert!(receipt.outcome_changes.is_empty());
    assert!(receipt.producer_splits.is_empty());
    assert!(receipt.newly_executable_handlers.is_empty());
    assert!(receipt.implicit_failure_retargets.is_empty());
    assert!(receipt.link_facts_need_refresh);
    assert!(receipt.block_remap.renumbered());
    assert_eq!(receipt.block_remap.get(handler), None);
    let mut next = 0;
    let mut moved = false;
    for before in (0..old_count as u32).map(BlockId) {
        if reachable.contains(&before) {
            let after = BlockId(next);
            assert_eq!(receipt.block_remap.get(before), Some(after));
            moved |= before != after;
            next += 1;
        } else {
            assert_eq!(receipt.block_remap.get(before), None);
        }
    }
    assert!(moved, "pruning must renumber a retained block");
    assert_eq!(next as usize, fixture.hir.blocks.len());
}

#[test]
fn inferred_range_guard_has_no_success_slots() {
    let (hir, _, _, _, _) =
        install_lanes("walk |step I32|:\n    loop 0 to 4 by step |value|:\n    ;\n;\n");
    let mut guards = 0;
    for block in &hir.blocks {
        if let HirTerminator::Invoke {
            operation: HirFallibleOperation::LoopStepFailure { .. },
            success_results,
            success,
            ..
        } = &block.terminator
        {
            guards += 1;
            assert!(
                success_results.is_empty(),
                "a terminal loop-step guard produces no success values"
            );
            assert!(
                matches!(
                    hir.blocks[success.0 as usize].terminator,
                    HirTerminator::RuntimeFailure { .. }
                ),
                "the structurally retained success continuation must remain terminal"
            );
        }
    }
    assert!(
        guards > 0,
        "the runtime step must retain its zero-step guard"
    );
}

#[test]
fn inferred_catch_error_adapter_defines_the_explicit_handler_destination() {
    let (hir, _, _, path_fork, string_table) = install_lanes(
        "double |value I32| -> I32:\n    return value * 2\n;\n\
         recover |value I32| -> I32:\n\
             return double(value) + value * 1000000 catch then -1\n;\n",
    );
    let double = function_id_by_name(&hir, &path_fork, &string_table, "double");
    let mut routed_calls = 0;
    for block in &hir.blocks {
        let HirTerminator::Invoke {
            operation:
                HirFallibleOperation::Call {
                    target: crate::compiler_frontend::external_packages::CallTarget::Local(target),
                    ..
                },
            failure,
            error_result,
            ..
        } = &block.terminator
        else {
            continue;
        };
        if *target != double {
            continue;
        }
        let HirTerminator::Jump { args, .. } = &hir.blocks[failure.0 as usize].terminator else {
            panic!("caught inferred failures must enter the shared handler through an adapter");
        };
        assert_eq!(
            args.len(),
            1,
            "handler transport must explicitly define its error destination"
        );
        assert_eq!(args[0].source, *error_result);
        assert_ne!(
            args[0].destination, *error_result,
            "the edge error is separate from the shared handler binding"
        );
        routed_calls += 1;
    }
    assert_eq!(routed_calls, 1);
}

#[test]
fn fast_traps_never_join_the_private_failure_lane() {
    // `divide` becomes a lane function through its escaping `failer` call. Its fast integer
    // division traps by definition, so lane installation must leave it a fatal statement instead
    // of retargeting it onto the inferred recoverable edge.
    let source = "failer |value I32| -> I32:\n    return value * 2\n;\n\
         $fast_math\n\
         divide |value I32, divisor I32| -> I32:\n    bumped = failer(value)\n    return value // divisor\n;\n";
    let LaneFixture {
        mut hir,
        mut type_environment,
        report,
        registry,
        path_fork,
        string_table,
    } = LaneFixture::new(source);
    let divide = function_id_by_name(&hir, &path_fork, &string_table, "divide");
    assert!(
        report.analysis.public_call_summaries[&divide].escapes_builtin_failure,
        "the escaping call must make the helper a lane function"
    );

    install_private_failure_lanes(&mut hir, &report, &mut type_environment, &registry)
        .expect("lane installation must succeed");

    assert_eq!(
        numeric_op_modes(&hir, NumericOperator::IntegerDivide),
        vec![NumericFailureMode::Fatal],
        "fast division must keep its fatal delivery"
    );
    assert!(
        !hir.blocks.iter().any(|block| matches!(
            &block.terminator,
            HirTerminator::Invoke {
                operation: HirFallibleOperation::NumericOp { op, .. }, ..
            } if op.operator == NumericOperator::IntegerDivide
        )),
        "no recoverable edge may carry a fast numeric operation"
    );
    assert!(
        hir.blocks.iter().any(|block| matches!(
            &block.terminator,
            HirTerminator::Invoke {
                operation: HirFallibleOperation::Call { .. },
                ..
            }
        )),
        "the escaping call still uses the inferred lane"
    );
}

#[test]
fn fast_range_guards_and_ordinal_never_join_the_private_failure_lane() {
    // The fast loop lives in a lane function because of an unrelated escaping call. Its
    // generated zero-step guard and ordinary checked ordinal overflow are fatal deliveries; lane
    // installation must not rewrite them into recoverable `Invoke` edges, or a catch handler
    // would intercept a `$fast_math` trap.
    let source = "failer |value I32| -> I32:\n    return value * 2\n;\n\
        \
         $fast_math\n\
         looper |start F32, end F32, step F32, seed I32| -> I32:\n\
         \x20   bumped = failer(seed)\n\
         \x20   loop start to end by step |i, ordinal|:\n\
         \x20   ;\n\
         \x20   return bumped\n\
         ;\n";
    let LaneFixture {
        mut hir,
        mut type_environment,
        report,
        registry,
        path_fork,
        string_table,
    } = LaneFixture::new(source);
    let looper = function_id_by_name(&hir, &path_fork, &string_table, "looper");
    assert!(
        report.analysis.public_call_summaries[&looper].escapes_builtin_failure,
        "the escaping call must make the loop function a lane function"
    );

    install_private_failure_lanes(&mut hir, &report, &mut type_environment, &registry)
        .expect("lane installation must succeed");

    let guard = hir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            HirStatementKind::LoopStepFailure {
                cause, semantics, ..
            } if *cause == LoopStepFailureCause::ZeroStep => {
                Some((*semantics, NumericFailureMode::range_guard(*semantics)))
            }
            _ => None,
        })
        .expect("the dynamic step must retain its zero-step guard");
    assert_eq!(guard.0, NumericSemantics::Fast);
    assert_eq!(
        guard.1,
        NumericFailureMode::Fatal,
        "fast loop guards must stay fatal inside a lane function"
    );
    assert_eq!(
        numeric_op_modes(&hir, NumericOperator::Add),
        vec![NumericFailureMode::Fatal],
        "the generated fast ordinal overflow must stay fatal inside a lane function"
    );
    assert!(
        !hir.blocks.iter().any(|block| matches!(
            &block.terminator,
            HirTerminator::Invoke {
                operation: HirFallibleOperation::LoopStepFailure { .. },
                ..
            }
        )),
        "no recoverable edge may carry a fast loop guard"
    );
    assert!(
        hir.blocks.iter().any(|block| matches!(
            &block.terminator,
            HirTerminator::Invoke {
                operation: HirFallibleOperation::Call { .. },
                ..
            }
        )),
        "the unrelated escaping call still uses the inferred lane"
    );
}
