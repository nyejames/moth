use crate::projects::html_project::external_js::parser::{
    parse_js_module, parsed_js_module::JsDiagnosticKind,
};
use crate::projects::html_project::external_js::runtime_module_registry::RuntimeModuleRegistry;

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

fn assert_diagnostic_message_contains(
    parsed: &crate::projects::html_project::external_js::parser::parsed_js_module::ParsedJsModule,
    expected: &str,
) {
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(expected)),
        "expected diagnostic message containing {expected:?}, got {:?}",
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
fn const_export_must_be_arrow_function() {
    let source = r#"
/**
 * @moth.sig answer || -> Int
 */
export const answer = 42;
"#;
    let parsed = parse(source);
    assert_diagnostic_kinds(
        &parsed,
        &[
            JsDiagnosticKind::UnsupportedParameterPattern,
            JsDiagnosticKind::MissingExportAfterSig,
        ],
    );
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
    assert!(
        parsed.diagnostics.is_empty(),
        "got: {:?}",
        parsed.diagnostics
    );
    assert_eq!(parsed.free_functions.len(), 1);
}

#[test]
fn explicit_empty_registry_rejects_all_imports() {
    let source = r#"
import { mothOk } from "@moth/runtime";
"#;
    let registry = RuntimeModuleRegistry::empty();
    let parsed = parse_js_module(source, &registry);
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| d.kind == JsDiagnosticKind::ArbitraryImport),
        "expected ArbitraryImport for empty registry"
    );
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
    assert_diagnostic_message_contains(&parsed, "External package functions cannot be generic");
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
    assert_diagnostic_message_contains(&parsed, "External package types cannot be generic");
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
    assert_runtime_imports(&parsed, &[("@moth/runtime", &["mothErr", "mothOk"])]);

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
