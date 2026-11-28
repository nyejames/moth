//! Focused tests for binding definitions, updates, and write-through access.
//!
//! WHAT: checks definition retirement and ordinary/dedicated-result Updates.
//! WHY: isolated transfers can retain incoming and previous bindings for direct comparison.

use super::borrow_checker_call_summary_tests::assert_borrow_code_at;
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckError;
use crate::compiler_frontend::analysis::borrow_checker::diagnostics::BorrowDiagnostics;
use crate::compiler_frontend::analysis::borrow_checker::engine::BorrowChecker;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    AddressSet, BindingSpec, BorrowState, CellId, Coverage, FieldKey, FunctionLayout, RootSet,
    WrapperId,
};
use crate::compiler_frontend::analysis::borrow_checker::transfer::facts::ValueFactBuffer;
use crate::compiler_frontend::analysis::borrow_checker::transfer::{
    BlockTransferStats, BorrowTransferContext, transfer_block,
};
use crate::compiler_frontend::analysis::borrow_checker::types::{
    LocalMode, ValueAccessClassification,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    InvalidMutableAccessView, SharedMutableConflictView, UseOfUninitializedLocalView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::{DiagnosticBag, InvalidMutableAccessReason};
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::ids::{HirValueId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::statements::{
    HirLocalDestination, HirStatement, HirStatementKind, HirWriteTarget,
};
use crate::compiler_frontend::hir::terminators::{HirTerminator, RuntimeFailureKind};
use crate::compiler_frontend::hir::validate_hir_module;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::borrow_fixture_support::{
    assert_borrow_error_view, assert_invalid_mutable_access_reason, run_borrow_checker_with_types,
};
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::tests::type_id_fixture_support::lower_ast;
use rustc_hash::FxHashMap;

fn assert_invalid_mutable_access_at(
    error: BorrowCheckError,
    expected_reason: InvalidMutableAccessReason,
    expected_span: SourceSpan,
) {
    let draft = error
        .into_draft_or_infrastructure()
        .expect("borrow failure should retain its typed diagnostic");
    let compact = compact_bag_for_test(DiagnosticBag::from(draft), None);
    let diagnostic = compact
        .first()
        .expect("invalid mutable access diagnostic should compact");
    let access = diagnostic
        .family_view::<InvalidMutableAccessView>()
        .expect("invalid mutable access should retain typed facts");
    assert_eq!(access.reason, expected_reason);
    assert!(matches!(
        diagnostic.primary_site(),
        SiteRef::Local(span) if span == expected_span
    ));
}

#[test]
fn ordinary_updates_reject_definitely_and_possibly_uninitialized_targets() {
    let source = "probe |flag Bool|:\n\
target ~= \"old\"\n\
~target = \"replacement\"\n\
sentinel = 0\n\
;";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut string_table);
    let (module, type_environment) = lower_ast(ast, &mut string_table, &mut path_fork, &registry)
        .expect("the update fixture should lower");
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
    let checker = BorrowChecker::new(
        &module,
        &type_environment,
        &registry,
        &path_fork,
        &string_table,
    );
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

    for possibly_initialized in [false, true] {
        let local_count = layout.local_count();
        let mut state = BorrowState::new_uninitialized(local_count);
        if possibly_initialized {
            let mut initialized = state.clone();
            initialized.initialize_parameter(target_index);
            assert!(state.join_in_place(&initialized));
        }
        let error = transfer_isolated_statement(&checker, &layout, block, update, &mut state)
            .expect_err("an update must not initialize any possibly uninitialized path");
        assert_borrow_error_view::<UseOfUninitializedLocalView>(error);
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
    let registry = default_external_package_registry(&mut string_table);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork, &registry)
            .expect("the dedicated update fixture should lower");
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
                    HirStatementKind::Call { args, results, .. } if args.len() == 1 => {
                        match results.as_slice() {
                            [HirLocalDestination::Define(result)] => {
                                Some((block.id, statement.id, *result))
                            }
                            _ => None,
                        }
                    }
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
    let HirStatementKind::Call { results, .. } = &mut call_statement.kind else {
        unreachable!("the selected statement is a call")
    };
    *results = vec![HirLocalDestination::Update(target)];

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
    let mut checker = BorrowChecker::new(
        &module,
        &type_environment,
        &registry,
        &path_fork,
        &string_table,
    );
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

    for possibly_initialized in [false, true] {
        let local_count = layout.local_count();
        let mut state = BorrowState::new_uninitialized(local_count);
        state.initialize_parameter(argument_index);
        if possibly_initialized {
            let mut initialized = state.clone();
            initialized.initialize_parameter(target_index);
            assert!(state.join_in_place(&initialized));
        }
        let error =
            transfer_isolated_statement(&checker, &layout, call_block, call_statement, &mut state)
                .expect_err(
                    "a dedicated result update must not initialize a possibly uninitialized path",
                );
        assert_borrow_error_view::<UseOfUninitializedLocalView>(error);
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
    let registry = default_external_package_registry(&mut string_table);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork, &registry)
            .expect("the immutable update fixture should lower");
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
                        results,
                        ..
                    } if args.len() == 1
                        && matches!(results.as_slice(), [HirLocalDestination::Define(_)])
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
    let HirStatementKind::Call { results, .. } = &mut call_statement.kind else {
        unreachable!("the selected statement is a call")
    };
    *results = vec![HirLocalDestination::Update(target)];

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
    let mut checker = BorrowChecker::new(
        &module,
        &type_environment,
        &registry,
        &path_fork,
        &string_table,
    );
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
        state.initialize_parameter(argument_index);
        state.initialize_parameter(mutable_root_index);
        if alias_only {
            let roots = state.effective_roots(mutable_root_index);
            seed_whole_alias(&mut state, target_index, mutable_root_index, roots);
        } else {
            state.initialize_parameter(target_index);
        }

        let error =
            transfer_isolated_statement(&checker, &layout, call_block, call_statement, &mut state)
                .expect_err("a dedicated update of an immutable binding must fail");
        assert_invalid_mutable_access_reason(error, InvalidMutableAccessReason::ImmutablePlace);
    }
}

#[test]
fn reentered_definition_reinitializes_a_previously_live_local() {
    let source = "probe |source String|:\n\
target ~= \"old\"\n\
sentinel = 0\n\
;";
    with_definition_probe(source, |checker, layout| {
        let (target, block, definition) = definition_named(checker, &layout, "target");
        let source = local_index_named(checker, &layout, "source");
        let local_count = layout.local_count();
        let mut state = BorrowState::new_uninitialized(local_count);
        state.initialize_parameter(source);
        let source_roots = state.effective_roots(source);
        seed_whole_alias(&mut state, target, source, source_roots);

        transfer_isolated_statement(checker, &layout, block, definition, &mut state)
            .expect("a reentered definition should establish the new binding");
        assert_eq!(state.binding_mode(target), LocalMode::SLOT);
        assert_eq!(
            state.resolve_cells(target).cells(),
            &[CellId::Slot(WrapperId::current(target))]
        );
        assert!(state.binding(target).value_roots().is_empty());
        assert!(state.binding(target).derivation().is_empty());
    });
}

const ENTRY_BOX_DEFINITION: &str = r#"
Box = |items {I32}|
probe |input Box|:
    prior ~= Box(input.items)
    target ~= Box(prior.items)
;
"#;

#[test]
fn first_heap_definition_matches_retirement_with_sparse_facts() {
    with_definition_probe(ENTRY_BOX_DEFINITION, |checker, layout| {
        let (prior, prior_block, prior_definition) = definition_named(checker, &layout, "prior");
        let (target, block, definition) = definition_named(checker, &layout, "target");
        let input = local_index_named(checker, &layout, "input");
        let (field, child_value) = constructor_field(checker.module, definition);
        let mut state = BorrowState::new_uninitialized(layout.local_count());
        state.initialize_parameter(input);
        transfer_isolated_statement(checker, &layout, prior_block, prior_definition, &mut state)
            .expect("the prefix constructor should store the immutable input child");
        assert!(
            !state.layer_is_empty(),
            "retirement must have sparse state to scan"
        );
        assert!(state.has_pristine_alloc_binding(target));
        assert!(layout.local_has_in_place_mutation_surface[target]);
        assert_eq!(layout.acyclic_entry_block, Some(block.id));
        assert_eq!(
            layout.local_first_write_order[target],
            layout.statement_order_or_unknown(definition.id)
        );

        let mut retirement_state = state.clone();
        let mut retirement_layout = layout.clone();
        retirement_layout.acyclic_entry_block = None;
        let (entry_stats, mut entry_facts) =
            transfer_isolated_statement(checker, &layout, block, definition, &mut state)
                .expect("the entry definition should keep the captured child");
        let (retirement_stats, mut retirement_facts) = transfer_isolated_statement(
            checker,
            &retirement_layout,
            block,
            definition,
            &mut retirement_state,
        )
        .expect("the same definition should also work with full retirement");
        assert_eq!(state, retirement_state);
        assert_eq!(
            entry_stats.statement_facts,
            retirement_stats.statement_facts
        );
        let entry_facts = entry_facts
            .drain_serialized(&layout)
            .collect::<FxHashMap<_, _>>();
        let retirement_facts = retirement_facts
            .drain_serialized(&retirement_layout)
            .collect::<FxHashMap<_, _>>();
        assert_eq!(entry_facts, retirement_facts);
        let child_fact = entry_facts
            .get(&child_value)
            .expect("the incoming projected read should publish a value fact");
        assert_eq!(
            child_fact.classification,
            ValueAccessClassification::SharedRead
        );
        assert_eq!(child_fact.roots, [layout.local_ids[prior]]);

        let retained = state.resolve_alloc_path(target, &layout.local_mutable, [field]);
        let original = state.resolve_alloc_path(input, &layout.local_mutable, [field]);
        assert!(retained.layer_only);
        assert_eq!(retained.ids, original.ids);
        assert!(
            retained.readonly,
            "the mutable holders must not grant write permission"
        );
        assert_eq!(retained.readonly_source, Some(input));
    });
}

#[test]
fn reentered_heap_definition_rebases_the_captured_and_retained_child() {
    let source = r#"
Box = |items {I32}|
probe |prior Box, flag Bool|:
    loop flag:
        target ~= Box(prior.items)
        retained = target.items
        observed = retained.length()
    ;
;
"#;
    with_definition_probe(source, |checker, layout| {
        let (target, block, definition) = definition_named(checker, &layout, "target");
        let (retained, alias_block, alias_definition) =
            definition_named(checker, &layout, "retained");
        let prior = local_index_named(checker, &layout, "prior");
        let (field, _) = constructor_field(checker.module, definition);
        let count = layout.local_count();
        let mut state = BorrowState::new_uninitialized(count);
        // Seed the previous dynamic binding and its carried provenance at the statement
        // frontier, without the CFG driver's lexical-scope kills.
        state.initialize_parameter(target);
        let roots = state.effective_roots(target);
        state.set_binding(prior, BindingSpec::Slot, roots, RootSet::empty(count));
        transfer_isolated_statement(checker, &layout, alias_block, alias_definition, &mut state)
            .expect("the previous iteration should retain the old projected value");
        let old = state.resolve_local_allocs(retained);
        assert!(old.layer_only && !old.readonly);
        assert_ne!(layout.acyclic_entry_block, Some(block.id));

        transfer_isolated_statement(checker, &layout, block, definition, &mut state)
            .expect("re-entry must retire the old binding without losing the capture");
        let retained = state.resolve_local_allocs(retained);
        let captured = state.resolve_alloc_path(target, &layout.local_mutable, [field]);
        assert_ne!(retained.ids, old.ids);
        assert_eq!(captured.ids, retained.ids);
        assert_eq!(retained.ids.len(), 1);
        assert_eq!(state.alloc_origin_local(retained.ids[0]), Some(target));
        assert!(
            !state.alloc_singular(retained.ids[0]),
            "the old child must be retired"
        );
        assert!(
            !retained.readonly,
            "the older projection keeps its permission"
        );
        assert!(
            captured.readonly,
            "the incoming capture came through immutable prior"
        );
    });
}

#[test]
fn entry_definition_rebases_a_root_first_interned_by_capture() {
    with_definition_probe(ENTRY_BOX_DEFINITION, |checker, layout| {
        let (prior, _, _) = definition_named(checker, &layout, "prior");
        let (target, block, definition) = definition_named(checker, &layout, "target");
        let (field, _) = constructor_field(checker.module, definition);
        let count = layout.local_count();
        let mut state = BorrowState::new_uninitialized(count);
        // Valid acyclic execution cannot carry the uninitialized target through prior. Seed
        // just that coarse provenance to exercise the guard when RHS capture interns the root.
        let mut roots = RootSet::empty(count);
        roots.insert(target);
        state.set_binding(prior, BindingSpec::Slot, roots, RootSet::empty(count));
        assert_eq!(layout.acyclic_entry_block, Some(block.id));
        assert!(state.layer_is_empty());
        assert!(state.has_pristine_alloc_binding(target));

        transfer_isolated_statement(checker, &layout, block, definition, &mut state)
            .expect("the captured child must be rebased rather than attached to itself");
        let captured = state.resolve_alloc_path(target, &layout.local_mutable, [field]);
        let new_root = state.resolve_local_allocs(target).ids[0];
        assert!(captured.layer_only && !captured.readonly);
        assert_eq!(captured.ids.len(), 1);
        let child = captured.ids[0];
        assert_ne!(child, state.alloc_child(new_root, field));
        assert_eq!(state.alloc_origin_local(child), Some(target));
        assert!(!state.alloc_singular(child));
    });
}

#[test]
fn ordinary_update_rejects_immutable_slot_and_alias_bindings() {
    let source = "probe ||:\n\
mutable_root ~= \"root\"\n\
mutable_target ~= \"old\"\n\
~mutable_target = \"replacement\"\n\
immutable_target = \"old\"\n\
sentinel = 0\n\
;";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut string_table);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork, &registry)
            .expect("the immutable update fixture should lower");
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
    let checker = BorrowChecker::new(
        &module,
        &type_environment,
        &registry,
        &path_fork,
        &string_table,
    );
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
        state.initialize_parameter(mutable_root_index);
        if alias_only {
            let roots = state.effective_roots(mutable_root_index);
            seed_whole_alias(&mut state, immutable_index, mutable_root_index, roots);
        } else {
            state.initialize_parameter(immutable_index);
        }

        let error = transfer_isolated_statement(&checker, &layout, block, update, &mut state)
            .expect_err("ordinary assignment to an immutable binding must fail");
        assert_invalid_mutable_access_reason(error, InvalidMutableAccessReason::ImmutablePlace);
    }
}

#[test]
fn ordinary_mixed_update_checks_a_seeded_competing_holder() {
    let source = "probe |flag Bool|:\n\
observer = \"observer\"\n\
source ~= \"source\"\n\
target ~= \"old\"\n\
if flag:\n\
    ~target = source\n\
else\n\
    ~target = \"branch\"\n\
;\n\
~target = \"replacement\"\n\
sentinel = 0\n\
observed = copy observer\n\
;";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut string_table);
    let (module, type_environment) = lower_ast(ast, &mut string_table, &mut path_fork, &registry)
        .expect("fixture source should lower");
    validate_hir_module(&module, &type_environment, &registry)
        .expect("the complete fixture should be structurally valid HIR");
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
    let checker = BorrowChecker::new(
        &module,
        &type_environment,
        &registry,
        &path_fork,
        &string_table,
    );
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
    assert_invalid_mutable_access_at(
        error,
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
        expected_span,
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
    ~target = source\n\
else\n\
    ~target = \"branch\"\n\
;\n\
unused = identity(argument)\n\
sentinel = 0\n\
observed = copy observer\n\
;";
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut string_table);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork, &registry)
            .expect("fixture source should lower");
    let target = local_named(&module, &path_fork, &string_table, "target");
    let (call_result, call_region) = module
        .blocks
        .iter()
        .find_map(|block| {
            block
                .statements
                .iter()
                .find_map(|statement| match &statement.kind {
                    HirStatementKind::Call { args, results, .. } if args.len() == 1 => {
                        match results.as_slice() {
                            [HirLocalDestination::Define(local)] => Some((*local, block.region)),
                            _ => None,
                        }
                    }
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
                HirStatementKind::Call { args, results, .. }
                    if args.len() == 1
                        && matches!(
                            results.as_slice(),
                            [HirLocalDestination::Define(local)] if *local == call_result
                        ) =>
                {
                    results[0] = HirLocalDestination::Update(target);
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
    validate_hir_module(&module, &type_environment, &registry)
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
    let mut checker = BorrowChecker::new(
        &module,
        &type_environment,
        &registry,
        &path_fork,
        &string_table,
    );
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
    assert_invalid_mutable_access_at(
        error,
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
        expected_span,
    );
}

// WHAT: a whole-binding write is a write through every direct alias of that binding cell.
// WHY: a source mutable alias (`first ~= data`) shares data's cell, so a fresh-value rebinding
// while first is still read afterwards is the same overlapping mutation as `~items.push(...)`.
#[test]
fn whole_binding_write_rejects_a_live_direct_alias() {
    for source in [
        r#"
data ~= 42
first ~= data
~data = 8
after = first
"#,
        r#"
data ~= 42
first = data
second = first
~data = 8
after = second
"#,
    ] {
        let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
        let registry = default_external_package_registry(&mut string_table);
        let (module, type_environment) =
            lower_ast(ast, &mut string_table, &mut path_fork, &registry)
                .expect("fixture source should lower");
        validate_hir_module(&module, &type_environment, &registry)
            .expect("the complete fixture should be structurally valid HIR");

        let error = run_borrow_checker_with_types(
            &module,
            &type_environment,
            &registry,
            &path_fork,
            &string_table,
        )
        .expect_err("a write through an aliased binding cell must be rejected");
        assert_invalid_mutable_access_reason(
            error,
            InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
        );
    }
}

// WHAT: the same write is legal once the alias can no longer observe it.
// WHY: alias activity is non-lexical, and `copy` produces independent provenance rather than a
// direct alias, so neither shape may turn the whole-binding write into a false conflict.
#[test]
fn whole_binding_write_allows_a_dead_or_copied_alias() {
    for source in [
        r#"
data ~= 42
first ~= data
observed = first
~data = 8
"#,
        r#"
data ~= 42
first ~= copy data
~data = 8
after = first
"#,
        r#"
identity |value I32| -> I32:
return value
;
original ~= 42
returned = identity(original)
~original = 8
after = returned
"#,
    ] {
        let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
        let registry = default_external_package_registry(&mut string_table);
        let (module, type_environment) =
            lower_ast(ast, &mut string_table, &mut path_fork, &registry)
                .expect("fixture source should lower");
        validate_hir_module(&module, &type_environment, &registry)
            .expect("the complete fixture should be structurally valid HIR");

        run_borrow_checker_with_types(
            &module,
            &type_environment,
            &registry,
            &path_fork,
            &string_table,
        )
        .expect("a dead or copied alias must not block the whole-binding write");
    }
}

// WHAT: a projected load through an intermediate alias observes the value's allocation, not the
//       alias binding cell that can later receive a fresh value.
// WHY: treating the projected result as a direct alias of the binding cell made the whole-binding
//      replacement an overlapping mutation and rejected legal source.
#[test]
fn projected_load_through_an_alias_keeps_the_old_allocation() {
    for source in [
        r#"
Pair = | first {I32}, second {I32} |
probe || -> I32:
pair ~= Pair(first = {1}, second = {2})
view = pair
old = view.first
~pair = Pair(first = {3, 4}, second = {5})
return old.length()
;
"#,
        r#"
Pair = | first {I32}, second {I32} |
probe || -> I32:
pair ~= Pair(first = {1}, second = {2})
old = pair.first
~pair = Pair(first = {3, 4}, second = {5})
return old.length()
;
"#,
    ] {
        check_source(source).expect("a projected value must survive rebinding its source binding");
    }
}

#[test]
fn projected_writer_can_outlive_a_directly_constructed_holder() {
    for (first, second) in [
        (
            "new_count = holder.first.length()",
            "old_count = writer.length()",
        ),
        (
            "old_count = writer.length()",
            "new_count = holder.first.length()",
        ),
    ] {
        let source = format!(
            "Pair = |first {{I32}}, second {{I32}}|\n\
             probe || -> I32:\n\
             holder ~= Pair({{1}}, {{0}})\n\
             writer ~= holder.first\n\
             ~holder = Pair({{2, 3}}, {{4}})\n\
             {first}\n\
             {second}\n\
             return new_count + old_count\n\
             ;"
        );
        check_source(&source).expect("the directly constructed holders are independent");
    }
}

// WHAT: a projected mutable alias keeps the holder cell as a conservative write identity even
//       when the containing allocation roots are recorded. Rebinding the holder to a fresh value
//       does not retarget the alias, but reading the rebound holder while the alias is live is
//       conservatively rejected.
// WHY: a fresh allocation's identity is its owning cell or empty provenance, and that identity
//      cannot be transported precisely through joins, alias chains, calls or scope exits. The
//      holder cell keeps those conflicts rather than losing a live alias. Track the fresh
//      allocation as an explicit transported root to relax this.
#[test]
fn mutable_projected_alias_conservatively_blocks_rebound_holder_read() {
    let source = r#"
Pair = | first {I32}, second {I32} |
same_pair |value Pair| -> Pair:
return value
;
probe || -> I32:
original ~= Pair(first = {1}, second = {0})
holder ~= same_pair(original)
writer ~= holder.first
~holder = Pair(first = {2}, second = {3})
read_after = holder.first.length()
return read_after + writer.length()
;
"#;
    let error = check_source(source)
        .expect_err("the conservative holder-cell identity must reject the live-alias read");
    assert_borrow_error_view::<SharedMutableConflictView>(error);
}

#[test]
fn retained_writer_can_read_the_old_leaf_of_a_rebound_call_result() {
    let source = "\
Pair = |first {I32}, second {I32}|\n\
same_pair |value Pair| -> Pair:\n\
return value\n\
;\n\
probe || -> I32:\n\
original ~= Pair({1}, {0})\n\
holder ~= same_pair(original)\n\
writer ~= holder.first\n\
~holder = Pair({2}, {3})\n\
return writer.length()\n\
;";
    check_source(source).expect("reading through the writer does not access the rebound holder");
}

// WHAT: a mutable alias writes through to its referent cell, so its value provenance is not the
//       destination whose mutability and exclusivity govern the write.
// WHY: consulting provenance rejected a legal scalar write when the slot stored a produced value
//      from an immutable origin.
#[test]
fn alias_write_through_uses_the_referent_cell() {
    let source = r#"
identity |value I32| -> I32:
return value
;
set_six |value ~I32|:
~value = 6
;
probe || -> I32:
original = 5
slot ~= 0
~slot = identity(original)
writer ~= slot
~writer = 6
set_six(~writer)
return slot
;
"#;
    check_source(source)
        .expect("a write and mutable call through a scalar alias must target its mutable referent");
}

// WHAT: a live shared alias of the referent cell blocks both a write through a mutable alias and
//       a mutable call through it.
// WHY: an earlier version of the write-through rule checked only the alias's value provenance, so
//      a second alias of the referent stayed invisible to the write. The shared alias is created
//      through the mutable alias here, which keeps alias acquisition itself legal; the positive
//      controls below differ only in dropping the final read, so they prove the rejection lands on
//      the write/call rather than on acquisition.
#[test]
fn alias_write_through_checks_live_referent_aliases_at_the_write() {
    for source in [
        r#"
probe || -> I32:
slot ~= 0
writer ~= slot
view = writer
~writer = 6
return view
;
"#,
        r#"
set_six |value ~I32|:
~value = 6
;
probe || -> I32:
slot ~= 0
writer ~= slot
view = writer
set_six(~writer)
return view
;
"#,
    ] {
        let error = check_source(source)
            .expect_err("a live shared alias of the referent must block alias write-through");
        assert_invalid_mutable_access_reason(
            error,
            InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
        );
    }

    for source in [
        r#"
probe || -> I32:
slot ~= 0
writer ~= slot
view = writer
~writer = 6
return slot
;
"#,
        r#"
set_six |value ~I32|:
~value = 6
;
probe || -> I32:
slot ~= 0
writer ~= slot
view = writer
set_six(~writer)
return slot
;
"#,
    ] {
        check_source(source).expect("a dead shared alias must not block alias write-through");
    }
}

// WHAT: an alias whose only overlap fact is value provenance checks that provenance on a write.
// WHY: a produced alias has no referent cell to check, so the write falls back to the provenance
//      roots. The seeded state isolates that rule from the alias-acquisition checks.
#[test]
fn alias_without_recorded_cells_checks_provenance_on_update() {
    let source = r#"
probe ||:
source ~= {1, 2}
target ~{I32} = {0}
observer = source
~target = {4}
after = observer.length()
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut string_table);
    let (module, type_environment) = lower_ast(ast, &mut string_table, &mut path_fork, &registry)
        .expect("fixture source should lower");
    validate_hir_module(&module, &type_environment, &registry)
        .expect("the complete fixture should be structurally valid HIR");
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
    let checker = BorrowChecker::new(
        &module,
        &type_environment,
        &registry,
        &path_fork,
        &string_table,
    );
    let reachable = checker
        .collect_reachable_blocks(function)
        .expect("fixture CFG should be reachable");
    let layout = checker
        .build_function_layout(function, &reachable)
        .expect("fixture local layout should build");
    let target = local_named(&module, &path_fork, &string_table, "target");
    let source = local_named(&module, &path_fork, &string_table, "source");
    let observer = local_named(&module, &path_fork, &string_table, "observer");
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
        .expect("the fixture should contain the fresh-value update");
    let target_index = layout
        .index_of(target)
        .expect("target should be in the layout");
    let source_index = layout
        .index_of(source)
        .expect("source should be in the layout");
    let observer_index = layout
        .index_of(observer)
        .expect("observer should be in the layout");
    assert!(layout.local_mutable[target_index]);
    let local_count = layout.local_count();
    let mut state = BorrowState::new_uninitialized(local_count);
    state.initialize_parameter(source_index);
    let source_roots = state.effective_roots(source_index);
    state.set_binding(
        target_index,
        BindingSpec::Alias {
            addresses: AddressSet::from_cells([], Coverage::Partial),
            route_roots: RootSet::empty(local_count),
        },
        source_roots.clone(),
        RootSet::empty(local_count),
    );
    let mut unheld = state.clone();
    seed_whole_alias(&mut state, observer_index, source_index, source_roots);
    let error = transfer_isolated_statement(&checker, &layout, block, update, &mut state)
        .expect_err("a cell-free alias write must check its provenance roots");
    assert_invalid_mutable_access_reason(
        error,
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );

    // Positive control: the same seeded alias is writable while nothing observes its provenance.
    unheld.initialize_parameter(observer_index);
    transfer_isolated_statement(&checker, &layout, block, update, &mut unheld)
        .expect("a dead provenance observer must not block the alias write");
}

// WHAT: a fresh value written through an alias is owned by the referent cell, so a later value
//       alias of the alias observes that storage.
// WHY: clearing the alias's value roots made the fresh allocation invisible; the push then failed
//      to see the live shared view and was accepted.
#[test]
fn fresh_alias_write_records_referent_storage_for_later_aliases() {
    let source = r#"
probe || -> I32:
slot ~= {0}
writer ~= slot
~writer = {1}
view = writer
~writer.push(2)
return view.length()
;
"#;
    let error = check_source(source)
        .expect_err("a shared view of a fresh alias value must block a later push");
    assert_invalid_mutable_access_reason(
        error,
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
}

// WHAT: the same referent-storage identity must reach a projected mutable argument.
// WHY: a projected argument's roots come from value provenance; after a fresh write that set was
//      empty, so one call could mutate and read the same field without an overlap diagnostic.
#[test]
fn fresh_alias_write_records_referent_storage_for_projected_same_call_arguments() {
    let source = r#"
Pair = | first I32, second I32 |

mutate_and_read |target ~I32, observed I32| -> I32:
~target = 9
return observed
;

probe || -> I32:
pair ~= Pair(1, 2)
writer ~= pair
~writer = Pair(3, 4)
return mutate_and_read(~writer.first, writer.first)
;
"#;
    let error = check_source(source)
        .expect_err("a mutable and shared projection argument must overlap after a fresh write");
    assert_borrow_error_view::<SharedMutableConflictView>(error);
}

// WHAT: a write through an alias on a loop backedge records the referent storage as well.
// WHY: the view is created in one iteration and must still block the next iteration's mutation;
//      dropping the fresh value identity at the backedge would let the loop-carried view escape.
#[test]
fn fresh_alias_write_records_referent_storage_across_a_loop_backedge() {
    let source = r#"
identity |value {I32}| -> {I32}:
    return value
;
probe || -> I32:
    slot ~= {1, 2}
    writer ~= slot
    view ~= {9}
    size ~= 0
    flag ~= true
    loop flag:
        ~writer.push(3)
        ~size = view.length()
        ~writer = {4}
        ~view = identity(writer)
        ~flag = false
    ;
    return size
;
"#;
    let error = check_source(source)
        .expect_err("a loop-carried view must block the next iteration's mutation");
    assert_invalid_mutable_access_reason(
        error,
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
}

// WHAT: one direct cell is not a definite referent when another path writes the alias's own slot.
// WHY: strong-updating the direct cell dropped its old value, so a shared observer of the slot
//      missed the still-live original and its later mutation was accepted.
#[test]
fn joined_alias_with_a_slot_path_keeps_the_referent_value() {
    let source = r#"
identity |value {I32}| -> {I32}:
    return value
;
probe |flag Bool| -> I32:
    original ~= {1}
    slot ~= {0}
    writer ~= {0}
    if flag:
        ~writer = slot
        ~writer = identity(original)
    else
        ~slot = identity(original)
    ;
    ~writer = {3}
    observed = identity(slot)
    ~original.push(4)
    return observed.length()
;
"#;
    let error =
        check_source(source).expect_err("the joined alias may leave the old value in the slot");
    assert_invalid_mutable_access_reason(
        error,
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
}

// WHAT: a shared view derived through a mutable alias does not conflict with that alias.
// WHY: value provenance names an allocation, not a binding cell; treating it as a foreign cell
//      access rejected a view that is dead before the alias's later mutation.
#[test]
fn derived_shared_view_read_is_not_a_foreign_slot_access() {
    let source = r#"
identity |value {I32}| -> {I32}:
    return value
;
probe || -> I32:
    slot ~= {0}
    writer ~= slot
    ~writer = {1}
    view = identity(writer)
    size = view.length()
    ~writer.push(2)
    return size
;
"#;
    check_source(source)
        .expect("reading a derived view must not conflict with the alias that produced it");
}

// WHAT: the same restriction must still reject a read of the slot itself while the alias is live.
// WHY: the alias scan moved to binding cells, so the source binding's own cell must remain an
//      active-exclusive conflict.
#[test]
fn source_slot_read_still_conflicts_with_the_mutable_alias() {
    let source = r#"
identity |value {I32}| -> {I32}:
    return value
;
probe || -> I32:
    slot ~= {0}
    writer ~= slot
    ~writer = {1}
    view = identity(writer)
    size = view.length()
    snapshot = slot
    ~writer.push(2)
    return size + snapshot.length()
;
"#;
    let error = check_source(source)
        .expect_err("the source slot must stay unreadable while the alias is live");
    assert_borrow_error_view::<SharedMutableConflictView>(error);
}

// WHAT: a projection-free alias of a projected mutable alias keeps the projected referent as
//       overlap identity through the chain.
// WHY: the replacement checks only written storage, so dropping the inherited target let the
//      live view escape even though the field storage is replaced through `writer`.
#[test]
fn projected_alias_chain_keeps_referent_overlap_for_replacement() {
    let source = r#"
Pair = | first {I32}, second {I32} |

same_pair |value Pair| -> Pair:
    return value
;

probe || -> I32:
    original ~= Pair({1}, {0})
    holder ~= same_pair(original)
    writer ~= holder.first
    view = writer
    ~writer = {2, 3}
    return view.length()
;
"#;
    let error = check_source(source)
        .expect_err("a projection-free alias chain must keep the projected referent overlap");
    assert_invalid_mutable_access_reason(
        error,
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess,
    );
}

// WHAT: replacing a projected field's value must not require mutability of the replaced value's
//       origin; only the storage that receives the write must be mutable.
// WHY: the old-value provenance was passed with `require_root_mutable`, so an immutable origin
//      rejected a legal field replacement.
#[test]
fn projected_alias_fresh_write_checks_mutability_of_storage_only() {
    let source = r#"
Pair = | first {I32}, second {I32} |

identity |value {I32}| -> {I32}:
return value
;

probe || -> I32:
original = {1, 2}
slot ~= Pair(first = {0}, second = {0})
writer ~{I32} = {0}
~writer = slot.first
~writer = identity(original)
~writer = {3}
return original.length()
;
"#;
    check_source(source)
        .expect("replacing a field must not require mutability of the replaced value's origin");
}

// WHAT: a join of a mutable alias into more than one referent cell must weakly update those
//       cells on a write-through, not replace their provenance.
// WHY: a strong update cleared the old value of a possible referent, so a value read from that
//      cell after the write appeared independent of the old allocation.
#[test]
fn joined_alias_fresh_write_weakly_updates_possible_referents() {
    let source = r#"
identity |value {I32}| -> {I32}:
return value
;

probe |flag Bool| -> I32:
original ~= {1, 2}
slot ~= {0}
~slot = identity(original)
spare ~= {5}
writer ~{I32} = {0}
if flag:
    ~writer = slot
else
    ~writer = spare
;
~writer = {3}
observed = identity(slot)
~original.push(4)
return observed.length()
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut string_table);
    let (module, type_environment) = lower_ast(ast, &mut string_table, &mut path_fork, &registry)
        .expect("fixture source should lower");
    validate_hir_module(&module, &type_environment, &registry)
        .expect("the complete fixture should be structurally valid HIR");
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
    let checker = BorrowChecker::new(
        &module,
        &type_environment,
        &registry,
        &path_fork,
        &string_table,
    );
    let reachable = checker
        .collect_reachable_blocks(function)
        .expect("fixture CFG should be reachable");
    let layout = checker
        .build_function_layout(function, &reachable)
        .expect("fixture local layout should build");
    let original = local_named(&module, &path_fork, &string_table, "original");
    let slot = local_named(&module, &path_fork, &string_table, "slot");
    let spare = local_named(&module, &path_fork, &string_table, "spare");
    let writer = local_named(&module, &path_fork, &string_table, "writer");
    let indices = [original, slot, spare, writer].map(|local| {
        layout
            .index_of(local)
            .expect("seeded local should be in the layout")
    });
    let [original_index, slot_index, spare_index, writer_index] = indices;
    let (block, write) = module
        .blocks
        .iter()
        .find_map(|block| {
            block
                .statements
                .iter()
                .find(|statement| {
                    is_local_update(&module, statement, writer)
                        && matches!(
                            &statement.kind,
                            HirStatementKind::Write { value, .. }
                                if !matches!(
                                    &module.expressions.expression(*value).kind,
                                    HirExpressionKind::Load(_)
                                )
                        )
                })
                .map(|statement| (block, statement))
        })
        .expect("the joined fresh write should lower");

    let local_count = layout.local_count();
    let mut state = BorrowState::new_uninitialized(local_count);
    state.initialize_parameter(original_index);
    let original_roots = state.effective_roots(original_index);
    state.set_binding(
        slot_index,
        BindingSpec::Slot,
        original_roots.clone(),
        RootSet::empty(local_count),
    );
    state.initialize_parameter(spare_index);
    let mut spare_path = state.clone();
    let spare_roots = spare_path.effective_roots(spare_index);
    seed_whole_alias(&mut state, writer_index, slot_index, original_roots);
    seed_whole_alias(&mut spare_path, writer_index, spare_index, spare_roots);
    assert!(state.join_in_place(&spare_path));
    assert_eq!(
        state.resolve_cells(writer_index).cells(),
        &[
            CellId::Slot(WrapperId::current(slot_index)),
            CellId::Slot(WrapperId::current(spare_index)),
        ]
    );

    transfer_isolated_statement(&checker, &layout, block, write, &mut state)
        .expect("a fresh write through a joined alias is legal");

    let writer_roots = state
        .binding(writer_index)
        .value_roots()
        .iter_ones()
        .collect::<Vec<_>>();
    assert_eq!(writer_roots, vec![slot_index, spare_index]);
    assert!(
        state
            .binding(slot_index)
            .value_roots()
            .contains(original_index),
        "a possible referent must keep its old value provenance"
    );
    assert!(
        state.binding(spare_index).value_roots().is_empty(),
        "a referent that did not hold the old value gains no provenance"
    );
}

#[test]
fn scalar_projected_aliases_allow_sibling_writes_but_reject_a_second_same_cell_loan() {
    for (right, expected_code) in [
        ("right ~= pair.right", None),
        ("right ~= pair.left", Some("MOTH-BORROW-0007")),
    ] {
        let source = format!(
            "Pair = |left I32, right I32|\n\
             probe || -> I32:\n\
             pair ~= Pair(1, 2)\n\
             left ~= pair.left\n\
             {right}\n\
             ~right = 7\n\
             return left + right\n\
             ;"
        );
        let result = check_source(&source);
        if let Some(code) = expected_code {
            let error = result.expect_err("the second mutable acquisition overlaps the first");
            assert_borrow_code_at(error, &source, right, code);
        } else {
            result.expect("the live sibling aliases designate disjoint scalar cells");
        }
    }
}

#[test]
fn prior_scalar_sibling_store_allows_direct_and_explicit_alias_access() {
    for sequence in [
        "observed = pair.right\n~pair.left = observed\nreturn observed + pair.left",
        "writer ~= pair.left\nobserved = pair.right\n~writer = observed\nreturn observed + writer",
    ] {
        let source = format!(
            "Pair = |left I32, right I32|\n\
             probe || -> I32:\n\
             pair ~= Pair(1, 2)\n\
             ~pair.right = 4\n\
             {sequence}\n\
             ;"
        );
        check_source(&source).expect("a prior store does not make sibling fields overlap");
    }
}

#[test]
fn shared_scalar_observation_overlaps_the_same_field_or_its_aggregate() {
    for (observation, later_read, overlaps) in [
        ("observed = pair.right", "observed", false),
        ("observed = pair.left", "observed", true),
        ("observed = pair", "observed.right", true),
    ] {
        let source = format!(
            "Pair = |left I32, right I32|\n\
             probe || -> I32:\n\
             pair ~= Pair(1, 2)\n\
             writer ~= pair.left\n\
             {observation}\n\
             ~writer = 7\n\
             return {later_read} + writer\n\
             ;"
        );
        let result = check_source(&source);
        if overlaps {
            let error = result.expect_err("the observation overlaps the live exclusive alias");
            assert_borrow_code_at(error, &source, observation, "MOTH-BORROW-0003");
        } else {
            result.expect("a shared read of the sibling scalar cell is disjoint");
        }
    }
}

#[test]
fn named_heap_sibling_writes_check_shared_children_across_holder_alternatives() {
    for (setup, overlaps) in [
        ("pair ~= Pair(items, {2})", false),
        ("pair ~= Pair(items, items)", true),
        (
            "pair ~= Pair({1}, {2})\nif flag:\n~pair = Pair(items, {5})\n;",
            false,
        ),
        (
            "pair ~= Pair({1}, {2})\nif flag:\n~pair = Pair(items, items)\n;",
            true,
        ),
    ] {
        let source = format!(
            "Pair = |left {{I32}}, right {{I32}}|\n\
             probe |flag Bool| -> I32:\n\
             items ~= {{3, 4}}\n\
             {setup}\n\
             observed = pair.left\n\
             ~pair.right.push(7)\n\
             return observed.length()\n\
             ;"
        );
        if overlaps {
            let error = check_source(&source).expect_err("a live alias observes the shared child");
            assert_borrow_code_at(error, &source, "~pair.right.push(7)", "MOTH-BORROW-0007");
        } else {
            check_source(&source).expect("the children are disjoint in every holder alternative");
        }
    }
}

#[test]
fn projected_cell_writes_allow_independent_siblings() {
    let pair = "pair ~= Pair(Box(1), Box(2))";
    for write in ["~pair.right = Box(3)", "~pair.right.value = 3"] {
        let source = projected_cell_fixture(pair, "pair.left", write, "hold.value");
        check_source(&source).unwrap_or_else(|error| panic!("{write} should be legal: {error:?}"));
    }
}

#[test]
fn projected_cell_writes_reject_live_overlaps() {
    let independent = "pair ~= Pair(Box(1), Box(2))";
    let shared = "child ~= Box(1)\npair ~= Pair(child, child)";
    for (pair, observer, write, read, access) in [
        (
            independent,
            "pair.left",
            "~pair.left = Box(3)",
            "hold.value",
            "= Box(3)",
        ),
        (
            independent,
            "pair",
            "~pair.right = Box(3)",
            "hold.left.value",
            "= Box(3)",
        ),
        (
            shared,
            "pair.left",
            "~pair.right.value = 3",
            "hold.value",
            "= 3",
        ),
        // The read retained the shared allocation, not a proven distinct cell occurrence.
        (
            shared,
            "pair.left",
            "~pair.right = Box(3)",
            "hold.value",
            "= Box(3)",
        ),
    ] {
        let source = projected_cell_fixture(pair, observer, write, read);
        assert_layer_conflict_at(&source, access);
    }
}

#[test]
fn projected_cell_replacement_checks_explicit_old_descendants() {
    let source = "\
Leaf = |value I32|\n\
Box = |leaf Leaf|\n\
Pair = |left Box, right Box|\n\
probe || -> I32:\n\
leaf ~= Leaf(1)\n\
pair ~= Pair(Box(leaf), Box(leaf))\n\
hold = pair.left.leaf\n\
~pair.right = Box(Leaf(3))\n\
return hold.value\n\
;";
    assert_layer_conflict_at(source, "= Box(Leaf(3))");
}

#[test]
fn projected_cell_replacement_does_not_mutate_old_shared_value() {
    for binding in ["child ~= Box(2)", "child = Box(2)"] {
        let pair = format!("{binding}\npair ~= Pair(child, child)");
        let source = projected_cell_fixture(&pair, "child", "~pair.right = Box(3)", "hold.value");
        check_source(&source).expect("the old shared value is not being mutated");
    }

    let pair = "child = Box(2)\npair ~= Pair(child, child)";
    let source = projected_cell_fixture(pair, "child", "~pair.right.value = 3", "hold.value");
    let error = check_source(&source).expect_err("mutating through the read-only child must fail");
    assert_invalid_mutable_access_reason(error, InvalidMutableAccessReason::ImmutablePlace);
}

fn projected_cell_fixture(pair: &str, observe: &str, write: &str, read: &str) -> String {
    format!(
        "Box = |value I32|\n\
         Pair = |left Box, right Box|\n\
         probe || -> I32:\n\
         {pair}\n\
         hold = {observe}\n\
         {write}\n\
         return {read}\n\
         ;"
    )
}

#[test]
fn duplicate_collection_children_remain_aliased_after_removal() {
    for (binding, removed, surviving) in [
        ("entries ~= {child, child}", "0", "0"),
        (
            r#"entries ~{String = {I32}} = {"first" = child, "second" = child}"#,
            r#""first""#,
            r#""second""#,
        ),
    ] {
        let source = format!(
            "probe || -> I32:\n\
                child ~= {{1}}\n\
                {binding}\n\
                detached ~= ~entries.remove({removed}) catch then {{0}}\n\
                ~detached.push(2)\n\
                remaining = entries.get({surviving}) catch then {{0}}\n\
                return remaining.length()\n\
            ;"
        );
        assert_layer_conflict_at(&source, "detached.push");
    }
}

#[test]
fn copy_keeps_a_lazy_child_reachable_by_two_paths() {
    let source = "\
Box = |items {I32}|\n\
Pair = |right {I32}, left Box|\n\
probe |input Box| -> I32:\n\
    source = Pair(input.items, input)\n\
    owned ~= copy source\n\
    observed = Box(owned.right)\n\
    ~owned.left.items.push(2)\n\
    return observed.items.length()\n\
;";
    assert_layer_conflict_at(source, "items.push");
}

#[test]
fn scalar_lookup_survives_write_through_until_exact_replacement() {
    use super::borrow_checker_fact_tests::{assert_lookup_conflict, check_place_access_source};
    let fixture = |replacement| {
        format!(
            r#"
scores ~{{String = I32}} = {{"k" = 7}}
slot ~= 0
writer ~= slot
score = scores.get("k")!
~writer = score
other ~= writer
~scores.clear()
{replacement}
seen = other + 0
~[:[seen]]
"#
        )
    };
    assert_lookup_conflict(&fixture(""), "~scores.clear()");
    check_place_access_source(&fixture("~other = 0"))
        .expect("another alias definitely replaces the same occurrence");
    assert_lookup_conflict(&fixture("~other = other + 1"), "~scores.clear()");
}

#[test]
fn lookup_observations_distinguish_direct_and_conditional_overwrite() {
    use super::borrow_checker_fact_tests::{assert_lookup_conflict, check_place_access_source};
    let fixture = |replacement| {
        format!(
            r#"
probe |flag Bool| -> I32, Error!:
    scores ~{{String = I32}} = {{"k" = 7}}
    score ~= scores.get("k")!
    ~scores.clear()
    {replacement}
    return score
;
~[:[probe(true)!],[probe(false)!]]
"#
        )
    };
    check_place_access_source(&fixture("~score = 0"))
        .expect("definite replacement does not observe the old lookup");
    assert_lookup_conflict(&fixture("if flag:\n~score = 0\n;"), "~scores.clear()");
    assert_lookup_conflict(&fixture("~score = score + 1"), "~scores.clear()");
}

#[test]
fn construction_retains_lookup_on_the_selected_field_only() {
    use super::borrow_checker_fact_tests::{assert_lookup_conflict, check_place_access_source};
    let fixture = |replacement, observed| {
        format!(
            r#"
Bucket = |scores {{String = {{I32}}}}|
Saved = |item {{I32}}, count I32|
bucket ~= Bucket({{"k" = {{1, 2}}}})
score ~= bucket.scores.get("k")!
saved ~= Saved(score, 3)
~score = {{0}}
{replacement}
first = saved.item
second = saved.item
~bucket.scores.clear()
~[:[{observed}]]
"#
        )
    };
    assert_lookup_conflict(
        &fixture("", "first.length() + second.length()"),
        "~bucket.scores.clear()",
    );
    check_place_access_source(&fixture("", "saved.count"))
        .expect("the sibling scalar field is independent");
    check_place_access_source(&fixture(
        "~saved.item = {0}",
        "first.length() + second.length()",
    ))
    .expect("an exact field replacement kills the retained lookup");
}

fn assert_layer_conflict_at(source: &str, access: &str) {
    let error = check_source(source).expect_err("a live alias must block this write");
    let draft = error
        .into_draft_or_infrastructure()
        .expect("typed borrow diagnostic");
    let bag = compact_bag_for_test(DiagnosticBag::from(draft), None);
    let diagnostic = bag.first().expect("borrow diagnostic");
    let view = diagnostic
        .family_view::<InvalidMutableAccessView>()
        .expect("invalid mutable access");
    assert_eq!(
        view.reason,
        InvalidMutableAccessReason::AliasedValueRequiresExclusiveAccess
    );
    let expected = source.find(access).expect("authored access") as u32;
    assert!(
        matches!(
            diagnostic.primary_site(),
            SiteRef::Local(span) if span.local().inline_range().is_some_and(|range| range.start() == expected)
        ),
        "diagnostic should point to the authored access {access}"
    );
}

fn check_source(source: &str) -> Result<(), BorrowCheckError> {
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut string_table);
    let (module, type_environment) = lower_ast(ast, &mut string_table, &mut path_fork, &registry)
        .expect("fixture source should lower");
    validate_hir_module(&module, &type_environment, &registry)
        .expect("the complete fixture should be structurally valid HIR");
    run_borrow_checker_with_types(
        &module,
        &type_environment,
        &registry,
        &path_fork,
        &string_table,
    )
    .map(|_| ())
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

fn with_definition_probe(source: &str, check: impl FnOnce(&BorrowChecker<'_>, FunctionLayout)) {
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut string_table);
    let (module, type_environment) = lower_ast(ast, &mut string_table, &mut path_fork, &registry)
        .expect("definition fixture should lower");
    validate_hir_module(&module, &type_environment, &registry)
        .expect("complete definition fixture should be valid HIR");
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
        .expect("probe should lower to HIR");
    let checker = BorrowChecker::new(
        &module,
        &type_environment,
        &registry,
        &path_fork,
        &string_table,
    );
    let reachable = checker
        .collect_reachable_blocks(function)
        .expect("fixture CFG should be reachable");
    let layout = checker
        .build_function_layout(function, &reachable)
        .expect("fixture layout should build");
    check(&checker, layout);
}

fn local_index_named(checker: &BorrowChecker<'_>, layout: &FunctionLayout, name: &str) -> usize {
    let local = local_named(
        checker.module,
        checker.path_fork,
        checker.string_table,
        name,
    );
    layout
        .index_of(local)
        .expect("local should be in the layout")
}

fn definition_named<'a>(
    checker: &BorrowChecker<'a>,
    layout: &FunctionLayout,
    name: &str,
) -> (usize, &'a HirBlock, &'a HirStatement) {
    let index = local_index_named(checker, layout, name);
    let local = layout.local_ids[index];
    checker
        .module
        .blocks
        .iter()
        .find_map(|block| {
            block.statements.iter().find_map(|statement| {
                matches!(
                    &statement.kind,
                    HirStatementKind::Write {
                        target: HirWriteTarget::DefineLocal(id),
                        ..
                    } if *id == local
                )
                .then_some((index, block, statement))
            })
        })
        .unwrap_or_else(|| panic!("{name} should have an ordinary definition"))
}

fn constructor_field(module: &HirModule, definition: &HirStatement) -> (FieldKey, HirValueId) {
    let HirStatementKind::Write { value, .. } = &definition.kind else {
        panic!("fixture must define the constructor through a write");
    };
    let HirExpressionKind::StructConstruct { fields, .. } =
        &module.expressions.expression(*value).kind
    else {
        panic!("fixture must have a struct constructor");
    };
    let &[(field, child)] = module.expressions.struct_fields(*fields) else {
        panic!("fixture constructor must have one field");
    };
    (FieldKey::Field(field.0), child)
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

fn seed_whole_alias(state: &mut BorrowState, alias: usize, source: usize, value_roots: RootSet) {
    let local_count = value_roots.bit_len();
    let mut route_roots = RootSet::empty(local_count);
    route_roots.insert(source);
    state.set_binding(
        alias,
        BindingSpec::Alias {
            addresses: state.resolve_cells(source),
            route_roots,
        },
        value_roots,
        RootSet::empty(local_count),
    );
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
    state.initialize_parameter(source_index);
    for local in additional_slots {
        let index = layout
            .index_of(*local)
            .expect("slot local should be in the layout");
        state.initialize_parameter(index);
    }

    state.initialize_parameter(target_index);
    state.initialize_parameter(observer_index);
    let mut alias_path = state.clone();
    let source_roots = state.effective_roots(source_index);
    seed_whole_alias(
        &mut alias_path,
        target_index,
        source_index,
        source_roots.clone(),
    );
    if observer_holds_source {
        // Only one path acquires the mutable alias, and only the other acquires the observer.
        seed_whole_alias(&mut state, observer_index, source_index, source_roots);
    }
    assert!(state.join_in_place(&alias_path));
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
    let observer_state = state.binding(observer_index);
    assert!(!layout.local_mutable[observer_index]);
    assert!(
        state
            .binding_mode(observer_index)
            .contains(LocalMode::ALIAS)
    );
    assert!(observer_state.value_roots().contains(source_index));
}

fn transfer_isolated_statement(
    checker: &BorrowChecker<'_>,
    layout: &FunctionLayout,
    original_block: &HirBlock,
    statement: &HirStatement,
    state: &mut BorrowState,
) -> Result<(BlockTransferStats, ValueFactBuffer), BorrowCheckError> {
    let transfer_context = BorrowTransferContext {
        external_package_registry: checker.external_package_registry,
        public_call_summaries: &checker.public_call_summaries,
        imported_call_summaries: &checker.module.imported_call_summaries,
        module_private_call_summaries: &checker.module.module_private_call_summaries,
        generated_call_summaries: &checker.module.generated_call_summaries,
        local_types: &checker.local_types,
        structs: &checker.module.structs,
        choices: &checker.module.choices,
        expressions: &checker.module.expressions,
        type_environment: checker.type_environment,
        type_projection:
            crate::compiler_frontend::ast::generic_functions::registered_type_projection(
                checker.type_environment,
                checker.external_package_registry,
            ),
        diagnostics: BorrowDiagnostics::new(
            checker.module,
            checker.path_fork,
            checker.string_table,
        ),
        typed_call_cache: Default::default(),
        #[cfg(feature = "detailed_timers")]
        timing: None,
    };
    let isolated_block = HirBlock {
        id: original_block.id,
        region: original_block.region,
        locals: original_block.locals.clone(),
        statements: vec![statement.clone()],
        terminator: HirTerminator::RuntimeFailure {
            message: "isolated transfer test".to_owned(),
            cause: None,
            kind: RuntimeFailureKind::NoMatchFallback,
        },
    };
    let mut value_facts = ValueFactBuffer::new(layout.local_count());
    let stats = transfer_block(
        &transfer_context,
        layout,
        &isolated_block,
        state,
        &mut value_facts,
    )?;
    Ok((stats, value_facts))
}
