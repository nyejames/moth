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
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalPackageId, ExternalSignatureType,
};
use crate::projects::html_project::external_js::runtime_glue::exports::ReferencedExport;
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::precision::BinaryFloatPrecision;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};
use std::collections::HashMap;
use std::fmt::Write as _;

/// Generate the glue module ES module source.
pub(super) fn generate_glue_module_source(
    exports: &[ReferencedExport],
    package_asset_paths: &HashMap<ExternalPackageId, String>,
    release_build: bool,
    numeric_profile: NumericProfile,
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
                numeric_profile,
                &export.parameter_types,
                &export.return_types,
            )?);
        } else {
            source.push_str(&generate_infallible_wrapper(
                &wrapper_name,
                &export.raw_import_name,
                &export.parameter_types,
                &export.return_types,
                numeric_profile,
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
    numeric_profile: NumericProfile,
) -> Result<String, CompilerError> {
    validate_supported_signature(wrapper_name, parameter_types, return_types, numeric_profile)?;
    let arguments = prepare_call_arguments(parameter_types, numeric_profile);
    let return_adapter = return_adapter_for_signature(return_types, numeric_profile);
    let return_body = adapted_return_body(
        return_adapter,
        &format!("{export_name}({})", arguments.arguments),
        numeric_profile,
        "    ",
        false,
        false,
    );

    Ok(format!(
        "export function {wrapper_name}({}) {{\n{}{return_body}\n}}",
        arguments.parameters, arguments.prelude,
    ))
}

/// Generates a fallible wrapper that validates the external result shape and converts it to
/// Moth's internal fallible carrier.
///
/// WHAT: calls the raw JS export, expects `{ ok: boolean, value? }` or `{ ok: false, error }`,
///       and returns `{ tag: "ok", value: ... }` or an internal Moth `Error` struct value. The
///       foreign `error.code` is read once and projected onto the builtin `U32` code domain:
///       exact JS Number integers in `0..=4294967295` pass,
///       integer `-0` normalizes to `0`, and every other value (negative, oversized,
///       non-finite, fractional, non-Number, missing) falls back to the zero code.
/// WHY: the JS backend consumes this carrier shape, and `Error.code` uses the exact Number
///      representation of `U32` independently of the numeric profile.
pub(super) fn generate_fallible_wrapper(
    wrapper_name: &str,
    export_name: &str,
    release_build: bool,
    numeric_profile: NumericProfile,
    parameter_types: &[ExternalSignatureType],
    return_types: &[ExternalSignatureType],
) -> Result<String, CompilerError> {
    validate_supported_signature(wrapper_name, parameter_types, return_types, numeric_profile)?;
    let arguments = prepare_call_arguments(parameter_types, numeric_profile);
    let return_adapter = return_adapter_for_signature(return_types, numeric_profile);
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
    let success_return = adapted_return_body(
        return_adapter,
        "result.value",
        numeric_profile,
        "            ",
        true,
        release_build,
    );

    Ok(format!(
        "export function {wrapper_name}({}) {{\n{}    let result;\n    try {{\n        result = {export_name}({});\n    }} catch (e) {{\n        return {{ tag: \"err\", value: {catch_error} }};\n    }}\n\n    if (result && typeof result.ok === \"boolean\") {{\n        if (result.ok === true) {{\n{success_return}\n        }}\n        if (result.ok === false) {{\n            const error = result.error || {{ message: \"Unknown error\", code: 0 }};\n            const errorMessage = error.message || \"Unknown error\";\n            const errorCode = error.code;\n            return {{ tag: \"err\", value: {returned_error} }};\n        }}\n    }}\n\n{invalid_wrapper_handling}\n}}",
        arguments.parameters, arguments.prelude, arguments.arguments,
    ))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReturnAdapter {
    Identity,
    NativeInt32,
    NativeInt64,
    NativeUint32,
    NativeUint64,
    NativeFloat32,
    Fixed(FixedScalarAdapter),
}

/// How one fixed foreign scalar crosses the HTML-JS boundary.
///
/// WHAT: derived from the JS backend's carrier for that scalar. Exact-Number integers check
///       their own inclusive range, and binary floats round once at their precision and must
///       stay finite.
/// WHY: the backend's `JsNumericCarrier` already owns each scalar's runtime representation, so
///      the glue validates against that one fact instead of a private width table. Fixed
///      scalars never follow the selected `Int` or `Float` profile.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FixedScalarAdapter {
    ExactInteger {
        scalar: FixedScalar,
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
    fn for_scalar(scalar: FixedScalar, numeric_profile: NumericProfile) -> Option<Self> {
        if !matches!(
            scalar,
            FixedScalar::I32 | FixedScalar::U32 | FixedScalar::F32 | FixedScalar::F64
        ) {
            return None;
        }

        match JsNumericCarrier::for_scalar(NumericScalar::Fixed(scalar), numeric_profile)? {
            JsNumericCarrier::ExactInteger { min, max } => Some(Self::ExactInteger {
                scalar,
                minimum: min,
                maximum: max,
            }),
            JsNumericCarrier::BinaryFloat {
                precision:
                    precision @ (BinaryFloatPrecision::Binary32 | BinaryFloatPrecision::Binary64),
            } => Some(Self::BinaryFloat { scalar, precision }),
            JsNumericCarrier::BinaryFloat { .. }
            | JsNumericCarrier::BigInteger { .. }
            | JsNumericCarrier::ScaledInteger { .. } => None,
        }
    }

    /// Rounds a JavaScript Number to this float precision. Binary64 Numbers are already exact.
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
    numeric_profile: NumericProfile,
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
            && FixedScalarAdapter::for_scalar(*scalar, numeric_profile).is_none()
        {
            return Err(CompilerError::compiler_error(format!(
                "HTML JS glue has no JavaScript Number adapter for fixed {} in external function '{wrapper_name}'.",
                scalar.name()
            )));
        }
    }

    if return_types.len() > 1
        && return_types.iter().any(|value_type| {
            return_adapter_for_type(value_type, numeric_profile) != ReturnAdapter::Identity
        })
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
        ExternalSignatureType::NativeInt
            | ExternalSignatureType::NativeUint
            | ExternalSignatureType::NativeFloat
            | ExternalSignatureType::Abi(ExternalAbiType::Fixed(_))
    )
}

fn return_adapter_for_signature(
    return_types: &[ExternalSignatureType],
    numeric_profile: NumericProfile,
) -> ReturnAdapter {
    return_types
        .first()
        .map(|value_type| return_adapter_for_type(value_type, numeric_profile))
        .unwrap_or(ReturnAdapter::Identity)
}

fn return_adapter_for_type(
    value_type: &ExternalSignatureType,
    numeric_profile: NumericProfile,
) -> ReturnAdapter {
    match value_type {
        ExternalSignatureType::NativeInt if numeric_profile.int_width == IntWidth::Bits32 => {
            ReturnAdapter::NativeInt32
        }
        ExternalSignatureType::NativeInt if numeric_profile.int_width == IntWidth::Bits64 => {
            ReturnAdapter::NativeInt64
        }
        ExternalSignatureType::NativeUint if numeric_profile.int_width == IntWidth::Bits32 => {
            ReturnAdapter::NativeUint32
        }
        ExternalSignatureType::NativeUint if numeric_profile.int_width == IntWidth::Bits64 => {
            ReturnAdapter::NativeUint64
        }
        ExternalSignatureType::NativeFloat
            if numeric_profile.float_precision == FloatPrecision::Bits32 =>
        {
            ReturnAdapter::NativeFloat32
        }
        ExternalSignatureType::Abi(ExternalAbiType::Fixed(scalar)) => ReturnAdapter::Fixed(
            FixedScalarAdapter::for_scalar(*scalar, numeric_profile)
                .expect("validate_supported_signature rejects fixed scalars without an adapter"),
        ),
        _ => ReturnAdapter::Identity,
    }
}

fn prepare_call_arguments(
    parameter_types: &[ExternalSignatureType],
    numeric_profile: NumericProfile,
) -> PreparedCallArguments {
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

        if matches!(
            parameter_type,
            ExternalSignatureType::NativeInt | ExternalSignatureType::NativeUint
                if numeric_profile.int_width == IntWidth::Bits64
        ) {
            // Int64 and Uint64 travel as BigInt inside Moth JS but cross to foreign
            // JS as a Number. The unsigned lane additionally rejects negative or
            // unsafe values before precision can be lost; it never widens the bridge.
            let is_unsigned = matches!(parameter_type, ExternalSignatureType::NativeUint);
            let (carrier, exactness_message) = if is_unsigned {
                (
                    "Uint64",
                    "Native Uint64 parameter is not exactly representable as a JavaScript Number",
                )
            } else {
                (
                    "Int64",
                    "Native Int64 parameter is not exactly representable as a JavaScript Number",
                )
            };
            if is_unsigned {
                writeln!(
                    &mut prelude,
                    "    if (typeof arg{index} !== \"bigint\") {{\n        throw new RangeError(\"Native {carrier} parameter must be a BigInt\");\n    }}\n    if (arg{index} < 0n || arg{index} > 9007199254740991n) {{\n        throw new RangeError(\"Native Uint64 parameter is outside the safe Number range\");\n    }}\n    const __moth_arg{index}_number = Number(arg{index});\n    if (!Number.isSafeInteger(__moth_arg{index}_number)) {{\n        throw new RangeError(\"{exactness_message}\");\n    }}"
                )
                .expect("writing generated wrapper checks into a String cannot fail");
            } else {
                writeln!(
                    &mut prelude,
                    "    if (typeof arg{index} !== \"bigint\") {{\n        throw new RangeError(\"Native {carrier} parameter must be a BigInt\");\n    }}\n    const __moth_arg{index}_number = Number(arg{index});\n    if (!Number.isSafeInteger(__moth_arg{index}_number)) {{\n        throw new RangeError(\"{exactness_message}\");\n    }}"
                )
                .expect("writing generated wrapper checks into a String cannot fail");
            }
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
                        let adapter = FixedScalarAdapter::for_scalar(*scalar, numeric_profile)
                            .expect(
                                "validate_supported_signature rejects unsupported fixed scalars",
                            );
                        prelude.push_str(&fixed_parameter_check(adapter, index));
                    }
                }
            }
            if matches!(parameter_type, ExternalSignatureType::NativeUint)
                && numeric_profile.int_width == IntWidth::Bits32
            {
                // Uint32 rides an exact Number. Integer -0 canonicalises through the
                // `=== 0` check shared with the Int32/I32 return gates.
                writeln!(
                    &mut prelude,
                    "    if (typeof arg{index} !== \"number\" || !Number.isInteger(arg{index}) || arg{index} < 0 || arg{index} > 4294967295) {{\n        throw new RangeError(\"Native Uint32 parameter is outside unsigned 32-bit range\");\n    }}\n    arg{index} = arg{index} === 0 ? 0 : arg{index};"
                )
                .expect("writing generated wrapper checks into a String cannot fail");
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
    numeric_profile: NumericProfile,
    indent: &str,
    fallible: bool,
    release_build: bool,
) -> String {
    let success_value = |expression: &str| {
        if fallible {
            format!("{{ tag: \"ok\", value: {expression} }}")
        } else {
            expression.to_owned()
        }
    };

    match adapter {
        ReturnAdapter::Identity => {
            format!("{indent}return {};", success_value(value_expression))
        }
        ReturnAdapter::NativeFloat32 => {
            let raw_value = "__moth_external_float32_raw";
            let adapted_value = "__moth_external_float32";
            let returned_value = success_value(adapted_value);
            format!(
                "{indent}const {raw_value} = {value_expression};\n{indent}const {adapted_value} = typeof {raw_value} === \"number\" ? Math.fround({raw_value}) : Number.NaN;\n{indent}return {returned_value};"
            )
        }
        // Native Int32, native Uint32 and exact-Number fixed integers share one value-gate shape,
        // not a semantic identity. Integer zero must be canonical before a later float
        // conversion can observe its sign. Only the bounds and the message differ:
        // the unsigned gates reject negatives where the signed gates admit them.
        ReturnAdapter::NativeInt32
        | ReturnAdapter::NativeUint32
        | ReturnAdapter::Fixed(FixedScalarAdapter::ExactInteger { .. }) => {
            let (boundary_message, minimum, maximum): (&str, i128, i128) = match adapter {
                ReturnAdapter::NativeUint32 => (
                    "External native Uint32 result is not an integer within the unsigned 32-bit range",
                    0,
                    4294967295,
                ),
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
                _ => (
                    "External native Int32 result is not an integer within the signed 32-bit range",
                    -2147483648,
                    2147483647,
                ),
            };
            let returned_value = success_value("__moth_external_canonical_integer");
            format!(
                "{indent}const __moth_external_integer = {value_expression};\n{indent}if (typeof __moth_external_integer !== \"number\" || !Number.isInteger(__moth_external_integer) || __moth_external_integer < {minimum} || __moth_external_integer > {maximum}) {{\n{indent}    throw new RangeError(\"{boundary_message}\");\n{indent}}}\n{indent}const __moth_external_canonical_integer = __moth_external_integer === 0 ? 0 : __moth_external_integer;\n{indent}return {returned_value};"
            )
        }
        // Int64 and Uint64 share one safe-Number bridge gate: check bounds before
        // BigInt conversion, since safe JavaScript integers are a strict subset of
        // either Moth range. BigInt(number) already canonicalizes integer -0.
        // The unsigned lane narrows the bridge to non-negative safe integers; it
        // never extends the Moth Uint64 range.
        ReturnAdapter::NativeInt64 | ReturnAdapter::NativeUint64 => {
            let returned_value = success_value("__moth_external_integer");
            let (boundary_message, minimum, maximum) = if adapter == ReturnAdapter::NativeUint64 {
                (
                    "External native Uint64 result is not a safe integer within the foreign Number range",
                    0,
                    9007199254740991,
                )
            } else {
                (
                    "External native Int64 result is not a safe integer within the Moth Int64 range",
                    numeric_profile.int_width.min_value(),
                    numeric_profile.int_width.max_value(),
                )
            };
            format!(
                "{indent}const __moth_external_number = {value_expression};\n{indent}if (typeof __moth_external_number !== \"number\" || !Number.isSafeInteger(__moth_external_number) || __moth_external_number < {minimum} || __moth_external_number > {maximum}) {{\n{indent}    throw new RangeError(\"{boundary_message}\");\n{indent}}}\n{indent}const __moth_external_integer = BigInt(__moth_external_number);\n{indent}return {returned_value};"
            )
        }
        // Fixed binary floats round once at their own precision, independent of the Float
        // profile, and keep signed zero. A non-Number or a value that rounds outside the finite
        // range is the shared Float boundary failure: code 304 in a fallible wrapper, a thrown
        // RangeError otherwise.
        ReturnAdapter::Fixed(FixedScalarAdapter::BinaryFloat { precision, .. }) => {
            let raw_value = "__moth_external_fixed_float_raw";
            let value = "__moth_external_fixed_float";
            let rounded_value = FixedScalarAdapter::rounded_float_source(precision, raw_value);
            let returned_value = success_value(value);
            let invalid_value = if fallible {
                let error = float_boundary_error_source(release_build);
                format!("{indent}    return {{ tag: \"err\", value: {error} }};")
            } else {
                format!(
                    "{indent}    throw new RangeError(\"{}\");",
                    BuiltinErrorCode::FloatBoundaryNonFinite.default_message()
                )
            };
            format!(
                "{indent}const {raw_value} = {value_expression};\n{indent}const {value} = typeof {raw_value} === \"number\" ? {rounded_value} : Number.NaN;\n{indent}if (!Number.isFinite({value})) {{\n{invalid_value}\n{indent}}}\n{indent}return {returned_value};"
            )
        }
    }
}

/// Validates a new `U32` or `F32` argument before the foreign call observes it.
///
/// Moth already produces valid fixed values. Direct wrapper misuse must not coerce a foreign
/// carrier: `U32` canonicalises integer zero and `F32` must be exact while preserving signed zero.
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
    }
}

fn float_boundary_error_source(release_build: bool) -> String {
    let error = BuiltinErrorCode::FloatBoundaryNonFinite;
    let message = format!("{:?}", error.default_message());
    // U32 builtin `Error.code` literals are plain decimal Number spellings, independent of the
    // build's `NumericProfile` Int/BigInt spellings.
    let code = error.as_u32().to_string();
    internal_error_object_source(&message, &code, release_build)
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
