use super::*;
use crate::build_system::resource_unions::ResourceOriginUnion;
use crate::builder_surface::PackageOrigin;
use crate::compiler_frontend::compiler_errors::CompilerMessages;
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
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
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

fn authored_span(start: u32) -> SourceSpan {
    let mut extended = ExtendedSpanBuilder::default();
    SourceSpan::new(
        SourceId::from_index(0),
        LocalSpan::exact(start, 1, &mut extended).expect("fixture span should fit"),
    )
}

#[test]
fn shared_origin_across_entries_emits_one_planned_record() {
    // WHAT: one origin observed across several contexts becomes one output record with one use
    //       per context.
    // WHY: output identity belongs to the semantic origin, not to a consumer entry, alias or
    //      observing artefact kind.
    let mut string_table = StringTable::new();
    let origin = module_resource_origin("app", "docs", "assets/logo.svg");
    let first_span = authored_span(0);
    let second_span = authored_span(2);
    let third_span = authored_span(4);

    let mut plan = HtmlResourceOutputPlan::new("app");

    plan.plan_origin(
        origin.clone(),
        Some(first_span),
        ResourceUrlContext::page_document(Path::new("docs/first.html")).unwrap(),
        &mut string_table,
        ResourceUseKind::Executable,
    )
    .unwrap();
    plan.plan_origin(
        origin.clone(),
        Some(second_span),
        ResourceUrlContext::page_document(Path::new("docs/second.html")).unwrap(),
        &mut string_table,
        ResourceUseKind::Executable,
    )
    .unwrap();
    plan.plan_origin(
        origin,
        Some(third_span),
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
fn distinct_origins_colliding_at_provider_path_report_both_spans() {
    // WHAT: provider-declared paths collide even when provider owners differ.
    // WHY: the output path is the provider contract, so silently choosing one origin would lose
    //      a semantic owner and make diagnostics depend on traversal order.
    let mut string_table = StringTable::new();
    let first = provider_resource_origin("images", "one", "shared/logo.svg");
    let second = provider_resource_origin("images", "two", "shared/logo.svg");
    let mut plan = HtmlResourceOutputPlan::new("app");

    plan.plan_origin(
        first,
        Some(authored_span(0)),
        ResourceUrlContext::page_document(Path::new("index.html")).unwrap(),
        &mut string_table,
        ResourceUseKind::Executable,
    )
    .unwrap();
    let error = plan
        .plan_origin(
            second,
            Some(authored_span(2)),
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
        Some(authored_span(0)),
        ResourceUrlContext::page_document(Path::new("index.html")).unwrap(),
        &mut string_table,
        ResourceUseKind::Executable,
    )
    .unwrap();
    let error = plan
        .plan_origin(
            second,
            Some(authored_span(2)),
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
        Some(authored_span(0)),
        ResourceUrlContext::page_document(Path::new("index.html")).unwrap(),
        &mut string_table,
        ResourceUseKind::Executable,
    )
    .unwrap();
    let error = plan
        .plan_origin(
            second,
            Some(authored_span(2)),
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
    let mut plan = HtmlResourceOutputPlan::new("app");
    let origin = module_resource_origin("app", "", "index.html");

    plan.reserve_builder_output_path(Path::new("index.html"), "HTML page", &mut string_table)
        .unwrap();
    let error = plan
        .plan_origin(
            origin,
            Some(authored_span(0)),
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
    let glue_path = Path::new("_moth/js/glue/module-0123456789abcdef.js");
    let origin = module_resource_origin("app", "_moth/js/glue", "module-0123456789abcdef.js");
    let mut plan = HtmlResourceOutputPlan::new("app");

    plan.reserve_builder_output_path(glue_path, "JavaScript", &mut string_table)
        .unwrap();
    let error = plan
        .plan_origin(
            origin,
            Some(authored_span(0)),
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
fn live_resource_use_spans_override_intern_span() {
    // WHAT: executable resource uses retain each live authored span in the output plan.
    // WHY: the resource table's first span can point at a declaration or folded value rather
    //      than the HIR expression that keeps the resource live.
    let mut string_table = StringTable::new();
    let mut module = crate::projects::html_project::tests::test_support::create_test_module(
        PathBuf::from("@page.moth"),
        &mut string_table,
    );
    let origin = module_resource_origin("app", "", "assets/logo.svg");
    let intern_span = authored_span(0);
    let resource_id = module
        .executable
        .resource_table
        .intern_origin(origin.clone(), Some(intern_span));
    let first_live_span = authored_span(2);
    let second_live_span = authored_span(4);
    let mut reachability = HirReachability::default();
    reachability
        .reachable_resource_uses
        .push(ReachableResourceUse {
            resource_id,
            owner: FunctionId(0),
            span: Some(first_live_span),
        });
    reachability
        .reachable_resource_uses
        .push(ReachableResourceUse {
            resource_id,
            owner: FunctionId(0),
            span: Some(second_live_span),
        });

    let mut spans = HashMap::new();
    record_reachable_resource_spans(&mut spans, &module, &reachability).unwrap();
    let mut union = ResourceOriginUnion::new();
    union.insert(origin);
    let mut plan = HtmlResourceOutputPlan::new("app");
    plan.plan_union(
        &union,
        &spans,
        ResourceUrlContext::PageDocument(PathBuf::from("index.html")),
        &mut string_table,
    )
    .unwrap();

    let record = &plan.records[0];
    assert_eq!(record.first_authored_span, Some(first_live_span));
    assert!(record.has_executable_use);
    assert_eq!(record.uses.len(), 2);
    assert_eq!(record.uses[0].authored_span, Some(first_live_span));
    assert_eq!(record.uses[1].authored_span, Some(second_live_span));
}

#[test]
fn later_live_use_replaces_intern_fallback_span() {
    // WHAT: a later live HIR use promotes first_authored_span over an earlier intern fallback.
    // WHY: sequential entry planning can intern metadata first; collision diagnostics must still
    //      point at the live executable use.
    let mut string_table = StringTable::new();
    let origin = module_resource_origin("app", "", "assets/logo.svg");
    let intern_span = authored_span(0);
    let live_span = authored_span(2);
    let mut union = ResourceOriginUnion::new();
    union.insert(origin.clone());
    let context = ResourceUrlContext::PageDocument(PathBuf::from("index.html"));
    let mut plan = HtmlResourceOutputPlan::new("app");

    let mut fallback_spans = HashMap::new();
    fallback_spans.insert(
        origin.clone(),
        OriginAuthoredSpans {
            executable: Vec::new(),
            metadata: Vec::new(),
            fallback: Some(intern_span),
        },
    );
    plan.plan_union(&union, &fallback_spans, context.clone(), &mut string_table)
        .unwrap();
    assert_eq!(plan.records[0].first_authored_span, Some(intern_span));

    let mut live_spans = HashMap::new();
    live_spans.insert(
        origin,
        OriginAuthoredSpans {
            executable: vec![Some(live_span)],
            metadata: Vec::new(),
            fallback: Some(intern_span),
        },
    );
    plan.plan_union(&union, &live_spans, context, &mut string_table)
        .unwrap();

    let record = &plan.records[0];
    assert_eq!(record.first_authored_span, Some(live_span));
    assert_eq!(record.uses.len(), 1);
    assert_eq!(record.uses[0].authored_span, Some(live_span));
}
#[test]
fn fragment_and_metadata_uses_keep_authored_spans_with_hir_use() {
    let mut string_table = StringTable::new();
    let mut module = crate::projects::html_project::tests::test_support::create_test_module(
        PathBuf::from("@page.moth"),
        &mut string_table,
    );
    let origin = module_resource_origin("app", "", "assets/logo.svg");
    let intern_span = authored_span(0);
    let fragment_span = authored_span(2);
    let metadata_span = authored_span(4);
    let resource_id = module
        .executable
        .resource_table
        .intern_origin(origin.clone(), Some(intern_span));
    module.metadata.const_top_level_fragments = vec![ResolvedConstFragment {
        runtime_insertion_index: 0,
        span: Some(fragment_span),
        value: OwnedFoldedString::Pieces(vec![OwnedFoldedStringPiece::Resource(origin.clone())]),
    }];

    let live_span = authored_span(6);
    let mut reachability = HirReachability::default();
    reachability
        .reachable_resource_uses
        .push(ReachableResourceUse {
            resource_id,
            owner: FunctionId(0),
            span: Some(live_span),
        });
    let mut spans = HashMap::new();
    record_reachable_resource_spans(&mut spans, &module, &reachability).unwrap();
    record_const_fragment_resource_spans(&mut spans, &module);

    let mut union = ResourceOriginUnion::new();
    union.insert(origin.clone());
    let context = ResourceUrlContext::PageDocument(PathBuf::from("index.html"));
    let mut plan = HtmlResourceOutputPlan::new("app");
    plan.plan_origin(
        origin.clone(),
        Some(metadata_span),
        context.clone(),
        &mut string_table,
        ResourceUseKind::Metadata,
    )
    .unwrap();
    plan.plan_origin(
        origin.clone(),
        Some(fragment_span),
        context.clone(),
        &mut string_table,
        ResourceUseKind::Metadata,
    )
    .unwrap();
    plan.plan_union(&union, &spans, context, &mut string_table)
        .unwrap();

    let record = &plan.records[0];
    assert_eq!(record.first_authored_span, Some(live_span));
    assert!(record.has_executable_use);
    assert!(
        record
            .uses
            .iter()
            .any(|use_record| use_record.authored_span == Some(fragment_span))
    );
    assert!(
        record
            .uses
            .iter()
            .any(|use_record| use_record.authored_span == Some(metadata_span))
    );
}

#[test]
fn metadata_only_use_plans_output_without_an_executable_use() {
    // WHAT: a metadata-only origin still plans a byte-free output record.
    // WHY: compile-time fragment and page-metadata uses keep a resource output-planned without
    //      any HIR-reachable reference, and they own the first span until an executable use
    //      overrides it.
    let mut string_table = StringTable::new();
    let origin = module_resource_origin("app", "", "assets/logo.svg");
    let metadata_span = authored_span(0);
    let mut plan = HtmlResourceOutputPlan::new("app");

    plan.plan_origin(
        origin.clone(),
        Some(metadata_span),
        ResourceUrlContext::PageDocument(PathBuf::from("index.html")),
        &mut string_table,
        ResourceUseKind::Metadata,
    )
    .unwrap();

    let record = plan
        .record_for_origin(&origin)
        .expect("a planned origin should be indexed");
    assert_eq!(record.output_path, PathBuf::from("assets/logo.svg"));
    assert_eq!(record.first_authored_span, Some(metadata_span));
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
        None,
        ResourceUrlContext::PageDocument(PathBuf::from("index.html")),
        &mut string_table,
        ResourceUseKind::Executable,
    )
    .unwrap();
    plan.plan_origin(
        second.clone(),
        None,
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
        None,
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
