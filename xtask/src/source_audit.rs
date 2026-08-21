//! The owned broad-source architecture audit.
//!
//! WHAT: walks every Rust source file in the workspace once, applies the architecture bans that
//!       are genuinely source-shaped, and reports typed findings plus a machine-readable report.
//! WHY:  a ban whose scope is "no file anywhere may contain this" is not a behaviour claim, and a
//!       unit test that asserts it reads one file's text and calls it evidence. Those bans belong
//!       to one audit with structured findings, so a reader can see every rule, every allowlist
//!       and every hit in one place — and so a behaviour test never stands in for a behaviour that
//!       source text cannot prove.
//!
//! # What this module owns
//! - The single walk of `src` and `xtask/src`, failing closed on unreadable paths
//! - Rule dispatch, typed findings and the JSON report
//! - The architecture bans that have no other owner
//!
//! # What this module does NOT own
//! - Timer rule definitions, which stay with the timer subsystem in `timers_erasure_check`
//! - Compiler/build dependency-direction rule definitions, which stay with
//!   `architecture_boundary`
//! - The compiled-artifact half of timer erasure, which needs a built binary
//! - Feature-lane coverage (see `feature_matrix`)

use crate::architecture_boundary::{BoundaryRule, audit_architecture_boundary_fragment};
use crate::report_file::{ReportRunIdentity, write_report_atomically};
use crate::source_tree::{relative_display_path, walk_rust_files, workspace_root};
use crate::timers_erasure_check::audit_timer_source_fragment;
use serde::Serialize;
use std::fmt;
use std::fs;
use std::path::Path;

/// Where the audit report is written, relative to the workspace root.
pub const SOURCE_AUDIT_REPORT_PATH: &str = "target/test-reports/source_audit.json";

/// Schema version of the source audit report.
pub const SOURCE_AUDIT_SCHEMA_VERSION: u32 = 1;

/// Source trees the audit walks, in scan order.
const AUDITED_SOURCE_ROOTS: &[&str] = &["src", "xtask/src"];

/// Files exempt from every rule because they are the audit's own implementation.
///
/// These necessarily contain the fragments the audit searches for. Their rules are proved by
/// focused unit tests against fixture text instead.
const AUDIT_IMPLEMENTATION_FILES: &[&str] = &[
    "xtask/src/timers_erasure_check.rs",
    "xtask/src/source_audit.rs",
    "xtask/src/source_audit/tests.rs",
];

/// Which rule produced a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRule {
    /// A timer-subsystem source rule owned by `timers_erasure_check`.
    TimerErasure,
    /// The removed legacy error conversion was reintroduced by name.
    RemovedLegacyConversionName,
    /// The removed legacy diagnostic payload variant was reintroduced by name.
    RemovedLegacyPayloadVariant,
    /// Production code outside `compiler_frontend` named a frontend semantic stage owner.
    ExternalStageOrchestration,
    /// Production `compiler_frontend` code named build-system or project config state.
    CompilerDependencyOnBuild,
    /// A source file could not be read, so no rule could be applied to it.
    UnreadableSource,
}

impl SourceRule {
    const fn label(self) -> &'static str {
        match self {
            Self::TimerErasure => "timer-erasure",
            Self::RemovedLegacyConversionName => "legacy-error-conversion-name",
            Self::RemovedLegacyPayloadVariant => "legacy-error-payload-variant",
            Self::ExternalStageOrchestration => "external-stage-orchestration",
            Self::CompilerDependencyOnBuild => "compiler-dependency-on-build",
            Self::UnreadableSource => "unreadable-source",
        }
    }
}

/// One rule hit, named by the file it was found in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceFinding {
    /// Workspace-relative path with `/` separators on every platform.
    pub file: String,
    pub rule: SourceRule,
    pub message: String,
}

impl fmt::Display for SourceFinding {
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

/// The complete machine-readable audit report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceAuditReport {
    pub schema_version: u32,
    pub run: ReportRunIdentity,
    pub audited_roots: Vec<String>,
    pub audited_file_count: usize,
    pub findings: Vec<SourceFinding>,
}

/// Run the audit and write its report.
///
/// The report is replaced by a `completed: false` one before the walk starts. Without that, a run
/// interrupted during the walk leaves the previous successful report untouched, and a reader has
/// no way to tell that file apart from evidence this run produced.
pub fn run_source_audit() -> Result<(), String> {
    let workspace_root = workspace_root()?;
    let report_path = workspace_root.join(SOURCE_AUDIT_REPORT_PATH);
    let run = ReportRunIdentity::started("source-audit", None);

    write_source_audit_report(&report_path, &started_report(run.clone()))?;

    let (audited_file_count, findings) = audit_sources(&workspace_root)?;

    let report = SourceAuditReport {
        schema_version: SOURCE_AUDIT_SCHEMA_VERSION,
        run: run.completed(),
        audited_roots: audited_roots(),
        audited_file_count,
        findings,
    };

    write_source_audit_report(&report_path, &report)?;

    println!(
        "source-audit: {} files audited, {} finding(s)",
        report.audited_file_count,
        report.findings.len()
    );

    if report.findings.is_empty() {
        return Ok(());
    }

    for finding in &report.findings {
        println!("  {finding}");
    }
    Err(format!(
        "source audit failed with {} finding(s)",
        report.findings.len()
    ))
}

/// The report a run writes before it has audited anything.
///
/// The counts are zero and the findings empty because that is what this run has measured so far;
/// `completed: false` is what tells a reader those numbers are not a result.
fn started_report(run: ReportRunIdentity) -> SourceAuditReport {
    SourceAuditReport {
        schema_version: SOURCE_AUDIT_SCHEMA_VERSION,
        run,
        audited_roots: audited_roots(),
        audited_file_count: 0,
        findings: Vec::new(),
    }
}

fn audited_roots() -> Vec<String> {
    AUDITED_SOURCE_ROOTS
        .iter()
        .map(|root| (*root).to_string())
        .collect()
}

fn write_source_audit_report(path: &Path, report: &SourceAuditReport) -> Result<(), String> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("failed to serialise the source audit report: {error}"))?;
    write_report_atomically(path, json.as_bytes())
}

/// Apply every rule to every audited source file, returning the file count and all findings.
pub fn audit_sources(workspace_root: &Path) -> Result<(usize, Vec<SourceFinding>), String> {
    let mut findings = Vec::new();
    let mut audited_file_count = 0;

    for root in AUDITED_SOURCE_ROOTS {
        for path in walk_rust_files(&workspace_root.join(root))? {
            let relative = relative_display_path(workspace_root, &path)?;
            if AUDIT_IMPLEMENTATION_FILES.contains(&relative.as_str()) {
                continue;
            }

            audited_file_count += 1;

            let content = match fs::read_to_string(&path) {
                Ok(content) => content,
                Err(error) => {
                    // A source that cannot be read is a finding, not a file the audit skips: a
                    // silently skipped file makes the audit pass by looking at less than it claims.
                    findings.push(SourceFinding {
                        file: relative,
                        rule: SourceRule::UnreadableSource,
                        message: format!("unreadable ({error})"),
                    });
                    continue;
                }
            };

            findings.extend(audit_source_fragment(&relative, &content));
        }
    }

    Ok((audited_file_count, findings))
}

/// Apply every rule to one file's text.
///
/// Separated from the walk so each rule has focused coverage against fixture text.
fn audit_source_fragment(relative: &str, content: &str) -> Vec<SourceFinding> {
    let mut findings: Vec<SourceFinding> = audit_timer_source_fragment(relative, content)
        .into_iter()
        .map(|message| SourceFinding {
            file: relative.to_owned(),
            rule: SourceRule::TimerErasure,
            message,
        })
        .collect();

    findings.extend(audit_legacy_error_conversion(relative, content));
    findings.extend(audit_legacy_error_payload(relative, content));
    findings.extend(
        audit_architecture_boundary_fragment(relative, content)
            .into_iter()
            .map(|(rule, message)| SourceFinding {
                file: relative.to_owned(),
                rule: match rule {
                    BoundaryRule::ExternalStageOrchestration => {
                        SourceRule::ExternalStageOrchestration
                    }
                    BoundaryRule::CompilerDependencyOnBuild => {
                        SourceRule::CompilerDependencyOnBuild
                    }
                },
                message,
            }),
    );
    findings
}

/// The removed legacy error conversion must not come back by name.
///
/// The behaviour this protects — failure-message assertions reading typed render-boundary output
/// — is owned by the integration assertion tests. Text matching cannot prove that behaviour: an
/// alias, a reformat or an equivalent reimplementation would all pass. This is a reintroduction
/// tripwire, and saying so is why it lives here rather than posing as a behaviour test.
fn audit_legacy_error_conversion(relative: &str, content: &str) -> Vec<SourceFinding> {
    // Assembled so the audit's own source does not contain the banned name.
    let removed_conversion_name = ["to", "_", "legacy", "_", "error"].concat();

    if !content.contains(&removed_conversion_name) {
        return Vec::new();
    }

    vec![SourceFinding {
        file: relative.to_owned(),
        rule: SourceRule::RemovedLegacyConversionName,
        message: format!(
            "names the removed legacy error conversion '{removed_conversion_name}'; \
             diagnostics must reach the render boundary through the typed path"
        ),
    }]
}

/// The removed legacy diagnostic payload variant must not come back by name.
///
/// Diagnostics carry typed payloads; the removed variant carried a rendered string, which is the
/// shape every diagnostic assertion in the suite exists to avoid. The behaviour is owned by the
/// payload and descriptor tests. This is the reintroduction tripwire, banned across the tree
/// rather than in the payload module alone: the name is a removed one, so an occurrence anywhere
/// is worth a reader's attention.
fn audit_legacy_error_payload(relative: &str, content: &str) -> Vec<SourceFinding> {
    // Assembled so the audit's own source does not contain the banned name.
    let removed_variant_name = ["Legacy", "Error"].concat();

    if !content.contains(&removed_variant_name) {
        return Vec::new();
    }

    vec![SourceFinding {
        file: relative.to_owned(),
        rule: SourceRule::RemovedLegacyPayloadVariant,
        message: format!(
            "names the removed diagnostic payload variant '{removed_variant_name}'; \
             diagnostic payloads are typed, never a rendered string"
        ),
    }]
}

#[cfg(test)]
mod tests;
