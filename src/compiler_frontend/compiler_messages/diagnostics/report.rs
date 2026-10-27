//! Frozen diagnostic reports and canonical runs across identity domains.
//!
//! WHAT: a report owns one compact store, its foreign identity contexts, type-display data and
//! the frozen identity needed to render it. A report set orders ranges from those reports without
//! copying their records.
//! WHY: canonical warning and error passes can interleave independent domains while each stored
//! record remains paired with the context that issued its compact ids.

use super::families::token_lexical::COMPILER_CAPACITY_EXCEEDED;
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
    /// Report-owned context slots indexed by foreign-site rows; `None` preserves unresolved foreign semantics.
    foreign_contexts: Vec<Option<Arc<FrozenIdentityContext>>>,
    diagnostics: DiagnosticStore,
    types: TypeDisplaySnapshot,
    counts: SeverityCounts,
}

impl DiagnosticReport {
    /// Build a report and resolve its severity counts once from the stored records.
    pub(super) fn new(
        context: Arc<FrozenIdentityContext>,
        diagnostics: DiagnosticStore,
        foreign_contexts: Vec<Option<Arc<FrozenIdentityContext>>>,
        types: TypeDisplaySnapshot,
    ) -> Self {
        let mut counts = SeverityCounts::default();
        for index in 0..diagnostics.records.len() {
            let id = DiagnosticId::from_index(index);
            let view = DiagnosticView::new(&diagnostics, id, &types, &foreign_contexts);
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
            foreign_contexts,
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

    /// A shared handle to this report's identity context.
    #[cfg(test)]
    pub(crate) fn shared_context(&self) -> Arc<FrozenIdentityContext> {
        Arc::clone(&self.context)
    }

    fn view_at(&self, index: usize) -> DiagnosticView<'_> {
        DiagnosticView::new(
            &self.diagnostics,
            DiagnosticId::from_index(index),
            &self.types,
            &self.foreign_contexts,
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
pub struct DiagnosticReportSet {
    reports: Vec<DiagnosticReport>,
    runs: Vec<DiagnosticRun>,
}

const _: () = assert!(std::mem::size_of::<DiagnosticReportSet>() <= 48);

#[cfg(any(test, feature = "data_layout_memory_probe"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiagnosticReportMemoryMetrics {
    pub(crate) report_rows: usize,
    pub(crate) report_capacity_bytes: usize,
    pub(crate) run_rows: usize,
    pub(crate) run_capacity_bytes: usize,
    pub(crate) foreign_context_rows: usize,
    pub(crate) foreign_context_capacity_bytes: usize,
    pub(crate) record_rows: usize,
    pub(crate) record_capacity_bytes: usize,
    pub(crate) extra_rows: usize,
    pub(crate) extra_capacity_bytes: usize,
    pub(crate) label_rows: usize,
    pub(crate) label_capacity_bytes: usize,
    pub(crate) label_data_rows: usize,
    pub(crate) label_data_capacity_bytes: usize,
    pub(crate) string_list_rows: usize,
    pub(crate) string_list_capacity_bytes: usize,
    pub(crate) substitution_rows: usize,
    pub(crate) substitution_capacity_bytes: usize,
    pub(crate) foreign_site_rows: usize,
    pub(crate) foreign_site_capacity_bytes: usize,
    pub(crate) type_display_record_rows: usize,
    pub(crate) type_display_record_capacity_bytes: usize,
    pub(crate) type_display_auxiliary_rows: usize,
    pub(crate) type_display_auxiliary_capacity_bytes: usize,
}

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
    /// Keep only error diagnostics from a successful-warning freeze.
    ///
    /// Successful warning batches can acquire an error only when their frozen store replaces a
    /// refused warning with its capacity diagnostic. The associated successful warnings must not
    /// leak into the Diagnosed lane.
    pub(crate) fn into_errors(self) -> Self {
        if self.counts().errors == 0 {
            return self;
        }

        let Self {
            reports: original_reports,
            runs: original_runs,
        } = self;
        let mut filtered_reports = Vec::new();
        let mut report_indices = vec![None; original_reports.len()];
        let mut diagnostic_indices = Vec::with_capacity(original_reports.len());

        for (old_report_index, mut report) in original_reports.into_iter().enumerate() {
            let mut keep = Vec::with_capacity(report.diagnostics.records.len());
            let mut only_capacity_errors = true;
            for (index, record) in report.diagnostics.records.iter().enumerate() {
                let is_error = report.view_at(index).severity() == DiagnosticSeverity::Error;
                if is_error
                    && (record.code != COMPILER_CAPACITY_EXCEEDED.code || record.extra.is_some())
                {
                    only_capacity_errors = false;
                }
                keep.push(is_error);
            }
            let error_count = keep.iter().filter(|&&is_error| is_error).count();
            if error_count == 0 {
                diagnostic_indices.push(Vec::new());
                continue;
            }

            let old_records = std::mem::take(&mut report.diagnostics.records);
            let mut new_indices = vec![None; old_records.len()];
            let mut error_records = Vec::with_capacity(error_count);
            for (old_index, record) in old_records.into_iter().enumerate() {
                if keep[old_index] {
                    new_indices[old_index] = Some(
                        u32::try_from(error_records.len())
                            .expect("error record indices fit in u32"),
                    );
                    error_records.push(record);
                }
            }
            report.diagnostics.records = error_records;
            diagnostic_indices.push(new_indices);

            let new_report_index = u32::try_from(filtered_reports.len())
                .expect("diagnostic report indices fit in u32");
            report_indices[old_report_index] = Some(new_report_index);
            let DiagnosticReport {
                context,
                mut foreign_contexts,
                mut diagnostics,
                mut types,
                ..
            } = report;
            if only_capacity_errors && COMPILER_CAPACITY_EXCEEDED.pending_mask() == 0 {
                // The capacity record needs only its local span and resource fact; its frozen report context remains
                // the source, string and path owner needed to render that span.
                diagnostics.extras = Vec::new();
                diagnostics.labels = Vec::new();
                diagnostics.label_data = Vec::new();
                diagnostics.string_ids = Vec::new();
                diagnostics.substitutions = Vec::new();
                diagnostics.foreign_sites = Vec::new();
                foreign_contexts = Vec::new();
                types = TypeDisplaySnapshot::default();
            }
            filtered_reports.push(DiagnosticReport::new(
                context,
                diagnostics,
                foreign_contexts,
                types,
            ));
        }

        let mut filtered_runs: Vec<DiagnosticRun> = Vec::new();
        for run in original_runs {
            let old_report_index = run.report as usize;
            let Some(report) = report_indices[old_report_index] else {
                continue;
            };
            let run_start = run.first as usize;
            let run_end = run_start + run.len as usize;
            for &first in diagnostic_indices[old_report_index][run_start..run_end]
                .iter()
                .flatten()
            {
                match filtered_runs.last_mut() {
                    Some(last) if last.report == report && last.first + last.len == first => {
                        last.len = last
                            .len
                            .checked_add(1)
                            .expect("diagnostic run lengths fit in u32");
                    }
                    _ => filtered_runs.push(DiagnosticRun {
                        report,
                        first,
                        len: 1,
                    }),
                }
            }
        }

        Self::new(filtered_reports, filtered_runs)
    }

    /// The report that owns domain `index`; report indices follow domain registration order.
    #[cfg(test)]
    pub(crate) fn report(&self, index: u32) -> &DiagnosticReport {
        &self.reports[index as usize]
    }
    /// Every retained identity context, including domains with no records and foreign sites.
    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn contexts(&self) -> impl Iterator<Item = &FrozenIdentityContext> {
        self.reports.iter().flat_map(|report| {
            std::iter::once(report.context()).chain(
                report
                    .foreign_contexts
                    .iter()
                    .filter_map(|context| context.as_deref()),
            )
        })
    }

    #[cfg(any(test, feature = "data_layout_memory_probe"))]
    pub(crate) fn memory_ledger_metrics(&self) -> DiagnosticReportMemoryMetrics {
        let mut metrics = DiagnosticReportMemoryMetrics {
            report_rows: self.reports.len(),
            report_capacity_bytes: self
                .reports
                .capacity()
                .saturating_mul(std::mem::size_of::<DiagnosticReport>()),
            run_rows: self.runs.len(),
            run_capacity_bytes: self
                .runs
                .capacity()
                .saturating_mul(std::mem::size_of::<DiagnosticRun>()),
            ..DiagnosticReportMemoryMetrics::default()
        };

        for report in &self.reports {
            let diagnostics = report.diagnostics.memory_ledger_metrics();
            metrics.foreign_context_rows += report.foreign_contexts.len();
            metrics.foreign_context_capacity_bytes += report
                .foreign_contexts
                .capacity()
                .saturating_mul(std::mem::size_of::<Option<Arc<FrozenIdentityContext>>>());
            metrics.record_rows += diagnostics.record_rows;
            metrics.record_capacity_bytes += diagnostics.record_capacity_bytes;
            metrics.extra_rows += diagnostics.extra_rows;
            metrics.extra_capacity_bytes += diagnostics.extra_capacity_bytes;
            metrics.label_rows += diagnostics.label_rows;
            metrics.label_capacity_bytes += diagnostics.label_capacity_bytes;
            metrics.label_data_rows += diagnostics.label_data_rows;
            metrics.label_data_capacity_bytes += diagnostics.label_data_capacity_bytes;
            metrics.string_list_rows += diagnostics.string_list_rows;
            metrics.string_list_capacity_bytes += diagnostics.string_list_capacity_bytes;
            metrics.substitution_rows += diagnostics.substitution_rows;
            metrics.substitution_capacity_bytes += diagnostics.substitution_capacity_bytes;
            metrics.foreign_site_rows += diagnostics.foreign_site_rows;
            metrics.foreign_site_capacity_bytes += diagnostics.foreign_site_capacity_bytes;

            let types = report.types.memory_ledger_metrics();
            metrics.type_display_record_rows += types.record_rows;
            metrics.type_display_record_capacity_bytes += types.record_capacity_bytes;
            metrics.type_display_auxiliary_rows += types.type_id_rows
                + types.variant_rows
                + types.choice_rows
                + types.callable_rows
                + types.generic_instance_rows
                + types.collection_rows;
            metrics.type_display_auxiliary_capacity_bytes += types
                .type_id_capacity_bytes
                .saturating_add(types.variant_capacity_bytes)
                .saturating_add(types.choice_capacity_bytes)
                .saturating_add(types.callable_capacity_bytes)
                .saturating_add(types.generic_instance_capacity_bytes)
                .saturating_add(types.collection_capacity_bytes);
        }
        metrics
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
