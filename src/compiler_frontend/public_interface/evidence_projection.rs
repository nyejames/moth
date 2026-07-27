//! Reusable evidence projection.
//!
//! WHAT: owns [`project_reusable_evidence`], which projects source-authored canonical evidence
//! into stable [`PublicEvidenceRecord`] values. The projection consumes the already-finalized
//! receiver-method surface attached to each struct or choice declaration record, so it never
//! iterates `ReceiverMethodCatalog` and never reconstructs a receiver-method origin.
//!
//! WHY: the compiler design overview requires reusable evidence as a separate collection, not
//! a declaration variant. Keeping the evidence projection in its own module separates it from
//! the declaration join, trait projection and local finalization while preserving its
//! proven projection logic.

use super::model::{
    PublicDeclarationRecord, PublicDeclarationSemantics, PublicEvidenceOwnership,
    PublicEvidenceRecord, PublicEvidenceRequirementMapping, PublicReceiverMethodSemantics,
};
use super::type_projection::project_trait_source_fact_to_canonical_identity;
use crate::compiler_frontend::ast::ResolvedTraitSourceFact;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalEvidenceIdentity, CanonicalTraitIdentity, CanonicalTypeIdentity,
    CanonicalTypeProjectionContext, StableTraitRequirementIdentity,
    project_type_id_to_canonical_identity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::semantic_identity::{
    OriginDeclarationId, OriginTraitId, OriginTypeId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::traits::definitions::TraitVisibility;
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::traits::evidence::environment::{
    TraitEvidenceDefinition, TraitRequirementEvidence,
};
use crate::compiler_frontend::traits::ids::TraitRequirementId;
use rustc_hash::{FxHashMap, FxHashSet};

/// Context for projecting direct reusable evidence into the draft.
///
/// WHAT: bundles the resolved trait environment (the single authority for classifying an
/// evidence `TraitId` as core or source), the validated evidence environment, both public
/// source origin indexes, the type environment, string table and canonical type projection
/// context. The projection uses all of these to filter, canonicalize and join evidence
/// records before the draft boundary.
///
/// Private so only the draft builder and the child test module can construct it.
pub(super) struct EvidenceProjectionContext<'a> {
    pub(super) trait_environment: &'a TraitEnvironment,
    pub(super) trait_evidence_environment: &'a TraitEvidenceEnvironment,
    pub(super) public_source_nominal_type_origins: &'a FxHashMap<InternedPath, OriginTypeId>,
    pub(super) public_source_trait_origins: &'a FxHashMap<InternedPath, OriginTraitId>,
    pub(super) type_environment: &'a TypeEnvironment,
    pub(super) string_table: &'a StringTable,
    pub(super) projection_context: &'a CanonicalTypeProjectionContext<'a>,
}

/// One receiver method exposed by the declaration-centric join, indexed for evidence lookup.
///
/// WHAT: pairs the exact stable receiver origin with the already-finalized
/// [`PublicReceiverMethodSemantics`] carried on the owning struct or choice record. The join
/// produces exactly one such entry per receiver method of every public nominal declaration;
/// duplicate same-named methods on the same receiver are rejected by the join. The evidence
/// projection never reconstructs receiver-method origins and never iterates
/// `ReceiverMethodCatalog`.
struct ReceiverMethodRecordRef<'a> {
    receiver_origin: OriginTypeId,
    method: &'a PublicReceiverMethodSemantics,
}

/// Build the receiver-method index the evidence projection joins against.
///
/// WHAT: walks the joined declaration records in their deterministic order and collects every
/// attached [`PublicReceiverMethodSemantics`] alongside the owning nominal record's exact
/// stable `OriginTypeId`. The index carries the already-finalized method origins; the evidence
/// projection consumes them by `(receiver_origin, defining_name)`. A same-named method on two
/// distinct receivers is reachable; a same-named method on a single receiver is impossible
/// because the join would have rejected the duplicate nominal or duplicate method at a
/// strictly earlier step.
fn collect_receiver_method_records(
    declarations: &[PublicDeclarationRecord],
) -> Vec<ReceiverMethodRecordRef<'_>> {
    let mut records = Vec::new();
    for declaration in declarations {
        let receiver_origin = match &declaration.origin {
            OriginDeclarationId::Type(origin) => origin.clone(),
            _ => continue,
        };
        let methods = match &declaration.semantics {
            PublicDeclarationSemantics::Struct(semantics) => &semantics.receiver_methods,
            PublicDeclarationSemantics::Choice(semantics) => &semantics.receiver_methods,
            _ => continue,
        };
        for method in methods {
            records.push(ReceiverMethodRecordRef {
                receiver_origin: receiver_origin.clone(),
                method,
            });
        }
    }
    records
}

/// Project direct reusable evidence into stable [`PublicEvidenceRecord`] values.
///
/// WHAT: iterates source-authored canonical evidence from the evidence environment. For each
/// record, it checks whether both the target nominal type and the trait are consumer-visible
/// through the already-established stable public projections. Visible evidence is projected
/// into a stable record keyed by canonical target type identity plus canonical trait identity,
/// with every trait requirement mapped in authored order to the stable receiver-method origin
/// already produced by the declaration-centric join. Private target or private source-trait
/// evidence is excluded. Evidence whose requirements include a method absent from the
/// completed consumer-visible receiver surface is also excluded as non-reusable: the whole
/// record is omitted without a diagnostic. Builtin evidence is compiler-global and never
/// duplicated into a direct module draft.
///
/// The projection is deterministic: duplicate stable evidence keys, duplicate requirement
/// mappings, requirement count or name mismatches, missing target or trait canonical identity,
/// duplicate public method matches, wrong receiver origins and free-function origins are
/// `CompilerError` values rather than silent omissions. A requirement whose method is simply
/// absent from the completed public receiver surface is a valid non-reusable omission, not an
/// infrastructure failure.
pub(super) fn project_reusable_evidence(
    declarations: &[PublicDeclarationRecord],
    context: &EvidenceProjectionContext,
) -> Result<Vec<PublicEvidenceRecord>, CompilerError> {
    let receiver_methods = collect_receiver_method_records(declarations);

    let mut records = Vec::new();
    let mut seen_identities: FxHashSet<CanonicalEvidenceIdentity> = FxHashSet::default();

    for evidence in context.trait_evidence_environment.canonical_evidence() {
        let (target_type_identity, target_origin) =
            match project_target_type_identity(evidence, context)? {
                ProjectedTarget::Public(identity, origin) => (*identity, origin),
                ProjectedTarget::Private => continue,
            };

        let trait_identity = match project_evidence_trait_identity(evidence, context)? {
            ProjectedTrait::Public(identity) => identity,
            ProjectedTrait::Private => continue,
        };

        let identity =
            CanonicalEvidenceIdentity::new(target_type_identity.clone(), trait_identity.clone());
        if !seen_identities.insert(identity.clone()) {
            return Err(CompilerError::compiler_error(format!(
                "public-interface draft evidence projection: two canonical evidence records \
                 resolved to the same target-plus-trait identity {:?}; a duplicate must not \
                 enter the reusable-evidence collection",
                identity
            )));
        }

        let Some(record) = project_one_evidence_record(
            evidence,
            identity,
            target_origin,
            &receiver_methods,
            context,
        )?
        else {
            continue;
        };

        records.push(record);
    }

    // The evidence environment stores records in a deterministic insertion Vec (header
    // processing order), and the canonical_evidence iterator preserves that order. The
    // output is therefore deterministic without a separate sort, and canonical identity
    // types need not derive Ord merely for evidence ordering.
    Ok(records)
}

/// The result of projecting an evidence target type: public and canonical, or private.
///
/// WHAT: surfaces both the canonical type identity (for the stable evidence key) and the
/// exact stable receiver origin (for the per-requirement receiver-method join). Only the
/// `SourceNominal` canonical variant is produced by `project_target_type_identity`, so the
/// paired `OriginTypeId` always matches the canonical identity; a mismatch is an internal
/// invariant violation. `Private` represents a valid source nominal target that is absent
/// from the module's public nominal-origin index.
enum ProjectedTarget {
    Public(Box<CanonicalTypeIdentity>, OriginTypeId),
    Private,
}

/// The result of projecting an evidence trait identity: public and canonical, or private.
enum ProjectedTrait {
    Public(CanonicalTraitIdentity),
    Private,
}

/// Project the evidence target type to a canonical identity, or return `Private` when the
/// target is not a consumer-visible public nominal type.
///
/// WHAT: resolves the target `TypeId` through the `TypeEnvironment` and checks whether the
/// nominal path is retained in the public source-nominal type origin index. A public nominal
/// target projects through the existing canonical type projection. A nominal without a public
/// origin is private and excluded from the draft. A missing or non-nominal target is malformed
/// source-evidence metadata and returns `CompilerError`.
fn project_target_type_identity(
    evidence: &TraitEvidenceDefinition,
    context: &EvidenceProjectionContext,
) -> Result<ProjectedTarget, CompilerError> {
    let Some(definition) = context.type_environment.get(evidence.target_type_id) else {
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft evidence projection: the evidence target TypeId({}) is not \
             registered in the TypeEnvironment; an unregistered target is an internal invariant \
             violation",
            evidence.target_type_id.0
        )));
    };

    let nominal_id = match definition {
        TypeDefinition::Struct(def) => def.id,
        TypeDefinition::Choice(def) => def.id,
        _ => {
            return Err(CompilerError::compiler_error(format!(
                "public-interface draft evidence projection: the evidence target TypeId({}) \
                 resolved to {:?}, not a nominal struct or choice; source conformance targets \
                 must be same-file nominal types",
                evidence.target_type_id.0, definition
            )));
        }
    };

    let Some(path) = context.type_environment.nominal_path_by_id(nominal_id) else {
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft evidence projection: the evidence target NominalTypeId({}) \
             has no registered path in the TypeEnvironment; an unregistered nominal is an \
             internal invariant violation",
            nominal_id.0
        )));
    };

    let Some(receiver_origin) = context
        .public_source_nominal_type_origins
        .get(path)
        .cloned()
    else {
        return Ok(ProjectedTarget::Private);
    };

    let identity = project_type_id_to_canonical_identity(
        evidence.target_type_id,
        context.type_environment,
        context.projection_context,
    )?;

    // The canonical projection only produces `SourceNominal(origin)` for a public same-file
    // nominal target, so the paired origin must equal the resolver origin; a mismatch is an
    // internal invariant violation rather than a silent coercion.
    let CanonicalTypeIdentity::SourceNominal(canonical_origin) = &identity else {
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft evidence projection: the evidence target TypeId({}) \
             projected to {:?}, not SourceNominal; a non-source-nominal canonical target for \
             source conformance is an internal invariant violation",
            evidence.target_type_id.0, identity
        )));
    };
    if canonical_origin != &receiver_origin {
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft evidence projection: the evidence target TypeId({}) \
             canonical origin {:?} does not match the public source-nominal origin {:?}; a \
             mismatch is an internal invariant violation",
            evidence.target_type_id.0, canonical_origin, receiver_origin
        )));
    }

    Ok(ProjectedTarget::Public(Box::new(identity), receiver_origin))
}

/// Project the evidence trait identity to a canonical identity, or return `Private` when the
/// trait is not consumer-visible.
///
/// WHAT: projects the evidence `TraitId` directly from the `TraitEnvironment`. A core trait
/// (identified by `core_trait_kind`) projects to `CanonicalTraitIdentity::Core` and is always
/// consumer-visible. A source trait resolves its canonical declaration path from the trait
/// definition and projects to `CanonicalTraitIdentity::Source` when that path is retained in
/// the public source-trait origin index. A source trait without a public origin is private and
/// excluded from the draft.
/// WHY: the `trait_source_facts` map on `ResolvedPublicTypeRootTable` intentionally retains
/// only traits needed by exported generic bounds and public incompatibility relations. A
/// public nominal target may have canonical evidence for a private source trait or a core
/// trait that appears in neither set, so the evidence projection must classify the trait from
/// the complete `TraitEnvironment` rather than from the partial `trait_source_facts` map.
fn project_evidence_trait_identity(
    evidence: &TraitEvidenceDefinition,
    context: &EvidenceProjectionContext,
) -> Result<ProjectedTrait, CompilerError> {
    // Classify the trait as core or source from the complete trait environment. A core trait
    // is always consumer-visible. A source trait is visible only when its canonical path has
    // a retained public source-trait origin. A definition whose visibility is `Core` but
    // which has no recorded `core_trait_kind` is malformed compiler metadata and must not
    // silently fall through to source-path visibility, so it is rejected.
    let definition = context.trait_environment.get(evidence.trait_id);
    if let Some(definition) = definition
        && matches!(definition.visibility, TraitVisibility::Core)
        && context
            .trait_environment
            .core_trait_kind(evidence.trait_id)
            .is_none()
    {
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft evidence projection: the evidence TraitId({}) is declared \
             as a core trait but has no recorded `CoreTraitKind`; a core trait without its \
             classifier is malformed compiler metadata and must not be silently classified as \
             a source trait",
            evidence.trait_id.0
        )));
    }

    let source_fact =
        if let Some(kind) = context.trait_environment.core_trait_kind(evidence.trait_id) {
            ResolvedTraitSourceFact::Core(kind)
        } else {
            let Some(definition) = context.trait_environment.get(evidence.trait_id) else {
                return Err(CompilerError::compiler_error(format!(
                    "public-interface draft evidence projection: the evidence TraitId({}) has no \
                     resolved definition in the TraitEnvironment and is not a registered core \
                     trait; a missing definition is an internal invariant violation",
                    evidence.trait_id.0
                )));
            };
            let path = &definition.canonical_path;
            if !context.public_source_trait_origins.contains_key(path) {
                return Ok(ProjectedTrait::Private);
            }
            ResolvedTraitSourceFact::Source(definition.canonical_path.clone())
        };

    let identity = project_trait_source_fact_to_canonical_identity(
        &source_fact,
        context.public_source_trait_origins,
    )?;
    Ok(ProjectedTrait::Public(identity))
}

/// Stable receiver-origin construction has one owner.
///
/// WHAT: `public_interface::receiver_projection` is the sole production authority that calls
/// `OriginFunctionId::new_receiver` while building the callable seed table; the declaration
/// record projection joins those exact stable receiver origins to receiver signatures and
/// attaches them to the owning struct or choice record as `PublicReceiverMethodSemantics`. The
/// evidence projection consumes that finalized surface through `collect_receiver_method_records`
/// rather than rebuilding the origin or independently iterating `ReceiverMethodCatalog`. It
/// only consumes the declaration-centric result.
///
/// Project one canonical evidence definition into a stable [`PublicEvidenceRecord`], or
/// return `Ok(None)` when the evidence is valid but non-reusable by a consumer.
///
/// WHAT: looks up the trait definition from the `TraitEnvironment` to get the authored
/// requirement order. For each requirement in that order, it finds the matching
/// `TraitRequirementEvidence` by `TraitRequirementId`, builds the stable requirement identity
/// from the canonical trait identity plus the owned requirement name, and joins the evidence
/// method path to the stable receiver-method origin produced by the declaration-centric join.
/// The join keys on the exact validated evidence method path's defining name against the
/// attached `PublicReceiverMethodSemantics` for the evidence target's exact stable receiver
/// origin. When a requirement names a method that is absent from the completed public receiver
/// surface, the whole evidence record is non-reusable: the projection returns `Ok(None)` so
/// the caller omits it without a diagnostic. A missing, extra, duplicate or name-mismatched
/// requirement, an unresolvable method path, a duplicate public method match, a wrong-receiver
/// origin or a free-function origin in the receiver surface is a `CompilerError`.
fn project_one_evidence_record(
    evidence: &TraitEvidenceDefinition,
    identity: CanonicalEvidenceIdentity,
    target_origin: OriginTypeId,
    receiver_methods: &[ReceiverMethodRecordRef<'_>],
    context: &EvidenceProjectionContext,
) -> Result<Option<PublicEvidenceRecord>, CompilerError> {
    let trait_definition = context
        .trait_environment
        .get(evidence.trait_id)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "public-interface draft evidence projection: the evidence TraitId({}) has no \
                 resolved definition in the TraitEnvironment; a missing definition is an \
                 internal invariant violation",
                evidence.trait_id.0
            ))
        })?;

    // Index the evidence requirement entries by dense requirement ID so each authored
    // requirement can be joined in authored order. A duplicate requirement ID in the evidence
    // is an internal invariant violation.
    let mut evidence_by_requirement_id: FxHashMap<TraitRequirementId, &TraitRequirementEvidence> =
        FxHashMap::default();
    for requirement_evidence in &evidence.requirements {
        if evidence_by_requirement_id
            .insert(requirement_evidence.requirement_id, requirement_evidence)
            .is_some()
        {
            return Err(CompilerError::compiler_error(format!(
                "public-interface draft evidence projection: two evidence requirement entries \
                 share TraitRequirementId({}) for trait TraitId({}); a duplicate must not \
                 silently overwrite the first",
                requirement_evidence.requirement_id.0, evidence.trait_id.0
            )));
        }
    }

    if trait_definition.requirements.len() != evidence.requirements.len() {
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft evidence projection: the trait definition for TraitId({}) \
             has {} requirement(s) but the evidence record has {}; a count mismatch is an \
             internal invariant violation",
            evidence.trait_id.0,
            trait_definition.requirements.len(),
            evidence.requirements.len()
        )));
    }

    let mut requirement_mappings = Vec::with_capacity(trait_definition.requirements.len());
    let mut seen_requirement_names: FxHashSet<String> = FxHashSet::default();

    for authored_requirement in &trait_definition.requirements {
        let requirement_name = context
            .string_table
            .resolve(authored_requirement.name)
            .to_owned();

        if !seen_requirement_names.insert(requirement_name.clone()) {
            return Err(CompilerError::compiler_error(format!(
                "public-interface draft evidence projection: the trait definition for TraitId({}) \
                 has two requirements named '{}'; a duplicate requirement name is an internal \
                 invariant violation",
                evidence.trait_id.0, requirement_name
            )));
        }

        let Some(requirement_evidence) = evidence_by_requirement_id.get(&authored_requirement.id)
        else {
            return Err(CompilerError::compiler_error(format!(
                "public-interface draft evidence projection: the authored requirement '{}' \
                 (TraitRequirementId({})) has no matching evidence entry for trait TraitId({}); \
                 a missing requirement mapping is an internal invariant violation",
                requirement_name, authored_requirement.id.0, evidence.trait_id.0
            )));
        };

        // Use the exact validated evidence method path to obtain its defining method name.
        // Matching only the public nominal declaration for the exact target origin and its
        // attached method with that defining name proves a same-named method on a different
        // receiver cannot satisfy the requirement.
        let method_name = requirement_evidence
            .method_path
            .name_str(context.string_table)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "public-interface draft evidence projection: the evidence requirement '{}' \
                     method path {:?} has no resolvable defining name; a missing defining name \
                     is an internal invariant violation",
                    requirement_name, requirement_evidence.method_path
                ))
            })?;

        let requirement_identity = StableTraitRequirementIdentity::new(
            identity.trait_identity().clone(),
            requirement_name,
        );

        let mut candidates = receiver_methods
            .iter()
            .filter(|entry| entry.receiver_origin == target_origin)
            .filter(|entry| entry.method.method_origin.defining_name() == method_name);
        let Some(method_entry) = candidates.next() else {
            // The requirement names a method absent from the completed consumer-visible
            // receiver surface. The evidence is valid inside the provider module but is
            // not reusable by a consumer, so the whole record is omitted without a
            // diagnostic rather than reported as an infrastructure failure.
            return Ok(None);
        };
        if candidates.next().is_some() {
            return Err(CompilerError::compiler_error(format!(
                "public-interface draft evidence projection: the evidence requirement '{}' \
                 matches multiple public receiver-method entries on receiver {:?} for method \
                 name '{}'; a duplicate attached method is an internal invariant violation",
                requirement_identity.requirement_name(),
                target_origin,
                method_name
            )));
        }

        // The method origin must be a receiver method for the exact target origin; a free
        // function or a different receiver's method origin is an internal invariant violation.
        let method_origin = &method_entry.method.method_origin;
        match method_origin.receiver() {
            Some(receiver) if receiver == &target_origin => {}
            Some(receiver) => {
                return Err(CompilerError::compiler_error(format!(
                    "public-interface draft evidence projection: the evidence requirement '{}' \
                     method origin {:?} names receiver {:?} but the evidence target origin is \
                     {:?}; a wrong-receiver method origin is an internal invariant violation",
                    requirement_identity.requirement_name(),
                    method_origin,
                    receiver,
                    target_origin
                )));
            }
            None => {
                return Err(CompilerError::compiler_error(format!(
                    "public-interface draft evidence projection: the evidence requirement '{}' \
                     resolved to a free-function method origin {:?} rather than a receiver \
                     method; a free-function origin in a receiver surface is an internal \
                     invariant violation",
                    requirement_identity.requirement_name(),
                    method_origin
                )));
            }
        }

        requirement_mappings.push(PublicEvidenceRequirementMapping {
            requirement_identity,
            method_origin: method_origin.clone(),
        });
    }

    Ok(Some(PublicEvidenceRecord {
        identity,
        ownership: PublicEvidenceOwnership::SourceCanonical,
        requirement_mappings,
    }))
}
