//! Private failure lane producer-selection tests.
//!
//! WHAT: proves lane installation splits only trapping numeric operations that carry the
//!       shared failure code set, leaving code-less traps (`F64` negation) untouched.
//! WHY: the installer cannot re-derive AST failure facts; it must consult the same code
//!      set or it would route a dead error edge for an operation the frontend calls
//!      infallible (and, on Wasm, report recoverable numeric failure for exact arithmetic).

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::numeric::NumericFailureMode;
use crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes;
use crate::compiler_frontend::hir::statements::{HirLocalDestination, HirStatementKind};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::borrow_fixture_support::run_borrow_checker;
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::lower_hir;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;

fn function_id_by_name(
    module: &crate::compiler_frontend::hir::module::HirModule,
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

fn numeric_op_modes(
    module: &crate::compiler_frontend::hir::module::HirModule,
    operator: NumericOperator,
) -> Vec<NumericFailureMode> {
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

fn install_lanes(
    source: &str,
) -> (
    crate::compiler_frontend::hir::module::HirModule,
    crate::compiler_frontend::datatypes::environment::TypeEnvironment,
    BorrowCheckReport,
    PathInternerFork,
    StringTable,
) {
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let mut type_environment = ast.type_environment.clone();
    let registry = default_external_package_registry(&mut string_table);
    let mut hir = lower_hir(ast, &mut string_table, &mut path_fork, &registry);
    let mut report = run_borrow_checker(&hir, &registry, &path_fork, &string_table)
        .expect("lane fixtures must pass borrow checking");
    infer_builtin_failure_summaries(&hir, &mut report)
        .expect("lane fixture summaries must converge");
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
        ("value = \"plain\"\n[:[value]]\n", false),
        (
            "load || -> String, Error!:\n    return! Error(\"failed\")\n;\n\
             value = load()!\n[:[value]]\n",
            true,
        ),
        (
            "load || -> String, Error!:\n    return! Error(\"failed\")\n;\n\
             value = load() catch then \"recovered\"\n[:[value]]\n",
            false,
        ),
    ] {
        let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
        let mut type_environment = ast.type_environment.clone();
        let registry = default_external_package_registry(&mut string_table);
        let mut hir = lower_hir(ast, &mut string_table, &mut path_fork, &registry);
        let start = hir.start_function.expect("normal roots must have a start");
        let function = hir
            .functions
            .iter()
            .find(|function| function.id == start)
            .expect("start must have a function record");
        let success_slots = function.returns.clone();
        let error_slot = function.error_return;
        assert!(
            error_slot.is_some(),
            "lowering retains the AST builtin Error slot"
        );
        let mut report = run_borrow_checker(&hir, &registry, &path_fork, &string_table)
            .expect("start fixtures must pass borrow checking");
        infer_builtin_failure_summaries(&hir, &mut report)
            .expect("start fixture summaries must converge");
        install_private_failure_lanes(&mut hir, &report, &mut type_environment, &registry)
            .expect("start narrowing must leave valid native HIR");
        let function = hir
            .functions
            .iter()
            .find(|function| function.id == start)
            .expect("installation must retain start");
        assert_eq!(
            function.returns, success_slots,
            "success slots remain unchanged"
        );
        assert_eq!(
            function.error_return,
            if escapes { error_slot } else { None }
        );
    }
}

#[test]
fn inferred_range_guard_has_no_success_slots() {
    let (hir, _, _, _, _) =
        install_lanes("walk |step I32|:\n    loop 0 to 4 by step |value|:\n    ;\n;\n");
    let mut guards = 0;
    for block in &hir.blocks {
        if let HirTerminator::Invoke {
            operation: HirFallibleOperation::RangeStepFailure { .. },
            success_results,
            success,
            ..
        } = &block.terminator
        {
            guards += 1;
            assert!(
                success_results.is_empty(),
                "a terminal range guard produces no success values"
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
