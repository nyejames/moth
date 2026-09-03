//! Focused hidden-invariant tests for the reusable evidence projection and evidence
//! identity, visibility and rejection invariants.
//!
//! WHAT: exercises the invariants of [`project_reusable_evidence`] that integration output
//! cannot inspect: stable evidence identity across local `TypeId`, `TraitId`,
//! `TraitRequirementId` and `TraitEvidenceId` allocation, authored-order requirement
//! mappings with exact receiver origins, private-target and private-source-trait
//! exclusion, non-public-receiver-method omission for valid local evidence, builtin-evidence
//! exclusion from direct module drafts, source-canonical core-trait evidence retention,
//! duplicate-stable-key rejection, core-trait-without-classifier rejection,
//! requirement-count-mismatch rejection, wrong-receiver-origin rejection,
//! free-function-origin rejection, duplicate-public-method-match rejection and
//! source-canonical ownership for direct drafts.
//! WHY: these are projection invariants owned by
//! `compiler_frontend::public_interface::evidence_projection`, so they own a focused test
//! beside the module rather than an end-to-end case.

use super::super::{
    EvidenceProjectionContext, PublicDeclarationRecord, PublicDeclarationSemantics,
    PublicEvidenceOwnership, PublicEvidenceRecord, PublicReceiverMethodCategory,
    PublicReceiverMethodSemantics, PublicStructSemantics, TransientNominalOriginResolver,
    project_reusable_evidence,
};
use super::test_support::{
    empty_fields, module_origin, path, register_struct, struct_origin, this_type, trait_origin,
};

use crate::compiler_frontend::ast::statements::functions::ReturnChannel;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalCoreTraitIdentity, CanonicalEvidenceIdentity, CanonicalTraitIdentity,
    CanonicalTypeIdentity, CanonicalTypeProjectionContext,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::FoldedValueGenericParameterResolver;
use crate::compiler_frontend::semantic_identity::{
    OriginDeclarationId, OriginFunctionId, OriginTraitId, OriginTypeCategory, OriginTypeId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::traits::definitions::{
    ResolvedTraitDefinition, ResolvedTraitRequirement, ResolvedTraitReturn,
    TraitReceiverRequirement, TraitVisibility,
};
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::traits::evidence::environment::{
    TraitEvidenceDefinition, TraitEvidenceKind, TraitRequirementEvidence,
};
use crate::compiler_frontend::traits::ids::{TraitEvidenceId, TraitId, TraitRequirementId};

use rustc_hash::FxHashMap;

fn trait_definition(
    trait_id: TraitId,
    name: &str,
    this_type: TypeId,
    start_requirement_id: u32,
    requirement_specs: &[(&str, TypeId)],
    string_table: &mut StringTable,
) -> ResolvedTraitDefinition {
    let requirements = requirement_specs
        .iter()
        .enumerate()
        .map(
            |(index, (req_name, return_type))| ResolvedTraitRequirement {
                id: TraitRequirementId(start_requirement_id + index as u32),
                name: string_table.intern(req_name),
                name_location: SourceLocation::default(),
                receiver: TraitReceiverRequirement::Immutable { this_type },
                parameters: vec![],
                returns: vec![ResolvedTraitReturn {
                    type_id: *return_type,
                    channel: ReturnChannel::Success,
                    location: SourceLocation::default(),
                }],
                location: SourceLocation::default(),
            },
        )
        .collect();

    ResolvedTraitDefinition {
        id: trait_id,
        name: string_table.intern(name),
        canonical_path: path(name, string_table),
        source_file: path(name, string_table),
        this_type,
        requirements,
        declaration_location: SourceLocation::default(),
        visibility: TraitVisibility::Source { exported: true },
    }
}

/// Build a canonical `TraitEvidenceDefinition` mapping each requirement, in authored order,
/// to a receiver method path.
fn canonical_evidence(
    trait_id: TraitId,
    target_type_id: TypeId,
    requirement_method_paths: &[(TraitRequirementId, &str)],
    string_table: &mut StringTable,
) -> TraitEvidenceDefinition {
    let requirements = requirement_method_paths
        .iter()
        .map(|(requirement_id, method_name)| TraitRequirementEvidence {
            requirement_id: *requirement_id,
            method_path: path(method_name, string_table),
        })
        .collect();

    TraitEvidenceDefinition {
        id: TraitEvidenceId(0),
        kind: TraitEvidenceKind::Canonical,
        target_type_id,
        trait_id,
        source_file: InternedPath::new(),
        declaration_location: SourceLocation::default(),
        requirements,
    }
}

fn builtin_evidence(trait_id: TraitId, target_type_id: TypeId) -> TraitEvidenceDefinition {
    TraitEvidenceDefinition {
        id: TraitEvidenceId(0),
        kind: TraitEvidenceKind::Builtin,
        target_type_id,
        trait_id,
        source_file: InternedPath::new(),
        declaration_location: SourceLocation::default(),
        requirements: vec![],
    }
}

/// Build the declared struct/choice declaration records the evidence projection joins
/// against, mirroring the declaration-centric join the builder uses.
///
/// WHAT: each entry becomes one `PublicDeclarationRecord` whose `receiver_methods` carry the
/// already-finalized `PublicReceiverMethodSemantics` values for that receiver. The
/// evidence projection joins its method-path keys against the
/// `(receiver_origin, defining_name)` pair on those records. The helper constructs stable
/// method origins directly as finalized test inputs and never builds a `ReceiverMethodCatalog`;
/// the production projection under test only consumes those origins.
fn declarations_with_receiver_methods(
    entries: &[(InternedPath, ReceiverKey)],
    type_environment: &TypeEnvironment,
    string_table: &StringTable,
) -> Vec<PublicDeclarationRecord> {
    let mut by_receiver: FxHashMap<&InternedPath, Vec<PublicReceiverMethodSemantics>> =
        FxHashMap::default();
    let mut by_receiver_name: FxHashMap<&InternedPath, String> = FxHashMap::default();
    for (function_path, receiver) in entries {
        let receiver_path = match receiver {
            ReceiverKey::Struct(path) | ReceiverKey::Choice(path) => path,
            ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_) => {
                panic!("test helper only supports nominal receivers")
            }
        };
        let method_name = function_path
            .name_str(string_table)
            .expect("test method path must have a defining name")
            .to_owned();
        let receiver_origin = type_environment
            .nominal_id_for_path(receiver_path)
            .map(|_| {
                OriginTypeId::new(
                    module_origin(),
                    receiver_path
                        .name_str(string_table)
                        .expect("receiver path has defining name")
                        .to_owned(),
                    match receiver {
                        ReceiverKey::Struct(_) => OriginTypeCategory::Struct,
                        ReceiverKey::Choice(_) => OriginTypeCategory::Choice,
                        _ => unreachable!("non-nominal receivers rejected above"),
                    },
                )
            })
            .expect("receiver nominal must be registered for the test");
        let method_origin =
            OriginFunctionId::new_receiver(module_origin(), method_name, receiver_origin.clone());
        by_receiver
            .entry(receiver_path)
            .or_default()
            .push(PublicReceiverMethodSemantics {
                method_origin,
                category: PublicReceiverMethodCategory::ConcreteLocal,
                parameters: vec![],
                returns: vec![],
                error_return: None,
            });
        by_receiver_name.insert(
            receiver_path,
            receiver_path.name_str(string_table).unwrap().to_owned(),
        );
    }

    by_receiver
        .into_iter()
        .map(|(receiver_path, receiver_methods)| {
            let name = by_receiver_name
                .get(receiver_path)
                .cloned()
                .unwrap_or_default();
            let category = if entries
                .iter()
                .any(|(_, key)| matches!(key, ReceiverKey::Choice(p) if p == receiver_path))
            {
                OriginTypeCategory::Choice
            } else {
                OriginTypeCategory::Struct
            };
            let origin = OriginTypeId::new(module_origin(), name, category);
            PublicDeclarationRecord {
                origin: OriginDeclarationId::Type(origin),
                synthetic_interface_provenance: Default::default(),
                semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
                    generic_parameters: vec![],
                    fields: vec![],
                    receiver_methods,
                }),
            }
        })
        .collect()
}

/// Build the evidence projection context and call `project_reusable_evidence` directly,
/// bypassing the full draft builder so the evidence projection is the only unit under test.
///
/// The declarations passed in must already expose the public receiver-method origins the
/// evidence projection will join against; the helper no longer accepts a raw
/// receiver-method catalog; the declarations already carry finalized origins.
#[allow(clippy::too_many_arguments)]
fn project_evidence(
    trait_environment: &TraitEnvironment,
    trait_evidence_environment: &TraitEvidenceEnvironment,
    declarations: &[PublicDeclarationRecord],
    nominal_origins: &FxHashMap<InternedPath, OriginTypeId>,
    trait_origins: &FxHashMap<InternedPath, OriginTraitId>,
    type_environment: &TypeEnvironment,
    string_table: &StringTable,
) -> Result<Vec<PublicEvidenceRecord>, CompilerError> {
    let registry = ExternalPackageRegistry::new();
    let nominal_resolver = TransientNominalOriginResolver::new(type_environment, nominal_origins);
    let generic_resolver = FoldedValueGenericParameterResolver;
    let projection_context =
        CanonicalTypeProjectionContext::new(&nominal_resolver, &generic_resolver, &registry);

    let context = EvidenceProjectionContext {
        trait_environment,
        trait_evidence_environment,
        public_source_nominal_type_origins: nominal_origins,
        public_source_trait_origins: trait_origins,
        type_environment,
        string_table,
        projection_context: &projection_context,
    };

    project_reusable_evidence(declarations, &context)
}

fn string_type_id(env: &TypeEnvironment) -> TypeId {
    env.builtins().string
}

/// Project one public `Label` conformance while varying only the completed receiver surface.
///
/// These focused scenarios distinguish a valid non-reusable omission from contradictory
/// public-interface metadata without repeating the evidence and canonical-origin setup.
fn project_label_display_evidence_with_method_origins(
    method_origins: Vec<OriginFunctionId>,
) -> Result<Vec<PublicEvidenceRecord>, CompilerError> {
    let mut string_table = StringTable::new();
    let mut type_environment = TypeEnvironment::new();

    let this_id = this_type(&mut type_environment, &mut string_table);
    let (_, label_type_id) = register_struct(
        &mut type_environment,
        &mut string_table,
        "Label",
        empty_fields(),
        None,
    );

    let trait_id = TraitId(0);
    let definition = trait_definition(
        trait_id,
        "DISPLAY_TEXT",
        this_id,
        0,
        &[("display", type_environment.builtins().string)],
        &mut string_table,
    );
    let mut trait_environment = TraitEnvironment::new();
    trait_environment.insert(definition);

    let mut evidence_environment = TraitEvidenceEnvironment::new();
    evidence_environment.insert_validated(canonical_evidence(
        trait_id,
        label_type_id,
        &[(TraitRequirementId(0), "display")],
        &mut string_table,
    ));

    let receiver_methods = method_origins
        .into_iter()
        .map(|method_origin| PublicReceiverMethodSemantics {
            method_origin,
            category: PublicReceiverMethodCategory::ConcreteLocal,
            parameters: vec![],
            returns: vec![],
            error_return: None,
        })
        .collect();
    let declarations = vec![PublicDeclarationRecord {
        origin: OriginDeclarationId::Type(struct_origin("Label")),
        synthetic_interface_provenance: Default::default(),
        semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
            generic_parameters: vec![],
            fields: vec![],
            receiver_methods,
        }),
    }];

    let mut nominal_origins = FxHashMap::default();
    nominal_origins.insert(path("Label", &mut string_table), struct_origin("Label"));

    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(
        path("DISPLAY_TEXT", &mut string_table),
        trait_origin("DISPLAY_TEXT"),
    );

    project_evidence(
        &trait_environment,
        &evidence_environment,
        &declarations,
        &nominal_origins,
        &trait_origins,
        &type_environment,
        &string_table,
    )
}

#[test]
fn evidence_identity_stable_across_local_allocations() {
    let mut string_table = StringTable::new();

    // Build two identical semantic configurations with different local allocation offsets.
    // The first uses TypeId, TraitId, TraitRequirementId and TraitEvidenceId starting from
    // their natural values. The second inserts dummy types, a dummy private trait with its
    // own requirement, and a dummy private-trait evidence row first so every dense local ID
    // differs while the retained public evidence is semantically identical.
    #[allow(clippy::type_complexity)]
    fn build_config(
        string_table: &mut StringTable,
        allocate_waste: bool,
    ) -> (
        TraitEnvironment,
        TraitEvidenceEnvironment,
        Vec<PublicDeclarationRecord>,
        FxHashMap<InternedPath, OriginTypeId>,
        FxHashMap<InternedPath, OriginTraitId>,
        TypeEnvironment,
    ) {
        let mut env = TypeEnvironment::new();

        if allocate_waste {
            let _waste_type = this_type(&mut env, string_table);
            let _ = register_struct(&mut env, string_table, "Waste", empty_fields(), None);
        }

        let this_id = this_type(&mut env, string_table);
        let (_, label_type_id) =
            register_struct(&mut env, string_table, "Label", empty_fields(), None);
        let _ = register_struct(&mut env, string_table, "Other", empty_fields(), None);

        let mut trait_env = TraitEnvironment::new();

        // When allocating waste, insert a coherent dummy private trait with one requirement
        // before the real trait. The real trait then receives a higher TraitId and its
        // requirement receives a higher TraitRequirementId, matching the TraitEnvironment
        // vector-index invariant.
        let trait_id = if allocate_waste {
            let dummy_this = this_type(&mut env, string_table);
            let dummy_definition = trait_definition(
                TraitId(0),
                "DummyPrivateTrait",
                dummy_this,
                0,
                &[("dummy_req", env.builtins().string)],
                string_table,
            );
            trait_env.insert(dummy_definition);
            TraitId(1)
        } else {
            TraitId(0)
        };

        // The dummy trait occupies requirement id 0; the real trait's display
        // requirement must start at id 1 so the four transient allocations
        // (TypeId, TraitId, TraitRequirementId, TraitEvidenceId) all
        // differ between the baseline and waste configurations.
        let real_start_requirement_id = if allocate_waste { 1 } else { 0 };
        let definition = trait_definition(
            trait_id,
            "DISPLAY_TEXT",
            this_id,
            real_start_requirement_id,
            &[("display", string_type_id(&env))],
            string_table,
        );
        trait_env.insert(definition);

        let display_path = path("display", string_table);
        let mut evidence_env = TraitEvidenceEnvironment::new();
        // The dummy private trait's evidence uses requirement id 0. The retained
        // evidence entry uses the real trait's authored requirement id, which is
        // real_start_requirement_id, so its TraitEvidenceId differs from the
        // baseline configuration.
        if allocate_waste {
            evidence_env.insert_validated(canonical_evidence(
                TraitId(0),
                label_type_id,
                &[(TraitRequirementId(0), "dummy_req")],
                string_table,
            ));
        }
        evidence_env.insert_validated(canonical_evidence(
            trait_id,
            label_type_id,
            &[(TraitRequirementId(real_start_requirement_id), "display")],
            string_table,
        ));

        let declarations = declarations_with_receiver_methods(
            &[(
                display_path,
                ReceiverKey::Struct(path("Label", string_table)),
            )],
            &env,
            string_table,
        );

        let mut nominal_origins = FxHashMap::default();
        nominal_origins.insert(path("Label", string_table), struct_origin("Label"));
        nominal_origins.insert(path("Other", string_table), struct_origin("Other"));

        let mut trait_origins = FxHashMap::default();
        trait_origins.insert(
            path("DISPLAY_TEXT", string_table),
            trait_origin("DISPLAY_TEXT"),
        );

        (
            trait_env,
            evidence_env,
            declarations,
            nominal_origins,
            trait_origins,
            env,
        )
    }

    let config_a = build_config(&mut string_table, false);
    let config_b = build_config(&mut string_table, true);

    let evidence_a = project_evidence(
        &config_a.0,
        &config_a.1,
        &config_a.2,
        &config_a.3,
        &config_a.4,
        &config_a.5,
        &string_table,
    )
    .expect("evidence projection A should succeed");

    let evidence_b = project_evidence(
        &config_b.0,
        &config_b.1,
        &config_b.2,
        &config_b.3,
        &config_b.4,
        &config_b.5,
        &string_table,
    )
    .expect("evidence projection B should succeed");

    assert_eq!(evidence_a.len(), 1);
    assert_eq!(evidence_b.len(), 1);

    let record_a = &evidence_a[0];
    let record_b = &evidence_b[0];

    assert_eq!(record_a.identity, record_b.identity);
    assert_eq!(record_a.ownership, record_b.ownership);
    assert_eq!(
        record_a.requirement_mappings.len(),
        record_b.requirement_mappings.len()
    );
    assert_eq!(
        record_a.requirement_mappings[0].requirement_identity,
        record_b.requirement_mappings[0].requirement_identity
    );
    assert_eq!(
        record_a.requirement_mappings[0].method_origin,
        record_b.requirement_mappings[0].method_origin
    );

    // Capture a small snapshot of the four dense local identity classes that drive
    // stable identity so the test proves the baseline and waste configurations
    // genuinely differ in every one of them. The four values are looked up from
    // the existing evidence-environment index and the trait/type environments
    // rather than from a new production getter.
    fn snapshot(
        trait_env: &TraitEnvironment,
        evidence_env: &TraitEvidenceEnvironment,
        env: &TypeEnvironment,
        label_path: &InternedPath,
        real_trait_canonical_path: &InternedPath,
    ) -> (TypeId, TraitId, TraitRequirementId, TraitEvidenceId) {
        let label_type_id = env
            .type_id_for_nominal_id(
                env.nominal_id_for_path(label_path)
                    .expect("Label nominal id"),
            )
            .expect("Label type id");
        let real_trait_id = trait_env
            .id_for_path(real_trait_canonical_path)
            .expect("real trait id");
        let definition = trait_env.get(real_trait_id).expect("real trait definition");
        let real_requirement_id = definition.requirements[0].id;
        let evidence_id = evidence_env
            .canonical_for(label_type_id, real_trait_id)
            .expect("real evidence id");
        (
            label_type_id,
            real_trait_id,
            real_requirement_id,
            evidence_id,
        )
    }

    let label_path = path("Label", &mut string_table);
    let trait_path = path("DISPLAY_TEXT", &mut string_table);
    let (type_id_a, trait_id_a, requirement_id_a, evidence_id_a) = snapshot(
        &config_a.0,
        &config_a.1,
        &config_a.5,
        &label_path,
        &trait_path,
    );
    let (type_id_b, trait_id_b, requirement_id_b, evidence_id_b) = snapshot(
        &config_b.0,
        &config_b.1,
        &config_b.5,
        &label_path,
        &trait_path,
    );
    assert_ne!(
        type_id_a, type_id_b,
        "TypeId must differ between configurations"
    );
    assert_ne!(
        trait_id_a, trait_id_b,
        "TraitId must differ between configurations"
    );
    assert_ne!(
        requirement_id_a, requirement_id_b,
        "TraitRequirementId must differ between configurations"
    );
    assert_ne!(
        evidence_id_a, evidence_id_b,
        "TraitEvidenceId must differ between configurations"
    );
}

#[test]
fn evidence_requirement_mappings_preserve_authored_order_and_exact_receiver_origins() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let this_id = this_type(&mut env, &mut string_table);
    let (_, label_type_id) =
        register_struct(&mut env, &mut string_table, "Label", empty_fields(), None);

    let trait_id = TraitId(0);
    let definition = trait_definition(
        trait_id,
        "NAMED",
        this_id,
        0,
        &[("name", env.builtins().string), ("id", env.builtins().int)],
        &mut string_table,
    );
    let mut trait_env = TraitEnvironment::new();
    trait_env.insert(definition);

    let mut evidence_env = TraitEvidenceEnvironment::new();
    evidence_env.insert_validated(canonical_evidence(
        trait_id,
        label_type_id,
        &[
            // Deliberately reverse the evidence requirement vector so the output proves
            // trait-definition authored order, not accidental evidence-vector order.
            (TraitRequirementId(1), "id"),
            (TraitRequirementId(0), "name"),
        ],
        &mut string_table,
    ));

    let label_path = path("Label", &mut string_table);
    let name_path = path("name", &mut string_table);
    let id_path = path("id", &mut string_table);
    let declarations = declarations_with_receiver_methods(
        &[
            (name_path.clone(), ReceiverKey::Struct(label_path.clone())),
            (id_path.clone(), ReceiverKey::Struct(label_path.clone())),
        ],
        &env,
        &string_table,
    );

    let mut nominal_origins = FxHashMap::default();
    nominal_origins.insert(label_path.clone(), struct_origin("Label"));

    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(path("NAMED", &mut string_table), trait_origin("NAMED"));

    let evidence = project_evidence(
        &trait_env,
        &evidence_env,
        &declarations,
        &nominal_origins,
        &trait_origins,
        &env,
        &string_table,
    )
    .expect("draft with two-requirement evidence should build");

    assert_eq!(evidence.len(), 1);
    let record = &evidence[0];

    assert_eq!(record.requirement_mappings.len(), 2);
    assert_eq!(
        record.requirement_mappings[0]
            .requirement_identity
            .requirement_name(),
        "name"
    );
    assert_eq!(
        record.requirement_mappings[1]
            .requirement_identity
            .requirement_name(),
        "id"
    );

    let expected_name_origin =
        OriginFunctionId::new_receiver(module_origin(), "name".to_owned(), struct_origin("Label"));
    let expected_id_origin =
        OriginFunctionId::new_receiver(module_origin(), "id".to_owned(), struct_origin("Label"));
    assert_eq!(
        record.requirement_mappings[0].method_origin,
        expected_name_origin
    );
    assert_eq!(
        record.requirement_mappings[1].method_origin,
        expected_id_origin
    );
}

#[test]
fn evidence_excludes_private_target_and_retains_public_target() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let this_id = this_type(&mut env, &mut string_table);
    let (_, public_type_id) =
        register_struct(&mut env, &mut string_table, "Public", empty_fields(), None);
    let (_, private_type_id) =
        register_struct(&mut env, &mut string_table, "Private", empty_fields(), None);

    let trait_id = TraitId(0);
    let definition = trait_definition(
        trait_id,
        "DISPLAY_TEXT",
        this_id,
        0,
        &[("display", env.builtins().string)],
        &mut string_table,
    );
    let mut trait_env = TraitEnvironment::new();
    trait_env.insert(definition);

    let mut evidence_env = TraitEvidenceEnvironment::new();
    evidence_env.insert_validated(canonical_evidence(
        trait_id,
        public_type_id,
        &[(TraitRequirementId(0), "display")],
        &mut string_table,
    ));
    evidence_env.insert_validated(canonical_evidence(
        trait_id,
        private_type_id,
        &[(TraitRequirementId(0), "display_private")],
        &mut string_table,
    ));

    let public_path = path("Public", &mut string_table);
    let display_path = path("display", &mut string_table);
    let display_private_path = path("display_private", &mut string_table);
    let declarations = declarations_with_receiver_methods(
        &[
            (
                display_path.clone(),
                ReceiverKey::Struct(public_path.clone()),
            ),
            (
                display_private_path,
                ReceiverKey::Struct(path("Private", &mut string_table)),
            ),
        ],
        &env,
        &string_table,
    );

    let mut nominal_origins = FxHashMap::default();
    nominal_origins.insert(public_path.clone(), struct_origin("Public"));

    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(
        path("DISPLAY_TEXT", &mut string_table),
        trait_origin("DISPLAY_TEXT"),
    );

    let evidence = project_evidence(
        &trait_env,
        &evidence_env,
        &declarations,
        &nominal_origins,
        &trait_origins,
        &env,
        &string_table,
    )
    .expect("draft with mixed public/private evidence should build");

    assert_eq!(evidence.len(), 1);
    let record = &evidence[0];
    assert_eq!(
        record.identity,
        CanonicalEvidenceIdentity::new(
            CanonicalTypeIdentity::SourceNominal(struct_origin("Public")),
            CanonicalTraitIdentity::Source(trait_origin("DISPLAY_TEXT")),
        )
    );
}

#[test]
fn evidence_excludes_private_source_trait() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let this_id = this_type(&mut env, &mut string_table);
    let (_, target_type_id) =
        register_struct(&mut env, &mut string_table, "Label", empty_fields(), None);

    let trait_id = TraitId(0);
    let definition = trait_definition(
        trait_id,
        "PrivateTrait",
        this_id,
        0,
        &[("display", env.builtins().string)],
        &mut string_table,
    );
    let mut trait_env = TraitEnvironment::new();
    trait_env.insert(definition);

    let mut evidence_env = TraitEvidenceEnvironment::new();
    evidence_env.insert_validated(canonical_evidence(
        trait_id,
        target_type_id,
        &[(TraitRequirementId(0), "display")],
        &mut string_table,
    ));

    let label_path = path("Label", &mut string_table);
    let display_path = path("display", &mut string_table);
    let declarations = declarations_with_receiver_methods(
        &[(
            display_path.clone(),
            ReceiverKey::Struct(label_path.clone()),
        )],
        &env,
        &string_table,
    );

    let mut nominal_origins = FxHashMap::default();
    nominal_origins.insert(label_path.clone(), struct_origin("Label"));

    // The trait is NOT in the public source-trait origin index, so it is private.
    let trait_origins = FxHashMap::default();

    let evidence = project_evidence(
        &trait_env,
        &evidence_env,
        &declarations,
        &nominal_origins,
        &trait_origins,
        &env,
        &string_table,
    )
    .expect("draft with private-trait evidence should build");

    assert_eq!(evidence.len(), 0);
}

#[test]
fn evidence_excludes_builtin_from_direct_module_draft() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let (_, target_type_id) =
        register_struct(&mut env, &mut string_table, "Label", empty_fields(), None);

    let trait_id = TraitId(0);
    let mut trait_env = TraitEnvironment::new();

    // Register a core trait so the trait environment has a definition for the builtin evidence.
    let string_type = env.builtins().string;
    trait_env.register_core_trait(
        &mut env,
        &mut string_table,
        "CASTABLE_TO_STRING",
        "to_string",
        string_type,
        None,
    );

    let mut evidence_env = TraitEvidenceEnvironment::new();
    evidence_env.insert_builtin(builtin_evidence(trait_id, target_type_id));

    let label_path = path("Label", &mut string_table);
    let nominal_origins = FxHashMap::default();
    let mut nominal_origins = nominal_origins;
    nominal_origins.insert(label_path.clone(), struct_origin("Label"));

    let evidence = project_evidence(
        &trait_env,
        &evidence_env,
        &[],
        &nominal_origins,
        &FxHashMap::default(),
        &env,
        &string_table,
    )
    .expect("evidence projection with builtin evidence should succeed");

    assert_eq!(
        evidence.len(),
        0,
        "builtin evidence must not be duplicated into a direct module draft"
    );
}

#[test]
fn evidence_retains_source_canonical_core_trait_evidence() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let (_, label_type_id) =
        register_struct(&mut env, &mut string_table, "Label", empty_fields(), None);

    let mut trait_env = TraitEnvironment::new();

    // Register the compiler-owned DISPLAYABLE core trait. The trait environment records
    // its CoreTraitKind so core_trait_kind resolves it without the partial root-table map.
    let displayable_id = trait_env.register_core_displayable(&mut env, &mut string_table);

    // Source-authored canonical evidence: the public Label target conforms to the core
    // DISPLAYABLE trait. This must be retained, not excluded, even though the core trait
    // has no entry in public_source_trait_origins or trait_source_facts.
    let display_path = path("display", &mut string_table);
    let mut evidence_env = TraitEvidenceEnvironment::new();
    evidence_env.insert_validated(canonical_evidence(
        displayable_id,
        label_type_id,
        &[(TraitRequirementId(0), "display")],
        &mut string_table,
    ));

    let label_path = path("Label", &mut string_table);
    let declarations = declarations_with_receiver_methods(
        &[(
            display_path.clone(),
            ReceiverKey::Struct(label_path.clone()),
        )],
        &env,
        &string_table,
    );

    let mut nominal_origins = FxHashMap::default();
    nominal_origins.insert(label_path.clone(), struct_origin("Label"));

    // The core trait is not in the public source-trait origin index, but core traits are
    // always consumer-visible so the evidence is retained.
    let trait_origins = FxHashMap::default();

    let evidence = project_evidence(
        &trait_env,
        &evidence_env,
        &declarations,
        &nominal_origins,
        &trait_origins,
        &env,
        &string_table,
    )
    .expect("source canonical evidence for a core trait should be retained");

    assert_eq!(evidence.len(), 1);
    let record = &evidence[0];
    assert_eq!(
        record.identity,
        CanonicalEvidenceIdentity::new(
            CanonicalTypeIdentity::SourceNominal(struct_origin("Label")),
            CanonicalTraitIdentity::Core(CanonicalCoreTraitIdentity::Displayable),
        )
    );
    assert_eq!(record.ownership, PublicEvidenceOwnership::SourceCanonical);
    assert_eq!(record.requirement_mappings.len(), 1);
    assert_eq!(
        record.requirement_mappings[0]
            .requirement_identity
            .requirement_name(),
        "display"
    );
    let expected_origin = OriginFunctionId::new_receiver(
        module_origin(),
        "display".to_owned(),
        struct_origin("Label"),
    );
    assert_eq!(
        record.requirement_mappings[0].method_origin,
        expected_origin
    );
}

#[test]
fn evidence_rejects_duplicate_stable_keys() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let this_id = this_type(&mut env, &mut string_table);
    let (_, label_type_id) =
        register_struct(&mut env, &mut string_table, "Label", empty_fields(), None);

    let trait_id = TraitId(0);
    let definition = trait_definition(
        trait_id,
        "DISPLAY_TEXT",
        this_id,
        0,
        &[("display", env.builtins().string)],
        &mut string_table,
    );
    let mut trait_env = TraitEnvironment::new();
    trait_env.insert(definition);

    let mut evidence_env = TraitEvidenceEnvironment::new();
    let evidence = canonical_evidence(
        trait_id,
        label_type_id,
        &[(TraitRequirementId(0), "display")],
        &mut string_table,
    );
    evidence_env.insert_validated(evidence.clone());
    evidence_env.insert_validated(evidence);

    let label_path = path("Label", &mut string_table);
    let display_path = path("display", &mut string_table);
    let declarations = declarations_with_receiver_methods(
        &[(
            display_path.clone(),
            ReceiverKey::Struct(label_path.clone()),
        )],
        &env,
        &string_table,
    );

    let mut nominal_origins = FxHashMap::default();
    nominal_origins.insert(label_path.clone(), struct_origin("Label"));

    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(
        path("DISPLAY_TEXT", &mut string_table),
        trait_origin("DISPLAY_TEXT"),
    );

    let result = project_evidence(
        &trait_env,
        &evidence_env,
        &declarations,
        &nominal_origins,
        &trait_origins,
        &env,
        &string_table,
    );

    let message = result
        .expect_err("duplicate evidence keys must be rejected")
        .msg;
    assert!(
        message.contains("target-plus-trait identity"),
        "expected a duplicate-key rejection, got: {message}"
    );
}

#[test]
fn evidence_rejects_core_trait_without_classifier() {
    // A trait definition with `TraitVisibility::Core` but no `CoreTraitKind` is malformed
    // compiler metadata. The projection must reject it as a `CompilerError` rather than
    // silently fall through to source-path visibility.
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let this_id = this_type(&mut env, &mut string_table);
    let (_, label_type_id) =
        register_struct(&mut env, &mut string_table, "Label", empty_fields(), None);

    let mut trait_env = TraitEnvironment::new();
    // Build a `Core` trait definition without recording any `CoreTraitKind`. The
    // `TraitEnvironment` allows raw inserts, so this test exercises the malformed
    // combination directly without expanding the public surface.
    let definition = ResolvedTraitDefinition {
        id: TraitId(0),
        name: string_table.intern("UnrecordedCore"),
        canonical_path: path("UnrecordedCore", &mut string_table),
        source_file: path("UnrecordedCore", &mut string_table),
        this_type: this_id,
        requirements: vec![ResolvedTraitRequirement {
            id: TraitRequirementId(0),
            name: string_table.intern("display"),
            name_location: SourceLocation::default(),
            receiver: TraitReceiverRequirement::Immutable { this_type: this_id },
            parameters: vec![],
            returns: vec![ResolvedTraitReturn {
                type_id: env.builtins().string,
                channel: ReturnChannel::Success,
                location: SourceLocation::default(),
            }],
            location: SourceLocation::default(),
        }],
        declaration_location: SourceLocation::default(),
        visibility: TraitVisibility::Core,
    };
    trait_env.insert(definition);

    let mut evidence_env = TraitEvidenceEnvironment::new();
    let display_path = path("display", &mut string_table);
    evidence_env.insert_validated(canonical_evidence(
        TraitId(0),
        label_type_id,
        &[(TraitRequirementId(0), "display")],
        &mut string_table,
    ));

    let label_path = path("Label", &mut string_table);
    let declarations = declarations_with_receiver_methods(
        &[(
            display_path.clone(),
            ReceiverKey::Struct(label_path.clone()),
        )],
        &env,
        &string_table,
    );

    let mut nominal_origins = FxHashMap::default();
    nominal_origins.insert(label_path.clone(), struct_origin("Label"));
    // No public source-trait origin and no `CoreTraitKind`; the projection must fail.
    let trait_origins = FxHashMap::default();

    let result = project_evidence(
        &trait_env,
        &evidence_env,
        &declarations,
        &nominal_origins,
        &trait_origins,
        &env,
        &string_table,
    );

    let message = result
        .expect_err("a core trait without a CoreTraitKind must be rejected")
        .msg;
    assert!(
        message.contains("no recorded `CoreTraitKind`"),
        "expected a core-without-classifier rejection, got: {message}"
    );
}

#[test]
fn evidence_rejects_requirement_count_mismatch() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let this_id = this_type(&mut env, &mut string_table);
    let (_, label_type_id) =
        register_struct(&mut env, &mut string_table, "Label", empty_fields(), None);

    let trait_id = TraitId(0);
    let definition = trait_definition(
        trait_id,
        "NAMED",
        this_id,
        0,
        &[("name", env.builtins().string), ("id", env.builtins().int)],
        &mut string_table,
    );
    let mut trait_env = TraitEnvironment::new();
    trait_env.insert(definition);

    let mut evidence_env = TraitEvidenceEnvironment::new();
    // Evidence has only one requirement but the trait definition has two.
    evidence_env.insert_validated(canonical_evidence(
        trait_id,
        label_type_id,
        &[(TraitRequirementId(0), "name")],
        &mut string_table,
    ));

    let label_path = path("Label", &mut string_table);
    let name_path = path("name", &mut string_table);
    let declarations = declarations_with_receiver_methods(
        &[(name_path.clone(), ReceiverKey::Struct(label_path.clone()))],
        &env,
        &string_table,
    );

    let mut nominal_origins = FxHashMap::default();
    nominal_origins.insert(label_path.clone(), struct_origin("Label"));

    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(path("NAMED", &mut string_table), trait_origin("NAMED"));

    let result = project_evidence(
        &trait_env,
        &evidence_env,
        &declarations,
        &nominal_origins,
        &trait_origins,
        &env,
        &string_table,
    );

    let message = result
        .expect_err("a requirement count mismatch must be rejected")
        .msg;
    assert!(
        message.contains("count mismatch"),
        "expected a count-mismatch rejection, got: {message}"
    );
}

#[test]
fn evidence_omits_non_public_receiver_method() {
    // A private implementation leaves the public receiver surface empty. The local
    // conformance remains valid but the complete evidence record cannot be reused.
    let evidence = project_label_display_evidence_with_method_origins(vec![])
        .expect("valid local evidence with a private requirement method is omitted, not rejected");

    assert!(
        evidence.is_empty(),
        "non-reusable evidence must be omitted without a partial record, got: {evidence:?}"
    );
}

#[test]
fn evidence_rejects_wrong_receiver_origin() {
    let wrong_receiver_method = OriginFunctionId::new_receiver(
        module_origin(),
        "display".to_owned(),
        struct_origin("Other"),
    );
    let result = project_label_display_evidence_with_method_origins(vec![wrong_receiver_method]);

    let message = result
        .expect_err("a wrong-receiver method origin must be rejected")
        .msg;
    assert!(
        message.contains("wrong-receiver method origin"),
        "expected a wrong-receiver rejection, got: {message}"
    );
}

#[test]
fn evidence_rejects_free_function_origin_in_receiver_surface() {
    let free_function = OriginFunctionId::new_free(module_origin(), "display".to_owned());
    let result = project_label_display_evidence_with_method_origins(vec![free_function]);

    let message = result
        .expect_err("a free-function method origin in a receiver surface must be rejected")
        .msg;
    assert!(
        message.contains("free-function origin"),
        "expected a free-function-origin rejection, got: {message}"
    );
}

#[test]
fn evidence_rejects_duplicate_public_method_match() {
    let duplicate_method = OriginFunctionId::new_receiver(
        module_origin(),
        "display".to_owned(),
        struct_origin("Label"),
    );
    let result = project_label_display_evidence_with_method_origins(vec![
        duplicate_method.clone(),
        duplicate_method,
    ]);

    let message = result
        .expect_err("a duplicate public method match must be rejected")
        .msg;
    assert!(
        message.contains("matches multiple public receiver-method entries"),
        "expected a duplicate-match rejection, got: {message}"
    );
}

#[test]
fn evidence_ownership_is_source_canonical_for_direct_drafts() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    let this_id = this_type(&mut env, &mut string_table);
    let (_, label_type_id) =
        register_struct(&mut env, &mut string_table, "Label", empty_fields(), None);

    let trait_id = TraitId(0);
    let definition = trait_definition(
        trait_id,
        "DISPLAY_TEXT",
        this_id,
        0,
        &[("display", env.builtins().string)],
        &mut string_table,
    );
    let mut trait_env = TraitEnvironment::new();
    trait_env.insert(definition);

    let mut evidence_env = TraitEvidenceEnvironment::new();
    evidence_env.insert_validated(canonical_evidence(
        trait_id,
        label_type_id,
        &[(TraitRequirementId(0), "display")],
        &mut string_table,
    ));

    let label_path = path("Label", &mut string_table);
    let display_path = path("display", &mut string_table);
    let declarations = declarations_with_receiver_methods(
        &[(
            display_path.clone(),
            ReceiverKey::Struct(label_path.clone()),
        )],
        &env,
        &string_table,
    );

    let mut nominal_origins = FxHashMap::default();
    nominal_origins.insert(label_path.clone(), struct_origin("Label"));

    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(
        path("DISPLAY_TEXT", &mut string_table),
        trait_origin("DISPLAY_TEXT"),
    );

    let evidence = project_evidence(
        &trait_env,
        &evidence_env,
        &declarations,
        &nominal_origins,
        &trait_origins,
        &env,
        &string_table,
    )
    .expect("draft with source-canonical evidence should build");

    assert_eq!(evidence.len(), 1);
    assert_eq!(
        evidence[0].ownership,
        PublicEvidenceOwnership::SourceCanonical
    );
}
