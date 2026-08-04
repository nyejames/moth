use super::*;

use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::public_call_summary::FunctionReturnAliasSummary;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModulePrivateExecutableCategory, ModulePrivateExecutableIdentity,
    ModuleRootRole, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};

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

fn facts(name: &str) -> GeneratedRequestFacts {
    GeneratedRequestFacts {
        identity: generated_identity(name),
        display_name: name.to_owned(),
        diagnostic_location: SourceLocation::new(
            crate::compiler_frontend::symbols::interned_path::InternedPath::from_single_str(
                "src/@page.moth",
                &mut StringTable::new(),
            ),
            CharPosition::default(),
            CharPosition::default(),
        ),
    }
}

#[test]
fn registration_sorts_and_deduplicates_stable_identities_before_assigning_dense_ids() {
    let alpha = generated_identity("alpha");
    let beta = generated_identity("beta");
    let known = FxHashMap::default();
    let mut worklist = GeneratedFunctionWorklist::new(&known);

    let ids = worklist.register_module_requests(
        &module_origin(),
        [
            facts("beta"),
            facts("alpha"),
            GeneratedRequestFacts {
                identity: beta,
                display_name: "beta".to_owned(),
                diagnostic_location: SourceLocation::new(
                    crate::compiler_frontend::symbols::interned_path::InternedPath::from_single_str(
                        "src/@page.moth",
                        &mut StringTable::new(),
                    ),
                    CharPosition::default(),
                    CharPosition::default(),
                ),
            },
        ],
    );

    assert_eq!(ids, vec![GeneratedRequestId(0), GeneratedRequestId(1)]);
    assert_eq!(worklist.identity(ids[0]).unwrap(), &alpha);
    assert_eq!(worklist.records.len(), 2);
}

#[test]
fn duplicate_requesters_and_dependency_edges_are_recorded_once() {
    let known = FxHashMap::default();
    let mut worklist = GeneratedFunctionWorklist::new(&known);
    let parent_id =
        worklist.register_module_requests(&module_origin(), [facts("parent"), facts("parent")])[0];

    let child_ids = worklist
        .register_generated_requests(parent_id, [facts("child"), facts("child"), facts("child")]);
    worklist.register_generated_requests(parent_id, [facts("child")]);

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
    let mut worklist = GeneratedFunctionWorklist::new(&known);

    let ids = worklist.register_module_requests(&module_origin(), [facts("known")]);

    assert!(ids.is_empty());
    assert_eq!(worklist.summary(&identity), Some(&expected));
    assert!(worklist.finish().is_ok());
}

#[test]
fn session_allocates_only_new_records() {
    let known_identity = generated_identity("known");
    let mut known = FxHashMap::default();
    known.insert(known_identity, summary());
    let mut worklist = GeneratedFunctionWorklist::new(&known);

    let known_ids = worklist.register_module_requests(&module_origin(), [facts("known")]);
    assert!(known_ids.is_empty());
    assert_eq!(
        worklist.records.len(),
        0,
        "known summaries seed the session"
    );

    let new_ids = worklist.register_module_requests(&module_origin(), [facts("new")]);
    assert_eq!(new_ids.len(), 1);
    assert_eq!(
        worklist.records.len(),
        1,
        "a session owns only its new delta"
    );
}

#[test]
fn request_records_own_diagnostic_facts() {
    let known = FxHashMap::default();
    let mut worklist = GeneratedFunctionWorklist::new(&known);
    let first_location = SourceLocation::new(
        crate::compiler_frontend::symbols::interned_path::InternedPath::from_single_str(
            "src/a.moth",
            &mut StringTable::new(),
        ),
        CharPosition {
            line_number: 3,
            char_column: 5,
        },
        CharPosition {
            line_number: 3,
            char_column: 9,
        },
    );
    let ids = worklist.register_module_requests(
        &module_origin(),
        [GeneratedRequestFacts {
            identity: generated_identity("make"),
            display_name: "make".to_owned(),
            diagnostic_location: first_location.clone(),
        }],
    );

    let (display_name, diagnostic_location) = worklist.request_facts(ids[0]).unwrap();
    assert_eq!(display_name, "make");
    assert_eq!(diagnostic_location, first_location);
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
