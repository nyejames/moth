//! Path display helpers for diagnostic rendering.
//!
//! WHAT: converts explicit host paths into stable user-facing relative text.
//! WHY: source diagnostics already carry their path through an attached source identity context;
//! only infrastructure errors need a direct host-path display helper.

use crate::compiler_frontend::utilities::basic::{normalize_path, portable_path_text};
use std::path::Path;

pub(crate) fn relative_display_path_from_root(scope: &Path, root: &Path) -> String {
    let normalized_scope = normalize_path(scope);
    let normalized_root = normalize_path(root);

    let display_path = normalized_scope
        .strip_prefix(&normalized_root)
        .unwrap_or(&normalized_scope);

    // A path equal to the display root is still a real host path.
    if display_path.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        portable_path_text(display_path)
    }
}
