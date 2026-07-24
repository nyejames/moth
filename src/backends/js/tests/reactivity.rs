//! JavaScript backend reactivity lowering tests.
//!
//! WHAT: verifies that reactive source bindings, source dirtying, demand-driven helper emission,
//! and template-string runtime values lower correctly for the JS backend.
//! WHY: reactivity is an opt-in runtime subsystem; these tests pin the exact source contracts so
//! non-reactive bundles stay unchanged and reactive bundles emit the expected helpers.

use super::support::*;
use crate::backends::js::{JsLoweringConfig, lower_hir_to_js};
use crate::compiler_frontend::analysis::borrow_checker::{
    BorrowCheckReport, ReactiveInvalidationFact, ReactiveInvalidationKind,
};
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{
    BlockId, FunctionId, HirNodeId, HirValueId, LocalId, RegionId,
};

use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::reactivity::{
    HirReactiveSource, HirReactiveSourceKind, HirReactiveTemplate, HirReactiveTemplateDependency,
    HirReactiveTemplateParameterDependency, ReactiveSourceId,
};
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

/// Builds a minimal HIR module containing one reactive `Int` source local assigned a literal value.
fn lower_minimal_reactive_source_module(function_name: &str) -> String {
    lower_minimal_reactive_source_module_with_report(function_name, BorrowCheckReport::default())
}

fn lower_minimal_reactive_source_module_with_report(
    function_name: &str,
    borrow_report: BorrowCheckReport,
) -> String {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let source_local = LocalId(0);
    let source_path = InternedPath::from_single_str("x", &mut string_table);

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![HirLocal {
            id: source_local,
            ty: types.int,
            mutable: true,
            region,
            source_info: Some(loc(1)),
        }],
        statements: vec![statement(
            1,
            HirStatementKind::Assign {
                target: HirPlace::Local(source_local),
                value: int_expression(1, 5, types.int, region),
            },
            2,
        )],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };

    let mut module = build_module(
        &mut string_table,
        function_name,
        vec![block],
        function,
        &[(source_local, "x")],
    );

    module.side_table.bind_reactive_source(HirReactiveSource {
        id: ReactiveSourceId(0),
        local_id: source_local,
        path: source_path,
        kind: HirReactiveSourceKind::Declaration,
        type_id: types.int,
        location: loc(1),
    });

    lower_hir_to_js(
        &module,
        &borrow_report,
        &string_table,
        JsLoweringConfig::direct_js(false),
        &type_environment,
    )
    .expect("JS lowering should succeed")
    .source
}

fn reactive_invalidation_report(statement_id: u32, source: ReactiveSourceId) -> BorrowCheckReport {
    let mut report = BorrowCheckReport::default();
    let statement_id = HirNodeId(statement_id);
    report.analysis.reactive_invalidations.insert(
        statement_id,
        vec![ReactiveInvalidationFact {
            statement_id,
            source,
            kind: ReactiveInvalidationKind::Assignment,
            location: loc(2),
        }],
    );
    report
}

/// Builds a minimal HIR module that pushes a reactive template value into a runtime fragment vector.
fn lower_minimal_reactive_template_module(function_name: &str) -> String {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let source_local = LocalId(0);
    let source_path = InternedPath::from_single_str("x", &mut string_table);
    let fragments_local = LocalId(1);

    let template_value = HirExpression {
        id: HirValueId(1),
        kind: HirExpressionKind::Load(HirPlace::Local(source_local)),
        ty: types.string,
        value_kind: ValueKind::RValue,
        region,
    };

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![
            HirLocal {
                id: source_local,
                ty: types.string,
                mutable: true,
                region,
                source_info: Some(loc(1)),
            },
            HirLocal {
                id: fragments_local,
                ty: types.collection_int,
                mutable: true,
                region,
                source_info: Some(loc(1)),
            },
        ],
        statements: vec![statement(
            1,
            HirStatementKind::PushRuntimeFragment {
                vec_local: fragments_local,
                value: template_value,
            },
            2,
        )],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
        return_aliases: vec![],
    };

    let mut module = build_module(
        &mut string_table,
        function_name,
        vec![block],
        function,
        &[(source_local, "x"), (fragments_local, "fragments")],
    );

    module.side_table.bind_reactive_source(HirReactiveSource {
        id: ReactiveSourceId(0),
        local_id: source_local,
        path: source_path,
        kind: HirReactiveSourceKind::Declaration,
        type_id: types.string,
        location: loc(1),
    });

    module
        .side_table
        .bind_reactive_template(HirReactiveTemplate {
            id: crate::compiler_frontend::hir::reactivity::ReactiveTemplateId(0),
            value_id: HirValueId(1),
            dependencies: vec![HirReactiveTemplateDependency {
                source: ReactiveSourceId(0),
                type_id: types.string,
                location: loc(1),
            }],
            template_value_parameters: vec![],
            template_backed: false,
            location: loc(2),
        });

    lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        JsLoweringConfig::direct_js(false),
        &type_environment,
    )
    .expect("JS lowering should succeed")
    .source
}

/// Builds a module with placeholder template metadata for a plain `String` parameter but no
/// concrete reactive source.
fn lower_placeholder_template_parameter_module(function_name: &str) -> String {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let parameter = LocalId(0);
    let parameter_value = HirExpression {
        id: HirValueId(1),
        kind: HirExpressionKind::Load(HirPlace::Local(parameter)),
        ty: types.string,
        value_kind: ValueKind::RValue,
        region,
    };

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![HirLocal {
            id: parameter,
            ty: types.string,
            mutable: false,
            region,
            source_info: Some(loc(1)),
        }],
        statements: vec![statement(1, HirStatementKind::Expr(parameter_value), 2)],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![parameter],
        return_type: types.unit,
        return_aliases: vec![],
    };

    let mut module = build_module(
        &mut string_table,
        function_name,
        vec![block],
        function,
        &[(parameter, "value")],
    );

    module
        .side_table
        .bind_reactive_template(HirReactiveTemplate {
            id: crate::compiler_frontend::hir::reactivity::ReactiveTemplateId(0),
            value_id: HirValueId(1),
            dependencies: vec![],
            template_value_parameters: vec![HirReactiveTemplateParameterDependency {
                parameter,
                location: loc(1),
            }],
            template_backed: true,
            location: loc(2),
        });

    lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        JsLoweringConfig::direct_js(false),
        &type_environment,
    )
    .expect("JS lowering should succeed")
    .source
}

/// Verifies that a non-reactive module does not pull in reactivity runtime helpers. [prelude]
#[test]
fn runtime_prelude_omits_reactivity_helpers_for_non_reactive_module() {
    let source = lower_minimal_module("main");

    assert!(
        !source.contains("function __moth_reactive_binding("),
        "non-reactive modules must not emit __moth_reactive_binding"
    );
    assert!(
        !source.contains("function __moth_reactive_schedule("),
        "non-reactive modules must not emit __moth_reactive_schedule"
    );
    assert!(
        !source.contains("function __moth_template_string("),
        "non-reactive modules must not emit __moth_template_string"
    );
}

/// Verifies that placeholder `String` parameter metadata does not make a non-reactive module emit
/// reactive helpers.
#[test]
fn placeholder_template_parameter_without_reactive_source_omits_helpers() {
    let source = lower_placeholder_template_parameter_module("main");

    assert!(
        !source.contains("function __moth_template_string(")
            && !source.contains("function __moth_template_snapshot(")
            && !source.contains("function __moth_reactive_binding("),
        "placeholder template metadata needs concrete reactive sources before emitting helpers"
    );
}

/// Verifies that a reactive source module emits the reactivity binding and scheduler helpers.
#[test]
fn runtime_prelude_contains_reactivity_helpers_for_reactive_source_module() {
    let source = lower_minimal_reactive_source_module("main");

    assert!(
        source.contains("function __moth_reactive_binding("),
        "reactive source modules must emit __moth_reactive_binding"
    );
    assert!(
        source.contains("function __moth_reactive_schedule("),
        "reactive source modules must emit __moth_reactive_schedule"
    );
    assert!(
        source.contains("function __moth_reactive_flush("),
        "reactive source modules must emit __moth_reactive_flush"
    );
}

/// Verifies that a reactive source-only module does not drag in template-string helpers.
#[test]
fn reactive_source_module_omits_template_string_helpers() {
    let source = lower_minimal_reactive_source_module("main");

    assert!(
        !source.contains("function __moth_template_string("),
        "source-only reactive modules must not emit __moth_template_string"
    );
    assert!(
        !source.contains("function __moth_template_snapshot("),
        "source-only reactive modules must not emit __moth_template_snapshot"
    );
}

/// Verifies that reactive source locals are initialized with the reactive binding constructor.
#[test]
fn reactive_source_local_uses_reactive_binding_initializer() {
    let source = lower_minimal_reactive_source_module("main");

    let local_name = expected_dev_local_name("x", 0);
    assert!(
        source.contains(&format!(
            "let {local_name} = __moth_reactive_binding(0, undefined);"
        )),
        "reactive source locals must initialize with __moth_reactive_binding(sourceId, undefined)"
    );
}

/// Verifies that borrow-analysis invalidation facts schedule reactive source dirtying.
#[test]
fn reactive_invalidation_fact_schedules_dirty_source() {
    let report = reactive_invalidation_report(1, ReactiveSourceId(0));
    let source = lower_minimal_reactive_source_module_with_report("main", report);

    let local_name = expected_dev_local_name("x", 0);
    assert!(
        source.contains(&format!("__moth_assign_value({local_name}, 5);"))
            && source.contains("__moth_reactive_schedule(0);"),
        "reactive source assignment must use normal storage assignment then schedule dirtying"
    );
}

/// Verifies that initialization without a borrow invalidation fact does not dirty the source.
#[test]
fn reactive_initialization_without_invalidation_does_not_schedule() {
    let source = lower_minimal_reactive_source_module("main");

    assert!(
        !source.contains("__moth_reactive_schedule(0);"),
        "reactive declaration initialization must not schedule source dirtying"
    );
}

/// Verifies that the scheduler helper batches dirty source flushing.
#[test]
fn reactive_schedule_helper_batches_flushes() {
    let source = lower_minimal_reactive_source_module("main");

    let helper = helper_source(&source, "__moth_reactive_schedule");
    assert!(
        helper.contains("__moth_reactive_dirty_sources.add(sourceId)")
            && helper.contains("__moth_reactive_flush_scheduled")
            && helper.contains("__moth_reactive_flush"),
        "__moth_reactive_schedule must mark the source dirty and arrange one flush"
    );
}

/// Verifies that `__moth_write` remains unchanged for non-reactive programs.
#[test]
fn write_helper_does_not_reference_reactivity_for_non_reactive_programs() {
    let source = lower_minimal_module("main");

    let write = helper_source(&source, "__moth_write");
    assert!(
        !write.contains("__moth_source_id") && !write.contains("__moth_reactive_schedule"),
        "__moth_write must not reference reactive scheduling in non-reactive bundles"
    );
}

/// Verifies that computed-place helpers stay shared; dirtying is emitted from borrow facts.
#[test]
fn computed_place_helpers_do_not_carry_reactive_source_ids() {
    let source = lower_minimal_reactive_source_module("main");

    let field = helper_source(&source, "__moth_field");
    assert!(
        !field.contains("sourceId"),
        "__moth_field must remain a general computed-place helper"
    );

    let index = helper_source(&source, "__moth_index");
    assert!(
        !index.contains("sourceId"),
        "__moth_index must remain a general computed-place helper"
    );
}

/// Verifies that a reactive template module emits the template-string helpers.
#[test]
fn runtime_prelude_contains_template_string_helpers_for_reactive_template_module() {
    let source = lower_minimal_reactive_template_module("main");

    assert!(
        source.contains("function __moth_template_string("),
        "reactive template modules must emit __moth_template_string"
    );
    assert!(
        source.contains("function __moth_template_snapshot("),
        "reactive template modules must emit __moth_template_snapshot"
    );
    assert!(
        source.contains("function __moth_template_dependencies("),
        "reactive template modules must emit __moth_template_dependencies"
    );
    assert!(
        source.contains("function __moth_template_collect_dependencies("),
        "reactive template modules must emit __moth_template_collect_dependencies"
    );
}

/// Verifies that runtime fragment pushes preserve reactive template objects for HTML mounting.
#[test]
fn runtime_fragment_push_preserves_reactive_template_object() {
    let source = lower_minimal_reactive_template_module("main");

    assert!(
        source.contains("push(__moth_template_string("),
        "runtime fragment pushes must preserve reactive template objects for HTML mounting"
    );
    assert!(
        !source.contains("push(__moth_template_snapshot(__moth_template_string("),
        "runtime fragment pushes must not snapshot reactive template values before mounting"
    );
}

/// Verifies that the template-string helper constructs a backend-owned value carrying a snapshot
/// function and a dependency array.
#[test]
fn template_string_helper_constructs_value_with_snapshot_and_dependencies() {
    let source = lower_minimal_reactive_template_module("main");

    let helper = helper_source(&source, "__moth_template_string");
    assert!(
        helper.contains("__moth_template: true")
            && helper.contains("snapshot,")
            && helper.contains("dependencies"),
        "__moth_template_string must create a value with __moth_template, snapshot, and dependencies"
    );
}

/// Verifies that the snapshot helper flattens template values to plain strings.
#[test]
fn template_snapshot_helper_returns_plain_string() {
    let source = lower_minimal_reactive_template_module("main");

    let helper = helper_source(&source, "__moth_template_snapshot");
    assert!(
        helper.contains("template.snapshot()") || helper.contains("return template;"),
        "__moth_template_snapshot must invoke the snapshot function or pass through plain strings"
    );
}

/// Verifies that dependency collection merges direct ids and nested template values.
#[test]
fn template_collect_dependencies_merges_direct_and_nested() {
    let source = lower_minimal_reactive_template_module("main");

    let helper = helper_source(&source, "__moth_template_collect_dependencies");
    assert!(
        helper.contains("directIds")
            && helper.contains("nestedValues")
            && helper.contains("__moth_template_dependencies"),
        "__moth_template_collect_dependencies must merge direct ids with nested template dependencies"
    );
}

/// Verifies that reactive template modules emit the DOM mount helper needed by the HTML bootstrap.
#[test]
fn reactive_template_module_emits_mount_helper() {
    let source = lower_minimal_reactive_template_module("main");

    assert!(
        source.contains("function __moth_mount_template_fragment("),
        "reactive template modules must emit __moth_mount_template_fragment for HTML slot mounting"
    );
}
