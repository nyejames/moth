//! JavaScript helpers for `@core/io` console functions and input polling.
//!
//! WHAT: emits the browser console helpers used by `io.print`, `io.line`, `io.debug`,
//! `io.warn`, and `io.error`, and the browser input polling helpers used by `io.input.*`,
//! only when the corresponding external function is reachable.
//! WHY: keeping IO helper emission demand-driven prevents the runtime prelude from
//! unconditionally including console output or input code in programs that never call it.

use crate::backends::js::JsEmitter;

/// Stable JS runtime names for the `@core/io` input helpers.
///
/// WHAT: centralizes the names used by `ExternalFunctionId::name()` and the emitted JS
///       helper bodies so the demand-driven check and helper emission stay in sync.
/// WHY: avoids scattering string literals across emission branches and makes the set of
///       input helpers easy to audit.
const INPUT_HELPER_NAMES: &[&str] = &[
    "__moth_io_input_new",
    "__moth_io_input_update",
    "__moth_io_input_close",
    "__moth_io_input_key_down",
    "__moth_io_input_key_pressed",
    "__moth_io_input_key_released",
    "__moth_io_input_pointer_x",
    "__moth_io_input_pointer_y",
    "__moth_io_input_pointer_down",
    "__moth_io_input_pointer_pressed",
    "__moth_io_input_pointer_released",
    "__moth_io_input_last_key_pressed",
    "__moth_io_input_last_key_released",
    "__moth_io_input_last_pointer_pressed",
    "__moth_io_input_last_pointer_released",
];

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_core_io_helpers(&mut self) {
        self.emit_core_io_console_helpers();
        self.emit_core_io_input_helpers();
    }

    fn emit_core_io_console_helpers(&mut self) {
        let helpers: &[(&str, &str)] = &[
            (
                "__moth_io_print",
                "function __moth_io_print(value) { __moth_io_write(console.log, value); }",
            ),
            (
                "__moth_io_line",
                "function __moth_io_line(value) { __moth_io_write(console.log, value); }",
            ),
            (
                "__moth_io_debug",
                "function __moth_io_debug(value) { __moth_io_write(console.debug || console.log, value); }",
            ),
            (
                "__moth_io_warn",
                "function __moth_io_warn(value) { __moth_io_write(console.warn || console.log, value); }",
            ),
            (
                "__moth_io_error",
                "function __moth_io_error(value) { __moth_io_write(console.error || console.log, value); }",
            ),
        ];

        if helpers
            .iter()
            .any(|(js_name, _)| self.referenced_external_runtime_function(js_name))
        {
            self.emit_line("function __moth_io_write(writer, value) {");
            self.with_indent(|emitter| {
                emitter.emit_line("writer.call(console, __moth_value_to_string(value));");
            });
            self.emit_line("}");
        }

        self.emit_referenced_core_helpers(helpers);
    }

    fn emit_core_io_input_helpers(&mut self) {
        if !INPUT_HELPER_NAMES
            .iter()
            .any(|name| self.referenced_external_runtime_function(name))
        {
            return;
        }

        self.emit_core_io_input_shared_helpers();
        self.emit_core_io_input_new_helper();
        self.emit_core_io_input_update_helper();
        self.emit_core_io_input_close_helper();
        self.emit_core_io_input_polling_helpers();
        self.emit_core_io_input_last_edge_helpers();
    }

    fn emit_core_io_input_shared_helpers(&mut self) {
        // Shared input helpers: button mapping and synthetic release edges.
        self.emit_line("function __moth_io_input_map_button(button) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (button === 0) return \"left\";");
            emitter.emit_line("if (button === 1) return \"middle\";");
            emitter.emit_line("if (button === 2) return \"right\";");
            emitter.emit_line("return null;");
        });
        self.emit_line("}");

        self.emit_line("function __moth_io_input_normalize_key(key) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (key === \" \") return \"Space\";");
            emitter.emit_line(
                "if (key.length === 1 && key >= \"A\" && key <= \"Z\") return key.toLowerCase();",
            );
            emitter.emit_line("return key;");
        });
        self.emit_line("}");

        self.emit_line("function __moth_io_input_release_buttons(handle) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed) return;");
            emitter.emit_line("for (const button of Array.from(handle.heldButtons)) {");
            emitter.with_indent(|emitter| {
                emitter.emit_line("handle.pending.push({ type: \"buttonup\", button });");
                emitter.emit_line("handle.heldButtons.delete(button);");
            });
            emitter.emit_line("}");
        });
        self.emit_line("}");

        self.emit_line("function __moth_io_input_release_all(handle) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed) return;");
            emitter.emit_line("for (const key of Array.from(handle.heldKeys)) {");
            emitter.with_indent(|emitter| {
                emitter.emit_line("handle.pending.push({ type: \"keyup\", key });");
                emitter.emit_line("handle.heldKeys.delete(key);");
            });
            emitter.emit_line("}");
            emitter.emit_line("__moth_io_input_release_buttons(handle);");
        });
        self.emit_line("}");
    }

    fn emit_core_io_input_new_helper(&mut self) {
        // Input handle creation.
        self.emit_line("function __moth_io_input_new() {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (typeof window === \"undefined\" || typeof document === \"undefined\" || typeof AbortController === \"undefined\" || typeof window.PointerEvent === \"undefined\") {");
            emitter.with_indent(|emitter| {
                emitter.emit_line("const err = __moth_make_error(\"Browser input APIs unavailable\", 500, null, null);");
                emitter.emit_line("return { tag: \"err\", value: err };");
            });
            emitter.emit_line("}");
            emitter.emit_line("const handle = {");
            emitter.with_indent(|emitter| {
                emitter.emit_line("closed: false,");
                emitter.emit_line("controller: new AbortController(),");
                emitter.emit_line("pending: [],");
                emitter.emit_line("heldKeys: new Set(),");
                emitter.emit_line("pressedKeys: new Set(),");
                emitter.emit_line("releasedKeys: new Set(),");
                emitter.emit_line("heldButtons: new Set(),");
                emitter.emit_line("pressedButtons: new Set(),");
                emitter.emit_line("releasedButtons: new Set(),");
                emitter.emit_line("pointerX: 0.0,");
                emitter.emit_line("pointerY: 0.0,");
                emitter.emit_line("lastKeyPressed: null,");
                emitter.emit_line("lastKeyReleased: null,");
                emitter.emit_line("lastPointerPressed: null,");
                emitter.emit_line("lastPointerReleased: null,");
            });
            emitter.emit_line("};");
            emitter.emit_line("const signal = handle.controller.signal;");
            emitter.emit_line("const options = { passive: true, signal };");

            emitter.emit_line("window.addEventListener(\"keydown\", function (event) {");
            emitter.with_indent(|emitter| {
                emitter.emit_line("const key = __moth_io_input_normalize_key(event.key);");
                emitter.emit_line("if (!handle.heldKeys.has(key)) {");
                emitter.with_indent(|emitter| {
                    emitter.emit_line("handle.pending.push({ type: \"keypress\", key });");
                });
                emitter.emit_line("}");
                emitter.emit_line("handle.heldKeys.add(key);");
            });
            emitter.emit_line("}, options);");

            emitter.emit_line("window.addEventListener(\"keyup\", function (event) {");
            emitter.with_indent(|emitter| {
                emitter.emit_line("const key = __moth_io_input_normalize_key(event.key);");
                emitter.emit_line("handle.pending.push({ type: \"keyup\", key });");
                emitter.emit_line("handle.heldKeys.delete(key);");
            });
            emitter.emit_line("}, options);");

            emitter.emit_line("window.addEventListener(\"pointermove\", function (event) {");
            emitter.with_indent(|emitter| {
                emitter.emit_line("handle.pointerX = event.clientX;");
                emitter.emit_line("handle.pointerY = event.clientY;");
            });
            emitter.emit_line("}, options);");

            emitter.emit_line("window.addEventListener(\"pointerdown\", function (event) {");
            emitter.with_indent(|emitter| {
                emitter.emit_line("const button = __moth_io_input_map_button(event.button);");
                emitter.emit_line("if (button !== null) {");
                emitter.with_indent(|emitter| {
                    emitter.emit_line("if (!handle.heldButtons.has(button)) {");
                    emitter.with_indent(|emitter| {
                        emitter.emit_line("handle.pending.push({ type: \"buttonpress\", button });");
                    });
                    emitter.emit_line("}");
                    emitter.emit_line("handle.heldButtons.add(button);");
                });
                emitter.emit_line("}");
                emitter.emit_line("handle.pointerX = event.clientX;");
                emitter.emit_line("handle.pointerY = event.clientY;");
            });
            emitter.emit_line("}, options);");

            emitter.emit_line("window.addEventListener(\"pointerup\", function (event) {");
            emitter.with_indent(|emitter| {
                emitter.emit_line("const button = __moth_io_input_map_button(event.button);");
                emitter.emit_line("if (button !== null) {");
                emitter.with_indent(|emitter| {
                    emitter.emit_line("handle.pending.push({ type: \"buttonup\", button });");
                    emitter.emit_line("handle.heldButtons.delete(button);");
                });
                emitter.emit_line("}");
                emitter.emit_line("handle.pointerX = event.clientX;");
                emitter.emit_line("handle.pointerY = event.clientY;");
            });
            emitter.emit_line("}, options);");

            emitter.emit_line("window.addEventListener(\"pointercancel\", function () {");
            emitter.with_indent(|emitter| {
                emitter.emit_line("__moth_io_input_release_buttons(handle);");
            });
            emitter.emit_line("}, options);");

            emitter.emit_line("window.addEventListener(\"blur\", function () {");
            emitter.with_indent(|emitter| {
                emitter.emit_line("__moth_io_input_release_all(handle);");
            });
            emitter.emit_line("}, options);");

            emitter.emit_line("document.addEventListener(\"visibilitychange\", function () {");
            emitter.with_indent(|emitter| {
                emitter.emit_line("if (document.hidden) { __moth_io_input_release_all(handle); }");
            });
            emitter.emit_line("}, options);");

            emitter.emit_line("return { tag: \"ok\", value: handle };");
        });
        self.emit_line("}");
    }

    fn emit_core_io_input_update_helper(&mut self) {
        // Update: drain pending events into edge sets and last_* fields.
        self.emit_line("function __moth_io_input_update(handle) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed) return;");
            emitter.emit_line("handle.pressedKeys.clear();");
            emitter.emit_line("handle.releasedKeys.clear();");
            emitter.emit_line("handle.pressedButtons.clear();");
            emitter.emit_line("handle.releasedButtons.clear();");
            emitter.emit_line("handle.lastKeyPressed = null;");
            emitter.emit_line("handle.lastKeyReleased = null;");
            emitter.emit_line("handle.lastPointerPressed = null;");
            emitter.emit_line("handle.lastPointerReleased = null;");
            emitter.emit_line("for (const event of handle.pending) {");
            emitter.with_indent(|emitter| {
                emitter.emit_line("if (event.type === \"keypress\") {");
                emitter.with_indent(|emitter| {
                    emitter.emit_line("handle.pressedKeys.add(event.key);");
                    emitter.emit_line("handle.lastKeyPressed = event.key;");
                });
                emitter.emit_line("} else if (event.type === \"keyup\") {");
                emitter.with_indent(|emitter| {
                    emitter.emit_line("handle.releasedKeys.add(event.key);");
                    emitter.emit_line("handle.lastKeyReleased = event.key;");
                });
                emitter.emit_line("} else if (event.type === \"buttonpress\") {");
                emitter.with_indent(|emitter| {
                    emitter.emit_line("handle.pressedButtons.add(event.button);");
                    emitter.emit_line("handle.lastPointerPressed = event.button;");
                });
                emitter.emit_line("} else if (event.type === \"buttonup\") {");
                emitter.with_indent(|emitter| {
                    emitter.emit_line("handle.releasedButtons.add(event.button);");
                    emitter.emit_line("handle.lastPointerReleased = event.button;");
                });
                emitter.emit_line("}");
            });
            emitter.emit_line("}");
            emitter.emit_line("handle.pending.length = 0;");
        });
        self.emit_line("}");
    }

    fn emit_core_io_input_close_helper(&mut self) {
        // Close: abort listeners and reset state.
        self.emit_line("function __moth_io_input_close(handle) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed) return;");
            emitter.emit_line("handle.controller.abort();");
            emitter.emit_line("handle.closed = true;");
            emitter.emit_line("handle.pending.length = 0;");
            emitter.emit_line("handle.heldKeys.clear();");
            emitter.emit_line("handle.pressedKeys.clear();");
            emitter.emit_line("handle.releasedKeys.clear();");
            emitter.emit_line("handle.heldButtons.clear();");
            emitter.emit_line("handle.pressedButtons.clear();");
            emitter.emit_line("handle.releasedButtons.clear();");
            emitter.emit_line("handle.pointerX = 0.0;");
            emitter.emit_line("handle.pointerY = 0.0;");
            emitter.emit_line("handle.lastKeyPressed = null;");
            emitter.emit_line("handle.lastKeyReleased = null;");
            emitter.emit_line("handle.lastPointerPressed = null;");
            emitter.emit_line("handle.lastPointerReleased = null;");
        });
        self.emit_line("}");
    }

    fn emit_core_io_input_polling_helpers(&mut self) {
        // Polling helpers.
        self.emit_line("function __moth_io_input_key_down(handle, key) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed) return false;");
            emitter.emit_line("return handle.heldKeys.has(__moth_io_input_normalize_key(key));");
        });
        self.emit_line("}");

        self.emit_line("function __moth_io_input_key_pressed(handle, key) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed) return false;");
            emitter.emit_line("return handle.pressedKeys.has(__moth_io_input_normalize_key(key));");
        });
        self.emit_line("}");

        self.emit_line("function __moth_io_input_key_released(handle, key) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed) return false;");
            emitter
                .emit_line("return handle.releasedKeys.has(__moth_io_input_normalize_key(key));");
        });
        self.emit_line("}");

        self.emit_line("function __moth_io_input_pointer_x(handle) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed) return 0.0;");
            emitter.emit_line("return handle.pointerX;");
        });
        self.emit_line("}");

        self.emit_line("function __moth_io_input_pointer_y(handle) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed) return 0.0;");
            emitter.emit_line("return handle.pointerY;");
        });
        self.emit_line("}");

        self.emit_line("function __moth_io_input_pointer_down(handle, button) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed) return false;");
            emitter.emit_line("return handle.heldButtons.has(button);");
        });
        self.emit_line("}");

        self.emit_line("function __moth_io_input_pointer_pressed(handle, button) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed) return false;");
            emitter.emit_line("return handle.pressedButtons.has(button);");
        });
        self.emit_line("}");

        self.emit_line("function __moth_io_input_pointer_released(handle, button) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed) return false;");
            emitter.emit_line("return handle.releasedButtons.has(button);");
        });
        self.emit_line("}");
    }

    fn emit_core_io_input_last_edge_helpers(&mut self) {
        // Last-edge helpers returning the canonical option carrier.
        self.emit_line("function __moth_io_input_last_key_pressed(handle) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed || handle.lastKeyPressed === null) return { tag: \"none\" };");
            emitter.emit_line("return { tag: \"some\", value: handle.lastKeyPressed };");
        });
        self.emit_line("}");

        self.emit_line("function __moth_io_input_last_key_released(handle) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed || handle.lastKeyReleased === null) return { tag: \"none\" };");
            emitter.emit_line("return { tag: \"some\", value: handle.lastKeyReleased };");
        });
        self.emit_line("}");

        self.emit_line("function __moth_io_input_last_pointer_pressed(handle) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed || handle.lastPointerPressed === null) return { tag: \"none\" };");
            emitter.emit_line("return { tag: \"some\", value: handle.lastPointerPressed };");
        });
        self.emit_line("}");

        self.emit_line("function __moth_io_input_last_pointer_released(handle) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!handle || handle.closed || handle.lastPointerReleased === null) return { tag: \"none\" };");
            emitter.emit_line("return { tag: \"some\", value: handle.lastPointerReleased };");
        });
        self.emit_line("}");
    }
}
