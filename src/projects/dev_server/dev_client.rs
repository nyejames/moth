//! Shared dev-client snippet helpers for hot reload pages.
//!
//! WHAT: owns the injected EventSource marker and script HTML used by both normal pages and
//! dev-server error pages.
//! WHY: keeping one origin-aware source of truth prevents drift between success and failure views.

use crate::projects::routing::prefix_origin;

pub const DEV_CLIENT_MARKER: &str = "<!-- moth-dev-client -->";

pub fn dev_client_snippet(origin: &str) -> String {
    let sse_path = prefix_origin(origin, "/__moth/events");
    format!(
        "\n{DEV_CLIENT_MARKER}\n<script>\n  (() => {{\n    const source = new EventSource('{sse_path}');\n    source.addEventListener('reload', () => window.location.reload());\n  }})();\n</script>\n"
    )
}
