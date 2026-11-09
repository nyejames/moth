//! Borrow-validation tests for declared terminating `-> !` calls.
//!
//! WHAT: pins ordinary argument access, conflict detection, the absence of post-call state and
//!       the explicit Never contract's difference from an ordinary zero-success return.
//! WHY: a Never call must apply its argument effects before divergence without inventing a
//!       result, a successor or a normal-return shape.

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::analysis::borrow_checker::problem::{
    EventKind, TerminatorEventKind, from_hir,
};
use crate::compiler_frontend::analysis::borrow_checker::types::ValueAccessClassification;
use crate::compiler_frontend::compiler_messages::InvalidMutableAccessReason;
use crate::compiler_frontend::external_packages::CallTarget;
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionReturnContract};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::utils::terminator_targets;
use crate::compiler_frontend::public_call_summary::PublicCallMutationEffect;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::borrow_fixture_support::{
    assert_invalid_mutable_access_reason, run_borrow_checker,
};
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::lower_hir;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;

struct CheckedSource {
    hir: HirModule,
    path_fork: PathInternerFork,
    string_table: StringTable,
    report: BorrowCheckReport,
}

fn check_source(source: &str) -> CheckedSource {
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let external_package_registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("valid Never source should pass borrow validation");
    CheckedSource {
        hir,
        path_fork,
        string_table,
        report,
    }
}

fn function_id(source: &CheckedSource, name: &str) -> FunctionId {
    source
        .hir
        .functions
        .iter()
        .find(|function| {
            source.hir.side_table.resolve_function_name(
                function.id,
                &source.path_fork,
                &source.string_table,
            ) == Some(name)
        })
        .unwrap_or_else(|| panic!("function '{name}' should lower to HIR"))
        .id
}

fn function(source: &CheckedSource, id: FunctionId) -> &HirFunction {
    source
        .hir
        .functions
        .iter()
        .find(|function| function.id == id)
        .expect("function id should resolve")
}

fn single_never_call_block(source: &CheckedSource) -> (BlockId, &HirTerminator) {
    let block = source
        .hir
        .blocks
        .iter()
        .find(|block| matches!(block.terminator, HirTerminator::NeverCall { .. }))
        .expect("fixture should contain a NeverCall terminator");
    (block.id, &block.terminator)
}

#[test]
fn never_call_records_shared_and_mutable_argument_effects_before_divergence() {
    let source = check_source(
        r#"fatal |first String, second ~String, third String| -> !:
    assert(false, first)
;

run |shared String, mutable ~String| -> !:
    fatal(shared, ~mutable, shared)
;"#,
    );
    let run = function_id(&source, "run");
    let fatal = function_id(&source, "fatal");
    let (block_id, terminator) = single_never_call_block(&source);
    let HirTerminator::NeverCall { target, args } = terminator else {
        panic!("run must end in a terminating call")
    };
    assert_eq!(*target, CallTarget::Local(fatal));
    assert!(
        terminator_targets(terminator).is_empty(),
        "a terminating call has no successor"
    );

    let run_function = function(&source, run);
    assert_eq!(run_function.entry, block_id);
    let shared_local = run_function.params[0];
    let mutable_local = run_function.params[1];
    let fact = source
        .report
        .analysis
        .terminator_fact(block_id)
        .expect("the terminating block must publish its argument effects");
    assert_eq!(fact.shared_roots, vec![shared_local]);
    assert_eq!(fact.mutable_roots, vec![mutable_local]);

    let arguments = source.hir.expressions.values(*args);
    assert_eq!(arguments.len(), 3);
    let shared_fact = source
        .report
        .analysis
        .value_fact(arguments[0])
        .expect("the shared argument must retain its value fact");
    assert_eq!(
        shared_fact.classification,
        ValueAccessClassification::SharedRead
    );
    assert_eq!(shared_fact.roots, vec![shared_local]);
    let repeated_shared_fact = source
        .report
        .analysis
        .value_fact(arguments[2])
        .expect("the repeated shared argument must retain its value fact");
    assert_eq!(
        repeated_shared_fact.classification,
        ValueAccessClassification::SharedRead
    );
    assert_eq!(repeated_shared_fact.roots, vec![shared_local]);
    let mutable_fact = source
        .report
        .analysis
        .value_fact(arguments[1])
        .expect("the mutable argument must retain its value fact");
    assert_eq!(
        mutable_fact.classification,
        ValueAccessClassification::MutableArgument
    );
    assert_eq!(mutable_fact.roots, vec![mutable_local]);

    let summary = source
        .report
        .analysis
        .function_summaries
        .get(&run)
        .expect("run should retain a function borrow summary");
    assert_eq!(summary.mutable_call_sites, 1);
}

#[test]
fn never_call_mutable_argument_rejects_a_live_shared_borrow() {
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(
        r#"fatal |value ~String, shared String| -> !:
    assert(false, shared)
;

run || -> !:
    value ~= "a"
    shared = value
    fatal(~value, shared)
;"#,
    );
    let external_package_registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("a live shared alias must reject a mutable terminating-call argument");
    assert_invalid_mutable_access_reason(
        error,
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
}

#[test]
fn never_call_has_no_post_call_state_or_successor() {
    let source = check_source(
        r#"fatal |value ~String| -> !:
    assert(false, "fatal")
;

run || -> !:
    value ~= "a"
    shared = value
    fatal(~value)
    result = shared
;"#,
    );
    let run = function_id(&source, "run");
    let (block_id, terminator) = single_never_call_block(&source);
    assert!(
        terminator_targets(terminator).is_empty(),
        "the block after a NeverCall must not exist"
    );
    assert_eq!(
        function(&source, run).entry,
        block_id,
        "the terminating call is the only reachable block"
    );

    let block = source
        .hir
        .blocks
        .iter()
        .find(|block| block.id == block_id)
        .expect("the NeverCall block should resolve");
    assert!(
        block
            .statements
            .iter()
            .all(|statement| matches!(statement.kind, HirStatementKind::Write { .. })),
        "only pre-call statements should lower before the terminating call"
    );
    assert!(
        !block.locals.iter().any(|local| {
            source
                .hir
                .side_table
                .resolve_local_name(local.id, &source.path_fork, &source.string_table)
                .is_some_and(|name| name == "result")
        }),
        "statements after a NeverCall must not add post-call locals"
    );
    assert!(
        source
            .report
            .analysis
            .drop_sites_for_block(block_id)
            .is_none(),
        "a terminal call must not expose a post-call drop site"
    );

    let problem = from_hir(
        &source.hir,
        function(&source, run),
        Some(&source.report.analysis.public_call_summaries),
        None,
    )
    .expect("run should extract a normalized borrow problem");
    assert!(
        problem.control_flow().edges.is_empty(),
        "the terminating block must not create normalized edges"
    );
    assert_eq!(
        problem.control_flow().exits.as_ref(),
        [problem.control_flow().entry]
    );
    let terminator = problem
        .events()
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::Terminator { kind } => Some(kind),
            _ => None,
        })
        .expect("the normalized problem must retain its terminator");
    assert_eq!(*terminator, TerminatorEventKind::NeverCall);
}

#[test]
fn never_call_diverges_before_a_branch_joins_a_live_shared_alias() {
    let accepted = check_source(
        r#"fatal |value ~String| -> !:
    assert(false, "fatal")
;

run |condition Bool| -> String:
    value ~= "a"
    shared = value
    if condition:
        fatal(~value)
    ;
    return shared
;"#,
    );
    let run = function_id(&accepted, "run");
    let entry = function(&accepted, run).entry;
    let (never_block, terminator) = single_never_call_block(&accepted);
    assert_ne!(
        never_block, entry,
        "the terminating call must sit in the taken branch rather than the entry block"
    );
    assert!(
        terminator_targets(terminator).is_empty(),
        "the taken branch ends without a successor"
    );

    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(
        r#"mutate |value ~String|:
    ~value = "x"
;

run |condition Bool| -> String:
    value ~= "a"
    shared = value
    if condition:
        mutate(~value)
    ;
    return shared
;"#,
    );
    let external_package_registry = default_external_package_registry(&mut string_table);
    let hir = lower_hir(
        ast,
        &mut string_table,
        &mut path_fork,
        &external_package_registry,
    );
    let error = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect_err("an ordinary mutable call must conflict with the live shared alias");
    assert_invalid_mutable_access_reason(
        error,
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
}

#[test]
fn never_contract_distinguishes_terminating_function_from_zero_success_return() {
    let source = check_source(
        r#"empty ||:
;

fatal |message String| -> !:
    assert(false, message)
;

run || -> !:
    empty()
    fatal("boom")
;"#,
    );
    let empty = function_id(&source, "empty");
    let fatal = function_id(&source, "fatal");

    let empty_contract = &function(&source, empty).return_contract;
    let HirFunctionReturnContract::Returns(empty_shape) = empty_contract else {
        panic!("an ordinary zero-success function must retain its Returns contract")
    };
    assert!(
        empty_shape.success().is_empty(),
        "ordinary zero-success completion is not Never"
    );
    assert!(empty_shape.error().is_none());
    assert_eq!(
        function(&source, fatal).return_contract,
        HirFunctionReturnContract::Never
    );

    let empty_summary = source
        .report
        .analysis
        .public_call_summaries
        .get(&empty)
        .expect("the zero-success function should retain a call summary");
    assert!(empty_summary.parameters.is_empty());
    let fatal_summary = source
        .report
        .analysis
        .public_call_summaries
        .get(&fatal)
        .expect("the Never function should retain a call summary");
    assert_eq!(
        fatal_summary.parameters.len(),
        1,
        "a Never callable keeps its parameter effects"
    );

    let has_zero_success_call = source
        .hir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .any(|statement| {
            matches!(
                &statement.kind,
                HirStatementKind::Call {
                    target: CallTarget::Local(callee),
                    results,
                    ..
                } if *callee == empty && results.is_empty()
            )
        });
    assert!(
        has_zero_success_call,
        "an ordinary zero-success call stays a statement with no results"
    );
    let (_, terminator) = single_never_call_block(&source);
    let HirTerminator::NeverCall { args, .. } = terminator else {
        panic!("the Never call must lower to a terminating terminator")
    };
    assert_eq!(
        source.hir.expressions.values(*args).len(),
        1,
        "the terminating call retains its argument without a result slot"
    );
}

#[test]
fn never_call_publishes_parameter_mutation_through_the_terminating_argument() {
    let source = check_source(
        r#"mutate |value ~String| -> !:
    ~value = "x"
    assert(false, "done")
;

wrapper |value ~String| -> !:
    mutate(~value)
;"#,
    );
    let wrapper = function_id(&source, "wrapper");
    let mutate = function_id(&source, "mutate");
    let mutate_summary = source
        .report
        .analysis
        .public_call_summaries
        .get(&mutate)
        .expect("the Never callee should retain a call summary");
    assert_eq!(
        mutate_summary.parameters[0].mutation,
        PublicCallMutationEffect::Writes
    );
    let wrapper_summary = source
        .report
        .analysis
        .public_call_summaries
        .get(&wrapper)
        .expect("the Never caller should retain a call summary");
    assert_eq!(
        wrapper_summary.parameters[0].mutation,
        PublicCallMutationEffect::Writes,
        "a terminating call still publishes mutation of its exclusive argument"
    );
}
