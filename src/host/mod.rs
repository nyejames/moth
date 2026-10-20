//! Native Rust host binding descriptors.
//!
//! WHAT: the closed V1 vocabulary a builder uses to describe one Rust host function: exact
//! scalars except `F16`, `String`, one optional layer and the builtin `Error!` channel.
//! WHY: builders describe host functions without naming compiler-internal signature types. Each
//! descriptor maps one-to-one onto the external-package registry's existing specification, so
//! there is no second signature language or type table.

use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalAccessKind, ExternalFunctionLowerings, ExternalFunctionSpec,
    ExternalParameter, ExternalReturnSlot, ExternalSignatureType,
};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

/// A Moth value type a native host function may accept or return.
///
/// `Byte` and `U8` are distinct identities with the same range. `Optional` allows one layer
/// over any other variant.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostType {
    Bool,
    Char,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Byte,
    String,
    Optional(Box<HostType>),
}

impl HostType {
    /// One optional layer over `inner`.
    pub fn optional(inner: HostType) -> Self {
        Self::Optional(Box::new(inner))
    }

    fn signature_type(&self) -> Result<ExternalSignatureType, &'static str> {
        let fixed = |scalar| Ok(ExternalAbiType::Fixed(scalar).into());
        match self {
            Self::Bool => Ok(ExternalAbiType::Bool.into()),
            Self::Char => Ok(ExternalAbiType::Char.into()),
            Self::I8 => fixed(FixedScalar::I8),
            Self::I16 => fixed(FixedScalar::I16),
            Self::I32 => fixed(FixedScalar::I32),
            Self::I64 => fixed(FixedScalar::I64),
            Self::U8 => fixed(FixedScalar::U8),
            Self::U16 => fixed(FixedScalar::U16),
            Self::U32 => fixed(FixedScalar::U32),
            Self::U64 => fixed(FixedScalar::U64),
            Self::F32 => fixed(FixedScalar::F32),
            Self::F64 => fixed(FixedScalar::F64),
            Self::Byte => fixed(FixedScalar::Byte),
            Self::String => Ok(ExternalAbiType::Utf8Str.into()),
            Self::Optional(inner) if matches!(**inner, Self::Optional(_)) => {
                Err("nests an optional inside an optional")
            }
            Self::Optional(inner) => Ok(ExternalSignatureType::Optional(Box::new(
                inner.signature_type()?,
            ))),
        }
    }
}

/// A manual descriptor for one native Rust host function.
///
/// Parameters are shared reads. A host function returns zero or one success value and may
/// declare the builtin `Error!` channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFunction {
    name: String,
    parameters: Vec<HostType>,
    returns: Vec<HostType>,
    fallible: bool,
}

impl HostFunction {
    /// Describes a host function exposed under `name` in its package.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            parameters: Vec::new(),
            returns: Vec::new(),
            fallible: false,
        }
    }

    /// Appends the next parameter.
    pub fn parameter(mut self, parameter: HostType) -> Self {
        self.parameters.push(parameter);
        self
    }

    /// Declares the success value. A second success value is rejected at registration.
    pub fn returns(mut self, success: HostType) -> Self {
        self.returns.push(success);
        self
    }

    /// Declares the builtin `Error!` channel.
    pub fn fallible(mut self) -> Self {
        self.fallible = true;
        self
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Maps the descriptor onto the registry's function specification.
    pub(crate) fn to_spec(&self) -> Result<ExternalFunctionSpec, &'static str> {
        if self.returns.len() > 1 {
            return Err(
                "declares several success values, but V1 host functions return at most one",
            );
        }
        let parameters = self
            .parameters
            .iter()
            .map(|parameter| {
                Ok(ExternalParameter {
                    language_type: parameter.signature_type()?,
                    access_kind: ExternalAccessKind::Shared,
                })
            })
            .collect::<Result<_, &'static str>>()?;
        let returns = self
            .returns
            .iter()
            .map(|success| Ok(ExternalReturnSlot::fresh(success.signature_type()?)))
            .collect::<Result<_, &'static str>>()?;

        Ok(ExternalFunctionSpec {
            name: self.name.clone(),
            parameters,
            returns,
            error_return_type: self.fallible.then_some(ExternalSignatureType::BuiltinError),
            const_eval: None,
            lowerings: ExternalFunctionLowerings::default(),
        })
    }
}
