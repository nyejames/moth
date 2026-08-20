//! Wasm artifact validation and HTML-Wasm backend baseline checks.
//!
//! WHAT: parses Wasm bytes for validity, imports and exports, and validates the universal
//!       HTML-Wasm output contract including the exports the emitted bootstrap calls.
//! WHY: Wasm structure belongs with Wasm assertions while artifact lookup and HTML kind checks
//!      remain shared in the artifact owner.

use super::super::ArtifactKind;
use super::artifacts::BuiltArtifactIndex;
use std::collections::BTreeMap;
use wasmparser::{ExternalKind, Imports, Parser, Payload};

/// The `page.js` prefix through which every runtime call reaches a Wasm export.
const RUNTIME_EXPORT_PREFIX: &str = "instance.exports.";

/// Exports the HTML-Wasm runtime ABI always requires, independent of page content.
///
/// The scan below derives the rest from the emitted bootstrap, but a bootstrap that stopped
/// calling one of these would then silently stop requiring it. These stay explicit so the ABI
/// floor cannot erode.
const REQUIRED_RUNTIME_EXPORTS: [(&str, ExternalKind); 5] = [
    ("memory", ExternalKind::Memory),
    ("moth_start", ExternalKind::Func),
    ("moth_str_ptr", ExternalKind::Func),
    ("moth_str_len", ExternalKind::Func),
    ("moth_release", ExternalKind::Func),
];

pub(super) fn validate_html_wasm_baseline_contract(
    index: &BuiltArtifactIndex<'_>,
) -> Option<String> {
    let html = match super::artifacts::validate_html_baseline_document(index, "html_wasm") {
        Ok(html) => html,
        Err(reason) => return Some(reason),
    };

    if let Some(reason) = validate_page_script_include(html) {
        return Some(reason);
    }

    let Some(page_js) = index.get("page.js") else {
        return Some(
            "html_wasm baseline contract expected 'page.js', but it was not produced.".to_string(),
        );
    };

    let Some(js) = super::artifacts::output_text_content(page_js, ArtifactKind::Js) else {
        return Some(
            "html_wasm baseline contract expected 'page.js' as a JS artifact.".to_string(),
        );
    };

    for required_fragment in ["__moth_instantiate_wasm", "\"./page.wasm\""] {
        if !js.contains(required_fragment) {
            return Some(format!(
                "html_wasm baseline contract expected 'page.js' to contain '{required_fragment}'."
            ));
        }
    }

    let required_exports = match required_runtime_exports(js) {
        Ok(exports) => exports,
        Err(reason) => return Some(reason),
    };

    let Some(page_wasm) = index.get("page.wasm") else {
        return Some(
            "html_wasm baseline contract expected 'page.wasm', but it was not produced."
                .to_string(),
        );
    };

    let Some(wasm_bytes) = super::artifacts::output_wasm_bytes(page_wasm) else {
        return Some(
            "html_wasm baseline contract expected 'page.wasm' as a wasm artifact.".to_string(),
        );
    };

    if let Err(error) = validate_wasm_bytes(wasm_bytes) {
        return Some(format!(
            "html_wasm baseline contract expected valid wasm bytes: {error}"
        ));
    }

    let exports = match collect_wasm_exports(wasm_bytes) {
        Ok(exports) => exports,
        Err(error) => {
            return Some(format!(
                "html_wasm baseline contract failed while reading wasm exports: {error}"
            ));
        }
    };

    for (name, expected_kind) in required_exports {
        match exports.get(&name) {
            None => {
                return Some(format!(
                    "html_wasm baseline contract missing required export '{name}'. Available exports: {:?}.",
                    export_summary(&exports)
                ));
            }
            Some(actual_kind) if *actual_kind != expected_kind => {
                return Some(format!(
                    "html_wasm baseline contract expected export '{name}' to be a \
                     {expected_kind:?}, but the module exports it as a {actual_kind:?}."
                ));
            }
            Some(_) => {}
        }
    }

    None
}

/// Requires the page script include exactly once, inside the body.
fn validate_page_script_include(html: &str) -> Option<String> {
    const PAGE_SCRIPT_TAG: &str = "<script src=\"./page.js\"></script>";

    let occurrences = html.match_indices(PAGE_SCRIPT_TAG).count();
    if occurrences != 1 {
        return Some(format!(
            "html_wasm baseline contract expected 'index.html' to include '{PAGE_SCRIPT_TAG}' \
             exactly once, but found it {occurrences} time(s)."
        ));
    }

    // The shell contract already proved `<body style="` and `</body>` each appear once in order,
    // so these positions are the body's real boundaries.
    let (Some(script_position), Some(body_open), Some(body_close)) = (
        html.find(PAGE_SCRIPT_TAG),
        html.find("<body style=\""),
        html.find("</body>"),
    ) else {
        return Some(
            "html_wasm baseline contract expected 'index.html' to carry the document shell body."
                .to_string(),
        );
    };

    if script_position < body_open || script_position > body_close {
        return Some(
            "html_wasm baseline contract expected './page.js' to be included inside the body."
                .to_string(),
        );
    }

    None
}

/// Derives every Wasm export the emitted bootstrap calls at runtime, with its required kind.
///
/// WHAT: scans `page.js` for `instance.exports.<name>` and classifies each use as a function call
///       or a memory access, then folds in the fixed runtime ABI floor.
/// WHY: asserting a hardcoded export list proves nothing about the bootstrap that actually runs —
///      it can call an export the module never defines and fail only at runtime, in a browser.
///      An unrecognised use shape is an error rather than a skipped requirement, so a new runtime
///      access pattern cannot quietly drop out of the contract.
fn required_runtime_exports(page_js: &str) -> Result<BTreeMap<String, ExternalKind>, String> {
    let mut required: BTreeMap<String, ExternalKind> = REQUIRED_RUNTIME_EXPORTS
        .iter()
        .map(|(name, kind)| ((*name).to_owned(), *kind))
        .collect();

    for (position, _) in page_js.match_indices(RUNTIME_EXPORT_PREFIX) {
        let after_prefix = position + RUNTIME_EXPORT_PREFIX.len();
        let rest = &page_js[after_prefix..];
        let name_length = rest
            .find(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .unwrap_or(rest.len());
        let name = &rest[..name_length];

        if name.is_empty() {
            return Err(format!(
                "html_wasm baseline contract found '{RUNTIME_EXPORT_PREFIX}' in 'page.js' with no \
                 export name, so the runtime export contract cannot be derived."
            ));
        }

        let kind = match rest[name_length..].chars().next() {
            Some('(') => ExternalKind::Func,
            Some('.') if name == "memory" => ExternalKind::Memory,
            other => {
                return Err(format!(
                    "html_wasm baseline contract found an unsupported runtime export use \
                     '{RUNTIME_EXPORT_PREFIX}{name}{}' in 'page.js'. The harness classifies a \
                     call as a function export and a 'memory' member access as a memory export.",
                    other.map_or_else(String::new, String::from)
                ));
            }
        };

        if let Some(existing) = required.insert(name.to_owned(), kind)
            && existing != kind
        {
            return Err(format!(
                "html_wasm baseline contract found 'page.js' using export '{name}' as both a \
                 {existing:?} and a {kind:?}."
            ));
        }
    }

    Ok(required)
}

pub(super) fn export_summary(exports: &BTreeMap<String, ExternalKind>) -> Vec<String> {
    exports
        .iter()
        .map(|(name, kind)| format!("{name}:{kind:?}"))
        .collect()
}

pub(super) fn validate_wasm_bytes(bytes: &[u8]) -> Result<(), String> {
    wasmparser::Validator::new()
        .validate_all(bytes)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Collects export names with their kinds, rejecting duplicate names.
///
/// Export names are unique in a well-formed module, so a duplicate means an assertion could match
/// whichever entry it happened to reach first.
pub(super) fn collect_wasm_exports(bytes: &[u8]) -> Result<BTreeMap<String, ExternalKind>, String> {
    let mut exports = BTreeMap::new();

    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|error| error.to_string())?;
        if let Payload::ExportSection(reader) = payload {
            for export in reader {
                let export = export.map_err(|error| error.to_string())?;
                if exports
                    .insert(export.name.to_owned(), export.kind)
                    .is_some()
                {
                    return Err(format!(
                        "the module exports '{}' more than once, so export assertions cannot \
                         identify which entry they inspect",
                        export.name
                    ));
                }
            }
        }
    }

    Ok(exports)
}

/// Collects `module.name` import identities, rejecting duplicates.
///
/// Moth emits each host import once, so a repeated identity is a backend defect rather than
/// something an import assertion should silently accept.
pub(super) fn collect_wasm_imports(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut imports = Vec::new();

    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|error| error.to_string())?;
        if let Payload::ImportSection(reader) = payload {
            for import in reader {
                let import = match import {
                    Ok(imports) => match imports {
                        Imports::Single(_, import) => import,
                        Imports::Compact1 { module, .. } | Imports::Compact2 { module, .. } => {
                            return Err(format!(
                                "collect_wasm_imports: compact import group for module '{module}' \
                                 is not supported; Moth does not emit compact imports"
                            ));
                        }
                    },
                    Err(error) => return Err(error.to_string()),
                };
                let identity = format!("{}.{}", import.module, import.name);
                if imports.contains(&identity) {
                    return Err(format!(
                        "the module imports '{identity}' more than once, so import assertions \
                         cannot identify which entry they inspect"
                    ));
                }
                imports.push(identity);
            }
        }
    }

    Ok(imports)
}
