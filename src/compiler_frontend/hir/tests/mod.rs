//! HIR lowering test modules and shared harness utilities.
//!
//! WHAT: groups the HIR test suites and exposes common naming and relationship helpers for them.
//! WHY: HIR tests should discover one another through a single module entry.

use crate::compiler_frontend::hir::expressions::HirExpression;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, LocalId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;

mod checked_numeric_lowering_tests;
mod float_formatting_lowering_tests;
mod hir_branch_lowering_tests;
mod hir_const_facts_tests;
mod hir_display_tests;
mod hir_expression_lowering_tests;
mod hir_function_origin_tests;
mod hir_function_provenance_tests;
mod hir_local_lowering_tests;
mod hir_loop_lowering_tests;
mod hir_match_lowering_tests;
mod hir_module_lowering_tests;
mod hir_reactivity_tests;
mod hir_result_lowering_tests;
mod hir_scoped_block_lowering_tests;
mod hir_validation_tests;
mod loop_lowering_tests;
mod reachability_tests;
mod value_block_lowering_tests;

pub(super) fn symbol(name: &str, string_table: &mut StringTable) -> InternedPath {
    InternedPath::from_single_str(name, string_table)
}

pub(super) fn entry_path_and_start_name(
    string_table: &mut StringTable,
) -> (InternedPath, InternedPath) {
    let entry_path = InternedPath::from_single_str("main.moth", string_table);
    let start_name = entry_path.join_str(IMPLICIT_START_FUNC_NAME, string_table);
    (entry_path, start_name)
}

pub(super) fn start_function(module: &HirModule) -> &HirFunction {
    module
        .functions
        .iter()
        .find(|function| Some(function.id) == module.start_function)
        .expect("start function should exist")
}

pub(super) fn assert_branches_join_same_merge_block(
    module: &HirModule,
    left_block: BlockId,
    right_block: BlockId,
) -> BlockId {
    let left_target = match module.blocks[left_block.0 as usize].terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("left branch should jump to the merge block"),
    };
    let right_target = match module.blocks[right_block.0 as usize].terminator {
        HirTerminator::Jump { target, .. } => target,
        _ => panic!("right branch should jump to the merge block"),
    };

    assert_eq!(
        left_target, right_target,
        "branches should rejoin at one merge block"
    );

    left_target
}

pub(super) fn assert_block_has_jump_args(
    module: &HirModule,
    block_id: BlockId,
    expected_len: usize,
) -> &[LocalId] {
    let HirTerminator::Jump { args, .. } = &module.blocks[block_id.0 as usize].terminator else {
        panic!("block should jump with merge arguments");
    };

    assert_eq!(
        args.len(),
        expected_len,
        "block should pass the expected number of merge arguments"
    );

    args.as_slice()
}

pub(super) fn assert_block_assigns_local(
    module: &HirModule,
    block_id: BlockId,
    local: LocalId,
) -> &HirExpression {
    module.blocks[block_id.0 as usize]
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Assign {
                target: HirPlace::Local(target),
                value,
            } if *target == local => Some(value),
            _ => None,
        })
        .expect("block should assign the expected local")
}
