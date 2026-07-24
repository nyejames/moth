//! Collection helpers for the JS runtime.
//!
//! WHAT: runtime contracts for ordered collections, including fixed-capacity collections.
//! WHY: fallible collection operations return structured carriers, while infallible operations
//! stay plain JS helpers so the backend surface matches the language semantics.
//!
//! Collection representations:
//! - Growable collections are plain JS arrays.
//! - Fixed collections are branded `{ __moth_kind, items, fixedCapacity }` wrappers created by
//!   `__moth_fixed_collection`.
//!
//! Semantic policy:
//! - `get`, `set`, and `remove` return `{ tag: "ok", value: ... }` or `{ tag: "err", value: ... }`.
//! - `push` is a fallible helper returning `{ tag: "ok", value: null }` on success.
//! - `length` is an infallible helper returning logical item count.
//! - `get`, `set`, and `remove` validate receivers with
//!   `BuiltinErrorCode::CollectionExpectedOrderedCollection`.
//! - Invalid index or out-of-bounds (get, set, remove) → `BuiltinErrorCode::CollectionIndexOutOfBounds`.
//!   This includes non-integer indices, negative indices, and `index >= length`.
//! - Fixed-capacity push when full → `BuiltinErrorCode::CollectionFixedCapacityExceeded`.

use crate::backends::js::JsEmitter;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_runtime_collection_helpers(&mut self) {
        let invalid_collection = BuiltinErrorCode::CollectionExpectedOrderedCollection;
        let invalid_collection_code = invalid_collection.as_i32();
        let invalid_collection_message = invalid_collection.default_message();

        let out_of_bounds = BuiltinErrorCode::CollectionIndexOutOfBounds;
        let out_of_bounds_code = out_of_bounds.as_i32();
        let out_of_bounds_message = out_of_bounds.default_message();

        let capacity_exceeded = BuiltinErrorCode::CollectionFixedCapacityExceeded;
        let capacity_exceeded_code = capacity_exceeded.as_i32();
        let capacity_exceeded_message = capacity_exceeded.default_message();

        // Fixed collections use a small branded wrapper so runtime helpers can
        // distinguish them from arbitrary objects with `items` fields.
        self.emit_line("function __moth_fixed_collection(items, fixedCapacity) {");
        self.with_indent(|emitter| {
            emitter.emit_line("return {");
            emitter.with_indent(|em| {
                em.emit_line("__moth_kind: \"fixed_collection\",");
                em.emit_line("items: items,");
                em.emit_line("fixedCapacity: fixedCapacity,");
            });
            emitter.emit_line("};");
        });
        self.emit_line("}");
        self.emit_line("");

        // Collection helpers share this accessor so fixed wrappers keep dense
        // array semantics for get, set, push, remove, and length.
        self.emit_line("function __moth_collection_items(collection) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (Array.isArray(collection)) {");
            emitter.with_indent(|em| {
                em.emit_line("return collection;");
            });
            emitter.emit_line("}");
            emitter.emit_line("return collection.items;");
        });
        self.emit_line("}");
        self.emit_line("");

        // Returns the fixed capacity for fixed collections, or null for growable.
        self.emit_line("function __moth_collection_fixed_capacity(collection) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (Array.isArray(collection)) {");
            emitter.with_indent(|em| {
                em.emit_line("return null;");
            });
            emitter.emit_line("}");
            emitter.emit_line("return collection.fixedCapacity;");
        });
        self.emit_line("}");
        self.emit_line("");

        // Fixed-wrapper validation is intentionally stricter than duck typing:
        // malformed external values should use the existing invalid-collection
        // error path instead of corrupting collection semantics.
        self.emit_line("function __moth_collection_is_valid(collection) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (Array.isArray(collection)) {");
            emitter.with_indent(|em| {
                em.emit_line("return true;");
            });
            emitter.emit_line("}");
            emitter.emit_line("if (collection === null || typeof collection !== \"object\") {");
            emitter.with_indent(|em| {
                em.emit_line("return false;");
            });
            emitter.emit_line("}");
            emitter.emit_line("if (collection.__moth_kind !== \"fixed_collection\") {");
            emitter.with_indent(|em| {
                em.emit_line("return false;");
            });
            emitter.emit_line("}");
            emitter.emit_line("if (!Array.isArray(collection.items)) {");
            emitter.with_indent(|em| {
                em.emit_line("return false;");
            });
            emitter.emit_line("}");
            emitter.emit_line("return (");
            emitter.with_indent(|em| {
                em.emit_line("Number.isInteger(collection.fixedCapacity)");
                em.emit_line("&& collection.fixedCapacity > 0");
                em.emit_line("&& collection.items.length <= collection.fixedCapacity");
            });
            emitter.emit_line(");");
        });
        self.emit_line("}");
        self.emit_line("");

        // Validates that `index` is an integer within the logical item bounds.
        // Works with both growable arrays and fixed wrappers via `__moth_collection_items`.
        self.emit_line("function __moth_collection_index_is_valid(collection, index) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const items = __moth_collection_items(collection);");
            emitter
                .emit_line("return Number.isInteger(index) && index >= 0 && index < items.length;");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_collection_get(collection, index) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!__moth_collection_is_valid(collection)) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return __moth_error_result(\"{invalid_collection_message}\", {invalid_collection_code});",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("if (!__moth_collection_index_is_valid(collection, index)) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return __moth_error_result(\"{out_of_bounds_message}\", {out_of_bounds_code});",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("const items = __moth_collection_items(collection);");
            emitter.emit_line("return { tag: \"ok\", value: items[index] };");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_collection_set(collection, index, value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!__moth_collection_is_valid(collection)) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return __moth_error_result(\"{invalid_collection_message}\", {invalid_collection_code});",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("if (!__moth_collection_index_is_valid(collection, index)) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return __moth_error_result(\"{out_of_bounds_message}\", {out_of_bounds_code});",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("const items = __moth_collection_items(collection);");
            emitter.emit_line("items[index] = value;");
            emitter.emit_line("return { tag: \"ok\", value: null };");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_collection_push(collection, value) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!__moth_collection_is_valid(collection)) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return __moth_error_result(\"{invalid_collection_message}\", {invalid_collection_code});",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("const items = __moth_collection_items(collection);");
            emitter.emit_line("const fixedCapacity = __moth_collection_fixed_capacity(collection);");
            emitter.emit_line("if (fixedCapacity !== null && items.length >= fixedCapacity) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return __moth_error_result(\"{capacity_exceeded_message}\", {capacity_exceeded_code});",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("items.push(value);");
            emitter.emit_line("return { tag: \"ok\", value: null };");
        });
        self.emit_line("}");
        self.emit_line("");

        self.emit_line("function __moth_collection_remove(collection, index) {");
        self.with_indent(|emitter| {
            emitter.emit_line("if (!__moth_collection_is_valid(collection)) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return __moth_error_result(\"{invalid_collection_message}\", {invalid_collection_code});",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("if (!__moth_collection_index_is_valid(collection, index)) {");
            emitter.with_indent(|em| {
                em.emit_line(&format!(
                    "return __moth_error_result(\"{out_of_bounds_message}\", {out_of_bounds_code});",
                ));
            });
            emitter.emit_line("}");
            emitter.emit_line("const items = __moth_collection_items(collection);");
            emitter.emit_line("const removed = items.splice(index, 1)[0];");
            emitter.emit_line("return { tag: \"ok\", value: removed };");
        });
        self.emit_line("}");
        self.emit_line("");

        // Returns the logical item count, not the fixed capacity.
        self.emit_line("function __moth_collection_length(collection) {");
        self.with_indent(|emitter| {
            emitter.emit_line("const items = __moth_collection_items(collection);");
            emitter.emit_line("return items.length;");
        });
        self.emit_line("}");
        self.emit_line("");
    }
}
