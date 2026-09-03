//! Receiver-method callable identity and signature projection.
//!
//! WHAT: owns the transient callable seed table built at the post-AST boundary and the one-shot
//! canonical signature projection for receiver-method seeds. The seed table is the single
//! authority pairing a donor-local declaration path with its stable public origin and callable
//! classification; the declaration-record projection, HIR origin seeding and generic-template
//! extraction all consume it.
//!
//! WHY: R2C3 requires one callable identity authority consumed by direct projection,
//! declaration-record projection, HIR origin seeding and generic-template extraction. Keeping
//! the seed table and receiver-method signature projection in one module separates callable
//! identity from the per-binding type and trait projection while preserving the proven,
//! deterministic construction logic.

use super::model::{
    PublicParameterTypeSlot, PublicReceiverMethodCategory, PublicReceiverMethodSemantics,
};
use super::type_projection::{
    ProjectedReceiverMethodSignature, RootIndex, origin_category_mismatch_error,
    project_defaults_provenance, project_folded_default, project_parameter_access,
    project_return_slots,
};
use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
use crate::compiler_frontend::ast::{
    ReceiverMethodEntry, ResolvedPublicTypeRootKind, ResolvedPublicTypeRootTable,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTypeProjectionContext, project_type_id_to_canonical_identity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::folded_value::FoldedValueProjectionContext;
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginDeclarationId, OriginFunctionId, OriginTypeCategory, OriginTypeId,
    StableModuleOriginIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::{FxHashMap, FxHashSet};

/// One transient callable identity seed: the single authority pairing a donor-local
/// declaration path with its stable public origin and callable classification.
///
/// WHAT: carries only the identity facts required before stable projection: the exact
/// donor-local declaration path, the stable public [`OriginFunctionId`], the generic-template
/// classification, and a [`CallableSeedKind`] that distinguishes free functions from receiver
/// methods without `Option` combinations. A receiver-method seed carries the stable receiver
/// [`OriginTypeId`] and a narrow `method_index` into the resolved public type-root table's
/// `receiver_methods` vector so the projected signature is read once through the existing type
/// owner rather than copied into the seed.
/// WHY: consolidates the former `PublicCallableOriginSeed` and
/// `DefinedPublicReceiverMethodTypeSurface` into one record so direct projection,
/// declaration-record projection, HIR origin seeding and generic-template extraction all
/// consume one authority without reconstructing path or origin. Private callables remain
/// excluded by the upstream public-surface filter and never receive a seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CallableSeed {
    pub(crate) path: InternedPath,
    pub(crate) origin: OriginFunctionId,
    pub(crate) generic_template: bool,
    pub(crate) kind: CallableSeedKind,
}

/// Distinguishes free-function and receiver-method callable sources.
///
/// WHAT: a `FreeFunction` seed references its signature through the free-function type surface.
/// A `ReceiverMethod` seed carries the stable receiver origin and a narrow index into the
/// resolved public type-root table's `receiver_methods` vector, so the projected canonical
/// signature is read once from the existing type owner rather than duplicated in the seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CallableSeedKind {
    FreeFunction,
    ReceiverMethod {
        receiver_origin: OriginTypeId,
        method_index: usize,
    },
}

/// Build the one transient callable seed table at the post-AST/pre-HIR public-projection
/// boundary.
///
/// WHAT: joins the pre-AST export bindings and module origin with the completed AST
///       [`ResolvedPublicTypeRootTable`] (resolved roots and receiver-method entries) and the
///       validated generic-template map to produce one deterministic, contiguous seed table.
///       Free-function seeds are built by matching function export bindings to roots for the
///       exact declaration path and generic-parameter classification. Receiver-method seeds are
///       built from `root_table.receiver_methods`, resolving each receiver path to its stable
///       [`OriginTypeId`] through the nominal-type origin index. Each receiver-method seed
///       carries a narrow `method_index` into the root table so the signature is projected
///       exactly once by the receiver projection owner.
/// WHY: R2C3 requires one callable identity authority consumed by direct projection,
///      declaration-record projection, HIR origin seeding and generic-template extraction.
///      Building the table at this single boundary — where all inputs are available — removes
///      the former `ReceiverSurfaceOrigins` parallel authority and prevents any consumer from
///      reconstructing callable identity from the receiver catalog.
///
/// Duplicate rejection: every duplicate exact declaration path and every duplicate stable public
/// origin is rejected. Same-named methods on distinct receivers have distinct declaration paths
/// (the source path includes the owning file) and distinct origins, so they remain valid.
pub(crate) fn build_callable_seed_table(
    export_bindings: &[ExportBinding],
    module_origin: &StableModuleOriginIdentity,
    public_nominal_type_origins: &FxHashMap<InternedPath, OriginTypeId>,
    root_table: &ResolvedPublicTypeRootTable,
    generic_function_templates: &FxHashMap<InternedPath, GenericFunctionTemplate>,
    string_table: &StringTable,
) -> Result<Vec<CallableSeed>, CompilerError> {
    let mut seeds: Vec<CallableSeed> = Vec::new();

    // Free-function seeds: match function export bindings to roots for the exact path and
    // generic-parameter classification. Non-function bindings produce no seed but still consume
    // their root so a binding with no matching root is rejected here.
    let mut root_index = RootIndex::new(&root_table.roots, string_table)?;

    let mut seen_origins = FxHashSet::default();

    for binding in export_bindings {
        if binding.origin().module_origin() != module_origin {
            continue;
        }
        if matches!(binding.origin(), OriginDeclarationId::Trait(_)) {
            continue;
        }
        if !seen_origins.insert(binding.origin().clone()) {
            continue;
        }
        let root = root_index.take_for_binding(binding)?;
        if let ResolvedPublicTypeRootKind::Function {
            generic_parameter_list_id,
            ..
        } = &root.kind
        {
            let OriginDeclarationId::Function(function_origin) = binding.origin() else {
                return Err(origin_category_mismatch_error("function", binding));
            };
            push_seed(
                &mut seeds,
                root.path.clone(),
                function_origin.clone(),
                generic_parameter_list_id.is_some(),
                CallableSeedKind::FreeFunction,
            )?;
        }
    }

    // Receiver-method seeds: iterate the root table's receiver-method entries (already filtered
    // to public nominal receivers) and resolve each receiver path to its stable origin through
    // the nominal-type origin index. Receiver seeds are pushed into the same `seeds` vec so
    // `push_seed` rejects any duplicate path or origin across free-function and receiver-method
    // seeds.
    let receiver_seed_start = seeds.len();
    if !root_table.receiver_methods.is_empty() {
        for (method_index, entry) in root_table.receiver_methods.iter().enumerate() {
            let (receiver_path, expected_category) = match &entry.receiver {
                ReceiverKey::Struct(path) => (path, OriginTypeCategory::Struct),
                ReceiverKey::Choice(path) => (path, OriginTypeCategory::Choice),
                ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_) => {
                    return Err(CompilerError::compiler_error(format!(
                        "callable seed construction: a resolved receiver-method entry carries a \
                         non-nominal receiver key ({:?}); receiver methods must live on a \
                         nominal struct or choice",
                        entry.receiver
                    )));
                }
            };

            let receiver_origin = public_nominal_type_origins.get(receiver_path).ok_or_else(
                || {
                    CompilerError::compiler_error(format!(
                        "callable seed construction: a receiver-method entry's receiver path is \
                         not in the public nominal-type origin index (path: {:?}); the root \
                         table should only select public nominal receivers",
                        receiver_path
                    ))
                },
            )?;
            if receiver_origin.category() != expected_category {
                return Err(CompilerError::compiler_error(format!(
                    "callable seed construction: a receiver-method entry's receiver key expects \
                     a {expected_category:?} origin but the resolved nominal origin is a {:?} \
                     (receiver path: {:?}); the receiver key and stable origin category disagree",
                    receiver_origin.category(),
                    receiver_path
                )));
            }

            let method_name = entry.function_path.name_str(string_table).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "callable seed construction: a receiver-method entry has no resolvable \
                         defining method name (path: {:?})",
                    entry.function_path
                ))
            })?;

            let method_origin = OriginFunctionId::new_receiver(
                module_origin.clone(),
                method_name.to_owned(),
                receiver_origin.clone(),
            );

            let generic_template = generic_function_templates.contains_key(&entry.function_path);
            push_seed(
                &mut seeds,
                entry.function_path.clone(),
                method_origin,
                generic_template,
                CallableSeedKind::ReceiverMethod {
                    receiver_origin: receiver_origin.clone(),
                    method_index,
                },
            )?;
        }

        // Stable semantic origins provide a deterministic total order without allocating or
        // falling back to rendered-name tuples.
        seeds[receiver_seed_start..].sort_by(|left, right| left.origin.cmp(&right.origin));
    }

    Ok(seeds)
}

/// Push one seed, rejecting every duplicate exact declaration path and every duplicate stable
/// public origin.
///
/// Same-named methods on distinct receivers have distinct declaration paths and distinct origins,
/// so they are never rejected here.
fn push_seed(
    seeds: &mut Vec<CallableSeed>,
    path: InternedPath,
    origin: OriginFunctionId,
    generic_template: bool,
    kind: CallableSeedKind,
) -> Result<(), CompilerError> {
    if seeds.iter().any(|existing| existing.origin == origin) {
        return Err(CompilerError::compiler_error(format!(
            "callable seed construction: duplicate stable function origin {:?}",
            origin
        )));
    }
    if seeds.iter().any(|existing| existing.path == path) {
        return Err(CompilerError::compiler_error(format!(
            "callable seed construction: duplicate public callable declaration path {:?}",
            path
        )));
    }
    seeds.push(CallableSeed {
        path,
        origin,
        generic_template,
        kind,
    });
    Ok(())
}
/// Inputs shared by receiver-method type and folded-default projection.
///
/// WHAT: keeps the canonical type environment, projection context, string table and structural
/// folded-string resources together so receiver signatures do not grow raw resource parameters.
/// WHY: receiver defaults use the same public folded-value owner as direct constants and fields.
pub(super) struct ReceiverProjectionContext<'a> {
    pub(super) type_environment: &'a TypeEnvironment,
    pub(super) projection_context: &'a CanonicalTypeProjectionContext<'a>,
    pub(super) string_table: &'a StringTable,
    pub(super) folded_value_context: &'a FoldedValueProjectionContext<'a>,
}

/// Project the canonical signature for each receiver-method callable seed.
///
/// WHAT: iterates the receiver-method seeds (filtered by kind) and reads the resolved
/// `ReceiverMethodEntry` by the seed's `method_index` into the root table's `receiver_methods`
/// vector. Each entry's `FunctionSignature` is projected once into a
/// [`ProjectedReceiverMethodSignature`] keyed by `method_index`. Free-function seeds are
/// skipped; their signatures are projected by the free-function projection owner.
/// WHY: the seed carries only an index, not a copied signature, so the canonical projection
/// happens exactly once through this owner. The parameter-default provenance is retained on the
/// transient projected signature for aggregation onto the owning nominal record.
pub(crate) fn project_receiver_method_signatures(
    callable_seeds: &[CallableSeed],
    receiver_method_entries: &[ReceiverMethodEntry],
    context: &ReceiverProjectionContext<'_>,
) -> Result<FxHashMap<usize, ProjectedReceiverMethodSignature>, CompilerError> {
    let type_environment = context.type_environment;
    let canonical_context = context.projection_context;
    let string_table = context.string_table;
    let mut signatures: FxHashMap<usize, ProjectedReceiverMethodSignature> = FxHashMap::default();

    for seed in callable_seeds {
        let CallableSeedKind::ReceiverMethod { method_index, .. } = &seed.kind else {
            continue;
        };

        let entry = receiver_method_entries.get(*method_index).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "receiver-method signature projection: a receiver-method callable seed carries \
                 method_index {} but the root table has only {} receiver-method entries",
                method_index,
                receiver_method_entries.len()
            ))
        })?;

        let default_provenance = project_defaults_provenance(&entry.signature.parameters);
        let parameters = entry
            .signature
            .parameters
            .iter()
            .map(|declaration| {
                let name = declaration
                    .id
                    .name_str(string_table)
                    .map(|name| name.to_owned());
                let type_identity = project_type_id_to_canonical_identity(
                    declaration.value.type_id,
                    type_environment,
                    canonical_context,
                )?;
                let folded_default =
                    project_folded_default(&declaration.value, context.folded_value_context)?;
                let access = project_parameter_access(declaration)?;
                Ok(PublicParameterTypeSlot {
                    name,
                    type_identity,
                    access,
                    folded_default,
                })
            })
            .collect::<Result<Vec<_>, CompilerError>>()?;

        let (returns, error_return) = project_return_slots(
            &entry.signature.returns,
            type_environment,
            canonical_context,
        )?;

        if signatures
            .insert(
                *method_index,
                ProjectedReceiverMethodSignature {
                    parameters,
                    returns,
                    error_return,
                    default_provenance,
                },
            )
            .is_some()
        {
            return Err(CompilerError::compiler_error(format!(
                "receiver-method signature projection: two receiver-method seeds share \
                 method_index {}; a duplicate must not silently overwrite the first",
                method_index
            )));
        }
    }

    Ok(signatures)
}

/// Build one [`PublicReceiverMethodSemantics`] from a receiver-method callable seed and its
/// projected signature.
///
/// WHAT: reads the method origin and generic-template classification from the seed, and the
/// projected canonical parameter/return/error-return types from the signature map keyed by the
/// seed's `method_index`. The receiver origin is not repeated here because the semantics attach
/// to the owning struct or choice declaration record. A seed that is not a `ReceiverMethod` or
/// whose `method_index` has no projected signature is a `CompilerError` rather than a panic:
/// the construction boundary makes invalid combinations impossible in production, but this
/// function remains total for test-facing callers.
pub(super) fn receiver_method_semantics_from_seed(
    seed: &CallableSeed,
    receiver_method_signatures: &FxHashMap<usize, ProjectedReceiverMethodSignature>,
) -> Result<PublicReceiverMethodSemantics, CompilerError> {
    let CallableSeedKind::ReceiverMethod { method_index, .. } = &seed.kind else {
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft join: a non-receiver callable seed {:?} was passed to \
             receiver-method semantics projection",
            seed.origin
        )));
    };
    let signature = receiver_method_signatures
        .get(method_index)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "public-interface draft join: receiver-method seed with method_index {} has no \
             projected signature",
                method_index
            ))
        })?;
    Ok(PublicReceiverMethodSemantics {
        method_origin: seed.origin.clone(),
        category: if seed.generic_template {
            PublicReceiverMethodCategory::GenericTemplate
        } else {
            PublicReceiverMethodCategory::ConcreteLocal
        },
        parameters: signature.parameters.clone(),
        returns: signature.returns.clone(),
        error_return: signature.error_return.clone(),
    })
}
