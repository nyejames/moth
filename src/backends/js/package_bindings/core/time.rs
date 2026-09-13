//! JavaScript helpers for `@core/time`.
//!
//! WHAT: emits the non-inline helpers used by the typed time package.
//! WHY: ISO parsing needs exact grammar validation and ISO rendering needs range checks;
//!      both return Moth's internal fallible carrier shape instead of throwing host errors.

use std::sync::LazyLock;

use crate::backends::js::JsEmitter;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;

/// First renderable instant, `0000-01-01T00:00:00.000Z`.
const RENDERABLE_MIN_MILLIS: i64 = -62_167_219_200_000;
/// Last renderable instant, `9999-12-31T23:59:59.999Z`.
const RENDERABLE_MAX_MILLIS: i64 = 253_402_300_799_999;

/// Formats the canonical `__moth_error_result(message, code)` failure lane for one error code.
fn error_result_source(error: BuiltinErrorCode) -> String {
    format!(
        "__moth_error_result(\"{}\", {})",
        error.default_message(),
        error.as_i32()
    )
}

/// Builds the ISO timestamp parser helper with compiler-owned failure lanes.
///
/// WHAT: accepts exactly the published `YYYY-MM-DDTHH:MM:SS[.f{1,9}](Z|±HH:MM)` grammar and
///       returns milliseconds since the Unix epoch.
/// WHY: `Date.parse` accepts calendar and locale variants the contract rejects, so acceptance is
///      matched against an anchored grammar and field ranges here, never delegated to the host.
///      Construction goes through `setUTCFullYear` because `Date.UTC` remaps years 0 to 99 into
///      the 1900s, which the four-digit grammar accepts.
fn timestamp_from_iso_string_helper() -> String {
    let invalid_text = error_result_source(BuiltinErrorCode::TimeInvalidTimestampText);
    let out_of_range = error_result_source(BuiltinErrorCode::TimeTimestampOutOfRange);

    format!(
        r#"function __moth_time_timestamp_from_iso_string(text) {{
    const value = __moth_string_value(text);
    const match = value.match(/^(\d{{4}})-(\d{{2}})-(\d{{2}})T(\d{{2}}):(\d{{2}}):(\d{{2}})(\.\d{{1,9}})?(Z|[+-]\d{{2}}:\d{{2}})$/);
    if (match === null) {{
        return {invalid_text};
    }}
    const year = Number(match[1]);
    const month = Number(match[2]);
    const day = Number(match[3]);
    const hour = Number(match[4]);
    const minute = Number(match[5]);
    const second = Number(match[6]);
    const fraction = match[7];
    const designator = match[8];
    const isLeapYear = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
    const monthLengths = [31, isLeapYear ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if (month < 1 || month > 12 || day < 1 || day > monthLengths[month - 1]) {{
        return {invalid_text};
    }}
    if (hour > 23 || minute > 59 || second > 59) {{
        return {invalid_text};
    }}
    const instant = new Date(0);
    instant.setUTCFullYear(year, month - 1, day);
    instant.setUTCHours(hour, minute, second, 0);
    let millis = instant.getTime();
    if (fraction !== undefined) {{
        const digits = fraction.slice(1).padEnd(3, "0");
        millis += Number(digits.slice(0, 3));
    }}
    if (designator !== "Z") {{
        const offsetHours = Number(designator.slice(1, 3));
        const offsetMinutes = Number(designator.slice(4, 6));
        if (offsetHours > 23 || offsetMinutes > 59) {{
            return {invalid_text};
        }}
        const offsetMillis = (offsetHours * 60 + offsetMinutes) * 60000;
        if (designator[0] === "+") {{
            millis -= offsetMillis;
        }} else {{
            millis += offsetMillis;
        }}
    }}
    if (!(millis >= {RENDERABLE_MIN_MILLIS} && millis <= {RENDERABLE_MAX_MILLIS})) {{
        return {out_of_range};
    }}
    return {{ tag: "ok", value: millis }};
}}"#
    )
}

/// Builds the ISO timestamp rendering helper with compiler-owned failure lanes.
///
/// WHAT: renders an instant as `YYYY-MM-DDTHH:MM:SS.sssZ` after range-checking it.
/// WHY: `toISOString()` throws a host `RangeError` beyond its supported range and switches to an
///      expanded-year spelling outside the four-digit window, so rendering rejects anything the
///      published format cannot express, then truncates a fractional millisecond toward zero.
///      The range test precedes truncation, so a fractional instant just outside the window is
///      rejected rather than pulled in.
fn to_iso_string_helper() -> String {
    let out_of_range = error_result_source(BuiltinErrorCode::TimeTimestampOutOfRange);

    format!(
        r#"function __moth_time_to_iso_string(millis) {{
    if (!(millis >= {RENDERABLE_MIN_MILLIS} && millis <= {RENDERABLE_MAX_MILLIS})) {{
        return {out_of_range};
    }}
    return {{ tag: "ok", value: new Date(Math.trunc(millis)).toISOString() }};
}}"#
    )
}

/// One compiler-owned `@core/time` helper body.
///
/// WHAT: pairs a helper name with its generated source.
/// WHY: the source embeds compiler-owned error codes and range bounds, so it is built once
///      rather than written as a literal.
pub(crate) struct TimeJsHelper {
    pub name: &'static str,
    pub source: String,
}

/// The compiler-owned `@core/time` helper bodies, built once for the process.
static CORE_TIME_JS_HELPERS: LazyLock<[TimeJsHelper; 2]> = LazyLock::new(|| {
    [
        TimeJsHelper {
            name: "__moth_time_timestamp_from_iso_string",
            source: timestamp_from_iso_string_helper(),
        },
        TimeJsHelper {
            name: "__moth_time_to_iso_string",
            source: to_iso_string_helper(),
        },
    ]
});

/// Returns the `@core/time` helpers consumed by emission and first-party validation.
///
/// WHY: both consumers need the same generated source, and the numeric codes come from
/// `BuiltinErrorCode` rather than a literal.
pub(crate) fn core_time_js_helpers() -> &'static [TimeJsHelper] {
    &*CORE_TIME_JS_HELPERS
}

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_core_time_helpers(&mut self) {
        for helper in core_time_js_helpers() {
            if self.referenced_external_runtime_function(helper.name) {
                self.emit_javascript_source(&helper.source);
            }
        }
    }
}
