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
//! - The single walk of `src`, `xtask/src` and the extracted crate sources, failing closed on
//!   unreadable paths
//! - Rule dispatch, typed findings and the JSON report
//! - The architecture bans that have no other owner
//!
//! # What this module does NOT own
//! - Timer rule definitions, which stay with the timer subsystem in `timers_erasure_check`
//! - Compiler/build dependency-direction rule definitions, which stay with
//!   `architecture_boundary`
//! - The compiled-artifact half of timer erasure, which needs a built binary
//! - Feature-lane coverage (see `feature_matrix`)

use crate::architecture_boundary::{
    BoundaryRule, audit_architecture_boundary_fragment, is_test_source,
};
use crate::report_file::{ReportRunIdentity, write_report_atomically};
use crate::rust_scanner::{code_mask, matches_at};
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
const AUDITED_SOURCE_ROOTS: &[&str] = &[
    "src",
    "crates/moth-lexical/src",
    "crates/moth-mon/src",
    "crates/moth-mon/tests",
    "crates/moth-host/src",
    "crates/moth-cli/src",
    "crates/moth-cli/tests",
    "xtask/src",
];

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
    /// Production `src/` code interned host-path display text as a diagnostic fact.
    UnportableDiagnosticPathFact,
    /// Production code outside `compiler_frontend` named a frontend semantic stage owner.
    ExternalStageOrchestration,
    /// Production `compiler_frontend` code named build-system or project config state.
    CompilerDependencyOnBuild,
    /// Production Boracle oracle code named a static-solver owner.
    OracleStaticSolverIndependence,
    /// A durable HIR carrier reintroduced a known recursive owning expression or place shape.
    RecursiveHirOwnership,
    /// Production backend code named borrow-state snapshots as local-write semantic selectors.
    BackendBorrowStateSemantics,
    /// A source file could not be read, so no rule could be applied to it.
    UnreadableSource,
}

impl SourceRule {
    const fn label(self) -> &'static str {
        match self {
            Self::TimerErasure => "timer-erasure",
            Self::RemovedLegacyConversionName => "legacy-error-conversion-name",
            Self::RemovedLegacyPayloadVariant => "legacy-error-payload-variant",
            Self::UnportableDiagnosticPathFact => "unportable-diagnostic-path-fact",
            Self::ExternalStageOrchestration => "external-stage-orchestration",
            Self::CompilerDependencyOnBuild => "compiler-dependency-on-build",
            Self::OracleStaticSolverIndependence => "oracle-static-solver-independence",
            Self::RecursiveHirOwnership => "recursive-hir-ownership",
            Self::BackendBorrowStateSemantics => "backend-borrow-state-semantics",
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
    println!("source-audit roots: {}", report.audited_roots.join(", "));

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
    findings.extend(audit_unportable_diagnostic_path_fact(relative, content));
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
                    BoundaryRule::OracleStaticSolverIndependence => {
                        SourceRule::OracleStaticSolverIndependence
                    }
                    BoundaryRule::RecursiveHirOwnership => SourceRule::RecursiveHirOwnership,
                    BoundaryRule::BackendBorrowStateSemantics => {
                        SourceRule::BackendBorrowStateSemantics
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

/// Host-path display text must reach diagnostics through `DiagnosticStringBuilder::intern_path`.
///
/// A path fact carries the native `Path`'s provenance. Interning `Path::display` or
/// `to_string_lossy` output stores a lossy rendering in place of the fact, which is why the
/// portable conversion has one owned operation. Every interning call is inspected whatever its
/// receiver, and each conversion is reported once per file.
///
/// This is a reintroduction tripwire, so an alias or an intermediate binding such as
/// `let text = path.display().to_string()` escapes it. Only the zero-argument
/// `.display()` counts, so typed `display(..)` calls are not reported, although a new
/// zero-argument `display()` would match inside an interning argument. Matching assumes
/// `rustfmt` layout, which never separates a method name from its `(` or puts whitespace
/// inside an empty argument list.
///
/// Scope is production `src/` only. Extracted crates and `xtask` have their own source surfaces,
/// and tests may assert displayed text directly. The shared path-based test-source skip applies,
/// so an inline `#[cfg(test)]` module inside a production file is still scanned.
fn audit_unportable_diagnostic_path_fact(relative: &str, content: &str) -> Vec<SourceFinding> {
    if !relative.starts_with("src/") || is_test_source(relative) {
        return Vec::new();
    }

    unportable_path_display_conversions(content)
        .into_iter()
        .map(|conversion| SourceFinding {
            file: relative.to_owned(),
            rule: SourceRule::UnportableDiagnosticPathFact,
            message: format!(
                "interns path text from `{conversion}`; diagnostic path facts intern the native \
                 path through `DiagnosticStringBuilder::intern_path`"
            ),
        })
        .collect()
}

/// Diagnostic string interning calls that can receive a rendered host path.
const DIAGNOSTIC_INTERNING_CALLS: &[&str] = &[".intern(", ".get_or_intern("];

/// Host-path display conversions this rule refuses to see interned as a diagnostic fact.
///
/// `.to_string_lossy(` needs no companion check because it already produces the rendered text.
/// `.display()` must be the zero-argument form followed by text production, so the typed
/// `TypeDisplaySnapshot::display(..)` calls this repository also uses do not match.
const LOSSY_PATH_DISPLAY: &str = "to_string_lossy";
const PATH_DISPLAY: &str = "Path::display";

/// Distinct path display conversions passed to diagnostic interning calls.
fn unportable_path_display_conversions(content: &str) -> Vec<&'static str> {
    let mut conversions = Vec::new();

    for argument in diagnostic_interning_arguments(content) {
        if argument.contains(".to_string_lossy(") && !conversions.contains(&LOSSY_PATH_DISPLAY) {
            conversions.push(LOSSY_PATH_DISPLAY);
        }

        let displays_path = argument.contains(".display()")
            && (argument.contains(".to_string(") || argument.contains("format!"));
        if displays_path && !conversions.contains(&PATH_DISPLAY) {
            conversions.push(PATH_DISPLAY);
        }
    }

    conversions
}

/// The code-only argument text of every diagnostic interning call in `content`.
///
/// `rust_scanner` decides which characters are code, so a call-shaped fragment in a string
/// literal or a comment is never the call and its text is blanked before the conversion search.
/// The same mask keeps a parenthesis in a character literal from unbalancing the argument.
fn diagnostic_interning_arguments(content: &str) -> Vec<String> {
    let characters: Vec<char> = content.chars().collect();
    let is_code = code_mask(&characters);
    let mut arguments = Vec::new();

    for index in 0..characters.len() {
        if !is_code[index] {
            continue;
        }

        let Some(call) = DIAGNOSTIC_INTERNING_CALLS
            .iter()
            .find(|call| matches_at(&characters, index, call))
        else {
            continue;
        };

        let opening_parenthesis = index + call.chars().count() - 1;
        if let Some(argument) = balanced_code_argument(&characters, &is_code, opening_parenthesis) {
            arguments.push(argument);
        }
    }
    arguments
}

/// The code text between a call's opening parenthesis and its matching closing parenthesis.
///
/// Literal and comment characters are replaced with spaces, so conversions can be matched with
/// plain substring tests without splicing across a blanked span.
fn balanced_code_argument(
    characters: &[char],
    is_code: &[bool],
    opening_parenthesis: usize,
) -> Option<String> {
    let mut depth = 1_usize;

    for index in opening_parenthesis + 1..characters.len() {
        if !is_code[index] {
            continue;
        }

        match characters[index] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let argument = &characters[opening_parenthesis + 1..index];
                    let argument_is_code = &is_code[opening_parenthesis + 1..index];
                    return Some(
                        argument
                            .iter()
                            .zip(argument_is_code)
                            .map(|(character, is_code)| if *is_code { *character } else { ' ' })
                            .collect(),
                    );
                }
            }
            _ => {}
        }
    }

    None
}

#[cfg(test)]
mod tests;
