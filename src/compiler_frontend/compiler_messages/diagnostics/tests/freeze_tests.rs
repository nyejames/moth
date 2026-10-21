//! Freeze tests: pending diagnostics become per-domain reports with their own identity contexts.
//!
//! WHAT: drives `PendingDiagnostics::freeze` and inspects the frozen reports, runs, type displays and installed
//! donor handles.
//! WHY: the freeze is the only place that compacts drafts by domain, merges runs and installs donor identities, so
//! each of those rules needs a direct check. Runs have no accessor, so their layout is read from the set's derived
//! `Debug` form.

use super::{exact_span, identity};
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot_builder::TypeDisplaySnapshotBuilder;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::FactCodec;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::unreachable_match_arm;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::{
    duplicate_declaration, unknown_value_name,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    CapacityResourceFact, CompilerCapacityExceededView, compiler_capacity_exceeded,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    IncompatibleChoiceComparisonView, incompatible_choice_comparison,
};
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::{
    ForeignSiteDraft, ForeignSlot, SiteRef,
};
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::{
    DomainSource, InstallMode, PendingDiagnostics,
};
use crate::compiler_frontend::compiler_messages::diagnostics::report::{
    DiagnosticReport, DiagnosticReportSet, SeverityCounts,
};
use crate::compiler_frontend::compiler_messages::diagnostics::resource::DiagnosticLimits;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::{
    CapacityResource, DiagnosticCapacityResource, DiagnosticLabelMessage,
    IncompatibleChoiceComparisonReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{
    FrozenIdentityContext, FrozenIdentityHandle, SourceDatabase,
};
use crate::compiler_frontend::symbols::path_interner::{PathInternerBuilder, PathTable};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use std::sync::Arc;

/// Pushes one choice comparison whose type words are captured through a fresh snapshot, which `domain` appends.
fn push_choice_comparison(
    batch: &mut PendingDiagnostics,
    domain: u32,
    environment: &TypeEnvironment,
    lhs: TypeId,
    rhs: TypeId,
    start: u32,
) {
    let mut builder = TypeDisplaySnapshotBuilder::new(Some(environment));
    let mut draft = incompatible_choice_comparison(
        IncompatibleChoiceComparisonReason::DifferentChoiceTypes,
        lhs,
        rhs,
        Some(exact_span(start, 1)),
    );
    draft
        .capture_types(&mut |type_id| builder.display_id(type_id))
        .expect("a builtin display fits");
    let base = batch
        .append_types(domain, builder.finish())
        .expect("the snapshot appends");
    draft
        .rebase_type_displays(base)
        .expect("the rebased display ids fit");
    batch.push(domain, None, draft);
}

/// The spelling that `type_id` gets from a snapshot that holds that type alone.
fn spelling_alone(environment: &TypeEnvironment, type_id: TypeId, paths: &PathTable) -> String {
    let mut builder = TypeDisplaySnapshotBuilder::new(Some(environment));
    let display = builder.display_id(type_id).expect("a builtin display fits");
    builder
        .finish()
        .display_for_render(display, &StringTable::new(), Some(paths))
}

/// The report that owns each record, in iteration order. Records of one report share a pointer.
fn owning_reports(set: &DiagnosticReportSet) -> Vec<*const DiagnosticReport> {
    set.iter()
        .map(|(report, _)| std::ptr::from_ref(report))
        .collect()
}

/// Asserts the run layout as `(report, first, len)` triples, in order. Runs have no accessor, so this reads the
/// set's derived `Debug` form, which ends with the `runs` list.
fn assert_runs(set: &DiagnosticReportSet, expected: &[(u32, u32, u32)]) {
    let listed = expected
        .iter()
        .map(|(report, first, len)| {
            format!("DiagnosticRun {{ report: {report}, first: {first}, len: {len} }}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let debug = format!("{set:?}");
    let (_, runs) = debug
        .split_once("runs: [")
        .expect("the debug form lists the runs");
    assert!(
        runs.starts_with(&format!("{listed}]")),
        "expected runs [{listed}], found [{runs}"
    );
}

#[test]
fn freeze_empty_batch_yields_an_empty_set() {
    let set = PendingDiagnostics::default().freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Frozen,
    );

    assert!(set.is_empty());
    assert_eq!(set.counts(), SeverityCounts::default());
    assert_eq!(set.iter().count(), 0);
    assert_eq!(
        format!("{set:?}"),
        format!("{:?}", DiagnosticReportSet::empty())
    );
}

#[test]
fn freeze_keeps_one_domain_records_in_one_run() {
    let mut batch = PendingDiagnostics::default();
    let domain = batch.add_domain(None, DomainSource::Sources(SourceDatabase::empty()));
    batch.push(
        domain,
        None,
        unknown_value_name(StringId::from_index(1), Some(exact_span(0, 1))),
    );
    batch.push(domain, None, unreachable_match_arm(Some(exact_span(2, 1))));
    batch.push(domain, None, unreachable_match_arm(Some(exact_span(4, 1))));

    let set = batch.freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Frozen,
    );

    assert_eq!(set.iter().count(), 3);
    assert_eq!(
        set.counts(),
        SeverityCounts {
            errors: 1,
            warnings: 2,
            notes: 0,
        }
    );
    assert_runs(&set, &[(0, 0, 3)]);
}

#[test]
fn freeze_interleaved_domains_keep_pushed_order() {
    let mut batch = PendingDiagnostics::default();
    let project = batch.add_domain(None, DomainSource::Sources(SourceDatabase::empty()));
    let package = batch.add_domain(
        Some(StablePackageIdentity::project_local("freeze-order")),
        DomainSource::Sources(SourceDatabase::empty()),
    );
    batch.push(project, None, unreachable_match_arm(Some(exact_span(0, 1))));
    batch.push(package, None, unreachable_match_arm(Some(exact_span(2, 1))));
    batch.push(project, None, unreachable_match_arm(Some(exact_span(4, 1))));

    let set = batch.freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Frozen,
    );
    let owners = owning_reports(&set);

    assert_eq!(owners.len(), 3);
    assert_eq!(owners[0], owners[2]);
    assert_ne!(owners[0], owners[1]);
    assert_runs(&set, &[(0, 0, 1), (1, 0, 1), (0, 1, 1)]);
}

#[test]
fn freeze_merges_a_run_across_a_refused_stretch() {
    // The limit applies to each domain's store. Each store keeps one draft and turns its second draft into the
    // stand-in, so its third draft is refused.
    let limits = DiagnosticLimits {
        records: 2,
        ..DiagnosticLimits::production()
    };
    let mut batch = PendingDiagnostics::default();
    let project = batch.add_domain(None, DomainSource::Sources(SourceDatabase::empty()));
    let package = batch.add_domain(
        Some(StablePackageIdentity::project_local("freeze-merge")),
        DomainSource::Sources(SourceDatabase::empty()),
    );
    batch.push(package, None, unreachable_match_arm(Some(exact_span(0, 1))));
    batch.push(package, None, unreachable_match_arm(Some(exact_span(2, 1))));
    batch.push(project, None, unreachable_match_arm(Some(exact_span(4, 1))));
    batch.push(package, None, unreachable_match_arm(Some(exact_span(6, 1))));
    batch.push(project, None, unreachable_match_arm(Some(exact_span(8, 1))));

    let set = batch.freeze(StringTable::new(), limits, InstallMode::Frozen);
    let owners = owning_reports(&set);

    assert_eq!(set.iter().count(), 4);
    assert_eq!(
        set.counts(),
        SeverityCounts {
            errors: 2,
            warnings: 2,
            notes: 0,
        }
    );
    assert_eq!(owners[0], owners[1]);
    assert_eq!(owners[2], owners[3]);
    assert_ne!(owners[0], owners[2]);
    // The package's refused draft adds no project record, so the project's stand-in continues the project's first run.
    assert_runs(&set, &[(1, 0, 2), (0, 0, 2)]);
}

#[test]
fn freeze_shared_domain_reuses_the_earlier_context() {
    let package = StablePackageIdentity::project_local("freeze-shared-context");
    let package_handle = FrozenIdentityHandle::for_domain(package.clone());
    let mut batch = PendingDiagnostics::default();
    let project = batch.add_domain(None, DomainSource::Sources(SourceDatabase::empty()));
    let shared = batch.add_domain(Some(package), DomainSource::Shares(project));
    batch.push(project, None, unreachable_match_arm(Some(exact_span(0, 1))));
    batch.push(
        shared,
        None,
        unreachable_match_arm(Some(exact_span(2, 1))).with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: package_handle.clone(),
        }),
    );

    let set = batch.freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Frozen,
    );
    let contexts: Vec<&FrozenIdentityContext> =
        set.iter().map(|(report, _)| report.context()).collect();

    assert_eq!(contexts.len(), 2);
    assert!(std::ptr::eq(contexts[0], contexts[1]));
    let installed = package_handle
        .get()
        .expect("the shared domain's donor is installed");
    assert!(std::ptr::eq(installed, contexts[1]));
}

#[test]
fn freeze_unowned_domain_resolves_no_spans() {
    let domain_identity = StablePackageIdentity::project_local("freeze-unowned");
    let handle = FrozenIdentityHandle::for_domain(domain_identity.clone());
    let paths = Arc::new(PathInternerBuilder::new().freeze());

    let mut batch = PendingDiagnostics::default();
    let domain = batch.add_domain(
        Some(domain_identity),
        DomainSource::Unowned(Arc::clone(&paths)),
    );
    batch.push(
        domain,
        None,
        unreachable_match_arm(Some(exact_span(0, 1))).with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: handle.clone(),
        }),
    );

    let set = batch.freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Frozen,
    );
    let (report, _) = set.iter().next().expect("the pushed draft is stored");
    assert!(std::ptr::eq(report.context().paths(), Arc::as_ptr(&paths)));

    let strings = StringTable::new();
    let position = DiagnosticRenderContext::new(&strings)
        .with_frozen_identity(report.context())
        .site_position(SiteRef::Foreign {
            span: exact_span(0, 1),
            context: &handle,
        });
    assert!(position.is_none());
}

#[test]
fn freeze_unowned_domain_leaves_an_unmatched_donor_uninstalled() {
    let handle = FrozenIdentityHandle::for_domain(StablePackageIdentity::project_local(
        "freeze-unowned-orphan",
    ));
    let paths = Arc::new(PathInternerBuilder::new().freeze());
    let mut batch = PendingDiagnostics::default();
    let domain = batch.add_domain(
        Some(StablePackageIdentity::project_local("freeze-unowned-owner")),
        DomainSource::Unowned(Arc::clone(&paths)),
    );
    batch.push(
        domain,
        None,
        unreachable_match_arm(Some(exact_span(0, 1))).with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: handle.clone(),
        }),
    );

    let set = batch.freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Frozen,
    );

    assert_eq!(set.iter().count(), 1);
    assert!(handle.get().is_none());
}

#[test]
fn freeze_domain_less_handle_on_package_draft_stays_uninstalled() {
    let handle = identity();
    let mut batch = PendingDiagnostics::default();
    let package = batch.add_domain(
        Some(StablePackageIdentity::project_local("freeze-package-owned")),
        DomainSource::Sources(SourceDatabase::empty()),
    );
    batch.push(
        package,
        None,
        unreachable_match_arm(Some(exact_span(0, 1))).with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: handle.clone(),
        }),
    );

    let set = batch.freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Frozen,
    );

    assert_eq!(set.iter().count(), 1);
    assert!(handle.get().is_none());
}

#[test]
fn freeze_domain_less_handles_install_their_own_none_domain() {
    let first_handle = identity();
    let second_handle = identity();
    let mut batch = PendingDiagnostics::default();
    let first = batch.add_domain(None, DomainSource::Sources(SourceDatabase::empty()));
    let second = batch.add_domain(None, DomainSource::Sources(SourceDatabase::empty()));
    batch.push(
        first,
        Some(first),
        unreachable_match_arm(Some(exact_span(0, 1))).with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: first_handle.clone(),
        }),
    );
    batch.push(
        second,
        Some(second),
        unreachable_match_arm(Some(exact_span(2, 1))).with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: second_handle.clone(),
        }),
    );

    let set = batch.freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Frozen,
    );
    let contexts: Vec<&FrozenIdentityContext> =
        set.iter().map(|(report, _)| report.context()).collect();

    assert_eq!(contexts.len(), 2);
    assert!(!std::ptr::eq(contexts[0], contexts[1]));
    let first_installed = first_handle
        .get()
        .expect("the first domain's donor is installed");
    let second_installed = second_handle
        .get()
        .expect("the second domain's donor is installed");
    assert!(std::ptr::eq(first_installed, contexts[0]));
    assert!(std::ptr::eq(second_installed, contexts[1]));
}

#[test]
fn freeze_domain_less_handles_follow_none_domain_across_interleaved_drafts() {
    let first_handle = identity();
    let second_handle = identity();
    let mut batch = PendingDiagnostics::default();
    let first = batch.add_domain(None, DomainSource::Sources(SourceDatabase::empty()));
    let second = batch.add_domain(
        Some(StablePackageIdentity::project_local("freeze-none-second")),
        DomainSource::Sources(SourceDatabase::empty()),
    );
    // Each identity-less handle takes its draft's `none_domain` context, not the owning domain's context.
    batch.push(
        first,
        Some(second),
        unreachable_match_arm(Some(exact_span(0, 1))).with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: first_handle.clone(),
        }),
    );
    batch.push(
        second,
        Some(first),
        unreachable_match_arm(Some(exact_span(2, 1))).with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: second_handle.clone(),
        }),
    );

    let set = batch.freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Frozen,
    );
    let contexts: Vec<&FrozenIdentityContext> =
        set.iter().map(|(report, _)| report.context()).collect();

    assert_eq!(contexts.len(), 2);
    let first_installed = first_handle
        .get()
        .expect("the first draft's handle is installed");
    let second_installed = second_handle
        .get()
        .expect("the second draft's handle is installed");
    assert!(std::ptr::eq(first_installed, contexts[1]));
    assert!(std::ptr::eq(second_installed, contexts[0]));
}

#[test]
fn freeze_package_handle_installs_the_first_matching_domain() {
    let shared = StablePackageIdentity::project_local("freeze-shared");
    let handle = FrozenIdentityHandle::for_domain(shared.clone());
    let mut batch = PendingDiagnostics::default();
    let first = batch.add_domain(
        Some(shared.clone()),
        DomainSource::Sources(SourceDatabase::empty()),
    );
    let second = batch.add_domain(Some(shared), DomainSource::Sources(SourceDatabase::empty()));
    batch.push(first, None, unreachable_match_arm(Some(exact_span(0, 1))));
    batch.push(
        second,
        None,
        unreachable_match_arm(Some(exact_span(2, 1))).with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: handle.clone(),
        }),
    );

    let set = batch.freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Frozen,
    );
    let contexts: Vec<&FrozenIdentityContext> =
        set.iter().map(|(report, _)| report.context()).collect();

    assert_eq!(contexts.len(), 2);
    let installed = handle.get().expect("the donor is installed");
    assert!(std::ptr::eq(installed, contexts[0]));
    assert!(!std::ptr::eq(installed, contexts[1]));
}

#[test]
fn freeze_keeps_a_preinstalled_handle() {
    let shared = StablePackageIdentity::project_local("freeze-preinstalled");
    let handle = FrozenIdentityHandle::for_domain(shared.clone());
    let early = Arc::new(FrozenIdentityContext::from_parts(
        StringTable::new(),
        SourceDatabase::empty(),
    ));
    handle
        .install(Arc::clone(&early))
        .expect("the handle accepts its first context");

    let mut batch = PendingDiagnostics::default();
    let domain = batch.add_domain(Some(shared), DomainSource::Sources(SourceDatabase::empty()));
    batch.push(
        domain,
        None,
        unreachable_match_arm(Some(exact_span(0, 1))).with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: handle.clone(),
        }),
    );

    let set = batch.freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Frozen,
    );
    let (report, _) = set.iter().next().expect("the pushed draft is stored");

    let installed = handle.get().expect("the early donor stays installed");
    assert!(std::ptr::eq(installed, Arc::as_ptr(&early)));
    assert!(!std::ptr::eq(report.context(), Arc::as_ptr(&early)));
}

#[test]
#[should_panic(
    expected = "diagnostic primary reached the frozen render boundary without its donor identity"
)]
fn freeze_panics_on_donor_primary_without_its_domain() {
    let mut batch = PendingDiagnostics::default();
    let project = batch.add_domain(None, DomainSource::Sources(SourceDatabase::empty()));
    batch.push(
        project,
        None,
        unreachable_match_arm(Some(exact_span(0, 1))).with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: FrozenIdentityHandle::for_domain(StablePackageIdentity::project_local(
                "freeze-orphan",
            )),
        }),
    );

    let _set = batch.freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Frozen,
    );
}

#[test]
#[should_panic(
    expected = "diagnostic label reached the frozen render boundary without its donor identity"
)]
fn freeze_panics_on_donor_label_without_its_domain() {
    let mut batch = PendingDiagnostics::default();
    let project = batch.add_domain(None, DomainSource::Sources(SourceDatabase::empty()));
    batch.push(
        project,
        None,
        duplicate_declaration(StringId::from_index(1), Some(exact_span(10, 2)))
            .with_label(
                Some(exact_span(0, 2)),
                Some(DiagnosticLabelMessage::PreviousDeclaration),
            )
            .with_foreign(ForeignSiteDraft {
                slot: ForeignSlot::Label(0),
                context: FrozenIdentityHandle::for_domain(StablePackageIdentity::project_local(
                    "freeze-orphan-label",
                )),
            }),
    );

    let _set = batch.freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Frozen,
    );
}

#[test]
fn unfrozen_freeze_skips_identity_installs_and_keeps_none_domain() {
    let package = StablePackageIdentity::project_local("freeze-unfrozen");
    let named_handle = FrozenIdentityHandle::for_domain(package.clone());
    let domain_less_handle = identity();
    let mut batch = PendingDiagnostics::default();
    let project = batch.add_domain(None, DomainSource::Sources(SourceDatabase::empty()));
    batch.add_domain(
        Some(package),
        DomainSource::Sources(SourceDatabase::empty()),
    );
    batch.push(
        project,
        Some(project),
        unreachable_match_arm(Some(exact_span(0, 1))).with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: domain_less_handle.clone(),
        }),
    );
    batch.push(
        project,
        None,
        unreachable_match_arm(Some(exact_span(2, 1))).with_foreign(ForeignSiteDraft {
            slot: ForeignSlot::Primary,
            context: named_handle.clone(),
        }),
    );

    let set = batch.freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Unfrozen,
    );
    let contexts: Vec<&FrozenIdentityContext> =
        set.iter().map(|(report, _)| report.context()).collect();

    assert_eq!(contexts.len(), 2);
    let installed = domain_less_handle
        .get()
        .expect("the none domain's donor is installed");
    assert!(std::ptr::eq(installed, contexts[0]));
    assert!(named_handle.get().is_none());
}

#[test]
#[should_panic(expected = "diagnostic domains may share only the context of an earlier domain")]
fn add_domain_panics_when_a_share_names_a_later_domain() {
    let mut batch = PendingDiagnostics::default();
    batch.add_domain(None, DomainSource::Sources(SourceDatabase::empty()));
    batch.add_domain(None, DomainSource::Shares(1));
}

#[test]
fn freeze_full_store_keeps_run_accounting() {
    let limits = DiagnosticLimits {
        records: 2,
        ..DiagnosticLimits::production()
    };
    let mut batch = PendingDiagnostics::default();
    let domain = batch.add_domain(None, DomainSource::Sources(SourceDatabase::empty()));
    for start in [0, 2, 4] {
        batch.push(
            domain,
            None,
            unreachable_match_arm(Some(exact_span(start, 1))),
        );
    }

    let set = batch.freeze(StringTable::new(), limits, InstallMode::Frozen);

    // One draft is stored, the second becomes the stand-in, and the third is dropped without a record.
    assert_eq!(set.iter().count(), 2);
    assert_eq!(
        set.counts(),
        SeverityCounts {
            errors: 1,
            warnings: 1,
            notes: 0,
        }
    );
    assert_runs(&set, &[(0, 0, 2)]);
}

#[test]
fn freeze_keeps_a_type_display_fault_as_one_error_record() {
    let mut batch = PendingDiagnostics::default();
    let domain = batch.add_domain(None, DomainSource::Sources(SourceDatabase::empty()));
    batch.push(
        domain,
        None,
        compiler_capacity_exceeded(
            CapacityResource::Store(DiagnosticCapacityResource::TypeDisplays),
            Some(exact_span(0, 1)),
        ),
    );

    let set = batch.freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Frozen,
    );

    assert_eq!(set.iter().count(), 1);
    assert_eq!(
        set.counts(),
        SeverityCounts {
            errors: 1,
            warnings: 0,
            notes: 0,
        }
    );

    let strings = StringTable::new();
    let (report, view) = set.iter().next().expect("the fault is stored");
    let facts = view
        .family_view::<CompilerCapacityExceededView>()
        .expect("the record is a capacity fault");
    assert!(matches!(
        facts.resource,
        CapacityResource::Store(DiagnosticCapacityResource::TypeDisplays)
    ));
    let render = view.family().render.expect("capacity faults render");
    let rendered = render(
        &view,
        DiagnosticRenderContext::new(&strings).with_frozen_identity(report.context()),
    );
    assert_eq!(
        rendered.message,
        "This source exceeds the compiler's capacity for diagnostic type displays."
    );
}

#[test]
fn freeze_resolves_type_words_through_their_domain_snapshot() {
    let environment = TypeEnvironment::new();
    let builtins = environment.builtins();
    let paths = PathInternerBuilder::new().freeze();
    let strings = StringTable::new();

    let mut batch = PendingDiagnostics::default();
    let project = batch.add_domain(None, DomainSource::Sources(SourceDatabase::empty()));
    let package = batch.add_domain(
        Some(StablePackageIdentity::project_local("freeze-types")),
        DomainSource::Sources(SourceDatabase::empty()),
    );
    // Two snapshots land in the project domain, so the second one's display ids rebase past the first.
    push_choice_comparison(
        &mut batch,
        project,
        &environment,
        builtins.string,
        builtins.bool,
        0,
    );
    push_choice_comparison(
        &mut batch,
        project,
        &environment,
        builtins.bool,
        builtins.string,
        2,
    );
    push_choice_comparison(
        &mut batch,
        package,
        &environment,
        builtins.bool,
        builtins.bool,
        4,
    );

    let set = batch.freeze(
        StringTable::new(),
        DiagnosticLimits::production(),
        InstallMode::Frozen,
    );
    let resolved: Vec<(String, String)> = set
        .iter()
        .map(|(report, view)| {
            let facts = view
                .family_view::<IncompatibleChoiceComparisonView>()
                .expect("the record is an incompatible choice comparison");
            let resolve = |display| {
                view.types()
                    .display_for_render(display, &strings, Some(report.context().paths()))
            };
            (resolve(facts.lhs), resolve(facts.rhs))
        })
        .collect();

    let string = spelling_alone(&environment, builtins.string, &paths);
    let boolean = spelling_alone(&environment, builtins.bool, &paths);
    assert_ne!(string, boolean);
    assert_eq!(
        resolved,
        vec![
            (string.clone(), boolean.clone()),
            (boolean.clone(), string.clone()),
            (boolean.clone(), boolean.clone()),
        ]
    );
}

#[test]
fn append_types_returns_each_snapshot_base_in_push_order() {
    let environment = TypeEnvironment::new();
    let mut first_builder = TypeDisplaySnapshotBuilder::new(Some(&environment));
    first_builder
        .display_id(environment.builtins().string)
        .expect("a builtin display fits");
    let first = first_builder.finish();
    let mut second_builder = TypeDisplaySnapshotBuilder::new(Some(&environment));
    second_builder
        .display_id(environment.builtins().bool)
        .expect("a builtin display fits");
    let second = second_builder.finish();

    let mut batch = PendingDiagnostics::default();
    let domain = batch.add_domain(None, DomainSource::Sources(SourceDatabase::empty()));
    let first_base = batch
        .append_types(domain, first)
        .expect("the first snapshot appends");
    let second_base = batch
        .append_types(domain, second)
        .expect("the second snapshot appends");

    assert_eq!(first_base, 0);
    assert!(second_base > first_base);
}

#[test]
fn type_capacity_draft_uses_type_displays_code() {
    let mut word = [0u32; 1];
    CapacityResourceFact::encode(
        &CapacityResource::Store(DiagnosticCapacityResource::TypeDisplays),
        &mut word,
    )
    .expect("a capacity resource always encodes");
    assert_eq!(word, [522]);
}
