use super::*;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::PreparedFailure;
use crate::compiler_frontend::compiler_messages::compiler_bug::CompilerBugReport;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::CircularDependencyView;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::render::{DiagnosticRenderContext, terse};
fn assert_compiler_bug(payload: Box<dyn std::any::Any + Send>, expected_message: &str) {
    let report = payload
        .downcast::<CompilerBugReport>()
        .unwrap_or_else(|_| panic!("expected a CompilerBugReport payload"));
    assert_eq!(report.stage, CompilerStage::ProjectBuildAssembly);
    assert_eq!(report.message.as_ref(), expected_message);
}
#[test]
fn package_ordering_invariant_failures_raise_compiler_bugs() {
    let package_prefixes = vec!["a".to_owned()];
    let dependency_entries = Vec::new();
    let payload = std::panic::catch_unwind(|| {
        super::compilation::order_packages_by_dependency(&package_prefixes, &dependency_entries)
    })
    .expect_err("dependency entry count mismatch must raise a compiler bug");
    assert_compiler_bug(
        payload,
        "package dependency schedule received 1 packages but 0 dependency entries",
    );

    let package_prefixes = vec!["a".to_owned(), "a".to_owned()];
    let dependency_entries = vec![Vec::new(), Vec::new()];
    let payload = std::panic::catch_unwind(|| {
        super::compilation::order_packages_by_dependency(&package_prefixes, &dependency_entries)
    })
    .expect_err("duplicate package inventory rows must raise a compiler bug");
    assert_compiler_bug(
        payload,
        "source package @a appears more than once in the package inventory",
    );

    let package_prefixes = vec!["a".to_owned()];
    let dependency_entries = dependency_prefixes(&[&["missing"]]);
    let payload = std::panic::catch_unwind(|| {
        super::compilation::order_packages_by_dependency(&package_prefixes, &dependency_entries)
    })
    .expect_err("an unindexed package dependency must raise a compiler bug");
    assert_compiler_bug(
        payload,
        "Source package @a depends on unindexed source package @missing",
    );
}
// ---------------------------------------------------------------------------
// Package ordering and registry invariants
// ---------------------------------------------------------------------------

#[test]
fn package_ordering_is_deterministic_across_reversed_discovery_order() {
    let prefixes = vec!["a".to_owned(), "b".to_owned()];
    let dependencies = dependency_prefixes(&[&[], &["a"]]);
    let order = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect("acyclic package graph orders");
    assert_eq!(order, vec![0, 1]);
    let prefixes_in_order = order
        .iter()
        .map(|index| prefixes[*index].as_str())
        .collect::<Vec<_>>();
    assert_eq!(prefixes_in_order, vec!["a", "b"]);

    // The same graph presented in reversed discovery order must yield the same prefix sequence.
    let prefixes = vec!["b".to_owned(), "a".to_owned()];
    let dependencies = dependency_prefixes(&[&["a"], &[]]);
    let order = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect("acyclic package graph orders");
    assert_eq!(order, vec![1, 0]);
    let prefixes_in_order = order
        .iter()
        .map(|index| prefixes[*index].as_str())
        .collect::<Vec<_>>();
    assert_eq!(prefixes_in_order, vec!["a", "b"]);

    // Independent packages tie-break by prefix, so reversed discovery order still yields the
    // same prefix sequence.
    let prefixes = vec!["a".to_owned(), "b".to_owned()];
    let dependencies = dependency_prefixes(&[&[], &[]]);
    let order = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect("independent package graph orders");
    assert_eq!(order, vec![0, 1]);

    let prefixes = vec!["b".to_owned(), "a".to_owned()];
    let dependencies = dependency_prefixes(&[&[], &[]]);
    let order = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect("independent package graph orders");
    assert_eq!(order, vec![1, 0]);
}

#[test]
fn package_ordering_tie_breaks_by_discovery_prefix_order() {
    let prefixes = vec!["z".to_owned(), "a".to_owned(), "m".to_owned()];
    let dependencies = dependency_prefixes(&[&[], &[], &[]]);
    let order = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect("independent packages order");
    assert_eq!(order, vec![1, 2, 0], "ready packages leave in prefix order");
}

#[test]
fn package_ordering_orders_diamond_dependencies_once() {
    let prefixes = vec![
        "a".to_owned(),
        "b".to_owned(),
        "c".to_owned(),
        "d".to_owned(),
    ];
    let dependencies = dependency_prefixes(&[&[], &["a"], &["a"], &["b", "c"]]);
    let order = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect("diamond package graph orders");
    assert_eq!(order, vec![0, 1, 2, 3]);
}

#[test]
fn package_ordering_reports_dependency_cycles_as_diagnosed_failures() {
    let prefixes = vec!["a".to_owned(), "b".to_owned()];
    let dependencies = dependency_prefixes(&[&["b"], &["a"]]);
    let error = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect_err("package dependency cycles are diagnosed");
    let PreparedFailure::Diagnosed(batch) = error else {
        panic!("package cycles must be diagnosed, not infrastructure: {error:?}");
    };
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(batch, None);
    let reports = accumulator
        .finish(InstallMode::Unfrozen)
        .expect("circular-dependency diagnostics should freeze");
    assert_eq!(reports.counts().errors, 1);
    assert!(
        reports
            .iter()
            .any(|(_, diagnostic)| diagnostic.family_view::<CircularDependencyView>().is_some()),
        "package cycles must use the circular-dependency family"
    );
}

#[test]
fn package_ordering_names_a_cycle_member_not_a_blocked_bystander() {
    // "a" only depends on the "b" <-> "c" cycle, and it precedes that cycle by index.
    let prefixes = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
    let dependencies = dependency_prefixes(&[&["b"], &["c"], &["b"]]);
    let error = super::compilation::order_packages_by_dependency(&prefixes, &dependencies)
        .expect_err("package dependency cycles are diagnosed");
    let PreparedFailure::Diagnosed(batch) = error else {
        panic!("package cycles must be diagnosed, not infrastructure: {error:?}");
    };
    let mut accumulator = BoundaryAccumulator::new();
    accumulator.append_prepared(batch, None);
    let reports = accumulator
        .finish(InstallMode::Unfrozen)
        .expect("circular-dependency diagnostics should freeze");
    assert_eq!(reports.counts().errors, 1);
    let (report, view) = reports
        .iter()
        .next()
        .expect("the cycle should retain one diagnostic");
    let rendered =
        terse::format_terse_diagnostic_view(view, DiagnosticRenderContext::for_report(report));
    assert!(
        rendered.contains("Circular dependency at 'b'")
            || rendered.contains("Circular dependency at 'c'"),
        "the reported package must lie on the cycle, not the blocked bystander: {rendered}"
    );
}

#[test]
fn completed_package_registry_rejects_duplicate_prefix() {
    let mut registry = CompletedSourcePackageRegistry::new();
    registry
        .publish(compiled_package("markdown"), &[])
        .expect("first package publishes");

    let error = registry
        .publish(compiled_package("markdown"), &[])
        .expect_err("one prefix must index exactly one completed package");
    assert!(error.message.contains("completed more than once"));
}

#[test]
fn completed_package_registry_records_direct_dependency_edges_once() {
    let mut registry = CompletedSourcePackageRegistry::new();
    let a = registry
        .publish(compiled_package("a"), &[])
        .expect("provider package publishes");
    let b = registry
        .publish(compiled_package("b"), &["a".to_owned()])
        .expect("consumer package publishes");

    registry
        .validate_dependency_edges()
        .expect("dependency-first publication order is valid");
    assert_eq!(
        registry.provider_packages(b).expect("b has provider edges"),
        &[a]
    );
    assert_eq!(
        registry.consumer_packages(a).expect("a has consumer edges"),
        &[b]
    );
    assert_eq!(registry.by_prefix("a"), Some(a));
    assert_eq!(registry.by_prefix("b"), Some(b));
    assert_eq!(registry.by_prefix("missing"), None);
}

#[test]
fn completed_package_registry_rejects_duplicate_dependency_input_without_mutation() {
    let mut registry = CompletedSourcePackageRegistry::new();
    let provider = registry
        .publish(compiled_package("provider"), &[])
        .expect("provider package publishes");

    let error = registry
        .publish(
            compiled_package("consumer"),
            &["provider".to_owned(), "provider".to_owned()],
        )
        .expect_err("duplicate provider input must be rejected before publication");

    assert!(error.message.contains("more than once"));
    assert_eq!(registry.len(), 1);
    assert_eq!(registry.by_prefix("consumer"), None);
    assert!(
        registry
            .consumer_packages(provider)
            .expect("provider has a consumer lane")
            .is_empty()
    );
}

#[test]
fn completed_package_registry_rejects_self_dependency_before_publication() {
    let mut registry = CompletedSourcePackageRegistry::new();
    let error = registry
        .publish(compiled_package("a"), &["a".to_owned()])
        .expect_err("a package must never depend on its own not-yet-published prefix");
    assert!(error.message.contains("unindexed source package @a"));
}

#[test]
fn module_package_dependency_index_walks_only_direct_dependencies() {
    let mut registry = CompletedSourcePackageRegistry::new();
    let a = registry
        .publish(compiled_package("a"), &[])
        .expect("provider package publishes");
    let b = registry
        .publish(compiled_package("b"), &["a".to_owned()])
        .expect("consumer package publishes");

    let consumer_module_id = ModuleId::from_index(5);
    let shell = DependencyShellId::new(SourceId::from_index(0), 0);
    let dependencies = vec![ResolvedSourcePackageDependency {
        consumer_module_id,
        dependency_prefix: "b".to_owned(),
        dependency_shell_id: shell,
    }];

    let index = super::compilation::build_module_package_dependency_index(&dependencies, &registry);
    assert_eq!(
        index.len(),
        1,
        "readiness must only visit direct package dependencies"
    );
    assert_eq!(
        index.get(&consumer_module_id),
        Some(&vec![b]),
        "the module depends on package b, not on transitive provider a"
    );
    assert!(a != b);
    assert!(
        !index.contains_key(&ModuleId::from_index(6)),
        "modules without package dependencies must not appear in the readiness index"
    );
}
