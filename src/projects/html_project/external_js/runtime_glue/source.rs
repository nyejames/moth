//! Glue module source generation for HTML JS external exports.
//!
//! WHAT: generates ES module source that imports raw JS exports and re-exports stable wrapper
//!       functions, including fallible result-shape validation.
//! WHY: the JS backend calls wrappers by stable names; wrappers adapt raw JS return shapes
//!      to Moth's internal conventions.

use crate::backends::js::{
    builtin_error_code_js_field_name, builtin_error_message_js_field_name,
    external_module_export_glue_function_name,
};
use crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalPackageId, ExternalSignatureType,
};
use crate::projects::html_project::external_js::runtime_glue::exports::ReferencedExport;
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
    NativeFloat32,
    AbiI32,
    AbiF64,
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
            | ExternalSignatureType::NativeFloat
            | ExternalSignatureType::Abi(ExternalAbiType::I32 | ExternalAbiType::F64)
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
        ExternalSignatureType::NativeFloat
            if numeric_profile.float_precision == FloatPrecision::Bits32 =>
        {
            ReturnAdapter::NativeFloat32
        }
        ExternalSignatureType::Abi(ExternalAbiType::I32) => ReturnAdapter::AbiI32,
        ExternalSignatureType::Abi(ExternalAbiType::F64) => ReturnAdapter::AbiF64,
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
            ExternalSignatureType::NativeInt
                if numeric_profile.int_width == IntWidth::Bits64
        ) {
            writeln!(
                &mut prelude,
                "    if (typeof arg{index} !== \"bigint\") {{\n        throw new RangeError(\"Native Int64 parameter must be a BigInt\");\n    }}\n    const __moth_arg{index}_number = Number(arg{index});\n    if (!Number.isSafeInteger(__moth_arg{index}_number)) {{\n        throw new RangeError(\"Native Int64 parameter is not exactly representable as a JavaScript Number\");\n    }}"
            )
            .expect("writing generated wrapper checks into a String cannot fail");
            write!(&mut arguments, "__moth_arg{index}_number")
                .expect("writing generated wrapper arguments into a String cannot fail");
        } else {
            if matches!(
                parameter_type,
                ExternalSignatureType::Abi(ExternalAbiType::I32)
            ) {
                writeln!(
                    &mut prelude,
                    "    if (typeof arg{index} !== \"number\" || !Number.isInteger(arg{index}) || arg{index} < -2147483648 || arg{index} > 2147483647) {{\n        throw new RangeError(\"External I32 parameter is outside signed 32-bit range\");\n    }}"
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
        // Native Int32 and fixed I32 share a value gate, not a semantic identity. Integer
        // zero must be canonical before a later float conversion can observe its sign.
        ReturnAdapter::NativeInt32 | ReturnAdapter::AbiI32 => {
            let boundary_message = if adapter == ReturnAdapter::NativeInt32 {
                "External native Int32 result is not an integer within the signed 32-bit range"
            } else {
                "External I32 result is outside signed 32-bit range"
            };
            let returned_value = success_value("__moth_external_canonical_i32");
            format!(
                "{indent}const __moth_external_i32 = {value_expression};\n{indent}if (typeof __moth_external_i32 !== \"number\" || !Number.isInteger(__moth_external_i32) || __moth_external_i32 < -2147483648 || __moth_external_i32 > 2147483647) {{\n{indent}    throw new RangeError(\"{boundary_message}\");\n{indent}}}\n{indent}const __moth_external_canonical_i32 = __moth_external_i32 === 0 ? 0 : __moth_external_i32;\n{indent}return {returned_value};"
            )
        }
        ReturnAdapter::NativeInt64 => {
            let returned_value = success_value("__moth_external_integer");
            // Safe JavaScript integers are a strict subset of Int64, so check bounds before
            // BigInt conversion. BigInt(number) already canonicalizes integer -0.
            let minimum = numeric_profile.int_width.min_value();
            let maximum = numeric_profile.int_width.max_value();
            format!(
                "{indent}const __moth_external_number = {value_expression};\n{indent}if (typeof __moth_external_number !== \"number\" || !Number.isSafeInteger(__moth_external_number) || __moth_external_number < {minimum} || __moth_external_number > {maximum}) {{\n{indent}    throw new RangeError(\"External native Int64 result is not a safe integer within the Moth Int64 range\");\n{indent}}}\n{indent}const __moth_external_integer = BigInt(__moth_external_number);\n{indent}return {returned_value};"
            )
        }
        ReturnAdapter::AbiF64 => {
            let value = "__moth_external_f64";
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
                "{indent}const {value} = {value_expression};\n{indent}if (!Number.isFinite({value})) {{\n{invalid_value}\n{indent}}}\n{indent}return {returned_value};"
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
