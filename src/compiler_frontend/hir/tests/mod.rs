//! HIR lowering test modules and shared harness utilities.
//!
//! WHAT: groups the HIR test suites and exposes common naming and relationship helpers for them.
//! WHY: HIR tests should discover one another through a single module entry.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::expression_store::HirProjection;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_builder::{build_ast_with_registered_types, lower_ast};
use crate::compiler_frontend::hir::ids::{BlockId, HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::statements::{HirStatementKind, HirWriteTarget};
use crate::compiler_frontend::hir::terminators::{HirJumpArgument, HirTerminator};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
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
mod hir_lexical_scope_lowering_tests;
mod hir_local_lowering_tests;
mod hir_loop_lowering_tests;
mod hir_match_lowering_tests;
mod hir_module_lowering_tests;
mod hir_result_lowering_tests;
mod hir_validation_tests;
mod loop_lowering_tests;
mod native_channel_validation_tests;
mod private_failure_lane_tests;
mod reachability_tests;
mod value_block_lowering_tests;

pub(super) fn symbol(
    name: &str,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> PathId {
    path_fork
        .try_intern_portable_path(name, string_table)
        .expect("test path fits")
}

pub(super) fn entry_path_and_start_name(
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> (PathId, PathId) {
    let entry_path = symbol("main.moth", path_fork, string_table);
    let start_name = path_fork
        .try_intern_child(entry_path, string_table.intern(IMPLICIT_START_FUNC_NAME))
        .expect("test path fits");
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
) -> &[HirJumpArgument] {
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

pub(super) fn assert_block_writes_local(
    module: &HirModule,
    block_id: BlockId,
    local: LocalId,
) -> &HirExpression {
    module.blocks[block_id.0 as usize]
        .statements
        .iter()
        .find_map(|statement| match &statement.kind {
            HirStatementKind::Write { target, value }
                if write_target_local(*target) == Some(local) =>
            {
                Some(module.expressions.expression(*value))
            }
            _ => None,
        })
        .expect("block should write the expected local")
}

pub(super) fn write_value<'a>(
    module: &'a HirModule,
    statement: &crate::compiler_frontend::hir::statements::HirStatement,
) -> Option<&'a HirExpression> {
    match &statement.kind {
        HirStatementKind::Write { value, .. } => Some(module.expressions.expression(*value)),
        _ => None,
    }
}

pub(super) fn write_target_local(target: HirWriteTarget) -> Option<LocalId> {
    match target {
        HirWriteTarget::DefineLocal(local) => Some(local),
        HirWriteTarget::AssignPlace(place) if is_local_place(&place) => Some(place.root),
        HirWriteTarget::AssignPlace(_) => None,
    }
}

pub(super) fn is_local_place(place: &HirPlace) -> bool {
    place.projections.is_empty()
}

pub(super) fn place_outermost_projection_is_index(module: &HirModule, place: &HirPlace) -> bool {
    module
        .expressions
        .projections(place.projections)
        .last()
        .is_some_and(|projection| matches!(projection, HirProjection::Index(_)))
}

fn node(kind: NodeKind, span: Option<SourceSpan>) -> AstNode {
    AstNode {
        kind,
        span,
        scope: PathId::ROOT,
    }
}

fn function_node(
    name: PathId,
    signature: FunctionSignature,
    body: Vec<AstNode>,
    span: Option<SourceSpan>,
) -> AstNode {
    node(NodeKind::Function(name, signature, body), span)
}

/// Lowers a module whose only item is the implicit start function.
///
/// WHAT: returns the string table, the lowered module, and its type environment so a test can append its own rows.
/// WHY: normal-entry validation tests and the same-instance walk tests share this fixture, so both start from an identical lowering path.
pub(super) fn minimal_lowered_hir_module() -> (StringTable, HirModule, TypeEnvironment) {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) = entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_fn = function_node(
        start_name,
        FunctionSignature {
            parameters: vec![],
            returns: vec![],
        },
        vec![node(NodeKind::Return(vec![]), None)],
        None,
    );

    let ast = build_ast_with_registered_types(
        vec![start_fn],
        entry_path,
        AstExpressionBuilder::new().unwrap(),
        ConstValueStore::default(),
        TypeEnvironment::new(),
    );
    let (module, type_environment) = lower_ast(
        ast,
        &mut string_table,
        &mut path_fork,
        &ExternalPackageRegistry::new(),
    )
    .expect("lowering should succeed");

    (string_table, module, type_environment)
}

/// Returns the entry block index of the start function in a lowered test module.
pub(super) fn start_entry_block_index(module: &HirModule) -> usize {
    module.functions[module
        .start_function
        .expect("normal test module should have start")
        .0 as usize]
        .entry
        .0 as usize
}

/// Appends one expression row to the module's expression store.
pub(super) fn append_hir_expression(
    module: &mut HirModule,
    kind: HirExpressionKind,
    type_id: TypeId,
    region: RegionId,
    value_kind: ValueKind,
) -> HirValueId {
    crate::compiler_frontend::tests::hir_fixture_support::expression(
        kind,
        type_id,
        region,
        value_kind,
        &mut module.expressions,
    )
}
