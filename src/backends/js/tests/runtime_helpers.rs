//! Runtime helper source contract tests for JavaScript output.

use super::support::*;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::hir::blocks::HirBlock;
use crate::compiler_frontend::hir::expressions::{HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId, LocalId, RegionId};
use crate::compiler_frontend::hir::numeric::NumericFailureMode;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::terminators::HirTerminator;

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

/// Verifies that `__moth_error_result` wraps `__moth_make_error` in an err carrier. [error]
#[test]
fn error_result_helper_wraps_make_error_in_err_carrier() {
    let source = lower_minimal_module("main");
    let helper = helper_source(&source, "__moth_error_result");

    assert!(
        helper.contains("__moth_make_error(message, code, null, null)")
            && helper.contains("tag: \"err\""),
        "__moth_error_result must wrap __moth_make_error in an err result carrier"
    );
}

/// Verifies that backend-created errors use the same lowered fields as builtin `Error(...)`.
/// [error]
#[test]
fn make_error_uses_builtin_error_field_symbols() {
    let source = lower_minimal_module("main");
    let helper = helper_source(&source, "__moth_make_error");

    let message_field = expected_dev_field_name("message", 0);
    let code_field = expected_dev_field_name("code", 1);

    assert!(
        helper.contains(&format!("{message_field}: message"))
            && helper.contains(&format!("{code_field}: code")),
        "__moth_make_error must construct the lowered builtin Error fields"
    );
    assert!(
        !helper.contains("\n        message,") && !helper.contains("\n        code,"),
        "__moth_make_error must not construct a parallel plain JS error shape"
    );
}

/// Verifies that location/trace helpers preserve canonical error fields.
/// [error]
#[test]
fn error_context_helpers_read_canonical_error_fields() {
    let source = lower_minimal_module("main");
    let with_location = helper_source(&source, "__moth_error_with_location");
    let push_trace = helper_source(&source, "__moth_error_push_trace");

    assert!(
        with_location.contains("__moth_error_message(error)")
            && with_location.contains("__moth_error_code(error)")
            && push_trace.contains("__moth_error_message(error)")
            && push_trace.contains("__moth_error_code(error)"),
        "runtime error context helpers must preserve canonical Error.message/Error.code fields"
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

/// Verifies that collection helpers use the expected error code for invalid receivers. [collection]
#[test]
fn collection_helpers_use_expected_error_code_for_invalid_receiver() {
    let source = lower_minimal_module("main");
    let get = helper_source(&source, "__moth_collection_get");
    let set = helper_source(&source, "__moth_collection_set");
    let push = helper_source(&source, "__moth_collection_push");
    let remove = helper_source(&source, "__moth_collection_remove");

    let expected_error = BuiltinErrorCode::CollectionExpectedOrderedCollection;
    let expected_message = expected_error.default_message();
    let expected_code = expected_error.as_i32();
    let expected = format!(r#"__moth_error_result("{expected_message}", {expected_code})"#);

    assert!(
        get.contains(&expected)
            && set.contains(&expected)
            && push.contains(&expected)
            && remove.contains(&expected),
        "collection helpers must use CollectionExpectedOrderedCollection for invalid receivers"
    );
}

/// Verifies that collection helpers use the expected error code for out-of-bounds indices. [collection]
#[test]
fn collection_helpers_use_expected_error_code_for_out_of_bounds() {
    let source = lower_minimal_module("main");
    let get = helper_source(&source, "__moth_collection_get");
    let set = helper_source(&source, "__moth_collection_set");
    let remove = helper_source(&source, "__moth_collection_remove");

    let expected_error = BuiltinErrorCode::CollectionIndexOutOfBounds;
    let expected_message = expected_error.default_message();
    let expected_code = expected_error.as_i32();
    let expected = format!(r#"__moth_error_result("{expected_message}", {expected_code})"#);

    assert!(
        get.contains(&expected) && set.contains(&expected) && remove.contains(&expected),
        "collection helpers must use CollectionIndexOutOfBounds for invalid indices"
    );
}

/// Verifies that `__moth_collection_get` returns an err carrier for invalid collection inputs. [collection]
#[test]
fn collection_get_returns_err_for_invalid_collection() {
    let source = lower_minimal_module("main");
    let get = helper_source(&source, "__moth_collection_get");

    assert!(
        get.contains("__moth_collection_is_valid(collection)")
            && get.contains("__moth_error_result"),
        "__moth_collection_get must return a Result-typed err for invalid collection inputs"
    );
}

/// Verifies that `__moth_collection_get` returns an err carrier for out-of-bounds indices. [collection]
#[test]
fn collection_get_returns_err_for_out_of_bounds() {
    let source = lower_minimal_module("main");
    let get = helper_source(&source, "__moth_collection_get");

    assert!(
        get.contains("!__moth_collection_index_is_valid(collection, index)")
            && get.contains("__moth_error_result"),
        "__moth_collection_get must return a Result-typed err for out-of-bounds indices"
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

/// Verifies that `__moth_collection_set` returns an err carrier for out-of-bounds indices. [collection]
#[test]
fn collection_set_returns_err_for_out_of_bounds() {
    let source = lower_minimal_module("main");
    let set = helper_source(&source, "__moth_collection_set");

    assert!(
        set.contains("!__moth_collection_index_is_valid(collection, index)")
            && set.contains("__moth_error_result"),
        "__moth_collection_set must return a Result-typed err for out-of-bounds indices"
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

/// Verifies that `__moth_collection_push` returns a fallible carrier on success. [collection]
#[test]
fn collection_push_returns_ok_after_mutation() {
    let source = lower_minimal_module("main");
    let push = helper_source(&source, "__moth_collection_push");

    assert!(
        push.contains("items.push(value)") && push.contains("{ tag: \"ok\", value: null }"),
        "__moth_collection_push must return a fallible-carrier success after pushing"
    );
}

/// Verifies that `__moth_collection_remove` returns an err carrier for invalid collection inputs. [collection]
#[test]
fn collection_remove_returns_err_for_invalid_collection() {
    let source = lower_minimal_module("main");
    let remove = helper_source(&source, "__moth_collection_remove");

    assert!(
        remove.contains("__moth_collection_is_valid(collection)")
            && remove.contains("__moth_error_result"),
        "__moth_collection_remove must return a Result-typed err for invalid collection inputs"
    );
}

/// Verifies that `__moth_collection_remove` returns an err carrier for out-of-bounds indices. [collection]
#[test]
fn collection_remove_returns_err_for_out_of_bounds() {
    let source = lower_minimal_module("main");
    let remove = helper_source(&source, "__moth_collection_remove");

    assert!(
        remove.contains("!__moth_collection_index_is_valid(collection, index)")
            && remove.contains("__moth_error_result"),
        "__moth_collection_remove must return a Result-typed err for out-of-bounds indices"
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

/// Verifies that emitted `__moth_collection_push` calls are plain statements. [collection]
#[test]
fn collection_push_call_is_not_wrapped_with_result_propagate() {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();

    let push_id = crate::compiler_frontend::external_packages::ExternalFunctionId::CollectionPush;

    let call_statement = statement(
        1,
        HirStatementKind::Call {
            target: CallTarget::External(push_id),
            args: vec![
                expression(
                    1,
                    HirExpressionKind::Collection(vec![]),
                    types.collection_int,
                    RegionId(0),
                    ValueKind::RValue,
                ),
                int_expression(2, 42, types.int, RegionId(0)),
            ],
            result: None,
        },
        1,
    );

    let block = HirBlock {
        id: BlockId(0),
        region: RegionId(0),
        locals: vec![],
        statements: vec![call_statement],
        terminator: HirTerminator::Return(unit_expression(3, types.unit, RegionId(0))),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(&mut string_table, "main", vec![block], function, &[]);

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    assert!(
        output.source.contains("__moth_collection_push(")
            && !output
                .source
                .contains("__moth_result_propagate(__moth_collection_push("),
        "__moth_collection_push host call must stay plain"
    );
}

/// Verifies that emitted `__moth_collection_remove` calls are not implicitly propagated. [collection]
#[test]
fn collection_remove_call_is_not_wrapped_with_result_propagate() {
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
        1,
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
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "removed")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
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
        1,
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
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "len")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
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

/// Verifies that `__moth_cast_int` rejects non-numeric strings with a Parse error. [cast]
#[test]
fn cast_int_rejects_non_numeric_string() {
    let source = lower_minimal_module_with_string_int_cast("main");
    let cast = helper_source(&source, "__moth_cast_int");

    assert!(
        cast.contains("Cannot parse Int from text") && cast.contains("{ tag: \"err\""),
        "__moth_cast_int must return a Parse err for non-numeric strings"
    );
}

/// Verifies that `__moth_cast_int` accepts integer strings via parseInt. [cast]
#[test]
fn cast_int_accepts_integer_string() {
    let source = lower_minimal_module_with_string_int_cast("main");
    let cast = helper_source(&source, "__moth_cast_int");

    assert!(
        cast.contains("Number.parseInt(value.replace(/_/g, ''), 10)")
            && cast.contains("{ tag: \"ok\""),
        "__moth_cast_int must parse integer strings and return ok"
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

/// Verifies that `__moth_cast_int` uses the shared i32 range helpers. [cast]
#[test]
fn cast_int_uses_i32_range_helpers() {
    let source = lower_minimal_module_with_string_int_cast("main");

    assert!(
        source.contains("function __moth_cast_int_in_range(value)")
            && source.contains("const __BS_INT_CAST_MIN = -2147483648")
            && source.contains("const __BS_INT_CAST_MAX = 2147483647"),
        "__moth_cast_int must rely on shared Alpha i32 range helpers"
    );

    let cast = helper_source(&source, "__moth_cast_int");
    assert!(
        cast.contains("__moth_cast_int_in_range(value)")
            && cast.contains("__moth_cast_int_in_range(parsed)"),
        "__moth_cast_int numeric and string branches must use the shared range predicate"
    );
}

/// Verifies that `__moth_cast_float_to_int` uses the shared i32 range helper. [cast]
#[test]
fn cast_float_to_int_uses_i32_range_helper() {
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
                crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId::FloatToInt,
            source: source_expr,
            result: Some(LocalId(0)),
        },
        1,
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
        &mut string_table,
        "main",
        vec![block],
        function,
        &[(LocalId(0), "result")],
    );

    let output = lower_hir_to_js(
        &module,
        &BorrowCheckReport::default(),
        &string_table,
        default_config(),
        &type_environment,
    )
    .expect("JS lowering should succeed");

    let cast = helper_source(&output.source, "__moth_cast_float_to_int");
    assert!(
        cast.contains("!__moth_cast_int_in_range(truncated)"),
        "__moth_cast_float_to_int must reject truncated values outside the i32 range"
    );
}

/// Verifies that `__moth_cast_float` rejects invalid strings with a Parse error. [cast]
#[test]
fn cast_float_rejects_invalid_string() {
    let source = lower_minimal_module_with_string_float_cast("main");
    let cast = helper_source(&source, "__moth_cast_float");

    assert!(
        cast.contains("Cannot parse Float from text") && cast.contains("{ tag: \"err\""),
        "__moth_cast_float must return a Parse err for invalid strings"
    );
}

/// Verifies that `__moth_cast_float` applies the shared Moth numeric text grammar to
/// the whole string and does not trim whitespace. [cast]
#[test]
fn cast_float_uses_shared_numeric_text_grammar() {
    let source = lower_minimal_module_with_string_float_cast("main");
    let cast = helper_source(&source, "__moth_cast_float");

    assert!(
        cast.contains("Number.parseFloat(value.replace(/_/g, \"\"))"),
        "__moth_cast_float must parse the string after removing digit separators"
    );
    assert!(
        cast.contains("/^-?\\d+(?:_\\d+)*(?:\\.\\d+(?:_\\d+)*)?(?:e[+-]?\\d+(?:_\\d+)*)?$/"),
        "__moth_cast_float must match the Moth numeric text grammar exactly"
    );
    assert!(
        !cast.contains("__moth_normalize_numeric_text"),
        "__moth_cast_float must not trim surrounding whitespace"
    );
    assert!(
        cast.contains("Number.isFinite(parsed)"),
        "__moth_cast_float must reject non-finite parsed values"
    );
}

/// Verifies that `__moth_cast_float` rejects non-finite parsed values with an out-of-range
/// error carrier. [cast]
#[test]
fn cast_float_rejects_non_finite_parsed_value() {
    let source = lower_minimal_module_with_string_float_cast("main");
    let cast = helper_source(&source, "__moth_cast_float");

    assert!(
        cast.contains("Float value is out of supported range") && cast.contains("{ tag: \"err\""),
        "__moth_cast_float must return an out-of-range err when Number.parseFloat yields Infinity"
    );
}

/// Verifies that `__moth_error_bubble` normalizes the file path and builds a trace frame. [error]
#[test]
fn error_bubble_normalizes_file_and_builds_trace() {
    let source = lower_minimal_module("main");
    let bubble = helper_source(&source, "__moth_error_bubble");

    assert!(
        bubble.contains("__moth_error_normalize_file(file)")
            && bubble.contains("const frame = { function: safeFunction, location }")
            && bubble.contains("__moth_error_push_trace"),
        "__moth_error_bubble must normalize file paths and push a trace frame"
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

/// Verifies that `__moth_collection_fixed_capacity` returns null for arrays. [fixed-collection]
#[test]
fn collection_fixed_capacity_returns_null_for_arrays() {
    let source = lower_minimal_module("main");
    let helper = helper_source(&source, "__moth_collection_fixed_capacity");

    assert!(
        helper.contains("Array.isArray(collection)") && helper.contains("return null;"),
        "__moth_collection_fixed_capacity must return null for growable arrays"
    );
}

/// Verifies that `__moth_collection_fixed_capacity` returns capacity for fixed wrappers. [fixed-collection]
#[test]
fn collection_fixed_capacity_returns_capacity_for_fixed() {
    let source = lower_minimal_module("main");
    let helper = helper_source(&source, "__moth_collection_fixed_capacity");

    assert!(
        helper.contains("return collection.fixedCapacity;"),
        "__moth_collection_fixed_capacity must return capacity for fixed wrappers"
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

/// Verifies that `__moth_collection_push` checks fixed capacity before pushing. [fixed-collection]
#[test]
fn collection_push_checks_fixed_capacity() {
    let source = lower_minimal_module("main");
    let push = helper_source(&source, "__moth_collection_push");

    assert!(
        push.contains("__moth_collection_fixed_capacity(collection)")
            && push.contains("items.length >= fixedCapacity"),
        "__moth_collection_push must check fixed capacity before pushing"
    );
}

/// Verifies that `__moth_collection_push` returns capacity exceeded error for fixed collections. [fixed-collection]
#[test]
fn collection_push_returns_capacity_exceeded_error() {
    let source = lower_minimal_module("main");
    let push = helper_source(&source, "__moth_collection_push");

    let expected_error = BuiltinErrorCode::CollectionFixedCapacityExceeded;
    let expected_message = expected_error.default_message();
    let expected_code = expected_error.as_i32();
    let expected = format!(r#"__moth_error_result("{expected_message}", {expected_code})"#);

    assert!(
        push.contains(&expected),
        "__moth_collection_push must return CollectionFixedCapacityExceeded when full"
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

/// Verifies that `__moth_map_get` returns MapKeyNotFound for missing keys. [map]
#[test]
fn map_get_returns_key_not_found_for_missing_key() {
    let source = lower_minimal_map_module("main");
    let helper = helper_source(&source, "__moth_map_get");

    let expected_error = BuiltinErrorCode::MapKeyNotFound;
    let expected_message = expected_error.default_message();
    let expected_code = expected_error.as_i32();
    let expected = format!(r#"__moth_error_result("{expected_message}", {expected_code})"#);

    assert!(
        helper.contains(&expected),
        "__moth_map_get must return MapKeyNotFound when key is missing"
    );
}

/// Verifies that `__moth_map_get` validates receiver type. [map]
#[test]
fn map_get_validates_receiver() {
    let source = lower_minimal_map_module("main");
    let helper = helper_source(&source, "__moth_map_get");

    let expected_error = BuiltinErrorCode::MapExpectedOrderedMap;
    let expected_message = expected_error.default_message();
    let expected_code = expected_error.as_i32();
    let expected = format!(r#"__moth_error_result("{expected_message}", {expected_code})"#);

    assert!(
        helper.contains(&expected),
        "__moth_map_get must validate receiver and return MapExpectedOrderedMap for invalid receivers"
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

/// Verifies that `__moth_map_remove` returns MapKeyNotFound for missing keys. [map]
#[test]
fn map_remove_returns_key_not_found_for_missing_key() {
    let source = lower_minimal_map_module("main");
    let helper = helper_source(&source, "__moth_map_remove");

    let expected_error = BuiltinErrorCode::MapKeyNotFound;
    let expected_message = expected_error.default_message();
    let expected_code = expected_error.as_i32();
    let expected = format!(r#"__moth_error_result("{expected_message}", {expected_code})"#);

    assert!(
        helper.contains(&expected),
        "__moth_map_remove must return MapKeyNotFound when key is missing"
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

// Float helper contract tests [float-helper]
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum FloatHelperEmission {
    Format,
    Validate,
}

/// Builds and lowers a minimal module that performs one Float helper statement.
///
/// WHY: Float helper contract tests need modules that emit each focused helper without duplicating
/// HIR construction in every fixture.
fn lower_minimal_module_with_float_helper(
    function_name: &str,
    helper: FloatHelperEmission,
) -> String {
    let mut string_table = StringTable::new();
    let (type_environment, types) = build_type_environment();
    let region = RegionId(0);

    let source_expr = float_expression(1, 1.5, types.float, region);
    let (statement_kind, result_type) = match helper {
        FloatHelperEmission::Format => (
            HirStatementKind::FormatFloat {
                source: source_expr,
                failure_mode: NumericFailureMode::Trap,
                result: LocalId(0),
            },
            types.string,
        ),
        FloatHelperEmission::Validate => (
            HirStatementKind::ValidateFloat {
                source: source_expr,
                failure_mode: NumericFailureMode::Trap,
                result: LocalId(0),
            },
            types.float,
        ),
    };

    let float_statement = statement(1, statement_kind, 1);

    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![local(0, result_type, region)],
        statements: vec![float_statement],
        terminator: HirTerminator::Return(unit_expression(2, types.unit, region)),
    };

    let function = HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: vec![],
        return_type: types.unit,
    };

    let module = build_module(
        &mut string_table,
        function_name,
        vec![block],
        function,
        &[(LocalId(0), "result")],
    );

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

fn lower_minimal_module_with_format_float(function_name: &str) -> String {
    lower_minimal_module_with_float_helper(function_name, FloatHelperEmission::Format)
}

fn lower_minimal_module_with_validate_float(function_name: &str) -> String {
    lower_minimal_module_with_float_helper(function_name, FloatHelperEmission::Validate)
}

/// Verifies that `__moth_float_validate` returns ok for finite values and rejects non-finite values.
#[test]
fn float_validate_helper_checks_finite() {
    let source = lower_minimal_module_with_validate_float("main");
    let validate = helper_source(&source, "__moth_float_validate");

    assert!(
        validate.contains("Number.isFinite(value)") && validate.contains("{ tag: \"ok\", value }"),
        "__moth_float_validate must return an ok carrier for finite values"
    );

    let non_finite = BuiltinErrorCode::FloatBoundaryNonFinite;
    let expected = format!(
        r#"__moth_error_result("{}", {})"#,
        non_finite.default_message(),
        non_finite.as_i32()
    );
    assert!(
        validate.contains(&expected),
        "__moth_float_validate must use FloatBoundaryNonFinite for non-finite values"
    );
}

/// Verifies that `__moth_format_float` rejects non-finite inputs defensively.
#[test]
fn format_float_helper_rejects_non_finite() {
    let source = lower_minimal_module_with_format_float("main");
    let format = helper_source(&source, "__moth_format_float");

    let non_finite = BuiltinErrorCode::FloatFormatInvariant;
    let expected = format!(
        r#"__moth_error_result("{}", {})"#,
        non_finite.default_message(),
        non_finite.as_i32()
    );
    assert!(
        format.contains(&expected),
        "__moth_format_float must use FloatFormatInvariant for unexpected non-finite values"
    );
}

/// Verifies that `__moth_format_float` normalizes `-0.0` to the string "0".
#[test]
fn format_float_helper_normalizes_negative_zero() {
    let source = lower_minimal_module_with_format_float("main");
    let format = helper_source(&source, "__moth_format_float");

    assert!(
        format.contains("Object.is(value, -0)")
            && format.contains(r#"return { tag: "ok", value: "0" };"#),
        "__moth_format_float must format -0.0 as the string 0"
    );
}

/// Verifies that `__moth_format_float` uses explicit JS number formatting and normalizes exponent signs.
#[test]
fn format_float_helper_uses_to_string_and_normalizes_exponent() {
    let source = lower_minimal_module_with_format_float("main");
    let format = helper_source(&source, "__moth_format_float");

    assert!(
        format.contains("value.toString()"),
        "__moth_format_float must format via Number.prototype.toString"
    );
    assert!(
        format.contains(r#"replace(/e([+-]?)(\d+)/i"#),
        "__moth_format_float must normalize exponent case and sign placeholders"
    );
    assert!(
        format.contains(r#"const explicitSign = sign === "-" ? "-" : "+";"#),
        "__moth_format_float must compute an explicit exponent sign"
    );
    assert!(
        format.contains(r#"return "e" + explicitSign + digits;"#),
        "__moth_format_float must emit lowercase e with explicit exponent sign"
    );
}

/// Verifies that the expression-level `Float -> String` cast delegates to the shared formatter.
#[test]
fn cast_float_to_string_helper_uses_shared_float_formatter() {
    let source = lower_minimal_module_with_float_string_cast("main");
    let cast = helper_source(&source, "__moth_cast_float_to_string");

    assert!(
        cast.contains("__moth_numeric_trap(__moth_format_float(value))"),
        "__moth_cast_float_to_string must use the Moth Float formatter contract"
    );
}
