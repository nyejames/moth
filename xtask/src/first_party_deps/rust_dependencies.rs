//! Cargo-owned package identities and dependency edges for the extracted library boundary.
//!
//! All dependency kinds and targets are inspected, including optional and renamed dependencies.
//! Workspace membership defines first-party packages, but the compiler and `xtask` are forbidden
//! regardless of membership. Cargo's canonical dependency name, not its import alias, controls the
//! policy.
//! This does not resolve Moth packages.

use super::{FirstPartyDepsFinding, FirstPartyDepsRule, ScanState};
use crate::source_tree::relative_display_path;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const EXTRACTED_PACKAGES: &[(&str, &[&str])] =
    &[("moth-lexical", &[]), ("moth-mon", &["moth-lexical"])];

/// Packages no extracted library may reach, whether or not they are workspace members.
const ALWAYS_FORBIDDEN_PACKAGES: &[&str] = &["moth", "xtask"];

/// Canonical package identities and the authored alias of a forbidden Cargo edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RustDependencyEdge {
    pub package: String,
    pub dependency: String,
    pub kind: RustDependencyKind,
    pub rename: Option<String>,
}

/// Cargo distinguishes normal, development and build dependencies; none bypass this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RustDependencyKind {
    Normal,
    Dev,
    Build,
}

impl fmt::Display for RustDependencyKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Normal => "normal",
            Self::Dev => "dev",
            Self::Build => "build",
        })
    }
}

#[derive(Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
    workspace_members: Vec<String>,
}

#[derive(Deserialize)]
struct CargoPackage {
    id: String,
    name: String,
    manifest_path: PathBuf,
    dependencies: Vec<CargoDependency>,
}

#[derive(Deserialize)]
struct CargoDependency {
    name: String,
    kind: Option<RustDependencyKind>,
    rename: Option<String>,
}

pub(super) fn audit_rust_dependencies(
    workspace_root: &Path,
    state: &mut ScanState,
) -> Result<(), String> {
    let workspace_root = fs::canonicalize(workspace_root)
        .map_err(|error| format!("failed to resolve Cargo workspace root: {error}"))?;
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .arg("--manifest-path")
        .arg(workspace_root.join("Cargo.toml"))
        .current_dir(&workspace_root)
        .output()
        .map_err(|error| {
            format!("failed to run cargo metadata for first-party dependencies: {error}")
        })?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata for first-party dependencies failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let metadata: CargoMetadata = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid cargo metadata for first-party dependencies: {error}"))?;
    let mut packages: Vec<&CargoPackage> = metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .collect();
    packages.sort_by(|left, right| left.manifest_path.cmp(&right.manifest_path));

    // Report Cargo's actual manifest inventory rather than assuming fixed crate directories.
    state.audited_roots.push("Cargo.toml".to_owned());
    for package in &packages {
        let manifest = relative_display_path(&workspace_root, &package.manifest_path)?;
        if !state.audited_roots.contains(&manifest) {
            state.audited_roots.push(manifest);
        }
    }

    for (package_name, allowed_dependencies) in EXTRACTED_PACKAGES {
        let package = packages
            .iter()
            .find(|package| package.name == *package_name)
            .ok_or_else(|| {
                format!("cargo metadata is missing configured workspace package '{package_name}'")
            })?;
        let manifest = relative_display_path(&workspace_root, &package.manifest_path)?;

        for dependency in &package.dependencies {
            let is_first_party = packages.iter().any(|member| member.name == dependency.name);
            let is_forbidden = ALWAYS_FORBIDDEN_PACKAGES.contains(&dependency.name.as_str())
                || (is_first_party && !allowed_dependencies.contains(&dependency.name.as_str()));
            if !is_forbidden {
                continue;
            }

            let kind = dependency.kind.unwrap_or(RustDependencyKind::Normal);
            state.findings.push(FirstPartyDepsFinding {
                file: manifest.clone(),
                rule: FirstPartyDepsRule::RustDependencyDirection,
                message: format!(
                    "'{package_name}' has a forbidden {kind} dependency on first-party package '{}'",
                    dependency.name
                ),
                rust_dependency: Some(RustDependencyEdge {
                    package: package.name.clone(),
                    dependency: dependency.name.clone(),
                    kind,
                    rename: dependency.rename.clone(),
                }),
            });
        }
    }

    Ok(())
}
