//! `@core/time` package registration.
//!
//! WHAT: registers the typed time surface for builders that opt into it.
//! WHY: replaces the old ambiguous `now_millis()` / `now_seconds()` API with explicit
//!      monotonic and wall-clock concepts that are safer for games, animations, and
//!      real-world timestamps.
//!
//! Registered types:
//! - `Duration`: signed elapsed amount, represented as milliseconds internally.
//! - `TimeMark`: monotonic clock mark for deltas, profiling, and frame timing.
//! - `Timestamp`: UTC wall-clock instant for logs, storage, and system boundaries.

use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalAccessKind, ExternalFunctionLowerings, ExternalFunctionSpec,
    ExternalJsLowering, ExternalPackageId, ExternalParameter, ExternalReturnSlot,
    ExternalSignatureType, ExternalTypeId, ExternalTypeSpec,
};

/// One registered `@core/time` function.
///
/// WHAT: pairs the published name and signature with its JavaScript lowering.
/// WHY: registration reads as named fields instead of a positional tuple, and the lowering
///      kind is carried by an enum instead of an inline/runtime boolean.
struct TimeFunctionSpec<'a> {
    name: &'static str,
    /// Parameters in published order; every parameter takes shared access.
    parameters: &'a [&'a ExternalSignatureType],
    return_type: &'a ExternalSignatureType,
    /// Set only for the two fallible parse and render helpers.
    error_return_type: Option<&'a ExternalSignatureType>,
    js_lowering: TimeJsLowering,
}

/// JavaScript lowering for one `@core/time` function.
///
/// WHAT: distinguishes inline millisecond expressions from compiler-owned runtime helpers.
/// WHY: the two kinds map to different `ExternalJsLowering` variants and different emission
///      paths.
enum TimeJsLowering {
    /// Inline expression over the millisecond representation, with `#N` substitution.
    Inline(&'static str),
    /// Named compiler-owned runtime helper reached through the shared reachability path.
    Runtime(&'static str),
}

pub fn register_core_time_package(registry: &mut ExternalPackageRegistry) {
    let package_id = registry
        .register_package("@core/time", crate::builder_surface::PackageOrigin::Core)
        .expect("builtin package registration should not collide");

    // ------------------------
    //  Register opaque types
    // ------------------------

    let duration_id = register_external_time_type(registry, package_id, "Duration");
    let time_mark_id = register_external_time_type(registry, package_id, "TimeMark");
    let timestamp_id = register_external_time_type(registry, package_id, "Timestamp");

    let duration_type = ExternalSignatureType::External(duration_id);
    let time_mark_type = ExternalSignatureType::External(time_mark_id);
    let timestamp_type = ExternalSignatureType::External(timestamp_id);
    let f64_type = ExternalSignatureType::Abi(ExternalAbiType::F64);
    let string_type = ExternalSignatureType::Abi(ExternalAbiType::Utf8Str);
    let bool_type = ExternalSignatureType::Abi(ExternalAbiType::Bool);
    let error_type = ExternalSignatureType::BuiltinError;

    // ------------------------
    //  Register free functions
    // ------------------------

    // One row per registered function, grouped as the canonical reference groups them. Inline
    // lowerings keep the published `#0`-substitution form; the two runtime helpers are reached
    // through the reachability path shared with the other optional Core packages.
    let time_functions = [
        // Monotonic clock
        TimeFunctionSpec {
            name: "mark_now",
            parameters: &[],
            return_type: &time_mark_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("globalThis.performance.now()"),
        },
        TimeFunctionSpec {
            name: "elapsed_since",
            parameters: &[&time_mark_type],
            return_type: &duration_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("(globalThis.performance.now() - #0)"),
        },
        TimeFunctionSpec {
            name: "duration_between",
            parameters: &[&time_mark_type, &time_mark_type],
            return_type: &duration_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("(#1 - #0)"),
        },
        // Wall-clock timestamp
        TimeFunctionSpec {
            name: "timestamp_now",
            parameters: &[],
            return_type: &timestamp_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("Date.now()"),
        },
        // Duration construction
        TimeFunctionSpec {
            name: "duration_from_seconds",
            parameters: &[&f64_type],
            return_type: &duration_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("(#0 * 1000.0)"),
        },
        TimeFunctionSpec {
            name: "duration_from_milliseconds",
            parameters: &[&f64_type],
            return_type: &duration_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("#0"),
        },
        // Timestamp construction
        TimeFunctionSpec {
            name: "timestamp_from_unix_seconds",
            parameters: &[&f64_type],
            return_type: &timestamp_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("(#0 * 1000.0)"),
        },
        TimeFunctionSpec {
            name: "timestamp_from_unix_milliseconds",
            parameters: &[&f64_type],
            return_type: &timestamp_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("#0"),
        },
        TimeFunctionSpec {
            name: "timestamp_from_iso_string",
            parameters: &[&string_type],
            return_type: &timestamp_type,
            error_return_type: Some(&error_type),
            js_lowering: TimeJsLowering::Runtime("__moth_time_timestamp_from_iso_string"),
        },
        // Duration helpers
        TimeFunctionSpec {
            name: "as_seconds",
            parameters: &[&duration_type],
            return_type: &f64_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("(#0 / 1000.0)"),
        },
        TimeFunctionSpec {
            name: "as_milliseconds",
            parameters: &[&duration_type],
            return_type: &f64_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("#0"),
        },
        TimeFunctionSpec {
            name: "is_negative",
            parameters: &[&duration_type],
            return_type: &bool_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("(#0 < 0)"),
        },
        TimeFunctionSpec {
            name: "abs",
            parameters: &[&duration_type],
            return_type: &duration_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("Math.abs(#0)"),
        },
        TimeFunctionSpec {
            name: "clamp",
            parameters: &[&duration_type, &duration_type, &duration_type],
            return_type: &duration_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("Math.min(Math.max(#0, #1), #2)"),
        },
        TimeFunctionSpec {
            name: "duration_add",
            parameters: &[&duration_type, &duration_type],
            return_type: &duration_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("(#0 + #1)"),
        },
        TimeFunctionSpec {
            name: "duration_subtract",
            parameters: &[&duration_type, &duration_type],
            return_type: &duration_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("(#0 - #1)"),
        },
        TimeFunctionSpec {
            name: "duration_scale",
            parameters: &[&duration_type, &f64_type],
            return_type: &duration_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("(#0 * #1)"),
        },
        // Timestamp helpers
        TimeFunctionSpec {
            name: "unix_seconds",
            parameters: &[&timestamp_type],
            return_type: &f64_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("(#0 / 1000.0)"),
        },
        TimeFunctionSpec {
            name: "unix_milliseconds",
            parameters: &[&timestamp_type],
            return_type: &f64_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("#0"),
        },
        TimeFunctionSpec {
            name: "to_iso_string",
            parameters: &[&timestamp_type],
            return_type: &string_type,
            error_return_type: Some(&error_type),
            js_lowering: TimeJsLowering::Runtime("__moth_time_to_iso_string"),
        },
        TimeFunctionSpec {
            name: "timestamp_offset",
            parameters: &[&timestamp_type, &duration_type],
            return_type: &timestamp_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("(#0 + #1)"),
        },
        TimeFunctionSpec {
            name: "timestamp_difference",
            parameters: &[&timestamp_type, &timestamp_type],
            return_type: &duration_type,
            error_return_type: None,
            js_lowering: TimeJsLowering::Inline("(#1 - #0)"),
        },
    ];

    for function in time_functions {
        let parameters: Vec<ExternalParameter> = function
            .parameters
            .iter()
            .map(|language_type| ExternalParameter {
                language_type: (*language_type).clone(),
                access_kind: ExternalAccessKind::Shared,
            })
            .collect();
        let returns = vec![ExternalReturnSlot::fresh(function.return_type.clone())];
        let js_lowering = match function.js_lowering {
            TimeJsLowering::Inline(expression) => {
                ExternalJsLowering::InlineExpression(expression.to_owned())
            }
            TimeJsLowering::Runtime(name) => ExternalJsLowering::RuntimeFunction(name.to_owned()),
        };

        registry
            .register_external_function(
                package_id,
                ExternalFunctionSpec {
                    name: function.name.to_owned(),
                    parameters,
                    returns,
                    error_return_type: function.error_return_type.cloned(),
                    lowerings: ExternalFunctionLowerings {
                        js: Some(js_lowering),
                        wasm: None,
                    },
                },
            )
            .expect("builtin time function registration should not collide");
    }
}

// ------------------------
//  Registration helpers
// ------------------------

/// Registers an opaque external type with the Handle ABI.
fn register_external_time_type(
    registry: &mut ExternalPackageRegistry,
    package_id: ExternalPackageId,
    name: &'static str,
) -> ExternalTypeId {
    registry
        .register_external_type(
            package_id,
            ExternalTypeSpec {
                name: name.to_owned(),
                abi_type: ExternalAbiType::Handle,
            },
        )
        .expect("builtin time type registration should not collide")
}
