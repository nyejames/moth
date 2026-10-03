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
use crate::backends::js::numeric_carrier::JsNumericCarrier;
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use moth_lexical::numeric::profile::NumericProfile;

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
        error.as_u32()
    )
}

/// Returns the complete `@core/collections` JavaScript source for the selected Int carrier.
/// First-party dependency validation inventories the standard-profile source.
pub(crate) fn collection_javascript_helpers(profile: NumericProfile) -> Vec<CollectionJsHelper> {
    let carrier = JsNumericCarrier::for_scalar(NumericScalar::Int, profile)
        .expect("Int always has a JavaScript numeric carrier");
    let (int_min, int_max) = carrier
        .integer_bounds_js()
        .expect("Int carrier always has integer bounds");
    let zero = JsNumericCarrier::int_literal(0, profile)
        .expect("zero always fits the Int numeric profile");

    let invalid_collection_error =
        error_result_source(BuiltinErrorCode::CollectionExpectedOrderedCollection);
    let out_of_bounds_error = error_result_source(BuiltinErrorCode::CollectionIndexOutOfBounds);
    let capacity_exceeded_error =
        error_result_source(BuiltinErrorCode::CollectionFixedCapacityExceeded);

    let (capacity_validation, index_validation, index_access, capacity_full, length_value) =
        match carrier {
            JsNumericCarrier::ExactInteger { .. } => (
                "Number.isInteger(collection.fixedCapacity)\n        && collection.fixedCapacity > 0\n        && collection.items.length <= collection.fixedCapacity".to_owned(),
                "Number.isInteger(index) && index >= 0 && index < items.length".to_owned(),
                "index",
                "items.length >= collection.fixedCapacity",
                "items.length",
            ),
            JsNumericCarrier::BigInteger { .. } => (
                format!(
                    "typeof collection.fixedCapacity === \"bigint\"\n        && collection.fixedCapacity >= {int_min}\n        && collection.fixedCapacity <= {int_max}\n        && collection.fixedCapacity > {zero}\n        && BigInt(collection.items.length) <= collection.fixedCapacity"
                ),
                format!(
                    "typeof index === \"bigint\" && index >= {int_min} && index <= {int_max} && index >= {zero} && index < BigInt(items.length)"
                ),
                "Number(index)",
                "BigInt(items.length) >= collection.fixedCapacity",
                "BigInt(items.length)",
            ),
            JsNumericCarrier::BinaryFloat { .. } => {
                unreachable!("Int carrier cannot be a binary float")
            }
            JsNumericCarrier::ScaledInteger { .. } => {
                unreachable!("Int carrier cannot be a Number")
            }
        };

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
            source: format!(
                r#"function __moth_collection_is_valid(collection) {{
    if (Array.isArray(collection)) {{
        return true;
    }}
    if (collection === null || typeof collection !== "object") {{
        return false;
    }}
    if (collection.__moth_kind !== "fixed_collection") {{
        return false;
    }}
    if (!Array.isArray(collection.items)) {{
        return false;
    }}
    return (
        {capacity_validation}
    );
}}"#
            ),
        },
        CollectionJsHelper {
            name: "__moth_collection_index_is_valid",
            source: format!(
                r#"function __moth_collection_index_is_valid(collection, index) {{
    const items = __moth_collection_items(collection);
    return {index_validation};
}}"#
            ),
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
    return {{ tag: "ok", value: items[{index_access}] }};
}}"#,
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
    items[{index_access}] = value;
    return {{ tag: "ok", value: null }};
}}"#,
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
    if ({capacity_full}) {{
        return {capacity_exceeded_error};
    }}
    items.push(value);
    return {{ tag: "ok", value: null }};
}}"#,
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
    const removed = items.splice({index_access}, 1)[0];
    return {{ tag: "ok", value: removed }};
}}"#,
            ),
        },
        CollectionJsHelper {
            name: "__moth_collection_length",
            source: format!(
                r#"function __moth_collection_length(collection) {{
    const items = __moth_collection_items(collection);
    return {length_value};
}}"#
            ),
        },
    ]
}

impl<'hir> JsEmitter<'hir> {
    pub(crate) fn emit_runtime_collection_helpers(&mut self) {
        for helper in collection_javascript_helpers(self.config.numeric_profile) {
            self.emit_javascript_source(&helper.source);
            self.emit_line("");
        }
    }
}
