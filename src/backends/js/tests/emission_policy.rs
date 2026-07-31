//! Tests for selecting the HIR function set emitted by the JavaScript backend.
//!
//! WHAT: pins the difference between direct JS lowering and HTML page-bundle lowering.
//! WHY: HTML bundles must not lower unreachable source-backed package wrappers because lowering external
//! calls is what requests generated glue and runtime assets.

use super::support::*;
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, RegionId};
use crate::compiler_frontend::hir::reachability::{
    collect_module_function_link_facts, collect_reachability_from_function_link_facts,
};
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;

#[test]
fn all_functions_is_default_for_direct_js_lowering() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let module = module_with_unreachable_function(&mut string_table, types.unit);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("direct JS lowering should emit all functions");

    let start_name = expected_dev_function_name("start_main", 0);
    let unused_name = expected_dev_function_name("unused_helper", 1);
    assert!(output.source.contains(&format!("function {start_name}(")));
    assert!(
        output.source.contains(&format!("function {unused_name}(")),
        "direct JS lowering should emit every function by default"
    );
}

#[test]
fn selected_functions_skip_unselected_functions_and_external_references() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let external_function = ExternalFunctionId::Synthetic(77);
    let module =
        module_with_unreachable_external_call(&mut string_table, types.unit, external_function);

    let mut config = default_config();
    let facts = collect_module_function_link_facts(&module)
        .expect("test HIR should produce function link facts");
    let reachability = collect_reachability_from_function_link_facts(
        &facts,
        &[module
            .start_function
            .expect("normal test module should have start")],
    )
    .expect("test HIR should produce entry reachability");
    config.function_emission_policy =
        JsFunctionEmissionPolicy::Selected(reachability.backend_selection().clone());

    let borrow_analysis = BorrowCheckReport::default();
    let mut emitter = crate::backends::js::JsEmitter::new(
        &module,
        &borrow_analysis,
        &string_table,
        config.clone(),
        &type_environment,
    );
    emitter
        .build_symbol_maps()
        .expect("test HIR should build symbol maps");
    assert!(emitter.function_name_by_id.contains_key(&FunctionId(0)));
    assert!(!emitter.function_name_by_id.contains_key(&FunctionId(1)));

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        config,
        &type_environment,
    )
    .expect("selected-only JS lowering should ignore unselected external calls");

    let start_name = expected_dev_function_name("start_main", 0);
    let unused_name = expected_dev_function_name("unused_helper", 1);
    assert!(output.source.contains(&format!("function {start_name}(")));
    assert!(
        !output.source.contains(&format!("function {unused_name}(")),
        "selected-only JS lowering should not emit unselected function bodies"
    );
    assert!(
        !output
            .referenced_external_functions
            .contains(&external_function),
        "unselected external calls should not request generated glue or runtime assets"
    );
}

fn module_with_unreachable_function(
    string_table: &mut StringTable,
    unit_type: crate::compiler_frontend::datatypes::ids::TypeId,
) -> crate::compiler_frontend::hir::module::HirModule {
    let mut module = build_module(
        string_table,
        "start_main",
        vec![return_block(0, unit_type)],
        function(0, 0, unit_type),
        &[],
    );

    module.blocks.push(return_block(1, unit_type));
    module.functions.push(function(1, 1, unit_type));
    module.regions.push(HirRegion::lexical(RegionId(1), None));

    let function_path = InternedPath::from_single_str("unused_helper", string_table);
    module
        .side_table
        .bind_function_name(FunctionId(1), function_path);

    module
}

fn module_with_unreachable_external_call(
    string_table: &mut StringTable,
    unit_type: crate::compiler_frontend::datatypes::ids::TypeId,
    external_function: ExternalFunctionId,
) -> crate::compiler_frontend::hir::module::HirModule {
    let mut module = module_with_unreachable_function(string_table, unit_type);
    module.blocks[1] = external_call_block(1, unit_type, external_function);
    module
}

fn function(
    function_id: u32,
    entry_block_id: u32,
    return_type: crate::compiler_frontend::datatypes::ids::TypeId,
) -> HirFunction {
    HirFunction {
        id: FunctionId(function_id),
        entry: BlockId(entry_block_id),
        params: vec![],
        return_type,
    }
}

fn return_block(
    block_id: u32,
    unit_type: crate::compiler_frontend::datatypes::ids::TypeId,
) -> HirBlock {
    HirBlock {
        id: BlockId(block_id),
        region: RegionId(block_id),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(unit_expression(block_id, unit_type, RegionId(block_id))),
    }
}

fn external_call_block(
    block_id: u32,
    unit_type: crate::compiler_frontend::datatypes::ids::TypeId,
    external_function: ExternalFunctionId,
) -> HirBlock {
    HirBlock {
        id: BlockId(block_id),
        region: RegionId(block_id),
        locals: vec![],
        statements: vec![statement(
            100 + block_id,
            HirStatementKind::Call {
                target: CallTarget::External(external_function),
                args: vec![],
                result: None,
            },
            1,
        )],
        terminator: HirTerminator::Return(unit_expression(block_id, unit_type, RegionId(block_id))),
    }
}
