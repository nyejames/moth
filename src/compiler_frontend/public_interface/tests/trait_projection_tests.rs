//! Focused hidden-invariant tests for the direct trait-requirement, trait-root and
//! trait-incompatibility projection.
//!
//! WHAT: exercises the invariants of [`DirectTraitProjection`] that integration output
//! cannot inspect: ordered trait requirements with immutable and mutable receivers,
//! trait-local `SelfType` for direct `this_type` parameter and return occurrences,
//! ordinary builtin and source-nominal concrete-type projection, `ValueMode` and
//! `ReturnChannel` fact retention, the trait-receiver `this_type` invariant, totality
//! failures for missing, duplicate, unmatched, wrong-origin, builtin and wrong-name
//! inputs, source-to-source and source-to-core incompatibility identity, stability
//! across local `TraitId` allocation, duplicate and self-relation rejection, and the
//! builder surfacing canonical incompatibilities on trait declaration records.
//! WHY: these are projection invariants owned by
//! `compiler_frontend::public_interface::trait_projection`, so they own a focused test
//! beside the module rather than an end-to-end case.

use super::super::{
    DirectExportSeed, DirectTraitProjection, DirectTraitProjectionInput,
    PublicDeclarationSemantics, PublicInterfaceDraftBuilder, PublicInterfaceDraftBuilderInput,
    PublicTraitReceiverAccess, PublicTraitSemantics, TraitSurfaceTypeIdentity,
};
use super::test_support::{
    empty_fields, module_origin, nominal_origins_map, path, register_struct, struct_origin,
    this_type, trait_binding, trait_origin, trait_origins_map, trait_root,
};

use crate::compiler_frontend::ast::statements::functions::ReturnChannel;
use crate::compiler_frontend::ast::{
    AstPublicInterfaceProjectionInput, ResolvedPublicTraitRoot, ResolvedPublicTypeRootTable,
    ResolvedTraitParameterFact, ResolvedTraitReceiverFact, ResolvedTraitRequirementFact,
    ResolvedTraitReturnFact, ResolvedTraitSourceFact, TraitReceiverAccessKind,
};
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastTarget,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalCoreTraitIdentity, CanonicalTraitIdentity, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginDeclarationId, OriginFunctionId, OriginTraitId, OriginTypeId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::traits::environment::{CoreTraitKind, TraitEnvironment};
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::traits::ids::TraitId;
use crate::compiler_frontend::value_mode::ValueMode;

use rustc_hash::FxHashMap;

fn requirement(
    name: &str,
    receiver: ResolvedTraitReceiverFact,
    parameters: Vec<ResolvedTraitParameterFact>,
    returns: Vec<ResolvedTraitReturnFact>,
    string_table: &mut StringTable,
) -> ResolvedTraitRequirementFact {
    ResolvedTraitRequirementFact {
        name: string_table.intern(name),
        receiver,
        parameters,
        returns,
    }
}

fn receiver_immutable(this_type: TypeId) -> ResolvedTraitReceiverFact {
    ResolvedTraitReceiverFact {
        access: TraitReceiverAccessKind::Immutable,
        this_type,
    }
}

fn receiver_mutable(this_type: TypeId) -> ResolvedTraitReceiverFact {
    ResolvedTraitReceiverFact {
        access: TraitReceiverAccessKind::Mutable,
        this_type,
    }
}

fn param(
    name: &str,
    value_mode: ValueMode,
    type_id: TypeId,
    string_table: &mut StringTable,
) -> ResolvedTraitParameterFact {
    ResolvedTraitParameterFact {
        name: path(name, string_table),
        value_mode,
        type_id,
    }
}

fn ret(type_id: TypeId, channel: ReturnChannel) -> ResolvedTraitReturnFact {
    ResolvedTraitReturnFact { type_id, channel }
}

fn build_traits(
    trait_roots: &[ResolvedPublicTraitRoot],
    bindings: Vec<ExportBinding>,
    nominal_origins: &FxHashMap<InternedPath, OriginTypeId>,
    trait_origins: &FxHashMap<InternedPath, OriginTraitId>,
    env: &TypeEnvironment,
    string_table: &StringTable,
) -> Result<Vec<PublicTraitSemantics>, CompilerError> {
    build_traits_with_facts(
        trait_roots,
        bindings,
        &FxHashMap::default(),
        nominal_origins,
        trait_origins,
        env,
        string_table,
    )
}

fn build_traits_with_facts(
    trait_roots: &[ResolvedPublicTraitRoot],
    bindings: Vec<ExportBinding>,
    trait_source_facts: &FxHashMap<TraitId, ResolvedTraitSourceFact>,
    nominal_origins: &FxHashMap<InternedPath, OriginTypeId>,
    trait_origins: &FxHashMap<InternedPath, OriginTraitId>,
    env: &TypeEnvironment,
    string_table: &StringTable,
) -> Result<Vec<PublicTraitSemantics>, CompilerError> {
    let registry = ExternalPackageRegistry::new();
    let mut projection = DirectTraitProjection::new(DirectTraitProjectionInput {
        trait_roots,
        trait_source_facts,
        public_source_nominal_type_origins: nominal_origins,
        public_source_trait_origins: trait_origins,
        type_environment: env,
        external_registry: &registry,
        string_table,
    })?;
    let mut semantics = Vec::new();
    for binding in &bindings {
        if matches!(binding.origin(), OriginDeclarationId::Trait(_)) {
            semantics.push(projection.project_binding(binding)?);
        }
    }
    // The builder rejects a trait root with no matching export binding through its leftover
    // check in `project_declaration_records`. Mirror that check here so the focused trait
    // helper proves the same totality invariant without the full declaration join.
    let leftover_traits = projection.remaining_names();
    if !leftover_traits.is_empty() {
        let mut names: Vec<String> = leftover_traits.into_iter().map(String::from).collect();
        names.sort();
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft trait projection: the trait root '{}' has no matching export binding; every direct trait root must join exactly one binding",
            names.join(", ")
        )));
    }
    Ok(semantics)
}

fn trait_root_with_incompatibilities(
    name: &str,
    this_type: TypeId,
    incompatible_trait_ids: Vec<TraitId>,
    string_table: &mut StringTable,
) -> ResolvedPublicTraitRoot {
    ResolvedPublicTraitRoot {
        canonical_path: path(name, string_table),
        this_type,
        requirements: Vec::new(),
        incompatible_trait_ids,
    }
}

fn core_cast_fact(
    trait_id: u32,
    target: BuiltinCastTarget,
    fallibility: BuiltinCastFallibility,
) -> (TraitId, ResolvedTraitSourceFact) {
    (
        TraitId(trait_id),
        ResolvedTraitSourceFact::Core(CoreTraitKind::Castable {
            target,
            fallibility,
        }),
    )
}
#[test]
fn projects_trait_with_ordered_requirements_immutable_and_mutable_receivers() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);
    let int_id = env.builtins().int;
    let bool_id = env.builtins().bool;

    let requirements = vec![
        requirement(
            "read",
            receiver_immutable(this_id),
            vec![param(
                "value",
                ValueMode::MutableOwned,
                int_id,
                &mut string_table,
            )],
            vec![ret(bool_id, ReturnChannel::Success)],
            &mut string_table,
        ),
        requirement(
            "write",
            receiver_mutable(this_id),
            vec![],
            vec![ret(bool_id, ReturnChannel::Success)],
            &mut string_table,
        ),
    ];

    let root = trait_root("Shape", this_id, requirements, &mut string_table);
    let binding = trait_binding("Shape");
    let trait_origins =
        trait_origins_map(vec![("Shape", trait_origin("Shape"))], &mut string_table);

    let surfaces = build_traits(
        &[root],
        vec![binding],
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    )
    .expect("trait projection succeeds");

    assert_eq!(surfaces.len(), 1);
    let surface = &surfaces[0];
    assert_eq!(surface.requirements.len(), 2);
    assert_eq!(&surface.requirements[0].name, "read");
    assert_eq!(
        surface.requirements[0].receiver_access,
        PublicTraitReceiverAccess::Immutable
    );
    assert_eq!(&surface.requirements[1].name, "write");
    assert_eq!(
        surface.requirements[1].receiver_access,
        PublicTraitReceiverAccess::Mutable
    );
}

#[test]
fn projects_self_type_for_direct_this_type_parameter_and_return_occurrences() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);

    let requirements = vec![requirement(
        "transform",
        receiver_immutable(this_id),
        vec![param(
            "other",
            ValueMode::default(),
            this_id,
            &mut string_table,
        )],
        vec![ret(this_id, ReturnChannel::Success)],
        &mut string_table,
    )];

    let root = trait_root("Shape", this_id, requirements, &mut string_table);
    let binding = trait_binding("Shape");
    let trait_origins =
        trait_origins_map(vec![("Shape", trait_origin("Shape"))], &mut string_table);

    let surfaces = build_traits(
        &[root],
        vec![binding],
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    )
    .expect("trait projection succeeds");

    let requirement = &surfaces[0].requirements[0];
    assert_eq!(
        requirement.parameters[0].type_identity,
        TraitSurfaceTypeIdentity::SelfType
    );
    assert_eq!(
        requirement.returns[0].type_identity,
        TraitSurfaceTypeIdentity::SelfType
    );
}

#[test]
fn projects_ordinary_builtin_and_source_nominal_types_as_concrete() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);
    let int_id = env.builtins().int;
    let (_, widget_id) =
        register_struct(&mut env, &mut string_table, "Widget", empty_fields(), None);

    let requirements = vec![requirement(
        "build",
        receiver_immutable(this_id),
        vec![param(
            "count",
            ValueMode::default(),
            int_id,
            &mut string_table,
        )],
        vec![ret(widget_id, ReturnChannel::Success)],
        &mut string_table,
    )];

    let root = trait_root("Shape", this_id, requirements, &mut string_table);
    let binding = trait_binding("Shape");
    let nominal_origins =
        nominal_origins_map(vec![("Widget", struct_origin("Widget"))], &mut string_table);
    let trait_origins =
        trait_origins_map(vec![("Shape", trait_origin("Shape"))], &mut string_table);

    let surfaces = build_traits(
        &[root],
        vec![binding],
        &nominal_origins,
        &trait_origins,
        &env,
        &string_table,
    )
    .expect("trait projection succeeds");

    let requirement = &surfaces[0].requirements[0];
    assert_eq!(
        requirement.parameters[0].type_identity,
        TraitSurfaceTypeIdentity::Concrete(Box::new(CanonicalTypeIdentity::Builtin(
            CanonicalBuiltinType::Int
        )))
    );
    assert!(matches!(
        &requirement.returns[0].type_identity,
        TraitSurfaceTypeIdentity::Concrete(canonical) if matches!(canonical.as_ref(), CanonicalTypeIdentity::SourceNominal(_))
    ));
}

#[test]
fn retains_value_mode_and_return_channel_facts() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);
    let int_id = env.builtins().int;
    let bool_id = env.builtins().bool;

    let requirements = vec![requirement(
        "parse",
        receiver_immutable(this_id),
        vec![param(
            "input",
            ValueMode::MutableOwned,
            int_id,
            &mut string_table,
        )],
        vec![
            ret(bool_id, ReturnChannel::Success),
            ret(int_id, ReturnChannel::Error),
        ],
        &mut string_table,
    )];

    let root = trait_root("Shape", this_id, requirements, &mut string_table);
    let binding = trait_binding("Shape");
    let trait_origins =
        trait_origins_map(vec![("Shape", trait_origin("Shape"))], &mut string_table);

    let surfaces = build_traits(
        &[root],
        vec![binding],
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    )
    .expect("trait projection succeeds");

    let requirement = &surfaces[0].requirements[0];
    assert_eq!(
        requirement.parameters[0].value_mode,
        ValueMode::MutableOwned
    );
    assert_eq!(requirement.returns.len(), 2);
    assert_eq!(requirement.returns[0].channel, ReturnChannel::Success);
    assert_eq!(requirement.returns[1].channel, ReturnChannel::Error);
}

// ---------------------------------------------------------------------------
//  Trait receiver this_type invariant
// ---------------------------------------------------------------------------

#[test]
fn rejects_requirement_receiver_this_type_mismatch() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);
    let other_id = this_type(&mut env, &mut string_table);

    let requirements = vec![requirement(
        "read",
        receiver_immutable(other_id),
        vec![],
        vec![],
        &mut string_table,
    )];

    let root = trait_root("Shape", this_id, requirements, &mut string_table);
    let binding = trait_binding("Shape");
    let trait_origins =
        trait_origins_map(vec![("Shape", trait_origin("Shape"))], &mut string_table);

    let result = build_traits(
        &[root],
        vec![binding],
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(
        message.contains("does not equal the owning trait this_type"),
        "expected a receiver this_type mismatch diagnostic, got: {message}"
    );
}

#[test]
fn rejects_mutable_receiver_this_type_mismatch() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);
    let other_id = this_type(&mut env, &mut string_table);

    let requirements = vec![requirement(
        "write",
        receiver_mutable(other_id),
        vec![],
        vec![],
        &mut string_table,
    )];

    let root = trait_root("Shape", this_id, requirements, &mut string_table);
    let binding = trait_binding("Shape");
    let trait_origins =
        trait_origins_map(vec![("Shape", trait_origin("Shape"))], &mut string_table);

    let result = build_traits(
        &[root],
        vec![binding],
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    );

    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
//  Inclusion boundaries: only direct, matched traits are projected
// ---------------------------------------------------------------------------

#[test]
fn ignores_non_trait_export_bindings() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);

    let root = trait_root("Shape", this_id, vec![], &mut string_table);
    // A free-function binding must not produce a trait surface and must not block the trait.
    let function_binding = ExportBinding::new(
        module_origin(),
        "helper".to_owned(),
        OriginDeclarationId::Function(OriginFunctionId::new_free(
            module_origin(),
            "helper".to_owned(),
        )),
    );
    let trait_binding = trait_binding("Shape");
    let trait_origins =
        trait_origins_map(vec![("Shape", trait_origin("Shape"))], &mut string_table);

    let surfaces = build_traits(
        &[root],
        vec![function_binding, trait_binding],
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    )
    .expect("non-trait bindings are skipped");

    assert_eq!(surfaces.len(), 1);
}

// ---------------------------------------------------------------------------
//  Totality failures
// ---------------------------------------------------------------------------

#[test]
fn rejects_trait_binding_without_matching_root() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();

    let binding = trait_binding("Missing");
    let trait_origins = trait_origins_map(vec![], &mut string_table);

    let result = build_traits(
        &[],
        vec![binding],
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(message.contains("no matching trait root"));
}

#[test]
fn rejects_duplicate_trait_roots_sharing_a_name() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);

    let root = trait_root("Shape", this_id, vec![], &mut string_table);
    let duplicate = trait_root("Shape", this_id, vec![], &mut string_table);
    let binding = trait_binding("Shape");
    let trait_origins =
        trait_origins_map(vec![("Shape", trait_origin("Shape"))], &mut string_table);

    let result = build_traits(
        &[root, duplicate],
        vec![binding],
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(message.contains("two trait roots share the public name"));
}

#[test]
fn rejects_trait_root_without_matching_binding() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);

    let root = trait_root("Orphan", this_id, vec![], &mut string_table);
    let trait_origins =
        trait_origins_map(vec![("Orphan", trait_origin("Orphan"))], &mut string_table);

    let result = build_traits(
        &[root],
        vec![],
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(message.contains("has no matching export binding"));
}

#[test]
fn rejects_trait_binding_origin_mismatching_root_resolved_origin() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);

    let root = trait_root("Shape", this_id, vec![], &mut string_table);
    // The binding names a different trait origin than the root resolves to.
    let wrong_binding = ExportBinding::new(
        module_origin(),
        "Shape".to_owned(),
        OriginDeclarationId::Trait(OriginTraitId::new(module_origin(), "OtherShape".to_owned())),
    );
    let trait_origins =
        trait_origins_map(vec![("Shape", trait_origin("Shape"))], &mut string_table);

    let result = build_traits(
        &[root],
        vec![wrong_binding],
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(message.contains("disagrees with its root resolved origin"));
}

#[test]
fn rejects_trait_root_without_retained_source_trait_origin() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);

    let root = trait_root("Shape", this_id, vec![], &mut string_table);
    let binding = trait_binding("Shape");
    // The public source-trait origin index is empty, so the root canonical path has no origin.

    let result = build_traits(
        &[root],
        vec![binding],
        &FxHashMap::default(),
        &FxHashMap::default(),
        &env,
        &string_table,
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(message.contains("no retained public source-trait origin"));
}

// ---------------------------------------------------------------------------
//  Trait root this_type validation
// ---------------------------------------------------------------------------

#[test]
fn rejects_trait_root_with_builtin_this_type() {
    let mut string_table = StringTable::new();
    let env = TypeEnvironment::new();
    let int_id = env.builtins().int;

    // A builtin int TypeId is not a GenericParameter named "This".
    let root = trait_root("Shape", int_id, vec![], &mut string_table);
    let binding = trait_binding("Shape");
    let trait_origins =
        trait_origins_map(vec![("Shape", trait_origin("Shape"))], &mut string_table);

    let result = build_traits(
        &[root],
        vec![binding],
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(
        message.contains("not a GenericParameter"),
        "expected a malformed this_type diagnostic, got: {message}"
    );
}

#[test]
fn rejects_trait_root_with_wrong_name_generic_parameter() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();

    // Register a synthetic generic parameter with the wrong name.
    let wrong_id = env.register_synthetic_generic_parameter(string_table.intern("Other"));

    let root = trait_root("Shape", wrong_id, vec![], &mut string_table);
    let binding = trait_binding("Shape");
    let trait_origins =
        trait_origins_map(vec![("Shape", trait_origin("Shape"))], &mut string_table);

    let result = build_traits(
        &[root],
        vec![binding],
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    );

    assert!(result.is_err());
    let message = result.unwrap_err().msg.clone();
    assert!(
        message.contains("not \"This\""),
        "expected a wrong-name this_type diagnostic, got: {message}"
    );
}

#[test]
fn projects_source_to_source_incompatibility_identity() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);

    // Alpha (the direct public trait) is incompatible with Beta (another source trait). Both
    // resolve to Source(OriginTraitId) through the public source-trait origin index.
    let alpha_path = path("Alpha", &mut string_table);
    let beta_path = path("Beta", &mut string_table);
    let mut trait_source_facts = FxHashMap::default();
    trait_source_facts.insert(TraitId(0), ResolvedTraitSourceFact::Source(alpha_path));
    trait_source_facts.insert(TraitId(1), ResolvedTraitSourceFact::Source(beta_path));

    let root =
        trait_root_with_incompatibilities("Alpha", this_id, vec![TraitId(1)], &mut string_table);
    let binding = trait_binding("Alpha");
    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(path("Alpha", &mut string_table), trait_origin("Alpha"));
    trait_origins.insert(path("Beta", &mut string_table), trait_origin("Beta"));

    let surfaces = build_traits_with_facts(
        &[root],
        vec![binding],
        &trait_source_facts,
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    )
    .expect("a source-to-source public incompatibility projects to a canonical Source identity");

    assert_eq!(surfaces.len(), 1);
    assert_eq!(
        surfaces[0].incompatibilities,
        vec![CanonicalTraitIdentity::Source(trait_origin("Beta"))]
    );
}

#[test]
fn projects_source_to_core_incompatibility_identity() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);

    // Alpha (a direct public source trait) is incompatible with a compiler-owned core cast
    // trait. The core trait side resolves to a stable CanonicalCoreTraitIdentity.
    let alpha_path = path("Alpha", &mut string_table);
    let (core_id, core_fact) = core_cast_fact(
        7,
        BuiltinCastTarget::String,
        BuiltinCastFallibility::Infallible,
    );
    let mut trait_source_facts = FxHashMap::default();
    trait_source_facts.insert(TraitId(0), ResolvedTraitSourceFact::Source(alpha_path));
    trait_source_facts.insert(core_id, core_fact);

    let root =
        trait_root_with_incompatibilities("Alpha", this_id, vec![core_id], &mut string_table);
    let binding = trait_binding("Alpha");
    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(path("Alpha", &mut string_table), trait_origin("Alpha"));

    let surfaces = build_traits_with_facts(
        &[root],
        vec![binding],
        &trait_source_facts,
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    )
    .expect("a source-to-core public incompatibility projects to a canonical Core identity");

    assert_eq!(surfaces.len(), 1);
    assert_eq!(
        surfaces[0].incompatibilities,
        vec![CanonicalTraitIdentity::Core(
            CanonicalCoreTraitIdentity::Castable {
                target: BuiltinCastTarget::String,
                fallibility: BuiltinCastFallibility::Infallible,
            }
        )]
    );
}

#[test]
fn incompatibility_identity_is_stable_across_local_trait_id_allocation() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);

    // Two independent local TraitId allocations (10 and 99) for the same source trait path
    // produce the same canonical incompatibility fact, because identity derives from the
    // stable OriginTraitId, not from the donor-local TraitId.
    let beta_path = path("Beta", &mut string_table);
    let alpha_path = path("Alpha", &mut string_table);

    let mut facts_a = FxHashMap::default();
    facts_a.insert(
        TraitId(0),
        ResolvedTraitSourceFact::Source(alpha_path.clone()),
    );
    facts_a.insert(
        TraitId(10),
        ResolvedTraitSourceFact::Source(beta_path.clone()),
    );

    let mut facts_b = FxHashMap::default();
    facts_b.insert(
        TraitId(0),
        ResolvedTraitSourceFact::Source(alpha_path.clone()),
    );
    facts_b.insert(
        TraitId(99),
        ResolvedTraitSourceFact::Source(beta_path.clone()),
    );

    let root_a =
        trait_root_with_incompatibilities("Alpha", this_id, vec![TraitId(10)], &mut string_table);
    let root_b =
        trait_root_with_incompatibilities("Alpha", this_id, vec![TraitId(99)], &mut string_table);
    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(path("Alpha", &mut string_table), trait_origin("Alpha"));
    trait_origins.insert(path("Beta", &mut string_table), trait_origin("Beta"));

    let surfaces_a = build_traits_with_facts(
        &[root_a],
        vec![trait_binding("Alpha")],
        &facts_a,
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    )
    .expect("first allocation projects");
    let surfaces_b = build_traits_with_facts(
        &[root_b],
        vec![trait_binding("Alpha")],
        &facts_b,
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    )
    .expect("second allocation projects");

    assert_eq!(
        surfaces_a[0].incompatibilities,
        surfaces_b[0].incompatibilities
    );
    assert_eq!(
        surfaces_a[0].incompatibilities,
        vec![CanonicalTraitIdentity::Source(trait_origin("Beta"))]
    );
}

#[test]
fn rejects_duplicate_canonical_incompatibility_identity() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);

    // Two distinct local TraitIds resolve to the same source trait path, so both
    // incompatibilities canonicalize to the same CanonicalTraitIdentity and the projection
    // rejects the duplicate.
    let beta_path = path("Beta", &mut string_table);
    let alpha_path = path("Alpha", &mut string_table);
    let mut trait_source_facts = FxHashMap::default();
    trait_source_facts.insert(TraitId(0), ResolvedTraitSourceFact::Source(alpha_path));
    trait_source_facts.insert(
        TraitId(1),
        ResolvedTraitSourceFact::Source(beta_path.clone()),
    );
    trait_source_facts.insert(TraitId(2), ResolvedTraitSourceFact::Source(beta_path));

    let root = trait_root_with_incompatibilities(
        "Alpha",
        this_id,
        vec![TraitId(1), TraitId(2)],
        &mut string_table,
    );
    let binding = trait_binding("Alpha");
    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(path("Alpha", &mut string_table), trait_origin("Alpha"));
    trait_origins.insert(path("Beta", &mut string_table), trait_origin("Beta"));

    let result = build_traits_with_facts(
        &[root],
        vec![binding],
        &trait_source_facts,
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    );

    let message = result
        .expect_err("a duplicate canonical incompatibility identity must be rejected")
        .msg;
    assert!(
        message.contains("a duplicate must not enter the public trait surface"),
        "expected a duplicate-identity rejection, got: {message}"
    );
}

#[test]
fn rejects_incompatibility_without_retained_source_fact() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);

    // The incompatible TraitId has no entry in the trait-source-fact table, so the projection
    // cannot classify it and fails through a CompilerError.
    let root =
        trait_root_with_incompatibilities("Alpha", this_id, vec![TraitId(5)], &mut string_table);
    let binding = trait_binding("Alpha");
    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(path("Alpha", &mut string_table), trait_origin("Alpha"));

    let result = build_traits_with_facts(
        &[root],
        vec![binding],
        &FxHashMap::default(),
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    );

    let message = result
        .expect_err("a missing trait source fact must be rejected")
        .msg;
    assert!(
        message.contains("has no retained trait source fact"),
        "expected a missing-source-fact rejection, got: {message}"
    );
}

#[test]
fn rejects_incompatibility_source_without_public_source_trait_origin() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);

    // The incompatible trait is a source trait, but its canonical path has no entry in the
    // public source-trait origin index, so it is private/unexported and must not enter the
    // public trait surface.
    let beta_path = path("Beta", &mut string_table);
    let alpha_path = path("Alpha", &mut string_table);
    let mut trait_source_facts = FxHashMap::default();
    trait_source_facts.insert(TraitId(0), ResolvedTraitSourceFact::Source(alpha_path));
    trait_source_facts.insert(TraitId(1), ResolvedTraitSourceFact::Source(beta_path));

    let root =
        trait_root_with_incompatibilities("Alpha", this_id, vec![TraitId(1)], &mut string_table);
    let binding = trait_binding("Alpha");
    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(path("Alpha", &mut string_table), trait_origin("Alpha"));

    let result = build_traits_with_facts(
        &[root],
        vec![binding],
        &trait_source_facts,
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    );

    let message = result
        .expect_err("a missing public source-trait origin must be rejected")
        .msg;
    assert!(
        message.contains("no retained public source-trait origin"),
        "expected a missing-origin rejection, got: {message}"
    );
}

#[test]
fn rejects_internal_self_incompatibility_relation() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);

    // The direct public trait Alpha carries an incompatibility that resolves to itself. An
    // authored self-relation is rejected earlier by a user-facing diagnostic, so reaching
    // this point means inconsistent resolved metadata and must fail through a CompilerError.
    let alpha_path = path("Alpha", &mut string_table);
    let mut trait_source_facts = FxHashMap::default();
    trait_source_facts.insert(TraitId(0), ResolvedTraitSourceFact::Source(alpha_path));

    let root =
        trait_root_with_incompatibilities("Alpha", this_id, vec![TraitId(0)], &mut string_table);
    let binding = trait_binding("Alpha");
    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(path("Alpha", &mut string_table), trait_origin("Alpha"));

    let result = build_traits_with_facts(
        &[root],
        vec![binding],
        &trait_source_facts,
        &FxHashMap::default(),
        &trait_origins,
        &env,
        &string_table,
    );

    let message = result
        .expect_err("an internal self-relation must be rejected")
        .msg;
    assert!(
        message.contains("resolves to itself"),
        "expected a self-relation rejection, got: {message}"
    );
}

#[test]
fn builder_carries_incompatibilities_on_trait_record() {
    let mut string_table = StringTable::new();
    let mut env = TypeEnvironment::new();
    let this_id = this_type(&mut env, &mut string_table);
    let int_id = env.builtins().int;

    // Build a minimal draft where the only trait (Shape) carries one public incompatibility
    // with another source trait (Mark). The builder must surface the canonical incompatibility
    // on the trait declaration record.
    let shape_path = path("Shape", &mut string_table);
    let mark_path = path("Mark", &mut string_table);
    let mut trait_source_facts = FxHashMap::default();
    trait_source_facts.insert(TraitId(0), ResolvedTraitSourceFact::Source(shape_path));
    trait_source_facts.insert(TraitId(1), ResolvedTraitSourceFact::Source(mark_path));

    let requirement_fact = requirement(
        "read",
        receiver_immutable(this_id),
        vec![param(
            "value",
            ValueMode::MutableOwned,
            int_id,
            &mut string_table,
        )],
        vec![ret(env.builtins().bool, ReturnChannel::Success)],
        &mut string_table,
    );
    let trait_root = ResolvedPublicTraitRoot {
        canonical_path: path("Shape", &mut string_table),
        this_type: this_id,
        requirements: vec![requirement_fact],
        incompatible_trait_ids: vec![TraitId(1)],
    };

    let root_table = ResolvedPublicTypeRootTable {
        roots: vec![],
        receiver_methods: vec![],
        trait_source_facts,
    };

    let nominal_origins: FxHashMap<InternedPath, OriginTypeId> = FxHashMap::default();
    let mut trait_origins = FxHashMap::default();
    trait_origins.insert(path("Shape", &mut string_table), trait_origin("Shape"));
    trait_origins.insert(path("Mark", &mut string_table), trait_origin("Mark"));

    let export_seed = DirectExportSeed::new(
        module_origin(),
        vec![trait_binding("Shape")],
        FxHashMap::default(),
    );

    let draft = PublicInterfaceDraftBuilder::new(PublicInterfaceDraftBuilderInput {
        export_seed,
        public_interface_projection_input: AstPublicInterfaceProjectionInput {
            root_table,
            trait_roots: vec![trait_root],
            trait_environment: Some(std::rc::Rc::new(TraitEnvironment::new())),
            trait_evidence_environment: Some(std::rc::Rc::new(TraitEvidenceEnvironment::new())),
        },
        public_source_nominal_type_origins: &nominal_origins,
        public_source_trait_origins: &trait_origins,
        type_environment: &env,
        external_registry: &ExternalPackageRegistry::new(),
        string_table: &string_table,
        generic_function_templates: &FxHashMap::default(),
        module_constants: &[],
    })
    .build()
    .expect("a trait record with one public incompatibility builds a draft")
    .draft;

    let trait_record = draft
        .declarations
        .iter()
        .find_map(|record| match &record.semantics {
            PublicDeclarationSemantics::Trait(semantics) => Some(semantics),
            _ => None,
        })
        .expect("the draft contains a trait record");

    assert_eq!(
        trait_record.incompatibilities,
        vec![CanonicalTraitIdentity::Source(trait_origin("Mark"))]
    );
}
