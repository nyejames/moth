//! Structured human timing summary model.
//!
//! WHAT: builds a typed, architecture-ordered report from dense timing
//!      aggregates so the basic report never infers meaning from dotted
//!      metric-name prefixes.
//! WHY:  presentation policy belongs in one static descriptor table; unknown
//!       typed metrics stay available to detailed and benchmark output but never
//!       appear in the basic report by accident.

use super::schema::{TIMING_METRIC_COUNT, TimingAccountingRole, TimingLevel, TimingParent};
use super::session::TimingSessionId;
use super::{
    BenchmarkObservationSnapshot, TimingBoundaryId, TimingCommandKind, TimingMetric,
    TimingModuleKey,
};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::time::Duration;

type TimingBoundaryMapKey = (TimingSessionId, usize);
type TimingModuleMapKey = (TimingSessionId, usize, u32);

fn boundary_map_key(boundary: TimingBoundaryId) -> TimingBoundaryMapKey {
    (boundary.session(), boundary.index())
}

fn module_map_key(module: TimingModuleKey) -> TimingModuleMapKey {
    let boundary = module.boundary();
    (boundary.session(), boundary.index(), module.module_index())
}

/// One display row in a summary section.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TimingSummaryRow {
    pub(crate) label: Cow<'static, str>,
    pub(crate) total: Duration,
    pub(crate) children: Vec<TimingSummaryRow>,
}

/// One titled group of rows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TimingSummarySection {
    pub(crate) title: String,
    pub(crate) rows: Vec<TimingSummaryRow>,
}

/// One source-package or main-project boundary row.
///
/// Phase 4 registers and populates these. The model owns the shape now so
/// boundary attribution never leaks through generic row metadata or
/// metric-name prefixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimingBoundarySummary {
    pub(crate) label: Cow<'static, str>,
    pub(crate) module_count: u64,
    pub(crate) total: Duration,
}

/// The single slowest-module attribution row.
///
/// Phase 4 computes module work as preparation attributed to the module plus
/// `frontend.module.semantic_total`. The model owns the shape now so
/// absolute filesystem paths cannot appear in basic output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimingSlowestModuleSummary {
    pub(crate) identity: Cow<'static, str>,
    pub(crate) source_file_count: u64,
    pub(crate) source_byte_count: u64,
    pub(crate) total: Duration,
}

/// The complete basic-mode report.
#[derive(Debug, Clone)]
pub(crate) struct TimingSummaryReport {
    /// Heading text without the duration, for example `Build timings` or
    /// `Build timings · failed`. The renderer prints the duration
    /// separately in the total colour role.
    pub(crate) title: String,
    pub(crate) command_total: Duration,
    /// Internal policy evidence retained for tests and debug assertions.
    pub(crate) accounting_issue: Option<TimingAccountingIssue>,
    /// Top-level report items in the accepted display order: pipeline,
    /// compilation boundaries, frontend, backend, slowest module.
    pub(crate) items: Vec<TimingReportItem>,
}

/// An internal timing-policy invariant that prevents a misleading `Other` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimingAccountingIssue {
    /// A wall/accounting span was recorded more than once in one command.
    DuplicateSpan { metric: TimingMetric, samples: u64 },
    /// The disjoint pipeline spans exceed the owning command total.
    OverAccounted {
        accounted: Duration,
        command_total: Duration,
    },
}

/// One ordered top-level report item.
///
/// The renderer walks this list verbatim, so the accepted section order is
/// owned by the model rather than by renderer field ordering.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TimingReportItem {
    Section(TimingSummarySection),
    CompilationBoundaries(Vec<TimingBoundarySummary>),
    SlowestModule(TimingSlowestModuleSummary),
}

/// One typed metric policy for a basic report row.
struct MetricPolicy {
    /// One or more typed metrics summed into this row. Multiple metrics are
    /// used when a human row owns disjoint evidence, for example direct
    /// borrow-check calls or generated-function materialisation plus its
    /// sidecar borrow rechecks.
    metrics: &'static [TimingMetric],
    label: &'static str,
    section: SectionId,
    /// Nested evidence rows shown only when they pass the child threshold.
    children: &'static [MetricChildPolicy],
}

/// One nested or grouped child row inside a parent policy row.
struct MetricChildPolicy {
    metric: TimingMetric,
    label: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionId {
    Pipeline,
    Frontend,
    Backend,
}

/// The single owner of basic-mode display labels and row grouping.
///
/// Metric identity, relation, parent, command applicability and accounting
/// role remain owned by the typed schema descriptor. Rows appear in table
/// order, which is the architecture order; metrics absent from this policy
/// never appear in basic output.
const BASIC_METRIC_POLICY: &[MetricPolicy] = &[
    MetricPolicy {
        metrics: &[TimingMetric::BuildBootstrapTotal],
        label: "Bootstrap",
        section: SectionId::Pipeline,
        children: &[],
    },
    MetricPolicy {
        metrics: &[TimingMetric::BuildFrontendTotal],
        label: "Frontend",
        section: SectionId::Pipeline,
        children: &[
            MetricChildPolicy {
                metric: TimingMetric::Stage0DirectoryInventory,
                label: "Directory inventory",
            },
            MetricChildPolicy {
                metric: TimingMetric::Stage0DirectoryCompile,
                label: "Directory compile",
            },
            MetricChildPolicy {
                metric: TimingMetric::Stage0SingleFileTotal,
                label: "Single-file frontend",
            },
        ],
    },
    MetricPolicy {
        metrics: &[TimingMetric::BuildBackendTotal],
        label: "Backend",
        section: SectionId::Pipeline,
        children: &[],
    },
    MetricPolicy {
        metrics: &[TimingMetric::BuildOutputTotal],
        label: "Output",
        section: SectionId::Pipeline,
        children: &[],
    },
    MetricPolicy {
        metrics: &[TimingMetric::FrontendPrepare],
        label: "Prepare source files",
        section: SectionId::Frontend,
        children: &[],
    },
    MetricPolicy {
        metrics: &[TimingMetric::FrontendBindHeaders],
        label: "Bind headers",
        section: SectionId::Frontend,
        children: &[],
    },
    MetricPolicy {
        metrics: &[TimingMetric::FrontendOrderDeclarations],
        label: "Order declarations",
        section: SectionId::Frontend,
        children: &[],
    },
    MetricPolicy {
        metrics: &[TimingMetric::FrontendAstTotal],
        label: "Semantic frontend / AST",
        section: SectionId::Frontend,
        children: &[
            MetricChildPolicy {
                metric: TimingMetric::FrontendAstEnvironment,
                label: "Environment, types and constants",
            },
            MetricChildPolicy {
                metric: TimingMetric::FrontendAstEmit,
                label: "Bodies and TIR construction",
            },
            MetricChildPolicy {
                metric: TimingMetric::FrontendAstFinalise,
                label: "Template and constant finalisation",
            },
        ],
    },
    MetricPolicy {
        metrics: &[
            TimingMetric::FrontendPublicInterfaceProject,
            TimingMetric::FrontendPublicInterfaceFinalise,
        ],
        label: "Public interface",
        section: SectionId::Frontend,
        children: &[
            MetricChildPolicy {
                metric: TimingMetric::FrontendPublicInterfaceProject,
                label: "Projection",
            },
            MetricChildPolicy {
                metric: TimingMetric::FrontendPublicInterfaceFinalise,
                label: "Finalisation",
            },
        ],
    },
    MetricPolicy {
        metrics: &[TimingMetric::FrontendHir],
        label: "HIR",
        section: SectionId::Frontend,
        children: &[],
    },
    MetricPolicy {
        metrics: &[
            TimingMetric::FrontendBorrowInitial,
            TimingMetric::FrontendBorrowConverge,
        ],
        label: "Borrow validation",
        section: SectionId::Frontend,
        children: &[],
    },
    MetricPolicy {
        metrics: &[
            TimingMetric::FrontendGeneratedMaterialise,
            TimingMetric::FrontendGeneratedBorrowRecheck,
        ],
        label: "Generated functions",
        section: SectionId::Frontend,
        children: &[],
    },
    MetricPolicy {
        metrics: &[
            TimingMetric::BackendJsLowerEntry,
            TimingMetric::BackendJsLowerLinked,
        ],
        label: "JS lowering",
        section: SectionId::Backend,
        children: &[],
    },
    MetricPolicy {
        metrics: &[TimingMetric::BackendHtmlRender],
        label: "HTML rendering",
        section: SectionId::Backend,
        children: &[],
    },
    MetricPolicy {
        metrics: &[TimingMetric::BackendWasmTotal],
        label: "Wasm build",
        section: SectionId::Backend,
        children: &[],
    },
    MetricPolicy {
        metrics: &[
            TimingMetric::BackendAssetsPlan,
            TimingMetric::BackendAssetsEmit,
        ],
        label: "Tracked assets",
        section: SectionId::Backend,
        children: &[],
    },
];

fn policy_applies_to_command(policy: &MetricPolicy, command: TimingCommandKind) -> bool {
    policy
        .metrics
        .iter()
        .copied()
        .all(|metric| metric.applies_to(command))
}

fn metric_policy_is_valid() -> bool {
    let mut seen_rows = [false; TIMING_METRIC_COUNT];
    for policy in BASIC_METRIC_POLICY {
        let Some(first_metric) = policy.metrics.first().copied() else {
            return false;
        };
        let first_descriptor = first_metric.descriptor();
        if first_descriptor.level != TimingLevel::Basic {
            return false;
        }
        for metric in policy.metrics.iter().copied() {
            let descriptor = metric.descriptor();
            if seen_rows[metric.index()] {
                return false;
            }
            seen_rows[metric.index()] = true;
            if descriptor.level != TimingLevel::Basic
                || descriptor.relation != first_descriptor.relation
                || descriptor.command_scope != first_descriptor.command_scope
            {
                return false;
            }
        }
        for child in policy.children {
            let descriptor = child.metric.descriptor();
            let valid_parent = match descriptor.parent {
                Some(TimingParent::Metric(parent)) => parent == first_metric,
                Some(TimingParent::SummaryGroup(group)) => {
                    policy.metrics.contains(&child.metric)
                        && policy.metrics.iter().all(|metric| {
                            metric.descriptor().parent == Some(TimingParent::SummaryGroup(group))
                        })
                }
                None => false,
            };
            if descriptor.level != TimingLevel::Basic || !valid_parent {
                return false;
            }
        }
    }
    true
}

/// Dense typed totals used only while constructing a report.
#[derive(Clone, Copy)]
struct MetricTotal {
    total: Duration,
    samples: u64,
}

impl MetricTotal {
    const fn new() -> Self {
        Self {
            total: Duration::ZERO,
            samples: 0,
        }
    }
}

struct MetricTotals {
    values: [MetricTotal; TIMING_METRIC_COUNT],
}

impl MetricTotals {
    fn new() -> Self {
        Self {
            values: [const { MetricTotal::new() }; TIMING_METRIC_COUNT],
        }
    }

    fn from_snapshot(snapshot: &BenchmarkObservationSnapshot) -> Self {
        let mut totals = Self::new();
        for aggregate in &snapshot.timings {
            if aggregate.samples == 0 {
                continue;
            }
            let slot = &mut totals.values[aggregate.metric.index()];
            slot.total += aggregate.total;
            slot.samples = slot.samples.saturating_add(aggregate.samples);
        }
        totals
    }

    fn add(&mut self, metric: TimingMetric, total: Duration, samples: u64) {
        let slot = &mut self.values[metric.index()];
        slot.total += total;
        slot.samples = slot.samples.saturating_add(samples);
    }

    fn total(&self, metric: TimingMetric) -> Duration {
        self.values[metric.index()].total
    }

    fn samples(&self, metric: TimingMetric) -> u64 {
        self.values[metric.index()].samples
    }
}

/// Build a deterministic basic report from one drained snapshot.
pub(crate) fn build_timing_summary(
    snapshot: &BenchmarkObservationSnapshot,
    command: TimingCommandKind,
    succeeded: bool,
) -> TimingSummaryReport {
    debug_assert!(
        metric_policy_is_valid(),
        "typed timing summary policy is invalid"
    );
    let aggregates = aggregate_by_metric(snapshot);
    let command_total = command_total(&aggregates, command);
    let accounted = accounted_pipeline_total(&aggregates, command);
    let accounting_issue = accounting_issue(&aggregates, command, command_total, accounted);

    let mut items = Vec::new();
    if let Some(section) = build_pipeline_section(
        &aggregates,
        command,
        command_total,
        accounted,
        accounting_issue.is_none(),
    ) {
        items.push(TimingReportItem::Section(section));
    }
    let boundaries = build_boundary_summaries(snapshot);
    if !boundaries.is_empty() {
        items.push(TimingReportItem::CompilationBoundaries(boundaries));
    }
    if let Some(section) = build_frontend_section(&aggregates, snapshot, command) {
        items.push(TimingReportItem::Section(section));
    }
    if let Some(section) = build_backend_section(&aggregates, command) {
        items.push(TimingReportItem::Section(section));
    }
    if let Some(slowest_module) = build_slowest_module_summary(snapshot) {
        items.push(TimingReportItem::SlowestModule(slowest_module));
    }

    let command_word = match command {
        TimingCommandKind::Build => "Build",
        TimingCommandKind::Check => "Check",
        TimingCommandKind::Dev => "Dev",
    };
    let title = if succeeded {
        format!("{command_word} timings")
    } else {
        format!("{command_word} timings · failed")
    };

    TimingSummaryReport {
        title,
        command_total,
        accounting_issue,
        items,
    }
}

/// Aggregate dense typed rows by schema identity. Zero-sample rows add no time.
fn aggregate_by_metric(snapshot: &BenchmarkObservationSnapshot) -> MetricTotals {
    MetricTotals::from_snapshot(snapshot)
}

fn command_total(aggregates: &MetricTotals, command: TimingCommandKind) -> Duration {
    TimingMetric::command_total(command)
        .map(|metric| aggregates.total(metric))
        .unwrap_or_default()
}

fn accounted_pipeline_total(aggregates: &MetricTotals, command: TimingCommandKind) -> Duration {
    TimingMetric::ALL
        .iter()
        .copied()
        .filter(|metric| {
            metric.applies_to(command)
                && matches!(
                    metric.descriptor().accounting,
                    TimingAccountingRole::Pipeline(_)
                )
        })
        .map(|metric| aggregates.total(metric))
        .sum()
}

fn accounting_issue(
    aggregates: &MetricTotals,
    command: TimingCommandKind,
    command_total: Duration,
    accounted: Duration,
) -> Option<TimingAccountingIssue> {
    for metric in TimingMetric::ALL.iter().copied().filter(|metric| {
        metric.applies_to(command)
            && matches!(
                metric.descriptor().accounting,
                TimingAccountingRole::CommandTotal | TimingAccountingRole::Pipeline(_)
            )
    }) {
        let samples = aggregates.samples(metric);
        if samples > 1 {
            return Some(TimingAccountingIssue::DuplicateSpan { metric, samples });
        }
    }

    (accounted > command_total).then_some(TimingAccountingIssue::OverAccounted {
        accounted,
        command_total,
    })
}

/// Sum boundary inventory and compile observations per boundary.
///
/// Package inventory and package compilation are disjoint passes, so the
/// human boundary total is accumulated work, never one contiguous wall span.
fn aggregate_boundary_totals(
    snapshot: &BenchmarkObservationSnapshot,
) -> BTreeMap<TimingBoundaryMapKey, Duration> {
    let mut totals = BTreeMap::new();
    for boundary in &snapshot.boundaries {
        for aggregate in &boundary.timings {
            if matches!(
                aggregate.metric,
                TimingMetric::BoundaryInventory | TimingMetric::BoundaryCompile
            ) && aggregate.samples > 0
            {
                *totals
                    .entry(boundary_map_key(boundary.id))
                    .or_insert(Duration::ZERO) += aggregate.total;
            }
        }
    }
    totals
}

/// Build boundary rows in deterministic registration order.
///
/// Registration order is the graph's deterministic package order followed by
/// the main project, so the display order never depends on event insertion.
fn build_boundary_summaries(snapshot: &BenchmarkObservationSnapshot) -> Vec<TimingBoundarySummary> {
    let totals = aggregate_boundary_totals(snapshot);
    let mut rows = Vec::new();

    for record in &snapshot.boundaries {
        let Some(total) = totals.get(&boundary_map_key(record.id)).copied() else {
            continue;
        };
        if rounds_to_zero(total) {
            continue;
        }
        rows.push(TimingBoundarySummary {
            label: Cow::Owned(record.display_name.clone()),
            module_count: record.module_count,
            total,
        });
    }

    rows
}

/// Build the single slowest-module row from registered module metadata.
///
/// Module work is source preparation attributed to the module plus
/// `frontend.module.semantic_total`; both aggregates are keyed by the compact
/// module key, so shuffled event insertion cannot change the winner. The
/// earliest registered module wins ties, keeping output deterministic.
fn build_slowest_module_summary(
    snapshot: &BenchmarkObservationSnapshot,
) -> Option<TimingSlowestModuleSummary> {
    let mut preparation = BTreeMap::<TimingModuleMapKey, Duration>::new();
    let mut semantic_total = BTreeMap::<TimingModuleMapKey, Duration>::new();

    for record in &snapshot.modules {
        // A failed Stage 0 preparation can leave a registered placeholder with
        // partial timing aggregates. Its source metadata is not a completed
        // module fact, so keep it out of both the evidence and winner passes.
        if !record.source_facts_finalized {
            continue;
        }
        for aggregate in &record.timings {
            match aggregate.metric {
                TimingMetric::FrontendPrepare if aggregate.samples > 0 => {
                    *preparation
                        .entry(module_map_key(record.key))
                        .or_insert(Duration::ZERO) += aggregate.total;
                }
                TimingMetric::FrontendModuleSemanticTotal if aggregate.samples > 0 => {
                    *semantic_total
                        .entry(module_map_key(record.key))
                        .or_insert(Duration::ZERO) += aggregate.total;
                }
                _ => {}
            }
        }
    }

    let mut slowest: Option<TimingSlowestModuleSummary> = None;
    for record in &snapshot.modules {
        if !record.source_facts_finalized {
            continue;
        }
        let module_key = module_map_key(record.key);
        let total = preparation.get(&module_key).copied().unwrap_or_default()
            + semantic_total.get(&module_key).copied().unwrap_or_default();
        if rounds_to_zero(total) {
            continue;
        }
        let is_slowest = slowest
            .as_ref()
            .is_none_or(|current: &TimingSlowestModuleSummary| total > current.total);
        if is_slowest {
            slowest = Some(TimingSlowestModuleSummary {
                identity: Cow::Owned(record.logical_identity.clone()),
                source_file_count: record.source_file_count,
                source_byte_count: record.source_byte_count,
                total,
            });
        }
    }

    slowest
}

fn build_pipeline_section(
    aggregates: &MetricTotals,
    command: TimingCommandKind,
    command_total: Duration,
    accounted: Duration,
    accounting_is_valid: bool,
) -> Option<TimingSummarySection> {
    let mut rows = Vec::new();
    for policy in BASIC_METRIC_POLICY.iter().filter(|policy| {
        policy.section == SectionId::Pipeline && policy_applies_to_command(policy, command)
    }) {
        push_policy_row(&mut rows, aggregates, &MetricTotals::new(), policy);
    }

    if accounting_is_valid {
        push_other_row(&mut rows, command_total, accounted);
    }

    if rows.is_empty() {
        return None;
    }

    Some(TimingSummarySection {
        title: match command {
            TimingCommandKind::Build => "Build pipeline",
            TimingCommandKind::Check => "Check pipeline",
            TimingCommandKind::Dev => "Dev pipeline",
        }
        .to_owned(),
        rows,
    })
}

fn push_policy_row(
    rows: &mut Vec<TimingSummaryRow>,
    aggregates: &MetricTotals,
    module_ast_children: &MetricTotals,
    policy: &MetricPolicy,
) {
    let total = policy
        .metrics
        .iter()
        .map(|metric| aggregates.total(*metric))
        .sum::<Duration>();
    if rounds_to_zero(total) {
        return;
    }

    let mut children = Vec::new();
    for child in policy.children {
        let child_aggregates = if child.metric.descriptor().parent
            == Some(TimingParent::Metric(TimingMetric::FrontendAstTotal))
        {
            module_ast_children
        } else {
            aggregates
        };
        let child_total = child_aggregates.total(child.metric);
        if is_significant_child(child_total, total) {
            children.push(TimingSummaryRow {
                label: Cow::Borrowed(child.label),
                total: child_total,
                children: Vec::new(),
            });
        }
    }

    rows.push(TimingSummaryRow {
        label: Cow::Borrowed(policy.label),
        total,
        children,
    });
}

/// Compute the bounded `Other` row from typed pipeline spans only.
fn push_other_row(rows: &mut Vec<TimingSummaryRow>, command_total: Duration, accounted: Duration) {
    let Some(other) = command_total.checked_sub(accounted) else {
        return;
    };
    let other_ms = other.as_secs_f64() * 1000.0;
    let command_ms = command_total.as_secs_f64() * 1000.0;
    let significant = other_ms >= 1.0 || (command_ms > 0.0 && other_ms >= command_ms * 0.02);

    if significant && !rounds_to_zero(other) {
        rows.push(TimingSummaryRow {
            label: Cow::Borrowed("Other"),
            total: other,
            children: Vec::new(),
        });
    }
}

/// Aggregate the AST child metrics from module-attributed dense rows only.
///
/// Config parsing and generated materialisation use separate schema-v1 AST
/// identities, so only frontend module child metrics may appear here.
fn aggregate_module_ast_children(snapshot: &BenchmarkObservationSnapshot) -> MetricTotals {
    let mut aggregates = MetricTotals::new();
    for module in &snapshot.modules {
        for aggregate in &module.timings {
            if aggregate.metric.descriptor().parent
                == Some(TimingParent::Metric(TimingMetric::FrontendAstTotal))
                && aggregate.samples > 0
            {
                aggregates.add(aggregate.metric, aggregate.total, aggregate.samples);
            }
        }
    }
    aggregates
}

fn build_frontend_section(
    aggregates: &MetricTotals,
    snapshot: &BenchmarkObservationSnapshot,
    command: TimingCommandKind,
) -> Option<TimingSummarySection> {
    let mut rows = Vec::new();
    let module_ast_children = aggregate_module_ast_children(snapshot);

    for policy in BASIC_METRIC_POLICY.iter().filter(|policy| {
        policy.section == SectionId::Frontend && policy_applies_to_command(policy, command)
    }) {
        push_policy_row(&mut rows, aggregates, &module_ast_children, policy);
    }

    if rows.is_empty() {
        return None;
    }

    let module_count = snapshot.modules.len();
    let title = if module_count > 0 {
        let module_word = if module_count == 1 {
            "module"
        } else {
            "modules"
        };
        format!("Frontend work · {module_count} {module_word} · accumulated")
    } else {
        "Frontend work · accumulated".to_owned()
    };

    Some(TimingSummarySection { title, rows })
}

fn build_backend_section(
    aggregates: &MetricTotals,
    command: TimingCommandKind,
) -> Option<TimingSummarySection> {
    if !TimingMetric::BuildBackendTotal.applies_to(command) {
        return None;
    }
    let parent = aggregates.total(TimingMetric::BuildBackendTotal);
    if rounds_to_zero(parent) {
        return None;
    }

    let mut children = Vec::new();
    for policy in BASIC_METRIC_POLICY.iter().filter(|policy| {
        policy.section == SectionId::Backend && policy_applies_to_command(policy, command)
    }) {
        push_significant_child(&mut children, aggregates, policy, parent);
    }

    if children.is_empty() {
        return None;
    }

    Some(TimingSummarySection {
        title: "Backend".to_owned(),
        rows: children,
    })
}

/// Show a nested child only when it is at least 1ms and 5% of its parent.
fn push_significant_child(
    rows: &mut Vec<TimingSummaryRow>,
    aggregates: &MetricTotals,
    policy: &MetricPolicy,
    parent_total: Duration,
) {
    let total = policy
        .metrics
        .iter()
        .map(|metric| aggregates.total(*metric))
        .sum::<Duration>();

    if is_significant_child(total, parent_total) {
        rows.push(TimingSummaryRow {
            label: Cow::Borrowed(policy.label),
            total,
            children: Vec::new(),
        });
    }
}

/// Whether a nested child is at least 1ms and 5% of its parent.
fn is_significant_child(child: Duration, parent: Duration) -> bool {
    let child_ms = child.as_secs_f64() * 1000.0;
    let parent_ms = parent.as_secs_f64() * 1000.0;
    child_ms >= 1.0 && (parent_ms > 0.0 && child_ms >= parent_ms * 0.05) && !rounds_to_zero(child)
}

/// Whether a duration would render as `0.00ms` after two-decimal rounding.
fn rounds_to_zero(duration: Duration) -> bool {
    duration.as_secs_f64() * 1000.0 < 0.005
}
