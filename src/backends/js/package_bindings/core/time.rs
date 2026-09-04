//! JavaScript helpers for `@core/time`.
//!
//! WHAT: emits the non-inline helper used by the typed time package.
//! WHY: most `@core/time` calls lower to pure JS expressions, but ISO parsing needs validation
//! and must return Moth's internal fallible carrier shape.

use super::CoreJsHelper;
use crate::backends::js::JsEmitter;

pub(crate) const CORE_TIME_JS_HELPER: CoreJsHelper = CoreJsHelper {
    name: "__moth_time_timestamp_from_iso_string",
    source: r#"function __moth_time_timestamp_from_iso_string(text) {
    const millis = Date.parse(text);
    if (Number.isNaN(millis)) {
        const err = __moth_make_error("Invalid ISO timestamp", 400, null, null);
        return { tag: "err", value: err };
    }
    return { tag: "ok", value: millis };
}"#,
};

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_core_time_helpers(&mut self) {
        if !self.referenced_external_runtime_function(CORE_TIME_JS_HELPER.name) {
            return;
        }

        self.emit_javascript_source(CORE_TIME_JS_HELPER.source);
    }
}
