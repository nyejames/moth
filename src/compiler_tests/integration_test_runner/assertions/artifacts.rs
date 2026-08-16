//! Output-artifact assertions and shared project-output lookup.
//!
//! WHAT: validates artifact presence, absence, kind and text or binary content, including the
//!       universal HTML output baseline.
//! WHY: all output lookup belongs here so golden, rendered-output and Wasm checks inspect the
//!      same normalized set of emitted artifacts.

use super::super::{ArtifactAssertion, ArtifactKind};
use crate::build_system::build::{BuildResult, FileKind, OutputFile};
use crate::compiler_frontend::utilities::basic::portable_path_text;

pub(super) fn validate_artifacts_must_not_exist(
    build_result: &BuildResult,
    forbidden_paths: &[String],
) -> Option<String> {
    if forbidden_paths.is_empty() {
        return None;
    }

    let built_paths = collect_built_artifact_paths(build_result);

    for forbidden in forbidden_paths {
        if built_paths.contains(forbidden) {
            return Some(format!(
                "Expected artifact '{}' to not exist, but it was produced. Built paths: {built_paths:?}.",
                forbidden
            ));
        }
    }

    None
}

pub(super) fn validate_artifact_assertions(
    build_result: &BuildResult,
    assertions: &[ArtifactAssertion],
) -> Option<String> {
    for assertion in assertions {
        let Some(output) = find_output_file(build_result, &assertion.path) else {
            return Some(format!(
                "Artifact assertion expected output '{}', but produced paths were {:?}.",
                assertion.path,
                collect_built_artifact_paths(build_result)
            ));
        };

        if let Some(reason) = validate_single_artifact_assertion(output, assertion) {
            return Some(reason);
        }
    }

    None
}

fn validate_single_artifact_assertion(
    output: &OutputFile,
    assertion: &ArtifactAssertion,
) -> Option<String> {
    match assertion.kind {
        ArtifactKind::Html | ArtifactKind::Js => {
            let Some(text) = output_text_content(output, assertion.kind) else {
                return Some(format!(
                    "Artifact '{}' expected kind '{}', but produced a different file kind.",
                    assertion.path,
                    artifact_kind_name(assertion.kind)
                ));
            };

            for required in &assertion.must_contain {
                if !text.contains(required) {
                    return Some(format!(
                        "Artifact '{}' did not contain required fragment '{}'.",
                        assertion.path, required
                    ));
                }
            }

            for forbidden in &assertion.must_not_contain {
                if text.contains(forbidden) {
                    return Some(format!(
                        "Artifact '{}' contained forbidden fragment '{}'.",
                        assertion.path, forbidden
                    ));
                }
            }

            if !assertion.must_contain_in_order.is_empty()
                && !super::contains_ordered_substrings(text, &assertion.must_contain_in_order)
            {
                return Some(format!(
                    "Artifact '{}' did not contain required ordered fragments {:?}.",
                    assertion.path, assertion.must_contain_in_order
                ));
            }

            for required_once in &assertion.must_contain_exactly_once {
                let count = count_occurrences(text, required_once);
                if count != 1 {
                    return Some(format!(
                        "Artifact '{}' expected fragment '{}' exactly once, but found {} time(s).",
                        assertion.path, required_once, count
                    ));
                }
            }

            if !assertion.normalized_contains.is_empty()
                || !assertion.normalized_not_contains.is_empty()
            {
                let normalized_text = super::goldens::normalize_text_for_comparison(text);
                for required in &assertion.normalized_contains {
                    let normalized_required =
                        super::goldens::normalize_text_for_comparison(required);
                    if !normalized_text.contains(normalized_required.as_str()) {
                        return Some(format!(
                            "Artifact '{}' did not contain required normalized fragment '{}'.",
                            assertion.path, required
                        ));
                    }
                }
                for forbidden in &assertion.normalized_not_contains {
                    let normalized_forbidden =
                        super::goldens::normalize_text_for_comparison(forbidden);
                    if normalized_text.contains(normalized_forbidden.as_str()) {
                        return Some(format!(
                            "Artifact '{}' contained forbidden normalized fragment '{}'.",
                            assertion.path, forbidden
                        ));
                    }
                }
            }
        }
        ArtifactKind::Wasm => {
            let Some(bytes) = output_wasm_bytes(output) else {
                return Some(format!(
                    "Artifact '{}' expected kind 'wasm', but produced a different file kind.",
                    assertion.path
                ));
            };

            if assertion.validate_wasm
                && let Err(error) = super::wasm::validate_wasm_bytes(bytes)
            {
                return Some(format!(
                    "Artifact '{}' failed wasm validation: {error}",
                    assertion.path
                ));
            }

            if !assertion.must_export.is_empty() {
                let exports = match super::wasm::collect_wasm_exports(bytes) {
                    Ok(exports) => exports,
                    Err(error) => {
                        return Some(format!(
                            "Artifact '{}' failed while reading wasm exports: {error}",
                            assertion.path
                        ));
                    }
                };

                for required_export in &assertion.must_export {
                    if !exports.contains(required_export) {
                        return Some(format!(
                            "Artifact '{}' missing required wasm export '{}'. Available exports: {:?}.",
                            assertion.path, required_export, exports
                        ));
                    }
                }
            }

            if !assertion.must_import.is_empty() {
                let imports = match super::wasm::collect_wasm_imports(bytes) {
                    Ok(imports) => imports,
                    Err(error) => {
                        return Some(format!(
                            "Artifact '{}' failed while reading wasm imports: {error}",
                            assertion.path
                        ));
                    }
                };

                for required_import in &assertion.must_import {
                    if !imports.contains(required_import) {
                        return Some(format!(
                            "Artifact '{}' missing required wasm import '{}'. Available imports: {:?}.",
                            assertion.path, required_import, imports
                        ));
                    }
                }
            }
        }
        ArtifactKind::Binary => {
            if output_binary_bytes(output).is_none() {
                return Some(format!(
                    "Artifact '{}' expected kind 'binary', but produced a different file kind.",
                    assertion.path
                ));
            }
        }
    }

    None
}

fn artifact_kind_name(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Html => "html",
        ArtifactKind::Js => "js",
        ArtifactKind::Wasm => "wasm",
        ArtifactKind::Binary => "binary",
    }
}

/// Verifies the baseline HTML backend interop/output contract.
///
/// WHAT: requires a built `index.html` HTML artifact for every HTML backend success case.
/// WHY: replacing legacy path assertions still needs a deterministic minimum output guarantee.
pub(super) fn validate_html_baseline_contract(build_result: &BuildResult) -> Option<String> {
    let Some(index_html) = find_output_file(build_result, "index.html") else {
        return Some(
            "html baseline contract expected 'index.html', but it was not produced.".to_string(),
        );
    };

    let Some(html) = output_text_content(index_html, ArtifactKind::Html) else {
        return Some(
            "html baseline contract expected 'index.html' as an HTML artifact.".to_string(),
        );
    };

    validate_html_document_structure(html, "html")
}

pub(super) fn validate_html_document_structure(html: &str, baseline_name: &str) -> Option<String> {
    for required_fragment in [
        "<!DOCTYPE html>",
        "<html",
        "<head>",
        "<body",
        "</body>",
        "</html>",
    ] {
        if !html.contains(required_fragment) {
            return Some(format!(
                "{baseline_name} baseline contract expected 'index.html' to contain '{required_fragment}'."
            ));
        }
    }

    None
}

pub(super) fn collect_built_artifact_paths(build_result: &BuildResult) -> Vec<String> {
    let mut actual_paths = build_result
        .project
        .output_files
        .iter()
        .filter(|output| !matches!(output.file_kind(), FileKind::NotBuilt))
        .map(|output| portable_path_text(output.relative_output_path()))
        .collect::<Vec<_>>();
    actual_paths.sort();
    actual_paths
}

pub(super) fn find_output_file<'a>(
    build_result: &'a BuildResult,
    relative_path: &str,
) -> Option<&'a OutputFile> {
    let normalized_target = portable_path_text(relative_path);

    build_result.project.output_files.iter().find(|output| {
        !matches!(output.file_kind(), FileKind::NotBuilt)
            && portable_path_text(output.relative_output_path()) == normalized_target
    })
}

pub(super) fn output_text_content(
    output: &OutputFile,
    expected_kind: ArtifactKind,
) -> Option<&str> {
    if matches!(expected_kind, ArtifactKind::Html)
        && let FileKind::Html(content) = output.file_kind()
    {
        return Some(content.as_str());
    }

    if matches!(expected_kind, ArtifactKind::Js)
        && let FileKind::Js(content) = output.file_kind()
    {
        return Some(content.as_str());
    }

    None
}

pub(super) fn output_wasm_bytes(output: &OutputFile) -> Option<&[u8]> {
    match output.file_kind() {
        FileKind::Wasm(bytes) => Some(bytes.as_slice()),
        _ => None,
    }
}

fn output_binary_bytes(output: &OutputFile) -> Option<&[u8]> {
    match output.file_kind() {
        FileKind::Bytes(bytes) => Some(bytes.as_slice()),
        _ => None,
    }
}

fn count_occurrences(text: &str, needle: &str) -> usize {
    let mut count = 0;
    let mut offset = 0;

    while let Some(position) = text[offset..].find(needle) {
        count += 1;
        offset += position + needle.len();
    }

    count
}
