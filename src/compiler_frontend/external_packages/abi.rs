//! Backend-agnostic ABI types for the external-call boundary.
//!
//! WHAT: defines the type system that external functions use to describe their parameters
//! and return values to the frontend. This is a narrower vocabulary than the full Moth
//! type system because host boundaries are intentionally restricted.
//! WHY: the frontend needs to know how to validate and lower arguments without embedding
//! backend-specific knowledge into the AST.
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};

use super::ids::ExternalTypeId;
use crate::compiler_frontend::datatypes::DataType;

/// Backend-agnostic ABI values that currently cross the host boundary.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExternalAbiType {
    I32,
    F64,
    Bool,
    Utf8Str,
    Char,
    Void,
    /// Opaque handle to an external type (lowers to `i32` in Wasm, object reference in JS).
    Handle,
    /// Parameter accepts any language type (used for polymorphic external functions
    /// such as collection helpers during the transition to explicit ABI types.
    Inferred,
}

impl ExternalAbiType {
    /// Maps this ABI type to the corresponding frontend `DataType` when one exists.
    pub(crate) fn to_datatype(&self) -> Option<DataType> {
        match self {
            ExternalAbiType::I32 => Some(DataType::Int),
            ExternalAbiType::F64 => Some(DataType::Float),
            ExternalAbiType::Bool => Some(DataType::Bool),
            ExternalAbiType::Utf8Str => Some(DataType::StringSlice),
            ExternalAbiType::Char => Some(DataType::Char),
            ExternalAbiType::Void => None,
            ExternalAbiType::Handle => None,
            ExternalAbiType::Inferred => Some(DataType::Inferred),
        }
    }

    /// Maps this ABI type to the canonical frontend `TypeId` when one exists.
    ///
    /// WHAT: resolves builtin scalar ABI types to their canonical TypeEnvironment IDs.
    /// WHY: host/external parameter expectations should carry real canonical TypeIds,
    ///      not placeholder NONE values that require diagnostic-type repair later.
    pub(crate) fn to_type_id(
        &self,
        type_environment: &crate::compiler_frontend::datatypes::environment::TypeEnvironment,
    ) -> Option<crate::compiler_frontend::datatypes::ids::TypeId> {
        match self {
            ExternalAbiType::I32 => Some(type_environment.builtins().int),
            ExternalAbiType::F64 => Some(type_environment.builtins().float),
            ExternalAbiType::Bool => Some(type_environment.builtins().bool),
            ExternalAbiType::Utf8Str => Some(type_environment.builtins().string),
            ExternalAbiType::Char => Some(type_environment.builtins().char),
            ExternalAbiType::Void | ExternalAbiType::Handle | ExternalAbiType::Inferred => None,
        }
    }
}

/// Frontend-visible type used by external function signatures.
///
/// WHAT: separates the backend ABI category from the Moth language type expected
///       at call sites. Builtin scalar parameters use `Abi(...)`, provider-owned
///       opaque types use `External(...)`, and reusable language-level content
///       policies such as string content use dedicated variants.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExternalSignatureType {
    Abi(ExternalAbiType),
    BuiltinError,
    External(ExternalTypeId),
    /// Any string-compatible value: string slices, owned strings, and templates.
    ///
    /// WHAT: a reusable language-level parameter policy that accepts every form of string
    ///       content without duplicating scalar renderable logic per host function.
    /// WHY: external boundaries such as console output need to accept `"text"`, variables,
    ///      and `[: templates]` through the normal compatibility path.
    StringContent,
    /// Optional value using the canonical built-in option representation.
    ///
    /// WHAT: lets external functions declare `T?` returns or parameters without inventing
    ///       a backend-specific sentinel. The inner type is resolved through the normal
    ///       signature-type conversion path and then interned as a built-in option.
    /// WHY: reusable host boundaries such as `last_key_pressed -> String?` must share the
    ///      same `TypeId` identity as source-authored `String?`.
    Optional(Box<ExternalSignatureType>),
}

impl ExternalSignatureType {
    pub(crate) fn to_datatype(&self) -> Option<DataType> {
        match self {
            Self::Abi(abi_type) => abi_type.to_datatype(),
            // The builtin Error type is nominal and registered per module, so the caller
            // supplies its resolved spelling at the AST boundary.
            Self::BuiltinError => None,
            Self::External(type_id) => Some(DataType::External { type_id: *type_id }),
            // StringContent resolves to the canonical string datatype for diagnostics.
            Self::StringContent => Some(DataType::StringSlice),
            Self::Optional(inner) => inner
                .to_datatype()
                .map(|inner_type| DataType::Option(Box::new(inner_type))),
        }
    }

    pub(crate) fn to_type_id(
        &self,
        type_environment: &mut crate::compiler_frontend::datatypes::environment::TypeEnvironment,
        builtin_error_type_id: crate::compiler_frontend::datatypes::ids::TypeId,
    ) -> Option<crate::compiler_frontend::datatypes::ids::TypeId> {
        match self {
            Self::Abi(abi_type) => abi_type.to_type_id(type_environment),
            Self::BuiltinError => Some(builtin_error_type_id),
            Self::External(type_id) => Some(type_environment.intern_external(*type_id)),
            // StringContent resolves to the canonical String TypeId so escaped slices, owned
            // strings, and templates all pass the normal compatibility check.
            Self::StringContent => Some(type_environment.builtins().string),
            Self::Optional(inner) => {
                let inner_type_id = inner.to_type_id(type_environment, builtin_error_type_id)?;
                Some(type_environment.intern_option(inner_type_id))
            }
        }
    }

    /// Resolves this signature type to a canonical `TypeId` for parameter validation.
    ///
    /// WHAT: parameter contexts do not need `builtin_error_type_id` because `BuiltinError`
    ///       is not a valid parameter type. Returning `None` for that variant maps it to
    ///       `UnknownExternal`, which safely skips compatibility checking.
    /// WHY: avoids threading `builtin_error_type_id` through every call-validation helper
    ///      when only return slots legitimately use `BuiltinError`.
    pub(crate) fn to_parameter_type_id(
        &self,
        type_environment: &mut crate::compiler_frontend::datatypes::environment::TypeEnvironment,
    ) -> Option<crate::compiler_frontend::datatypes::ids::TypeId> {
        match self {
            Self::Abi(abi_type) => abi_type.to_type_id(type_environment),
            // BuiltinError is not expected in parameter position; treat as unknown.
            Self::BuiltinError => None,
            Self::External(type_id) => Some(type_environment.intern_external(*type_id)),
            // StringContent accepts any string-compatible value at call sites.
            Self::StringContent => Some(type_environment.builtins().string),
            Self::Optional(inner) => {
                let inner_type_id = inner.to_parameter_type_id(type_environment)?;
                Some(type_environment.intern_option(inner_type_id))
            }
        }
    }
}

impl From<ExternalAbiType> for ExternalSignatureType {
    fn from(value: ExternalAbiType) -> Self {
        Self::Abi(value)
    }
}

/// A single external-call parameter definition.
#[derive(Debug)]
pub struct ExternalParameter {
    /// What the Moth language accepts.
    ///
    /// WHAT: `ExternalSignatureType` so exact package-scoped opaque type identity can be
    ///       carried for provider-created types, while builtin scalars wrap through `Abi(...)`.
    /// WHY: collapsing opaque types to `Handle` loses the distinction between external type A
    ///      and external type B at call boundaries.
    pub language_type: ExternalSignatureType,
    /// Borrow access mode required for this argument.
    pub access_kind: ExternalAccessKind,
}

impl Clone for ExternalParameter {
    fn clone(&self) -> Self {
        increment_frontend_counter(FrontendCounter::ExternalAbiParameterCloneCount);
        Self {
            language_type: self.language_type.clone(),
            access_kind: self.access_kind,
        }
    }
}

/// Borrow access mode for an external parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExternalAccessKind {
    Shared,
    Mutable,
}

/// Describes how an external function's return value aliases its arguments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExternalReturnAlias {
    /// Return value is freshly allocated and does not alias any argument.
    Fresh,
    /// Return value may alias the arguments at the given parameter indices.
    AliasArgs(Vec<usize>),
}
