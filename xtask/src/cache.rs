//! Conservative post-validation housekeeping for Cargo's selected target directory.
//!
//! Cargo owns build artifacts and incremental state. This command only prunes old HTML timing
//! reports, then reports a soft size budget. Package eviction is an explicit `cargo clean`
//! recipe, outside this process, so Windows never has to delete a running xtask executable.

use crate::source_tree::{WalkDecision, walk_source_tree, workspace_root};
use serde::Deserialize;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

const GIB: u64 = 1024 * 1024 * 1024;
const REPORTS_TO_KEEP: usize = 10;
const MINIMUM_REPORT_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Deserialize)]
struct TargetMetadata {
    target_directory: PathBuf,
}

pub(crate) fn run_cache_maintenance() -> Result<(), String> {
    let budget = match env::var("MOTH_CACHE_BUDGET_GIB") {
        Ok(value) => budget_bytes(&value)?,
        Err(env::VarError::NotPresent) => 32 * GIB,
        Err(error) => return Err(format!("invalid MOTH_CACHE_BUDGET_GIB: {error}")),
    };

    let workspace = workspace_root()?;
    let output = Command::new("cargo")
        .current_dir(&workspace)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output()
        .map_err(|error| format!("failed to query Cargo's target directory: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let metadata: TargetMetadata = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid Cargo target metadata: {error}"))?;
    let target = metadata.target_directory;

    if !target.is_absolute() || target.parent().is_none() {
        return Err(format!("refusing target directory '{}'", target.display()));
    }

    // Check the target itself before opening its timing-report child. A redirected root must
    // not turn report retention into deletion through a symlink or Windows junction.
    if !plain_directory(&target)? {
        return Ok(());
    }

    let removed = prune_timing_reports(&target.join("cargo-timings"), SystemTime::now())?;
    let bytes = logical_target_bytes(&target)?;
    println!(
        "cache-maintain: {} MiB under '{}' (logical bytes, hard links counted per path), \
         removed {removed} old timing report(s); build artifacts retained",
        bytes / (1024 * 1024),
        target.display()
    );

    if bytes > budget {
        println!(
            "cache-maintain: over the {} GiB soft budget. With other builds stopped, review \
             `just cache-evict-preview`, then use `just cache-evict` to discard workspace \
             dev/test artifacts. This costs a rebuild and is never automatic.",
            budget / GIB
        );
    }

    Ok(())
}

fn budget_bytes(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .and_then(|value| value.checked_mul(GIB))
        .ok_or_else(|| "MOTH_CACHE_BUDGET_GIB must be a positive, representable integer".to_owned())
}

fn prune_timing_reports(directory: &Path, now: SystemTime) -> Result<usize, String> {
    if !plain_directory(directory)? {
        return Ok(0);
    }

    let mut reports = Vec::new();
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("failed to read '{}': {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to read timing entry: {error}"))?;
        let name = entry.file_name();
        let Some(stamp) = name
            .to_str()
            .and_then(|name| name.strip_prefix("cargo-timing-"))
            .and_then(|name| name.strip_suffix(".html"))
        else {
            continue;
        };
        if stamp.is_empty()
            || !stamp
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'T' | b'Z' | b'-' | b'.'))
        {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to stat '{}': {error}", path.display()))?;
        if !metadata.is_file() || is_redirect(&metadata) {
            continue;
        }
        let modified = metadata
            .modified()
            .map_err(|error| format!("failed to date '{}': {error}", path.display()))?;
        reports.push((modified, path));
    }

    // Retain at least ten reports and everything from the last day. The grace period preserves
    // recent investigation evidence. Run maintenance with other Cargo invocations stopped.
    reports.sort_unstable();
    let mut removed = 0;
    for (modified, path) in reports.into_iter().rev().skip(REPORTS_TO_KEEP) {
        if now
            .duration_since(modified)
            .is_ok_and(|age| age >= MINIMUM_REPORT_AGE)
        {
            fs::remove_file(&path)
                .map_err(|error| format!("failed to remove '{}': {error}", path.display()))?;
            removed += 1;
        }
    }

    Ok(removed)
}

fn logical_target_bytes(target: &Path) -> Result<u64, String> {
    if !plain_directory(target)? {
        return Ok(0);
    }

    let mut bytes = 0_u64;
    walk_source_tree(target, |_, metadata| {
        if is_redirect(metadata) {
            return Ok(WalkDecision::SkipDescendants);
        }

        if metadata.is_file() {
            bytes = bytes
                .checked_add(metadata.len())
                .ok_or_else(|| "target byte count overflowed".to_owned())?;
        }
        Ok(WalkDecision::Continue)
    })?;

    Ok(bytes)
}

fn plain_directory(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !is_redirect(&metadata) => Ok(true),
        Ok(_) => Err(format!(
            "expected an unredirected directory at '{}'",
            path.display()
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("failed to stat '{}': {error}", path.display())),
    }
}

fn is_redirect(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT also covers junctions, not only symbolic links.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

#[cfg(test)]
mod tests;
