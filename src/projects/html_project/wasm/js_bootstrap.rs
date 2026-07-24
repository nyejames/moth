//! JS bootstrap generator for HTML+Wasm mode.
//!
//! WHAT: emits builder-owned JS that instantiates Wasm, wires host imports, hydrates
//! runtime slots from the fragment list returned by entry start(), and runs the lifecycle.
//! WHY: HTML assembly/orchestration remains builder policy while Wasm stays backend-generic.

use crate::compiler_frontend::compiler_errors::CompilerError;

/// Emits `page.js` for HTML Wasm mode.
///
/// WHAT: appends a Wasm bootstrap around lowered JS helpers and slot hydration.
/// WHY: entry start() is exported as "moth_start"; JS calls it directly and uses the
///      returned fragment list to hydrate slots. No per-function wrapper bindings needed.
pub(crate) fn generate_wasm_bootstrap_js(
    js_bundle: &str,
    slot_ids: &[String],
    start_invocation_js: &str,
) -> Result<String, CompilerError> {
    let mut out = String::new();
    out.push_str(js_bundle);
    out.push('\n');
    out.push('\n');
    out.push_str("const __moth_decoder = new TextDecoder(\"utf-8\");\n");
    out.push_str("const __moth_dom_registry = new Map();\n");
    out.push_str("let __moth_next_dom_handle = 1;\n");
    out.push('\n');
    out.push_str("function __moth_register_dom_node(node) {\n");
    out.push_str("  const handle = __moth_next_dom_handle;\n");
    out.push_str("  __moth_next_dom_handle += 1;\n");
    out.push_str("  __moth_dom_registry.set(handle, node);\n");
    out.push_str("  return handle;\n");
    out.push_str("}\n");
    out.push('\n');
    out.push_str("function __moth_lookup_dom_node(handle) {\n");
    out.push_str("  const node = __moth_dom_registry.get(handle);\n");
    out.push_str(
        "  if (!node) throw new Error(\"Unknown DOM node handle from Wasm host call: \" + handle);\n",
    );
    out.push_str("  return node;\n");
    out.push_str("}\n");
    out.push('\n');
    out.push_str("function __moth_read_string(instance, handle) {\n");
    out.push_str("  if (handle === 0 || handle === undefined || handle === null) return \"\";\n");
    out.push_str("  const ptr = instance.exports.moth_str_ptr(handle);\n");
    out.push_str("  const len = instance.exports.moth_str_len(handle);\n");
    out.push_str("  const bytes = new Uint8Array(instance.exports.memory.buffer, ptr, len);\n");
    out.push_str("  return __moth_decoder.decode(bytes);\n");
    out.push_str("}\n");
    out.push('\n');
    out.push_str("function __moth_take_string(instance, handle) {\n");
    out.push_str("  if (handle === 0 || handle === undefined || handle === null) return \"\";\n");
    out.push_str("  try {\n");
    out.push_str("    return __moth_read_string(instance, handle);\n");
    out.push_str("  } finally {\n");
    out.push_str("    instance.exports.moth_release(handle);\n");
    out.push_str("  }\n");
    out.push_str("}\n");
    out.push('\n');
    out.push_str("function __moth_build_imports(instance_ref) {\n");
    out.push_str("  return {\n");
    out.push_str("    host: {\n");
    out.push_str("      dom_create_text(handle) {\n");
    out.push_str(
        "        const text = __moth_take_string(instance_ref.current, handle);\n        return __moth_register_dom_node(document.createTextNode(text));\n",
    );
    out.push_str("      },\n");
    out.push_str("      dom_set_text(node_handle, text_handle) {\n");
    out.push_str(
        "        const node = __moth_lookup_dom_node(node_handle);\n        node.textContent = __moth_take_string(instance_ref.current, text_handle);\n",
    );
    out.push_str("      },\n");
    out.push_str("      dom_set_html(node_handle, html_handle) {\n");
    out.push_str(
        "        const node = __moth_lookup_dom_node(node_handle);\n        node.innerHTML = __moth_take_string(instance_ref.current, html_handle);\n",
    );
    out.push_str("      },\n");
    out.push_str("    },\n");
    out.push_str("  };\n");
    out.push_str("}\n");
    out.push('\n');
    out.push_str("async function __moth_instantiate_wasm(wasm_url, imports) {\n");
    out.push_str("  if (typeof WebAssembly.instantiateStreaming === \"function\") {\n");
    out.push_str("    try {\n");
    out.push_str(
        "      return await WebAssembly.instantiateStreaming(fetch(wasm_url), imports);\n",
    );
    out.push_str("    } catch (_error) {\n");
    out.push_str(
        "      // Fall back when streaming compilation is unavailable (for example MIME setup).\n",
    );
    out.push_str("    }\n");
    out.push_str("  }\n");
    out.push_str(
        "  const bytes = await fetch(wasm_url).then((response) => response.arrayBuffer());\n",
    );
    out.push_str("  return WebAssembly.instantiate(bytes, imports);\n");
    out.push_str("}\n");
    out.push('\n');
    out.push_str("(async function () {\n");
    out.push_str("  const instance_ref = { current: null };\n");
    out.push_str("  const imports = __moth_build_imports(instance_ref);\n");
    out.push_str(
        "  const { instance } = await __moth_instantiate_wasm(\"./page.wasm\", imports);\n",
    );
    out.push_str("  instance_ref.current = instance;\n");
    out.push('\n');

    if slot_ids.is_empty() {
        // No runtime slots — still call moth_start() once for lifecycle effects, then release
        // the returned fragment Vec handle.
        out.push_str("  const moth_frag_vec = ");
        out.push_str(start_invocation_js);
        out.push_str(";\n");
        out.push_str("  instance.exports.moth_release(moth_frag_vec);\n");
    } else {
        // WHAT: call moth_start() and decode the returned runtime fragment list to hydrate slots.
        // WHY: entry start() is the sole runtime fragment producer; builders call it once and
        //      use the returned Vec<String> elements to fill source-order slot placeholders.
        out.push_str("  const moth_slot_ids = [\n");
        for slot_id in slot_ids {
            out.push_str(&format!("    \"{slot_id}\",\n"));
        }
        out.push_str("  ];\n");
        out.push_str("  const moth_frag_vec = ");
        out.push_str(start_invocation_js);
        out.push_str(";\n");
        out.push_str("  try {\n");
        out.push_str("    const moth_frag_count = instance.exports.moth_vec_len(moth_frag_vec);\n");
        out.push_str("    for (let i = 0; i < moth_slot_ids.length; i += 1) {\n");
        out.push_str("      const el = document.getElementById(moth_slot_ids[i]);\n");
        out.push_str(
            "      if (!el) throw new Error(\"Missing runtime mount slot: \" + moth_slot_ids[i]);\n",
        );
        out.push_str("      if (i >= moth_frag_count) continue;\n");
        out.push_str(
            "      const moth_str_handle = instance.exports.moth_vec_get(moth_frag_vec, i);\n",
        );
        out.push_str("      const moth_ptr = instance.exports.moth_str_ptr(moth_str_handle);\n");
        out.push_str("      const moth_len = instance.exports.moth_str_len(moth_str_handle);\n");
        out.push_str(
            "      const moth_bytes = new Uint8Array(instance.exports.memory.buffer, moth_ptr, moth_len);\n",
        );
        out.push_str("      const moth_text = __moth_decoder.decode(moth_bytes);\n");
        out.push_str("      el.insertAdjacentHTML(\"beforeend\", moth_text);\n");
        out.push_str("    }\n");
        out.push_str("  } finally {\n");
        out.push_str("    instance.exports.moth_release(moth_frag_vec);\n");
        out.push_str("  }\n");
    }

    out.push_str("})().catch((error) => {\n");
    out.push_str("  console.error(\"Moth Wasm bootstrap failed\", error);\n");
    out.push_str("  throw error;\n");
    out.push_str("});\n");

    Ok(out)
}
