//! WHAT: release speculation folds only eligible ordinary call sites and their arguments.
//! WHY:  release rewriting walks every expression root to discover candidates. Auxiliary template,
//!       collection-read and fallible-unwrap folding must stay scoped to a candidate argument so
//!       an unrelated runtime expression neither consumes speculative budget nor produces a
//!       release-only Note.

use super::*;
use crate::build_system::create_project_modules::compiled_boundary::ProjectFrontendCompilation;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::external_packages::{CallTarget, ExternalFunctionId};
use crate::compiler_frontend::hir::terminators::{HirFallibleOperation, HirTerminator};
use crate::compiler_frontend::instrumentation::lock_counter_test;
use crate::compiler_frontend::module_compilation::CompiledModuleArtifact;
use crate::compiler_frontend::source::SourceSpan;

const TIGHT_CONFIG_SOURCE: &str = "project #= (\n    name = \"docs\",\n    const_work_limit = 3,\n    const_memory_limit_mb = 1,\n)\nhtml #= ()\n";

/// WHAT: one eligible pure scalar call beside unrelated runtime collection reads.
/// WHY:  `consume` carries the direct candidate call that activates release speculation, while
///       `probe` performs four constant-index collection reads that do not call it.
const UNRELATED_READS_ENTRY: &str = "Point = |\n    x I32,\n|\n\nmake #|| -> Point:\n    return Point(2)\n;\n\ndouble |value I32| -> I32:\n    return value * 2\n;\n\ntriple #|value I32| -> I32:\n    return value * 3\n;\n\nvalues #= {1, 2, 3}\n\nprobe || -> I32, Error!:\n    return values.get(0)! + values.get(1)! + values.get(2)! + values.get(3)!\n;\n\nconsume |value I32| -> I32:\n    return double(value) + double(2)\n;\n\nconst_argument || -> I32:\n    return double(make().x)\n;\n\nresult #I32 = triple(2)\n[: [result] ]\n";

/// WHAT: an eligible loop call refused by a tight budget beside unrelated fallible reads.
/// WHY:  the refused call must own the only Note; the unrelated `measure` reads must never be
///       charged against speculative execution or named by budget advice.
const TIGHT_BUDGET_ENTRY: &str = "count |limit I32| -> I32:\n    total ~I32 = 0\n    index ~I32 = 0\n    loop index < limit:\n        ~total = total + 1\n        ~index = index + 1\n    ;\n    return total\n;\n\nprobe |value I32| -> I32:\n    return count(20) + value\n;\n\nmeasure |items {I32}| -> I32, Error!:\n    return items.get(0)! + items.get(1)! + items.get(2)! + items.get(3)!\n;\n\nresult #I32 = 4\n[: [result] ]\n";

fn entry_hir_collection_get_count(artifact: &CompiledModuleArtifact) -> usize {
    artifact
        .module
        .executable
        .hir
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                &block.terminator,
                HirTerminator::Invoke {
                    operation: HirFallibleOperation::Call {
                        target: CallTarget::External(ExternalFunctionId::CollectionGet),
                        ..
                    },
                    ..
                }
            )
        })
        .count()
}

fn entry_hir_local_call_invoke_spans(artifact: &CompiledModuleArtifact) -> Vec<SourceSpan> {
    artifact
        .module
        .executable
        .hir
        .blocks
        .iter()
        .filter_map(|block| match &block.terminator {
            HirTerminator::Invoke {
                span: Some(span),
                operation:
                    HirFallibleOperation::Call {
                        target: CallTarget::Local(_) | CallTarget::ModulePrivate(_),
                        ..
                    },
                ..
            } => Some(*span),
            _ => None,
        })
        .collect()
}

fn note_spans(reports: &DiagnosticReportSet) -> Vec<SourceSpan> {
    reports
        .iter_severity(DiagnosticSeverity::Note)
        .filter_map(|(_, view)| match view.primary_site() {
            SiteRef::Local(span) => Some(span),
            SiteRef::Absent | SiteRef::Foreign { .. } => None,
        })
        .collect()
}

fn resolved_site_excerpt(
    frontend: &ProjectFrontendCompilation,
    span: SourceSpan,
) -> Option<String> {
    let database = frontend.project_source_database.as_ref()?;
    let range = span.byte_range(&**database);
    let text = database.retained_text(span.source())?;
    text.get(range.start() as usize..range.end() as usize)
        .map(str::to_owned)
}

/// WHAT: an unrelated runtime collection expression keeps its debug HIR shape in release, while
///       an eligible pure scalar call still folds and required const calls fold in both profiles.
/// WHY:  release speculation must not rewrite expressions that are not arguments of a candidate.
///       The four collection reads in `probe` are the audit's independent runtime expression.
#[test]
fn release_speculation_leaves_unrelated_runtime_expressions_unchanged() {
    let _test_guard = lock_counter_test();
    let files = [
        ("src/@entry.moth", UNRELATED_READS_ENTRY),
        ("config.moth", CONFIG_SOURCE),
    ];
    let (debug, debug_warnings) = compile_fixture_with_warnings(&files, BuildProfile::Dev);
    let (release, release_warnings) = compile_fixture_with_warnings(&files, BuildProfile::Release);

    assert!(
        debug_warnings.is_none(),
        "the fixture must not emit warnings in debug: {debug_warnings:?}"
    );
    assert!(
        release_warnings.is_none(),
        "an ample budget must not emit release Notes: {release_warnings:?}"
    );

    let debug_entry = entry_artifact(&debug);
    let release_entry = entry_artifact(&release);
    assert_eq!(
        entry_folded_fragments(debug_entry),
        [" 6 "],
        "a required const call must fold identically in debug"
    );
    assert_eq!(
        entry_folded_fragments(release_entry),
        [" 6 "],
        "release must keep the same folded output as debug"
    );

    let debug_calls = entry_hir_local_call_count(debug_entry);
    let release_calls = entry_hir_local_call_count(release_entry);
    assert_eq!(
        debug_calls, 3,
        "debug keeps `double(value)`, `double(2)` and the constant-argument `double(make().x)`"
    );
    assert_eq!(
        release_calls, 1,
        "release folds `double(2)` and the constant-argument `double(make().x)`, leaving the runtime `double(value)` call"
    );

    let debug_reads = entry_hir_collection_get_count(debug_entry);
    let release_reads = entry_hir_collection_get_count(release_entry);
    assert_eq!(
        debug_reads, 4,
        "the unrelated probe must keep four constant-index reads in debug"
    );
    assert_eq!(
        release_reads, debug_reads,
        "release speculation must not fold the unrelated probe reads: debug={debug_reads} release={release_reads}"
    );
}

/// WHAT: a tight budget names only the attempted eligible call site.
/// WHY:  auxiliary folding of the unrelated `measure` reads previously exhausted the same
///       speculative allowance and produced a Note at an expression that never calls a candidate.
#[test]
fn release_budget_note_names_only_an_attempted_candidate_call() {
    let _test_guard = lock_counter_test();
    let files = [
        ("src/@entry.moth", TIGHT_BUDGET_ENTRY),
        ("config.moth", TIGHT_CONFIG_SOURCE),
    ];
    let (debug, debug_warnings) = compile_fixture_with_warnings(&files, BuildProfile::Dev);
    let (release, release_warnings) = compile_fixture_with_warnings(&files, BuildProfile::Release);

    assert!(
        debug_warnings.is_none(),
        "debug must not run release speculation: {debug_warnings:?}"
    );
    let warnings = release_warnings.expect("the refused candidate call must emit a Note");
    let notes = note_spans(&warnings);
    assert_eq!(
        notes.len(),
        1,
        "exactly the refused candidate call may own budget advice: {notes:?}"
    );
    let note_span = notes[0];

    assert_eq!(
        entry_folded_fragments(entry_artifact(&debug)),
        entry_folded_fragments(entry_artifact(&release)),
        "profile choice must not change folded output"
    );

    let release_entry = entry_artifact(&release);
    let candidate_call_spans = entry_hir_local_call_invoke_spans(release_entry);
    assert_eq!(
        candidate_call_spans,
        [note_span],
        "the Note must name the refused `count(20)` call that survives at runtime"
    );
    assert_eq!(
        resolved_site_excerpt(&release, note_span).as_deref(),
        Some("count"),
        "the Note must point at the eligible ordinary call, never the unrelated `items` read"
    );
    assert_eq!(
        entry_hir_collection_get_count(release_entry),
        4,
        "the unrelated measure reads must remain runtime in release"
    );
}
