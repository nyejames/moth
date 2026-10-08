use crate::projects::html_project::external_js::parser::{
    parse_js_module, parsed_js_module::JsDiagnosticKind, scan_exports,
};
use crate::projects::html_project::external_js::runtime_module_registry::{
    RUNTIME_ERROR_CODE_EXPORTS, RuntimeModuleRegistry,
};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

// ------------------------
//  Helpers
// ------------------------

fn parse(
    source: &str,
) -> crate::projects::html_project::external_js::parser::parsed_js_module::ParsedJsModule {
    let registry = RuntimeModuleRegistry::v1();
    parse_js_module(source, &registry)
}

fn assert_opaque_types(
    parsed: &crate::projects::html_project::external_js::parser::parsed_js_module::ParsedJsModule,
    expected: &[&str],
) {
    let names: Vec<&str> = parsed
        .opaque_types
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(names, expected, "opaque types mismatch");
}

fn assert_free_functions(
    parsed: &crate::projects::html_project::external_js::parser::parsed_js_module::ParsedJsModule,
    expected: &[&str],
) {
    let names: Vec<&str> = parsed
        .free_functions
        .iter()
        .map(|f| f.moth_name.as_str())
        .collect();
    assert_eq!(names, expected, "free functions mismatch");
}

fn assert_receiver_methods(
    parsed: &crate::projects::html_project::external_js::parser::parsed_js_module::ParsedJsModule,
    expected: &[&str],
) {
    let names: Vec<&str> = parsed
        .receiver_methods
        .iter()
        .map(|f| f.moth_name.as_str())
        .collect();
    assert_eq!(names, expected, "receiver methods mismatch");
}

fn assert_diagnostic_kinds(
    parsed: &crate::projects::html_project::external_js::parser::parsed_js_module::ParsedJsModule,
    expected: &[JsDiagnosticKind],
) {
    let kinds: Vec<JsDiagnosticKind> = parsed.diagnostics.iter().map(|d| d.kind.clone()).collect();
    assert_eq!(
        kinds,
        expected,
        "diagnostic kinds mismatch. Messages: {:?}",
        parsed
            .diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
    );
}

fn assert_runtime_imports(
    parsed: &crate::projects::html_project::external_js::parser::parsed_js_module::ParsedJsModule,
    expected: &[(&str, &[&str])],
) {
    assert_eq!(
        parsed.runtime_imports.len(),
        expected.len(),
        "runtime import count mismatch"
    );
    for (index, (module_name, names)) in expected.iter().enumerate() {
        let runtime_import = &parsed.runtime_imports[index];
        assert_eq!(
            runtime_import.module_name, *module_name,
            "runtime import module name mismatch at index {index}"
        );
        let expected_names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        assert_eq!(
            runtime_import.imported_names, expected_names,
            "runtime import names mismatch at index {index}"
        );
    }
}

fn assert_no_diagnostics(
    parsed: &crate::projects::html_project::external_js::parser::parsed_js_module::ParsedJsModule,
) {
    assert!(
        parsed.diagnostics.is_empty(),
        "expected no diagnostics, got: {:?}",
        parsed.diagnostics
    );
}

// ------------------------
//  Opaque types
// ------------------------

#[test]
fn opaque_type_declarations_are_parsed() {
    let source = r#"
/**
 * @moth.opaque Canvas
 * @moth.opaque Canvas2d
 */
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_opaque_types(&parsed, &["Canvas", "Canvas2d"]);
}

#[test]
fn opaque_type_single_line_block() {
    let source = r#"/** @moth.opaque Handle */"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_opaque_types(&parsed, &["Handle"]);
}

// ------------------------
//  Free function signatures
// ------------------------

#[test]
fn free_function_signature_parsed() {
    let source = r#"
/**
 * @moth.opaque Canvas
 * @moth.sig get_canvas |id String| -> Canvas, Error!
 */
export function getCanvas(id) {
    return mothOk(document.getElementById(id));
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_free_functions(&parsed, &["get_canvas"]);

    let func = &parsed.free_functions[0];
    assert_eq!(func.js_name, "getCanvas");
    assert_eq!(func.signature.parameters.len(), 1);
    assert_eq!(func.signature.parameters[0].name, "id");
    assert_eq!(func.signature.parameters[0].type_name, "String");
    assert!(!func.signature.parameters[0].is_receiver);
    assert_eq!(func.signature.returns.len(), 1);
    assert_eq!(func.signature.returns[0].type_name, "Canvas");
    assert!(func.signature.has_error_return);
}

#[test]
fn free_function_no_return() {
    let source = r#"
/**
 * @moth.sig log_message |msg String|
 */
export function logMessage(msg) {
    console.log(msg);
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_free_functions(&parsed, &["log_message"]);
    let func = &parsed.free_functions[0];
    assert_eq!(func.signature.returns.len(), 0);
    assert!(!func.signature.has_error_return);
}

#[test]
fn free_function_error_only_return() {
    let source = r#"
/**
 * @moth.sig do_fallible || -> Error!
 */
export function doFallible() {
    return mothOk();
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_free_functions(&parsed, &["do_fallible"]);
    assert!(parsed.free_functions[0].signature.has_error_return);
    assert_eq!(parsed.free_functions[0].signature.returns.len(), 0);
}

#[test]
fn const_arrow_export_parsed() {
    let source = r#"
/**
 * @moth.sig add |a Int, b Int| -> Int
 */
export const add = (a, b) => {
    return a + b;
};
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_free_functions(&parsed, &["add"]);
    assert_eq!(parsed.free_functions[0].js_name, "add");
    assert_eq!(parsed.free_functions[0].signature.parameters.len(), 2);
}

#[test]
fn uint_signature_parsed_with_existing_restrictions() {
    let source = r#"
/**
 * @moth.sig identity_uint |value Uint| -> Uint
 */
export function identityUint(value) {
    return value;
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_free_functions(&parsed, &["identity_uint"]);
    let func = &parsed.free_functions[0];
    assert_eq!(func.signature.parameters[0].type_name, "Uint");
    assert_eq!(func.signature.returns[0].type_name, "Uint");
    assert_eq!(func.signature.abi_parameter_count(), 1);
}

#[test]
fn fixed_u32_and_f32_signatures_parse_parameters_and_both_result_lanes() {
    for scalar in ["U32", "F32"] {
        for error_slot in ["", ", Error!"] {
            let source = format!(
                "/** @moth.sig identity |value {scalar}| -> {scalar}{error_slot} */\nexport function identity(value) {{ return value; }}"
            );
            let parsed = parse(&source);
            assert_no_diagnostics(&parsed);
            let function = &parsed.free_functions[0];
            assert_eq!(function.signature.parameters[0].type_name, scalar);
            assert_eq!(function.signature.returns[0].type_name, scalar);
            assert_eq!(function.signature.has_error_return, !error_slot.is_empty());
        }
    }
}

#[test]
fn other_fixed_scalar_annotations_remain_outside_the_signature_subset() {
    for scalar in [
        "I8", "I16", "I32", "I64", "U8", "U16", "U64", "F16", "F64", "Byte",
    ] {
        let source = format!(
            "/** @moth.sig identity |value {scalar}| -> {scalar} */\nexport function identity(value) {{ return value; }}"
        );
        let parsed = parse(&source);
        assert_diagnostic_kinds(
            &parsed,
            &[
                JsDiagnosticKind::UnknownExternalType,
                JsDiagnosticKind::UnknownExternalType,
            ],
        );
    }
}

#[test]
fn fixed_scalar_annotations_do_not_admit_dotted_or_collection_types() {
    for signature in [
        "|value F32.extra| -> F32",
        "|| -> F32.extra",
        "|value {U32}| -> U32",
    ] {
        let source = format!(
            "/** @moth.sig identity {signature} */\nexport function identity{} {{ return 0; }}",
            if signature.starts_with("||") {
                "()"
            } else {
                "(value)"
            }
        );
        let parsed = parse(&source);
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.kind == JsDiagnosticKind::UnsupportedTypeSyntax })
        );
    }
}

#[test]
fn signature_annotation_cannot_bind_literal_constant() {
    let source = r#"
/**
 * @moth.sig answer || -> Int
 */
export const answer = 42;
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::AnnotationExportKindMismatch]);
}

// ------------------------
//  Receiver method signatures
// ------------------------

#[test]
fn receiver_method_signature_parsed() {
    let source = r#"
/**
 * @moth.opaque Canvas2d
 */

/**
 * @moth.sig fill_rect |this ~Canvas2d, x Float, y Float, width Float, height Float|
 */
export function fillRect(ctx, x, y, width, height) {
    ctx.fillRect(x, y, width, height);
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_receiver_methods(&parsed, &["fill_rect"]);

    let func = &parsed.receiver_methods[0];
    assert_eq!(func.js_name, "fillRect");
    assert_eq!(func.signature.parameters.len(), 5);
    assert!(func.signature.parameters[0].is_receiver);
    assert_eq!(func.signature.parameters[0].name, "this");
    assert_eq!(func.signature.parameters[0].type_name, "Canvas2d");
    assert!(func.signature.parameters[0].is_mutable);
}

#[test]
fn receiver_method_immutable_receiver() {
    let source = r#"
/**
 * @moth.sig describe |this String| -> String
 */
export const describe = (self) => {
    return self;
};
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_receiver_methods(&parsed, &["describe"]);
    assert!(!parsed.receiver_methods[0].signature.parameters[0].is_mutable);
}

#[test]
fn regular_mutable_parameter_marker_is_parsed() {
    let source = r#"
/**
 * @moth.opaque Buffer
 * @moth.sig write |buffer ~Buffer, text String|
 */
export function write(buffer, text) {
    buffer.value = text;
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_free_functions(&parsed, &["write"]);
    assert!(parsed.free_functions[0].signature.parameters[0].is_mutable);
    assert_eq!(
        parsed.free_functions[0].signature.parameters[0].type_name,
        "Buffer"
    );
}

// ------------------------
//  Invalid receiver parameter
// ------------------------

#[test]
fn receiver_parameter_must_be_first() {
    let source = r#"
/**
 * @moth.opaque Canvas2d
 * @moth.sig bad |x Float, this ~Canvas2d|
 */
export function bad(x, ctx) {}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::InvalidReceiverParameter]);
    // Malformed `this` at index 1 means has_receiver() is false, so the function
    // lands in free_functions rather than receiver_methods.
    assert_free_functions(&parsed, &["bad"]);
    assert!(parsed.receiver_methods.is_empty());
}

#[test]
fn duplicate_receiver_parameter_rejected() {
    let source = r#"
/**
 * @moth.opaque Canvas2d
 * @moth.sig bad |this ~Canvas2d, this Canvas2d|
 */
export function bad(ctx, other) {}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(
        &parsed,
        &[
            JsDiagnosticKind::InvalidReceiverParameter,
            JsDiagnosticKind::InvalidReceiverParameter,
        ],
    );
    // First parameter is a valid receiver, so it still becomes a receiver method.
    assert_receiver_methods(&parsed, &["bad"]);
}

#[test]
fn receiver_parameter_after_recovered_invalid_parameter_is_rejected() {
    let source = r#"
/**
 * @moth.opaque Canvas2d
 * @moth.sig bad |...values, this Canvas2d|
 */
export function bad(values, ctx) {}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(
        &parsed,
        &[
            JsDiagnosticKind::UnsupportedParameterPattern,
            JsDiagnosticKind::InvalidReceiverParameter,
            JsDiagnosticKind::ArityMismatch,
        ],
    );
}

#[test]
fn receiver_parameter_missing_type_annotation_still_reported() {
    let source = r#"
/**
 * @moth.sig bad |this|
 */
export function bad(ctx) {}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::UnsupportedTypeSyntax]);
    assert_receiver_methods(&parsed, &["bad"]);
}

// ------------------------
//  Arity validation
// ------------------------

#[test]
fn arity_mismatch_reported() {
    let source = r#"
/**
 * @moth.opaque Canvas
 * @moth.sig get_canvas |id String, extra String| -> Canvas, Error!
 */
export function getCanvas(id) {
    return mothOk(id);
}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ArityMismatch]);
    assert_free_functions(&parsed, &["get_canvas"]);
}

#[test]
fn receiver_this_counts_in_arity() {
    let source = r#"
/**
 * @moth.opaque Canvas2d
 * @moth.sig fill_rect |this ~Canvas2d, x Float|
 */
export function fillRect(ctx, x, y) {
    ctx.fillRect(x, y);
}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ArityMismatch]);
    assert_receiver_methods(&parsed, &["fill_rect"]);
}

// ------------------------
//  Missing export after @moth.sig
// ------------------------

#[test]
fn missing_export_after_sig_reported() {
    let source = r#"
/**
 * @moth.sig orphaned |id String| -> String
 */
// no export here
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::MissingExportAfterSig]);
    assert!(parsed.free_functions.is_empty());
}

#[test]
fn unknown_external_type_reported() {
    let source = r#"
/**
 * @moth.sig get_canvas |id String| -> Canvas, Error!
 */
export function getCanvas(id) {
    return mothOk(id);
}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::UnknownExternalType]);
}

#[test]
fn unknown_receiver_type_reported() {
    let source = r#"
/**
 * @moth.sig fill_rect |this ~Canvas2d, x Float|
 */
export function fillRect(ctx, x) {
    ctx.fillRect(x, x);
}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::UnknownExternalType]);
    assert_receiver_methods(&parsed, &["fill_rect"]);
}

// ------------------------
//  Duplicate names
// ------------------------

#[test]
fn duplicate_moth_name_reported() {
    let source = r#"
/**
 * @moth.sig get_canvas |id String| -> String
 */
export function getCanvas1(id) { return id; }

/**
 * @moth.sig get_canvas |name String| -> String
 */
export function getCanvas2(name) { return name; }
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::DuplicateMothName]);
}

#[test]
fn duplicate_js_export_name_reported() {
    let source = r#"
/**
 * @moth.sig first |id String| -> String
 */
export function getCanvas(id) { return id; }

/**
 * @moth.sig second |name String| -> String
 */
export function getCanvas(name) { return name; }
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::DuplicateJsExportName]);
}

#[test]
fn duplicate_opaque_type_name_reported() {
    let source = r#"
/**
 * @moth.opaque Handle
 * @moth.opaque Handle
 */
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::DuplicateMothName]);
}

// ------------------------
//  Unannotated exports
// ------------------------

#[test]
fn unannotated_export_rejected() {
    let source = r#"
export function helper(x) {
    return x;
}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::UnannotatedExport]);
}

#[test]
fn unannotated_and_annotated_exports_mixed() {
    let source = r#"
/**
 * @moth.sig public_fn |x Int| -> Int
 */
export function publicFn(x) { return x; }

export function privateHelper(x) { return x; }
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::UnannotatedExport]);
    assert_free_functions(&parsed, &["public_fn"]);
}

// ------------------------
//  @moth.package rejection
// ------------------------

#[test]
fn moth_package_rejected() {
    let source = r#"
/**
 * @moth.package my_package
 */
export function foo() {}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(
        &parsed,
        &[
            JsDiagnosticKind::UnsupportedPackageTag,
            JsDiagnosticKind::UnannotatedExport,
        ],
    );
}

#[test]
fn unknown_moth_directive_rejected() {
    let source = r#"
/**
 * @moth.future value
 */
export function foo() {}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(
        &parsed,
        &[
            JsDiagnosticKind::UnknownMothDirective,
            JsDiagnosticKind::UnannotatedExport,
        ],
    );
}

// ------------------------
//  Default export rejection
// ------------------------

#[test]
fn default_export_rejected() {
    let source = r#"
export default function foo() {}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::DefaultExport]);
}

// ------------------------
//  Re-export rejection
// ------------------------

#[test]
fn re_export_rejected() {
    let source = r#"
export { foo };
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ReExport]);
}

#[test]
fn local_export_list_ignores_comment_text_that_looks_like_from_clause() {
    let source = r#"
export { foo /* from "not-a-module" */ };
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ReExport]);
}

#[test]
fn re_export_from_is_classified_as_module_loading() {
    let source = r#"
export { foo } from "./helper.js";
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ReExportFrom]);
}

#[test]
fn multiline_export_from_is_classified_as_module_loading() {
    let source = "export { foo }\nfrom \"./helper.js\";\n";
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ReExportFrom]);
}

#[test]
fn multiline_star_export_from_is_classified_as_module_loading() {
    let source = "export *\nfrom \"./helper.js\";\n";
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ReExportFrom]);
}

#[test]
fn namespace_star_export_from_is_classified_as_module_loading() {
    let source = "export * as ns from \"./helper.js\";\n";
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ReExportFrom]);
}

#[test]
fn multiline_namespace_star_export_from_is_classified_as_module_loading() {
    let source = "export * as ns\nfrom \"./helper.js\";\n";
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ReExportFrom]);
}

// ------------------------
//  CommonJS rejection
// ------------------------

#[test]
fn commonjs_module_exports_rejected() {
    let source = r#"
module.exports = { foo };
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::CommonJsExport]);
}

#[test]
fn commonjs_exports_dot_rejected() {
    let source = r#"
exports.foo = function() {};
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::CommonJsExport]);
}

#[test]
fn keyword_substrings_and_member_access_are_not_rejected() {
    let source = r#"
const reimport = fn;
object.require("x");
const myexport = 1;
someexports.value = 1;
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
}

// ------------------------
//  Class export rejection
// ------------------------

#[test]
fn class_export_rejected() {
    let source = r#"
export class Widget {}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ClassExport]);
}

// ------------------------
//  Import rejection
// ------------------------

#[test]
fn dynamic_import_rejected() {
    let source = r#"
const m = import("./helper.js");
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::DynamicImport]);
}

#[test]
fn require_call_rejected() {
    let source = r#"
const helper = require("./helper.js");
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::CommonJsRequire]);
}

#[test]
fn star_reexport_rejected() {
    let source = r#"
export * from "./helper.js";
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ReExportFrom]);
}

#[test]
fn star_export_without_from_remains_syntax_only() {
    let source = r#"
export *;
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ReExport]);
}

#[test]
fn import_meta_is_not_an_import_declaration() {
    let source = r#"
const url = import.meta.url;
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
}

#[test]
fn import_meta_does_not_treat_a_later_from_binding_as_an_import() {
    let source = r#"
const base = import.meta.url;
const from = "lodash";
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
}

#[test]
fn regex_literals_with_quotes_do_not_hide_later_imports() {
    let source = r#"
const text = "x";
text.replace(/"/g, "");
import { foo } from "./helper.js";
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ArbitraryImport]);
}

#[test]
fn division_does_not_hide_a_later_dynamic_import() {
    let source = "let value = 1; value++ / 2; import(\"./helper.js\");\n";
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::DynamicImport]);
}

#[test]
fn from_binding_and_string_named_import_clauses_still_load_a_module() {
    let source = r#"
import { from as source } from "./from-helper.js";
import { "feature-name" as feature } from "./named-helper.js";
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(
        &parsed,
        &[
            JsDiagnosticKind::ArbitraryImport,
            JsDiagnosticKind::ArbitraryImport,
        ],
    );
}

#[test]
fn arbitrary_static_import_rejected() {
    let source = r#"
import { foo } from "./helper.js";
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ArbitraryImport]);
}

#[test]
fn namespace_static_import_rejected() {
    let source = r#"
import * as helper from "./helper.js";
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ArbitraryImport]);
}

#[test]
fn side_effect_static_import_rejected() {
    let source = r#"
import "./helper.js";
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ArbitraryImport]);
}

#[test]
fn registered_runtime_import_accepted() {
    let source = r#"
import { mothOk, mothErr } from "@moth/runtime";

/**
 * @moth.sig do_thing || -> Error!
 */
export function doThing() {
    return mothOk();
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_free_functions(&parsed, &["do_thing"]);
}

#[test]
fn unregistered_runtime_looking_module_is_rejected() {
    let source = r#"
import { foo } from "@moth/other-runtime";
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ArbitraryImport]);
}

#[test]
fn escaped_runtime_looking_specifiers_are_not_allowlisted() {
    let source = "import { mothOk } from \"@moth/ru\\ntime\";\n";
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ArbitraryImport]);
}

#[test]
fn v1_runtime_registry_contains_only_moth_runtime() {
    let registry = RuntimeModuleRegistry::v1();
    assert!(registry.is_registered("@moth/runtime"));
    assert!(!registry.is_registered("@moth/other-runtime"));
    assert!(!registry.is_registered("./helper.js"));
    let modules = registry.registered_modules();
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0].specifier, "@moth/runtime");
}

#[test]
fn runtime_named_import_is_recorded() {
    let source = r#"
import { mothOk, mothErr } from "@moth/runtime";

/**
 * @moth.sig do_thing || -> Int, Error!
 */
export function doThing() {
    return mothOk(7);
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_runtime_imports(&parsed, &[("@moth/runtime", &["mothErr", "mothOk"])]);
}

#[test]
fn runtime_error_code_exports_are_exact_named_imports() {
    let error_code_names = RUNTIME_ERROR_CODE_EXPORTS
        .iter()
        .map(|(export_name, _)| *export_name)
        .collect::<Vec<_>>();
    let source = format!(
        "import {{ mothErr, {} }} from \"@moth/runtime\";\n",
        error_code_names.join(", ")
    );
    let parsed = parse(&source);
    assert_no_diagnostics(&parsed);
    let mut expected_names = error_code_names.clone();
    expected_names.push("mothErr");
    expected_names.sort_unstable();
    assert_runtime_imports(&parsed, &[("@moth/runtime", &expected_names)]);

    // The allowlist is the generated name set, not a `MOTH_ERROR_` prefix rule.
    let parsed = parse("import { MOTH_ERROR_NOT_REGISTERED } from \"@moth/runtime\";\n");
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::UnknownRuntimeImportName]);
    assert!(parsed.runtime_imports.is_empty());
}

#[test]
fn multiline_registered_runtime_import_accepted() {
    let source = r#"
import {
    mothOk,
    mothErr,
} from "@moth/runtime";

/**
 * @moth.sig do_thing || -> Int, Error!
 */
export function doThing() {
    return mothOk(7);
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_runtime_imports(&parsed, &[("@moth/runtime", &["mothErr", "mothOk"])]);
}

#[test]
fn multiline_arbitrary_import_rejected() {
    let source = r#"
import {
    foo,
    bar,
} from "./helper.js";
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ArbitraryImport]);
    assert!(parsed.runtime_imports.is_empty());
}

#[test]
fn non_fallible_function_with_runtime_import_records_import() {
    let source = r#"
import { mothOk } from "@moth/runtime";

/**
 * @moth.sig get_number || -> Int
 */
export function getNumber() {
    return mothOk(7).value;
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_eq!(parsed.free_functions.len(), 1);
    assert!(!parsed.free_functions[0].signature.has_error_return);
    assert_runtime_imports(&parsed, &[("@moth/runtime", &["mothOk"])]);
}

#[test]
fn runtime_import_alias_rejected() {
    let source = r#"
import { mothOk as ok } from "@moth/runtime";
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::UnsupportedRuntimeImportForm]);
    assert!(parsed.runtime_imports.is_empty());
}

#[test]
fn runtime_default_import_rejected() {
    let source = r#"
import runtime from "@moth/runtime";
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::UnsupportedRuntimeImportForm]);
    assert!(parsed.runtime_imports.is_empty());
}

#[test]
fn runtime_namespace_import_rejected() {
    let source = r#"
import * as runtime from "@moth/runtime";
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::UnsupportedRuntimeImportForm]);
    assert!(parsed.runtime_imports.is_empty());
}

#[test]
fn unknown_runtime_import_name_rejected() {
    let source = r#"
import { nope } from "@moth/runtime";
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::UnknownRuntimeImportName]);
    assert!(parsed.runtime_imports.is_empty());
}

#[test]
fn duplicate_runtime_imports_deduplicate() {
    let source = r#"
import { mothOk } from "@moth/runtime";
import { mothErr } from "@moth/runtime";

/**
 * @moth.sig do_thing || -> Int, Error!
 */
export function doThing() {
    return mothOk(7);
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_eq!(parsed.runtime_imports.len(), 1);
    assert_eq!(parsed.runtime_imports[0].module_name, "@moth/runtime");
    assert_eq!(
        parsed.runtime_imports[0].imported_names,
        vec!["mothErr", "mothOk"]
    );
}

#[test]
fn runtime_import_duplicate_names_are_deduplicated() {
    let source = r#"
import { mothOk } from "@moth/runtime";
import { mothOk } from "@moth/runtime";

/**
 * @moth.sig do_thing || -> Int, Error!
 */
export function doThing() {
    return mothOk(7);
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_eq!(parsed.runtime_imports.len(), 1);
    assert_eq!(parsed.runtime_imports[0].module_name, "@moth/runtime");
    assert_eq!(parsed.runtime_imports[0].imported_names, vec!["mothOk"]);
}

#[test]
fn explicit_registry_injected_into_parser() {
    let source = r#"
import { mothOk } from "@moth/runtime";

/**
 * @moth.sig do_thing || -> Error!
 */
export function doThing() {
    return mothOk();
}
"#;
    let registry = RuntimeModuleRegistry::v1();
    let parsed = parse_js_module(source, &registry);
    assert_no_diagnostics(&parsed);
    assert_eq!(parsed.free_functions.len(), 1);
}

#[test]
fn explicit_empty_registry_rejects_all_imports() {
    let source = r#"
import { mothOk } from "@moth/runtime";
"#;
    let registry = RuntimeModuleRegistry::empty();
    let parsed = parse_js_module(source, &registry);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::ArbitraryImport]);
}

#[test]
fn export_keywords_inside_comments_and_strings_are_ignored() {
    let source = r#"
// export function commentedOut() {}
const text = "export function stringOnly() {}";
/*
export function blockCommented() {}
*/
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert!(parsed.free_functions.is_empty());
}

#[test]
fn export_body_with_brace_in_string_does_not_break_scanning() {
    let source = r#"
/**
 * @moth.sig tricky || -> String
 */
export function tricky() {
    const text = "} export function fake() {}";
    return text;
}

/**
 * @moth.sig next || -> Int
 */
export function next() {
    return 1;
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_free_functions(&parsed, &["tricky", "next"]);
}

#[test]
fn export_body_with_import_in_string_does_not_emit_import_diagnostic() {
    let source = r#"
/**
 * @moth.sig tricky || -> String
 */
export function tricky() {
    const text = "import { foo } from './bar.js';";
    return text;
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_free_functions(&parsed, &["tricky"]);
}

#[test]
fn export_inside_template_literal_is_ignored() {
    let source = r#"
const hint = `export function fake() {}`;
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert!(parsed.free_functions.is_empty());
}

#[test]
fn import_inside_line_comment_is_ignored() {
    let source = r#"
// import { foo } from "./helper.js";
const x = 1;
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert!(parsed.runtime_imports.is_empty());
}

#[test]
fn import_inside_block_comment_is_ignored() {
    let source = r#"
/* import { foo } from "./helper.js"; */
const x = 1;
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert!(parsed.runtime_imports.is_empty());
}

#[test]
fn import_inside_template_literal_is_ignored() {
    let source = r#"
const hint = `import { foo } from "./helper.js";`;
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert!(parsed.runtime_imports.is_empty());
}

#[test]
fn template_literal_at_top_level_before_export() {
    let source = r#"
const hint = `}; export function fake() {}`;

/**
 * @moth.sig real || -> Int
 */
export function real() {
    return 1;
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_free_functions(&parsed, &["real"]);
}

#[test]
fn import_statement_with_comment_containing_semicolon() {
    let source = r#"
import {
    mothOk, // this is ok;
    mothErr // this is err;
} from "@moth/runtime";

/**
 * @moth.sig do_thing || -> Int, Error!
 */
export function doThing() {
    return mothOk(7);
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_runtime_imports(&parsed, &[("@moth/runtime", &["mothErr", "mothOk"])]);
}

#[test]
fn export_body_comments_containing_export_are_ignored() {
    let source = r#"
/**
 * @moth.sig tricky || -> Int
 */
export function tricky() {
    // export function fake() {}
    /* export function fake() {} */
    return 1;
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_free_functions(&parsed, &["tricky"]);
}

#[test]
fn template_literal_with_braces_does_not_break_scanning() {
    let source = r#"
/**
 * @moth.sig tricky || -> String
 */
export function tricky() {
    const text = `value ${"{ }"}`;
    return text;
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_free_functions(&parsed, &["tricky"]);
}

#[test]
fn arrow_block_body_with_brace_in_string_handled() {
    let source = r#"
/**
 * @moth.sig tricky || -> String
 */
export const tricky = () => {
    const text = "}";
    return text;
};
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_free_functions(&parsed, &["tricky"]);
}

#[test]
fn expression_bodied_arrow_export_rejected() {
    let source = r#"
/**
 * @moth.sig add |a Int, b Int| -> Int
 */
export const add = (a, b) => a + b;
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(
        &parsed,
        &[
            JsDiagnosticKind::ExpressionBodiedArrowExport,
            JsDiagnosticKind::MissingExportAfterSig,
        ],
    );
    assert!(parsed.free_functions.is_empty());
}

// ------------------------
//  Unsupported parameter patterns
// ------------------------

#[test]
fn rest_parameter_rejected() {
    let source = r#"
/**
 * @moth.sig sum |...values| -> Int
 */
export function sum(...values) {
    return values.reduce((a, b) => a + b, 0);
}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(
        &parsed,
        &[
            JsDiagnosticKind::UnsupportedParameterPattern,
            JsDiagnosticKind::UnsupportedParameterPattern,
        ],
    );
}

#[test]
fn default_parameter_rejected() {
    let source = r#"
/**
 * @moth.sig greet |name String| -> String
 */
export function greet(name = "world") {
    return name;
}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::UnsupportedParameterPattern]);
}

#[test]
fn destructuring_parameter_rejected() {
    let source = r#"
/**
 * @moth.sig unpack |point| -> Int
 */
export function unpack({ x }) {
    return x;
}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(
        &parsed,
        &[
            JsDiagnosticKind::UnsupportedParameterPattern,
            JsDiagnosticKind::UnsupportedTypeSyntax,
            JsDiagnosticKind::ArityMismatch,
        ],
    );
}

// ------------------------
//  Unsupported type syntax
// ------------------------

#[test]
fn collection_type_in_signature_rejected() {
    let source = r#"
/**
 * @moth.sig process |items {String}| -> String
 */
export function process(items) {
    return items[0];
}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::UnsupportedTypeSyntax]);
}

#[test]
fn option_type_in_signature_rejected() {
    let source = r#"
/**
 * @moth.sig maybe |name String?| -> String
 */
export function maybe(name) {
    return name || "";
}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::UnsupportedTypeSyntax]);
}

#[test]
fn generic_external_function_signature_rejected() {
    let source = r#"
/**
 * @moth.sig identity type A |value A| -> A
 */
export function identity(value) {
    return value;
}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::GenericExternalFunction]);
}

#[test]
fn generic_external_opaque_type_rejected() {
    let source = r#"
/**
 * @moth.opaque Canvas of Int
 */
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::GenericExternalType]);
    assert_opaque_types(&parsed, &[]);
}

#[test]
fn void_return_rejected() {
    let source = r#"
/**
 * @moth.sig noop || -> Void
 */
export function noop() {}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::VoidReturn]);
}

#[test]
fn multi_success_return_rejected() {
    let source = r#"
/**
 * @moth.sig pair || -> Int, String
 */
export function pair() {
    return [1, "a"];
}
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::MultiSuccessReturn]);
}

// ------------------------
//  Snake-case / camelCase mapping
// ------------------------

#[test]
fn snake_case_moth_name_maps_to_camel_case_js() {
    let source = r#"
/**
 * @moth.sig get_canvas_context |id String| -> String
 */
export function getCanvasContext(id) {
    return id;
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_eq!(parsed.free_functions[0].moth_name, "get_canvas_context");
    assert_eq!(parsed.free_functions[0].js_name, "getCanvasContext");
}

// ------------------------
//  Private helpers (unexported) are allowed
// ------------------------

#[test]
fn unexported_helpers_are_allowed() {
    let source = r#"
function privateHelper(x) {
    return x * 2;
}

/**
 * @moth.sig double |x Int| -> Int
 */
export function double(x) {
    return privateHelper(x);
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_free_functions(&parsed, &["double"]);
}

// ------------------------
//  Multiple annotations and exports
// ------------------------

#[test]
fn full_module_parse() {
    let source = r#"
import { mothOk, mothErr } from "@moth/runtime";

/**
 * @moth.opaque Canvas
 * @moth.opaque Canvas2d
 */

/**
 * @moth.sig get_canvas |id String| -> Canvas, Error!
 */
export function getCanvas(id) {
    const canvas = document.getElementById(id);
    if (!canvas) {
        return mothErr(404, "Canvas not found");
    }
    return mothOk(canvas);
}

/**
 * @moth.sig fill_rect |this ~Canvas2d, x Float, y Float, width Float, height Float|
 */
export function fillRect(ctx, x, y, width, height) {
    ctx.fillRect(x, y, width, height);
}
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_opaque_types(&parsed, &["Canvas", "Canvas2d"]);
    assert_free_functions(&parsed, &["get_canvas"]);
    assert_receiver_methods(&parsed, &["fill_rect"]);
}

#[test]
fn builtin_web_canvas_package_parses_expanded_surface() {
    let source = include_str!("../../../binding_packages/web/canvas/canvas.js");
    let parsed = parse(source);

    assert_no_diagnostics(&parsed);
    assert_opaque_types(
        &parsed,
        &[
            "CanvasElement",
            "Canvas2d",
            "CanvasGradient",
            "CanvasPattern",
            "CanvasImage",
            "CanvasImageData",
            "CanvasTextMetrics",
        ],
    );
    assert_runtime_imports(
        &parsed,
        &[(
            "@moth/runtime",
            &[
                "MOTH_ERROR_HOST_INVALID_ARGUMENT",
                "MOTH_ERROR_HOST_OPERATION_FAILED",
                "MOTH_ERROR_HOST_RESOURCE_NOT_FOUND",
                "MOTH_ERROR_HOST_RESOURCE_UNAVAILABLE",
                "mothErr",
                "mothOk",
            ],
        )],
    );

    let free_function_names: Vec<&str> = parsed
        .free_functions
        .iter()
        .map(|function| function.moth_name.as_str())
        .collect();
    for expected in [
        "get_canvas",
        "get_image",
        "context_2d",
        "to_data_url_quality",
        "image_data_get_red",
        "text_width",
        "set_canvas_size",
        "set_fill_style",
        "create_linear_gradient",
        "add_color_stop",
        "draw_image_scaled",
        "image_data_set_pixel",
    ] {
        assert!(
            free_function_names.contains(&expected),
            "expected expanded canvas free function {expected}"
        );
    }

    assert!(
        parsed.receiver_methods.is_empty(),
        "built-in @web/canvas must expose only opaque types and free functions"
    );
}

// Literal constants exercise scanner boundaries as well as the shared numeric receiving owner.
#[test]
fn literal_constants_keep_exact_u32_values_names_and_source_spans() {
    for (literal, expected) in [("0", 0_u64), ("4", 4), ("4294967295", 4294967295)] {
        for terminator in [";", "", "\n"] {
            let source = format!(
                "/** @moth.const primitive_triangles U32 */\n\
                 export /* export const FAKE = 9; */ const /* name */ TRIANGLES \
                 /* equals */ = /* literal */ {literal} /* end */ {terminator}"
            );
            let parsed = parse(&source);
            assert_no_diagnostics(&parsed);
            assert!(parsed.free_functions.is_empty());
            assert_eq!(parsed.constants.len(), 1);
            let constant = &parsed.constants[0];
            assert_eq!(constant.moth_name, "primitive_triangles");
            assert_eq!(constant.js_name, "TRIANGLES");
            assert_eq!(constant.value.scalar(), FixedScalar::U32);
            assert_eq!(constant.value.as_u64(), Some(expected));
            assert_eq!(
                &source[constant.literal_span.byte_start..constant.literal_span.byte_end],
                literal
            );
            assert_eq!(
                &source[constant.annotation_span.byte_start..constant.annotation_span.byte_end],
                "/** @moth.const primitive_triangles U32 */"
            );
            assert!(
                source[constant.export_span.byte_start..constant.export_span.byte_end]
                    .starts_with("export")
            );
        }
    }
}

#[test]
fn literal_constants_reject_every_nonliteral_or_nondecimal_initializer() {
    for initializer in [
        "4294967296",
        "18446744073709551616",
        "-1",
        "-0",
        "+4",
        "1.5",
        "1.0",
        ".5",
        "1.",
        "4e0",
        "4E0",
        "4n",
        "4_000",
        "0xff",
        "0Xff",
        "0b100",
        "0o4",
        "04",
        "00",
        "08",
        "09",
        "other",
        "other.value",
        "other()",
        "Math.round(4)",
        "(4)",
        "(4",
        "4 + 1",
        "4+1",
        "4 - 1",
        "4 * 1",
        "4 / 1",
        "4 ** 1",
        "4 | 1",
        "4 << 1",
        "4 ? 1 : 0",
        "4, OTHER = 1",
        "4 /* digits do not join */ 0",
        "4 /* expression */ + /* operand */ 1",
        "true",
        "\"4\"",
        "[]",
        "{}",
        "",
    ] {
        let source = format!("/** @moth.const mode U32 */\nexport const MODE = {initializer};");
        let parsed = parse(&source);
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == JsDiagnosticKind::InvalidConstant),
            "initializer {initializer:?} must produce an InvalidConstant diagnostic: {:?}",
            parsed.diagnostics
        );
        assert!(
            parsed.constants.is_empty(),
            "initializer {initializer:?} cannot publish a value"
        );
    }
}

#[test]
fn literal_constant_annotations_reject_missing_or_unsupported_types_and_nested_names() {
    for annotation in [
        "mode",
        "mode Uint",
        "mode Int",
        "mode F32",
        "mode U64",
        "mode Bool",
        "mode {U32}",
        "mode U32.extra",
        "primitive.triangles U32",
        "mode U32 extra",
        "1mode U32",
        "mode<U32> U32",
        "",
    ] {
        let source = format!("/** @moth.const {annotation} */\nexport const MODE = 4;");
        let parsed = parse(&source);
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == JsDiagnosticKind::InvalidConstant)
        );
        assert!(parsed.constants.is_empty());
    }
}

#[test]
fn unannotated_literal_constant_remains_rejected() {
    let parsed = parse("export const MODE = 4;");
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::UnannotatedExport]);
    assert!(parsed.constants.is_empty());
}

#[test]
fn constant_annotation_cannot_bind_callable_exports() {
    for declaration in [
        "export function mode() { return 4; }",
        "export const mode = () => { return 4; };",
    ] {
        let parsed = parse(&format!("/** @moth.const mode U32 */\n{declaration}"));
        assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::AnnotationExportKindMismatch]);
        assert!(parsed.constants.is_empty());
        assert!(parsed.free_functions.is_empty());
    }
    let missing = parse("/** @moth.const mode U32 */");
    assert_diagnostic_kinds(&missing, &[JsDiagnosticKind::MissingExportAfterConst]);
}

#[test]
fn constant_exports_reject_duplicate_moth_and_js_names_across_symbol_kinds() {
    for source in [
        "/** @moth.const same U32 */ export const FIRST = 1;\n\
         /** @moth.const same U32 */ export const SECOND = 2;",
        "/** @moth.sig same || -> U32 */ export function first() { return 1; }\n\
         /** @moth.const same U32 */ export const SECOND = 2;",
        "/** @moth.const same U32 */ export const FIRST = 1;\n\
         /** @moth.sig same || -> U32 */ export function second() { return 2; }",
        "/** @moth.opaque same */\n\
         /** @moth.const same U32 */ export const FIRST = 1;",
    ] {
        let parsed = parse(source);
        assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::DuplicateMothName]);
    }
    for source in [
        "/** @moth.const first U32 */ export const SAME = 1;\n\
         /** @moth.const second U32 */ export const SAME = 2;",
        "/** @moth.sig first || -> U32 */ export function SAME() { return 1; }\n\
         /** @moth.const second U32 */ export const SAME = 2;",
        "/** @moth.const first U32 */ export const SAME = 1;\n\
         /** @moth.sig second || -> U32 */ export function SAME() { return 2; }",
    ] {
        let parsed = parse(source);
        assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::DuplicateJsExportName]);
    }
}

#[test]
fn literal_constant_scanning_ignores_comment_and_string_lookalikes() {
    let source = r#"
// /** @moth.const fake U32 */ export const FAKE = 4;
/* export const FAKE = 4; @moth.const fake U32 */
const text = "/** @moth.const fake U32 */ export const FAKE = 4;";
const template = `/** @moth.const fake U32 */ export const FAKE = 4;`;
const pattern = /\/\*\* @moth.const fake U32 \*\//;
/** @moth.const actual U32 */
export const ACTUAL /* export function fake() {} */ = 4;
"#;
    let parsed = parse(source);
    assert_no_diagnostics(&parsed);
    assert_eq!(parsed.constants.len(), 1);
    assert_eq!(parsed.constants[0].moth_name, "actual");
}

#[test]
fn constant_statement_boundaries_preserve_following_declarations() {
    for first in [
        "export const FIRST = 4",
        "export const FIRST = 4 // trailing comment",
        "export const FIRST = 4 /* comment\nwith newline */",
        "export const FIRST = 4 /* comment */;",
    ] {
        let source = format!(
            "/** @moth.const first U32 */\n{first}\n\
             /** @moth.const second U32 */\nexport const SECOND = 5;\n\
             /** @moth.sig callable || -> U32 */\nexport function callable() {{ return 6; }}"
        );
        let parsed = parse(&source);
        assert_no_diagnostics(&parsed);
        assert_eq!(parsed.constants.len(), 2);
        assert_eq!(parsed.constants[1].moth_name, "second");
        assert_free_functions(&parsed, &["callable"]);
    }
}

#[test]
fn newline_continuations_do_not_turn_expressions_into_literal_constants() {
    for continuation in [
        "+ 1",
        "- 1",
        "* 1",
        "/ 1",
        "** 1",
        "% 1",
        "| 1",
        "& 1",
        "^ 1",
        "<< 1",
        "< 5",
        "!= 3",
        "=== 4",
        "!== 3",
        "? 1 : 0",
        ".toString()",
        "(other)",
        "[0]",
        "`tagged`",
        ", SECOND = 5",
        "in other",
        "instanceof Other",
    ] {
        for trivia in [
            "\n",
            " /* comment */\n",
            " /*\ncomment */ ",
            " // comment\n",
        ] {
            let source = format!(
                "/** @moth.const first U32 */\nexport const FIRST = 4{trivia}{continuation};\n\
                 /** @moth.const second U32 */\nexport const SECOND = 5;"
            );
            let parsed = parse(&source);
            assert!(
                parsed
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.kind == JsDiagnosticKind::InvalidConstant),
                "continuation {trivia:?}{continuation} must be rejected"
            );
            assert_eq!(parsed.constants.len(), 1);
            assert_eq!(parsed.constants[0].moth_name, "second");
        }
    }
    let explicit_terminator = parse(
        "/** @moth.const first U32 */ export const FIRST = 4;\n\
         (privateCall());",
    );
    assert_no_diagnostics(&explicit_terminator);
    assert_eq!(explicit_terminator.constants.len(), 1);
}

#[test]
fn invalid_or_missing_initializer_does_not_consume_the_next_declaration() {
    for first in [
        "export const FIRST",
        "export const FIRST =",
        "export const FIRST = (4",
        "export const FIRST = other",
        "export const FIRST = 4 + 1",
    ] {
        let source = format!(
            "/** @moth.const first U32 */\n{first}\n\
             /** @moth.const second U32 */\nexport const SECOND = 5;"
        );
        let parsed = parse(&source);
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == JsDiagnosticKind::InvalidConstant)
        );
        assert_eq!(
            parsed.constants.len(),
            1,
            "declaration {first:?} must retain the next export"
        );
        assert_eq!(parsed.constants[0].moth_name, "second");
    }
}

#[test]
fn unterminated_constant_trivia_cannot_publish_a_literal() {
    for declaration in [
        "export const MODE = 4 /*",
        "export const MODE = 4 /* unterminated\ncomment",
        "export const MODE = /*",
        "export const /*",
    ] {
        let source = format!("/** @moth.const mode U32 */\n{declaration}");
        let parsed = parse(&source);
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == JsDiagnosticKind::InvalidConstant)
        );
        assert!(parsed.constants.is_empty());
    }
}

#[test]
fn constant_annotation_does_not_skip_an_intervening_source_item() {
    for intervening in ["const privateMode = 4;", "export let unsupported = 4;"] {
        let parsed = parse(&format!(
            "/** @moth.const first U32 */\n{intervening}\n\
             /** @moth.const actual U32 */\nexport const ACTUAL = 5;"
        ));
        assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::MissingExportAfterConst]);
        assert_eq!(parsed.constants.len(), 1);
        assert_eq!(parsed.constants[0].moth_name, "actual");
    }
}

#[test]
fn duplicate_binding_annotations_cannot_move_to_the_next_export() {
    for annotations in [
        "/**\n * @moth.const first U32\n * @moth.sig callable || -> U32\n */",
        "/** @moth.const first U32 */\n/** @moth.const second U32 */",
    ] {
        let parsed = parse(&format!(
            "{annotations}\nexport const FIRST = 4;\n\
             /** @moth.const actual U32 */\nexport const ACTUAL = 5;"
        ));
        assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::AnnotationExportKindMismatch]);
        assert_eq!(parsed.constants.len(), 2);
        assert_eq!(parsed.constants[1].moth_name, "actual");
    }
}

#[test]
fn constant_boundaries_use_js_line_terminators_including_line_comments() {
    for line_break in ["\n", "\r\n", "\r", "\u{2028}", "\u{2029}"] {
        let parsed = parse(&format!(
            "/** @moth.const first U32 */\nexport const FIRST = 4 // comment{line_break}\
             /** @moth.const second U32 */ export const SECOND = 5;"
        ));
        assert_no_diagnostics(&parsed);
        assert_eq!(parsed.constants.len(), 2);
    }
}

#[test]
fn constant_range_diagnostic_identifies_the_authored_literal_span() {
    let source = "/** @moth.const overflow U32 */ export const OVERFLOW = 4294967296;";
    let parsed = parse(source);
    assert_diagnostic_kinds(&parsed, &[JsDiagnosticKind::InvalidConstant]);
    let span = &parsed.diagnostics[0].span;
    assert_eq!(&source[span.byte_start..span.byte_end], "4294967296");
}

#[test]
fn newline_prefix_statements_do_not_continue_constant_initializers() {
    for statement in ["++counter", "--counter", "!counter"] {
        for trivia in ["\n", " // comment\n", " /* comment\n */ "] {
            let source = format!(
                "let counter = 0;\n\
                 /** @moth.const mode U32 */\nexport const MODE = 4{trivia}{statement};\n\
                 /** @moth.const actual U32 */\nexport const ACTUAL = 5;"
            );
            let parsed = parse(&source);
            assert_no_diagnostics(&parsed);
            assert_eq!(parsed.constants.len(), 2);
            assert_eq!(parsed.constants[0].value.as_u64(), Some(4));
            assert_eq!(parsed.constants[1].moth_name, "actual");
        }
    }
}

#[test]
fn constant_initializer_scanning_preserves_module_loading_diagnostics() {
    for (initializer, expected) in [
        (
            "require(\"third-party\")",
            JsDiagnosticKind::CommonJsRequire,
        ),
        (
            "load(import(\"third-party\"))",
            JsDiagnosticKind::DynamicImport,
        ),
    ] {
        let source = format!("export const dependency = {initializer};");
        let scanned = scan_exports(&source, &RuntimeModuleRegistry::v1());
        assert!(
            scanned
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == expected),
            "initializer {initializer:?} must retain {expected:?}: {:?}",
            scanned.diagnostics
        );
        let parsed = parse(&format!(
            "/** @moth.const dependency U32 */\nexport const dependency = {initializer};\n\
             /** @moth.const actual U32 */\nexport const ACTUAL = 5;"
        ));
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == expected)
        );
        assert_eq!(parsed.constants.len(), 1);
        assert_eq!(parsed.constants[0].moth_name, "actual");
    }
}
