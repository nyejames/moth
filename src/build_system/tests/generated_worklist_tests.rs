use super::*;

use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::public_call_summary::FunctionReturnAliasSummary;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModulePrivateExecutableCategory, ModulePrivateExecutableIdentity,
    ModuleRootRole, StablePackageIdentity,
};

fn module_origin() -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("worklist-tests"),
        "main".to_owned(),
        ModuleRootRole::Normal,
    )
}

fn generated_identity(name: &str) -> GeneratedFunctionIdentity {
    GeneratedFunctionIdentity::new(
        GeneratedDeclarationIdentity::ModulePrivate(ModulePrivateExecutableIdentity::new(
            module_origin(),
            "@page.moth".to_owned(),
            ModulePrivateExecutableCategory::GenericFunction,
            name.to_owned(),
            None,
        )),
        Box::new([CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int)]),
        Box::new([]),
    )
}

fn summary() -> PublicCallSummary {
    PublicCallSummary {
        parameters: Vec::new(),
        return_alias: FunctionReturnAliasSummary::Fresh,
    }
}

#[test]
fn registration_sorts_and_deduplicates_stable_identities_before_assigning_dense_ids() {
    let alpha = generated_identity("alpha");
    let beta = generated_identity("beta");
    let mut worklist = GeneratedFunctionWorklist::new(FxHashMap::default());

    let ids =
        worklist.register_module_requests(&module_origin(), [beta.clone(), alpha.clone(), beta]);

    assert_eq!(ids, vec![GeneratedRequestId(0), GeneratedRequestId(1)]);
    assert_eq!(worklist.identity(ids[0]).unwrap(), &alpha);
    assert_eq!(worklist.records.len(), 2);
}

#[test]
fn duplicate_requesters_and_dependency_edges_are_recorded_once() {
    let parent = generated_identity("parent");
    let child = generated_identity("child");
    let mut worklist = GeneratedFunctionWorklist::new(FxHashMap::default());
    let parent_id =
        worklist.register_module_requests(&module_origin(), [parent.clone(), parent])[0];

    let child_ids = worklist
        .register_generated_requests(parent_id, [child.clone(), child.clone(), child.clone()]);
    worklist.register_generated_requests(parent_id, [child]);

    assert_eq!(child_ids.len(), 1);
    assert_eq!(worklist.records[parent_id.index()].dependencies, child_ids);
    assert_eq!(worklist.records[child_ids[0].index()].requesters.len(), 1);
}

#[test]
fn completed_boundary_summary_suppresses_rematerialisation() {
    let identity = generated_identity("known");
    let expected = summary();
    let mut known = FxHashMap::default();
    known.insert(identity.clone(), expected.clone());
    let mut worklist = GeneratedFunctionWorklist::new(known);

    let ids = worklist.register_module_requests(&module_origin(), [identity.clone()]);

    assert!(ids.is_empty());
    assert_eq!(worklist.summary(&identity), Some(&expected));
    assert!(worklist.finish().is_ok());
}

#[test]
fn boundary_rejects_publishing_the_same_generated_identity_twice() {
    let identity = generated_identity("duplicate");
    let mut boundary = BoundaryGeneratedFunctionStore::default();
    let mut first = FxHashMap::default();
    first.insert(identity.clone(), summary());
    boundary
        .publish(GeneratedFunctionWorklistDelta {
            summaries: first,
            sidecars: Vec::new(),
        })
        .unwrap();
    let mut duplicate = FxHashMap::default();
    duplicate.insert(identity, summary());

    let error = boundary
        .publish(GeneratedFunctionWorklistDelta {
            summaries: duplicate,
            sidecars: Vec::new(),
        })
        .unwrap_err();

    assert!(error.msg.contains("published more than once"));
}
