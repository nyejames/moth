//! Generic identity bridge types for diagnostics and HIR lowering.
//!
//! WHAT: owns diagnostic and HIR-facing keys that can describe generic instances
//!      outside the canonical `TypeEnvironment`.
//! WHY: HIR still registers generic nominal layouts through a lowering-local side table,
//!      and diagnostics still need source-like spelling. These keys must not decide
//!      semantic equality in AST or HIR; use `TypeId` in `TypeEnvironment` for that.

use super::DataType;
use super::display::format_fallible_signature_parts;
use super::environment::TypeEnvironment;
use super::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalTypeId;
use crate::compiler_frontend::symbols::path_interner::{
    PathId, PathIdRemap, PathInternerFork, PathTable,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

// -----------------------------------------------------------
//  Identity Keys (HIR / Diagnostic Bridge)
// -----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GenericBaseType {
    Named(StringId),
    ResolvedNominal(PathId),
    #[allow(dead_code)] // Deferred until external generic type metadata exists.
    External(ExternalTypeId),
    Builtin(BuiltinGenericType),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinGenericType {
    Collection { fixed_capacity: Option<usize> },
    Map,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinTypeKey {
    Bool,
    Int,
    Float,
    // Decimal is intentionally inactive in the Alpha surface. The key is kept only
    // to preserve the stable builtin TypeId layout and diagnostic bridge spelling.
    Decimal,
    String,
    Char,
    Range,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenericInstantiationKey {
    pub base_path: PathId,
    pub arguments: Vec<TypeIdentityKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeIdentityKey {
    Builtin(BuiltinTypeKey),
    Nominal(PathId),
    External(ExternalTypeId),
    Collection {
        element: Box<TypeIdentityKey>,
        fixed_capacity: Option<usize>,
    },
    Map {
        key: Box<TypeIdentityKey>,
        value: Box<TypeIdentityKey>,
    },
    Option(Box<TypeIdentityKey>),
    FallibleCarrier {
        success: Box<TypeIdentityKey>,
        error: Box<TypeIdentityKey>,
    },
    GenericInstance(GenericInstantiationKey),
}
impl GenericInstantiationKey {
    /// Rewrite all path identities after the module-local path fork merges.
    pub(crate) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.base_path = remap.get(self.base_path);
        for argument in &mut self.arguments {
            argument.remap_path_ids(remap);
        }
    }
}

impl TypeIdentityKey {
    /// Rewrite every nested nominal path after the module-local path fork merges.
    pub(crate) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        match self {
            Self::Nominal(path) => *path = remap.get(*path),
            Self::Collection { element, .. } | Self::Option(element) => {
                element.remap_path_ids(remap)
            }
            Self::Map { key, value } => {
                key.remap_path_ids(remap);
                value.remap_path_ids(remap);
            }
            Self::FallibleCarrier { success, error } => {
                success.remap_path_ids(remap);
                error.remap_path_ids(remap);
            }
            Self::GenericInstance(instance) => instance.remap_path_ids(remap),
            Self::Builtin(_) | Self::External(_) => {}
        }
    }
}

// Display helpers below retain the existing two-argument compatibility surface for
// callers that have not yet been threaded with a path reader. New callers must use
// one of the path-aware variants so a `PathId` is never rendered as source text.

trait PathNameResolver {
    fn component(&self, path: PathId) -> Option<StringId>;
}

impl PathNameResolver for PathInternerFork {
    fn component(&self, path: PathId) -> Option<StringId> {
        PathInternerFork::component(self, path)
    }
}

impl PathNameResolver for PathTable {
    fn component(&self, path: PathId) -> Option<StringId> {
        PathTable::component(self, path)
    }
}

pub fn display_generic_instantiation_key_with_fork(
    key: &GenericInstantiationKey,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> String {
    display_generic_instantiation_key_with_resolver(key, path_fork, string_table)
}

pub fn display_generic_instantiation_key(
    key: &GenericInstantiationKey,
    path_table: &PathTable,
    string_table: &StringTable,
) -> String {
    display_generic_instantiation_key_with_resolver(key, path_table, string_table)
}

fn display_generic_instantiation_key_with_resolver<R: PathNameResolver>(
    key: &GenericInstantiationKey,
    path_reader: &R,
    string_table: &StringTable,
) -> String {
    let base_name = path_reader
        .component(key.base_path)
        .map(|component| string_table.resolve(component))
        .unwrap_or("<generic>");
    let args = key
        .arguments
        .iter()
        .map(|arg| display_type_identity_key_with_resolver(arg, path_reader, string_table))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{base_name} of {args}")
}

fn display_type_identity_key_with_resolver<R: PathNameResolver>(
    key: &TypeIdentityKey,
    path_reader: &R,
    string_table: &StringTable,
) -> String {
    match key {
        TypeIdentityKey::Builtin(builtin) => match builtin {
            BuiltinTypeKey::Bool => "Bool".to_owned(),
            BuiltinTypeKey::Int => "Int".to_owned(),
            BuiltinTypeKey::Float => "Float".to_owned(),
            BuiltinTypeKey::Decimal => "Decimal".to_owned(),
            BuiltinTypeKey::String => "String".to_owned(),
            BuiltinTypeKey::Char => "Char".to_owned(),
            BuiltinTypeKey::Range => "Range".to_owned(),
        },
        TypeIdentityKey::Nominal(path) => path_reader
            .component(*path)
            .map(|component| string_table.resolve(component).to_owned())
            .unwrap_or_else(|| "<nominal>".to_owned()),
        TypeIdentityKey::Collection {
            element: inner,
            fixed_capacity,
        } => match fixed_capacity {
            Some(cap) => format!(
                "{{{cap} {}}}",
                display_type_identity_key_with_resolver(inner, path_reader, string_table)
            ),
            None => format!(
                "{{{}}}",
                display_type_identity_key_with_resolver(inner, path_reader, string_table)
            ),
        },
        TypeIdentityKey::Map { key, value } => format!(
            "{{{} = {}}}",
            display_type_identity_key_with_resolver(key, path_reader, string_table),
            display_type_identity_key_with_resolver(value, path_reader, string_table)
        ),
        TypeIdentityKey::Option(inner) => format!(
            "{}?",
            display_type_identity_key_with_resolver(inner, path_reader, string_table)
        ),
        TypeIdentityKey::FallibleCarrier { success, error } => format_fallible_signature_parts(
            vec![display_type_identity_key_with_resolver(
                success,
                path_reader,
                string_table,
            )],
            display_type_identity_key_with_resolver(error, path_reader, string_table),
        ),
        TypeIdentityKey::GenericInstance(instance) => {
            display_generic_instantiation_key_with_resolver(instance, path_reader, string_table)
        }
        TypeIdentityKey::External(external) => format!("<external:{}>", external.0),
    }
}



// -----------------------------------------------------------
//  DataType -> Identity Key Bridge
// -----------------------------------------------------------

/// Converts a diagnostic `DataType` into a stable `TypeIdentityKey`.
///
/// WHAT: diagnostic/HIR compatibility bridge for generic instance registration.
/// WHY: HIR generic struct/choice registration and parse resolution still use
///      `TypeIdentityKey` as a lowering-local key. Prefer `TypeEnvironment::type_id_to_type_identity_key`
///      for new code that starts from a canonical `TypeId`.
///
/// Returns `None` for unresolved or unsupported types (e.g., `TypeParameter`, `Inferred`).
pub fn data_type_to_type_identity_key(data_type: &DataType) -> Option<TypeIdentityKey> {
    match data_type {
        DataType::Bool => Some(TypeIdentityKey::Builtin(BuiltinTypeKey::Bool)),
        DataType::Int => Some(TypeIdentityKey::Builtin(BuiltinTypeKey::Int)),
        DataType::Float => Some(TypeIdentityKey::Builtin(BuiltinTypeKey::Float)),
        // Decimal is intentionally inactive in the Alpha surface. Keep the bridge
        // spelling for diagnostics, but no parser or operator path may produce it.
        DataType::Decimal => Some(TypeIdentityKey::Builtin(BuiltinTypeKey::Decimal)),
        DataType::StringSlice => Some(TypeIdentityKey::Builtin(BuiltinTypeKey::String)),
        DataType::Char => Some(TypeIdentityKey::Builtin(BuiltinTypeKey::Char)),
        DataType::Range => Some(TypeIdentityKey::Builtin(BuiltinTypeKey::Range)),
        DataType::Struct {
            nominal_path,
            generic_instance_key: None,
            ..
        }
        | DataType::Choices {
            nominal_path,
            generic_instance_key: None,
            ..
        } => Some(TypeIdentityKey::Nominal(nominal_path.to_owned())),
        DataType::External { type_id } => Some(TypeIdentityKey::External(*type_id)),
        DataType::GenericInstance {
            base: GenericBaseType::ResolvedNominal(path),
            arguments,
        } => {
            let argument_keys = arguments
                .iter()
                .map(data_type_to_type_identity_key)
                .collect::<Option<Vec<_>>>()?;

            Some(TypeIdentityKey::GenericInstance(GenericInstantiationKey {
                base_path: path.to_owned(),
                arguments: argument_keys,
            }))
        }
        DataType::Struct {
            generic_instance_key: Some(key),
            ..
        }
        | DataType::Choices {
            generic_instance_key: Some(key),
            ..
        } => Some(TypeIdentityKey::GenericInstance(key.to_owned())),
        DataType::GenericInstance {
            base: GenericBaseType::Builtin(BuiltinGenericType::Collection { fixed_capacity }),
            arguments,
        } => match arguments.as_slice() {
            [element] => {
                data_type_to_type_identity_key(element).map(|key| TypeIdentityKey::Collection {
                    element: Box::new(key),
                    fixed_capacity: *fixed_capacity,
                })
            }
            _ => None,
        },
        DataType::GenericInstance {
            base: GenericBaseType::Builtin(BuiltinGenericType::Map),
            arguments,
        } => match arguments.as_slice() {
            [key, value] => {
                let key_id = data_type_to_type_identity_key(key)?;
                let value_id = data_type_to_type_identity_key(value)?;
                Some(TypeIdentityKey::Map {
                    key: Box::new(key_id),
                    value: Box::new(value_id),
                })
            }
            _ => None,
        },
        DataType::Option(inner) => {
            data_type_to_type_identity_key(inner).map(|key| TypeIdentityKey::Option(Box::new(key)))
        }
        DataType::FallibleCarrier { success, error } => {
            let ok_key = data_type_to_type_identity_key(success)?;
            let err_key = data_type_to_type_identity_key(error)?;
            Some(TypeIdentityKey::FallibleCarrier {
                success: Box::new(ok_key),
                error: Box::new(err_key),
            })
        }
        _ => None,
    }
}

// -----------------------------------------------------------
//  Identity Key -> TypeId Bridge
// -----------------------------------------------------------

/// Converts a `TypeIdentityKey` into canonical `TypeId` identity.
///
/// WHAT: reverse lookup from the diagnostic/HIR bridge key to the module's canonical
///       `TypeEnvironment`.
/// WHY: HIR still has to register generic nominal layouts from bridge keys, but the
///      concrete instance it registers must still be the frontend `TypeId`.
pub(crate) fn type_identity_key_to_type_id(
    key: &TypeIdentityKey,
    type_environment: &mut TypeEnvironment,
) -> Option<TypeId> {
    match key {
        TypeIdentityKey::Builtin(builtin) => Some(match builtin {
            BuiltinTypeKey::Bool => type_environment.builtins().bool,
            BuiltinTypeKey::Int => type_environment.builtins().int,
            BuiltinTypeKey::Float => type_environment.builtins().float,
            // Decimal is intentionally inactive in the Alpha surface. The reverse
            // lookup is preserved only for diagnostic/HID bridge round-tripping.
            BuiltinTypeKey::Decimal => type_environment.builtins().decimal,
            BuiltinTypeKey::String => type_environment.builtins().string,
            BuiltinTypeKey::Char => type_environment.builtins().char,
            BuiltinTypeKey::Range => type_environment.builtins().range,
        }),
        TypeIdentityKey::Nominal(path) => type_environment
            .nominal_id_for_path(path)
            .and_then(|nominal_id| type_environment.type_id_for_nominal_id(nominal_id)),
        TypeIdentityKey::External(type_id) => Some(type_environment.intern_external(*type_id)),
        TypeIdentityKey::Collection {
            element: inner,
            fixed_capacity,
        } => {
            let element_id = type_identity_key_to_type_id(inner, type_environment)?;
            Some(type_environment.intern_collection(element_id, *fixed_capacity))
        }
        TypeIdentityKey::Map { key, value } => {
            let key_id = type_identity_key_to_type_id(key, type_environment)?;
            let value_id = type_identity_key_to_type_id(value, type_environment)?;
            Some(type_environment.intern_map(key_id, value_id))
        }
        TypeIdentityKey::Option(inner) => {
            let inner_id = type_identity_key_to_type_id(inner, type_environment)?;
            Some(type_environment.intern_option(inner_id))
        }
        TypeIdentityKey::FallibleCarrier { success, error } => {
            let success_id = type_identity_key_to_type_id(success, type_environment)?;
            let error_id = type_identity_key_to_type_id(error, type_environment)?;
            Some(type_environment.intern_fallible_carrier(success_id, error_id))
        }
        TypeIdentityKey::GenericInstance(instance) => {
            let nominal_id = type_environment.nominal_id_for_path(&instance.base_path)?;
            let argument_ids =
                generic_instantiation_key_argument_type_ids(instance, type_environment)?;
            Some(type_environment.intern_generic_instance(nominal_id, argument_ids))
        }
    }
}

/// Converts every argument in a generic instantiation key to canonical `TypeId`s.
///
/// WHAT: returns `None` if any argument cannot be represented in the target environment.
/// WHY: generic instances must never be interned with a silently truncated argument list.
pub(crate) fn generic_instantiation_key_argument_type_ids(
    key: &GenericInstantiationKey,
    type_environment: &mut TypeEnvironment,
) -> Option<Box<[TypeId]>> {
    key.arguments
        .iter()
        .map(|argument| type_identity_key_to_type_id(argument, type_environment))
        .collect::<Option<Vec<_>>>()
        .map(Vec::into_boxed_slice)
}
