//! Runtime coverage for the builder-owned asynchronous Wasm startup boundary.

use super::generate_wasm_bootstrap_js;

#[test]
fn wasm_startup_faults_preserve_identity_and_distinguish_assertions() {
    for scenario in [
        "fetch",
        "instantiate",
        "start",
        "assertion",
        "assertion-import",
        "throwing-getter",
        "revoked-proxy",
        "throwing-hook",
        "host",
        "missing-slot",
        "hydrate",
    ] {
        let fault = match scenario {
            "throwing-getter" => {
                "({ get __moth_assertion() { throw new Error('marker inspection failed'); } })"
            }
            "revoked-proxy" => {
                "(() => { const { proxy, revoke } = Proxy.revocable({}, {}); revoke(); return proxy; })()"
            }
            _ => "new Error('assertion failed')",
        };
        let setup = format!(
            r#"
const events = [];
const fault = {fault};
const scenario = {scenario:?};
let owned_imports;
let starts = 0;
const memory = {{ buffer: new ArrayBuffer(64) }};
new Uint8Array(memory.buffer).set(new TextEncoder().encode("assertion failed"));
const instance = {{ exports: {{
    memory,
    moth_start() {{
        starts++;
        if (scenario === "assertion") Object.defineProperty(fault, "__moth_assertion", {{ value: true }});
        if (["start", "assertion", "throwing-getter", "revoked-proxy", "throwing-hook"].includes(scenario)) throw fault;
        if (scenario === "assertion-import") owned_imports.host.assertion_failed(9);
        if (scenario === "host") owned_imports.host.dom_set_text(999, 9);
        return 7;
    }},
    moth_release(handle) {{ events.push("release:" + handle); }},
    moth_vec_len() {{ return 1; }},
    moth_vec_get() {{ return 9; }},
    moth_str_ptr() {{ return 0; }},
    moth_str_len() {{ return 16; }}
}} }};
globalThis.fetch = async () => {{
    if (scenario === "fetch") throw fault;
    return {{ arrayBuffer: async () => new ArrayBuffer(0) }};
}};
globalThis.WebAssembly = {{ instantiate: async (_, imports) => {{
    owned_imports = imports;
    if (scenario === "instantiate") throw fault;
    return {{ instance }};
}} }};
globalThis.document = {{ getElementById() {{
    if (scenario === "missing-slot") return null;
    return {{ insertAdjacentHTML() {{ throw fault; }} }};
}} }};
globalThis.__moth_record_entry_failure = () => events.push("entry");
let recorded_error;
globalThis.__moth_record_startup_fault = (category, error) => {{
    recorded_error = error;
    events.push(category);
    if (scenario === "throwing-hook") throw new Error("reporting failed");
}};
console.error = (message, error) => events.push(["console", message, error === recorded_error]);
process.once("unhandledRejection", error => {{
    events.push(["rethrow", error === recorded_error, error === fault, starts]);
    if (scenario === "assertion-import") {{
        events.push([error.message, Object.hasOwn(error, "__moth_assertion"), Object.getOwnPropertyDescriptor(error, "__moth_assertion").enumerable]);
    }}
    console.log(JSON.stringify(events));
}});
"#
        );
        let slots = if matches!(scenario, "hydrate" | "missing-slot") {
            vec![String::from("moth-slot-0")]
        } else {
            vec![]
        };
        let bootstrap = generate_wasm_bootstrap_js(&setup, &slots, "instance.exports.moth_start()")
            .expect("Wasm bootstrap generation succeeds");
        assert!(!bootstrap.contains("onerror"));
        assert!(!bootstrap.contains("unhandledrejection"));
        let output = std::process::Command::new("node")
            .args(["--eval", &bootstrap])
            .output()
            .expect("Node.js is required for Wasm bootstrap tests");
        assert!(
            output.status.success(),
            "{scenario}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let events: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("Node emits event JSON");
        let expected_category = if matches!(scenario, "assertion" | "assertion-import") {
            "assertion"
        } else {
            "startup_fault"
        };
        let category_index = if matches!(scenario, "hydrate" | "missing-slot" | "assertion-import")
        {
            1
        } else {
            0
        };
        assert_eq!(
            events[category_index], expected_category,
            "{scenario}: {events}"
        );
        assert_eq!(
            events[category_index + 1],
            serde_json::json!(["console", "Moth Wasm bootstrap failed", true])
        );
        assert_eq!(
            events[category_index + 2],
            serde_json::json!([
                "rethrow",
                true,
                !matches!(scenario, "host" | "missing-slot" | "assertion-import"),
                if matches!(scenario, "fetch" | "instantiate") {
                    0
                } else {
                    1
                }
            ])
        );
        if scenario == "assertion-import" {
            assert_eq!(events[0], "release:9");
            assert_eq!(
                events[category_index + 3],
                serde_json::json!(["assertion failed", true, false])
            );
        }
    }
}

#[test]
fn wasm_success_and_unrelated_later_fault_do_not_report_startup_failure() {
    let setup = r#"
const events = [];
globalThis.fetch = async () => ({ arrayBuffer: async () => new ArrayBuffer(0) });
globalThis.WebAssembly = { instantiate: async () => ({ instance: { exports: {
    moth_start() { events.push("start"); return 7; },
    moth_release(handle) { events.push("release:" + handle); }
} } }) };
globalThis.__moth_record_startup_fault = () => events.push("startup-fault");
setImmediate(() => {
    try { throw new Error("unrelated later script"); } catch (_) { events.push("later"); }
    console.log(JSON.stringify(events));
});
"#;
    let bootstrap = generate_wasm_bootstrap_js(setup, &[], "instance.exports.moth_start()")
        .expect("Wasm bootstrap generation succeeds");
    let output = std::process::Command::new("node")
        .args(["--eval", &bootstrap])
        .output()
        .expect("Node.js is required for Wasm bootstrap tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        r#"["start","release:7","later"]"#
    );
}
