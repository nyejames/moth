//! Private failure lane producer-selection tests.
//!
//! WHAT: proves lane installation splits only trapping numeric operations that carry the
//!       shared failure code set, leaving code-less traps (`Float` negation) untouched.
//! WHY: the installer cannot re-derive AST failure facts; it must consult the same code
//!      set or it would route a dead error edge for an operation the frontend calls
//!      infallible (and, on Wasm, report recoverable numeric failure for exact arithmetic).

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::numeric::NumericFailureMode;
use crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
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
    let mut hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let mut report = run_borrow_checker(
        &hir,
        &default_external_package_registry(&mut string_table),
        &path_fork,
        &string_table,
    )
    .expect("lane fixtures must pass borrow checking");
    infer_builtin_failure_summaries(&hir, &mut report)
        .expect("lane fixture summaries must converge");
    install_private_failure_lanes(&mut hir, &report, &mut type_environment)
        .expect("lane installation must succeed");
    (hir, type_environment, report, path_fork, string_table)
}

#[test]
fn lane_installation_splits_code_carrying_traps_but_not_codeless_float_negation() {
    // `negate` becomes a lane function through its escaping `failer` call. Its `Int`
    // multiplication (code-carrying) must split onto the inferred lane, while its
    // `Float` negation (empty shared code set) must keep its lowering-time trap.
    let (hir, _, report, path_fork, string_table) = install_lanes(
        "failer |value Int| -> Int:\n    return value * 2\n;\n\
         negate |value Float, seed Int| -> Float:\n    bumped = failer(seed)\n    return -value\n;\n",
    );
    let negate = function_id_by_name(&hir, &path_fork, &string_table, "negate");
    assert!(
        report.analysis.public_call_summaries[&negate].escapes_builtin_failure,
        "the escaping call must make the helper a lane function"
    );
    assert_eq!(
        numeric_op_modes(&hir, NumericOperator::Multiply),
        vec![NumericFailureMode::ReturnError],
        "code-carrying arithmetic in a lane function must join the inferred lane"
    );
    assert_eq!(
        numeric_op_modes(&hir, NumericOperator::Negate),
        vec![NumericFailureMode::Trap],
        "codeless Float negation must keep its trap instead of gaining a dead error edge"
    );
    assert!(
        hir.blocks
            .iter()
            .any(|block| matches!(block.terminator, HirTerminator::ReturnError(_))),
        "the lane function must still return its genuine failure"
    );
}
