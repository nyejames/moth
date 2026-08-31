//! Focused tests for the boundary provider materialisation registry.
//!
//! WHAT: which declaring context one generated request resolves to — a published provider, the
//!       requester's own preparation, or nothing — and whether a cross-lane collision is diagnosed.
//! WHY: the registry previously replaced a package seed when a project module published the same
//!      identity. The focused collision test now proves that duplicate is diagnosed while the
//!      original seed remains authoritative.

use super::*;
use crate::compiler_frontend::ast::generic_functions::ModuleMaterialisationContext;
use crate::compiler_frontend::module_compilation::generated::test_fixtures::generated_identity;
use crate::compiler_frontend::semantic_identity::GeneratedDeclarationIdentity;

use std::sync::Arc;

fn declaration(name: &str) -> GeneratedDeclarationIdentity {
    generated_identity(name).declaration().clone()
}

fn context(names: &[&str]) -> Arc<ModuleMaterialisationContext> {
    Arc::new(ModuleMaterialisationContext::from_identities_for_test(
        names.iter().map(|name| declaration(name)).collect(),
    ))
}

#[test]
fn a_published_identity_resolves_to_its_own_declaring_row() {
    let mut registry = ProviderMaterialisationRegistry::default();
    let published = context(&["first", "second"]);
    for (identity, template_index) in published.declaration_rows() {
        registry
            .publish(identity.clone(), Arc::clone(&published), template_index)
            .expect("each unique declaration identity publishes");
    }

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
        .publish(declaration("shared"), Arc::clone(&package_seed), 0)
        .expect("the package seed publishes");
    let error = registry
        .publish(declaration("shared"), Arc::clone(&project_module), 0)
        .expect_err("a different declaring context must be diagnosed");
    assert!(
        error.msg.contains("multiple declaring contexts"),
        "collision should identify the duplicate declaring contexts: {}",
        error.msg
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
fn republishing_the_same_declaring_row_is_idempotent() {
    let mut registry = ProviderMaterialisationRegistry::default();
    let published = context(&["shared"]);

    registry
        .publish(declaration("shared"), Arc::clone(&published), 0)
        .expect("the first publication succeeds");
    registry
        .publish(declaration("shared"), Arc::clone(&published), 0)
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
        .publish(declaration("shared"), Arc::clone(&package_seed), 0)
        .expect("the package seed publishes");

    let project_module = context(&["fresh", "shared"]);
    let error = registry
        .publish_context(&project_module)
        .expect_err("a later colliding row must reject the whole context");
    assert!(
        error.msg.contains("multiple declaring contexts"),
        "collision should identify the duplicate declaring contexts: {}",
        error.msg
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
