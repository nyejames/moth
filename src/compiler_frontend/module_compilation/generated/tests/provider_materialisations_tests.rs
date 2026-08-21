//! Focused tests for the boundary provider materialisation registry.
//!
//! WHAT: which declaring context one generated request resolves to — a published provider, the
//!       requester's own preparation, or nothing — and what a cross-lane collision resolves to.
//! WHY: this registry replaced a lookup that walked the build system's live stores in a fixed
//!       preference order. Nothing else pins that resolution order now: the end-to-end case
//!       `generic_receiver_source_package_facade_success` proves a published provider resolves at
//!       all, but cannot show which context wins when two lanes publish the same identity.

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
        registry.publish(identity.clone(), Arc::clone(&published), template_index);
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
fn a_later_publish_replaces_an_earlier_seed_for_the_same_identity() {
    // Package seeds are installed before any module publishes, and the two lanes' uniqueness
    // proofs do not see each other. `publish` documents that a cross-lane collision therefore
    // reaches the insert and resolves to the module — the same answer the pre-registry lookup
    // gave, and the reason seed-before-publish ordering is load-bearing.
    let mut registry = ProviderMaterialisationRegistry::default();
    let package_seed = context(&["shared"]);
    let project_module = context(&["shared"]);

    registry.publish(declaration("shared"), Arc::clone(&package_seed), 0);
    registry.publish(declaration("shared"), Arc::clone(&project_module), 0);

    let resolved = registry
        .published_template(&declaration("shared"))
        .expect("the colliding identity still resolves");
    assert!(
        Arc::ptr_eq(&resolved.context, &project_module),
        "the module that published last must win, matching the pre-registry lookup order"
    );
}
