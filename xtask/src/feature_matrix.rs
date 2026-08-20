//! Feature-lane matrix: one executed command per feature-gated test owner.
//!
//! WHAT: owns the curated feature lanes, runs each lane's package-scoped test command, and proves
//!       that every Cargo feature the workspace declares is named by a lane and that every feature
//!       name a `cfg` attribute mentions is a declared feature.
//! WHY:  a feature-gated test that no command runs is not a test, and a `cfg` on a misspelled
//!       feature name is a test that can never compile. Neither appears as a failure anywhere, so
//!       both need a gate that reads the tree instead of a hand-maintained list in prose.
//!
//! Lanes are package-scoped on purpose. Cargo unifies features across one resolve graph, and
//! `xtask` depends on `moth` with `features = ["timers"]`, so `cargo test --workspace` always
//! compiles `moth` with `timers` enabled and can never execute a `#[cfg(not(feature = "timers"))]`
//! branch. Only `cargo test -p moth` with an explicit feature list configures the compiler crate
//! the way a lane claims it does.
//!
//! # What this module owns
//! - The lane table: which feature sets are executed, and what each lane uniquely covers
//! - The per-lane test command and the complete outcome table
//! - Declared-feature and `cfg`-name coverage, and the machine-readable coverage report
//!
//! # What this module does NOT own
//! - The tests themselves, or their pass criteria
//! - Thread and repeat coverage (see `stress`)
//! - Zero-cost erasure of the timer system (see `timers_erasure_check`)

use crate::report_file::{ReportRunIdentity, write_report_atomically};
use crate::rust_scanner::{code_mask, is_identifier_character, matches_at};
use crate::source_tree::{relative_display_path, walk_rust_files, workspace_root};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Where the coverage report is written, relative to the workspace root.
pub const COVERAGE_REPORT_PATH: &str = "target/test-reports/feature_lane_coverage.json";

/// Where the lane-outcome report is written, relative to the workspace root.
///
/// This is a separate file from the coverage map on purpose. The coverage report answers "does
/// every declared feature have a lane", which `feature-lane-check` can answer without running
/// anything; this one answers "what happened when each lane ran", which only `feature-matrix`
/// can. One file carrying both would have to claim outcomes it did not measure whenever the
/// cheap command wrote it.
pub const MATRIX_RESULTS_REPORT_PATH: &str = "target/test-reports/feature_matrix_results.json";

/// Schema version of the coverage report.
///
/// Bump whenever a field is added, removed or re-meant, so a consumer can reject a report it
/// cannot read instead of silently misreading it.
///
/// Schema 2 added the completion state, build features and thread count of the run identity, and
/// moved lane outcomes out into the separate matrix-results report.
pub const COVERAGE_REPORT_SCHEMA_VERSION: u32 = 2;

/// Schema version of the lane-outcome report.
pub const MATRIX_RESULTS_SCHEMA_VERSION: u32 = 1;

/// Cargo target directory the matrix builds into.
///
/// A matrix run compiles the compiler crate under every lane's feature set. Sharing the developer's
/// default target directory is correct for Cargo but leaves the tree's most recent build being
/// whichever lane finished last, which silently changes what a following `cargo test` runs.
const MATRIX_TARGET_DIR: &str = "target/feature-matrix";

/// One executed feature configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureLane {
    /// Stable lane name used by reports and by the summary table.
    pub name: &'static str,
    /// Cargo package the lane tests. Feature selection is only meaningful per package.
    pub package: &'static str,
    /// Features enabled for `package`, in declaration order.
    pub features: &'static [&'static str],
    /// What only this lane covers. Reviewed when a lane is added, removed or merged.
    pub owns: &'static str,
}

impl FeatureLane {
    /// The exact command this lane runs, as a reader would type it.
    pub fn command_line(&self) -> String {
        let mut line = format!("cargo test -p {} --quiet", self.package);
        if !self.features.is_empty() {
            line.push_str(" --features ");
            line.push_str(&self.features.join(","));
        }
        line.push_str(" -- --format terse");
        line
    }
}

impl fmt::Display for FeatureLane {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.features.is_empty() {
            write!(formatter, "{} ({}, no features)", self.name, self.package)
        } else {
            write!(
                formatter,
                "{} ({}, {})",
                self.name,
                self.package,
                self.features.join(",")
            )
        }
    }
}

/// The curated lane matrix, in execution order.
///
/// The union of the `moth` lanes' features is exactly the set of features `Cargo.toml` declares;
/// `run_feature_lane_check` fails when that stops being true, in either direction. `xtask` declares
/// no features of its own: its lane exists because its tests are only reachable through its own
/// package, and it pulls `moth` with `timers` by its own dependency declaration.
pub const FEATURE_LANES: &[FeatureLane] = &[
    FeatureLane {
        name: "default",
        package: "moth",
        features: &[],
        owns: "the shipped configuration and every `cfg(not(feature = ...))` branch",
    },
    FeatureLane {
        name: "timers",
        package: "moth",
        features: &["timers"],
        owns: "the timing collector, boundary identities and command/build timing tests",
    },
    FeatureLane {
        name: "detailed-timers",
        package: "moth",
        features: &["detailed_timers"],
        owns: "AST substage timings and the detailed-only summary shape",
    },
    FeatureLane {
        name: "counters",
        package: "moth",
        features: &["benchmark_counters"],
        owns: "counter-only builds, where counters record without a timing collector",
    },
    FeatureLane {
        name: "timers-counters",
        package: "moth",
        features: &["timers", "benchmark_counters"],
        owns: "collector-backed counters and the counter summary carried by a timing session",
    },
    FeatureLane {
        name: "scoped-blocks",
        package: "moth",
        features: &["checked_blocks", "async_blocks"],
        owns: "the deferred-feature diagnostics for `checked:` and `async:` blocks",
    },
    FeatureLane {
        name: "dev-output",
        package: "moth",
        features: &[
            "show_tokens",
            "show_headers",
            "show_ast",
            "show_eval",
            "show_hir",
            "show_codegen",
            "show_borrow_checker",
        ],
        owns: "the developer stage-dump branches, which no other lane compiles",
    },
    FeatureLane {
        name: "xtask",
        package: "xtask",
        features: &[],
        owns: "the benchmark, profiling and process-runner tests in the xtask package",
    },
];

/// Package manifests scanned for declared features, paired with the source tree they own.
const PACKAGE_SOURCES: &[(&str, &str, &str)] = &[
    ("moth", "Cargo.toml", "src"),
    ("xtask", "xtask/Cargo.toml", "xtask/src"),
];

/// Why one lane failed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LaneFailure {
    /// The lane's command could not be started.
    Launch(String),
    /// The lane ran and reported failure.
    Exit(Option<i32>),
}

impl LaneFailure {
    /// The reported outcome for a lane that did not pass.
    fn into_outcome(self) -> LaneOutcome {
        match self {
            Self::Launch(error) => LaneOutcome::LaunchFailed { error },
            Self::Exit(exit_code) => LaneOutcome::Failed { exit_code },
        }
    }
}

impl fmt::Display for LaneFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Launch(error) => write!(formatter, "could not start: {error}"),
            Self::Exit(Some(code)) => write!(formatter, "exit code {code}"),
            Self::Exit(None) => formatter.write_str("terminated without an exit code"),
        }
    }
}

/// One `cfg` site naming a feature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CfgSite {
    /// Workspace-relative path, with `/` separators on every platform.
    pub file: String,
    /// How many `cfg` attributes in that file name the feature.
    pub occurrences: usize,
    /// Whether the file owns `#[test]` functions, so a lane decides whether tests execute here.
    pub has_test_items: bool,
}

/// Coverage of one declared feature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FeatureCoverage {
    pub feature: String,
    pub package: String,
    /// Lanes that enable the feature, in matrix order. Empty is a hard finding.
    pub lanes: Vec<String>,
    /// Files whose `cfg` attributes name the feature, in path order.
    pub cfg_sites: Vec<CfgSite>,
    /// `cfg_sites` entries that also own `#[test]` functions.
    pub test_owner_files: usize,
}

/// One executed lane, as reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LaneReport {
    pub name: String,
    pub package: String,
    pub features: Vec<String>,
    pub command: String,
    pub owns: String,
}

/// The machine-readable coverage map: which lane covers which declared feature.
///
/// This report describes coverage, never outcomes. `feature-lane-check` writes it without running
/// a single lane, so a lane result here would be a claim no run had measured. Lane outcomes live
/// in [`MatrixResultsReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CoverageReport {
    pub schema_version: u32,
    pub run: ReportRunIdentity,
    pub lanes: Vec<LaneReport>,
    pub features: Vec<FeatureCoverage>,
    /// Feature names a `cfg` attribute uses that the owning package does not declare.
    pub undeclared_cfg_features: Vec<CfgSite>,
    /// Hard findings, in report order. A non-empty list is a failed check.
    pub findings: Vec<String>,
}

/// What one lane's run produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum LaneOutcome {
    /// The matrix has not reached this lane yet.
    ///
    /// A pending lane is recorded rather than omitted so an interrupted matrix reports which
    /// lanes it never got to, instead of a shorter list that reads as a complete one.
    Pending,
    /// The lane's test command exited successfully.
    Passed,
    /// The lane's test command could not be started.
    LaunchFailed { error: String },
    /// The lane ran and reported failure.
    Failed { exit_code: Option<i32> },
}

/// One lane and what running it produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LaneResult {
    pub lane: LaneReport,
    pub result: LaneOutcome,
}

/// The machine-readable outcome table of one `feature-matrix` run.
///
/// Written before the first lane starts and rewritten after each lane finishes, so an interrupted
/// matrix leaves the lanes it did measure plus `completed: false`, rather than a stale table from
/// a previous run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatrixResultsReport {
    pub schema_version: u32,
    pub run: ReportRunIdentity,
    pub lanes: Vec<LaneResult>,
}

impl MatrixResultsReport {
    fn passed(&self) -> usize {
        self.lanes
            .iter()
            .filter(|lane| lane.result == LaneOutcome::Passed)
            .count()
    }

    fn failures(&self) -> Vec<&LaneResult> {
        self.lanes
            .iter()
            .filter(|lane| !matches!(lane.result, LaneOutcome::Passed | LaneOutcome::Pending))
            .collect()
    }
}

/// Validate lane coverage and write the report, without running any lane.
pub fn run_feature_lane_check() -> Result<(), String> {
    let workspace_root = workspace_root()?;
    let run = ReportRunIdentity::started("feature-lane-check", None);
    let report = build_coverage_report(&workspace_root, run)?;

    print_coverage(&report);
    write_coverage_report(&workspace_root, &report)?;

    if report.findings.is_empty() {
        return Ok(());
    }

    for finding in &report.findings {
        println!("  {finding}");
    }
    Err(format!(
        "{} feature-lane coverage finding(s)",
        report.findings.len()
    ))
}

/// Validate coverage, then run every lane and report the complete outcome table.
///
/// Lanes keep running after a failure. A matrix exists to show which configurations are broken,
/// and stopping at the first one hides the rest.
///
/// Two reports are written, because there are two separate facts. The coverage map is finished
/// before any lane starts and is complete at that point. The outcome table is only complete when
/// the last lane has run, so it starts as `Pending` for every lane and is rewritten as each one
/// resolves.
pub fn run_feature_matrix() -> Result<(), String> {
    let workspace_root = workspace_root()?;
    let coverage_run = ReportRunIdentity::started("feature-matrix", None);
    let coverage = build_coverage_report(&workspace_root, coverage_run)?;

    print_coverage(&coverage);
    write_coverage_report(&workspace_root, &coverage)?;

    if !coverage.findings.is_empty() {
        for finding in &coverage.findings {
            println!("  {finding}");
        }
        return Err(format!(
            "{} feature-lane coverage finding(s); no lane was run",
            coverage.findings.len()
        ));
    }

    let mut results = MatrixResultsReport {
        schema_version: MATRIX_RESULTS_SCHEMA_VERSION,
        run: ReportRunIdentity::started("feature-matrix", None),
        lanes: FEATURE_LANES
            .iter()
            .map(|lane| LaneResult {
                lane: lane_report(lane),
                result: LaneOutcome::Pending,
            })
            .collect(),
    };
    write_matrix_results(&workspace_root, &results)?;

    for (index, lane) in FEATURE_LANES.iter().enumerate() {
        println!("\n=== feature lane: {lane} ===");
        println!("{}", lane.command_line());

        results.lanes[index].result = match run_lane(&workspace_root, lane) {
            Ok(()) => LaneOutcome::Passed,
            Err(failure) => {
                println!("lane failed: {failure}");
                failure.into_outcome()
            }
        };
        write_matrix_results(&workspace_root, &results)?;
    }

    results.run = results.run.completed();
    write_matrix_results(&workspace_root, &results)?;

    let failures = results.failures();
    println!("\n=== feature matrix summary ===");
    println!("lanes run: {}", FEATURE_LANES.len());
    println!("lanes passed: {}", results.passed());
    println!("lanes failed: {}", failures.len());
    if failures.is_empty() {
        return Ok(());
    }

    for failure in &failures {
        println!("  {}: {}", failure.lane.name, describe(&failure.result));
    }
    Err(format!(
        "{} of {} feature lanes failed",
        failures.len(),
        FEATURE_LANES.len()
    ))
}

/// How a resolved lane outcome reads in the summary table.
fn describe(outcome: &LaneOutcome) -> String {
    match outcome {
        LaneOutcome::Pending => "never run".to_string(),
        LaneOutcome::Passed => "passed".to_string(),
        LaneOutcome::LaunchFailed { error } => format!("could not start: {error}"),
        LaneOutcome::Failed {
            exit_code: Some(code),
        } => format!("exit code {code}"),
        LaneOutcome::Failed { exit_code: None } => "terminated without an exit code".to_string(),
    }
}

fn write_matrix_results(workspace_root: &Path, report: &MatrixResultsReport) -> Result<(), String> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("failed to serialise the feature matrix results: {error}"))?;
    write_report_atomically(
        &workspace_root.join(MATRIX_RESULTS_REPORT_PATH),
        json.as_bytes(),
    )
}

/// Execute one lane, inheriting stdio so a failing lane shows its own output.
fn run_lane(workspace_root: &Path, lane: &FeatureLane) -> Result<(), LaneFailure> {
    let mut command = Command::new("cargo");
    command
        .arg("test")
        .arg("-p")
        .arg(lane.package)
        .arg("--quiet");
    if !lane.features.is_empty() {
        command.arg("--features").arg(lane.features.join(","));
    }
    command.arg("--").arg("--format").arg("terse");

    let status = command
        .current_dir(workspace_root)
        .env("CARGO_TARGET_DIR", MATRIX_TARGET_DIR)
        .status()
        .map_err(|error| LaneFailure::Launch(error.to_string()))?;

    if status.success() {
        Ok(())
    } else {
        Err(LaneFailure::Exit(status.code()))
    }
}

/// Build the coverage report for another audit to compose, without writing it.
///
/// The honesty audit runs this rather than reading `feature_lane_coverage.json`, because reading
/// the file would make its verdict depend on when somebody last ran `feature-lane-check`. The
/// coverage report itself stays owned by that command.
pub fn build_coverage_report_for_audit(workspace_root: &Path) -> Result<CoverageReport, String> {
    build_coverage_report(
        workspace_root,
        ReportRunIdentity::started("honesty-audit", None),
    )
}

/// Read the tree and build the complete coverage report.
fn build_coverage_report(
    workspace_root: &Path,
    run: ReportRunIdentity,
) -> Result<CoverageReport, String> {
    let mut features: Vec<FeatureCoverage> = Vec::new();
    let mut undeclared: Vec<CfgSite> = Vec::new();
    let mut findings: Vec<String> = Vec::new();

    for (package, manifest_relative, source_relative) in PACKAGE_SOURCES {
        let manifest_path = workspace_root.join(manifest_relative);
        let manifest = fs::read_to_string(&manifest_path)
            .map_err(|error| format!("failed to read '{}': {error}", manifest_path.display()))?;
        let declared = declared_features(&manifest).map_err(|error| {
            format!("failed to read features from '{manifest_relative}': {error}")
        })?;

        let sites = scan_cfg_features(workspace_root, &workspace_root.join(source_relative))?;

        for feature in &declared {
            let lanes = lanes_enabling(package, feature);
            if lanes.is_empty() {
                findings.push(format!(
                    "feature '{feature}' is declared by package '{package}' but no lane enables it"
                ));
            }
            let cfg_sites = sites.get(feature).cloned().unwrap_or_default();
            let test_owner_files = cfg_sites.iter().filter(|site| site.has_test_items).count();
            features.push(FeatureCoverage {
                feature: feature.clone(),
                package: (*package).to_string(),
                lanes,
                cfg_sites,
                test_owner_files,
            });
        }

        for (feature, mut sites) in sites {
            if declared.contains(&feature) {
                continue;
            }
            for site in &mut sites {
                findings.push(format!(
                    "{}: cfg names feature '{feature}', which package '{package}' does not declare",
                    site.file
                ));
            }
            undeclared.extend(sites);
        }

        for lane in FEATURE_LANES.iter().filter(|lane| lane.package == *package) {
            for feature in lane.features {
                if !declared.iter().any(|declared| declared == feature) {
                    findings.push(format!(
                        "lane '{}' enables feature '{feature}', which package '{package}' does not declare",
                        lane.name
                    ));
                }
            }
        }
    }

    Ok(CoverageReport {
        schema_version: COVERAGE_REPORT_SCHEMA_VERSION,
        // The coverage map is finished the moment this function returns: reading the tree is the
        // whole of the work it describes.
        run: run.completed(),
        lanes: FEATURE_LANES.iter().map(lane_report).collect(),
        features,
        undeclared_cfg_features: undeclared,
        findings,
    })
}

fn lane_report(lane: &FeatureLane) -> LaneReport {
    LaneReport {
        name: lane.name.to_string(),
        package: lane.package.to_string(),
        features: lane
            .features
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
        command: lane.command_line(),
        owns: lane.owns.to_string(),
    }
}

/// Lanes that enable `feature` for `package`, in matrix order.
fn lanes_enabling(package: &str, feature: &str) -> Vec<String> {
    FEATURE_LANES
        .iter()
        .filter(|lane| lane.package == package && lane.features.contains(&feature))
        .map(|lane| lane.name.to_string())
        .collect()
}

/// Print the feature-to-lane mapping in report order.
fn print_coverage(report: &CoverageReport) {
    println!("=== feature lanes ===");
    for lane in &report.lanes {
        println!("  {:<16} {}", lane.name, lane.command);
    }

    println!("\n=== feature coverage ===");
    for feature in &report.features {
        let lanes = if feature.lanes.is_empty() {
            "NO LANE".to_string()
        } else {
            feature.lanes.join(", ")
        };
        println!(
            "  {:<20} {:<8} lanes: {lanes:<28} cfg files: {:>2} (test owners: {})",
            feature.feature,
            feature.package,
            feature.cfg_sites.len(),
            feature.test_owner_files
        );
    }

    println!("\nfindings: {}", report.findings.len());
}

fn write_coverage_report(workspace_root: &Path, report: &CoverageReport) -> Result<(), String> {
    let report_path = workspace_root.join(COVERAGE_REPORT_PATH);
    let json = serde_json::to_string_pretty(report).map_err(|error| {
        format!("failed to serialise the feature-lane coverage report: {error}")
    })?;
    write_report_atomically(&report_path, json.as_bytes())
}

/// Feature names declared in a Cargo manifest's `[features]` table, sorted.
///
/// A feature key whose value is a list of enabled features is still one declared feature; the
/// lane matrix enables features by name and lets Cargo expand the implications.
fn declared_features(manifest: &str) -> Result<BTreeSet<String>, String> {
    let document: toml::Value =
        toml::from_str(manifest).map_err(|error| format!("invalid TOML: {error}"))?;

    let Some(features) = document.get("features") else {
        return Ok(BTreeSet::new());
    };

    let table = features
        .as_table()
        .ok_or_else(|| "[features] is not a table".to_string())?;

    Ok(table.keys().cloned().collect())
}

/// Every feature name a `cfg` attribute in `root` mentions, by feature, in path order.
fn scan_cfg_features(
    workspace_root: &Path,
    root: &Path,
) -> Result<BTreeMap<String, Vec<CfgSite>>, String> {
    let mut sites: BTreeMap<String, Vec<CfgSite>> = BTreeMap::new();

    for path in walk_rust_files(root)? {
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read '{}': {error}", path.display()))?;

        let names = cfg_feature_names(&content);
        if names.is_empty() {
            continue;
        }

        let relative = relative_display_path(workspace_root, &path)?;
        let has_test_items = content.contains("#[test]");

        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for name in names {
            *counts.entry(name).or_insert(0) += 1;
        }

        for (name, occurrences) in counts {
            sites.entry(name).or_default().push(CfgSite {
                file: relative.clone(),
                occurrences,
                has_test_items,
            });
        }
    }

    for entries in sites.values_mut() {
        entries.sort_by(|left, right| left.file.cmp(&right.file));
    }

    Ok(sites)
}

/// Feature names named by `cfg(...)` and `cfg_attr(...)` attributes in one file, with repeats.
///
/// `cfg` text inside a comment or a string literal is not an attribute. Several files here carry
/// one as prose or as scan input for another gate — this module's own doc comment and self-tests,
/// and `xtask/src/timers_erasure_check.rs` — and counting those would attribute a `moth` feature
/// to the `xtask` package, which declares none. The feature name itself is a string literal, so
/// literals are located rather than removed: only the position of the `cfg` token decides whether
/// an attribute is real.
fn cfg_feature_names(content: &str) -> Vec<String> {
    let characters: Vec<char> = content.chars().collect();
    let is_code = code_mask(&characters);
    let mut names = Vec::new();

    for index in 0..characters.len() {
        if !is_code[index]
            || index
                .checked_sub(1)
                .is_some_and(|previous| is_identifier_character(characters[previous]))
        {
            continue;
        }

        let Some(prefix_len) = cfg_prefix_length(&characters, index) else {
            continue;
        };
        let Some(span) = balanced_span(&characters, &is_code, index + prefix_len) else {
            continue;
        };
        collect_feature_names(span, &mut names);
    }

    names
}

/// Length of the `cfg(` or `cfg_attr(` prefix at `index`, including the opening parenthesis.
///
/// `cfg_attr` is matched separately because `"cfg_attr("` does not start with `"cfg("`.
fn cfg_prefix_length(characters: &[char], index: usize) -> Option<usize> {
    for prefix in ["cfg(", "cfg_attr("] {
        if matches_at(characters, index, prefix) {
            return Some(prefix.chars().count());
        }
    }
    None
}

/// The characters between `open_index` and the matching close parenthesis.
///
/// Only code parentheses change the depth, so a parenthesis inside a feature name or any other
/// literal cannot close the span early. An unbalanced span is `None` rather than a scan to end of
/// file, so a malformed attribute cannot pull unrelated source in.
fn balanced_span<'a>(
    characters: &'a [char],
    is_code: &[bool],
    open_index: usize,
) -> Option<&'a [char]> {
    let mut depth = 1_usize;

    for cursor in open_index..characters.len() {
        if !is_code[cursor] {
            continue;
        }
        match characters[cursor] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&characters[open_index..cursor]);
                }
            }
            _ => {}
        }
    }

    None
}

/// Append every `feature = "name"` value found in one `cfg` span.
fn collect_feature_names(span: &[char], names: &mut Vec<String>) {
    let mut index = 0;

    while index < span.len() {
        if !matches_at(span, index, "feature") {
            index += 1;
            continue;
        }

        let mut cursor = index + "feature".len();
        index = cursor;
        if span
            .get(cursor)
            .is_some_and(|next| is_identifier_character(*next))
        {
            continue;
        }

        cursor = skip_whitespace(span, cursor);
        if span.get(cursor) != Some(&'=') {
            continue;
        }
        cursor = skip_whitespace(span, cursor + 1);
        if span.get(cursor) != Some(&'"') {
            continue;
        }
        cursor += 1;

        let name_start = cursor;
        while cursor < span.len() && span[cursor] != '"' {
            cursor += 1;
        }
        if cursor == span.len() {
            continue;
        }

        names.push(span[name_start..cursor].iter().collect());
        index = cursor + 1;
    }
}

fn skip_whitespace(span: &[char], mut index: usize) -> usize {
    while span
        .get(index)
        .is_some_and(|character| character.is_whitespace())
    {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests;
