//! The first-party package dependency audit.
//!
//! WHAT: walks the explicitly owned first-party package roots, rejects package-manager metadata
//!       and vendored dependency directories, and applies the moth first-party JavaScript import
//!       policy to physical `.js` assets plus the compiler-owned JavaScript inventory.
//! WHY: first-party packages promise zero third-party runtime dependencies. That promise needs one
//!      narrow source owner rather than a repository-wide text search that mistakes documentation,
//!      tests or benchmarks for package implementations.
//!
//! # What this module owns
//! - The scoped walk of first-party package implementation roots.
//! - Findings, the atomic JSON report and the `first-party-deps` command result.
//!
//! # What this module does NOT own
//! - JavaScript lexical scanning or the runtime-module allowlist; those live in `moth::first_party_js`.
//! - User-owned or future dependency packages and their manifests.
//! - Package declarations, aliases, resolution or package-graph design.
//! - Generated HTML runtime glue, documentation, tests, benchmarks or repository-root manifests.

use crate::report_file::{ReportRunIdentity, write_report_atomically};
use crate::source_tree::{WalkDecision, relative_display_path, walk_source_tree, workspace_root};
use moth::first_party_js::{inventoried_javascript_sources, javascript_import_findings};
use serde::Serialize;
use std::fmt;
use std::fs;
use std::path::Path;

/// Where the first-party dependency report is written, relative to the workspace root.
pub const FIRST_PARTY_DEPS_REPORT_PATH: &str = "target/test-reports/first_party_deps.json";

/// Schema version of the first-party dependency report.
pub const FIRST_PARTY_DEPS_SCHEMA_VERSION: u32 = 2;

/// First-party implementation roots, in deterministic scan order.
///
/// These are deliberately package-owned roots rather than all source, documentation or fixture
/// trees. Keep this list synchronized with the ownership statement in `validation.mtf`.
pub const FIRST_PARTY_SOURCE_ROOTS: &[&str] = &[
    "packages",
    "src/builder_surface/core_packages",
    "src/backends/js/package_bindings",
    "src/projects/html_project/binding_packages",
];

const INVENTORIED_JS_ROOT_LABEL: &str = "moth::first_party_js::inventoried_javascript_sources";

/// Exact package-manager files forbidden inside a first-party implementation root.
const FORBIDDEN_MANIFEST_BASENAMES: &[&str] = &[
    "package.json",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "deno.json",
    "deno.jsonc",
    "deno.lock",
];

/// Directory names that identify copied or vendored third-party code.
const FORBIDDEN_VENDOR_DIRECTORY_NAMES: &[&str] = &["node_modules", "vendor", "third_party"];

/// Which first-party dependency rule produced a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirstPartyDepsRule {
    /// A package-manager manifest or lockfile was found by exact basename.
    PackageManagerManifest,
    /// A known vendored dependency directory was found by exact directory name.
    VendoredDependencyRoot,
    /// A JavaScript import, require or re-export is not an exact registered runtime module.
    UnapprovedModuleImport,
    /// A path could not be read or represented, so the audit could not inspect it.
    UnreadablePath,
}

impl FirstPartyDepsRule {
    const fn label(self) -> &'static str {
        match self {
            Self::PackageManagerManifest => "package-manager-manifest",
            Self::VendoredDependencyRoot => "vendored-dependency-root",
            Self::UnapprovedModuleImport => "unapproved-module-import",
            Self::UnreadablePath => "unreadable-path",
        }
    }
}

/// One first-party dependency finding, named by the implementation file or directory involved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FirstPartyDepsFinding {
    /// Workspace-relative path with `/` separators, or an inventoried JavaScript label.
    pub file: String,
    pub rule: FirstPartyDepsRule,
    pub message: String,
}

impl fmt::Display for FirstPartyDepsFinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "[{}] {}: {}",
            self.rule.label(),
            self.file,
            self.message
        )
    }
}

/// Complete machine-readable first-party dependency audit report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FirstPartyDepsReport {
    pub schema_version: u32,
    pub run: ReportRunIdentity,
    pub audited_roots: Vec<String>,
    pub visited_file_count: usize,
    pub javascript_source_count: usize,
    pub findings: Vec<FirstPartyDepsFinding>,
}

/// Run the scoped first-party dependency audit and write its report.
pub fn run_first_party_deps() -> Result<(), String> {
    let workspace_root = workspace_root()?;
    let report_path = workspace_root.join(FIRST_PARTY_DEPS_REPORT_PATH);
    let run = ReportRunIdentity::started("first-party-deps", None);

    write_first_party_deps_report(&report_path, &started_report(run.clone()))?;

    let (visited_file_count, javascript_source_count, findings) =
        audit_first_party_deps(&workspace_root)?;
    let report = FirstPartyDepsReport {
        schema_version: FIRST_PARTY_DEPS_SCHEMA_VERSION,
        run: run.completed(),
        audited_roots: audited_roots(),
        visited_file_count,
        javascript_source_count,
        findings,
    };

    write_first_party_deps_report(&report_path, &report)?;

    println!(
        "first-party-deps: {} files visited, {} JavaScript sources, {} finding(s)",
        report.visited_file_count,
        report.javascript_source_count,
        report.findings.len()
    );

    if report.findings.is_empty() {
        return Ok(());
    }

    for finding in &report.findings {
        println!("  {finding}");
    }
    Err(format!(
        "first-party dependency audit failed with {} finding(s)",
        report.findings.len()
    ))
}

/// Audit the first-party roots and the compiler-owned JavaScript inventory.
///
/// Root traversal errors return `Err`; unreadable files become typed findings so a completed report
/// still records exactly which paths prevented inspection.
pub(crate) fn audit_first_party_deps(
    workspace_root: &Path,
) -> Result<(usize, usize, Vec<FirstPartyDepsFinding>), String> {
    let mut state = ScanState::default();

    for root in FIRST_PARTY_SOURCE_ROOTS {
        scan_first_party_root(workspace_root, root, &mut state)?;
    }

    scan_inventoried_javascript(&mut state);
    Ok((
        state.visited_file_count,
        state.javascript_source_count,
        state.findings,
    ))
}

fn started_report(run: ReportRunIdentity) -> FirstPartyDepsReport {
    FirstPartyDepsReport {
        schema_version: FIRST_PARTY_DEPS_SCHEMA_VERSION,
        run,
        audited_roots: audited_roots(),
        visited_file_count: 0,
        javascript_source_count: 0,
        findings: Vec::new(),
    }
}

fn audited_roots() -> Vec<String> {
    let mut roots: Vec<String> = FIRST_PARTY_SOURCE_ROOTS
        .iter()
        .map(|root| (*root).to_owned())
        .collect();
    roots.push(INVENTORIED_JS_ROOT_LABEL.to_owned());
    roots
}

fn write_first_party_deps_report(path: &Path, report: &FirstPartyDepsReport) -> Result<(), String> {
    let json = serde_json::to_string_pretty(report).map_err(|error| {
        format!("failed to serialise the first-party dependency report: {error}")
    })?;
    write_report_atomically(path, json.as_bytes())
}

#[derive(Default)]
struct ScanState {
    visited_file_count: usize,
    javascript_source_count: usize,
    findings: Vec<FirstPartyDepsFinding>,
}

fn scan_first_party_root(
    workspace_root: &Path,
    relative_root: &str,
    state: &mut ScanState,
) -> Result<(), String> {
    let root = workspace_root.join(relative_root);
    let metadata = fs::symlink_metadata(&root).map_err(|error| {
        format!(
            "failed to read first-party root '{}': {error}",
            root.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "first-party root '{}' is not a directory",
            root.display()
        ));
    }

    walk_source_tree(&root, |path, metadata| {
        let relative = relative_display_path(workspace_root, path)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                format!(
                    "path '{}' has a file name that is not valid UTF-8",
                    path.display()
                )
            })?;

        if FORBIDDEN_MANIFEST_BASENAMES.contains(&name) {
            state.findings.push(FirstPartyDepsFinding {
                file: relative.clone(),
                rule: FirstPartyDepsRule::PackageManagerManifest,
                message: format!("forbidden package-manager file '{name}'"),
            });
        }

        if metadata.is_dir() {
            if FORBIDDEN_VENDOR_DIRECTORY_NAMES.contains(&name) {
                state.findings.push(FirstPartyDepsFinding {
                    file: relative,
                    rule: FirstPartyDepsRule::VendoredDependencyRoot,
                    message: format!("forbidden vendored dependency directory '{name}'"),
                });
                return Ok(WalkDecision::SkipDescendants);
            }

            return Ok(WalkDecision::Continue);
        }

        if metadata.is_file() {
            state.visited_file_count += 1;
            let bytes = match fs::read(path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    state.findings.push(FirstPartyDepsFinding {
                        file: relative,
                        rule: FirstPartyDepsRule::UnreadablePath,
                        message: format!("unreadable file ({error})"),
                    });
                    return Ok(WalkDecision::Continue);
                }
            };

            if is_javascript_file(path) {
                match String::from_utf8(bytes) {
                    Ok(source) => {
                        state.javascript_source_count += 1;
                        state
                            .findings
                            .extend(audit_javascript_source(&relative, &source));
                    }
                    Err(error) => state.findings.push(FirstPartyDepsFinding {
                        file: relative,
                        rule: FirstPartyDepsRule::UnreadablePath,
                        message: format!("JavaScript source is not valid UTF-8 ({error})"),
                    }),
                }
            }

            return Ok(WalkDecision::Continue);
        }

        state.visited_file_count += 1;
        state.findings.push(FirstPartyDepsFinding {
            file: relative,
            rule: FirstPartyDepsRule::UnreadablePath,
            message: "path is neither a regular file nor a directory".to_owned(),
        });
        Ok(WalkDecision::Continue)
    })
}

fn scan_inventoried_javascript(state: &mut ScanState) {
    for source in inventoried_javascript_sources() {
        state.javascript_source_count += 1;
        state
            .findings
            .extend(audit_javascript_source(&source.label, &source.source));
    }
}

fn is_javascript_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("js" | "mjs" | "cjs")
    )
}

/// Apply the moth first-party import policy to one JavaScript source fragment.
fn audit_javascript_source(file: &str, source: &str) -> Vec<FirstPartyDepsFinding> {
    javascript_import_findings(source)
        .into_iter()
        .map(|message| FirstPartyDepsFinding {
            file: file.to_owned(),
            rule: FirstPartyDepsRule::UnapprovedModuleImport,
            message,
        })
        .collect()
}

#[cfg(test)]
mod tests;
