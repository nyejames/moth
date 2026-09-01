//! The `DataType` enum and its intrinsic methods.
//!
//! WHAT: carries parsed and diagnostic-only type spellings used before and during AST
//!       construction. Contains constructors, queries, remapping, display helpers, and
//!       the reverse bridge from canonical `TypeId` back to diagnostic spellings.
//! WHY: separating the data shape from `mod.rs` keeps the module entry point focused
//!      on orchestration and re-exports.
//!
//! NOTE: `DataType` owns type structure and structural helper methods only.
//! Compatibility policy lives in `type_coercion::compatibility`.
//! Contextual promotion lives in `type_coercion::contextual`.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::external_packages::ExternalTypeId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

use super::definitions::TypeDefinition;
use super::display::format_fallible_signature_parts;
use super::environment::TypeEnvironment;
use super::generic_identity_bridge::display_generic_instantiation_key;
use super::generic_identity_bridge::{
    BuiltinGenericType, GenericBaseType, GenericInstantiationKey,
};
use super::generic_parameters::TypeParameterId;
use super::ids::{self, GenericParameterId, TypeId};

use super::{BuiltinScalarReceiver, ReceiverKey};
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
        nominal_path: InternedPath,
        type_id: TypeId,
        /// Diagnostic/render-only marker. Semantic const-record decisions must
        /// use `Expression::const_record_state`, not this field.
        const_record: bool,
        generic_instance_key: Option<GenericInstantiationKey>,
    },
    Range, // Iterable that must always be owned.
    Returns(Vec<DataType>),
    Function(Box<Option<ReceiverKey>>, FunctionSignature), // Receiver, signature

    // Compile-time/frontend-specific composite values.
    Template,

    // Scalar/runtime-leaf types.
    Bool,
    Int,
    Float,
    // Decimal is intentionally inactive in the Alpha surface. The variant is kept
    // only as a diagnostic placeholder for the inactive builtin TypeId; no parser,
    // operator, or HIR path may produce a live Decimal value.
    #[allow(dead_code)]
    Decimal,
    StringSlice, // UTF-8 read-only string slice
    Char,

    // Reserved or not-yet-wired variants kept for planned language work.
    #[allow(dead_code)] // Planned: explicit parameter/record type surfaces.
    Parameters(Vec<Declaration>), // Struct definitions and parameters

    Choices {
        nominal_path: InternedPath,
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
    /// Temporary diagnostic/control-flow bridge for fallible operation handling.
    ///
    /// V1 fallibility is represented by function return metadata and explicit
    /// control flow. This variant is implementation machinery for fallible
    /// casts, collection operations, and external/source calls that are
    /// immediately consumed by postfix `!` or boundary `catch`.
    FallibleCarrier {
        success: Box<DataType>,
        error: Box<DataType>,
    },
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

    pub fn runtime_struct(nominal_path: InternedPath, type_id: TypeId) -> Self {
        Self::runtime_struct_with_generic_key(nominal_path, type_id, None)
    }

    pub fn runtime_struct_with_generic_key(
        nominal_path: InternedPath,
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

    pub fn const_struct_record(nominal_path: InternedPath, type_id: TypeId) -> Self {
        Self::const_struct_record_with_generic_key(nominal_path, type_id, None)
    }

    pub fn const_struct_record_with_generic_key(
        nominal_path: InternedPath,
        type_id: TypeId,
        generic_instance_key: Option<GenericInstantiationKey>,
    ) -> Self {
        Self::Struct {
            nominal_path,
            type_id,
            const_record: true,
            generic_instance_key,
        }
    }

    // -----------------
    //  Queries
    // -----------------

    /// Returns the receiver key for this type.
    ///
    /// WHAT: derives a receiver key from inline `DataType` fields for HIR/diagnostic
    ///       compatibility paths that do not have `TypeEnvironment` access.
    /// WHY: HIR lowering and some diagnostic paths still need `DataType`-shaped
    ///      receiver keys; this is display/compatibility logic, not semantic identity.
    pub fn receiver_key_from_type(&self) -> Option<ReceiverKey> {
        match self {
            DataType::Struct {
                nominal_path,
                const_record,
                generic_instance_key: None,
                ..
            } if !const_record => Some(ReceiverKey::Struct(nominal_path.to_owned())),
            DataType::Choices {
                nominal_path,
                generic_instance_key: None,
                ..
            } => Some(ReceiverKey::Choice(nominal_path.to_owned())),
            DataType::Int => Some(ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::Int)),
            DataType::Float => Some(ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::Float)),
            DataType::Bool => Some(ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::Bool)),
            DataType::StringSlice => {
                Some(ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::String))
            }
            DataType::Char => Some(ReceiverKey::BuiltinScalar(BuiltinScalarReceiver::Char)),
            DataType::External { type_id } => Some(ReceiverKey::External(*type_id)),
            _ => None,
        }
    }

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

    /// Test-only diagnostic spelling query for collection fixtures.
    #[cfg(test)]
    pub fn is_collection(&self) -> bool {
        self.is_builtin_generic_collection()
    }

    /// Test-only implementation shared by collection fixture queries.
    #[cfg(test)]
    pub fn is_builtin_generic_collection(&self) -> bool {
        matches!(
            self,
            DataType::GenericInstance {
                base: GenericBaseType::Builtin(BuiltinGenericType::Collection { .. }),
                arguments,
            } if arguments.len() == 1
        )
    }

    // -----------------
    //  Display
    // -----------------

    /// Display the DataType with proper string resolution for interned strings.
    /// This method should be used instead of Display when a StringTable is available.
    pub fn display_with_table(&self, string_table: &StringTable) -> String {
        match self {
            DataType::Inferred => "Inferred".to_string(),
            DataType::NamedType(name) => string_table.resolve(*name).to_string(),
            DataType::NamespacedType { path } => path
                .iter()
                .map(|component| string_table.resolve(*component))
                .collect::<Vec<_>>()
                .join("."),
            DataType::TypeParameter { name, .. } => string_table.resolve(*name).to_string(),
            DataType::GenericInstance { base, arguments } => {
                display_generic_instance(base, arguments, string_table)
            }
            DataType::Bool => "Bool".to_string(),
            DataType::StringSlice => "String".to_string(),
            DataType::Char => "Char".to_string(),
            DataType::Float => "Float".to_string(),
            DataType::Int => "Int".to_string(),
            DataType::Decimal => "Decimal".to_string(),
            DataType::Parameters(args) => {
                let mut arg_str = String::new();
                for arg in args {
                    let name = arg.id.to_string(string_table);
                    arg_str.push_str(&format!(
                        "{}: {}, ",
                        name,
                        arg.value.diagnostic_type.display_with_table(string_table)
                    ));
                }
                format!("Parameters({arg_str})")
            }
            DataType::Struct {
                nominal_path,
                const_record,
                generic_instance_key,
                ..
            } => {
                if let Some(key) = generic_instance_key {
                    return display_generic_instantiation_key(key, string_table);
                }
                let bare_name = nominal_path
                    .name_str(string_table)
                    .unwrap_or("<anonymous struct>");
                if *const_record {
                    format!("const record {bare_name}")
                } else {
                    bare_name.to_owned()
                }
            }
            DataType::External { type_id } => {
                // External types are opaque; display uses the stable ID.
                format!("External({})", type_id.0)
            }
            DataType::Returns(returns) => {
                let returns_string = returns
                    .iter()
                    .map(|return_type| return_type.display_with_table(string_table))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("Returns({returns_string})")
            }
            DataType::Function(_, signature) => {
                let mut arg_str = String::new();
                for arg in &signature.parameters {
                    let name = arg.id.to_string(string_table);
                    arg_str.push_str(&format!(
                        "{}: {}, ",
                        name,
                        arg.value.diagnostic_type.display_with_table(string_table)
                    ));
                }

                let returns_string = display_function_return_signature(signature, string_table);
                format!("Function({arg_str} -> {returns_string})")
            }

            DataType::Template => "Template".to_string(),
            DataType::None => "None".to_string(),
            DataType::True => "True".to_string(),
            DataType::False => "False".to_string(),
            DataType::Range => "Range".to_string(),
            DataType::Option(inner_type) => {
                if displays_better_in_generic_surface(inner_type) {
                    format!("{}?", inner_type.display_with_table(string_table))
                } else {
                    format!("Option({})", inner_type.display_with_table(string_table))
                }
            }
            DataType::FallibleCarrier { success, error } => {
                display_fallible_data_type_signature(success, error, string_table)
            }
            DataType::Choices {
                nominal_path,
                generic_instance_key,
                ..
            } => {
                if let Some(key) = generic_instance_key {
                    return display_generic_instantiation_key(key, string_table);
                }
                nominal_path
                    .name_str(string_table)
                    .unwrap_or("<choice>")
                    .to_owned()
            }
        }
    }
}

// -----------------------------------------------------------
//  Display Helpers
// -----------------------------------------------------------

fn displays_better_in_generic_surface(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::TypeParameter { .. } | DataType::GenericInstance { .. } | DataType::Option(_)
    )
}

fn display_generic_instance(
    base: &GenericBaseType,
    arguments: &[DataType],
    string_table: &StringTable,
) -> String {
    if let GenericBaseType::Builtin(BuiltinGenericType::Collection { fixed_capacity }) = base
        && let [single_argument] = arguments
    {
        let element_display = single_argument.display_with_table(string_table);
        return match fixed_capacity {
            Some(capacity) => format!("{{{capacity} {element_display}}}"),
            None => format!("{{{element_display}}}"),
        };
    }

    if let GenericBaseType::Builtin(BuiltinGenericType::Map) = base
        && let [key_argument, value_argument] = arguments
    {
        let key_display = key_argument.display_with_table(string_table);
        let value_display = value_argument.display_with_table(string_table);
        return format!("{{{key_display} = {value_display}}}");
    }

    let base_display = display_generic_base(base, string_table);
    if arguments.is_empty() {
        return base_display;
    }

    let arguments_display = arguments
        .iter()
        .map(|argument| argument.display_with_table(string_table))
        .collect::<Vec<_>>()
        .join(", ");

    format!("{base_display} of {arguments_display}")
}

fn display_generic_base(base: &GenericBaseType, string_table: &StringTable) -> String {
    match base {
        GenericBaseType::Named(name) => string_table.resolve(*name).to_owned(),
        GenericBaseType::ResolvedNominal(path) => path
            .name_str(string_table)
            .unwrap_or("<generic>")
            .to_owned(),
        GenericBaseType::External(type_id) => format!("External({})", type_id.0),
        GenericBaseType::Builtin(BuiltinGenericType::Collection { .. }) => {
            String::from("Collection")
        }
        GenericBaseType::Builtin(BuiltinGenericType::Map) => String::from("Map"),
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
            (DataType::Float, DataType::Float) => true,
            (DataType::Int, DataType::Int) => true,
            (DataType::Decimal, DataType::Decimal) => true,
            (
                DataType::FallibleCarrier {
                    success: success_a,
                    error: error_a,
                },
                DataType::FallibleCarrier {
                    success: success_b,
                    error: error_b,
                },
            ) => success_a == success_b && error_a == error_b,
            (DataType::Template, DataType::Template) => true,
            (DataType::Option(a), DataType::Option(b)) => a == b,
            // For Args, Struct, Function, and Choices, we compare by name/structure
            // but not by the actual Arg values since they contain Expressions
            (DataType::Parameters(a), DataType::Parameters(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|(arg_a, arg_b)| arg_a.id == arg_b.id)
            }
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
            (DataType::Function(_, signature1), DataType::Function(_, signature2)) => {
                // If both functions have the same signature.returns types,
                // then they are equal
                signature1.returns.len() == signature2.returns.len()
                    && signature1
                        .returns
                        .iter()
                        .zip(signature2.returns.iter())
                        .all(|(return1, return2)| return1.data_type() == return2.data_type())
            }
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

fn display_fallible_data_type_signature(
    success_type: &DataType,
    error_type: &DataType,
    string_table: &StringTable,
) -> String {
    let success_parts = match success_type {
        DataType::None => Vec::new(),
        DataType::Returns(success_types) => success_types
            .iter()
            .map(|success_type| success_type.display_with_table(string_table))
            .collect(),
        success_type => vec![success_type.display_with_table(string_table)],
    };
    let error_part = error_type.display_with_table(string_table);

    format_fallible_signature_parts(success_parts, error_part)
}

fn display_function_return_signature(
    signature: &FunctionSignature,
    string_table: &StringTable,
) -> String {
    let parts = signature
        .success_returns()
        .iter()
        .map(|return_value| return_value.display_with_table(string_table))
        .collect::<Vec<_>>();

    if let Some(error_return) = signature.error_return() {
        return format_fallible_signature_parts(
            parts,
            error_return.display_with_table(string_table),
        );
    }

    parts.join(", ")
}

// -----------------------------------------------------------
//  TypeId -> DataType Conversion
// -----------------------------------------------------------

/// Converts a canonical `TypeId` back into a `DataType`.
/// Diagnostic and display-only reverse bridge from canonical `TypeId` to `DataType` spelling.
///
/// WHAT: converts a resolved `TypeId` back to a `DataType` suitable for diagnostics,
///      display, and non-authoritative `Expression.diagnostic_type` fields.
/// WHY: AST nodes and error messages still carry `DataType` as written/display spelling.
///
/// DO NOT use this for semantic type decisions. Semantic identity is `TypeId` equality
/// in `TypeEnvironment`. Prefer `TypeEnvironment` queries or `display_type` for diagnostics.
fn type_id_to_data_type(type_id: ids::TypeId, type_environment: &TypeEnvironment) -> DataType {
    match type_environment.get(type_id) {
        Some(TypeDefinition::Builtin(builtin)) => match builtin.key {
            ids::BuiltinTypeKey::Bool => DataType::Bool,
            ids::BuiltinTypeKey::Int => DataType::Int,
            ids::BuiltinTypeKey::Float => DataType::Float,
            ids::BuiltinTypeKey::Decimal => DataType::Decimal,
            ids::BuiltinTypeKey::String => DataType::StringSlice,
            ids::BuiltinTypeKey::Char => DataType::Char,
            ids::BuiltinTypeKey::Range => DataType::Range,
            ids::BuiltinTypeKey::None => DataType::None,
        },
        Some(TypeDefinition::Struct(def)) => DataType::Struct {
            nominal_path: def.path.clone(),
            type_id,
            const_record: def.const_record,
            generic_instance_key: None,
        },
        Some(TypeDefinition::Choice(def)) => DataType::Choices {
            nominal_path: def.path.clone(),
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
            ids::TypeConstructor::Builtin(ids::BuiltinTypeConstructor::FallibleCarrier) => {
                if let [success_id, error_id] = con.arguments.as_ref() {
                    DataType::fallible_carrier(
                        type_id_to_data_type(*success_id, type_environment),
                        type_id_to_data_type(*error_id, type_environment),
                    )
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
            ids::TypeConstructor::Builtin(ids::BuiltinTypeConstructor::Tuple) => DataType::Returns(
                con.arguments
                    .iter()
                    .map(|argument| type_id_to_data_type(*argument, type_environment))
                    .collect(),
            ),
        },
        Some(TypeDefinition::Function(_)) => {
            // Function types cannot be fully reconstructed as DataType because
            // FunctionSignature contains Declaration nodes with Expressions.
            // Function types cannot be represented as `DataType` because
            // `FunctionSignature` contains AST nodes. Return a placeholder.
            DataType::None
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
                    base: GenericBaseType::ResolvedNominal(path.clone()),
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
/// WHAT: narrow display helper for sites that create new `Expression`/`Declaration` nodes
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
