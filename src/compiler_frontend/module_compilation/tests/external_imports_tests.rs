//! Runtime candidate identity, compatibility and finalized-call lookup invariants.

use super::*;
use crate::builder_surface::external_import_providers::provider::ResolvedExternalImport;
use crate::builder_surface::{BuilderSurface, PackageOrigin};
use crate::compiler_frontend::external_packages::ExternalFunctionId;
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableProviderResourceOwnerId, StableResourceOriginId,
    StableResourceOwnerId,
};
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use std::path::PathBuf;

fn authored_span(start: u32) -> SourceSpan {
    let local = LocalSpan::exact(start, 1, &mut ExtendedSpanBuilder::new())
        .expect("fixture span fits inline");
    SourceSpan::new(SourceId::from_index(1), local)
}

fn runtime_asset() -> RuntimeAssetIdentity {
    let owner = StableResourceOwnerId::Provider(StableProviderResourceOwnerId::new(
        "test-js",
        StablePackageIdentity::binding(PackageOrigin::ProjectLocal, "@test/runtime"),
    ));
    RuntimeAssetIdentity {
        origin: StableResourceOriginId::new(
            owner,
            PortableResourcePath::from_portable_spelling("_moth/js/runtime.js".to_owned())
                .expect("fixture path is portable"),
        ),
        canonical_source_path: PathBuf::from("/fixture/runtime.js"),
        asset_kind: "js".to_owned(),
        source_span: Some(authored_span(10)),
    }
}

fn runtime_import(module_name: &str, names: &[&str]) -> RequiredRuntimeImport {
    RequiredRuntimeImport {
        module_name: module_name.to_owned(),
        imported_names: names.iter().map(|name| (*name).to_owned()).collect(),
    }
}

fn provider_record(package_id: ExternalPackageId) -> ResolvedExternalImport {
    ResolvedExternalImport {
        package_id,
        exported_types: Vec::new(),
        exported_free_functions: Vec::new(),
        runtime_asset: Some(runtime_asset()),
        required_runtime_imports: vec![
            runtime_import("@moth/runtime", &["mothOk", "mothErr"]),
            runtime_import("@test/empty", &[]),
        ],
    }
}

#[test]
fn runtime_candidates_merge_semantic_contracts_and_preserve_provider_provenance() {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package("@test/runtime", PackageOrigin::ProjectLocal)
        .expect("fixture package registers");
    let provider = provider_record(package_id);
    let canonical_span = provider
        .runtime_asset
        .as_ref()
        .expect("fixture asset")
        .source_span;
    let mut repeated_asset = runtime_asset();
    repeated_asset.source_span = Some(authored_span(40));
    let builder = BuilderRuntimePackageMetadata {
        package_id,
        runtime_asset: Some(repeated_asset),
        required_runtime_imports: vec![
            runtime_import("@test/empty", &[]),
            runtime_import("@moth/runtime", &["mothErr", "mothErr"]),
            runtime_import("@moth/runtime", &["mothOk"]),
        ],
    };
    let mut table = ExternalImportResolutionTable::new();
    table.insert("provider.moth", "runtime.js", provider);
    let other_package_id = registry
        .register_package("@test/other", PackageOrigin::Builder)
        .expect("second fixture package registers");
    let other_builder = BuilderRuntimePackageMetadata {
        package_id: other_package_id,
        runtime_asset: None,
        required_runtime_imports: Vec::new(),
    };
    let requested = FxHashSet::from_iter([other_package_id, package_id]);
    let candidates = collect_external_import_candidates_for_packages(
        &requested,
        &table,
        &[other_builder, builder.clone(), builder],
        &registry,
    )
    .expect("location/order/duplicate-name differences are compatible");
    assert_eq!(candidates.len(), 2);
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.package_id)
            .collect::<Vec<_>>(),
        vec![package_id, other_package_id],
        "publication is in deterministic package order"
    );
    assert!(
        candidates[1].runtime_asset.is_none(),
        "known no-asset records remain valid"
    );
    assert_eq!(
        candidates[0]
            .runtime_asset
            .as_ref()
            .expect("canonical asset")
            .source_span,
        canonical_span
    );
    assert_eq!(
        candidates[0].required_runtime_imports,
        vec![
            runtime_import("@moth/runtime", &["mothOk", "mothErr"]),
            runtime_import("@test/empty", &[]),
        ]
    );
}

#[test]
fn runtime_candidates_reject_conflicting_provider_builder_facts() {
    let mut registry = ExternalPackageRegistry::new();
    let package_id = registry
        .register_package("@test/runtime", PackageOrigin::ProjectLocal)
        .expect("fixture package registers");
    let provider = provider_record(package_id);
    let canonical_span = provider
        .runtime_asset
        .as_ref()
        .expect("fixture asset")
        .source_span;
    let mut table = ExternalImportResolutionTable::new();
    table.insert("provider.moth", "runtime.js", provider.clone());
    let requested = FxHashSet::from_iter([package_id]);

    for fact in [
        "presence",
        "origin",
        "kind",
        "canonical byte-source path",
        "module/import contract",
    ] {
        let mut builder = BuilderRuntimePackageMetadata {
            package_id,
            runtime_asset: provider.runtime_asset.clone(),
            required_runtime_imports: provider.required_runtime_imports.clone(),
        };
        builder
            .runtime_asset
            .as_mut()
            .expect("fixture asset")
            .source_span = Some(authored_span(40));
        match fact {
            "presence" => builder.runtime_asset = None,
            "origin" => {
                let asset = builder.runtime_asset.as_mut().expect("fixture asset");
                asset.origin = StableResourceOriginId::new(
                    StableResourceOwnerId::Provider(StableProviderResourceOwnerId::new(
                        "conflicting-provider",
                        StablePackageIdentity::binding(
                            PackageOrigin::ProjectLocal,
                            "@test/runtime",
                        ),
                    )),
                    PortableResourcePath::from_portable_spelling("_moth/js/runtime.js".to_owned())
                        .expect("fixture path is portable"),
                );
            }
            "kind" => {
                builder
                    .runtime_asset
                    .as_mut()
                    .expect("fixture asset")
                    .asset_kind = "wit".to_owned()
            }
            "canonical byte-source path" => {
                builder
                    .runtime_asset
                    .as_mut()
                    .expect("fixture asset")
                    .canonical_source_path = PathBuf::from("/fixture/other.js")
            }
            "module/import contract" => builder.required_runtime_imports[0]
                .imported_names
                .push("unexpected".to_owned()),
            _ => unreachable!("closed conflict table"),
        }
        let error = collect_external_import_candidates_for_packages(
            &requested,
            &table,
            &[builder],
            &registry,
        )
        .expect_err("incompatible trusted runtime metadata must fail");
        assert_eq!(error.error_type, ErrorType::Compiler, "{fact}");
        assert!(error.msg.contains("@test/runtime"), "{}", error.msg);
        assert!(error.msg.contains(fact), "{}", error.msg);
        assert!(
            error.msg.contains("Canonical asset/provenance"),
            "{}",
            error.msg
        );
        assert!(
            error.msg.contains("conflicting asset/provenance"),
            "{}",
            error.msg
        );
        assert_eq!(error.source_span, canonical_span, "{fact}");
        if fact != "presence" {
            assert!(
                error.msg.contains(&format!("{:?}", authored_span(40))),
                "{}",
                error.msg
            );
        }
    }

    // A no-asset canonical record cannot hide a later asset either.
    let mut provider = provider;
    provider.runtime_asset = None;
    table.insert("provider.moth", "runtime.js", provider);
    let builder = BuilderRuntimePackageMetadata {
        package_id,
        runtime_asset: Some(runtime_asset()),
        required_runtime_imports: Vec::new(),
    };
    let error =
        collect_external_import_candidates_for_packages(&requested, &table, &[builder], &registry)
            .expect_err("None versus Some must conflict in both directions");
    assert!(
        error.msg.contains("runtime asset presence"),
        "{}",
        error.msg
    );
}

#[test]
fn finalized_external_calls_check_registry_and_allow_core_helpers_without_assets() {
    let surface = BuilderSurface::with_mandatory_core();
    let registry = &surface.binding_packages;
    let (helper_id, _) = registry
        .resolve_package_function("@core/io", "line")
        .expect("mandatory Core IO helper registers");
    let calls = [
        ReachableExternalCall {
            function_id: helper_id,
            span: Some(authored_span(10)),
        },
        ReachableExternalCall {
            function_id: helper_id,
            span: Some(authored_span(40)),
        },
    ];
    let packages = collect_external_import_package_ids(calls.iter(), registry)
        .expect("Core helper definition and owner are valid");
    assert_eq!(packages.len(), 1, "package demand deduplicates call sites");
    let candidates = collect_external_import_candidates_for_packages(
        &packages,
        &ExternalImportResolutionTable::new(),
        &[],
        registry,
    )
    .expect("Core inline/runtime helpers need no provider asset");
    assert!(candidates.is_empty());

    let missing = ReachableExternalCall {
        function_id: ExternalFunctionId::Synthetic(u32::MAX),
        span: Some(authored_span(70)),
    };
    let error = collect_external_import_package_ids(std::iter::once(&missing), registry)
        .expect_err("unknown finalized callable is not an optional dependency");
    assert_eq!(error.error_type, ErrorType::Compiler);
    assert_eq!(error.source_span, missing.span);
    assert!(error.msg.contains("not registered"));

    let unknown_packages = FxHashSet::from_iter([ExternalPackageId(u32::MAX)]);
    let error = collect_external_import_candidates_for_packages(
        &unknown_packages,
        &ExternalImportResolutionTable::new(),
        &[],
        registry,
    )
    .expect_err("candidate metadata cannot refer to an unregistered owner");
    assert_eq!(error.error_type, ErrorType::Compiler);
    assert!(error.msg.contains("candidate package"));
}
