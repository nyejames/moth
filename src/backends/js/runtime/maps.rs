//! Map helpers for the JS runtime.
//!
//! WHAT: runtime contracts for ordered hashmaps.
//! WHY: fallible map operations return structured carriers, while infallible operations
//! stay plain JS helpers so the backend surface matches the language semantics.
//!
//! Map representation:
//! - Maps are branded `{ __moth_kind: "ordered_map", map: new Map() }` wrappers.
//!
//! Semantic policy:
//! - `get`, `set`, and `remove` return `{ tag: "ok", value: ... }` or `{ tag: "err", value: ... }`.
//! - `contains`, `clear`, and `length` are infallible helpers matching language semantics.
//! - `get` missing key → `BuiltinErrorCode::MapKeyNotFound`.
//! - `remove` missing key → `BuiltinErrorCode::MapKeyNotFound`.
//! - Invalid receivers for fallible helpers → `BuiltinErrorCode::MapExpectedOrderedMap`.
//! - Error messages are deterministic and do not render arbitrary keys.

use crate::backends::js::JsEmitter;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_runtime_map_helpers(&mut self) {
        let invalid_map = BuiltinErrorCode::MapExpectedOrderedMap;
        let invalid_map_code = invalid_map.as_i32();
        let invalid_map_message = invalid_map.default_message();

        let key_not_found = BuiltinErrorCode::MapKeyNotFound;
        let key_not_found_code = key_not_found.as_i32();
        let key_not_found_message = key_not_found.default_message();

        // Branded wrapper so runtime helpers can distinguish maps from arbitrary objects.
        self.emit_line("function __moth_map_new(entries) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const map = new Map();");
            emitter.emit_line("if (Array.isArray(entries)) {");
            emitter.with_indent(|em| {
                em.emit_line("for (const entry of entries) {");
                em.with_indent(|inner| {
                    inner.emit_line("if (Array.isArray(entry) && entry.length === 2) {");
                    inner.with_indent(|deepest| {
                        deepest.emit_line("map.set(__moth_map_key(entry[0]), entry[1]);");
                    });
                    inner.emit_line("}");
                });
                em.emit_line("}");
            });
            emitter.emit_line("}");
            emitter.emit_line("return { __moth_kind: \"ordered_map\", map: map };");
        });
        self.emit_line("}");
        self.emit_line("");

        // Validation gate shared by all fallible map operations.
        self.emit_line("function __moth_map_is_valid(value) {");
        self.with_indent(|emitter| {
            emitter.emit_line(
                "return value !== null && typeof value === \"object\" && value.__moth_kind === \"ordered_map\" && value.map instanceof Map;",
            );
        });
        self.emit_line("}");
        self.emit_line("");

        // Fallible accessor: invalid receiver -> MapExpectedOrderedMap, missing key -> MapKeyNotFound.
        self.emit_line("function __moth_map_get(map, key) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!__moth_map_is_valid(map)) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return __moth_error_result(\"{invalid_map_message}\", {invalid_map_code});",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("key = __moth_map_key(key);");
            emitter.emit_line("if (!map.map.has(key)) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return __moth_error_result(\"{key_not_found_message}\", {key_not_found_code});",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("return { tag: \"ok\", value: map.map.get(key) };");
        });
        self.emit_line("}");
        self.emit_line("");

        // Infallible helpers do not validate the receiver or return error carriers.
        self.emit_line("function __moth_map_contains(map, key) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return map.map.has(__moth_map_key(key));");
        });
        self.emit_line("}");
        self.emit_line("");

        // Fallible mutation: only validates the receiver; the key is always inserted.
        self.emit_line("function __moth_map_set(map, key, value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!__moth_map_is_valid(map)) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return __moth_error_result(\"{invalid_map_message}\", {invalid_map_code});",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("map.map.set(__moth_map_key(key), value);");
            emitter.emit_line("return { tag: \"ok\", value: null };");
        });
        self.emit_line("}");
        self.emit_line("");

        // Fallible mutation: missing key -> MapKeyNotFound.
        self.emit_line("function __moth_map_remove(map, key) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!__moth_map_is_valid(map)) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return __moth_error_result(\"{invalid_map_message}\", {invalid_map_code});",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("key = __moth_map_key(key);");
            emitter.emit_line("if (!map.map.has(key)) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return __moth_error_result(\"{key_not_found_message}\", {key_not_found_code});",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("const removed = map.map.get(key);");
            emitter.emit_line("map.map.delete(key);");
            emitter.emit_line("return { tag: \"ok\", value: removed };");
        });
        self.emit_line("}");
        self.emit_line("");

        // Infallible mutation: clears every entry without returning an error carrier.
        self.emit_line("function __moth_map_clear(map) {");
        self.with_indent(|emitter| {
            emitter.emit_line("map.map.clear();");
        });
        self.emit_line("}");
        self.emit_line("");

        // Infallible query: returns the entry count.
        self.emit_line("function __moth_map_length(map) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return map.map.size;");
        });
        self.emit_line("}");
        self.emit_line("");
    }
}
