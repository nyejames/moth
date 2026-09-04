//! Fixture discovery and loading for the integration test suite.
//!
//! WHAT: locates canonical case directories, validates the manifest, and builds typed
//!       `TestCaseSpec` values ready for execution.
//! WHY: keeping fixture loading separate from expectation parsing and case execution gives
//!      each piece a single clear responsibility.

use super::path_validation::{CurrentDirectoryRule, validate_relative_path};
use super::types::GoldenExpectation;
use super::types::SuccessContract;
use super::{
    BackendId, CANONICAL_TESTS_PATH, DiagnosticMatchMode, EXPECT_FILE_NAME, ExpectationMode,
    ExpectedOutcome, FailureExpectation, GOLDEN_DIR_NAME, INPUT_DIR_NAME, MANIFEST_FILE_NAME,
    ManifestCaseSpec, ParsedExpectationFile, SuccessExpectation, TestCaseSpec, TestSuiteSpec,
    WarningExpectation,
};
use crate::compiler_frontend::Flag;
use crate::compiler_frontend::utilities::basic::portable_path_text;
use crate::compiler_tests::integration_test_runner::errors::FixtureLoadError;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn load_test_suite() -> Result<TestSuiteSpec, FixtureLoadError> {
    load_test_suite_from_root(Path::new(CANONICAL_TESTS_PATH))
}

pub(crate) fn load_test_suite_from_root(root: &Path) -> Result<TestSuiteSpec, FixtureLoadError> {
    let canonical_suite_root = fs::canonicalize(root).map_err(|error| {
        FixtureLoadError::filesystem(format!(
            "Failed to resolve canonical integration test root '{}': {error}",
            root.display()
        ))
    })?;
    let mut cases = Vec::new();
    let manifest_path = canonical_suite_root.join(MANIFEST_FILE_NAME);
    // Use symlink_metadata to distinguish absence from IO errors so metadata
    // failures do not masquerade as a missing manifest.
    match std::fs::symlink_metadata(&manifest_path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(FixtureLoadError::manifest(format!(
                "Canonical integration root '{}' has '{}' but it is not a regular file.",
                canonical_suite_root.display(),
                MANIFEST_FILE_NAME
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(FixtureLoadError::manifest(format!(
                "Canonical integration root '{}' must define '{}'.",
                canonical_suite_root.display(),
                MANIFEST_FILE_NAME
            )));
        }
        Err(error) => {
            return Err(FixtureLoadError::manifest(format!(
                "Failed to read metadata for manifest '{}': {error}",
                manifest_path.display()
            )));
        }
    }

    let manifest_cases = super::manifest::parse_manifest_file(&manifest_path)?;
    let canonical_fixture_roots = manifest_cases
        .iter()
        .map(|manifest_case| {
            resolve_declared_fixture_root(&canonical_suite_root, &manifest_path, manifest_case)
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_canonical_fixture_root_uniqueness(
        &manifest_path,
        &manifest_cases,
        &canonical_fixture_roots,
    )?;
    validate_manifest_authoritativeness(&canonical_suite_root, &canonical_fixture_roots)?;

    for (manifest_case, fixture_root) in manifest_cases.into_iter().zip(canonical_fixture_roots) {
        let case_specs = load_canonical_case_specs_at(&fixture_root, Some(manifest_case))?;
        cases.extend(case_specs);
    }

    Ok(TestSuiteSpec { cases })
}

fn validate_canonical_fixture_root_uniqueness(
    manifest_path: &Path,
    manifest_cases: &[ManifestCaseSpec],
    canonical_fixture_roots: &[PathBuf],
) -> Result<(), FixtureLoadError> {
    let mut seen_roots: HashMap<&Path, &ManifestCaseSpec> = HashMap::new();
    for (manifest_case, canonical_root) in manifest_cases.iter().zip(canonical_fixture_roots) {
        if let Some(existing_case) = seen_roots.get(canonical_root.as_path()) {
            return Err(FixtureLoadError::manifest(format!(
                "Manifest '{}' has a duplicate canonical fixture root: case '{}' path '{}' and case '{}' path '{}' both resolve to '{}'. Each fixture must have a unique canonical path.",
                manifest_path.display(),
                existing_case.id,
                existing_case.path.display(),
                manifest_case.id,
                manifest_case.path.display(),
                canonical_root.display()
            )));
        }
        seen_roots.insert(canonical_root.as_path(), manifest_case);
    }

    Ok(())
}

fn validate_manifest_authoritativeness(
    canonical_suite_root: &Path,
    canonical_fixture_roots: &[PathBuf],
) -> Result<(), FixtureLoadError> {
    let declared_paths = canonical_fixture_roots
        .iter()
        .cloned()
        .collect::<HashSet<_>>();

    let discovered_roots = discover_canonical_fixture_roots(canonical_suite_root)?;
    let mut undeclared_fixtures = Vec::new();
    for discovered_root in discovered_roots {
        let canonical_discovered = fs::canonicalize(&discovered_root).map_err(|error| {
            FixtureLoadError::manifest(format!(
                "Failed to resolve discovered canonical fixture '{}': {error}",
                discovered_root.display()
            ))
        })?;
        ensure_strictly_inside(
            &canonical_discovered,
            canonical_suite_root,
            &format!("discovered fixture '{}'", discovered_root.display()),
        )?;
        if !declared_paths.contains(&canonical_discovered) {
            undeclared_fixtures.push(discovered_root);
        }
    }

    if !undeclared_fixtures.is_empty() {
        undeclared_fixtures.sort();
        let preview = undeclared_fixtures
            .iter()
            .take(6)
            .map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("unknown_case")
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(FixtureLoadError::manifest(format!(
            "Manifest '{}' must list every canonical case; found undeclared fixtures: {preview}.",
            canonical_suite_root.join(MANIFEST_FILE_NAME).display()
        )));
    }

    Ok(())
}

fn resolve_declared_fixture_root(
    canonical_suite_root: &Path,
    manifest_path: &Path,
    manifest_case: &ManifestCaseSpec,
) -> Result<PathBuf, FixtureLoadError> {
    let declared_path = canonical_suite_root.join(&manifest_case.path);
    let canonical_fixture_root = fs::canonicalize(&declared_path).map_err(|error| {
        FixtureLoadError::filesystem(format!(
            "Manifest '{}' case '{}' path '{}' could not be resolved: {error}.",
            manifest_path.display(),
            manifest_case.id,
            manifest_case.path.display()
        ))
    })?;
    ensure_strictly_inside(
        &canonical_fixture_root,
        canonical_suite_root,
        &format!(
            "manifest case '{}' path '{}'",
            manifest_case.id,
            manifest_case.path.display()
        ),
    )?;
    Ok(canonical_fixture_root)
}

fn discover_canonical_fixture_roots(root: &Path) -> Result<Vec<PathBuf>, FixtureLoadError> {
    let entries = fs::read_dir(root).map_err(|error| {
        FixtureLoadError::filesystem(format!(
            "Failed to read canonical test root '{}': {error}",
            root.display()
        ))
    })?;

    let mut discovered_dirs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            FixtureLoadError::filesystem(format!("Failed to read test entry: {error}"))
        })?;
        let path = entry.path();

        // Use symlink_metadata to distinguish NotFound (a legitimate skip) from
        // other IO errors (which must surface as failures, not silent skips).
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(FixtureLoadError::filesystem(format!(
                    "Failed to read metadata for fixture entry '{}': {error}",
                    path.display()
                )));
            }
        };
        if !metadata.is_dir() {
            continue;
        }

        // Non-UTF-8 fixture identities cannot be declared in the manifest, so they
        // are a fixture-discovery error, not a silent skip.
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return Err(FixtureLoadError::filesystem(format!(
                "Fixture directory '{}' has a non-UTF-8 name; fixture identities must be UTF-8",
                path.display()
            )));
        };

        if matches!(name, "success" | "failure") {
            continue;
        }

        // Check for the input directory using the same metadata-based approach.
        let input_dir = path.join(INPUT_DIR_NAME);
        let input_metadata = match std::fs::symlink_metadata(&input_dir) {
            Ok(m) => m,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(FixtureLoadError::filesystem(format!(
                    "Failed to read metadata for input dir '{}': {error}",
                    input_dir.display()
                )));
            }
        };
        if !input_metadata.is_dir() {
            continue;
        }

        discovered_dirs.push(path);
    }

    discovered_dirs.sort();
    Ok(discovered_dirs)
}

#[cfg(test)]
pub(crate) fn load_canonical_case_specs(
    fixture_root: &Path,
    manifest_case: Option<ManifestCaseSpec>,
) -> Result<Vec<TestCaseSpec>, FixtureLoadError> {
    let canonical_fixture_root = fs::canonicalize(fixture_root).map_err(|error| {
        FixtureLoadError::filesystem(format!(
            "Failed to resolve canonical fixture '{}': {error}",
            fixture_root.display()
        ))
    })?;
    load_canonical_case_specs_at(&canonical_fixture_root, manifest_case)
}

fn load_canonical_case_specs_at(
    fixture_root: &Path,
    manifest_case: Option<ManifestCaseSpec>,
) -> Result<Vec<TestCaseSpec>, FixtureLoadError> {
    let input_path = fixture_root.join(INPUT_DIR_NAME);
    let input_root = fs::canonicalize(&input_path).map_err(|error| {
        FixtureLoadError::filesystem(format!(
            "Canonical fixture '{}' could not resolve '{}': {error}",
            fixture_root.display(),
            INPUT_DIR_NAME
        ))
    })?;
    ensure_strictly_inside(
        &input_root,
        fixture_root,
        &format!("fixture '{}' input directory", fixture_root.display()),
    )?;
    if !input_root.is_dir() {
        return Err(FixtureLoadError::filesystem(format!(
            "Canonical fixture '{}' is missing '{}', or it is not a directory",
            fixture_root.display(),
            INPUT_DIR_NAME
        )));
    }

    let expect_path = fixture_root.join(EXPECT_FILE_NAME);

    // Use symlink_metadata to distinguish absence from IO errors.
    match std::fs::symlink_metadata(&expect_path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            let case_name = manifest_case
                .as_ref()
                .map(|case| case.id.as_str())
                .or_else(|| fixture_root.file_name().and_then(|name| name.to_str()))
                .unwrap_or("unnamed_case");
            return Err(FixtureLoadError::filesystem(format!(
                "Canonical case '{}' at fixture '{}' has '{}' but it is not a regular file.",
                case_name,
                portable_path_text(fixture_root),
                EXPECT_FILE_NAME
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let case_name = manifest_case
                .as_ref()
                .map(|case| case.id.as_str())
                .or_else(|| fixture_root.file_name().and_then(|name| name.to_str()))
                .unwrap_or("unnamed_case");
            return Err(FixtureLoadError::filesystem(format!(
                "Canonical case '{}' at fixture '{}' is missing required expectation file '{}'.",
                case_name,
                portable_path_text(fixture_root),
                portable_path_text(&expect_path)
            )));
        }
        Err(error) => {
            return Err(FixtureLoadError::filesystem(format!(
                "Failed to read metadata for expectation file '{}': {error}",
                expect_path.display()
            )));
        }
    }

    let parsed_expectation = super::expectations::parse_expectation_file(&expect_path)?;
    let golden_expectations = parsed_expectation
        .backend_expectations
        .iter()
        .map(|backend_expectation| {
            let golden_dir = golden_dir_for_backend(fixture_root, backend_expectation.backend_id);
            super::assertions::discover_golden_expectation(
                &golden_dir,
                backend_expectation.golden_mode,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_fixture_contract(fixture_root, &parsed_expectation, &golden_expectations)?;
    let entry_path = resolve_case_entry_path(
        fixture_root,
        &input_root,
        parsed_expectation.entry.as_deref(),
    )?;
    let manifest_relative_path = manifest_case
        .as_ref()
        .map(|case| portable_path_text(&case.path))
        .unwrap_or_else(|| {
            fixture_root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unnamed_case")
                .to_owned()
        });
    let (case_id, tags, contract, role) = match manifest_case {
        Some(manifest_case) => (
            manifest_case.id,
            manifest_case.tags,
            manifest_case.contract,
            manifest_case.role,
        ),
        None => (
            fixture_root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unnamed_case")
                .to_string(),
            Vec::new(),
            None,
            None,
        ),
    };

    let mut case_specs = Vec::new();
    for (backend_expectation, golden) in parsed_expectation
        .backend_expectations
        .into_iter()
        .zip(golden_expectations)
    {
        let expected = match backend_expectation.mode {
            ExpectationMode::Success => ExpectedOutcome::Success(SuccessExpectation {
                warnings: backend_expectation.warnings,
                success_contract: backend_expectation.success_contract,
                artifact_assertions: backend_expectation.artifact_assertions,
                golden,
                rendered_output: backend_expectation.rendered_output,
                artifacts_must_not_exist: backend_expectation.artifacts_must_not_exist,
            }),
            ExpectationMode::Failure => ExpectedOutcome::Failure(FailureExpectation {
                warnings: backend_expectation.warnings,
                message_contains: backend_expectation.message_contains,
                diagnostic_codes: backend_expectation.diagnostic_codes,
                diagnostic_assertions: backend_expectation.diagnostic_assertions,
                diagnostic_match: backend_expectation
                    .diagnostic_match
                    .unwrap_or(DiagnosticMatchMode::Exact),
                diagnostic_match_reason: backend_expectation.diagnostic_match_reason,
            }),
        };

        let flags = merge_flags(
            backend_expectation.backend_id.default_flags(),
            backend_expectation.flags,
        );
        let backend_name = backend_expectation.backend_id.as_str();

        case_specs.push(TestCaseSpec {
            display_name: format!("{case_id} [{backend_name}]"),
            case_id: case_id.clone(),
            manifest_relative_path: manifest_relative_path.clone(),
            fixture_root: fixture_root.to_path_buf(),
            tags: tags.clone(),
            contract: contract.clone(),
            role,
            backend_id: backend_expectation.backend_id,
            entry_path: entry_path.clone(),
            flags,
            expected,
        });
    }

    Ok(case_specs)
}

fn merge_flags(default_flags: Vec<Flag>, extra_flags: Vec<Flag>) -> Vec<Flag> {
    // Default backend flags establish the runtime mode, while fixture flags
    // can layer additional toggles without duplicating the same flag value.
    let mut merged = default_flags;
    for flag in extra_flags {
        if !merged.contains(&flag) {
            merged.push(flag);
        }
    }

    merged
}

fn validate_fixture_contract(
    fixture_root: &Path,
    expectation: &ParsedExpectationFile,
    golden_expectations: &[GoldenExpectation],
) -> Result<(), FixtureLoadError> {
    if expectation.backend_expectations.is_empty() {
        return Err(FixtureLoadError::fixture_contract(format!(
            "Fixture '{}' does not define any backend expectations.",
            fixture_root.display()
        )));
    }

    for (backend_expectation, golden) in expectation
        .backend_expectations
        .iter()
        .zip(golden_expectations)
    {
        let has_golden_files = golden.is_present();

        match backend_expectation.mode {
            ExpectationMode::Failure => {
                if backend_expectation.diagnostic_codes.is_empty() {
                    return Err(FixtureLoadError::fixture_contract(format!(
                        "Fixture '{}' backend '{}' uses mode = \"failure\" but is missing required 'diagnostic_codes'.",
                        fixture_root.display(),
                        backend_expectation.backend_id.as_str()
                    )));
                }
                if !backend_expectation.artifact_assertions.is_empty() {
                    return Err(FixtureLoadError::fixture_contract(format!(
                        "Fixture '{}' backend '{}' uses mode = \"failure\" and must not define artifact assertions.",
                        fixture_root.display(),
                        backend_expectation.backend_id.as_str()
                    )));
                }
                // Failure backends never produce artifacts, so an authored golden_mode or any
                // discovered file-backed golden is invalid. Reject before constructing
                // ExpectedOutcome so the audit inventory cannot silently report the golden as
                // absent while golden files linger on disk.
                if backend_expectation.golden_mode.is_some() {
                    return Err(FixtureLoadError::fixture_contract(format!(
                        "Fixture '{}' backend '{}' uses mode = \"failure\" and must not author 'golden_mode'.",
                        fixture_root.display(),
                        backend_expectation.backend_id.as_str()
                    )));
                }
                if has_golden_files {
                    return Err(FixtureLoadError::fixture_contract(format!(
                        "Fixture '{}' backend '{}' uses mode = \"failure\" but has golden artifacts in '{}'.",
                        fixture_root.display(),
                        backend_expectation.backend_id.as_str(),
                        golden_dir_for_backend(fixture_root, backend_expectation.backend_id)
                            .display()
                    )));
                }
            }
            ExpectationMode::Success => {
                if backend_expectation.success_contract == Some(SuccessContract::AcceptanceOnly)
                    && has_golden_files
                {
                    return Err(FixtureLoadError::fixture_contract(format!(
                        "Fixture '{}' backend '{}' declares success_contract = \"acceptance_only\" but has golden artifacts in '{}'.",
                        fixture_root.display(),
                        backend_expectation.backend_id.as_str(),
                        golden_dir_for_backend(fixture_root, backend_expectation.backend_id)
                            .display()
                    )));
                }

                if !has_authored_success_contract(backend_expectation, golden) {
                    return Err(FixtureLoadError::fixture_contract(format!(
                        "Fixture '{}' backend '{}' uses mode = \"success\" and must author at least one accepted success contract: \
                         success_contract = \"acceptance_only\", artifact assertions, a non-empty '{}' directory, \
                         rendered-output assertions, artifact-absence assertions, or warnings = \"exact\" with warning_codes.",
                        fixture_root.display(),
                        backend_expectation.backend_id.as_str(),
                        golden_dir_for_backend(fixture_root, backend_expectation.backend_id)
                            .display()
                    )));
                }
                if !backend_expectation.message_contains.is_empty()
                    || !backend_expectation.diagnostic_codes.is_empty()
                    || !backend_expectation.diagnostic_assertions.is_empty()
                    || backend_expectation.diagnostic_match.is_some()
                    || backend_expectation.diagnostic_match_reason.is_some()
                {
                    return Err(FixtureLoadError::fixture_contract(format!(
                        "Fixture '{}' backend '{}' uses mode = \"success\" and must not set failure-only keys ('diagnostic_codes'/'diagnostic_assertions'/'message_contains'/'diagnostic_match'/'diagnostic_match_reason').",
                        fixture_root.display(),
                        backend_expectation.backend_id.as_str()
                    )));
                }
            }
        }
    }

    Ok(())
}

fn has_authored_success_contract(
    backend_expectation: &super::ParsedBackendExpectation,
    golden: &GoldenExpectation,
) -> bool {
    backend_expectation.success_contract == Some(SuccessContract::AcceptanceOnly)
        || !backend_expectation.artifact_assertions.is_empty()
        || golden.is_present()
        || backend_expectation.rendered_output.is_present()
        || !backend_expectation.artifacts_must_not_exist.is_empty()
        || matches!(&backend_expectation.warnings, WarningExpectation::Exact(_))
}

fn resolve_case_entry_path(
    fixture_root: &Path,
    input_root: &Path,
    configured_entry: Option<&str>,
) -> Result<PathBuf, FixtureLoadError> {
    if let Some(entry) = configured_entry {
        validate_relative_path(
            entry,
            "Configured entry",
            CurrentDirectoryRule::AllowExactSentinel,
        )
        .map_err(|error| {
            FixtureLoadError::path_boundary(format!(
                "Fixture '{}' has an invalid entry '{}': {error}.",
                fixture_root.display(),
                entry
            ))
        })?;

        if entry == "." {
            return Ok(input_root.to_path_buf());
        }

        return canonicalize_contained_entry(fixture_root, input_root, entry);
    }

    let default_entry = input_root.join("@page.moth");
    if default_entry.is_file() {
        return canonicalize_contained_entry(fixture_root, input_root, "@page.moth");
    }

    if input_root.join("config.moth").is_file() {
        return Ok(input_root.to_path_buf());
    }

    Err(FixtureLoadError::path_boundary(format!(
        "Could not determine canonical test entry for '{}'. Add 'entry = ...' to '{}' or provide @page.moth.",
        input_root.display(),
        EXPECT_FILE_NAME
    )))
}

fn canonicalize_contained_entry(
    fixture_root: &Path,
    input_root: &Path,
    authored_entry: &str,
) -> Result<PathBuf, FixtureLoadError> {
    let entry_path = input_root.join(authored_entry);
    let canonical_entry = fs::canonicalize(&entry_path).map_err(|error| {
        FixtureLoadError::path_boundary(format!(
            "Fixture '{}' entry '{}' could not be resolved: {error}.",
            fixture_root.display(),
            authored_entry
        ))
    })?;
    ensure_strictly_inside(
        &canonical_entry,
        input_root,
        &format!(
            "fixture '{}' entry '{}'",
            fixture_root.display(),
            authored_entry
        ),
    )?;
    Ok(canonical_entry)
}

fn ensure_strictly_inside(path: &Path, root: &Path, context: &str) -> Result<(), FixtureLoadError> {
    let is_strictly_inside = path
        .strip_prefix(root)
        .is_ok_and(|relative| !relative.as_os_str().is_empty());
    if !is_strictly_inside {
        return Err(FixtureLoadError::path_boundary(format!(
            "{context} resolves to '{}' outside the required root '{}'.",
            path.display(),
            root.display()
        )));
    }
    Ok(())
}

/// Resolves backend-scoped golden directories for fixture assertions.
///
/// WHAT: maps each backend execution to `golden/<backend>/...`.
/// WHY: keeps artifact snapshots backend-specific even for non-matrix fixtures.
pub(crate) fn golden_dir_for_backend(fixture_root: &Path, backend_id: BackendId) -> PathBuf {
    fixture_root.join(GOLDEN_DIR_NAME).join(backend_id.as_str())
}
