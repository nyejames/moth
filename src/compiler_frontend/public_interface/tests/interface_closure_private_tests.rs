//! Private closure-work invariants.
//!
//! WHAT: verifies that private generic arguments enter the closure queue and evidence index
//! without selecting the private generic base.
//! WHY: this exercises the production owner's private traversal callback without placing tests
//! in the implementation file.

use super::*;
use crate::compiler_frontend::canonical_type_identity::ModulePrivateNominalIdentity;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, OriginTypeCategory, OriginTypeId, StableModuleOriginIdentity,
    StablePackageIdentity,
};

#[test]
fn private_generic_arguments_remain_closure_reachable_without_private_base_selection() {
    let module = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("closure-test"),
        "closure".to_owned(),
        ModuleRootRole::Normal,
    );
    let argument = OriginTypeId::new(
        module.clone(),
        "Argument".to_owned(),
        OriginTypeCategory::Struct,
    );
    let identity = CanonicalTypeIdentity::ModulePrivateGenericInstance {
        base: ModulePrivateNominalIdentity::new(
            module,
            "HiddenBox".to_owned(),
            OriginTypeCategory::Struct,
        ),
        arguments: vec![CanonicalTypeIdentity::SourceNominal(argument.clone())].into_boxed_slice(),
    };

    let mut work = ClosureWork::new();
    work.enqueue_type(&identity);
    assert_eq!(
        work.pending.len(),
        1,
        "only the public argument should enter the closure queue",
    );
    assert!(matches!(
        work.pending.front(),
        Some(ClosureWorkItem::Declaration(OriginDeclarationId::Type(origin)))
            if origin == &argument
    ));

    let mut evidence_origins = Vec::new();
    collect_type_origins(&identity, &mut evidence_origins);
    assert_eq!(
        evidence_origins,
        vec![OriginDeclarationId::Type(argument)],
        "evidence indexing must retain private-generic arguments without selecting the base",
    );
}
