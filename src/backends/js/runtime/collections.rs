//! Collection helpers for the JS runtime.
//!
//! WHAT: runtime contracts for ordered collections, including fixed-capacity collections.
//! WHY: operations with recoverable `Error!` paths return structured carriers, while operations
//! without that source-visible path stay plain JS helpers so the backend surface matches the
//! language semantics.
//!
//! Collection representations:
//! - Growable collections are plain JS arrays.
//! - Fixed collections are branded `{ __moth_kind, items, fixedCapacity }` wrappers created by
//!   `__moth_fixed_collection`.
//!
//! Semantic policy:
//! - `get`, `set`, and `remove` return `{ tag: "ok", value: ... }` or `{ tag: "err", value: ... }`.
//! - Growable `push` has no recoverable source-visible `Error!` path and mutates the array
//!   directly with no result carrier; allocation exhaustion may still trap or abort.
//! - Fixed `push` has a recoverable source-visible `Error!` path at capacity and returns
//!   `{ tag: "ok", value: null }` on success.
//! - `length` has no recoverable source-visible `Error!` path and returns logical item count.
//! - `get`, `set`, and `remove` validate receivers with
//!   `BuiltinErrorCode::CollectionExpectedOrderedCollection`; fixed `push` rejects growable
//!   arrays with the same error through a strict branded-wrapper check.
//! - Invalid index or out-of-bounds (get, set, remove) → `BuiltinErrorCode::CollectionIndexOutOfBounds`.
//!   This includes non-integer indices, negative indices, and `index >= length`.
//! - Fixed-capacity push when full → `BuiltinErrorCode::CollectionFixedCapacityExceeded`.
//!
//! These helpers are the compiler-owned JavaScript implementation of `@core/collections`.
//! Emission and first-party dependency validation consume the same source from
//! [`collection_javascript_helpers`].

use crate::backends::js::JsEmitter;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;

/// One emitted JavaScript helper implementing the compiler-owned `@core/collections` package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CollectionJsHelper {
    pub(crate) name: &'static str,
    pub(crate) source: String,
}

fn error_result_source(error: BuiltinErrorCode) -> String {
    format!(
        "__moth_error_result(\"{}\", {})",
        error.default_message(),
        error.as_i32()
    )
}

/// Returns the complete `@core/collections` JavaScript source consumed by both emission and
/// first-party dependency validation.
pub(crate) fn collection_javascript_helpers() -> Vec<CollectionJsHelper> {
    let invalid_collection_error =
        error_result_source(BuiltinErrorCode::CollectionExpectedOrderedCollection);
    let out_of_bounds_error = error_result_source(BuiltinErrorCode::CollectionIndexOutOfBounds);
    let capacity_exceeded_error =
        error_result_source(BuiltinErrorCode::CollectionFixedCapacityExceeded);

    vec![
        CollectionJsHelper {
            name: "__moth_fixed_collection",
            source: r#"function __moth_fixed_collection(items, fixedCapacity) {
    return {
        __moth_kind: "fixed_collection",
        items: items,
        fixedCapacity: fixedCapacity,
    };
}"#
            .to_owned(),
        },
        CollectionJsHelper {
            name: "__moth_collection_items",
            source: r#"function __moth_collection_items(collection) {
    if (Array.isArray(collection)) {
        return collection;
    }
    return collection.items;
}"#
            .to_owned(),
        },
        CollectionJsHelper {
            name: "__moth_collection_is_valid",
            source: r#"function __moth_collection_is_valid(collection) {
    if (Array.isArray(collection)) {
        return true;
    }
    if (collection === null || typeof collection !== "object") {
        return false;
    }
    if (collection.__moth_kind !== "fixed_collection") {
        return false;
    }
    if (!Array.isArray(collection.items)) {
        return false;
    }
    return (
        Number.isInteger(collection.fixedCapacity)
        && collection.fixedCapacity > 0
        && collection.items.length <= collection.fixedCapacity
    );
}"#
            .to_owned(),
        },
        CollectionJsHelper {
            name: "__moth_collection_index_is_valid",
            source: r#"function __moth_collection_index_is_valid(collection, index) {
    const items = __moth_collection_items(collection);
    return Number.isInteger(index) && index >= 0 && index < items.length;
}"#
            .to_owned(),
        },
        CollectionJsHelper {
            name: "__moth_collection_get",
            source: format!(
                r#"function __moth_collection_get(collection, index) {{
    if (!__moth_collection_is_valid(collection)) {{
        return {invalid_collection_error};
    }}
    if (!__moth_collection_index_is_valid(collection, index)) {{
        return {out_of_bounds_error};
    }}
    const items = __moth_collection_items(collection);
    return {{ tag: "ok", value: items[index] }};
}}"#,
                invalid_collection_error = invalid_collection_error,
                out_of_bounds_error = out_of_bounds_error,
            ),
        },
        CollectionJsHelper {
            name: "__moth_collection_set",
            source: format!(
                r#"function __moth_collection_set(collection, index, value) {{
    if (!__moth_collection_is_valid(collection)) {{
        return {invalid_collection_error};
    }}
    if (!__moth_collection_index_is_valid(collection, index)) {{
        return {out_of_bounds_error};
    }}
    const items = __moth_collection_items(collection);
    items[index] = value;
    return {{ tag: "ok", value: null }};
}}"#,
                invalid_collection_error = invalid_collection_error,
                out_of_bounds_error = out_of_bounds_error,
            ),
        },
        CollectionJsHelper {
            name: "__moth_collection_push_growable",
            source: r#"function __moth_collection_push_growable(collection, value) {
    collection.push(value);
}"#
            .to_owned(),
        },
        CollectionJsHelper {
            name: "__moth_collection_push_fixed",
            source: format!(
                r#"function __moth_collection_push_fixed(collection, value) {{
    if (Array.isArray(collection) || !__moth_collection_is_valid(collection)) {{
        return {invalid_collection_error};
    }}
    const items = __moth_collection_items(collection);
    if (items.length >= collection.fixedCapacity) {{
        return {capacity_exceeded_error};
    }}
    items.push(value);
    return {{ tag: "ok", value: null }};
}}"#,
                invalid_collection_error = invalid_collection_error,
                capacity_exceeded_error = capacity_exceeded_error,
            ),
        },
        CollectionJsHelper {
            name: "__moth_collection_remove",
            source: format!(
                r#"function __moth_collection_remove(collection, index) {{
    if (!__moth_collection_is_valid(collection)) {{
        return {invalid_collection_error};
    }}
    if (!__moth_collection_index_is_valid(collection, index)) {{
        return {out_of_bounds_error};
    }}
    const items = __moth_collection_items(collection);
    const removed = items.splice(index, 1)[0];
    return {{ tag: "ok", value: removed }};
}}"#,
                invalid_collection_error = invalid_collection_error,
                out_of_bounds_error = out_of_bounds_error,
            ),
        },
        CollectionJsHelper {
            name: "__moth_collection_length",
            source: r#"function __moth_collection_length(collection) {
    const items = __moth_collection_items(collection);
    return items.length;
}"#
            .to_owned(),
        },
    ]
}

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_runtime_collection_helpers(&mut self) {
        for helper in collection_javascript_helpers() {
            self.emit_javascript_source(&helper.source);
            self.emit_line("");
        }
    }
}
