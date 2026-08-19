//! Wasm artifact validation and HTML-Wasm backend baseline checks.
//!
//! WHAT: parses Wasm bytes for validity, imports and exports, and validates the universal
//!       HTML-Wasm output contract.
//! WHY: Wasm structure belongs with Wasm assertions while artifact lookup and HTML kind checks
//!      remain shared in the artifact owner.

use super::super::ArtifactKind;
use super::artifacts::BuiltArtifactIndex;
use wasmparser::{Imports, Parser, Payload};

pub(super) fn validate_html_wasm_baseline_contract(
    index: &BuiltArtifactIndex<'_>,
) -> Option<String> {
    let Some(index_html) = index.get("index.html") else {
        return Some(
            "html_wasm baseline contract expected 'index.html', but it was not produced."
                .to_string(),
        );
    };

    let Some(html) = super::artifacts::output_text_content(index_html, ArtifactKind::Html) else {
        return Some(
            "html_wasm baseline contract expected 'index.html' as an HTML artifact.".to_string(),
        );
    };
    if let Some(reason) = super::artifacts::validate_html_document_structure(html, "html_wasm") {
        return Some(reason);
    }
    if !html.contains("<script src=\"./page.js\"></script>") {
        return Some(
            "html_wasm baseline contract expected 'index.html' to include './page.js'.".to_string(),
        );
    }
    let Some(script_pos) = html.find("<script src=\"./page.js\"></script>") else {
        return Some(
            "html_wasm baseline contract expected 'index.html' to include './page.js'.".to_string(),
        );
    };
    let Some(body_close) = html.find("</body>") else {
        return Some(
            "html_wasm baseline contract expected 'index.html' to contain '</body>'.".to_string(),
        );
    };
    if script_pos > body_close {
        return Some(
            "html_wasm baseline contract expected './page.js' to appear before '</body>'."
                .to_string(),
        );
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

    for required_fragment in [
        "__moth_instantiate_wasm",
        "instance.exports.moth_start()",
        "\"./page.wasm\"",
    ] {
        if !js.contains(required_fragment) {
            return Some(format!(
                "html_wasm baseline contract expected 'page.js' to contain '{required_fragment}'."
            ));
        }
    }

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

    for required_export in ["memory", "moth_str_ptr", "moth_str_len", "moth_release"] {
        if !exports.contains(&required_export.to_string()) {
            return Some(format!(
                "html_wasm baseline contract missing required export '{required_export}'. Available exports: {exports:?}."
            ));
        }
    }

    None
}

pub(super) fn validate_wasm_bytes(bytes: &[u8]) -> Result<(), String> {
    wasmparser::Validator::new()
        .validate_all(bytes)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub(super) fn collect_wasm_exports(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut exports = Vec::new();

    for payload in Parser::new(0).parse_all(bytes) {
        let payload = payload.map_err(|error| error.to_string())?;
        if let Payload::ExportSection(reader) = payload {
            for export in reader {
                let export = export.map_err(|error| error.to_string())?;
                exports.push(export.name.to_string());
            }
        }
    }

    Ok(exports)
}

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
                imports.push(format!("{}.{}", import.module, import.name));
            }
        }
    }

    Ok(imports)
}
