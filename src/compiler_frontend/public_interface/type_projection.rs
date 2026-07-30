//! Canonical type-surface projection for the directly-defined public surface.
//!
//! WHAT: owns the transient nominal and generic-parameter origin resolvers, the resolved
//! root-to-binding join index, the shared trait-source-fact projection, and the per-root
//! canonical type projection helpers that produce final `Public*` semantic parts directly.
//! Free-function signatures, nominal field/variant types, transparent alias targets and
//! constant types are projected through the existing `canonical_type_identity` projection
//! owner into owned stable values.
//!
//! WHY: the compiler design overview requires AST to own canonical export projection and to
//! emit stable semantic identities before HIR. Donor-local `TypeId` values must not cross the
//! module result boundary. This module is the single production consumer of the transient root
//! table: it takes the table and the `TypeEnvironment` while both are still available, projects
//! the canonical type parts, and leaves no donor-local `TypeId` in the output. The declaration
//! join in `direct_projection` consumes these parts to build one declaration record per origin
//! without first assembling complete aggregate type-surface vectors.
//!
//! ## Transient resolvers
//!
//! Two transient resolvers implement the `canonical_type_identity` traits for the duration of one
//! projection:
//!
//! - [`TransientNominalOriginResolver`]: maps `NominalTypeId` to `OriginTypeId` through
//!   `TypeEnvironment` nominal paths plus the transient expanded public source-nominal origin
//!   index of source nominals targeted by retained public exports. Direct public declarations,
//!   imported project-graph nominals and private normal-file nominals exposed through a public
//!   alias resolve to their owning module origin; unexported, unregistered and source-package
//!   nominals without a project-module owner fail through `CompilerError`.
//!
//! - [`TransientGenericParameterOriginResolver`]: maps `GenericParameterId` to
//!   `ExportedGenericParameterIdentity` from the roots' own generic parameter lists and the
//!   stable declaration origins. Free functions and struct/choice declarations are the only
//!   generic declaration owners; receiver methods reuse their receiver nominal's parameters and
//!   must not become owners.

use super::model::{
    PublicChoiceVariantSurface, PublicFieldTypeSlot, PublicFunctionCategory,
    PublicFunctionSemantics, PublicGenericParameterSurface, PublicGenericTemplateDescriptor,
    PublicParameterTypeSlot, PublicReturnTypeSlot,
};
use crate::compiler_frontend::ast::ReceiverMethodEntry;
use crate::compiler_frontend::ast::ResolvedTraitSourceFact;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::generic_functions::GenericFunctionTemplate;
use crate::compiler_frontend::ast::statements::functions::{
    FunctionSignature, ReturnChannel, ReturnSlot,
};
use crate::compiler_frontend::ast::{
    ResolvedPublicTypeRoot, ResolvedPublicTypeRootKind, ResolvedPublicTypeRootTable,
};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalCoreTraitIdentity, CanonicalTraitIdentity, CanonicalTypeIdentity,
    CanonicalTypeProjectionContext, ExportedGenericParameterIdentity, GenericDeclarationOrigin,
    GenericParameterOriginResolver, NominalOriginResolver, project_type_id_to_canonical_identity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantPayloadDefinition, StructTypeDefinition, TypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{
    GenericParameterId, GenericParameterListId, NominalTypeId, TypeId,
};
use crate::compiler_frontend::folded_value::{
    PublicFoldedValue, convert_expression_to_folded_value,
};
use crate::compiler_frontend::public_call_summary::PublicCallParameterAccess;
use crate::compiler_frontend::semantic_identity::{
    ExportBinding, OriginDeclarationId, OriginFunctionId, OriginTraitId, OriginTypeId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::traits::environment::CoreTraitKind;
use crate::compiler_frontend::traits::ids::TraitId;
use rustc_hash::{FxHashMap, FxHashSet};

/// The projected canonical signature for one receiver-method callable seed.
///
/// WHAT: the canonical parameter, return and error-return types for one receiver method,
/// projected once during receiver projection and keyed by the seed's `method_index`.
/// Free-function seeds do not carry this; their signature is projected by the free-function
/// projection owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProjectedReceiverMethodSignature {
    pub(super) parameters: Vec<PublicParameterTypeSlot>,
    pub(super) returns: Vec<PublicReturnTypeSlot>,
    pub(super) error_return: Option<CanonicalTypeIdentity>,
}

// ---------------------------------------------------------------------------
//  Transient nominal origin resolver
// ---------------------------------------------------------------------------

/// Transient resolver that maps module-local `NominalTypeId` to stable `OriginTypeId`.
///
/// WHAT: looks up the nominal's `InternedPath` through `TypeEnvironment` then resolves it
/// through the transient expanded public source-nominal origin index. Direct public declarations,
/// imported project-graph nominals and private normal-file nominals exposed through a public alias
/// resolve to their owning module origin; unexported, unregistered and source-package nominals
/// without a project-module owner fail through `CompilerError`.
pub(super) struct TransientNominalOriginResolver<'a> {
    type_environment: &'a TypeEnvironment,
    public_source_nominal_type_origins: &'a FxHashMap<InternedPath, OriginTypeId>,
}

impl<'a> TransientNominalOriginResolver<'a> {
    pub(super) fn new(
        type_environment: &'a TypeEnvironment,
        public_source_nominal_type_origins: &'a FxHashMap<InternedPath, OriginTypeId>,
    ) -> Self {
        Self {
            type_environment,
            public_source_nominal_type_origins,
        }
    }
}

impl NominalOriginResolver for TransientNominalOriginResolver<'_> {
    fn resolve_nominal_origin(
        &self,
        nominal_id: NominalTypeId,
    ) -> Result<OriginTypeId, CompilerError> {
        let path = self
            .type_environment
            .nominal_path_by_id(nominal_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "defined public type-surface projection: NominalTypeId({}) has no registered \
                 path in the TypeEnvironment",
                    nominal_id.0
                ))
            })?;

        self.public_source_nominal_type_origins
            .get(path)
            .cloned()
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "defined public type-surface projection: NominalTypeId({}) resolves to a \
                     path that is not targeted by a retained public source export with a stable \
                     owning source-module origin",
                    nominal_id.0
                ))
            })
    }
}

// ---------------------------------------------------------------------------
//  Transient generic-parameter origin resolver
// ---------------------------------------------------------------------------

/// Transient resolver that maps module-local `GenericParameterId` to stable
/// `ExportedGenericParameterIdentity`.
///
/// WHAT: built once from the resolved public type roots and the stable declaration origins.
/// Free functions and struct/choice declarations are the only generic declaration owners.
/// Receiver methods reuse their receiver nominal's parameters and never add entries.
/// A `GenericParameterId` with no registered owner fails through `CompilerError`.
pub(super) struct TransientGenericParameterOriginResolver {
    origins: FxHashMap<GenericParameterId, ExportedGenericParameterIdentity>,
}

impl TransientGenericParameterOriginResolver {
    pub(super) fn new() -> Self {
        Self {
            origins: FxHashMap::default(),
        }
    }

    /// Register every generic parameter in a list under the given declaration origin.
    fn register_list(
        &mut self,
        type_environment: &TypeEnvironment,
        list_id: GenericParameterListId,
        declaration_origin: GenericDeclarationOrigin,
        string_table: &StringTable,
    ) -> Result<(), CompilerError> {
        let list = type_environment
            .generic_parameters(list_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "defined public type-surface projection: GenericParameterListId({}) is missing \
                 from the TypeEnvironment while registering generic-parameter origins",
                    list_id.0
                ))
            })?;

        for (position, parameter) in list.parameters.iter().enumerate() {
            let authored_name = string_table.resolve(parameter.name).to_owned();
            let identity = ExportedGenericParameterIdentity::new(
                declaration_origin.clone(),
                position as u32,
                authored_name,
            );
            if self.origins.insert(parameter.id, identity).is_some() {
                return Err(CompilerError::compiler_error(format!(
                    "defined public type-surface projection: GenericParameterId({}) is \
                     registered under two different declaration origins; ambiguous generic \
                     ownership",
                    parameter.id.0
                )));
            }
        }

        Ok(())
    }

    /// Register one donor-local `GenericParameterId` under an already-established stable
    /// identity.
    ///
    /// WHAT: used by receiver-method parameter aliasing so the method's local parameter ID
    /// maps to the receiver nominal's stable exported identity without making the method a
    /// generic declaration owner. A donor-local ID already registered under the same identity
    /// is idempotent; under a conflicting identity it is a `CompilerError`.
    fn register_aligned_parameter_alias(
        &mut self,
        parameter_id: GenericParameterId,
        identity: ExportedGenericParameterIdentity,
    ) -> Result<(), CompilerError> {
        match self.origins.get(&parameter_id) {
            Some(existing) if existing == &identity => Ok(()),
            Some(existing) => Err(CompilerError::compiler_error(format!(
                "defined public type-surface projection: GenericParameterId({}) is already \
                 registered under a different stable identity ({:?}); receiver-method \
                 parameter aliasing cannot override an existing registration",
                parameter_id.0, existing
            ))),
            None => {
                self.origins.insert(parameter_id, identity);
                Ok(())
            }
        }
    }
}

impl GenericParameterOriginResolver for TransientGenericParameterOriginResolver {
    fn resolve_generic_parameter_origin(
        &self,
        parameter_id: GenericParameterId,
    ) -> Result<ExportedGenericParameterIdentity, CompilerError> {
        self.origins.get(&parameter_id).cloned().ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "defined public type-surface projection: GenericParameterId({}) has no \
                 registered exported generic declaration owner; receiver methods must not become \
                 generic declaration owners and private or unregistered parameters cannot \
                 enter the public type surface",
                parameter_id.0
            ))
        })
    }
}

// ---------------------------------------------------------------------------
//  Root-to-binding join index
// ---------------------------------------------------------------------------

/// Indexes the resolved roots by their public declaration name for deterministic join to export
/// bindings.
///
/// The export bindings are keyed by `public_name: String`, which is the last component of the
/// root's declaration path. Building this index by name lets the projection iterate over the
/// already-sorted export bindings and find each matching root without re-scanning headers.
///
/// Construction is total: a root without a resolvable name is a `CompilerError`, and two roots
/// sharing a public name is a duplicate that is rejected rather than silently overwriting the
/// first. Roots are removed as bindings consume them, so a root left unmatched after every
/// binding has joined is a stale/extra root that is reported explicitly.
pub(super) struct RootIndex<'a> {
    roots_by_name: FxHashMap<String, &'a ResolvedPublicTypeRoot>,
}

impl<'a> RootIndex<'a> {
    pub(super) fn new(
        roots: &'a [ResolvedPublicTypeRoot],
        string_table: &StringTable,
    ) -> Result<Self, CompilerError> {
        let mut roots_by_name = FxHashMap::default();
        for root in roots {
            let name = root.path.name_str(string_table).ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "defined public type-surface projection: a public type root has no \
                     resolvable defining name (path: {:?})",
                    root.path
                ))
            })?;
            if roots_by_name.insert(name.to_owned(), root).is_some() {
                return Err(CompilerError::compiler_error(format!(
                    "defined public type-surface projection: two public type roots share the \
                     public name '{}'; a duplicate public root must not silently overwrite the \
                     first",
                    name
                )));
            }
        }
        Ok(Self { roots_by_name })
    }

    /// Remove and return the root for an export binding, or `CompilerError` when no root matches.
    ///
    /// Roots retain their defining declaration names while bindings retain public aliases. The
    /// stable origin joins those two identities without adding a duplicate alias entry that would
    /// leave the defining root unmatched after projection.
    pub(super) fn take_for_binding(
        &mut self,
        binding: &ExportBinding,
    ) -> Result<&'a ResolvedPublicTypeRoot, CompilerError> {
        let defining_name = binding.origin().defining_name();
        self.roots_by_name.remove(defining_name).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "defined public type-surface projection: the export binding '{}' for defining \
                 declaration '{}' has no matching public type root; every non-trait binding must \
                 join exactly one root",
                binding.public_name(),
                defining_name
            ))
        })
    }

    /// The remaining unmatched root names in deterministic sorted order, for an
    /// unmatched-extra-root diagnostic. Determinism here is diagnostic-only: it never affects
    /// the projected surface, only which names appear in the error.
    pub(super) fn remaining_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.roots_by_name.keys().cloned().collect();
        names.sort();
        names
    }
}

// ---------------------------------------------------------------------------
//  Generic-parameter origin registration
// ---------------------------------------------------------------------------

/// Register generic-parameter origins from function and nominal roots, then alias
/// receiver-method generic parameters to their receiver nominal's stable identities.
///
/// Free functions with a `GenericParameterListId` register their parameters under a
/// `GenericDeclarationOrigin::free_function`. Struct/choice roots register their parameters
/// under a `GenericDeclarationOrigin::nominal_type`. Receiver methods with a validated
/// `GenericFunctionTemplate` alias their local `GenericParameterId` values to the receiver
/// nominal's already-registered stable identities without becoming declaration owners.
pub(super) fn register_generic_parameter_origins(
    generic_resolver: &mut TransientGenericParameterOriginResolver,
    root_table: &ResolvedPublicTypeRootTable,
    export_bindings: &[ExportBinding],
    generic_function_templates: &FxHashMap<InternedPath, GenericFunctionTemplate>,
    nominal_resolver: &TransientNominalOriginResolver,
    type_environment: &TypeEnvironment,
    string_table: &StringTable,
) -> Result<(), CompilerError> {
    // Build a defining-name-to-function-origin lookup from the export bindings. Public aliases
    // remain spelling only and must not affect generic declaration identity.
    let mut function_origin_by_name: FxHashMap<&str, &OriginFunctionId> = FxHashMap::default();
    for binding in export_bindings {
        if let OriginDeclarationId::Function(function_origin) = binding.origin() {
            if function_origin.module_origin() != binding.exporting_module() {
                continue;
            }
            function_origin_by_name.insert(function_origin.defining_name(), function_origin);
        }
    }

    for root in &root_table.roots {
        match &root.kind {
            ResolvedPublicTypeRootKind::Function {
                generic_parameter_list_id: Some(list_id),
                ..
            } => {
                let function_name = root.path.name_str(string_table).ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "defined public type-surface projection: a public free-function root \
                         has no resolvable name (path: {:?})",
                        root.path
                    ))
                })?;

                let function_origin = function_origin_by_name
                    .get(function_name)
                    .copied()
                    .ok_or_else(|| {
                        CompilerError::compiler_error(format!(
                            "defined public type-surface projection: the generic free \
                                 function '{}' has no matching export binding",
                            function_name
                        ))
                    })?;

                let declaration_origin =
                    GenericDeclarationOrigin::free_function(function_origin.clone())?;

                generic_resolver.register_list(
                    type_environment,
                    *list_id,
                    declaration_origin,
                    string_table,
                )?;
            }
            ResolvedPublicTypeRootKind::Function {
                generic_parameter_list_id: None,
                ..
            } => {}

            ResolvedPublicTypeRootKind::Struct { type_id, .. } => {
                register_nominal_generic_origins(
                    generic_resolver,
                    *type_id,
                    type_environment,
                    nominal_resolver,
                    string_table,
                )?;
            }
            ResolvedPublicTypeRootKind::Choice { type_id } => {
                register_nominal_generic_origins(
                    generic_resolver,
                    *type_id,
                    type_environment,
                    nominal_resolver,
                    string_table,
                )?;
            }
            ResolvedPublicTypeRootKind::TransparentAlias { .. }
            | ResolvedPublicTypeRootKind::Constant { .. } => {}
        }
    }

    // After nominal origins are registered, alias receiver-method local generic parameter IDs
    // to their receiver nominal's stable identities. Receiver methods must not become
    // generic declaration owners; they reuse the nominal's parameters by alignment.
    register_receiver_method_generic_parameter_aliases(
        generic_resolver,
        &root_table.receiver_methods,
        generic_function_templates,
        type_environment,
        string_table,
    )?;

    Ok(())
}

/// Register generic-parameter origins for one nominal (struct or choice) root.
fn register_nominal_generic_origins(
    generic_resolver: &mut TransientGenericParameterOriginResolver,
    type_id: TypeId,
    type_environment: &TypeEnvironment,
    nominal_resolver: &TransientNominalOriginResolver,
    string_table: &StringTable,
) -> Result<(), CompilerError> {
    let (nominal_id, generic_parameter_list_id) = match type_environment.get(type_id) {
        Some(TypeDefinition::Struct(def)) => (def.id, def.generic_parameters),
        Some(TypeDefinition::Choice(def)) => (def.id, def.generic_parameters),
        _ => {
            return Err(CompilerError::compiler_error(format!(
                "defined public type-surface projection: a nominal root TypeId({}) is neither a \
                 struct nor a choice definition",
                type_id.0
            )));
        }
    };

    let Some(list_id) = generic_parameter_list_id else {
        return Ok(());
    };

    let nominal_origin =
        NominalOriginResolver::resolve_nominal_origin(nominal_resolver, nominal_id)?;

    let declaration_origin = GenericDeclarationOrigin::nominal_type(nominal_origin)?;
    generic_resolver.register_list(type_environment, list_id, declaration_origin, string_table)?;

    Ok(())
}

/// Alias receiver-method local generic parameter IDs to their receiver nominal's stable
/// exported generic parameter identities.
///
/// WHAT: for each receiver method with a validated `GenericFunctionTemplate`, resolves the
/// method's `GenericParameterListId` and the receiver nominal's `GenericParameterListId` from
/// the `TypeEnvironment`, verifies position-by-position authored-name alignment, then aliases
/// each receiver-local `GenericParameterId` to the nominal's already-registered
/// `ExportedGenericParameterIdentity`. The receiver method must not become a
/// `GenericDeclarationOrigin` owner.
fn register_receiver_method_generic_parameter_aliases(
    generic_resolver: &mut TransientGenericParameterOriginResolver,
    receiver_method_entries: &[ReceiverMethodEntry],
    generic_function_templates: &FxHashMap<InternedPath, GenericFunctionTemplate>,
    type_environment: &TypeEnvironment,
    string_table: &StringTable,
) -> Result<(), CompilerError> {
    for entry in receiver_method_entries {
        let Some(template) = generic_function_templates.get(&entry.function_path) else {
            continue;
        };

        let receiver_path = match &entry.receiver {
            ReceiverKey::Struct(path) | ReceiverKey::Choice(path) => path,
            ReceiverKey::External(_) | ReceiverKey::BuiltinScalar(_) => {
                return Err(CompilerError::compiler_error(format!(
                    "defined public type-surface projection: a receiver method with a \
                     validated generic template carries a non-nominal receiver key ({:?}); \
                     aligned generic parameters require a nominal receiver",
                    entry.receiver
                )));
            }
        };

        let nominal_id = type_environment
            .nominal_id_for_path(receiver_path)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "defined public type-surface projection: a generic receiver method's \
                     receiver path is not a registered nominal (path: {:?})",
                    receiver_path
                ))
            })?;

        let nominal_generic_list_id = match type_environment.struct_definition(nominal_id) {
            Some(def) => def.generic_parameters,
            None => match type_environment.choice_definition(nominal_id) {
                Some(def) => def.generic_parameters,
                None => {
                    return Err(CompilerError::compiler_error(format!(
                        "defined public type-surface projection: a generic receiver method's \
                         nominal ID ({}) is neither a struct nor a choice definition",
                        nominal_id.0
                    )));
                }
            },
        };

        let Some(nominal_list_id) = nominal_generic_list_id else {
            return Err(CompilerError::compiler_error(format!(
                "defined public type-surface projection: a generic receiver method on \
                 non-generic nominal {:?} has a validated generic template; a generic \
                 receiver method requires a generic receiver nominal",
                receiver_path
            )));
        };

        alias_aligned_generic_parameters(
            generic_resolver,
            type_environment,
            nominal_list_id,
            template.generic_parameter_list_id,
            string_table,
        )?;
    }

    Ok(())
}

/// Alias each receiver-method generic parameter to the receiver nominal's already-registered
/// stable identity, verifying authored-name alignment at each position.
fn alias_aligned_generic_parameters(
    generic_resolver: &mut TransientGenericParameterOriginResolver,
    type_environment: &TypeEnvironment,
    nominal_list_id: GenericParameterListId,
    method_list_id: GenericParameterListId,
    string_table: &StringTable,
) -> Result<(), CompilerError> {
    let nominal_list = type_environment
        .generic_parameters(nominal_list_id)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "defined public type-surface projection: the receiver nominal's \
                 GenericParameterListId({}) is missing from the TypeEnvironment while \
                 aliasing receiver-method generic parameters",
                nominal_list_id.0
            ))
        })?;

    let method_list = type_environment
        .generic_parameters(method_list_id)
        .ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "defined public type-surface projection: the receiver method's \
                 GenericParameterListId({}) is missing from the TypeEnvironment while \
                 aliasing receiver-method generic parameters",
                method_list_id.0
            ))
        })?;

    if nominal_list.parameters.len() != method_list.parameters.len() {
        return Err(CompilerError::compiler_error(format!(
            "defined public type-surface projection: a generic receiver method has {} \
             generic parameters but its receiver nominal has {}; aligned parameters \
             must match in count",
            method_list.parameters.len(),
            nominal_list.parameters.len()
        )));
    }

    for (nominal_param, method_param) in nominal_list
        .parameters
        .iter()
        .zip(method_list.parameters.iter())
    {
        let nominal_name = string_table.resolve(nominal_param.name);
        let method_name = string_table.resolve(method_param.name);
        if nominal_name != method_name {
            return Err(CompilerError::compiler_error(format!(
                "defined public type-surface projection: a generic receiver method's \
                 parameter '{}' does not match the receiver nominal's parameter '{}'; \
                 aligned parameters must match in authored name and order",
                method_name, nominal_name
            )));
        }

        // The nominal's GenericParameterId is already registered; resolve its stable identity
        // and alias the method's local ID under it.
        let nominal_identity =
            generic_resolver.resolve_generic_parameter_origin(nominal_param.id)?;
        generic_resolver.register_aligned_parameter_alias(method_param.id, nominal_identity)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
//  Shared trait source-fact projection
// ---------------------------------------------------------------------------

/// Project one resolved trait source fact to its stable canonical trait identity.
///
/// WHAT: a source trait ([`ResolvedTraitSourceFact::Source`]) resolves to
/// `CanonicalTraitIdentity::Source` through the public source-trait origin index; a core
/// trait ([`ResolvedTraitSourceFact::Core`]) resolves to its stable
/// [`CanonicalCoreTraitIdentity`]. A source trait whose canonical path has no retained
/// public source-trait origin is a `CompilerError`.
/// WHY: this is the single source/core mapping owner shared by generic-bound projection,
/// direct public trait incompatibility projection and evidence projection, so every path
/// resolves a retained trait source fact to the same canonical identity through one
/// implementation. Extracting it keeps the source/core classification logic in the type
/// projection owner rather than duplicating it.
pub(super) fn project_trait_source_fact_to_canonical_identity(
    source_fact: &ResolvedTraitSourceFact,
    public_source_trait_origins: &FxHashMap<InternedPath, OriginTraitId>,
) -> Result<CanonicalTraitIdentity, CompilerError> {
    match source_fact {
        ResolvedTraitSourceFact::Source(path) => {
            let Some(origin) = public_source_trait_origins.get(path) else {
                return Err(CompilerError::compiler_error(format!(
                    "defined public type-surface projection: a trait source path {:?} has no retained public source-trait origin; a private, unexported or unowned trait must not enter the public type surface",
                    path
                )));
            };
            Ok(CanonicalTraitIdentity::Source(origin.clone()))
        }
        ResolvedTraitSourceFact::Core(kind) => {
            let core_identity = match kind {
                CoreTraitKind::Displayable => CanonicalCoreTraitIdentity::Displayable,
                CoreTraitKind::Castable {
                    target,
                    fallibility,
                } => CanonicalCoreTraitIdentity::Castable {
                    target: *target,
                    fallibility: *fallibility,
                },
            };
            Ok(CanonicalTraitIdentity::Core(core_identity))
        }
    }
}

/// Project ordered canonical trait bound identities for one generic parameter.
fn project_generic_parameter_bounds(
    parameter_id: GenericParameterId,
    type_environment: &TypeEnvironment,
    trait_source_facts: &FxHashMap<TraitId, ResolvedTraitSourceFact>,
    public_source_trait_origins: &FxHashMap<InternedPath, OriginTraitId>,
) -> Result<Vec<CanonicalTraitIdentity>, CompilerError> {
    let Some(bounds) = type_environment.trait_bounds_for_generic_parameter(parameter_id) else {
        return Ok(Vec::new());
    };

    let mut canonical_bounds = Vec::with_capacity(bounds.len());
    for trait_id in bounds {
        let Some(source_fact) = trait_source_facts.get(trait_id) else {
            return Err(CompilerError::compiler_error(format!(
                "defined public type-surface projection: a generic parameter bound TraitId({}) has no retained trait source fact; a missing local mapping is an internal invariant violation",
                trait_id.0
            )));
        };

        let canonical_identity = project_trait_source_fact_to_canonical_identity(
            source_fact,
            public_source_trait_origins,
        )?;

        // Reject a duplicate canonical bound identity.
        if canonical_bounds.contains(&canonical_identity) {
            return Err(CompilerError::compiler_error(format!(
                "defined public type-surface projection: two generic parameter bounds resolved to the same canonical trait identity {:?}; a duplicate bound identity must not enter the public type surface",
                canonical_identity
            )));
        }

        canonical_bounds.push(canonical_identity);
    }

    Ok(canonical_bounds)
}

/// Project one root's exported generic parameter surfaces (identity plus ordered bounds) in
/// declaration-local order.
fn project_exported_generic_parameter_surfaces(
    generic_parameter_list_id: Option<GenericParameterListId>,
    type_environment: &TypeEnvironment,
    generic_resolver: &dyn GenericParameterOriginResolver,
    expected_origin: &GenericDeclarationOrigin,
    trait_source_facts: &FxHashMap<TraitId, ResolvedTraitSourceFact>,
    public_source_trait_origins: &FxHashMap<InternedPath, OriginTraitId>,
) -> Result<Vec<PublicGenericParameterSurface>, CompilerError> {
    let Some(list_id) = generic_parameter_list_id else {
        return Ok(Vec::new());
    };

    let list = type_environment.generic_parameters(list_id).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "defined public type-surface projection: GenericParameterListId({}) is missing from the TypeEnvironment while projecting exported generic parameter surfaces",
            list_id.0
        ))
    })?;

    let mut surfaces: Vec<PublicGenericParameterSurface> =
        Vec::with_capacity(list.parameters.len());
    for parameter in &list.parameters {
        let identity = generic_resolver.resolve_generic_parameter_origin(parameter.id)?;

        if identity.declaration_origin() != expected_origin {
            return Err(CompilerError::compiler_error(format!(
                "defined public type-surface projection: an exported generic parameter resolved to declaration origin {:?} but its root owner is {:?}; a wrong-owner parameter must not enter the public type surface",
                identity.declaration_origin(),
                expected_origin,
            )));
        }

        if surfaces.iter().any(|surface| surface.identity == identity) {
            return Err(CompilerError::compiler_error(format!(
                "defined public type-surface projection: two exported generic parameters resolved to the same identity {:?}; a duplicate parameter identity must not enter the public type surface",
                identity,
            )));
        }

        let bounds = project_generic_parameter_bounds(
            parameter.id,
            type_environment,
            trait_source_facts,
            public_source_trait_origins,
        )?;

        surfaces.push(PublicGenericParameterSurface { identity, bounds });
    }

    Ok(surfaces)
}

// ---------------------------------------------------------------------------
//  Per-root type projection
// ---------------------------------------------------------------------------

/// Project one free-function root into its final [`PublicFunctionSemantics`].
///
/// WHAT: projects the exported generic parameter surfaces, the canonical parameter slots and the
/// success/error return slots. The callable category is derived from whether the function has
/// exported generic parameters: a non-generic function is `ConcreteLocal`, a generic function
/// carries a [`PublicGenericTemplateDescriptor`].
#[allow(clippy::too_many_arguments)]
pub(super) fn project_free_function_semantics(
    function_origin: OriginFunctionId,
    generic_parameter_list_id: Option<GenericParameterListId>,
    signature: &FunctionSignature,
    type_environment: &TypeEnvironment,
    context: &CanonicalTypeProjectionContext,
    trait_source_facts: &FxHashMap<TraitId, ResolvedTraitSourceFact>,
    public_source_trait_origins: &FxHashMap<InternedPath, OriginTraitId>,
    string_table: &StringTable,
) -> Result<PublicFunctionSemantics, CompilerError> {
    let expected_origin = GenericDeclarationOrigin::free_function(function_origin.clone())?;

    let generic_parameters = project_exported_generic_parameter_surfaces(
        generic_parameter_list_id,
        type_environment,
        context.generic_parameter_origins(),
        &expected_origin,
        trait_source_facts,
        public_source_trait_origins,
    )?;

    let parameters = signature
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
                context,
            )?;
            let folded_default = project_folded_default(
                &declaration.value,
                type_environment,
                context,
                string_table,
            )?;
            let access = project_parameter_access(declaration)?;
            Ok(PublicParameterTypeSlot {
                name,
                type_identity,
                access,
                folded_default,
            })
        })
        .collect::<Result<Vec<_>, CompilerError>>()?;

    let (returns, error_return) =
        project_return_slots(&signature.returns, type_environment, context)?;

    let category = if generic_parameters.is_empty() {
        PublicFunctionCategory::ConcreteLocal
    } else {
        PublicFunctionCategory::GenericTemplate(PublicGenericTemplateDescriptor {
            generic_parameters,
        })
    };

    Ok(PublicFunctionSemantics {
        category,
        parameters,
        returns,
        error_return,
    })
}

/// Project one struct root into its exported generic parameter surfaces and ordered field slots.
///
/// WHAT: validates the nominal resolves to the export binding's origin, projects the exported
/// generic parameter surfaces and joins the retained field declarations against the canonical
/// `StructTypeDefinition` fields. Returns the parts the declaration join assembles into a
/// [`super::model::PublicStructSemantics`] after attaching receiver methods.
#[allow(clippy::too_many_arguments)]
pub(super) fn project_struct_parts(
    type_origin: OriginTypeId,
    type_id: TypeId,
    fields: &[Declaration],
    type_environment: &TypeEnvironment,
    context: &CanonicalTypeProjectionContext,
    trait_source_facts: &FxHashMap<TraitId, ResolvedTraitSourceFact>,
    public_source_trait_origins: &FxHashMap<InternedPath, OriginTraitId>,
    string_table: &StringTable,
) -> Result<(Vec<PublicGenericParameterSurface>, Vec<PublicFieldTypeSlot>), CompilerError> {
    let definition = type_environment.get(type_id).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "defined public type-surface projection: struct root TypeId({}) is not registered \
             in the TypeEnvironment",
            type_id.0
        ))
    })?;

    let struct_definition = match definition {
        TypeDefinition::Struct(def) => def,
        _ => {
            return Err(CompilerError::compiler_error(format!(
                "defined public type-surface projection: struct root TypeId({}) does not \
                 resolve to a struct definition",
                type_id.0
            )));
        }
    };

    // Validate that the nominal resolves through the public nominal origin resolver to the
    // same origin as the export binding.
    let resolved_origin = context
        .nominal_origins()
        .resolve_nominal_origin(struct_definition.id)?;
    if resolved_origin != type_origin {
        return Err(CompilerError::compiler_error(format!(
            "defined public type-surface projection: struct root TypeId({}) resolves to \
             origin {:?} but the export binding carries origin {:?}",
            type_id.0, resolved_origin, type_origin
        )));
    }

    let expected_origin = GenericDeclarationOrigin::nominal_type(type_origin.clone())?;

    let generic_parameters = project_exported_generic_parameter_surfaces(
        struct_definition.generic_parameters,
        type_environment,
        context.generic_parameter_origins(),
        &expected_origin,
        trait_source_facts,
        public_source_trait_origins,
    )?;

    let projected_fields = project_fields_with_defaults(
        type_id,
        struct_definition,
        fields,
        type_environment,
        context,
        string_table,
    )?;

    Ok((generic_parameters, projected_fields))
}

/// Project one choice root into its exported generic parameter surfaces and ordered variants.
///
/// WHAT: validates the nominal resolves to the export binding's origin, projects the exported
/// generic parameter surfaces and the choice variants. Returns the parts the declaration join
/// assembles into a [`super::model::PublicChoiceSemantics`] after attaching receiver methods.
#[allow(clippy::too_many_arguments)]
pub(super) fn project_choice_parts(
    type_origin: OriginTypeId,
    type_id: TypeId,
    type_environment: &TypeEnvironment,
    context: &CanonicalTypeProjectionContext,
    trait_source_facts: &FxHashMap<TraitId, ResolvedTraitSourceFact>,
    public_source_trait_origins: &FxHashMap<InternedPath, OriginTraitId>,
    string_table: &StringTable,
) -> Result<
    (
        Vec<PublicGenericParameterSurface>,
        Vec<PublicChoiceVariantSurface>,
    ),
    CompilerError,
> {
    let definition = type_environment.get(type_id).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "defined public type-surface projection: choice root TypeId({}) is not registered \
             in the TypeEnvironment",
            type_id.0
        ))
    })?;

    let choice_definition = match definition {
        TypeDefinition::Choice(def) => def,
        _ => {
            return Err(CompilerError::compiler_error(format!(
                "defined public type-surface projection: choice root TypeId({}) does not \
                 resolve to a choice definition",
                type_id.0
            )));
        }
    };

    // Validate that the nominal resolves through the public nominal origin resolver to the
    // same origin as the export binding.
    let resolved_origin = context
        .nominal_origins()
        .resolve_nominal_origin(choice_definition.id)?;
    if resolved_origin != type_origin {
        return Err(CompilerError::compiler_error(format!(
            "defined public type-surface projection: choice root TypeId({}) resolves to \
             origin {:?} but the export binding carries origin {:?}",
            type_id.0, resolved_origin, type_origin
        )));
    }

    let expected_origin = GenericDeclarationOrigin::nominal_type(type_origin.clone())?;

    let generic_parameters = project_exported_generic_parameter_surfaces(
        choice_definition.generic_parameters,
        type_environment,
        context.generic_parameter_origins(),
        &expected_origin,
        trait_source_facts,
        public_source_trait_origins,
    )?;

    let variants =
        project_choice_variants(choice_definition, type_environment, context, string_table)?;

    Ok((generic_parameters, variants))
}

/// Project one transparent-alias root into its resolved target type identity.
pub(super) fn project_alias_target(
    target_type_id: TypeId,
    type_environment: &TypeEnvironment,
    context: &CanonicalTypeProjectionContext,
) -> Result<CanonicalTypeIdentity, CompilerError> {
    project_type_id_to_canonical_identity(target_type_id, type_environment, context)
}

/// Project one constant root's canonical type identity.
///
/// The defining declaration path used to fold the constant value is retained on the resolved
/// root (`root.path`) and consumed by the declaration join, not by this helper.
pub(super) fn project_constant_type_identity(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
    context: &CanonicalTypeProjectionContext,
) -> Result<CanonicalTypeIdentity, CompilerError> {
    project_type_id_to_canonical_identity(type_id, type_environment, context)
}

// ---------------------------------------------------------------------------
//  Shared leaf projection helpers
// ---------------------------------------------------------------------------

/// Project success and error return slots, returning them separately.
///
/// A resolved public signature slot missing `TypeId` is `CompilerError`; no slot is omitted.
pub(super) fn project_return_slots(
    return_slots: &[ReturnSlot],
    type_environment: &TypeEnvironment,
    context: &CanonicalTypeProjectionContext,
) -> Result<(Vec<PublicReturnTypeSlot>, Option<CanonicalTypeIdentity>), CompilerError> {
    let mut returns = Vec::new();
    let mut error_return = None;

    for slot in return_slots {
        let type_id = slot.type_id.ok_or_else(|| {
            CompilerError::compiler_error(
                "defined public type-surface projection: a resolved public signature return \
                 slot has no TypeId; the signature was not fully resolved before projection",
            )
        })?;

        let type_identity =
            project_type_id_to_canonical_identity(type_id, type_environment, context)?;

        match slot.channel {
            ReturnChannel::Success => returns.push(PublicReturnTypeSlot { type_identity }),
            ReturnChannel::Error => {
                if error_return.is_some() {
                    return Err(CompilerError::compiler_error(
                        "defined public type-surface projection: a public signature carries \
                         multiple error-channel return slots",
                    ));
                }
                error_return = Some(type_identity);
            }
        }
    }

    Ok((returns, error_return))
}

/// Total-join retained struct field declarations against the canonical
/// [`StructTypeDefinition`] fields and project stable field type slots with folded defaults.
///
/// WHAT: the canonical `StructTypeDefinition.fields` is the sole type authority for field
/// names, order and `TypeId`s. The retained `Declaration` values supply only the folded
/// default expression for each field. The join rejects count, name/order, duplicate-name and
/// declaration-value `TypeId` mismatches with a `CompilerError` so the retained declaration
/// vector is never trusted as a parallel type authority.
#[allow(clippy::too_many_arguments)]
fn project_fields_with_defaults(
    root_type_id: TypeId,
    struct_definition: &StructTypeDefinition,
    field_declarations: &[Declaration],
    type_environment: &TypeEnvironment,
    context: &CanonicalTypeProjectionContext,
    string_table: &StringTable,
) -> Result<Vec<PublicFieldTypeSlot>, CompilerError> {
    if struct_definition.fields.len() != field_declarations.len() {
        return Err(CompilerError::compiler_error(format!(
            "defined public type-surface projection: struct root TypeId({}) has {} canonical fields but {} retained field declarations; the retained declaration count must match the canonical struct definition",
            root_type_id.0,
            struct_definition.fields.len(),
            field_declarations.len(),
        )));
    }

    let mut seen_names: FxHashSet<StringId> = FxHashSet::default();

    let mut projected_fields = Vec::with_capacity(struct_definition.fields.len());
    for (canonical_field, declaration) in struct_definition
        .fields
        .iter()
        .zip(field_declarations.iter())
    {
        let canonical_name_id = canonical_field.name.name();
        let declaration_name_id = declaration.id.name();

        if canonical_name_id != declaration_name_id {
            return Err(CompilerError::compiler_error(format!(
                "defined public type-surface projection: struct root TypeId({}) has a field name or order mismatch at canonical field {:?}; the retained declaration carries {:?}; the retained declarations must match the canonical field order",
                root_type_id.0, canonical_field.name, declaration.id,
            )));
        }

        if let Some(name_id) = canonical_name_id
            && !seen_names.insert(name_id)
        {
            return Err(CompilerError::compiler_error(format!(
                "defined public type-surface projection: struct root TypeId({}) has a duplicate \
                 field name {:?}; canonical struct definitions must not contain duplicates",
                root_type_id.0, canonical_field.name,
            )));
        }

        let name = canonical_field
            .name
            .name_str(string_table)
            .map(|name| name.to_owned())
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "defined public type-surface projection: a struct field has no resolvable name (path: {:?})",
                    canonical_field.name
                ))
            })?;

        if canonical_field.type_id != declaration.value.type_id {
            return Err(CompilerError::compiler_error(format!(
                "defined public type-surface projection: struct root TypeId({}) field {:?} has a TypeId mismatch: canonical TypeId({}) vs retained declaration TypeId({}); the retained declaration must agree with the canonical struct definition",
                root_type_id.0,
                canonical_field.name,
                canonical_field.type_id.0,
                declaration.value.type_id.0,
            )));
        }

        let type_identity = project_type_id_to_canonical_identity(
            canonical_field.type_id,
            type_environment,
            context,
        )?;

        let folded_default =
            project_folded_default(&declaration.value, type_environment, context, string_table)?;
        projected_fields.push(PublicFieldTypeSlot {
            name,
            type_identity,
            folded_default,
        });
    }
    Ok(projected_fields)
}

/// Project choice variants into stable variant type surfaces.
fn project_choice_variants(
    choice_definition: &ChoiceTypeDefinition,
    type_environment: &TypeEnvironment,
    context: &CanonicalTypeProjectionContext,
    string_table: &StringTable,
) -> Result<Vec<PublicChoiceVariantSurface>, CompilerError> {
    let mut variants = Vec::with_capacity(choice_definition.variants.len());
    for variant in choice_definition.variants.iter() {
        let name = string_table.resolve(variant.name).to_owned();

        let payload_fields = match &variant.payload {
            ChoiceVariantPayloadDefinition::Unit => Vec::new(),
            ChoiceVariantPayloadDefinition::Record { fields } => {
                let mut projected_fields = Vec::with_capacity(fields.len());
                for field in fields.iter() {
                    let field_name = field
                        .name
                        .name_str(string_table)
                        .map(|name| name.to_owned())
                        .ok_or_else(|| {
                            CompilerError::compiler_error(format!(
                                "defined public type-surface projection: a choice variant \
                                 payload field has no resolvable name (path: {:?})",
                                field.name
                            ))
                        })?;

                    let type_identity = project_type_id_to_canonical_identity(
                        field.type_id,
                        type_environment,
                        context,
                    )?;

                    projected_fields.push(PublicFieldTypeSlot {
                        name: field_name,
                        type_identity,
                        folded_default: None,
                    });
                }
                projected_fields
            }
        };

        variants.push(PublicChoiceVariantSurface {
            name,
            payload_fields,
        });
    }
    Ok(variants)
}

/// Project one parameter or field default expression to an owned [`PublicFoldedValue`].
///
/// WHAT: a NoValue expression means the slot has no default and returns None. Any other
/// expression kind is converted through the shared folded-value converter, which expects the
/// expression to be already normalized. A Template or Reference reaching this boundary
/// is an internal CompilerError naming the invariant violation.
pub(super) fn project_folded_default(
    expression: &Expression,
    type_environment: &TypeEnvironment,
    context: &CanonicalTypeProjectionContext,
    string_table: &StringTable,
) -> Result<Option<PublicFoldedValue>, CompilerError> {
    if matches!(expression.kind, ExpressionKind::NoValue) {
        return Ok(None);
    }
    convert_expression_to_folded_value(expression, type_environment, string_table, context)
        .map(Some)
}

/// Projects declaration-owned access without consulting HIR or borrow-analysis side tables.
pub(super) fn project_parameter_access(
    declaration: &Declaration,
) -> Result<PublicCallParameterAccess, CompilerError> {
    match declaration.value.reactive_source.as_ref() {
        Some(source) if source.kind == ReactiveSourceKind::Declaration => {
            Err(CompilerError::compiler_error(format!(
                "defined public type-surface projection found reactive declaration metadata on function parameter {:?}",
                declaration.id
            )))
        }
        Some(_) if declaration.value.value_mode.is_mutable() => {
            Err(CompilerError::compiler_error(format!(
                "defined public type-surface projection found mutable reactive function parameter {:?}",
                declaration.id
            )))
        }
        Some(_) => Ok(PublicCallParameterAccess::Reactive),
        None if declaration.value.value_mode.is_mutable() => Ok(PublicCallParameterAccess::Mutable),
        None => Ok(PublicCallParameterAccess::Shared),
    }
}

/// Construct a `CompilerError` for a root-to-binding origin category mismatch.
pub(super) fn origin_category_mismatch_error(
    expected: &str,
    binding: &ExportBinding,
) -> CompilerError {
    CompilerError::compiler_error(format!(
        "defined public type-surface projection: a {} root matched an export binding with \
         origin {:?} (public name '{}'); the root category and binding origin category disagree",
        expected,
        binding.origin(),
        binding.public_name()
    ))
}
