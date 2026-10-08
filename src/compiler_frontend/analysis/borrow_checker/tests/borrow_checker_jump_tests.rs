//! Focused tests for explicit HIR jump destination transfer.

use crate::compiler_frontend::analysis::borrow_checker::engine::BorrowChecker;
use crate::compiler_frontend::analysis::borrow_checker::state::{BorrowState, LocalState, RootSet};
use crate::compiler_frontend::analysis::borrow_checker::types::LocalMode;
use crate::compiler_frontend::hir::terminators::{HirJumpArgument, HirTerminator};
use crate::compiler_frontend::hir::validate_hir_module;
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::tests::type_id_fixture_support::lower_ast;

#[test]
fn jump_defines_explicit_reordered_destinations_as_value_backed_slots() {
    let source = r#"probe || -> Int:
source_a ~= 1
source_b ~= 2
root_a ~= 3
root_b ~= 4
slot_source ~= 5
slot_destination ~= 6
return source_a
;"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("fixture source should lower");
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
    let function_id = function.id;
    let entry = function.entry;
    let local_named = |name| {
        module
            .blocks
            .iter()
            .flat_map(|block| &block.locals)
            .find(|local| {
                module
                    .side_table
                    .resolve_local_name(local.id, &path_fork, &string_table)
                    == Some(name)
            })
            .map(|local| local.id)
            .unwrap_or_else(|| panic!("fixture local {name} should lower"))
    };
    let source_a = local_named("source_a");
    let source_b = local_named("source_b");
    let root_a = local_named("root_a");
    let root_b = local_named("root_b");
    let slot_source = local_named("slot_source");
    let slot_destination = local_named("slot_destination");

    // These are ordinary locals declared in the function body, not ABI parameters. Put the
    // destinations after unrelated locals so the test also rejects prefix-based reconstruction.
    let explicit_order = [
        root_a,
        root_b,
        source_b,
        source_a,
        slot_source,
        slot_destination,
    ];
    let entry_block = &mut module.blocks[entry.0 as usize];
    entry_block.locals.sort_by_key(|local| {
        explicit_order
            .iter()
            .position(|ordered| *ordered == local.id)
            .unwrap_or(explicit_order.len() + local.id.0 as usize)
    });
    for destination in [source_a, source_b, slot_destination] {
        let position = entry_block
            .locals
            .iter()
            .position(|local| local.id == destination)
            .expect("jump destination should belong to the target block");
        assert!(
            position >= 2,
            "destination should be outside the local prefix"
        );
    }
    entry_block.terminator = HirTerminator::Jump {
        target: entry,
        args: vec![
            HirJumpArgument {
                source: source_a,
                destination: source_b,
            },
            HirJumpArgument {
                source: source_b,
                destination: source_a,
            },
            HirJumpArgument {
                source: slot_source,
                destination: slot_destination,
            },
        ],
    };
    validate_hir_module(&module, &type_environment)
        .expect("ordinary locals and explicit self-edge definitions should form valid HIR");

    let checker = BorrowChecker::new(&module, &registry, &path_fork, &string_table);
    let function = module
        .functions
        .iter()
        .find(|function| function.id == function_id)
        .expect("fixture function should remain present");
    let reachable = checker
        .collect_reachable_blocks(function)
        .expect("fixture CFG should be reachable");
    let layout = checker
        .build_function_layout(function, &reachable)
        .expect("fixture local layout should build");
    let source_a_index = layout
        .index_of(source_a)
        .expect("source A is in the layout");
    let source_b_index = layout
        .index_of(source_b)
        .expect("source B is in the layout");
    let root_a_index = layout.index_of(root_a).expect("root A is in the layout");
    let root_b_index = layout.index_of(root_b).expect("root B is in the layout");
    let slot_source_index = layout
        .index_of(slot_source)
        .expect("slot source is in the layout");
    let slot_destination_index = layout
        .index_of(slot_destination)
        .expect("slot destination is in the layout");

    let mut state = BorrowState::new_uninitialized(layout.local_count());
    let mut source_a_roots = RootSet::empty(layout.local_count());
    source_a_roots.insert(root_a_index);
    state.update_local_state(
        source_a_index,
        LocalState::alias_with_direct(source_a_roots.clone(), source_a_roots),
    );
    let mut source_b_roots = RootSet::empty(layout.local_count());
    source_b_roots.insert(root_b_index);
    state.update_local_state(
        source_b_index,
        LocalState::alias_with_direct(source_b_roots.clone(), source_b_roots),
    );
    state.update_local_state(slot_source_index, LocalState::slot(layout.local_count()));
    let mut stale_destination_roots = RootSet::empty(layout.local_count());
    stale_destination_roots.insert(root_a_index);
    state.update_local_state(
        slot_destination_index,
        LocalState::alias_with_direct(stale_destination_roots.clone(), stale_destination_roots),
    );

    let jump = module.blocks[entry.0 as usize].terminator.clone();
    checker
        .apply_jump_argument_transfer(function_id, &layout, &jump, entry, &mut state)
        .expect("explicit local destinations should transfer");

    let destination_a_state = state.local_state(source_a_index);
    assert!(destination_a_state.mode.contains(LocalMode::SLOT));
    assert!(!destination_a_state.mode.contains(LocalMode::ALIAS));
    assert!(destination_a_state.value_roots.contains(root_b_index));
    assert!(
        destination_a_state
            .direct_alias_roots
            .contains(root_b_index)
    );
    assert!(!destination_a_state.value_roots.contains(source_a_index));

    let destination_b_state = state.local_state(source_b_index);
    assert!(destination_b_state.mode.contains(LocalMode::SLOT));
    assert!(!destination_b_state.mode.contains(LocalMode::ALIAS));
    assert!(destination_b_state.value_roots.contains(root_a_index));
    assert!(
        destination_b_state
            .direct_alias_roots
            .contains(root_a_index)
    );
    assert!(!destination_b_state.value_roots.contains(source_b_index));

    let slot_result = state.local_state(slot_destination_index);
    assert!(slot_result.mode.contains(LocalMode::SLOT));
    assert!(!slot_result.mode.contains(LocalMode::ALIAS));
    assert!(slot_result.value_roots.contains(slot_source_index));
    assert!(!slot_result.value_roots.contains(root_a_index));
}
