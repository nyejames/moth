//! `TypeEnvironment` — table-backed owner of resolved frontend semantic type identity.
//!
//! WHAT: stores all type definitions and guarantees canonical identity through interning.
//! WHY: AST should carry compact `TypeId`s instead of repeatedly cloning large
//!      `DataType` payloads.
//!
//! Boundaries:
//! - Parsed type refs (`ParsedTypeRef`) do NOT belong here.
//! - Backend layout, ABI, drop strategy, and runtime representation do NOT belong here.
//! - Type compatibility POLICY does NOT belong here (see `type_coercion`).

use crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::ExternalTypeId;
use crate::compiler_frontend::instrumentation::{
    FrontendCounter, add_frontend_counter, increment_frontend_counter,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};
use crate::compiler_frontend::traits::ids::TraitId;

use rustc_hash::{FxHashMap, FxHashSet};

use super::definitions::{
    BuiltinTypeDefinition, ChoiceTypeDefinition, ChoiceVariantDefinition,
    ChoiceVariantPayloadDefinition, ConstructedTypeDefinition, ExternalTypeDefinition,
    FieldDefinition, FunctionParameterDefinition, FunctionTypeDefinition,
    GenericInstanceDefinition, GenericParameterDefinition, StructTypeDefinition, TypeDefinition,
};
use super::generic_bindings::{BindingConflict, GenericTypeBindings};
use super::generic_identity_bridge::{
    BuiltinTypeKey as BridgeBuiltinTypeKey, GenericInstantiationKey, TypeIdentityKey,
};
use super::generic_parameters::{
    GenericParameterList as ParsedGenericParameterList, TypeParameterId,
};
use super::ids::{
    BuiltinTypeConstructor, BuiltinTypeKey, ConstructedTypeKey, FunctionTypeKey,
    GenericInstanceKey, GenericParameterId, GenericParameterListId, NominalTypeId, TypeConstructor,
    TypeId,
};
use super::queries::TypeKind;
use super::{BuiltinScalarReceiver, ReceiverKey};

// -----------------------------------------------------------
//  Supporting Types
// -----------------------------------------------------------

/// The resolved shape of a collection type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CollectionShape {
    pub(crate) element_type: TypeId,
    pub(crate) fixed_capacity: Option<usize>,
}

/// The resolved shape of an ordered map type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MapShape {
    pub(crate) key_type: TypeId,
    pub(crate) value_type: TypeId,
}

/// Compact handles for all builtin types seeded in a fresh `TypeEnvironment`.
#[derive(Debug, Clone, Copy)]
pub struct BuiltinTypes {
    pub bool: TypeId,
    pub int: TypeId,
    pub float: TypeId,
    // Decimal is intentionally inactive in the Alpha surface. The handle is kept
    // only to preserve the stable builtin TypeId layout seeded by TypeEnvironment::new.
    pub decimal: TypeId,
    pub string: TypeId,
    pub char: TypeId,
    pub range: TypeId,
    pub none: TypeId,
}

// -----------------------------------------------------------
//  Type Environment
// -----------------------------------------------------------

/// Owns resolved frontend semantic type identity.
#[derive(Debug, Clone)]
pub struct TypeEnvironment {
    // Sequential storage: index == TypeId.0
    types: Vec<TypeDefinition>,

    // Canonical key -> TypeId maps for interning.
    builtin_ids: FxHashMap<BuiltinTypeKey, TypeId>,
    constructed_ids: FxHashMap<ConstructedTypeKey, TypeId>,
    function_ids: FxHashMap<FunctionTypeKey, TypeId>,
    external_ids: FxHashMap<ExternalTypeId, TypeId>,
    generic_instance_ids: FxHashMap<GenericInstanceKey, TypeId>,
    canonical_type_ids: FxHashMap<CanonicalTypeIdentity, TypeId>,
    canonical_identities: FxHashMap<TypeId, CanonicalTypeIdentity>,

    // Nominal definition storage.
    // `NominalTypeId.0` indexes into `nominal_registry`.
    nominal_registry: Vec<NominalEntry>,
    struct_definitions: Vec<StructTypeDefinition>,
    choice_definitions: Vec<ChoiceTypeDefinition>,

    // Generic parameter list storage (indexed by GenericParameterListId.0).
    generic_parameter_lists: Vec<GenericParameterList>,

    // Generic parameter -> TypeId lookup.
    generic_parameter_ids: FxHashMap<GenericParameterId, TypeId>,

    // Direct index from generic parameter ID to its declaration-site trait bounds.
    //
    // INVARIANT: this map and `GenericParameter.trait_bounds` inside each list in
    // `generic_parameter_lists` are updated together by `register_generic_parameter_list`
    // and `update_generic_parameter_bounds`. Callers that hold a `GenericParameterList`
    // may still inspect `GenericParameter.trait_bounds` directly; this index exists so
    // that `trait_bounds_for_generic_parameter` does not scan every list.
    trait_bounds_by_generic_parameter_id: FxHashMap<GenericParameterId, Vec<TraitId>>,

    // Path -> NominalTypeId lookup.
    nominal_by_path: FxHashMap<InternedPath, NominalTypeId>,

    // NominalTypeId -> TypeId lookup.
    nominal_to_type_id: FxHashMap<NominalTypeId, TypeId>,

    // ID counters.
    next_generic_parameter_id: u32,
    next_generic_parameter_list_id: u32,

    // Seeded builtins.
    builtins: BuiltinTypes,

    // Cached substituted fields/variants for generic instances.
    // WHAT: eagerly computed when a generic instance is interned so that
    //       fields_for/variants_for can return substituted views without &mut self.
    // WHY: many AST call sites hold &TypeEnvironment; lazy substitution with
    //      interning would require &mut self or interior mutability.
    generic_instance_fields: FxHashMap<TypeId, Vec<FieldDefinition>>,
    generic_instance_variants: FxHashMap<TypeId, Vec<ChoiceVariantDefinition>>,

    // Reuses recursive generic substitutions within one module-local environment.
    // WHAT: maps a source TypeId plus a deterministic parameter mapping to the
    //       canonical substituted TypeId.
    // WHY: generic instance fields, variants, and function templates often ask
    //      for the same nested substitution shape repeatedly.
    substitution_cache: FxHashMap<TypeSubstitutionKey, TypeId>,
}

/// Internal enum mapping a `NominalTypeId` to its actual definition storage.
#[derive(Debug, Clone)]
enum NominalEntry {
    Struct(usize), // index into struct_definitions
    Choice(usize), // index into choice_definitions
}

/// A list of generic parameters for a nominal or function definition.
#[derive(Debug, Clone, Default)]
pub struct GenericParameterList {
    pub parameters: Vec<GenericParameter>,
}

/// A single generic parameter descriptor.
#[derive(Debug, Clone)]
pub struct GenericParameter {
    pub id: GenericParameterId,
    pub name: StringId,
    pub(crate) trait_bounds: Vec<TraitId>,
}

/// Result of registering a parsed generic parameter list.
///
/// WHAT: gives callers both the canonical list ID and the declaration-local to
/// canonical parameter mapping.
/// WHY: parser-local `TypeParameterId`s restart at zero for every declaration;
/// semantic generic parameter identity must be unique in this `TypeEnvironment`.
#[derive(Debug, Clone)]
pub struct RegisteredGenericParameterList {
    pub list_id: GenericParameterListId,
    pub canonical_by_local: FxHashMap<TypeParameterId, GenericParameterId>,
}

/// Cache key for substituting one type under one concrete generic mapping.
///
/// WHAT: keeps the unordered `FxHashMap` mapping stable by storing sorted
/// parameter/replacement pairs.
/// WHY: cache hits must be deterministic and independent of hash iteration order.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TypeSubstitutionKey {
    source_type_id: TypeId,
    mapping: Box<[TypeSubstitutionPair]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TypeSubstitutionPair {
    parameter_id: GenericParameterId,
    replacement_type_id: TypeId,
}

impl TypeSubstitutionKey {
    fn new(source_type_id: TypeId, mapping: &FxHashMap<GenericParameterId, TypeId>) -> Self {
        let mut mapping_pairs: Vec<TypeSubstitutionPair> = mapping
            .iter()
            .map(|(parameter_id, replacement_type_id)| TypeSubstitutionPair {
                parameter_id: *parameter_id,
                replacement_type_id: *replacement_type_id,
            })
            .collect();

        mapping_pairs.sort_by_key(|pair| pair.parameter_id.0);

        Self {
            source_type_id,
            mapping: mapping_pairs.into_boxed_slice(),
        }
    }
}

/// Minimal data copied out before recursive substitution mutates the environment.
///
/// WHAT: carries only compact `TypeId`s and small canonical keys.
/// WHY: this avoids cloning full `TypeDefinition` payloads just to satisfy the
/// borrow checker while recursive calls intern derived types.
#[derive(Debug)]
enum TypeSubstitutionSource {
    Constructed {
        constructor: TypeConstructor,
        arguments: Box<[TypeId]>,
    },
    Function {
        parameters: Box<[TypeId]>,
        returns: Box<[TypeId]>,
        error_return: Option<TypeId>,
    },
    GenericInstance {
        base: NominalTypeId,
        arguments: Box<[TypeId]>,
    },
}

impl Default for TypeEnvironment {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeEnvironment {
    // -----------------
    //  Initialization
    // -----------------

    /// Creates a new environment with all builtin types seeded.
    pub fn new() -> Self {
        let mut env = Self {
            types: Vec::new(),
            builtin_ids: FxHashMap::default(),
            constructed_ids: FxHashMap::default(),
            function_ids: FxHashMap::default(),
            external_ids: FxHashMap::default(),
            generic_instance_ids: FxHashMap::default(),
            canonical_type_ids: FxHashMap::default(),
            canonical_identities: FxHashMap::default(),
            nominal_registry: Vec::new(),
            struct_definitions: Vec::new(),
            choice_definitions: Vec::new(),
            generic_parameter_lists: Vec::new(),
            generic_parameter_ids: FxHashMap::default(),
            trait_bounds_by_generic_parameter_id: FxHashMap::default(),
            nominal_by_path: FxHashMap::default(),
            nominal_to_type_id: FxHashMap::default(),
            next_generic_parameter_id: 0,
            next_generic_parameter_list_id: 0,
            builtins: BuiltinTypes {
                bool: TypeId(0),
                int: TypeId(0),
                float: TypeId(0),
                decimal: TypeId(0),
                string: TypeId(0),
                char: TypeId(0),
                range: TypeId(0),
                none: TypeId(0),
            },
            generic_instance_fields: FxHashMap::default(),
            generic_instance_variants: FxHashMap::default(),
            substitution_cache: FxHashMap::default(),
        };

        // Seed builtins. The order here is arbitrary but deterministic.
        let bool_id = env.insert_builtin(BuiltinTypeKey::Bool);
        let int_id = env.insert_builtin(BuiltinTypeKey::Int);
        let float_id = env.insert_builtin(BuiltinTypeKey::Float);
        // Decimal is intentionally inactive in the Alpha surface. It is seeded only
        // to keep the stable builtin TypeId layout; no parser or operator path may
        // produce a live Decimal type.
        let decimal_id = env.insert_builtin(BuiltinTypeKey::Decimal);
        let string_id = env.insert_builtin(BuiltinTypeKey::String);
        let char_id = env.insert_builtin(BuiltinTypeKey::Char);
        let range_id = env.insert_builtin(BuiltinTypeKey::Range);
        let none_id = env.insert_builtin(BuiltinTypeKey::None);

        env.builtins = BuiltinTypes {
            bool: bool_id,
            int: int_id,
            float: float_id,
            decimal: decimal_id,
            string: string_id,
            char: char_id,
            range: range_id,
            none: none_id,
        };

        env
    }

    /// Returns handles for all seeded builtin types.
    pub fn builtins(&self) -> &BuiltinTypes {
        &self.builtins
    }

    // -----------------
    //  Maintenance
    // -----------------

    /// Remaps all interned string handles so this environment can be used with a merged table.
    ///
    /// WHAT: updates nominal paths, field names, variant names, generic parameter names,
    /// function parameter names, and cached generic instance views.
    /// WHY: module compilation uses local `StringTable`s and later merges them into the build
    /// table; every frontend payload that stores `StringId`s must be rewritten at that boundary.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for definition in &mut self.types {
            Self::remap_type_definition(definition, remap);
        }

        for definition in &mut self.struct_definitions {
            Self::remap_struct_definition(definition, remap);
        }

        for definition in &mut self.choice_definitions {
            Self::remap_choice_definition(definition, remap);
        }

        for list in &mut self.generic_parameter_lists {
            for parameter in &mut list.parameters {
                parameter.name = remap.get(parameter.name);
            }
        }

        for fields in self.generic_instance_fields.values_mut() {
            Self::remap_fields(fields, remap);
        }

        for variants in self.generic_instance_variants.values_mut() {
            Self::remap_variants(variants, remap);
        }

        self.rebuild_nominal_path_index();
    }

    // -----------------
    //  Generic Parameters
    // -----------------

    /// Returns the generic parameter list for a given ID, if it exists.
    pub fn generic_parameters(&self, id: GenericParameterListId) -> Option<&GenericParameterList> {
        self.generic_parameter_lists.get(id.0 as usize)
    }

    /// Returns the trait bounds recorded on a canonical generic parameter.
    ///
    /// WHAT: looks up the parameter by semantic ID using the direct index.
    /// WHY: executable body parsing can see only a receiver `TypeId` or concrete substitution;
    ///      bound-method dispatch still needs the declaration-site bounds without rebuilding
    ///      generic metadata in AST lookup code.
    pub(crate) fn trait_bounds_for_generic_parameter(
        &self,
        parameter_id: GenericParameterId,
    ) -> Option<&[TraitId]> {
        self.trait_bounds_by_generic_parameter_id
            .get(&parameter_id)
            .map(|bounds| bounds.as_slice())
    }

    /// Registers a parsed generic parameter list and returns its canonical semantic IDs.
    pub(crate) fn register_generic_parameter_list(
        &mut self,
        parsed_parameters: &ParsedGenericParameterList,
        resolved_bounds_by_local: &FxHashMap<TypeParameterId, Vec<TraitId>>,
    ) -> RegisteredGenericParameterList {
        let id = GenericParameterListId(self.next_generic_parameter_list_id);
        self.next_generic_parameter_list_id += 1;

        let mut parameters = Vec::with_capacity(parsed_parameters.parameters.len());
        let mut canonical_by_local = FxHashMap::default();
        for parameter in &parsed_parameters.parameters {
            let canonical_id = self.allocate_generic_parameter_id();
            canonical_by_local.insert(parameter.id, canonical_id);

            // Registering the list also interns each parameter as a type so later
            // resolution can query a stable TypeId without re-promoting local IDs.
            self.intern_generic_parameter(canonical_id, parameter.name);
            let trait_bounds = resolved_bounds_by_local
                .get(&parameter.id)
                .cloned()
                .unwrap_or_default();

            self.trait_bounds_by_generic_parameter_id
                .insert(canonical_id, trait_bounds.clone());

            parameters.push(GenericParameter {
                id: canonical_id,
                name: parameter.name,
                trait_bounds,
            });
        }

        self.generic_parameter_lists
            .push(GenericParameterList { parameters });

        RegisteredGenericParameterList {
            list_id: id,
            canonical_by_local,
        }
    }

    /// Updates canonical bounds after trait definitions become available.
    ///
    /// WHAT: nominal structs/choices are registered before trait declarations are resolved so
    /// trait requirement signatures can refer to those nominal types. Their generic lists are
    /// patched once the trait environment has stable `TraitId`s.
    /// WHY: field/variant type resolution needs canonical generic parameter IDs early, while
    /// later instantiation checks need the declaration-site bounds on the same canonical list.
    pub(crate) fn update_generic_parameter_bounds(
        &mut self,
        list_id: GenericParameterListId,
        resolved_bounds_by_local: &FxHashMap<TypeParameterId, Vec<TraitId>>,
        canonical_by_local: &FxHashMap<TypeParameterId, GenericParameterId>,
    ) {
        let Some(list) = self.generic_parameter_lists.get_mut(list_id.0 as usize) else {
            return;
        };

        for (local_id, canonical_id) in canonical_by_local {
            let Some(parameter) = list
                .parameters
                .iter_mut()
                .find(|parameter| parameter.id == *canonical_id)
            else {
                continue;
            };

            let trait_bounds = resolved_bounds_by_local
                .get(local_id)
                .cloned()
                .unwrap_or_default();

            parameter.trait_bounds = trait_bounds.clone();
            self.trait_bounds_by_generic_parameter_id
                .insert(*canonical_id, trait_bounds);
        }
    }

    /// Registers one compiler-owned placeholder type parameter outside a parsed generic list.
    ///
    /// WHAT: creates a canonical `TypeId` for internal type variables such as trait `This`.
    /// WHY: trait requirement signatures need semantic `TypeId`s before any concrete implementor
    /// exists. Keeping the placeholder inside `TypeEnvironment` preserves the normal TypeId-first
    /// contract without representing trait declarations themselves as datatypes.
    pub(crate) fn register_synthetic_generic_parameter(&mut self, name: StringId) -> TypeId {
        let canonical_id = self.allocate_generic_parameter_id();
        self.intern_generic_parameter(canonical_id, name)
    }

    /// Returns the generic parameter list ID associated with a type, if any.
    ///
    /// WHAT: resolves struct, choice, and generic instance types to their parameter list.
    /// WHY: generic substitution needs to know which parameters the instance arguments replace.
    pub fn generic_parameter_list_id_for_type(
        &self,
        type_id: TypeId,
    ) -> Option<GenericParameterListId> {
        match self.get(type_id)? {
            TypeDefinition::Struct(def) => def.generic_parameters,
            TypeDefinition::Choice(def) => def.generic_parameters,
            TypeDefinition::GenericInstance(instance) => {
                match self.nominal_registry.get(instance.base.0 as usize)? {
                    NominalEntry::Struct(index) => {
                        self.struct_definitions.get(*index)?.generic_parameters
                    }
                    NominalEntry::Choice(index) => {
                        self.choice_definitions.get(*index)?.generic_parameters
                    }
                }
            }
            _ => None,
        }
    }

    // -----------------
    //  Substitution
    // -----------------

    /// Builds a mapping from generic parameter IDs to concrete argument TypeIds.
    fn build_parameter_mapping(
        &self,
        param_list_id: GenericParameterListId,
        arguments: &[TypeId],
    ) -> Option<FxHashMap<GenericParameterId, TypeId>> {
        let param_list = self.generic_parameters(param_list_id)?;
        if param_list.parameters.len() != arguments.len() {
            return None;
        }
        let mut mapping = FxHashMap::default();
        for (param, arg) in param_list.parameters.iter().zip(arguments.iter()) {
            mapping.insert(param.id, *arg);
        }
        Some(mapping)
    }

    /// Substitutes generic parameters inside a type using the given mapping.
    ///
    /// WHAT: recursively walks the type structure and replaces `GenericParameter` types
    ///       with their concrete arguments. Re-interns constructed/function/generic-instance
    ///       types so canonical identity is preserved.
    ///
    /// WHY: lazy field/variant substitution for generic instances needs concrete types.
    pub fn substitute_type_id(
        &mut self,
        type_id: TypeId,
        mapping: &FxHashMap<GenericParameterId, TypeId>,
    ) -> TypeId {
        increment_frontend_counter(FrontendCounter::TypeEnvironmentSubstituteTypeIdCalls);

        if mapping.is_empty() {
            return type_id;
        }

        if let Some(TypeDefinition::GenericParameter(param)) = self.get(type_id) {
            return mapping.get(&param.id).copied().unwrap_or(type_id);
        }

        let Some(source) = self.substitution_source_for(type_id) else {
            return type_id;
        };

        let cache_key = TypeSubstitutionKey::new(type_id, mapping);
        increment_frontend_counter(FrontendCounter::TypeEnvironmentSubstitutionCacheLookups);
        if let Some(substituted_type_id) = self.substitution_cache.get(&cache_key) {
            increment_frontend_counter(FrontendCounter::TypeEnvironmentSubstitutionCacheHits);
            return *substituted_type_id;
        }

        increment_frontend_counter(FrontendCounter::TypeEnvironmentSubstitutionCacheMisses);
        let substituted_type_id = match source {
            TypeSubstitutionSource::Constructed {
                constructor,
                arguments,
            } => self.substitute_constructed_type(constructor, &arguments, mapping),

            TypeSubstitutionSource::Function {
                parameters,
                returns,
                error_return,
            } => self.substitute_function_type(&parameters, &returns, error_return, mapping),

            TypeSubstitutionSource::GenericInstance { base, arguments } => {
                self.substitute_generic_instance_type(base, &arguments, mapping)
            }
        };

        self.substitution_cache
            .insert(cache_key, substituted_type_id);

        substituted_type_id
    }

    fn substitution_source_for(&self, type_id: TypeId) -> Option<TypeSubstitutionSource> {
        match self.get(type_id)? {
            TypeDefinition::Constructed(constructed) => Some(TypeSubstitutionSource::Constructed {
                constructor: constructed.constructor.clone(),
                arguments: constructed.arguments.clone(),
            }),

            TypeDefinition::Function(function) => {
                let parameters: Box<[TypeId]> = function
                    .parameters
                    .iter()
                    .map(|parameter| parameter.type_id)
                    .collect();

                Some(TypeSubstitutionSource::Function {
                    parameters,
                    returns: function.returns.clone(),
                    error_return: function.error_return,
                })
            }

            TypeDefinition::GenericInstance(instance) => {
                Some(TypeSubstitutionSource::GenericInstance {
                    base: instance.base,
                    arguments: instance.arguments.clone(),
                })
            }

            TypeDefinition::Builtin(..)
            | TypeDefinition::Struct(..)
            | TypeDefinition::Choice(..)
            | TypeDefinition::External(..)
            | TypeDefinition::GenericParameter(..) => None,
        }
    }

    fn substitute_constructed_type(
        &mut self,
        constructor: TypeConstructor,
        arguments: &[TypeId],
        mapping: &FxHashMap<GenericParameterId, TypeId>,
    ) -> TypeId {
        let substituted_arguments: Box<[TypeId]> = arguments
            .iter()
            .map(|argument| self.substitute_type_id(*argument, mapping))
            .collect();

        self.intern_constructed(constructor, substituted_arguments)
    }

    fn substitute_function_type(
        &mut self,
        parameters: &[TypeId],
        returns: &[TypeId],
        error_return: Option<TypeId>,
        mapping: &FxHashMap<GenericParameterId, TypeId>,
    ) -> TypeId {
        let substituted_parameters: Box<[TypeId]> = parameters
            .iter()
            .map(|parameter_type_id| self.substitute_type_id(*parameter_type_id, mapping))
            .collect();

        let substituted_returns: Box<[TypeId]> = returns
            .iter()
            .map(|return_type_id| self.substitute_type_id(*return_type_id, mapping))
            .collect();

        let substituted_error =
            error_return.map(|error_type_id| self.substitute_type_id(error_type_id, mapping));

        self.intern_function(FunctionTypeKey {
            parameters: substituted_parameters,
            returns: substituted_returns,
            error_return: substituted_error,
        })
    }

    fn substitute_generic_instance_type(
        &mut self,
        base: NominalTypeId,
        arguments: &[TypeId],
        mapping: &FxHashMap<GenericParameterId, TypeId>,
    ) -> TypeId {
        let substituted_arguments: Box<[TypeId]> = arguments
            .iter()
            .map(|argument| self.substitute_type_id(*argument, mapping))
            .collect();

        self.intern_generic_instance(base, substituted_arguments)
    }

    /// Eagerly computes and caches substituted fields/variants for a generic instance.
    ///
    /// WHAT: called automatically when a generic instance is interned. Pre-computes
    ///       the field and variant definitions with concrete types so that later
    ///       queries can use &self only.
    fn populate_generic_instance_substitutions(
        &mut self,
        instance_type_id: TypeId,
        base: NominalTypeId,
        arguments: &[TypeId],
    ) {
        // Struct: compute substituted fields.
        let struct_def = self.struct_definition(base).cloned();
        if let Some(struct_def) = struct_def {
            if let Some(param_list_id) = struct_def.generic_parameters
                && let Some(mapping) = self.build_parameter_mapping(param_list_id, arguments)
            {
                let mut substituted_fields = Vec::with_capacity(struct_def.fields.len());
                for field in struct_def.fields.iter() {
                    substituted_fields.push(FieldDefinition {
                        name: field.name.clone(),
                        type_id: self.substitute_type_id(field.type_id, &mapping),
                        location: field.location.clone(),
                    });
                }
                self.generic_instance_fields
                    .insert(instance_type_id, substituted_fields);
            }
            return;
        }

        // Choice: compute substituted variants.
        let choice_def = self.choice_definition(base).cloned();
        if let Some(choice_def) = choice_def
            && let Some(param_list_id) = choice_def.generic_parameters
            && let Some(mapping) = self.build_parameter_mapping(param_list_id, arguments)
        {
            let mut substituted_variants = Vec::with_capacity(choice_def.variants.len());
            for variant in choice_def.variants.iter() {
                let substituted_payload = match &variant.payload {
                    ChoiceVariantPayloadDefinition::Unit => ChoiceVariantPayloadDefinition::Unit,
                    ChoiceVariantPayloadDefinition::Record { fields } => {
                        let mut substituted_record_fields = Vec::with_capacity(fields.len());
                        for field in fields.iter() {
                            substituted_record_fields.push(FieldDefinition {
                                name: field.name.clone(),
                                type_id: self.substitute_type_id(field.type_id, &mapping),
                                location: field.location.clone(),
                            });
                        }
                        ChoiceVariantPayloadDefinition::Record {
                            fields: substituted_record_fields.into_boxed_slice(),
                        }
                    }
                };
                substituted_variants.push(ChoiceVariantDefinition {
                    name: variant.name,
                    tag: variant.tag,
                    payload: substituted_payload,
                    location: variant.location.clone(),
                });
            }
            self.generic_instance_variants
                .insert(instance_type_id, substituted_variants);
        }
    }

    /// Refreshes cached substituted definitions for every instance of one nominal type.
    ///
    /// WHAT: recomputes generic instance field/variant views after a nominal shell is patched
    /// with resolved fields or variants.
    /// WHY: instances may be interned while headers still carry unresolved shell data, and the
    /// cached substituted view must track the final canonical definition.
    fn refresh_generic_instance_substitutions_for_nominal(&mut self, base: NominalTypeId) {
        // Nominal patching can change the substituted field/variant views that
        // generic instances expose. The substituted TypeId usually stays the
        // same, but clearing keeps the cache contract tied to current nominal
        // definitions instead of relying on callers to reason about refresh order.
        self.substitution_cache.clear();

        let instances: Vec<(TypeId, Box<[TypeId]>)> = self
            .generic_instance_ids
            .iter()
            .filter(|(key, _)| key.base == base)
            .map(|(key, type_id)| (*type_id, key.arguments.clone()))
            .collect();

        for (instance_type_id, arguments) in instances {
            self.generic_instance_fields.remove(&instance_type_id);
            self.generic_instance_variants.remove(&instance_type_id);
            self.populate_generic_instance_substitutions(instance_type_id, base, &arguments);
        }
    }

    // --------------------------------------------------------
    //  Interning
    // --------------------------------------------------------

    /// Interns a constructed type, reusing an existing `TypeId` if the same
    /// constructor and arguments were already registered.
    pub fn intern_constructed(
        &mut self,
        constructor: TypeConstructor,
        arguments: Box<[TypeId]>,
    ) -> TypeId {
        let key = ConstructedTypeKey {
            constructor,
            arguments,
        };

        if let Some(&existing) = self.constructed_ids.get(&key) {
            return existing;
        }

        let id = self.insert_definition(TypeDefinition::Constructed(ConstructedTypeDefinition {
            constructor: key.constructor.clone(),
            arguments: key.arguments.clone(),
        }));

        self.constructed_ids.insert(key, id);
        id
    }

    /// Interns a function type.
    pub fn intern_function(&mut self, key: FunctionTypeKey) -> TypeId {
        if let Some(&existing) = self.function_ids.get(&key) {
            return existing;
        }

        let parameters: Box<[FunctionParameterDefinition]> = key
            .parameters
            .iter()
            .map(|&type_id| FunctionParameterDefinition {
                name: None,
                type_id,
            })
            .collect();

        let id = self.insert_definition(TypeDefinition::Function(FunctionTypeDefinition {
            parameters,
            returns: key.returns.clone(),
            error_return: key.error_return,
        }));

        self.function_ids.insert(key, id);
        id
    }

    #[cfg(test)]
    pub(crate) fn insert_function_type_for_test(
        &mut self,
        definition: FunctionTypeDefinition,
    ) -> TypeId {
        self.insert_definition(TypeDefinition::Function(definition))
    }

    /// Interns an opaque external type exposed by a backend package.
    ///
    /// WHAT: preserves frontend identity for external types instead of collapsing
    /// them to `None` during diagnostic-only `DataType -> TypeId` conversion.
    /// WHY: external types are opaque but still semantically distinct type identities.
    pub fn intern_external(&mut self, external_type_id: ExternalTypeId) -> TypeId {
        if let Some(&existing) = self.external_ids.get(&external_type_id) {
            return existing;
        }

        let id = self.insert_definition(TypeDefinition::External(ExternalTypeDefinition {
            type_id: external_type_id,
        }));

        self.external_ids.insert(external_type_id, id);
        id
    }

    /// Interns a generic nominal instance (e.g. `Box of Int`).
    pub fn intern_generic_instance(
        &mut self,
        base: NominalTypeId,
        arguments: Box<[TypeId]>,
    ) -> TypeId {
        let key = GenericInstanceKey {
            base,
            arguments: arguments.clone(),
        };

        if let Some(&existing) = self.generic_instance_ids.get(&key) {
            return existing;
        }

        let id =
            self.insert_definition(TypeDefinition::GenericInstance(GenericInstanceDefinition {
                base,
                arguments,
                source_key: key.clone(),
            }));

        self.generic_instance_ids.insert(key.clone(), id);

        // Eagerly compute substituted fields/variants so that later queries
        // can use &self only.
        let arguments_slice: &[TypeId] = &key.arguments;
        self.populate_generic_instance_substitutions(id, base, arguments_slice);

        id
    }

    // --------------------------------------------------------
    //  Nominal Registration
    // --------------------------------------------------------

    /// Registers a struct definition and returns both its allocated `NominalTypeId`
    /// and its canonical `TypeId`.
    pub fn register_nominal_struct(
        &mut self,
        mut definition: StructTypeDefinition,
    ) -> (NominalTypeId, TypeId) {
        let struct_index = self.struct_definitions.len();
        let nominal_id = NominalTypeId(self.nominal_registry.len() as u32);
        definition.id = nominal_id;
        let canonical_path = definition.path.clone();

        self.nominal_registry
            .push(NominalEntry::Struct(struct_index));
        self.struct_definitions.push(definition.clone());

        let type_id = self.insert_definition(TypeDefinition::Struct(definition));

        self.nominal_by_path.insert(canonical_path, nominal_id);
        self.nominal_to_type_id.insert(nominal_id, type_id);

        (nominal_id, type_id)
    }

    /// Updates the fields of an already-registered struct definition.
    ///
    /// WHAT: replaces the field list for a struct that was previously registered
    /// with identity and generic metadata only.
    /// WHY: early nominal registration must make recursive and generic names visible
    /// before AST field shells are fully resolved; this method writes the final
    /// canonical member definitions in-place once semantic `TypeId`s are known.
    pub fn update_struct_fields(&mut self, type_id: TypeId, fields: Box<[FieldDefinition]>) {
        let Some(TypeDefinition::Struct(def)) = self.get(type_id) else {
            return;
        };
        let nominal_id = def.id;
        let struct_index = match self.nominal_registry.get(nominal_id.0 as usize) {
            Some(NominalEntry::Struct(index)) => *index,
            _ => return,
        };
        if let Some(def) = self.struct_definitions.get_mut(struct_index) {
            def.fields = fields.clone();
        }
        // Also update the cached TypeDefinition so subsequent `get()` calls
        // see the resolved fields.
        if let Some(TypeDefinition::Struct(cached)) = self.types.get_mut(type_id.0 as usize) {
            cached.fields = fields;
        }

        self.refresh_generic_instance_substitutions_for_nominal(nominal_id);
    }

    /// Registers a choice definition and returns both its allocated `NominalTypeId`
    /// and its canonical `TypeId`.
    pub fn register_nominal_choice(
        &mut self,
        mut definition: ChoiceTypeDefinition,
    ) -> (NominalTypeId, TypeId) {
        let choice_index = self.choice_definitions.len();
        let nominal_id = NominalTypeId(self.nominal_registry.len() as u32);
        definition.id = nominal_id;
        let canonical_path = definition.path.clone();

        self.nominal_registry
            .push(NominalEntry::Choice(choice_index));
        self.choice_definitions.push(definition.clone());

        let type_id = self.insert_definition(TypeDefinition::Choice(definition));

        self.nominal_by_path.insert(canonical_path, nominal_id);
        self.nominal_to_type_id.insert(nominal_id, type_id);

        (nominal_id, type_id)
    }

    /// Updates the variants of an already-registered choice definition.
    ///
    /// WHAT: replaces the variant list for a choice registered during the identity pass.
    /// WHY: preserving the original choice definition keeps nominal identity and generic
    /// metadata stable while AST-owned variant shells resolve into final semantic payload
    /// definitions.
    pub fn update_choice_variants(
        &mut self,
        type_id: TypeId,
        variants: Box<[ChoiceVariantDefinition]>,
    ) {
        let Some(TypeDefinition::Choice(def)) = self.get(type_id) else {
            return;
        };
        let nominal_id = def.id;
        let choice_index = match self.nominal_registry.get(nominal_id.0 as usize) {
            Some(NominalEntry::Choice(index)) => *index,
            _ => return,
        };

        if let Some(def) = self.choice_definitions.get_mut(choice_index) {
            def.variants = variants.clone();
        }

        if let Some(TypeDefinition::Choice(cached)) = self.types.get_mut(type_id.0 as usize) {
            cached.variants = variants;
        }

        self.refresh_generic_instance_substitutions_for_nominal(nominal_id);
    }

    // --------------------------------------------------------
    //  Lookup
    // --------------------------------------------------------

    /// Returns the definition for a given `TypeId`, if it exists.
    pub fn get(&self, id: TypeId) -> Option<&TypeDefinition> {
        self.types.get(id.0 as usize)
    }

    /// Returns the high-level kind of the type.
    pub fn type_kind(&self, id: TypeId) -> Option<TypeKind> {
        self.get(id).map(|definition| match definition {
            TypeDefinition::Builtin(..) => TypeKind::Builtin,
            TypeDefinition::Struct(..) => TypeKind::Struct,
            TypeDefinition::Choice(..) => TypeKind::Choice,
            TypeDefinition::Constructed(..) => TypeKind::Constructed,
            TypeDefinition::Function(..) => TypeKind::Function,
            TypeDefinition::External(..) => TypeKind::External,
            TypeDefinition::GenericParameter(..) => TypeKind::GenericParameter,
            TypeDefinition::GenericInstance(..) => TypeKind::GenericInstance,
        })
    }

    /// Returns the nominal path registered for a `NominalTypeId`, if any.
    pub fn nominal_path_by_id(&self, id: NominalTypeId) -> Option<&InternedPath> {
        match self.nominal_registry.get(id.0 as usize)? {
            NominalEntry::Struct(index) => self.struct_definitions.get(*index).map(|s| &s.path),
            NominalEntry::Choice(index) => self.choice_definitions.get(*index).map(|c| &c.path),
        }
    }

    /// Returns the `NominalTypeId` for a path, if registered.
    pub fn nominal_id_for_path(&self, path: &InternedPath) -> Option<NominalTypeId> {
        self.nominal_by_path.get(path).copied()
    }

    /// Register an additional consumer-local lookup spelling for an existing nominal type.
    ///
    /// Imported declarations use a collision-free internal canonical path while source generic
    /// instantiation starts from the file-local imported spelling. Both paths must resolve to the
    /// same nominal identity; the alias is a lookup fact and does not create another type.
    pub(crate) fn register_nominal_path_alias(
        &mut self,
        path: InternedPath,
        type_id: TypeId,
    ) -> Result<(), CompilerError> {
        let nominal_id = match self.get(type_id) {
            Some(TypeDefinition::Struct(definition)) => definition.id,
            Some(TypeDefinition::Choice(definition)) => definition.id,
            _ => {
                return Err(CompilerError::compiler_error(
                    "Nominal path alias target is not a registered struct or choice",
                ));
            }
        };

        if let Some(existing) = self.nominal_by_path.get(&path)
            && *existing != nominal_id
        {
            return Err(CompilerError::compiler_error(
                "Nominal path alias collides with a different registered nominal type",
            ));
        }

        self.nominal_by_path.insert(path, nominal_id);
        Ok(())
    }

    /// Returns the `TypeId` for a nominal, if registered.
    pub fn type_id_for_nominal_id(&self, id: NominalTypeId) -> Option<TypeId> {
        self.nominal_to_type_id.get(&id).copied()
    }

    /// Registers one exact stable identity for a consumer-local type handle.
    pub(crate) fn register_canonical_identity(
        &mut self,
        identity: CanonicalTypeIdentity,
        type_id: TypeId,
    ) -> Result<(), CompilerError> {
        if let Some(existing_type_id) = self.canonical_type_ids.get(&identity)
            && *existing_type_id != type_id
        {
            return Err(CompilerError::compiler_error(format!(
                "Canonical type identity {identity:?} was assigned to both TypeId({}) and TypeId({})",
                existing_type_id.0, type_id.0
            )));
        }
        if let Some(existing_identity) = self.canonical_identities.get(&type_id)
            && existing_identity != &identity
        {
            return Err(CompilerError::compiler_error(format!(
                "TypeId({}) was assigned conflicting canonical identities {existing_identity:?} and {identity:?}",
                type_id.0
            )));
        }

        self.canonical_type_ids.insert(identity.clone(), type_id);
        self.canonical_identities.insert(type_id, identity);
        Ok(())
    }

    pub(crate) fn type_id_for_canonical_identity(
        &self,
        identity: &CanonicalTypeIdentity,
    ) -> Option<TypeId> {
        self.canonical_type_ids.get(identity).copied()
    }

    pub(crate) fn canonical_identity_for_type_id(
        &self,
        type_id: TypeId,
    ) -> Option<&CanonicalTypeIdentity> {
        self.canonical_identities.get(&type_id)
    }

    /// Iterate every exact stable identity paired with its environment-local handle.
    ///
    /// Materialisation blueprint extraction uses this canonical index rather than the
    /// source-spelling table, because interface closure may retain hidden imported nominals that
    /// deliberately have no consumer-local path.
    pub(crate) fn canonical_type_identities(
        &self,
    ) -> impl Iterator<Item = (&CanonicalTypeIdentity, TypeId)> {
        self.canonical_type_ids
            .iter()
            .map(|(identity, type_id)| (identity, *type_id))
    }

    /// Returns the struct definition for a nominal ID, if it is a struct.
    pub fn struct_definition(&self, id: NominalTypeId) -> Option<&StructTypeDefinition> {
        match self.nominal_registry.get(id.0 as usize)? {
            NominalEntry::Struct(index) => self.struct_definitions.get(*index),
            NominalEntry::Choice(..) => None,
        }
    }

    /// Returns the choice definition for a nominal ID, if it is a choice.
    pub fn choice_definition(&self, id: NominalTypeId) -> Option<&ChoiceTypeDefinition> {
        match self.nominal_registry.get(id.0 as usize)? {
            NominalEntry::Struct(..) => None,
            NominalEntry::Choice(index) => self.choice_definitions.get(*index),
        }
    }

    // --------------------------------------------------------
    //  Queries
    // --------------------------------------------------------

    /// Test-only query for canonical numeric classification fixtures.
    ///
    /// Decimal is intentionally excluded: it is seeded in the environment to keep
    /// stable builtin TypeId layout, but it is not an authorable or operator-active
    /// numeric type in the Alpha surface.
    #[cfg(test)]
    pub fn is_numeric(&self, id: TypeId) -> bool {
        matches!(
            self.get(id),
            Some(TypeDefinition::Builtin(builtin)) if matches!(
                builtin.key,
                BuiltinTypeKey::Int | BuiltinTypeKey::Float
            )
        )
    }

    /// Returns true if the type is a collection.
    pub fn is_collection(&self, id: TypeId) -> bool {
        self.collection_shape(id).is_some()
    }

    /// Returns the element type of a collection, if any.
    pub fn collection_element_type(&self, id: TypeId) -> Option<TypeId> {
        self.collection_shape(id).map(|shape| shape.element_type)
    }

    /// Returns the full shape of a collection type, if this type is a collection.
    pub(crate) fn collection_shape(&self, id: TypeId) -> Option<CollectionShape> {
        match self.get(id) {
            Some(TypeDefinition::Constructed(constructed)) => match &constructed.constructor {
                TypeConstructor::Builtin(BuiltinTypeConstructor::Collection { fixed_capacity }) => {
                    Some(CollectionShape {
                        element_type: *constructed.arguments.first()?,
                        fixed_capacity: *fixed_capacity,
                    })
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Returns the fixed capacity of a collection, if this type is a fixed collection.
    pub(crate) fn collection_fixed_capacity(&self, id: TypeId) -> Option<usize> {
        self.collection_shape(id)
            .and_then(|shape| shape.fixed_capacity)
    }

    /// Interns the canonical built-in collection type for an element semantic type.
    ///
    /// WHAT: gives collection construction a named owner in `TypeEnvironment`.
    /// WHY: collection identity is semantic `TypeId` identity; parser-shaped
    ///      `DataType::collection` must remain a diagnostic spelling instead of the
    ///      canonical representation.
    pub fn intern_collection(
        &mut self,
        element_type: TypeId,
        fixed_capacity: Option<usize>,
    ) -> TypeId {
        self.intern_constructed(
            TypeConstructor::Builtin(BuiltinTypeConstructor::Collection { fixed_capacity }),
            Box::new([element_type]),
        )
    }

    /// Interns the canonical ordered map type for key and value semantic types.
    ///
    /// WHAT: gives map construction a named owner in `TypeEnvironment`.
    /// WHY: map identity is semantic `TypeId` identity; parser-shaped `DataType::Map`
    ///      must remain a diagnostic spelling instead of the canonical representation.
    pub fn intern_map(&mut self, key_type: TypeId, value_type: TypeId) -> TypeId {
        self.intern_constructed(
            TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap),
            Box::new([key_type, value_type]),
        )
    }

    /// Returns true if the type is an ordered map.
    pub fn is_map_type(&self, id: TypeId) -> bool {
        self.map_shape(id).is_some()
    }

    /// Test-only query for a canonical map key type.
    #[cfg(test)]
    pub fn map_key_type(&self, id: TypeId) -> Option<TypeId> {
        self.map_shape(id).map(|shape| shape.key_type)
    }

    /// Test-only query for a canonical map value type.
    #[cfg(test)]
    pub fn map_value_type(&self, id: TypeId) -> Option<TypeId> {
        self.map_shape(id).map(|shape| shape.value_type)
    }

    /// Returns the full shape of a map type, if this type is a map.
    pub(crate) fn map_shape(&self, id: TypeId) -> Option<MapShape> {
        match self.get(id) {
            Some(TypeDefinition::Constructed(constructed)) => match &constructed.constructor {
                TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap) => {
                    if let [key_type, value_type] = constructed.arguments.as_ref() {
                        Some(MapShape {
                            key_type: *key_type,
                            value_type: *value_type,
                        })
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Interns the canonical built-in option type for an inner semantic type.
    ///
    /// WHAT: gives option construction a named owner in `TypeEnvironment`.
    /// WHY: option identity is semantic `TypeId` identity; parser-shaped `DataType::Option`
    ///      must remain a diagnostic spelling instead of the canonical representation.
    pub fn intern_option(&mut self, inner: TypeId) -> TypeId {
        self.intern_constructed(
            TypeConstructor::Builtin(BuiltinTypeConstructor::Option),
            Box::new([inner]),
        )
    }

    /// Returns true if the type is an option.
    pub fn is_option(&self, id: TypeId) -> bool {
        self.option_inner_type(id).is_some()
    }

    /// Returns the inner type of an option, if any.
    pub fn option_inner_type(&self, id: TypeId) -> Option<TypeId> {
        match self.get(id) {
            Some(TypeDefinition::Constructed(constructed)) => match &constructed.constructor {
                TypeConstructor::Builtin(BuiltinTypeConstructor::Option) => {
                    constructed.arguments.first().copied()
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Interns a tuple type with the given field types.
    ///
    /// WHAT: creates a canonical `TypeId` for a tuple/multi-return type.
    /// WHY: HIR needs to represent multi-return values as a single `TypeId` for
    ///      `HirExpression.ty`, `HirLocal.ty`, and `HirFunction.return_type`.
    pub fn intern_tuple(&mut self, fields: Vec<TypeId>) -> TypeId {
        self.intern_constructed(
            TypeConstructor::Builtin(BuiltinTypeConstructor::Tuple),
            fields.into_boxed_slice(),
        )
    }

    /// Returns the field types of a tuple, if the type is a tuple.
    pub fn tuple_field_ids(&self, id: TypeId) -> Option<&[TypeId]> {
        match self.get(id) {
            Some(TypeDefinition::Constructed(constructed)) => match &constructed.constructor {
                TypeConstructor::Builtin(BuiltinTypeConstructor::Tuple) => {
                    Some(constructed.arguments.as_ref())
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Returns true if values of this type can be compared with runtime equality.
    ///
    /// WHAT: scalar equality and choice payload equality are frontend semantic facts.
    /// WHY: comparison typing needs one canonical query over `TypeId + TypeEnvironment`
    ///      instead of reconstructing parse-only `DataType` trees.
    pub fn supports_runtime_equality(&self, id: TypeId) -> bool {
        self.supports_runtime_equality_with_visited(id, &mut FxHashSet::default())
    }

    fn supports_runtime_equality_with_visited(
        &self,
        id: TypeId,
        visited_choices: &mut FxHashSet<TypeId>,
    ) -> bool {
        match self.get(id) {
            Some(TypeDefinition::Builtin(builtin)) => matches!(
                builtin.key,
                BuiltinTypeKey::Bool
                    | BuiltinTypeKey::Int
                    | BuiltinTypeKey::Float
                    | BuiltinTypeKey::Char
                    | BuiltinTypeKey::String
            ),

            Some(TypeDefinition::Choice(..)) | Some(TypeDefinition::GenericInstance(..)) => {
                self.choice_payloads_support_runtime_equality(id, visited_choices)
            }

            Some(TypeDefinition::Struct(..))
            | Some(TypeDefinition::Constructed(..))
            | Some(TypeDefinition::Function(..))
            | Some(TypeDefinition::External(..))
            | Some(TypeDefinition::GenericParameter(..))
            | None => false,
        }
    }

    fn choice_payloads_support_runtime_equality(
        &self,
        id: TypeId,
        visited_choices: &mut FxHashSet<TypeId>,
    ) -> bool {
        if !visited_choices.insert(id) {
            return false;
        }

        let Some(variants) = self.variants_for(id) else {
            visited_choices.remove(&id);
            return false;
        };

        for variant in variants {
            let ChoiceVariantPayloadDefinition::Record { fields } = &variant.payload else {
                continue;
            };

            for field in fields {
                if !self.supports_runtime_equality_with_visited(field.type_id, visited_choices) {
                    visited_choices.remove(&id);
                    return false;
                }
            }
        }

        visited_choices.remove(&id);
        true
    }

    /// Returns the `TypeId` for a generic parameter, if already registered.
    pub fn type_id_for_generic_parameter(&self, id: GenericParameterId) -> Option<TypeId> {
        self.generic_parameter_ids.get(&id).copied()
    }

    /// Interns a generic parameter as a type identity.
    pub fn intern_generic_parameter(&mut self, id: GenericParameterId, name: StringId) -> TypeId {
        if let Some(&existing) = self.generic_parameter_ids.get(&id) {
            return existing;
        }
        let type_id = self.insert_definition(TypeDefinition::GenericParameter(
            GenericParameterDefinition { id, name },
        ));
        self.generic_parameter_ids.insert(id, type_id);
        type_id
    }

    fn allocate_generic_parameter_id(&mut self) -> GenericParameterId {
        let id = GenericParameterId(self.next_generic_parameter_id);
        self.next_generic_parameter_id += 1;
        id
    }

    // --------------------------------------------------------
    //  Nominal Definition Queries
    // --------------------------------------------------------

    /// Returns the struct definition for a `TypeId`, if it points to a struct.
    pub fn struct_definition_for(&self, id: TypeId) -> Option<&StructTypeDefinition> {
        match self.get(id)? {
            TypeDefinition::Struct(def) => Some(def),
            _ => None,
        }
    }

    /// Returns the choice definition for a `TypeId`, if it points to a choice.
    pub fn choice_definition_for(&self, id: TypeId) -> Option<&ChoiceTypeDefinition> {
        match self.get(id)? {
            TypeDefinition::Choice(def) => Some(def),
            _ => None,
        }
    }

    /// Returns the borrowed fields of a struct or generic instance type, if any.
    ///
    /// WHAT: for base structs returns the canonical fields; for generic instances
    ///       returns the eagerly-substituted fields cached at interning time.
    /// WHY: AST and HIR hot paths should read semantic field facts without
    ///      cloning declaration-sized payloads on every member lookup.
    pub fn fields_for(&self, id: TypeId) -> Option<&[FieldDefinition]> {
        increment_frontend_counter(FrontendCounter::TypeEnvironmentFieldsForQueries);

        // Generic instances carry pre-substituted field views keyed by their canonical TypeId.
        if let Some(cached) = self.generic_instance_fields.get(&id) {
            add_frontend_counter(FrontendCounter::TypeEnvironmentFieldsReturned, cached.len());
            return Some(cached.as_slice());
        }

        // Fallback to base struct definition.
        let fields = self
            .struct_definition_for(id)
            .map(|def| def.fields.as_ref());

        if let Some(fields) = fields {
            add_frontend_counter(FrontendCounter::TypeEnvironmentFieldsReturned, fields.len());
        }

        fields
    }

    /// Returns one borrowed field definition by source-level field name.
    ///
    /// WHAT: this is the direct lookup path for member access and constructor
    /// inference. It keeps lookup ownership inside `TypeEnvironment` instead of
    /// making callers clone field lists and search them locally.
    pub fn field_for(&self, type_id: TypeId, field_name: StringId) -> Option<&FieldDefinition> {
        self.fields_for(type_id)?
            .iter()
            .find(|field| field.name.name() == Some(field_name))
    }

    /// Returns the borrowed variants of a choice or generic instance type, if any.
    ///
    /// WHAT: for base choices returns the canonical variants; for generic instances
    ///       returns the eagerly-substituted variants cached at interning time.
    /// WHY: choice matching, equality, and HIR lowering inspect variants often;
    ///      semantic lookup should borrow the canonical environment payload.
    pub fn variants_for(&self, id: TypeId) -> Option<&[ChoiceVariantDefinition]> {
        increment_frontend_counter(FrontendCounter::TypeEnvironmentVariantsForQueries);

        // Generic instances carry pre-substituted variant views keyed by their canonical TypeId.
        if let Some(cached) = self.generic_instance_variants.get(&id) {
            add_frontend_counter(
                FrontendCounter::TypeEnvironmentVariantsReturned,
                cached.len(),
            );
            return Some(cached.as_slice());
        }

        // Fallback to base choice definition.
        let variants = self
            .choice_definition_for(id)
            .map(|def| def.variants.as_ref());

        if let Some(variants) = variants {
            add_frontend_counter(
                FrontendCounter::TypeEnvironmentVariantsReturned,
                variants.len(),
            );
        }

        variants
    }

    /// Returns one borrowed choice variant for environment fixtures.
    ///
    /// WHAT: centralizes direct variant lookup over base and generic choice
    /// instances so callers do not need a clone-returning query surface.
    #[cfg(test)]
    pub fn variant_for(
        &self,
        type_id: TypeId,
        variant_name: StringId,
    ) -> Option<&ChoiceVariantDefinition> {
        self.variants_for(type_id)?
            .iter()
            .find(|variant| variant.name == variant_name)
    }

    /// Returns true if the type is a const record struct.
    pub fn is_const_record(&self, id: TypeId) -> bool {
        matches!(
            self.get(id),
            Some(TypeDefinition::Struct(def)) if def.const_record
        )
    }

    /// Returns the nominal path for a type, if it has one.
    ///
    /// WHAT: extracts the user-declared path for struct, choice, and generic instance types.
    /// WHY: receiver key derivation and diagnostics need the nominal identity.
    pub fn nominal_path(&self, id: TypeId) -> Option<&InternedPath> {
        match self.get(id)? {
            TypeDefinition::Struct(def) => Some(&def.path),
            TypeDefinition::Choice(def) => Some(&def.path),
            TypeDefinition::GenericInstance(def) => self.nominal_path_by_id(def.base),
            _ => None,
        }
    }

    /// Returns the generic instance key for a generic instance type.
    ///
    /// WHAT: reverse lookup from a generic instance TypeId to its canonical key.
    /// WHY: generic nominal inference and HIR lowering need the instance key to
    ///      resolve cached generic struct/choice IDs.
    pub fn generic_instance_key(&self, id: TypeId) -> Option<&GenericInstanceKey> {
        match self.get(id)? {
            TypeDefinition::GenericInstance(def) => Some(&def.source_key),
            _ => None,
        }
    }

    /// Returns the receiver-method lookup key for a canonical frontend type.
    ///
    /// WHAT: derives receiver keys from `TypeId` and the environment-owned type
    /// definition instead of parse-only `DataType` payloads.
    /// WHY: AST member lookup should use canonical semantic type identity on the
    /// hot path. Generic nominal instances resolve to their base constructor key
    /// so the receiver catalog can store one declaration-site method for every
    /// concrete instance of that constructor.
    pub fn receiver_key_for_type_id(&self, id: TypeId) -> Option<ReceiverKey> {
        match self.get(id)? {
            TypeDefinition::Builtin(builtin) => match builtin.key {
                BuiltinTypeKey::Int => Some(ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::Int)),
                BuiltinTypeKey::Float => {
                    Some(ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::Float))
                }
                BuiltinTypeKey::Bool => {
                    Some(ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::Bool))
                }
                BuiltinTypeKey::String => {
                    Some(ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::String))
                }
                BuiltinTypeKey::Char => {
                    Some(ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::Char))
                }
                BuiltinTypeKey::Decimal | BuiltinTypeKey::Range | BuiltinTypeKey::None => None,
            },

            TypeDefinition::Struct(definition) => {
                Some(ReceiverKey::Struct(definition.path.clone()))
            }

            TypeDefinition::Choice(definition) => {
                Some(ReceiverKey::Choice(definition.path.clone()))
            }

            TypeDefinition::GenericInstance(instance) => {
                let base_type_id = self.type_id_for_nominal_id(instance.base)?;
                match self.get(base_type_id)? {
                    TypeDefinition::Struct(base_definition) => {
                        Some(ReceiverKey::Struct(base_definition.path.clone()))
                    }

                    TypeDefinition::Choice(base_definition) => {
                        Some(ReceiverKey::Choice(base_definition.path.clone()))
                    }

                    _ => None,
                }
            }

            TypeDefinition::External(definition) => Some(ReceiverKey::External(definition.type_id)),

            TypeDefinition::Constructed(..)
            | TypeDefinition::Function(..)
            | TypeDefinition::GenericParameter(..) => None,
        }
    }

    /// Converts a `TypeId` to a `TypeIdentityKey` for HIR compatibility.
    ///
    /// WHAT: reverse-maps a canonical `TypeId` to the identity key used by
    ///       `GenericInstantiationKey` and HIR generic struct/choice registration.
    /// WHY: eliminates the `TypeId -> DataType -> TypeIdentityKey` double bridge.
    pub fn type_id_to_type_identity_key(&self, id: TypeId) -> Option<TypeIdentityKey> {
        match self.get(id)? {
            TypeDefinition::Builtin(builtin) => match builtin.key {
                BuiltinTypeKey::Bool => Some(TypeIdentityKey::Builtin(BridgeBuiltinTypeKey::Bool)),
                BuiltinTypeKey::Int => Some(TypeIdentityKey::Builtin(BridgeBuiltinTypeKey::Int)),
                BuiltinTypeKey::Float => {
                    Some(TypeIdentityKey::Builtin(BridgeBuiltinTypeKey::Float))
                }
                BuiltinTypeKey::Decimal => {
                    Some(TypeIdentityKey::Builtin(BridgeBuiltinTypeKey::Decimal))
                }
                BuiltinTypeKey::String => {
                    Some(TypeIdentityKey::Builtin(BridgeBuiltinTypeKey::String))
                }
                BuiltinTypeKey::Char => Some(TypeIdentityKey::Builtin(BridgeBuiltinTypeKey::Char)),
                BuiltinTypeKey::Range => {
                    Some(TypeIdentityKey::Builtin(BridgeBuiltinTypeKey::Range))
                }
                BuiltinTypeKey::None => None,
            },
            TypeDefinition::Struct(def) => Some(TypeIdentityKey::Nominal(def.path.clone())),
            TypeDefinition::Choice(def) => Some(TypeIdentityKey::Nominal(def.path.clone())),
            TypeDefinition::Constructed(constructed) => match constructed.constructor {
                TypeConstructor::Builtin(BuiltinTypeConstructor::Collection { fixed_capacity }) => {
                    let element_id = constructed.arguments.first()?;
                    Some(TypeIdentityKey::Collection {
                        element: Box::new(self.type_id_to_type_identity_key(*element_id)?),
                        fixed_capacity,
                    })
                }
                TypeConstructor::Builtin(BuiltinTypeConstructor::Option) => {
                    let inner_id = constructed.arguments.first()?;
                    Some(TypeIdentityKey::Option(Box::new(
                        self.type_id_to_type_identity_key(*inner_id)?,
                    )))
                }
                TypeConstructor::Builtin(BuiltinTypeConstructor::FallibleCarrier) => {
                    let success_id = constructed.arguments.first()?;
                    let error_id = constructed.arguments.get(1)?;
                    Some(TypeIdentityKey::FallibleCarrier {
                        success: Box::new(self.type_id_to_type_identity_key(*success_id)?),
                        error: Box::new(self.type_id_to_type_identity_key(*error_id)?),
                    })
                }
                TypeConstructor::Builtin(BuiltinTypeConstructor::OrderedMap) => {
                    let key_id = constructed.arguments.first()?;
                    let value_id = constructed.arguments.get(1)?;
                    Some(TypeIdentityKey::Map {
                        key: Box::new(self.type_id_to_type_identity_key(*key_id)?),
                        value: Box::new(self.type_id_to_type_identity_key(*value_id)?),
                    })
                }
                TypeConstructor::Builtin(BuiltinTypeConstructor::Tuple) => None,
            },
            TypeDefinition::GenericInstance(instance) => {
                let base_path = self.nominal_path_by_id(instance.base)?.clone();
                let arguments = instance
                    .arguments
                    .iter()
                    .map(|arg| self.type_id_to_type_identity_key(*arg))
                    .collect::<Option<Vec<_>>>()?;
                Some(TypeIdentityKey::GenericInstance(GenericInstantiationKey {
                    base_path,
                    arguments,
                }))
            }
            TypeDefinition::External(ext) => Some(TypeIdentityKey::External(ext.type_id)),
            TypeDefinition::Function(_) | TypeDefinition::GenericParameter(_) => None,
        }
    }

    /// Unifies a template type (containing generic parameters) with a concrete type,
    /// collecting parameter-to-argument bindings.
    ///
    /// WHAT: recursively walks two `TypeId` trees and records which concrete `TypeId`
    ///       each generic parameter must map to, preserving binding-conflict facts.
    /// WHY: generic function and nominal constructor inference need to distinguish
    ///      structural mismatches from a repeated `T` being inferred as two different
    ///      concrete types.
    pub(crate) fn try_collect_type_parameter_bindings_typeid(
        &self,
        template_type_id: TypeId,
        concrete_type_id: TypeId,
        bindings: &mut GenericTypeBindings,
    ) -> Result<bool, BindingConflict> {
        // WHAT: stage every binding produced by one complete structural walk in a clone
        //       and commit it into the caller's map only when the walk reports a match.
        // WHY: a structural mismatch (`Ok(false)`) or a binding conflict (`Err`) partway
        //      through a nested constructed/generic-instance walk must not leave partial
        //      bindings behind, since those could poison a later constraint or turn a
        //      mismatch into a spurious binding-conflict diagnostic.
        let mut staged = bindings.clone();
        match self.collect_type_parameter_bindings_inner(
            template_type_id,
            concrete_type_id,
            &mut staged,
        ) {
            Ok(true) => {
                *bindings = staged;
                Ok(true)
            }
            Ok(false) => Ok(false),
            Err(conflict) => Err(conflict),
        }
    }

    /// Recursive structural walker that mutates only the staged binding map.
    ///
    /// WHAT: walks template and concrete `TypeId` trees in lock-step, recording each
    ///       generic-parameter binding in `staged` and returning whether the structure
    ///       matched.
    /// WHY: keeping recursion on a private helper means nested arguments share one
    ///      staged transaction instead of cloning and committing independently at
    ///      every level, so a deep mismatch rolls back the whole walk atomically.
    fn collect_type_parameter_bindings_inner(
        &self,
        template_type_id: TypeId,
        concrete_type_id: TypeId,
        staged: &mut GenericTypeBindings,
    ) -> Result<bool, BindingConflict> {
        // If template is a generic parameter, record the binding.
        if let Some(TypeDefinition::GenericParameter(param)) = self.get(template_type_id) {
            if concrete_type_id == self.builtins().none {
                return Ok(false); // Inferred/unknown — don't bind
            }
            return staged
                .insert_consistent(param.id, concrete_type_id)
                .map(|()| true);
        }

        if template_type_id == concrete_type_id {
            return Ok(true);
        }

        let matched = match (self.get(template_type_id), self.get(concrete_type_id)) {
            (
                Some(TypeDefinition::Constructed(template_def)),
                Some(TypeDefinition::Constructed(concrete_def)),
            ) if template_def.constructor == concrete_def.constructor
                && template_def.arguments.len() == concrete_def.arguments.len() =>
            {
                for (template_argument, concrete_argument) in template_def
                    .arguments
                    .iter()
                    .zip(concrete_def.arguments.iter())
                {
                    if !self.collect_type_parameter_bindings_inner(
                        *template_argument,
                        *concrete_argument,
                        staged,
                    )? {
                        return Ok(false);
                    }
                }

                true
            }
            (
                Some(TypeDefinition::GenericInstance(template_inst)),
                Some(TypeDefinition::GenericInstance(concrete_inst)),
            ) if template_inst.base == concrete_inst.base
                && template_inst.arguments.len() == concrete_inst.arguments.len() =>
            {
                for (template_argument, concrete_argument) in template_inst
                    .arguments
                    .iter()
                    .zip(concrete_inst.arguments.iter())
                {
                    if !self.collect_type_parameter_bindings_inner(
                        *template_argument,
                        *concrete_argument,
                        staged,
                    )? {
                        return Ok(false);
                    }
                }

                true
            }
            _ => false,
        };

        Ok(matched)
    }

    // --------------------------------------------------------
    //  Private Helpers
    // --------------------------------------------------------

    fn insert_definition(&mut self, definition: TypeDefinition) -> TypeId {
        let id = TypeId(self.types.len() as u32);
        self.types.push(definition);
        id
    }

    fn insert_builtin(&mut self, key: BuiltinTypeKey) -> TypeId {
        let id = self.insert_definition(TypeDefinition::Builtin(BuiltinTypeDefinition { key }));
        self.builtin_ids.insert(key, id);
        id
    }

    fn remap_type_definition(definition: &mut TypeDefinition, remap: &StringIdRemap) {
        match definition {
            TypeDefinition::Struct(definition) => {
                Self::remap_struct_definition(definition, remap);
            }
            TypeDefinition::Choice(definition) => {
                Self::remap_choice_definition(definition, remap);
            }
            TypeDefinition::Function(definition) => {
                for parameter in &mut definition.parameters {
                    if let Some(name) = &mut parameter.name {
                        *name = remap.get(*name);
                    }
                }
            }
            TypeDefinition::GenericParameter(definition) => {
                definition.name = remap.get(definition.name);
            }

            TypeDefinition::Builtin(..)
            | TypeDefinition::Constructed(..)
            | TypeDefinition::External(..)
            | TypeDefinition::GenericInstance(..) => {}
        }
    }

    fn remap_struct_definition(definition: &mut StructTypeDefinition, remap: &StringIdRemap) {
        definition.path.remap_string_ids(remap);
        Self::remap_fields(definition.fields.as_mut(), remap);
    }

    fn remap_choice_definition(definition: &mut ChoiceTypeDefinition, remap: &StringIdRemap) {
        definition.path.remap_string_ids(remap);
        Self::remap_variants(definition.variants.as_mut(), remap);
    }

    fn remap_fields(fields: &mut [FieldDefinition], remap: &StringIdRemap) {
        for field in fields {
            field.name.remap_string_ids(remap);
            field.location.remap_string_ids(remap);
        }
    }

    fn remap_variants(variants: &mut [ChoiceVariantDefinition], remap: &StringIdRemap) {
        for variant in variants {
            variant.name = remap.get(variant.name);
            variant.location.remap_string_ids(remap);

            match &mut variant.payload {
                ChoiceVariantPayloadDefinition::Unit => {}
                ChoiceVariantPayloadDefinition::Record { fields } => {
                    Self::remap_fields(fields.as_mut(), remap);
                }
            }
        }
    }

    fn rebuild_nominal_path_index(&mut self) {
        self.nominal_by_path.clear();

        for definition in &self.struct_definitions {
            self.nominal_by_path
                .insert(definition.path.clone(), definition.id);
        }

        for definition in &self.choice_definitions {
            self.nominal_by_path
                .insert(definition.path.clone(), definition.id);
        }
    }
}
