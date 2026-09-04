//! Focused tests for the exact provider interface table and shell-identity binding.
//!
//! WHAT: proves `SourceProviderDependencySet` resolves retained dependency shells by `DependencyShellId`,
//!       assigns one dense `ProviderInterfaceId` per unique interface, rejects duplicate shells
//!       and implicit scopes, and fails deterministically when equal module origins disagree.
//! WHY: R5C2A replaces optional shell/prefix/boolean provider states and module-origin cache
//!       keys with one exact build-local provider identity; these hidden join invariants are
//!       not observable through end-to-end output.

use super::super::{
    ProviderDependencyKind, ProviderInterfaceId, PublicSemanticInterface, SourceProviderDependency,
    SourceProviderDependencySet,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::identity::DependencyShellId;

fn provider_interface(name: &str) -> PublicSemanticInterface {
    PublicSemanticInterface {
        module_origin: StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local(name),
            format!("{name}/@mod.moth"),
            ModuleRootRole::Normal,
        ),
        export_bindings: Vec::new(),
        export_diagnostic_provenance: Vec::new(),
        binding_exports: Vec::new(),
        declarations: Vec::new(),
        reusable_evidence: Vec::new(),
        concrete_call_summaries: Vec::new(),
    }
}

fn authored(
    shell_id: DependencyShellId,
    interface: &PublicSemanticInterface,
) -> SourceProviderDependency<'_> {
    SourceProviderDependency {
        kind: ProviderDependencyKind::Authored { shell: shell_id },
        interface,
    }
}

fn implicit<'a>(
    package_prefix: &'a str,
    interface: &'a PublicSemanticInterface,
) -> SourceProviderDependency<'a> {
    SourceProviderDependency {
        kind: ProviderDependencyKind::ImplicitTemplate { package_prefix },
        interface,
    }
}

#[test]
fn same_suffix_bindings_resolve_by_shell_identity_only() {
    let provider = provider_interface("provider");
    let nested = provider_interface("nested");

    // Both bindings share the same authored-path suffix shape in the old matcher
    // (`nested/provider/item` ends with `provider/item`). Only the shell identity separates
    // them, and the lookup must never fall back to path text.
    let set = SourceProviderDependencySet::new(vec![
        authored(
            DependencyShellId::new(SourceId::from_index(1), 0),
            &provider,
        ),
        authored(DependencyShellId::new(SourceId::from_index(2), 0), &nested),
    ])
    .expect("distinct shells should register");

    let first = set
        .resolve_clause(DependencyShellId::new(SourceId::from_index(1), 0))
        .expect("shell one must bind the provider");
    let second = set
        .resolve_clause(DependencyShellId::new(SourceId::from_index(2), 0))
        .expect("shell two must bind the nested provider");
    assert!(std::ptr::eq(
        set.interface(first.provider)
            .expect("provider one resolves"),
        &provider,
    ));
    assert!(std::ptr::eq(
        set.interface(second.provider)
            .expect("provider two resolves"),
        &nested,
    ));
    assert!(
        set.resolve_clause(DependencyShellId::new(SourceId::from_index(1), 1))
            .is_none()
    );
}

#[test]
fn duplicate_shell_fails_instead_of_overwriting() {
    let provider = provider_interface("provider");
    let nested = provider_interface("nested");

    let error = SourceProviderDependencySet::new(vec![
        SourceProviderDependency {
            kind: ProviderDependencyKind::Authored {
                shell: DependencyShellId::new(SourceId::from_index(3), 0),
            },
            interface: &provider,
        },
        SourceProviderDependency {
            kind: ProviderDependencyKind::Authored {
                shell: DependencyShellId::new(SourceId::from_index(3), 0),
            },
            interface: &nested,
        },
    ])
    .expect_err("one retained shell must not resolve to two provider interfaces");

    assert!(error.msg.contains("resolved dependency shell"));
}

#[test]
fn one_shell_resolves_one_provider_surface() {
    let provider = provider_interface("provider");
    let shell = DependencyShellId::new(SourceId::from_index(3), 0);
    let set = SourceProviderDependencySet::new(vec![authored(shell, &provider)])
        .expect("one authored clause should register one provider surface");
    let provider_id = set
        .resolve_clause(shell)
        .expect("the clause shell binds the provider");
    assert!(std::ptr::eq(
        set.interface(provider_id.provider)
            .expect("provider resolves"),
        &provider,
    ));
}

#[test]
fn duplicate_implicit_template_scope_fails() {
    let provider = provider_interface("html");

    let error = SourceProviderDependencySet::new(vec![
        implicit("html", &provider),
        implicit("html", &provider),
    ])
    .expect_err("one implicit scope prefix must not register twice");

    assert!(error.msg.contains("implicit template scope @html"));
}

#[test]
fn equal_origins_with_differing_interfaces_fail_in_either_input_order() {
    let first = provider_interface("shared");
    let mut second = provider_interface("shared");
    second.export_bindings.push(
        crate::compiler_frontend::semantic_identity::ExportBinding::new(
            second.module_origin.clone(),
            "extra".to_owned(),
            crate::compiler_frontend::semantic_identity::OriginDeclarationId::Constant(
                crate::compiler_frontend::semantic_identity::OriginConstantId::new(
                    second.module_origin.clone(),
                    "extra".to_owned(),
                ),
            ),
        ),
    );

    for order in [vec![&first, &second], vec![&second, &first]] {
        let error = SourceProviderDependencySet::new(
            order
                .into_iter()
                .enumerate()
                .map(|(index, interface)| {
                    authored(
                        DependencyShellId::new(SourceId::from_index(4), index as u32),
                        interface,
                    )
                })
                .collect(),
        )
        .expect_err("equal origins with different contents must fail");

        assert!(
            error
                .msg
                .contains("disagrees with an equal-origin provider interface")
        );
    }
}

#[test]
fn exact_repeated_interfaces_receive_one_provider_id() {
    let provider = provider_interface("provider");

    let set = SourceProviderDependencySet::new(vec![
        authored(
            DependencyShellId::new(SourceId::from_index(5), 0),
            &provider,
        ),
        authored(
            DependencyShellId::new(SourceId::from_index(5), 1),
            &provider,
        ),
    ])
    .expect("exact repeated provider interfaces should collapse");

    let first = set
        .resolve_clause(DependencyShellId::new(SourceId::from_index(5), 0))
        .expect("first shell resolves");
    let second = set
        .resolve_clause(DependencyShellId::new(SourceId::from_index(5), 1))
        .expect("second shell resolves");
    assert_eq!(first.provider, second.provider);
    assert_eq!(
        set.interfaces().count(),
        1,
        "one unique provider interface must receive one provider id"
    );
}

#[test]
fn grouped_reexport_lookup_uses_the_same_shell_identity() {
    let provider = provider_interface("provider");
    let set = SourceProviderDependencySet::new(vec![authored(
        DependencyShellId::new(SourceId::from_index(6), 0),
        &provider,
    )])
    .expect("one authored provider should register");

    let provider_id = set
        .resolve_reexport(DependencyShellId::new(SourceId::from_index(6), 0))
        .expect("the direct-selection re-export shell must resolve");
    assert!(std::ptr::eq(
        set.interface(provider_id).expect("provider resolves"),
        &provider,
    ));
    assert!(
        set.resolve_reexport(DependencyShellId::new(SourceId::from_index(6), 1))
            .is_none(),
        "a different shell ordinal must not borrow the re-export binding"
    );
}

#[test]
fn unbound_shell_stays_outside_the_provider_map() {
    let provider = provider_interface("provider");
    let set = SourceProviderDependencySet::new(vec![authored(
        DependencyShellId::new(SourceId::from_index(7), 0),
        &provider,
    )])
    .expect("one authored provider should register");

    assert!(
        set.resolve_clause(DependencyShellId::new(SourceId::from_index(7), 5))
            .is_none(),
        "unbound same-module shells must stay local compiler bindings"
    );
}

#[test]
fn implicit_scope_bindings_have_no_shell_and_keep_their_prefix() {
    let provider = provider_interface("html");
    let set = SourceProviderDependencySet::new(vec![implicit("html", &provider)])
        .expect("one implicit template provider should register");

    let implicit: Vec<_> = set.implicit_template_scope_providers().collect();
    assert_eq!(implicit.len(), 1);
    assert_eq!(implicit[0].0, "html");
    assert!(std::ptr::eq(
        set.interface(implicit[0].1)
            .expect("implicit provider resolves"),
        &provider,
    ));
    assert!(
        set.resolve_clause(DependencyShellId::new(SourceId::from_index(0), 0))
            .is_none(),
        "implicit scope must never satisfy an explicit authored shell"
    );
}

#[test]
fn provider_ids_are_dense_and_stable() {
    let provider = provider_interface("provider");
    let nested = provider_interface("nested");

    let set = SourceProviderDependencySet::new(vec![
        authored(
            DependencyShellId::new(SourceId::from_index(8), 0),
            &provider,
        ),
        authored(DependencyShellId::new(SourceId::from_index(8), 1), &nested),
    ])
    .expect("distinct shells should register");

    let first = set
        .resolve_clause(DependencyShellId::new(SourceId::from_index(8), 0))
        .expect("first shell resolves");
    let second = set
        .resolve_clause(DependencyShellId::new(SourceId::from_index(8), 1))
        .expect("second shell resolves");
    assert!(first.provider != second.provider);
    assert!(set.interface(ProviderInterfaceId::new(0)).is_ok());
    assert!(set.interface(ProviderInterfaceId::new(99)).is_err());
}

#[test]
fn ten_shells_from_one_provider_share_one_binding_view() {
    let provider = provider_interface("provider");

    let set = SourceProviderDependencySet::new(
        (0..10)
            .map(|index| {
                authored(
                    DependencyShellId::new(SourceId::from_index(9), index),
                    &provider,
                )
            })
            .collect(),
    )
    .expect("ten distinct shells should register");

    assert_eq!(
        set.interfaces().count(),
        1,
        "ten shells from one provider must collapse to one provider id"
    );
    let first_view = set
        .binding_view(ProviderInterfaceId::new(0))
        .expect("the provider binding view exists");
    let second_view = set
        .binding_view(ProviderInterfaceId::new(0))
        .expect("the provider binding view exists");
    assert!(
        std::ptr::eq(first_view, second_view),
        "all shells must reuse the one binding view"
    );
}
