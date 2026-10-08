//! Focused tests for mutable binding updates and write-through access.
//!
//! WHAT: checks ordinary and dedicated-result Updates against mixed slot/alias states.
//! WHY: source fixtures cannot construct some joined holder states without conflicting earlier.

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::diagnostics::BorrowDiagnostics;
use crate::compiler_frontend::analysis::borrow_checker::engine::BorrowChecker;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    BorrowState, FunctionLayout, LocalState, RootSet,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::{
    BorrowTransferContext, transfer_block,
};
use crate::compiler_frontend::analysis::borrow_checker::types::LocalMode;
use crate::compiler_frontend::compiler_messages::{
    BorrowDiagnosticKind, InvalidMutableAccessReason,
};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::ids::LocalId;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::hir::validate_hir_module;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::borrow_fixture_support::{
    assert_borrow_error_kind, assert_invalid_mutable_access_reason,
};
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::tests::type_id_fixture_support::lower_ast;

#[test]
fn ordinary_updates_reject_definitely_and_possibly_uninitialized_targets() {
    let source = "probe |flag Bool|:\n\
target ~= \"old\"\n\
target = \"replacement\"\n\
sentinel = 0\n\
;";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let (module, _) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("the update fixture should lower");
    let registry = default_external_package_registry(&mut string_table);
    let function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .and_then(|path| path_fork.component(path))
                .is_some_and(|name| string_table.resolve(name) == "probe")
        })
        .expect("the fixture function should lower to HIR");
    let checker = BorrowChecker::new(&module, &registry, &path_fork, &string_table);
    let reachable = checker
        .collect_reachable_blocks(function)
        .expect("fixture CFG should be reachable");
    let layout = checker
        .build_function_layout(function, &reachable)
        .expect("fixture local layout should build");
    let target = local_named(&module, &path_fork, &string_table, "target");
    let (block, update) = module
        .blocks
        .iter()
        .find_map(|block| {
            block
                .statements
                .iter()
                .find(|statement| is_local_update(&module, statement, target))
                .map(|statement| (block, statement))
        })
        .expect("the fixture should contain an ordinary update");
    let target_index = layout
        .index_of(target)
        .expect("the update target should be in the function layout");

    for mode in [LocalMode::UNINIT, LocalMode::UNINIT.union(LocalMode::SLOT)] {
        let local_count = layout.local_count();
        let mut state = BorrowState::new_uninitialized(local_count);
        state.update_local_state(
            target_index,
            LocalState {
                mode,
                value_roots: RootSet::empty(local_count),
                direct_alias_roots: RootSet::empty(local_count),
            },
        );
        let error = transfer_isolated_statement(&checker, &layout, block, update, &mut state)
            .expect_err("an update must not initialize any possibly uninitialized path");
        assert_borrow_error_kind(&error, BorrowDiagnosticKind::UseOfUninitializedLocal);
    }
}

#[test]
fn dedicated_result_updates_reject_definitely_and_possibly_uninitialized_targets() {
    let source = "identity |input String| -> String:\n\
return input\n\
;\n\
probe ||:\n\
argument ~= \"argument\"\n\
target ~= \"old\"\n\
unused = identity(argument)\n\
sentinel = 0\n\
;";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let (mut module, _) = lower_ast(ast, &mut string_table, &mut path_fork)
        .expect("the dedicated update fixture should lower");
    let registry = default_external_package_registry(&mut string_table);
    let target = local_named(&module, &path_fork, &string_table, "target");
    let argument = local_named(&module, &path_fork, &string_table, "argument");
    let (call_block_id, call_statement_id, call_result) = module
        .blocks
        .iter()
        .find_map(|block| {
            block
                .statements
                .iter()
                .find_map(|statement| match &statement.kind {
                    HirStatementKind::Call {
                        args,
                        result: Some(HirLocalDestination::Define(result)),
                        ..
                    } if args.len() == 1 => Some((block.id, statement.id, *result)),
                    _ => None,
                })
        })
        .expect("the fixture should contain an identity call");
    let call_statement = module
        .blocks
        .iter_mut()
        .find(|block| block.id == call_block_id)
        .and_then(|block| {
            block
                .statements
                .iter_mut()
                .find(|statement| statement.id == call_statement_id)
        })
        .expect("the identity call should remain present");
    let HirStatementKind::Call { result, .. } = &mut call_statement.kind else {
        unreachable!("the selected statement is a call")
    };
    *result = Some(HirLocalDestination::Update(target));

    let function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .and_then(|path| path_fork.component(path))
                .is_some_and(|name| string_table.resolve(name) == "probe")
        })
        .expect("the fixture function should lower to HIR");
    let mut checker = BorrowChecker::new(&module, &registry, &path_fork, &string_table);
    checker
        .build_public_call_summaries()
        .expect("the identity call summary should be available");
    let reachable = checker
        .collect_reachable_blocks(function)
        .expect("fixture CFG should be reachable");
    let layout = checker
        .build_function_layout(function, &reachable)
        .expect("fixture local layout should build");
    let target_index = layout
        .index_of(target)
        .expect("the update target should be in the function layout");
    let argument_index = layout
        .index_of(argument)
        .expect("the call argument should be in the function layout");
    let (call_block, call_statement) = module
        .blocks
        .iter()
        .find_map(|block| {
            block
                .statements
                .iter()
                .find(|statement| statement.id == call_statement_id)
                .map(|statement| (block, statement))
        })
        .expect("the updated call should retain its block");
    assert_ne!(call_result, target);

    for mode in [LocalMode::UNINIT, LocalMode::UNINIT.union(LocalMode::SLOT)] {
        let local_count = layout.local_count();
        let mut state = BorrowState::new_uninitialized(local_count);
        state.update_local_state(argument_index, LocalState::slot(local_count));
        state.update_local_state(
            target_index,
            LocalState {
                mode,
                value_roots: RootSet::empty(local_count),
                direct_alias_roots: RootSet::empty(local_count),
            },
        );
        let error =
            transfer_isolated_statement(&checker, &layout, call_block, call_statement, &mut state)
                .expect_err(
                    "a dedicated result update must not initialize a possibly uninitialized path",
                );
        assert_borrow_error_kind(&error, BorrowDiagnosticKind::UseOfUninitializedLocal);
    }
}

#[test]
fn dedicated_result_update_rejects_immutable_slot_and_alias_bindings() {
    let source = "identity |input String| -> String:\n\
return input\n\
;\n\
probe ||:\n\
mutable_root ~= \"root\"\n\
argument ~= \"argument\"\n\
target = \"old\"\n\
unused = identity(argument)\n\
sentinel = 0\n\
;";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let (mut module, _) = lower_ast(ast, &mut string_table, &mut path_fork)
        .expect("the immutable update fixture should lower");
    let registry = default_external_package_registry(&mut string_table);
    let target = local_named(&module, &path_fork, &string_table, "target");
    let argument = local_named(&module, &path_fork, &string_table, "argument");
    let mutable_root = local_named(&module, &path_fork, &string_table, "mutable_root");
    let (call_block_id, call_statement_id) = module
        .blocks
        .iter()
        .find_map(|block| {
            block.statements.iter().find_map(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Call {
                        args,
                        result: Some(HirLocalDestination::Define(_)),
                        ..
                    } if args.len() == 1
                )
                .then_some((block.id, statement.id))
            })
        })
        .expect("the fixture should contain an identity call");
    let call_statement = module
        .blocks
        .iter_mut()
        .find(|block| block.id == call_block_id)
        .and_then(|block| {
            block
                .statements
                .iter_mut()
                .find(|statement| statement.id == call_statement_id)
        })
        .expect("the identity call should remain present");
    let HirStatementKind::Call { result, .. } = &mut call_statement.kind else {
        unreachable!("the selected statement is a call")
    };
    *result = Some(HirLocalDestination::Update(target));

    let function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .and_then(|path| path_fork.component(path))
                .is_some_and(|name| string_table.resolve(name) == "probe")
        })
        .expect("the fixture function should lower to HIR");
    let mut checker = BorrowChecker::new(&module, &registry, &path_fork, &string_table);
    checker
        .build_public_call_summaries()
        .expect("the identity call summary should be available");
    let reachable = checker
        .collect_reachable_blocks(function)
        .expect("fixture CFG should be reachable");
    let layout = checker
        .build_function_layout(function, &reachable)
        .expect("fixture local layout should build");
    let target_index = layout
        .index_of(target)
        .expect("the immutable target should be in the function layout");
    let argument_index = layout
        .index_of(argument)
        .expect("the call argument should be in the function layout");
    let mutable_root_index = layout
        .index_of(mutable_root)
        .expect("the mutable referent should be in the function layout");
    let (call_block, call_statement) = module
        .blocks
        .iter()
        .find_map(|block| {
            block
                .statements
                .iter()
                .find(|statement| statement.id == call_statement_id)
                .map(|statement| (block, statement))
        })
        .expect("the updated call should retain its block");
    let local_count = layout.local_count();
    for alias_only in [false, true] {
        let mut state = BorrowState::new_uninitialized(local_count);
        state.update_local_state(argument_index, LocalState::slot(local_count));
        state.update_local_state(mutable_root_index, LocalState::slot(local_count));
        let target_state = if alias_only {
            let mut roots = RootSet::empty(local_count);
            roots.insert(mutable_root_index);
            LocalState::alias_with_direct(roots.clone(), roots)
        } else {
            LocalState::slot(local_count)
        };
        state.update_local_state(target_index, target_state);

        let error =
            transfer_isolated_statement(&checker, &layout, call_block, call_statement, &mut state)
                .expect_err("a dedicated update of an immutable binding must fail");
        assert_invalid_mutable_access_reason(&error, InvalidMutableAccessReason::ImmutablePlace);
    }
}

#[test]
fn reentered_definition_reinitializes_a_previously_live_local() {
    let source = "probe |source String|:\n\
target ~= \"old\"\n\
sentinel = 0\n\
;";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let (module, _) = lower_ast(ast, &mut string_table, &mut path_fork)
        .expect("the definition fixture should lower");
    let registry = default_external_package_registry(&mut string_table);
    let function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .and_then(|path| path_fork.component(path))
                .is_some_and(|name| string_table.resolve(name) == "probe")
        })
        .expect("the fixture function should lower to HIR");
    let checker = BorrowChecker::new(&module, &registry, &path_fork, &string_table);
    let reachable = checker
        .collect_reachable_blocks(function)
        .expect("fixture CFG should be reachable");
    let layout = checker
        .build_function_layout(function, &reachable)
        .expect("fixture local layout should build");
    let target = local_named(&module, &path_fork, &string_table, "target");
    let source = local_named(&module, &path_fork, &string_table, "source");
    let (block, definition) = module
        .blocks
        .iter()
        .find_map(|block| {
            block
                .statements
                .iter()
                .find(|statement| {
                    matches!(
                        &statement.kind,
                        HirStatementKind::Write {
                            target: HirWriteTarget::DefineLocal(local),
                            ..
                        } if *local == target
                    )
                })
                .map(|statement| (block, statement))
        })
        .expect("the fixture should contain a target definition");
    let local_count = layout.local_count();
    let source_index = layout
        .index_of(source)
        .expect("the source parameter should be in the function layout");
    let target_index = layout
        .index_of(target)
        .expect("the target should be in the function layout");
    let mut state = BorrowState::new_uninitialized(local_count);
    state.update_local_state(source_index, LocalState::slot(local_count));
    let mut source_roots = RootSet::empty(local_count);
    source_roots.insert(source_index);
    state.update_local_state(
        target_index,
        LocalState::alias_with_direct(source_roots.clone(), source_roots),
    );

    transfer_isolated_statement(&checker, &layout, block, definition, &mut state)
        .expect("a reentered definition should establish the new binding");
    let target_state = state.local_state(target_index);
    assert!(target_state.mode.contains(LocalMode::SLOT));
    assert!(!target_state.mode.contains(LocalMode::ALIAS));
    assert!(target_state.value_roots.is_empty());
}

#[test]
fn ordinary_update_rejects_immutable_slot_and_alias_bindings() {
    let source = "probe ||:\n\
mutable_root ~= \"root\"\n\
mutable_target ~= \"old\"\n\
mutable_target = \"replacement\"\n\
immutable_target = \"old\"\n\
sentinel = 0\n\
;";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let (mut module, _) = lower_ast(ast, &mut string_table, &mut path_fork)
        .expect("the immutable update fixture should lower");
    let registry = default_external_package_registry(&mut string_table);
    let mutable = local_named(&module, &path_fork, &string_table, "mutable_target");
    let immutable = local_named(&module, &path_fork, &string_table, "immutable_target");
    let mutable_root = local_named(&module, &path_fork, &string_table, "mutable_root");
    let (block_id, update_id) = module
        .blocks
        .iter()
        .find_map(|block| {
            block
                .statements
                .iter()
                .find(|statement| is_local_update(&module, statement, mutable))
                .map(|statement| (block.id, statement.id))
        })
        .expect("the fixture should contain an ordinary update");
    let update = module
        .blocks
        .iter_mut()
        .find(|block| block.id == block_id)
        .and_then(|block| {
            block
                .statements
                .iter_mut()
                .find(|statement| statement.id == update_id)
        })
        .expect("the update should remain present");
    let HirStatementKind::Write {
        target: HirWriteTarget::AssignPlace(place),
        ..
    } = &mut update.kind
    else {
        unreachable!("the selected statement is an ordinary update")
    };
    place.root = immutable;

    let function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .and_then(|path| path_fork.component(path))
                .is_some_and(|name| string_table.resolve(name) == "probe")
        })
        .expect("the fixture function should lower to HIR");
    let checker = BorrowChecker::new(&module, &registry, &path_fork, &string_table);
    let reachable = checker
        .collect_reachable_blocks(function)
        .expect("fixture CFG should be reachable");
    let layout = checker
        .build_function_layout(function, &reachable)
        .expect("fixture local layout should build");
    let immutable_index = layout
        .index_of(immutable)
        .expect("the immutable target should be in the function layout");
    let mutable_root_index = layout
        .index_of(mutable_root)
        .expect("the mutable referent should be in the function layout");
    let (block, update) = module
        .blocks
        .iter()
        .find_map(|block| {
            block
                .statements
                .iter()
                .find(|statement| statement.id == update_id)
                .map(|statement| (block, statement))
        })
        .expect("the updated statement should retain its block");
    let local_count = layout.local_count();
    for alias_only in [false, true] {
        let mut state = BorrowState::new_uninitialized(local_count);
        state.update_local_state(mutable_root_index, LocalState::slot(local_count));
        let immutable_state = if alias_only {
            let mut roots = RootSet::empty(local_count);
            roots.insert(mutable_root_index);
            LocalState::alias_with_direct(roots.clone(), roots)
        } else {
            LocalState::slot(local_count)
        };
        state.update_local_state(immutable_index, immutable_state);

        let error = transfer_isolated_statement(&checker, &layout, block, update, &mut state)
            .expect_err("ordinary assignment to an immutable binding must fail");
        assert_invalid_mutable_access_reason(&error, InvalidMutableAccessReason::ImmutablePlace);
    }
}

#[test]
fn ordinary_mixed_update_checks_a_seeded_competing_holder() {
    let source = "probe |flag Bool|:\n\
observer = \"observer\"\n\
source ~= \"source\"\n\
target ~= \"old\"\n\
if flag:\n\
    target = source\n\
else\n\
    target = \"branch\"\n\
;\n\
target = \"replacement\"\n\
sentinel = 0\n\
observed = copy observer\n\
;";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let (module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("fixture source should lower");
    validate_hir_module(&module, &type_environment)
        .expect("the complete fixture should be structurally valid HIR");
    let registry = default_external_package_registry(&mut string_table);
    let function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .and_then(|path| path_fork.component(path))
                .is_some_and(|name| string_table.resolve(name) == "probe")
        })
        .expect("the fixture function should lower");
    let checker = BorrowChecker::new(&module, &registry, &path_fork, &string_table);
    let reachable = checker
        .collect_reachable_blocks(function)
        .expect("fixture CFG should be reachable");
    let layout = checker
        .build_function_layout(function, &reachable)
        .expect("fixture local layout should build");
    let target = local_named(&module, &path_fork, &string_table, "target");
    let source = local_named(&module, &path_fork, &string_table, "source");
    let observer = local_named(&module, &path_fork, &string_table, "observer");
    let sentinel = local_named(&module, &path_fork, &string_table, "sentinel");
    let (block, update) = module
        .blocks
        .iter()
        .find_map(|block| {
            let sentinel_definition = block.statements.iter().position(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Write {
                        target: HirWriteTarget::DefineLocal(local),
                        ..
                    } if *local == sentinel
                )
            })?;
            let update = block.statements[..sentinel_definition]
                .iter()
                .rfind(|statement| is_local_update(&module, statement, target))?;
            Some((block, update))
        })
        .expect("the post-join update should precede its sentinel");
    let expected_span = update.span.expect("the source update should have a span");
    let mut unheld_state =
        seed_mixed_target_with_observer(&layout, target, source, observer, &[], false);
    transfer_isolated_statement(&checker, &layout, block, update, &mut unheld_state)
        .expect("the mixed update should pass without a competing shared holder");

    let mut state = seed_mixed_target_with_observer(&layout, target, source, observer, &[], true);
    assert_seeded_shared_holder(&layout, &state, source, observer);

    let error = transfer_isolated_statement(&checker, &layout, block, update, &mut state)
        .expect_err("a mixed update may still write through to its source root");
    assert_invalid_mutable_access_reason(
        &error,
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
    assert_eq!(
        error
            .diagnostic()
            .and_then(|diagnostic| diagnostic.primary_span),
        Some(expected_span),
        "the failure must be reported at the isolated post-join update"
    );
}

#[test]
fn dedicated_result_mixed_update_checks_a_seeded_competing_holder() {
    let source = "identity |input String| -> String:\n\
return input\n\
;\n\
probe |flag Bool|:\n\
argument ~= \"argument\"\n\
source ~= \"source\"\n\
observer = \"observer\"\n\
target ~= \"old\"\n\
if flag:\n\
    target = source\n\
else\n\
    target = \"branch\"\n\
;\n\
unused = identity(argument)\n\
sentinel = 0\n\
observed = copy observer\n\
;";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("fixture source should lower");
    let registry = default_external_package_registry(&mut string_table);
    let target = local_named(&module, &path_fork, &string_table, "target");
    let (call_result, call_region) = module
        .blocks
        .iter()
        .find_map(|block| {
            block
                .statements
                .iter()
                .find_map(|statement| match &statement.kind {
                    HirStatementKind::Call {
                        args,
                        result: Some(HirLocalDestination::Define(local)),
                        ..
                    } if args.len() == 1 => Some((*local, block.region)),
                    _ => None,
                })
        })
        .expect("the identity call should define a result local");
    let target_type = module
        .blocks
        .iter()
        .flat_map(|block| &block.locals)
        .find(|local| local.id == target)
        .map(|local| local.ty)
        .expect("the update destination should have a declared type");
    let replacement_value = crate::compiler_frontend::tests::hir_fixture_support::expression(
        HirExpressionKind::Load(crate::compiler_frontend::hir::places::HirPlace::local(
            target,
        )),
        target_type,
        call_region,
        ValueKind::Place,
        &mut module.expressions,
    );

    let mut updated_call_id = None;
    let mut retargeted_consumer = false;
    for block in &mut module.blocks {
        for statement in &mut block.statements {
            match &mut statement.kind {
                HirStatementKind::Call {
                    args,
                    result: Some(destination),
                    ..
                } if args.len() == 1
                    && matches!(
                        &*destination,
                        HirLocalDestination::Define(local) if *local == call_result
                    ) =>
                {
                    *destination = HirLocalDestination::Update(target);
                    updated_call_id = Some(statement.id);
                }
                HirStatementKind::Write { value, .. }
                    if matches!(
                        &module.expressions.expression(*value).kind,
                        HirExpressionKind::Load(place) if place.root == call_result
                    ) =>
                {
                    *value = replacement_value;
                    retargeted_consumer = true;
                }
                _ => {}
            }
        }
    }
    assert!(updated_call_id.is_some(), "the result should update target");
    assert!(
        retargeted_consumer,
        "the consumer should read the updated target"
    );
    validate_hir_module(&module, &type_environment)
        .expect("the retargeted call result should remain structurally valid HIR");

    let function = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .and_then(|path| path_fork.component(path))
                .is_some_and(|name| string_table.resolve(name) == "probe")
        })
        .expect("the fixture function should lower");
    let mut checker = BorrowChecker::new(&module, &registry, &path_fork, &string_table);
    checker
        .build_public_call_summaries()
        .expect("the identity call summary should be available");
    let reachable = checker
        .collect_reachable_blocks(function)
        .expect("fixture CFG should be reachable");
    let layout = checker
        .build_function_layout(function, &reachable)
        .expect("fixture local layout should build");
    let source = local_named(&module, &path_fork, &string_table, "source");
    let observer = local_named(&module, &path_fork, &string_table, "observer");
    let argument = local_named(&module, &path_fork, &string_table, "argument");
    let call_statement = module
        .blocks
        .iter()
        .find_map(|block| {
            block
                .statements
                .iter()
                .find(|statement| Some(statement.id) == updated_call_id)
                .map(|statement| (block, statement))
        })
        .expect("the updated call should retain its block");
    let expected_span = call_statement
        .1
        .span
        .expect("the dedicated result update should have a source span");
    let mut unheld_state =
        seed_mixed_target_with_observer(&layout, target, source, observer, &[argument], false);
    transfer_isolated_statement(
        &checker,
        &layout,
        call_statement.0,
        call_statement.1,
        &mut unheld_state,
    )
    .expect("the dedicated mixed update should pass without a competing shared holder");

    let mut state =
        seed_mixed_target_with_observer(&layout, target, source, observer, &[argument], true);
    assert_seeded_shared_holder(&layout, &state, source, observer);

    let error = transfer_isolated_statement(
        &checker,
        &layout,
        call_statement.0,
        call_statement.1,
        &mut state,
    )
    .expect_err("the dedicated update may write through to the competing source alias");
    assert_invalid_mutable_access_reason(
        &error,
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
    assert_eq!(
        error
            .diagnostic()
            .and_then(|diagnostic| diagnostic.primary_span),
        Some(expected_span),
        "the failure must be reported at the isolated result update"
    );
}

fn local_named(
    module: &HirModule,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> LocalId {
    module
        .blocks
        .iter()
        .flat_map(|block| &block.locals)
        .find(|local| {
            module
                .side_table
                .resolve_local_name(local.id, path_fork, string_table)
                == Some(name)
        })
        .map(|local| local.id)
        .unwrap_or_else(|| panic!("fixture local {name} should lower"))
}

fn is_local_update(module: &HirModule, statement: &HirStatement, target: LocalId) -> bool {
    matches!(
        &statement.kind,
        HirStatementKind::Write {
            target: HirWriteTarget::AssignPlace(place),
            ..
        } if place.root == target && module.expressions.projections(place.projections).is_empty()
    )
}

fn seed_mixed_target_with_observer(
    layout: &FunctionLayout,
    target: LocalId,
    source: LocalId,
    observer: LocalId,
    additional_slots: &[LocalId],
    observer_holds_source: bool,
) -> BorrowState {
    let local_count = layout.local_count();
    let source_index = layout
        .index_of(source)
        .expect("source should be in the layout");
    let target_index = layout
        .index_of(target)
        .expect("target should be in the layout");
    let observer_index = layout
        .index_of(observer)
        .expect("observer should be in the layout");
    let mut state = BorrowState::new_uninitialized(local_count);
    state.update_local_state(source_index, LocalState::slot(local_count));
    for local in additional_slots {
        let index = layout
            .index_of(*local)
            .expect("slot local should be in the layout");
        state.update_local_state(index, LocalState::slot(local_count));
    }

    let mut source_roots = RootSet::empty(local_count);
    source_roots.insert(source_index);
    state.update_local_state(
        target_index,
        LocalState {
            mode: LocalMode::SLOT.union(LocalMode::ALIAS),
            value_roots: source_roots.clone(),
            direct_alias_roots: source_roots.clone(),
        },
    );
    let observer_state = if observer_holds_source {
        LocalState::alias_with_direct(source_roots.clone(), source_roots)
    } else {
        LocalState::slot(local_count)
    };
    state.update_local_state(observer_index, observer_state);
    state
}

fn assert_seeded_shared_holder(
    layout: &FunctionLayout,
    state: &BorrowState,
    source: LocalId,
    observer: LocalId,
) {
    let source_index = layout
        .index_of(source)
        .expect("source should be in the layout");
    let observer_index = layout
        .index_of(observer)
        .expect("observer should be in the layout");
    let observer_state = state.local_state(observer_index);
    assert!(!layout.local_mutable[observer_index]);
    assert!(observer_state.mode.contains(LocalMode::ALIAS));
    assert!(observer_state.value_roots.contains(source_index));
}

fn transfer_isolated_statement(
    checker: &BorrowChecker<'_>,
    layout: &FunctionLayout,
    original_block: &HirBlock,
    statement: &HirStatement,
    state: &mut BorrowState,
) -> Result<(), BorrowCheckError> {
    let transfer_context = BorrowTransferContext {
        external_package_registry: checker.external_package_registry,
        public_call_summaries: &checker.public_call_summaries,
        imported_call_summaries: &checker.module.imported_call_summaries,
        module_private_call_summaries: &checker.module.module_private_call_summaries,
        generated_call_summaries: &checker.module.generated_call_summaries,
        expressions: &checker.module.expressions,
        diagnostics: BorrowDiagnostics::new(
            checker.module,
            checker.path_fork,
            checker.string_table,
        ),
    };
    let isolated_block = HirBlock {
        id: original_block.id,
        region: original_block.region,
        locals: original_block.locals.clone(),
        statements: vec![statement.clone()],
        terminator: HirTerminator::RuntimeFailure {
            message: "isolated transfer test".to_owned(),
            cause: None,
        },
    };
    transfer_block(&transfer_context, layout, &isolated_block, state).map(|_| ())
}
