//! Maps the shared host descriptors onto the compiler's external-package specification.
//!
//! WHAT: converts `moth_host::HostType` and `moth_host::HostFunction` into the
//! `ExternalFunctionSpec` that `BuilderDefinition::finish` registers.
//! WHY: the descriptors live in the `moth_host` crate, so the compiler keeps the semantic
//! conversion and its restricted-profile rejections beside their only caller rather than in the
//! crate both sides share.

use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalAccessKind, ExternalFunctionLowerings, ExternalFunctionSpec,
    ExternalParameter, ExternalReturnSlot, ExternalSignatureType, external_returns,
};
use moth_host::{HostFunction, HostType};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

/// Maps one descriptor onto the registry's function specification.
///
/// A rejection reason is returned unformatted so `BuilderDefinition::finish` can attribute it to
/// the function and package the builder declared.
pub(super) fn external_function_spec(
    function: &HostFunction,
) -> Result<ExternalFunctionSpec, &'static str> {
    if function.success_values().len() > 1 {
        return Err("declares several success values, but host functions return at most one");
    }
    let parameters = function
        .parameters()
        .iter()
        .map(|parameter| {
            Ok(ExternalParameter {
                language_type: signature_type(parameter)?,
                access_kind: ExternalAccessKind::Shared,
            })
        })
        .collect::<Result<_, &'static str>>()?;
    let returns = function
        .success_values()
        .iter()
        .map(|success| Ok(ExternalReturnSlot::fresh(signature_type(success)?)))
        .collect::<Result<_, &'static str>>()?;

    Ok(ExternalFunctionSpec {
        name: function.name().to_owned(),
        parameters,
        return_contract: external_returns(
            returns,
            function
                .is_fallible()
                .then_some(ExternalSignatureType::BuiltinError),
        ),
        const_eval: None,
        lowerings: ExternalFunctionLowerings::default(),
    })
}

/// Maps one descriptor type onto the registry's signature type.
fn signature_type(host_type: &HostType) -> Result<ExternalSignatureType, &'static str> {
    let fixed = |scalar| Ok(ExternalAbiType::Fixed(scalar).into());
    match host_type {
        HostType::Bool => Ok(ExternalAbiType::Bool.into()),
        HostType::Char => Ok(ExternalAbiType::Char.into()),
        HostType::I8 => fixed(FixedScalar::I8),
        HostType::I16 => fixed(FixedScalar::I16),
        HostType::I32 => fixed(FixedScalar::I32),
        HostType::I64 => fixed(FixedScalar::I64),
        HostType::U8 => fixed(FixedScalar::U8),
        HostType::U16 => fixed(FixedScalar::U16),
        HostType::U32 => fixed(FixedScalar::U32),
        HostType::U64 => fixed(FixedScalar::U64),
        HostType::F32 => fixed(FixedScalar::F32),
        HostType::F64 => fixed(FixedScalar::F64),
        HostType::Byte => fixed(FixedScalar::Byte),
        HostType::String => Ok(ExternalAbiType::Utf8Str.into()),
        HostType::Optional(inner) if matches!(**inner, HostType::Optional(_)) => {
            Err("nests an optional inside an optional")
        }
        HostType::Optional(inner) => Ok(ExternalSignatureType::Optional(Box::new(signature_type(
            inner,
        )?))),
        // `HostType` is non-exhaustive, so a variant added later must be classified here
        // deliberately before the host profile accepts it.
        _ => Err("uses a host type outside the supported vocabulary"),
    }
}
