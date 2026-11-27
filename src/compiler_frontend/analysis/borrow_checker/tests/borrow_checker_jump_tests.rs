//! Focused tests for explicit HIR jump destination transfer.

use crate::compiler_frontend::analysis::borrow_checker::engine::BorrowChecker;
use crate::compiler_frontend::analysis::borrow_checker::state::{
    BindingSpec, BorrowState, CellId, FieldKey, RootSet, StoredValue, WrapperId,
};
use crate::compiler_frontend::analysis::borrow_checker::types::LocalMode;
use crate::compiler_frontend::hir::terminators::{HirJumpArgument, HirTerminator};
use crate::compiler_frontend::hir::validate_hir_module;
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::tests::type_id_fixture_support::lower_ast;

#[test]
fn jump_defines_explicit_reordered_destinations_as_value_backed_slots() {
    let source = r#"probe || -> I32:
source_a ~= 1
source_b ~= 2
root_a ~= 3
root_b ~= 4
slot_source ~= 5
slot_destination ~= 6
return source_a
;"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let registry = default_external_package_registry(&mut string_table);
    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork, &registry)
            .expect("fixture source should lower");
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
    validate_hir_module(&module, &type_environment, &registry)
        .expect("ordinary locals and explicit self-edge definitions should form valid HIR");

    let checker = BorrowChecker::new(
        &module,
        &type_environment,
        &registry,
        &path_fork,
        &string_table,
    );
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
    let count = layout.local_count();
    state.initialize_parameter(root_a_index);
    state.initialize_parameter(root_b_index);
    for (alias, source) in [
        (source_a_index, root_a_index),
        (source_b_index, root_b_index),
        (slot_destination_index, root_a_index),
    ] {
        let roots = state.effective_roots(source);
        state.set_binding(
            alias,
            BindingSpec::Alias {
                addresses: state.resolve_cells(source),
                route_roots: roots.clone(),
            },
            roots,
            RootSet::empty(count),
        );
    }
    state.initialize_parameter(slot_source_index);

    let jump = module.blocks[entry.0 as usize].terminator.clone();
    checker
        .apply_jump_argument_transfer(function_id, &layout, &jump, entry, 0, &mut state)
        .expect("explicit local destinations should transfer");

    for (destination, expected_root, excluded_root) in [
        (source_a_index, root_b_index, source_a_index),
        (source_b_index, root_a_index, source_b_index),
        (slot_destination_index, slot_source_index, root_a_index),
    ] {
        let mode = state.binding_mode(destination);
        let value_roots = state.binding(destination).value_roots();
        assert!(mode.contains(LocalMode::SLOT));
        assert!(!mode.contains(LocalMode::ALIAS));
        assert!(value_roots.contains(expected_root));
        assert!(!value_roots.contains(excluded_root));
        assert_eq!(
            state.resolve_cells(destination).cells(),
            &[CellId::Slot(WrapperId::current(destination))]
        );
    }

    // Exercise the same parallel jump when both destinations own live, observed graphs.
    // Each source transports a projection whose root is retired by the opposite argument.
    let mut state = BorrowState::new_uninitialized(layout.local_count());
    state.initialize_parameter(source_a_index);
    state.initialize_parameter(source_b_index);
    for (observer, source) in [
        (root_a_index, source_a_index),
        (root_b_index, source_b_index),
    ] {
        let roots = state.effective_roots(source);
        state.set_binding(observer, BindingSpec::Slot, roots, RootSet::empty(count));
    }
    // The third argument carries the same old root as observer A, without an override.
    let roots = state.binding(root_a_index).value_roots().clone();
    state.set_binding(
        slot_source_index,
        BindingSpec::Slot,
        roots,
        RootSet::empty(count),
    );
    let old_a = state.local_alloc_ids(source_a_index).0[0];
    let old_b = state.local_alloc_ids(source_b_index).0[0];
    let field = FieldKey::Field(0);
    let child_a = state.alloc_child(old_a, field);
    let child_b = state.alloc_child(old_b, field);
    let marker_a = state.site_id(0, entry, &layout.block_successors);
    let marker_b = state.site_id(1, entry, &layout.block_successors);
    let payload = FieldKey::Field(1);
    for (child, marker) in [(child_a, marker_a), (child_b, marker_b)] {
        state.add_alloc_edge(
            child,
            payload,
            StoredValue {
                id: marker,
                writable: true,
            },
        );
    }
    state.bind_alloc_ids(source_a_index, &[child_a], true);
    state.bind_alloc_ids(source_b_index, &[child_b], false);
    state.bind_detached_holders(source_a_index, [old_b]);
    state.bind_detached_holders(source_b_index, [old_a]);

    checker
        .apply_jump_argument_transfer(function_id, &layout, &jump, entry, 1, &mut state)
        .expect("the parallel jump should transport pre-edge allocations");

    let (observed_a, observed_b) = (
        state.local_alloc_ids(root_a_index).0,
        state.local_alloc_ids(root_b_index).0,
    );
    assert_ne!(observed_a, [old_a]);
    assert_ne!(observed_b, [old_b]);
    assert_eq!(state.local_alloc_ids(slot_destination_index).0, observed_a);
    for (destination, observer, marker, readonly, detached) in [
        (source_b_index, root_a_index, marker_a, true, &observed_b),
        (source_a_index, root_b_index, marker_b, false, &observed_a),
    ] {
        let transported = state.resolve_alloc_path(destination, &layout.local_mutable, [payload]);
        let observed = state.resolve_alloc_path(observer, &layout.local_mutable, [field, payload]);
        assert_eq!(transported.ids, [marker]);
        assert_eq!(transported.ids, observed.ids);
        assert_eq!(transported.readonly, readonly);
        assert!(state.detached_from(destination, detached));
    }
}
