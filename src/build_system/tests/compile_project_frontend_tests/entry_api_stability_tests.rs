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
    let style_directives = StyleDirectiveRegistry::built_ins();
    let mut warning_owner = BoundaryAccumulator::with_root_string_table(StringTable::new());
    successful_compilation_value(compile_project_frontend(
        &mut config,
        BuildProfile::Dev,
        None,
        &style_directives,
        &mut BuilderSurface::with_mandatory_core(),
        &mut warning_owner,
    ))
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
