//! Transient AST-owned resolved public type-root table.
//!
//! WHAT: builds one deterministic vector of directly-defined active-root public type-root
//! records plus the receiver methods attached to directly-defined public nominal receivers,
//! and the transient trait source facts for exported generic bounds, from the same
//! already-resolved facts consumed by public-surface validation.
//! WHY: canonical type projection needs resolved roots available immediately before HIR
//! lowering without reconstructing public semantics from HIR or source. This is transient
//! donor-local AST data consumed before HIR; it never enters `ModuleSemanticResult`,
//! `Module`, or a cross-module interface.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
use crate::compiler_frontend::ast::module_ast::environment::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::module_ast::scope_context::ReceiverMethodCatalog;
use crate::compiler_frontend::ast::receiver_methods::ReceiverMethodEntry;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::type_resolution::ResolvedFunctionSignature;
use crate::compiler_frontend::ast::type_resolution::ResolvedTypeAnnotation;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterListId, TypeId};
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::traits::environment::{CoreTraitKind, TraitEnvironment};
use crate::compiler_frontend::traits::ids::TraitId;

use rustc_hash::{FxHashMap, FxHashSet};

/// The resolved category and identity of one directly-defined active-root public type root.
///
/// WHAT: one root admitted by the active-root public declaration-kind gate, carrying the
/// resolved `TypeId` facts already produced by AST environment construction.
/// WHY: keeps the table value-shaped and deterministic so later projection reads named
/// roots and their resolved identities without re-scanning headers or HIR.
/// Consumed by the defined public type-surface projection immediately before HIR lowering.
#[derive(Clone, Debug)]
pub(crate) enum ResolvedPublicTypeRootKind {
    /// Public free function with resolved parameter and return `TypeId`s.
    ///
    /// WHAT: carries the full resolved signature so parameter and return `TypeId`s, including
    /// generic-parameter `TypeId`s, remain intact for later projection. Receiver methods are
    /// not free roots; they are selected in a separate pass into `receiver_methods`.
    Function {
        signature: FunctionSignature,
        generic_parameter_list_id: Option<GenericParameterListId>,
    },

    /// Public nominal struct with its canonical `TypeId` and resolved field declarations.
    ///
    /// WHAT: carries the resolved field `Declaration` values so field defaults (compile-time
    /// expressions in each `Declaration.value`) survive alongside the `TypeId` for later
    /// default projection. Choice payload fields remain default-free and do not need a
    /// retained declaration copy.
    Struct {
        type_id: TypeId,
        fields: Vec<Declaration>,
    },

    /// Public nominal choice with its canonical `TypeId`.
    Choice { type_id: TypeId },

    /// Public transparent type alias with its resolved target `TypeId`.
    ///
    /// WHAT: the target `TypeId` is materialized once by the public-surface validation owner
    /// and retained on the resolved annotation. This table consumes the retained fact; it
    /// does not resolve the alias target a second time. The alias does not introduce its own
    /// type identity; only the resolved target `TypeId` is carried.
    TransparentAlias { target_type_id: TypeId },

    /// Public compile-time constant with its resolved `TypeId`.
    Constant { type_id: TypeId },
}

/// One named directly-defined active-root public type root.
/// Consumed by the defined public type-surface projection immediately before HIR lowering.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedPublicTypeRoot {
    pub(crate) path: InternedPath,
    pub(crate) kind: ResolvedPublicTypeRootKind,
}

/// Resolved source fact for one local trait, retained transiently for bound projection.
///
/// WHAT: carries either the trait's source canonical declaration path or its compiler-owned
/// core classifier. It never carries the `TraitEnvironment`, requirement bodies, evidence
/// or a `TraitId` registry handle.
/// WHY: generic-bound projection needs to resolve each local bound `TraitId` to a stable
/// canonical trait identity. Retaining only these two facts keeps the transient table
/// minimal and consumed with the rest of [`ResolvedPublicTypeRootTable`] before HIR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedTraitSourceFact {
    /// A compiler-owned core trait identified by its canonical kind.
    Core(CoreTraitKind),
    /// A source-defined trait identified by its canonical declaration path.
    Source(InternedPath),
}

/// Transient AST-owned table of resolved public type roots for the module.
///
/// WHAT: `roots` holds the named declaration roots in deterministic sorted-header order;
/// `receiver_methods` holds the receiver methods attached to directly-defined public
/// nominal receivers, selected in a separate pass so private method headers outside
/// `export:` are admitted by receiver ownership rather than their own export binding.
/// `trait_source_facts` retains the resolved trait identity source facts needed by
/// generic bounds on the exported free-function and struct/choice roots and by public
/// trait incompatibility relations, so bound and incompatibility projection can resolve
/// each local `TraitId` without the `TraitEnvironment`.
/// WHY: the semantic orchestration consumes this immediately before HIR lowering. This
/// table is type-only: direct trait-root facts and their requirement construction live in
/// [`super::resolved_public_trait_roots`]. Donor-local `TypeId`s and `TraitId`s must not
/// cross the module result boundary.
/// Consumed by the defined public type-surface projection immediately before HIR lowering;
/// retained as transient donor-local AST data that never crosses the module result boundary.
#[derive(Clone, Default)]
pub(crate) struct ResolvedPublicTypeRootTable {
    pub(crate) roots: Vec<ResolvedPublicTypeRoot>,
    pub(crate) receiver_methods: Vec<ReceiverMethodEntry>,
    pub(crate) trait_source_facts: FxHashMap<TraitId, ResolvedTraitSourceFact>,
}

/// Inputs required to build the resolved public type-root table.
///
/// WHAT: groups the resolved signature, nominal identity, alias, constant and receiver
/// side tables used while building the root table, plus the set of re-exported declaration
/// paths targeted by public export entries from the root's `export:` block.
/// WHY: building the table sits at the join of completed AST environment facts; keeping the
/// inputs named makes that boundary easier to audit than a long positional list.
pub(crate) struct BuildResolvedPublicTypeRootsInput<'a> {
    pub sorted_headers: &'a [Header],
    pub resolved_struct_fields_by_path: &'a FxHashMap<InternedPath, Vec<Declaration>>,
    pub resolved_function_signatures_by_path:
        &'a FxHashMap<InternedPath, ResolvedFunctionSignature>,
    pub nominal_type_ids_by_path: &'a FxHashMap<InternedPath, TypeId>,
    pub resolved_type_aliases_by_path: &'a FxHashMap<InternedPath, ResolvedTypeAnnotation>,
    pub declaration_table: &'a TopLevelDeclarationTable,
    pub generic_function_templates_by_path: &'a FxHashMap<InternedPath, GenericFunctionTemplate>,
    pub receiver_methods: &'a ReceiverMethodCatalog,
    pub trait_environment: &'a TraitEnvironment,
    pub type_environment: &'a TypeEnvironment,
    pub string_table: &'a StringTable,
    /// Source declaration paths re-exported through the root's `export:` block from private files
    /// in the same module. These are not in the active root file but are targeted by public export
    /// entries and need their resolved roots included in the table.
    pub reexport_target_paths: &'a FxHashSet<InternedPath>,
}

/// Build the resolved public type-root table from completed AST environment facts.
///
/// WHAT: two passes over the same sorted headers collect every directly-defined active-root
/// public type-root and the receiver methods attached to directly-defined public nominal
/// receivers. Pass 1 collects declaration roots (functions, structs, choices, transparent
/// aliases and constants) and the complete public nominal path set. Pass 2 selects receiver
/// methods by nominal receiver ownership, independent of the method's own export binding.
/// Trait-source facts for exported generic bounds are resolved from the `TraitEnvironment`.
/// A missing, wrong-category or unresolvable fact is an internal `CompilerError` rather than
/// silent omission.
/// WHY: two passes over the same sorted headers keep a single deterministic owner and let
/// the method pass see the complete public nominal set even when a method precedes its
/// receiver in supplied order. One pass would miss methods that appear before their nominal
/// receiver and would wrongly gate private methods on their own export binding.
pub(crate) fn build_resolved_public_type_roots(
    input: BuildResolvedPublicTypeRootsInput<'_>,
) -> Result<ResolvedPublicTypeRootTable, CompilerError> {
    let BuildResolvedPublicTypeRootsInput {
        sorted_headers,
        resolved_struct_fields_by_path,
        resolved_function_signatures_by_path,
        nominal_type_ids_by_path,
        resolved_type_aliases_by_path,
        declaration_table,
        generic_function_templates_by_path,
        receiver_methods,
        trait_environment,
        type_environment,
        string_table,
        reexport_target_paths,
    } = input;

    let mut roots = Vec::new();
    // Directly-defined active-root public nominal paths, collected in full before the method
    // pass so method selection is independent of where a method appears relative to its
    // receiver in sorted-header order.
    let mut public_nominal_paths: FxHashSet<InternedPath> = FxHashSet::default();

    // Pass 1: collect declaration roots and the complete public nominal path set.
    for header in sorted_headers {
        if !is_active_root_public_declaration(header) {
            continue;
        }

        let path = &header.tokens.src_path;

        match &header.kind {
            HeaderKind::Function { .. } => {
                let Some(resolved) = resolved_function_signatures_by_path.get(path) else {
                    return Err(missing_resolved_function_signature(path, string_table));
                };

                // Receiver methods are not free export bindings. They are selected in the
                // separate method pass by receiver ownership, not here.
                if resolved.receiver.is_some() {
                    continue;
                }

                let generic_parameter_list_id = generic_function_templates_by_path
                    .get(path)
                    .map(|template| template.generic_parameter_list_id);

                roots.push(ResolvedPublicTypeRoot {
                    path: path.to_owned(),
                    kind: ResolvedPublicTypeRootKind::Function {
                        signature: resolved.signature.clone(),
                        generic_parameter_list_id,
                    },
                });
            }

            HeaderKind::Struct { .. } => {
                let Some(&type_id) = nominal_type_ids_by_path.get(path) else {
                    return Err(missing_nominal_type_id(path, string_table));
                };
                public_nominal_paths.insert(path.to_owned());
                let fields = resolved_struct_fields_by_path
                    .get(path)
                    .cloned()
                    .ok_or_else(|| missing_resolved_struct_fields(path, string_table))?;
                roots.push(ResolvedPublicTypeRoot {
                    path: path.to_owned(),
                    kind: ResolvedPublicTypeRootKind::Struct { type_id, fields },
                });
            }

            HeaderKind::Choice { .. } => {
                let Some(&type_id) = nominal_type_ids_by_path.get(path) else {
                    return Err(missing_nominal_type_id(path, string_table));
                };
                public_nominal_paths.insert(path.to_owned());
                roots.push(ResolvedPublicTypeRoot {
                    path: path.to_owned(),
                    kind: ResolvedPublicTypeRootKind::Choice { type_id },
                });
            }

            HeaderKind::TypeAlias { .. } => {
                let Some(annotation) = resolved_type_aliases_by_path.get(path) else {
                    return Err(missing_resolved_alias(path, string_table));
                };
                // The public-surface validation owner materializes and retains the alias
                // target `TypeId`. This table consumes the retained fact; a missing `TypeId`
                // here is an internal invariant failure, not a reason to resolve again.
                let Some(target_type_id) = annotation.type_id else {
                    return Err(missing_resolved_alias_type_id(path, string_table));
                };
                roots.push(ResolvedPublicTypeRoot {
                    path: path.to_owned(),
                    kind: ResolvedPublicTypeRootKind::TransparentAlias { target_type_id },
                });
            }

            HeaderKind::Constant { .. } => {
                let Some(declaration) = declaration_table.get_by_path(path) else {
                    return Err(missing_resolved_constant(path, string_table));
                };
                roots.push(ResolvedPublicTypeRoot {
                    path: path.to_owned(),
                    kind: ResolvedPublicTypeRootKind::Constant {
                        type_id: declaration.value.type_id,
                    },
                });
            }

            // Trait declarations are not type roots. Direct trait-root facts are built by
            // `build_resolved_public_trait_roots` in the dedicated trait-roots owner.
            HeaderKind::Trait { .. }
            | HeaderKind::ConstTemplate { .. }
            | HeaderKind::StartFunction
            | HeaderKind::TraitConformance { .. }
            | HeaderKind::TraitIncompatibility { .. } => {}
        }
    }

    // Pass 1b: collect re-exported declaration roots from private files targeted by public export
    // entries. These declarations are not in the active root file, so Pass 1 excluded them. The
    // header-built public export maps already resolved the re-export target paths, so this pass
    // joins them into the root table using the same resolved AST environment facts.
    for header in sorted_headers {
        if !reexport_target_paths.contains(&header.tokens.src_path) {
            continue;
        }

        // Skip declarations already collected by Pass 1 (directly-defined active-root public
        // declarations). A re-export that targets a directly-defined declaration is already in the
        // table.
        if is_active_root_public_declaration(header) {
            continue;
        }

        let path = &header.tokens.src_path;

        match &header.kind {
            HeaderKind::Function { .. } => {
                let Some(resolved) = resolved_function_signatures_by_path.get(path) else {
                    return Err(missing_resolved_function_signature(path, string_table));
                };

                // Receiver methods are not free export bindings. They travel with their receiver
                // surface and are selected by receiver ownership in Pass 2.
                if resolved.receiver.is_some() {
                    continue;
                }

                let generic_parameter_list_id = generic_function_templates_by_path
                    .get(path)
                    .map(|template| template.generic_parameter_list_id);

                roots.push(ResolvedPublicTypeRoot {
                    path: path.to_owned(),
                    kind: ResolvedPublicTypeRootKind::Function {
                        signature: resolved.signature.clone(),
                        generic_parameter_list_id,
                    },
                });
            }

            HeaderKind::Struct { .. } => {
                let Some(&type_id) = nominal_type_ids_by_path.get(path) else {
                    return Err(missing_nominal_type_id(path, string_table));
                };
                public_nominal_paths.insert(path.to_owned());
                let fields = resolved_struct_fields_by_path
                    .get(path)
                    .cloned()
                    .ok_or_else(|| missing_resolved_struct_fields(path, string_table))?;
                roots.push(ResolvedPublicTypeRoot {
                    path: path.to_owned(),
                    kind: ResolvedPublicTypeRootKind::Struct { type_id, fields },
                });
            }

            HeaderKind::Choice { .. } => {
                let Some(&type_id) = nominal_type_ids_by_path.get(path) else {
                    return Err(missing_nominal_type_id(path, string_table));
                };
                public_nominal_paths.insert(path.to_owned());
                roots.push(ResolvedPublicTypeRoot {
                    path: path.to_owned(),
                    kind: ResolvedPublicTypeRootKind::Choice { type_id },
                });
            }

            HeaderKind::TypeAlias { .. } => {
                let Some(annotation) = resolved_type_aliases_by_path.get(path) else {
                    return Err(missing_resolved_alias(path, string_table));
                };
                let Some(target_type_id) = annotation.type_id else {
                    return Err(missing_resolved_alias_type_id(path, string_table));
                };
                roots.push(ResolvedPublicTypeRoot {
                    path: path.to_owned(),
                    kind: ResolvedPublicTypeRootKind::TransparentAlias { target_type_id },
                });
            }

            HeaderKind::Constant { .. } => {
                let Some(declaration) = declaration_table.get_by_path(path) else {
                    return Err(missing_resolved_constant(path, string_table));
                };
                roots.push(ResolvedPublicTypeRoot {
                    path: path.to_owned(),
                    kind: ResolvedPublicTypeRootKind::Constant {
                        type_id: declaration.value.type_id,
                    },
                });
            }

            // Trait declarations, non-declaration headers and trait relations are not type roots.
            HeaderKind::Trait { .. }
            | HeaderKind::ConstTemplate { .. }
            | HeaderKind::StartFunction
            | HeaderKind::TraitConformance { .. }
            | HeaderKind::TraitIncompatibility { .. } => {}
        }
    }

    // Pass 2: select receiver methods attached to public nominal receivers owned by this module.
    // Real methods on a public receiver are normally private headers outside `export:`, so this
    // pass ignores the method header's export mode and file role and selects by the receiver path
    // admitted in Pass 1/1b. Imported-provider methods cannot enter because their receiver paths
    // are not in this module's public nominal set.
    let mut receiver_method_entries = Vec::new();
    for header in sorted_headers {
        if !matches!(&header.kind, HeaderKind::Function { .. }) {
            continue;
        }

        let path = &header.tokens.src_path;
        // AST environment construction resolves every function signature before this table, so
        // a function whose receiver is selected below must have a resolved signature.
        let Some(resolved) = resolved_function_signatures_by_path.get(path) else {
            return Err(missing_resolved_function_signature(path, string_table));
        };
        let Some(receiver) = resolved.receiver.as_ref() else {
            continue;
        };
        let Some(receiver_path) = nominal_receiver_path(receiver) else {
            continue;
        };
        if !public_nominal_paths.contains(receiver_path) {
            continue;
        }

        let Some(entry) = receiver_methods.by_function_path.get(path) else {
            return Err(missing_receiver_method_entry(path, string_table));
        };
        receiver_method_entries.push(entry.clone());
    }

    // Retain the resolved trait identity source facts needed by exported generic bounds and
    // public trait incompatibilities. The `TraitEnvironment` is dropped after this table is
    // built; the projection consumes these facts to resolve each local `TraitId` to a stable
    // canonical trait identity without reconstructing the trait source.
    let trait_source_facts = build_trait_source_facts(&roots, type_environment, trait_environment)?;

    Ok(ResolvedPublicTypeRootTable {
        roots,
        receiver_methods: receiver_method_entries,
        trait_source_facts,
    })
}

/// Build the transient resolved trait identity source facts for generic bounds on exported
/// free-function and struct/choice roots.
///
/// WHAT: iterates the roots to collect every local bound `TraitId`, then resolves each
/// through the `TraitEnvironment`: a core trait maps to `Core(CoreTraitKind)` and a source
/// trait maps to `Source(canonical_path)`. A `TraitId` that is neither core nor source is
/// a `CompilerError`, never a silent omission.
/// WHY: the transient facts let bound projection resolve each local `TraitId` after the
/// `TraitEnvironment` is dropped. The facts needed by exported generic bounds and by public
/// trait incompatibility relations are retained, keeping the table minimal while giving the
/// public-interface draft one source/core mapping owner for both paths.
fn build_trait_source_facts(
    roots: &[ResolvedPublicTypeRoot],
    type_environment: &TypeEnvironment,
    trait_environment: &TraitEnvironment,
) -> Result<FxHashMap<TraitId, ResolvedTraitSourceFact>, CompilerError> {
    let mut trait_ids = collect_bound_trait_ids_from_roots(roots, type_environment)?;

    // Also retain source/core classification for every trait id appearing in a public
    // incompatibility relation, so the public-interface draft can canonicalize direct
    // public trait incompatibilities after the `TraitEnvironment` is dropped. The ids are
    // already resolved by `resolve_trait_incompatibilities`, so each is a registered source
    // or core trait in this compilation boundary.
    for trait_id in trait_environment.public_incompatible_trait_ids() {
        trait_ids.insert(trait_id);
    }

    let mut facts = FxHashMap::default();
    for trait_id in trait_ids {
        if let Some(kind) = trait_environment.core_trait_kind(trait_id) {
            if facts
                .insert(trait_id, ResolvedTraitSourceFact::Core(kind))
                .is_some()
            {
                return Err(CompilerError::compiler_error(format!(
                    "resolved public type-root construction: TraitId({}) has a duplicate conflicting core trait mapping",
                    trait_id.0
                )));
            }
            continue;
        }

        let Some(definition) = trait_environment.get(trait_id) else {
            return Err(CompilerError::compiler_error(format!(
                "resolved public type-root construction: retained TraitId({}) has no resolved trait definition and is not a core trait; a missing definition is an internal invariant violation",
                trait_id.0
            )));
        };

        if facts
            .insert(
                trait_id,
                ResolvedTraitSourceFact::Source(definition.canonical_path.clone()),
            )
            .is_some()
        {
            return Err(CompilerError::compiler_error(format!(
                "resolved public type-root construction: TraitId({}) has a duplicate conflicting source trait mapping (path: {:?})",
                trait_id.0, definition.canonical_path
            )));
        }
    }

    Ok(facts)
}

/// Collect every local bound `TraitId` from the exported generic parameter lists of the
/// resolved roots.
///
/// WHAT: gathers `TraitId`s from the `TypeEnvironment`'s `trait_bounds` for each root's
/// canonical generic parameter list. Function roots carry their list ID directly;
/// struct/choice roots resolve their list ID through the `TypeEnvironment` definition.
/// WHY: only the bounds on exported free-function and struct/choice roots need stable
/// trait identity projection, so the transient facts are scoped to exactly those bounds.
fn collect_bound_trait_ids_from_roots(
    roots: &[ResolvedPublicTypeRoot],
    type_environment: &TypeEnvironment,
) -> Result<FxHashSet<TraitId>, CompilerError> {
    let mut bound_trait_ids = FxHashSet::default();

    for root in roots {
        match &root.kind {
            ResolvedPublicTypeRootKind::Function {
                generic_parameter_list_id: Some(list_id),
                ..
            } => {
                collect_bound_trait_ids_from_list(
                    type_environment,
                    *list_id,
                    &mut bound_trait_ids,
                )?;
            }
            ResolvedPublicTypeRootKind::Struct { type_id, .. } => {
                if let Some(list_id) =
                    nominal_generic_parameter_list_id(type_environment, *type_id)?
                {
                    collect_bound_trait_ids_from_list(
                        type_environment,
                        list_id,
                        &mut bound_trait_ids,
                    )?;
                }
            }
            ResolvedPublicTypeRootKind::Choice { type_id } => {
                if let Some(list_id) =
                    nominal_generic_parameter_list_id(type_environment, *type_id)?
                {
                    collect_bound_trait_ids_from_list(
                        type_environment,
                        list_id,
                        &mut bound_trait_ids,
                    )?;
                }
            }
            _ => {}
        }
    }

    Ok(bound_trait_ids)
}

/// Collect bound `TraitId`s from one generic parameter list.
fn collect_bound_trait_ids_from_list(
    type_environment: &TypeEnvironment,
    list_id: GenericParameterListId,
    bound_trait_ids: &mut FxHashSet<TraitId>,
) -> Result<(), CompilerError> {
    let Some(list) = type_environment.generic_parameters(list_id) else {
        return Err(CompilerError::compiler_error(format!(
            "resolved public type-root construction: GenericParameterListId({}) is missing from the TypeEnvironment while collecting bound trait IDs",
            list_id.0
        )));
    };

    for parameter in &list.parameters {
        for trait_id in &parameter.trait_bounds {
            bound_trait_ids.insert(*trait_id);
        }
    }

    Ok(())
}

/// Resolve the generic parameter list ID for a struct or choice root's nominal type.
///
/// Returns `Ok(None)` when the struct or choice definition has no generic parameter list
/// (a non-generic nominal). A `TypeId` that is not registered in the `TypeEnvironment`, or
/// that resolves to a definition that is neither a struct nor a choice, is a `CompilerError`:
/// the root was admitted as a nominal declaration, so a missing or wrong-category definition
/// is an internal invariant violation, not a silent skip.
fn nominal_generic_parameter_list_id(
    type_environment: &TypeEnvironment,
    type_id: TypeId,
) -> Result<Option<GenericParameterListId>, CompilerError> {
    let Some(definition) = type_environment.get(type_id) else {
        return Err(CompilerError::compiler_error(format!(
            "resolved public type-root construction: a nominal root TypeId({}) is not registered in the TypeEnvironment while collecting bound trait IDs",
            type_id.0
        )));
    };

    Ok(match definition {
        TypeDefinition::Struct(def) => def.generic_parameters,
        TypeDefinition::Choice(def) => def.generic_parameters,
        _ => {
            return Err(CompilerError::compiler_error(format!(
                "resolved public type-root construction: a nominal root TypeId({}) resolved to a non-nominal definition while collecting bound trait IDs",
                type_id.0
            )));
        }
    })
}

/// Whether a header is a directly-defined active-root public authored declaration.
///
/// WHAT: only declarations in the active module root's public export surface, admitted by the
/// shared [`HeaderKind::is_authored_public_export_declaration`] gate, become roots. Imported
/// module roots, private declarations, builtin declarations and source-package-only facts
/// are excluded.
/// WHY: the retained table covers only roots directly defined by the module being compiled.
fn is_active_root_public_declaration(header: &Header) -> bool {
    header.file_role.is_active_module_root()
        && header.export_mode.is_public()
        && header.kind.is_authored_public_export_declaration()
}

/// The nominal receiver path for a struct/choice receiver, if any.
///
/// WHAT: external and builtin-scalar receivers are not directly-defined nominal roots, so
/// their methods are not retained by this table.
fn nominal_receiver_path(receiver: &ReceiverKey) -> Option<&InternedPath> {
    match receiver {
        ReceiverKey::Struct(path) | ReceiverKey::Choice(path) => Some(path),
        ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_) => None,
    }
}

fn missing_resolved_function_signature(
    path: &InternedPath,
    string_table: &StringTable,
) -> CompilerError {
    CompilerError::compiler_error(format!(
        "Active-root function '{}' had no resolved signature during root-table construction.",
        path.to_string(string_table)
    ))
}

fn missing_nominal_type_id(path: &InternedPath, string_table: &StringTable) -> CompilerError {
    CompilerError::compiler_error(format!(
        "Public active-root nominal declaration '{}' had no canonical TypeId during root-table construction.",
        path.to_string(string_table)
    ))
}

fn missing_resolved_struct_fields(
    path: &InternedPath,
    string_table: &StringTable,
) -> CompilerError {
    CompilerError::compiler_error(format!(
        "Public active-root struct '{}' had no resolved field declarations during root-table construction; every public struct must have an entry, including an empty vector for an empty struct.",
        path.to_string(string_table)
    ))
}

fn missing_resolved_alias(path: &InternedPath, string_table: &StringTable) -> CompilerError {
    CompilerError::compiler_error(format!(
        "Public active-root transparent alias '{}' had no resolved annotation during root-table construction.",
        path.to_string(string_table)
    ))
}

fn missing_resolved_alias_type_id(
    path: &InternedPath,
    string_table: &StringTable,
) -> CompilerError {
    CompilerError::compiler_error(format!(
        "Public active-root transparent alias '{}' had no retained target TypeId during root-table construction; the public-surface owner should have materialized and retained it.",
        path.to_string(string_table)
    ))
}

fn missing_resolved_constant(path: &InternedPath, string_table: &StringTable) -> CompilerError {
    CompilerError::compiler_error(format!(
        "Public active-root constant '{}' had no resolved declaration during root-table construction.",
        path.to_string(string_table)
    ))
}

fn missing_receiver_method_entry(path: &InternedPath, string_table: &StringTable) -> CompilerError {
    CompilerError::compiler_error(format!(
        "Public receiver method '{}' was missing from the receiver catalog during root-table construction.",
        path.to_string(string_table)
    ))
}
