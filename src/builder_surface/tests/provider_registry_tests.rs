//! Unit tests for the external import provider registry and cache.
//!
//! WHAT: proves that `BuilderSurface` initializes the registry correctly, providers can be
//!       registered and discovered, the registry clones correctly, and cache keys behave
//!       as expected.

use crate::builder_surface::BuilderSurface;
use crate::builder_surface::external_import_providers::cache::{
    ExternalImportCacheKey, ExternalImportProviderCache,
};
use crate::builder_surface::external_import_providers::provider::{
    ExternalFileExtension, ExternalImportProvider, ExternalImportProviderContext,
    ExternalImportProviderKind, ExternalImportRequest, RequiredRuntimeImport,
    ResolvedExternalImport, RuntimeAssetIdentity,
};
use crate::builder_surface::external_import_providers::registry::ExternalImportProviderRegistry;
use crate::builder_surface::external_import_providers::resolution_table::ExternalImportResolutionTable;
use crate::compiler_frontend::compiler_messages::compiler_errors::CompilerMessages;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::external_packages::{
    ExternalFunctionId, ExternalPackageId, ExternalPackageRegistry, ExternalTypeId,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug)]
struct DummyProvider {
    kind: ExternalImportProviderKind,
    extensions: Vec<ExternalFileExtension>,
}

impl ExternalImportProvider for DummyProvider {
    fn kind(&self) -> ExternalImportProviderKind {
        self.kind.clone()
    }

    fn supported_extensions(&self) -> &[ExternalFileExtension] {
        &self.extensions
    }

    fn resolve_external_import(
        &self,
        _request: ExternalImportRequest,
        context: &mut ExternalImportProviderContext,
    ) -> Result<Option<ResolvedExternalImport>, CompilerMessages> {
        let package_id = context
            .package_registry
            .register_package(
                "@test/dummy",
                crate::builder_surface::PackageOrigin::Builder,
            )
            .expect("test package registration should succeed");

        Ok(Some(ResolvedExternalImport {
            package_id,
            exported_types: vec![ExternalTypeId(1)],
            exported_free_functions: vec![ExternalFunctionId::Synthetic(2)],
            runtime_asset: Some(RuntimeAssetIdentity {
                canonical_source_path: PathBuf::from("/test/asset.js"),
                asset_kind: "js".to_owned(),
            }),
            diagnostics: vec![],
            required_runtime_imports: vec![RequiredRuntimeImport {
                module_name: "@moth/runtime".to_owned(),
                imported_names: vec!["mothOk".to_owned(), "mothErr".to_owned()],
            }],
        }))
    }
}

// ------------------------------
//  BuilderSurface initialization
// ------------------------------

#[test]
fn builder_surface_with_mandatory_core_has_no_external_import_providers() {
    let builder_surface = BuilderSurface::with_mandatory_core();

    assert!(builder_surface.external_import_providers.is_empty());
}

// ------------------------------
//  Provider registration
// ------------------------------

#[test]
fn registering_provider_makes_extension_discoverable() {
    let mut registry = ExternalImportProviderRegistry::empty();
    let provider = Arc::new(DummyProvider {
        kind: "js".into(),
        extensions: vec!["js".into()],
    });

    registry.register(provider);

    assert!(registry.supports_extension("js"));
    assert!(!registry.supports_extension("wit"));
}

#[test]
fn find_by_extension_returns_matching_provider() {
    let mut registry = ExternalImportProviderRegistry::empty();
    let provider = Arc::new(DummyProvider {
        kind: "js".into(),
        extensions: vec!["js".into(), "mjs".into()],
    });

    registry.register(provider);

    let found = registry.find_by_extension("mjs");
    assert!(found.is_some());
    assert_eq!(found.unwrap().kind(), ExternalImportProviderKind::new("js"));
}

// ------------------------------
//  Clone behavior
// ------------------------------

#[test]
fn provider_registry_is_cloneable() {
    let mut registry = ExternalImportProviderRegistry::empty();
    let provider = Arc::new(DummyProvider {
        kind: "js".into(),
        extensions: vec!["js".into()],
    });

    registry.register(provider);
    let cloned = registry.clone();

    assert_eq!(cloned.len(), 1);
    assert!(cloned.supports_extension("js"));
}

#[test]
fn builder_surface_with_providers_is_cloneable() {
    let mut builder_surface = BuilderSurface::with_mandatory_core();
    let provider = Arc::new(DummyProvider {
        kind: "js".into(),
        extensions: vec!["js".into()],
    });

    builder_surface.external_import_providers.register(provider);
    let cloned = builder_surface.clone();

    assert!(cloned.external_import_providers.supports_extension("js"));
}

// ------------------------------
//  Cache key behavior
// ------------------------------

#[test]
fn cache_key_distinguishes_provider_kinds_and_matches_identical_keys() {
    let path = PathBuf::from("/project/lib/helper.js");

    let key_js = ExternalImportCacheKey {
        canonical_source_path: path.clone(),
        provider_kind: "js".into(),
    };
    let key_wit = ExternalImportCacheKey {
        canonical_source_path: path.clone(),
        provider_kind: "wit".into(),
    };

    assert_ne!(key_js, key_wit);

    // Same canonical path and provider kind must produce equal keys so repeated
    // imports of one file reuse a single cache entry.
    let key_js_again = ExternalImportCacheKey {
        canonical_source_path: path,
        provider_kind: "js".into(),
    };
    assert_eq!(key_js, key_js_again);
}

// ------------------------------
//  Provider resolution result
// ------------------------------

#[test]
fn dummy_provider_resolves_import_with_all_fields() {
    let provider = Arc::new(DummyProvider {
        kind: "js".into(),
        extensions: vec!["js".into()],
    });
    let mut registry = ExternalPackageRegistry::default();
    let mut cache = ExternalImportProviderCache::new();
    let mut string_table = StringTable::new();
    let mut context = ExternalImportProviderContext {
        package_registry: &mut registry,
        cache: &mut cache,
        string_table: &mut string_table,
    };

    let request = ExternalImportRequest {
        import_path: "@test/dummy".to_owned(),
        canonical_source_path: PathBuf::from("/test/dummy.js"),
        source_location: SourceLocation::default(),
    };

    let result = provider.resolve_external_import(request, &mut context);

    let resolved = result
        .expect("resolution should succeed")
        .expect("resolution should return Some");

    assert_eq!(resolved.exported_types.len(), 1);
    assert_eq!(resolved.exported_free_functions.len(), 1);
    assert!(resolved.runtime_asset.is_some());
    assert_eq!(resolved.required_runtime_imports.len(), 1);
}

// ------------------------------
//  Resolution table indexing
// ------------------------------

fn resolution_table_import(package_id: u32) -> ResolvedExternalImport {
    ResolvedExternalImport {
        package_id: ExternalPackageId(package_id),
        exported_types: vec![ExternalTypeId(package_id)],
        exported_free_functions: vec![ExternalFunctionId::Synthetic(package_id)],
        runtime_asset: None,
        diagnostics: vec![],
        required_runtime_imports: vec![],
    }
}

#[test]
fn resolution_table_deduplicates_repeated_imports_from_same_source_file() {
    let mut table = ExternalImportResolutionTable::new();
    let resolved = resolution_table_import(1);

    // Same source file, two different prefixes, same package.
    table.insert("src/main.moth", "@./helper.js", resolved.clone());
    table.insert("src/main.moth", "@./helper", resolved);

    let result =
        table.collect_unique_resolved_imports_for_source_files(&["src/main.moth".to_owned()]);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].package_id, ExternalPackageId(1));
}

#[test]
fn resolution_table_replacing_same_source_prefix_updates_collected_package() {
    let mut table = ExternalImportResolutionTable::new();

    table.insert("src/main.moth", "@./helper.js", resolution_table_import(1));
    table.insert("src/main.moth", "@./helper.js", resolution_table_import(2));

    let result =
        table.collect_unique_resolved_imports_for_source_files(&["src/main.moth".to_owned()]);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].package_id, ExternalPackageId(2));

    let retrieved = table
        .get("src/main.moth", "@./helper.js")
        .expect("replacement should keep exact lookup available");
    assert_eq!(retrieved.package_id, ExternalPackageId(2));
    assert_eq!(retrieved.exported_types, vec![ExternalTypeId(2)]);
    assert_eq!(
        retrieved.exported_free_functions,
        vec![ExternalFunctionId::Synthetic(2)]
    );
}

#[test]
fn resolution_table_collects_different_packages_from_different_source_files() {
    let mut table = ExternalImportResolutionTable::new();

    // Same prefix, different source files, different packages.
    table.insert("src/main.moth", "@./lib.js", resolution_table_import(1));
    table.insert("src/other.moth", "@./lib.js", resolution_table_import(2));

    let result = table.collect_unique_resolved_imports_for_source_files(&[
        "src/main.moth".to_owned(),
        "src/other.moth".to_owned(),
    ]);

    assert_eq!(result.len(), 2);
    let package_ids: Vec<_> = result.iter().map(|r| r.package_id).collect();
    assert!(package_ids.contains(&ExternalPackageId(1)));
    assert!(package_ids.contains(&ExternalPackageId(2)));
}

#[test]
fn resolution_table_orders_results_by_package_id_regardless_of_insert_order() {
    let mut table = ExternalImportResolutionTable::new();

    // Insert out of order.
    table.insert("src/a.moth", "@./a.js", resolution_table_import(5));
    table.insert("src/b.moth", "@./b.js", resolution_table_import(3));
    table.insert("src/c.moth", "@./c.js", resolution_table_import(7));

    let result = table.collect_unique_resolved_imports_for_source_files(&[
        "src/a.moth".to_owned(),
        "src/b.moth".to_owned(),
        "src/c.moth".to_owned(),
    ]);

    assert_eq!(result.len(), 3);
    assert_eq!(result[0].package_id, ExternalPackageId(3));
    assert_eq!(result[1].package_id, ExternalPackageId(5));
    assert_eq!(result[2].package_id, ExternalPackageId(7));
}

// ------------------------------
//  Cache operations
// ------------------------------

#[test]
fn cache_stores_and_retrieves_resolved_import() {
    let mut cache = ExternalImportProviderCache::new();
    let key = ExternalImportCacheKey {
        canonical_source_path: PathBuf::from("/test/lib.js"),
        provider_kind: "js".into(),
    };
    let resolved = ResolvedExternalImport {
        package_id: ExternalPackageId(1),
        exported_types: vec![],
        exported_free_functions: vec![],
        runtime_asset: None,
        diagnostics: vec![],
        required_runtime_imports: vec![],
    };

    assert!(!cache.contains_key(&key));
    cache.insert(key.clone(), resolved.clone());
    assert!(cache.contains_key(&key));

    let retrieved = cache.get(&key).expect("should retrieve cached entry");
    assert_eq!(retrieved.package_id, ExternalPackageId(1));
}
