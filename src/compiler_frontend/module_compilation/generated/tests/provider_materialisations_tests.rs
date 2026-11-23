//! Focused tests for the boundary provider materialisation registry.
//!
//! WHAT: which declaring context one generated request resolves to — a published provider, the
//!       requester's own preparation, or nothing — and whether a cross-lane collision is diagnosed.
//! WHY: a colliding project-module publication must be diagnosed without replacing the package
//!      seed. The focused test checks both outcomes.

use super::*;
use crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationContext;
use crate::compiler_frontend::module_compilation::generated::test_fixtures::generated_identity;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModuleRootRole, OriginFunctionId, StableModuleOriginIdentity,
    StablePackageIdentity,
};

use std::sync::Arc;

fn declaration(name: &str) -> GeneratedDeclarationIdentity {
    generated_identity(name).declaration().clone()
}

fn context(names: &[&str]) -> Arc<ModuleMaterialisationContext> {
    Arc::new(ModuleMaterialisationContext::from_identities_for_test(
        names.iter().map(|name| declaration(name)).collect(),
    ))
}

fn const_origin(name: &str) -> OriginFunctionId {
    OriginFunctionId::new_free(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("test"),
            "main".to_owned(),
            ModuleRootRole::Normal,
        ),
        name.to_owned(),
    )
}

fn const_context(origin: &OriginFunctionId) -> Arc<ModuleMaterialisationContext> {
    Arc::new(ModuleMaterialisationContext::from_const_origins_for_test(
        vec![Some(origin.clone())],
    ))
}

#[test]
fn a_published_identity_resolves_to_its_own_declaring_row() {
    let mut registry = ProviderMaterialisationRegistry::default();
    let published = context(&["first", "second"]);
    registry
        .publish_context(&published)
        .expect("each unique declaration identity publishes");

    let resolved = registry
        .published_template(&declaration("second"))
        .expect("a published identity resolves");
    assert_eq!(
        resolved.template_index, 1,
        "the row index must be the declaring context's own position"
    );
    assert!(Arc::ptr_eq(&resolved.context, &published));

    assert!(
        registry
            .published_template(&declaration("absent"))
            .is_none(),
        "an identity nobody published must not resolve"
    );
}

#[test]
fn a_cross_lane_publish_for_the_same_identity_is_diagnosed() {
    // Package seeds are installed before any module publishes. The two lanes' uniqueness proofs
    // do not see each other, so the boundary registry must reject their collision before replacing
    // the package seed.
    let mut registry = ProviderMaterialisationRegistry::default();
    let package_seed = context(&["shared"]);
    let project_module = context(&["shared"]);

    registry
        .publish_context(&package_seed)
        .expect("the package seed publishes");
    let error = registry
        .publish_context(&project_module)
        .expect_err("a different declaring context must be diagnosed");
    assert!(
        error.message.contains("multiple declaring contexts"),
        "collision should identify the duplicate declaring contexts: {}",
        error.message
    );

    let resolved = registry
        .published_template(&declaration("shared"))
        .expect("the original package seed remains published");
    assert!(
        Arc::ptr_eq(&resolved.context, &package_seed),
        "a rejected module publish must not replace the package seed"
    );
}

#[test]
fn a_borrowed_completed_generic_row_rejects_a_different_declaring_context() {
    let package_seed = context(&["shared"]);
    let mut index = CompletedMaterialisationIndex::default();
    index.insert_materialisation(declaration("shared"), Arc::clone(&package_seed), 0, 3);
    let mut registry = ProviderMaterialisationRegistry::default();
    registry.borrow_completed_index(Arc::new(index));

    let project_module = context(&["shared"]);
    let error = registry
        .publish_context(&project_module)
        .expect_err("a borrowed package row must participate in cross-lane collision checks");
    assert!(
        error.message.contains("multiple declaring contexts"),
        "collision should identify the borrowed declaring context: {}",
        error.message
    );
    let resolved = registry
        .published_template(&declaration("shared"))
        .expect("the borrowed package row remains published");
    assert!(Arc::ptr_eq(&resolved.context, &package_seed));
}

#[test]
fn replaying_a_borrowed_completed_generic_row_stays_borrowed() {
    let package_seed = context(&["shared"]);
    let mut index = CompletedMaterialisationIndex::default();
    index.insert_materialisation(declaration("shared"), Arc::clone(&package_seed), 0, 3);
    let mut registry = ProviderMaterialisationRegistry::default();
    registry.borrow_completed_index(Arc::new(index));

    registry
        .publish_context(&package_seed)
        .expect("replaying the same borrowed row is idempotent");
    assert_eq!(
        registry.local_materialisation_count(),
        0,
        "a replayed borrowed row must not be copied into the local registry"
    );
    assert_eq!(registry.borrowed_completed_entry_count(), 1);
}

#[test]
fn a_borrowed_completed_const_row_rejects_a_different_declaring_context() {
    let origin = const_origin("shared");
    let package_seed = const_context(&origin);
    let mut index = CompletedMaterialisationIndex::default();
    index.insert_const_function(origin.clone(), Arc::clone(&package_seed), 0, 4);
    let mut registry = ProviderMaterialisationRegistry::default();
    registry.borrow_completed_index(Arc::new(index));

    let project_module = const_context(&origin);
    let error = registry
        .publish_context(&project_module)
        .expect_err("a borrowed const package row must participate in collision checks");
    assert!(
        error.message.contains("multiple declaring contexts"),
        "collision should identify the borrowed const declaring context: {}",
        error.message
    );
    let resolved = registry
        .const_function(&origin)
        .expect("the borrowed const package row remains published");
    assert!(Arc::ptr_eq(&resolved.context, &package_seed));
}

#[test]
fn replaying_a_borrowed_completed_const_row_stays_borrowed() {
    let origin = const_origin("shared");
    let package_seed = const_context(&origin);
    let mut index = CompletedMaterialisationIndex::default();
    index.insert_const_function(origin.clone(), Arc::clone(&package_seed), 0, 4);
    let mut registry = ProviderMaterialisationRegistry::default();
    registry.borrow_completed_index(Arc::new(index));

    registry
        .publish_context(&package_seed)
        .expect("replaying the same borrowed const row is idempotent");
    assert_eq!(
        registry.local_materialisation_count(),
        0,
        "a replayed borrowed const row must not be copied into the local registry"
    );
    assert_eq!(registry.borrowed_completed_entry_count(), 1);
}

#[test]
fn republishing_the_same_declaring_row_is_idempotent() {
    let mut registry = ProviderMaterialisationRegistry::default();
    let published = context(&["shared"]);

    registry
        .publish_context(&published)
        .expect("the first publication succeeds");
    registry
        .publish_context(&published)
        .expect("the same context and row may be published idempotently");

    let resolved = registry
        .published_template(&declaration("shared"))
        .expect("the idempotent publication remains available");
    assert!(Arc::ptr_eq(&resolved.context, &published));
    assert_eq!(resolved.template_index, 0);
}

#[test]
fn a_context_publish_is_atomic_when_a_later_row_collides() {
    let mut registry = ProviderMaterialisationRegistry::default();
    let package_seed = context(&["shared"]);
    registry
        .publish_context(&package_seed)
        .expect("the package seed publishes");
    let project_module = context(&["fresh", "shared"]);
    let error = registry
        .publish_context(&project_module)
        .expect_err("a later colliding row must reject the whole context");
    assert!(
        error.message.contains("multiple declaring contexts"),
        "collision should identify the duplicate declaring contexts: {}",
        error.message
    );

    assert!(
        registry.published_template(&declaration("fresh")).is_none(),
        "a rejected context must not leave earlier rows published"
    );
    let resolved = registry
        .published_template(&declaration("shared"))
        .expect("the original package seed remains published");
    assert!(Arc::ptr_eq(&resolved.context, &package_seed));
}

#[test]
fn context_publication_preserves_the_identity_domain_rebase_marker() {
    let canonical_context = context(&["canonical"]);
    let mut canonical_registry = ProviderMaterialisationRegistry::default();
    canonical_registry
        .publish_context(&canonical_context)
        .expect("canonical context publication should succeed");
    assert!(
        !canonical_registry
            .published_template(&declaration("canonical"))
            .expect("canonical row should be published")
            .rebase_required,
        "same-boundary publication should not require a cross-domain rebase"
    );

    let deferred_context = context(&["deferred"]);
    let mut deferred_registry = ProviderMaterialisationRegistry::default();
    deferred_registry
        .publish_context_with_rebase_required(&deferred_context, true)
        .expect("deferred context publication should succeed");
    assert!(
        deferred_registry
            .published_template(&declaration("deferred"))
            .expect("deferred row should be published")
            .rebase_required,
        "deferred canonical rows must retain the explicit rebase marker"
    );
}
