//! Tests for generated HTML JS glue and runtime module resolution.

use super::import_map::build_import_map_html;
use super::paths::relative_url_path;
use super::runtime_modules::emit_build_runtime_modules;
use super::source::{generate_fallible_wrapper, generate_infallible_wrapper};
use super::*;
use crate::build_system::build::FileKind;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalFunctionDef, ExternalFunctionId, ExternalFunctionLowerings,
    ExternalJsLowering, ExternalPackageId, ExternalPackageRegistry, ExternalReturnSlot,
    ExternalSignatureType, ExternalTypeId,
};
use crate::compiler_frontend::module_compilation::Module;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::external_js::runtime_emission_plan::HtmlExternalRuntimeEmissionPlan;
use crate::projects::html_project::external_js::runtime_module_registry::{
    CoreJsRuntimeModule, RUNTIME_ERROR_CODE_EXPORTS,
};
use crate::projects::html_project::tests::test_support::{
    create_test_module, js_runtime_asset_import,
};
use moth_lexical::numeric::fixed_scalar::FixedScalar;
use moth_lexical::numeric::profile::{FloatPrecision, IntWidth, NumericProfile};
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
        let wrapper = generate_fallible_wrapper(
            &format!("__moth_glue_{suffix}"),
            &format!("invalid{suffix}"),
            release_build,
            profile,
            &[],
            &[],
        )
        .expect("fallible wrapper generation should succeed");
        let code_field = crate::backends::js::builtin_error_code_js_field_name(release_build);
        let message_field = crate::backends::js::builtin_error_message_js_field_name(release_build);

        script.push_str(&format!(
            r#"
function invalid{suffix}() {{ return {{ ok: "yes" }}; }}
{wrapper}
"#
        ));
        if release_build {
            script.push_str(&format!(
                r#"
const releaseResult{suffix} = __moth_glue_{suffix}();
assert.equal(releaseResult{suffix}.tag, "err");
assert.equal(typeof releaseResult{suffix}.value[{code_field:?}], "number");
assert.equal(releaseResult{suffix}.value[{code_field:?}], 0);
assert.equal(
    releaseResult{suffix}.value[{message_field:?}],
    "Invalid result wrapper from external JavaScript function"
);
"#
            ));
        } else {
            script.push_str(&format!(
                r#"
assert.throws(() => __moth_glue_{suffix}(), /Invalid result wrapper/);
"#
            ));
        }
    }

    run_generated_node_script(&script);
}

/// Executes one fallible wrapper per Int profile and build mode against the full foreign-error
/// surface: valid, invalid, missing and integer `-0` code payloads, plus thrown payloads
/// (ordinary `Error`, string, `null`, `undefined`), which must all recover the canonical error
/// tag, the exact thrown message and the plain Number U32 zero code, before the success shape.
#[test]
fn fallible_wrapper_bounds_foreign_error_code_to_u32_across_profiles_and_builds() {
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

        script.push_str(&format!(
            r#"
let mode{suffix} = "error";
let supplied{suffix};
const thrownControls{suffix} = [
    {{ payload: new Error("thrown failure"), message: "thrown failure" }},
    {{ payload: "thrown failure", message: "thrown failure" }},
    {{ payload: null, message: "null" }},
    {{ payload: undefined, message: "undefined" }},
];
let thrownIndex{suffix} = 0;
function {export_name}() {{
    if (mode{suffix} === "throw") throw thrownControls{suffix}[thrownIndex{suffix}].payload;
    if (mode{suffix} === "success") return {{ ok: true, value: 42 }};
    return {{ ok: false, error: supplied{suffix} }};
}}
{wrapper}
const codeField{suffix} = {code_field:?};
const messageField{suffix} = {message_field:?};
for (const validCode of [0, 2147483648, 4294967295]) {{
    supplied{suffix} = {{ message: "failure", code: validCode }};
    const result = {wrapper_name}();
    assert.equal(result.tag, "err");
    assert.equal(typeof result.value[codeField{suffix}], "number");
    assert.equal(result.value[codeField{suffix}], validCode);
    assert.equal(result.value[messageField{suffix}], "failure");
}}
for (const invalidCode of [
    -1,
    4294967296,
    1.5,
    NaN,
    Infinity,
    -Infinity,
    "500",
    undefined,
]) {{
    supplied{suffix} = {{ message: "failure", code: invalidCode }};
    const result = {wrapper_name}();
    assert.equal(result.tag, "err");
    assert.equal(typeof result.value[codeField{suffix}], "number");
    assert.equal(result.value[codeField{suffix}], 0);
    assert.equal(result.value[messageField{suffix}], "failure");
}}
supplied{suffix} = {{ message: "failure" }};
let missing{suffix} = {wrapper_name}();
assert.equal(typeof missing{suffix}.value[codeField{suffix}], "number");
assert.equal(missing{suffix}.value[codeField{suffix}], 0);
supplied{suffix} = {{ message: "failure", code: -0 }};
let negZero{suffix} = {wrapper_name}();
assert.equal(negZero{suffix}.value[codeField{suffix}], 0);
assert.equal(
    Object.is(negZero{suffix}.value[codeField{suffix}], -0),
    false,
    "foreign integer -0 must normalize to positive zero at code ingress"
);

mode{suffix} = "throw";
for (let thrownLane = 0; thrownLane < thrownControls{suffix}.length; thrownLane += 1) {{
    thrownIndex{suffix} = thrownLane;
    const thrownControl = thrownControls{suffix}[thrownLane];
    const thrown = {wrapper_name}();
    assert.equal(thrown.tag, "err");
    assert.equal(typeof thrown.value[codeField{suffix}], "number");
    assert.equal(
        thrown.value[codeField{suffix}],
        0,
        "every thrown foreign payload must recover the plain Number U32 zero code"
    );
    assert.equal(thrown.value[messageField{suffix}], thrownControl.message);
}}
mode{suffix} = "success";
let success{suffix} = {wrapper_name}();
assert.deepEqual(success{suffix}, {{ tag: "ok", value: 42 }});
mode{suffix} = "error";
"#
        ));
    }

    run_generated_node_script(&script);
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
            IntWidth::Bits32 => ("Infinity", "-1", "17", "0"),
            IntWidth::Bits64 => ("1e20", "-1", "17", "0"),
        };

        script.push_str(&format!(
            r#"
let getterReads{suffix} = 0;
let initialCode{suffix} = 17;
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
assert.equal(typeof result{suffix}.value[codeField{suffix}], "number");
assert.equal(result{suffix}.value[codeField{suffix}], {expected_code});
assert.equal(result{suffix}.value[messageField{suffix}], "accessor failure");
assert.equal(getterReads{suffix}, 1);

getterReads{suffix} = 0;
initialCode{suffix} = {invalid_initial_code};
changingCode{suffix} = 29;
repeatedReads{suffix} = 1;
result{suffix} = {wrapper_name}();
assert.equal(result{suffix}.tag, "err");
assert.equal(typeof result{suffix}.value[codeField{suffix}], "number");
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
fn native_uint_parameters_validate_before_foreign_calls() {
    let uint32_wrapper = generate_infallible_wrapper(
        "wrapNativeUintInput",
        "rawNativeUintInput",
        &[ExternalSignatureType::NativeUint],
        &[ExternalSignatureType::NativeUint],
        NumericProfile::STANDARD,
    )
    .expect("native Uint32 input wrapper generation should succeed");
    let uint64_profile = NumericProfile {
        int_width: IntWidth::Bits64,
        ..NumericProfile::STANDARD
    };
    let uint64_wrapper = generate_infallible_wrapper(
        "wrapNativeUint64Input",
        "rawNativeUint64Input",
        &[ExternalSignatureType::NativeUint],
        &[ExternalSignatureType::NativeUint],
        uint64_profile,
    )
    .expect("native Uint64 input wrapper generation should succeed");
    let script = format!(
        r#"
import assert from "node:assert/strict";
let uint32Calls = 0;
let lastUint32;
function rawNativeUintInput(value) {{
    uint32Calls += 1;
    lastUint32 = value;
    return value;
}}
let uint64Calls = 0;
let lastUint64;
function rawNativeUint64Input(value) {{
    uint64Calls += 1;
    lastUint64 = value;
    return value;
}}
{uint32_wrapper}
{uint64_wrapper}

// Uint32 accepts the unsigned endpoints and canonicalises integer -0; wrong
// carriers, non-integers, negatives and neighbours of the range never call out.
for (const value of [0, 4294967295]) {{
    assert.equal(wrapNativeUintInput(value), value);
    assert.equal(typeof lastUint32, "number");
    assert.equal(lastUint32, value);
}}
assert.equal(uint32Calls, 2);
assert.equal(Object.is(wrapNativeUintInput(-0), 0), true);
assert.equal(Object.is(lastUint32, 0), true);
assert.equal(uint32Calls, 3);
for (const value of [-1, 4294967296, 1.5, NaN, Infinity, -Infinity, "7", 1n, null, undefined, true]) {{
    assert.throws(() => wrapNativeUintInput(value), RangeError);
}}
assert.equal(uint32Calls, 3, "invalid Uint32 inputs must be rejected before the raw export call");

// Uint64 crosses as a safe Number only: BigInt inputs inside the foreign range
// forward exactly, while negatives, unsafe magnitudes and non-BigInt carriers fail.
for (const value of [0n, 9007199254740991n]) {{
    const expected = Number(value);
    assert.equal(wrapNativeUint64Input(value), BigInt(expected));
    assert.equal(typeof lastUint64, "number");
    assert.equal(lastUint64, expected);
}}
assert.equal(uint64Calls, 2);
assert.equal(wrapNativeUint64Input(0n), 0n);
assert.equal(uint64Calls, 3);
for (const value of [-1n, 9007199254740992n, 18446744073709551615n, 0, -0, 9007199254740991, "7", null, undefined, true]) {{
    assert.throws(() => wrapNativeUint64Input(value), RangeError);
}}
assert.equal(uint64Calls, 3, "invalid Uint64 inputs must be rejected before the raw export call");
"#
    );
    run_generated_node_script(&script);
}

#[test]
fn fallible_native_uint32_adapter_reads_success_once_before_boundary_throw() {
    let wrapper = generate_fallible_wrapper(
        "wrapNativeUint",
        "rawNativeUint",
        false,
        NumericProfile::STANDARD,
        &[],
        &[ExternalSignatureType::NativeUint],
    )
    .expect("fallible native Uint32 wrapper generation should succeed");
    let script = format!(
        r#"
import assert from "node:assert/strict";
let rawValue = 4294967295;
let rawValueReads = 0;
function rawNativeUint() {{
    return {{ ok: true, get value() {{ rawValueReads += 1; return rawValue; }} }};
}}
{wrapper}
let result = wrapNativeUint();
assert.equal(result.tag, "ok");
assert.equal(result.value, 4294967295);
assert.equal(rawValueReads, 1);
rawValueReads = 0;
rawValue = 4294967296;
assert.throws(() => wrapNativeUint(), RangeError);
assert.equal(rawValueReads, 1, "the violating success value is evaluated once before rejection");
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
        &[ExternalSignatureType::Abi(ExternalAbiType::Fixed(
            FixedScalar::I32,
        ))],
        &[ExternalSignatureType::Abi(ExternalAbiType::Fixed(
            FixedScalar::I32,
        ))],
        NumericProfile::STANDARD,
    )
    .expect("fixed I32 wrapper generation should succeed");
    let mixed_wrapper = generate_infallible_wrapper(
        "wrapMixed",
        "rawMixed",
        &[
            ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::F64)),
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
assert.equal(typeof result.value[codeField], "number");
assert.equal(result.value[codeField], 500);
assert.equal(result.value[messageField], "foreign error");
mode = "throw";
result = wrapNativeInt(5000000000n);
assert.equal(result.tag, "err");
assert.equal(typeof result.value[codeField], "number");
assert.equal(result.value[codeField], 0);
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
    for (int_width, suffix) in [(IntWidth::Bits32, "Int32"), (IntWidth::Bits64, "Int64")] {
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
            &[ExternalSignatureType::Abi(ExternalAbiType::Fixed(
                FixedScalar::F64,
            ))],
        )
        .expect("fallible F64 wrapper generation should succeed");
        let infallible_wrapper_name = format!("wrapInfallibleF64{suffix}");
        let infallible_export_name = format!("rawInfallibleF64{suffix}");
        let infallible_wrapper = generate_infallible_wrapper(
            &infallible_wrapper_name,
            &infallible_export_name,
            &[],
            &[ExternalSignatureType::Abi(ExternalAbiType::Fixed(
                FixedScalar::F64,
            ))],
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
    assert.equal(typeof result{suffix}.value[codeField{suffix}], "number");
    assert.equal(result{suffix}.value[codeField{suffix}], 304);
    assert.equal(
        result{suffix}.value[messageField{suffix}],
        "External Float boundary produced a non-finite value"
    );
    assert.throws(() => {infallible_wrapper_name}(), RangeError);
}}

mode{suffix} = "foreign-error";
result{suffix} = {wrapper_name}();
assert.equal(result{suffix}.tag, "err");
assert.equal(typeof result{suffix}.value[codeField{suffix}], "number");
assert.equal(result{suffix}.value[codeField{suffix}], 91);
assert.equal(result{suffix}.value[messageField{suffix}], "foreign error");
mode{suffix} = "throw";
result{suffix} = {wrapper_name}();
assert.equal(result{suffix}.tag, "err");
assert.equal(typeof result{suffix}.value[codeField{suffix}], "number");
assert.equal(result{suffix}.value[codeField{suffix}], 0);
assert.equal(result{suffix}.value[messageField{suffix}], "foreign exception");
"#
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

/// Executes every generated integer-result wrapper configuration against one shared assertion
/// body: valid endpoints, integer -0 canonicalization (including the signed-zero Number
/// conversion on the Int64 bridge), exact-once result evaluation and the full invalid-payload
/// gate (fractional, non-finite, adjacent out-of-range and non-Number) on every lane.
#[test]
fn native_and_fixed_integer_results_validate_before_moth_observation() {
    let standard_profile = NumericProfile::STANDARD;
    let int64_profile = NumericProfile {
        int_width: IntWidth::Bits64,
        ..NumericProfile::STANDARD
    };

    #[derive(Clone, Copy)]
    enum WrapperLane {
        Infallible,
        DebugFallible,
        ReleaseFallible,
    }

    let lane_suffix = |lane| match lane {
        WrapperLane::Infallible => "Infallible",
        WrapperLane::DebugFallible => "DebugFallible",
        WrapperLane::ReleaseFallible => "ReleaseFallible",
    };

    let mut script = String::from(
        "import assert from \"node:assert/strict\";\n\nconst payloads = new Map();\nconst lanes = new Map();\nconst rawCalls = new Map();\nconst getterReads = new Map();\n\nfunction rawPayload(rawName) {\n    rawCalls.set(rawName, (rawCalls.get(rawName) ?? 0) + 1);\n    const value = payloads.get(rawName);\n    if (lanes.get(rawName) === \"wrapped\") {\n        return {\n            ok: true,\n            get value() {\n                getterReads.set(rawName, (getterReads.get(rawName) ?? 0) + 1);\n                return value;\n            },\n        };\n    }\n    return value;\n}\n",
    );

    for (label, return_type, profile, lane) in [
        (
            "NativeInt32",
            ExternalSignatureType::NativeInt,
            standard_profile,
            WrapperLane::Infallible,
        ),
        (
            "NativeInt32",
            ExternalSignatureType::NativeInt,
            standard_profile,
            WrapperLane::DebugFallible,
        ),
        (
            "NativeInt32",
            ExternalSignatureType::NativeInt,
            standard_profile,
            WrapperLane::ReleaseFallible,
        ),
        (
            "NativeUint32",
            ExternalSignatureType::NativeUint,
            standard_profile,
            WrapperLane::Infallible,
        ),
        (
            "NativeUint32",
            ExternalSignatureType::NativeUint,
            standard_profile,
            WrapperLane::DebugFallible,
        ),
        (
            "NativeUint32",
            ExternalSignatureType::NativeUint,
            standard_profile,
            WrapperLane::ReleaseFallible,
        ),
        (
            "FixedI32",
            ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::I32)),
            standard_profile,
            WrapperLane::Infallible,
        ),
        (
            "FixedI32",
            ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::I32)),
            standard_profile,
            WrapperLane::DebugFallible,
        ),
        (
            "FixedI32",
            ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::I32)),
            standard_profile,
            WrapperLane::ReleaseFallible,
        ),
        (
            "NativeInt64",
            ExternalSignatureType::NativeInt,
            int64_profile,
            WrapperLane::Infallible,
        ),
        (
            "NativeInt64",
            ExternalSignatureType::NativeInt,
            int64_profile,
            WrapperLane::DebugFallible,
        ),
        (
            "NativeInt64",
            ExternalSignatureType::NativeInt,
            int64_profile,
            WrapperLane::ReleaseFallible,
        ),
        (
            "NativeUint64",
            ExternalSignatureType::NativeUint,
            int64_profile,
            WrapperLane::Infallible,
        ),
        (
            "NativeUint64",
            ExternalSignatureType::NativeUint,
            int64_profile,
            WrapperLane::DebugFallible,
        ),
        (
            "NativeUint64",
            ExternalSignatureType::NativeUint,
            int64_profile,
            WrapperLane::ReleaseFallible,
        ),
    ] {
        let suffix = lane_suffix(lane);
        let wrapper_name = format!("wrap{label}{suffix}");
        let export_name = format!("raw{label}{suffix}");
        let generated = match lane {
            WrapperLane::Infallible => generate_infallible_wrapper(
                &wrapper_name,
                &export_name,
                &[],
                &[return_type],
                profile,
            ),
            WrapperLane::DebugFallible => generate_fallible_wrapper(
                &wrapper_name,
                &export_name,
                false,
                profile,
                &[],
                &[return_type],
            ),
            WrapperLane::ReleaseFallible => generate_fallible_wrapper(
                &wrapper_name,
                &export_name,
                true,
                profile,
                &[],
                &[return_type],
            ),
        }
        .unwrap_or_else(|error| {
            panic!("{label} {suffix} wrapper generation should succeed: {error:?}")
        });

        let lane_state = if matches!(lane, WrapperLane::Infallible) {
            "plain"
        } else {
            "wrapped"
        };
        script.push_str(&format!(
            "\nlanes.set(\"{export_name}\", \"{lane_state}\");\nfunction {export_name}() {{ return rawPayload(\"{export_name}\"); }}\n{generated}\n"
        ));
        script.push_str(&format!(
            "\nassertIntegerResultBehavior({wrapper_name}, \"{export_name}\", {}, {});\n",
            label == "NativeInt64" || label == "NativeUint64",
            label == "NativeUint32" || label == "NativeUint64",
        ));
    }

    // One assertion body runs per configuration; only the value carrier differs (BigInt for the
    // 64-bit bridge lanes, exact Number for 32-bit/fixed lanes) and only the unsigned lanes
    // shift the valid/invalid integer window.
    script.push_str(
        r#"
function unwrapIntegerResult(result) {
    if (result && typeof result === "object" && "tag" in result) {
        assert.equal(result.tag, "ok");
        return result.value;
    }
    return result;
}

function assertIntegerResultBehavior(wrapper, rawName, isBigInt, isUnsigned) {
    const endpoints = isBigInt
        ? (isUnsigned ? [0, 9007199254740991] : [-9007199254740991, 0, 9007199254740991])
        : (isUnsigned ? [0, 4294967295] : [-2147483648, 0, 2147483647]);
    const invalids = isBigInt
        ? [
              1.5,
              NaN,
              Infinity,
              -Infinity,
              9007199254740992,
              isUnsigned ? -1 : -9007199254740992,
              "7",
              1n,
              null,
              undefined,
              true,
          ]
        : [
              1.5,
              NaN,
              Infinity,
              -Infinity,
              isUnsigned ? 4294967296 : 2147483648,
              isUnsigned ? -1 : -2147483649,
              "7",
              1n,
              null,
              undefined,
              true,
          ];
    // Valid endpoints survive, and every wrapper call evaluates the raw result exactly once.
    for (const endpointValue of endpoints) {
        payloads.set(rawName, endpointValue);
        const callsBefore = rawCalls.get(rawName) ?? 0;
        const adapted = unwrapIntegerResult(wrapper());
        assert.equal(
            rawCalls.get(rawName),
            callsBefore + 1,
            "the raw export must run exactly once per wrapper call"
        );
        assert.equal(adapted, isBigInt ? BigInt(endpointValue) : endpointValue);
    }

    // Integer -0 canonicalizes at ingress: a foreign signed zero cannot enter Moth as an
    // integer -0, and neither integer-to-Number nor integer-to-Float conversion can inherit it.
    payloads.set(rawName, -0);
    const adaptedZero = unwrapIntegerResult(wrapper());
    if (isBigInt) {
        assert.equal(adaptedZero, 0n);
        assert.equal(
            Object.is(Number(adaptedZero), -0),
            false,
            "the bridged integer zero must convert to Number +0"
        );
    } else {
        assert.equal(Object.is(adaptedZero, 0), true);
        assert.equal(Object.is(adaptedZero, -0), false);
        assert.equal(
            1 / adaptedZero,
            Infinity,
            "integer -0 must not survive into a promoted Float zero"
        );
    }

    // Fallible success getters are read exactly once.
    if (lanes.get(rawName) === "wrapped") {
        payloads.set(rawName, 0);
        getterReads.set(rawName, 0);
        unwrapIntegerResult(wrapper());
        assert.equal(
            getterReads.get(rawName),
            1,
            "the success payload must be read exactly once"
        );
    }

    // Every invalid payload is rejected on every lane: adaptation happens outside the
    // raw-export try/catch, so non-finite and non-Number payloads throw the same way.
    for (const invalid of invalids) {
        payloads.set(rawName, invalid);
        assert.throws(() => wrapper(), RangeError);
    }
}
"#,
    );

    run_generated_node_script(&script);
}

/// Imports the generated `@moth/runtime` source and returns every exported error code through
/// real fallible wrappers, so the asset-visible name, the canonical enum value and Moth's
/// `Error.code` agree under both Int widths and both wrapper build modes.
#[test]
fn runtime_error_code_exports_reach_moth_error_unchanged() {
    let runtime_source = CoreJsRuntimeModule::moth_runtime_v1().source;
    let mut script = format!(
        "import assert from \"node:assert/strict\";\nconst runtime = await import(\"data:text/javascript,\" + encodeURIComponent({runtime_source:?}));\n"
    );

    for (export_name, code) in RUNTIME_ERROR_CODE_EXPORTS {
        let expected_code = code.as_u32();
        script.push_str(&format!(
            "assert.equal(runtime.{export_name}, {expected_code});\nfunction raw{export_name}() {{ return runtime.mothErr(runtime.{export_name}, \"{export_name} failed\"); }}\n"
        ));

        for int_width in [IntWidth::Bits32, IntWidth::Bits64] {
            for release_build in [false, true] {
                let profile = NumericProfile {
                    int_width,
                    ..NumericProfile::STANDARD
                };
                let wrapper_name = format!("wrap{export_name}{int_width:?}{release_build}");
                let wrapper = generate_fallible_wrapper(
                    &wrapper_name,
                    &format!("raw{export_name}"),
                    release_build,
                    profile,
                    &[],
                    &[ExternalSignatureType::NativeInt],
                )
                .expect("fallible wrapper generation should succeed");
                let code_field =
                    crate::backends::js::builtin_error_code_js_field_name(release_build);
                let message_field =
                    crate::backends::js::builtin_error_message_js_field_name(release_build);
                script.push_str(&format!(
                    "{wrapper}\n{{\n    const result = {wrapper_name}();\n    assert.equal(result.tag, \"err\");\n    assert.equal(typeof result.value[{code_field:?}], \"number\");\n    assert.equal(result.value[{code_field:?}], {expected_code});\n    assert.equal(result.value[{message_field:?}], \"{export_name} failed\");\n}}\n"
                ));
            }
        }
    }

    run_generated_node_script(&script);
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
                returns: vec![ExternalReturnSlot::fresh(ExternalAbiType::Fixed(
                    FixedScalar::I32,
                ))],
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

/// Fixed boundaries use the same Number carriers in all four native profiles. Execute real
/// wrappers, including success getters, rather than pinning their generated implementation.
#[test]
fn fixed_u32_and_f32_boundaries_are_profile_independent_on_every_wrapper_lane() {
    let mut script = String::from(
        r#"import assert from "node:assert/strict";
function assertFixedBoundary(wrapper, state, isFloat, fallible, codeField, boundaryCode) {
    const maxFloat = (2 - 2 ** -23) * 2 ** 127;
    const minFloat = 2 ** -149;
    const overflowMidpoint = 2 ** 128 - 2 ** 103;
    const validParameters = isFloat
        ? [0, -0, 1.5, Math.fround(0.1), minFloat, -minFloat, maxFloat, -maxFloat]
        : [0, -0, 17, 2147483648, 4294967295];
    function callOnce(input) {
        const calls = state.calls;
        const reads = state.reads;
        let result;
        try {
            result = wrapper(input);
        } finally {
            assert.equal(state.calls, calls + 1, "foreign export evaluated once");
            assert.equal(state.reads, reads + (fallible ? 1 : 0), "success snapshot read once");
        }
        if (fallible) {
            assert.equal(result.tag, "ok");
            return result.value;
        }
        return result;
    }
    state.mode = "echo";
    for (const input of validParameters) {
        const value = callOnce(input);
        const expected = !isFloat && input === 0 ? 0 : input;
        assert.equal(Object.is(state.received, expected), true);
        assert.equal(Object.is(value, expected), true);
        assert.equal(typeof value, "number");
    }
    const hostileCarrier = { valueOf() { throw new Error("must not coerce"); } };
    const wrongCarriers = ["1", 1n, null, undefined, true, hostileCarrier];
    const invalidParameters = isFloat
        ? [0.1, 2 ** -150, overflowMidpoint, -overflowMidpoint, NaN, Infinity, -Infinity, ...wrongCarriers]
        : [-1, 1.5, 4294967296, NaN, Infinity, -Infinity, ...wrongCarriers];
    for (const invalid of invalidParameters) {
        const calls = state.calls;
        assert.throws(() => wrapper(invalid), RangeError);
        assert.equal(state.calls, calls, "invalid argument rejected before foreign invocation");
    }
    state.mode = "value";
    const validResults = isFloat
        ? [
            [1.5, 1.5],
            [0.1, Math.fround(0.1)],
            [1 + 2 ** -24, 1],
            [1 + 3 * 2 ** -24, 1 + 2 ** -22],
            [-0, -0],
            [minFloat, minFloat],
            [2 ** -150, 0],
            [-(2 ** -150), -0],
            [3 * 2 ** -150, 2 * minFloat],
            [maxFloat, maxFloat],
            [overflowMidpoint - 2 ** 75, maxFloat],
            [-overflowMidpoint + 2 ** 75, -maxFloat],
        ]
        : [[0, 0], [-0, 0], [2147483648, 2147483648], [4294967295, 4294967295]];
    for (const [raw, expected] of validResults) {
        state.value = raw;
        assert.equal(Object.is(callOnce(1), expected), true);
    }
    const invalidResults = isFloat
        ? [overflowMidpoint, -overflowMidpoint, 3.5e38, -3.5e38, NaN, Infinity, -Infinity, ...wrongCarriers]
        : [-1, 1.5, 4294967296, NaN, Infinity, -Infinity, ...wrongCarriers];
    for (const invalid of invalidResults) {
        state.value = invalid;
        const calls = state.calls;
        const reads = state.reads;
        if (isFloat && fallible) {
            const result = wrapper(1);
            assert.equal(result.tag, "err");
            assert.equal(result.value[codeField], boundaryCode);
            assert.equal(typeof result.value[codeField], "number");
        } else {
            assert.throws(() => wrapper(1), RangeError);
        }
        assert.equal(state.calls, calls + 1);
        assert.equal(state.reads, reads + (fallible ? 1 : 0));
    }
    if (fallible) {
        state.mode = "error";
        const foreignError = wrapper(1);
        assert.equal(foreignError.tag, "err");
        assert.equal(foreignError.value[codeField], 4294967295);
        state.mode = "throw";
        const foreignThrow = wrapper(1);
        assert.equal(foreignThrow.tag, "err");
        assert.equal(foreignThrow.value[codeField], 0);
    }
}
"#,
    );
    for int_width in [IntWidth::Bits32, IntWidth::Bits64] {
        for float_precision in [FloatPrecision::Bits32, FloatPrecision::Bits64] {
            let profile = NumericProfile {
                int_width,
                float_precision,
            };
            for scalar in [FixedScalar::U32, FixedScalar::F32] {
                let signature = ExternalSignatureType::Abi(ExternalAbiType::Fixed(scalar));
                for (lane_index, release_build) in
                    [None, Some(false), Some(true)].into_iter().enumerate()
                {
                    let suffix = format!(
                        "{}{:?}{:?}{lane_index}",
                        scalar.name(),
                        int_width,
                        float_precision
                    );
                    let wrapper_name = format!("wrap{suffix}");
                    let raw_name = format!("raw{suffix}");
                    let generated = if let Some(release_build) = release_build {
                        generate_fallible_wrapper(
                            &wrapper_name,
                            &raw_name,
                            release_build,
                            profile,
                            std::slice::from_ref(&signature),
                            std::slice::from_ref(&signature),
                        )
                    } else {
                        generate_infallible_wrapper(
                            &wrapper_name,
                            &raw_name,
                            std::slice::from_ref(&signature),
                            std::slice::from_ref(&signature),
                            profile,
                        )
                    }
                    .expect("supported fixed wrapper generates");
                    let fallible = release_build.is_some();
                    let code_field = crate::backends::js::builtin_error_code_js_field_name(
                        release_build.unwrap_or(false),
                    );
                    let boundary_code = crate::compiler_frontend::builtins::error_codes::BuiltinErrorCode::FloatBoundaryNonFinite.as_u32();
                    script.push_str(&format!(r#"
const state{suffix} = {{ mode: "echo", calls: 0, reads: 0, value: 0, received: undefined }};
function {raw_name}(input) {{
    const state = state{suffix};
    state.calls += 1;
    state.received = input;
    if (state.mode === "throw") throw new Error("foreign exception");
    if (state.mode === "error") return {{ ok: false, error: {{ code: 4294967295, message: "foreign error" }} }};
    const value = state.mode === "echo" ? input : state.value;
    return {fallible} ? {{ ok: true, get value() {{ state.reads += 1; return value; }} }} : value;
}}
{generated}
assertFixedBoundary({wrapper_name}, state{suffix}, {}, {fallible}, {code_field:?}, {boundary_code});
"#, scalar == FixedScalar::F32));
                }
            }
        }
    }
    run_generated_node_script(&script);
}

#[test]
fn glue_rejects_undelivered_fixed_scalars_despite_internal_carrier_support() {
    for scalar in [
        FixedScalar::I8,
        FixedScalar::I16,
        FixedScalar::I64,
        FixedScalar::U8,
        FixedScalar::U16,
        FixedScalar::U64,
        FixedScalar::F16,
        FixedScalar::Byte,
    ] {
        let signature = ExternalSignatureType::Abi(ExternalAbiType::Fixed(scalar));
        for (parameters, returns) in [
            (std::slice::from_ref(&signature), &[][..]),
            (&[][..], std::slice::from_ref(&signature)),
        ] {
            assert!(
                generate_infallible_wrapper(
                    "wrapUnsupported",
                    "rawUnsupported",
                    parameters,
                    returns,
                    NumericProfile::STANDARD,
                )
                .is_err()
            );
            assert!(
                generate_fallible_wrapper(
                    "wrapUnsupported",
                    "rawUnsupported",
                    false,
                    NumericProfile::STANDARD,
                    parameters,
                    returns,
                )
                .is_err()
            );
        }
    }
}
