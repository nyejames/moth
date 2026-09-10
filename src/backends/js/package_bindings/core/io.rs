//! JavaScript helpers for `@core/io` console functions and input polling.
//!
//! WHAT: emits the browser console helpers used by `io.print`, `io.line`, `io.debug`,
//! `io.warn`, and `io.error`, and the browser input polling helpers used by `io.input.*`,
//! only when the corresponding external function is reachable.
//! WHY: keeping IO helper emission demand-driven prevents the runtime prelude from
//! unconditionally including console output or input code in programs that never call it.

use super::CoreJsHelper;
use crate::backends::js::JsEmitter;

const IO_WRITE_JS: &str = "function __moth_io_write(writer, value) {\n    writer.call(console, __moth_value_to_string(value));\n}";

const IO_INPUT_JS: &str = include_str!("io_input.js");

pub(crate) const CORE_IO_JS_HELPERS: &[CoreJsHelper] = &[
    CoreJsHelper {
        name: "__moth_io_write",
        source: IO_WRITE_JS,
    },
    CoreJsHelper {
        name: "__moth_io_print",
        source: "function __moth_io_print(value) { __moth_io_write(console.log, value); }",
    },
    CoreJsHelper {
        name: "__moth_io_line",
        source: "function __moth_io_line(value) { __moth_io_write(console.log, value); }",
    },
    CoreJsHelper {
        name: "__moth_io_debug",
        source: "function __moth_io_debug(value) { __moth_io_write(console.debug || console.log, value); }",
    },
    CoreJsHelper {
        name: "__moth_io_warn",
        source: "function __moth_io_warn(value) { __moth_io_write(console.warn || console.log, value); }",
    },
    CoreJsHelper {
        name: "__moth_io_error",
        source: "function __moth_io_error(value) { __moth_io_write(console.error || console.log, value); }",
    },
    CoreJsHelper {
        name: "__moth_io_input",
        source: IO_INPUT_JS,
    },
];

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
        let console_helpers = &CORE_IO_JS_HELPERS[1..6];
        if console_helpers
            .iter()
            .any(|helper| self.referenced_external_runtime_function(helper.name))
        {
            self.emit_javascript_source(IO_WRITE_JS);
        }

        self.emit_referenced_core_helpers(console_helpers);
    }

    fn emit_core_io_input_helpers(&mut self) {
        if !INPUT_HELPER_NAMES
            .iter()
            .any(|name| self.referenced_external_runtime_function(name))
        {
            return;
        }

        self.emit_javascript_source(IO_INPUT_JS);
    }
}
