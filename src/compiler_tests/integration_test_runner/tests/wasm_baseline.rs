//! Self-tests for the HTML-Wasm baseline runtime-export contract.
//!
//! WHAT: proves the baseline requires every export the emitted bootstrap actually calls, with the
//!       right export kind, and rejects an ambiguous export set.
//! WHY: the previous baseline compared a hardcoded name list and never required `moth_start`,
//!      the one export the generated JavaScript calls unconditionally. A module missing it, or
//!      exporting it as something other than a function, passed the suite and failed in a browser.

use super::super::assertions::validate_success_result;
use super::super::types::{ArtifactAssertion, ArtifactKind, GoldenExpectation};
use super::super::{BackendId, FailureKind, SuccessExpectation, WarningExpectation};
use super::synthetic_build_results::{
    VALID_HTML_WASM, VALID_PAGE_JS, acceptance_only_expectation, build_result_with_output_files,
    success_test_case,
};
use crate::build_system::build::{BuildResult, FileKind};
use std::path::PathBuf;
use wasm_encoder::{
    CodeSection, ExportKind, ExportSection, Function, FunctionSection, Instruction, MemorySection,
    MemoryType, Module, TypeSection, ValType,
};

/// The exports the real bootstrap always uses, all as functions except `memory`.
const BASELINE_FUNCTION_EXPORTS: [&str; 4] =
    ["moth_start", "moth_str_ptr", "moth_str_len", "moth_release"];

/// Builds a valid module exporting one memory plus one `() -> i32` function per name.
///
/// Every export name is backed by a real entity, so a rejection can only come from the contract
/// under test rather than from a malformed module.
fn wasm_module_with_exports(
    memory_export: Option<&str>,
    function_exports: &[&str],
    extra_exports: &[(&str, ExportKind, u32)],
) -> Vec<u8> {
    let mut module = Module::new();

    let mut types = TypeSection::new();
    types.ty().function(Vec::new(), vec![ValType::I32]);
    module.section(&types);

    let mut functions = FunctionSection::new();
    for _ in function_exports {
        functions.function(0);
    }
    module.section(&functions);

    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: 1,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    module.section(&memories);

    let mut exports = ExportSection::new();
    if let Some(name) = memory_export {
        exports.export(name, ExportKind::Memory, 0);
    }
    for (index, name) in function_exports.iter().enumerate() {
        exports.export(name, ExportKind::Func, index as u32);
    }
    for (name, kind, index) in extra_exports {
        exports.export(name, *kind, *index);
    }
    module.section(&exports);

    let mut code = CodeSection::new();
    for _ in function_exports {
        let mut body = Function::new(Vec::new());
        body.instruction(&Instruction::I32Const(0));
        body.instruction(&Instruction::End);
        code.function(&body);
    }
    module.section(&code);

    module.finish()
}

fn html_wasm_build_result(page_js: &str, wasm_bytes: Vec<u8>) -> BuildResult {
    build_result_with_output_files(vec![
        (
            PathBuf::from("index.html"),
            FileKind::Html(VALID_HTML_WASM.to_owned()),
        ),
        (PathBuf::from("page.js"), FileKind::Js(page_js.to_owned())),
        (PathBuf::from("page.wasm"), FileKind::Wasm(wasm_bytes)),
    ])
}

/// Runs the HTML-Wasm baseline and returns its rejection reason, if any.
#[track_caller]
fn html_wasm_baseline_reason(build_result: BuildResult) -> Option<String> {
    let expectation = acceptance_only_expectation();
    let case = success_test_case(BackendId::HtmlWasm, expectation.clone());
    let result = validate_success_result(&case, build_result, &expectation);

    if result.passed {
        return None;
    }

    Some(
        result
            .failure_reason
            .expect("a failed baseline must report a reason"),
    )
}

#[test]
fn html_wasm_baseline_accepts_a_module_exporting_every_runtime_called_entity() {
    let build_result = html_wasm_build_result(
        VALID_PAGE_JS,
        wasm_module_with_exports(Some("memory"), &BASELINE_FUNCTION_EXPORTS, &[]),
    );

    assert_eq!(html_wasm_baseline_reason(build_result), None);
}

#[test]
fn html_wasm_baseline_requires_the_start_export_the_bootstrap_calls() {
    // `instance.exports.moth_start()` runs on every page load. A module without it is broken at
    // runtime, and the old name-list baseline never checked for it.
    let without_start: Vec<&str> = BASELINE_FUNCTION_EXPORTS
        .iter()
        .copied()
        .filter(|name| *name != "moth_start")
        .collect();
    let build_result = html_wasm_build_result(
        VALID_PAGE_JS,
        wasm_module_with_exports(Some("memory"), &without_start, &[]),
    );

    let reason = html_wasm_baseline_reason(build_result)
        .expect("a module without moth_start cannot satisfy the baseline");
    assert!(
        reason.contains("missing required export 'moth_start'"),
        "{reason}"
    );
}

#[test]
fn html_wasm_baseline_requires_the_expected_export_kind() {
    // Exporting `memory` as a function would make `instance.exports.memory.buffer` undefined.
    let build_result = html_wasm_build_result(
        VALID_PAGE_JS,
        wasm_module_with_exports(
            None,
            &BASELINE_FUNCTION_EXPORTS,
            &[("memory", ExportKind::Func, 0)],
        ),
    );

    let reason = html_wasm_baseline_reason(build_result)
        .expect("a memory export that is not a memory cannot satisfy the baseline");
    assert!(
        reason.contains("expected export 'memory' to be a Memory"),
        "{reason}"
    );
}

#[test]
fn html_wasm_baseline_derives_extra_required_exports_from_the_emitted_bootstrap() {
    // A page with runtime slots also calls the vec helpers. The requirement follows the emitted
    // bootstrap instead of a fixed list, so a page that calls more must export more.
    let page_js = format!("{VALID_PAGE_JS}instance.exports.moth_vec_len(v);\n");
    let build_result = html_wasm_build_result(
        &page_js,
        wasm_module_with_exports(Some("memory"), &BASELINE_FUNCTION_EXPORTS, &[]),
    );

    let reason = html_wasm_baseline_reason(build_result)
        .expect("a bootstrap calling moth_vec_len requires that export");
    assert!(
        reason.contains("missing required export 'moth_vec_len'"),
        "{reason}"
    );
}

#[test]
fn html_wasm_baseline_rejects_an_unrecognised_runtime_export_use() {
    // An unknown access shape must not silently drop out of the derived contract.
    let page_js = format!("{VALID_PAGE_JS}const handle = instance.exports.moth_table.length;\n");
    let build_result = html_wasm_build_result(
        &page_js,
        wasm_module_with_exports(Some("memory"), &BASELINE_FUNCTION_EXPORTS, &[]),
    );

    let reason = html_wasm_baseline_reason(build_result)
        .expect("an unsupported runtime export use must be reported");
    assert!(
        reason.contains("unsupported runtime export use"),
        "{reason}"
    );
}

#[test]
fn html_wasm_baseline_requires_the_page_script_include_exactly_once() {
    let duplicated_include = VALID_HTML_WASM.replace(
        "<script src=\"./page.js\"></script>\n",
        "<script src=\"./page.js\"></script>\n<script src=\"./page.js\"></script>\n",
    );
    let build_result = build_result_with_output_files(vec![
        (
            PathBuf::from("index.html"),
            FileKind::Html(duplicated_include),
        ),
        (
            PathBuf::from("page.js"),
            FileKind::Js(VALID_PAGE_JS.to_owned()),
        ),
        (
            PathBuf::from("page.wasm"),
            FileKind::Wasm(wasm_module_with_exports(
                Some("memory"),
                &BASELINE_FUNCTION_EXPORTS,
                &[],
            )),
        ),
    ]);

    let reason = html_wasm_baseline_reason(build_result)
        .expect("a page included twice would run the bootstrap twice");
    assert!(reason.contains("exactly once"), "{reason}");
}

#[test]
fn html_wasm_baseline_rejects_invalid_wasm_bytes() {
    let build_result = html_wasm_build_result(VALID_PAGE_JS, vec![0, 1, 2]);

    let reason = html_wasm_baseline_reason(build_result)
        .expect("bytes that are not a wasm module cannot satisfy the baseline");
    assert!(reason.contains("valid wasm bytes"), "{reason}");
}

#[test]
fn html_wasm_baseline_rejects_missing_output() {
    let build_result = build_result_with_output_files(Vec::new());

    let reason = html_wasm_baseline_reason(build_result)
        .expect("a build with no artifacts cannot satisfy the baseline");
    assert!(reason.contains("html_wasm baseline contract"), "{reason}");
}

#[test]
fn artifact_export_assertions_reject_a_module_exporting_one_name_twice() {
    // Export names are unique in a well-formed module. A duplicate means a `must_export`
    // assertion would inspect whichever entry it reached first.
    let wasm_bytes = wasm_module_with_exports(
        Some("memory"),
        &BASELINE_FUNCTION_EXPORTS,
        &[("moth_start", ExportKind::Func, 1)],
    );
    let expectation = SuccessExpectation {
        warnings: WarningExpectation::Forbid,
        success_contract: None,
        artifact_assertions: vec![ArtifactAssertion {
            path: "page.wasm".to_owned(),
            kind: ArtifactKind::Wasm,
            must_contain: Vec::new(),
            must_not_contain: Vec::new(),
            must_contain_in_order: Vec::new(),
            must_contain_exactly_once: Vec::new(),
            normalized_contains: Vec::new(),
            normalized_not_contains: Vec::new(),
            validate_wasm: false,
            must_export: vec!["moth_start".to_owned()],
            must_import: Vec::new(),
        }],
        golden: GoldenExpectation::default(),
        rendered_output: Default::default(),
        artifacts_must_not_exist: Vec::new(),
    };
    // The HTML backend has no wasm baseline, so this case exercises the artifact assertion alone.
    let case = success_test_case(BackendId::Html, expectation.clone());
    let build_result = build_result_with_output_files(vec![
        (
            PathBuf::from("index.html"),
            FileKind::Html(super::synthetic_build_results::VALID_HTML.to_owned()),
        ),
        (PathBuf::from("page.wasm"), FileKind::Wasm(wasm_bytes)),
    ]);

    let result = validate_success_result(&case, build_result, &expectation);

    assert!(!result.passed, "a duplicated export name is ambiguous");
    let reason = result
        .failure_reason
        .expect("a failed artifact assertion must report a reason");
    assert!(
        reason.contains("exports 'moth_start' more than once"),
        "{reason}"
    );
    assert_eq!(result.failure_kind, Some(FailureKind::ExpectationViolation));
}
