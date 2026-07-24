//! Runtime prelude presence and ordering tests for JavaScript output.

use super::support::*;

// Prelude helper presence tests [binding] [alias] [computed] [clone]
// ---------------------------------------------------------------------------

/// Verifies that all six binding helpers are present in the emitted prelude. [binding]
#[test]
fn runtime_prelude_contains_all_binding_helpers() {
    let source = lower_minimal_module("main");

    assert!(
        source.contains("function __moth_is_ref("),
        "prelude must contain __moth_is_ref"
    );
    assert!(
        source.contains("function __moth_binding("),
        "prelude must contain __moth_binding"
    );
    assert!(
        source.contains("function __moth_param_binding("),
        "prelude must contain __moth_param_binding"
    );
    assert!(
        source.contains("function __moth_resolve("),
        "prelude must contain __moth_resolve"
    );
    assert!(
        source.contains("function __moth_read("),
        "prelude must contain __moth_read"
    );
    assert!(
        source.contains("function __moth_write("),
        "prelude must contain __moth_write"
    );
}

/// Verifies that both alias helpers are present in the emitted prelude. [alias]
#[test]
fn runtime_prelude_contains_alias_helpers() {
    let source = lower_minimal_module("main");

    assert!(
        source.contains("function __moth_assign_borrow("),
        "prelude must contain __moth_assign_borrow"
    );
    assert!(
        source.contains("function __moth_assign_value("),
        "prelude must contain __moth_assign_value"
    );
}

/// Verifies that both computed-place helpers are present in the emitted prelude. [computed]
#[test]
fn runtime_prelude_contains_computed_place_helpers() {
    let source = lower_minimal_module("main");

    assert!(
        source.contains("function __moth_field("),
        "prelude must contain __moth_field"
    );
    assert!(
        source.contains("function __moth_index("),
        "prelude must contain __moth_index"
    );
}

/// Verifies that the deep-copy helper is present in the emitted prelude. [clone]
#[test]
fn runtime_prelude_contains_clone_helper() {
    let source = lower_minimal_module("main");

    assert!(
        source.contains("function __moth_clone_value("),
        "prelude must contain __moth_clone_value"
    );
}

// ---------------------------------------------------------------------------
// Prelude ordering tests [prelude-order]
// ---------------------------------------------------------------------------

/// Verifies that binding helpers precede alias helpers in emitted output. [prelude-order]
#[test]
fn binding_helpers_appear_before_alias_helpers() {
    let source = lower_minimal_module("main");

    let binding_pos = source
        .find("function __moth_binding(")
        .expect("__moth_binding must be present");
    let alias_pos = source
        .find("function __moth_assign_borrow(")
        .expect("__moth_assign_borrow must be present");

    assert!(
        binding_pos < alias_pos,
        "binding helpers must appear before alias helpers in emitted JS"
    );
}

/// Verifies that alias helpers precede computed-place helpers in emitted output. [prelude-order]
#[test]
fn alias_helpers_appear_before_computed_place_helpers() {
    let source = lower_minimal_module("main");

    let alias_pos = source
        .find("function __moth_assign_value(")
        .expect("__moth_assign_value must be present");
    let computed_pos = source
        .find("function __moth_field(")
        .expect("__moth_field must be present");

    assert!(
        alias_pos < computed_pos,
        "alias helpers must appear before computed-place helpers in emitted JS"
    );
}

/// Verifies that computed-place helpers precede the clone helper in emitted output. [prelude-order]
#[test]
fn computed_place_helpers_appear_before_clone_helper() {
    let source = lower_minimal_module("main");

    let computed_pos = source
        .find("function __moth_index(")
        .expect("__moth_index must be present");
    let clone_pos = source
        .find("function __moth_clone_value(")
        .expect("__moth_clone_value must be present");

    assert!(
        computed_pos < clone_pos,
        "computed-place helpers must appear before the clone helper in emitted JS"
    );
}

// ---------------------------------------------------------------------------
// Prelude helper group presence tests [prelude-presence]
// ---------------------------------------------------------------------------

/// Verifies that the error helper group is present in the emitted prelude. [prelude-presence]
#[test]
fn runtime_prelude_contains_error_helpers() {
    let source = lower_minimal_module("main");

    assert!(
        source.contains("function __moth_make_error("),
        "prelude must contain __moth_make_error"
    );
}

/// Verifies that the result helper group is present in the emitted prelude. [prelude-presence]
#[test]
fn runtime_prelude_contains_result_helpers() {
    let source = lower_minimal_module("main");

    assert!(
        source.contains("function __moth_result_propagate("),
        "prelude must contain __moth_result_propagate"
    );
}

/// Verifies that the string helper group is present in the emitted prelude. [prelude-presence]
#[test]
fn runtime_prelude_contains_string_helpers() {
    let source = lower_minimal_module("main");

    assert!(
        source.contains("function __moth_value_to_string("),
        "prelude must contain __moth_value_to_string"
    );
    assert!(
        !source.contains("function __moth_io("),
        "old __moth_io helper must no longer be emitted unconditionally"
    );
}

/// Verifies that the collection helper group is present in the emitted prelude. [prelude-presence]
#[test]
fn runtime_prelude_contains_collection_helpers() {
    let source = lower_minimal_module("main");

    assert!(
        source.contains("function __moth_collection_get("),
        "prelude must contain __moth_collection_get"
    );
}

/// Verifies that a module that exercises runtime cast policies emits every
/// helper those policies require. [prelude-presence]
#[test]
fn runtime_prelude_contains_cast_helpers() {
    let source = lower_minimal_module_with_string_int_cast("main");

    assert!(
        source.contains("function __moth_cast_int("),
        "prelude must contain __moth_cast_int when StringToInt is used"
    );
    assert!(
        source.contains("function __moth_cast_int_in_range(value)"),
        "prelude must contain the shared i32 range predicate"
    );
    assert!(
        !source.contains("function __moth_normalize_numeric_text("),
        "String -> Int must not emit the whitespace-trimming numeric text normalizer"
    );
}

/// Verifies that the expression-level `Float -> String` helper also emits the shared formatter.
/// [prelude-presence] [float-helper]
#[test]
fn runtime_prelude_contains_float_formatter_for_float_to_string_cast() {
    let source = lower_minimal_module_with_float_string_cast("main");

    assert!(
        source.contains("function __moth_cast_float_to_string("),
        "Float -> String expression casts must emit their cast helper"
    );
    assert!(
        source.contains("function __moth_format_float("),
        "Float -> String expression casts must emit the Moth formatter helper"
    );
    assert!(
        source.contains("function __moth_numeric_trap("),
        "Float -> String expression casts use the shared numeric trap helper"
    );
}

/// Verifies that a module that only performs an `Int -> Float` cast does not
/// emit the numeric parsing helpers. [prelude-presence]
#[test]
fn runtime_prelude_omits_cast_helpers_when_only_identity_cast_is_used() {
    let source = lower_minimal_module_with_int_to_float_cast("main");

    assert!(
        !source.contains("function __moth_cast_int("),
        "prelude must not contain __moth_cast_int when only Int -> Float is used"
    );
    assert!(
        !source.contains("function __moth_cast_float("),
        "prelude must not contain __moth_cast_float when only Int -> Float is used"
    );
    assert!(
        !source.contains("function __moth_normalize_numeric_text("),
        "prelude must not contain the numeric text normalizer when no numeric parsing helper is used"
    );
    assert!(
        !source.contains("function __moth_cast_int_in_range(value)"),
        "prelude must not contain the shared range predicate when no helper needs it"
    );
}

// ---------------------------------------------------------------------------

/// Verifies that the map helper group is present in the emitted prelude. [prelude-presence]
#[test]
fn runtime_prelude_contains_map_helpers() {
    let source = lower_minimal_map_module("main");

    assert!(
        source.contains("function __moth_map_new("),
        "prelude must contain __moth_map_new"
    );
    assert!(
        source.contains("function __moth_map_is_valid("),
        "prelude must contain __moth_map_is_valid"
    );
    assert!(
        source.contains("function __moth_map_get("),
        "prelude must contain __moth_map_get"
    );
    assert!(
        source.contains("function __moth_map_contains("),
        "prelude must contain __moth_map_contains"
    );
    assert!(
        source.contains("function __moth_map_set("),
        "prelude must contain __moth_map_set"
    );
    assert!(
        source.contains("function __moth_map_remove("),
        "prelude must contain __moth_map_remove"
    );
    assert!(
        source.contains("function __moth_map_clear("),
        "prelude must contain __moth_map_clear"
    );
    assert!(
        source.contains("function __moth_map_length("),
        "prelude must contain __moth_map_length"
    );
}

// ---------------------------------------------------------------------------
// IO helper demand-driven emission tests [io-helper]
// ---------------------------------------------------------------------------

/// Verifies that a module with no console calls does not emit any IO helper. [io-helper]
#[test]
fn runtime_prelude_omits_io_helpers_when_unused() {
    let source = lower_minimal_module("main");

    assert!(
        !source.contains("function __moth_io_print("),
        "unused io.print should not emit __moth_io_print"
    );
    assert!(
        !source.contains("function __moth_io_line("),
        "unused io.line should not emit __moth_io_line"
    );
    assert!(
        !source.contains("function __moth_io_debug("),
        "unused io.debug should not emit __moth_io_debug"
    );
    assert!(
        !source.contains("function __moth_io_warn("),
        "unused io.warn should not emit __moth_io_warn"
    );
    assert!(
        !source.contains("function __moth_io_error("),
        "unused io.error should not emit __moth_io_error"
    );
    assert!(
        !source.contains("function __moth_io("),
        "old __moth_io helper must not be emitted"
    );
}

/// Verifies that each console helper is emitted only when its corresponding function is reachable. [io-helper]
#[test]
fn io_print_helper_is_emitted_when_reachable() {
    let source = lower_minimal_module_with_io_call(
        "main",
        crate::compiler_frontend::external_packages::ExternalFunctionId::IoPrint,
    );
    assert!(
        source.contains("function __moth_io_print("),
        "io.print reachability should emit __moth_io_print"
    );
    assert!(
        !source.contains("function __moth_io_line("),
        "only io.print should be emitted"
    );
    assert!(
        !source.contains("function __moth_io("),
        "old __moth_io helper must not be emitted"
    );
}

#[test]
fn io_line_helper_is_emitted_when_reachable() {
    let source = lower_minimal_module_with_io_call(
        "main",
        crate::compiler_frontend::external_packages::ExternalFunctionId::IoLine,
    );
    assert!(
        source.contains("function __moth_io_line("),
        "io.line reachability should emit __moth_io_line"
    );
    assert!(
        !source.contains("function __moth_io_print("),
        "only io.line should be emitted"
    );
    assert!(
        !source.contains("function __moth_io("),
        "old __moth_io helper must not be emitted"
    );
}

#[test]
fn io_debug_helper_is_emitted_when_reachable() {
    let source = lower_minimal_module_with_io_call(
        "main",
        crate::compiler_frontend::external_packages::ExternalFunctionId::IoDebug,
    );
    assert!(
        source.contains("function __moth_io_debug("),
        "io.debug reachability should emit __moth_io_debug"
    );
    assert!(
        !source.contains("function __moth_io("),
        "old __moth_io helper must not be emitted"
    );
}

#[test]
fn io_warn_helper_is_emitted_when_reachable() {
    let source = lower_minimal_module_with_io_call(
        "main",
        crate::compiler_frontend::external_packages::ExternalFunctionId::IoWarn,
    );
    assert!(
        source.contains("function __moth_io_warn("),
        "io.warn reachability should emit __moth_io_warn"
    );
    assert!(
        !source.contains("function __moth_io("),
        "old __moth_io helper must not be emitted"
    );
}

#[test]
fn io_error_helper_is_emitted_when_reachable() {
    let source = lower_minimal_module_with_io_call(
        "main",
        crate::compiler_frontend::external_packages::ExternalFunctionId::IoError,
    );
    assert!(
        source.contains("function __moth_io_error("),
        "io.error reachability should emit __moth_io_error"
    );
    assert!(
        !source.contains("function __moth_io("),
        "old __moth_io helper must not be emitted"
    );
}

// ---------------------------------------------------------------------------
// Input helper demand-driven emission tests [io-input-helper]
// ---------------------------------------------------------------------------

/// Verifies that input helpers are not emitted when no input function is reachable. [io-input-helper]
#[test]
fn runtime_prelude_omits_input_helpers_when_unused() {
    let source = lower_minimal_module("main");

    assert!(
        !source.contains("function __moth_io_input_new("),
        "unused io.input.new should not emit __moth_io_input_new"
    );
    assert!(
        !source.contains("function __moth_io_input_update("),
        "unused io.input.update should not emit __moth_io_input_update"
    );
    assert!(
        !source.contains("function __moth_io_input_close("),
        "unused io.input.close should not emit __moth_io_input_close"
    );
    assert!(
        !source.contains("function __moth_io_input_key_down("),
        "unused io.input.key_down should not emit __moth_io_input_key_down"
    );
    assert!(
        !source.contains("function __moth_io_input_pointer_x("),
        "unused io.input.pointer_x should not emit __moth_io_input_pointer_x"
    );
    assert!(
        !source.contains("function __moth_io_input_last_key_pressed("),
        "unused io.input.last_key_pressed should not emit __moth_io_input_last_key_pressed"
    );
}

/// Verifies that console helpers alone do not pull in input helpers. [io-input-helper]
#[test]
fn console_io_does_not_emit_input_helpers() {
    let source = lower_minimal_module_with_io_call(
        "main",
        crate::compiler_frontend::external_packages::ExternalFunctionId::IoLine,
    );

    assert!(
        source.contains("function __moth_io_line("),
        "io.line reachability should emit __moth_io_line"
    );
    assert!(
        !source.contains("function __moth_io_input_new("),
        "console-only IO should not emit input helpers"
    );
}

/// Verifies that `io.input.new()` reachability emits the new helper and its shared dependencies. [io-input-helper]
#[test]
fn io_input_new_helper_is_emitted_when_reachable() {
    let source = lower_minimal_module_with_io_input_call(
        "main",
        crate::compiler_frontend::external_packages::ExternalFunctionId::IoInputNew,
    );

    assert!(
        source.contains("function __moth_io_input_new("),
        "io.input.new reachability should emit __moth_io_input_new"
    );
    assert!(
        source.contains("function __moth_io_input_map_button("),
        "input helpers should emit shared button mapper"
    );
    assert!(
        source.contains("function __moth_io_input_normalize_key("),
        "input helpers should emit shared key normalizer"
    );
    assert!(
        source.contains("function __moth_io_input_release_all("),
        "input helpers should emit shared release helper"
    );

    let new_helper = helper_source(&source, "__moth_io_input_new");
    assert!(
        new_helper.contains("typeof window.PointerEvent === \"undefined\""),
        "io.input.new must feature-detect Pointer Events"
    );
    assert!(
        new_helper.contains("const options = { passive: true, signal };"),
        "io.input.new must register passive abortable listeners"
    );
}

/// Verifies that `io.input.update(~input)` reachability emits a real update body. [io-input-helper]
#[test]
fn io_input_update_helper_is_emitted_when_reachable() {
    let source = lower_minimal_module_with_io_input_call(
        "main",
        crate::compiler_frontend::external_packages::ExternalFunctionId::IoInputUpdate,
    );

    let helper = helper_source(&source, "__moth_io_input_update");
    assert!(
        helper.contains("handle.pending.length = 0"),
        "__moth_io_input_update must drain pending events"
    );
    assert!(
        helper.contains("handle.pressedKeys.clear()"),
        "__moth_io_input_update must clear previous key press edges"
    );
}

/// Verifies that `io.input.close(~input)` reachability emits a real close body. [io-input-helper]
#[test]
fn io_input_close_helper_is_emitted_when_reachable() {
    let source = lower_minimal_module_with_io_input_call(
        "main",
        crate::compiler_frontend::external_packages::ExternalFunctionId::IoInputClose,
    );

    let helper = helper_source(&source, "__moth_io_input_close");
    assert!(
        helper.contains("handle.controller.abort()"),
        "__moth_io_input_close must abort the AbortController"
    );
    assert!(
        helper.contains("handle.pointerX = 0.0"),
        "__moth_io_input_close must reset pointer coordinates"
    );
}

/// Verifies that key polling helpers emit real bodies with neutral closed-handle behavior. [io-input-helper]
#[test]
fn io_input_key_helpers_are_emitted_when_reachable() {
    let source = lower_minimal_module_with_io_input_call(
        "main",
        crate::compiler_frontend::external_packages::ExternalFunctionId::IoInputKeyDown,
    );

    assert!(
        source.contains("function __moth_io_input_key_down("),
        "io.input.key_down reachability should emit __moth_io_input_key_down"
    );
    assert!(
        source.contains("function __moth_io_input_key_pressed("),
        "input key helpers should be emitted together"
    );
    assert!(
        source.contains("function __moth_io_input_key_released("),
        "input key helpers should be emitted together"
    );

    let key_down = helper_source(&source, "__moth_io_input_key_down");
    assert!(
        key_down.contains("return handle.heldKeys.has(__moth_io_input_normalize_key(key))"),
        "__moth_io_input_key_down must normalize query keys before checking held keys"
    );
}

/// Verifies that pointer polling helpers emit real bodies. [io-input-helper]
#[test]
fn io_input_pointer_helpers_are_emitted_when_reachable() {
    let source = lower_minimal_module_with_io_input_call(
        "main",
        crate::compiler_frontend::external_packages::ExternalFunctionId::IoInputPointerX,
    );

    assert!(
        source.contains("function __moth_io_input_pointer_x("),
        "io.input.pointer_x reachability should emit __moth_io_input_pointer_x"
    );
    assert!(
        source.contains("function __moth_io_input_pointer_down("),
        "input pointer helpers should be emitted together"
    );
}

/// Verifies that `last_*` helpers return the canonical option carrier. [io-input-helper]
#[test]
fn io_input_last_helpers_return_canonical_option_carrier() {
    let source = lower_minimal_module_with_io_input_call(
        "main",
        crate::compiler_frontend::external_packages::ExternalFunctionId::IoInputLastKeyPressed,
    );

    assert!(
        source.contains("function __moth_io_input_last_key_pressed("),
        "io.input.last_key_pressed reachability should emit __moth_io_input_last_key_pressed"
    );

    let helper = helper_source(&source, "__moth_io_input_last_key_pressed");
    assert!(
        helper.contains("return { tag: \"some\", value: handle.lastKeyPressed }"),
        "last_key_pressed must return the some carrier"
    );
    assert!(
        helper.contains("return { tag: \"none\" }"),
        "last_key_pressed must return the none carrier"
    );

    let pointer_source = lower_minimal_module_with_io_input_call(
        "main",
        crate::compiler_frontend::external_packages::ExternalFunctionId::IoInputLastPointerPressed,
    );
    let pointer_helper = helper_source(&pointer_source, "__moth_io_input_last_pointer_pressed");
    assert!(
        pointer_helper.contains("handle.lastPointerPressed"),
        "last_pointer_pressed must use the plan's pointer-edge field"
    );
}

/// Verifies that emitted input helpers never call preventDefault. [io-input-helper]
#[test]
fn io_input_helpers_do_not_call_prevent_default() {
    let source = lower_minimal_module_with_io_input_call(
        "main",
        crate::compiler_frontend::external_packages::ExternalFunctionId::IoInputNew,
    );

    assert!(
        !source.contains("preventDefault"),
        "input helpers must not call preventDefault by default"
    );
}
