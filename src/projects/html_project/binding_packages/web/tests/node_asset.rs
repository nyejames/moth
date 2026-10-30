//! Shared Node runner for the built-in `@web` binding assets.
//!
//! WHAT: links one asset against the generated `@moth/runtime` module, imports it as `binding` and
//! runs `body` with `assert`, `runtime` and the asset in scope.
//! WHY: every asset check needs the same import rewrite, and the rewrite must be proven to happen
//! so a renamed runtime import can't silently load a stale module.

use crate::projects::html_project::external_js::runtime_module_registry::CoreJsRuntimeModule;
use std::process::Command;

pub(super) fn run_asset_in_node(binding: &str, asset_source: &str, body: &str) {
    let runtime_source = CoreJsRuntimeModule::moth_runtime_v1().source;
    let script = format!(
        r#"import assert from "node:assert/strict";
const runtimeUrl = "data:text/javascript," + encodeURIComponent({runtime_source:?});
const runtime = await import(runtimeUrl);
const assetSource = {asset_source:?};
const linkedSource = assetSource.replace('"@moth/runtime"', JSON.stringify(runtimeUrl));
assert.notEqual(linkedSource, assetSource, "{binding} asset must import @moth/runtime");
const {binding} = await import("data:text/javascript," + encodeURIComponent(linkedSource));
{body}"#
    );

    let directory =
        tempfile::tempdir().expect("should create temporary directory for asset script");
    let script_path = directory.path().join(format!("{binding}_check.mjs"));
    std::fs::write(&script_path, script).expect("should write asset script");

    let output = Command::new("node")
        .arg(&script_path)
        .output()
        .expect("Node must be available to execute built-in assets");
    assert!(
        output.status.success(),
        "{binding} asset failed in Node:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
