//! Public-export route and facade-name tests.
//!
//! WHAT: end-to-end Stage 0 and Stage 3 fixtures for namespaced `$export` routes, route collisions
//! and facade publication through the real project frontend.
//! WHY: these scenarios share the indexed registered-package and facade fixtures. Keeping them in
//! one owner makes route publication, collision precedence and deterministic route bindings
//! reviewable together.

use super::entry_api_stability_tests::{
    exported_constant_text, interface_fingerprint_content, normal_interface_for,
    reachable_external_functions,
};
use super::package_materialisation_tests::{
    RegisteredPackageFixture, RegisteredPackagesHtmlBuilder, compile_registered_package_project,
    compile_registered_packages_project, compile_registered_packages_project_retained,
    compile_registered_packages_project_with_damaged_sources, diagnostic_canonical_sites,
    diagnostic_sites,
};
use super::*;
use crate::compiler_frontend::compiler_messages::DiagnosticDiscoveryStops;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::render::terminal::format_terminal_locations_for_test;
use crate::compiler_frontend::public_interface::{
    PublicDeclarationSemantics, PublicFunctionCategory, PublicSemanticInterface,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginDeclarationId, OriginTypeCategory,
};
use crate::compiler_frontend::source::ForcedSourceSnapshotLimitGuard;
use crate::compiler_frontend::utilities::portable_path_display::portable_path_text;

/// Returns the normal interface that publishes the leaf `name` under `route`.
fn normal_interface_with_route<'a>(
    frontend: &'a ProjectFrontendCompilation,
    route: &str,
    name: &str,
) -> &'a PublicSemanticInterface {
    frontend
        .project
        .successful_artefacts_in_module_id_order()
        .filter_map(|artifact| artifact.normal_interface.as_ref())
        .find(|interface| {
            interface
                .export_bindings
                .iter()
                .any(|binding| binding.route().as_str() == route && binding.public_name() == name)
        })
        .unwrap_or_else(|| panic!("expected a facade publishing {name:?} under route {route:?}"))
}

/// Returns the project module artefact whose support facade path ends with `suffix`.
fn artifact_with_support_facade<'a>(
    frontend: &'a ProjectFrontendCompilation,
    suffix: &str,
) -> &'a CompiledModuleArtifact {
    frontend
        .project
        .successful_artefacts_in_module_id_order()
        .find(|artifact| {
            artifact
                .support_interface
                .as_ref()
                .is_some_and(|interface| {
                    interface
                        .module_origin
                        .logical_module_path()
                        .ends_with(suffix)
                })
        })
        .unwrap_or_else(|| {
            panic!("expected a module artefact whose support facade ends with {suffix:?}")
        })
}

/// Returns the support interface that publishes the leaf `name` under `route`.
fn support_interface_with_route<'a>(
    frontend: &'a ProjectFrontendCompilation,
    route: &str,
    name: &str,
) -> &'a PublicSemanticInterface {
    frontend
        .project
        .successful_artefacts_in_module_id_order()
        .filter_map(|artifact| artifact.support_interface.as_ref())
        .find(|interface| {
            interface
                .export_bindings
                .iter()
                .any(|binding| binding.route().as_str() == route && binding.public_name() == name)
        })
        .unwrap_or_else(|| panic!("expected a support facade publishing {name:?} under {route:?}"))
}

/// Exact authored facts for one diagnostic site: start line/column (both zero-based) and excerpt.
type SiteFacts = ((u32, u32), String);

/// Return the primary and related authored sites for the first diagnostic with `code`.
fn diagnostic_site_facts<T>(result: &CompilationResult<T>, code: &str) -> (SiteFacts, SiteFacts) {
    let reports = super::super::create_project_modules_tests::diagnosed_reports(result);
    let (report, diagnostic) = reports
        .iter()
        .find(|(_, diagnostic)| diagnostic.descriptor().code == code)
        .unwrap_or_else(|| panic!("expected diagnostic {code}"));
    let context = DiagnosticRenderContext::for_report(report);
    let facts = |site: SiteRef<'_>| {
        let position = context
            .site_position(site)
            .unwrap_or_else(|| panic!("{code} site must resolve to authored source"));
        let excerpt = position
            .line
            .chars()
            .skip(position.start.column as usize)
            .take((position.end.column - position.start.column) as usize)
            .collect::<String>();
        ((position.start.line, position.start.column), excerpt)
    };
    let primary = facts(diagnostic.primary_site());
    let related_site = diagnostic
        .durable_labels()
        .find_map(|label| (!matches!(label.site, SiteRef::Absent)).then_some(label.site))
        .unwrap_or_else(|| panic!("{code} must retain a related authored site"));
    (primary, facts(related_site))
}

/// Assert one diagnostic by exact code retains a primary site and one related site.
fn assert_two_site_diagnostic<T>(result: &CompilationResult<T>, code: &str) {
    let reports = super::super::create_project_modules_tests::diagnosed_reports(result);
    let diagnostic = reports
        .iter()
        .find_map(|(_, diagnostic)| (diagnostic.descriptor().code == code).then_some(diagnostic))
        .unwrap_or_else(|| panic!("expected diagnostic {code}"));
    let has_site = |site: SiteRef<'_>| !matches!(site, SiteRef::Absent);
    assert!(
        has_site(diagnostic.primary_site()),
        "{code} must retain its primary authored site"
    );
    assert!(
        diagnostic
            .durable_labels()
            .any(|label| has_site(label.site)),
        "{code} must retain a related authored site"
    );
}

/// Assert exactly one report carries `code`.
///
/// WHY: a route collision must surface once whether it reports eagerly at attachment or after
///      deferral. A duplicated report, or a consumer import error replacing the boundary report,
///      means the publication path failed twice.
fn assert_single_diagnostic<T>(result: &CompilationResult<T>, code: &str, reason: &str) {
    let reports = super::super::create_project_modules_tests::diagnosed_reports(result);
    let occurrences = reports
        .iter()
        .filter(|(_, diagnostic)| diagnostic.descriptor().code == code)
        .count();
    assert_eq!(occurrences, 1, "{reason}");
}

#[test]
fn diagnosed_package_facade_is_prepared_once_and_reported_by_the_walk() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temporary = tempfile::tempdir().expect("should create temporary project");
    let project_root = temporary.path().to_path_buf();
    let package_root = project_root.join("packages/builder/test/once");
    fs::create_dir_all(project_root.join("src")).expect("should create source root");
    fs::create_dir_all(&package_root).expect("should create package root");
    fs::write(
        project_root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write project config");
    fs::write(project_root.join("src/@entry.moth"), "[:entry]\n")
        .expect("should write project entry");
    fs::create_dir_all(package_root.join("widgets")).expect("should create child facade folder");
    let broken_facade = package_root.join("widgets/@mod.moth");
    fs::write(
        package_root.join("@package.moth"),
        "$export\nroot_label #= \"root\"\n",
    )
    .expect("should write package root facade");
    fs::write(&broken_facade, "$export\nvalue #= 1\nvalue #= 2\n")
        .expect("should write broken child facade");

    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root("builder/test/once", package_root, PackageOrigin::Builder)
        .expect("the package root should register");
    crate::compiler_frontend::pipeline_test_support::reset_file_frontend_prepare_count_for_test(
        &project_root,
    );

    let mut config = Config::new(project_root.join("src"));
    config.project_name = "docs".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    let code = reports
        .iter()
        .next()
        .map(|(_, diagnostic)| diagnostic.descriptor().code)
        .expect("the broken child facade must report its preparation diagnostic");
    let (primary, _) = diagnostic_sites(&result, code);
    assert!(
        primary.0.ends_with("widgets/@mod.moth"),
        "the cached child failure must be reported by its boundary walk: {primary:?}"
    );
    assert_eq!(
        crate::compiler_frontend::pipeline_test_support::file_frontend_prepare_count_for_path_for_test(
            &broken_facade
        ),
        1,
        "a diagnosed facade must be tokenized and parsed exactly once"
    );
}

#[test]
fn child_facade_failure_does_not_mask_earlier_external_route_miss() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let result = compile_registered_packages_project(
        &[
            (
                "builder/a",
                PackageOrigin::Builder,
                &[(
                    "@package.moth",
                    "@builder/z/missing value\n$export\nconsumer #= value\n",
                )],
            ),
            (
                "builder/z",
                PackageOrigin::Builder,
                &[
                    ("@package.moth", "$export\nroot_label #= \"root\"\n"),
                    ("widgets/@mod.moth", "$export\nbroken #= \"unterminated\n"),
                ],
            ),
        ],
        "[:entry]\n",
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert_eq!(
        reports
            .iter()
            .next()
            .map(|(_, diagnostic)| diagnostic.descriptor().code),
        Some("MOTH-IMPORT-0015"),
        "a failed child facade must not make the whole package prefix unavailable"
    );
    let (primary, _) = diagnostic_sites(&result, "MOTH-IMPORT-0015");
    assert!(
        primary.0.ends_with("@package.moth"),
        "the earlier package consumer owns the route-miss diagnostic: {primary:?}"
    );
}

#[test]
fn unreferenced_descendant_local_provider_collides_with_package_route() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temporary = tempfile::tempdir().expect("should create temporary project");
    let project_root =
        fs::canonicalize(temporary.path()).expect("the temporary project root must canonicalize");
    let package_root = project_root.join("packages/vendor/lib");
    fs::create_dir_all(project_root.join("src/client/vendor/lib/published"))
        .expect("should create descendant local provider folder");
    fs::create_dir_all(&package_root).expect("should create package root");
    fs::write(
        project_root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write project config");
    fs::write(project_root.join("src/@entry.moth"), "[:entry]\n")
        .expect("should write project entry");
    fs::write(
        project_root.join("src/client/@mod.moth"),
        "$export\nclient_label #= \"client\"\n",
    )
    .expect("should write rooted consumer module");
    let local_provider = project_root.join("src/client/vendor/lib/published/details.moth");
    fs::write(&local_provider, "value #= 1\n").expect("should write descendant local provider");
    fs::write(
        package_root.join("@package.moth"),
        "$export @published\nvalue #= 1\n",
    )
    .expect("should write routed package facade");

    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root("vendor/lib", package_root, PackageOrigin::ProjectLocal)
        .expect("the package root should register");

    let mut config = Config::new(project_root.join("src"));
    config.project_name = "docs".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    );
    let provider_path =
        portable_path_text(project_root.join("src/client/vendor/lib/published/details.moth"));
    let operand_path = portable_path_text(project_root.join("packages/vendor/lib/@package.moth"));
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert_eq!(
        primary.0, provider_path,
        "the matching local provider resolves to its exact canonical host path: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 0));
    let related = related.expect(
        "the package `$export` operand keeps its authored site through the package report domain",
    );
    assert_eq!(
        related.0, operand_path,
        "the published route operand resolves to its exact canonical host path: {related:?}"
    );
    assert_eq!(
        (related.1, related.2),
        (0, 8),
        "the related site is the `$export` operand, not the package root file start"
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    let (report, view) = reports
        .iter()
        .find(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006")
        .expect("the eager route collision must be diagnosed");
    let terminal = format_terminal_locations_for_test(
        view,
        DiagnosticRenderContext::for_report(report),
        &project_root,
    );
    assert_eq!(
        terminal,
        [
            "src/client/vendor/lib/published/details.moth:1:1".to_owned(),
            "info: packages/vendor/lib/@package.moth:1:9 - previous declaration here".to_owned()
        ],
        "terminal must name exactly the provider and package operand locations"
    );
    let rendered = render_compilation_result_html(&result, &project_root);
    assert!(
        rendered.contains("Ambiguous dependency target &#39;vendor/lib/published&#39;"),
        "the diagnostic facts must name the published package route: {rendered}"
    );
    assert!(
        rendered.contains("--> src/client/vendor/lib/published/details.moth:1:1"),
        "HTML must render the project provider at its exact location: {rendered}"
    );
    assert!(
        rendered.contains("packages/vendor/lib/@package.moth:1:9"),
        "HTML must render the package operand at its exact location: {rendered}"
    );
}

#[cfg(unix)]
#[test]
fn route_collision_canonicalises_a_symlink_spelled_display_root() {
    use std::os::unix::fs::symlink;

    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temporary = tempfile::tempdir().expect("should create temporary project");
    let real_root = temporary.path().join("real");
    fs::create_dir_all(real_root.join("src/client/vendor/lib/published"))
        .expect("should create descendant local provider folder");
    fs::create_dir_all(real_root.join("packages/vendor/lib")).expect("should create package root");
    fs::write(
        real_root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write project config");
    fs::write(real_root.join("src/@entry.moth"), "[:entry]\n").expect("should write project entry");
    fs::write(
        real_root.join("src/client/@mod.moth"),
        "$export\nclient_label #= \"client\"\n",
    )
    .expect("should write rooted consumer module");
    fs::write(
        real_root.join("src/client/vendor/lib/published/details.moth"),
        "value #= 1\n",
    )
    .expect("should write descendant local provider");
    fs::write(
        real_root.join("packages/vendor/lib/@package.moth"),
        "$export @published\nvalue #= 1\n",
    )
    .expect("should write routed package facade");
    let alias_root = temporary.path().join("alias");
    symlink(&real_root, &alias_root).expect("should create the project root alias");
    let canonical_root = fs::canonicalize(&alias_root).expect("the alias must resolve");

    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root(
            "vendor/lib",
            alias_root.join("packages/vendor/lib"),
            PackageOrigin::ProjectLocal,
        )
        .expect("the package root should register through the alias");
    let mut config = Config::new(alias_root.join("src"));
    config.project_name = "docs".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert_eq!(
        primary.0,
        portable_path_text(canonical_root.join("src/client/vendor/lib/published/details.moth")),
        "the display root must be canonical, not the symlink spelling: {primary:?}"
    );
    let related = related.expect("the package operand keeps its authored site");
    assert_eq!(
        related.0,
        portable_path_text(canonical_root.join("packages/vendor/lib/@package.moth")),
        "the package domain must also resolve through the canonical root: {related:?}"
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    let (report, view) = reports
        .iter()
        .find(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006")
        .expect("the route collision must be diagnosed");
    let terminal = format_terminal_locations_for_test(
        view,
        DiagnosticRenderContext::for_report(report),
        &canonical_root,
    );
    assert_eq!(
        terminal,
        [
            "src/client/vendor/lib/published/details.moth:1:1".to_owned(),
            "info: packages/vendor/lib/@package.moth:1:9 - previous declaration here".to_owned()
        ],
        "terminal rendering must relativise against exactly the canonical display root"
    );
    let rendered = render_compilation_result_html(&result, &canonical_root);
    assert!(
        rendered.contains("--> src/client/vendor/lib/published/details.moth:1:1"),
        "HTML must use the canonical display root: {rendered}"
    );
}

#[test]
fn registered_source_package_route_selects_root_facade_and_internal_child_route() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let mut discovery_stops = DiagnosticDiscoveryStops::NONE;
    let _temporary_directory = tempfile::tempdir().expect("should create temporary project");
    let project_root = _temporary_directory.path().to_path_buf();
    let package_root = project_root.join("packages/builder/test/routepkg");
    fs::create_dir_all(project_root.join("src")).expect("should create project source root");
    fs::create_dir_all(&package_root).expect("should create routed package root");
    fs::write(
        project_root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write project config");
    fs::write(
        project_root.join("@entry.moth"),
        "@builder/test/routepkg/forms decorate as decorate_button\n\
         @builder/test/routepkg/forms as route_ns\n\
         @builder/test/routepkg root_label\n\n\
         primary = decorate_button\n\
         bare = route_ns.decorate\n\
         ~[:primary=[primary] bare=[bare] root=[root_label]]\n",
    )
    .expect("should write routed route consumer");
    fs::create_dir_all(package_root.join("widgets")).expect("should create child facade folder");
    fs::create_dir_all(package_root.join("forms")).expect("should create private route folder");
    fs::write(
        package_root.join("@package.moth"),
        "@widgets/forms helper\n\n\
         $export @forms\n\
         decorate #= [:decorated:[helper]]\n\n\
         $export\n\
         root_label #= \"root\"\n",
    )
    .expect("should write routed package root facade");
    fs::write(
        package_root.join("widgets/@mod.moth"),
        "$export @forms\n\
         helper #= \"child\"\n",
    )
    .expect("should write internal child facade");
    fs::write(
        package_root.join("forms/secret.moth"),
        "secret #= \"private\"\n",
    )
    .expect("should write unrelated private source beside the route name");

    let builder = RegisteredPackagesHtmlBuilder {
        packages: vec![("builder/test/routepkg".to_owned(), package_root)],
    };
    let mut built = successful_compilation_value(
        build_project(
            &builder,
            project_root
                .to_str()
                .expect("temporary project path should be UTF-8"),
            &[],
            &BuildConfigInputSet::new(),
            &mut discovery_stops,
        )
        .unwrap_or_else(|never| match never {}),
    );
    let rendered =
        crate::compiler_tests::integration_test_runner::assertions::execute_html_harness_for_test(
            &mut built,
        )
        .expect("the routed package build should execute");
    assert_eq!(rendered.runtime_error_message(), None);
    assert_eq!(
        rendered.combined_output(),
        "primary=decorated:child bare=decorated:child root=root",
        "the external route, bare route namespace and root surface must all bind: {:?}",
        rendered.events()
    );
}

#[test]
fn route_under_an_ambiguous_base_reports_the_base_collision() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // Both support facades share the consumer-relative base `forms`. Only one `CrossModule` row
    // can remain in the entry table, so the second facade's route must still participate in
    // ambiguity: `@forms/y` names a real route on the discarded claimant, not an undeclared path.
    let result = compile_fixture_result(
        &[
            ("src/@entry.moth", "@forms/y value\n\n~[:[value]]\n"),
            ("src/@mod.moth", ""),
            ("src/a/forms/@package.moth", "$export @x\nvalue #= 1\n"),
            ("src/b/forms/@package.moth", "$export @y\nvalue #= 2\n"),
        ],
        false,
        BuildProfile::Dev,
    );
    let (primary, related) = diagnostic_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("@entry.moth"),
        "the consumer clause owns the ambiguous-base diagnostic: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 0));
    assert!(
        related.is_none(),
        "an ambiguous facade base has no single authored competing site"
    );
}

#[test]
fn ambiguous_base_route_overlapping_a_local_source_is_rejected_eagerly() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The retained `forms` facade publishes `x`; the discarded claimant publishes `cards`, which
    // is also a real local source. Route planning must see both claimants and reject the local
    // overlap even though no dependency names `@forms/cards`.
    let result = compile_fixture_result(
        &[
            ("src/@entry.moth", "[:entry]\n"),
            ("src/@mod.moth", ""),
            ("src/forms/cards.moth", "value #= 1\n"),
            ("src/a/forms/@package.moth", "$export @x\nvalue #= 2\n"),
            ("src/b/forms/@package.moth", "$export @cards\nvalue #= 3\n"),
        ],
        false,
        BuildProfile::Dev,
    );
    let (primary, related) = diagnostic_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("b/forms/@package.moth"),
        "the discarded claimant's route operand owns the eager collision: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the competing local source keeps its authored site");
    assert!(
        related.0.ends_with("forms/cards.moth"),
        "the local source is the second authored site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
}

#[test]
fn unreferenced_route_colliding_with_child_facade_reports_both_sites() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The support facade publishes `@forms`, which folds onto the normal child facade
    // `vendor/forms`. No consumer names the route, so only eager publication can report it.
    let result = compile_fixture_result(
        &[
            ("src/@entry.moth", "[:entry]\n"),
            ("src/@mod.moth", ""),
            (
                "src/elsewhere/vendor/@package.moth",
                "$export @forms\nvalue #= 1\n",
            ),
            ("src/vendor/forms/@mod.moth", "$export\nvalue #= 2\n"),
        ],
        false,
        BuildProfile::Dev,
    );
    let (primary, related) = diagnostic_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("elsewhere/vendor/@package.moth"),
        "the route operand owns the collision: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the child facade keeps its authored root site");
    assert!(
        related.0.ends_with("vendor/forms/@mod.moth"),
        "the child facade root is the second authored site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
}

#[test]
fn ambiguous_base_route_colliding_with_child_facade_reports_both_sites() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The ambiguous `forms` base cannot hide a route that a visible child facade already owns.
    // Eager validation must report the discarded claimant's route and the child root.
    let result = compile_fixture_result(
        &[
            ("src/@entry.moth", "[:entry]\n"),
            ("src/@mod.moth", ""),
            ("src/a/forms/@package.moth", "$export\nvalue #= 1\n"),
            ("src/b/forms/@package.moth", "$export @cards\nvalue #= 2\n"),
            ("src/forms/cards/@mod.moth", "$export\nvalue #= 3\n"),
        ],
        false,
        BuildProfile::Dev,
    );
    let (primary, related) = diagnostic_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("b/forms/@package.moth"),
        "the discarded claimant's route operand owns the collision: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the visible child facade keeps its authored root site");
    assert!(
        related.0.ends_with("forms/cards/@mod.moth"),
        "the child facade root is the second authored site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
}

#[test]
fn ambiguous_base_route_colliding_with_registered_binding_prefix_is_rejected() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temporary = tempfile::tempdir().expect("should create temporary project");
    let project_root = temporary.path().to_path_buf();
    fs::create_dir_all(project_root.join("src/a/forms"))
        .expect("should create first support facade folder");
    fs::create_dir_all(project_root.join("src/b/forms"))
        .expect("should create second support facade folder");
    fs::write(
        project_root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write project config");
    fs::write(project_root.join("src/@entry.moth"), "[:entry]\n")
        .expect("should write project entry");
    fs::write(project_root.join("src/@mod.moth"), "").expect("should write project module root");
    fs::write(
        project_root.join("src/a/forms/@package.moth"),
        "$export\nvalue #= 1\n",
    )
    .expect("should write steady support facade");
    fs::write(
        project_root.join("src/b/forms/@package.moth"),
        "$export @cards\nvalue #= 2\n",
    )
    .expect("should write routed support facade");

    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .binding_packages
        .register_package("@forms/cards", PackageOrigin::ProjectLocal)
        .expect("the binding package should register");

    let mut config = Config::new(project_root.join("src"));
    config.project_name = "docs".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    );
    let (primary, related) = diagnostic_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("b/forms/@package.moth"),
        "the discarded claimant's route operand owns the prefix collision: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    assert!(
        related.is_none(),
        "a registered binding package is not an authored content site"
    );
}

#[test]
fn deeper_route_under_an_ambiguous_base_does_not_reject_a_distinct_local_source() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The ambiguous facades publish the deeper route `forms/x/y`, but the project imports only
    // its own `forms/x` source. The route must stay ambiguous without colliding with the
    // ancestor local key; ancestor overlap is not the local dependency lookup rule.
    let frontend = compile_fixture(&[
        ("src/@entry.moth", "@forms/x value\n\n[: [value] ]\n"),
        ("src/@mod.moth", ""),
        ("src/forms/x.moth", "value #= \"local\"\n"),
        (
            "src/a/forms/@package.moth",
            "$export @x/y\nvalue #= \"route\"\n",
        ),
        ("src/b/forms/@package.moth", "$export\nvalue #= \"other\"\n"),
    ]);
    let folded = entry_folded_output(&frontend);
    assert!(
        folded.contains("local"),
        "the distinct local provider must remain importable and fold its own value: {folded:?}"
    );
}

#[test]
fn paired_support_alias_collision_names_the_support_facade() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temporary = tempfile::tempdir().expect("should create temporary project");
    let project_root = fs::canonicalize(temporary.path()).expect("project root must canonicalize");
    // The donor root sits beside `src`, so both sites' logical spellings are
    // `alt/vendor/@package.moth`. A cross-boundary collision must still resolve both through
    // their physical paths or a same-file renderer reduction hides one authored operand.
    let package_root = project_root.join("alt/vendor");
    fs::create_dir_all(project_root.join("src/client")).expect("should create consumer module");
    fs::create_dir_all(project_root.join("src/alt/vendor")).expect("should create paired module");
    fs::create_dir_all(&package_root).expect("should create package root");
    fs::write(
        project_root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write project config");
    fs::write(project_root.join("src/@entry.moth"), "[:entry]\n")
        .expect("should write project entry");
    fs::write(project_root.join("src/@mod.moth"), "").expect("should write project module root");
    fs::write(
        project_root.join("src/client/@mod.moth"),
        "$export\nclient_label #= \"client\"\n",
    )
    .expect("should write sibling consumer");
    // The paired module publishes both surfaces under one structural identity. The sibling sees
    // only the support alias `vendor`, so the collision site must be the support facade rather
    // than the normal root selected first by identity order.
    fs::write(
        project_root.join("src/alt/vendor/@mod.moth"),
        "$export\nnormal_label #= \"normal\"\n",
    )
    .expect("should write paired normal facade");
    fs::write(
        project_root.join("src/alt/vendor/@package.moth"),
        "$export\nsupport_label #= \"support\"\n",
    )
    .expect("should write paired support facade");
    fs::write(
        package_root.join("@package.moth"),
        "$export @published\nvalue #= 1\n",
    )
    .expect("should write routed package facade");

    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root("vendor/lib", package_root, PackageOrigin::ProjectLocal)
        .expect("the package root should register");

    let mut config = Config::new(project_root.join("src"));
    config.project_name = "docs".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("alt/vendor/@package.moth"),
        "the support facade that published `vendor` owns the collision site: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 0));
    let related = related.expect(
        "the package `$export` operand is a related site through the package report domain",
    );
    assert!(
        related.0.ends_with("alt/vendor/@package.moth"),
        "the published route operand is the related site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 8));
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    let (report, view) = reports
        .iter()
        .find(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006")
        .expect("the eager route collision must be diagnosed");
    let terminal = format_terminal_locations_for_test(
        view,
        DiagnosticRenderContext::for_report(report),
        &project_root,
    );
    assert_eq!(
        terminal,
        [
            "src/alt/vendor/@package.moth:1:1".to_owned(),
            "info: alt/vendor/@package.moth:1:9 - previous declaration here".to_owned()
        ],
        "both collision sites must retain their physical paths instead of collapsing as one file"
    );
}

#[test]
fn registered_source_package_route_unexported_leaf_reports_selection_site() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let result = compile_registered_package_project(
        "builder/test/routepkg",
        &[
            ("@package.moth", "$export @forms\n@widgets/forms decorate\n"),
            (
                "widgets/@mod.moth",
                "$export @forms\ndecorate #|label String| -> String:\n    return label\n;\n",
            ),
        ],
        "@builder/test/routepkg/forms missing\n\n[:entry]\n",
    );
    let (primary, related) = diagnostic_sites(&result, "MOTH-IMPORT-0011");
    assert!(
        primary.0.ends_with("@entry.moth"),
        "the unexported leaf must report the consumer clause: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 29));
    assert!(
        related.is_none(),
        "an unexported leaf has one authored site"
    );
}

#[test]
fn registered_source_package_route_rejects_unpublished_paths() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    for (dependency, scenario) in [
        (
            "@builder/test/routepkg/forms/secret secret\n\n[:entry]\n",
            "a private source beside the route name",
        ),
        (
            "@builder/test/routepkg/widgets/implementation secret\n\n[:entry]\n",
            "a deeper private child implementation",
        ),
        (
            "@builder/test/routepkg/not_a_route secret\n\n[:entry]\n",
            "a missing exact route",
        ),
    ] {
        let result = compile_registered_package_project(
            "builder/test/routepkg",
            &[
                ("@package.moth", "$export @forms\n@widgets/forms decorate\n"),
                (
                    "widgets/@mod.moth",
                    "@implementation secret\n$export @forms\ndecorate #|label String| -> String:\n    return [:child:[secret]]\n;\n",
                ),
                ("widgets/implementation.moth", "secret #= \"private\"\n"),
                ("forms/secret.moth", "secret #= \"private\"\n"),
            ],
            dependency,
        );
        let (primary, related) = diagnostic_sites(&result, "MOTH-IMPORT-0015");
        assert!(
            primary.0.ends_with("@entry.moth"),
            "{scenario}: the consumer clause owns the diagnostic: {primary:?}"
        );
        assert_eq!((primary.1, primary.2), (0, 0), "{scenario}");
        assert!(
            related.is_none(),
            "{scenario}: an unpublished package path has one authored site"
        );
    }
}

#[test]
fn registered_package_route_source_diagnostic_preempts_route_miss() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let result = compile_registered_package_project(
        "builder/test/broken",
        &[("@package.moth", "$export @forms\nvalue #= \"unterminated\n")],
        "@builder/test/broken/forms value\n~[:[value]]\n",
    );
    let (primary, related) = diagnostic_sites(&result, "MOTH-SYNTAX-0006");
    assert!(
        primary.0.ends_with("@package.moth"),
        "the unavailable provider owns its syntax diagnostic: {primary:?}"
    );
    assert!(
        related.is_none(),
        "a source preparation failure retains its single authored site"
    );
}

#[test]
fn earlier_package_consumer_of_later_unavailable_route_reports_provider_diagnostic() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let result = compile_registered_packages_project(
        &[
            (
                "builder/web/site",
                PackageOrigin::Builder,
                &[(
                    "@package.moth",
                    "@builder/zzz/provider/forms value\n$export\nconsumer #= value\n",
                )],
            ),
            (
                "builder/zzz/provider",
                PackageOrigin::Builder,
                &[("@package.moth", "$export @forms\nvalue #= \"unterminated\n")],
            ),
        ],
        "[:entry]\n",
    );
    let (primary, _) = diagnostic_sites(&result, "MOTH-SYNTAX-0006");
    assert!(
        primary.0.ends_with("@package.moth"),
        "the later provider's walk-owned diagnostic must replace a consumer route miss: {primary:?}"
    );
}

#[test]
fn earlier_package_discovery_error_precedes_later_route_prepass_failure() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let result = compile_registered_packages_project(
        &[
            (
                "builder/web/site",
                PackageOrigin::Builder,
                &[(
                    "@package.moth",
                    "@absent_provider missing\n$export\nsite #= \"site\"\n",
                )],
            ),
            (
                "builder/zeta",
                PackageOrigin::Builder,
                &[(
                    "@package.moth",
                    "$export @forms\none #= 1\n$export @FORMS\ntwo #= 2\n",
                )],
            ),
        ],
        "[:entry]\n",
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert_eq!(
        reports
            .iter()
            .next()
            .map(|(_, diagnostic)| diagnostic.descriptor().code),
        Some("MOTH-IMPORT-0005"),
        "the earlier boundary's walk error must be the first reported failure"
    );
    let (primary, _) = diagnostic_sites(&result, "MOTH-IMPORT-0005");
    assert!(
        primary.0.ends_with("@package.moth"),
        "the earlier boundary's walk error must own the diagnostic: {primary:?}"
    );
}

#[test]
fn registered_source_package_route_collision_with_same_package_provider_reports_both_sites() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The @tools route `@forms` folds onto the package-local source at `tools/forms.moth`.
    // Both spans live in the package source database, so the diagnostic retains both sites.
    let result = compile_registered_packages_project(
        &[(
            "tools",
            PackageOrigin::ProjectLocal,
            &[
                ("@package.moth", "$export @forms\nvalue #= 1\n"),
                ("tools/forms.moth", "local_value #= 2\n"),
            ],
        )],
        "[:entry]\n",
    );
    let (primary, related) = diagnostic_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("@package.moth"),
        "the published route operand owns the primary site: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the package-local provider keeps its authored site");
    assert!(
        related.0.ends_with("tools/forms.moth"),
        "the same-package provider is the related site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
}

#[test]
fn registered_source_package_route_collision_reports_both_operand_sites() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let result = compile_registered_package_project(
        "builder/test/collision",
        &[
            ("@package.moth", "$export\nlabel #= \"root\"\n"),
            ("@mod.moth", "$export\nnormal_label #= \"normal\"\n"),
            ("vendor/forms/@mod.moth", "$export @x\nvalue #= 1\n"),
            (
                "elsewhere/vendor/@package.moth",
                "$export @forms/x\nother #= 2\n",
            ),
        ],
        "[:entry]\n",
    );
    let (primary, related) = diagnostic_sites(&result, "MOTH-IMPORT-0006");
    let related = related.expect("two public namespace claims must retain both authored sites");
    assert_eq!(
        (primary.1, primary.2),
        (0, 8),
        "the route operand owns the site"
    );
    assert_eq!((related.1, related.2), (0, 8));
    let mut sites = [primary.0, related.0];
    sites.sort();
    assert_eq!(
        sites,
        [
            "elsewhere/vendor/@package.moth".to_owned(),
            "vendor/forms/@mod.moth".to_owned(),
        ],
        "both visible facades' route operands must be named"
    );
}

#[test]
fn registered_source_package_case_only_route_conflict_reports_both_sites() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let result = compile_registered_package_project(
        "builder/test/casepkg",
        &[(
            "@package.moth",
            "$export @forms\none #= 1\n$export @FORMS\ntwo #= 2\n",
        )],
        "[:entry]\n",
    );
    let (primary, related) = diagnostic_sites(&result, "MOTH-RULE-0092");
    let related = related.expect("case-only route registration retains both operand sites");
    assert!(primary.0.ends_with("@package.moth"), "{primary:?}");
    assert!(related.0.ends_with("@package.moth"), "{related:?}");
    assert_eq!((primary.2, related.2), (8, 8));
    let mut lines = [primary.1, related.1];
    lines.sort_unstable();
    assert_eq!(lines, [0, 2], "both authored route operands must be named");
}

#[test]
fn package_case_conflict_precedes_route_collision_at_the_same_boundary_turn() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The `aa` boundary owns both an indexed `tools/forms` provider and a same-facade
    // `@Forms`/`@FORMS` conflict. Its own prepass failure owns the first walk turn; reporting the
    // cross-package route collision there would replace MOTH-RULE-0092 with MOTH-IMPORT-0006.
    let result = compile_registered_packages_project(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[
                    (
                        "@package.moth",
                        "$export @Forms\none #= 1\n$export @FORMS\ntwo #= 2\n",
                    ),
                    ("tools/forms/@mod.moth", "$export\nform_label #= 1\n"),
                ],
            ),
            (
                "tools",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export @forms\ntool_value #= 1\n")],
            ),
        ],
        "[:entry]\n",
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert_eq!(
        reports
            .iter()
            .next()
            .map(|(_, diagnostic)| diagnostic.descriptor().code),
        Some("MOTH-RULE-0092"),
        "the current boundary's prepass failure must preempt a route collision"
    );
    assert!(
        !reports
            .iter()
            .any(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006"),
        "the downstream route collision must not mask the boundary's own route conflict"
    );
    let (primary, related) = diagnostic_sites(&result, "MOTH-RULE-0092");
    assert!(
        primary.0.ends_with("@package.moth"),
        "the case conflict keeps its boundary-owned operand: {primary:?}"
    );
    assert!(
        related
            .as_ref()
            .is_some_and(|related| related.0.ends_with("@package.moth")),
        "the case conflict retains both authored operands"
    );
    let mut lines = [primary.1, related.expect("both operands are present").1];
    lines.sort_unstable();
    assert_eq!(lines, [0, 2], "both authored route operands must be named");
}

#[cfg(unix)]
#[test]
fn consumed_prepass_infrastructure_survives_route_collision_cleanup() {
    use std::os::unix::fs::PermissionsExt;

    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temporary = tempfile::tempdir().expect("should create temporary project");
    let project_root = temporary.path().to_path_buf();
    fs::create_dir_all(project_root.join("src")).expect("should create source root");
    fs::write(
        project_root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write config");
    fs::write(project_root.join("src/@entry.moth"), "[:entry]\n")
        .expect("should write project entry");

    let tools_root = project_root.join("packages/tools");
    let zz_root = project_root.join("packages/zz");
    fs::create_dir_all(&tools_root).expect("should create tools package folder");
    fs::create_dir_all(zz_root.join("tools/forms")).expect("should create zz provider folder");
    fs::write(
        tools_root.join("@package.moth"),
        "$export @forms\ntool_value #= 1\n",
    )
    .expect("should write tools route facade");
    fs::write(zz_root.join("@package.moth"), "$export\nlabel #= 1\n")
        .expect("should write later boundary facade");
    let unreadable_provider = zz_root.join("tools/forms/@mod.moth");
    fs::write(&unreadable_provider, "$export\nform_label #= 1\n")
        .expect("should write later unreadable provider");
    fs::set_permissions(&unreadable_provider, fs::Permissions::from_mode(0o000))
        .expect("should make the later provider unreadable");
    if fs::read_to_string(&unreadable_provider).is_ok() {
        fs::set_permissions(&unreadable_provider, fs::Permissions::from_mode(0o600))
            .expect("should restore provider permissions");
        eprintln!("skipping unreadable-provider assertion: process can still read the file");
        return;
    }

    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root("tools", tools_root, PackageOrigin::ProjectLocal)
        .expect("tools package should register");
    frontend_surface
        .source_packages
        .register_filesystem_root("zz", zz_root, PackageOrigin::ProjectLocal)
        .expect("zz package should register");
    let mut config = Config::new(project_root.join("src"));
    config.project_name = "docs".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    );
    fs::set_permissions(&unreadable_provider, fs::Permissions::from_mode(0o600))
        .expect("should restore provider permissions");

    let Err(errors) = result else {
        panic!("the consumed boundary's stored infrastructure cause must dominate the collision");
    };
    assert_eq!(
        errors.iter().count(),
        1,
        "the stored prepass cause and donor finalization must report one failed read once: {errors:?}"
    );
    assert_eq!(errors.primary.kind, InfrastructureFailureKind::Filesystem);
    assert!(
        errors
            .primary
            .host_path
            .as_deref()
            .is_some_and(|path| path.ends_with("packages/zz/tools/forms/@mod.moth")),
        "the preserved cause names the unreadable provider: {errors:?}"
    );
}

#[test]
fn package_case_conflict_precedes_consumer_route_ambiguity() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The package root imports `@forms/x`; two support facades share the basename `forms`.
    // One facade has a same-facade `@x`/`@X` conflict, so its MOTH-RULE-0092 must be the package
    // boundary's prepass failure. Deferring it would let the healthy facade make `forms/x`
    // ambiguous and the root walk would report MOTH-IMPORT-0006 first.
    let result = compile_registered_package_project(
        "builder/test/routecase",
        &[
            ("@package.moth", "$export\nroot_label #= \"root\"\n"),
            (
                "@mod.moth",
                "@forms/x value\n$export\npackage_value #= value\n",
            ),
            (
                "a/forms/@package.moth",
                "$export @x\nvalue #= 1\n$export @X\nother #= 2\n",
            ),
            ("b/forms/@package.moth", "$export @x\nvalue #= 3\n"),
        ],
        "@builder/test/routecase root_label\n~[:[root_label]]\n",
    );
    let (primary, related) = diagnostic_sites(&result, "MOTH-RULE-0092");
    assert!(
        primary.0.ends_with("a/forms/@package.moth"),
        "the conflicting facade owns the diagnostic: {primary:?}"
    );
    let related = related.expect("case-only route conflict retains both operand sites");
    assert!(related.0.ends_with("a/forms/@package.moth"), "{related:?}");
    assert_eq!((primary.2, related.2), (8, 8));
    let mut lines = [primary.1, related.1];
    lines.sort_unstable();
    assert_eq!(lines, [0, 2], "both authored route operands must be named");
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert!(
        !reports
            .iter_severity(DiagnosticSeverity::Error)
            .any(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006"),
        "consumer route ambiguity must not mask the package route conflict"
    );
}

#[test]
fn package_internal_route_colliding_with_binding_package_is_rejected() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temporary = tempfile::tempdir().expect("should create temporary project");
    let project_root = temporary.path().to_path_buf();
    let package_root = project_root.join("packages/builder/test/bindcollision");
    fs::create_dir_all(project_root.join("src")).expect("should create source root");
    fs::create_dir_all(&package_root).expect("should create package root");
    fs::write(
        project_root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write project config");
    fs::write(project_root.join("src/@entry.moth"), "[:entry]\n")
        .expect("should write project entry");
    fs::write(
        package_root.join("@package.moth"),
        "$export\nroot_label #= \"root\"\n",
    )
    .expect("should write package root facade");
    fs::create_dir_all(package_root.join("vendor")).expect("should create vendor module folder");
    fs::write(
        package_root.join("vendor/@mod.moth"),
        "$export @forms\nvalue #= 1\n",
    )
    .expect("should write internal route facade");

    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root(
            "builder/test/bindcollision",
            package_root,
            PackageOrigin::Builder,
        )
        .expect("the package root should register");
    frontend_surface
        .binding_packages
        .register_package("@vendor/forms", PackageOrigin::ProjectLocal)
        .expect("the binding package should register");

    let mut config = Config::new(project_root.join("src"));
    config.project_name = "docs".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    );
    let (primary, related) = diagnostic_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("vendor/@mod.moth"),
        "the internal route operand owns the binding-package collision: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    assert!(
        related.is_none(),
        "a registered binding package is not an authored content site"
    );
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "the binding-collision report must not be duplicated by the deferred package path",
    );
}

#[test]
fn self_provider_collision_keeps_a_capacity_rejected_publisher_snapshot() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // Stay above the mandatory `@core/html` root (2161 bytes), which walks before `mm/forms`.
    const CORE_ROOT_SAFE_LIMIT: usize = 4096;
    let _limit = ForcedSourceSnapshotLimitGuard::new(CORE_ROOT_SAFE_LIMIT);
    // `mm/forms`'s child `mm/@mod.moth` publishes `@forms`, whose package-relative key
    // `mm/forms` overlaps the package's own registered prefix. The child parses during the
    // prepass, but its snapshot exceeds the forced limit and cannot be retained. HEAD reported
    // the collision without a primary excerpt; it must stay a single-site collision instead of
    // failing as an internal error.
    let child = format!(
        "$export @forms\nvalue #= 1\n// {}\n",
        "#".repeat(CORE_ROOT_SAFE_LIMIT + 512)
    );
    let files: &[(&str, &str)] = &[
        ("@package.moth", "$export\nform_label #= 1\n"),
        ("mm/@mod.moth", child.as_str()),
    ];
    let result = compile_registered_packages_project(
        &[("mm/forms", PackageOrigin::ProjectLocal, files)],
        "[:entry]\n",
    );
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "the self-provider collision must report exactly once",
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    let (report, diagnostic) = reports
        .iter()
        .find(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006")
        .expect("the collision diagnostic is present");
    assert!(
        DiagnosticRenderContext::for_report(report)
            .site_position(diagnostic.primary_site())
            .is_none(),
        "HEAD omitted the capacity-rejected primary excerpt"
    );
    assert_eq!(
        diagnostic.durable_labels().count(),
        0,
        "HEAD's self-provider collision is single-site"
    );
}

#[test]
fn package_internal_route_overlapping_a_registered_package_reports_both_sites() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The `bundle` boundary's internal `tools` facade publishes `@forms`, which folds onto the
    // separately registered `tools/forms` package prefix. The publisher sorts before the provider,
    // so the collision is known before that package's root facade is inventoried.
    let bundle: RegisteredPackageFixture<'_> = (
        "apps/bundle",
        PackageOrigin::ProjectLocal,
        &[
            ("@package.moth", "$export\nroot_label #= 1\n"),
            ("tools/@mod.moth", "$export @forms\nvalue #= 1\n"),
        ],
    );
    let forms: RegisteredPackageFixture<'_> = (
        "tools/forms",
        PackageOrigin::ProjectLocal,
        &[("@package.moth", "$export\nform_label #= 1\n")],
    );
    let forward = [bundle, forms];
    let reversed = [forms, bundle];
    for packages in [&forward[..], &reversed[..]] {
        let (_temporary, result) =
            compile_registered_packages_project_retained(packages, "[:entry]\n");
        let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
        assert!(
            primary.0.ends_with("bundle/tools/@mod.moth"),
            "the internal route operand owns the primary site: {primary:?}"
        );
        assert_eq!((primary.1, primary.2), (0, 8));
        let related = related.expect("the registered package supplies the previous site");
        assert!(
            related.0.ends_with("packages/tools/forms/@package.moth"),
            "the later package's indexed root facade is the related site: {related:?}"
        );
        assert_eq!((related.1, related.2), (0, 0));
        assert_single_diagnostic(
            &result,
            "MOTH-IMPORT-0006",
            "the deferred collision must report exactly once",
        );
    }
}

#[test]
fn earlier_consumer_of_a_failed_package_route_collision_reports_the_collision_first() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // `aa` walks first and imports below the `bundle` prefix, whose route attachment stopped at
    // the `tools/forms` collision. The failed publisher must stay unavailable so the consumer
    // resolves to the boundary diagnostic instead of claiming MOTH-IMPORT-0015 first.
    let result = compile_registered_packages_project(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[(
                    "@package.moth",
                    "@apps/bundle/tools/forms value\n$export\nlabel #= value\n",
                )],
            ),
            (
                "apps/bundle",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("tools/@mod.moth", "$export @forms\nvalue #= 1\n"),
                ],
            ),
            (
                "tools/forms",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export\nform_label #= 1\n")],
            ),
        ],
        "[:entry]\n",
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert_eq!(
        reports
            .iter()
            .next()
            .map(|(_, diagnostic)| diagnostic.descriptor().code),
        Some("MOTH-IMPORT-0006"),
        "the collision owns the first failure instead of the consumer's import error"
    );
    assert_eq!(
        reports
            .iter()
            .filter(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006")
            .count(),
        1,
        "the deferred collision must report exactly once"
    );
    assert!(
        !reports
            .iter()
            .any(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0015"),
        "the failed publisher's namespace must not degrade to a consumer import error"
    );
}

#[test]
fn package_internal_route_collisions_report_at_the_first_publisher_turn() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // `mm` attaches before `zz`, so HEAD reported `mm`'s collision at `mm`'s turn. The later
    // `aa/forms` collision must not replace it.
    let result = compile_registered_packages_project(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export\nlabel #= 1\n")],
            ),
            (
                "bb/forms",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export\nform_label #= 1\n")],
            ),
            (
                "mm",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("bb/@mod.moth", "$export @forms\nvalue #= 1\n"),
                ],
            ),
            (
                "zz",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("aa/@mod.moth", "$export @forms\nvalue #= 1\n"),
                ],
            ),
        ],
        "[:entry]\n",
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("mm/bb/@mod.moth"),
        "HEAD reported the first attached publisher's collision: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the competing package keeps its authored site");
    assert!(
        related.0.ends_with("packages/bb/forms/@package.moth"),
        "the first publisher's provider supplies the related site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "only the first attached collision may report",
    );
}

#[test]
fn package_internal_route_collision_selects_the_exact_published_operand() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The provider publishes both a descendant `@forms/details` and the exact `@details` route.
    // Only the exact key is the competing authored claim; selecting the descendant operand or the
    // root facade start would mislabel the collision.
    let provider_files: &[(&str, &str)] = &[(
        "@package.moth",
        "$export @forms/details\none #= 1\n$export @details\ntwo #= 2\n",
    )];
    let publisher_first: [RegisteredPackageFixture<'_>; 2] = [
        (
            "apps/bundle",
            PackageOrigin::ProjectLocal,
            &[
                ("@package.moth", "$export\nroot_label #= 1\n"),
                ("tools/@mod.moth", "$export @forms/details\nvalue #= 1\n"),
            ],
        ),
        ("tools/forms", PackageOrigin::ProjectLocal, provider_files),
    ];
    let (_temporary, result) =
        compile_registered_packages_project_retained(&publisher_first, "[:entry]\n");
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the exact published operand keeps its authored site");
    assert!(
        related.0.ends_with("packages/tools/forms/@package.moth"),
        "the provider's root facade owns the operand: {related:?}"
    );
    assert_eq!(
        (related.1, related.2),
        (2, 8),
        "the exact `@details` operand is the related site, not the root start"
    );
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "the exact-operand collision must report exactly once",
    );

    // When the provider sorts first, HEAD reported its published `@details` colliding with the
    // later package's local `aa` module at the provider's turn, before the prefix collision.
    let provider_first: [RegisteredPackageFixture<'_>; 2] = [
        ("aa/forms", PackageOrigin::ProjectLocal, provider_files),
        (
            "zz/bundle",
            PackageOrigin::ProjectLocal,
            &[
                ("@package.moth", "$export\nroot_label #= 1\n"),
                ("aa/@mod.moth", "$export @forms/details\nvalue #= 1\n"),
            ],
        ),
    ];
    let (_temporary, result) =
        compile_registered_packages_project_retained(&provider_first, "[:entry]\n");
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("packages/aa/forms/@package.moth"),
        "HEAD reported the provider's published operand first: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (2, 8));
    let related = related.expect("the later package's local module keeps its authored site");
    assert!(
        related.0.ends_with("zz/bundle/aa/@mod.moth"),
        "the local module facade is the related site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "HEAD's first collision must report exactly once",
    );
}

#[test]
fn package_internal_route_collision_retains_an_indexed_donor_after_a_prepass_failure() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The `zz/forms` provider never publishes its root routes because of its own case conflict.
    // The earlier `aa` collision must still report both authored sites through that boundary's
    // indexed root facade, and the failing provider's own turn never arrives first.
    let result = compile_registered_packages_project(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("zz/@mod.moth", "$export @forms\nvalue #= 1\n"),
                ],
            ),
            (
                "zz/forms",
                PackageOrigin::ProjectLocal,
                &[(
                    "@package.moth",
                    "$export @Forms\none #= 1\n$export @FORMS\ntwo #= 2\n",
                )],
            ),
        ],
        "[:entry]\n",
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("aa/zz/@mod.moth"),
        "the earlier publisher owns the primary site: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the failed provider still supplies its indexed root site");
    assert!(
        related.0.ends_with("packages/zz/forms/@package.moth"),
        "the provider root facade is the related site: {related:?}"
    );
    assert_eq!(
        (related.1, related.2),
        (0, 0),
        "a failed provider has no published operand, so its root facade start is the site"
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert!(
        !reports
            .iter()
            .any(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-RULE-0092"),
        "the provider's later prepass failure must not mask the earlier collision"
    );
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "the collision must report exactly once",
    );
}

#[test]
fn a_failed_provider_keeps_its_own_prepass_failure_at_its_turn() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The failing provider sorts first, so its own case conflict owns the first walk turn. The
    // deferred collision must not replace it with a report from a boundary that has not walked.
    let result = compile_registered_packages_project(
        &[
            (
                "aa/forms",
                PackageOrigin::ProjectLocal,
                &[(
                    "@package.moth",
                    "$export @Forms\none #= 1\n$export @FORMS\ntwo #= 2\n",
                )],
            ),
            (
                "zz",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("aa/@mod.moth", "$export @forms\nvalue #= 1\n"),
                ],
            ),
        ],
        "[:entry]\n",
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert_eq!(
        reports
            .iter()
            .next()
            .map(|(_, diagnostic)| diagnostic.descriptor().code),
        Some("MOTH-RULE-0092"),
        "the failing provider's own prepass diagnostic owns its turn"
    );
    assert!(
        !reports
            .iter()
            .any(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006"),
        "the deferred collision must not replace the failing provider's diagnostic"
    );
}

#[test]
fn cached_root_failure_owns_its_turn_before_a_deferred_collision() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The failing provider sorts first and its root facade failure is cached for its walk rather
    // than stored as `prepass_failure`. The deferred collision must let that authored syntax error
    // report at the boundary's own turn, as HEAD did before deferred publication existed.
    let result = compile_registered_packages_project(
        &[
            (
                "aa/forms",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export\nlabel #= \"unterminated\n")],
            ),
            (
                "zz",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("aa/@mod.moth", "$export @forms\nvalue #= 1\n"),
                ],
            ),
        ],
        "[:entry]\n",
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert_eq!(
        reports
            .iter()
            .next()
            .map(|(_, diagnostic)| diagnostic.descriptor().code),
        Some("MOTH-SYNTAX-0006"),
        "the boundary's cached root failure must own its walk turn"
    );
    assert!(
        !reports
            .iter()
            .any(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006"),
        "the deferred collision must not replace the failing provider's own diagnostic"
    );
}

#[test]
fn deferred_collision_keeps_a_syntax_failed_provider_as_its_donor_site() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The publisher sorts first, so the collision reports before the provider's walk. The
    // provider's root facade syntax failure still supplies the related authored site through its
    // own frozen report domain.
    let result = compile_registered_packages_project(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("zz/@mod.moth", "$export @forms\nvalue #= 1\n"),
                ],
            ),
            (
                "zz/forms",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export\nlabel #= \"unterminated\n")],
            ),
        ],
        "[:entry]\n",
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("aa/zz/@mod.moth"),
        "the earlier publisher owns the primary site: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the syntax-failed provider still supplies its root site");
    assert!(
        related.0.ends_with("packages/zz/forms/@package.moth"),
        "the failed provider's root facade is the related site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "the collision must report exactly once",
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert!(
        !reports
            .iter()
            .any(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-SYNTAX-0006"),
        "the provider's later walk failure must not add a second report"
    );
}

#[test]
fn deferred_collision_reports_the_first_plan_in_one_attachment() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // `@Forms` sorts before `@details`, so HEAD reported that first plan at the publisher's turn.
    // The related site is `tools/forms`, not the earlier-walking `tools/details` prefix.
    let result = compile_registered_packages_project(
        &[
            (
                "tools/details",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export\nlabel #= 1\n")],
            ),
            (
                "tools/forms",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export\nlabel #= 2\n")],
            ),
            (
                "zz",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    (
                        "tools/@mod.moth",
                        "$export @Forms\none #= 1\n$export @details\ntwo #= 2\n",
                    ),
                ],
            ),
        ],
        "[:entry]\n",
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("zz/tools/@mod.moth"),
        "the internal route operand owns the primary site: {primary:?}"
    );
    assert_eq!(
        (primary.1, primary.2),
        (0, 8),
        "HEAD reported the first plan `@Forms`"
    );
    let related = related.expect("the first plan's provider supplies the related site");
    assert!(
        related.0.ends_with("packages/tools/forms/@package.moth"),
        "the first plan's overlapping prefix is the competing site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "the first-plan collision must report exactly once",
    );
}

#[test]
fn cached_child_failure_owns_its_turn_before_a_deferred_collision() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The earlier provider's root facade is healthy; a child facade's syntax error is cached for
    // its walk. HEAD reported that child diagnostic before the later publisher's collision.
    let result = compile_registered_packages_project(
        &[
            (
                "aa/forms",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("widgets/@mod.moth", "$export\nvalue #= \"unterminated\n"),
                ],
            ),
            (
                "zz",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("aa/@mod.moth", "$export @forms\nvalue #= 1\n"),
                ],
            ),
        ],
        "[:entry]\n",
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert_eq!(
        reports
            .iter()
            .next()
            .map(|(_, diagnostic)| diagnostic.descriptor().code),
        Some("MOTH-SYNTAX-0006"),
        "the earlier provider's cached child failure must own its walk turn"
    );
    assert!(
        !reports
            .iter()
            .any(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006"),
        "the deferred collision must not mask the earlier child diagnostic"
    );
}

#[test]
fn deferred_collision_keeps_exact_operand_when_provider_attachment_fails() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // `mm/forms` publishes `@details` but its own attachment stops on a child collision with
    // `zz/forms`, so those root claims never enter `package_public_routes`. The earlier `aa`
    // collision must still label the exact `@details` operand from the retained prepass claims.
    let result = compile_registered_packages_project(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("mm/@mod.moth", "$export @forms/details\nvalue #= 1\n"),
                ],
            ),
            (
                "mm/forms",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export @details\ntwo #= 2\n"),
                    ("zz/@mod.moth", "$export @forms\nvalue #= 1\n"),
                ],
            ),
            (
                "zz/forms",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export\nlabel #= 1\n")],
            ),
        ],
        "[:entry]\n",
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("aa/mm/@mod.moth"),
        "the earlier publisher owns the primary site: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the failed provider still supplies its authored operand");
    assert!(
        related.0.ends_with("packages/mm/forms/@package.moth"),
        "the provider root facade owns the operand: {related:?}"
    );
    assert_eq!(
        (related.1, related.2),
        (0, 8),
        "the exact `@details` operand is the related site, not the root start"
    );
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "the first publisher's collision must report exactly once",
    );
}

#[test]
fn package_internal_first_namespace_collision_reports_at_publisher_turn() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // `zz`'s root namespace finds `b/forms` before the nested namespace finds the earlier
    // `a/forms` prefix. HEAD reported the first namespace's collision at `zz`'s turn.
    let result = compile_registered_packages_project(
        &[
            (
                "a/forms",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export\nlabel #= 1\n")],
            ),
            (
                "b/forms",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export\nlabel #= 2\n")],
            ),
            (
                "zz",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("b/@mod.moth", "$export @forms\nvalue #= 1\n"),
                    ("nested/@mod.moth", "$export\nnested_label #= 1\n"),
                    ("nested/a/@mod.moth", "$export @forms\nvalue #= 2\n"),
                ],
            ),
        ],
        "[:entry]\n",
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("zz/b/@mod.moth"),
        "HEAD reported the first namespace's collision: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the first namespace's provider supplies the related site");
    assert!(
        related.0.ends_with("packages/b/forms/@package.moth"),
        "the first namespace's overlapping prefix is the competing site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "the first namespace collision must report exactly once",
    );
}

/// Valid UTF-8 root facade longer than the forced snapshot limit in the collision tests.
const OVERSIZED_PROVIDER_ROOT: &str = concat!(
    "$export\nform_label #= 1\n",
    "// ############################################################",
    "############################################################\n",
);

#[test]
fn a_losing_prefix_collision_never_reads_its_provider() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The limit must stay above the mandatory `@core/html` root (2161 bytes) because a claim
    // collision cleanup finalizes every preparation, including the core packages. The provider
    // root below is larger than the limit, so reading it would emit a syntax failure.
    const CORE_ROOT_SAFE_LIMIT: usize = 4096;
    let _limit = ForcedSourceSnapshotLimitGuard::new(CORE_ROOT_SAFE_LIMIT);
    let oversized_provider_root = format!(
        "$export\nform_label #= 1\n// {}\n",
        "#".repeat(CORE_ROOT_SAFE_LIMIT + 512)
    );
    // `aa`'s local `tools/forms` source collides with the `tools` package's published `@forms`,
    // and that claim collision owns `aa`'s earlier turn. `ab`'s separate prefix collision names
    // `zz/forms`, whose root facade is over the limit. Resolving the losing prefix collision at
    // prep time would cache that snapshot, and the winning cleanup would reject it as a syntax
    // failure even though HEAD never reads the file.
    let forms_files: &[(&str, &str)] = &[
        ("@package.moth", oversized_provider_root.as_str()),
        (
            "@mod.moth",
            "$export @Forms\none #= 1\n$export @FORMS\ntwo #= 2\n",
        ),
    ];
    let result = compile_registered_packages_project(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("tools/forms.moth", "local_value #= 1\n"),
                ],
            ),
            (
                "ab",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("zz/@mod.moth", "$export @forms\nvalue #= 1\n"),
                ],
            ),
            (
                "tools",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export @forms\nroot_label #= 1\n")],
            ),
            ("zz/forms", PackageOrigin::ProjectLocal, forms_files),
        ],
        "[:entry]\n",
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert_eq!(
        reports
            .iter()
            .next()
            .map(|(_, diagnostic)| diagnostic.descriptor().code),
        Some("MOTH-IMPORT-0006"),
        "the aa/tools claim collision must own the first report"
    );
    assert!(
        reports
            .iter()
            .all(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006"),
        "the losing prefix collision must not read its oversized provider"
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    let related = related.expect("the aa/tools claim collision names both authored sites");
    let sites = [primary.0.as_str(), related.0.as_str()];
    assert!(
        sites
            .iter()
            .any(|site| site.ends_with("aa/tools/forms.moth")),
        "the local claim is one site: {sites:?}"
    );
    assert!(
        sites
            .iter()
            .any(|site| site.ends_with("packages/tools/@package.moth")),
        "the published tools route is the other site: {sites:?}"
    );
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "the winning collision must report exactly once",
    );
}

#[test]
fn package_prefix_collision_preempts_a_later_unreadable_package() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // HEAD reported `aa`'s collision at its own turn, before the unrelated later `zz` package's
    // unreadable root facade could become a failure. Finalizing every remaining preparation would
    // promote that file error and replace the collision.
    let (_temporary, result) = compile_registered_packages_project_with_damaged_sources(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("mm/@mod.moth", "$export @forms\nvalue #= 1\n"),
                ],
            ),
            (
                "mm/forms",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export\nform_label #= 1\n")],
            ),
            (
                "zz",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export\nzz_label #= 1\n")],
            ),
        ],
        "[:entry]\n",
        &[("zz", "@package.moth", b"\xFF\xFE")],
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert_eq!(
        reports
            .iter()
            .next()
            .map(|(_, diagnostic)| diagnostic.descriptor().code),
        Some("MOTH-IMPORT-0006"),
        "the publisher's collision must own the report"
    );
    assert!(
        reports
            .iter()
            .all(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006"),
        "the later unreadable package must not contribute a failure"
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("aa/mm/@mod.moth"),
        "the publisher owns the primary site: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the readable provider supplies its root site");
    assert!(
        related.0.ends_with("packages/mm/forms/@package.moth"),
        "the provider root facade is the related site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "the collision must report exactly once",
    );
}

#[test]
fn package_prefix_collision_reads_a_provider_root_skipped_by_its_prepass() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The provider's normal facade stops case-only before the prepass reads its readable root
    // facade. The earlier publisher's collision still owns the first turn and must name that
    // indexed root after reading it once, rather than degrading to a single site.
    let result = compile_registered_packages_project(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("mm/@mod.moth", "$export @forms\nvalue #= 1\n"),
                ],
            ),
            (
                "mm/forms",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nform_label #= 1\n"),
                    (
                        "@mod.moth",
                        "$export @Forms\none #= 1\n$export @FORMS\ntwo #= 2\n",
                    ),
                ],
            ),
        ],
        "[:entry]\n",
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("aa/mm/@mod.moth"),
        "the publisher owns the primary site: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the readable skipped provider root supplies its site");
    assert!(
        related.0.ends_with("packages/mm/forms/@package.moth"),
        "the provider's root facade owns the related site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert!(
        !reports
            .iter()
            .any(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-RULE-0092"),
        "the provider's later case conflict must not mask the earlier collision"
    );
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "the collision must report exactly once",
    );
}

#[test]
fn package_prefix_collision_uses_single_site_for_an_oversized_skipped_provider_root() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let _limit = ForcedSourceSnapshotLimitGuard::new(64);
    // The provider's normal facade stops case-only before its readable root is read. Loading that
    // root for the label succeeds, but retention rejects the snapshot for exceeding the limit.
    // The publisher must keep HEAD's package-relative single-site report instead of a two-site
    // shape whose related label cannot render.
    let result = compile_registered_packages_project(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("mm/@mod.moth", "$export @forms\nvalue #= 1\n"),
                ],
            ),
            (
                "mm/forms",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", OVERSIZED_PROVIDER_ROOT),
                    (
                        "@mod.moth",
                        "$export @Forms\none #= 1\n$export @FORMS\ntwo #= 2\n",
                    ),
                ],
            ),
        ],
        "[:entry]\n",
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    let (primary, related) = diagnostic_sites(&result, "MOTH-IMPORT-0006");
    assert_eq!(
        primary,
        ("mm/@mod.moth".to_owned(), 0, 8),
        "the lone publisher keeps HEAD's package-relative display"
    );
    assert!(
        related.is_none(),
        "a capacity-rejected provider has no renderable related site: {related:?}"
    );
    assert!(
        reports
            .iter()
            .all(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006"),
        "the capacity-rejected donor must not report its own syntax failure"
    );
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "the collision must report exactly once",
    );
}

#[test]
fn package_prefix_collision_keeps_a_prepass_captured_exact_operand() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The provider's root publishes the exact `@details` operand before its child facade stops
    // case-only. Attachment never runs, but the captured operand is still the authored competing
    // claim and must be the related site, not the root facade start.
    let result = compile_registered_packages_project(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("mm/@mod.moth", "$export @forms/details\nvalue #= 1\n"),
                ],
            ),
            (
                "mm/forms",
                PackageOrigin::ProjectLocal,
                &[
                    (
                        "@package.moth",
                        "$export @forms/details\none #= 1\n$export @details\ntwo #= 2\n",
                    ),
                    (
                        "widgets/@mod.moth",
                        "$export @Forms\none #= 1\n$export @FORMS\ntwo #= 2\n",
                    ),
                ],
            ),
        ],
        "[:entry]\n",
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("aa/mm/@mod.moth"),
        "the publisher owns the primary site: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the captured exact operand keeps its authored site");
    assert!(
        related.0.ends_with("packages/mm/forms/@package.moth"),
        "the provider's root facade owns the operand: {related:?}"
    );
    assert_eq!(
        (related.1, related.2),
        (2, 8),
        "the captured `@details` operand is the related site, not the root start"
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert!(
        !reports
            .iter()
            .any(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-RULE-0092"),
        "the provider's later case conflict must not mask the earlier collision"
    );
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "the collision must report exactly once",
    );
}

#[test]
fn package_prefix_collision_degrades_to_single_site_for_an_unreadable_provider() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The competing facade itself is unreadable, so its indexed root start has no line index a
    // frozen donor could resolve. HEAD reported the publisher's collision at its turn with one
    // site; the report must keep that first error and drop the unrenderable label.
    let (_temporary, result) = compile_registered_packages_project_with_damaged_sources(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nroot_label #= 1\n"),
                    ("mm/@mod.moth", "$export @forms\nvalue #= 1\n"),
                ],
            ),
            (
                "mm/forms",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export\nform_label #= 1\n")],
            ),
        ],
        "[:entry]\n",
        &[("mm/forms", "@package.moth", b"\xFF\xFE")],
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert_eq!(
        reports
            .iter()
            .next()
            .map(|(_, diagnostic)| diagnostic.descriptor().code),
        Some("MOTH-IMPORT-0006"),
        "the publisher's collision must own the report"
    );
    assert_single_diagnostic(
        &result,
        "MOTH-IMPORT-0006",
        "the collision must report exactly once",
    );
    assert!(
        reports
            .iter()
            .all(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006"),
        "the unreadable provider must not contribute its own failure"
    );
    let (primary, related) = diagnostic_sites(&result, "MOTH-IMPORT-0006");
    assert_eq!(
        primary,
        ("mm/@mod.moth".to_owned(), 0, 8),
        "the lone publisher keeps HEAD's package-relative display"
    );
    assert!(
        related.is_none(),
        "an unreadable provider has no renderable indexed location: {related:?}"
    );
    let (host, host_related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        host.0.ends_with("aa/mm/@mod.moth"),
        "the publisher's physical source is still the host site: {host:?}"
    );
    assert_eq!((host.1, host.2), (0, 8));
    assert!(
        host_related.is_none(),
        "the canonical projection must not invent a related site: {host_related:?}"
    );
}

#[test]
fn published_package_route_colliding_with_local_provider_is_rejected_eagerly() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temporary = tempfile::tempdir().expect("should create temporary project");
    let project_root = temporary.path().to_path_buf();
    let package_root = project_root.join("packages/vendor/lib");
    fs::create_dir_all(project_root.join("src/client/vendor/lib"))
        .expect("should create rooted consumer module");
    fs::create_dir_all(&package_root).expect("should create package root");
    fs::write(
        project_root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write project config");
    fs::write(project_root.join("src/@entry.moth"), "[:entry]\n")
        .expect("should write project entry");
    fs::write(
        project_root.join("src/client/@mod.moth"),
        "$export\nclient_label #= \"client\"\n",
    )
    .expect("should write rooted consumer module");
    fs::write(
        project_root.join("src/client/vendor/lib/forms.moth"),
        "value #= 1\n",
    )
    .expect("should write local provider");
    fs::write(
        package_root.join("@package.moth"),
        "$export @forms\nvalue #= 1\n",
    )
    .expect("should write routed package facade");

    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root("vendor/lib", package_root, PackageOrigin::ProjectLocal)
        .expect("the package root should register");

    let mut config = Config::new(project_root.join("src"));
    config.project_name = "docs".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert!(
        reports
            .iter()
            .any(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006"),
        "the published package route must collide eagerly with the visible local provider"
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("client/vendor/lib/forms.moth"),
        "the local provider owns the primary site: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 0));
    let related = related.expect("the package route operand keeps its authored site");
    assert!(
        related.0.ends_with("packages/vendor/lib/@package.moth"),
        "the published route operand is the related site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 8));
}

#[test]
fn local_facade_route_overlapping_registered_prefix_reports_package_facade() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temporary = tempfile::tempdir().expect("should create temporary project");
    let project_root = temporary.path().to_path_buf();
    let package_root = project_root.join("packages/vendor/lib");
    fs::create_dir_all(project_root.join("src/vendor")).expect("should create local facade folder");
    fs::create_dir_all(&package_root).expect("should create package root");
    fs::write(
        project_root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write project config");
    fs::write(project_root.join("src/@entry.moth"), "[:entry]\n")
        .expect("should write project entry");
    fs::write(
        project_root.join("src/vendor/@mod.moth"),
        "$export @lib\nproject_value #= 1\n",
    )
    .expect("should write local route facade");
    fs::write(package_root.join("@package.moth"), "$export\nlabel #= 1\n")
        .expect("should write package root facade");

    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root("vendor/lib", package_root, PackageOrigin::ProjectLocal)
        .expect("the package root should register");

    let mut config = Config::new(project_root.join("src"));
    config.project_name = "docs".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("src/vendor/@mod.moth"),
        "the local facade's route operand owns the primary site: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related =
        related.expect("the registered package's authored root facade supplies the previous site");
    assert!(
        related.0.ends_with("packages/vendor/lib/@package.moth"),
        "the package root facade is the related site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
}

#[test]
fn local_facade_route_colliding_with_package_route_reports_operand() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temporary = tempfile::tempdir().expect("should create temporary project");
    let project_root = temporary.path().to_path_buf();
    let package_root = project_root.join("packages/vendor/lib");
    fs::create_dir_all(project_root.join("src/vendor")).expect("should create local facade folder");
    fs::create_dir_all(&package_root).expect("should create package root");
    fs::write(
        project_root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write project config");
    fs::write(project_root.join("src/@entry.moth"), "[:entry]\n")
        .expect("should write project entry");
    fs::write(project_root.join("src/@mod.moth"), "").expect("should write project module root");
    fs::write(
        project_root.join("src/vendor/@mod.moth"),
        "$export @lib/forms\nproject_value #= 1\n",
    )
    .expect("should write local routed facade");
    fs::write(
        package_root.join("@package.moth"),
        "$export @forms\npackage_value #= 1\n",
    )
    .expect("should write package routed facade");

    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root("vendor/lib", package_root, PackageOrigin::ProjectLocal)
        .expect("the package root should register");

    let mut config = Config::new(project_root.join("src"));
    config.project_name = "docs".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("src/vendor/@mod.moth"),
        "the local facade's route operand owns the primary site: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the package route operand supplies the previous site");
    assert!(
        related.0.ends_with("packages/vendor/lib/@package.moth"),
        "the package root facade is the related site: {related:?}"
    );
    assert_eq!(
        (related.1, related.2),
        (0, 8),
        "the related site is the package's `$export` operand, not the file start"
    );
}

#[test]
fn ancestor_package_prefix_collision_reports_same_site_in_both_registration_orders() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let mut first_related: Option<(String, u32, u32)> = None;
    for reverse_registration in [false, true] {
        let temporary = tempfile::tempdir().expect("should create temporary project");
        let project_root = temporary.path().to_path_buf();
        let lib_root = project_root.join("packages/lib-root");
        let other_root = project_root.join("packages/other-root");
        // The local facade publishes `alpha/vendor`; both registered package prefixes descend
        // from that route key, so both match it through the shared ancestor-prefix rule.
        // Registration order must not change the lexicographically first related prefix.
        fs::create_dir_all(project_root.join("src/alpha"))
            .expect("should create local facade folder");
        fs::create_dir_all(&lib_root).expect("should create first package root");
        fs::create_dir_all(&other_root).expect("should create second package root");
        fs::write(
            project_root.join("@config.moth"),
            "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
        )
        .expect("should write project config");
        fs::write(project_root.join("src/@entry.moth"), "[:entry]\n")
            .expect("should write project entry");
        fs::write(
            project_root.join("src/alpha/@mod.moth"),
            "$export @vendor\nproject_value #= 1\n",
        )
        .expect("should write local routed facade");
        fs::write(lib_root.join("@package.moth"), "$export\nlib_label #= 1\n")
            .expect("should write first package facade");
        fs::write(
            other_root.join("@package.moth"),
            "$export\nother_label #= 1\n",
        )
        .expect("should write second package facade");

        let mut registrations = vec![
            ("alpha/vendor/lib", lib_root),
            ("alpha/vendor/other", other_root),
        ];
        if reverse_registration {
            registrations.reverse();
        }
        let mut frontend_surface = BuilderSurface::with_mandatory_core();
        for (package_prefix, package_root) in registrations {
            frontend_surface
                .source_packages
                .register_filesystem_root(package_prefix, package_root, PackageOrigin::ProjectLocal)
                .expect("the package root should register");
        }

        let mut config = Config::new(project_root.join("src"));
        config.project_name = "docs".to_owned();
        let style_directives = StyleDirectiveRegistry::built_ins();
        let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
        let result = compile_project_frontend(
            &mut config,
            BuildProfile::Dev,
            None,
            &style_directives,
            &mut frontend_surface,
            &mut warning_owner,
        );
        let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
        assert!(
            primary.0.ends_with("src/alpha/@mod.moth"),
            "the local facade's route operand owns the primary site: {primary:?}"
        );
        assert_eq!((primary.1, primary.2), (0, 8));
        let related = related.expect("the first package supplies the previous site");
        assert!(
            related.0.ends_with("packages/lib-root/@package.moth"),
            "the lexicographically first matching prefix is the deterministic related site: {related:?}"
        );
        assert_eq!((related.1, related.2), (0, 0));
        let relative_related = related
            .0
            .rsplit_once("packages/")
            .map(|(_, tail)| tail.to_owned())
            .unwrap_or_else(|| related.0.clone());
        let site = (relative_related, related.1, related.2);
        match &first_related {
            Some(first) => assert_eq!(
                &site, first,
                "registration order must not change the reported package site"
            ),
            None => first_related = Some(site),
        }
    }
}

#[test]
fn package_route_collision_reports_both_sites_after_an_unrelated_package_boundary() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The colliding package is authored on disk first but registers after an unrelated package.
    // Preparation order therefore differs from authored creation order; the donor boundary is not
    // the first inventory, and both authored sites must still be reported.
    let temporary = tempfile::tempdir().expect("should create temporary project");
    let project_root = temporary.path().to_path_buf();
    let colliding_root = project_root.join("packages/vendor/lib");
    let unrelated_root = project_root.join("packages/alpha/reference");
    fs::create_dir_all(project_root.join("src/client/vendor/lib/published"))
        .expect("should create descendant local provider folder");
    fs::create_dir_all(&colliding_root).expect("should create colliding package root");
    fs::create_dir_all(&unrelated_root).expect("should create unrelated package root");
    fs::write(
        project_root.join("@config.moth"),
        "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n",
    )
    .expect("should write project config");
    fs::write(project_root.join("src/@entry.moth"), "[:entry]\n")
        .expect("should write project entry");
    fs::write(
        project_root.join("src/client/@mod.moth"),
        "$export\nclient_label #= \"client\"\n",
    )
    .expect("should write rooted consumer module");
    fs::write(
        project_root.join("src/client/vendor/lib/published/details.moth"),
        "value #= 1\n",
    )
    .expect("should write descendant local provider");
    fs::write(
        colliding_root.join("@package.moth"),
        "$export @published\nvalue #= 1\n",
    )
    .expect("should write routed package facade");
    fs::write(
        unrelated_root.join("@package.moth"),
        "$export\nreference_label #= \"reference\"\n",
    )
    .expect("should write unrelated package facade");

    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root(
            "alpha/reference",
            unrelated_root,
            PackageOrigin::ProjectLocal,
        )
        .expect("the unrelated package should register first");
    frontend_surface
        .source_packages
        .register_filesystem_root("vendor/lib", colliding_root, PackageOrigin::ProjectLocal)
        .expect("the colliding package should register second");

    let mut config = Config::new(project_root.join("src"));
    config.project_name = "docs".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary
            .0
            .ends_with("client/vendor/lib/published/details.moth"),
        "the local provider keeps the primary site regardless of package order: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 0));
    let related = related.expect("the colliding package keeps its authored route operand site");
    assert!(
        related.0.ends_with("packages/vendor/lib/@package.moth"),
        "the second registered package's operand is the related site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 8));
}

#[test]
fn package_route_overlapping_another_package_namespace_reports_both_sites_without_an_importer() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let routed: RegisteredPackageFixture<'_> = (
        "tools",
        PackageOrigin::ProjectLocal,
        &[("@package.moth", "$export @forms\nvalue #= 1\n")],
    );
    let local_provider: RegisteredPackageFixture<'_> = (
        "apps/guest",
        PackageOrigin::ProjectLocal,
        &[
            ("@package.moth", "$export\nlabel #= 1\n"),
            ("tools/forms/@mod.moth", "$export\nform_label #= 1\n"),
        ],
    );
    let forward = [routed, local_provider];
    let reversed = [local_provider, routed];
    for packages in [&forward[..], &reversed[..]] {
        let (_temporary, result) =
            compile_registered_packages_project_retained(packages, "[:entry]\n");
        let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
        assert!(
            primary.0.ends_with("packages/tools/@package.moth"),
            "the routed package operand is primary regardless of registration order: {primary:?}"
        );
        assert_eq!((primary.1, primary.2), (0, 8));
        let related = related
            .expect("the other package's local provider is the related site even with no importer");
        assert!(
            related
                .0
                .ends_with("packages/apps/guest/tools/forms/@mod.moth"),
            "the competing package provider keeps its authored file: {related:?}"
        );
        assert_eq!((related.1, related.2), (0, 0));
    }
}

#[test]
fn descendant_route_collision_selects_the_claim_at_the_earliest_affected_boundary() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // Both `aa` and `zz` provide descendants under `tools/forms`. `aa` sorts first and imports
    // the route, so selecting the lexicographically first descendant (`zz`'s `a.moth`) would
    // defer the collision past `aa`'s walk and lose the two-site report to the importer span.
    let result = compile_registered_packages_project(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[
                    (
                        "@package.moth",
                        "@tools/forms value\n$export\nlabel #= value\n",
                    ),
                    ("tools/forms/z.moth", "local_value #= 1\n"),
                ],
            ),
            (
                "tools",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export @forms\ntool_value #= 1\n")],
            ),
            (
                "zz",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nlabel #= 1\n"),
                    ("tools/forms/a.moth", "later_value #= 1\n"),
                ],
            ),
        ],
        "[:entry]\n",
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("packages/tools/@package.moth"),
        "the routed package operand owns the collision: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the earliest affected provider keeps its authored site");
    assert!(
        related.0.ends_with("packages/aa/tools/forms/z.moth"),
        "the earliest affected boundary supplies the related site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
}

#[test]
fn distinct_route_collisions_select_the_earliest_affected_boundary_not_publisher_order() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // `forms` sorts before `other`, but its provider boundary sorts after `tools`. The `other`
    // collision involves `aa`, which walks first and imports the route; publisher-order selection
    // would therefore lose the two-site report.
    let result = compile_registered_packages_project(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[
                    (
                        "@package.moth",
                        "@tools/other value\n$export\nlabel #= value\n",
                    ),
                    ("tools/other/z.moth", "local_value #= 1\n"),
                ],
            ),
            (
                "tools",
                PackageOrigin::ProjectLocal,
                &[(
                    "@package.moth",
                    "$export @forms\none #= 1\n$export @other\ntwo #= 2\n",
                )],
            ),
            (
                "zz",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nlabel #= 1\n"),
                    ("tools/forms/a.moth", "later_value #= 1\n"),
                ],
            ),
        ],
        "[:entry]\n",
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("packages/tools/@package.moth"),
        "the routed package operand owns the collision: {primary:?}"
    );
    assert_eq!(
        (primary.1, primary.2),
        (2, 8),
        "the selected route is `@other`, whose provider boundary sorts first"
    );
    let related = related.expect("the earliest affected provider keeps its authored site");
    assert!(
        related.0.ends_with("packages/aa/tools/other/z.moth"),
        "the earliest affected boundary supplies the related site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
}

#[test]
fn later_package_route_collision_does_not_mask_an_earlier_missing_import() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // `aa` fails during its own discovery walk before the `zz`/`zzz` cross-package collision turn.
    // Eager collision detection must not replace that earlier authored first cause.
    let result = compile_registered_packages_project(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[(
                    "@package.moth",
                    "@zz/missing value\n$export\nlabel #= value\n",
                )],
            ),
            (
                "zz",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export @forms\nvalue #= 1\n")],
            ),
            (
                "zzz",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nlabel #= 1\n"),
                    ("zz/forms/@mod.moth", "$export\nform_label #= 1\n"),
                ],
            ),
        ],
        "[:entry]\n",
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert_eq!(
        reports
            .iter()
            .next()
            .map(|(_, diagnostic)| diagnostic.descriptor().code),
        Some("MOTH-IMPORT-0015"),
        "the earlier package's missing import owns the first diagnostic"
    );
    let (primary, _) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0015");
    assert!(
        primary.0.ends_with("packages/aa/@package.moth"),
        "the missing-import span belongs to the earlier package consumer: {primary:?}"
    );
}

#[test]
fn earlier_package_route_collision_survives_a_later_prepass_failure() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The unrelated `zz` package fails its facade prepass. The earlier `aa`/`ab` collision must
    // still be detected before imports and reported at `aa`'s prefix turn.
    let result = compile_registered_packages_project(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export @forms\nvalue #= 1\n")],
            ),
            (
                "ab",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nlabel #= 1\n"),
                    ("aa/forms/@mod.moth", "$export\nform_label #= 1\n"),
                ],
            ),
            (
                "zz",
                PackageOrigin::ProjectLocal,
                &[(
                    "@package.moth",
                    "$export @forms\none #= 1\n$export @FORMS\ntwo #= 2\n",
                )],
            ),
        ],
        "[:entry]\n",
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert_eq!(
        reports
            .iter()
            .next()
            .map(|(_, diagnostic)| diagnostic.descriptor().code),
        Some("MOTH-IMPORT-0006"),
        "the earlier collision must not be disabled by a later package's prepass failure"
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("packages/aa/@package.moth"),
        "the routed package operand owns the collision: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the competing package provider keeps its authored site");
    assert!(
        related.0.ends_with("packages/ab/aa/forms/@mod.moth"),
        "the provider keeps its authored file: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
}

#[test]
fn package_route_overlapping_another_package_namespace_with_an_importer_reports_both_sites() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The guest package imports the routed external path, so its own namespace overlap is also
    // visible to ordinary consumer resolution. Eager package validation must still be the
    // authority and retain both authored sites instead of degrading to the importer-only span.
    let routed: RegisteredPackageFixture<'_> = (
        "tools",
        PackageOrigin::ProjectLocal,
        &[("@package.moth", "$export @forms\nvalue #= 1\n")],
    );
    let local_provider: RegisteredPackageFixture<'_> = (
        "apps/guest",
        PackageOrigin::ProjectLocal,
        &[
            (
                "@package.moth",
                "@tools/forms value\n$export\nlabel #= value\n",
            ),
            ("tools/forms/@mod.moth", "$export\nform_label #= 1\n"),
        ],
    );
    let forward = [routed, local_provider];
    let reversed = [local_provider, routed];
    for packages in [&forward[..], &reversed[..]] {
        let (_temporary, result) =
            compile_registered_packages_project_retained(packages, "[:entry]\n");
        let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
        assert!(
            primary.0.ends_with("packages/tools/@package.moth"),
            "the routed package operand is primary regardless of registration order: {primary:?}"
        );
        assert_eq!((primary.1, primary.2), (0, 8));
        let related =
            related.expect("the importer package's local provider is the related authored site");
        assert!(
            related
                .0
                .ends_with("packages/apps/guest/tools/forms/@mod.moth"),
            "the competing package provider keeps its authored file: {related:?}"
        );
        assert_eq!((related.1, related.2), (0, 0));
    }
}

#[test]
fn single_file_package_inventory_lane_rejects_unreferenced_package_route_collision() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    let temporary = tempfile::tempdir().expect("should create temporary project");
    let project_root = temporary.path().to_path_buf();
    let routed_root = project_root.join("packages/tools");
    let local_provider_root = project_root.join("packages/apps/guest");
    fs::create_dir_all(project_root.join("src")).expect("should create source root");
    fs::create_dir_all(&routed_root).expect("should create routed package root");
    fs::create_dir_all(local_provider_root.join("tools/forms"))
        .expect("should create provider package folder");
    let entry_path = project_root.join("src/entry.moth");
    fs::write(project_root.join("src/intro.mtf"), "[p, center: Hello]")
        .expect("should write the single-file template input");
    fs::write(&entry_path, "body #= @intro.mtf\n").expect("should write the single-file entry");
    fs::write(
        routed_root.join("@package.moth"),
        "$export @forms\nvalue #= 1\n",
    )
    .expect("should write routed package facade");
    fs::write(
        local_provider_root.join("@package.moth"),
        "$export\nlabel #= 1\n",
    )
    .expect("should write provider package facade");
    fs::write(
        local_provider_root.join("tools/forms/@mod.moth"),
        "$export\nform_label #= 1\n",
    )
    .expect("should write competing package provider");

    let mut frontend_surface = BuilderSurface::with_mandatory_core();
    frontend_surface
        .source_packages
        .register_filesystem_root("tools", routed_root, PackageOrigin::ProjectLocal)
        .expect("the routed package should register");
    frontend_surface
        .source_packages
        .register_filesystem_root(
            "apps/guest",
            local_provider_root,
            PackageOrigin::ProjectLocal,
        )
        .expect("the provider package should register");
    let mut config = Config::new(entry_path);
    config.project_name = "docs".to_owned();
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    let result = compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut frontend_surface,
        &mut warning_owner,
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("packages/tools/@package.moth"),
        "the single-file package lane must reject the package route collision: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the competing package provider keeps its authored site");
    assert!(
        related
            .0
            .ends_with("packages/apps/guest/tools/forms/@mod.moth"),
        "the provider package keeps its authored file: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
}

#[test]
fn package_route_overlapping_an_unselected_package_descendant_reports_both_sites() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The provider is an ordinary package source that no module imports. Its namespace entry
    // still participates in the collision, so the donor boundary must retain the authored file
    // and its line index on this failure path instead of dropping the label.
    let routed: RegisteredPackageFixture<'_> = (
        "tools",
        PackageOrigin::ProjectLocal,
        &[("@package.moth", "$export @forms/details\nvalue #= 1\n")],
    );
    let local_provider: RegisteredPackageFixture<'_> = (
        "apps/guest",
        PackageOrigin::ProjectLocal,
        &[
            ("@package.moth", "$export\nlabel #= 1\n"),
            ("tools/forms/details.moth", "form_label #= 1\n"),
        ],
    );
    let forward = [routed, local_provider];
    let reversed = [local_provider, routed];
    for packages in [&forward[..], &reversed[..]] {
        let (_temporary, result) =
            compile_registered_packages_project_retained(packages, "[:entry]\n");
        let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
        assert!(
            primary.0.ends_with("packages/tools/@package.moth"),
            "the routed package operand is primary regardless of registration order: {primary:?}"
        );
        assert_eq!((primary.1, primary.2), (0, 8));
        let related = related.expect(
            "the unselected package provider keeps its authored site through its report domain",
        );
        assert!(
            related
                .0
                .ends_with("packages/apps/guest/tools/forms/details.moth"),
            "the competing package provider keeps its authored file: {related:?}"
        );
        assert_eq!((related.1, related.2), (0, 0));
    }
}

/// The routed package and competing provider shared by the unrenderable-excerpt tests.
fn competing_route_provider_packages() -> [RegisteredPackageFixture<'static>; 2] {
    [
        (
            "tools",
            PackageOrigin::ProjectLocal,
            &[("@package.moth", "$export @forms/details\nvalue #= 1\n")],
        ),
        (
            "apps/guest",
            PackageOrigin::ProjectLocal,
            &[
                ("@package.moth", "$export\nlabel #= 1\n"),
                ("tools/forms/details.moth", "form_label #= 1\n"),
            ],
        ),
    ]
}

/// Assert the collision survived while its related excerpt could not be loaded.
fn assert_route_collision_survives_without_related_excerpt(
    result: &CompilationResult<ProjectFrontendCompilation>,
) {
    let reports = super::super::create_project_modules_tests::diagnosed_reports(result);
    let (report, diagnostic) = reports
        .iter()
        .find(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006")
        .expect("the route collision must remain the report");
    let context = DiagnosticRenderContext::for_report(report);
    let primary = context
        .site_position(diagnostic.primary_site())
        .expect("the routed package operand still resolves");
    assert!(
        primary
            .host_path
            .is_some_and(|path| portable_path_text(path).ends_with("packages/tools/@package.moth")),
        "the routed package operand remains the primary authored site: {primary:?}"
    );
    assert_eq!((primary.start.line, primary.start.column), (0, 8));
    let labels = diagnostic.durable_labels().collect::<Vec<_>>();
    assert_eq!(
        labels.len(),
        1,
        "the collision keeps exactly one previous-site label"
    );
    assert!(
        labels
            .iter()
            .all(|label| context.site_position(label.site).is_none()),
        "an unloadable related excerpt stays unresolved instead of erasing the collision"
    );
}

#[test]
fn invalid_utf8_in_an_unselected_route_provider_keeps_the_route_collision() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The provider is unselected report material. Its invalid bytes must not replace the
    // authored route collision with an internal read failure; only the related excerpt is lost.
    let packages = competing_route_provider_packages();
    let (_temporary, result) = compile_registered_packages_project_with_damaged_sources(
        &packages,
        "[:entry]\n",
        &[("apps/guest", "tools/forms/details.moth", &[0xff])],
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert_eq!(
        reports
            .iter()
            .next()
            .map(|(_, diagnostic)| diagnostic.descriptor().code),
        Some("MOTH-IMPORT-0006"),
        "the route collision must survive an unreadable related excerpt"
    );
    assert_route_collision_survives_without_related_excerpt(&result);
}

#[test]
fn oversized_competing_route_provider_keeps_the_route_collision() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The real snapshot limit is u32::MAX; a thread-local test limit drives the same authored
    // capacity rejection. Every other fixture source is far below it, so only the unselected
    // provider exceeds the snapshot budget.
    let _limit = ForcedSourceSnapshotLimitGuard::new(64 * 1024);
    let oversized = vec![b'a'; 128 * 1024];
    let packages = competing_route_provider_packages();
    let (_temporary, result) = compile_registered_packages_project_with_damaged_sources(
        &packages,
        "[:entry]\n",
        &[("apps/guest", "tools/forms/details.moth", &oversized)],
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert!(
        reports
            .iter()
            .any(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006"),
        "the authored route collision must survive an oversized related excerpt"
    );
    assert_route_collision_survives_without_related_excerpt(&result);
}

#[test]
fn route_collision_preparation_failure_keeps_package_owner_for_donor_sites() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // A selected source in the `aa` donor package exceeds the snapshot budget during collision
    // finalization. The partially retained package database still owns the related provider site,
    // so both the capacity diagnostic and the collision must render through `aa`, never through
    // the project source table.
    let _limit = ForcedSourceSnapshotLimitGuard::new(64 * 1024);
    let oversized = vec![b'\n'; 128 * 1024];
    let (_temporary, result) = compile_registered_packages_project_with_damaged_sources(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nlabel #= 1\n"),
                    ("tools/forms.moth", "$export\nform_label #= 1\n"),
                    ("widgets/@mod.moth", "$export\nwidget_label #= 1\n"),
                ],
            ),
            (
                "tools",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export @forms\ntool_value #= 1\n")],
            ),
        ],
        "[:entry]\n",
        &[("aa", "widgets/@mod.moth", &oversized)],
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert!(
        reports
            .iter()
            .any(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006"),
        "the route collision must survive the donor's selected-source failure"
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("packages/tools/@package.moth"),
        "the routed package operand remains the primary authored site: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect(
        "the failed donor still supplies its finished package database for the related site",
    );
    assert!(
        related.0.ends_with("packages/aa/tools/forms.moth"),
        "the related package site resolves through the donor's own database: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
}

#[test]
fn cleanup_keeps_each_package_capacity_diagnostic_after_shared_root_batches() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // Both package owners reject a selected snapshot during collision cleanup. The first batch
    // must capture against the shared root table without taking it; otherwise the second batch
    // validates its original string IDs against an empty table and infrastructure suppresses both
    // capacity diagnostics and the collision.
    let _limit = ForcedSourceSnapshotLimitGuard::new(64 * 1024);
    let oversized_selected = vec![b'\n'; 128 * 1024];
    let (_temporary, result) = compile_registered_packages_project_with_damaged_sources(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nlabel #= 1\n"),
                    ("tools/forms/@mod.moth", "$export\nform_label #= 1\n"),
                    ("widgets/@mod.moth", "$export\nwidget_label #= 1\n"),
                ],
            ),
            (
                "tools",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export @forms\ntool_value #= 1\n"),
                    ("extra/@mod.moth", "$export\nextra_label #= 1\n"),
                ],
            ),
        ],
        "[:entry]\n",
        &[
            ("aa", "widgets/@mod.moth", &oversized_selected),
            ("tools", "extra/@mod.moth", &oversized_selected),
        ],
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    let capacity_reports = reports
        .iter()
        .filter(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-SYNTAX-0036")
        .map(|(report, diagnostic)| {
            assert!(
                matches!(
                    diagnostic.primary_site(),
                    crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef::Absent
                ),
                "an oversized snapshot is named by its path fact, not a positional site"
            );
            terse::format_terse_diagnostic_view(
                diagnostic,
                DiagnosticRenderContext::for_report(report),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        capacity_reports.len(),
        2,
        "both package capacity diagnostics must survive the shared root table: {capacity_reports:?}"
    );
    assert!(
        capacity_reports
            .iter()
            .any(|report| report.contains("widgets/@mod.moth")),
        "the donor's capacity diagnostic keeps its package-relative path: {capacity_reports:?}"
    );
    assert!(
        capacity_reports
            .iter()
            .any(|report| report.contains("extra/@mod.moth")),
        "the route package's capacity diagnostic keeps its package-relative path: \
         {capacity_reports:?}"
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("packages/tools/@package.moth"),
        "the route operand remains the primary collision site: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the competing package provider keeps its authored site");
    assert!(
        related.0.ends_with("packages/aa/tools/forms/@mod.moth"),
        "the related provider keeps its package path: {related:?}"
    );
}

#[test]
fn capacity_rejected_donor_site_is_not_re_read_during_cleanup() {
    let _test_guard = crate::timing::lock_instrumentation_tests();
    let _limit = ForcedSourceSnapshotLimitGuard::new(64 * 1024);
    let oversized = vec![b'\n'; 128 * 1024];
    let temporary_root =
        fs::canonicalize(std::env::temp_dir()).expect("the system temp root should canonicalize");
    super::super::source_loading_test_support::reset_source_read_count_for_test(&temporary_root);
    let (temporary, result) = compile_registered_packages_project_with_damaged_sources(
        &[
            (
                "aa",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nlabel #= 1\n"),
                    ("tools/forms/@mod.moth", "$export\nform_label #= 1\n"),
                ],
            ),
            (
                "tools",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export @forms\ntool_value #= 1\n")],
            ),
        ],
        "[:entry]\n",
        &[("aa", "tools/forms/@mod.moth", &oversized)],
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert!(
        reports
            .iter()
            .any(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006"),
        "the collision must survive a capacity-rejected donor excerpt"
    );
    let provider_path =
        fs::canonicalize(temporary.path().join("packages/aa/tools/forms/@mod.moth"))
            .expect("the donor provider should canonicalize");
    assert_eq!(
        super::super::source_loading_test_support::source_read_count_for_path_for_test(
            &provider_path
        ),
        1,
        "the capacity-rejected donor snapshot must not be read again during cleanup"
    );
}

#[test]
fn already_recorded_non_utf8_competing_facade_keeps_the_route_collision() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // Facade preparation reads the competing `@mod.moth` and records its invalid UTF-8 before
    // collision donor finalization. The donor must consult that terminal slot state instead of
    // attempting a second NonUtf8 transition, which would replace the collision with an internal
    // source-lifecycle error.
    let (_temporary, result) = compile_registered_packages_project_with_damaged_sources(
        &[
            (
                "tools",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export @forms\ntool_value #= 1\n")],
            ),
            (
                "apps/guest",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nlabel #= 1\n"),
                    ("tools/forms/@mod.moth", "$export\nform_label #= 1\n"),
                ],
            ),
        ],
        "[:entry]\n",
        &[("apps/guest", "tools/forms/@mod.moth", &[0xff])],
    );
    let reports = super::super::create_project_modules_tests::diagnosed_reports(&result);
    assert!(
        reports
            .iter()
            .any(|(_, diagnostic)| diagnostic.descriptor().code == "MOTH-IMPORT-0006"),
        "the authored route collision must survive an already-recorded NonUtf8 facade"
    );
    assert_route_collision_survives_without_related_excerpt(&result);
}

#[test]
fn package_route_overlapping_another_registered_prefix_reports_both_sites() {
    let _test_guard = crate::compiler_frontend::instrumentation::lock_counter_test();
    // The `local/host` boundary's internal `local/guest` facade publishes `@forms`, which folds
    // onto the separately registered `local/guest` prefix. Both authored surfaces exist, so the
    // report names the internal operand and the registered package's root facade.
    let result = compile_registered_packages_project(
        &[
            (
                "local/host",
                PackageOrigin::ProjectLocal,
                &[
                    ("@package.moth", "$export\nlabel #= \"host\"\n"),
                    ("local/guest/@mod.moth", "$export @forms\nvalue #= 1\n"),
                ],
            ),
            (
                "local/guest",
                PackageOrigin::ProjectLocal,
                &[("@package.moth", "$export\nlabel #= \"guest\"\n")],
            ),
        ],
        "[:entry]\n",
    );
    let (primary, related) = diagnostic_canonical_sites(&result, "MOTH-IMPORT-0006");
    assert!(
        primary.0.ends_with("local/host/local/guest/@mod.moth"),
        "the internal route operand owns the diagnostic: {primary:?}"
    );
    assert_eq!((primary.1, primary.2), (0, 8));
    let related = related.expect("the registered prefix supplies its authored root facade site");
    assert!(
        related.0.ends_with("packages/local/guest/@package.moth"),
        "the competing package root facade is the related site: {related:?}"
    );
    assert_eq!((related.1, related.2), (0, 0));
}

#[test]
fn reexport_clause_before_declaration_reports_the_earlier_authored_route_site() {
    const FACADE: &str = "$export @Forms\n@child label\n$export @forms\nfirst #= 1\n";
    let result = compile_fixture_result(
        &[
            ("src/child/@mod.moth", "$export\nlabel #= \"child\"\n"),
            ("src/@mod.moth", FACADE),
        ],
        false,
        BuildProfile::Dev,
    );
    let (primary, related) = diagnostic_site_facts(&result, "MOTH-RULE-0092");
    // Pass 1 records the later declaration first; the diagnostic must still point at the later
    // declaration as primary and the earlier re-export operand as related.
    assert_eq!(primary, ((2, 8), "@forms".to_owned()));
    assert_eq!(related, ((0, 8), "@Forms".to_owned()));
}

#[test]
fn repeated_route_claims_keep_the_earliest_authored_operand() {
    const FACADE: &str = concat!(
        "$export @Forms\n",
        "@child one\n",
        "$export @forms\n",
        "@child two\n",
        "$export @Forms\n",
        "render #= 1\n",
    );
    let result = compile_fixture_result(
        &[
            (
                "src/child/@mod.moth",
                "$export\none #= 1\n$export\ntwo #= 2\n",
            ),
            ("src/@mod.moth", FACADE),
        ],
        false,
        BuildProfile::Dev,
    );
    let (primary, related) = diagnostic_site_facts(&result, "MOTH-RULE-0092");
    // The final `@Forms` declaration is pass-1's first claim, but the earlier authored `@Forms`
    // clause replaces it. The middle `@forms` clause therefore conflicts with the first clause.
    assert_eq!(primary, ((2, 8), "@forms".to_owned()));
    assert_eq!(related, ((0, 8), "@Forms".to_owned()));
}

#[test]
fn duplicate_leaf_in_one_route_across_declaration_and_reexport_reports_both_sites() {
    let result = compile_fixture_result(
        &[
            ("src/child/@mod.moth", "$export\nrender #= \"child\"\n"),
            (
                "src/@mod.moth",
                "$export @forms\nrender #= \"direct\"\n$export @forms\n@child render\n",
            ),
        ],
        false,
        BuildProfile::Dev,
    );
    assert_two_site_diagnostic(&result, "MOTH-RULE-0081");
}

#[test]
fn routed_facade_export_of_private_struct_field_type_is_rejected() {
    let result = compile_fixture_result(
        &[
            (
                "src/sub/@mod.moth",
                "PrivateData = |\n    value String,\n|\n\n$export @forms\nPublicData = |\n    private_data PrivateData,\n|\n",
            ),
            (
                "src/@entry.moth",
                "@sub/forms PublicData\n\n~[:routed_private_field]",
            ),
        ],
        false,
        BuildProfile::Dev,
    );
    assert_has_diagnostic_code(&result, "MOTH-RULE-0082");
}

#[test]
fn root_export_may_use_a_type_published_only_under_a_nested_route() {
    let frontend = compile_fixture(&[
        (
            "src/sub/@mod.moth",
            "$export @forms\nPublicData = |\n    value String,\n|\n\n$export\nRootData = |\n    public_data PublicData,\n|\n",
        ),
        (
            "src/@entry.moth",
            "@sub RootData\n\n~[:root_uses_nested_type]",
        ),
    ]);
    let interface = normal_interface_for(&frontend, "sub");
    assert!(
        interface
            .export_bindings
            .iter()
            .any(|binding| binding.route().is_root() && binding.public_name() == "RootData"),
        "the root export whose signature uses a nested-route type must remain public"
    );
    assert!(
        interface
            .export_bindings
            .iter()
            .any(|binding| binding.route().as_str() == "forms"
                && binding.public_name() == "PublicData"),
        "the field type stays published under its own nested route"
    );
}

#[test]
fn same_leaf_from_different_targets_across_routes_keeps_local_name_collision() {
    let result = compile_fixture_result(
        &[
            ("src/left/@mod.moth", "$export\nrender #= \"left\"\n"),
            ("src/right/@mod.moth", "$export\nrender #= \"right\"\n"),
            (
                "src/@mod.moth",
                "$export @forms\n@left render\n$export @tables\n@right render\n",
            ),
        ],
        false,
        BuildProfile::Dev,
    );
    assert_has_diagnostic_code(&result, "MOTH-IMPORT-0009");
}

#[test]
fn same_target_reexported_under_two_routes_keeps_both_bindings_and_one_origin() {
    let frontend = compile_fixture(&[
        ("src/left/@mod.moth", "$export\nrender #= \"left\"\n"),
        (
            "src/@mod.moth",
            "$export @forms\n@left render\n$export @tables\n@left render\n",
        ),
    ]);
    let interface = normal_interface_with_route(&frontend, "forms", "render");
    let bindings: Vec<_> = interface
        .export_bindings
        .iter()
        .filter(|binding| binding.public_name() == "render")
        .collect();
    assert_eq!(bindings.len(), 2, "both routes must publish a binding");
    let routes: Vec<_> = bindings
        .iter()
        .map(|binding| binding.route().as_str())
        .collect();
    assert_eq!(routes, vec!["forms", "tables"]);
    let origin = bindings[0].origin();
    assert!(
        bindings.iter().all(|binding| binding.origin() == origin),
        "publishing one target under two routes must keep one declaration origin"
    );
    assert_eq!(
        interface
            .declarations
            .iter()
            .filter(|declaration| &declaration.origin == origin)
            .count(),
        1,
        "the two route bindings must join one declaration record"
    );
}

#[test]
fn changing_only_one_reexport_route_changes_the_fingerprint_not_origins() {
    let compile_with_second_route = |second_route: &str| {
        let facade =
            format!("$export @forms\n@left render\n$export @{second_route}\n@left render\n");
        compile_fixture(&[
            ("src/left/@mod.moth", "$export\nrender #= \"left\"\n"),
            ("src/@mod.moth", facade.as_str()),
        ])
    };
    let before = compile_with_second_route("tables");
    let after = compile_with_second_route("cards");
    let before_interface =
        interface_fingerprint_content(normal_interface_with_route(&before, "forms", "render"));
    let after_interface =
        interface_fingerprint_content(normal_interface_with_route(&after, "forms", "render"));
    assert_eq!(
        before_interface
            .export_bindings
            .iter()
            .map(|binding| binding.origin())
            .collect::<Vec<_>>(),
        after_interface
            .export_bindings
            .iter()
            .map(|binding| binding.origin())
            .collect::<Vec<_>>(),
        "changing a second route must not change declaration origins"
    );
    assert_ne!(
        before_interface, after_interface,
        "the route is part of the public-interface fingerprint"
    );
}

/// Compiles one member of the repeated-build route family.
///
/// WHAT: the provider facade publishes its one const function under `provider_route`; the consumer
///       facade selects it through `consumer_route`, folds it into its own constant and an
///       unrelated facade publishes no dependency on either.
/// WHY:  this project frontend has no cross-build artefact cache, so the reuse fact is the canonical
///       public interface. A provider route change must alter the provider interface, and a consumer
///       clause that still names the old route must be rejected rather than resolved against the
///       new surface.
fn compile_route_reuse_family(
    provider_route: &str,
    consumer_route: &str,
) -> CompilationResult<ProjectFrontendCompilation> {
    let provider = format!(
        "$export @{provider_route}\nrender #|label String| -> String:\n    return label\n;\n"
    );
    let consumer =
        format!("@shapes/{consumer_route} render\n\n$export\nrendered #= render(\"route\")\n");
    compile_fixture_result(
        &[
            ("src/shapes/@package.moth", provider.as_str()),
            ("src/affected/@mod.moth", consumer.as_str()),
            ("src/bystander/@mod.moth", "$export\nunrelated #= 1\n"),
            (
                "src/@entry.moth",
                "@affected rendered\n@bystander unrelated\n~[: [rendered] [unrelated] ]\n",
            ),
        ],
        false,
        BuildProfile::Dev,
    )
}

/// WHAT: changing only a provider export route changes the provider public interface and rejects a
///       consumer whose retained clause still names the old route; following the new route keeps
///       the consumer's own canonical interface and an unrelated consumer's interface equal.
/// WHY:  the accepted interface equality is the consumer invalidation key. With no cross-build
///       cache, the observable contract is the route-keyed provider surface plus the stale clause's
///       rejection. The unrelated facade proves the change does not widen every consumer.
#[test]
fn route_only_export_change_invalidates_the_provider_binding_and_rejects_the_stale_consumer() {
    let before = successful_compilation_value(compile_route_reuse_family("forms", "forms"));

    // The same authored project must reproduce the exact canonical interfaces consumers reuse.
    let unchanged = successful_compilation_value(compile_route_reuse_family("forms", "forms"));
    assert_eq!(
        interface_fingerprint_content(support_interface_with_route(&unchanged, "forms", "render")),
        interface_fingerprint_content(support_interface_with_route(&before, "forms", "render")),
        "an unchanged routed provider must reproduce its reusable interface"
    );
    assert_eq!(
        interface_fingerprint_content(normal_interface_for(&unchanged, "affected")),
        interface_fingerprint_content(normal_interface_for(&before, "affected")),
        "an unchanged routed consumer must reproduce its reusable interface"
    );

    // A real cache would reuse the consumer only against the matching provider interface. The same
    // unchanged clause against a moved route must therefore fail to bind, not silently resolve.
    let stale = compile_route_reuse_family("cards", "forms");
    assert_has_diagnostic_code(&stale, "MOTH-IMPORT-0015");

    let followed = successful_compilation_value(compile_route_reuse_family("cards", "cards"));

    let before_provider = support_interface_with_route(&before, "forms", "render");
    let after_provider = support_interface_with_route(&followed, "cards", "render");
    assert!(
        before_provider
            .export_bindings
            .iter()
            .all(|binding| binding.route().as_str() != "cards"),
        "the first build must publish only the forms route"
    );
    assert!(
        after_provider
            .export_bindings
            .iter()
            .all(|binding| binding.route().as_str() != "forms"),
        "the moved route must no longer be a provider binding key"
    );
    assert_ne!(
        interface_fingerprint_content(before_provider),
        interface_fingerprint_content(after_provider),
        "an exported route change must change the provider public interface"
    );

    // Only the binding route and its authored provenance may differ: the declaration origin and
    // every declaration-owned semantic fact must survive the route change unchanged.
    assert_eq!(before_provider.module_origin, after_provider.module_origin);
    assert_eq!(before_provider.declarations, after_provider.declarations);
    assert_eq!(
        before_provider.binding_exports,
        after_provider.binding_exports
    );
    assert_eq!(
        before_provider.reusable_evidence,
        after_provider.reusable_evidence
    );
    assert_eq!(
        before_provider.concrete_call_summaries,
        after_provider.concrete_call_summaries
    );

    let before_consumer = normal_interface_for(&before, "affected");
    let after_consumer = normal_interface_for(&followed, "affected");
    assert_eq!(
        interface_fingerprint_content(before_consumer),
        interface_fingerprint_content(after_consumer),
        "a consumer that follows the same declaration through a renamed route keeps its own interface"
    );
    assert_eq!(
        exported_constant_text(before_consumer, "rendered"),
        "route",
        "the consumer must fold the routed declaration value from the first build"
    );
    assert_eq!(
        exported_constant_text(after_consumer, "rendered"),
        "route",
        "the consumer must fold the same routed declaration value after the route change"
    );

    assert_eq!(
        interface_fingerprint_content(normal_interface_for(&before, "bystander")),
        interface_fingerprint_content(normal_interface_for(&followed, "bystander")),
        "a module outside the changed provider edge must keep an unchanged interface"
    );
}

/// Compiles one member of the routed private-body family.
fn compile_routed_private_body(body_line: &str) -> ProjectFrontendCompilation {
    let provider = format!(
        "$export @forms\nrender |label String| -> String:\n{body_line}    return label\n;\n"
    );
    compile_fixture(&[
        ("src/shapes/@package.moth", provider.as_str()),
        (
            "src/consumer/@mod.moth",
            "@shapes/forms render\n\n$export\nwrapped |label String| -> String:\n    return render(label)\n;\n",
        ),
        (
            "src/@entry.moth",
            "@consumer wrapped\n\n~[: [wrapped(\"ok\")] ]\n",
        ),
    ])
}

/// WHAT: editing only an exported function's private implementation body leaves the routed public
///       interface and a consumer's canonical view equal, while the provider artefact really
///       compiles different implementation work.
/// WHY:  the provider public interface is the semantic consumer's reuse key. The added `io.line`
///       demand proves the fixture changed the implementation rather than silently compiling the
///       same body twice; it adds no call-summary trap, failure or fast-math bit.
#[test]
fn private_implementation_body_edit_preserves_the_routed_interface_and_consumer_view() {
    let before = compile_routed_private_body("");
    let after = compile_routed_private_body("    io.line(\"edited\")\n");

    let before_provider = support_interface_with_route(&before, "forms", "render");
    let after_provider = support_interface_with_route(&after, "forms", "render");
    assert_eq!(
        interface_fingerprint_content(before_provider),
        interface_fingerprint_content(after_provider),
        "a private implementation body must not change the routed provider interface"
    );
    assert_eq!(
        interface_fingerprint_content(normal_interface_for(&before, "consumer")),
        interface_fingerprint_content(normal_interface_for(&after, "consumer")),
        "a semantic consumer of the unchanged provider interface must be reusable"
    );

    let before_provider_artifact = artifact_with_support_facade(&before, "shapes");
    let after_provider_artifact = artifact_with_support_facade(&after, "shapes");
    assert!(
        !reachable_external_functions(before_provider_artifact)
            .contains(&ExternalFunctionId::IoLine),
        "the unchanged implementation must not demand io.line"
    );
    assert!(
        reachable_external_functions(after_provider_artifact).contains(&ExternalFunctionId::IoLine),
        "the edited implementation must publish its io.line demand"
    );
}

/// One routed provider facade exporting every declaration category that must survive a route.
const ROUTED_SHAPES_FACADE: &str = "$export @forms\nBox = |\n    value I32,\n|\n\npeek |this Box| -> I32:\n    return this.value + 1 catch then 0\n;\n\n$export @forms\ntwice type T |value T| -> T:\n    return value\n;\n\n$export @forms\nROUTE_LABEL #= \"route constant\"\n\n$export @forms\nwrap #|label String| -> String:\n    return label\n;\n\n$export @forms\nCrate as Box\n";

/// One re-exporting facade that republishes the routed declarations under a second route and
/// aliases their leaf names.
const ROUTED_LIBRARY_FACADE: &str = "$export @mirror\n@shapes/forms Box as Crate, twice, ROUTE_LABEL, wrap as label, Crate as BoxAlias\n";

/// Returns the export binding published at exactly `route` and `name`.
fn route_binding<'a>(
    interface: &'a PublicSemanticInterface,
    route: &str,
    name: &str,
) -> &'a ExportBinding {
    interface
        .export_bindings
        .iter()
        .find(|binding| binding.route().as_str() == route && binding.public_name() == name)
        .unwrap_or_else(|| panic!("expected {name:?} under route {route:?}"))
}

/// WHAT: a route is a published location, not part of the declaration. The provider and a
///       re-exporting facade must publish one stable origin for each declaration, preserve nominal
///       receiver surfaces, generic-template declarations, folded constants and leaf aliases.
/// WHY:  consumer invalidation, generated instances and runtime linking key on these canonical
///       facts. Runtime integration cases already execute the same categories; this asserts the
///       canonical interface data that a cold execution case cannot observe.
#[test]
fn routed_declarations_publish_stable_origins_types_templates_receivers_constants_and_aliases() {
    let frontend = compile_fixture(&[
        ("src/shapes/@package.moth", ROUTED_SHAPES_FACADE),
        ("src/library/@mod.moth", ROUTED_LIBRARY_FACADE),
        (
            "src/@entry.moth",
            "@library/mirror Crate, twice, ROUTE_LABEL, label, BoxAlias\n\nitem Crate = Crate(2)\nmirrored = twice(\"twice\")\naliased BoxAlias = Crate(3)\ntext = label(\"ok\")\n\n~[: [mirrored] [ROUTE_LABEL] [text] [item.peek()] [aliased.value] ]\n",
        ),
    ]);

    let provider = support_interface_with_route(&frontend, "forms", "Box");
    assert!(
        provider
            .export_bindings
            .iter()
            .all(|binding| binding.route().as_str() == "forms"),
        "every provider declaration is routed; no root binding may leak from the nested route"
    );

    let box_binding = route_binding(provider, "forms", "Box");
    let OriginDeclarationId::Type(box_origin) = box_binding.origin() else {
        panic!("Box must publish a type origin");
    };
    assert_eq!(box_origin.defining_name(), "Box");
    assert_eq!(box_origin.category(), OriginTypeCategory::Struct);
    let box_declaration = provider
        .declaration(box_binding.origin())
        .expect("the routed type binding must join its declaration record");
    let PublicDeclarationSemantics::Struct(structure) = &box_declaration.semantics else {
        panic!("Box must retain struct semantics under its route");
    };
    assert_eq!(structure.fields.len(), 1);
    assert_eq!(
        structure.receiver_methods.len(),
        1,
        "the receiver method must travel with its exported type"
    );
    let peek = &structure.receiver_methods[0];
    assert_eq!(peek.method_origin.defining_name(), "peek");
    assert!(
        peek.method_origin.receiver().is_some(),
        "the receiver surface must retain its receiver identity"
    );

    let twice_binding = route_binding(provider, "forms", "twice");
    let twice_declaration = provider
        .declaration(twice_binding.origin())
        .expect("the routed generic declaration must publish a record");
    let PublicDeclarationSemantics::Function(twice) = &twice_declaration.semantics else {
        panic!("twice must retain function semantics under its route");
    };
    let PublicFunctionCategory::GenericTemplate(template) = &twice.category else {
        panic!("twice must remain a generic template under its route");
    };
    assert_eq!(template.generic_parameters.len(), 1);

    let label_binding = route_binding(provider, "forms", "ROUTE_LABEL");
    assert_eq!(
        exported_constant_text(provider, "ROUTE_LABEL"),
        "route constant",
        "a routed constant must publish its fully folded value"
    );
    let OriginDeclarationId::Constant(label_origin) = label_binding.origin() else {
        panic!("ROUTE_LABEL must publish a constant origin");
    };
    assert_eq!(label_origin.defining_name(), "ROUTE_LABEL");

    let wrap_binding = route_binding(provider, "forms", "wrap");
    assert_eq!(wrap_binding.origin().defining_name(), "wrap");
    let crate_alias_binding = route_binding(provider, "forms", "Crate");
    let OriginDeclarationId::Type(crate_alias_origin) = crate_alias_binding.origin() else {
        panic!("the transparent alias must publish a type origin");
    };
    assert_eq!(
        crate_alias_origin.category(),
        OriginTypeCategory::TransparentAlias
    );

    // The re-exporter publishes its own route and leaf aliases, but must not manufacture new
    // declaration identities: every binding joins the provider-owned origin and semantics.
    let library = normal_interface_with_route(&frontend, "mirror", "Crate");
    assert_eq!(
        box_binding.origin().module_origin(),
        &provider.module_origin,
        "the routed declaration origin must name the declaring provider module"
    );
    assert_ne!(
        box_binding.origin().module_origin(),
        &library.module_origin,
        "the re-exporter must not rebase the origin onto its own module"
    );
    assert_eq!(
        route_binding(library, "mirror", "Crate").origin(),
        box_binding.origin(),
        "a leaf alias must not change the routed type origin"
    );
    assert_eq!(
        route_binding(library, "mirror", "label").origin(),
        wrap_binding.origin(),
        "a leaf alias must not change the routed function origin"
    );
    assert_eq!(
        route_binding(library, "mirror", "ROUTE_LABEL").origin(),
        label_binding.origin(),
        "a re-export must not change the routed constant origin"
    );
    assert_eq!(
        route_binding(library, "mirror", "twice").origin(),
        twice_binding.origin()
    );
    assert_eq!(
        route_binding(library, "mirror", "BoxAlias").origin(),
        crate_alias_binding.origin(),
        "a re-exported transparent alias keeps its authored origin"
    );

    let library_box = library
        .declaration(box_binding.origin())
        .expect("the re-exporter must join the provider's struct declaration");
    let PublicDeclarationSemantics::Struct(library_structure) = &library_box.semantics else {
        panic!("the re-exported type must retain struct semantics");
    };
    assert_eq!(
        library_structure.receiver_methods.len(),
        1,
        "the re-exported type must retain its receiver surface"
    );
    let library_twice = library
        .declaration(twice_binding.origin())
        .expect("the re-exporter must join the provider's generic declaration");
    let PublicDeclarationSemantics::Function(library_twice) = &library_twice.semantics else {
        panic!("the re-exported generic must retain function semantics");
    };
    assert!(matches!(
        library_twice.category,
        PublicFunctionCategory::GenericTemplate(_)
    ));
}

#[test]
fn case_only_route_spellings_remain_legal_across_separate_facades() {
    let frontend = compile_fixture(&[
        ("src/@mod.moth", "$export\nunrelated #= 1\n"),
        ("src/one/@mod.moth", "$export @forms\nvalue #= 1\n"),
        ("src/two/@mod.moth", "$export @Forms\nvalue #= 2\n"),
    ]);
    let forms = normal_interface_with_route(&frontend, "forms", "value");
    let forms_upper = normal_interface_with_route(&frontend, "Forms", "value");
    assert_ne!(
        forms.module_origin, forms_upper.module_origin,
        "the fixtures must exercise two separate facades"
    );
}

/// Returns the published `(route, leaf)` bindings of one normal interface in publication order.
///
/// WHY: route bindings are ordered facts in the canonical public interface, so comparing the
///      sequence catches provider-order drift that unordered set comparisons would hide.
fn interface_route_bindings(interface: &PublicSemanticInterface) -> Vec<(String, String)> {
    interface
        .export_bindings
        .iter()
        .map(|binding| {
            (
                binding.route().as_str().to_owned(),
                binding.public_name().to_owned(),
            )
        })
        .collect()
}

/// Returns every normal facade's route binding sequence in module identity order.
fn published_route_bindings(frontend: &ProjectFrontendCompilation) -> Vec<Vec<(String, String)>> {
    frontend
        .project
        .successful_artefacts_in_module_id_order()
        .filter_map(|artifact| artifact.normal_interface.as_ref())
        .map(interface_route_bindings)
        .collect()
}

/// WHAT: two builds of one routed project publish identical module identities, route binding
///       sequences and facade interfaces, including when fixture files are created in reverse.
/// WHY:  the Stage 0 facade route prepass and Stage 3 public-export publication must not depend on
///       filesystem enumeration or source registration order. Reordered bindings would invalidate
///       unrelated consumers and make route publication host-order dependent.
#[test]
fn namespaced_route_builds_are_deterministic_across_creation_order() {
    let files = [
        (
            "src/@entry.moth",
            "@alpha/forms left as alpha_left\n\
             @zeta/mirror left as mirrored_left\n\
             @alpha/forms as route_ns\n\n\
             alpha = alpha_left(\"a\")\n\
             mirrored = mirrored_left(\"m\")\n\
             also = route_ns.also\n\
             folded #I32 = 40 + 2\n\
             [: folded=[folded] ]\n\
             ~[: [alpha] [mirrored] [also] ]\n",
        ),
        (
            "src/alpha/@mod.moth",
            "$export @forms\n\
             left #|value String| -> String:\n    return [:alpha:[value]]\n;\n\n\
             $export @forms\n\
             also #= \"also\"\n",
        ),
        ("src/zeta/@mod.moth", "$export @mirror\n@inner/forms left\n"),
        (
            "src/zeta/inner/@mod.moth",
            "$export @forms\n\
             left #|value String| -> String:\n    return [:inner:[value]]\n;\n",
        ),
        (
            "src/omega/@mod.moth",
            "$export @forms\n\
             left #|value String| -> String:\n    return [:omega:[value]]\n;\n",
        ),
    ];
    let mut reversed = files.to_vec();
    reversed.reverse();

    let first = compile_template_fixture(&files);
    let second = compile_template_fixture(&reversed);
    let repeat = compile_template_fixture(&files);

    let stable_origins = |frontend: &ProjectFrontendCompilation| -> Vec<_> {
        frontend
            .project
            .structure
            .nodes()
            .iter()
            .map(|node| node.stable_origin().clone())
            .collect()
    };
    assert_eq!(
        stable_origins(&first),
        stable_origins(&second),
        "module graph identities must not depend on source creation order"
    );
    assert_eq!(
        stable_origins(&first),
        stable_origins(&repeat),
        "a repeated build must reproduce module graph identities"
    );

    let interfaces = |frontend: &ProjectFrontendCompilation| -> Vec<_> {
        frontend
            .project
            .successful_artefacts_in_module_id_order()
            .map(|artifact| {
                (
                    artifact
                        .normal_interface
                        .as_ref()
                        .map(interface_fingerprint_content),
                    artifact
                        .support_interface
                        .as_ref()
                        .map(interface_fingerprint_content),
                )
            })
            .collect()
    };
    assert_eq!(
        interfaces(&first),
        interfaces(&second),
        "facade interfaces, including ordered route bindings, must repeat exactly"
    );
    assert_eq!(
        interfaces(&first),
        interfaces(&repeat),
        "a repeated build must reproduce every facade interface"
    );

    let first_routes = published_route_bindings(&first);
    assert!(
        first_routes
            .iter()
            .flatten()
            .any(|(route, leaf)| { route == "forms" && leaf == "left" })
            && first_routes
                .iter()
                .flatten()
                .any(|(route, leaf)| route == "mirror" && leaf == "left"),
        "the fixture must publish the same leaf through two routes: {first_routes:?}"
    );
    let forms_left_facades = first_routes
        .iter()
        .filter(|bindings| {
            bindings
                .iter()
                .any(|(route, leaf)| route == "forms" && leaf == "left")
        })
        .count();
    assert_eq!(
        forms_left_facades, 3,
        "alpha, omega and zeta/inner must all publish `forms/left`: {first_routes:?}"
    );

    assert_eq!(
        entry_folded_output(&first),
        " folded=42 ",
        "the fixture must publish a non-empty const-folded entry fragment"
    );
    assert_eq!(
        entry_folded_output(&first),
        entry_folded_output(&second),
        "folded entry content must not depend on creation order"
    );
    assert_eq!(
        entry_folded_output(&first),
        entry_folded_output(&repeat),
        "folded entry content must repeat across builds"
    );
}

/// WHAT: independent case-conflicting public routes in two facades report the same typed
///       diagnostic, sites and terse text on repeated compilations and when fixture files are
///       created in reverse order.
/// WHY:  route conflicts are discovered by the eager Stage 0 prepass, which must pick and label a
///       conflict deterministically rather than follow filesystem discovery order.
#[test]
fn namespaced_route_diagnostics_are_deterministic_across_creation_order() {
    let files = [
        ("src/@entry.moth", "[:entry]\n"),
        (
            "src/alpha/@mod.moth",
            "$export @forms\nvalue #= 1\n$export @Forms\nother #= 2\n",
        ),
        (
            "src/zeta/@mod.moth",
            "$export @cards\nvalue #= 1\n$export @Cards\nother #= 2\n",
        ),
    ];
    let mut reversed = files.to_vec();
    reversed.reverse();

    let diagnosed_reports =
        |files: &[(&str, &str)]| match compile_fixture_result(files, false, BuildProfile::Dev) {
            Ok(CompilationOutcome::Diagnosed(reports)) => reports,
            Ok(CompilationOutcome::Success { .. }) => {
                panic!("a case-conflicting public route must be rejected")
            }
            Err(errors) => {
                panic!("source route conflicts must not reach infrastructure errors: {errors:?}")
            }
        };

    let first = diagnosed_reports(&files);
    let second = diagnosed_reports(&reversed);
    let repeat = diagnosed_reports(&files);
    assert!(
        first
            .iter_severity(DiagnosticSeverity::Error)
            .any(|(_, view)| view.descriptor().code == "MOTH-RULE-0092"),
        "the fixture must retain the typed case-conflict diagnostic"
    );
    assert_eq!(
        first.counts(),
        second.counts(),
        "reversed creation order must retain identical diagnostic counts"
    );
    assert_eq!(
        first.counts(),
        repeat.counts(),
        "a repeated compilation must retain identical diagnostic counts"
    );
    let first_rendered = terse::format_terse_diagnostic_reports(&first);
    assert_eq!(
        first_rendered,
        terse::format_terse_diagnostic_reports(&second),
        "canonical diagnostic text must not depend on source creation order"
    );
    assert_eq!(
        first_rendered,
        terse::format_terse_diagnostic_reports(&repeat),
        "canonical diagnostic text must repeat across compilations"
    );
    assert!(
        first
            .iter_severity(DiagnosticSeverity::Error)
            .all(|(_, view)| view.descriptor().code.starts_with("MOTH-")),
        "every route-conflict failure must be a typed user diagnostic"
    );
}
