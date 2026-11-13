//! Focused unit tests for constant dependency invariant behaviour.
//!
//! WHAT: verifies that missing constant position metadata produces an internal compiler error,
//!       not a user-facing source diagnostic, and that qualified-reference diagnostics emit in
//!       authored source-span order instead of the retained hint set's hash order.
//! WHY: the position record is built in the same inventory pass as constant classification, so
//!      a missing record is a compiler invariant violation. Diagnostics preserve insertion order
//!      and bounded error reporting keeps the first retained errors, so hash iteration order must
//!      never decide which authored occurrence reports first.

use super::*;
use crate::compiler_frontend::compiler_messages::CompileTimeEvaluationErrorReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::CompileTimeEvaluationErrorView;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::{
    CompactDiagnosticBagForTest, compact_bag_for_test,
};
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use std::collections::HashSet;
use std::path::PathBuf;

fn test_span(start: u32, length: u32) -> SourceSpan {
    SourceSpan::new(
        SourceId::from_index(2),
        LocalSpan::exact_inline(start, length).expect("test span should pack inline"),
    )
}

fn intern_path(
    path: &str,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> PathId {
    path_fork
        .try_intern_filesystem_path(&PathBuf::from(path), string_table)
        .expect("test path should be UTF-8")
}

/// Resolve the test spans without a source database: every test span packs inline.
fn inline_byte_range(span: SourceSpan) -> Option<(u32, u32)> {
    span.local()
        .inline_range()
        .map(|range| (range.start(), range.end()))
}

fn diagnostic_spans(diagnostics: &CompactDiagnosticBagForTest) -> Vec<SourceSpan> {
    diagnostics
        .views()
        .map(|view| match view.primary_site() {
            SiteRef::Local(span) | SiteRef::Foreign { span, .. } => span,
            SiteRef::Absent => panic!("qualified-reference diagnostics must retain their span"),
        })
        .collect()
}

fn diagnostic_reasons(
    diagnostics: &CompactDiagnosticBagForTest,
) -> Vec<CompileTimeEvaluationErrorReason> {
    diagnostics
        .views()
        .map(|view| {
            view.family_view::<CompileTimeEvaluationErrorView>()
                .expect("the qualified-reference failure keeps compile-time evaluation facts")
                .reason
        })
        .collect()
}

#[test]
fn missing_constant_position_produces_infrastructure_error() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let constant_path = path_fork
        .try_intern_filesystem_path(&PathBuf::from("src/missing.moth"), &mut string_table)
        .expect("test path should be UTF-8");
    let error = missing_constant_position_error(&constant_path, &path_fork, &string_table);

    assert!(
        error.message.contains("Missing constant position metadata"),
        "missing constant position must be an infrastructure error, not a user-facing diagnostic: {error:?}"
    );
}

#[test]
fn qualified_forward_reference_diagnostics_follow_source_span_order() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_file = intern_path("src/settings.moth", &mut string_table, &mut path_fork);
    let declaration_path = intern_path(
        "src/settings.moth#combined",
        &mut string_table,
        &mut path_fork,
    );
    // The later authored occurrence's target is interned first, so path order alone would put it
    // first; only the authored span can produce the required diagnostic order.
    let later_authored_target = intern_path(
        "src/settings.moth#late_later",
        &mut string_table,
        &mut path_fork,
    );
    let earlier_authored_target = intern_path(
        "src/settings.moth#late_earlier",
        &mut string_table,
        &mut path_fork,
    );
    let source_owned_target = intern_path(
        "src/settings.moth#source_owned",
        &mut string_table,
        &mut path_fork,
    );

    let mut constant_positions: FxHashMap<PathId, ConstantPosition> = FxHashMap::default();
    for (path, header_index) in [
        (earlier_authored_target, 1usize),
        (later_authored_target, 2usize),
        (source_owned_target, 3usize),
    ] {
        constant_positions.insert(
            path,
            ConstantPosition {
                source_file,
                header_index,
                kind: OrderedValueKind::Constant,
            },
        );
    }

    let earlier_span = test_span(10, 4);
    let later_span = test_span(30, 5);
    let mut hints = HashSet::new();
    // Insert the later authored occurrence first: only span ordering can recover source order.
    hints.insert(
        LocalDeclarationOrderingHint::qualified_reference(later_authored_target)
            .with_occurrence_span(later_span),
    );
    hints.insert(
        LocalDeclarationOrderingHint::qualified_reference(earlier_authored_target)
            .with_occurrence_span(earlier_span),
    );
    // Non-qualified hints share the set for edge deduplication but never emit diagnostics.
    hints.insert(LocalDeclarationOrderingHint::source_owned(
        source_owned_target,
    ));

    let mut diagnostic_bag = DiagnosticBag::new();
    push_qualified_reference_diagnostics(
        QualifiedReferenceDiagnosticContext {
            declaration_path,
            name_span: Some(test_span(0, 8)),
            header_index: 0,
            current_source_file: source_file,
            is_moth_template_content: false,
            constant_positions: &constant_positions,
            path_fork: &path_fork,
        },
        &hints,
        &mut diagnostic_bag,
        inline_byte_range,
    );

    let diagnostics = compact_bag_for_test(diagnostic_bag, None);
    assert_eq!(
        diagnostics.len(),
        2,
        "only the qualified occurrences report"
    );
    assert_eq!(
        diagnostic_spans(&diagnostics),
        vec![earlier_span, later_span],
        "diagnostics must emit in authored span order, not hash order"
    );
    for view in diagnostics.views() {
        assert_eq!(view.descriptor().code, "MOTH-RULE-0053");
    }
    assert_eq!(
        diagnostic_reasons(&diagnostics),
        vec![
            CompileTimeEvaluationErrorReason::SameFileForwardConstantReference,
            CompileTimeEvaluationErrorReason::SameFileForwardConstantReference,
        ]
    );
}

#[test]
fn qualified_self_and_forward_references_emit_in_span_order_with_preserved_reasons() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_file = intern_path("src/settings.moth", &mut string_table, &mut path_fork);
    let declaration_path = intern_path(
        "src/settings.moth#combined",
        &mut string_table,
        &mut path_fork,
    );
    let forward_target = intern_path("src/settings.moth#later", &mut string_table, &mut path_fork);

    let mut constant_positions: FxHashMap<PathId, ConstantPosition> = FxHashMap::default();
    constant_positions.insert(
        declaration_path,
        ConstantPosition {
            source_file,
            header_index: 0,
            kind: OrderedValueKind::Constant,
        },
    );
    constant_positions.insert(
        forward_target,
        ConstantPosition {
            source_file,
            header_index: 3,
            kind: OrderedValueKind::Constant,
        },
    );

    let forward_span = test_span(10, 5);
    let self_span = test_span(40, 8);
    let mut hints = HashSet::new();
    // Insert the self-reference first; its authored span is later.
    hints.insert(
        LocalDeclarationOrderingHint::qualified_reference(declaration_path)
            .with_occurrence_span(self_span),
    );
    hints.insert(
        LocalDeclarationOrderingHint::qualified_reference(forward_target)
            .with_occurrence_span(forward_span),
    );

    let mut diagnostic_bag = DiagnosticBag::new();
    push_qualified_reference_diagnostics(
        QualifiedReferenceDiagnosticContext {
            declaration_path,
            name_span: Some(test_span(0, 8)),
            header_index: 0,
            current_source_file: source_file,
            is_moth_template_content: false,
            constant_positions: &constant_positions,
            path_fork: &path_fork,
        },
        &hints,
        &mut diagnostic_bag,
        inline_byte_range,
    );

    let diagnostics = compact_bag_for_test(diagnostic_bag, None);
    assert_eq!(
        diagnostic_spans(&diagnostics),
        vec![forward_span, self_span],
        "the earlier forward reference must report before the later self-reference"
    );
    assert_eq!(
        diagnostic_reasons(&diagnostics),
        vec![
            CompileTimeEvaluationErrorReason::SameFileForwardConstantReference,
            CompileTimeEvaluationErrorReason::ConstantSelfReference,
        ]
    );
}

#[test]
fn mtf_content_self_cycle_skip_keeps_the_self_reference_out_of_header_diagnostics() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let source_file = intern_path("src/intro.mtf", &mut string_table, &mut path_fork);
    let content_path = intern_path("src/intro.mtf#content", &mut string_table, &mut path_fork);

    let mut constant_positions: FxHashMap<PathId, ConstantPosition> = FxHashMap::default();
    constant_positions.insert(
        content_path,
        ConstantPosition {
            source_file,
            header_index: 0,
            kind: OrderedValueKind::Constant,
        },
    );

    let mut hints = HashSet::new();
    hints.insert(
        LocalDeclarationOrderingHint::qualified_reference(content_path)
            .with_occurrence_span(test_span(1, 8)),
    );

    let mut diagnostic_bag = DiagnosticBag::new();
    push_qualified_reference_diagnostics(
        QualifiedReferenceDiagnosticContext {
            declaration_path: content_path,
            name_span: Some(test_span(0, 6)),
            header_index: 0,
            current_source_file: source_file,
            is_moth_template_content: true,
            constant_positions: &constant_positions,
            path_fork: &path_fork,
        },
        &hints,
        &mut diagnostic_bag,
        inline_byte_range,
    );

    let diagnostics = compact_bag_for_test(diagnostic_bag, None);
    assert!(
        diagnostics.is_empty(),
        "a MOTH template content self-cycle belongs to Stage 3's cycle diagnostic"
    );
}

#[test]
fn qualified_hints_without_occurrence_spans_sort_after_spanned_hints() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let alpha = intern_path("src/alpha.moth#value", &mut string_table, &mut path_fork);
    let beta = intern_path("src/beta.moth#value", &mut string_table, &mut path_fork);
    let gamma = intern_path("src/gamma.moth#value", &mut string_table, &mut path_fork);

    let mut hints = HashSet::new();
    hints.insert(LocalDeclarationOrderingHint::qualified_reference(beta));
    hints.insert(LocalDeclarationOrderingHint::qualified_reference(alpha));
    hints.insert(
        LocalDeclarationOrderingHint::qualified_reference(gamma)
            .with_occurrence_span(test_span(20, 5)),
    );

    let ordered = qualified_reference_hints_in_diagnostic_order(
        &hints,
        Some(test_span(5, 5)),
        inline_byte_range,
    );
    assert_eq!(
        ordered.iter().map(|hint| hint.path()).collect::<Vec<_>>(),
        vec![gamma, alpha, beta],
        "spanned hints precede name-span hints, which tie-break by path"
    );
}

#[test]
fn qualified_hints_with_equal_spans_tie_break_by_path_deterministically() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let alpha = intern_path("src/alpha.moth#value", &mut string_table, &mut path_fork);
    let beta = intern_path("src/beta.moth#value", &mut string_table, &mut path_fork);
    let shared_span = test_span(12, 4);

    for insertion_order in [[alpha, beta], [beta, alpha]] {
        let mut hints = HashSet::new();
        for path in insertion_order {
            hints.insert(
                LocalDeclarationOrderingHint::qualified_reference(path)
                    .with_occurrence_span(shared_span),
            );
        }

        let ordered =
            qualified_reference_hints_in_diagnostic_order(&hints, None, inline_byte_range);
        assert_eq!(
            ordered.iter().map(|hint| hint.path()).collect::<Vec<_>>(),
            vec![alpha, beta],
            "equal spans must order by path, independent of hash-set insertion order"
        );
    }
}
