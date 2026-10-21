//! Frozen diagnostic reports and canonical runs across identity domains.
//!
//! WHAT: a report owns one compact store, its type-display data and the frozen identity needed to
//! render it. A report set orders ranges from those reports without copying their records.
//! WHY: canonical warning and error passes can interleave independent domains while each stored
//! record remains paired with the context that issued its compact ids.

use super::store::{DiagnosticId, DiagnosticStore};
use super::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot::TypeDisplaySnapshot;
use crate::compiler_frontend::source::FrozenIdentityContext;
use std::sync::Arc;

/// The number of stored diagnostics at each resolved severity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SeverityCounts {
    pub(crate) errors: u32,
    pub(crate) warnings: u32,
    pub(crate) notes: u32,
}

/// One frozen diagnostic identity domain and the compact records produced in it.
#[derive(Debug)]
pub(crate) struct DiagnosticReport {
    context: Arc<FrozenIdentityContext>,
    diagnostics: DiagnosticStore,
    types: TypeDisplaySnapshot,
    counts: SeverityCounts,
}

impl DiagnosticReport {
    /// Build a report and resolve its severity counts once from the stored records.
    pub(super) fn new(
        context: Arc<FrozenIdentityContext>,
        diagnostics: DiagnosticStore,
        types: TypeDisplaySnapshot,
    ) -> Self {
        let mut counts = SeverityCounts::default();
        for index in 0..diagnostics.records.len() {
            let id = DiagnosticId::from_index(index);
            let view = DiagnosticView::new(&diagnostics, id, &types);
            let count = match view.severity() {
                DiagnosticSeverity::Error => &mut counts.errors,
                DiagnosticSeverity::Warning => &mut counts.warnings,
                DiagnosticSeverity::Note => &mut counts.notes,
            };
            *count = (*count)
                .checked_add(1)
                .expect("a report's severity counts fit in u32");
        }

        Self {
            context,
            diagnostics,
            types,
            counts,
        }
    }

    /// The resolved severity counts computed when this report was built.
    pub(crate) fn counts(&self) -> SeverityCounts {
        self.counts
    }

    /// Number of stored diagnostics in production order.
    pub(crate) fn len(&self) -> usize {
        self.diagnostics.records.len()
    }

    /// The immutable identity context that owns this report's compact ids.
    pub(crate) fn context(&self) -> &FrozenIdentityContext {
        &self.context
    }

    /// A shared handle to this report's identity context, for infrastructure cause rows.
    pub(crate) fn shared_context(&self) -> Arc<FrozenIdentityContext> {
        Arc::clone(&self.context)
    }

    /// Every diagnostic in this report's production order, without allocating.
    #[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3.3a take functions"))]
    pub(crate) fn iter(&self) -> impl Iterator<Item = DiagnosticView<'_>> + '_ {
        (0..self.len()).map(move |index| self.view_at(index))
    }

    /// Diagnostics of `severity` in production order, without allocating.
    #[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3.3a take functions"))]
    pub(crate) fn iter_severity(
        &self,
        severity: DiagnosticSeverity,
    ) -> impl Iterator<Item = DiagnosticView<'_>> + '_ {
        self.iter().filter(move |view| view.severity() == severity)
    }

    fn view_at(&self, index: usize) -> DiagnosticView<'_> {
        DiagnosticView::new(
            &self.diagnostics,
            DiagnosticId::from_index(index),
            &self.types,
        )
    }
}

/// One contiguous run of a report's records in canonical boundary order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiagnosticRun {
    pub(super) report: u32,
    pub(super) first: u32,
    pub(super) len: u32,
}

const _: () = assert!(std::mem::size_of::<DiagnosticRun>() == 12);

/// Move-only ordered reports and the runs that preserve their canonical cross-domain order.
#[derive(Debug)]
pub(crate) struct DiagnosticReportSet {
    reports: Vec<DiagnosticReport>,
    runs: Vec<DiagnosticRun>,
}

const _: () = assert!(std::mem::size_of::<DiagnosticReportSet>() <= 48);

impl DiagnosticReportSet {
    /// Construct a report set after validating its producer-built run index.
    pub(super) fn new(reports: Vec<DiagnosticReport>, runs: Vec<DiagnosticRun>) -> Self {
        let mut next_records = vec![0_u32; reports.len()];

        for run in &runs {
            let report_index = run.report as usize;
            let report = reports
                .get(report_index)
                .expect("diagnostic run refers to a report that does not exist");
            let report_len =
                u32::try_from(report.len()).expect("diagnostic report record counts fit in u32");
            let end = run
                .first
                .checked_add(run.len)
                .expect("diagnostic run end index fits in u32");
            assert!(
                end <= report_len,
                "diagnostic run extends beyond its report's records"
            );

            let expected_first = &mut next_records[report_index];
            assert_eq!(
                run.first, *expected_first,
                "diagnostic runs must cover each report's records exactly once in ascending order"
            );
            *expected_first = end;
        }

        for (report_index, report) in reports.iter().enumerate() {
            let report_len =
                u32::try_from(report.len()).expect("diagnostic report record counts fit in u32");
            assert_eq!(
                next_records[report_index], report_len,
                "diagnostic runs must cover every report record exactly once"
            );
        }

        Self { reports, runs }
    }

    /// An empty set for a clean success. It intentionally owns no report or identity context.
    #[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3.3b build_project"))]
    pub(crate) fn empty() -> Self {
        Self {
            reports: Vec::new(),
            runs: Vec::new(),
        }
    }

    /// Sum of each report's severity counts.
    pub(crate) fn counts(&self) -> SeverityCounts {
        let mut total = SeverityCounts::default();
        for report in &self.reports {
            let counts = report.counts();
            total.errors = total
                .errors
                .checked_add(counts.errors)
                .expect("report-set error counts fit in u32");
            total.warnings = total
                .warnings
                .checked_add(counts.warnings)
                .expect("report-set warning counts fit in u32");
            total.notes = total
                .notes
                .checked_add(counts.notes)
                .expect("report-set note counts fit in u32");
        }
        total
    }

    /// Whether this set contains no diagnostic records.
    pub(crate) fn is_empty(&self) -> bool {
        self.runs.iter().all(|run| run.len == 0)
    }

    /// The report that owns domain `index`; report indices follow domain registration order.
    pub(crate) fn report(&self, index: u32) -> &DiagnosticReport {
        &self.reports[index as usize]
    }
    /// Every retained identity context, including domains with no records.
    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn contexts(&self) -> impl Iterator<Item = &FrozenIdentityContext> {
        self.reports.iter().map(DiagnosticReport::context)
    }

    /// All records in canonical run order, paired with the report that owns each view.
    pub(crate) fn iter(
        &self,
    ) -> impl Iterator<Item = (&DiagnosticReport, DiagnosticView<'_>)> + '_ {
        self.runs.iter().flat_map(move |run| {
            let report_index = run.report as usize;
            let report = &self.reports[report_index];
            let first = run.first as usize;
            let end = (run.first + run.len) as usize;
            (first..end).map(move |index| (report, report.view_at(index)))
        })
    }

    /// Records of `severity` in canonical run order, paired with their owning reports.
    pub(crate) fn iter_severity(
        &self,
        severity: DiagnosticSeverity,
    ) -> impl Iterator<Item = (&DiagnosticReport, DiagnosticView<'_>)> + '_ {
        self.iter()
            .filter(move |(_, view)| view.severity() == severity)
    }
}
