//! Tests for the JS-only HTML rendering path.

use super::*;
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::module_compilation::ResolvedConstFragment;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::projects::html_project::document_config::HtmlDocumentConfig;
use crate::projects::html_project::tests::test_support::{
    create_test_hir_module, create_test_module,
};
use std::collections::HashMap;
use std::path::Path;

#[test]
fn bootstrap_script_calls_start_once_and_hydrates_slots() {
    // WHAT: with runtime slots, the bootstrap calls start() to get fragments and hydrates them.
    // WHY: start() is the sole fragment producer; no per-function wrapper calls needed.
    let slot_ids = vec![String::from("moth-slot-0")];
    let script = render_runtime_bootstrap_script_html(
        "start_entry",
        "function start_entry() { return []; }",
        &slot_ids,
        false,
        false,
    );

    assert!(
        script.contains("moth_frags = start_entry()"),
        "bootstrap must call start() to get the fragment array"
    );
    assert!(
        script.contains("moth_slots"),
        "bootstrap must set up the slot ID list"
    );
    assert!(
        script.contains("insertAdjacentHTML"),
        "bootstrap must hydrate each slot"
    );
    // Verify start() call comes before slot list setup in emission order.
    let start_frag_pos = script
        .find("moth_frags = start_entry()")
        .expect("start call must be present");
    let slot_list_pos = script
        .find("moth_slots")
        .expect("slot list must be present");
    assert!(
        start_frag_pos < slot_list_pos,
        "start() must be called before the slot ID list is set up"
    );
}

#[test]
fn render_entry_fragments_preserves_runtime_slot_order() {
    let (body_html, slot_ids) =
        render_entry_fragments(&[], 2).expect("plain runtime slots should render");

    let slot0_pos = body_html
        .find("moth-slot-0")
        .expect("moth-slot-0 must be present");
    let slot1_pos = body_html
        .find("moth-slot-1")
        .expect("moth-slot-1 must be present");

    assert!(
        slot0_pos < slot1_pos,
        "runtime slots must appear in source fragment order"
    );
    assert_eq!(slot_ids.len(), 2);
    assert_eq!(slot_ids[0], "moth-slot-0");
    assert_eq!(slot_ids[1], "moth-slot-1");
}

/// Builds a stable resource origin for renderer error-path tests.
fn fixture_resource_origin() -> StableResourceOriginId {
    StableResourceOriginId::module_owned(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("js-path-tests"),
            String::new(),
            ModuleRootRole::Normal,
        ),
        PortableResourcePath::from_relative_logical_path(Path::new("assets/logo.svg"))
            .expect("fixture resource path should be portable"),
    )
}

#[test]
fn render_entry_fragments_preserves_text_and_all_text_piece_bytes() {
    // WHAT: identical authored bytes, insertion indices, and slot counts render identically
    //      whether const fragments use concrete text or all-text structural pieces.
    // WHY: converting an all-text Pieces value at the builder boundary must not change source
    //      order, introduce separators, or otherwise alter the final HTML bytes.
    let text_fragments = vec![
        ResolvedConstFragment {
            runtime_insertion_index: 0,
            value: OwnedFoldedString::Text(String::from("<head>")),
        },
        ResolvedConstFragment {
            runtime_insertion_index: 2,
            value: OwnedFoldedString::Text(String::from("</html>")),
        },
        ResolvedConstFragment {
            runtime_insertion_index: 1,
            value: OwnedFoldedString::Text(String::from("<main>body")),
        },
    ];
    let piece_fragments = vec![
        ResolvedConstFragment {
            runtime_insertion_index: 0,
            value: OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Text(String::from(
                "<head>",
            ))]),
        },
        ResolvedConstFragment {
            runtime_insertion_index: 2,
            value: OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Text(String::from(
                "</html>",
            ))]),
        },
        ResolvedConstFragment {
            runtime_insertion_index: 1,
            value: OwnedFoldedString::Pieces(vec![
                OwnedFoldedStringPiece::Text(String::from("<main>")),
                OwnedFoldedStringPiece::Text(String::from("body")),
            ]),
        },
    ];

    let (text_html, text_slot_ids) =
        render_entry_fragments(&text_fragments, 2).expect("text fragments should render");
    let (piece_html, piece_slot_ids) =
        render_entry_fragments(&piece_fragments, 2).expect("all-text fragments should render");
    let expected_html = "<head>\n<div id=\"moth-slot-0\"></div>\n<main>body\n<div id=\"moth-slot-1\"></div>\n</html>\n";
    let expected_slot_ids = vec![String::from("moth-slot-0"), String::from("moth-slot-1")];

    assert_eq!(text_html, expected_html);
    assert_eq!(piece_html, expected_html);
    assert_eq!(
        text_html, piece_html,
        "Text and all-text Pieces fragments must render byte-identical HTML"
    );
    assert_eq!(
        text_slot_ids, piece_slot_ids,
        "Text and all-text Pieces fragments must produce identical slot IDs"
    );
    assert_eq!(text_slot_ids, expected_slot_ids);
}

#[test]
fn render_entry_fragments_rejects_resource_piece_at_builder_boundary() {
    // WHAT: a resource-bearing const fragment remains an internal renderer error.
    // WHY: URL assignment has not landed yet, so final HTML text cannot be produced for a
    //      structural resource piece.
    let fragments = vec![ResolvedConstFragment {
        runtime_insertion_index: 0,
        value: OwnedFoldedString::Pieces(vec![
            OwnedFoldedStringPiece::Text(String::from("before")),
            OwnedFoldedStringPiece::Resource(fixture_resource_origin()),
            OwnedFoldedStringPiece::Text(String::from("after")),
        ]),
    }];

    let error = render_entry_fragments(&fragments, 0)
        .expect_err("a resource piece must remain unresolved at the builder boundary");

    let message = format!("{error:?}");
    assert!(
        message.contains("HTML builder boundary"),
        "resource rendering should report the builder-boundary wall: {message}"
    );
    assert!(
        message.contains("URL assignment"),
        "resource rendering should identify the missing URL assignment: {message}"
    );
}

#[test]
fn render_entry_fragments_rejects_site_root_piece_at_builder_boundary() {
    // WHAT: a site-root const fragment remains an internal renderer error.
    // WHY: site-root URL context is also assigned only after this builder boundary.
    let fragments = vec![ResolvedConstFragment {
        runtime_insertion_index: 0,
        value: OwnedFoldedString::Pieces(vec![
            OwnedFoldedStringPiece::Text(String::from("before")),
            OwnedFoldedStringPiece::SiteRoot,
            OwnedFoldedStringPiece::Text(String::from("after")),
        ]),
    }];

    let error = render_entry_fragments(&fragments, 0)
        .expect_err("a site-root piece must remain unresolved at the builder boundary");

    let message = format!("{error:?}");
    assert!(
        message.contains("HTML builder boundary"),
        "site-root rendering should report the builder-boundary wall: {message}"
    );
    assert!(
        message.contains("URL assignment"),
        "site-root rendering should identify the missing URL assignment: {message}"
    );
}

#[test]
fn no_runtime_fragments_still_emits_start_call() {
    let mut string_table = StringTable::new();
    let module = create_test_module(std::path::PathBuf::from("@page.moth"), &mut string_table);
    let function_names = HashMap::from([(
        module
            .executable
            .hir
            .start_function
            .expect("entry module should have start"),
        String::from("start_entry"),
    )]);

    let html = render_html_document(
        &mut crate::projects::html_project::js_path::HtmlDocumentRenderInput {
            hir_module: &module.executable.hir,
            const_fragments: &[],
            string_table: &mut string_table,
            document_config: &HtmlDocumentConfig::default(),
            logical_html_path: Path::new("index.html"),
            project_name: "",
            js_bundle: "function start_entry() { return []; }",
            function_names: &function_names,
            entry_runtime_fragment_count: 0,
            uses_reactive_runtime_fragments: false,
            import_map_html: None,
            use_module_script: false,
        },
    )
    .expect("render_html_document should succeed");

    assert!(
        !html.contains("moth-slot-"),
        "no runtime slots should be present when there are no runtime fragments"
    );
    assert!(
        html.contains("start_entry()"),
        "start() must still be called when there are no runtime fragments"
    );
}

#[test]
fn escape_inline_script_replaces_closing_tag_sequence() {
    let js = "const x = \"</script>\";";
    let escaped = escape_inline_script(js);

    assert_eq!(escaped, "const x = \"<\\/script>\";");
    assert!(
        !escaped.contains("</"),
        "escaped JS must not contain any '</' sequence"
    );
}

#[test]
fn inline_js_bundle_with_closing_script_tag_is_escaped_in_html() {
    let hir_module = create_test_hir_module();
    let function_names = HashMap::from([(
        hir_module
            .start_function
            .expect("entry module should have start"),
        String::from("start_entry"),
    )]);

    let mut string_table = crate::compiler_frontend::symbols::string_interning::StringTable::new();
    let html = render_html_document(
        &mut crate::projects::html_project::js_path::HtmlDocumentRenderInput {
            hir_module: &hir_module,
            const_fragments: &[],
            string_table: &mut string_table,
            document_config: &HtmlDocumentConfig::default(),
            logical_html_path: Path::new("index.html"),
            project_name: "",
            js_bundle: "const msg = \"</script>\";\n",
            function_names: &function_names,
            entry_runtime_fragment_count: 0,
            uses_reactive_runtime_fragments: false,
            import_map_html: None,
            use_module_script: false,
        },
    )
    .expect("render_html_document should succeed");

    assert!(
        !html.contains("</script>\";"),
        "raw </script> inside a JS string must not appear unescaped in HTML output"
    );
    assert!(
        html.contains("<\\/script>"),
        "the closing-tag sequence must be escaped as <\\/script> in the output"
    );
}

#[test]
fn bootstrap_uses_mount_helper_for_reactive_runtime_fragments() {
    // WHAT: when the module has reachable reactive runtime fragments, the bootstrap must call the
    // backend mount helper so template objects register for rerendering instead of being snapshot.
    let slot_ids = vec![String::from("moth-slot-0")];
    let script = render_runtime_bootstrap_script_html(
        "start_entry",
        "function start_entry() { return []; }",
        &slot_ids,
        false,
        true,
    );

    assert!(
        script.contains("__moth_mount_template_fragment(el, moth_frags[i])"),
        "reactive bootstrap must hydrate slots through the mount helper"
    );
    assert!(
        !script.contains("el.insertAdjacentHTML(\"beforeend\", moth_frags[i] || \"\")"),
        "reactive bootstrap must not use the plain direct insertion path"
    );
}

#[test]
fn bootstrap_uses_plain_insertion_for_non_reactive_runtime_fragments() {
    // WHAT: non-reactive pages must not reference the optional mount helper global.
    let slot_ids = vec![String::from("moth-slot-0")];
    let script = render_runtime_bootstrap_script_html(
        "start_entry",
        "function start_entry() { return []; }",
        &slot_ids,
        false,
        false,
    );

    assert!(
        script.contains("el.insertAdjacentHTML(\"beforeend\", moth_frags[i] || \"\")"),
        "non-reactive bootstrap must keep the plain direct insertion path"
    );
    assert!(
        !script.contains("__moth_mount_template_fragment"),
        "non-reactive bootstrap must not reference the mount helper"
    );
}
