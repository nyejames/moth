//! Glue module source generation for HTML JS external exports.
//!
//! WHAT: generates ES module source that imports raw JS exports and re-exports stable wrapper
//!       functions, including fallible result-shape validation.
//! WHY: the JS backend calls wrappers by stable names; wrappers adapt raw JS return shapes
//!      to Moth's internal conventions.

use crate::backends::js::{
    JsNumericCarrier, builtin_error_code_js_field_name, builtin_error_message_js_field_name,
    external_module_export_glue_function_name,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalPackageId, ExternalSignatureType,
};
use crate::projects::html_project::external_js::runtime_glue::exports::ReferencedExport;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::precision::BinaryFloatPrecision;
use std::collections::HashMap;
use std::fmt::Write as _;

/// The largest integer a foreign JavaScript `Number` represents exactly, `2^53 - 1`.
///
/// `I64`/`U64` keep the exact BigInt carrier inside Moth and cross the foreign boundary as
/// Numbers only inside this window. The JS backend's carrier selection uses the same exactness
/// threshold, but the bridge window written into generated checks is owned here.
const MAX_SAFE_FOREIGN_INTEGER: i128 = (1_i128 << 53) - 1;

/// Generate the glue module ES module source.
pub(super) fn generate_glue_module_source(
    exports: &[ReferencedExport],
    package_asset_paths: &HashMap<ExternalPackageId, String>,
    release_build: bool,
) -> Result<String, CompilerError> {
    let mut source = String::new();

    // Group imports by asset path so we emit one import statement per asset.
    let mut imports_by_path: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for export in exports {
        let path = package_asset_paths.get(&export.package_id).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "HTML JS glue could not find a runtime asset for external package {:?}.",
                export.package_id
            ))
        })?;
        imports_by_path
            .entry(path.clone())
            .or_default()
            .push((export.export_name.clone(), export.raw_import_name.clone()));
    }

    // Emit import statements.
    let mut sorted_paths: Vec<_> = imports_by_path.keys().cloned().collect();
    sorted_paths.sort();
    for path in sorted_paths {
        let mut names = imports_by_path.get(&path).cloned().unwrap_or_default();
        names.sort();
        names.dedup();
        let import_names = names
            .iter()
            .map(|(export_name, local_name)| format!("{export_name} as {local_name}"))
            .collect::<Vec<_>>();
        source.push_str(&format!(
            "import {{ {} }} from \"{}\";\n",
            import_names.join(", "),
            path
        ));
    }

    // Emit wrapper functions.
    for export in exports {
        let wrapper_name = external_module_export_glue_function_name(export.function_id);
        source.push('\n');

        if export.is_fallible {
            source.push_str(&generate_fallible_wrapper(
                &wrapper_name,
                &export.raw_import_name,
                release_build,
                &export.parameter_types,
                &export.return_types,
            )?);
        } else {
            source.push_str(&generate_infallible_wrapper(
                &wrapper_name,
                &export.raw_import_name,
                &export.parameter_types,
                &export.return_types,
            )?);
        }
    }

    Ok(source)
}

/// Generates an infallible wrapper and adapts the raw export to its registered signature.
pub(super) fn generate_infallible_wrapper(
    wrapper_name: &str,
    export_name: &str,
    parameter_types: &[ExternalSignatureType],
    return_types: &[ExternalSignatureType],
) -> Result<String, CompilerError> {
    validate_supported_signature(wrapper_name, parameter_types, return_types)?;
    let arguments = prepare_call_arguments(parameter_types);
    let return_adapter = return_adapter_for_signature(return_types);
    let return_body = adapted_return_body(
        return_adapter,
        &format!("{export_name}({})", arguments.arguments),
        "    ",
        false,
    );

    Ok(format!(
        "export function {wrapper_name}({}) {{\n{}{return_body}\n}}",
        arguments.parameters, arguments.prelude,
    ))
}

/// Generates a fallible wrapper that validates the external result shape and converts it to
/// Moth's JavaScript ABI envelope.
///
/// WHAT: calls the raw JS export, expects `{ ok: boolean, value? }` or `{ ok: false, error }`
///       and returns `{ tag: "ok", value: ... }` or `{ tag: "err", value: ... }` with a Moth `Error`.
///       The foreign `error.code` is read once and projected onto the builtin `U32` code domain:
///       exact JS Number integers in `0..=4294967295` pass,
///       integer `-0` normalises to `0` and every other value (negative, oversized,
///       non-finite, fractional, non-Number, missing) falls back to the zero code.
/// WHY: the JS boundary consumes this ABI envelope and `Error.code` uses the exact Number
///      representation of `U32`.
pub(super) fn generate_fallible_wrapper(
    wrapper_name: &str,
    export_name: &str,
    release_build: bool,
    parameter_types: &[ExternalSignatureType],
    return_types: &[ExternalSignatureType],
) -> Result<String, CompilerError> {
    validate_supported_signature(wrapper_name, parameter_types, return_types)?;
    let arguments = prepare_call_arguments(parameter_types);
    let return_adapter = return_adapter_for_signature(return_types);
    let zero_code = "0";
    // The single `error.code` snapshot is validated against the builtin U32 code domain, and
    // integer -0 canonicalizes at ingress like every other integer boundary in these wrappers.
    let returned_error_code = "Number.isSafeInteger(errorCode) && errorCode >= 0 && errorCode <= 4294967295 ? (errorCode === 0 ? 0 : errorCode) : 0";
    let invalid_error = internal_error_object_source(
        "\"Invalid result wrapper from external JavaScript function\"",
        zero_code,
        release_build,
    );
    let catch_error =
        internal_error_object_source("String(e?.message || e)", zero_code, release_build);
    let returned_error =
        internal_error_object_source("errorMessage", returned_error_code, release_build);
    let invalid_wrapper_handling = if release_build {
        format!("        return {{ tag: \"err\", value: {invalid_error} }};")
    } else {
        format!(
            "        throw new Error(\n            \"Invalid result wrapper from external function '{wrapper_name}': \" +\n            \"expected {{ ok: boolean, value? }} or {{ ok: false, error: {{ code, message }} }}\"\n        );"
        )
    };
    let success_return = adapted_return_body(return_adapter, "result.value", "            ", true);

    Ok(format!(
        "export function {wrapper_name}({}) {{\n{}    let result;\n    try {{\n        result = {export_name}({});\n    }} catch (e) {{\n        return {{ tag: \"err\", value: {catch_error} }};\n    }}\n\n    if (result && typeof result.ok === \"boolean\") {{\n        if (result.ok === true) {{\n{success_return}\n        }}\n        if (result.ok === false) {{\n            const error = result.error || {{ message: \"Unknown error\", code: 0 }};\n            const errorMessage = error.message || \"Unknown error\";\n            const errorCode = error.code;\n            return {{ tag: \"err\", value: {returned_error} }};\n        }}\n    }}\n\n{invalid_wrapper_handling}\n}}",
        arguments.parameters, arguments.prelude, arguments.arguments,
    ))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReturnAdapter {
    Identity,
    Fixed(FixedScalarAdapter),
}

/// How one fixed foreign scalar crosses the HTML-JS boundary.
///
/// WHAT: derived from the JS backend's carrier for that scalar. Exact-Number integers check
///       their own inclusive range, 64-bit integers stay BigInt inside Moth and bridge through
///       a safe foreign Number, and binary floats keep their carrier precision so the
///       parameter check and the HIR binary-float boundary agree on one Number representation.
/// WHY: the backend's `JsNumericCarrier` already owns each scalar's runtime representation, so
///      the glue validates against that one fact instead of a private width table.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FixedScalarAdapter {
    ExactInteger {
        scalar: FixedScalar,
        minimum: i128,
        maximum: i128,
    },
    /// `I64`/`U64`: BigInt inside Moth, safe Number across the foreign boundary.
    SafeNumber64 {
        scalar: FixedScalar,
        /// Inclusive exactly-representable foreign window, clamped to
        /// `MAX_SAFE_FOREIGN_INTEGER`.
        minimum: i128,
        maximum: i128,
    },
    BinaryFloat {
        scalar: FixedScalar,
        precision: BinaryFloatPrecision,
    },
}

impl FixedScalarAdapter {
    /// Selects an adapter for the delivered fixed foreign subset.
    ///
    /// Carrier availability alone does not authorise another binding form. Other fixed scalars
    /// remain unsupported even when the JS backend can represent them internally.
    fn for_scalar(scalar: FixedScalar) -> Option<Self> {
        if !matches!(
            scalar,
            FixedScalar::I32
                | FixedScalar::I64
                | FixedScalar::U32
                | FixedScalar::U64
                | FixedScalar::F32
                | FixedScalar::F64
        ) {
            return None;
        }

        match JsNumericCarrier::for_scalar(NumericScalar::Fixed(scalar))? {
            JsNumericCarrier::ExactInteger { min, max } => Some(Self::ExactInteger {
                scalar,
                minimum: min,
                maximum: max,
            }),
            // The 64-bit integers ride the exact BigInt carrier; only the foreign Number side
            // of the bridge is narrowed to what a Number represents exactly.
            JsNumericCarrier::BigInteger { min, max } => Some(Self::SafeNumber64 {
                scalar,
                minimum: min.max(-MAX_SAFE_FOREIGN_INTEGER),
                maximum: max.min(MAX_SAFE_FOREIGN_INTEGER),
            }),
            JsNumericCarrier::BinaryFloat {
                precision:
                    precision @ (BinaryFloatPrecision::Binary32 | BinaryFloatPrecision::Binary64),
            } => Some(Self::BinaryFloat { scalar, precision }),
            JsNumericCarrier::BinaryFloat { .. } | JsNumericCarrier::ScaledInteger { .. } => None,
        }
    }

    /// Rounds a fixed-float parameter candidate to this precision for the exactness check.
    /// Binary64 Numbers are already exact.
    fn rounded_float_source(precision: BinaryFloatPrecision, value: &str) -> String {
        match precision {
            BinaryFloatPrecision::Binary32 => format!("Math.fround({value})"),
            _ => value.to_owned(),
        }
    }
}

struct PreparedCallArguments {
    parameters: String,
    arguments: String,
    prelude: String,
}

fn validate_supported_signature(
    wrapper_name: &str,
    parameter_types: &[ExternalSignatureType],
    return_types: &[ExternalSignatureType],
) -> Result<(), CompilerError> {
    if parameter_types
        .iter()
        .chain(return_types)
        .any(contains_optional_numeric_payload)
    {
        return Err(CompilerError::compiler_error(format!(
            "HTML JS glue cannot adapt optional numeric signature metadata for external function '{wrapper_name}'."
        )));
    }

    // Fixed scalars must have an exact adapter before any adapter is selected below.
    for signature_type in parameter_types.iter().chain(return_types) {
        if let ExternalSignatureType::Abi(ExternalAbiType::Fixed(scalar)) = signature_type
            && FixedScalarAdapter::for_scalar(*scalar).is_none()
        {
            return Err(CompilerError::compiler_error(format!(
                "HTML JS glue has no JavaScript Number adapter for fixed {} in external function '{wrapper_name}'.",
                scalar.name()
            )));
        }
    }

    if return_types.len() > 1
        && return_types
            .iter()
            .any(|value_type| return_adapter_for_type(value_type) != ReturnAdapter::Identity)
    {
        return Err(CompilerError::compiler_error(format!(
            "HTML JS glue cannot adapt multiple numeric success slots for external function '{wrapper_name}'."
        )));
    }

    Ok(())
}

fn contains_optional_numeric_payload(signature_type: &ExternalSignatureType) -> bool {
    match signature_type {
        ExternalSignatureType::Optional(inner) => {
            is_numeric_signature_type(inner) || contains_optional_numeric_payload(inner)
        }
        _ => false,
    }
}

fn is_numeric_signature_type(signature_type: &ExternalSignatureType) -> bool {
    matches!(
        signature_type,
        ExternalSignatureType::Abi(ExternalAbiType::Fixed(_))
    )
}

fn return_adapter_for_signature(return_types: &[ExternalSignatureType]) -> ReturnAdapter {
    return_types
        .first()
        .map(return_adapter_for_type)
        .unwrap_or(ReturnAdapter::Identity)
}

fn return_adapter_for_type(value_type: &ExternalSignatureType) -> ReturnAdapter {
    match value_type {
        ExternalSignatureType::Abi(ExternalAbiType::Fixed(scalar)) => ReturnAdapter::Fixed(
            FixedScalarAdapter::for_scalar(*scalar)
                .expect("validate_supported_signature rejects fixed scalars without an adapter"),
        ),
        _ => ReturnAdapter::Identity,
    }
}

fn prepare_call_arguments(parameter_types: &[ExternalSignatureType]) -> PreparedCallArguments {
    let mut parameters = String::new();
    let mut arguments = String::new();
    let mut prelude = String::new();

    for (index, parameter_type) in parameter_types.iter().enumerate() {
        if index > 0 {
            parameters.push_str(", ");
            arguments.push_str(", ");
        }
        write!(&mut parameters, "arg{index}")
            .expect("writing generated wrapper parameters into a String cannot fail");

        let fixed_adapter = match parameter_type {
            ExternalSignatureType::Abi(ExternalAbiType::Fixed(scalar)) => {
                FixedScalarAdapter::for_scalar(*scalar)
            }
            _ => None,
        };

        if let Some(adapter @ FixedScalarAdapter::SafeNumber64 { .. }) = fixed_adapter {
            // Fixed I64 and U64 travel as BigInt inside Moth JS but cross to foreign JS as a
            // Number. The guard rejects wrong carriers, negatives on the unsigned lane and
            // unsafe magnitudes before precision can be lost or the raw export observes them.
            prelude.push_str(&safe_number64_parameter_check(adapter, index));
            write!(&mut arguments, "__moth_arg{index}_number")
                .expect("writing generated wrapper arguments into a String cannot fail");
        } else {
            if let ExternalSignatureType::Abi(ExternalAbiType::Fixed(scalar)) = parameter_type {
                match scalar {
                    FixedScalar::I32 => {
                        writeln!(
                            &mut prelude,
                            "    if (typeof arg{index} !== \"number\" || !Number.isInteger(arg{index}) || arg{index} < -2147483648 || arg{index} > 2147483647) {{\n        throw new RangeError(\"External I32 parameter is outside signed 32-bit range\");\n    }}"
                        )
                        .expect("writing generated wrapper checks into a String cannot fail");
                    }
                    // Existing F64 parameters keep their pass-through policy.
                    FixedScalar::F64 => {}
                    _ => {
                        let adapter = fixed_adapter.expect(
                            "validate_supported_signature rejects unsupported fixed scalars",
                        );
                        prelude.push_str(&fixed_parameter_check(adapter, index));
                    }
                }
            }
            write!(&mut arguments, "arg{index}")
                .expect("writing generated wrapper arguments into a String cannot fail");
        }
    }

    PreparedCallArguments {
        parameters,
        arguments,
        prelude,
    }
}

fn adapted_return_body(
    adapter: ReturnAdapter,
    value_expression: &str,
    indent: &str,
    fallible: bool,
) -> String {
    let success_value = |expression: &str| {
        if fallible {
            format!("{{ tag: \"ok\", value: {expression} }}")
        } else {
            expression.to_owned()
        }
    };

    match adapter {
        // A fixed binary-float success value crosses as the raw foreign carrier: the HIR binary-float
        // boundary owns the single rounding and finite check at the resolved source precision,
        // so glue must not round or reject the snapshot first. The float adapter stays distinct
        // from `Identity` so multi-slot returns, which the single-slot HIR guard cannot cover,
        // are still rejected during signature validation.
        ReturnAdapter::Identity | ReturnAdapter::Fixed(FixedScalarAdapter::BinaryFloat { .. }) => {
            format!("{indent}return {};", success_value(value_expression))
        }
        // Fixed I32 and U32 are exact-Number integers that share one value-gate shape, not a
        // semantic identity. Integer zero must be canonical before a later float conversion can
        // observe its sign. Only the bounds and the message differ: the unsigned gate rejects
        // negatives where the signed gate admits them.
        ReturnAdapter::Fixed(FixedScalarAdapter::ExactInteger { .. }) => {
            let (boundary_message, minimum, maximum): (&str, i128, i128) = match adapter {
                ReturnAdapter::Fixed(FixedScalarAdapter::ExactInteger {
                    scalar,
                    minimum,
                    maximum,
                }) => (
                    if scalar == FixedScalar::I32 {
                        "External I32 result is outside signed 32-bit range"
                    } else {
                        "External U32 result is not an integer within the unsigned 32-bit range"
                    },
                    minimum,
                    maximum,
                ),
                _ => unreachable!("only exact-Number fixed integers use the integer result gate"),
            };
            let returned_value = success_value("__moth_external_canonical_integer");
            format!(
                "{indent}const __moth_external_integer = {value_expression};\n{indent}if (typeof __moth_external_integer !== \"number\" || !Number.isInteger(__moth_external_integer) || __moth_external_integer < {minimum} || __moth_external_integer > {maximum}) {{\n{indent}    throw new RangeError(\"{boundary_message}\");\n{indent}}}\n{indent}const __moth_external_canonical_integer = __moth_external_integer === 0 ? 0 : __moth_external_integer;\n{indent}return {returned_value};"
            )
        }
        // Fixed I64 and U64 share one safe-Number bridge gate: check bounds before BigInt
        // conversion, since safe JavaScript integers are a strict subset of either Moth range.
        // BigInt(number) already canonicalizes integer -0. The unsigned lane narrows the bridge
        // to non-negative safe integers and never extends the Moth Uint64 range. Both use the
        // exactly-representable window.
        ReturnAdapter::Fixed(FixedScalarAdapter::SafeNumber64 { .. }) => {
            let returned_value = success_value("__moth_external_integer");
            let (boundary_message, minimum, maximum): (&str, i128, i128) = match adapter {
                ReturnAdapter::Fixed(FixedScalarAdapter::SafeNumber64 {
                    scalar: FixedScalar::U64,
                    minimum,
                    maximum,
                }) => (
                    "External U64 result is not a safe integer within the foreign Number range",
                    minimum,
                    maximum,
                ),
                ReturnAdapter::Fixed(FixedScalarAdapter::SafeNumber64 {
                    scalar: FixedScalar::I64,
                    minimum,
                    maximum,
                }) => (
                    "External I64 result is not a safe integer within the foreign Number range",
                    minimum,
                    maximum,
                ),
                _ => unreachable!("only the 64-bit lanes use the safe-Number result gate"),
            };
            safe_number64_result_gate(
                value_expression,
                minimum,
                maximum,
                boundary_message,
                indent,
                &returned_value,
            )
        }
    }
}

/// Validates a fixed `U32` or `F32` argument before the foreign call observes it.
///
/// Moth already produces valid fixed values. Direct wrapper misuse must not coerce a foreign
/// carrier: `U32` canonicalises integer zero and `F32` must be exact while preserving signed zero.
/// `I32`/`F64` keep their own pass-through or inline checks, and the 64-bit integers use the
/// BigInt ingress bridge instead of a foreign Number carrier.
fn fixed_parameter_check(adapter: FixedScalarAdapter, index: usize) -> String {
    let argument = format!("arg{index}");
    match adapter {
        FixedScalarAdapter::ExactInteger {
            scalar,
            minimum,
            maximum,
        } => {
            let name = scalar.name();
            format!(
                "    if (typeof {argument} !== \"number\" || !Number.isInteger({argument}) || {argument} < {minimum} || {argument} > {maximum}) {{\n        throw new RangeError(\"External {name} parameter is not an integer within the {name} range\");\n    }}\n    {argument} = {argument} === 0 ? 0 : {argument};\n"
            )
        }
        FixedScalarAdapter::BinaryFloat { scalar, precision } => {
            let name = scalar.name();
            let rounded = FixedScalarAdapter::rounded_float_source(precision, &argument);
            format!(
                "    if (typeof {argument} !== \"number\" || !Number.isFinite({argument}) || {rounded} !== {argument}) {{\n        throw new RangeError(\"External {name} parameter is not a finite {name} value\");\n    }}\n"
            )
        }
        FixedScalarAdapter::SafeNumber64 { .. } => {
            unreachable!("fixed 64-bit parameters use the safe-Number bridge")
        }
    }
}

/// Emits the BigInt-to-safe-Number ingress guard for one fixed `I64`/`U64` parameter.
///
/// WHAT: requires the Moth BigInt carrier, rejects magnitudes whose conversion to a foreign
///       Number would lose precision, and leaves the safe Number in `__moth_arg{index}_number`
///       for the call.
/// WHY: the guard runs before the raw export call, so an unbridgeable carrier or an unsafe
///      magnitude never reaches foreign code and never loses precision on the way.
fn safe_number64_parameter_check(adapter: FixedScalarAdapter, index: usize) -> String {
    let FixedScalarAdapter::SafeNumber64 {
        scalar,
        minimum,
        maximum,
    } = adapter
    else {
        unreachable!("only I64 and U64 use the fixed safe-Number parameter bridge");
    };

    let name = scalar.name();
    let argument = format!("arg{index}");
    let number = format!("__moth_arg{index}_number");
    let carrier_check = format!(
        "    if (typeof {argument} !== \"bigint\") {{\n        throw new RangeError(\"External {name} parameter must be a BigInt\");\n    }}\n"
    );
    // The unsigned lane names its lower bound explicitly, so a negative BigInt fails with the
    // range message before any conversion; the signed lane's `Number.isSafeInteger` is
    // symmetric and already owns that failure.
    let window_check = if scalar == FixedScalar::U64 {
        format!(
            "    if ({argument} < {minimum}n || {argument} > {maximum}n) {{\n        throw new RangeError(\"External U64 parameter is outside the safe Number range\");\n    }}\n"
        )
    } else {
        String::new()
    };

    format!(
        "{carrier_check}{window_check}    const {number} = Number({argument});\n    if (!Number.isSafeInteger({number})) {{\n        throw new RangeError(\"External {name} parameter is not exactly representable as a JavaScript Number\");\n    }}\n"
    )
}

/// Gates one 64-bit integer result crossing back from a foreign Number.
///
/// WHAT: evaluates the foreign value once, checks it is a Number inside the inclusive bridge
///       window, and only then converts it to the Moth BigInt carrier. `BigInt(number)`
///       canonicalizes integer `-0`.
/// WHY: the gate is fatal in both wrapper lanes because a foreign contract violation must not
///      become a new implicit failure or `Error` code.
fn safe_number64_result_gate(
    value_expression: &str,
    minimum: i128,
    maximum: i128,
    boundary_message: &str,
    indent: &str,
    returned_value: &str,
) -> String {
    format!(
        "{indent}const __moth_external_number = {value_expression};\n{indent}if (typeof __moth_external_number !== \"number\" || !Number.isSafeInteger(__moth_external_number) || __moth_external_number < {minimum} || __moth_external_number > {maximum}) {{\n{indent}    throw new RangeError(\"{boundary_message}\");\n{indent}}}\n{indent}const __moth_external_integer = BigInt(__moth_external_number);\n{indent}return {returned_value};"
    )
}

/// Generates an internal Error object with build-specific field names.
fn internal_error_object_source(
    message_expression: &str,
    code_expression: &str,
    release_build: bool,
) -> String {
    let message_field = builtin_error_message_js_field_name(release_build);
    let code_field = builtin_error_code_js_field_name(release_build);

    format!("{{ {message_field}: {message_expression}, {code_field}: {code_expression} }}")
}
