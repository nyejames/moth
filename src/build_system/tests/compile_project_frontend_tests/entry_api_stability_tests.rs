use super::*;
use crate::build_system::create_project_modules::compiled_boundary::ProjectFrontendCompilation;
use crate::build_system::create_project_modules::create_project_modules_tests::successful_compilation_value;
use crate::compiler_frontend::compiler_messages::diagnostics::boundary_accumulator::BoundaryAccumulator;
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::hir::reachability::collect_reachability_from_function_link_facts;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::module_compilation::CompiledModuleArtifact;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;

const CONFIG_SOURCE: &str = "project #= (\n    name = \"docs\",\n)\nhtml #= ()\n";

const BASE_HELPER: &str = "compute |value I32| -> I32:\n    return value\n;\n";

const BASE_FACADE: &str = "@impl compute\n\nexport:\n    @impl compute\n;\n";

const SUPPORT_FACADE: &str =
    "export:\n    support_label || -> String:\n        return \"support\"\n    ;\n;\n";

/// Compiles one directory project from the given `(relative path, source text)` files.
fn compile_fixture(files: &[(&str, &str)]) -> ProjectFrontendCompilation {
    successful_compilation_value(compile_fixture_result(files, false))
}

/// Compiles one `.mtf`-bearing directory project through the real HTML builder surface.
///
/// WHY: the HTML builder registers `MothTemplate` source kinds. A fixture that uses `.mtf`
///      content must compile through the same surface a real HTML project uses, not a bare
///      mandatory-core surface that would reject the template source.
fn compile_template_fixture(files: &[(&str, &str)]) -> ProjectFrontendCompilation {
    successful_compilation_value(compile_fixture_result(files, true))
}

/// Freezes the diagnostics of a `.mtf`-bearing project that must be rejected as user source.
fn compile_template_fixture_reports(
    files: &[(&str, &str)],
) -> crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet {
    match compile_fixture_result(files, true) {
        Ok(CompilationOutcome::Diagnosed(reports)) => reports,
        Ok(CompilationOutcome::Success { .. }) => {
            panic!("expected qualified-reference diagnostics")
        }
        Err(errors) => {
            panic!("user-authored qualified input must not reach infrastructure errors: {errors:?}")
        }
    }
}

fn compile_fixture_result(
    files: &[(&str, &str)],
    template_surface: bool,
) -> CompilationResult<ProjectFrontendCompilation> {
    let temporary_directory = tempfile::tempdir().expect("should create temporary project");
    let project_root = temporary_directory.path().to_path_buf();
    fs::create_dir_all(project_root.join("src")).expect("should create project source root");
    fs::write(project_root.join("config.moth"), CONFIG_SOURCE)
        .expect("should write project config");
    for (relative_path, source) in files {
        let path = project_root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("should create fixture source directory");
        }
        fs::write(path, source).expect("should write fixture source");
    }

    let mut config = Config::new(project_root);
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    if template_surface {
        let builder =
            crate::projects::html_project::html_project_builder::HtmlProjectBuilder::new();
        let style_directives = StyleDirectiveRegistry::merged(&builder.frontend_style_directives())
            .expect("HTML style directives should merge");
        let mut frontend_surface = builder.frontend_surface();
        compile_project_frontend(
            &mut config,
            BuildProfile::Dev,
            None,
            &style_directives,
            &mut frontend_surface,
            &mut warning_owner,
        )
    } else {
        let style_directives = StyleDirectiveRegistry::built_ins();
        compile_project_frontend(
            &mut config,
            BuildProfile::Dev,
            None,
            &style_directives,
            &mut BuilderSurface::with_mandatory_core(),
            &mut warning_owner,
        )
    }
}

/// Returns the single project module artefact for a one-module fixture.
fn only_project_artifact(frontend: &ProjectFrontendCompilation) -> &CompiledModuleArtifact {
    let mut artifacts = frontend.project.successful_artefacts_in_module_id_order();
    let artifact = artifacts
        .next()
        .expect("the fixture should compile one project module");
    assert!(
        artifacts.next().is_none(),
        "the fixture should not publish extra project modules"
    );
    artifact
}

/// The accepted public-interface fingerprint content excludes source locations, so diagnostic
/// provenance is not part of the invalidation decision. Everything else is the complete
/// canonical interface.
fn interface_fingerprint_content(interface: &PublicSemanticInterface) -> PublicSemanticInterface {
    let mut canonical = interface.clone();
    canonical.export_diagnostic_provenance.clear();
    canonical
}

/// Returns the artifact that owns the fixture's single entry root.
fn entry_artifact(frontend: &ProjectFrontendCompilation) -> &CompiledModuleArtifact {
    let mut entries = frontend
        .project
        .successful_artefacts_in_module_id_order()
        .filter(|artifact| artifact.module.metadata.entry.is_some());
    let artifact = entries
        .next()
        .expect("the fixture should compile one entry module");
    assert!(
        entries.next().is_none(),
        "the fixture should own exactly one entry root"
    );
    artifact
}

/// Returns the facade interface whose portable module path ends with `suffix`.
fn normal_interface_for<'a>(
    frontend: &'a ProjectFrontendCompilation,
    suffix: &str,
) -> &'a PublicSemanticInterface {
    frontend
        .project
        .successful_artefacts_in_module_id_order()
        .filter_map(|artifact| artifact.normal_interface.as_ref())
        .find(|interface| {
            interface
                .module_origin
                .logical_module_path()
                .ends_with(suffix)
        })
        .unwrap_or_else(|| panic!("expected a normal facade ending with {suffix:?}"))
}

/// Returns every fully folded textual const fragment of the entry root in authored order.
///
/// WHY: a qualified reference must fold before backend lowering, so the retained const fragment is
///      the earliest observable dependent content. A resource-bearing value has no text until the
///      builder assigns URLs and is rejected here because these fixtures are fully textual.
fn entry_folded_fragments(artifact: &CompiledModuleArtifact) -> Vec<String> {
    artifact
        .module
        .metadata
        .entry
        .as_ref()
        .expect("the fixture owns an entry root")
        .const_top_level_fragments
        .iter()
        .map(|fragment| {
            fragment
                .value
                .clone()
                .into_text()
                .expect("the fixture's const fragments must be fully textual")
        })
        .collect()
}

/// Counts runtime-fragment pushes in the entry start function body.
fn entry_runtime_fragment_pushes(artifact: &CompiledModuleArtifact) -> usize {
    let hir = &artifact.module.executable.hir;
    let start_id = hir
        .start_function
        .expect("the entry root must synthesize a start function");
    let start = hir
        .functions
        .iter()
        .find(|function| function.id == start_id)
        .expect("the start function must exist in HIR");
    let entry_block = hir
        .blocks
        .iter()
        .find(|block| block.id == start.entry)
        .expect("the start function entry block must exist in HIR");
    entry_block
        .statements
        .iter()
        .filter(|statement| matches!(statement.kind, HirStatementKind::PushRuntimeFragment { .. }))
        .count()
}

/// Returns the external functions reachable through the published link facts.
fn reachable_external_functions(
    artifact: &CompiledModuleArtifact,
) -> rustc_hash::FxHashSet<ExternalFunctionId> {
    let hir = &artifact.module.executable.hir;
    let all_functions = hir
        .functions
        .iter()
        .map(|function| function.id)
        .collect::<Vec<_>>();
    collect_reachability_from_function_link_facts(
        &artifact.module.link_facts.functions,
        &all_functions,
    )
    .expect("published link facts should cover every HIR function")
    .reachable_external_functions
}

/// WHAT: changing only `@entry.moth` implementation leaves both facade interfaces unchanged.
/// WHY:  the accepted public-interface fingerprint hashes the canonical facade interface. Entry
///       source text, private entry declarations and entry runtime work are not public facts, so an
///       entry edit must not invalidate normal or support API consumers.
#[test]
fn entry_implementation_edits_preserve_facade_interface_fingerprints() {
    let entry_before = "@impl compute\n\nresult = compute(1)\n[1]\n";
    let entry_after = "@impl compute\n\nresult = compute(2)\nio.line(\"entry implementation changed\")\n[2]\n[3]\n";

    let before = compile_fixture(&[
        ("src/@mod.moth", BASE_FACADE),
        ("src/@package.moth", SUPPORT_FACADE),
        ("src/@entry.moth", entry_before),
        ("src/impl.moth", BASE_HELPER),
    ]);
    let after = compile_fixture(&[
        ("src/@mod.moth", BASE_FACADE),
        ("src/@package.moth", SUPPORT_FACADE),
        ("src/@entry.moth", entry_after),
        ("src/impl.moth", BASE_HELPER),
    ]);

    let before_artifact = only_project_artifact(&before);
    let after_artifact = only_project_artifact(&after);

    let before_normal = before_artifact
        .normal_interface
        .as_ref()
        .expect("the fixture must publish its normal facade interface");
    let after_normal = after_artifact
        .normal_interface
        .as_ref()
        .expect("the edited fixture must publish its normal facade interface");
    let before_support = before_artifact
        .support_interface
        .as_ref()
        .expect("the fixture must publish its support facade interface");
    let after_support = after_artifact
        .support_interface
        .as_ref()
        .expect("the edited fixture must publish its support facade interface");

    assert_eq!(
        interface_fingerprint_content(before_normal),
        interface_fingerprint_content(after_normal),
        "an entry-only implementation edit must not change the normal facade interface"
    );
    assert_eq!(
        interface_fingerprint_content(before_support),
        interface_fingerprint_content(after_support),
        "an entry-only implementation edit must not change the support facade interface"
    );

    let before_entry = before_artifact
        .module
        .metadata
        .entry
        .as_ref()
        .expect("the fixture owns an entry root");
    let after_entry = after_artifact
        .module
        .metadata
        .entry
        .as_ref()
        .expect("the fixture owns an entry root");

    assert_eq!(
        before_entry.entry_point.file_name(),
        after_entry.entry_point.file_name()
    );
    assert_eq!(
        before_entry.root_activity.runtime_fragment_count, 1,
        "the unchanged entry keeps one runtime fragment"
    );
    assert_eq!(
        after_entry.root_activity.runtime_fragment_count, 2,
        "the edited entry adds a second runtime fragment"
    );
    assert_eq!(entry_runtime_fragment_pushes(before_artifact), 1);
    assert_eq!(entry_runtime_fragment_pushes(after_artifact), 2);
    assert!(
        !reachable_external_functions(before_artifact).contains(&ExternalFunctionId::IoLine),
        "the unchanged entry has no io.line demand"
    );
    assert!(
        reachable_external_functions(after_artifact).contains(&ExternalFunctionId::IoLine),
        "the edited entry's io.line call must be a link-fact demand"
    );
}

/// WHAT: changing a shared helper's exported surface changes the provider interface, while a
///       private helper body edit does not.
/// WHY:  consumers rebuild only when the provider public-interface fingerprint changes. The
///       exported helper signature and effects are public facts; its private body is implementation
///       alone.
#[test]
fn shared_helper_export_changes_invalidate_the_provider_interface_fingerprint() {
    let base_entry = "@impl compute\n\nresult = compute(1)\n[1]\n";
    let base = compile_fixture(&[
        ("src/@mod.moth", BASE_FACADE),
        ("src/@entry.moth", base_entry),
        ("src/impl.moth", BASE_HELPER),
    ]);

    let private_body_change = "compute |value I32| -> I32:\n    if value > 0:\n        return value\n    ;\n    return value\n;\n";
    let private_change = compile_fixture(&[
        ("src/@mod.moth", BASE_FACADE),
        ("src/@entry.moth", base_entry),
        ("src/impl.moth", private_body_change),
    ]);

    let exported_surface_change = "compute |text String| -> String:\n    return text\n;\n";
    let exported_change = compile_fixture(&[
        ("src/@mod.moth", BASE_FACADE),
        (
            "src/@entry.moth",
            "@impl compute\n\nresult = compute(\"changed\")\n[1]\n",
        ),
        ("src/impl.moth", exported_surface_change),
    ]);

    let base_interface = interface_fingerprint_content(
        only_project_artifact(&base)
            .normal_interface
            .as_ref()
            .expect("the base fixture must publish its normal facade interface"),
    );
    let private_interface = interface_fingerprint_content(
        only_project_artifact(&private_change)
            .normal_interface
            .as_ref()
            .expect("the private-body fixture must publish its normal facade interface"),
    );
    let exported_interface = interface_fingerprint_content(
        only_project_artifact(&exported_change)
            .normal_interface
            .as_ref()
            .expect("the exported-signature fixture must publish its normal facade interface"),
    );

    assert_eq!(
        base_interface, private_interface,
        "a private shared-helper body edit must not change the provider public interface"
    );
    assert_ne!(
        base_interface, exported_interface,
        "an exported shared-helper signature change must change the provider public interface"
    );
}

/// WHAT: editing the private helper constant selected by `[@settings site_name]` inside a `.mtf`
///       body changes the entry's folded content and nothing else in the public surface.
/// WHY:  qualified references are ordinary declaration dependencies. A private helper value is an
///       implementation fact of its own module, so dependants that folded it must observe the new
///       content, while facades that never selected it keep byte-identical interfaces.
#[test]
fn qualified_private_helper_edit_changes_only_the_dependent_folded_content() {
    let before = compile_template_fixture(&[
        ("src/@mod.moth", "export:\n    unrelated #= 1\n;\n"),
        ("src/@entry.moth", "#[: [@intro content] ]\n"),
        ("src/intro.mtf", "[@settings site_name]\n"),
        ("src/settings.moth", "site_name #= \"Moth Site\"\n"),
        ("src/other/@mod.moth", "export:\n    stable #= 1\n;\n"),
    ]);
    let after = compile_template_fixture(&[
        ("src/@entry.moth", "#[: [@intro content] ]\n"),
        ("src/settings.moth", "site_name #= \"Changed Site\"\n"),
        ("src/intro.mtf", "[@settings site_name]\n"),
        ("src/other/@mod.moth", "export:\n    stable #= 1\n;\n"),
        ("src/@mod.moth", "export:\n    unrelated #= 1\n;\n"),
    ]);

    let before_content = entry_folded_fragments(entry_artifact(&before));
    let after_content = entry_folded_fragments(entry_artifact(&after));
    assert!(
        before_content
            .iter()
            .any(|content| content.contains("Moth Site")),
        "the unchanged helper must fold into the dependent template content: {before_content:?}"
    );
    assert!(
        after_content
            .iter()
            .any(|content| content.contains("Changed Site")),
        "editing the private helper must change the dependent folded content: {after_content:?}"
    );
    assert!(
        after_content
            .iter()
            .all(|content| !content.contains("Moth Site")),
        "the previous helper value must not survive in the rebuilt content: {after_content:?}"
    );

    assert_eq!(
        interface_fingerprint_content(normal_interface_for(&before, "other")),
        interface_fingerprint_content(normal_interface_for(&after, "other")),
        "an unrelated module interface must not change when the helper target changes"
    );
    assert_eq!(
        interface_fingerprint_content(
            entry_artifact(&before)
                .normal_interface
                .as_ref()
                .expect("the fixture publishes its root facade")
        ),
        interface_fingerprint_content(
            entry_artifact(&after)
                .normal_interface
                .as_ref()
                .expect("the fixture publishes its root facade")
        ),
        "the root facade exports no selected fact and must keep its fingerprint"
    );
}

/// WHAT: editing an exported constant selected cross-module changes the consumer's folded
///       implementation fact and the provider interface, while unrelated interfaces stay equal.
/// WHY:  a qualified reference must invalidate the consumer that folded the provider constant and
///       the provider's public constant fact, but no module outside that dependency edge. A stale
///       or over-broad fingerprint comparison would either miss the edit or invalidate everything.
#[test]
fn qualified_cross_module_constant_edit_changes_the_consumer_and_provider_only() {
    let before = compile_template_fixture(&[
        ("src/@mod.moth", "export:\n    unrelated #= 1\n;\n"),
        (
            "src/@entry.moth",
            "message #= @child label\n#[: [message] ]\n",
        ),
        ("src/child/@mod.moth", "export:\n    label #= \"one\"\n;\n"),
        ("src/other/@mod.moth", "export:\n    stable #= 1\n;\n"),
    ]);
    let after = compile_template_fixture(&[
        ("src/@mod.moth", "export:\n    unrelated #= 1\n;\n"),
        (
            "src/@entry.moth",
            "message #= @child label\n#[: [message] ]\n",
        ),
        ("src/child/@mod.moth", "export:\n    label #= \"two\"\n;\n"),
        ("src/other/@mod.moth", "export:\n    stable #= 1\n;\n"),
    ]);

    let before_content = entry_folded_fragments(entry_artifact(&before));
    let after_content = entry_folded_fragments(entry_artifact(&after));
    assert!(
        before_content.iter().any(|content| content.contains("one")),
        "the consumer folds the current exported constant: {before_content:?}"
    );
    assert!(
        after_content.iter().any(|content| content.contains("two")),
        "the consumer must fold the edited exported constant: {after_content:?}"
    );
    assert!(
        after_content.iter().all(|content| !content.contains("one")),
        "the edited constant must not leave the old consumer implementation fact: {after_content:?}"
    );

    assert_ne!(
        interface_fingerprint_content(normal_interface_for(&before, "child")),
        interface_fingerprint_content(normal_interface_for(&after, "child")),
        "the provider's exported constant value is a public interface fact"
    );
    assert_eq!(
        interface_fingerprint_content(
            entry_artifact(&before)
                .normal_interface
                .as_ref()
                .expect("the fixture publishes its root facade")
        ),
        interface_fingerprint_content(
            entry_artifact(&after)
                .normal_interface
                .as_ref()
                .expect("the fixture publishes its root facade")
        ),
        "the root facade is outside the dependency edge and must stay equal"
    );
    assert_eq!(
        interface_fingerprint_content(normal_interface_for(&before, "other")),
        interface_fingerprint_content(normal_interface_for(&after, "other")),
        "an unrelated module interface must stay equal"
    );
}

/// WHAT: two compilations of the same qualified-reference project produce identical module
///       identities, facade interfaces and folded content, including when fixture files are
///       created in reverse order.
/// WHY:  Stage 0 provider preparation and Stage 3 declaration ordering must not depend on filesystem
///       enumeration or source registration order. Visible drift would make rebuilds and artifact
///       caches unstable even though the authored project is identical.
#[test]
fn qualified_reference_builds_are_deterministic_across_source_creation_order() {
    let files = [
        ("src/@mod.moth", "export:\n    unrelated #= 1\n;\n"),
        ("src/@entry.moth", "page #= @child intro\n#[: [page] ]\n"),
        (
            "src/child/@mod.moth",
            "export:\n    @intro content as intro\n;\n",
        ),
        ("src/child/intro.mtf", "[@settings site_name]\n"),
        ("src/child/settings.moth", "site_name #= \"stable\"\n"),
        ("src/other/@mod.moth", "export:\n    stable #= 1\n;\n"),
    ];
    let mut reversed = files.to_vec();
    reversed.reverse();

    let first = compile_template_fixture(&files);
    let second = compile_template_fixture(&reversed);

    let first_origins: Vec<_> = first
        .project
        .structure
        .nodes()
        .iter()
        .map(|node| node.stable_origin().clone())
        .collect();
    let second_origins: Vec<_> = second
        .project
        .structure
        .nodes()
        .iter()
        .map(|node| node.stable_origin().clone())
        .collect();
    assert_eq!(
        first_origins, second_origins,
        "module graph identities must not depend on source creation order"
    );

    let first_artifacts: Vec<_> = first
        .project
        .successful_artefacts_in_module_id_order()
        .collect();
    let second_artifacts: Vec<_> = second
        .project
        .successful_artefacts_in_module_id_order()
        .collect();
    assert_eq!(
        first_artifacts.len(),
        second_artifacts.len(),
        "the same project must publish the same module count"
    );
    for (first_artifact, second_artifact) in first_artifacts.iter().zip(&second_artifacts) {
        assert_eq!(
            first_artifact
                .normal_interface
                .as_ref()
                .map(interface_fingerprint_content),
            second_artifact
                .normal_interface
                .as_ref()
                .map(interface_fingerprint_content),
            "normal facade identities must repeat exactly"
        );
        assert_eq!(
            first_artifact
                .support_interface
                .as_ref()
                .map(interface_fingerprint_content),
            second_artifact
                .support_interface
                .as_ref()
                .map(interface_fingerprint_content),
            "support facade identities must repeat exactly"
        );
    }
    let first_entry = entry_artifact(&first);
    let second_entry = entry_artifact(&second);
    assert_eq!(
        first_entry
            .module
            .metadata
            .entry
            .as_ref()
            .map(|entry| entry.entry_point.file_name()),
        second_entry
            .module
            .metadata
            .entry
            .as_ref()
            .map(|entry| entry.entry_point.file_name()),
        "entry identities must repeat exactly"
    );
    assert_eq!(
        entry_folded_fragments(first_entry),
        entry_folded_fragments(second_entry),
        "folded qualified content must repeat exactly"
    );
    assert_eq!(
        entry_runtime_fragment_pushes(first_entry),
        entry_runtime_fragment_pushes(second_entry),
        "the entry's runtime fragment shape must repeat exactly"
    );
}

/// WHAT: a cyclic qualified `.mtf` dependency reports the same user diagnostics on repeated
///       compilations, with no infrastructure failure and no panic.
/// WHY:  cycle discovery during declaration ordering must be deterministic under repeat builds and
///       must stay in the user-diagnostic failure lane. A `compiler_bug!` or worker-order-dependent
///       report would make the rejection unusable.
#[test]
fn qualified_cycle_diagnostics_are_deterministic_across_repeated_compiles() {
    let files = [
        (
            "src/@entry.moth",
            "@settings title\n[: [@settings title] ]\n",
        ),
        ("src/settings.moth", "title #= @intro.mtf\n"),
        ("src/intro.mtf", "[@settings title]\n"),
    ];
    let mut reversed = files.to_vec();
    reversed.reverse();

    let first = compile_template_fixture_reports(&files);
    let second = compile_template_fixture_reports(&reversed);

    assert!(
        first.counts().errors > 0,
        "the cyclic qualified reference must be a user error"
    );
    assert_eq!(
        first.counts(),
        second.counts(),
        "repeated compilation must retain identical diagnostic counts"
    );
    let first_rendered = terse::format_terse_diagnostic_reports(&first);
    let second_rendered = terse::format_terse_diagnostic_reports(&second);
    assert_eq!(
        first_rendered, second_rendered,
        "canonical diagnostic ordering must not depend on source creation order"
    );
    assert!(
        first
            .iter_severity(DiagnosticSeverity::Error)
            .all(|(_, view)| view.descriptor().code.starts_with("MOTH-")),
        "every qualified-cycle failure must be a typed user diagnostic"
    );
}
