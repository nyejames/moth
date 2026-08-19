//! Recursive golden-file discovery for integration expectations.
//!
//! WHAT: owns the filesystem inventory that determines whether a backend has a golden contract.
//! WHY: fixture validation and comparison must consume the same deterministic file set.

use super::super::FailureKind;
use super::super::types::{GoldenExpectation, GoldenFile, GoldenFileInventory, GoldenMode};
use crate::build_system::build::{BuildResult, FileKind};
use crate::compiler_frontend::utilities::basic::portable_path_text;
use crate::compiler_tests::integration_test_runner::errors::FixtureLoadError;
use std::fs;
use std::path::Path;

/// Discovers one backend's golden files and resolves its effective comparison mode.
pub(crate) fn discover_golden_expectation(
    golden_dir: &Path,
    authored_mode: Option<GoldenMode>,
) -> Result<GoldenExpectation, FixtureLoadError> {
    let inventory = discover_golden_files(golden_dir)?;

    if inventory.is_empty() {
        if let Some(mode) = authored_mode {
            return Err(FixtureLoadError::fixture_contract(format!(
                "Golden directory '{}' has golden_mode = \"{}\" but contains no golden files.",
                golden_dir.display(),
                golden_mode_label(mode)
            )));
        }

        return Ok(GoldenExpectation {
            inventory,
            mode: None,
        });
    }

    Ok(GoldenExpectation {
        inventory,
        mode: Some(authored_mode.unwrap_or(GoldenMode::Strict)),
    })
}

/// Recursively inventories actual files under a backend golden directory.
///
/// WHAT: returns relative paths in deterministic order and preserves filesystem failures.
/// WHY: directories alone are not golden contracts, while unreadable golden state must not be
///      silently treated as absent. Symlink entries are rejected so a golden tree
///      cannot follow an authored link outside its owning backend or inventory the same
///      file twice.
fn discover_golden_files(root: &Path) -> Result<GoldenFileInventory, FixtureLoadError> {
    // The direct golden parent (e.g. case/golden) must not be a symlink. Without
    // this guard, a symlinked parent whose target contains the backend directory
    // would bypass the backend-root symlink rejection below and let outside files
    // be inventoried. Inspect the parent without following links before the
    // backend-directory absence handling, so the rejection still fires when the
    // symlink target lacks that backend.
    if let Some(parent) = root.parent() {
        match fs::symlink_metadata(parent) {
            Ok(parent_metadata) if parent_metadata.file_type().is_symlink() => {
                return Err(FixtureLoadError::filesystem(format!(
                    "Golden parent '{}' is a symlink. Golden trees must live inside the owning fixture.",
                    parent.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(FixtureLoadError::filesystem(format!(
                    "Failed to inspect golden parent '{}': {error}",
                    parent.display()
                )));
            }
        }
    }

    let root_metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(GoldenFileInventory::default());
        }
        Err(error) => {
            return Err(FixtureLoadError::filesystem(format!(
                "Failed to inspect golden directory '{}': {error}",
                root.display()
            )));
        }
    };

    if root_metadata.file_type().is_symlink() {
        return Err(FixtureLoadError::filesystem(format!(
            "Golden path '{}' is a symlink. Golden trees must contain only regular files and directories.",
            root.display()
        )));
    }

    if !root_metadata.is_dir() {
        return Err(FixtureLoadError::filesystem(format!(
            "Golden path '{}' exists but is not a directory.",
            root.display()
        )));
    }

    let mut files = Vec::new();
    visit_golden_directory(root, root, &mut files)?;
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    Ok(GoldenFileInventory { files })
}

fn visit_golden_directory(
    directory: &Path,
    root: &Path,
    files: &mut Vec<GoldenFile>,
) -> Result<(), FixtureLoadError> {
    let entries = fs::read_dir(directory).map_err(|error| {
        FixtureLoadError::filesystem(format!(
            "Failed to read golden directory '{}': {error}",
            directory.display()
        ))
    })?;

    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            FixtureLoadError::filesystem(format!(
                "Failed to read an entry in golden directory '{}': {error}",
                directory.display()
            ))
        })?;
        paths.push(entry.path());
    }
    paths.sort();

    for path in paths {
        let entry_metadata = fs::symlink_metadata(&path).map_err(|error| {
            FixtureLoadError::filesystem(format!(
                "Failed to inspect golden entry '{}': {error}",
                path.display()
            ))
        })?;

        if entry_metadata.file_type().is_symlink() {
            return Err(FixtureLoadError::filesystem(format!(
                "Golden directory '{}' contains a symlink entry '{}'. Golden trees must contain only regular files and directories.",
                root.display(),
                path.display()
            )));
        }

        if entry_metadata.is_dir() {
            visit_golden_directory(&path, root, files)?;
            continue;
        }

        if !entry_metadata.is_file() {
            continue;
        }

        let relative_path = path.strip_prefix(root).map_err(|error| {
            FixtureLoadError::filesystem(format!(
                "Failed to make golden entry '{}' relative to '{}': {error}",
                path.display(),
                root.display()
            ))
        })?;
        files.push(GoldenFile {
            relative_path: portable_path_text(relative_path),
            absolute_path: path,
        });
    }

    Ok(())
}

fn golden_mode_label(mode: GoldenMode) -> &'static str {
    match mode {
        GoldenMode::Strict => "strict",
        GoldenMode::Normalized => "normalized",
    }
}

pub(super) fn validate_golden_outputs(
    build_result: &BuildResult,
    golden: &GoldenExpectation,
) -> Option<(String, FailureKind)> {
    if golden.inventory.is_empty() {
        return None;
    }

    let Some(mode) = golden.mode else {
        return Some((
            "Golden files were discovered without an effective golden mode.".to_owned(),
            FailureKind::HarnessFailed,
        ));
    };

    let expected_paths = golden
        .inventory
        .files
        .iter()
        .map(|file| file.relative_path.clone())
        .collect::<Vec<_>>();

    if let Some(reason) = validate_expected_artifact_paths(build_result, &expected_paths) {
        return Some((reason, FailureKind::StrictGoldenMismatch));
    }

    for file in &golden.inventory.files {
        let relative = &file.relative_path;

        let Some(output) = super::artifacts::find_output_file(build_result, relative) else {
            return Some((
                format!("Golden output '{relative}' was not produced."),
                FailureKind::StrictGoldenMismatch,
            ));
        };

        let expected_bytes = match fs::read(&file.absolute_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                return Some((
                    format!(
                        "Failed to read golden output '{}': {error}",
                        file.absolute_path.display()
                    ),
                    FailureKind::HarnessFailed,
                ));
            }
        };

        let actual_bytes = match output.file_kind() {
            FileKind::Html(content) | FileKind::Js(content) => content.as_bytes().to_vec(),
            FileKind::Wasm(bytes) | FileKind::Bytes(bytes) => bytes.clone(),
            FileKind::Directory | FileKind::NotBuilt => Vec::new(),
        };

        // Text artifacts support normalized comparison; binary/wasm always use strict.
        let is_text = matches!(output.file_kind(), FileKind::Html(_) | FileKind::Js(_));
        if is_text {
            let expected_str = String::from_utf8_lossy(&expected_bytes);
            let actual_str = match output.file_kind() {
                FileKind::Html(s) | FileKind::Js(s) => s.as_str(),
                _ => unreachable!("is_text is true"),
            };

            if let Some(detail) = compare_text_golden(expected_str.as_ref(), actual_str, mode) {
                let failure_kind = if mode == GoldenMode::Normalized {
                    FailureKind::NormalizedSemanticMismatch
                } else {
                    FailureKind::StrictGoldenMismatch
                };
                let context = if mode == GoldenMode::Normalized {
                    "did not match after normalization"
                } else {
                    "did not match the produced artifact"
                };
                return Some((
                    format!("Golden output '{relative}' {context}.\n{detail}"),
                    failure_kind,
                ));
            }
            continue;
        }

        if actual_bytes != expected_bytes {
            let detail = format!(
                "expected {} bytes, got {} bytes",
                expected_bytes.len(),
                actual_bytes.len()
            );
            return Some((
                format!(
                    "Golden output '{relative}' did not match the produced artifact ({detail})."
                ),
                FailureKind::StrictGoldenMismatch,
            ));
        }
    }

    None
}

fn validate_expected_artifact_paths(
    build_result: &BuildResult,
    expected_paths: &[String],
) -> Option<String> {
    let actual_paths = super::artifacts::collect_built_artifact_paths(build_result);

    let mut expected = expected_paths
        .iter()
        .map(portable_path_text)
        .collect::<Vec<_>>();
    expected.sort();

    if actual_paths != expected {
        return Some(format!(
            "Expected output paths {expected:?}, but produced {actual_paths:?}."
        ));
    }

    None
}

pub(super) fn compare_text_golden(
    expected: &str,
    actual: &str,
    mode: GoldenMode,
) -> Option<String> {
    let normalized_expected = normalize_text_line_endings(expected);
    let normalized_actual = normalize_text_line_endings(actual);

    match mode {
        GoldenMode::Strict => {
            if normalized_expected == normalized_actual {
                return None;
            }
            Some(generate_text_diff(
                &normalized_expected,
                &normalized_actual,
                8,
            ))
        }
        GoldenMode::Normalized => {
            let semantic_expected = normalize_text_for_comparison(&normalized_expected);
            let semantic_actual = normalize_text_for_comparison(&normalized_actual);
            if semantic_expected == semantic_actual {
                return None;
            }
            Some(generate_text_diff(&semantic_expected, &semantic_actual, 8))
        }
    }
}

fn generate_text_diff(expected: &str, actual: &str, max_pairs: usize) -> String {
    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();
    let max_len = exp_lines.len().max(act_lines.len());

    let mut diff_lines: Vec<String> = Vec::new();
    let mut extra = 0usize;

    for i in 0..max_len {
        let e = exp_lines.get(i).copied();
        let a = act_lines.get(i).copied();
        if e == a {
            continue;
        }
        if diff_lines.len() >= max_pairs * 2 {
            extra += 1;
            continue;
        }
        if let Some(line) = e {
            diff_lines.push(format!("- {line}"));
        }
        if let Some(line) = a {
            diff_lines.push(format!("+ {line}"));
        }
    }

    let mut out = format!("--- expected\n+++ actual\n{}", diff_lines.join("\n"));
    if extra > 0 {
        out.push_str(&format!("\n... ({extra} more differing lines)"));
    }
    out
}

/// Normalizes compiler-generated counter suffixes in JS/HTML text for comparison.
///
/// WHAT: replaces unstable numeric counters in `moth_`-prefixed identifiers with the placeholder
///       `N` while preserving line endings and the embedded core-CSS contract.
/// WHY: generated names can vary between compilations even when emitted structure is equivalent.
pub(super) fn normalize_text_for_comparison(text: &str) -> String {
    let line_normalized = normalize_text_line_endings(text);
    let text = strip_embedded_css(&line_normalized);
    let mut result = String::with_capacity(text.len());
    let mut token_start: Option<usize> = None;

    for (index, character) in text.char_indices() {
        let in_identifier = character.is_ascii_alphanumeric() || character == '_';
        match (token_start, in_identifier) {
            (None, true) => token_start = Some(index),
            (Some(start), false) => {
                result.push_str(&normalize_moth_identifier(&text[start..index]));
                result.push(character);
                token_start = None;
            }
            (Some(_), true) => {}
            (None, false) => result.push(character),
        }
    }
    if let Some(start) = token_start {
        result.push_str(&normalize_moth_identifier(&text[start..]));
    }
    result
}

fn normalize_text_line_endings(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(character) = chars.next() {
        if character == '\r' {
            if matches!(chars.peek(), Some('\n')) {
                chars.next();
            }
            normalized.push('\n');
            continue;
        }

        normalized.push(character);
    }

    normalized
}

/// Strips the embedded core CSS block so golden files stay stable when the core stylesheet changes.
fn strip_embedded_css(text: &str) -> String {
    const STYLE_OPEN: &str = "<style>";
    const STYLE_CLOSE: &str = "</style>";

    let Some(start) = text.find(STYLE_OPEN) else {
        return text.to_owned();
    };
    let after_open = start + STYLE_OPEN.len();
    let Some(close_start) = text[after_open..].find(STYLE_CLOSE) else {
        return text.to_owned();
    };
    let close_end = after_open + close_start + STYLE_CLOSE.len();

    let mut result = String::with_capacity(text.len() - (close_end - start) + 28);
    result.push_str(&text[..start]);
    result.push_str("<style>/* CORE_CSS */</style>");
    result.push_str(&text[close_end..]);
    result
}

fn normalize_moth_identifier(token: &str) -> String {
    if !token.starts_with("moth_") {
        return token.to_owned();
    }

    let parts: Vec<&str> = token.split('_').collect();
    let mut result: Vec<String> = Vec::with_capacity(parts.len());

    for (index, &part) in parts.iter().enumerate() {
        let previous = if index > 0 { parts[index - 1] } else { "" };

        let is_pure_digit =
            !part.is_empty() && part.chars().all(|character| character.is_ascii_digit());
        let previous_is_trigger = matches!(previous, "fn" | "tmp" | "frag");
        if is_pure_digit && previous_is_trigger {
            result.push("N".to_owned());
            continue;
        }

        if let Some(normalized) = normalize_counter_suffix(part) {
            result.push(normalized);
            continue;
        }

        result.push(part.to_owned());
    }

    result.join("_")
}

fn normalize_counter_suffix(segment: &str) -> Option<String> {
    let digit_start = segment
        .char_indices()
        .rev()
        .take_while(|(_, character)| character.is_ascii_digit())
        .last()
        .map(|(index, _)| index);

    let digit_start = digit_start?;

    if digit_start == 0 {
        return None;
    }

    let prefix = &segment[..digit_start];
    if matches!(prefix, "fn" | "l") {
        Some(format!("{prefix}N"))
    } else {
        None
    }
}
