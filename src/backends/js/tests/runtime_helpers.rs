//! Runtime helper source and emitted behavior tests for JavaScript output.

use super::support::*;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, HirMapEntry, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth};
use std::process::Command;

// Runtime helper contract tests
// ---------------------------------------------------------------------------

/// Verifies that `__moth_result_propagate` unwraps ok values and throws a structured
/// sentinel for err values. [result]
#[test]
fn result_propagate_unwraps_ok_and_throws_sentinel_for_err() {
    let source = lower_minimal_module("main");
    let propagate = helper_source(&source, "__moth_result_propagate");

    assert!(
        propagate.contains("result.tag === \"ok\"") && propagate.contains("return result.value;"),
        "__moth_result_propagate must return the ok value"
    );
    assert!(
        propagate.contains("throw { __moth_result_propagate: true, value: result.value }")
            || propagate.contains("throw { __moth_result_propagate: true, value: result.value };"),
        "__moth_result_propagate must throw a structured sentinel for err"
    );
}

/// Verifies that `__moth_result_fallback` returns the ok value directly without calling
/// the fallback function. [result]
#[test]
fn result_fallback_returns_ok_value_without_calling_fallback() {
    let source = lower_minimal_module("main");
    let fallback = helper_source(&source, "__moth_result_fallback");

    assert!(
        fallback.contains("result.tag === \"ok\"") && fallback.contains("return result.value;"),
        "__moth_result_fallback must return the ok value directly"
    );
    // The fallback callback should only be invoked in the err branch.
    let ok_pos = fallback
        .find("if (result && result.tag === \"ok\")")
        .expect("ok branch must exist");
    let ok_branch_end = fallback[ok_pos..]
        .find("return result.value;")
        .map(|i| ok_pos + i)
        .expect("ok return must exist");
    let ok_branch = &fallback[ok_pos..ok_branch_end];
    assert!(
        !ok_branch.contains("fallback()"),
        "ok branch must not call the fallback callback"
    );
}

/// Verifies that `__moth_result_fallback` invokes the fallback callback for err carriers. [result]
#[test]
fn result_fallback_invokes_callback_for_err() {
    let source = lower_minimal_module("main");
    let fallback = helper_source(&source, "__moth_result_fallback");

    assert!(
        fallback.contains("result.tag === \"err\"") && fallback.contains("return fallback();"),
        "__moth_result_fallback must invoke fallback() for err carriers"
    );
}

/// Verifies that `__moth_clone_value` deep-copies arrays via `.map(__moth_clone_value)`. [clone]
#[test]
fn clone_value_uses_map_for_arrays() {
    let source = lower_minimal_module("main");
    let clone = helper_source(&source, "__moth_clone_value");

    assert!(
        clone.contains("Array.isArray(value)") && clone.contains("value.map(__moth_clone_value)"),
        "__moth_clone_value must deep-copy arrays using .map(__moth_clone_value)"
    );
}

/// Verifies that `__moth_clone_value` deep-copies plain objects key-by-key. [clone]
#[test]
fn clone_value_iterates_object_keys() {
    let source = lower_minimal_module("main");
    let clone = helper_source(&source, "__moth_clone_value");

    assert!(
        clone.contains("Object.keys(value)")
            && clone.contains("result[key] = __moth_clone_value(value[key])"),
        "__moth_clone_value must deep-copy objects by iterating Object.keys"
    );
}

/// Verifies that generic string conversion does not fall through to JS object formatting for maps.
/// [string] [map]
#[test]
fn value_to_string_uses_deterministic_map_placeholder() {
    let source = lower_minimal_map_module("main");
    let helper = helper_source(&source, "__moth_value_to_string");

    assert!(
        helper.contains("__moth_map_is_valid(value)")
            && helper.contains("return \"[map display unavailable]\";"),
        "__moth_value_to_string must avoid JS fallback object output for maps"
    );
}

/// Verifies that `__moth_collection_index_is_valid` checks integer, bounds, and item length. [collection]
#[test]
fn collection_index_is_valid_checks_integer_bounds_and_length() {
    let source = lower_minimal_module("main");
    let helper = helper_source(&source, "__moth_collection_index_is_valid");

    assert!(
        helper.contains("Number.isInteger(index)")
            && helper.contains("index >= 0")
            && helper.contains("items.length"),
        "__moth_collection_index_is_valid must validate integer, non-negative, and in-bounds via items"
    );
}

/// Verifies that `__moth_collection_get` returns an ok carrier for valid inputs. [collection]
#[test]
fn collection_get_returns_ok_for_valid_index() {
    let source = lower_minimal_module("main");
    let get = helper_source(&source, "__moth_collection_get");

    assert!(
        get.contains("{ tag: \"ok\", value: items[index] }"),
        "__moth_collection_get must return a Result-typed ok for valid indices"
    );
}

/// Verifies that `__moth_collection_set` returns an ok carrier after writing. [collection]
#[test]
fn collection_set_returns_ok_after_write() {
    let source = lower_minimal_module("main");
    let set = helper_source(&source, "__moth_collection_set");

    assert!(
        set.contains("items[index] = value;") && set.contains("{ tag: \"ok\", value: null }"),
        "__moth_collection_set must return a fallible-carrier success after writing"
    );
}

/// Verifies that `__moth_collection_push_growable` pushes directly without a fallible carrier. [collection]
#[test]
fn collection_push_growable_pushes_directly_without_carrier() {
    let source = lower_minimal_module("main");
    let push = helper_source(&source, "__moth_collection_push_growable");

    assert!(
        push.contains("collection.push(value);"),
        "__moth_collection_push_growable must push the value directly onto the growable array"
    );
    assert!(
        !push.contains("{ tag:") && !push.contains("__moth_error_result"),
        "__moth_collection_push_growable must not build fallible result carriers"
    );
    assert!(
        !push.contains("__moth_collection_is_") && !push.contains("fixedCapacity"),
        "__moth_collection_push_growable must not validate receivers or consult fixed capacity"
    );
}

/// Verifies that `__moth_collection_remove` returns an ok carrier for valid inputs. [collection]
#[test]
fn collection_remove_returns_ok_for_valid_index() {
    let source = lower_minimal_module("main");
    let remove = helper_source(&source, "__moth_collection_remove");

    assert!(
        remove.contains("const removed = items.splice(index, 1)[0];")
            && remove.contains("{ tag: \"ok\", value: removed }"),
        "__moth_collection_remove must return the removed element in its ok carrier"
    );
}

/// Verifies that `__moth_collection_length` returns a plain length value. [collection]
#[test]
fn collection_length_is_infallible_runtime_helper() {
    let source = lower_minimal_module("main");
    let length = helper_source(&source, "__moth_collection_length");

    assert!(
        length.contains("return items.length;") && !length.contains("{ tag:"),
        "__moth_collection_length must return a plain length value"
    );
}

/// Verifies that emitted `__moth_collection_remove` calls are not implicitly propagated. [collection]
#[test]
fn collection_remove_call_is_not_wrapped_with_result_propagate() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let remove_id =
        crate::compiler_frontend::external_packages::ExternalFunctionId::CollectionRemove;

    let call_statement = statement(
        1,
        HirStatementKind::Call {
            target: CallTarget::External(remove_id),
            args: vec![
                expression(
                    1,
                    HirExpressionKind::Collection(vec![]),
                    types.collection_int,
                    RegionId(0),
                    ValueKind::RValue,
                ),
                int_expression(2, 0, types.int, RegionId(0)),
            ],
            result: Some(LocalId(0)),
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![call_statement],
        terminator: HirTerminator::Return(unit_expression(3, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "removed")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("JS lowering should succeed");

    let removed_name = expected_dev_local_name("removed", 0);

    assert!(
        output.source.contains(&format!(
            "__moth_assign_value({removed_name}, __moth_collection_remove("
        )),
        "external fallible call result carriers must be assigned as fresh values"
    );
    assert!(
        output.source.contains("__moth_collection_remove(")
            && !output
                .source
                .contains("__moth_result_propagate(__moth_collection_remove("),
        "__moth_collection_remove host call must not be auto-propagated by JS statement lowering"
    );
}

/// Verifies that emitted `__moth_collection_length` calls are plain value calls. [collection]
#[test]
fn collection_length_call_is_not_wrapped_with_result_propagate() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let length_id =
        crate::compiler_frontend::external_packages::ExternalFunctionId::CollectionLength;

    let call_statement = statement(
        1,
        HirStatementKind::Call {
            target: CallTarget::External(length_id),
            args: vec![expression(
                1,
                HirExpressionKind::Collection(vec![]),
                types.collection_int,
                RegionId(0),
                ValueKind::RValue,
            )],
            result: Some(LocalId(0)),
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![local(0, types.int, RegionId(0))],
        statements: vec![call_statement],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "len")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("JS lowering should succeed");

    assert!(
        output.source.contains("__moth_collection_length(")
            && !output
                .source
                .contains("__moth_result_propagate(__moth_collection_length("),
        "__moth_collection_length host call must stay plain"
    );
}

/// Verifies that `__moth_cast_int` applies numeric grammar to the whole string. [cast]
#[test]
fn cast_int_does_not_trim_string_input() {
    let source = lower_minimal_module_with_string_int_cast("main");
    let cast = helper_source(&source, "__moth_cast_int");

    assert!(
        cast.contains("/^-?(?:\\d+(?:_\\d+)*)$/.test(value)")
            && !cast.contains("__moth_normalize_numeric_text(value)"),
        "__moth_cast_int must reject surrounding whitespace instead of trimming it"
    );
}

/// Verifies that Float -> Int casts use the standard profile's Int range helper. [cast]
#[test]
fn cast_float_to_int_uses_standard_profile_range_helper() {
    let mut path_fork = PathInternerFork::empty();
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let source_expr = expression(
        1,
        HirExpressionKind::Float((i32::MAX as f64) + 1.0),
        type_environment.builtins().float,
        region,
        ValueKind::Const,
    );

    let cast_statement = statement(
        2,
        HirStatementKind::CastOp {
            policy:
                crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId::NumericConversion {
                source: crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar::Float,
                target: crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar::Int,
            },
            source: source_expr,
            result: Some(LocalId(0)),
        },
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, types.int, region)],
        statements: vec![cast_statement],
        terminator: HirTerminator::Return(unit_expression(3, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "result")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &NumericProofs::default(),
        &string_table,
        default_config(),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("JS lowering should succeed");

    let cast = helper_source(&output.source, "__moth_cast_float_to_int");
    assert!(
        cast.contains("!__moth_cast_integer_in_range(truncated, min, max)"),
        "__moth_cast_float_to_int must reject truncated values outside the target range"
    );
}

// ---------------------------------------------------------------------------
// Fixed collection runtime helper tests [fixed-collection]
// ---------------------------------------------------------------------------

/// Verifies that `__moth_fixed_collection` creates a wrapper with items and capacity. [fixed-collection]
#[test]
fn fixed_collection_helper_creates_wrapper_with_items_and_capacity() {
    let source = lower_minimal_module("main");
    let helper = helper_source(&source, "__moth_fixed_collection");

    assert!(
        helper.contains("__moth_kind: \"fixed_collection\"")
            && helper.contains("items: items")
            && helper.contains("fixedCapacity: fixedCapacity"),
        "__moth_fixed_collection must create a branded wrapper with items and fixed capacity"
    );
}

/// Verifies that `__moth_collection_items` returns plain arrays as-is. [fixed-collection]
#[test]
fn collection_items_returns_array_as_is() {
    let source = lower_minimal_module("main");
    let helper = helper_source(&source, "__moth_collection_items");

    assert!(
        helper.contains("Array.isArray(collection)") && helper.contains("return collection;"),
        "__moth_collection_items must return plain arrays directly"
    );
}

/// Verifies that `__moth_collection_items` extracts items from fixed wrappers. [fixed-collection]
#[test]
fn collection_items_extracts_from_fixed_wrapper() {
    let source = lower_minimal_module("main");
    let helper = helper_source(&source, "__moth_collection_items");

    assert!(
        helper.contains("return collection.items;"),
        "__moth_collection_items must extract items from fixed wrappers"
    );
}

/// Verifies that `__moth_collection_is_valid` accepts arrays. [fixed-collection]
#[test]
fn collection_is_valid_accepts_arrays() {
    let source = lower_minimal_module("main");
    let helper = helper_source(&source, "__moth_collection_is_valid");

    assert!(
        helper.contains("Array.isArray(collection)") && helper.contains("return true;"),
        "__moth_collection_is_valid must accept plain arrays"
    );
}

/// Verifies that `__moth_collection_is_valid` accepts fixed wrappers. [fixed-collection]
#[test]
fn collection_is_valid_accepts_fixed_wrappers() {
    let source = lower_minimal_module("main");
    let helper = helper_source(&source, "__moth_collection_is_valid");

    assert!(
        helper.contains("collection.__moth_kind !== \"fixed_collection\"")
            && helper.contains("Array.isArray(collection.items)")
            && helper.contains("Number.isInteger(collection.fixedCapacity)")
            && helper.contains("collection.items.length <= collection.fixedCapacity"),
        "__moth_collection_is_valid must validate the branded fixed wrapper shape"
    );
}

/// Verifies that `__moth_collection_push_fixed` validates the branded wrapper and checks capacity before pushing. [fixed-collection]
#[test]
fn collection_push_fixed_checks_capacity_before_pushing() {
    let source = lower_minimal_module("main");
    let push = helper_source(&source, "__moth_collection_push_fixed");

    assert!(
        push.contains("Array.isArray(collection)")
            && push.contains("!__moth_collection_is_valid(collection)")
            && push.contains("items.length >= collection.fixedCapacity")
            && push.contains("items.push(value);")
            && push.contains("{ tag: \"ok\", value: null }"),
        "__moth_collection_push_fixed must validate the branded wrapper, check capacity, push, and return the ok carrier"
    );
}

/// Verifies that `__moth_collection_length` returns logical item count via items.length. [fixed-collection]
#[test]
fn collection_length_returns_logical_item_count() {
    let source = lower_minimal_module("main");
    let length = helper_source(&source, "__moth_collection_length");

    assert!(
        length.contains("__moth_collection_items(collection)")
            && length.contains("return items.length;"),
        "__moth_collection_length must return logical item count, not capacity"
    );
}

/// Verifies that `__moth_collection_get` uses items from `__moth_collection_items`. [fixed-collection]
#[test]
fn collection_get_uses_collection_items() {
    let source = lower_minimal_module("main");
    let get = helper_source(&source, "__moth_collection_get");

    assert!(
        get.contains("__moth_collection_items(collection)"),
        "__moth_collection_get must operate on the dense items array"
    );
}

/// Verifies that `__moth_collection_set` uses items from `__moth_collection_items`. [fixed-collection]
#[test]
fn collection_set_uses_collection_items() {
    let source = lower_minimal_module("main");
    let set = helper_source(&source, "__moth_collection_set");

    assert!(
        set.contains("__moth_collection_items(collection)"),
        "__moth_collection_set must operate on the dense items array"
    );
}

/// Verifies that `__moth_collection_remove` uses items from `__moth_collection_items`. [fixed-collection]
#[test]
fn collection_remove_uses_collection_items() {
    let source = lower_minimal_module("main");
    let remove = helper_source(&source, "__moth_collection_remove");

    assert!(
        remove.contains("__moth_collection_items(collection)"),
        "__moth_collection_remove must operate on the dense items array"
    );
}

// Map helper contract tests
// ---------------------------------------------------------------------------

/// Verifies that `__moth_map_new` creates a branded wrapper with `new Map()`. [map]
#[test]
fn map_new_creates_branded_wrapper_with_map() {
    let source = lower_minimal_map_module("main");
    let helper = helper_source(&source, "__moth_map_new");

    assert!(
        helper.contains("__moth_kind: \"ordered_map\"") && helper.contains("new Map()"),
        "__moth_map_new must emit a branded ordered_map wrapper using new Map()"
    );
}

/// Verifies that `__moth_map_get` returns ok for present keys. [map]
#[test]
fn map_get_returns_ok_for_present_key() {
    let source = lower_minimal_map_module("main");
    let helper = helper_source(&source, "__moth_map_get");

    assert!(
        helper.contains("{ tag: \"ok\", value: map.map.get(key) }"),
        "__moth_map_get must return an ok carrier for present keys"
    );
}

/// Verifies that `__moth_map_set` stores via `map.map.set` and returns ok. [map]
#[test]
fn map_set_stores_and_returns_ok() {
    let source = lower_minimal_map_module("main");
    let helper = helper_source(&source, "__moth_map_set");

    assert!(
        helper.contains("map.map.set(__moth_map_key(key), value)")
            && helper.contains("{ tag: \"ok\", value: null }"),
        "__moth_map_set must store via map.map.set and return ok unit carrier"
    );
}

/// Verifies that `__moth_map_remove` returns the removed value and deletes the key. [map]
#[test]
fn map_remove_returns_removed_and_deletes_key() {
    let source = lower_minimal_map_module("main");
    let helper = helper_source(&source, "__moth_map_remove");

    assert!(
        helper.contains("const removed = map.map.get(key);")
            && helper.contains("map.map.delete(key);")
            && helper.contains("{ tag: \"ok\", value: removed }"),
        "__moth_map_remove must return removed value and delete the key"
    );
}

/// Verifies that `__moth_map_contains` is an infallible plain helper. [map]
#[test]
fn map_contains_is_plain_infallible_helper() {
    let source = lower_minimal_map_module("main");
    let helper = helper_source(&source, "__moth_map_contains");

    assert!(
        helper.contains("return map.map.has(__moth_map_key(key));")
            && !helper.contains("__moth_error_result"),
        "__moth_map_contains must be a plain infallible helper"
    );
}

/// Verifies that `__moth_map_clear` is an infallible plain helper. [map]
#[test]
fn map_clear_is_plain_infallible_helper() {
    let source = lower_minimal_map_module("main");
    let helper = helper_source(&source, "__moth_map_clear");

    assert!(
        helper.contains("map.map.clear();") && !helper.contains("__moth_error_result"),
        "__moth_map_clear must be a plain infallible helper"
    );
}

/// Verifies that `__moth_map_length` is an infallible plain helper. [map]
#[test]
fn map_length_is_plain_infallible_helper() {
    let source = lower_minimal_map_module("main");
    let helper = helper_source(&source, "__moth_map_length");

    assert!(
        helper.contains("return map.map.size;") && !helper.contains("__moth_error_result"),
        "__moth_map_length must be a plain infallible helper"
    );
}

/// Verifies that `__moth_clone_value` has a map branch using `__moth_map_is_valid`. [map] [clone]
#[test]
fn clone_value_has_map_branch() {
    let source = lower_minimal_map_module("main");
    let clone = helper_source(&source, "__moth_clone_value");

    assert!(
        clone.contains("__moth_map_is_valid(value)"),
        "__moth_clone_value must check for map validity"
    );
}

/// Verifies that `__moth_clone_value` deep-copies map entries with recursive clone. [map] [clone]
#[test]
fn clone_value_deep_copies_map_entries() {
    let source = lower_minimal_map_module("main");
    let clone = helper_source(&source, "__moth_clone_value");

    assert!(
        clone.contains("Array.from(value.map.entries())")
            && clone.contains("__moth_clone_value(key)")
            && clone.contains("__moth_clone_value(item)"),
        "__moth_clone_value must deep-copy map entries with recursive clone for keys and values"
    );
}

// Emitted error-code behavior tests
// ---------------------------------------------------------------------------

fn lower_error_runtime_module(profile: NumericProfile) -> String {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let map_expression = expression(
        1,
        HirExpressionKind::MapLiteral(vec![HirMapEntry {
            key: string_expression(2, "seed", types.string, region),
            value: int_expression(3, 1, types.int, region),
        }]),
        types.map_string_int,
        region,
        ValueKind::RValue,
    );

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![
            statement(1, HirStatementKind::Expr(map_expression)),
            statement(
                4,
                HirStatementKind::Call {
                    target: CallTarget::External(ExternalFunctionId::IoInputNew),
                    args: vec![],
                    result: None,
                },
            ),
        ],
        terminator: HirTerminator::Return(unit_expression(5, types.unit, region)),
    };
    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };
    let module = build_module(
        &mut path_fork,
        &mut string_table,
        "main",
        vec![block],
        function,
        &[],
    );

    lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &NumericProofs::default(),
        &string_table,
        JsLoweringConfig::direct_js(false, profile),
        &type_environment,
        &path_fork.snapshot_table(),
    )
    .expect("error runtime fixture should lower to JavaScript")
    .source
}

fn run_javascript(source: &str) -> String {
    let output = Command::new("node")
        .args(["--eval", source])
        .output()
        .expect("Node.js is required for JavaScript runtime behavior tests");
    assert!(
        output.status.success(),
        "Node.js runtime failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("Node.js output is UTF-8")
}

/// Verifies that emitted collection, map, and Core IO errors retain their unsigned code records
/// and exact messages under both Int profiles. [collection] [map] [io-input-helper]
#[test]
fn emitted_error_producers_preserve_u32_codes_across_int_profiles() {
    let profiles = [
        NumericProfile::STANDARD,
        NumericProfile {
            int_width: IntWidth::Bits64,
            float_precision: FloatPrecision::Bits64,
        },
    ];
    let code_field = format!(
        "{:?}",
        crate::backends::js::builtin_error_code_js_field_name(false)
    );

    for profile in profiles {
        let source = lower_error_runtime_module(profile);
        let (zero_index, out_of_bounds_index) = match profile.int_width {
            IntWidth::Bits32 => ("0", "1"),
            IntWidth::Bits64 => ("0n", "1n"),
        };
        let driver = r#"
delete globalThis.window;
delete globalThis.document;
if (typeof window !== "undefined" || typeof document !== "undefined") {
    throw new Error("Node.js runtime unexpectedly provides browser input globals");
}
const results = [
    __moth_collection_get(null, INDEX_ZERO),
    __moth_collection_remove(null, INDEX_ZERO),
    __moth_collection_get([42], INDEX_OUT_OF_BOUNDS),
    __moth_collection_set([42], INDEX_OUT_OF_BOUNDS, 7),
    __moth_collection_remove([42], INDEX_OUT_OF_BOUNDS),
    __moth_map_get(null, "missing"),
    __moth_map_get(__moth_map_new([]), "missing"),
    __moth_map_remove(null, "missing"),
    __moth_map_remove(__moth_map_new([["present", 1]]), "missing"),
    __moth_io_input_new()
];
for (const result of results) {
    const error = result.value;
    const code = error[ERROR_CODE_FIELD];
    console.log(JSON.stringify([
        result.tag,
        __moth_error_message(error),
        typeof code,
        String(code),
        typeof (code + 1),
        String(code + 1),
        __moth_error_code(error)
    ]));
}
"#
        .replace("INDEX_ZERO", zero_index)
        .replace("INDEX_OUT_OF_BOUNDS", out_of_bounds_index)
        .replace("ERROR_CODE_FIELD", &code_field);

        let output = run_javascript(&format!("{source}\n{driver}"));
        assert_eq!(
            output.lines().collect::<Vec<_>>(),
            vec![
                r#"["err","Collection operation expects an ordered collection","number","100","number","101",100]"#,
                r#"["err","Collection operation expects an ordered collection","number","100","number","101",100]"#,
                r#"["err","Collection index out of bounds","number","101","number","102",101]"#,
                r#"["err","Collection index out of bounds","number","101","number","102",101]"#,
                r#"["err","Collection index out of bounds","number","101","number","102",101]"#,
                r#"["err","Map operation expects an ordered map","number","110","number","111",110]"#,
                r#"["err","Map key not found","number","111","number","112",111]"#,
                r#"["err","Map operation expects an ordered map","number","110","number","111",110]"#,
                r#"["err","Map key not found","number","111","number","112",111]"#,
                r#"["err","Browser input APIs unavailable","number","500","number","501",500]"#,
            ],
            "generated error records must retain U32-number codes under {profile}"
        );
    }
}
