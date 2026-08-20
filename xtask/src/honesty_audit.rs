//! The owned test-honesty audit and the canonical honesty inventory.
//!
//! WHAT: classifies the test-suite honesty findings this campaign is built around — hard findings
//!       that fail the gate and review findings that carry a recorded disposition — composes the
//!       specialised audits that own their own evidence, and writes the canonical inventory at
//!       `target/test-reports/test_honesty_inventory.json`.
//! WHY:  the campaign's central inventory was being maintained by hand beside the commands that
//!       measure it. A hand-maintained inventory is a claim about the tree, not a reading of it:
//!       it can say a finding is resolved after the code that resolved it was reverted, and its
//!       `generated_at` can name a date no run happened on. One executable owner makes the
//!       inventory a result.
//!
//! # What this module owns
//! - The hard rules whose every hit fails the audit
//! - The review rules, each with the campaign's recorded disposition for that category
//! - Ledger integrity: no duplicate code, no undisposed finding, no open hard finding
//! - The canonical inventory report, and the durable evidence copy on request
//!
//! # What this module does NOT own
//! - The broad-source architecture bans (see `source_audit`, composed here)
//! - Feature-lane coverage (see `feature_matrix`, composed here)
//! - The integration suite inventory (owned by `moth tests --audit`, composed here)
//!
//! Composition, not collapse. Each specialised audit keeps writing its own report and stays the
//! place to look for its own detail; this inventory records what each one found and whether its
//! evidence was current, so a reviewer reading one file can see whether the campaign's claims
//! still hold and where to go for the rest.
//!
//! # Why this is not a regex ban
//! Several of these categories are legal, common and correct in the great majority of their uses.
//! `is_err`, `contains` and `any` all have honest uses, and a gate that failed on each of them
//! would be turned off within a week. Those categories are *review* findings: the audit counts
//! them, names the files they live in, and records the disposition the campaign decided for the
//! category, so the decision is visible and re-checkable rather than implicit. Only the rules
//! that have no honest use in this tree are hard.

use crate::feature_matrix::{
    COVERAGE_REPORT_PATH, CoverageReport, build_coverage_report_for_audit,
};
use crate::report_file::{ReportRunIdentity, write_report_atomically};
use crate::rust_scanner::{TextClass, classify};
use crate::source_audit::{SOURCE_AUDIT_REPORT_PATH, SourceFinding, audit_sources};
use crate::source_tree::{relative_display_path, walk_rust_files, workspace_root};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Where the canonical inventory is written, relative to the workspace root.
pub const HONESTY_INVENTORY_REPORT_PATH: &str = "target/test-reports/test_honesty_inventory.json";

/// The tracked declared ledger this audit reads.
pub const HONESTY_LEDGER_PATH: &str = "docs/roadmap/evidence/honesty_ledger.json";

/// The tracked durable copy of the inventory, refreshed only on request.
pub const DURABLE_INVENTORY_PATH: &str = "docs/roadmap/evidence/test_honesty_inventory.json";

/// Schema version of the inventory report.
pub const HONESTY_INVENTORY_SCHEMA_VERSION: u32 = 1;

/// The integration suite inventory this audit composes, written by `moth tests --audit`.
const SUITE_INVENTORY_PATH: &str = "target/test-reports/integration_suite_inventory.json";

/// Source trees the audit scans, in scan order.
const SCANNED_SOURCE_ROOTS: &[&str] = &["src", "xtask/src"];

/// Files exempt from every scan rule because they are the audit's own implementation.
///
/// These necessarily contain the fragments the audit searches for, in the tables that define
/// them. Their rules are proved by focused unit tests against fixture text instead.
const AUDIT_IMPLEMENTATION_FILES: &[&str] = &[
    "xtask/src/honesty_audit.rs",
    "xtask/src/honesty_audit/tests.rs",
];

/// Which view of a source line a rule reads.
///
/// A rule that reads the wrong view is the classic false audit: a rule looking for a banned call
/// in raw text reports the doc comment that warns against it, and a rule looking for a path
/// literal in code text never finds one at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleView {
    /// Source the compiler executes. Comments and literal text are blanked out.
    Code,
    /// String and character literal text. Code and comments are blanked out.
    Literal,
}

/// What the campaign decided about a review category.
///
/// A review finding is not a defect. It is a shape that *can* hide a weak contract, so the
/// campaign owes a reader a reason it is acceptable here. These are those reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// The uses that could hide a weak contract were rewritten; what remains cannot.
    Hardened,
    /// The API under test has exactly one way to fail, so the broad assertion is exact.
    NarrowApiWithOneValidOutcome,
    /// The case deliberately claims only that the operation completed, and says so.
    IntentionallySmokeLevel,
    /// The rendered prose is the contract, so matching its text is matching the contract.
    RenderedProseIsTheContract,
    /// The behaviour belongs to a real platform API that only some platforms own.
    PlatformSpecificByRealApiOwnership,
    /// The claim is not behaviour, and now belongs to a structured audit instead of a test.
    MovedToStructuredAudit,
}

/// Which files a rule applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleScope {
    /// Files that own `#[test]` items. Most of these rules are about test honesty, and the same
    /// shape in production code is often correct.
    TestOwningFiles,
    /// Every Rust file under the audited roots.
    EveryFile,
}

/// What one view of a line must and must not contain.
#[derive(Debug, Clone, Copy)]
struct ViewTest {
    /// Every fragment must be present.
    required: &'static [&'static str],
    /// At least one must be present. Empty means no such requirement.
    any_of: &'static [&'static str],
    /// None may be present.
    absent: &'static [&'static str],
}

/// A view test that constrains nothing.
const ANY: ViewTest = ViewTest {
    required: &[],
    any_of: &[],
    absent: &[],
};

impl ViewTest {
    fn matches(&self, view: &str) -> bool {
        self.required.iter().all(|fragment| view.contains(fragment))
            && (self.any_of.is_empty()
                || self.any_of.iter().any(|fragment| view.contains(fragment)))
            && !self.absent.iter().any(|fragment| view.contains(fragment))
    }
}

/// A declarative match over one source line.
///
/// Every condition is a plain fragment test against one view. The audit stays a search with named
/// rules rather than a parser with opinions: a rule a reader cannot restate in a sentence is a
/// rule nobody will trust a finding from.
///
/// Testing both views is what separates "a `/tmp` literal" from "a `/tmp` literal handed to a call
/// that creates something on disk": the path is literal text and the call is code, and only the
/// pair is a defect.
#[derive(Debug, Clone, Copy)]
struct LineMatcher {
    /// The view the rule is primarily about, recorded in the report.
    view: RuleView,
    /// Applied to `view`.
    primary: ViewTest,
    /// Applied to the other view.
    other: ViewTest,
}

impl LineMatcher {
    fn matches(&self, line: &ScannedLine) -> bool {
        let (primary, other) = match self.view {
            RuleView::Code => (line.code.as_str(), line.literals.as_str()),
            RuleView::Literal => (line.literals.as_str(), line.code.as_str()),
        };

        self.primary.matches(primary) && self.other.matches(other)
    }
}

/// A rule whose every hit fails the audit.
#[derive(Debug, Clone, Copy)]
struct HardRule {
    code: &'static str,
    /// What the rule finds, and why no honest use of it exists in this tree.
    description: &'static str,
    scope: RuleScope,
    matcher: LineMatcher,
}

/// A rule whose hits are counted and dispositioned rather than rejected.
#[derive(Debug, Clone, Copy)]
struct ReviewRule {
    code: &'static str,
    description: &'static str,
    disposition: Disposition,
    /// Why that disposition is the right one for this category in this tree.
    rationale: &'static str,
    scope: RuleScope,
    matcher: LineMatcher,
}

/// Calls that create, write or remove something on the filesystem.
///
/// A path literal only matters when something acts on it. These are the acts.
const FILESYSTEM_MUTATIONS: &[&str] = &[
    "create_dir",
    "create_dir_all",
    "File::create",
    "fs::write",
    "write_all",
    "remove_file",
    "remove_dir",
    "remove_dir_all",
    "copy(",
    "rename(",
    "set_permissions",
];

/// The rules with no honest use in this tree. Every hit fails the audit.
const HARD_RULES: &[HardRule] = &[
    HardRule {
        code: "retired_temp_dir_helper",
        description: "names the retired test_support::temp_dir helper, which returned a path it \
                      had not created; owned temporary workspaces come from tempfile::tempdir",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Code,
            primary: ViewTest {
                required: &["test_support::temp_dir"],
                any_of: &[],
                absent: &[],
            },
            other: ANY,
        },
    },
    HardRule {
        code: "tmp_literal_reaching_the_filesystem",
        description: "hands a hardcoded /tmp path to a call that creates, writes or removes \
                      something; a shared system directory is not an owned workspace, so a \
                      previous run's leftovers become this run's starting state",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Literal,
            primary: ViewTest {
                required: &["/tmp"],
                any_of: &[],
                absent: &[],
            },
            other: ViewTest {
                required: &[],
                any_of: FILESYSTEM_MUTATIONS,
                absent: &[],
            },
        },
    },
    HardRule {
        code: "discarded_directory_or_environment_restoration",
        description: "discards the result of restoring the current directory or an environment \
                      variable; a failed restore leaves every later test running against the \
                      wrong global state, and reports the wrong test as the failure",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Code,
            primary: ViewTest {
                required: &["let _ ="],
                any_of: &["set_current_dir", "set_var", "remove_var"],
                absent: &[],
            },
            other: ANY,
        },
    },
    HardRule {
        code: "discarded_thread_join",
        description: "discards a thread join result, which silently swallows a worker panic and \
                      turns it into a hang or a passing test",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Code,
            primary: ViewTest {
                required: &["let _ =", ".join()"],
                any_of: &[],
                absent: &[],
            },
            other: ANY,
        },
    },
    HardRule {
        code: "unowned_ignore",
        description: "an #[ignore] with no reason; an ignored test with no owner is a test that \
                      stopped running and nobody agreed to that",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Code,
            primary: ViewTest {
                required: &["#[ignore]"],
                any_of: &[],
                absent: &[],
            },
            other: ANY,
        },
    },
    HardRule {
        code: "unatomic_report_write",
        description: "writes a machine-readable report straight to its final path; an interrupted \
                      run then leaves a half-written file where a reader expects evidence",
        scope: RuleScope::EveryFile,
        matcher: LineMatcher {
            view: RuleView::Literal,
            primary: ViewTest {
                required: &["test-reports/"],
                any_of: &[],
                absent: &[],
            },
            other: ViewTest {
                required: &[],
                any_of: &["fs::write(", "File::create("],
                absent: &[],
            },
        },
    },
];

/// The rules whose hits are review prompts with a recorded disposition.
///
/// None of these fails the audit. Each is a shape that *can* hide a weak contract and is also
/// correct in most of its uses, so the audit records how many there are, which files they are in,
/// and the decision the campaign reached about the category.
const REVIEW_RULES: &[ReviewRule] = &[
    ReviewRule {
        code: "broad_failure_assertions",
        description: "is_err, is_ok, catch_unwind or should_panic, which accept any failure \
                      rather than the failure the case is about",
        disposition: Disposition::Hardened,
        rationale: "Phase 2 replaced every broad is_err assertion on a wide API with a typed \
                    expected-failure assertion carrying a structured rejection reason. What \
                    remains is on APIs whose failure set has one member.",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Code,
            primary: ViewTest {
                required: &[],
                any_of: &[".is_err()", ".is_ok()", "catch_unwind", "#[should_panic"],
                absent: &[],
            },
            other: ANY,
        },
    },
    ReviewRule {
        code: "filesystem_existence_predicates",
        description: "exists, is_file or is_dir, each of which reports an IO failure as absence",
        disposition: Disposition::MovedToStructuredAudit,
        rationale: "Phase 1 migrated every assertion about the filesystem to the test_fs helpers, \
                    which use symlink_metadata and fail on an IO error instead of reading it as a \
                    missing file. Remaining uses inside test files are setup guards, where \
                    absence and failure lead to the same next step.",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Code,
            primary: ViewTest {
                required: &[],
                any_of: &[".exists()", ".is_file()", ".is_dir()"],
                absent: &[],
            },
            other: ANY,
        },
    },
    ReviewRule {
        code: "lossy_text_conversion",
        description: "to_string_lossy or from_utf8_lossy, which replace unsupported input with a \
                      substitute character and then assert on the substitute",
        disposition: Disposition::Hardened,
        rationale: "Phase 5 removed lossy conversion from every assertion path. Remaining uses \
                    are in rendered diagnostic output, where a replacement character is what a \
                    reader should see rather than a failure.",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Code,
            primary: ViewTest {
                required: &[],
                any_of: &["to_string_lossy", "from_utf8_lossy"],
                absent: &[],
            },
            other: ANY,
        },
    },
    ReviewRule {
        code: "weak_positive_assertions",
        description: "a bound comparison, a negated emptiness check or an any() assertion, each \
                      of which passes on more states than the one the case is about",
        disposition: Disposition::Hardened,
        rationale: "Phase 4 replaced the positive assertions that only proved something existed \
                    with exact counts and exact identities. What remains asserts a genuine bound \
                    or a genuine existential, where the exact value is not the contract.",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Code,
            primary: ViewTest {
                required: &["assert!("],
                any_of: &[">=", "<=", "(!", ".any("],
                absent: &[],
            },
            other: ANY,
        },
    },
    ReviewRule {
        code: "first_match_artifact_selection",
        description: "find_map selection, which silently picks one of several matches",
        disposition: Disposition::Hardened,
        rationale: "Phase 4 gave artifact lookup a typed inventory that rejects a duplicate path \
                    instead of returning whichever entry came first.",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Code,
            primary: ViewTest {
                required: &["find_map("],
                any_of: &[],
                absent: &[],
            },
            other: ANY,
        },
    },
    ReviewRule {
        code: "broad_contains_assertions",
        description: "an assertion that some output contains a fragment, which passes on any \
                      output that happens to include it",
        disposition: Disposition::RenderedProseIsTheContract,
        rationale: "The integration suite has zero contains-mode diagnostic contracts after Phase \
                    7. Remaining contains assertions are over rendered prose and over error \
                    messages, where the wording a reader sees is the contract and an exact \
                    whole-output match would pin unrelated formatting.",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Code,
            primary: ViewTest {
                required: &["assert!(", ".contains("],
                any_of: &[],
                absent: &[],
            },
            other: ANY,
        },
    },
    ReviewRule {
        code: "wall_clock_and_sleep_dependence",
        description: "a sleep, a spin loop or a wall-clock reading, each of which can make the \
                      result depend on how fast the machine is",
        disposition: Disposition::Hardened,
        rationale: "Phase 6 replaced every expected transition with a condition variable or a \
                    channel. A remaining wall-clock deadline is deadlock protection whose failure \
                    names the observed state, not the thing being asserted.",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Code,
            primary: ViewTest {
                required: &[],
                any_of: &["sleep(", "spin_loop", "Instant::now", ".modified()"],
                absent: &[],
            },
            other: ANY,
        },
    },
    ReviewRule {
        code: "source_text_assertions",
        description: "an assertion over source text read at compile time, which proves what a \
                      file says rather than what the code does",
        disposition: Disposition::MovedToStructuredAudit,
        rationale: "Phase 8 moved the tree-wide source bans into the source audit, which reports \
                    typed findings and says plainly that source text is not behaviour evidence. \
                    Remaining include_str! uses are fixtures and manifests, not claims about \
                    behaviour.",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Code,
            primary: ViewTest {
                required: &["include_str!", ".contains("],
                any_of: &[],
                absent: &[],
            },
            other: ANY,
        },
    },
    ReviewRule {
        code: "platform_gated_tests",
        description: "a test item gated to one platform family, which no other platform's lane \
                      executes",
        disposition: Disposition::PlatformSpecificByRealApiOwnership,
        rationale: "These own behaviour of a real platform API — symlinks, permission bits, \
                    non-UTF-8 filenames — that the other platforms do not have. Phase 8 makes \
                    them visible instead of silent by running the Linux, macOS and Windows CI \
                    gates, so a platform-owned test is executed somewhere rather than nowhere.",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Code,
            primary: ViewTest {
                required: &["cfg("],
                any_of: &["unix", "windows", "target_family", "target_os"],
                absent: &[],
            },
            other: ANY,
        },
    },
    ReviewRule {
        code: "inert_tmp_path_literals",
        description: "a hardcoded /tmp literal that no filesystem call on the same line acts on",
        disposition: Disposition::NarrowApiWithOneValidOutcome,
        rationale: "These are path values handed to pure path logic — rejection rules, prefix \
                    comparison, command argument assembly — which never touch the filesystem. A \
                    /tmp literal that does reach the filesystem is the hard rule \
                    tmp_literal_reaching_the_filesystem, which fails this audit.",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Literal,
            primary: ViewTest {
                required: &["/tmp"],
                any_of: &[],
                absent: &[],
            },
            other: ViewTest {
                required: &[],
                any_of: &[],
                absent: FILESYSTEM_MUTATIONS,
            },
        },
    },
    ReviewRule {
        code: "shared_system_temp_directory_values",
        description: "std::env::temp_dir(), which names a directory every process on the machine \
                      shares",
        disposition: Disposition::NarrowApiWithOneValidOutcome,
        rationale: "Used as an existing-directory value for path logic that must not create \
                    anything, in tests that assert on the resulting path rather than on disk. An \
                    owned workspace comes from tempfile::tempdir.",
        scope: RuleScope::TestOwningFiles,
        matcher: LineMatcher {
            view: RuleView::Code,
            primary: ViewTest {
                required: &["env::temp_dir("],
                any_of: &[],
                absent: &[],
            },
            other: ANY,
        },
    },
];

/// One line of one file, split into the views a rule can read.
///
/// Both views keep the original character positions, with the other view's characters replaced by
/// spaces. A fragment therefore cannot be assembled out of text that was never adjacent.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ScannedLine {
    number: usize,
    code: String,
    literals: String,
    text: String,
}

/// Where a hard rule matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FindingSite {
    pub file: String,
    pub line: usize,
    /// The source line, trimmed, so a reader can judge the hit without opening the file.
    pub source: String,
}

/// A hard finding and every place it occurs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HardFinding {
    pub code: String,
    pub description: String,
    pub scope: RuleScope,
    pub view: RuleView,
    pub sites: Vec<FindingSite>,
}

/// How many times a review category occurs in one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileOccurrences {
    pub file: String,
    pub occurrences: usize,
}

/// A review category, its recorded disposition, and where it occurs.
///
/// Per-file counts rather than per-line sites: a category with thousands of legal occurrences is
/// the normal case, and a list of every line would be a report nobody reads. The count is exact
/// and the files are named, which is what a reviewer needs to re-check the disposition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewFinding {
    pub code: String,
    pub description: String,
    pub disposition: Disposition,
    pub rationale: String,
    pub scope: RuleScope,
    pub view: RuleView,
    pub occurrences: usize,
    pub files: Vec<FileOccurrences>,
}

/// One finding the campaign declared, as tracked in the ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerFinding {
    pub code: String,
    pub severity: String,
    pub status: String,
    pub description: String,
    pub disposition: String,
    pub owning_phase: String,
}

/// A measurement the plan recorded, with the command that produced it.
///
/// These are declared, not measured by this audit: a suite count comes from running the suite.
/// They are carried here so the inventory is one place to look, and named `declared` so nobody
/// reads them as something this run observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredMeasurement {
    pub name: String,
    pub command: String,
    pub result: String,
    pub recorded_by_phase: String,
}

/// The tracked ledger this audit reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HonestyLedger {
    pub schema_version: u32,
    pub work_id: String,
    pub base_revision: String,
    pub declared_measurements: Vec<DeclaredMeasurement>,
    pub findings: Vec<LedgerFinding>,
    pub infrastructure_added: Vec<String>,
}

/// What the audit found wrong with the ledger itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LedgerIntegrityFinding {
    pub code: String,
    pub problem: String,
}

/// A composed audit's findings, and where its own report lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComposedAudit {
    /// The specialised report that owns the detail.
    pub report_path: String,
    pub finding_count: usize,
    pub findings: Vec<String>,
}

/// What this run could learn from the integration suite inventory.
///
/// A missing report and a report full of findings are different facts, and so is a report an
/// interrupted run abandoned. Collapsing them into "no findings" is how an audit passes by
/// reading nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum SuiteInventoryEvidence {
    /// No report exists. `moth tests --audit` has not run in this checkout.
    Absent { path: String },
    /// A report exists but could not be read or parsed.
    Unusable { path: String, reason: String },
    /// A report exists and was read.
    Present {
        path: String,
        schema_version: u64,
        run_id: String,
        run_command: String,
        /// Whether the run that wrote it finished. A `false` here means its counts are partial.
        run_completed: bool,
        hard_policy_violation_count: usize,
        advisory_finding_count: usize,
        summary: serde_json::Value,
    },
}

/// A weak-contract category the integration suite counts, and what the campaign decided about it.
///
/// These are not scan results. The integration suite classifies its own backend blocks and
/// reports the counters; this records the disposition for each, so a legal smoke case stays legal
/// and still has a visible decision behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuiteWeakContractReview {
    /// The counter in the suite inventory's summary this reads.
    pub counter: String,
    /// `None` when no readable suite inventory named the counter.
    pub occurrences: Option<u64>,
    pub disposition: Disposition,
    pub rationale: String,
}

/// The suite counters this audit dispositions, with the decision recorded for each.
const SUITE_WEAK_CONTRACT_REVIEWS: &[(&str, Disposition, &str)] = &[
    (
        "smoke_role_cases",
        Disposition::IntentionallySmokeLevel,
        "A case whose every backend claims only that compilation succeeded is legal when its role \
         says so. Phase 7 promoted every acceptance-only block whose behaviour was observable and \
         renamed the cases that overclaimed, so what this counts is smoke that admits to being \
         smoke.",
    ),
    (
        "warning_ignore_backend_blocks",
        Disposition::IntentionallySmokeLevel,
        "A block that makes warnings non-contractual accepts every future warning on that case. \
         Phase 7 replaced all four with forbid or exact warning sets; the counter stays so a \
         returning one is a number a reviewer sees rather than a silent weakening.",
    ),
    (
        "diagnostic_contains_backend_blocks",
        Disposition::RenderedProseIsTheContract,
        "A failure block matching diagnostics by containment accepts diagnostics beyond the \
         authored multiset. Phase 7 re-measured every one under exact matching and left none; the \
         counter stays because a stale authored reason keeps the weaker contract alive.",
    ),
    (
        "acceptance_only_backend_blocks",
        Disposition::IntentionallySmokeLevel,
        "Phase 7 promoted all 12 acceptance-only blocks to observable contracts. The counter is \
         the gate on that staying true.",
    ),
];

/// Every specialised audit this inventory composes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComposedEvidence {
    pub source_audit: ComposedAudit,
    pub feature_lane_coverage: ComposedAudit,
    pub integration_suite_inventory: SuiteInventoryEvidence,
    /// Dispositions for the weak-contract counters the suite inventory reports.
    pub suite_weak_contract_reviews: Vec<SuiteWeakContractReview>,
}

/// Why the audit passed or failed, in one place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GateSummary {
    pub scan_hard_finding_count: usize,
    pub ledger_integrity_finding_count: usize,
    pub open_hard_ledger_finding_count: usize,
    pub composed_finding_count: usize,
    pub review_finding_count: usize,
    pub review_occurrence_count: usize,
    pub passed: bool,
}

/// The canonical machine-readable honesty inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HonestyInventoryReport {
    pub schema_version: u32,
    pub run: ReportRunIdentity,
    /// When this run wrote the report, in UTC. Measured, never authored.
    pub generated_at: String,
    pub work_id: String,
    pub base_revision: String,
    pub scanned_roots: Vec<String>,
    pub scanned_file_count: usize,
    /// Files that own `#[test]` items, which is where the scan rules apply.
    pub test_owning_file_count: usize,
    pub hard_findings: Vec<HardFinding>,
    pub review_findings: Vec<ReviewFinding>,
    pub declared_findings: Vec<LedgerFinding>,
    pub ledger_integrity_findings: Vec<LedgerIntegrityFinding>,
    pub declared_measurements: Vec<DeclaredMeasurement>,
    pub infrastructure_added: Vec<String>,
    pub composed: ComposedEvidence,
    pub gate: GateSummary,
}

/// Run the audit, write the canonical inventory, and fail on any hard finding.
///
/// The report is replaced by a `completed: false` one before the scan starts, so a run
/// interrupted partway leaves a report that says so rather than the previous successful one.
pub fn run_honesty_audit(update_evidence: bool) -> Result<(), String> {
    let workspace_root = workspace_root()?;
    let report_path = workspace_root.join(HONESTY_INVENTORY_REPORT_PATH);
    let run = ReportRunIdentity::started("honesty-audit", None);

    write_inventory(&report_path, &started_report(run.clone()))?;

    let report = build_inventory(&workspace_root, run)?;
    write_inventory(&report_path, &report)?;

    if update_evidence {
        write_inventory(&workspace_root.join(DURABLE_INVENTORY_PATH), &report)?;
        println!("durable evidence refreshed: {DURABLE_INVENTORY_PATH}");
    }

    print_inventory(&report);

    if report.gate.passed {
        return Ok(());
    }
    Err(format!(
        "honesty audit failed: {} scan hard finding(s), {} ledger integrity finding(s), \
         {} open hard ledger finding(s), {} composed audit finding(s)",
        report.gate.scan_hard_finding_count,
        report.gate.ledger_integrity_finding_count,
        report.gate.open_hard_ledger_finding_count,
        report.gate.composed_finding_count
    ))
}

/// Read the tree, compose the specialised audits and assemble the complete inventory.
pub fn build_inventory(
    workspace_root: &Path,
    run: ReportRunIdentity,
) -> Result<HonestyInventoryReport, String> {
    let ledger = read_ledger(workspace_root)?;
    let ledger_integrity_findings = check_ledger_integrity(workspace_root, &ledger);
    let open_hard_ledger_finding_count = ledger
        .findings
        .iter()
        .filter(|finding| finding.severity == "hard" && finding.status == "open")
        .count();

    let scan = scan_tree(workspace_root)?;
    let composed = compose_evidence(workspace_root)?;

    let composed_finding_count =
        composed.source_audit.finding_count + composed.feature_lane_coverage.finding_count;
    let review_occurrence_count = scan
        .review_findings
        .iter()
        .map(|finding| finding.occurrences)
        .sum();

    let gate = GateSummary {
        scan_hard_finding_count: scan
            .hard_findings
            .iter()
            .map(|finding| finding.sites.len())
            .sum(),
        ledger_integrity_finding_count: ledger_integrity_findings.len(),
        open_hard_ledger_finding_count,
        composed_finding_count,
        review_finding_count: scan.review_findings.len(),
        review_occurrence_count,
        passed: scan.hard_findings.is_empty()
            && ledger_integrity_findings.is_empty()
            && open_hard_ledger_finding_count == 0
            && composed_finding_count == 0,
    };

    Ok(HonestyInventoryReport {
        schema_version: HONESTY_INVENTORY_SCHEMA_VERSION,
        run: run.completed(),
        generated_at: utc_timestamp(SystemTime::now()),
        work_id: ledger.work_id,
        base_revision: ledger.base_revision,
        scanned_roots: scanned_roots(),
        scanned_file_count: scan.scanned_file_count,
        test_owning_file_count: scan.test_owning_file_count,
        hard_findings: scan.hard_findings,
        review_findings: scan.review_findings,
        declared_findings: ledger.findings,
        ledger_integrity_findings,
        declared_measurements: ledger.declared_measurements,
        infrastructure_added: ledger.infrastructure_added,
        composed,
        gate,
    })
}

/// The report a run writes before it has scanned anything.
fn started_report(run: ReportRunIdentity) -> HonestyInventoryReport {
    HonestyInventoryReport {
        schema_version: HONESTY_INVENTORY_SCHEMA_VERSION,
        run,
        generated_at: utc_timestamp(SystemTime::now()),
        work_id: String::new(),
        base_revision: String::new(),
        scanned_roots: scanned_roots(),
        scanned_file_count: 0,
        test_owning_file_count: 0,
        hard_findings: Vec::new(),
        review_findings: Vec::new(),
        declared_findings: Vec::new(),
        ledger_integrity_findings: Vec::new(),
        declared_measurements: Vec::new(),
        infrastructure_added: Vec::new(),
        composed: ComposedEvidence {
            source_audit: ComposedAudit {
                report_path: SOURCE_AUDIT_REPORT_PATH.to_string(),
                finding_count: 0,
                findings: Vec::new(),
            },
            feature_lane_coverage: ComposedAudit {
                report_path: COVERAGE_REPORT_PATH.to_string(),
                finding_count: 0,
                findings: Vec::new(),
            },
            integration_suite_inventory: SuiteInventoryEvidence::Absent {
                path: SUITE_INVENTORY_PATH.to_string(),
            },
            suite_weak_contract_reviews: Vec::new(),
        },
        gate: GateSummary {
            scan_hard_finding_count: 0,
            ledger_integrity_finding_count: 0,
            open_hard_ledger_finding_count: 0,
            composed_finding_count: 0,
            review_finding_count: 0,
            review_occurrence_count: 0,
            passed: false,
        },
    }
}

fn scanned_roots() -> Vec<String> {
    SCANNED_SOURCE_ROOTS
        .iter()
        .map(|root| (*root).to_string())
        .collect()
}

/// What one scan of the tree produced.
struct ScanResult {
    scanned_file_count: usize,
    test_owning_file_count: usize,
    hard_findings: Vec<HardFinding>,
    review_findings: Vec<ReviewFinding>,
}

/// Apply every scan rule to every test-owning file under the audited roots.
fn scan_tree(workspace_root: &Path) -> Result<ScanResult, String> {
    let mut hard_sites: BTreeMap<&str, Vec<FindingSite>> = BTreeMap::new();
    let mut review_counts: BTreeMap<&str, BTreeMap<String, usize>> = BTreeMap::new();
    let mut scanned_file_count = 0;
    let mut test_owning_file_count = 0;

    for root in SCANNED_SOURCE_ROOTS {
        for path in walk_rust_files(&workspace_root.join(root))? {
            let relative = relative_display_path(workspace_root, &path)?;
            if AUDIT_IMPLEMENTATION_FILES.contains(&relative.as_str()) {
                continue;
            }
            scanned_file_count += 1;

            // A file that cannot be read fails the audit rather than being skipped: an audit that
            // silently reads fewer files than it names passes by looking at less.
            let content = fs::read_to_string(&path)
                .map_err(|error| format!("failed to read '{relative}': {error}"))?;

            let lines = scan_lines(&content);
            let owns_tests = owns_tests(&lines);
            if owns_tests {
                test_owning_file_count += 1;
            }

            for line in &lines {
                for rule in HARD_RULES {
                    if in_scope(rule.scope, owns_tests) && rule.matcher.matches(line) {
                        hard_sites.entry(rule.code).or_default().push(FindingSite {
                            file: relative.clone(),
                            line: line.number,
                            source: line.text.trim().to_string(),
                        });
                    }
                }
                for rule in REVIEW_RULES {
                    if in_scope(rule.scope, owns_tests) && rule.matcher.matches(line) {
                        *review_counts
                            .entry(rule.code)
                            .or_default()
                            .entry(relative.clone())
                            .or_default() += 1;
                    }
                }
            }
        }
    }

    Ok(ScanResult {
        scanned_file_count,
        test_owning_file_count,
        hard_findings: HARD_RULES
            .iter()
            .filter_map(|rule| {
                hard_sites.remove(rule.code).map(|sites| HardFinding {
                    code: rule.code.to_string(),
                    description: rule.description.to_string(),
                    scope: rule.scope,
                    view: rule.matcher.view,
                    sites,
                })
            })
            .collect(),
        review_findings: REVIEW_RULES
            .iter()
            .map(|rule| {
                let per_file = review_counts.remove(rule.code).unwrap_or_default();
                ReviewFinding {
                    code: rule.code.to_string(),
                    description: rule.description.to_string(),
                    disposition: rule.disposition,
                    rationale: rule.rationale.to_string(),
                    scope: rule.scope,
                    view: rule.matcher.view,
                    occurrences: per_file.values().sum(),
                    files: per_file
                        .into_iter()
                        .map(|(file, occurrences)| FileOccurrences { file, occurrences })
                        .collect(),
                }
            })
            .collect(),
    })
}

/// Whether a rule applies to a file, given whether that file owns tests.
fn in_scope(scope: RuleScope, owns_tests: bool) -> bool {
    match scope {
        RuleScope::TestOwningFiles => owns_tests,
        RuleScope::EveryFile => true,
    }
}

/// Whether a file owns `#[test]` items, in code rather than in a comment about them.
fn owns_tests(lines: &[ScannedLine]) -> bool {
    lines
        .iter()
        .any(|line| line.code.contains("#[test]") || line.code.contains("#[cfg(test)]"))
}

/// Split a file into lines and split each line into its code and literal views.
///
/// Every view keeps the original character positions, so a fragment can only match text that was
/// genuinely adjacent in that view.
fn scan_lines(content: &str) -> Vec<ScannedLine> {
    let characters: Vec<char> = content.chars().collect();
    let classes = classify(&characters);

    let mut lines = Vec::new();
    let mut number = 1;
    let mut code = String::new();
    let mut literals = String::new();
    let mut text = String::new();

    for (index, character) in characters.iter().enumerate() {
        if *character == '\n' {
            lines.push(ScannedLine {
                number,
                code: std::mem::take(&mut code),
                literals: std::mem::take(&mut literals),
                text: std::mem::take(&mut text),
            });
            number += 1;
            continue;
        }

        text.push(*character);
        match classes[index] {
            TextClass::Code => {
                code.push(*character);
                literals.push(' ');
            }
            TextClass::Literal => {
                code.push(' ');
                literals.push(*character);
            }
            TextClass::Comment => {
                code.push(' ');
                literals.push(' ');
            }
        }
    }

    if !text.is_empty() || !code.is_empty() {
        lines.push(ScannedLine {
            number,
            code,
            literals,
            text,
        });
    }

    lines
}

/// Run the specialised audits and record what each found.
///
/// They are run rather than read: reading their last reports would make this inventory's verdict
/// depend on when someone else last ran them.
fn compose_evidence(workspace_root: &Path) -> Result<ComposedEvidence, String> {
    let (_, source_findings) = audit_sources(workspace_root)?;
    let coverage: CoverageReport = build_coverage_report_for_audit(workspace_root)?;

    let integration_suite_inventory = read_suite_inventory(workspace_root);
    let suite_weak_contract_reviews = disposition_suite_counters(&integration_suite_inventory);

    Ok(ComposedEvidence {
        source_audit: ComposedAudit {
            report_path: SOURCE_AUDIT_REPORT_PATH.to_string(),
            finding_count: source_findings.len(),
            findings: source_findings
                .iter()
                .map(SourceFinding::to_string)
                .collect(),
        },
        feature_lane_coverage: ComposedAudit {
            report_path: COVERAGE_REPORT_PATH.to_string(),
            finding_count: coverage.findings.len(),
            findings: coverage.findings,
        },
        integration_suite_inventory,
        suite_weak_contract_reviews,
    })
}

/// Pair each suite weak-contract counter with its recorded disposition.
///
/// A counter the suite inventory does not name is reported as `None` rather than zero: "the suite
/// counted none" and "no suite inventory was read" are different facts, and reading the second as
/// the first is how an audit passes on evidence it never had.
fn disposition_suite_counters(evidence: &SuiteInventoryEvidence) -> Vec<SuiteWeakContractReview> {
    let summary = match evidence {
        SuiteInventoryEvidence::Present { summary, .. } => Some(summary),
        SuiteInventoryEvidence::Absent { .. } | SuiteInventoryEvidence::Unusable { .. } => None,
    };

    SUITE_WEAK_CONTRACT_REVIEWS
        .iter()
        .map(
            |(counter, disposition, rationale)| SuiteWeakContractReview {
                counter: (*counter).to_string(),
                occurrences: summary
                    .and_then(|summary| summary.get(counter))
                    .and_then(serde_json::Value::as_u64),
                disposition: *disposition,
                rationale: (*rationale).to_string(),
            },
        )
        .collect()
}

/// Read the integration suite inventory if a run left one, reporting its state either way.
fn read_suite_inventory(workspace_root: &Path) -> SuiteInventoryEvidence {
    let path = workspace_root.join(SUITE_INVENTORY_PATH);
    let reported_path = SUITE_INVENTORY_PATH.to_string();

    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return SuiteInventoryEvidence::Absent {
                path: reported_path,
            };
        }
        Err(error) => {
            return SuiteInventoryEvidence::Unusable {
                path: reported_path,
                reason: error.to_string(),
            };
        }
    };

    let document: serde_json::Value = match serde_json::from_str(&content) {
        Ok(document) => document,
        Err(error) => {
            return SuiteInventoryEvidence::Unusable {
                path: reported_path,
                reason: format!("not valid JSON: {error}"),
            };
        }
    };

    let Some(run) = document.get("run") else {
        return SuiteInventoryEvidence::Unusable {
            path: reported_path,
            reason: "the report has no run identity".to_string(),
        };
    };

    SuiteInventoryEvidence::Present {
        path: reported_path,
        schema_version: document
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default(),
        run_id: string_field(run, "id"),
        run_command: string_field(run, "command"),
        run_completed: run
            .get("completed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        hard_policy_violation_count: array_len(&document, "hard_policy_violations"),
        advisory_finding_count: array_len(&document, "advisory_findings"),
        summary: document
            .get("summary")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    }
}

fn string_field(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn array_len(value: &serde_json::Value, key: &str) -> usize {
    value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len)
}

/// Read and parse the tracked ledger.
fn read_ledger(workspace_root: &Path) -> Result<HonestyLedger, String> {
    let path = workspace_root.join(HONESTY_LEDGER_PATH);
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read '{HONESTY_LEDGER_PATH}': {error}"))?;

    serde_json::from_str(&content)
        .map_err(|error| format!("failed to parse '{HONESTY_LEDGER_PATH}': {error}"))
}

/// The severities and statuses a ledger entry may carry.
const LEDGER_SEVERITIES: &[&str] = &["hard", "review"];
const LEDGER_STATUSES: &[&str] = &["open", "resolved"];

/// Check the ledger says something complete about every finding it declares.
///
/// A ledger entry with no disposition is the exact thing this campaign exists to remove: a known
/// weakness recorded without a decision about it.
/// Repository paths a declared measurement names, in its command and in its result text.
///
/// A measurement is allowed to cite the file that records it. When that file is deleted the
/// citation becomes a claim about evidence that no longer exists, which is the failure this
/// extraction exists to make findable. A token counts as a workspace-relative path when it has
/// a directory component and a short file extension, and is neither absolute, nor a flag, nor a
/// URL — `cargo run --quiet -- tests --terse` yields nothing, `docs/roadmap/evidence/x.json`
/// yields itself.
fn cited_repository_paths(text: &str) -> Vec<&str> {
    text.split_whitespace()
        .map(|token| {
            token.trim_matches(|character: char| {
                matches!(character, ',' | ';' | ')' | '(' | '"' | '\'')
            })
        })
        .filter(|token| {
            !token.starts_with('-')
                && !token.starts_with('/')
                && !token.starts_with("http")
                && token.contains('/')
                && file_extension_of(token).is_some_and(|extension| {
                    (2..=4).contains(&extension.len())
                        && extension
                            .chars()
                            .all(|character| character.is_ascii_alphanumeric())
                })
        })
        .collect()
}

/// The extension of the final path segment, when it has one.
fn file_extension_of(token: &str) -> Option<&str> {
    let last_segment = token.rsplit('/').next()?;
    let (name, extension) = last_segment.rsplit_once('.')?;
    (!name.is_empty()).then_some(extension)
}

/// Checks the declared half of the ledger for the ways it can silently go stale.
///
/// The findings themselves are validated below. These are not measured by this audit — a suite
/// count comes from running the suite — so the only integrity this run can enforce is that each
/// one names something, names it once, and does not cite evidence that has been deleted. That
/// last check is what turns a closed campaign's leftover citation into a gate failure instead of
/// a sentence nobody re-reads.
fn check_declared_measurements(
    workspace_root: &Path,
    measurements: &[DeclaredMeasurement],
) -> Vec<LedgerIntegrityFinding> {
    let mut findings = Vec::new();
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();

    for measurement in measurements {
        let code = format!("declared_measurement '{}'", measurement.name);
        *seen.entry(measurement.name.as_str()).or_default() += 1;

        if measurement.name.trim().is_empty() {
            findings.push(LedgerIntegrityFinding {
                code: code.clone(),
                problem: "has no name".to_string(),
            });
        }
        for (field, value) in [
            ("command", &measurement.command),
            ("result", &measurement.result),
            ("recorded_by_phase", &measurement.recorded_by_phase),
        ] {
            if value.trim().is_empty() {
                findings.push(LedgerIntegrityFinding {
                    code: code.clone(),
                    problem: format!(
                        "has no {field}; a declared measurement that does not say what produced \
                         it, what it found, or who recorded it cannot be checked by anyone"
                    ),
                });
            }
        }

        for cited in cited_repository_paths(&measurement.command)
            .into_iter()
            .chain(cited_repository_paths(&measurement.result))
        {
            if !workspace_root.join(cited).exists() {
                findings.push(LedgerIntegrityFinding {
                    code: code.clone(),
                    problem: format!(
                        "cites '{cited}', which does not exist in this checkout; a measurement \
                         may not point at deleted evidence"
                    ),
                });
            }
        }
    }

    for (name, count) in seen {
        if count > 1 {
            findings.push(LedgerIntegrityFinding {
                code: format!("declared_measurement '{name}'"),
                problem: format!(
                    "is declared {count} times; a measurement name records one measurement"
                ),
            });
        }
    }

    findings
}

fn check_ledger_integrity(
    workspace_root: &Path,
    ledger: &HonestyLedger,
) -> Vec<LedgerIntegrityFinding> {
    let mut findings = check_declared_measurements(workspace_root, &ledger.declared_measurements);
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();

    for finding in &ledger.findings {
        *seen.entry(finding.code.as_str()).or_default() += 1;

        if finding.description.trim().is_empty() {
            findings.push(LedgerIntegrityFinding {
                code: finding.code.clone(),
                problem: "has no description".to_string(),
            });
        }
        if finding.disposition.trim().is_empty() {
            findings.push(LedgerIntegrityFinding {
                code: finding.code.clone(),
                problem: "has no disposition; every finding needs a recorded decision".to_string(),
            });
        }
        if finding.owning_phase.trim().is_empty() {
            findings.push(LedgerIntegrityFinding {
                code: finding.code.clone(),
                problem: "names no owning phase".to_string(),
            });
        }
        if !LEDGER_SEVERITIES.contains(&finding.severity.as_str()) {
            findings.push(LedgerIntegrityFinding {
                code: finding.code.clone(),
                problem: format!(
                    "has severity '{}', which is not one of {LEDGER_SEVERITIES:?}",
                    finding.severity
                ),
            });
        }
        if !LEDGER_STATUSES.contains(&finding.status.as_str()) {
            findings.push(LedgerIntegrityFinding {
                code: finding.code.clone(),
                problem: format!(
                    "has status '{}', which is not one of {LEDGER_STATUSES:?}",
                    finding.status
                ),
            });
        }
    }

    for (code, count) in seen {
        if count > 1 {
            findings.push(LedgerIntegrityFinding {
                code: code.to_string(),
                problem: format!("is declared {count} times; a finding code names one finding"),
            });
        }
    }

    findings
}

fn write_inventory(path: &Path, report: &HonestyInventoryReport) -> Result<(), String> {
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("failed to serialise the honesty inventory: {error}"))?;
    write_report_atomically(path, json.as_bytes())
}

/// Print the inventory in the order a reviewer reads it: what failed, then what was decided.
fn print_inventory(report: &HonestyInventoryReport) {
    println!(
        "=== honesty audit: {} files scanned, {} own tests ===",
        report.scanned_file_count, report.test_owning_file_count
    );

    println!("\nhard findings: {}", report.gate.scan_hard_finding_count);
    for finding in &report.hard_findings {
        println!("  [{}] {}", finding.code, finding.description);
        for site in &finding.sites {
            println!("    {}:{} {}", site.file, site.line, site.source);
        }
    }

    println!("\nreview findings (disposition recorded, not a failure):");
    for finding in &report.review_findings {
        println!(
            "  {:<40} {:>5} occurrence(s) in {:>3} file(s)  {:?}",
            finding.code,
            finding.occurrences,
            finding.files.len(),
            finding.disposition
        );
    }

    println!("\ncomposed audits:");
    println!(
        "  source-audit          {} finding(s)",
        report.composed.source_audit.finding_count
    );
    for finding in &report.composed.source_audit.findings {
        println!("    {finding}");
    }
    println!(
        "  feature-lane coverage {} finding(s)",
        report.composed.feature_lane_coverage.finding_count
    );
    for finding in &report.composed.feature_lane_coverage.findings {
        println!("    {finding}");
    }
    print_suite_inventory(&report.composed.integration_suite_inventory);
    for review in &report.composed.suite_weak_contract_reviews {
        println!(
            "    {:<38} {:>5}  {:?}",
            review.counter,
            review
                .occurrences
                .map_or("n/a".to_string(), |count| count.to_string()),
            review.disposition
        );
    }

    println!(
        "\ndeclared ledger: {} finding(s), {} open hard, {} integrity finding(s)",
        report.declared_findings.len(),
        report.gate.open_hard_ledger_finding_count,
        report.gate.ledger_integrity_finding_count
    );
    for finding in &report.ledger_integrity_findings {
        println!("  {}: {}", finding.code, finding.problem);
    }

    println!("\ninventory written: {HONESTY_INVENTORY_REPORT_PATH}");
}

fn print_suite_inventory(evidence: &SuiteInventoryEvidence) {
    match evidence {
        SuiteInventoryEvidence::Absent { path } => println!(
            "  suite inventory       absent at {path}; run `cargo run -- tests --audit` first"
        ),
        SuiteInventoryEvidence::Unusable { path, reason } => {
            println!("  suite inventory       unusable at {path}: {reason}");
        }
        SuiteInventoryEvidence::Present {
            run_completed,
            hard_policy_violation_count,
            advisory_finding_count,
            ..
        } => println!(
            "  suite inventory       {} run, {hard_policy_violation_count} hard policy \
             violation(s), {advisory_finding_count} advisory finding(s)",
            if *run_completed {
                "completed"
            } else {
                "INCOMPLETE"
            }
        ),
    }
}

/// Format a moment as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Written out rather than pulled in: a date dependency for one field in one report is a cost the
/// next reader pays, and the civil-date arithmetic below is proved by its own tests.
fn utc_timestamp(moment: SystemTime) -> String {
    let seconds = moment
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());

    let days = (seconds / 86_400) as i64;
    let second_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);

    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        second_of_day / 3600,
        (second_of_day % 3600) / 60,
        second_of_day % 60
    )
}

/// The civil date `days` after 1970-01-01, by Howard Hinnant's `civil_from_days`.
///
/// The algorithm shifts the epoch to 0000-03-01 so a leap day lands at the end of a year, which
/// is what removes every special case from the month arithmetic.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;

    (year + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests;
