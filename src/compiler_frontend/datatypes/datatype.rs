//! The `DataType` enum and its intrinsic methods.
//!
//! WHAT: carries parsed and diagnostic-only type spellings used before and during AST
//!       construction. Contains constructors, queries, remapping and the reverse bridge
//!       from canonical `TypeId` back to diagnostic spellings.
//! WHY: separating the data shape from `mod.rs` keeps the module entry point focused
//!      on orchestration and re-exports.
//!
//! NOTE: `DataType` owns type structure and structural helper methods only.
//! Compatibility policy lives in `type_coercion::compatibility`.
//! Contextual promotion lives in `type_coercion::contextual`.

use crate::compiler_frontend::external_packages::ExternalTypeId;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;

use super::definitions::TypeDefinition;
use super::environment::TypeEnvironment;
use super::generic_identity_bridge::{
    BuiltinGenericType, GenericBaseType, GenericInstantiationKey,
};
use super::generic_parameters::TypeParameterId;
use super::ids::{self, GenericParameterId, TypeId};
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

#[derive(Debug, Clone)]
pub enum DataType {
    // Meta-types used during earlier frontend stages.
    // Type is inferred, This only exists before the type checking stage.
    // All 'inferred' variables must be evaluated to other types after the AST stage for the program to compile.
    // At the header parsing stage, 'inferred' is used where a symbol type is not yet known (as the type might be another header).
    Inferred,
    // AST-only placeholder for nominal types that are resolved once the module declaration set is known.
    NamedType(StringId),
    // AST-only placeholder for namespace-qualified types (e.g. `canvas.Canvas2d`).
    // Resolved during type checking against visible namespace records.
    NamespacedType {
        /// Unresolved dotted path components such as `["canvas", "Canvas2d"]`.
        ///
        /// WHAT: carries the full namespace path before resolution so type checking can
        ///      walk child namespace records for multi-segment paths.
        /// WHY: external package surfaces support nested namespaces; a single namespace
        ///      plus name pair is no longer enough for `io.input.Input`-style paths.
        path: Vec<StringId>,
    },
    TypeParameter {
        id: TypeParameterId,
        canonical_id: Option<GenericParameterId>,
        name: StringId,
    },
    GenericInstance {
        base: GenericBaseType,
        arguments: Vec<DataType>,
    },

    // Container and composite runtime types.
    Struct {
        nominal_path: PathId,
        type_id: TypeId,
        /// Diagnostic/render-only marker. Semantic const-record decisions must use
        /// `TypeEnvironment::is_const_record` or `ExpressionView::is_const_record_value`,
        /// not this field.
        const_record: bool,
        generic_instance_key: Option<GenericInstantiationKey>,
    },
    Range, // Iterable that must always be owned.

    // Scalar/runtime-leaf types.
    Bool,
    /// Diagnostic spelling for one arbitrary-precision `Dec` scale identity.
    ///
    /// `Dec` and `Dec0` share scale zero; `Dec1`..`Dec256` carry their scale.
    /// Semantic identity stays in `TypeEnvironment`; this spelling only mirrors it.
    Number(NumberScale),
    StringSlice, // UTF-8 read-only string slice
    Char,
    /// Explicit-width builtin scalar or the `Byte` octet type.
    ///
    /// `Byte` is distinct from `U8`, `I8` and `Char`.
    FixedScalar(FixedScalar),

    // Reserved or not-yet-wired variants kept for planned language work.
    Choices {
        nominal_path: PathId,
        type_id: TypeId,
        generic_instance_key: Option<GenericInstantiationKey>,
    }, // Choice declaration identity + variant list
    /// Opaque external type provided by a platform package (e.g. `Canvas`).
    /// Cannot be constructed with struct literals or field-accessed.
    External {
        type_id: ExternalTypeId,
    },

    /// Parse/diagnostic spelling for built-in options.
    ///
    /// Semantic option identity is owned by `TypeEnvironment::intern_option`.
    /// AST/HIR type checks should use `TypeId` queries instead of this variant.
    Option(Box<DataType>), // Shorthand for a choice of a type or None
    #[allow(dead_code)] // Planned: explicit None literal/type flows.
    None, // The None result of an option, or empty argument
    #[allow(dead_code)] // Planned: boolean literal singleton typing extensions.
    True,
    #[allow(dead_code)] // Planned: boolean literal singleton typing extensions.
    False,
}

// NOTE: DataType owns type structure and structural helper methods only.
// Compatibility policy (what type is accepted in what position) lives exclusively
// in `type_coercion::compatibility::is_type_compatible`.
// Contextual promotion logic lives in `type_coercion::contextual`.
impl DataType {
    // -----------------
    //  Constructors
    // -----------------

    pub fn runtime_struct(nominal_path: PathId, type_id: TypeId) -> Self {
        Self::runtime_struct_with_generic_key(nominal_path, type_id, None)
    }

    pub fn runtime_struct_with_generic_key(
        nominal_path: PathId,
        type_id: TypeId,
        generic_instance_key: Option<GenericInstantiationKey>,
    ) -> Self {
        Self::Struct {
            nominal_path,
            type_id,
            const_record: false,
            generic_instance_key,
        }
    }

    // -----------------
    //  Queries
    // -----------------

    /// Constructs a growable collection type using the canonical generic instance representation.
    ///
    /// WHAT: `DataType::Collection` is being removed; this is the one canonical constructor.
    /// WHY: keeps collection construction readable while unifying on generic infrastructure.
    pub fn collection(element_type: DataType) -> Self {
        Self::GenericInstance {
            base: GenericBaseType::Builtin(BuiltinGenericType::Collection {
                fixed_capacity: None,
            }),
            arguments: vec![element_type],
        }
    }

    /// Constructs a fixed-collection diagnostic spelling.
    ///
    /// WHAT: narrow display helper for sites that need a non-authoritative
    ///      `DataType` for a fixed collection.
    /// WHY: makes fixed-collection diagnostic spelling explicit without changing
    ///      the growable constructor.
    pub fn fixed_collection(element_type: DataType, capacity: usize) -> Self {
        Self::GenericInstance {
            base: GenericBaseType::Builtin(BuiltinGenericType::Collection {
                fixed_capacity: Some(capacity),
            }),
            arguments: vec![element_type],
        }
    }

    /// Constructs an ordered map diagnostic spelling.
    ///
    /// WHAT: `DataType::Map` is represented through the canonical generic instance
    ///      representation to stay consistent with collections.
    /// WHY: map identity is semantic `TypeId` identity; this constructor keeps
    ///      diagnostic spellings aligned with the canonical environment shape.
    pub fn map(key_type: DataType, value_type: DataType) -> Self {
        Self::GenericInstance {
            base: GenericBaseType::Builtin(BuiltinGenericType::Map),
            arguments: vec![key_type, value_type],
        }
    }
}

// -----------------------------------------------------------
//  Structural Equality
// -----------------------------------------------------------

impl PartialEq for DataType {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (DataType::Inferred, DataType::Inferred) => true,
            (DataType::NamedType(a), DataType::NamedType(b)) => a == b,
            (
                DataType::NamespacedType { path: path_a },
                DataType::NamespacedType { path: path_b },
            ) => path_a == path_b,
            (
                DataType::TypeParameter {
                    id: id_a,
                    canonical_id: canonical_id_a,
                    name: name_a,
                },
                DataType::TypeParameter {
                    id: id_b,
                    canonical_id: canonical_id_b,
                    name: name_b,
                },
            ) => {
                if let (Some(canonical_id_a), Some(canonical_id_b)) =
                    (canonical_id_a, canonical_id_b)
                {
                    canonical_id_a == canonical_id_b
                } else {
                    id_a == id_b && name_a == name_b
                }
            }
            (
                DataType::GenericInstance {
                    base: base_a,
                    arguments: arguments_a,
                },
                DataType::GenericInstance {
                    base: base_b,
                    arguments: arguments_b,
                },
            ) => base_a == base_b && arguments_a == arguments_b,
            (DataType::Bool, DataType::Bool) => true,
            (DataType::Range, DataType::Range) => true,
            (DataType::None, DataType::None) => true,
            (DataType::True, DataType::True) => true,
            (DataType::False, DataType::False) => true,
            (DataType::StringSlice, DataType::StringSlice) => true,
            (DataType::Char, DataType::Char) => true,
            (DataType::FixedScalar(left), DataType::FixedScalar(right)) => left == right,
            (DataType::Number(left), DataType::Number(right)) => left == right,
            (DataType::Option(a), DataType::Option(b)) => a == b,
            (
                DataType::Struct {
                    nominal_path: path_a,
                    const_record: const_a,
                    generic_instance_key: key_a,
                    ..
                },
                DataType::Struct {
                    nominal_path: path_b,
                    const_record: const_b,
                    generic_instance_key: key_b,
                    ..
                },
            ) => match (key_a, key_b) {
                (Some(a), Some(b)) => a == b && const_a == const_b,
                (None, None) => path_a == path_b && const_a == const_b,
                _ => false,
            },
            (
                DataType::Choices {
                    nominal_path: path_a,
                    generic_instance_key: key_a,
                    ..
                },
                DataType::Choices {
                    nominal_path: path_b,
                    generic_instance_key: key_b,
                    ..
                },
            ) => match (key_a, key_b) {
                (Some(a), Some(b)) => a == b,
                (None, None) => path_a == path_b,
                _ => false,
            },
            (DataType::External { type_id: id_a }, DataType::External { type_id: id_b }) => {
                id_a == id_b
            }

            _ => false,
        }
    }
}

// -----------------------------------------------------------
//  TypeId -> DataType Conversion
// -----------------------------------------------------------

/// Converts a canonical `TypeId` back into a `DataType`.
/// Diagnostic and display-only reverse bridge from canonical `TypeId` to `DataType` spelling.
///
/// WHAT: converts a resolved `TypeId` back to a `DataType` suitable for diagnostics and display.
///      Its output feeds the non-authoritative `diagnostic_type` fields on `Declaration`
///      and signature members.
/// WHY: AST nodes and error messages still carry `DataType` as written/display spelling.
///
/// DO NOT use this for semantic type decisions. Semantic identity is `TypeId` equality
/// in `TypeEnvironment`. Prefer `TypeEnvironment` queries or `display_type` for diagnostics.
fn type_id_to_data_type(type_id: ids::TypeId, type_environment: &TypeEnvironment) -> DataType {
    match type_environment.get(type_id) {
        Some(TypeDefinition::Builtin(builtin)) => match builtin.key {
            ids::BuiltinTypeKey::Bool => DataType::Bool,
            ids::BuiltinTypeKey::Number(scale) => DataType::Number(scale),
            ids::BuiltinTypeKey::String => DataType::StringSlice,
            ids::BuiltinTypeKey::Char => DataType::Char,
            ids::BuiltinTypeKey::Range => DataType::Range,
            ids::BuiltinTypeKey::FixedScalar(scalar) => DataType::FixedScalar(scalar),
            ids::BuiltinTypeKey::None => DataType::None,
        },
        Some(TypeDefinition::Struct(def)) => DataType::Struct {
            nominal_path: def.path,
            type_id,
            const_record: def.const_record,
            generic_instance_key: None,
        },
        Some(TypeDefinition::Choice(def)) => DataType::Choices {
            nominal_path: def.path,
            type_id,
            generic_instance_key: None,
        },
        Some(TypeDefinition::Constructed(con)) => match con.constructor {
            ids::TypeConstructor::Builtin(ids::BuiltinTypeConstructor::Collection {
                fixed_capacity,
            }) => {
                if let [element_id] = con.arguments.as_ref() {
                    match fixed_capacity {
                        Some(cap) => DataType::fixed_collection(
                            type_id_to_data_type(*element_id, type_environment),
                            cap,
                        ),
                        None => DataType::collection(type_id_to_data_type(
                            *element_id,
                            type_environment,
                        )),
                    }
                } else {
                    DataType::None
                }
            }
            ids::TypeConstructor::Builtin(ids::BuiltinTypeConstructor::Option) => {
                if let [inner_id] = con.arguments.as_ref() {
                    DataType::Option(Box::new(type_id_to_data_type(*inner_id, type_environment)))
                } else {
                    DataType::None
                }
            }
            ids::TypeConstructor::Builtin(ids::BuiltinTypeConstructor::OrderedMap) => {
                if let [key_id, value_id] = con.arguments.as_ref() {
                    DataType::map(
                        type_id_to_data_type(*key_id, type_environment),
                        type_id_to_data_type(*value_id, type_environment),
                    )
                } else {
                    DataType::None
                }
            }
        },
        Some(TypeDefinition::Function(_)) => {
            // Callable signatures have their own semantic owner, not a diagnostic type shape.
            DataType::Inferred
        }
        Some(TypeDefinition::External(def)) => DataType::External {
            type_id: def.type_id,
        },
        Some(TypeDefinition::GenericParameter(def)) => DataType::TypeParameter {
            id: TypeParameterId(def.id.0),
            canonical_id: Some(def.id),
            name: def.name,
        },
        Some(TypeDefinition::GenericInstance(def)) => {
            if let Some(path) = type_environment.nominal_path_by_id(def.base) {
                let arguments: Vec<DataType> = def
                    .arguments
                    .iter()
                    .map(|arg| type_id_to_data_type(*arg, type_environment))
                    .collect();
                DataType::GenericInstance {
                    base: GenericBaseType::ResolvedNominal(*path),
                    arguments,
                }
            } else {
                DataType::None
            }
        }

        // The compile-time-only anonymous const-record marker has no source spelling, so
        // the diagnostic reverse bridge keeps the same `None` placeholder as function types.
        Some(TypeDefinition::AnonymousConstRecordMarker) => DataType::None,
        None => DataType::None,
    }
}

/// Converts a canonical `TypeId` to a diagnostic `DataType` spelling.
///
/// WHAT: narrow display helper for sites that create new `Declaration` nodes
///      and need a non-authoritative `diagnostic_type`.
/// WHY: makes display-only bridge use explicit at call sites so semantic code cannot
///      accidentally call the reverse bridge.
///
/// DO NOT use this for semantic type decisions.
pub(crate) fn diagnostic_type_spelling(
    type_id: ids::TypeId,
    type_environment: &TypeEnvironment,
) -> DataType {
    type_id_to_data_type(type_id, type_environment)
}
