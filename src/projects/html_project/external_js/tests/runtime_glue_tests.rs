//! Tests for generated HTML JS glue and runtime module resolution.

use super::import_map::build_import_map_html;
use super::paths::relative_url_path;
use super::runtime_modules::emit_build_runtime_modules;
use super::source::{generate_fallible_wrapper, generate_infallible_wrapper};
use super::*;
use crate::build_system::build::FileKind;
use crate::compiler_frontend::datatypes::numeric_profile::{
    FloatPrecision, IntWidth, NumericProfile,
};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalFunctionDef, ExternalFunctionId, ExternalFunctionLowerings,
    ExternalJsLowering, ExternalPackageId, ExternalPackageRegistry, ExternalReturnSlot,
    ExternalSignatureType, ExternalTypeId,
};
use crate::compiler_frontend::module_compilation::Module;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::external_js::runtime_emission_plan::HtmlExternalRuntimeEmissionPlan;
use crate::projects::html_project::tests::test_support::{
    create_test_module, js_runtime_asset_import,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn relative_url_path_same_directory() {
    let html = PathBuf::from("index.html");
    let asset = PathBuf::from("_moth/js/glue/module.js");
    assert_eq!(
        relative_url_path(&html, &asset),
        "./_moth/js/glue/module.js"
    );
}

#[test]
fn relative_url_path_one_level_deep() {
    let html = PathBuf::from("about/index.html");
    let asset = PathBuf::from("_moth/js/glue/module.js");
    assert_eq!(
        relative_url_path(&html, &asset),
        "../_moth/js/glue/module.js"
    );
}

#[test]
fn relative_url_path_two_levels_deep() {
    let html = PathBuf::from("a/b/index.html");
    let asset = PathBuf::from("_moth/js/runtime/moth-runtime.js");
    assert_eq!(
        relative_url_path(&html, &asset),
        "../../_moth/js/runtime/moth-runtime.js"
    );
}

#[test]
fn relative_url_path_shared_prefix() {
    let html = PathBuf::from("docs/index.html");
    let asset = PathBuf::from("docs/assets/file.js");
    assert_eq!(relative_url_path(&html, &asset), "./assets/file.js");
}

#[test]
fn generate_module_glue_returns_empty_when_no_external_exports() {
    let mut string_table = StringTable::new();
    let module = create_test_module(PathBuf::from("@page.moth"), &mut string_table);
    let referenced = HashSet::new();
    let registry = ExternalPackageRegistry::new();

    let result = generate_module_glue(
        &module,
        &module.link_facts.external_import_candidates,
        &referenced,
        &registry,
        &PathBuf::from("index.html"),
        false,
        NumericProfile::STANDARD,
    )
    .expect("empty glue generation should succeed");

    assert!(result.glue_output_files.is_empty());
    assert!(result.bundle_import_preamble.is_none());
    assert!(result.import_map_html.is_none());
}

#[test]
fn generate_module_glue_empty_when_export_registered_but_not_referenced() {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("@page.moth"), &mut string_table);
    module.link_facts.external_import_candidates.push(
        crate::compiler_frontend::module_compilation::ModuleExternalImport {
            package_id: ExternalPackageId(0),
            runtime_asset: Some(js_runtime_asset_import(
                Path::new("lib.js"),
                PathBuf::from("/project/lib.js"),
            )),
            required_runtime_imports: Vec::new(),
        },
    );

    let (registry, _function_id, package_id) = create_registry_with_export("get_value", "getValue");
    module.link_facts.external_import_candidates[0].package_id = package_id;

    // Export is registered but not referenced by emitted JS.
    let referenced = HashSet::new();

    let result = generate_module_glue(
        &module,
        &module.link_facts.external_import_candidates,
        &referenced,
        &registry,
        &PathBuf::from("index.html"),
        false,
        NumericProfile::STANDARD,
    )
    .expect("glue generation should succeed");

    assert!(result.glue_output_files.is_empty());
    assert!(result.bundle_import_preamble.is_none());
    assert!(result.import_map_html.is_none());
}

#[test]
fn generate_module_glue_emits_glue_file_for_referenced_export() {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("@page.moth"), &mut string_table);
    module.link_facts.external_import_candidates.push(
        crate::compiler_frontend::module_compilation::ModuleExternalImport {
            package_id: ExternalPackageId(0),
            runtime_asset: Some(js_runtime_asset_import(
                Path::new("lib.js"),
                PathBuf::from("/project/lib.js"),
            )),
            required_runtime_imports: Vec::new(),
        },
    );

    let (registry, function_id, package_id) = create_registry_with_export("get_value", "getValue");
    module.link_facts.external_import_candidates[0].package_id = package_id;
    let referenced = HashSet::from([function_id]);

    let result = generate_module_glue(
        &module,
        &module.link_facts.external_import_candidates,
        &referenced,
        &registry,
        &PathBuf::from("index.html"),
        false,
        NumericProfile::STANDARD,
    )
    .expect("glue generation should succeed");

    assert_eq!(result.glue_output_files.len(), 1);
    let glue_file = &result.glue_output_files[0];
    assert!(
        glue_file
            .relative_output_path()
            .starts_with("_moth/js/glue/")
    );

    let FileKind::Js(source) = glue_file.file_kind() else {
        panic!("glue file must be JS");
    };
    assert!(
        source.contains("import { getValue as __moth_external_fn"),
        "missing aliased getValue import in:\n{}",
        source
    );
    assert!(source.contains("export function __moth_glue_fn"));

    assert!(result.bundle_import_preamble.is_some());
    let preamble = result.bundle_import_preamble.unwrap();
    assert!(preamble.starts_with("import { __moth_glue_fn"));
    assert!(preamble.contains("from \""));
}

#[test]
fn generate_module_glue_nested_html_output_path() {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("@page.moth"), &mut string_table);
    module.link_facts.external_import_candidates.push(
        crate::compiler_frontend::module_compilation::ModuleExternalImport {
            package_id: ExternalPackageId(0),
            runtime_asset: Some(js_runtime_asset_import(
                Path::new("lib.js"),
                PathBuf::from("/project/lib.js"),
            )),
            required_runtime_imports: Vec::new(),
        },
    );

    let (registry, function_id, package_id) = create_registry_with_export("get_value", "getValue");
    module.link_facts.external_import_candidates[0].package_id = package_id;
    let referenced = HashSet::from([function_id]);

    let result = generate_module_glue(
        &module,
        &module.link_facts.external_import_candidates,
        &referenced,
        &registry,
        &PathBuf::from("a/b/index.html"),
        false,
        NumericProfile::STANDARD,
    )
    .expect("glue generation should succeed");

    assert!(result.bundle_import_preamble.is_some());
    let preamble = result.bundle_import_preamble.unwrap();
    assert!(
        preamble.contains("from \"../../_moth/js/glue/module-"),
        "expected nested relative path in preamble, got:\n{preamble}"
    );
}

#[test]
fn generate_module_glue_asset_import_relative_to_glue_module() {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("@page.moth"), &mut string_table);
    module.link_facts.external_import_candidates.push(
        crate::compiler_frontend::module_compilation::ModuleExternalImport {
            package_id: ExternalPackageId(0),
            runtime_asset: Some(js_runtime_asset_import(
                Path::new("lib.js"),
                PathBuf::from("/project/lib.js"),
            )),
            required_runtime_imports: Vec::new(),
        },
    );

    let (registry, function_id, package_id) = create_registry_with_export("get_value", "getValue");
    module.link_facts.external_import_candidates[0].package_id = package_id;
    let referenced = HashSet::from([function_id]);

    let result = generate_module_glue(
        &module,
        &module.link_facts.external_import_candidates,
        &referenced,
        &registry,
        &PathBuf::from("index.html"),
        false,
        NumericProfile::STANDARD,
    )
    .expect("glue generation should succeed");

    let FileKind::Js(source) = result.glue_output_files[0].file_kind() else {
        panic!("glue file must be JS");
    };
    // Asset is at _moth/js/lib.js; glue is at _moth/js/glue/module-{hash}.js.
    // Relative path from glue to asset should be ../lib.js.
    assert!(
        source.contains("from \"../lib.js\""),
        "expected asset import relative to glue module, got:\n{source}"
    );
}

#[test]
fn fallible_wrapper_handles_invalid_shape_differently_for_debug_and_release() {
    let debug_source = generate_fallible_wrapper(
        "__moth_glue_debug",
        "invalidDebug",
        false,
        NumericProfile::STANDARD,
        &[],
        &[],
    )
    .expect("debug wrapper generation should succeed");
    let release_source = generate_fallible_wrapper(
        "__moth_glue_release",
        "invalidRelease",
        true,
        NumericProfile::STANDARD,
        &[],
        &[],
    )
    .expect("release wrapper generation should succeed");
    let code_field = crate::backends::js::builtin_error_code_js_field_name(true);
    let message_field = crate::backends::js::builtin_error_message_js_field_name(true);
    let script = format!(
        r#"
import assert from "node:assert/strict";
function invalidDebug() {{ return {{ ok: "yes" }}; }}
function invalidRelease() {{ return {{ ok: "yes" }}; }}
{debug_source}
{release_source}
assert.throws(() => __moth_glue_debug(), /Invalid result wrapper/);
const result = __moth_glue_release();
assert.equal(result.tag, "err");
assert.equal(result.value[{code_field:?}], 0);
assert.equal(
    result.value[{message_field:?}],
    "Invalid result wrapper from external JavaScript function"
);
"#
    );
    run_generated_node_script(&script);
}

#[test]
fn standard_fallible_wrapper_bounds_error_code_to_int32() {
    let wrapper = generate_fallible_wrapper(
        "wrapExternal",
        "failExternal",
        false,
        NumericProfile::STANDARD,
        &[],
        &[],
    )
    .expect("fallible wrapper generation should succeed");
    let code_field = crate::backends::js::builtin_error_code_js_field_name(false);
    let message_field = crate::backends::js::builtin_error_message_js_field_name(false);
    let script = format!(
        r#"
import assert from "node:assert/strict";
let suppliedError;
let shouldThrow = false;
let shouldSucceed = false;
function failExternal() {{
    if (shouldThrow) throw new Error("thrown failure");
    if (shouldSucceed) return {{ ok: true, value: 42 }};
    return {{ ok: false, error: suppliedError }};
}}
{wrapper}
const codeField = {code_field:?};
const messageField = {message_field:?};
for (const validCode of [-2147483648, 2147483647, -1, 0]) {{
    suppliedError = {{ message: "failure", code: validCode }};
    const result = wrapExternal();
    assert.equal(result.tag, "err");
    assert.equal(typeof result.value[codeField], "number");
    assert.equal(result.value[codeField], validCode);
    assert.equal(result.value[messageField], "failure");
}}
for (const invalidError of [
    {{ message: "failure", code: 1.5 }},
    {{ message: "failure", code: NaN }},
    {{ message: "failure", code: Infinity }},
    {{ message: "failure", code: -Infinity }},
    {{ message: "failure", code: 2147483648 }},
    {{ message: "failure", code: -2147483649 }},
    {{ message: "failure", code: "500" }},
    {{ message: "failure" }},
]) {{
    suppliedError = invalidError;
    const result = wrapExternal();
    assert.equal(result.tag, "err");
    assert.equal(typeof result.value[codeField], "number");
    assert.equal(result.value[codeField], 0);
    assert.equal(result.value[messageField], "failure");
}}
shouldThrow = true;
let result = wrapExternal();
assert.equal(result.tag, "err");
assert.equal(typeof result.value[codeField], "number");
assert.equal(result.value[codeField], 0);
assert.equal(result.value[messageField], "thrown failure");
shouldThrow = false;
shouldSucceed = true;
result = wrapExternal();
assert.deepEqual(result, {{ tag: "ok", value: 42 }});
"#
    );
    run_generated_node_script(&script);
}

#[test]
fn int64_fallible_wrapper_executes_with_bigint_error_code() {
    let profile = NumericProfile {
        int_width: IntWidth::Bits64,
        ..NumericProfile::STANDARD
    };
    let wrapper =
        generate_fallible_wrapper("wrapExternal", "failExternal", false, profile, &[], &[])
            .expect("fallible wrapper generation should succeed");
    let code_field = crate::backends::js::builtin_error_code_js_field_name(false);
    let code_field = format!("{code_field:?}");
    let script = format!(
        r#"
import assert from "node:assert/strict";
let suppliedCode;
let shouldThrow = false;
function failExternal() {{
    if (shouldThrow) throw new Error("thrown failure");
    return {{ ok: false, error: {{ message: "failure", code: suppliedCode }} }};
}}
{wrapper}
const codeField = {code_field};
suppliedCode = 500;
let result = wrapExternal();
assert.equal(result.tag, "err");
assert.equal(typeof result.value[codeField], "bigint");
assert.equal(result.value[codeField] === 500n, true);
for (const invalidCode of ["500", Number.MAX_SAFE_INTEGER + 1, undefined]) {{
    suppliedCode = invalidCode;
    result = wrapExternal();
    assert.equal(result.value[codeField], 0n);
}}
shouldThrow = true;
result = wrapExternal();
assert.equal(result.value[codeField], 0n);
"#
    );

    let output = Command::new("node")
        .args(["--input-type=module", "--eval", &script])
        .output()
        .expect("Node must be available to execute the generated external glue");
    assert!(
        output.status.success(),
        "generated glue failed in Node:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fallible_wrapper_reads_foreign_error_code_once_for_each_profile_and_build_mode() {
    let mut script = String::from(r#"import assert from "node:assert/strict";"#);

    for (int_width, release_build, suffix) in [
        (IntWidth::Bits32, false, "Int32Debug"),
        (IntWidth::Bits32, true, "Int32Release"),
        (IntWidth::Bits64, false, "Int64Debug"),
        (IntWidth::Bits64, true, "Int64Release"),
    ] {
        let profile = NumericProfile {
            int_width,
            ..NumericProfile::STANDARD
        };
        let wrapper_name = format!("wrap{suffix}");
        let export_name = format!("fail{suffix}");
        let wrapper = generate_fallible_wrapper(
            &wrapper_name,
            &export_name,
            release_build,
            profile,
            &[],
            &[],
        )
        .expect("fallible wrapper generation should succeed");
        let code_field = crate::backends::js::builtin_error_code_js_field_name(release_build);
        let message_field = crate::backends::js::builtin_error_message_js_field_name(release_build);
        let (changing_code, invalid_initial_code, expected_code, zero_code) = match int_width {
            IntWidth::Bits32 => ("Infinity", "Infinity", "-17", "0"),
            IntWidth::Bits64 => ("1e20", "1e20", "-17n", "0n"),
        };

        script.push_str(&format!(
            r#"
let getterReads{suffix} = 0;
let initialCode{suffix} = -17;
let changingCode{suffix} = {changing_code};
let repeatedReads{suffix} = 3;
function {export_name}() {{
    return {{ ok: false, error: {{
        message: "accessor failure",
        get code() {{
            getterReads{suffix} += 1;
            return getterReads{suffix} <= repeatedReads{suffix}
                ? initialCode{suffix}
                : changingCode{suffix};
        }}
    }} }};
}}
"#
        ));
        script.push_str(&wrapper);
        script.push_str(&format!(
            r#"
const codeField{suffix} = {code_field:?};
const messageField{suffix} = {message_field:?};
let result{suffix} = {wrapper_name}();
assert.equal(result{suffix}.tag, "err");
assert.equal(result{suffix}.value[codeField{suffix}], {expected_code});
assert.equal(result{suffix}.value[messageField{suffix}], "accessor failure");
assert.equal(getterReads{suffix}, 1);

getterReads{suffix} = 0;
initialCode{suffix} = {invalid_initial_code};
changingCode{suffix} = 29;
repeatedReads{suffix} = 1;
result{suffix} = {wrapper_name}();
assert.equal(result{suffix}.tag, "err");
assert.equal(result{suffix}.value[codeField{suffix}], {zero_code});
assert.equal(result{suffix}.value[messageField{suffix}], "accessor failure");
assert.equal(getterReads{suffix}, 1);
"#
        ));
    }

    let output = Command::new("node")
        .args(["--input-type=module", "--eval", &script])
        .output()
        .expect("Node must be available to execute the generated external glue");
    assert!(
        output.status.success(),
        "generated glue failed in Node:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn native_int64_wrapper_bridges_only_exact_safe_javascript_numbers() {
    let profile = NumericProfile {
        int_width: IntWidth::Bits64,
        ..NumericProfile::STANDARD
    };
    let input_wrapper = generate_infallible_wrapper(
        "wrapNativeIntInput",
        "rawNativeIntInput",
        &[ExternalSignatureType::NativeInt],
        &[ExternalSignatureType::NativeInt],
        profile,
    )
    .expect("native Int64 input wrapper generation should succeed");
    let output_wrapper = generate_infallible_wrapper(
        "wrapNativeIntOutput",
        "rawNativeIntOutput",
        &[],
        &[ExternalSignatureType::NativeInt],
        profile,
    )
    .expect("native Int64 output wrapper generation should succeed");
    let script = format!(
        r#"
import assert from "node:assert/strict";
let inputCalls = 0;
let lastInput;
function rawNativeIntInput(value) {{
    inputCalls += 1;
    lastInput = value;
    return value;
}}
let rawNumber = 0;
function rawNativeIntOutput() {{ return rawNumber; }}
{input_wrapper}
{output_wrapper}

for (const value of [
    0n,
    5000000000n,
    BigInt(Number.MAX_SAFE_INTEGER),
    BigInt(Number.MIN_SAFE_INTEGER),
]) {{
    assert.equal(wrapNativeIntInput(value), value);
    assert.equal(typeof lastInput, "number");
    assert.equal(lastInput, Number(value));
}}
assert.equal(inputCalls, 4);
for (const value of [
    BigInt(Number.MAX_SAFE_INTEGER) + 1n,
    BigInt(Number.MIN_SAFE_INTEGER) - 1n,
    9223372036854775807n,
    -9223372036854775808n,
    5000000000,
]) {{
    assert.throws(() => wrapNativeIntInput(value), RangeError);
}}
assert.equal(inputCalls, 4, "unsafe Int64 inputs must be rejected before the raw export call");

for (const value of [5000000000, Number.MAX_SAFE_INTEGER, Number.MIN_SAFE_INTEGER]) {{
    rawNumber = value;
    assert.equal(wrapNativeIntOutput(), BigInt(value));
}}
for (const value of [
    1.5,
    NaN,
    Infinity,
    -Infinity,
    Number.MAX_SAFE_INTEGER + 1,
    Number.MIN_SAFE_INTEGER - 1,
    "5000000000",
]) {{
    rawNumber = value;
    assert.throws(() => wrapNativeIntOutput(), RangeError);
}}
"#
    );
    run_generated_node_script(&script);
}

#[test]
fn native_float32_fallible_wrapper_rejects_non_numbers_and_preserves_signed_zero() {
    let profile = NumericProfile {
        float_precision: FloatPrecision::Bits32,
        ..NumericProfile::STANDARD
    };
    let wrapper = generate_fallible_wrapper(
        "wrapNativeFloat",
        "rawNativeFloat",
        false,
        profile,
        &[ExternalSignatureType::NativeFloat],
        &[ExternalSignatureType::NativeFloat],
    )
    .expect("native Float32 wrapper generation should succeed");
    let script = format!(
        r#"
import assert from "node:assert/strict";
let rawResult = 0.1;
let rawValueReads = 0;
let received;
function rawNativeFloat(value) {{
    received = value;
    return {{ ok: true, get value() {{ rawValueReads += 1; return rawResult; }} }};
}}
{wrapper}
const input = Math.fround(0.1);
let result = wrapNativeFloat(input);
assert.equal(received, input, "Float32 input is already representable and must not be rounded again");
assert.equal(result.tag, "ok");
assert.equal(result.value, Math.fround(0.1));
assert.equal(Number.isFinite(result.value), true);
assert.equal(rawValueReads, 1, "the success value getter must be read once");

rawValueReads = 0;
rawResult = -0;
result = wrapNativeFloat(-0);
assert.equal(Object.is(received, -0), true);
assert.equal(Object.is(result.value, -0), true);
assert.equal(rawValueReads, 1, "the signed-zero success value getter must be read once");

rawValueReads = 0;
rawResult = 1.00000006;
result = wrapNativeFloat(1.0);
assert.equal(result.value, Math.fround(1.00000006));
assert.equal(Number.isFinite(result.value), true);
assert.equal(rawValueReads, 1);

rawValueReads = 0;
rawResult = "1.5";
result = wrapNativeFloat(1.0);
assert.equal(result.tag, "ok");
assert.equal(
    Number.isNaN(result.value),
    true,
    "numeric strings must not be coerced by Math.fround and must reach HIR ValidateFloat as invalid",
);
assert.equal(rawValueReads, 1, "the invalid success value getter must be read once");

for (const nonFinite of [NaN, Infinity, -Infinity]) {{
    rawValueReads = 0;
    rawResult = nonFinite;
    result = wrapNativeFloat(1.0);
    assert.equal(Object.is(result.value, nonFinite), true);
    assert.equal(rawValueReads, 1);
}}

rawValueReads = 0;
rawResult = 3.5e38;
assert.equal(Number.isFinite(rawResult), true);
result = wrapNativeFloat(1.0);
assert.equal(result.value, Infinity, "finite f64 overflow rounds to Float32 infinity for HIR validation");
assert.equal(rawValueReads, 1);
"#
    );
    run_generated_node_script(&script);
}

#[test]
fn fixed_i32_glue_checks_bounds_and_other_signatures_preserve_js_values() {
    let i32_wrapper = generate_infallible_wrapper(
        "wrapI32",
        "rawI32",
        &[ExternalSignatureType::Abi(ExternalAbiType::I32)],
        &[ExternalSignatureType::Abi(ExternalAbiType::I32)],
        NumericProfile::STANDARD,
    )
    .expect("fixed I32 wrapper generation should succeed");
    let mixed_wrapper = generate_infallible_wrapper(
        "wrapMixed",
        "rawMixed",
        &[
            ExternalSignatureType::Abi(ExternalAbiType::F64),
            ExternalSignatureType::Abi(ExternalAbiType::Bool),
            ExternalSignatureType::Abi(ExternalAbiType::Utf8Str),
            ExternalSignatureType::Abi(ExternalAbiType::Char),
            ExternalSignatureType::External(ExternalTypeId(501)),
            ExternalSignatureType::Abi(ExternalAbiType::Handle),
            ExternalSignatureType::NativeInt,
            ExternalSignatureType::NativeFloat,
        ],
        &[ExternalSignatureType::External(ExternalTypeId(501))],
        NumericProfile {
            float_precision: FloatPrecision::Bits32,
            ..NumericProfile::STANDARD
        },
    )
    .expect("mixed signature wrapper generation should succeed");
    let mut output_wrappers = String::new();
    for (wrapper_name, export_name, return_type) in [
        (
            "wrapBool",
            "rawBool",
            ExternalSignatureType::Abi(ExternalAbiType::Bool),
        ),
        (
            "wrapString",
            "rawString",
            ExternalSignatureType::Abi(ExternalAbiType::Utf8Str),
        ),
        (
            "wrapChar",
            "rawChar",
            ExternalSignatureType::Abi(ExternalAbiType::Char),
        ),
        (
            "wrapHandle",
            "rawHandle",
            ExternalSignatureType::Abi(ExternalAbiType::Handle),
        ),
        (
            "wrapNativeInt",
            "rawNativeInt",
            ExternalSignatureType::NativeInt,
        ),
        (
            "wrapNativeFloat",
            "rawNativeFloat",
            ExternalSignatureType::NativeFloat,
        ),
    ] {
        let wrapper = generate_infallible_wrapper(
            wrapper_name,
            export_name,
            &[],
            &[return_type],
            NumericProfile::STANDARD,
        )
        .expect("identity result wrapper generation should succeed");
        output_wrappers.push_str(&wrapper);
        output_wrappers.push('\n');
    }
    let script = format!(
        r#"
import assert from "node:assert/strict";
let i32Calls = 0;
let useI32Override = false;
let i32Override;
function rawI32(value) {{
    i32Calls += 1;
    return useI32Override ? i32Override : value;
}}
let received;
const opaque = {{}};
function rawMixed(f64, boolValue, text, character, external, handle, nativeInt, nativeFloat) {{
    received = [f64, boolValue, text, character, external, handle, nativeInt, nativeFloat];
    return external;
}}
function rawBool() {{ return true; }}
function rawString() {{ return "result"; }}
function rawChar() {{ return "λ"; }}
function rawHandle() {{ return handle; }}
function rawNativeInt() {{ return 17; }}
function rawNativeFloat() {{ return 0.1; }}
{i32_wrapper}
{mixed_wrapper}
{output_wrappers}

for (const value of [-2147483648, 0, 2147483647]) {{
    assert.equal(wrapI32(value), value);
}}
assert.equal(i32Calls, 3);
for (const value of [-2147483649, 2147483648, 1.5, NaN, Infinity, "1", 1n]) {{
    assert.throws(() => wrapI32(value), RangeError);
}}
assert.equal(i32Calls, 3, "invalid I32 inputs must be rejected before the raw export call");
for (const value of [5000000000, -2147483649, 1.25, NaN, Infinity]) {{
    useI32Override = true;
    i32Override = value;
    assert.throws(() => wrapI32(1), RangeError);
}}

const handle = {{}};
const f64 = 1.23456789012345;
const text = "value";
const character = "λ";
const nativeFloat = Math.fround(0.1);
assert.equal(
    wrapMixed(f64, true, text, character, opaque, handle, 17, nativeFloat),
    opaque
);
assert.equal(received[0], f64);
assert.equal(received[1], true);
assert.equal(received[2], text);
assert.equal(received[3], character);
assert.equal(received[4], opaque);
assert.equal(received[5], handle);
assert.equal(received[6], 17);
assert.equal(received[7], nativeFloat);
assert.equal(wrapBool(), true);
assert.equal(wrapString(), "result");
assert.equal(wrapChar(), "λ");
assert.equal(wrapHandle(), handle);
assert.equal(wrapNativeInt(), 17);
assert.equal(wrapNativeFloat(), 0.1);
"#
    );
    run_generated_node_script(&script);
}

#[test]
fn fallible_native_int64_adapter_preserves_success_and_foreign_errors() {
    let profile = NumericProfile {
        int_width: IntWidth::Bits64,
        ..NumericProfile::STANDARD
    };
    let wrapper = generate_fallible_wrapper(
        "wrapNativeInt",
        "rawNativeInt",
        false,
        profile,
        &[ExternalSignatureType::NativeInt],
        &[ExternalSignatureType::NativeInt],
    )
    .expect("fallible native Int64 wrapper generation should succeed");
    let code_field = crate::backends::js::builtin_error_code_js_field_name(false);
    let message_field = crate::backends::js::builtin_error_message_js_field_name(false);
    let script = format!(
        r#"
import assert from "node:assert/strict";
let mode = "success";
let calls = 0;
function rawNativeInt(value) {{
    calls += 1;
    if (mode === "throw") throw new Error("foreign exception");
    if (mode === "error") return {{ ok: false, error: {{ message: "foreign error", code: 500 }} }};
    return {{ ok: true, value }};
}}
{wrapper}
const codeField = {code_field:?};
const messageField = {message_field:?};
let result = wrapNativeInt(5000000000n);
assert.equal(result.tag, "ok");
assert.equal(result.value, 5000000000n);
mode = "error";
result = wrapNativeInt(5000000000n);
assert.equal(result.tag, "err");
assert.equal(result.value[codeField], 500n);
assert.equal(result.value[messageField], "foreign error");
mode = "throw";
result = wrapNativeInt(5000000000n);
assert.equal(result.tag, "err");
assert.equal(result.value[codeField], 0n);
assert.equal(result.value[messageField], "foreign exception");
const callsBeforeUnsafe = calls;
assert.throws(() => wrapNativeInt(9223372036854775807n), RangeError);
assert.equal(calls, callsBeforeUnsafe);
"#
    );
    run_generated_node_script(&script);
}

#[test]
fn fallible_fixed_f64_nonfinite_result_uses_profiled_moth_error() {
    let mut script = String::from(r#"import assert from "node:assert/strict";"#);
    for (int_width, suffix, error_code, zero_code) in [
        (IntWidth::Bits32, "Int32", "304", "0"),
        (IntWidth::Bits64, "Int64", "304n", "0n"),
    ] {
        let profile = NumericProfile {
            int_width,
            float_precision: FloatPrecision::Bits32,
        };
        let wrapper_name = format!("wrapF64{suffix}");
        let export_name = format!("rawF64{suffix}");
        let wrapper = generate_fallible_wrapper(
            &wrapper_name,
            &export_name,
            false,
            profile,
            &[],
            &[ExternalSignatureType::Abi(ExternalAbiType::F64)],
        )
        .expect("fallible F64 wrapper generation should succeed");
        let infallible_wrapper_name = format!("wrapInfallibleF64{suffix}");
        let infallible_export_name = format!("rawInfallibleF64{suffix}");
        let infallible_wrapper = generate_infallible_wrapper(
            &infallible_wrapper_name,
            &infallible_export_name,
            &[],
            &[ExternalSignatureType::Abi(ExternalAbiType::F64)],
            profile,
        )
        .expect("infallible F64 wrapper generation should succeed");
        let code_field = crate::backends::js::builtin_error_code_js_field_name(false);
        let message_field = crate::backends::js::builtin_error_message_js_field_name(false);
        script.push_str(&format!(
            r#"
let mode{suffix} = "finite";
let rawValue{suffix} = 1.23456789012345;
function {export_name}() {{
    if (mode{suffix} === "throw") throw new Error("foreign exception");
    if (mode{suffix} === "foreign-error") {{
        return {{ ok: false, error: {{ message: "foreign error", code: 91 }} }};
    }}
    return {{ ok: true, value: rawValue{suffix} }};
}}
"#
        ));
        script.push_str(&wrapper);
        script.push_str(&format!(
            r#"
function {infallible_export_name}() {{ return rawValue{suffix}; }}
{infallible_wrapper}
"#
        ));
        script.push_str(&format!(
            r#"
const codeField{suffix} = {code_field:?};
const messageField{suffix} = {message_field:?};
let result{suffix} = {wrapper_name}();
assert.deepEqual(result{suffix}, {{ tag: "ok", value: 1.23456789012345 }});
assert.equal({infallible_wrapper_name}(), 1.23456789012345);

rawValue{suffix} = -0;
result{suffix} = {wrapper_name}();
assert.equal(Object.is(result{suffix}.value, -0), true);
rawValue{suffix} = 3.5e38;
result{suffix} = {wrapper_name}();
assert.equal(result{suffix}.value, 3.5e38);
assert.equal(Number.isFinite(result{suffix}.value), true, "fixed F64 must not be rounded to the Float32 profile");
assert.equal({infallible_wrapper_name}(), 3.5e38);

for (const invalid of [NaN, Infinity, -Infinity]) {{
    rawValue{suffix} = invalid;
    result{suffix} = {wrapper_name}();
    assert.equal(result{suffix}.tag, "err");
    assert.equal(result{suffix}.value[codeField{suffix}], {error_code});
    assert.equal(
        result{suffix}.value[messageField{suffix}],
        "External Float boundary produced a non-finite value"
    );
    assert.throws(() => {infallible_wrapper_name}(), RangeError);
}}

mode{suffix} = "foreign-error";
result{suffix} = {wrapper_name}();
assert.equal(result{suffix}.tag, "err");
assert.equal(result{suffix}.value[codeField{suffix}], 91{});
assert.equal(result{suffix}.value[messageField{suffix}], "foreign error");
mode{suffix} = "throw";
result{suffix} = {wrapper_name}();
assert.equal(result{suffix}.tag, "err");
assert.equal(result{suffix}.value[codeField{suffix}], {zero_code});
assert.equal(result{suffix}.value[messageField{suffix}], "foreign exception");
"#,
            if int_width == IntWidth::Bits64 { "n" } else { "" },
        ));
    }
    run_generated_node_script(&script);
}

#[test]
fn glue_rejects_optional_numeric_metadata_instead_of_forwarding_it() {
    let error = generate_infallible_wrapper(
        "wrapOptional",
        "rawOptional",
        &[ExternalSignatureType::Optional(Box::new(
            ExternalSignatureType::NativeInt,
        ))],
        &[],
        NumericProfile {
            int_width: IntWidth::Bits64,
            ..NumericProfile::STANDARD
        },
    )
    .expect_err("optional numeric metadata has no adapter representation");
    assert!(error.msg.contains("optional numeric signature metadata"));
}

fn run_generated_node_script(script: &str) {
    let output = Command::new("node")
        .args(["--input-type=module", "--eval", script])
        .output()
        .expect("Node must be available to execute the generated external glue");
    assert!(
        output.status.success(),
        "generated glue failed in Node:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn emit_build_runtime_modules_dedupes_by_specifier() {
    let module_a = create_module_with_runtime_requirement();
    let module_b = create_module_with_runtime_requirement();

    let plan = HtmlExternalRuntimeEmissionPlan::from_import_sets([
        module_a.link_facts.external_import_candidates.as_slice(),
        module_b.link_facts.external_import_candidates.as_slice(),
    ]);
    let mut occupied = HashSet::new();
    let string_table = StringTable::new();
    let files = emit_build_runtime_modules(&plan, &mut occupied, &string_table)
        .expect("runtime module emission should succeed");

    // Only one runtime module emitted despite two modules requiring it.
    assert_eq!(files.len(), 1);
    assert!(files[0].relative_output_path().ends_with("moth-runtime.js"));
}

#[test]
fn emit_build_runtime_modules_rejects_unregistered_specifier() {
    let mut module = create_module_with_runtime_requirement();
    module.link_facts.external_import_candidates[0].required_runtime_imports[0].module_name =
        "@moth/missing".to_owned();

    let plan = HtmlExternalRuntimeEmissionPlan::from_import_sets([module
        .link_facts
        .external_import_candidates
        .as_slice()]);
    let mut occupied = HashSet::new();
    let string_table = StringTable::new();
    let error = match emit_build_runtime_modules(&plan, &mut occupied, &string_table) {
        Ok(_) => panic!("unregistered runtime module should fail"),
        Err(error) => error,
    };
    let error = error
        .infrastructure_error()
        .expect("runtime module failure should be an infrastructure error");

    assert!(
        error.msg.contains("@moth/missing"),
        "expected unregistered module name in error"
    );
}

#[test]
fn build_import_map_html_includes_moth_runtime() {
    let module = create_module_with_runtime_requirement();
    let html = build_import_map_html(
        &module.link_facts.external_import_candidates,
        &PathBuf::from("index.html"),
    );

    assert!(html.is_some());
    let map = html.unwrap();
    assert!(map.contains("<script type=\"importmap\">"));
    assert!(map.contains("@moth/runtime"));
    assert!(map.contains("./_moth/js/runtime/moth-runtime.js"));
}

#[test]
fn build_import_map_html_deduplicates_by_specifier() {
    let mut module = create_module_with_runtime_requirement();
    module.link_facts.external_import_candidates.push(
        crate::compiler_frontend::module_compilation::ModuleExternalImport {
            package_id: ExternalPackageId(1),
            runtime_asset: None,
            required_runtime_imports: vec![
                crate::builder_surface::external_import_providers::provider::RequiredRuntimeImport {
                    module_name: "@moth/runtime".to_owned(),
                    imported_names: vec!["mothOk".to_owned()],
                },
            ],
        },
    );

    let html = build_import_map_html(
        &module.link_facts.external_import_candidates,
        &PathBuf::from("index.html"),
    );
    assert!(html.is_some());
    let map = html.unwrap();

    let occurrences = map.matches("@moth/runtime").count();
    assert_eq!(
        occurrences, 1,
        "expected exactly one @moth/runtime entry, got:\n{map}"
    );
}

// Test helpers

fn create_module_with_runtime_requirement() -> Module {
    let mut string_table = StringTable::new();
    let mut module = create_test_module(PathBuf::from("@page.moth"), &mut string_table);
    module.link_facts.external_import_candidates.push(
        crate::compiler_frontend::module_compilation::ModuleExternalImport {
            package_id: ExternalPackageId(0),
            runtime_asset: None,
            required_runtime_imports: vec![
                crate::builder_surface::external_import_providers::provider::RequiredRuntimeImport {
                    module_name: "@moth/runtime".to_owned(),
                    imported_names: vec!["mothOk".to_owned(), "mothErr".to_owned()],
                },
            ],
        },
    );
    module
}

fn create_registry_with_export(
    name: &str,
    export_name: &str,
) -> (
    ExternalPackageRegistry,
    ExternalFunctionId,
    ExternalPackageId,
) {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package(
            "test/pkg",
            crate::builder_surface::PackageOrigin::ProjectLocal,
        )
        .unwrap();
    let function_id = ExternalFunctionId::Synthetic(42);
    registry
        .register_function_in_package(
            package_id,
            function_id,
            ExternalFunctionDef {
                name: name.to_owned(),
                parameters: Vec::new(),
                returns: vec![ExternalReturnSlot::fresh(ExternalAbiType::I32)],
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::ExternalModuleExport {
                        export_name: export_name.to_owned(),
                    }),
                    wasm: None,
                },
            },
        )
        .unwrap();
    (registry, function_id, package_id)
}
