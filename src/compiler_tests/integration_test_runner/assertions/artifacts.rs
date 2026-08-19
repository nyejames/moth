//! Output-artifact assertions and shared project-output lookup.
//!
//! WHAT: validates artifact presence, absence, kind and text or binary content, including the
//!       universal HTML output baseline.
//! WHY: all output lookup belongs here so golden, rendered-output and Wasm checks inspect the
//!      same normalized set of emitted artifacts.

use super::super::{ArtifactAssertion, ArtifactKind};
use crate::build_system::build::{BuildResult, FileKind, OutputFile};
use crate::build_system::output::output_path_identity;
use crate::compiler_frontend::compiler_messages::InvalidOutputFolderReason;
use crate::compiler_frontend::utilities::basic::portable_path_text;
use std::collections::{BTreeMap, HashMap};
use std::fmt;

/// One normalized, unique view of the artifacts a build actually produced.
///
/// WHAT: maps every built artifact's portable relative path to its `OutputFile`, rejecting
///       invalid output destinations, duplicate paths and portability aliases.
/// WHY: first-match lookup silently inspects one of several artifacts claiming the same path,
///      so every later assertion would read the winner and ignore the rest. Building the index
///      once, before any success assertion runs, makes path identity a proved precondition
///      rather than an assumption each assertion family repeats.
pub(super) struct BuiltArtifactIndex<'a> {
    by_path: BTreeMap<String, &'a OutputFile>,
}

/// Why a build result could not produce a usable artifact index.
///
/// These are harness-level facts about the produced output set, not expectation violations:
/// no authored expectation can be evaluated honestly against an ambiguous artifact set.
#[derive(Debug)]
pub(crate) enum ArtifactIndexError {
    /// Two built artifacts share one canonical output-path identity and one spelling.
    DuplicatePath { path: String },
    /// Two built artifacts spell one canonical output-path identity differently, so they
    /// collide on hosts that fold case.
    PortabilityAlias { first: String, second: String },
    /// A relative output path is not valid UTF-8, so it cannot be compared with an
    /// authored expectation without lossy replacement.
    NonUtf8Path { path: String },
    /// A relative output path is not a destination the output writer would accept.
    InvalidOutputPath {
        path: String,
        reason: InvalidOutputFolderReason,
    },
}

impl fmt::Display for ArtifactIndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicatePath { path } => write!(
                formatter,
                "the build produced more than one artifact at '{path}', so artifact assertions \
                 cannot identify which one they inspect"
            ),
            Self::PortabilityAlias { first, second } => write!(
                formatter,
                "the build produced artifacts '{first}' and '{second}', which share one output \
                 path identity and collide on case-insensitive filesystems"
            ),
            Self::NonUtf8Path { path } => write!(
                formatter,
                "the build produced an artifact whose relative path {path} is not valid UTF-8, \
                 so it cannot be matched against an authored expectation"
            ),
            Self::InvalidOutputPath { path, reason } => write!(
                formatter,
                "the build produced an artifact at '{path}', which the output writer would \
                 reject as an invalid portable destination ({reason:?})"
            ),
        }
    }
}

impl<'a> BuiltArtifactIndex<'a> {
    /// Build the index, or explain why the produced artifact set is ambiguous.
    ///
    /// Validity and collision identity come from `output_path_identity`, the same canonical
    /// output-path policy the writer enforces, so the harness cannot accept a destination the
    /// writer would reject or fold case differently than production does. `portable_path_text`
    /// is used only for lookup keys, sorted display and failure reporting.
    pub(super) fn build(build_result: &'a BuildResult) -> Result<Self, ArtifactIndexError> {
        let mut by_path: BTreeMap<String, &'a OutputFile> = BTreeMap::new();
        // `Page.js` and `page.js` are distinct spellings but one canonical identity, so
        // collisions are decided on the identity and reported with the spellings.
        let mut spelling_by_identity = HashMap::new();

        for output in &build_result.project.output_files {
            if matches!(output.file_kind(), FileKind::NotBuilt) {
                continue;
            }

            let relative_path = output.relative_output_path();
            let identity = match output_path_identity(relative_path) {
                Ok(identity) => identity,
                Err(InvalidOutputFolderReason::NonUtf8) => {
                    return Err(ArtifactIndexError::NonUtf8Path {
                        path: format!("{relative_path:?}"),
                    });
                }
                Err(reason) => {
                    return Err(ArtifactIndexError::InvalidOutputPath {
                        path: portable_path_text(relative_path),
                        reason,
                    });
                }
            };

            let spelling = portable_path_text(relative_path);
            if let Some(existing) = spelling_by_identity.insert(identity, spelling.clone()) {
                return Err(if existing == spelling {
                    ArtifactIndexError::DuplicatePath { path: spelling }
                } else {
                    ArtifactIndexError::PortabilityAlias {
                        first: existing,
                        second: spelling,
                    }
                });
            }

            // Two spellings sharing one identity were rejected above, so this insert never
            // displaces an entry.
            by_path.insert(spelling, output);
        }

        Ok(Self { by_path })
    }

    /// Look up exactly one built artifact by its authored relative path.
    pub(super) fn get(&self, relative_path: &str) -> Option<&'a OutputFile> {
        self.by_path
            .get(&portable_path_text(relative_path))
            .copied()
    }

    /// Every built artifact path, in portable sorted order.
    pub(super) fn paths(&self) -> Vec<&str> {
        self.by_path.keys().map(String::as_str).collect()
    }

    /// Whether an artifact was produced at the authored relative path.
    pub(super) fn contains(&self, relative_path: &str) -> bool {
        self.by_path
            .contains_key(&portable_path_text(relative_path))
    }
}

pub(super) fn validate_artifacts_must_not_exist(
    index: &BuiltArtifactIndex<'_>,
    forbidden_paths: &[String],
) -> Option<String> {
    for forbidden in forbidden_paths {
        if index.contains(forbidden) {
            return Some(format!(
                "Expected artifact '{}' to not exist, but it was produced. Built paths: {:?}.",
                forbidden,
                index.paths()
            ));
        }
    }

    None
}

pub(super) fn validate_artifact_assertions(
    index: &BuiltArtifactIndex<'_>,
    assertions: &[ArtifactAssertion],
) -> Option<String> {
    for assertion in assertions {
        let Some(output) = index.get(&assertion.path) else {
            return Some(format!(
                "Artifact assertion expected output '{}', but produced paths were {:?}.",
                assertion.path,
                index.paths()
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
pub(super) fn validate_html_baseline_contract(index: &BuiltArtifactIndex<'_>) -> Option<String> {
    let Some(index_html) = index.get("index.html") else {
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
