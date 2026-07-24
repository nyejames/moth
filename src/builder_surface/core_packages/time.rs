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

    // ------------------------
    //  Register free functions
    // ------------------------

    // Monotonic clock

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "mark_now",
            parameters: vec![],
            returns: vec![ExternalReturnSlot::fresh(time_mark_type.clone())],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression(
                "globalThis.performance.now()".to_owned(),
            ),
        },
    );

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "elapsed_since",
            parameters: vec![shared_param(time_mark_type.clone())],
            returns: vec![ExternalReturnSlot::fresh(duration_type.clone())],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression(
                "(globalThis.performance.now() - #0)".to_owned(),
            ),
        },
    );

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "duration_between",
            parameters: vec![
                shared_param(time_mark_type.clone()),
                shared_param(time_mark_type.clone()),
            ],
            returns: vec![ExternalReturnSlot::fresh(duration_type.clone())],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression("(#1 - #0)".to_owned()),
        },
    );

    // Wall-clock timestamp

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "timestamp_now",
            parameters: vec![],
            returns: vec![ExternalReturnSlot::fresh(timestamp_type.clone())],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression("Date.now()".to_owned()),
        },
    );

    // Duration construction

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "duration_from_seconds",
            parameters: vec![shared_param(ExternalSignatureType::Abi(
                ExternalAbiType::F64,
            ))],
            returns: vec![ExternalReturnSlot::fresh(duration_type.clone())],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression("(#0 * 1000.0)".to_owned()),
        },
    );

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "duration_from_milliseconds",
            parameters: vec![shared_param(ExternalSignatureType::Abi(
                ExternalAbiType::F64,
            ))],
            returns: vec![ExternalReturnSlot::fresh(duration_type.clone())],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression("#0".to_owned()),
        },
    );

    // Timestamp construction

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "timestamp_from_unix_seconds",
            parameters: vec![shared_param(ExternalSignatureType::Abi(
                ExternalAbiType::F64,
            ))],
            returns: vec![ExternalReturnSlot::fresh(timestamp_type.clone())],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression("(#0 * 1000.0)".to_owned()),
        },
    );

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "timestamp_from_unix_milliseconds",
            parameters: vec![shared_param(ExternalSignatureType::Abi(
                ExternalAbiType::F64,
            ))],
            returns: vec![ExternalReturnSlot::fresh(timestamp_type.clone())],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression("#0".to_owned()),
        },
    );

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "timestamp_from_iso_string",
            parameters: vec![shared_param(ExternalSignatureType::Abi(
                ExternalAbiType::Utf8Str,
            ))],
            returns: vec![ExternalReturnSlot::fresh(timestamp_type.clone())],
            error_return_type: Some(ExternalSignatureType::BuiltinError),
            js_lowering: ExternalJsLowering::RuntimeFunction(
                "__moth_time_timestamp_from_iso_string".to_owned(),
            ),
        },
    );

    // ------------------------
    //  Duration helpers
    // ------------------------

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "as_seconds",
            parameters: vec![shared_param(duration_type.clone())],
            returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::Abi(
                ExternalAbiType::F64,
            ))],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression("(#0 / 1000.0)".to_owned()),
        },
    );

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "as_milliseconds",
            parameters: vec![shared_param(duration_type.clone())],
            returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::Abi(
                ExternalAbiType::F64,
            ))],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression("#0".to_owned()),
        },
    );

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "is_negative",
            parameters: vec![shared_param(duration_type.clone())],
            returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::Abi(
                ExternalAbiType::Bool,
            ))],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression("(#0 < 0)".to_owned()),
        },
    );

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "abs",
            parameters: vec![shared_param(duration_type.clone())],
            returns: vec![ExternalReturnSlot::fresh(duration_type.clone())],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression("Math.abs(#0)".to_owned()),
        },
    );

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "clamp",
            parameters: vec![
                shared_param(duration_type.clone()),
                shared_param(duration_type.clone()),
                shared_param(duration_type.clone()),
            ],
            returns: vec![ExternalReturnSlot::fresh(duration_type.clone())],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression(
                "Math.min(Math.max(#0, #1), #2)".to_owned(),
            ),
        },
    );

    // ------------------------
    //  Timestamp helpers
    // ------------------------

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "unix_seconds",
            parameters: vec![shared_param(timestamp_type.clone())],
            returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::Abi(
                ExternalAbiType::F64,
            ))],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression("(#0 / 1000.0)".to_owned()),
        },
    );

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "unix_milliseconds",
            parameters: vec![shared_param(timestamp_type.clone())],
            returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::Abi(
                ExternalAbiType::F64,
            ))],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression("#0".to_owned()),
        },
    );

    register_external_time_function(
        registry,
        package_id,
        TimeFunctionSpec {
            name: "to_iso_string",
            parameters: vec![shared_param(timestamp_type.clone())],
            returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::Abi(
                ExternalAbiType::Utf8Str,
            ))],
            error_return_type: None,
            js_lowering: ExternalJsLowering::InlineExpression(
                "(new Date(#0)).toISOString()".to_owned(),
            ),
        },
    );
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

/// Builds a shared-access parameter for the given signature type.
fn shared_param(language_type: ExternalSignatureType) -> ExternalParameter {
    ExternalParameter {
        language_type,
        access_kind: ExternalAccessKind::Shared,
    }
}

/// Local spec for registering one external time function.
///
/// WHAT: collapses the per-function metadata so the registration helper does not need
///       a long argument list.
/// WHY: keeps call sites readable and avoids clippy warnings for too many arguments.
struct TimeFunctionSpec {
    name: &'static str,
    parameters: Vec<ExternalParameter>,
    returns: Vec<ExternalReturnSlot>,
    error_return_type: Option<ExternalSignatureType>,
    js_lowering: ExternalJsLowering,
}

/// Registers a single external function in the time package.
fn register_external_time_function(
    registry: &mut ExternalPackageRegistry,
    package_id: ExternalPackageId,
    spec: TimeFunctionSpec,
) {
    registry
        .register_external_function(
            package_id,
            ExternalFunctionSpec {
                name: spec.name.to_owned(),
                parameters: spec.parameters,
                returns: spec.returns,
                error_return_type: spec.error_return_type,
                lowerings: ExternalFunctionLowerings {
                    js: Some(spec.js_lowering),
                    wasm: None,
                },
            },
        )
        .expect("builtin time function registration should not collide");
}
