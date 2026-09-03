//! Direct declaration projection: the builder, named input/result, and the per-binding
//! declaration-record projection.
//!
//! WHAT: owns [`PublicInterfaceDraftBuilder`] and the declaration-centric projection that
//! consumes the direct-export seed, the resolved public type-root table, the callable seed
//! table, the direct trait projection and the folded-value context into one
//! [`PublicDeclarationRecord`] per stable origin. Export bindings drive the deterministic
//! iteration. Each non-trait binding consumes exactly one resolved root and immediately
//! produces its final [`PublicDeclarationSemantics`]; a direct trait binding projects its
//! final [`PublicTraitSemantics`]. Receiver methods are grouped by receiver origin and
//! attached to their owning struct or choice record. Folded constant values are projected by
//! exact defining path during the owning declaration projection.
//!
//! WHY: the compiler design overview requires one aggregate producer boundary with a
//! declaration-centric shape. Building the records directly in export-binding order removes
//! the former `DefinedPublic*` aggregate vectors and the rejoin pass, while preserving the
//! proven totality checks (every root, trait root and receiver method joins exactly one
//! binding) and the deterministic ordering. Keeping the builder and folded-value context in
//! one module separates the join from the model vocabulary, type projection, receiver
//! projection, trait projection, evidence projection and local finalization.

use super::evidence_projection::{EvidenceProjectionContext, project_reusable_evidence};
use super::export_projection::{DirectExportSeed, DirectExportSeedParts};
use super::model::{
    PublicAliasSemantics, PublicChoiceSemantics, PublicConstantSemantics, PublicDeclarationRecord,
    PublicDeclarationSemantics, PublicInterfaceDraft, PublicStructSemantics,
};
use super::receiver_projection::{
    CallableSeed, CallableSeedKind, ReceiverProjectionContext, build_callable_seed_table,
    project_receiver_method_signatures, receiver_method_semantics_from_seed,
};
use super::trait_projection::DirectTraitProjection;
use super::type_projection::{
    RootIndex, TransientGenericParameterOriginResolver, TransientNominalOriginResolver,
    project_alias_target, project_choice_parts, project_constant_type_identity,
    project_defaults_provenance, project_free_function_semantics, project_struct_parts,
};
use crate::compiler_frontend::ast::AstPublicInterfaceProjectionInput;
use crate::compiler_frontend::ast::const_values::store::{ConstValueId, ConstValueStore};
use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
use crate::compiler_frontend::canonical_type_identity::CanonicalTypeProjectionContext;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    FoldedValueGenericParameterResolver, FoldedValueProjectionContext, PublicFoldedValue,
    convert_const_value_to_folded_value_with_provenance,
};
use crate::compiler_frontend::hir::functions::FunctionOriginSeed;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginDeclarationId, OriginTypeCategory, OriginTypeId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use rustc_hash::{FxHashMap, FxHashSet};

/// Typed pre-HIR result carrying the draft and its transient exact declaration-path metadata.
///
/// WHAT: keeps the stable public draft separate from the `InternedPath` values needed only to
/// seed the HIR stable-origin/local-`FunctionId` relationship and validate generic body joins.
/// The path side table is consumed before HIR lowering and never enters `PublicInterfaceDraft` or
/// `ModuleSemanticResult`.
/// WHY: the origin join must be established while exact AST declaration identity is available;
/// later stages must not reconstruct it from rendered names, paths or declaration order.
pub(crate) struct PublicInterfaceDraftBuildResult {
    pub(crate) draft: PublicInterfaceDraft,
    pub(crate) function_origin_seeds: Vec<FunctionOriginSeed>,
    pub(crate) callable_seeds: Vec<CallableSeed>,
}

// ===========================================================================
//  Builder
// ===========================================================================

/// Named inputs for [`PublicInterfaceDraftBuilder`].
///
/// WHAT: groups the pre-AST direct-export seed, the post-AST public-interface projection
/// input and the shared projection side tables into one construction value so the builder
/// does not take a long positional parameter list.
/// WHY: keeping the inputs named makes the construction boundary easier to audit than seven
/// positional arguments.
pub(in crate::compiler_frontend) struct PublicInterfaceDraftBuilderInput<'a> {
    pub export_seed: DirectExportSeed,
    pub public_interface_projection_input: AstPublicInterfaceProjectionInput,
    pub public_source_nominal_type_origins: &'a FxHashMap<InternedPath, OriginTypeId>,
    pub public_source_trait_origins:
        &'a FxHashMap<InternedPath, crate::compiler_frontend::semantic_identity::OriginTraitId>,
    pub type_environment: &'a TypeEnvironment,
    pub external_registry: &'a ExternalPackageRegistry,
    pub string_table: &'a StringTable,
    /// Validated generic callable templates retained by AST generic-template validation.
    /// Borrowed only while the transient type projection records the corresponding stable callable
    /// origin and aliases receiver-method local generic parameter IDs; no path or template
    /// enters the declaration-centric draft.
    pub generic_function_templates: &'a FxHashMap<InternedPath, GenericFunctionTemplate>,
    /// The module-local folded-value authority from AST finalization.
    ///
    /// WHAT: each public constant root joins to a store row by its exact defining
    /// `InternedPath`, then converts the stored value to an owned [`PublicFoldedValue`].
    pub const_values: &'a ConstValueStore,
    /// The module-local resource table that issued structural-string resource handles, when
    /// available. Public projection never flattens a structural value when this is absent.
    pub module_resources: Option<&'a ModuleResourceTable>,
}

/// Builds the one aggregate [`PublicInterfaceDraft`] from already-resolved pre-HIR facts.
///
/// WHAT: the sole construction path for the draft. It internalizes the callable seed table,
/// the canonical type projection, the direct trait projection and the per-binding
/// declaration-record projection as private builder steps, so no parallel `DefinedPublic*`
/// producer result crosses orchestration. It consumes the pre-AST direct-export seed, the
/// consolidated AST public-interface projection input and the transient expanded public
/// source-nominal and source-trait origin indexes, while both the `TypeEnvironment` and
/// `ExternalPackageRegistry` are still available. The output is retained only on overall
/// semantic success.
pub(in crate::compiler_frontend) struct PublicInterfaceDraftBuilder<'a> {
    input: PublicInterfaceDraftBuilderInput<'a>,
}

impl<'a> PublicInterfaceDraftBuilder<'a> {
    /// Construct the builder from one named input value.
    ///
    /// Compiler-internal: the frontend orchestration constructs this once per module
    /// compilation, after AST construction succeeds and before HIR lowering consumes the AST.
    pub(crate) fn new(input: PublicInterfaceDraftBuilderInput<'a>) -> Self {
        Self { input }
    }

    /// Build the aggregate draft.
    ///
    /// WHAT: runs the projection steps in order, then projects one declaration record per
    /// stable origin in deterministic export-binding order:
    /// 1. build the callable seed table from the seed and the resolved root table,
    /// 2. build the canonical type projection context (transient nominal and generic
    ///    parameter origin resolvers) and project receiver-method signatures once,
    /// 3. build the direct trait projection state,
    /// 4. iterate the export bindings: each non-trait binding consumes exactly one resolved
    ///    root and immediately produces its final `PublicDeclarationSemantics`; a direct
    ///    trait binding projects its final `PublicTraitSemantics`. Receiver methods attach
    ///    once to their owning struct or choice record, and constant values fold by exact
    ///    defining path.
    /// 5. project reusable evidence from the completed declaration records.
    ///
    /// Each step is total: a missing, duplicate, unmatched or malformed fact is a
    /// `CompilerError` rather than a silent omission. The transient projection indexes and
    /// the seed are consumed before the draft boundary: the draft never stores a
    /// `DefinedPublic*` component.
    pub(crate) fn build(self) -> Result<PublicInterfaceDraftBuildResult, CompilerError> {
        let PublicInterfaceDraftBuilderInput {
            export_seed,
            public_interface_projection_input,
            public_source_nominal_type_origins,
            public_source_trait_origins,
            type_environment,
            external_registry,
            string_table,
            generic_function_templates,
            const_values,
            module_resources,
        } = self.input;

        let AstPublicInterfaceProjectionInput {
            root_table,
            trait_roots,
            trait_environment,
            trait_evidence_environment,
        } = public_interface_projection_input;

        let trait_environment = trait_environment.ok_or_else(|| {
            CompilerError::compiler_error(
                "public-interface draft construction: AST finalization did not retain its \
                 resolved trait environment; the reusable-evidence projection cannot look up \
                 trait definitions without it",
            )
        })?;
        let trait_evidence_environment = trait_evidence_environment.ok_or_else(|| {
            CompilerError::compiler_error(
                "public-interface draft construction: AST finalization did not retain its \
                 validated trait evidence environment; the reusable-evidence projection cannot \
                 iterate source-authored evidence without it",
            )
        })?;

        // One shared transient nominal origin resolver backs the type, trait, folded-value
        // and evidence projections. It reads the expanded public source-nominal origin index
        // (the builder input), distinct from the directly-defined nominal index carried on the
        // seed for receiver-method resolution.
        let nominal_resolver = TransientNominalOriginResolver::new(
            type_environment,
            public_source_nominal_type_origins,
        );

        // Build the transient generic-parameter origin resolver from the resolved roots and
        // the seed's export bindings, then the canonical type projection context used by the
        // free-function, nominal, alias, constant-type and receiver-method projections.
        let mut type_generic_resolver = TransientGenericParameterOriginResolver::new();
        super::type_projection::register_generic_parameter_origins(
            &mut type_generic_resolver,
            &root_table,
            export_seed.export_bindings(),
            generic_function_templates,
            &nominal_resolver,
            type_environment,
            string_table,
        )?;
        let type_projection_context = CanonicalTypeProjectionContext::new(
            &nominal_resolver,
            &type_generic_resolver,
            external_registry,
        );

        // Build the one transient callable seed table at the post-AST boundary where the
        // resolved root table, receiver entries, generic-template classification and stable
        // export origins are all available. This is the single callable identity authority.
        let callable_seeds = build_callable_seed_table(
            export_seed.export_bindings(),
            export_seed.module_origin(),
            export_seed.public_nominal_type_origins(),
            &root_table,
            generic_function_templates,
            string_table,
        )?;

        // Build the folded-value projection context from the same shared nominal resolver and
        // module-local resource table. Folded constants are concrete, so a generic parameter
        // reaching this projection remains an invariant violation.
        let folded_generic_resolver = FoldedValueGenericParameterResolver;
        let folded_projection_context = CanonicalTypeProjectionContext::new(
            &nominal_resolver,
            &folded_generic_resolver,
            external_registry,
        );
        let folded_value_projection_context = FoldedValueProjectionContext {
            type_environment,
            string_table,
            projection_context: &folded_projection_context,
            resources: module_resources,
        };
        let folded_value_context = FoldedValueJoinContext {
            folded_value_projection_context: &folded_value_projection_context,
            const_values,
        };
        let receiver_projection_context = ReceiverProjectionContext {
            type_environment,
            projection_context: &type_projection_context,
            string_table,
            folded_value_context: &folded_value_projection_context,
        };
        let receiver_method_signatures = project_receiver_method_signatures(
            &callable_seeds,
            &root_table.receiver_methods,
            &receiver_projection_context,
        )?;

        // Group receiver-method seeds by their stable receiver origin so each struct or choice
        // record attaches its methods exactly once. A duplicate method origin is rejected here.
        let mut receiver_seeds_by_receiver: FxHashMap<OriginTypeId, Vec<&CallableSeed>> =
            FxHashMap::default();
        let mut seen_method_origins: FxHashSet<
            crate::compiler_frontend::semantic_identity::OriginFunctionId,
        > = FxHashSet::default();
        for seed in &callable_seeds {
            let CallableSeedKind::ReceiverMethod {
                receiver_origin, ..
            } = &seed.kind
            else {
                continue;
            };
            if !seen_method_origins.insert(seed.origin.clone()) {
                return Err(CompilerError::compiler_error(format!(
                    "public-interface draft join: two callable seeds share method origin {:?}; a duplicate must not silently overwrite the first",
                    seed.origin
                )));
            }
            receiver_seeds_by_receiver
                .entry(receiver_origin.clone())
                .or_default()
                .push(seed);
        }

        // Track consumption by store ID so each public constant joins one owned value row
        // exactly once. The store remains the sole folded-value owner.
        let mut consumed_const_values = FxHashSet::default();

        // The direct trait projection holds the trait-root index and projects one final
        // PublicTraitSemantics per trait binding.
        let mut trait_projection =
            DirectTraitProjection::new(super::trait_projection::DirectTraitProjectionInput {
                trait_roots: &trait_roots,
                trait_source_facts: &root_table.trait_source_facts,
                public_source_nominal_type_origins,
                public_source_trait_origins,
                type_environment,
                external_registry,
                string_table,
            })?;

        let type_context = DeclarationTypeProjectionContext {
            type_projection_context: &type_projection_context,
            public_source_trait_origins,
            type_environment,
            string_table,
            folded_value_context: &folded_value_projection_context,
        };
        let mut state = DeclarationRecordProjectionState {
            receiver_seeds_by_receiver: &mut receiver_seeds_by_receiver,
            receiver_method_signatures: &receiver_method_signatures,
            consumed_const_values: &mut consumed_const_values,
            trait_projection: &mut trait_projection,
            folded_value_context: &folded_value_context,
        };
        let declarations = project_declaration_records(
            export_seed.export_bindings(),
            &root_table,
            &type_context,
            &mut state,
        )?;

        let function_origin_seeds = callable_seeds
            .iter()
            .filter(|seed| !seed.generic_template)
            .map(|seed| FunctionOriginSeed {
                path: seed.path.clone(),
                origin: seed.origin.clone(),
            })
            .collect();

        let evidence_projection_context = EvidenceProjectionContext {
            trait_environment: &trait_environment,
            trait_evidence_environment: &trait_evidence_environment,
            public_source_nominal_type_origins,
            public_source_trait_origins,
            type_environment,
            string_table,
            projection_context: &folded_projection_context,
        };
        // Reusable evidence projection runs after the declaration-centric projection so the
        // already-finalized `PublicReceiverMethodSemantics` values attached to each struct
        // or choice record are the evidence projection's sole receiver-origin authority. The
        // evidence projection never reconstructs receiver-method origins and never iterates
        // `ReceiverMethodCatalog`.
        let reusable_evidence =
            project_reusable_evidence(&declarations, &evidence_projection_context)?;

        // Consume the seed: the module origin and export bindings move into the draft and the
        // directly-defined nominal origin index is dropped.
        let DirectExportSeedParts {
            module_origin,
            export_bindings,
            export_diagnostic_provenance,
            binding_exports,
        } = export_seed.into_parts();

        Ok(PublicInterfaceDraftBuildResult {
            draft: PublicInterfaceDraft {
                module_origin,
                export_bindings,
                export_diagnostic_provenance,
                binding_exports,
                declarations,
                reusable_evidence,
            },
            function_origin_seeds,
            callable_seeds,
        })
    }
}
/// Context for joining module-store folded values into public declaration records.
///
/// WHAT: bundles the shared public folded-value conversion context with the module value store,
/// keeping resource-table lookup and canonical projection together at the declaration boundary.
/// WHY: the converter must read structural strings through the table that issued their local
/// handles and emit portable stable origins, never flattening them into text.
pub(super) struct FoldedValueJoinContext<'a> {
    pub(super) folded_value_projection_context: &'a FoldedValueProjectionContext<'a>,
    pub(super) const_values: &'a ConstValueStore,
}

// ===========================================================================
//  Per-binding declaration-record projection
// ===========================================================================

/// Read-only shared type-projection inputs for per-binding declaration-record projection.
///
/// WHAT: bundles the canonical type projection context, the public source-trait origin index,
/// the type environment and the string table so each per-binding projection helper receives one
/// named context instead of four positional references.
struct DeclarationTypeProjectionContext<'a> {
    type_projection_context: &'a CanonicalTypeProjectionContext<'a>,
    public_source_trait_origins:
        &'a FxHashMap<InternedPath, crate::compiler_frontend::semantic_identity::OriginTraitId>,
    type_environment: &'a TypeEnvironment,
    string_table: &'a StringTable,
    folded_value_context: &'a FoldedValueProjectionContext<'a>,
}

/// Mutable per-binding join state consumed while projecting declaration records.
///
/// WHAT: bundles the receiver-method seed index and signatures, the module-constant value IDs
/// index, the trait projection and the folded-value context so the per-binding projection
/// mutates one named state value instead of managing five mutable positional parameters. The
/// `'a` lifetime is the shared data lifetime for borrowed callable seeds, declarations and the
/// trait projection internals; the second lifetime is the mutable borrow of this state.
struct DeclarationRecordProjectionState<'a, 'b> {
    receiver_seeds_by_receiver: &'b mut FxHashMap<OriginTypeId, Vec<&'a CallableSeed>>,
    receiver_method_signatures:
        &'b FxHashMap<usize, super::type_projection::ProjectedReceiverMethodSignature>,
    consumed_const_values: &'b mut FxHashSet<ConstValueId>,
    trait_projection: &'b mut DirectTraitProjection<'a>,
    folded_value_context: &'b FoldedValueJoinContext<'a>,
}

/// Project one [`PublicDeclarationRecord`] per stable origin in deterministic export-binding
/// order.
///
/// WHAT: iterates the export bindings in their deterministic order. For each unique origin the
/// matching resolved root is consumed (non-trait bindings) and its semantic facts are projected
/// directly into a [`PublicDeclarationRecord`]. When multiple bindings name the same origin, one
/// record is produced at the first binding's deterministic position and every binding is
/// preserved separately in the draft. Receiver methods are grouped by receiver origin and
/// attached to their owning struct or choice record. Direct trait bindings project their final
/// [`PublicTraitSemantics`]. A struct/choice category mismatch against the resolved root is
/// rejected rather than silently dropped. A missing root, trait root or receiver method is an
/// extra fact that must not leak: it is reported after the loop by the caller.
///
/// WHY: the resolved roots, callable seeds and trait projection already validate binding
/// joins, so the inputs are consistent. This projection reshapes them into the
/// declaration-centric model the draft owns, producing final records directly in export-binding
/// order rather than first assembling complete aggregate category vectors and rejoining them.
fn project_declaration_records<'a>(
    export_bindings: &'a [ExportBinding],
    root_table: &'a crate::compiler_frontend::ast::ResolvedPublicTypeRootTable,
    type_context: &DeclarationTypeProjectionContext<'_>,
    state: &mut DeclarationRecordProjectionState<'a, '_>,
) -> Result<Vec<PublicDeclarationRecord>, CompilerError> {
    let mut root_index = RootIndex::new(&root_table.roots, type_context.string_table)?;

    let mut declarations = Vec::new();
    let mut seen_origins: FxHashSet<OriginDeclarationId> = FxHashSet::default();

    for binding in export_bindings {
        // Provider-owned bindings stay in the draft for publication closure. They must not
        // consume local roots or trait records during direct projection.
        if binding.origin().module_origin() != binding.exporting_module() {
            continue;
        }

        // One declaration record per unique origin. A second binding for the same origin is
        // preserved in the export-bindings list but does not produce a second record.
        if !seen_origins.insert(binding.origin().clone()) {
            continue;
        }

        match binding.origin() {
            OriginDeclarationId::Function(function_origin) => {
                let root = root_index.take_for_binding(binding)?;
                let crate::compiler_frontend::ast::ResolvedPublicTypeRootKind::Function {
                    signature,
                    generic_parameter_list_id,
                } = &root.kind
                else {
                    return Err(super::type_projection::origin_category_mismatch_error(
                        "function", binding,
                    ));
                };
                let default_provenance = project_defaults_provenance(&signature.parameters);
                let semantics = project_free_function_semantics(
                    function_origin.clone(),
                    *generic_parameter_list_id,
                    signature,
                    type_context.type_environment,
                    type_context.type_projection_context,
                    &root_table.trait_source_facts,
                    type_context.public_source_trait_origins,
                    type_context.string_table,
                    type_context.folded_value_context,
                )?;
                declarations.push(PublicDeclarationRecord {
                    origin: binding.origin().clone(),
                    synthetic_interface_provenance: default_provenance,
                    semantics: PublicDeclarationSemantics::Function(semantics),
                });
            }
            OriginDeclarationId::Type(type_origin) => match type_origin.category() {
                OriginTypeCategory::Struct => {
                    let root = root_index.take_for_binding(binding)?;
                    let crate::compiler_frontend::ast::ResolvedPublicTypeRootKind::Struct {
                        type_id,
                        fields,
                    } = &root.kind
                    else {
                        return Err(super::type_projection::origin_category_mismatch_error(
                            "struct", binding,
                        ));
                    };
                    let field_default_provenance = project_defaults_provenance(fields);
                    let (generic_parameters, projected_fields) = project_struct_parts(
                        type_origin.clone(),
                        *type_id,
                        fields,
                        type_context.type_environment,
                        type_context.type_projection_context,
                        &root_table.trait_source_facts,
                        type_context.public_source_trait_origins,
                        type_context.string_table,
                        type_context.folded_value_context,
                    )?;
                    let (receiver_methods, receiver_default_provenance) =
                        receiver_methods_for_origin(
                            type_origin,
                            state.receiver_seeds_by_receiver,
                            state.receiver_method_signatures,
                        )?;
                    let synthetic_interface_provenance =
                        field_default_provenance.union(&receiver_default_provenance);
                    declarations.push(PublicDeclarationRecord {
                        origin: binding.origin().clone(),
                        synthetic_interface_provenance,
                        semantics: PublicDeclarationSemantics::Struct(PublicStructSemantics {
                            generic_parameters,
                            fields: projected_fields,
                            receiver_methods,
                        }),
                    });
                }
                OriginTypeCategory::Choice => {
                    let root = root_index.take_for_binding(binding)?;
                    let crate::compiler_frontend::ast::ResolvedPublicTypeRootKind::Choice {
                        type_id,
                    } = &root.kind
                    else {
                        return Err(super::type_projection::origin_category_mismatch_error(
                            "choice", binding,
                        ));
                    };
                    let (generic_parameters, variants) = project_choice_parts(
                        type_origin.clone(),
                        *type_id,
                        type_context.type_environment,
                        type_context.type_projection_context,
                        &root_table.trait_source_facts,
                        type_context.public_source_trait_origins,
                        type_context.string_table,
                    )?;
                    let (receiver_methods, synthetic_interface_provenance) =
                        receiver_methods_for_origin(
                            type_origin,
                            state.receiver_seeds_by_receiver,
                            state.receiver_method_signatures,
                        )?;
                    declarations.push(PublicDeclarationRecord {
                        origin: binding.origin().clone(),
                        synthetic_interface_provenance,
                        semantics: PublicDeclarationSemantics::Choice(PublicChoiceSemantics {
                            generic_parameters,
                            variants,
                            receiver_methods,
                        }),
                    });
                }
                OriginTypeCategory::TransparentAlias => {
                    let root = root_index.take_for_binding(binding)?;
                    let crate::compiler_frontend::ast::ResolvedPublicTypeRootKind::TransparentAlias {
                        target_type_id,
                    } = &root.kind
                    else {
                        return Err(super::type_projection::origin_category_mismatch_error(
                            "alias", binding,
                        ));
                    };
                    let target_type_identity = project_alias_target(
                        *target_type_id,
                        type_context.type_environment,
                        type_context.type_projection_context,
                    )?;
                    declarations.push(PublicDeclarationRecord {
                        origin: binding.origin().clone(),
                        synthetic_interface_provenance: SyntheticInterfaceProvenance::empty(),
                        semantics: PublicDeclarationSemantics::TransparentAlias(
                            PublicAliasSemantics {
                                target_type_identity,
                            },
                        ),
                    });
                }
            },
            OriginDeclarationId::Constant(_) => {
                let root = root_index.take_for_binding(binding)?;
                let crate::compiler_frontend::ast::ResolvedPublicTypeRootKind::Constant { type_id } =
                    &root.kind
                else {
                    return Err(super::type_projection::origin_category_mismatch_error(
                        "constant", binding,
                    ));
                };
                let type_identity = project_constant_type_identity(
                    *type_id,
                    type_context.type_environment,
                    type_context.type_projection_context,
                )?;
                let (folded_value, synthetic_interface_provenance) = fold_constant_value(
                    &root.path,
                    state.consumed_const_values,
                    state.folded_value_context,
                )?;
                declarations.push(PublicDeclarationRecord {
                    origin: binding.origin().clone(),
                    synthetic_interface_provenance,
                    semantics: PublicDeclarationSemantics::Constant(PublicConstantSemantics {
                        type_identity,
                        folded_value,
                    }),
                });
            }
            OriginDeclarationId::Trait(_) => {
                let semantics = state.trait_projection.project_binding(binding)?;
                declarations.push(PublicDeclarationRecord {
                    origin: binding.origin().clone(),
                    synthetic_interface_provenance: SyntheticInterfaceProvenance::empty(),
                    semantics: PublicDeclarationSemantics::Trait(semantics),
                });
            }
        }
    }

    // Every non-trait root must have joined a binding. A root left in the index is stale
    // or extra: it has no matching export binding, so it would otherwise leak into no record.
    // Deterministic name reporting avoids unordered hash-map iteration.
    let leftover_roots = root_index.remaining_names();
    if !leftover_roots.is_empty() {
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft join: the public type root(s) {} have no matching export binding; every non-trait root must join exactly one binding",
            leftover_roots.join(", ")
        )));
    }

    // Every trait root must have joined a binding.
    let leftover_traits = state.trait_projection.remaining_names();
    if !leftover_traits.is_empty() {
        let mut names: Vec<String> = leftover_traits.into_iter().map(String::from).collect();
        names.sort();
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft join: the trait root(s) {} have no matching trait export binding; every direct trait root must join exactly one binding",
            names.join(", ")
        )));
    }

    // Every receiver method must have attached to its owning nominal record.
    let leftover_receiver_methods: usize = state
        .receiver_seeds_by_receiver
        .values()
        .map(Vec::len)
        .sum();
    if leftover_receiver_methods > 0 {
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft join: {} receiver method(s) have no matching struct or choice export binding",
            leftover_receiver_methods
        )));
    }

    Ok(declarations)
}

/// Take the receiver-method seeds for one nominal origin and project each into a
/// [`super::model::PublicReceiverMethodSemantics`] attached to that record.
///
/// The returned provenance is the aggregate of every receiver parameter default. It is kept on
/// the declaration record rather than on the folded-value payload or method semantic leaf.
fn receiver_methods_for_origin(
    type_origin: &OriginTypeId,
    receiver_seeds_by_receiver: &mut FxHashMap<OriginTypeId, Vec<&CallableSeed>>,
    receiver_method_signatures: &FxHashMap<
        usize,
        super::type_projection::ProjectedReceiverMethodSignature,
    >,
) -> Result<
    (
        Vec<super::model::PublicReceiverMethodSemantics>,
        SyntheticInterfaceProvenance,
    ),
    CompilerError,
> {
    let seeds = receiver_seeds_by_receiver
        .remove(type_origin)
        .unwrap_or_default();
    let mut provenance = SyntheticInterfaceProvenance::empty();
    let mut methods = Vec::with_capacity(seeds.len());

    for seed in seeds {
        let method_index = match &seed.kind {
            CallableSeedKind::ReceiverMethod { method_index, .. } => *method_index,
            CallableSeedKind::FreeFunction => {
                return Err(CompilerError::compiler_error(format!(
                    "public-interface draft join: free-function callable seed {:?} was indexed \
                     under receiver origin {:?}",
                    seed.origin, type_origin
                )));
            }
        };
        let signature = receiver_method_signatures
            .get(&method_index)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "public-interface draft join: receiver-method seed with method_index {} has no \
                 projected signature",
                    method_index
                ))
            })?;
        provenance.merge(&signature.default_provenance);
        methods.push(receiver_method_semantics_from_seed(
            seed,
            receiver_method_signatures,
        )?);
    }

    Ok((methods, provenance))
}

/// Fold one public constant root's value by exact defining path and collect its value provenance.
///
/// WHAT: looks up the root's defining `InternedPath` in the module-local store, marks the
/// resulting value ID consumed, and converts it through the shared store visitor. The conversion
/// returns the canonical union of the root metadata and every nested folded value node, while the
/// public folded payload remains unchanged.
fn fold_constant_value(
    defining_path: &InternedPath,
    consumed_const_values: &mut FxHashSet<ConstValueId>,
    context: &FoldedValueJoinContext,
) -> Result<(PublicFoldedValue, SyntheticInterfaceProvenance), CompilerError> {
    let Some(value_id) = context.const_values.value_for_path(defining_path) else {
        let defining_path =
            defining_path.to_path_buf(context.folded_value_projection_context.string_table);
        let mut available_paths = context
            .const_values
            .module_constant_paths()
            .map(|path| path.to_path_buf(context.folded_value_projection_context.string_table))
            .collect::<Vec<_>>();
        available_paths.sort();
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft join: constant export binding {defining_path:?} has no \
             matching finalized module constant declaration; available defining paths are \
             {available_paths:?} and the folded value cannot be projected without the \
             donor-local AST expression"
        )));
    };

    if !consumed_const_values.insert(value_id) {
        return Err(CompilerError::compiler_error(format!(
            "public-interface draft join: constant export binding {defining_path:?} consumed its finalized store value more than once"
        )));
    }

    convert_const_value_to_folded_value_with_provenance(
        context.const_values,
        value_id,
        context.folded_value_projection_context,
    )
}
