//! Public-export collector regressions.
//!
//! WHAT: verifies that duplicate public exports retain the first authored owner across the
//! source-package and module-root collection passes.
//! WHY: duplicate diagnostics must identify both the current duplicate and the first declaration
//! without storing test-only logic in the production collector module.

use super::*;
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    CaseConflictingPublicExportRouteView, DuplicatePublicExportView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;

fn span(start: u32, length: u32, builder: &mut ExtendedSpanBuilder) -> SourceSpan {
    SourceSpan::new(
        SourceId::from_index(1),
        LocalSpan::exact(start, length, builder).expect("test span should fit"),
    )
}

fn site_span(site: SiteRef<'_>) -> Option<SourceSpan> {
    match site {
        SiteRef::Absent => None,
        SiteRef::Local(span) | SiteRef::Foreign { span, .. } => Some(span),
    }
}

fn route_operand_at(
    path: PathId,
    span: SourceSpan,
    path_syntax_index: usize,
) -> PublicExportRouteOperand {
    PublicExportRouteOperand {
        path,
        span,
        path_syntax: PathSyntaxId::try_from_index(path_syntax_index).expect("a test path row fits"),
    }
}

fn route_operand(path: PathId, span: SourceSpan) -> PublicExportRouteOperand {
    route_operand_at(path, span, 0)
}

fn intern_test_path(
    path: &str,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> PathId {
    path_fork
        .try_intern_portable_path(path, string_table)
        .expect("test path fits")
}

fn insert_export(
    collector: &mut PublicExportCollector,
    route: Option<PublicExportRouteOperand>,
    export_name: &str,
    target_path: PathId,
    span: SourceSpan,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> PublicExportDataResult<()> {
    let export_name = string_table.intern(export_name);
    collector.insert(
        route,
        export_name,
        PublicExportTarget::SourceDeclaration { path: target_path },
        Some(span),
        string_table,
        path_fork,
    )
}

#[test]
fn duplicate_public_export_retains_first_owner_span_across_passes() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let export_name = string_table.intern("greet");
    let source_path = path_fork
        .try_intern_portable_path("src/greet", &mut string_table)
        .expect("test path fits");
    let first_span = span(5, 5, &mut span_builder);
    let duplicate_span = span(12, 5, &mut span_builder);

    let mut first_pass = PublicExportCollector::default();
    first_pass
        .insert(
            None,
            export_name,
            PublicExportTarget::SourceDeclaration { path: source_path },
            Some(first_span),
            &string_table,
            &path_fork,
        )
        .expect("first public export should be accepted");

    let existing_locations = PublicExportLocations {
        exports: FxHashMap::from_iter([(
            PublicExportKey {
                route: None,
                export_name,
            },
            first_span,
        )]),
        routes: FxHashMap::default(),
        route_folds: CaseFoldedRouteIndex::default(),
    };
    let mut second_pass =
        PublicExportCollector::from_existing(first_pass.exports, Some(existing_locations));
    let HeaderParseFailure::Diagnostic(diagnostic) = second_pass
        .insert(
            None,
            export_name,
            PublicExportTarget::SourceDeclaration { path: source_path },
            Some(duplicate_span),
            &string_table,
            &path_fork,
        )
        .expect_err("the second public export should be rejected")
    else {
        panic!("duplicate public export must be a source diagnostic");
    };

    let diagnostics = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = diagnostics
        .first()
        .expect("expected duplicate export diagnostic");
    assert_eq!(
        match diagnostic.primary_site() {
            SiteRef::Absent => None,
            SiteRef::Local(span) | SiteRef::Foreign { span, .. } => Some(span),
        },
        Some(duplicate_span)
    );
    assert_eq!(
        diagnostic
            .durable_labels()
            .next()
            .and_then(|label| match label.site {
                SiteRef::Absent => None,
                SiteRef::Local(span) | SiteRef::Foreign { span, .. } => Some(span),
            }),
        Some(first_span)
    );
    assert!(
        diagnostic
            .family_view::<DuplicatePublicExportView>()
            .is_some()
    );
}

#[test]
fn distinct_routes_may_publish_the_same_leaf_and_entries_carry_the_route() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let target_path = intern_test_path("src/Button", &mut string_table, &mut path_fork);
    let forms_route = intern_test_path("forms", &mut string_table, &mut path_fork);
    let validation_route = intern_test_path("forms/validation", &mut string_table, &mut path_fork);
    let mut collector = PublicExportCollector::default();

    insert_export(
        &mut collector,
        Some(route_operand(forms_route, span(5, 5, &mut span_builder))),
        "Button",
        target_path,
        span(12, 6, &mut span_builder),
        &mut string_table,
        &path_fork,
    )
    .expect("the @forms export should be accepted");
    insert_export(
        &mut collector,
        Some(route_operand(
            validation_route,
            span(20, 16, &mut span_builder),
        )),
        "Button",
        target_path,
        span(40, 6, &mut span_builder),
        &mut string_table,
        &path_fork,
    )
    .expect("the same leaf under a different route should be accepted");

    assert_eq!(collector.exports.len(), 2);
    let routes: Vec<_> = collector.exports.iter().map(|entry| entry.route).collect();
    assert!(routes.contains(&Some(forms_route)));
    assert!(routes.contains(&Some(validation_route)));
}

#[test]
fn two_symbols_may_share_one_route() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let forms_route = intern_test_path("forms", &mut string_table, &mut path_fork);
    let cells_path = intern_test_path("src/cells", &mut string_table, &mut path_fork);
    let heading_path = intern_test_path("src/heading", &mut string_table, &mut path_fork);
    let mut collector = PublicExportCollector::default();

    insert_export(
        &mut collector,
        Some(route_operand(forms_route, span(5, 5, &mut span_builder))),
        "cells",
        cells_path,
        span(12, 5, &mut span_builder),
        &mut string_table,
        &path_fork,
    )
    .expect("the first shared-route symbol should be accepted");
    insert_export(
        &mut collector,
        Some(route_operand(forms_route, span(5, 5, &mut span_builder))),
        "heading",
        heading_path,
        span(24, 7, &mut span_builder),
        &mut string_table,
        &path_fork,
    )
    .expect("a second symbol under the same route should be accepted");

    assert_eq!(collector.exports.len(), 2);
}

#[test]
fn duplicate_leaf_in_one_route_reports_both_export_sites() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let forms_route = intern_test_path("forms", &mut string_table, &mut path_fork);
    let target_path = intern_test_path("src/Button", &mut string_table, &mut path_fork);
    let first_span = span(12, 6, &mut span_builder);
    let duplicate_span = span(30, 6, &mut span_builder);
    let mut collector = PublicExportCollector::default();

    insert_export(
        &mut collector,
        Some(route_operand(forms_route, span(5, 5, &mut span_builder))),
        "Button",
        target_path,
        first_span,
        &mut string_table,
        &path_fork,
    )
    .expect("the first route leaf should be accepted");
    let HeaderParseFailure::Diagnostic(diagnostic) = insert_export(
        &mut collector,
        Some(route_operand(forms_route, span(5, 5, &mut span_builder))),
        "Button",
        target_path,
        duplicate_span,
        &mut string_table,
        &path_fork,
    )
    .expect_err("a repeated leaf in one route must be rejected") else {
        panic!("duplicate route leaf must be a source diagnostic");
    };

    let diagnostics = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = diagnostics
        .first()
        .expect("expected duplicate route leaf diagnostic");
    assert_eq!(site_span(diagnostic.primary_site()), Some(duplicate_span));
    assert_eq!(
        diagnostic
            .durable_labels()
            .next()
            .and_then(|label| site_span(label.site)),
        Some(first_span)
    );
    assert!(
        diagnostic
            .family_view::<DuplicatePublicExportView>()
            .is_some()
    );
}

#[test]
fn root_and_nested_routes_may_share_a_leaf() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let forms_route = intern_test_path("forms", &mut string_table, &mut path_fork);
    let target_path = intern_test_path("src/Button", &mut string_table, &mut path_fork);
    let mut collector = PublicExportCollector::default();

    insert_export(
        &mut collector,
        None,
        "Button",
        target_path,
        span(2, 6, &mut span_builder),
        &mut string_table,
        &path_fork,
    )
    .expect("the root export should be accepted");
    insert_export(
        &mut collector,
        Some(route_operand(forms_route, span(5, 5, &mut span_builder))),
        "Button",
        target_path,
        span(20, 6, &mut span_builder),
        &mut string_table,
        &path_fork,
    )
    .expect("a nested route may repeat the root leaf");

    assert!(collector.exports.iter().any(|entry| entry.route.is_none()));
    assert!(
        collector
            .exports
            .iter()
            .any(|entry| entry.route == Some(forms_route))
    );
}

#[test]
fn case_only_route_conflict_reports_both_route_operand_sites() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let lower_route = intern_test_path("forms", &mut string_table, &mut path_fork);
    let upper_route = intern_test_path("Forms", &mut string_table, &mut path_fork);
    let target_path = intern_test_path("src/Button", &mut string_table, &mut path_fork);
    let first_route_span = span(5, 5, &mut span_builder);
    let conflicting_route_span = span(25, 5, &mut span_builder);
    let mut collector = PublicExportCollector::default();

    insert_export(
        &mut collector,
        Some(route_operand(lower_route, first_route_span)),
        "Button",
        target_path,
        span(12, 6, &mut span_builder),
        &mut string_table,
        &path_fork,
    )
    .expect("the first route spelling should be accepted");
    let HeaderParseFailure::Diagnostic(diagnostic) = insert_export(
        &mut collector,
        Some(route_operand(upper_route, conflicting_route_span)),
        "Label",
        target_path,
        span(32, 5, &mut span_builder),
        &mut string_table,
        &path_fork,
    )
    .expect_err("a case-only route conflict must be rejected") else {
        panic!("case-only route conflict must be a source diagnostic");
    };

    let diagnostics = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = diagnostics
        .first()
        .expect("expected case-only route diagnostic");
    assert_eq!(
        site_span(diagnostic.primary_site()),
        Some(conflicting_route_span)
    );
    assert_eq!(
        diagnostic
            .durable_labels()
            .next()
            .and_then(|label| site_span(label.site)),
        Some(first_route_span)
    );
    assert!(
        diagnostic
            .family_view::<CaseConflictingPublicExportRouteView>()
            .is_some()
    );
}

#[test]
fn case_only_nested_route_conflict_compares_each_component() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let validation_route = intern_test_path("forms/Validation", &mut string_table, &mut path_fork);
    let lower_validation_route =
        intern_test_path("Forms/validation", &mut string_table, &mut path_fork);
    let target_path = intern_test_path("src/validate", &mut string_table, &mut path_fork);
    let mut collector = PublicExportCollector::default();

    insert_export(
        &mut collector,
        Some(route_operand(
            validation_route,
            span(5, 16, &mut span_builder),
        )),
        "validate",
        target_path,
        span(24, 8, &mut span_builder),
        &mut string_table,
        &path_fork,
    )
    .expect("the first nested route should be accepted");
    let error = insert_export(
        &mut collector,
        Some(route_operand(
            lower_validation_route,
            span(40, 16, &mut span_builder),
        )),
        "check",
        target_path,
        span(60, 5, &mut span_builder),
        &mut string_table,
        &path_fork,
    )
    .expect_err("component-wise case folding must reject the nested conflict");

    let HeaderParseFailure::Diagnostic(diagnostic) = error else {
        panic!("nested case conflict must be a source diagnostic");
    };
    let diagnostics = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    assert!(
        diagnostics
            .first()
            .expect("expected nested case diagnostic")
            .family_view::<CaseConflictingPublicExportRouteView>()
            .is_some()
    );
}

#[test]
fn cross_pass_route_conflict_labels_the_earlier_authored_operand() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let declaration_route = intern_test_path("forms", &mut string_table, &mut path_fork);
    let reexport_route = intern_test_path("Forms", &mut string_table, &mut path_fork);
    let target_path = intern_test_path("src/Button", &mut string_table, &mut path_fork);
    // Both route spans are extended rows, so authored ordering cannot fall back to inline span
    // starts; it must use the retained PathSyntaxId rows.
    let reexport_route_span = span(20_000_000, 6, &mut span_builder);
    let declaration_route_span = span(40_000_000, 5, &mut span_builder);

    // Pass 1 records the later authored declaration route first.
    let mut first_pass = PublicExportCollector::default();
    insert_export(
        &mut first_pass,
        Some(route_operand_at(
            declaration_route,
            declaration_route_span,
            2,
        )),
        "Button",
        target_path,
        span(52, 6, &mut span_builder),
        &mut string_table,
        &path_fork,
    )
    .expect("the declaration route should be accepted");
    let (exports, locations) = first_pass.into_parts();

    // Pass 2 records the earlier authored re-export clause route.
    let mut second_pass = PublicExportCollector::from_existing(exports, Some(locations));
    let HeaderParseFailure::Diagnostic(diagnostic) = insert_export(
        &mut second_pass,
        Some(route_operand_at(reexport_route, reexport_route_span, 0)),
        "Label",
        target_path,
        span(20, 5, &mut span_builder),
        &mut string_table,
        &path_fork,
    )
    .expect_err("case-only routes must conflict across collection passes") else {
        panic!("cross-pass case conflict must be a source diagnostic");
    };

    let diagnostics = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = diagnostics
        .first()
        .expect("expected cross-pass case diagnostic");
    assert_eq!(
        site_span(diagnostic.primary_site()),
        Some(declaration_route_span),
        "the later authored declaration operand must not be labelled as previous"
    );
    assert_eq!(
        diagnostic
            .durable_labels()
            .next()
            .and_then(|label| site_span(label.site)),
        Some(reexport_route_span),
        "the earlier authored re-export operand should be the related site"
    );
}

#[test]
fn parent_and_child_routes_remain_legal() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let parent_route = intern_test_path("forms", &mut string_table, &mut path_fork);
    let child_route = intern_test_path("forms/validation", &mut string_table, &mut path_fork);
    let first_path = intern_test_path("src/cells", &mut string_table, &mut path_fork);
    let second_path = intern_test_path("src/validate", &mut string_table, &mut path_fork);
    let mut collector = PublicExportCollector::default();

    insert_export(
        &mut collector,
        Some(route_operand(parent_route, span(5, 5, &mut span_builder))),
        "cells",
        first_path,
        span(12, 5, &mut span_builder),
        &mut string_table,
        &path_fork,
    )
    .expect("the parent route should be accepted");
    insert_export(
        &mut collector,
        Some(route_operand(child_route, span(20, 16, &mut span_builder))),
        "validate",
        second_path,
        span(40, 8, &mut span_builder),
        &mut string_table,
        &path_fork,
    )
    .expect("a deeper route under the parent route should be accepted");

    assert_eq!(collector.exports.len(), 2);
}
