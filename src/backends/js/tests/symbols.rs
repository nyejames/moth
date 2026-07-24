//! Deterministic JavaScript symbol naming tests.

use super::support::*;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::terminators::HirTerminator;

// Identifier sanitisation tests [names]
// ---------------------------------------------------------------------------

/// Verifies that a function whose name is a JS reserved word gets an underscore prefix. [names]
#[test]
fn reserved_word_function_name_gets_underscore_prefix() {
    // `for` is a reserved JS identifier; the emitter must prefix it to avoid a syntax error.
    let source = lower_minimal_module("for");
    let expected_name = expected_dev_function_name("for", 0);

    assert!(
        source.contains(&format!("function {expected_name}(")),
        "reserved identifiers should still lower to deterministic id-based names"
    );
}

/// Verifies that invalid JS identifier characters are replaced with underscores. [names]
#[test]
fn invalid_identifier_chars_are_replaced_with_underscore() {
    // Hyphens are not valid in JS identifiers; they must be replaced.
    let source = lower_minimal_module("foo-bar");
    let expected_name = expected_dev_function_name("foo-bar", 0);

    assert!(
        source.contains(&format!("function {expected_name}(")),
        "hyphens in identifiers must be replaced with underscores"
    );
}

// ---------------------------------------------------------------------------

// Function name map test [names]
// ---------------------------------------------------------------------------

/// Verifies that function_name_by_id exposes stable JS names for runtime-fragment lookup. [names]
#[test]
fn exposes_function_name_map_for_runtime_fragments() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let block0 = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(unit_expression(0, types.unit, RegionId(0))),
    };

    let block1 = HirBlock {
        id: BlockId(1),
        region: RegionId(0),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(unit_expression(1, types.unit, RegionId(0))),
    };

    let mut module = HirModule::new();
    module.blocks = vec![block0, block1];
    module.start_function = FunctionId(0);
    module.functions = vec![
        HirFunction {
            id: FunctionId(0),
            entry: BlockId(0),
            params: vec![],
            return_type: types.unit,
            return_aliases: vec![],
        },
        HirFunction {
            id: FunctionId(1),
            entry: BlockId(1),
            params: vec![],
            return_type: types.unit,
            return_aliases: vec![],
        },
    ];
    module.regions = vec![HirRegion::lexical(RegionId(0), None)];
    module.side_table.bind_function_name(
        FunctionId(0),
        InternedPath::from_single_str("start", &mut string_table),
    );
    module.side_table.bind_function_name(
        FunctionId(1),
        InternedPath::from_single_str("__moth_frag_0", &mut string_table),
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");
    let expected_start = expected_dev_function_name("start", 0);
    let expected_fragment = expected_dev_function_name("__moth_frag_0", 1);

    assert_eq!(
        output
            .function_name_by_id
            .get(&FunctionId(0))
            .map(String::as_str),
        Some(expected_start.as_str())
    );
    assert_eq!(
        output
            .function_name_by_id
            .get(&FunctionId(1))
            .map(String::as_str),
        Some(expected_fragment.as_str())
    );
}

// ---------------------------------------------------------------------------
