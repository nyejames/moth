//! Transient AST-owned resolved public trait-root facts.
//!
//! WHAT: builds one deterministic vector of directly-defined active-root public trait
//! declarations with their owning `this_type`, ordered location-free requirement facts and
//! publicly-authored incompatible trait ids, from the same already-resolved
//! `TraitEnvironment` consumed by public-surface validation.
//! Also owns [`AstPublicInterfaceProjectionInput`], the one closed aggregate that bundles the
//! type-root table, the direct trait-root vector and the validated receiver catalog for the
//! sole `PublicInterfaceDraftBuilder` consumer.
//! WHY: the public-interface draft trait-requirement projection needs resolved trait facts
//! available immediately before HIR lowering without reconstructing trait semantics from HIR
//! or source. This is transient donor-local AST data consumed before HIR; it never enters
//! `ModuleSemanticDraft`, `Module`, or a cross-module interface.

use crate::compiler_frontend::ast::module_ast::environment::resolved_public_type_roots::ResolvedPublicTypeRootTable;
use crate::compiler_frontend::ast::statements::functions::ReturnChannel;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::traits::definitions::{
    ResolvedTraitRequirement, TraitReceiverRequirement, TraitVisibility,
};
use crate::compiler_frontend::traits::environment::TraitEnvironment;
use crate::compiler_frontend::traits::evidence::TraitEvidenceEnvironment;
use crate::compiler_frontend::traits::ids::TraitId;
use crate::compiler_frontend::value_mode::ValueMode;

use std::rc::Rc;

/// Required receiver access kind for one trait requirement, stored separately from the self type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TraitReceiverAccessKind {
    Immutable,
    Mutable,
}

/// Transient location-free receiver fact for one trait requirement.
///
/// WHAT: carries the receiver access kind and the embedded `this_type` `TypeId`. The
/// `this_type` is retained so the projection validates it equals the owning trait `this_type`
/// before accepting immutable or mutable receiver access. Mutability is stored separately
/// from the self type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedTraitReceiverFact {
    pub(crate) access: TraitReceiverAccessKind,
    pub(crate) this_type: TypeId,
}

/// One non-receiver requirement parameter, location-free.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedTraitParameterFact {
    pub(crate) name: InternedPath,
    pub(crate) value_mode: ValueMode,
    pub(crate) type_id: TypeId,
}

/// One requirement return slot, location-free.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedTraitReturnFact {
    pub(crate) type_id: TypeId,
    pub(crate) channel: ReturnChannel,
}

/// One resolved trait method requirement, location-free and transient.
///
/// WHAT: carries the owned requirement name (`StringId`), the receiver access plus embedded
/// `this_type`, the ordered non-receiver parameters and the ordered return slots. It drops
/// every `SourceLocation` from the resolved trait requirement so the transient fact stays
/// minimal. Donor-local `TypeId`, `StringId` and `InternedPath` are consumed transiently and
/// never cross the module result boundary.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedTraitRequirementFact {
    pub(crate) name: StringId,
    pub(crate) receiver: ResolvedTraitReceiverFact,
    pub(crate) parameters: Vec<ResolvedTraitParameterFact>,
    pub(crate) returns: Vec<ResolvedTraitReturnFact>,
}

/// One directly-defined active-root public trait root, transient and donor-local.
///
/// WHAT: carries the trait canonical declaration path, its owning `this_type` `TypeId`, its
/// ordered requirement facts and its publicly-authored incompatibility `TraitId`s. The
/// `this_type` is the trait-local synthetic generic parameter owned by the resolved trait
/// definition; the projection validates every requirement receiver embedded `this_type`
/// against this value before mapping self access. Only traits authored directly in the active
/// module root under `export:` are admitted.
/// WHY: the public-interface draft projection needs resolved trait facts available
/// immediately before HIR lowering without reconstructing trait semantics from HIR or
/// source. Donor-local `TypeId`s and `StringId`s stay inside this transient fact and
/// never enter a cross-module artefact.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedPublicTraitRoot {
    pub(crate) canonical_path: InternedPath,
    pub(crate) this_type: TypeId,
    pub(crate) requirements: Vec<ResolvedTraitRequirementFact>,
    /// The publicly-authored incompatible trait ids for this direct public trait, in
    /// authored source order.
    ///
    /// WHAT: donor-local `TraitId`s resolved once from the `TraitEnvironment` public
    ///      incompatibility store. They are transient and consumed by the
    ///      public-interface draft projection, which canonicalizes them to stable
    ///      `CanonicalTraitIdentity` values before they cross the draft boundary.
    /// WHY: retaining the resolved `TraitId`s here lets the draft project
    ///      incompatibilities for each direct public trait without the
    ///      `TraitEnvironment`, which is dropped before the projection runs. The
    ///      order is the deterministic authored source order recorded by the
    ///      trait environment.
    pub(crate) incompatible_trait_ids: Vec<TraitId>,
}

/// The one closed aggregate AST-owned public-interface projection input.
///
/// WHAT: bundles the transient type-root [`ResolvedPublicTypeRootTable`] (declaration roots,
/// receiver methods and trait-source facts for generic bounds and public incompatibilities),
/// the directly-defined active-root public [`ResolvedPublicTraitRoot`] vector and the resolved
/// trait and evidence environments into one closed input for the sole
/// `PublicInterfaceDraftBuilder` consumer. It has a closed purpose: feeding the public-interface
/// draft projection (declarations, trait surfaces and reusable evidence) immediately before HIR
/// lowering. It is not an open-ended future-facts bag and never enters `ModuleSemanticDraft`,
/// `Module` or a cross-module artefact.
/// WHY: keeping the public-surface projection input in one named aggregate prevents later
/// phases from widening executable `Ast` with transient public facts. It is carried on
/// [`AstBuildResult`] and consumed by the public-interface draft builder before HIR; it never
/// reaches the `Ast` passed to HIR and may not gain unrelated future facts.
///
/// [`AstBuildResult`]: crate::compiler_frontend::ast::AstBuildResult
#[derive(Clone, Default)]
pub(crate) struct AstPublicInterfaceProjectionInput {
    pub(crate) root_table: ResolvedPublicTypeRootTable,
    pub(crate) trait_roots: Vec<ResolvedPublicTraitRoot>,
    /// The resolved trait environment, retained for direct reusable-evidence projection.
    pub(crate) trait_environment: Option<Rc<TraitEnvironment>>,
    /// The validated trait evidence environment, retained for direct reusable-evidence
    /// projection.
    pub(crate) trait_evidence_environment: Option<Rc<TraitEvidenceEnvironment>>,
}

/// Build the transient resolved public trait roots from completed AST environment facts.
///
/// WHAT: iterates sorted headers once to admit only directly-defined active-root public trait
/// declarations, then resolves each through the `TraitEnvironment` into a location-free
/// [`ResolvedPublicTraitRoot`] with its owning `this_type`, ordered requirement facts and the
/// publicly-authored incompatible trait ids for that trait. A trait header that passes the
/// active-root public declaration gate must resolve to exactly one definition; a missing
/// definition or a missing `this_type` is a `CompilerError` rather than a silent omission. The
/// requirement order is preserved exactly as the trait definition records it. Private traits,
/// non-active-root/imported traits and compiler-owned core traits are excluded because they do
/// not pass the active-root public declaration gate. The incompatible trait ids come from the
/// trait environment public incompatibility store, so a private `must not` relation never
/// reaches a direct public trait record.
/// WHY: one pass over the same sorted headers keeps a single deterministic owner. The
/// `TraitEnvironment` is dropped after this vector is built; the projection consumes these
/// facts to build trait surfaces without the `TraitEnvironment`.
pub(crate) fn build_resolved_public_trait_roots(
    sorted_headers: &[Header],
    trait_environment: &TraitEnvironment,
    string_table: &StringTable,
) -> Result<Vec<ResolvedPublicTraitRoot>, CompilerError> {
    let mut trait_roots = Vec::new();

    for header in sorted_headers {
        if !is_active_root_public_trait_declaration(header) {
            continue;
        }

        let path = &header.tokens.src_path;
        trait_roots.push(build_trait_root(path, trait_environment, string_table)?);
    }

    Ok(trait_roots)
}

/// Whether a header is a directly-defined active-root public authored trait declaration.
///
/// WHAT: only trait declarations in the active module root's public export surface, admitted
/// by the shared [`HeaderKind::is_authored_public_export_declaration`] gate, become trait
/// roots. Imported module roots, private traits, builtin declarations and source-package-only
/// facts are excluded.
fn is_active_root_public_trait_declaration(header: &Header) -> bool {
    header.file_role.is_active_module_root()
        && header.export_mode.is_public()
        && header.kind.is_authored_public_export_declaration()
        && matches!(header.kind, HeaderKind::Trait { .. })
}

/// Build one transient public trait root from the resolved trait definition for a
/// directly-defined active-root public trait header.
///
/// WHAT: looks up the trait by its canonical declaration path in the `TraitEnvironment`,
/// then copies its owning `this_type` and its ordered requirements into location-free
/// transient facts. A trait header that passes the active-root public declaration gate must
/// resolve to exactly one definition; a missing definition or a missing `this_type` is a
/// `CompilerError` rather than a silent omission. The requirement order is preserved exactly
/// as the trait definition records it.
fn build_trait_root(
    canonical_path: &InternedPath,
    trait_environment: &TraitEnvironment,
    string_table: &StringTable,
) -> Result<ResolvedPublicTraitRoot, CompilerError> {
    let Some(trait_id) = trait_environment.id_for_path(canonical_path) else {
        return Err(CompilerError::compiler_error(format!(
            "resolved public trait-root construction: a public active-root trait '{}' has no \
             registered TraitEnvironment definition",
            canonical_path.to_string(string_table)
        )));
    };

    let Some(definition) = trait_environment.get(trait_id) else {
        return Err(CompilerError::compiler_error(format!(
            "resolved public trait-root construction: TraitId({}) for trait '{}' has no resolved \
             definition",
            trait_id.0,
            canonical_path.to_string(string_table)
        )));
    };

    // Compiler-owned core traits are never authored as source declarations and must not
    // enter the public trait-root vector. A header that resolves to a core trait is
    // malformed transient AST data.
    if matches!(definition.visibility, TraitVisibility::Core) {
        return Err(CompilerError::compiler_error(format!(
            "resolved public trait-root construction: a public active-root trait '{}' resolved \
             to a compiler-owned core trait; core traits are not authored source declarations",
            canonical_path.to_string(string_table)
        )));
    }

    let requirements = definition
        .requirements
        .iter()
        .map(build_trait_requirement_fact)
        .collect();

    // Carry the publicly-authored incompatible trait ids for this direct public trait. They
    // are donor-local and transient; the public-interface draft projection canonicalizes them
    // to stable `CanonicalTraitIdentity` values before they cross the draft boundary. The
    // order is the deterministic authored source order recorded by the trait environment.
    let incompatible_trait_ids = trait_environment
        .public_incompatibilities_for(trait_id)
        .to_vec();

    Ok(ResolvedPublicTraitRoot {
        canonical_path: canonical_path.to_owned(),
        this_type: definition.this_type,
        requirements,
        incompatible_trait_ids,
    })
}

/// Copy one resolved trait requirement into a location-free transient fact.
///
/// WHAT: keeps the owned name, the receiver access kind plus embedded `this_type`, the ordered
/// non-receiver parameters and the ordered return slots, dropping every `SourceLocation`. The
/// resolved receiver enum always contains `this_type`, so this copy is infallible.
fn build_trait_requirement_fact(
    requirement: &ResolvedTraitRequirement,
) -> ResolvedTraitRequirementFact {
    let (access, this_type) = match requirement.receiver {
        TraitReceiverRequirement::Immutable { this_type } => {
            (TraitReceiverAccessKind::Immutable, this_type)
        }
        TraitReceiverRequirement::Mutable { this_type } => {
            (TraitReceiverAccessKind::Mutable, this_type)
        }
    };

    let parameters = requirement
        .parameters
        .iter()
        .map(|parameter| ResolvedTraitParameterFact {
            name: parameter.name.clone(),
            value_mode: parameter.value_mode.clone(),
            type_id: parameter.type_id,
        })
        .collect();

    let returns = requirement
        .returns
        .iter()
        .map(|return_slot| ResolvedTraitReturnFact {
            type_id: return_slot.type_id,
            channel: return_slot.channel,
        })
        .collect();

    ResolvedTraitRequirementFact {
        name: requirement.name,
        receiver: ResolvedTraitReceiverFact { access, this_type },
        parameters,
        returns,
    }
}
