//! `@core/collections` package registration.

use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::external_packages::external_returns;
use crate::compiler_frontend::external_packages::{
    COLLECTION_GET_HOST_NAME, COLLECTION_LENGTH_HOST_NAME, COLLECTION_PUSH_FIXED_HOST_NAME,
    COLLECTION_PUSH_GROWABLE_HOST_NAME, COLLECTION_REMOVE_HOST_NAME, COLLECTION_SET_HOST_NAME,
    ExternalFunctionId,
};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalAccessKind, ExternalNumericContract, ExternalParameter,
    ExternalReturnAlias, ExternalSignatureType,
};
use crate::compiler_frontend::external_packages::{
    ExternalFunctionDef, ExternalFunctionLowerings, ExternalJsLowering, external_success_returns,
};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

fn inferred_parameter(access_kind: ExternalAccessKind) -> ExternalParameter {
    ExternalParameter {
        language_type: ExternalSignatureType::Abi(ExternalAbiType::Inferred),
        access_kind,
    }
}

fn index_parameter() -> ExternalParameter {
    ExternalParameter {
        language_type: ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::I32)),
        access_kind: ExternalAccessKind::Shared,
    }
}

pub fn register_core_collections_package(registry: &mut ExternalPackageRegistry) {
    // Collection callables keep an `Unknown` numeric contract: the language-owned builtin
    // surface owns each operation's real fallible contract, so these registrations carry
    // placeholder infallible return shapes. Growable push and length are infallible in that
    // surface; get, set, remove and fixed push fail through its `Error!` channel. `get`'s slot
    // uses `Inferred` because the element type is resolved at the call site, and aliases
    // parameter 0 because its result is a shared alias to stored data, not a fresh value. A
    // trusted contract must describe the registered return contract, so it cannot certify these
    // callables without moving the builtin signature first.
    //
    // The package is implicit: collection syntax is language-owned, so no source dependency
    // clause may bind these host functions directly.
    let package_id = registry
        .register_implicit_package(
            "@core/collections",
            crate::builder_surface::PackageOrigin::Core,
        )
        .expect("builtin package registration should not collide");

    // One row per callable so each registered contract stays inspectable in one place.
    let functions = [
        (
            ExternalFunctionId::CollectionGet,
            ExternalFunctionDef {
                name: COLLECTION_GET_HOST_NAME.to_owned(),
                parameters: vec![
                    inferred_parameter(ExternalAccessKind::Shared),
                    index_parameter(),
                ],
                return_contract: external_returns(
                    external_success_returns(
                        ExternalAbiType::Inferred,
                        ExternalReturnAlias::AliasArgs(vec![0]),
                    ),
                    None,
                ),
                const_eval: None,
                numeric_contract: ExternalNumericContract::Unknown,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        COLLECTION_GET_HOST_NAME.to_owned(),
                    )),
                    wasm: None,
                },
            },
        ),
        (
            ExternalFunctionId::CollectionSet,
            ExternalFunctionDef {
                name: COLLECTION_SET_HOST_NAME.to_owned(),
                parameters: vec![
                    inferred_parameter(ExternalAccessKind::Mutable),
                    index_parameter(),
                    inferred_parameter(ExternalAccessKind::Shared),
                ],
                return_contract: external_returns(
                    external_success_returns(ExternalAbiType::Void, ExternalReturnAlias::Fresh),
                    None,
                ),
                const_eval: None,
                numeric_contract: ExternalNumericContract::Unknown,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        COLLECTION_SET_HOST_NAME.to_owned(),
                    )),
                    wasm: None,
                },
            },
        ),
        (
            ExternalFunctionId::CollectionPushGrowable,
            ExternalFunctionDef {
                name: COLLECTION_PUSH_GROWABLE_HOST_NAME.to_owned(),
                parameters: vec![
                    inferred_parameter(ExternalAccessKind::Mutable),
                    inferred_parameter(ExternalAccessKind::Shared),
                ],
                return_contract: external_returns(
                    external_success_returns(ExternalAbiType::Void, ExternalReturnAlias::Fresh),
                    None,
                ),
                const_eval: None,
                numeric_contract: ExternalNumericContract::Unknown,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        COLLECTION_PUSH_GROWABLE_HOST_NAME.to_owned(),
                    )),
                    wasm: None,
                },
            },
        ),
        (
            ExternalFunctionId::CollectionPushFixed,
            ExternalFunctionDef {
                name: COLLECTION_PUSH_FIXED_HOST_NAME.to_owned(),
                parameters: vec![
                    inferred_parameter(ExternalAccessKind::Mutable),
                    inferred_parameter(ExternalAccessKind::Shared),
                ],
                return_contract: external_returns(
                    external_success_returns(ExternalAbiType::Void, ExternalReturnAlias::Fresh),
                    None,
                ),
                const_eval: None,
                numeric_contract: ExternalNumericContract::Unknown,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        COLLECTION_PUSH_FIXED_HOST_NAME.to_owned(),
                    )),
                    wasm: None,
                },
            },
        ),
        (
            ExternalFunctionId::CollectionRemove,
            ExternalFunctionDef {
                name: COLLECTION_REMOVE_HOST_NAME.to_owned(),
                parameters: vec![
                    inferred_parameter(ExternalAccessKind::Mutable),
                    index_parameter(),
                ],
                return_contract: external_returns(
                    external_success_returns(ExternalAbiType::Void, ExternalReturnAlias::Fresh),
                    None,
                ),
                const_eval: None,
                numeric_contract: ExternalNumericContract::Unknown,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        COLLECTION_REMOVE_HOST_NAME.to_owned(),
                    )),
                    wasm: None,
                },
            },
        ),
        (
            ExternalFunctionId::CollectionLength,
            ExternalFunctionDef {
                name: COLLECTION_LENGTH_HOST_NAME.to_owned(),
                parameters: vec![inferred_parameter(ExternalAccessKind::Shared)],
                return_contract: external_returns(
                    external_success_returns(
                        ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::I32)),
                        ExternalReturnAlias::Fresh,
                    ),
                    None,
                ),
                const_eval: None,
                numeric_contract: ExternalNumericContract::Unknown,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        COLLECTION_LENGTH_HOST_NAME.to_owned(),
                    )),
                    wasm: None,
                },
            },
        ),
    ];

    for (id, function) in functions {
        registry
            .register_function_in_package(package_id, id, function)
            .expect("builtin function registration should not collide");
    }
}
