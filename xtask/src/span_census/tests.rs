use super::{SPAN_CENSUS_SCHEMA_VERSION, started_report};
use crate::report_file::ReportRunIdentity;

/// The report a run writes before it walks anything must say it measured nothing yet.
///
/// This is what stops an interrupted run from leaving the previous successful report in place,
/// where a reader would take it for this run's evidence. The zero counts are only safe to write
/// because `completed: false` is written with them.
#[test]
fn the_report_written_before_the_walk_claims_no_result() {
    let report = started_report(ReportRunIdentity::started("span-census", None));

    assert!(!report.run.completed);
    assert_eq!(report.schema_version, SPAN_CENSUS_SCHEMA_VERSION);
    assert_eq!(report.files_walked, 0);
    assert_eq!(report.files_tokenized, 0);
    assert!(report.tokenize_failures.is_empty());
    assert_eq!(report.total_spans, 0);
    assert!(report.candidates.is_empty());
    assert_eq!(report.excluded_js.extension, "js");
    assert!(!report.excluded_js.reason.is_empty());
}
