use super::*;
use crate::build_system::resource_unions::ResourceOriginUnion;
use crate::builder_surface::PackageOrigin;
use crate::compiler_frontend::compiler_errors::{CompilerMessages, SourceLocation};
use crate::compiler_frontend::compiler_messages::{DiagnosticPayload, InvalidConfigReason};
use crate::compiler_frontend::folded_value::{OwnedFoldedString, OwnedFoldedStringPiece};
use crate::compiler_frontend::hir::ids::FunctionId;
use crate::compiler_frontend::hir::reachability::{HirReachability, ReachableResourceUse};
use crate::compiler_frontend::module_compilation::ResolvedConstFragment;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableProviderResourceOwnerId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn invalid_config_reason(messages: &CompilerMessages) -> &InvalidConfigReason {
    let diagnostic = messages
        .first_error()
        .expect("expected an error-severity diagnostic");
    match &diagnostic.payload {
        DiagnosticPayload::InvalidConfig { reason, .. } => reason,
        _ => panic!("expected an invalid config diagnostic"),
    }
}
fn module_resource_origin(
    package_name: &str,
    module_path: &str,
    resource_path: &str,
) -> StableResourceOriginId {
    module_resource_origin_with_identity(
        StablePackageIdentity::project_local(package_name),
        module_path,
        ModuleRootRole::Normal,
        resource_path,
    )
}

fn module_resource_origin_with_identity(
    package: StablePackageIdentity,
    module_path: &str,
    role: ModuleRootRole,
    resource_path: &str,
) -> StableResourceOriginId {
    let module_origin =
        StableModuleOriginIdentity::from_portable_path(package, module_path.to_owned(), role);
    StableResourceOriginId::module_owned(
        module_origin,
        PortableResourcePath::from_portable_spelling(resource_path.to_owned()).unwrap(),
    )
}

fn provider_resource_origin(
    provider_kind: &str,
    package_name: &str,
    resource_path: &str,
) -> StableResourceOriginId {
    provider_resource_origin_with_package_origin(
        provider_kind,
        PackageOrigin::Dependency,
        package_name,
        resource_path,
    )
}

fn provider_resource_origin_with_package_origin(
    provider_kind: &str,
    package_origin: PackageOrigin,
    package_name: &str,
    resource_path: &str,
) -> StableResourceOriginId {
    let owner = StableProviderResourceOwnerId::new(
        provider_kind,
        StablePackageIdentity::source_package(package_origin, package_name),
    );
    StableResourceOriginId::new(
        StableResourceOwnerId::Provider(owner),
        PortableResourcePath::from_portable_spelling(resource_path.to_owned()).unwrap(),
    )
}

fn authored_location(path: &str, string_table: &mut StringTable) -> SourceLocation {
    SourceLocation::from_path(Path::new(path), string_table)
}

#[test]
fn shared_origin_across_entries_emits_one_planned_record() {
    // WHAT: one origin observed across several contexts becomes one output record with one use
    //       per context.
    // WHY: output identity belongs to the semantic origin, not to a consumer entry, alias or
    //      observing artefact kind.
    let mut string_table = StringTable::new();
    let origin = module_resource_origin("app", "docs", "assets/logo.svg");
    let first_location = authored_location("docs/first.moth", &mut string_table);
    let second_location = authored_location("docs/second.moth", &mut string_table);
    let third_location = authored_location("docs/third.moth", &mut string_table);

    let mut plan = HtmlResourceOutputPlan::new("app");

    plan.plan_origin(
        origin.clone(),
        first_location,
        ResourceUrlContext::page_document(Path::new("docs/first.html")).unwrap(),
        &mut string_table,
        ResourceUseKind::Executable,
    )
    .unwrap();
    plan.plan_origin(
        origin.clone(),
        second_location,
        ResourceUrlContext::page_document(Path::new("docs/second.html")).unwrap(),
        &mut string_table,
        ResourceUseKind::Executable,
    )
    .unwrap();
    plan.plan_origin(
        origin,
        third_location,
        ResourceUrlContext::Stylesheet(PathBuf::from("docs/styles/main.css")),
        &mut string_table,
        ResourceUseKind::Executable,
    )
    .unwrap();

    assert_eq!(plan.records.len(), 1);
    assert_eq!(
        plan.records[0].output_path,
        PathBuf::from("docs/assets/logo.svg")
    );
    assert_eq!(plan.records[0].uses.len(), 3);
}

#[test]
fn distinct_origins_colliding_at_provider_path_report_both_locations() {
    // WHAT: provider-declared paths collide even when provider owners differ.
    // WHY: the output path is the provider contract, so silently choosing one origin would lose
    //      a semantic owner and make diagnostics depend on traversal order.
    let mut string_table = StringTable::new();
    let first_location = authored_location("first.moth", &mut string_table);
    let second_location = authored_location("second.moth", &mut string_table);
    let first = provider_resource_origin("images", "one", "shared/logo.svg");
    let second = provider_resource_origin("images", "two", "shared/logo.svg");
    let mut plan = HtmlResourceOutputPlan::new("app");

    plan.plan_origin(
        first,
        first_location,
        ResourceUrlContext::page_document(Path::new("index.html")).unwrap(),
        &mut string_table,
        ResourceUseKind::Executable,
    )
    .unwrap();
    let error = plan
        .plan_origin(
            second,
            second_location,
            ResourceUrlContext::page_document(Path::new("about.html")).unwrap(),
            &mut string_table,
            ResourceUseKind::Executable,
        )
        .unwrap_err();

    let InvalidConfigReason::ResourceOutputPathCollision {
        output_path,
        existing_origin,
        conflicting_origin,
    } = invalid_config_reason(&error)
    else {
        panic!("expected a resource output path collision reason");
    };
    assert_eq!(string_table.resolve(*output_path), "shared/logo.svg");
    assert!(
        string_table
            .resolve(*existing_origin)
            .contains("name 'one'")
    );
    assert!(
        string_table
            .resolve(*conflicting_origin)
            .contains("name 'two'")
    );
}

#[test]
fn distinct_module_origins_with_same_package_name_report_roles() {
    // WHAT: module origins with one package name but different root roles remain distinguishable.
    // WHY: diagnostics must identify both semantic owners when their output paths coincide.
    let mut string_table = StringTable::new();
    let first_location = authored_location("normal.moth", &mut string_table);
    let second_location = authored_location("support.moth", &mut string_table);
    let first = module_resource_origin_with_identity(
        StablePackageIdentity::source_package(PackageOrigin::ProjectLocal, "shared"),
        "module",
        ModuleRootRole::Normal,
        "assets/logo.svg",
    );
    let second = module_resource_origin_with_identity(
        StablePackageIdentity::source_package(PackageOrigin::ProjectLocal, "shared"),
        "module",
        ModuleRootRole::Support,
        "assets/logo.svg",
    );
    let mut plan = HtmlResourceOutputPlan::new("app");

    plan.plan_origin(
        first,
        first_location,
        ResourceUrlContext::page_document(Path::new("index.html")).unwrap(),
        &mut string_table,
        ResourceUseKind::Executable,
    )
    .unwrap();
    let error = plan
        .plan_origin(
            second,
            second_location,
            ResourceUrlContext::page_document(Path::new("about.html")).unwrap(),
            &mut string_table,
            ResourceUseKind::Executable,
        )
        .unwrap_err();

    let InvalidConfigReason::ResourceOutputPathCollision {
        existing_origin,
        conflicting_origin,
        ..
    } = invalid_config_reason(&error)
    else {
        panic!("expected a resource output path collision reason");
    };
    assert!(
        string_table
            .resolve(*existing_origin)
            .contains("role 'normal'")
    );
    assert!(
        string_table
            .resolve(*conflicting_origin)
            .contains("role 'support'")
    );
}

#[test]
fn distinct_provider_origins_with_same_package_name_report_package_origins() {
    // WHAT: provider origins with one package name but different package origins collide at the
    //      provider-declared path without losing either stable identity.
    // WHY: provider package origin is part of semantic ownership even when output placement is
    //      deliberately independent of package metadata.
    let mut string_table = StringTable::new();
    let first_location = authored_location("builder.moth", &mut string_table);
    let second_location = authored_location("dependency.moth", &mut string_table);
    let first = provider_resource_origin_with_package_origin(
        "images",
        PackageOrigin::Builder,
        "shared",
        "shared/logo.svg",
    );
    let second = provider_resource_origin_with_package_origin(
        "images",
        PackageOrigin::Dependency,
        "shared",
        "shared/logo.svg",
    );
    let mut plan = HtmlResourceOutputPlan::new("app");

    plan.plan_origin(
        first,
        first_location,
        ResourceUrlContext::page_document(Path::new("index.html")).unwrap(),
        &mut string_table,
        ResourceUseKind::Executable,
    )
    .unwrap();
    let error = plan
        .plan_origin(
            second,
            second_location,
            ResourceUrlContext::page_document(Path::new("about.html")).unwrap(),
            &mut string_table,
            ResourceUseKind::Executable,
        )
        .unwrap_err();

    let InvalidConfigReason::ResourceOutputPathCollision {
        existing_origin,
        conflicting_origin,
        ..
    } = invalid_config_reason(&error)
    else {
        panic!("expected a resource output path collision reason");
    };
    assert!(
        string_table
            .resolve(*existing_origin)
            .contains("package origin 'builder'")
    );
    assert!(
        string_table
            .resolve(*conflicting_origin)
            .contains("package origin 'dependency'")
    );
    assert!(
        string_table
            .resolve(*existing_origin)
            .contains("name 'shared'")
    );
    assert!(
        string_table
            .resolve(*conflicting_origin)
            .contains("name 'shared'")
    );
}

#[test]
fn reserved_html_output_rejects_resource_planning() {
    // WHAT: a resource cannot be planned into an already reserved page destination.
    // WHY: builder-owned output reservations must protect later resource planning, not only
    //      reject a resource that happened to be planned first.
    let mut string_table = StringTable::new();
    let location = authored_location("index.moth", &mut string_table);
    let mut plan = HtmlResourceOutputPlan::new("app");
    let origin = module_resource_origin("app", "", "index.html");

    plan.reserve_builder_output_path(Path::new("index.html"), "HTML page", &mut string_table)
        .unwrap();
    let error = plan
        .plan_origin(
            origin,
            location,
            ResourceUrlContext::page_document(Path::new("other.html")).unwrap(),
            &mut string_table,
            ResourceUseKind::Executable,
        )
        .unwrap_err();

    let InvalidConfigReason::ResourceOutputPathReserved {
        output_path,
        origin,
        artefact_kind,
    } = invalid_config_reason(&error)
    else {
        panic!("expected a reserved resource output path reason");
    };
    assert_eq!(string_table.resolve(*output_path), "index.html");
    assert_eq!(string_table.resolve(*artefact_kind), "HTML page");
    assert!(string_table.resolve(*origin).contains("index.html"));
}

#[test]
fn reserved_javascript_glue_output_rejects_resource_planning() {
    // WHAT: a resource cannot be planned into an already reserved generated glue destination.
    // WHY: generated JavaScript paths must participate in the same reservation set as pages so a
    //      later resource cannot overwrite a glue module.
    let mut string_table = StringTable::new();
    let location = authored_location("glue.moth", &mut string_table);
    let glue_path = Path::new("_moth/js/glue/module-0123456789abcdef.js");
    let origin = module_resource_origin("app", "_moth/js/glue", "module-0123456789abcdef.js");
    let mut plan = HtmlResourceOutputPlan::new("app");

    plan.reserve_builder_output_path(glue_path, "JavaScript", &mut string_table)
        .unwrap();
    let error = plan
        .plan_origin(
            origin,
            location,
            ResourceUrlContext::page_document(Path::new("index.html")).unwrap(),
            &mut string_table,
            ResourceUseKind::Executable,
        )
        .unwrap_err();

    let InvalidConfigReason::ResourceOutputPathReserved {
        output_path,
        origin,
        artefact_kind,
    } = invalid_config_reason(&error)
    else {
        panic!("expected a reserved resource output path reason");
    };
    assert_eq!(
        string_table.resolve(*output_path),
        glue_path.to_str().unwrap()
    );
    assert_eq!(string_table.resolve(*artefact_kind), "JavaScript");
    assert!(
        string_table
            .resolve(*origin)
            .contains("module-0123456789abcdef.js")
    );
}
#[test]
fn live_resource_use_locations_override_intern_location() {
    // WHAT: executable resource uses retain each live authored location in the output plan.
    // WHY: the resource table's first location can point at a declaration or folded value rather
    //      than the HIR expression that keeps the resource live.
    let mut string_table = StringTable::new();
    let mut module = crate::projects::html_project::tests::test_support::create_test_module(
        PathBuf::from("@page.moth"),
        &mut string_table,
    );
    let origin = module_resource_origin("app", "", "assets/logo.svg");
    let intern_location = authored_location("intern.moth", &mut string_table);
    let resource_id = module
        .executable
        .resource_table
        .intern_origin(origin.clone(), intern_location);
    let first_live_location = authored_location("live-first.moth", &mut string_table);
    let second_live_location = authored_location("live-second.moth", &mut string_table);
    let mut reachability = HirReachability::default();
    reachability
        .reachable_resource_uses
        .push(ReachableResourceUse {
            resource_id,
            owner: FunctionId(0),
            location: first_live_location.clone(),
        });
    reachability
        .reachable_resource_uses
        .push(ReachableResourceUse {
            resource_id,
            owner: FunctionId(0),
            location: second_live_location.clone(),
        });

    let mut locations = HashMap::new();
    record_reachable_resource_locations(&mut locations, &module, &reachability).unwrap();
    let mut union = ResourceOriginUnion::new();
    union.insert(origin);
    let mut plan = HtmlResourceOutputPlan::new("app");
    plan.plan_union(
        &union,
        &locations,
        ResourceUrlContext::PageDocument(PathBuf::from("index.html")),
        &mut string_table,
    )
    .unwrap();

    let record = &plan.records[0];
    assert_eq!(record.first_authored_location, first_live_location);
    assert!(record.has_executable_use);
    assert_eq!(record.uses.len(), 2);
    assert_eq!(record.uses[0].authored_location, first_live_location);
    assert_eq!(record.uses[1].authored_location, second_live_location);
}

#[test]
fn later_live_use_replaces_intern_fallback_location() {
    // WHAT: a later live HIR use promotes first_authored_location over an earlier intern fallback.
    // WHY: sequential entry planning can intern metadata first; collision diagnostics must still
    //      point at the live executable use.
    let mut string_table = StringTable::new();
    let origin = module_resource_origin("app", "", "assets/logo.svg");
    let intern_location = authored_location("intern.moth", &mut string_table);
    let live_location = authored_location("live.moth", &mut string_table);
    let mut union = ResourceOriginUnion::new();
    union.insert(origin.clone());
    let context = ResourceUrlContext::PageDocument(PathBuf::from("index.html"));
    let mut plan = HtmlResourceOutputPlan::new("app");

    let mut fallback_locations = HashMap::new();
    fallback_locations.insert(
        origin.clone(),
        OriginAuthoredLocations {
            executable: Vec::new(),
            metadata: Vec::new(),
            fallback: Some(intern_location.clone()),
        },
    );
    plan.plan_union(
        &union,
        &fallback_locations,
        context.clone(),
        &mut string_table,
    )
    .unwrap();
    assert_eq!(plan.records[0].first_authored_location, intern_location);

    let mut live_locations = HashMap::new();
    live_locations.insert(
        origin,
        OriginAuthoredLocations {
            executable: vec![live_location.clone()],
            metadata: Vec::new(),
            fallback: Some(intern_location),
        },
    );
    plan.plan_union(&union, &live_locations, context, &mut string_table)
        .unwrap();

    let record = &plan.records[0];
    assert_eq!(record.first_authored_location, live_location);
    assert_eq!(record.uses.len(), 1);
    assert_eq!(record.uses[0].authored_location, live_location);
}

#[test]
fn fragment_and_metadata_uses_keep_authored_locations_with_hir_use() {
    let mut string_table = StringTable::new();
    let mut module = crate::projects::html_project::tests::test_support::create_test_module(
        PathBuf::from("@page.moth"),
        &mut string_table,
    );
    let origin = module_resource_origin("app", "", "assets/logo.svg");
    let intern_location = authored_location("intern.moth", &mut string_table);
    let fragment_location = authored_location("fragment.moth", &mut string_table);
    let metadata_location = authored_location("metadata.moth", &mut string_table);
    let resource_id = module
        .executable
        .resource_table
        .intern_origin(origin.clone(), intern_location);
    module.metadata.const_top_level_fragments = vec![ResolvedConstFragment {
        runtime_insertion_index: 0,
        location: fragment_location.clone(),
        value: OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Resource(origin.clone())]),
    }];

    let live_location = authored_location("live.moth", &mut string_table);
    let mut reachability = HirReachability::default();
    reachability
        .reachable_resource_uses
        .push(ReachableResourceUse {
            resource_id,
            owner: FunctionId(0),
            location: live_location.clone(),
        });
    let mut locations = HashMap::new();
    record_reachable_resource_locations(&mut locations, &module, &reachability).unwrap();
    record_const_fragment_resource_locations(&mut locations, &module);

    let mut union = ResourceOriginUnion::new();
    union.insert(origin.clone());
    let context = ResourceUrlContext::PageDocument(PathBuf::from("index.html"));
    let mut plan = HtmlResourceOutputPlan::new("app");
    plan.plan_origin(
        origin.clone(),
        metadata_location.clone(),
        context.clone(),
        &mut string_table,
        ResourceUseKind::Metadata,
    )
    .unwrap();
    plan.plan_origin(
        origin.clone(),
        fragment_location.clone(),
        context.clone(),
        &mut string_table,
        ResourceUseKind::Metadata,
    )
    .unwrap();
    plan.plan_union(&union, &locations, context, &mut string_table)
        .unwrap();

    let record = &plan.records[0];
    assert_eq!(record.first_authored_location, live_location);
    assert!(record.has_executable_use);
    assert!(
        record
            .uses
            .iter()
            .any(|use_record| { use_record.authored_location == fragment_location })
    );
    assert!(
        record
            .uses
            .iter()
            .any(|use_record| { use_record.authored_location == metadata_location })
    );
}

#[test]
fn metadata_only_use_plans_output_without_an_executable_use() {
    // WHAT: a metadata-only origin still plans a byte-free output record.
    // WHY: compile-time fragment and page-metadata uses keep a resource output-planned without
    //      any HIR-reachable reference, and they own the first location until an executable use
    //      overrides it.
    let mut string_table = StringTable::new();
    let origin = module_resource_origin("app", "", "assets/logo.svg");
    let metadata_location = authored_location("metadata.moth", &mut string_table);
    let mut plan = HtmlResourceOutputPlan::new("app");

    plan.plan_origin(
        origin.clone(),
        metadata_location.clone(),
        ResourceUrlContext::PageDocument(PathBuf::from("index.html")),
        &mut string_table,
        ResourceUseKind::Metadata,
    )
    .unwrap();

    let record = plan
        .record_for_origin(&origin)
        .expect("a planned origin should be indexed");
    assert_eq!(record.output_path, PathBuf::from("assets/logo.svg"));
    assert_eq!(record.first_authored_location, metadata_location);
    assert!(!record.has_executable_use);
    assert_eq!(record.uses.len(), 1);
}

#[test]
fn record_for_origin_resolves_planned_records_directly() {
    // WHAT: the origin index resolves one planned record per origin without scanning.
    // WHY: the structural URL renderer looks up one origin per rendered resource piece.
    let mut string_table = StringTable::new();
    let first = module_resource_origin("app", "", "assets/logo.svg");
    let second = module_resource_origin("app", "docs", "assets/banner.svg");
    let unrecorded = module_resource_origin("app", "", "assets/absent.svg");
    let mut plan = HtmlResourceOutputPlan::new("app");

    plan.plan_origin(
        first.clone(),
        authored_location("first.moth", &mut string_table),
        ResourceUrlContext::PageDocument(PathBuf::from("index.html")),
        &mut string_table,
        ResourceUseKind::Executable,
    )
    .unwrap();
    plan.plan_origin(
        second.clone(),
        authored_location("second.moth", &mut string_table),
        ResourceUrlContext::PageDocument(PathBuf::from("docs/index.html")),
        &mut string_table,
        ResourceUseKind::Executable,
    )
    .unwrap();

    assert_eq!(
        plan.record_for_origin(&first)
            .expect("first origin should be indexed")
            .output_path,
        PathBuf::from("assets/logo.svg")
    );
    assert_eq!(
        plan.record_for_origin(&second)
            .expect("second origin should be indexed")
            .output_path,
        PathBuf::from("docs/assets/banner.svg")
    );
    assert!(plan.record_for_origin(&unrecorded).is_none());
}

#[test]
fn project_local_origin_preserves_entry_root_relative_path() {
    // WHAT: project-local module resources retain their module-relative output spelling.
    // WHY: adding an artificial package prefix would break authored page-relative URLs.
    let mut string_table = StringTable::new();
    let origin = module_resource_origin("app", "docs/getting-started", "assets/logo.svg");
    let mut plan = HtmlResourceOutputPlan::new("app");

    plan.plan_origin(
        origin,
        authored_location("docs/getting-started/@page.moth", &mut string_table),
        ResourceUrlContext::page_document(Path::new("docs/getting-started/index.html")).unwrap(),
        &mut string_table,
        ResourceUseKind::Executable,
    )
    .unwrap();

    assert_eq!(
        plan.records[0].output_path,
        PathBuf::from("docs/getting-started/assets/logo.svg")
    );
}

#[test]
fn site_root_is_not_planned_as_a_resource_output() {
    // WHAT: an empty resource plan has no synthetic root output.
    // WHY: SiteRoot is a semantic reachability fact, not a resource and has no output path.
    let plan = HtmlResourceOutputPlan::new("app");
    assert!(plan.records.is_empty());
}
