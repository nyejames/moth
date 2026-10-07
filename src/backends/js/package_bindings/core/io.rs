//! JavaScript helpers for `@core/io` console functions and input polling.
//!
//! WHAT: emits the browser console helpers used by `io.print`, `io.line`, `io.debug`,
//! `io.warn`, and `io.error`, and the browser input polling helpers used by `io.input.*`,
//! only when the corresponding external function is reachable.
//! WHY: keeping IO helper emission demand-driven prevents the runtime prelude from
//! unconditionally including console output or input code in programs that never call it.

use std::sync::LazyLock;

use super::CoreJsHelper;
use crate::backends::js::JsEmitter;
use crate::backends::js::runtime::error_result_source;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;

const IO_WRITE_JS: &str = "function __moth_io_write(writer, value) {\n    writer.call(console, __moth_value_to_string(value));\n}";

/// Console helper bodies. `__moth_io_write` comes first so emission can skip it as a dependency.
pub(crate) const CORE_IO_JS_HELPERS: &[CoreJsHelper] = &[
    CoreJsHelper {
        name: "__moth_io_write",
        source: IO_WRITE_JS,
    },
    // Browser consoles are record-oriented, so one `console.log` record already completes a line.
    // `print` and `line` therefore share a body. Appending "\n" would only render a blank line.
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
];

/// Token in `io_input.js` replaced by the unsupported-host failure result.
const INPUT_UNSUPPORTED_RESULT_PLACEHOLDER: &str = "__MOTH_IO_INPUT_UNSUPPORTED_RESULT__";

/// Input polling helpers with the compiler-owned unsupported-host failure lane interpolated.
///
/// WHY: `Error.code` values have one Rust owner, so the JavaScript asset names a placeholder
/// instead of a number. Emission and first-party dependency validation share this source.
static IO_INPUT_JS: LazyLock<String> = LazyLock::new(|| {
    include_str!("io_input.js").replace(
        INPUT_UNSUPPORTED_RESULT_PLACEHOLDER,
        &error_result_source(BuiltinErrorCode::Unsupported),
    )
});

/// Returns the input polling helper blob inventoried for dependency validation.
pub(crate) fn core_io_input_js_helper() -> CoreJsHelper {
    CoreJsHelper {
        name: "__moth_io_input",
        source: IO_INPUT_JS.as_str(),
    }
}

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
        let console_helpers = &CORE_IO_JS_HELPERS[1..];
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

        self.emit_javascript_source(&IO_INPUT_JS);
    }
}
