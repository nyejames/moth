//! Zero-cost timer erasure gate.
//!
//! WHAT: builds the `moth` release binary without any timer-related feature
//!      and proves that timer-only implementation markers are absent from the
//!      produced bytes. It also audits the source tree for runtime `cfg!`
//!      checks and for direct calls into the enabled timer implementation
//!      outside the facade. It also rejects no-op timer closure wrappers:
//!      disabled frontend timer expansions must be the production expression
//!      itself, never a closure call.
//! WHY:  the plan's primary invariant is that a compiler built without
//!       `timers` performs no timer-system runtime work. Source-level erasure
//!       tests prove macro semantics; this gate proves the compiled artifact
//!       contains none of the timer-only strings that a live implementation
//!       would retain.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Timer-only marker strings that must never appear in a no-timer binary.
///
/// `MOTH_BENCH status` is deliberately absent: it is the separate
/// benchmark-status contract and remains valid without `timers`.
const TIMER_ONLY_MARKERS: &[&str] = &[
    "MOTH_TIMERS",
    "MOTH_BENCH timing",
    "Timing summary:",
    "Build timings",
    "Compilation boundaries",
    "backend.js.lower_hir",
    "backend.js.lower_linked_hir",
    "build.boundary.inventory",
    "frontend.module.semantic_total",
    "frontend.public_interface",
    "frontend.generated_functions",
    "frontend.borrow.generated",
    "ast_build_environment_ms",
    "ast_emit_nodes_ms",
    "ast_finalize_ms",
    "command.dev.build_and_write",
    "command.dev.cycle",
];

/// Run the complete erasure gate.
pub fn run_timers_erasure_check() -> Result<(), String> {
    let workspace_root = workspace_root()?;

    build_no_timer_release_binary(&workspace_root)?;

    let binary_path = no_timer_release_binary_path(&workspace_root);
    let binary_bytes = fs::read(&binary_path).map_err(|error| {
        format!(
            "failed to read the no-timer release binary '{}': {error}",
            binary_path.display()
        )
    })?;

    let present_markers = find_present_markers(&binary_bytes, TIMER_ONLY_MARKERS);
    if !present_markers.is_empty() {
        return Err(format!(
            "no-timer release binary contains timer-only markers: {}",
            present_markers.join(", ")
        ));
    }

    let source_failures = audit_timer_sources(&workspace_root);
    if !source_failures.is_empty() {
        return Err(format!(
            "timer source audit failed:\n{}",
            source_failures.join("\n")
        ));
    }

    println!(
        "timers-erasure-check: no-timer binary clean ({} bytes), source audit clean",
        binary_bytes.len()
    );
    Ok(())
}

/// Return the markers present in the given bytes, in declaration order.
fn find_present_markers<'a>(bytes: &[u8], markers: &'a [&str]) -> Vec<&'a str> {
    markers
        .iter()
        .copied()
        .filter(|marker| {
            bytes
                .windows(marker.len())
                .any(|window| window == marker.as_bytes())
        })
        .collect()
}

/// Build the `moth` binary with no default features in an isolated target dir.
fn build_no_timer_release_binary(workspace_root: &Path) -> Result<(), String> {
    let target_dir = workspace_root.join("target").join("timers-erasure-check");

    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--no-default-features")
        .arg("--bin")
        .arg("moth")
        .env("CARGO_TARGET_DIR", &target_dir)
        .current_dir(workspace_root)
        .status()
        .map_err(|error| format!("failed to launch the no-timer release build: {error}"))?;

    if !status.success() {
        return Err("the no-timer release build failed".to_string());
    }

    Ok(())
}

/// Locate the built no-timer release binary.
fn no_timer_release_binary_path(workspace_root: &Path) -> PathBuf {
    let binary_name = if cfg!(windows) { "moth.exe" } else { "moth" };
    workspace_root
        .join("target")
        .join("timers-erasure-check")
        .join("release")
        .join(binary_name)
}

/// Audit source for runtime timer checks and direct implementation calls.
fn audit_timer_sources(workspace_root: &Path) -> Vec<String> {
    let mut failures = Vec::new();
    let src_dir = workspace_root.join("src");

    for entry in walk_rust_files(&src_dir) {
        let relative = entry
            .strip_prefix(workspace_root)
            .unwrap_or(&entry)
            .display()
            .to_string();
        let is_facade_path = entry.starts_with(src_dir.join("timing"));
        let content = match fs::read_to_string(&entry) {
            Ok(content) => content,
            Err(error) => {
                failures.push(format!("{relative}: unreadable ({error})"));
                continue;
            }
        };

        failures.extend(audit_source_fragment(&relative, &content, is_facade_path));
    }

    failures
}

/// Run the source-level audit rules against one Rust source fragment.
///
/// Separated from the directory walk so the closure-wrapper rules have
/// focused unit coverage.
fn audit_source_fragment(relative: &str, content: &str, is_facade_path: bool) -> Vec<String> {
    let mut failures = Vec::new();

    if content.contains("cfg!(feature = \"timers\")") {
        failures.push(format!(
            "{relative}: uses runtime cfg! check; use #[cfg] macro definitions instead"
        ));
    }

    if !is_facade_path
        && (content.contains("timing::enabled::") || content.contains("timing::collector::"))
    {
        failures.push(format!(
            "{relative}: calls enabled timer implementation directly; use the facade macros"
        ));
    }

    if content.contains("$stage()") {
        failures.push(format!(
            "{relative}: timed_frontend_stage! expands through a closure call; expand the production expression directly"
        ));
    }

    if content.contains("$substep()") || content.contains("substep()") {
        failures.push(format!(
            "{relative}: timed_frontend_substep! expands through a closure call; expand the production expression directly"
        ));
    }

    if content.contains("fn timed_frontend_substep<") || content.contains("timed_frontend_substep(")
    {
        failures.push(format!(
            "{relative}: timed_frontend_substep must be a direct-expression macro, not a function wrapper"
        ));
    }

    failures
}

fn walk_rust_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];

    while let Some(directory) = pending.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }

    files
}

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "xtask manifest has no parent directory".to_string())
}

#[cfg(test)]
mod tests {
    use super::{audit_source_fragment, find_present_markers};

    #[test]
    fn finds_present_markers_in_declaration_order() {
        let bytes = b"prefix MOTH_BENCH timing suffix MOTH_TIMERS";
        let markers = &["MOTH_TIMERS", "MOTH_BENCH timing", "Timing summary:"];

        assert_eq!(
            find_present_markers(bytes, markers),
            vec!["MOTH_TIMERS", "MOTH_BENCH timing"]
        );
    }

    #[test]
    fn rejects_none_when_markers_are_absent() {
        let bytes = b"MOTH_BENCH status errors=0 warnings=0";
        let markers = &["MOTH_TIMERS", "MOTH_BENCH timing", "Timing summary:"];

        assert!(find_present_markers(bytes, markers).is_empty());
    }

    #[test]
    fn rejects_closure_wrapper_expansion_in_frontend_stage_macro() {
        let failures = audit_source_fragment(
            "src/timing.rs",
            "macro_rules! timed_frontend_stage {\n  ($stage:expr) => {{ $stage() }};\n}",
            true,
        );

        assert!(
            failures
                .iter()
                .any(|failure| failure.contains("closure call")),
            "expected a closure-wrapper failure, got: {failures:?}"
        );
    }

    #[test]
    fn rejects_function_wrapper_form_of_frontend_substep() {
        let failures = audit_source_fragment(
            "src/build_system/create_project_modules/frontend_orchestration.rs",
            "fn timed_frontend_substep<T>(_m: &'static str, _l: &str, s: impl FnOnce() -> T) -> T {\n  s()\n}",
            false,
        );

        assert!(
            failures
                .iter()
                .any(|failure| failure.contains("direct-expression macro")),
            "expected a function-wrapper failure, got: {failures:?}"
        );
    }

    #[test]
    fn accepts_direct_expression_macro_bodies() {
        let failures = audit_source_fragment(
            "src/timing.rs",
            "macro_rules! timed_frontend_stage {\n  ($stage:expr) => {{ $stage }};\n}\nmacro_rules! timed_frontend_substep {\n  ($substep:expr) => {{ $substep }};\n}",
            true,
        );

        assert!(failures.is_empty(), "unexpected failures: {failures:?}");
    }
}
