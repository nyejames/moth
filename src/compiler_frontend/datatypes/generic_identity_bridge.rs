//! Generic identity bridge types for diagnostics and HIR lowering.
//!
//! WHAT: owns diagnostic and HIR-facing keys that can describe generic instances
//!      outside the canonical `TypeEnvironment`.
//! WHY: HIR still registers generic nominal layouts through a lowering-local side table,
//!      and diagnostics still need source-like spelling. These keys must not decide
//!      semantic equality in AST or HIR; use `TypeId` in `TypeEnvironment` for that.

use super::environment::TypeEnvironment;
use super::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::external_packages::ExternalTypeId;
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap};
#[cfg(any(test, feature = "show_hir"))]
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

// -----------------------------------------------------------
//  Identity Keys (HIR / Diagnostic Bridge)
// -----------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinTypeKey {
    Bool,
    /// One arbitrary-precision `Dec` scale identity; scale zero is the `Dec` alias.
    Number(NumberScale),
    String,
    Char,
    Range,
    FixedScalar(FixedScalar),
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
            Self::GenericInstance(instance) => instance.remap_path_ids(remap),
            Self::Builtin(_) | Self::External(_) => {}
        }
    }
}

// Display helpers below render a `GenericInstantiationKey` through an explicit path reader.
// The retained entry point is the fork-based diagnostic surface; a `PathId` is never rendered
// as source text.

#[cfg(any(test, feature = "show_hir"))]
trait PathNameResolver {
    fn component(&self, path: PathId) -> Option<StringId>;
}

#[cfg(any(test, feature = "show_hir"))]
impl PathNameResolver for PathInternerFork {
    fn component(&self, path: PathId) -> Option<StringId> {
        PathInternerFork::component(self, path)
    }
}

#[cfg(any(test, feature = "show_hir"))]
pub fn display_generic_instantiation_key_with_fork(
    key: &GenericInstantiationKey,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> String {
    display_generic_instantiation_key_with_resolver(key, path_fork, string_table)
}

#[cfg(any(test, feature = "show_hir"))]
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

#[cfg(any(test, feature = "show_hir"))]
fn display_type_identity_key_with_resolver<R: PathNameResolver>(
    key: &TypeIdentityKey,
    path_reader: &R,
    string_table: &StringTable,
) -> String {
    match key {
        TypeIdentityKey::Builtin(builtin) => match builtin {
            BuiltinTypeKey::Bool => "Bool".to_owned(),
            BuiltinTypeKey::Number(scale) => scale.to_string(),
            BuiltinTypeKey::String => "String".to_owned(),
            BuiltinTypeKey::Char => "Char".to_owned(),
            BuiltinTypeKey::Range => "Range".to_owned(),
            BuiltinTypeKey::FixedScalar(scalar) => scalar.name().to_owned(),
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
        TypeIdentityKey::GenericInstance(instance) => {
            display_generic_instantiation_key_with_resolver(instance, path_reader, string_table)
        }
        TypeIdentityKey::External(external) => format!("<external:{}>", external.0),
    }
}

// -----------------------------------------------------------
//  Identity Key -> TypeId Bridge
// -----------------------------------------------------------

/// Converts a `TypeIdentityKey` into canonical `TypeId` identity.
///
/// WHAT: resolves the lowering-local HIR bridge key against the module's canonical
///       `TypeEnvironment`.
/// WHY: HIR registers generic nominal layouts from bridge keys, but concrete instances
///      must retain the frontend's canonical `TypeId`.
pub(crate) fn type_identity_key_to_type_id(
    key: &TypeIdentityKey,
    type_environment: &mut TypeEnvironment,
) -> Option<TypeId> {
    match key {
        TypeIdentityKey::Builtin(builtin) => Some(match builtin {
            BuiltinTypeKey::Bool => type_environment.builtins().bool,
            BuiltinTypeKey::Number(scale) => type_environment.intern_number(*scale),
            BuiltinTypeKey::String => type_environment.builtins().string,
            BuiltinTypeKey::Char => type_environment.builtins().char,
            BuiltinTypeKey::Range => type_environment.builtins().range,
            BuiltinTypeKey::FixedScalar(scalar) => builtin_type_ids::fixed_scalar(*scalar),
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
