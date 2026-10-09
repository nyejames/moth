//! HIR function provenance association and validation tests.
//!
//! WHAT: verifies that direct synthetic-interface provenance from AST value metadata is retained
//!      as one immutable fact per local HIR function, that empty facts are explicit, that HIR
//!      validation rejects missing, extra or out-of-range coverage, and that post-convergence
//!      catch pruning rebuilds the aggregate instead of leaving pruned handler facts behind.
//! WHY: the per-function link-fact lane needs exact AST-to-HIR function association through the
//!      existing path-to-FunctionId lowering owner. These are hidden invariants that integration
//!      output cannot inspect.

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleHandling,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_builder::{
    build_ast_with_registered_types, lower_ast, lower_ast_with_metadata,
};
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::private_failure_lane::install_private_failure_lanes;
use crate::compiler_frontend::module_compilation::generated::infer_builtin_failure_summaries;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_node, make_test_variable, node,
};
use crate::compiler_frontend::tests::borrow_fixture_support::run_borrow_checker;
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast;
use crate::compiler_frontend::value_mode::ValueMode;

fn provenance_member(interface: &str, member_name: &str) -> SyntheticInterfaceMemberIdentity {
    SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        interface,
        member_name,
    )
}

fn int_with_provenance(value: i64, provenance: SyntheticInterfaceProvenance) -> Expression {
    Expression::int(value, None, ValueMode::ImmutableOwned)
        .with_synthetic_interface_provenance(provenance)
}

/// Fixture source whose only provenance lives in a handler that summary convergence proves
/// unreachable.
///
/// `calm` is declared after its callers so the protected calls stay provisional private-failure
/// candidates until HIR summary convergence resolves them as infallible; the authored handler is
/// lowered first and pruned only afterwards.
const PRUNED_HANDLER_ONLY_SOURCE: &str = "handler_only |left Int, right Int| -> Bool:\n    return calm(left) < calm(right) catch then false\n;\n\
     calm |value Int| -> Int:\n    return value\n;\n";

/// Fixture source with live protected provenance, pruned handler provenance and one unrelated
/// function fact.
const PRUNED_HANDLER_MIXED_SOURCE: &str = "mixed |left Int, right Int| -> Bool:\n    return calm(left) < calm(right) catch then true\n;\n\
     plain |value Int| -> Int:\n    return value\n;\n\
     calm |value Int| -> Int:\n    return value\n;\n";

fn function_name_matches(
    path: PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> bool {
    path_fork
        .component(path)
        .is_some_and(|component| string_table.resolve(component) == name)
}

fn function_body_by_name_mut<'a>(
    ast: &'a mut Ast,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    name: &str,
) -> &'a mut Vec<AstNode> {
    let index = ast
        .nodes
        .iter()
        .position(|node| match &node.kind {
            NodeKind::Function(path, ..) => {
                function_name_matches(*path, path_fork, string_table, name)
            }
            _ => false,
        })
        .unwrap_or_else(|| panic!("expected function '{name}' in the parsed fixture AST"));
    let NodeKind::Function(_, _, body) = &mut ast.nodes[index].kind else {
        unreachable!("function lookup only selects function nodes");
    };
    body
}

/// Attaches provenance to a parsed fixture's catch handler fallback and protected value.
///
/// WHAT: source fixtures cannot spell synthetic-interface provenance, so the fixture injects it
///       into the finalized AST value before HIR construction.
/// WHY: the regression exercises the real catch-pruning path instead of hand-editing the transient
///      construction projection.
fn assign_catch_provenance(
    ast: &mut Ast,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    function: &str,
    handler_provenance: &SyntheticInterfaceProvenance,
    protected_provenance: &SyntheticInterfaceProvenance,
) {
    let body = function_body_by_name_mut(ast, path_fork, string_table, function);
    let Some(AstNode {
        kind: NodeKind::Return(expressions),
        ..
    }) = body.first_mut()
    else {
        panic!("fixture function '{function}' must return its protected catch");
    };
    let Some(expression) = expressions.first_mut() else {
        panic!("fixture function '{function}' must return one value");
    };
    let ExpressionKind::ValueBlock { block } = &mut expression.kind else {
        panic!("fixture function '{function}' must return a catch value block");
    };
    let ValueBlock::Catch(catch) = block.as_mut() else {
        panic!("fixture function '{function}' must return a catch");
    };
    let FallibleHandling::Handler {
        body: handler_body, ..
    } = &mut catch.handler
    else {
        panic!("fixture function '{function}' must own a catch handler");
    };
    let Some(AstNode {
        kind: NodeKind::ThenValue(fallback),
        ..
    }) = handler_body.first_mut()
    else {
        panic!("fixture catch handler must produce a then value");
    };
    let Some(handler_value) = fallback.expressions.first_mut() else {
        panic!("fixture catch handler must produce one fallback value");
    };
    handler_value.synthetic_interface_provenance = handler_provenance.clone();
    catch.handled_value.synthetic_interface_provenance = protected_provenance.clone();
}

/// Attaches provenance to a parsed fixture function's returned value.
fn assign_return_provenance(
    ast: &mut Ast,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    function: &str,
    provenance: &SyntheticInterfaceProvenance,
) {
    let body = function_body_by_name_mut(ast, path_fork, string_table, function);
    let Some(AstNode {
        kind: NodeKind::Return(expressions),
        ..
    }) = body.first_mut()
    else {
        panic!("fixture function '{function}' must return one value");
    };
    let Some(expression) = expressions.first_mut() else {
        panic!("fixture function '{function}' must return one value");
    };
    expression.synthetic_interface_provenance = provenance.clone();
}

/// Lowers a parsed fixture and runs the pre-installation pipeline: borrow validation followed by
/// private builtin-failure summary convergence.
///
/// WHAT: destructures the canonical `HirLoweringResult` so lowering owns and extends one type
///       environment through the whole pre-install pipeline.
/// WHY: the unreachable-handler regression must start from the same pre-install state the
///      production lane installer consumes; the caller installs so it can assert around pruning.
fn lower_with_converged_summaries(
    ast: Ast,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> (HirModule, BorrowCheckReport, TypeEnvironment) {
    let lowered =
        lower_ast_with_metadata(ast, string_table, path_fork).expect("HIR lowering should succeed");
    let hir = lowered.hir_module;
    let type_environment = lowered.type_environment;
    let mut report = run_borrow_checker(
        &hir,
        &default_external_package_registry(string_table),
        path_fork,
        string_table,
    )
    .expect("catch-pruning fixtures must pass borrow checking");
    infer_builtin_failure_summaries(&hir, &mut report)
        .expect("catch-pruning fixture summaries must converge");
    (hir, report, type_environment)
}

fn function_id_by_name(
    module: &HirModule,
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
                .is_some_and(|path| function_name_matches(path, path_fork, string_table, name))
        })
        .unwrap_or_else(|| panic!("expected lowered function '{name}'"))
        .id
}

fn handler_block_for(module: &HirModule, function: FunctionId) -> BlockId {
    let record = module
        .catch_handlers
        .iter()
        .find(|record| record.owner == function)
        .unwrap_or_else(|| panic!("expected one lowered catch handler for {function:?}"));
    record.handler.block
}

#[test]
fn retains_direct_provenance_per_function() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);
    let helper_name = path_fork
        .try_intern_child(entry_path, string_table.intern("helper"))
        .expect("test path fits");
    let var_name = super::symbol("result", &mut path_fork, &mut string_table);

    let member_a = provenance_member("render", "html");
    let member_b = provenance_member("render", "wasm");
    let injected =
        SyntheticInterfaceProvenance::from_members(vec![member_a.clone(), member_b.clone()]);

    let start_body = vec![node(NodeKind::Return(vec![]), None)];
    let helper_body = vec![
        node(
            NodeKind::VariableDeclaration(make_test_variable(
                var_name,
                int_with_provenance(42, injected),
            )),
            None,
        ),
        node(NodeKind::Return(vec![]), None),
    ];

    let ast = build_ast_with_registered_types(
        vec![
            function_node(
                start_name,
                FunctionSignature {
                    parameters: vec![],
                    returns: vec![],
                },
                start_body,
                None,
            ),
            function_node(
                helper_name,
                FunctionSignature {
                    parameters: vec![],
                    returns: vec![],
                },
                helper_body,
                None,
            ),
        ],
        entry_path,
    );

    let (module, _type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("HIR lowering should succeed");

    // Every function has exactly one provenance fact.
    assert_eq!(module.function_provenance.len(), module.functions.len());

    // The start function has an explicit empty (portable) fact.
    let start_provenance = module
        .function_provenance
        .get(
            &module
                .start_function
                .expect("normal test module should have start"),
        )
        .expect("start function should have a provenance fact");
    assert!(start_provenance.is_empty());

    // The helper function carries the injected member-granular dependencies.
    let helper_id = module
        .functions
        .iter()
        .find(|function| {
            module
                .side_table
                .function_name_path(function.id)
                .is_some_and(|path| path == helper_name)
        })
        .map(|function| function.id)
        .expect("helper function should be present");

    let helper_provenance = module
        .function_provenance
        .get(&helper_id)
        .expect("helper function should have a provenance fact");
    assert_eq!(
        helper_provenance.members(),
        &[
            provenance_member("render", "html"),
            provenance_member("render", "wasm"),
        ]
    );
}

#[test]
fn empty_function_has_explicit_empty_provenance() {
    let mut path_fork = super::PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let start_body = vec![node(NodeKind::Return(vec![]), None)];

    let ast = build_ast_with_registered_types(
        vec![function_node(
            start_name,
            FunctionSignature {
                parameters: vec![],
                returns: vec![],
            },
            start_body,
            None,
        )],
        entry_path,
    );

    let (module, _type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("HIR lowering should succeed");

    assert_eq!(module.function_provenance.len(), module.functions.len());
    let provenance = module
        .function_provenance
        .get(
            &module
                .start_function
                .expect("normal test module should have start"),
        )
        .expect("start function should have a provenance fact");
    assert!(provenance.is_empty());
}

#[test]
fn validation_rejects_missing_provenance_coverage() {
    let mut path_fork = super::PathInternerFork::empty();
    use crate::compiler_frontend::hir::validation::validate_hir_module;

    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let ast = build_ast_with_registered_types(
        vec![function_node(
            start_name,
            FunctionSignature {
                parameters: vec![],
                returns: vec![],
            },
            vec![node(NodeKind::Return(vec![]), None)],
            None,
        )],
        entry_path,
    );

    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("HIR lowering should succeed");

    // Remove the provenance fact to simulate missing coverage.
    module.function_provenance.clear();

    let result = validate_hir_module(&module, &type_environment);
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(
        error.msg.contains("function_provenance"),
        "error should mention function_provenance, got: {}",
        error.msg
    );
}

#[test]
fn validation_rejects_extra_provenance_entry() {
    let mut path_fork = super::PathInternerFork::empty();
    use crate::compiler_frontend::hir::ids::FunctionId;
    use crate::compiler_frontend::hir::validation::validate_hir_module;

    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let ast = build_ast_with_registered_types(
        vec![function_node(
            start_name,
            FunctionSignature {
                parameters: vec![],
                returns: vec![],
            },
            vec![node(NodeKind::Return(vec![]), None)],
            None,
        )],
        entry_path,
    );

    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("HIR lowering should succeed");

    // Add an extra provenance entry for a non-existent function.
    module
        .function_provenance
        .insert(FunctionId(999), SyntheticInterfaceProvenance::empty());

    let result = validate_hir_module(&module, &type_environment);
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(
        error.msg.contains("function_provenance"),
        "error should mention function_provenance, got: {}",
        error.msg
    );
}

#[test]
fn validation_rejects_replaced_out_of_range_provenance_key() {
    let mut path_fork = super::PathInternerFork::empty();
    use crate::compiler_frontend::hir::ids::FunctionId;
    use crate::compiler_frontend::hir::validation::validate_hir_module;

    let mut string_table = StringTable::new();
    let (entry_path, start_name) =
        super::entry_path_and_start_name(&mut path_fork, &mut string_table);

    let ast = build_ast_with_registered_types(
        vec![function_node(
            start_name,
            FunctionSignature {
                parameters: vec![],
                returns: vec![],
            },
            vec![node(NodeKind::Return(vec![]), None)],
            None,
        )],
        entry_path,
    );

    let (mut module, type_environment) =
        lower_ast(ast, &mut string_table, &mut path_fork).expect("HIR lowering should succeed");

    let start_function = module
        .start_function
        .expect("normal test module should have start");
    module.function_provenance.remove(&start_function);
    module
        .function_provenance
        .insert(FunctionId(999), SyntheticInterfaceProvenance::empty());

    let result = validate_hir_module(&module, &type_environment);
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert!(
        error.msg.contains("function_provenance"),
        "error should mention function_provenance, got: {}",
        error.msg
    );
}

#[test]
fn finalized_provenance_is_empty_when_pruning_removes_every_handler_row() {
    let (mut ast, mut path_fork, mut string_table) =
        parse_single_file_ast(PRUNED_HANDLER_ONLY_SOURCE);
    let handler_provenance =
        SyntheticInterfaceProvenance::single(provenance_member("source-config", "pruned-handler"));
    assign_catch_provenance(
        &mut ast,
        &path_fork,
        &string_table,
        "handler_only",
        &handler_provenance,
        &SyntheticInterfaceProvenance::empty(),
    );

    let (mut hir, report, mut type_environment) =
        lower_with_converged_summaries(ast, &mut string_table, &mut path_fork);
    let handler_only = function_id_by_name(&hir, &path_fork, &string_table, "handler_only");
    let handler_block = handler_block_for(&hir, handler_only);

    // The fixture must really carry handler provenance in the lowered handler block's row.
    assert_eq!(
        hir.function_provenance_by_block
            .get(&(handler_only, Some(handler_block))),
        Some(&handler_provenance),
    );
    assert_eq!(
        hir.function_provenance.get(&handler_only),
        Some(&handler_provenance),
    );
    let blocks_before_pruning = hir.blocks.len();

    install_private_failure_lanes(&mut hir, &report, &mut type_environment)
        .expect("infallible protected calls must prune their unreachable handler");

    // Pruning removed the handler subgraph, so the only construction row is gone: the aggregate
    // must settle to the explicit empty fact instead of keeping the stale handler provenance.
    assert!(hir.blocks.len() < blocks_before_pruning);
    assert_eq!(
        hir.function_provenance.get(&handler_only),
        Some(&SyntheticInterfaceProvenance::empty()),
    );
    assert_eq!(hir.function_provenance.len(), hir.functions.len());
    assert!(hir.function_provenance_by_block.is_empty());
    assert_eq!(hir.function_provenance_by_block.capacity(), 0);
}

#[test]
fn finalized_provenance_keeps_live_rows_and_drops_pruned_handler_rows() {
    let (mut ast, mut path_fork, mut string_table) =
        parse_single_file_ast(PRUNED_HANDLER_MIXED_SOURCE);
    let live_provenance =
        SyntheticInterfaceProvenance::single(provenance_member("source-config", "live-protected"));
    let handler_provenance =
        SyntheticInterfaceProvenance::single(provenance_member("source-config", "pruned-handler"));
    let unrelated_provenance =
        SyntheticInterfaceProvenance::single(provenance_member("source-config", "unrelated"));
    assign_catch_provenance(
        &mut ast,
        &path_fork,
        &string_table,
        "mixed",
        &handler_provenance,
        &live_provenance,
    );
    assign_return_provenance(
        &mut ast,
        &path_fork,
        &string_table,
        "plain",
        &unrelated_provenance,
    );

    let (mut hir, report, mut type_environment) =
        lower_with_converged_summaries(ast, &mut string_table, &mut path_fork);
    let mixed = function_id_by_name(&hir, &path_fork, &string_table, "mixed");
    let plain = function_id_by_name(&hir, &path_fork, &string_table, "plain");
    let handler_block = handler_block_for(&hir, mixed);

    assert_eq!(
        hir.function_provenance_by_block
            .get(&(mixed, Some(handler_block))),
        Some(&handler_provenance),
    );
    assert_eq!(
        hir.function_provenance.get(&mixed),
        Some(&live_provenance.union(&handler_provenance)),
    );
    assert_eq!(
        hir.function_provenance.get(&plain),
        Some(&unrelated_provenance),
    );
    let blocks_before_pruning = hir.blocks.len();

    install_private_failure_lanes(&mut hir, &report, &mut type_environment)
        .expect("infallible protected calls must prune their unreachable handler");

    // Only the pruned handler row disappears; live protected and unrelated facts survive the
    // rebuild from the surviving construction rows.
    assert!(hir.blocks.len() < blocks_before_pruning);
    assert_eq!(hir.function_provenance.get(&mixed), Some(&live_provenance));
    assert_eq!(
        hir.function_provenance.get(&plain),
        Some(&unrelated_provenance),
    );
    assert_eq!(hir.function_provenance.len(), hir.functions.len());
    assert!(hir.function_provenance_by_block.is_empty());
    assert_eq!(hir.function_provenance_by_block.capacity(), 0);
}

#[test]
fn finalized_provenance_preserves_direct_construction_without_block_projection() {
    let mut module = HirModule::new();
    let function_id = FunctionId(0);
    module.functions.push(HirFunction {
        id: function_id,
        entry: BlockId(0),
        params: Vec::new(),
        return_type: TypeId(0),
    });
    let direct_provenance =
        SyntheticInterfaceProvenance::single(provenance_member("source-config", "direct"));
    module
        .function_provenance
        .insert(function_id, direct_provenance.clone());

    // Direct construction never used the lowering projection, so finalization has no aggregate to
    // rebuild and must keep the directly assigned fact authoritative.
    assert!(
        !module
            .finalize_function_provenance_after_rewrites()
            .expect("direct construction finalization must succeed")
    );
    assert_eq!(
        module.function_provenance.get(&function_id),
        Some(&direct_provenance),
    );
    assert!(module.function_provenance_by_block.is_empty());
}
