use super::*;
use crate::build_system::create_project_modules::compiled_boundary::ProjectFrontendCompilation;
use crate::compiler_frontend::ast::generic_functions::reuse_probe;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext;
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::hir::reachability::collect_reachability_from_function_link_facts;
use crate::compiler_frontend::hir::statements::HirStatementKind;
use crate::compiler_frontend::hir::utils::{collect_reachable_blocks, for_each_terminator_target};
use crate::compiler_frontend::module_compilation::CompiledModuleArtifact;
use crate::compiler_frontend::public_call_summary::PublicCallSummary;
use crate::compiler_frontend::public_interface::{
    PublicDeclarationSemantics, PublicFunctionCategory, PublicSemanticInterface,
};
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginDeclarationId, OriginTypeCategory,
};
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceClass;

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
fn interface_fingerprint_content(interface: &PublicSemanticInterface) -> PublicSemanticInterface {
    let mut canonical = interface.clone();
    canonical.export_diagnostic_provenance.clear();
    canonical
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

/// Returns the fully folded text of an exported string constant in a normal interface.
///
/// WHY: a `.mtf` file value and a qualified `content` reference must publish the same concrete
///      constant text, not merely compile.
fn exported_constant_text(interface: &PublicSemanticInterface, name: &str) -> String {
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
/// Whether the entry root still reaches a direct ProjectContext dependency in surviving HIR.
///
/// WHAT: collects the published link facts from the synthetic start function, falling back to
///       every HIR function only when a fixture has no start root.
/// WHY:  a release replacement must preserve authored project/config provenance in the actual
///       HIR, not merely keep a call count or an AST-side fact.
fn reachable_project_context(artifact: &CompiledModuleArtifact) -> bool {
    let hir = &artifact.module.executable.hir;
    let roots = match hir.start_function {
        Some(start) => vec![start],
        None => hir.functions.iter().map(|function| function.id).collect(),
    };
    collect_reachability_from_function_link_facts(&artifact.module.link_facts.functions, &roots)
        .expect("published link facts should cover every HIR function")
        .reachable_function_provenance()
        .contains_class(SyntheticInterfaceClass::ProjectContext)
}

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

/// WHAT: release folds a pure ordinary call with constant arguments, while debug keeps it and
///       impure or parameter-dependent bodies stay runtime in both profiles.
/// WHY:  the speculative rewrite must reduce HIR calls without changing observable runtime work
///       or accepting a body whose arguments or effects are not proven constant and pure.
#[test]
fn release_folds_only_proven_pure_ordinary_calls() {
    let files = [
        (
            "src/@entry.moth",
            "double |value I32| -> I32:\n    return value * 2\n;\n\ndynamic |value I32| -> I32:\n    return double(value)\n;\n\neffect |value I32| -> I32:\n    io.line(\"effect\")\n    return value\n;\n\nmutate |value ~I32| -> I32:\n    ~value = value + 1\n    return value\n;\n\nlocal_mutate |value I32| -> I32:\n    local ~I32 = 0\n    ~local = value + 1\n    return local\n;\n\ncounter ~= 2\n\n~[: [double(2)] [dynamic(3)] [local_mutate(5)] [effect(4)] [mutate(~counter)] ]\n",
        ),
        ("src/@config.moth", CONFIG_SOURCE),
    ];
    let debug = compile_template_fixture_with_profile(&files, BuildProfile::Dev);
    let release = compile_template_fixture_with_profile(&files, BuildProfile::Release);

    let debug_calls = entry_hir_local_call_count(entry_artifact(&debug));
    let release_calls = entry_hir_local_call_count(entry_artifact(&release));
    assert_eq!(
        release_calls + 3,
        debug_calls,
        "release must fold the direct call, its transitive pure callee and a pure local mutation: debug={debug_calls} release={release_calls}"
    );
    assert!(
        release_calls > 0,
        "parameter-dependent and impure calls must remain in release HIR"
    );
    assert!(
        !entry_folded_fragments(entry_artifact(&release)).is_empty()
            || entry_runtime_fragment_pushes(entry_artifact(&release)) > 0,
        "release must still emit the entry template output path"
    );
}

/// WHAT: a mutually recursive ordinary pair never becomes a release candidate.
/// WHY:  the executor would otherwise recurse until a depth refusal, and a partially evaluated
///       recursive call must never replace its caller. The acyclic closure rejects both callees.
#[test]
fn release_rejects_recursive_ordinary_candidates() {
    let files = [
        (
            "src/@entry.moth",
            "plus_one |value I32| -> I32:\n    return value + 1\n;\n\nleft |value I32| -> I32:\n    return right(value)\n;\n\nright |value I32| -> I32:\n    return left(value)\n;\n\n~[: [left(1)] [plus_one(2)] ]\n",
        ),
        ("src/@config.moth", CONFIG_SOURCE),
    ];
    let debug = compile_template_fixture_with_profile(&files, BuildProfile::Dev);
    let release = compile_template_fixture_with_profile(&files, BuildProfile::Release);

    let debug_calls = entry_hir_local_call_count(entry_artifact(&debug));
    let release_calls = entry_hir_local_call_count(entry_artifact(&release));
    assert_eq!(
        release_calls + 1,
        debug_calls,
        "release may fold the independent helper but must keep the recursive call: debug={debug_calls} release={release_calls}"
    );
}

/// WHAT: a branch-local project dependency survives release folding through the replacement merge.
/// WHY:  the executed value's own metadata may omit the dependency, so the candidate's complete
///       effective provenance is attached to the replacement.
#[test]
fn release_folds_candidate_and_preserves_branch_local_project_dependency() {
    let files = [
        (
            "src/@entry.moth",
            "@core/text\n@project name\n\nsubject #|| -> I32:\n    return text.length(name)\n;\n\npick |flag Bool| -> I32:\n    if flag:\n        return subject()\n    else\n        return 1\n    ;\n;\n\n~[: [pick(false)] ]\n",
        ),
        ("src/@config.moth", CONFIG_SOURCE),
    ];
    let debug = compile_template_fixture_with_profile(&files, BuildProfile::Dev);
    let release = compile_template_fixture_with_profile(&files, BuildProfile::Release);

    let debug_calls = entry_hir_local_call_count(entry_artifact(&debug));
    let release_calls = entry_hir_local_call_count(entry_artifact(&release));
    assert!(
        debug_calls > 0 && release_calls == 0,
        "the branch candidate must be erased before its replacement provenance is judged: debug={debug_calls} release={release_calls}"
    );
    assert!(
        reachable_project_context(entry_artifact(&debug)),
        "debug must retain the authored project dependency"
    );
    assert!(
        reachable_project_context(entry_artifact(&release)),
        "release must not erase the branch-local project dependency from HIR"
    );
}

/// WHAT: a transitive callee dependency survives folding the outer ordinary call.
/// WHY:  the caller's expression occurrences do not include the callee body's project dependency;
///       the candidate closure propagates it and the replacement merge retains it.
#[test]
fn release_folds_outer_candidate_and_preserves_transitive_callee_dependency() {
    let files = [
        (
            "src/@entry.moth",
            "@core/text\n@project name\n\nsubject #|| -> I32:\n    return text.length(name)\n;\n\nhelper || -> I32:\n    return subject()\n;\n\nouter |flag Bool| -> I32:\n    if flag:\n        helper()\n    else\n        helper()\n    ;\n    return 1\n;\n\n~[: [outer(false)] ]\n",
        ),
        ("src/@config.moth", CONFIG_SOURCE),
    ];
    let debug = compile_template_fixture_with_profile(&files, BuildProfile::Dev);
    let release = compile_template_fixture_with_profile(&files, BuildProfile::Release);

    let debug_calls = entry_hir_local_call_count(entry_artifact(&debug));
    let release_calls = entry_hir_local_call_count(entry_artifact(&release));
    assert!(
        debug_calls > 0 && release_calls == 0,
        "the outer call and its helper calls must be erased before transitive provenance is judged: debug={debug_calls} release={release_calls}"
    );
    assert!(
        reachable_project_context(entry_artifact(&debug)),
        "debug must retain the transitive project dependency"
    );
    assert!(
        reachable_project_context(entry_artifact(&release)),
        "release must not erase the transitive project dependency from HIR"
    );
}

/// WHAT: a short-circuited return retains the project dependency in its skipped operand.
/// WHY:  evaluation returns the left boolean value; the replacement merge must carry the complete
///       candidate provenance even though the skipped subtree was not evaluated.
#[test]
fn release_folds_short_circuit_and_preserves_skipped_dependency() {
    let files = [
        (
            "src/@entry.moth",
            "@core/text\n@project name\n\nsubject #|| -> I32:\n    return text.length(name)\n;\n\nprobe |flag Bool| -> Bool:\n    return flag and (subject() > 0)\n;\n\n~[: [probe(false)] ]\n",
        ),
        ("src/@config.moth", CONFIG_SOURCE),
    ];
    let debug = compile_template_fixture_with_profile(&files, BuildProfile::Dev);
    let release = compile_template_fixture_with_profile(&files, BuildProfile::Release);

    let debug_calls = entry_hir_local_call_count(entry_artifact(&debug));
    let release_calls = entry_hir_local_call_count(entry_artifact(&release));
    assert!(
        debug_calls > 0 && release_calls == 0,
        "the short-circuit candidate must be erased before its replacement provenance is judged: debug={debug_calls} release={release_calls}"
    );
    assert!(
        reachable_project_context(entry_artifact(&debug)),
        "debug must retain the short-circuit operand dependency"
    );
    assert!(
        reachable_project_context(entry_artifact(&release)),
        "release must not erase the skipped short-circuit dependency from HIR"
    );
}

/// WHAT: a smaller configured work budget leaves the call at runtime and emits one Note, while a
///       larger budget folds the same call.
/// WHY:  budget advice must be actionable and non-fatal, and the configured allowance must reach
///       the const executor rather than remaining a fixed compiler constant.
#[test]
fn release_budget_note_is_deduplicated_and_larger_budget_folds_more() {
    let entry = "count |limit I32| -> I32:\n    total ~I32 = 0\n    index ~I32 = 0\n    loop index < limit:\n        ~total = total + 1\n        ~index = index + 1\n    ;\n    return total\n;\n\nprobe |value I32| -> I32:\n    return count(20) + value\n;\n\n~[: ]\n";
    let small_config = "project #= (\n    name = \"docs\",\n    const_work_limit = 1,\n    const_memory_limit_mb = 1,\n)\nhtml #= ()\n";
    let large_config = "project #= (\n    name = \"docs\",\n    const_work_limit = 1000,\n    const_memory_limit_mb = 1,\n)\nhtml #= ()\n";

    let (small_compiled, small_warnings) = compile_fixture_with_warnings(
        &[("src/@entry.moth", entry), ("@config.moth", small_config)],
        BuildProfile::Release,
    );
    let (large_compiled, large_warnings) = compile_fixture_with_warnings(
        &[("src/@entry.moth", entry), ("@config.moth", large_config)],
        BuildProfile::Release,
    );
    let small_warnings = small_warnings.expect("the small budget must emit a Note");
    let notes: Vec<_> = small_warnings
        .iter_severity(DiagnosticSeverity::Note)
        .map(|(_, view)| view)
        .collect();
    assert_eq!(
        notes.len(),
        1,
        "one exhausted call site must produce one deduplicated Note"
    );
    let rendered = terse::format_terse_diagnostic_reports(&small_warnings);
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("const_work_limit")),
        "the Note must name the setting to raise: {rendered:?}"
    );
    assert!(
        large_warnings.is_none(),
        "the larger budget must fold without budget advice"
    );
    let small_calls = entry_hir_local_call_count(entry_artifact(&small_compiled));
    let large_calls = entry_hir_local_call_count(entry_artifact(&large_compiled));
    assert_eq!(
        small_calls,
        large_calls + 1,
        "the larger configured budget must fold the previously refused call"
    );
}

/// WHAT: an authored borrow violation remains a user error in both profiles, so release
///       optimisation cannot erase a call that the borrow checker owns.
/// WHY:  candidate removal happens before HIR borrow validation. A release-only success would
///       hide an illegal mutable access.
#[test]
fn release_preserves_authored_borrow_diagnostics() {
    let files = [
        (
            "src/@entry.moth",
            "mutate |value ~I32| -> I32:\n    ~value = value + 1\n    return value\n;\n\ncounter #I32 = 1\n\n~[: [mutate(counter)] ]\n",
        ),
        ("src/@config.moth", CONFIG_SOURCE),
    ];
    let debug = compile_template_fixture_reports_with_profile(&files, BuildProfile::Dev);
    let release = compile_template_fixture_reports_with_profile(&files, BuildProfile::Release);
    assert!(
        debug.counts().errors > 0,
        "the authored mutable-access violation must fail in debug"
    );
    assert!(
        release.counts().errors > 0,
        "release must not fold away the authored mutable-access violation"
    );
    assert_eq!(
        terse::format_terse_diagnostic_reports(&debug),
        terse::format_terse_diagnostic_reports(&release),
        "profile choice must not change the authored borrow diagnostic"
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
