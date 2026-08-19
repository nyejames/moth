//! Manifest file parsing for the integration test suite.
//!
//! WHAT: reads and validates `manifest.toml`, which declares canonical case order and metadata.
//! WHY: the manifest is the authoritative source for test execution order, so parsing it is
//!      isolated here to keep the fixture loader free of TOML deserialization details.

use super::path_validation::{CurrentDirectoryRule, validate_relative_path};
use super::{CaseRole, ManifestCaseSpec};
use crate::compiler_tests::integration_test_runner::errors::FixtureLoadError;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestToml {
    #[serde(default)]
    pub case: Vec<ManifestCaseToml>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestCaseToml {
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub contract: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
}

pub(crate) fn parse_manifest_file(path: &Path) -> Result<Vec<ManifestCaseSpec>, FixtureLoadError> {
    let source = fs::read_to_string(path).map_err(|error| {
        FixtureLoadError::manifest(format!(
            "Failed to read manifest '{}': {error}",
            path.display()
        ))
    })?;

    let parsed: ManifestToml = toml::from_str(&source).map_err(|error| {
        FixtureLoadError::manifest(format!(
            "Failed to parse manifest '{}' as TOML: {error}",
            path.display()
        ))
    })?;

    let mut seen_ids = HashSet::new();
    let mut seen_paths = HashSet::new();
    let mut cases = Vec::with_capacity(parsed.case.len());
    for case in parsed.case {
        if case.id.trim().is_empty() {
            return Err(FixtureLoadError::manifest(format!(
                "Manifest '{}' has a case with an empty id",
                path.display()
            )));
        }
        if case.id != case.id.trim() {
            return Err(FixtureLoadError::manifest(format!(
                "Manifest '{}' case id '{}' must not have leading or trailing whitespace.",
                path.display(),
                case.id
            )));
        }
        if case.path.trim().is_empty() {
            return Err(FixtureLoadError::manifest(format!(
                "Manifest '{}' has a case with an empty path",
                path.display()
            )));
        }
        validate_relative_path(
            &case.path,
            "Manifest case path",
            CurrentDirectoryRule::Forbid,
        )
        .map_err(|error| {
            FixtureLoadError::manifest(format!(
                "Manifest '{}' case '{}' has an invalid path: {error}.",
                path.display(),
                case.id
            ))
        })?;
        if case.tags.is_empty() {
            return Err(FixtureLoadError::manifest(format!(
                "Manifest '{}' case '{}' is missing required tags.",
                path.display(),
                case.id
            )));
        }
        let mut seen_tags = HashSet::new();
        for tag in &case.tags {
            if tag.trim().is_empty() {
                return Err(FixtureLoadError::manifest(format!(
                    "Manifest '{}' case '{}' has an empty tag value.",
                    path.display(),
                    case.id
                )));
            }
            if tag != tag.trim() {
                return Err(FixtureLoadError::manifest(format!(
                    "Manifest '{}' case '{}' tag '{}' must not have leading or trailing whitespace.",
                    path.display(),
                    case.id,
                    tag
                )));
            }
            if !seen_tags.insert(tag) {
                return Err(FixtureLoadError::manifest(format!(
                    "Manifest '{}' case '{}' has duplicate tag '{}'.",
                    path.display(),
                    case.id,
                    tag
                )));
            }
        }

        if let Some(contract) = case.contract.as_deref() {
            if contract.trim().is_empty() {
                return Err(FixtureLoadError::manifest(format!(
                    "Manifest '{}' case '{}' has an empty contract value.",
                    path.display(),
                    case.id
                )));
            }
            if contract != contract.trim() {
                return Err(FixtureLoadError::manifest(format!(
                    "Manifest '{}' case '{}' contract '{}' must not have leading or trailing whitespace.",
                    path.display(),
                    case.id,
                    contract
                )));
            }
        }

        if let Some(role) = case.role.as_deref()
            && role != role.trim()
        {
            return Err(FixtureLoadError::manifest(format!(
                "Manifest '{}' case '{}' role '{}' must not have leading or trailing whitespace.",
                path.display(),
                case.id,
                role
            )));
        }

        let role = case
            .role
            .as_deref()
            .map(CaseRole::parse)
            .transpose()
            .map_err(|error| {
                FixtureLoadError::manifest(format!(
                    "Manifest '{}' case '{}' has an invalid role: {error}.",
                    path.display(),
                    case.id
                ))
            })?;

        if !seen_ids.insert(case.id.clone()) {
            return Err(FixtureLoadError::manifest(format!(
                "Manifest '{}' has duplicate case id '{}'.",
                path.display(),
                case.id
            )));
        }
        if !seen_paths.insert(case.path.clone()) {
            return Err(FixtureLoadError::manifest(format!(
                "Manifest '{}' has duplicate case path '{}'.",
                path.display(),
                case.path
            )));
        }

        cases.push(ManifestCaseSpec {
            id: case.id,
            path: PathBuf::from(case.path),
            tags: case.tags,
            contract: case.contract,
            role,
        });
    }

    Ok(cases)
}
