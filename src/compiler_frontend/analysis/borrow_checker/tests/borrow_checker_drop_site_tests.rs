//! Borrow-checker drop-site regression tests.
//!
//! WHAT: exercises where locals are considered dropped as scopes and statements complete.
//! WHY: incorrect drop placement can silently change borrow lifetimes and ownership outcomes.

use crate::compiler_frontend::analysis::borrow_checker::BorrowDropSiteKind;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::datatypes::{DataType, builtin_type_ids};
use crate::compiler_frontend::hir::expression_store::HirProjection;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    assignment_target, function_node, make_test_variable, node, symbol, test_if_branch_metadata,
    test_source_location,
};
use crate::compiler_frontend::tests::borrow_fixture_support::run_borrow_checker;
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::hir_fixture_support::{entry_and_start, lower_hir};
use crate::compiler_frontend::tests::type_id_fixture_support::{
    build_ast_with_registered_types, runtime_expr, runtime_operand_item,
};

use crate::compiler_frontend::value_mode::ValueMode;

#[test]
fn emits_advisory_return_drop_sites() {
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut path_fork, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let value = symbol("value", &mut path_fork, &mut string_table);
    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(
            NodeKind::VariableDeclaration(make_test_variable(
                value,
                Expression::int(1, test_source_location(1), ValueMode::MutableOwned),
            )),
            test_source_location(1),
        )],
        None,
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start_fn], entry_path),
        &mut string_table,
        &mut path_fork,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("borrow checking should succeed");

    let has_return_site = report
        .analysis
        .advisory_drop_sites
        .values()
        .flatten()
        .any(|site| matches!(site.kind, BorrowDropSiteKind::Return));
    assert!(
        has_return_site,
        "expected at least one advisory return drop site"
    );

    for site in report.analysis.advisory_drop_sites.values().flatten() {
        let mut sorted = site.locals.clone();
        sorted.sort_by_key(|local| local.0);
        assert_eq!(
            site.locals, sorted,
            "drop-site locals should be in deterministic local-id order"
        );
    }
}

#[test]
fn emits_advisory_break_and_region_exit_drop_sites() {
    let mut path_fork = crate::compiler_frontend::symbols::path_interner::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_and_start(&mut path_fork, &mut string_table);
    let external_package_registry = default_external_package_registry(&mut string_table);

    let x = symbol("x", &mut path_fork, &mut string_table);
    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![
            node(
                NodeKind::VariableDeclaration(make_test_variable(
                    x,
                    Expression::int(1, test_source_location(1), ValueMode::MutableOwned),
                )),
                test_source_location(1),
            ),
            node(
                NodeKind::If(
                    runtime_expr(
                        vec![runtime_operand_item(Expression::bool(
                            true,
                            test_source_location(2),
                            ValueMode::ImmutableOwned,
                        ))],
                        builtin_type_ids::BOOL,
                        test_source_location(2),
                        ValueMode::ImmutableOwned,
                    ),
                    vec![node(
                        NodeKind::Assignment {
                            target: assignment_target(
                                x,
                                DataType::Int,
                                builtin_type_ids::INT,
                                test_source_location(3),
                            ),
                            value: Expression::int(
                                2,
                                test_source_location(3),
                                ValueMode::ImmutableOwned,
                            ),
                        },
                        test_source_location(3),
                    )],
                    Some(vec![node(
                        NodeKind::Assignment {
                            target: assignment_target(
                                x,
                                DataType::Int,
                                builtin_type_ids::INT,
                                test_source_location(4),
                            ),
                            value: Expression::int(
                                3,
                                test_source_location(4),
                                ValueMode::ImmutableOwned,
                            ),
                        },
                        test_source_location(4),
                    )]),
                    test_if_branch_metadata(true),
                ),
                test_source_location(2),
            ),
            node(
                NodeKind::WhileLoop(
                    Expression::bool(true, test_source_location(5), ValueMode::ImmutableOwned),
                    vec![node(NodeKind::Break, test_source_location(6))],
                ),
                test_source_location(5),
            ),
        ],
        None,
    );

    let hir = lower_hir(
        build_ast_with_registered_types(vec![start_fn], entry_path),
        &mut string_table,
        &mut path_fork,
    );
    let report = run_borrow_checker(&hir, &external_package_registry, &path_fork, &string_table)
        .expect("borrow checking should succeed");

    let has_break_site = report
        .analysis
        .advisory_drop_sites
        .values()
        .flatten()
        .any(|site| matches!(site.kind, BorrowDropSiteKind::Break));
    assert!(has_break_site, "expected advisory break drop sites");

    let has_region_exit_site = report
        .analysis
        .advisory_drop_sites
        .values()
        .flatten()
        .any(|site| matches!(site.kind, BorrowDropSiteKind::BlockExit));
    assert!(
        has_region_exit_site,
        "expected advisory region-exit (block-exit) drop sites"
    );
}

#[test]
fn catch_outward_handler_failure_drops_abandoned_argument_and_handler_local() {
    use crate::compiler_frontend::hir::statements::HirStatementKind;
    use crate::compiler_frontend::hir::terminators::HirTerminator;
    use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;

    let source = "fresh || -> String:\nreturn \"argument\"\n;\n\
                  combine |text String, count Int| -> String:\nreturn text\n;\n\
                  probe |trigger Int| -> String, Error!:\n\
                  return combine(fresh(), trigger + 2147483647) catch:\n\
                      handler_local = \"handler\"\n\
                      count = trigger + 2147483647\n\
                      then handler_local\n\
                  ;\n;\n";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &registry, &path_fork, &string_table)
        .expect("the outward handler edge must preserve normal cleanup");
    let argument = hir
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match statement.kind {
            HirStatementKind::Call {
                ref args,
                result: Some(HirLocalDestination::Define(local)),
                ..
            } if args.len() == 0 => Some(local),
            _ => None,
        })
        .expect("the earlier argument must materialise a temporary");
    let handler_local = hir
        .blocks
        .iter()
        .flat_map(|block| &block.locals)
        .find(|local| {
            hir.side_table
                .resolve_local_name(local.id, &path_fork, &string_table)
                == Some("handler_local")
        })
        .expect("the handler local must exist")
        .id;
    let handler = hir
        .blocks
        .iter()
        .find(|block| block.locals.iter().any(|local| local.id == handler_local))
        .expect("the handler must have a lexical region");
    let HirTerminator::FallibleBranch { error_block, .. } = handler.terminator else {
        panic!("the failing handler operation must branch");
    };
    assert!(matches!(
        hir.blocks[error_block.0 as usize].terminator,
        HirTerminator::ReturnError(_)
    ));
    let sites = report
        .analysis
        .drop_sites_for_block(error_block)
        .expect("the outward return must carry cleanup guidance");
    let site = sites
        .iter()
        .find(|site| site.kind == BorrowDropSiteKind::Return)
        .expect("the outward error return must have a return drop site");
    assert!(
        site.locals.contains(&argument),
        "the skipped outer call cannot leak its earlier argument"
    );
    assert!(
        site.locals.contains(&handler_local),
        "a failing handler cannot leak its own locals"
    );
}

#[test]
fn failed_field_compound_writeback_keeps_borrow_obligations_on_error_edge() {
    use crate::compiler_frontend::analysis::borrow_checker::LocalMode;
    use crate::compiler_frontend::hir::expressions::HirExpressionKind;
    use crate::compiler_frontend::hir::terminators::HirTerminator;
    use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
    use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;

    // The write-back cast narrows the promoted sum back into `box.level`; its
    // error edge must skip the field store while still cleaning up the
    // abandoned right-hand-side temporaries.
    let source = "Bucket = |\n    level U8,\n|\n\
                  risky |amount U8| -> U8, Error!:\n    wide U32 = amount + 200\n    return cast! wide\n;\n\
                  bump |box ~Bucket, extra U8| -> U8, Error!:\n    box.level += risky(extra) catch then extra\n    return box.level\n;\n";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let hir = lower_hir(ast, &mut string_table, &mut path_fork);
    let registry = default_external_package_registry(&mut string_table);
    let report = run_borrow_checker(&hir, &registry, &path_fork, &string_table)
        .expect("the failed write-back must still borrow-check");
    // Locate the write-back: the narrowing cast whose success continuation
    // stores into the struct field.
    let mut writeback = None;
    for block in &hir.blocks {
        let cast = block
            .statements
            .iter()
            .find_map(|statement| match &statement.kind {
                HirStatementKind::CastOp {
                    source,
                    result: Some(HirLocalDestination::Define(carrier)),
                    ..
                } => Some((statement.id, source, *carrier)),
                _ => None,
            });
        let Some((cast_id, cast_source, carrier)) = cast else {
            continue;
        };
        let HirTerminator::FallibleBranch {
            success_block,
            error_block,
            ..
        } = &block.terminator
        else {
            continue;
        };
        let success = &hir.blocks[success_block.0 as usize];
        let field_store = success
            .statements
            .iter()
            .find_map(|statement| match &statement.kind {
                HirStatementKind::Write {
                    target: HirWriteTarget::AssignPlace(place),
                    ..
                } if hir.expressions.projections(place.projections).len() == 1
                    && matches!(
                        hir.expressions.projections(place.projections).first(),
                        Some(HirProjection::Field(_))
                    ) =>
                {
                    Some(place.root)
                }
                _ => None,
            });
        if let Some(receiver) = field_store {
            writeback = Some((
                block.id,
                cast_id,
                cast_source,
                carrier,
                receiver,
                *success_block,
                *error_block,
            ));
            break;
        }
    }
    let (writeback_block, cast_id, cast_source, carrier, receiver, success_block, error_block) =
        writeback.expect(
            "the compound field write-back must lower to a cast with a field store on success",
        );
    let error = &hir.blocks[error_block.0 as usize];
    assert!(
        matches!(error.terminator, HirTerminator::ReturnError(_)),
        "the failed write-back must propagate the narrowing failure"
    );
    assert!(
        !error.statements.iter().any(|statement| matches!(
            &statement.kind,
            HirStatementKind::Write {
                target: HirWriteTarget::AssignPlace(place),
                ..
            } if hir.expressions.projections(place.projections)
                    .iter()
                    .any(|projection| matches!(projection, HirProjection::Field(_)))
        )),
        "the failure edge must not write the field target"
    );
    // The promoted sum feeding the cast is abandoned on the failure edge.
    let HirExpressionKind::Load(promoted_place) = &hir.expressions.expression(*cast_source).kind
    else {
        panic!("the write-back cast must read the promoted sum from a local");
    };
    assert!(
        hir.expressions
            .projections(promoted_place.projections)
            .is_empty()
    );
    let promoted = promoted_place.root;
    let sites = report
        .analysis
        .drop_sites_for_block(error_block)
        .expect("the failed write-back must carry cleanup guidance");
    let site = sites
        .iter()
        .find(|site| site.kind == BorrowDropSiteKind::Return)
        .expect("the failed write-back must have a return drop site");
    assert!(
        site.locals.contains(&promoted),
        "the abandoned promoted sum cannot leak on the failure edge"
    );
    assert!(
        site.locals.contains(&carrier),
        "the abandoned narrowing carrier cannot leak on the failure edge"
    );
    let terminator_fact = report
        .analysis
        .terminator_fact(writeback_block)
        .expect("the write-back branch must record a borrow fact");
    assert!(
        terminator_fact.mutable_roots.is_empty(),
        "no mutable borrow may stay live into the failure edge"
    );
    let cast_fact = report
        .analysis
        .statement_fact(cast_id)
        .expect("the write-back cast must record a borrow fact");
    assert!(
        cast_fact.mutable_roots.is_empty(),
        "the failing cast must not hold the receiver mutably"
    );
    let snapshots = &report.analysis.block_entry_states;
    let error_entry = snapshots
        .get(&error_block)
        .expect("the failure edge must have an entry snapshot");
    let success_entry = snapshots
        .get(&success_block)
        .expect("the success edge must have an entry snapshot");
    assert_eq!(
        error_entry, success_entry,
        "the failure edge must carry the exact pre-write state"
    );
    let receiver_snapshot = error_entry
        .locals
        .iter()
        .find(|snapshot| snapshot.local == receiver)
        .expect("the receiver must be visible on the failure edge");
    assert!(
        receiver_snapshot.mode.contains(LocalMode::SLOT),
        "the unwritten receiver must keep its initialized slot"
    );
    assert!(!receiver_snapshot.mode.contains(LocalMode::ALIAS));
    assert!(receiver_snapshot.alias_roots.is_empty());
}
