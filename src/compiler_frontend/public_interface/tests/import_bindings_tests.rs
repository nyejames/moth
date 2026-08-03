//! Focused tests for the shell-identity provider binding index.
//!
//! WHAT: proves `SourceProviderImportSet` resolves retained import shells by `ImportShellId`
//!       and never by path text, so bindings whose authored paths share a suffix cannot
//!       cross-address each other and a shell without a binding never enters the provider map.
//! WHY: R5C2 removes path-component and suffix joins from provider binding; these hidden join
//!       invariants are not observable through end-to-end output.

use super::super::{PublicSemanticInterface, SourceProviderImport, SourceProviderImportSet};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::identity::{FileId, ImportShellId};

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

#[test]
fn same_suffix_bindings_resolve_by_shell_identity_only() {
    let provider = provider_interface("provider");
    let nested = provider_interface("nested");

    // Both bindings share the same authored-path suffix shape in the old matcher
    // (`nested/provider/item` ends with `provider/item`). Only the shell identity separates
    // them, and the lookup must never fall back to path text.
    let set = SourceProviderImportSet::new(vec![
        SourceProviderImport {
            import_shell_id: Some(ImportShellId::new(Some(FileId(1)), 0)),
            import_prefix: None,
            implicit_template_scope: false,
            interface: &provider,
        },
        SourceProviderImport {
            import_shell_id: Some(ImportShellId::new(Some(FileId(2)), 0)),
            import_prefix: None,
            implicit_template_scope: false,
            interface: &nested,
        },
    ]);

    assert!(std::ptr::eq(
        set.resolve(ImportShellId::new(Some(FileId(1)), 0))
            .expect("shell one must bind the provider"),
        &provider,
    ));
    assert!(std::ptr::eq(
        set.resolve(ImportShellId::new(Some(FileId(2)), 0))
            .expect("shell two must bind the nested provider"),
        &nested,
    ));
    assert!(
        set.resolve(ImportShellId::new(Some(FileId(1)), 1))
            .is_none()
    );
}

#[test]
fn grouped_reexport_lookup_uses_the_same_shell_identity() {
    let provider = provider_interface("provider");
    let set = SourceProviderImportSet::new(vec![SourceProviderImport {
        import_shell_id: Some(ImportShellId::new(Some(FileId(3)), 0)),
        import_prefix: None,
        implicit_template_scope: false,
        interface: &provider,
    }]);

    assert!(std::ptr::eq(
        set.resolve_reexport(ImportShellId::new(Some(FileId(3)), 0))
            .expect("the grouped re-export shell must resolve"),
        &provider,
    ));
    assert!(
        set.resolve_reexport(ImportShellId::new(Some(FileId(3)), 1))
            .is_none(),
        "a different shell ordinal must not borrow the re-export binding"
    );
}

#[test]
fn unbound_shell_stays_outside_the_provider_map() {
    let provider = provider_interface("provider");
    let set = SourceProviderImportSet::new(vec![SourceProviderImport {
        import_shell_id: Some(ImportShellId::new(Some(FileId(4)), 0)),
        import_prefix: None,
        implicit_template_scope: false,
        interface: &provider,
    }]);

    // A same-module import has no provider edge, so its shell must never resolve to a provider
    // interface merely because a provider binding exists in the same module.
    assert!(
        set.resolve(ImportShellId::new(Some(FileId(4)), 5))
            .is_none(),
        "unbound same-module shells must stay local compiler bindings"
    );
}

#[test]
fn implicit_scope_bindings_have_no_shell_and_keep_their_prefix() {
    let provider = provider_interface("html");
    let set = SourceProviderImportSet::new(vec![SourceProviderImport {
        import_shell_id: None,
        import_prefix: Some("html"),
        implicit_template_scope: true,
        interface: &provider,
    }]);

    let implicit: Vec<_> = set.implicit_template_scope_interfaces().collect();
    assert_eq!(implicit.len(), 1);
    assert_eq!(implicit[0].0, "html");
    assert!(std::ptr::eq(implicit[0].1, &provider));
    assert!(
        set.resolve(ImportShellId::new(Some(FileId(0)), 0))
            .is_none(),
        "implicit scope must never satisfy an explicit authored shell"
    );
}
