//! Definitions for external functions, types, constants, and packages.
//!
//! WHAT: describes the metadata for individual external symbols and the packages that group them.
//! WHY: the registry stores these definitions so the frontend and backends can query signatures
//! and lowering metadata without re-parsing binding files.
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};

use super::abi::{ExternalAbiType, ExternalParameter, ExternalReturnAlias, ExternalSignatureType};
use super::ids::{ExternalConstantId, ExternalFunctionId, ExternalPackageId, ExternalTypeId};
use super::symbol_path::ExternalSymbolPath;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use std::collections::HashMap;

/// Backend-specific lowering metadata for an external function.
#[derive(Debug, Clone, Default)]
pub struct ExternalFunctionLowerings {
    pub js: Option<ExternalJsLowering>,
    pub wasm: Option<ExternalWasmLowering>,
}

/// JavaScript backend lowering strategy for an external function.
#[derive(Debug, Clone)]
pub enum ExternalJsLowering {
    /// Emit a call to a named JS runtime helper function.
    RuntimeFunction(String),
    /// Emit an inline JS expression by substituting lowered arguments into a template.
    InlineExpression(String),
    /// Provider-created ES module export that generated HTML glue must import and call.
    ExternalModuleExport { export_name: String },
}

/// Wasm backend lowering strategy for an external function.
/// Placeholder: Wasm external support is still experimental.
#[derive(Debug, Clone)]
pub enum ExternalWasmLowering {
    HostFunction(&'static str),
}

/// Compiler-owned Rust evaluation selected by trusted registration; it is not a source
/// annotation, callback, name lookup, or lowering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalConstEvalOp {
    TextLength,
    TextIsEmpty,
    TextContains,
    TextStartsWith,
    TextEndsWith,
}

impl ExternalConstEvalOp {
    /// Reports whether the parameter and success-slot types exactly match this operation.
    pub fn matches_signature(
        self,
        parameters: &[ExternalParameter],
        success_slots: &[ExternalReturnSlot],
    ) -> bool {
        let (parameter_count, return_type) = match self {
            Self::TextLength => (
                1,
                ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::I32)),
            ),
            Self::TextIsEmpty => (1, ExternalSignatureType::Abi(ExternalAbiType::Bool)),
            Self::TextContains | Self::TextStartsWith | Self::TextEndsWith => {
                (2, ExternalSignatureType::Abi(ExternalAbiType::Bool))
            }
        };

        parameters.len() == parameter_count
            && parameters.iter().all(|parameter| {
                parameter.language_type == ExternalSignatureType::Abi(ExternalAbiType::Utf8Str)
            })
            && success_slots.len() == 1
            && success_slots[0].value_type == return_type
    }
}

/// Full definition of a single external function.
#[derive(Debug)]
pub struct ExternalFunctionDef {
    /// Leaf symbol name within its package.
    ///
    /// WHAT: the final path component, used for diagnostics and lowering.
    /// WHY: the full path is owned by the registry's package path-to-ID maps; the definition
    /// only needs its leaf identity.
    pub name: String,
    pub parameters: Vec<ExternalParameter>,
    /// Success-channel return slots exposed to Moth callers.
    ///
    /// WHAT: external functions use the same success/error signature shape as source
    /// functions, but the signature is supplied by builder metadata rather than parsed
    /// source.
    /// WHY: fallible external calls must be handled with postfix `!` or `catch` without
    /// manufacturing public raw `Result` values.
    pub returns: Vec<ExternalReturnSlot>,
    /// Optional final error slot. This maps to `T!` on source functions.
    pub error_return_type: Option<ExternalSignatureType>,
    /// Optional compiler-owned constant-evaluation operation.
    pub const_eval: Option<ExternalConstEvalOp>,
    /// Backend-specific lowering metadata.
    pub lowerings: ExternalFunctionLowerings,
}

impl Clone for ExternalFunctionDef {
    fn clone(&self) -> Self {
        increment_frontend_counter(FrontendCounter::ExternalFunctionDefinitionCloneCount);
        Self {
            name: self.name.clone(),
            parameters: self.parameters.clone(),
            returns: self.returns.clone(),
            error_return_type: self.error_return_type.clone(),
            const_eval: self.const_eval,
            lowerings: self.lowerings.clone(),
        }
    }
}

impl ExternalFunctionDef {
    pub(crate) fn success_return_type_ids(
        &self,
        type_environment: &mut crate::compiler_frontend::datatypes::environment::TypeEnvironment,
        builtin_error_type_id: crate::compiler_frontend::datatypes::ids::TypeId,
    ) -> Vec<crate::compiler_frontend::datatypes::ids::TypeId> {
        self.returns
            .iter()
            .filter_map(|slot| {
                slot.value_type
                    .to_type_id(type_environment, builtin_error_type_id)
            })
            .collect()
    }

    pub(crate) fn error_return_type_id(
        &self,
        type_environment: &mut crate::compiler_frontend::datatypes::environment::TypeEnvironment,
        builtin_error_type_id: crate::compiler_frontend::datatypes::ids::TypeId,
    ) -> Option<crate::compiler_frontend::datatypes::ids::TypeId> {
        self.error_return_type
            .as_ref()
            .and_then(|error_type| error_type.to_type_id(type_environment, builtin_error_type_id))
    }

    pub(crate) fn is_fallible(&self) -> bool {
        self.error_return_type.is_some()
    }

    /// Borrow-analysis projection for ordered HIR success destinations.
    ///
    /// The compact boundary summary has no per-slot result graph contract, so multiple
    /// results remain Unknown even when their individual metadata says Fresh. Calls,
    /// Invoke edges and wrapper summaries consume this same conservative projection.
    /// This does not alter the registry's ordered slots or fuse them into a value.
    pub(crate) fn hir_return_alias(&self) -> ExternalHirReturnAlias<'_> {
        match self.returns.as_slice() {
            [] => ExternalHirReturnAlias::Fresh,
            [single] => match &single.alias {
                ExternalReturnAlias::Fresh => ExternalHirReturnAlias::Fresh,
                ExternalReturnAlias::AliasArgs(indices) => {
                    ExternalHirReturnAlias::AliasArgs(indices)
                }
            },
            _ => ExternalHirReturnAlias::Unknown,
        }
    }
}

/// Derived borrow-analysis policy, not source-language or registry slot metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExternalHirReturnAlias<'a> {
    Fresh,
    AliasArgs(&'a [usize]),
    Unknown,
}

/// One success-channel return slot for an external function.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExternalReturnSlot {
    pub value_type: ExternalSignatureType,
    pub alias: ExternalReturnAlias,
}

impl ExternalReturnSlot {
    pub fn fresh(value_type: impl Into<ExternalSignatureType>) -> Self {
        Self {
            value_type: value_type.into(),
            alias: ExternalReturnAlias::Fresh,
        }
    }

    pub fn alias_args(
        value_type: impl Into<ExternalSignatureType>,
        parameter_indices: Vec<usize>,
    ) -> Self {
        Self {
            value_type: value_type.into(),
            alias: ExternalReturnAlias::AliasArgs(parameter_indices),
        }
    }
}

/// Builder-friendly signature constructor for one-slot success metadata.
pub fn external_success_returns(
    success_type: impl Into<ExternalSignatureType>,
    success_alias: ExternalReturnAlias,
) -> Vec<ExternalReturnSlot> {
    let success_type = success_type.into();
    if success_type.supports_success_return_type() {
        vec![ExternalReturnSlot {
            value_type: success_type,
            alias: success_alias,
        }]
    } else {
        Vec::new()
    }
}

/// Definition of a single opaque external type exposed by a virtual package.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExternalTypeDef {
    /// Leaf symbol name within its package.
    pub name: String,
    /// Stable package ID rather than a static string so dynamic and built-in packages
    /// share the same identity model.
    pub package_id: ExternalPackageId,
    pub abi_type: ExternalAbiType,
}

/// Compile-time value for an external package constant.
///
/// WHAT: separates materialised constants from raw host registration inputs.
/// WHY: publication and literal bindings already own valid exact values, while raw host
/// inputs need source-use range and finiteness diagnostics before becoming scalar values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExternalConstantValue {
    /// Already validated scalar payload used by literal bindings and published constants.
    Fixed(FixedScalarValue),
    /// Host boundary input retained until projection can report a diagnostic at its source use.
    ///
    /// Unlike `Fixed`, this is not a materialised value: its exact scalar identity and raw
    /// payload may describe an out-of-range `U32` or a non-finite `F64`.
    Registered {
        scalar: FixedScalar,
        raw: ExternalConstantRawValue,
    },
    StringSlice(&'static str),
    Bool(bool),
}

/// Unvalidated host payload families accepted at the external constant registration boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExternalConstantRawValue {
    Unsigned(u64),
    BinaryFloat(f64),
}

impl ExternalConstantValue {
    /// Returns true for scalar values that are valid in const contexts.
    pub fn is_scalar(self) -> bool {
        matches!(
            self,
            Self::Fixed(_) | Self::Registered { .. } | Self::Bool(_)
        )
    }

    /// Returns true when this payload is the representation `data_type` requires.
    ///
    /// Fixed payloads must carry exactly the declared scalar, so a `U32` slot cannot hold an
    /// `I32` value even when the number would fit.
    pub(crate) fn matches_signature_type(self, data_type: &ExternalSignatureType) -> bool {
        match (data_type, self) {
            (ExternalSignatureType::Abi(ExternalAbiType::Bool), Self::Bool(_))
            | (ExternalSignatureType::Abi(ExternalAbiType::Utf8Str), Self::StringSlice(_)) => true,

            (ExternalSignatureType::Abi(ExternalAbiType::Fixed(scalar)), Self::Fixed(value)) => {
                value.scalar() == *scalar
            }
            (
                ExternalSignatureType::Abi(ExternalAbiType::Fixed(signature_scalar)),
                Self::Registered { scalar, raw },
            ) => {
                scalar == *signature_scalar
                    && matches!(
                        (scalar, raw),
                        (FixedScalar::U32, ExternalConstantRawValue::Unsigned(_))
                            | (FixedScalar::F64, ExternalConstantRawValue::BinaryFloat(_))
                    )
            }

            _ => false,
        }
    }
}

/// Definition of a single external constant exposed by a virtual package.
#[derive(Debug, Clone)]
pub struct ExternalConstantDef {
    /// Leaf symbol name within its package.
    pub name: String,
    /// Semantic signature type visible to Moth callers.
    pub data_type: ExternalSignatureType,
    pub value: ExternalConstantValue,
}

/// A single virtual package provided by a project builder.
#[derive(Debug)]
pub struct ExternalPackage {
    pub id: ExternalPackageId,
    pub path: String,
    pub metadata: crate::builder_surface::PackageMetadata,
    /// Path-to-ID surface map for registered functions.
    ///
    /// WHAT: maps the full symbol path inside this package to its stable function ID.
    /// WHY: this is the canonical package surface; full definitions live in the registry's
    /// `functions_by_id` map so the package does not store duplicate clones.
    pub function_ids: HashMap<ExternalSymbolPath, ExternalFunctionId>,
    /// Path-to-ID surface map for registered types.
    pub type_ids: HashMap<ExternalSymbolPath, ExternalTypeId>,
    /// Path-to-ID surface map for registered constants.
    pub constant_ids: HashMap<ExternalSymbolPath, ExternalConstantId>,
}

impl Clone for ExternalPackage {
    fn clone(&self) -> Self {
        increment_frontend_counter(FrontendCounter::ExternalPackageDefinitionCloneCount);
        Self {
            id: self.id,
            path: self.path.clone(),
            metadata: self.metadata,
            function_ids: self.function_ids.clone(),
            type_ids: self.type_ids.clone(),
            constant_ids: self.constant_ids.clone(),
        }
    }
}

impl ExternalPackage {
    pub(crate) fn new(
        id: ExternalPackageId,
        path: impl Into<String>,
        metadata: crate::builder_surface::PackageMetadata,
    ) -> Self {
        Self {
            id,
            path: path.into(),
            metadata,
            function_ids: HashMap::new(),
            type_ids: HashMap::new(),
            constant_ids: HashMap::new(),
        }
    }

    /// Iterates over function symbol paths and their IDs.
    ///
    /// WHAT: exposes the package function surface for namespace-record construction.
    /// WHY: callers should not depend on the internal HashMap representation.
    pub fn function_symbol_ids(
        &self,
    ) -> impl Iterator<Item = (&ExternalSymbolPath, &ExternalFunctionId)> {
        self.function_ids.iter()
    }

    /// Iterates over type symbol paths and their IDs.
    pub fn type_symbol_ids(&self) -> impl Iterator<Item = (&ExternalSymbolPath, &ExternalTypeId)> {
        self.type_ids.iter()
    }

    /// Iterates over constant symbol paths and their IDs.
    pub fn constant_symbol_ids(
        &self,
    ) -> impl Iterator<Item = (&ExternalSymbolPath, &ExternalConstantId)> {
        self.constant_ids.iter()
    }
}

/// Builder-friendly spec for registering an external function.
///
/// WHAT: carries the metadata needed to register a function without forcing
/// the caller to construct the full `ExternalFunctionDef` and pick a stable ID.
/// WHY: builder packages should not need to hardcode `ExternalFunctionId` enum variants.
#[derive(Debug, Clone)]
pub struct ExternalFunctionSpec {
    /// Leaf symbol name within its package.
    pub name: String,
    pub parameters: Vec<ExternalParameter>,
    pub returns: Vec<ExternalReturnSlot>,
    pub error_return_type: Option<ExternalSignatureType>,
    /// Optional compiler-owned constant-evaluation operation.
    pub const_eval: Option<ExternalConstEvalOp>,
    pub lowerings: ExternalFunctionLowerings,
}

impl From<ExternalFunctionSpec> for ExternalFunctionDef {
    fn from(spec: ExternalFunctionSpec) -> Self {
        ExternalFunctionDef {
            name: spec.name,
            parameters: spec.parameters,
            returns: spec.returns,
            error_return_type: spec.error_return_type,
            const_eval: spec.const_eval,
            lowerings: spec.lowerings,
        }
    }
}

/// Builder-friendly spec for registering an external type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExternalTypeSpec {
    /// Leaf symbol name within its package.
    pub name: String,
    pub abi_type: ExternalAbiType,
}
