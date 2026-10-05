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
use crate::projects::html_project::output_plan::derive_logical_html_path;
use crate::projects::html_project::page_metadata::HtmlPageMetadataPlan;
use crate::projects::html_project::resource_output_plan::{
    HtmlResourceOutputPlan, ResourceUrlContext, ResourceUseKind,
};
use crate::projects::html_project::structural_url_renderer::StructuralUrlRenderer;
use crate::projects::html_project::tests::test_support::{
    create_test_hir_module, create_test_module,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    let plan = HtmlResourceOutputPlan::new("js-path-tests");
    let context = ResourceUrlContext::PageDocument(PathBuf::from("index.html"));
    let renderer = StructuralUrlRenderer::new(&plan, &context, "/");
    let (body_html, slot_ids) =
        render_entry_fragments(&[], 2, &renderer).expect("plain runtime slots should render");

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
            span: None,
            value: OwnedFoldedString::Text(String::from("<head>")),
        },
        ResolvedConstFragment {
            runtime_insertion_index: 2,
            span: None,
            value: OwnedFoldedString::Text(String::from("</html>")),
        },
        ResolvedConstFragment {
            runtime_insertion_index: 1,
            span: None,
            value: OwnedFoldedString::Text(String::from("<main>body")),
        },
    ];
    let piece_fragments = vec![
        ResolvedConstFragment {
            runtime_insertion_index: 0,
            span: None,
            value: OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Text(String::from(
                "<head>",
            ))]),
        },
        ResolvedConstFragment {
            runtime_insertion_index: 2,
            span: None,
            value: OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Text(String::from(
                "</html>",
            ))]),
        },
        ResolvedConstFragment {
            runtime_insertion_index: 1,
            span: None,
            value: OwnedFoldedString::Pieces(vec![
                OwnedFoldedStringPiece::Text(String::from("<main>")),
                OwnedFoldedStringPiece::Text(String::from("body")),
            ]),
        },
    ];

    let plan = HtmlResourceOutputPlan::new("js-path-tests");
    let context = ResourceUrlContext::PageDocument(PathBuf::from("index.html"));
    let renderer = StructuralUrlRenderer::new(&plan, &context, "/");
    let (text_html, text_slot_ids) = render_entry_fragments(&text_fragments, 2, &renderer)
        .expect("text fragments should render");
    let (piece_html, piece_slot_ids) = render_entry_fragments(&piece_fragments, 2, &renderer)
        .expect("all-text fragments should render");
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
fn render_entry_fragments_renders_resource_piece_at_builder_boundary() {
    let origin = fixture_resource_origin();
    let mut string_table = StringTable::new();
    let mut plan = HtmlResourceOutputPlan::new("js-path-tests");
    let context = ResourceUrlContext::PageDocument(PathBuf::from("docs/index.html"));
    plan.plan_origin(
        origin.clone(),
        None,
        context.clone(),
        &mut string_table,
        ResourceUseKind::Metadata,
    )
    .expect("resource should be planned");
    let renderer = StructuralUrlRenderer::new(&plan, &context, "/");
    let fragments = vec![ResolvedConstFragment {
        runtime_insertion_index: 0,
        span: None,
        value: OwnedFoldedString::Pieces(vec![
            OwnedFoldedStringPiece::Text(String::from("before")),
            OwnedFoldedStringPiece::Resource(origin),
            OwnedFoldedStringPiece::Text(String::from("after")),
        ]),
    }];

    let (html, slot_ids) =
        render_entry_fragments(&fragments, 0, &renderer).expect("resource should render");

    assert_eq!(html, "before../assets/logo.svgafter\n");
    assert!(slot_ids.is_empty());
}

#[test]
fn render_entry_fragments_renders_site_root_piece_at_builder_boundary() {
    let plan = HtmlResourceOutputPlan::new("js-path-tests");
    let context = ResourceUrlContext::PageDocument(PathBuf::from("docs/index.html"));
    let renderer = StructuralUrlRenderer::new(&plan, &context, "/moth");
    let fragments = vec![ResolvedConstFragment {
        runtime_insertion_index: 0,
        span: None,
        value: OwnedFoldedString::Pieces(vec![
            OwnedFoldedStringPiece::Text(String::from("before")),
            OwnedFoldedStringPiece::SiteRoot,
            OwnedFoldedStringPiece::Text(String::from("after")),
        ]),
    }];

    let (html, slot_ids) =
        render_entry_fragments(&fragments, 0, &renderer).expect("site root should render");

    assert_eq!(html, "before/moth/after\n");
    assert!(slot_ids.is_empty());
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

    let route =
        derive_logical_html_path(Path::new("@page.moth"), None).expect("root route should resolve");
    let plan = HtmlResourceOutputPlan::new("");
    let context = ResourceUrlContext::PageDocument(route.logical_html_path.clone());
    let renderer = StructuralUrlRenderer::new(&plan, &context, "/");
    let page_metadata_plan = HtmlPageMetadataPlan::default();
    let html = render_html_document(
        &mut crate::projects::html_project::js_path::HtmlDocumentRenderInput {
            hir_module: &module.executable.hir,
            page_metadata_plan: &page_metadata_plan,
            const_fragments: &[],
            string_table: &mut string_table,
            structural_url_renderer: &renderer,
            document_config: &HtmlDocumentConfig::default(),
            route: &route,
            project_name: "",
            js_bundle: "function start_entry() { return []; }",
            function_names: &function_names,
            start_is_fallible: false,
            release_build: false,
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
    let route =
        derive_logical_html_path(Path::new("@page.moth"), None).expect("root route should resolve");
    let plan = HtmlResourceOutputPlan::new("");
    let context = ResourceUrlContext::PageDocument(route.logical_html_path.clone());
    let renderer = StructuralUrlRenderer::new(&plan, &context, "/");
    let page_metadata_plan = HtmlPageMetadataPlan::default();
    let html = render_html_document(
        &mut crate::projects::html_project::js_path::HtmlDocumentRenderInput {
            hir_module: &hir_module,
            page_metadata_plan: &page_metadata_plan,
            const_fragments: &[],
            string_table: &mut string_table,
            structural_url_renderer: &renderer,
            document_config: &HtmlDocumentConfig::default(),
            route: &route,
            project_name: "",
            js_bundle: "const msg = \"</script>\";\n",
            function_names: &function_names,
            start_is_fallible: false,
            release_build: false,
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
        false,
        false,
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

#[test]
fn infallible_bootstrap_invokes_once_with_or_without_fragments() {
    for is_module_script in [false, true] {
        for slot_count in [0, 1] {
            let slots = (0..slot_count)
                .map(|index| format!("moth-slot-{index}"))
                .collect::<Vec<_>>();
            let html = render_runtime_bootstrap_script_html(
                "start_entry",
                r#"const events = [];
function start_entry() { events.push("start"); return ["fragment"]; }
const document = { getElementById() { return { insertAdjacentHTML(_, text) { events.push(text); } }; } };"#,
                &slots,
                is_module_script,
                false,
                false,
                false,
            );
            let output = run_bootstrap_scripts(
                &html,
                "console.log(JSON.stringify(events));",
                is_module_script,
            );
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                if slot_count == 0 {
                    r#"["start"]"#
                } else {
                    r#"["start","fragment"]"#
                }
            );
        }
    }
}

#[test]
fn fallible_bootstrap_failure_records_details_once_without_publishing_application_text() {
    use crate::backends::js::ENTRY_FAILURE_NOTICE;

    for is_module_script in [false, true] {
        for reactive in [false, true] {
            for slot_count in [0, 2] {
                for release_build in [false, true] {
                    let slots = (0..slot_count)
                        .map(|index| format!("moth-slot-{index}"))
                        .collect::<Vec<_>>();
                    let caller = render_runtime_bootstrap_script_html(
                        "start_entry",
                        "",
                        &slots,
                        is_module_script,
                        reactive,
                        true,
                        release_build,
                    );
                    assert!(caller.contains(
                        "typeof globalThis.__moth_record_entry_failure === \"function\""
                    ));
                    assert!(caller.contains(
                        "globalThis.__moth_record_entry_failure(__moth_error_code(moth_error), __moth_error_message(moth_error), moth_error.__moth_location ?? null);"
                    ));
                    assert!(!caller.contains(".message"));
                    assert!(!caller.contains(".code"));
                    assert!(
                        caller.contains(&format!("process.stderr.write({ENTRY_FAILURE_NOTICE:?})"))
                    );
                    assert!(caller.contains("process.exitCode = 1"));
                    assert!(!caller.contains("<application-error>"));
                    if release_build {
                        assert!(caller.contains(&format!(
                            "document.createTextNode({RELEASE_ENTRY_FAILURE_NOTICE:?})"
                        )));
                    } else {
                        assert!(!caller.contains(RELEASE_ENTRY_FAILURE_NOTICE));
                        assert!(!caller.contains("createTextNode"));
                    }

                    for missing_dom in
                        [None, Some("document"), Some("body"), Some("createTextNode")]
                    {
                        let mut bundle = String::from(
                            r#"
const events = [];
let payload_reads = 0;
let error_message_reads = 0;
let error_code_reads = 0;
function __moth_error_code(error) { return error.code; }
function __moth_error_message(error) { return error.message; }
globalThis.__moth_record_startup_fault = () => { throw new Error("Returned Error is not a startup fault"); };
globalThis.__moth_record_entry_failure = (code, message, location) => {
    if (process.exitCode !== undefined) throw new Error("Entry recording must precede host status");
    events.push("entry-failure:" + code);
    if (message !== "<application-error>" || location !== source_location) throw new Error("Entry details lost");
};
const source_location = { file: "page.moth", line: 2, column: 3, function: "start" };
const failure = {
    get message() { error_message_reads++; return "<application-error>"; },
    get code() { error_code_reads++; return 0; },
    __moth_location: source_location,
};
function start_entry() {
    events.push("start");
    return { tag: "err", get value() { payload_reads++; return failure; } };
}
const static_content = { nodeType: 1, outerHTML: "<main>static authored content</main>" };
const children = [static_content];
globalThis.document = {
    body: {
        appendChild(node) { children.push(node); },
        set innerHTML(_) { throw new Error("Static content must not be replaced"); },
        set textContent(_) { throw new Error("Static content must not be replaced"); }
    },
    createTextNode(data) {
        events.push("text-node");
        return { nodeType: 3, data };
    },
    getElementById(id) {
        events.push("lookup:" + id);
        return { insertAdjacentHTML(_, text) { events.push(text); } };
    }
};
function __moth_mount_template_fragment(_, fragment) { events.push(fragment); }
"#,
                        );
                        match missing_dom {
                            None => {}
                            Some("document") => bundle.push_str("delete globalThis.document;\n"),
                            Some(property) => {
                                bundle.push_str(&format!("delete document.{property};\n"));
                            }
                        }
                        let html = render_runtime_bootstrap_script_html(
                            "start_entry",
                            &bundle,
                            &slots,
                            is_module_script,
                            reactive,
                            true,
                            release_build,
                        );
                        let output = run_bootstrap_scripts(
                            &html,
                            "console.log(JSON.stringify([events, payload_reads, error_message_reads, error_code_reads, children[0] === static_content, children]));",
                            is_module_script,
                        );
                        assert_eq!(
                            output.status.code(),
                            Some(1),
                            "Failure must exit 1 without a host exception: {}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                        assert_eq!(output.stderr, ENTRY_FAILURE_NOTICE.as_bytes());
                        let stdout =
                            String::from_utf8(output.stdout).expect("Node.js output is UTF-8");
                        assert!(!stdout.contains("<application-error>"));
                        if release_build && missing_dom.is_none() {
                            assert_eq!(
                                stdout.trim(),
                                format!(
                                    r#"[["start","entry-failure:0","text-node"],1,1,1,true,[{{"nodeType":1,"outerHTML":"<main>static authored content</main>"}},{{"nodeType":3,"data":{RELEASE_ENTRY_FAILURE_NOTICE:?}}}]]"#
                                )
                            );
                        } else {
                            assert_eq!(
                                stdout.trim(),
                                r#"[["start","entry-failure:0"],1,1,1,true,[{"nodeType":1,"outerHTML":"<main>static authored content</main>"}]]"#
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn startup_faults_are_classified_and_rethrown_without_intercepting_later_scripts() {
    for is_module_script in [false, true] {
        for fallible in [false, true] {
            for scenario in [
                "assertion",
                "plain",
                "primitive",
                "throwing-getter",
                "revoked-proxy",
                "throwing-hook",
                "host",
                "missing-slot",
                "later",
            ] {
                let action = match scenario {
                    "assertion" => {
                        "Object.defineProperty(fault, '__moth_assertion', { value: true }); throw fault;"
                    }
                    "plain" | "primitive" | "throwing-getter" | "revoked-proxy"
                    | "throwing-hook" => "throw fault;",
                    "host" => "host_binding();",
                    _ => "",
                };
                let result = if fallible {
                    "{ tag: 'ok', value: ['fragment'] }"
                } else {
                    "['fragment']"
                };
                let fault = match scenario {
                    "primitive" => "'plain thrown string'",
                    "throwing-getter" => {
                        "({ get __moth_assertion() { throw new Error('marker inspection failed'); } })"
                    }
                    "revoked-proxy" => {
                        "(() => { const { proxy, revoke } = Proxy.revocable({}, {}); revoke(); return proxy; })()"
                    }
                    _ => "new Error('assertion failed')",
                };
                let hook_failure = if scenario == "throwing-hook" {
                    "throw new Error('reporting failed');"
                } else {
                    ""
                };
                let bundle = format!(
                    "const events = [];\nconst fault = {fault};\n\
                     globalThis.__moth_record_entry_failure = () => events.push('entry');\n\
                     globalThis.__moth_record_startup_fault = (category, error) => {{ globalThis.startup_error = error; events.push([category, error === fault]); {hook_failure} }};\n\
                     function host_binding() {{ throw fault; }}\n\
                     function start_entry() {{ events.push('start'); {action} return {result}; }}\n\
                     const document = {{ getElementById() {{ return {}; }} }};\n\
                     try {{\n",
                    if scenario == "missing-slot" {
                        "null"
                    } else {
                        "{ insertAdjacentHTML(_, text) { events.push(text); } }"
                    },
                );
                // This outer host catch observes the raw rethrow, not a browser-wide handler.
                let html = render_runtime_bootstrap_script_html(
                    "start_entry",
                    &bundle,
                    &[String::from("moth-slot-0")],
                    is_module_script,
                    false,
                    fallible,
                    false,
                );
                assert!(!html.contains("onerror"));
                assert!(!html.contains("unhandledrejection"));
                let summary = if scenario == "later" {
                    "} catch (error) { events.push(['rethrow', error === fault]); }\ntry { throw fault; } catch (error) { events.push(['later', error === fault]); }\nconsole.log(JSON.stringify(events));"
                } else {
                    "} catch (error) { if (error !== globalThis.startup_error) throw new Error('Rethrow changed identity'); events.push(['rethrow', error === fault]); }\nconsole.log(JSON.stringify(events));"
                };
                let output = run_bootstrap_scripts(&html, summary, is_module_script);
                assert!(
                    output.status.success(),
                    "{scenario}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let expected = match scenario {
                    "assertion" => r#"["start",["assertion",true],["rethrow",true]]"#,
                    "missing-slot" => r#"["start",["startup_fault",false],["rethrow",false]]"#,
                    "later" => r#"["start","fragment",["later",true]]"#,
                    _ => r#"["start",["startup_fault",true],["rethrow",true]]"#,
                };
                assert_eq!(
                    String::from_utf8_lossy(&output.stdout).trim(),
                    expected,
                    "{scenario}"
                );
            }
        }
    }
}

#[test]
fn fallible_bootstrap_success_unwraps_once_and_hydrates_in_source_order() {
    let slots = vec![String::from("moth-slot-0"), String::from("moth-slot-1")];
    for is_module_script in [false, true] {
        for reactive in [false, true] {
            for release_build in [false, true] {
                let bundle = r#"
const events = [];
globalThis.__moth_record_entry_failure = (code) => events.push("entry-failure:" + code);
function __moth_error_code(_) { throw new Error("Success must not inspect an Error code"); }
function start_entry() {
    events.push("start");
    return {
        tag: "ok",
        get value() { events.push("unwrap"); return ["first", "second"]; }
    };
}
const document = {
    getElementById(id) {
        return { id, insertAdjacentHTML(_, text) { events.push(id + ":" + text); } };
    }
};
function __moth_mount_template_fragment(element, fragment) {
    events.push(element.id + ":" + fragment);
}
"#;
                let html = render_runtime_bootstrap_script_html(
                    "start_entry",
                    bundle,
                    &slots,
                    is_module_script,
                    reactive,
                    true,
                    release_build,
                );
                let output = run_bootstrap_scripts(
                    &html,
                    "console.log(JSON.stringify(events));",
                    is_module_script,
                );
                assert!(
                    output.status.success(),
                    "Bootstrap runtime failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(
                    output.stderr.is_empty(),
                    "Success must not report a terminal notice"
                );
                assert_eq!(
                    String::from_utf8(output.stdout)
                        .expect("Node.js output is UTF-8")
                        .trim(),
                    r#"["start","unwrap","moth-slot-0:first","moth-slot-1:second"]"#
                );
            }
        }
    }
}

/// Executes the generated inline scripts in document order, preserving their shared scope.
fn run_bootstrap_scripts(
    html: &str,
    summary: &str,
    is_module_script: bool,
) -> std::process::Output {
    let mut source = String::new();
    for script in html.split("<script").skip(1) {
        let (_, body) = script
            .split_once('>')
            .expect("generated script has an opener");
        let (body, _) = body
            .split_once("</script>")
            .expect("generated script has a closer");
        source.push_str(body);
        source.push('\n');
    }
    source.push_str(summary);

    let mut command = std::process::Command::new("node");
    if is_module_script {
        command.arg("--input-type=module");
    }
    command
        .args(["--eval", &source])
        .output()
        .expect("Node.js is required for bootstrap runtime tests")
}
