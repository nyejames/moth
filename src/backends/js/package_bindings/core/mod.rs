//! JavaScript helper emission for optional `@core/*` packages.
//!
//! WHAT: emits JS helpers only for referenced core external functions.
//! WHY: optional core packages are builder-provided surface; keeping helper emission here
//! prevents the generic runtime prelude from becoming a package implementation dump.

pub(crate) mod io;
pub(crate) mod random;
pub(crate) mod text;
pub(crate) mod time;

use crate::backends::js::JsEmitter;
use crate::compiler_frontend::external_packages::ExternalJsLowering;

/// One Core package helper body shared by JS emission and first-party validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CoreJsHelper {
    pub name: &'static str,
    pub source: &'static str,
}

/// Optional `@core/*` helper bodies emitted from `package_bindings`, including unreferenced ones.
///
/// Generic runtime-prelude helpers such as `@core/collections` stay with `src/backends/js/runtime`
/// and are outside this first-party package inventory.
pub(crate) fn core_javascript_helpers() -> Vec<CoreJsHelper> {
    let mut helpers = Vec::new();
    helpers.extend_from_slice(text::CORE_TEXT_JS_HELPERS);
    helpers.extend_from_slice(random::CORE_RANDOM_JS_HELPERS);
    helpers.extend_from_slice(io::CORE_IO_JS_HELPERS);
    helpers.push(time::CORE_TIME_JS_HELPER);
    helpers
}

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_core_package_helpers(&mut self) {
        self.emit_core_text_helpers();
        self.emit_core_io_helpers();
        self.emit_core_random_helpers();
        self.emit_core_time_helpers();
    }

    pub(super) fn referenced_external_runtime_function(&self, js_name: &str) -> bool {
        self.referenced_external_functions.iter().any(|id| {
            self.config
                .external_package_registry
                .get_function_by_id(*id)
                .and_then(|def| def.lowerings.js.as_ref())
                .is_some_and(|lowering| {
                    matches!(lowering, ExternalJsLowering::RuntimeFunction(name) if name == js_name)
                })
        })
    }

    pub(super) fn emit_referenced_core_helpers(&mut self, helpers: &[CoreJsHelper]) {
        for helper in helpers {
            if self.referenced_external_runtime_function(helper.name) {
                self.emit_javascript_source(helper.source);
            }
        }
    }
}
