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
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

#[test]
fn all_functions_is_default_for_direct_js_lowering() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let module =
        module_with_unreachable_function(&mut path_fork, &mut string_table, types.unit, None);

    let output = lower_hir_to_js(
        &module,
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
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
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let external_function = ExternalFunctionId::Synthetic(77);
    let module = module_with_unreachable_function(
        &mut path_fork,
        &mut string_table,
        types.unit,
        Some(external_function),
    );

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

    let numeric_proofs = NumericProofs::default();
    let path_table = path_fork.snapshot_table();
    let mut emitter = crate::backends::js::JsEmitter::new(
        &module,
        &numeric_proofs,
        &string_table,
        &path_table,
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
        &NumericProofs::default(),
        &string_table,
        config,
        &type_environment,
        &path_fork.snapshot_table(),
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

#[test]
fn start_fallibility_metadata_uses_only_the_emitted_hir_return_type() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (mut type_environment, types) = build_type_environment();
    let carrier = type_environment.intern_fallible_carrier(types.unit, types.string);
    let mut module =
        module_with_unreachable_function(&mut path_fork, &mut string_table, types.unit, None);
    let start_unit = match &module.blocks[0].terminator {
        HirTerminator::Return(value) => *value,
        _ => panic!("start fixture should begin with its module-owned unit return"),
    };
    module.functions[0].return_type = carrier;
    module.blocks[0].terminator = HirTerminator::ReturnSuccess(start_unit);
    let lower = |module: &crate::compiler_frontend::hir::module::HirModule, config| {
        lower_hir_to_js(
            module,
            &NumericProofs::default(),
            &string_table,
            config,
            &type_environment,
            &path_fork.snapshot_table(),
        )
        .expect("start metadata fixture should lower")
    };
    assert!(
        lower(&module, default_config()).start_is_fallible,
        "the HIR carrier type owns fallibility, not the absence of an error return in this fixture"
    );

    let facts =
        collect_module_function_link_facts(&module).expect("fixture should have link facts");
    let reachability = collect_reachability_from_function_link_facts(&facts, &[FunctionId(1)])
        .expect("helper-only selection should be valid");
    let mut selected_config = default_config();
    selected_config.function_emission_policy =
        JsFunctionEmissionPolicy::Selected(reachability.backend_selection().clone());
    let selected = lower(&module, selected_config);
    assert!(!selected.start_is_fallible);
    assert!(!selected.function_name_by_id.contains_key(&FunctionId(0)));

    module.start_function = None;
    assert!(!lower(&module, default_config()).start_is_fallible);

    module.start_function = Some(FunctionId(0));
    module.functions[0].return_type = types.unit;
    module.blocks[0].terminator = HirTerminator::Return(start_unit);
    assert!(!lower(&module, default_config()).start_is_fallible);
}

fn module_with_unreachable_function(
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
    unit_type: crate::compiler_frontend::datatypes::ids::TypeId,
    unreachable_external_call: Option<ExternalFunctionId>,
) -> crate::compiler_frontend::hir::module::HirModule {
    let mut expressions = HirExpressionStore::default();
    let start_block = return_block(0, unit_type, &mut expressions);
    let unreachable_block = match unreachable_external_call {
        Some(external_function) => {
            external_call_block(1, unit_type, external_function, &mut expressions)
        }
        None => return_block(1, unit_type, &mut expressions),
    };
    let mut module = build_module(
        expressions,
        path_fork,
        string_table,
        "start_main",
        vec![start_block, unreachable_block],
        function(0, 0, unit_type),
        &[],
    );

    module.functions.push(function(1, 1, unit_type));
    module
        .function_provenance
        .insert(FunctionId(1), Default::default());
    module.regions.push(HirRegion::lexical(RegionId(1), None));

    let function_path = path_fork
        .try_intern_portable_path("unused_helper", string_table)
        .expect("test path fits");
    module
        .side_table
        .bind_function_name(FunctionId(1), function_path);

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
    expressions: &mut HirExpressionStore,
) -> HirBlock {
    HirBlock {
        id: BlockId(block_id),
        region: RegionId(block_id),
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Return(unit_expression(
            unit_type,
            RegionId(block_id),
            expressions,
        )),
    }
}

fn external_call_block(
    block_id: u32,
    unit_type: crate::compiler_frontend::datatypes::ids::TypeId,
    external_function: ExternalFunctionId,
    expressions: &mut HirExpressionStore,
) -> HirBlock {
    HirBlock {
        id: BlockId(block_id),
        region: RegionId(block_id),
        locals: vec![],
        statements: vec![statement(
            100 + block_id,
            HirStatementKind::Call {
                target: CallTarget::External(external_function),
                args: append_values(&[], expressions),
                result: None,
            },
        )],
        terminator: HirTerminator::Return(unit_expression(
            unit_type,
            RegionId(block_id),
            expressions,
        )),
    }
}
