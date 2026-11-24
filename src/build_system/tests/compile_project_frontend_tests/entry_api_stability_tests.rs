use super::*;
use crate::build_system::create_project_modules::compiled_boundary::ProjectFrontendCompilation;
use crate::compiler_frontend::ast::generic_functions::reuse_probe;
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::hir::reachability::collect_reachability_from_function_link_facts;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::utils::{collect_reachable_blocks, for_each_terminator_target};
use crate::compiler_frontend::module_compilation::CompiledModuleArtifact;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::public_interface::{
    PublicDeclarationSemantics, PublicSemanticInterface,
};
use crate::compiler_frontend::semantic_identity::OriginDeclarationId;

const BASE_HELPER: &str = "compute |value I32| -> I32:\n    return value\n;\n";

const BASE_FACADE: &str = "@impl compute\n\n$export\n@impl compute\n";

const SUPPORT_FACADE: &str = "$export\nsupport_label || -> String:\n    return \"support\"\n;\n";

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
pub(super) fn interface_fingerprint_content(
    interface: &PublicSemanticInterface,
) -> PublicSemanticInterface {
    let mut canonical = interface.clone();
    canonical.export_diagnostic_provenance.clear();
    canonical
}

/// Returns the facade interface whose portable module path ends with `suffix`.
pub(super) fn normal_interface_for<'a>(
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

/// Returns the fully folded text of an exported string constant in a normal interface.
///
/// WHY: a `.mtf` file value and a qualified `content` reference must publish the same concrete
///      constant text, not merely compile.
pub(super) fn exported_constant_text(interface: &PublicSemanticInterface, name: &str) -> String {
    let declaration = interface
        .declarations
        .iter()
        .find(|declaration| {
            matches!(
                &declaration.origin,
                OriginDeclarationId::Constant(origin) if origin.defining_name() == name
            )
        })
        .unwrap_or_else(|| panic!("the interface must export constant `{name}`"));
    let PublicDeclarationSemantics::Constant(constant) = &declaration.semantics else {
        panic!("`{name}` must retain constant semantics");
    };
    let PublicFoldedValue::String(value) = &constant.folded_value else {
        panic!(
            "`{name}` must fold to a string, got {:?}",
            constant.folded_value
        );
    };
    value
        .clone()
        .into_text()
        .unwrap_or_else(|| panic!("`{name}` must be fully textual for this fixture"))
}

/// Asserts the entry's complete folded output without accepting extra adjacent values.
fn assert_entry_folded_output(compiled: &ProjectFrontendCompilation, expected: &str) {
    let artifact = entry_artifact(compiled);
    let content = entry_folded_fragments(artifact);
    assert_eq!(
        content.concat(),
        expected,
        "the entry's complete folded output must match: {content:?}"
    );
    assert_eq!(
        entry_runtime_fragment_pushes(artifact),
        0,
        "a fully folded const entry must not retain a runtime fragment push"
    );
}

/// Counts runtime-fragment pushes in every block reachable from the entry start function.
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
    let reachable = collect_reachable_blocks(start.entry, |block_id, visit| {
        let block = hir
            .blocks
            .iter()
            .find(|block| block.id == block_id)
            .ok_or_else(|| format!("reachable block {block_id:?} must exist in HIR"))?;
        for_each_terminator_target(&block.terminator, visit);
        Ok::<_, String>(())
    })
    .expect("the start function's reachable blocks must resolve");
    reachable
        .iter()
        .filter_map(|block_id| hir.blocks.iter().find(|block| block.id == *block_id))
        .flat_map(|block| &block.statements)
        .filter(|statement| matches!(statement.kind, HirStatementKind::PushRuntimeFragment { .. }))
        .count()
}

/// Returns the external functions reachable through the published link facts.
pub(super) fn reachable_external_functions(
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
    let entry_before = "@impl compute\n\nresult = compute(1)\n~[1]\n";
    let entry_after = "@impl compute\n\nresult = compute(2)\nio.line(\"entry implementation changed\")\n~[2]\n~[3]\n";

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
        ("src/@mod.moth", "$export\nunrelated #= 1\n"),
        ("src/@entry.moth", "[: [@intro content] ]\n"),
        ("src/intro.mtf", "[@settings site_name]\n"),
        ("src/settings.moth", "site_name #= \"Moth Site\"\n"),
        ("src/other/@mod.moth", "$export\nstable #= 1\n"),
    ]);
    let after = compile_template_fixture(&[
        ("src/@entry.moth", "[: [@intro content] ]\n"),
        ("src/settings.moth", "site_name #= \"Changed Site\"\n"),
        ("src/intro.mtf", "[@settings site_name]\n"),
        ("src/other/@mod.moth", "$export\nstable #= 1\n"),
        ("src/@mod.moth", "$export\nunrelated #= 1\n"),
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

/// WHAT: under a builder that selects no optional source kinds, a `.mtf` file value and the
///       qualified `content` reference publish the same exact folded text, including a nested
///       `.mtf` body and markup from a bare `@core/html` prelude name.
/// WHY:  `.mtf` support is compiler-owned, and both reference forms must select the one synthetic
///       constant. A compile-only assertion would accept a value that folded to empty or wrong
///       text.
#[test]
fn compiler_supported_mtf_file_value_and_qualified_content_fold_identically() {
    let compiled = compile_fixture(&[
        (
            "src/@entry.moth",
            "@content file_value, qualified\n[: [file_value] [qualified] ]\n",
        ),
        (
            "src/content/@mod.moth",
            "$export\nfile_value #= @intro.mtf\n$export\nqualified #= @intro content\n",
        ),
        (
            "src/content/intro.mtf",
            "intro body [: [p: nested [@nested.mtf] ] ]\n",
        ),
        ("src/content/nested.mtf", "nested body\n"),
    ]);

    let interface = normal_interface_for(&compiled, "content");
    let file_value = exported_constant_text(interface, "file_value");
    let qualified = exported_constant_text(interface, "qualified");
    assert_eq!(
        file_value, qualified,
        "an explicit `.mtf` file value and `@intro content` must select the same synthetic constant"
    );
    assert_eq!(
        file_value, "<p>intro body <p style=\"\"><p> nested <p>nested body</p> </p></p> </p>",
        "the folded content must retain the outer body, the nested template body and `@core/html` markup"
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
        ("src/@mod.moth", "$export\nunrelated #= 1\n"),
        (
            "src/@entry.moth",
            "message #= @child label\n[: [message] ]\n",
        ),
        ("src/child/@mod.moth", "$export\nlabel #= \"one\"\n"),
        ("src/other/@mod.moth", "$export\nstable #= 1\n"),
    ]);
    let after = compile_template_fixture(&[
        ("src/@mod.moth", "$export\nunrelated #= 1\n"),
        (
            "src/@entry.moth",
            "message #= @child label\n[: [message] ]\n",
        ),
        ("src/child/@mod.moth", "$export\nlabel #= \"two\"\n"),
        ("src/other/@mod.moth", "$export\nstable #= 1\n"),
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

/// WHAT: editing an exported const function's private body changes the consumer's folded result
///       while the provider's public callable signature stays fixed.
/// WHY:  a consumer that folded a cross-module const call must re-run the provider closure on the
///       next compilation. The dependency edge is the exported const declaration, not only its
///       parameter and return types.
#[test]
fn const_function_private_body_edit_changes_consumer_folded_content() {
    let entry = "@child answer\nresult #I32 = answer(1)\n[: [result] ]\n";
    let before_provider = "$export\nanswer #|value I32| -> I32:\n    return plus(value)\n;\n\nplus #|value I32| -> I32:\n    return value * 2 + private_increment\n;\n\nprivate_increment #I32 = 1\n";
    let after_provider = "$export\nanswer #|value I32| -> I32:\n    return plus(value) + 10\n;\n\nplus #|value I32| -> I32:\n    return value * 2 + private_increment\n;\n\nprivate_increment #I32 = 1\n";

    let before = compile_template_fixture(&[
        ("src/@mod.moth", "$export\nunrelated #= 1\n"),
        ("src/@entry.moth", entry),
        ("src/child/@mod.moth", before_provider),
        ("src/other/@mod.moth", "$export\nstable #= 1\n"),
    ]);
    let after = compile_template_fixture(&[
        ("src/@mod.moth", "$export\nunrelated #= 1\n"),
        ("src/@entry.moth", entry),
        ("src/child/@mod.moth", after_provider),
        ("src/other/@mod.moth", "$export\nstable #= 1\n"),
    ]);

    assert_entry_folded_output(&before, " 3 ");
    assert_entry_folded_output(&after, " 13 ");
    assert_eq!(
        interface_fingerprint_content(normal_interface_for(&before, "other")),
        interface_fingerprint_content(normal_interface_for(&after, "other")),
        "an unrelated module interface must stay equal"
    );
}

/// WHAT: editing only a private const helper body reachable from an exported const function
///       changes the consumer's folded result while the exported body and private constant stay
///       fixed.
/// WHY:  the private helper is part of the consumer dependency closure. A regression that only
///       invalidates direct provider bodies would leave the stale folded result in place.
#[test]
fn const_function_private_helper_body_edit_changes_consumer_folded_content() {
    let entry = "@child answer\nresult #I32 = answer(1)\n[: [result] ]\n";
    let provider = |helper_body: &str| {
        format!(
            "$export\nanswer #|value I32| -> I32:\n    return plus(value)\n;\n\nplus #|value I32| -> I32:\n    {helper_body}\n;\n\nprivate_increment #I32 = 1\n"
        )
    };
    let before = compile_template_fixture(&[
        ("src/@entry.moth", entry),
        (
            "src/child/@mod.moth",
            &provider("return value * 2 + private_increment"),
        ),
        ("src/other/@mod.moth", "$export\nstable #= 1\n"),
    ]);
    let after = compile_template_fixture(&[
        ("src/@entry.moth", entry),
        (
            "src/child/@mod.moth",
            &provider("return value * 3 + private_increment"),
        ),
        ("src/other/@mod.moth", "$export\nstable #= 1\n"),
    ]);

    assert_entry_folded_output(&before, " 3 ");
    assert_entry_folded_output(&after, " 4 ");
}

/// WHAT: editing only a private constant reachable from an exported const body changes the
///       consumer's folded result while the provider's public callable signature stays fixed.
/// WHY:  the transitive private constant is part of the consumer dependency edge even though it is
///       not importable. A stale consumer result would prove the edge stops at the exported body.
#[test]
fn const_function_private_constant_edit_changes_consumer_folded_content() {
    let entry = "@child answer\nresult #I32 = answer(1)\n[: [result] ]\n";
    let provider = |increment: &str| {
        format!(
            "$export\nanswer #|value I32| -> I32:\n    return plus(value)\n;\n\nplus #|value I32| -> I32:\n    return value * 2 + private_increment\n;\n\nprivate_increment #I32 = {increment}\n"
        )
    };
    let before = compile_template_fixture(&[
        ("src/@entry.moth", entry),
        ("src/child/@mod.moth", &provider("1")),
        ("src/other/@mod.moth", "$export\nstable #= 1\n"),
    ]);
    let after = compile_template_fixture(&[
        ("src/@entry.moth", entry),
        ("src/child/@mod.moth", &provider("9")),
        ("src/other/@mod.moth", "$export\nstable #= 1\n"),
    ]);

    assert_entry_folded_output(&before, " 3 ");
    assert_entry_folded_output(&after, " 11 ");
    assert_eq!(
        interface_fingerprint_content(normal_interface_for(&before, "other")),
        interface_fingerprint_content(normal_interface_for(&after, "other")),
        "an unrelated module interface must stay equal"
    );
}

/// WHAT: importing an exported const function does not make its retained private helper
///       importable as ordinary source.
/// WHY:  the executable closure carries the helper for execution, but publication must not widen
///       the provider's source visibility. A successful import would expose provider internals.
#[test]
fn private_const_helper_closure_is_not_an_importable_declaration() {
    let reports = compile_template_fixture_reports(&[
        (
            "src/@entry.moth",
            "@helper private_double\nresult #I32 = private_double(2)\n[: [result] ]\n",
        ),
        ("src/helper/@mod.moth", "$export\n@internal public_double\n"),
        (
            "src/helper/internal.moth",
            "private_double #|value I32| -> I32:\n    return value * 2\n;\n\npublic_double #|value I32| -> I32:\n    return private_double(value) + 1\n;\n",
        ),
    ]);

    assert!(
        reports.counts().errors > 0,
        "a private helper must not be importable through its provider module"
    );
    let rendered = terse::format_terse_diagnostic_reports(&reports);
    assert!(
        rendered.iter().any(|line| line.contains("private_double")),
        "the visibility diagnostic must name the rejected private helper: {rendered:?}"
    );
    assert!(
        reports
            .iter_severity(DiagnosticSeverity::Error)
            .all(|(_, view)| view.descriptor().code.starts_with("MOTH-")),
        "the rejected private import must stay in the user-diagnostic lane"
    );
}

/// WHAT: two local aliases of the same imported const function prepare one provider body.
/// WHY:  materialisation is keyed by provider identity, so repeated imports and repeated calls must
///       not reparse the provider body or duplicate its declaration. Preparation is observed per
///       registered row, so a row outside the imported closure stays visible as absent, and the
///       exact folded output rejects an alias that folds the wrong value or shares an output slot.
#[test]
fn repeated_import_aliases_install_one_provider_const_closure() {
    let entry = "@helper phase5_provider_reuse_marker\n@helper phase5_provider_reuse_marker as phase5_alias\n\nfirst #I32 = phase5_provider_reuse_marker(20)\nsecond #I32 = phase5_alias(2)\n\n[: first=[first] second=[second] ]\n";
    let provider_module = "$export\n@internal phase5_provider_reuse_marker\n@internal phase5_unused_provider_marker\n";
    let provider_body = "private_increment #I32 = 1\n\nphase5_provider_reuse_marker #|value I32| -> I32:\n    return value + private_increment\n;\n\nphase5_unused_provider_marker #|value I32| -> I32:\n    return value * 99\n;\n";

    let compiled = compile_template_fixture(&[
        ("src/@entry.moth", entry),
        ("src/helper/@mod.moth", provider_module),
        ("src/helper/internal.moth", provider_body),
    ]);

    assert_entry_folded_output(&compiled, " first=21 second=3 ");
    assert_eq!(
        reuse_probe::preparations("phase5_provider_reuse_marker"),
        1,
        "one provider origin must prepare its body exactly once"
    );
    assert_eq!(
        reuse_probe::preparations("phase5_unused_provider_marker"),
        0,
        "an unrelated internal provider body must stay outside the imported closure"
    );
    assert_eq!(
        reuse_probe::rebases("phase5_provider_reuse_marker"),
        1,
        "the second import alias must reuse the completed origin before context rebase"
    );
    assert_eq!(
        reuse_probe::selected_rows("phase5_provider_reuse_marker"),
        1,
        "one cold origin materialisation must select only the reachable entry row"
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
        ("src/@mod.moth", "$export\nunrelated #= 1\n"),
        ("src/@entry.moth", "page #= @child intro\n[: [page] ]\n"),
        ("src/child/@mod.moth", "$export\n@intro content as intro\n"),
        ("src/child/intro.mtf", "[@settings site_name]\n"),
        ("src/child/settings.moth", "site_name #= \"stable\"\n"),
        ("src/other/@mod.moth", "$export\nstable #= 1\n"),
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

/// Returns the published call summary for one free exported function.
fn exported_summary<'a>(
    interface: &'a PublicSemanticInterface,
    name: &str,
) -> &'a PublicCallSummary {
    &interface
        .concrete_call_summaries
        .iter()
        .find(|record| record.origin.defining_name() == name && record.origin.receiver().is_none())
        .unwrap_or_else(|| panic!("the interface must publish a summary for `{name}`"))
        .summary
}

/// Compiles one exported callable whose private helper body is the only changing source fact.
fn compile_exported_callable_with_helper(helper_body: &str) -> ProjectFrontendCompilation {
    let entry = "@impl compute\n\nresult = compute(1) catch:\n    then 0\n;\n~[result]\n";
    let provider = format!(
        "compute |value I32| -> I32, Error!:\n    result = helper(value)!\n    return result\n;\n\nhelper |value I32| -> I32, Error!:\n{helper_body}\n;\n"
    );
    compile_fixture(&[
        ("src/@mod.moth", BASE_FACADE),
        ("src/@entry.moth", entry),
        ("src/impl.moth", &provider),
    ])
}

/// WHAT: an exported callable's private helper becoming fast, trapping or escaping changes the
///       provider public interface even though the exported signature is byte-identical.
/// WHY:  interface equality is the cache invalidation key. If a private helper's constraint bits
///       did not ride the public call summary, a cached consumer would keep the stale guarantee.
#[test]
fn private_helper_constraint_changes_invalidate_the_provider_interface_fingerprint() {
    let baseline = compile_exported_callable_with_helper("    return value");
    let fast = compile_exported_callable_with_helper("    $fast_math\n    return value + 1");
    let trapping = compile_exported_callable_with_helper("    assert(value > 0)\n    return value");
    let escaping = compile_exported_callable_with_helper("    return! Error(\"no value\")");

    let baseline_interface = interface_fingerprint_content(
        only_project_artifact(&baseline)
            .normal_interface
            .as_ref()
            .expect("the fixture must publish its normal facade interface"),
    );
    let baseline_summary = exported_summary(&baseline_interface, "compute");
    assert!(
        !baseline_summary.uses_fast_semantics
            && !baseline_summary.may_trap
            && !baseline_summary.may_fail,
        "an ordinary total helper must leave every constraint bit false"
    );

    for (name, compiled, bit) in [
        (
            "fast",
            &fast,
            (|summary: &PublicCallSummary| summary.uses_fast_semantics)
                as fn(&PublicCallSummary) -> bool,
        ),
        ("trapping", &trapping, |summary: &PublicCallSummary| {
            summary.may_trap
        }),
        ("escaping", &escaping, |summary: &PublicCallSummary| {
            summary.may_fail
        }),
    ] {
        let interface = interface_fingerprint_content(
            only_project_artifact(compiled)
                .normal_interface
                .as_ref()
                .expect("the variant fixture must publish its normal facade interface"),
        );
        let summary = exported_summary(&interface, "compute");
        assert!(
            bit(summary),
            "the {name} private helper must widen the exported callable's matching bit"
        );
        assert_ne!(
            baseline_interface, interface,
            "a {name} private helper must invalidate the provider public interface"
        );
    }
}

/// WHAT: a receiver method on an exported struct publishes its constraint bits.
/// WHY:  a method is not independently exported, so its only public evidence is the summary
///       attached to the exported receiver declaration. Dropping it would leave method consumers
///       without an invalidation fact.
#[test]
fn public_receiver_method_constraint_bits_are_published() {
    let files = [
        (
            "src/@entry.moth",
            "@child Box\n\nbox = Box(1)\n~[: [box.peek()] ]\n",
        ),
        (
            "src/child/@mod.moth",
            "$export\nBox = |\n    value I32,\n|\n\n$fast_math\npeek |this Box| -> I32:\n    return this.value + 1\n;\n",
        ),
    ];
    let compiled = compile_fixture(&files);
    let interface = interface_fingerprint_content(normal_interface_for(&compiled, "child"));
    let method_summary = &interface
        .concrete_call_summaries
        .iter()
        .find(|record| {
            record.origin.defining_name() == "peek" && record.origin.receiver().is_some()
        })
        .expect("an exported struct must publish its receiver method summary")
        .summary;
    assert!(
        method_summary.uses_fast_semantics,
        "the method's selected fast operation must publish through its receiver declaration"
    );
    assert!(
        !method_summary.may_trap && !method_summary.may_fail,
        "the ordinary method body must not publish trap or failure evidence"
    );
}
