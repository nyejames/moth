use super::*;

use crate::build_system::build::{
    GeneratedFunctionSidecar, Module, ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts,
    ModuleRootActivity,
};
use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::HirModuleLinkFacts;
use crate::compiler_frontend::public_call_summary::FunctionReturnAliasSummary;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModulePrivateExecutableCategory, ModulePrivateExecutableIdentity,
    ModuleRootRole, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};
use std::path::PathBuf;
use std::sync::Arc;

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

fn test_module() -> Module {
    Module {
        executable: ModuleExecutable {
            hir: HirModule::new(),
            type_environment: TypeEnvironment::new(),
            borrow_analysis: BorrowCheckReport::default(),
        },
        link_facts: ModuleLinkFacts {
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            external_import_candidates: Vec::new(),
            functions: HirModuleLinkFacts::default(),
        },
        metadata: ModuleCompilerMetadata {
            entry_point: PathBuf::new(),
            warnings: Vec::new(),
            const_top_level_fragments: Vec::new(),
            root_activity: ModuleRootActivity::default(),
            doc_fragments: Vec::new(),
            rendered_path_usages: Vec::new(),
            materialisation_context: None,
        },
    }
}

fn test_sidecar(
    identity: GeneratedFunctionIdentity,
    summary: PublicCallSummary,
) -> GeneratedFunctionSidecar {
    let mut module = test_module();
    module.executable.hir.functions.push(HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: Vec::new(),
        return_type: TypeId(0),
    });
    module
        .executable
        .hir
        .function_ids_by_generated
        .insert(identity.clone(), FunctionId(0));
    module
        .executable
        .borrow_analysis
        .analysis
        .public_call_summaries
        .insert(FunctionId(0), summary);
    GeneratedFunctionSidecar::new(identity, module)
}

fn store_with(
    identity: GeneratedFunctionIdentity,
    summary: PublicCallSummary,
) -> BoundaryGeneratedFunctionStore {
    let mut store = BoundaryGeneratedFunctionStore::default();
    store.push_completed_for_test(CompletedGeneratedFunction {
        identity: identity.clone(),
        summary: summary.clone(),
        sidecar: test_sidecar(identity, summary),
    });
    store
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
    let known = BoundaryGeneratedFunctionStore::default();
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
    let known = BoundaryGeneratedFunctionStore::default();
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
    let known = store_with(identity.clone(), expected.clone());
    let mut worklist = GeneratedFunctionWorklist::new(&known);

    let ids = worklist.register_module_requests(&module_origin(), [facts("known")]);

    assert!(ids.is_empty());
    assert_eq!(worklist.summary(&identity), Some(&expected));
    assert!(worklist.finish().is_ok());
}

#[test]
fn session_allocates_only_new_records() {
    let known_identity = generated_identity("known");
    let known = store_with(known_identity, summary());
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
    let known = BoundaryGeneratedFunctionStore::default();
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
fn every_completed_record_has_one_identity_summary_and_sidecar() {
    let identity = generated_identity("record");
    let expected = summary();
    let store = store_with(identity.clone(), expected.clone());

    assert_eq!(store.records.len(), 1);
    assert_eq!(store.by_identity.len(), 1);
    assert_eq!(store.sidecars().count(), 1);
    assert_eq!(store.summary(&identity), Some(&expected));
    assert_eq!(store.sidecar_at(0).unwrap().identity, identity);
}

#[test]
fn equal_generated_identities_publish_across_independent_boundaries() {
    let identity = generated_identity("shared");
    let mut first_store = BoundaryGeneratedFunctionStore::default();
    first_store
        .publish(GeneratedFunctionWorklistDelta {
            records: vec![CompletedGeneratedFunction {
                identity: identity.clone(),
                summary: summary(),
                sidecar: test_sidecar(identity.clone(), summary()),
            }],
        })
        .unwrap();
    let mut second_store = BoundaryGeneratedFunctionStore::default();
    second_store
        .publish(GeneratedFunctionWorklistDelta {
            records: vec![CompletedGeneratedFunction {
                identity: identity.clone(),
                summary: summary(),
                sidecar: test_sidecar(identity.clone(), summary()),
            }],
        })
        .unwrap();

    assert_eq!(first_store.sidecars().count(), 1);
    assert_eq!(second_store.sidecars().count(), 1);
    assert_eq!(first_store.sidecar_at(0).unwrap().identity, identity);
    assert_eq!(second_store.sidecar_at(0).unwrap().identity, identity);
}

#[test]
fn session_does_not_suppress_requests_known_only_in_another_boundary() {
    let identity = generated_identity("shared");
    let other_boundary = store_with(identity.clone(), summary());
    let local_boundary = BoundaryGeneratedFunctionStore::default();
    let mut local_worklist = GeneratedFunctionWorklist::new(&local_boundary);

    let ids = local_worklist.register_module_requests(&module_origin(), [facts("shared")]);

    assert_eq!(
        ids.len(),
        1,
        "an identity completed in another boundary must still materialise locally"
    );
    assert!(local_worklist.summary(&identity).is_none());
    assert!(other_boundary.summary(&identity).is_some());
}

#[test]
fn session_summary_lookup_stays_inside_its_own_boundary() {
    let identity = generated_identity("shared");
    let first_summary = summary();
    let mut second_summary = summary();
    second_summary.parameters.push(
        crate::compiler_frontend::public_call_summary::PublicCallParameterSummary {
            access: crate::compiler_frontend::public_call_summary::PublicCallParameterAccess::Shared,
            mutation: crate::compiler_frontend::public_call_summary::PublicCallMutationEffect::NoWrite,
            transfer_eligibility: crate::compiler_frontend::public_call_summary::PublicCallTransferEligibility::Ineligible,
            transfer_effect: crate::compiler_frontend::public_call_summary::PublicCallTransferEffect::NeverConsumes,
            reactive_effect: crate::compiler_frontend::public_call_summary::PublicCallReactiveEffect::None,
        },
    );
    let first_store = store_with(identity.clone(), first_summary.clone());
    let second_store = store_with(identity.clone(), second_summary.clone());

    let first_worklist = GeneratedFunctionWorklist::new(&first_store);
    let second_worklist = GeneratedFunctionWorklist::new(&second_store);

    assert_eq!(first_worklist.summary(&identity), Some(&first_summary));
    assert_eq!(second_worklist.summary(&identity), Some(&second_summary));
    assert_ne!(
        first_worklist.summary(&identity),
        second_worklist.summary(&identity),
        "equal identities in unrelated boundaries must not share summaries"
    );
}

#[test]
fn boundary_rejects_publishing_the_same_generated_identity_twice() {
    let identity = generated_identity("duplicate");
    let mut boundary = BoundaryGeneratedFunctionStore::default();
    boundary
        .publish(GeneratedFunctionWorklistDelta {
            records: vec![CompletedGeneratedFunction {
                identity: identity.clone(),
                summary: summary(),
                sidecar: test_sidecar(identity.clone(), summary()),
            }],
        })
        .unwrap();

    let error = boundary
        .publish(GeneratedFunctionWorklistDelta {
            records: vec![CompletedGeneratedFunction {
                identity: identity.clone(),
                summary: summary(),
                sidecar: test_sidecar(identity, summary()),
            }],
        })
        .unwrap_err();

    assert!(error.msg.contains("published more than once"));
}

#[test]
fn late_generated_duplicate_leaves_existing_owners_unchanged() {
    let existing = generated_identity("existing");
    let late_duplicate = generated_identity("late");
    let mut store = BoundaryGeneratedFunctionStore::default();
    store
        .publish(GeneratedFunctionWorklistDelta {
            records: vec![CompletedGeneratedFunction {
                identity: existing.clone(),
                summary: summary(),
                sidecar: test_sidecar(existing.clone(), summary()),
            }],
        })
        .unwrap();
    let records_before = store.records.len();
    let sidecars_before = store.sidecars().count();

    let error = store
        .publish(GeneratedFunctionWorklistDelta {
            records: vec![
                CompletedGeneratedFunction {
                    identity: late_duplicate.clone(),
                    summary: summary(),
                    sidecar: test_sidecar(late_duplicate.clone(), summary()),
                },
                CompletedGeneratedFunction {
                    identity: late_duplicate.clone(),
                    summary: summary(),
                    sidecar: test_sidecar(late_duplicate.clone(), summary()),
                },
            ],
        })
        .unwrap_err();

    assert!(
        error
            .msg
            .contains("duplicated inside one publication delta")
    );
    assert_eq!(
        store.records.len(),
        records_before,
        "a failing delta must not append any row"
    );
    assert_eq!(store.sidecars().count(), sidecars_before);
    assert!(store.summary(&existing).is_some());
    assert!(store.by_identity.len() == 1);
}

#[test]
fn sidecar_record_identity_disagreement_leaves_store_unchanged() {
    let record_identity = generated_identity("record");
    let other_identity = generated_identity("other");
    let mut store = BoundaryGeneratedFunctionStore::default();

    let error = store
        .publish(GeneratedFunctionWorklistDelta {
            records: vec![CompletedGeneratedFunction {
                identity: record_identity,
                summary: summary(),
                sidecar: test_sidecar(other_identity, summary()),
            }],
        })
        .unwrap_err();

    assert!(error.msg.contains("disagrees with its record identity"));
    assert!(store.records.is_empty());
    assert!(store.by_identity.is_empty());
    assert_eq!(store.sidecars().count(), 0);
}
