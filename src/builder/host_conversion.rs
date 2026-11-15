//! Maps the shared host descriptors onto the compiler's external-package specification and back.
//!
//! WHAT: converts `moth_host::HostType` and `moth_host::HostFunction` into the
//! `ExternalFunctionSpec` that `BuilderDefinition::finish` registers, and converts the compiler's
//! registered signature types and canonical public type identities back into `HostType` for host
//! requirement production.
//! WHY: the descriptor vocabulary lives in the `moth_host` crate, so the compiler keeps the
//! semantic conversion and its restricted-profile rejections beside their callers rather than in
//! the crate both sides share. Requirement production reads two compiler spellings of the same
//! leaf type, so one table maps the descriptor, ABI and canonical spellings and the directions
//! cannot drift.

use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalAccessKind, ExternalFunctionLowerings, ExternalFunctionSpec,
    ExternalParameter, ExternalReturnSlot, ExternalSignatureType, external_returns,
};
use moth_host::{HostFunction, HostType};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

/// One row of the closed host leaf vocabulary.
///
/// WHAT: one descriptor leaf type with its external-package ABI spelling and its canonical
/// builtin identity.
/// WHY: registration writes the ABI spelling, host import production reads it back, and selected
/// export signatures carry the canonical identity. One row per Moth leaf type keeps those three
/// spellings together, so a type added to one conversion direction cannot be missing from another.
struct HostLeafMapping {
    host: HostType,
    abi: ExternalAbiType,
    canonical: CanonicalBuiltinType,
}

/// Every Moth leaf type the V1 host profile carries, in source-width order.
///
/// `F16`, `Dec`, `Range`, `None`, the builtin `Error` channel and every nominal, collection,
/// record and map type are absent: the native host ABI defers or excludes them.
const HOST_LEAF_MAPPINGS: &[HostLeafMapping] = &[
    HostLeafMapping {
        host: HostType::Bool,
        abi: ExternalAbiType::Bool,
        canonical: CanonicalBuiltinType::Bool,
    },
    HostLeafMapping {
        host: HostType::Char,
        abi: ExternalAbiType::Char,
        canonical: CanonicalBuiltinType::Char,
    },
    HostLeafMapping {
        host: HostType::I8,
        abi: ExternalAbiType::Fixed(FixedScalar::I8),
        canonical: CanonicalBuiltinType::FixedScalar(FixedScalar::I8),
    },
    HostLeafMapping {
        host: HostType::I16,
        abi: ExternalAbiType::Fixed(FixedScalar::I16),
        canonical: CanonicalBuiltinType::FixedScalar(FixedScalar::I16),
    },
    HostLeafMapping {
        host: HostType::I32,
        abi: ExternalAbiType::Fixed(FixedScalar::I32),
        canonical: CanonicalBuiltinType::FixedScalar(FixedScalar::I32),
    },
    HostLeafMapping {
        host: HostType::I64,
        abi: ExternalAbiType::Fixed(FixedScalar::I64),
        canonical: CanonicalBuiltinType::FixedScalar(FixedScalar::I64),
    },
    HostLeafMapping {
        host: HostType::U8,
        abi: ExternalAbiType::Fixed(FixedScalar::U8),
        canonical: CanonicalBuiltinType::FixedScalar(FixedScalar::U8),
    },
    HostLeafMapping {
        host: HostType::U16,
        abi: ExternalAbiType::Fixed(FixedScalar::U16),
        canonical: CanonicalBuiltinType::FixedScalar(FixedScalar::U16),
    },
    HostLeafMapping {
        host: HostType::U32,
        abi: ExternalAbiType::Fixed(FixedScalar::U32),
        canonical: CanonicalBuiltinType::FixedScalar(FixedScalar::U32),
    },
    HostLeafMapping {
        host: HostType::U64,
        abi: ExternalAbiType::Fixed(FixedScalar::U64),
        canonical: CanonicalBuiltinType::FixedScalar(FixedScalar::U64),
    },
    HostLeafMapping {
        host: HostType::F32,
        abi: ExternalAbiType::Fixed(FixedScalar::F32),
        canonical: CanonicalBuiltinType::FixedScalar(FixedScalar::F32),
    },
    HostLeafMapping {
        host: HostType::F64,
        abi: ExternalAbiType::Fixed(FixedScalar::F64),
        canonical: CanonicalBuiltinType::FixedScalar(FixedScalar::F64),
    },
    HostLeafMapping {
        host: HostType::Byte,
        abi: ExternalAbiType::Fixed(FixedScalar::Byte),
        canonical: CanonicalBuiltinType::FixedScalar(FixedScalar::Byte),
    },
    HostLeafMapping {
        host: HostType::String,
        abi: ExternalAbiType::Utf8Str,
        canonical: CanonicalBuiltinType::String,
    },
];

/// Maps one descriptor onto the registry's function specification.
///
/// A rejection reason is returned unformatted so `BuilderDefinition::finish` can attribute it to
/// the function and package the builder declared.
pub(super) fn external_function_spec(
    function: &HostFunction,
) -> Result<ExternalFunctionSpec, &'static str> {
    let signature = function.signature();
    if signature.success_values().len() > 1 {
        return Err("declares several success values, but host functions return at most one");
    }
    let parameters = signature
        .parameters()
        .iter()
        .map(|parameter| {
            Ok(ExternalParameter {
                language_type: signature_type(parameter)?,
                access_kind: ExternalAccessKind::Shared,
            })
        })
        .collect::<Result<_, &'static str>>()?;
    let returns = signature
        .success_values()
        .iter()
        .map(|success| Ok(ExternalReturnSlot::fresh(signature_type(success)?)))
        .collect::<Result<_, &'static str>>()?;

    Ok(ExternalFunctionSpec {
        name: function.name().to_owned(),
        parameters,
        return_contract: external_returns(
            returns,
            signature
                .is_fallible()
                .then_some(ExternalSignatureType::BuiltinError),
        ),
        const_eval: None,
        lowerings: ExternalFunctionLowerings::default(),
    })
}

/// Maps one registered signature type onto the descriptor vocabulary.
///
/// Returns `None` when the type has no V1 host spelling, which includes the builtin error
/// channel, a binding-backed opaque type and the raw ABI carriers. The caller owns what that
/// means for its boundary.
pub(super) fn host_type_of_signature_type(
    signature_type: &ExternalSignatureType,
) -> Option<HostType> {
    match signature_type {
        ExternalSignatureType::Abi(abi_type) => host_type_of_abi_type(abi_type),
        // `StringContent` is a language-level string policy rather than a distinct Moth type, so
        // a registered signature that uses it still names the Moth `String` type.
        ExternalSignatureType::StringContent => Some(HostType::String),
        ExternalSignatureType::Optional(inner) => {
            optional_host_type(host_type_of_signature_type(inner)?)
        }
        ExternalSignatureType::BuiltinError | ExternalSignatureType::External(_) => None,
    }
}

/// Maps one canonical public type identity onto the descriptor vocabulary.
///
/// The facade's public semantic interface carries canonical identities, so selected export
/// signatures read their types here rather than through the registry.
pub(super) fn host_type_of_canonical_type(identity: &CanonicalTypeIdentity) -> Option<HostType> {
    match identity {
        CanonicalTypeIdentity::Builtin(builtin) => host_type_of_canonical_builtin(builtin),
        CanonicalTypeIdentity::Option(inner) => {
            optional_host_type(host_type_of_canonical_type(inner)?)
        }
        _ => None,
    }
}

/// Maps one descriptor type onto the registry's signature type.
fn signature_type(host_type: &HostType) -> Result<ExternalSignatureType, &'static str> {
    if let Some(abi_type) = abi_type_of_host_type(host_type) {
        return Ok(abi_type.into());
    }

    match host_type {
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

/// The single optional layer the V1 host profile carries.
fn optional_host_type(inner: HostType) -> Option<HostType> {
    // A second layer has no V1 spelling, so nesting is rejected rather than flattened.
    if matches!(inner, HostType::Optional(_)) {
        return None;
    }
    Some(HostType::optional(inner))
}

fn abi_type_of_host_type(host_type: &HostType) -> Option<ExternalAbiType> {
    HOST_LEAF_MAPPINGS
        .iter()
        .find(|mapping| mapping.host == *host_type)
        .map(|mapping| mapping.abi.clone())
}

fn host_type_of_abi_type(abi_type: &ExternalAbiType) -> Option<HostType> {
    HOST_LEAF_MAPPINGS
        .iter()
        .find(|mapping| mapping.abi == *abi_type)
        .map(|mapping| mapping.host.clone())
}

fn host_type_of_canonical_builtin(builtin: &CanonicalBuiltinType) -> Option<HostType> {
    HOST_LEAF_MAPPINGS
        .iter()
        .find(|mapping| mapping.canonical == *builtin)
        .map(|mapping| mapping.host.clone())
}
