//! `@core/collections` package registration.

use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::external_packages::{
    COLLECTION_GET_HOST_NAME, COLLECTION_LENGTH_HOST_NAME, COLLECTION_PUSH_FIXED_HOST_NAME,
    COLLECTION_PUSH_GROWABLE_HOST_NAME, COLLECTION_REMOVE_HOST_NAME, COLLECTION_SET_HOST_NAME,
    ExternalFunctionId,
};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalAccessKind, ExternalReturnAlias, ExternalSignatureType,
};
use crate::compiler_frontend::external_packages::{
    ExternalFunctionDef, ExternalFunctionLowerings, ExternalJsLowering, external_success_returns,
};

pub fn register_core_collections_package(registry: &mut ExternalPackageRegistry) {
    let package_id = registry
        .register_package(
            "@core/collections",
            crate::builder_surface::PackageOrigin::Core,
        )
        .expect("builtin package registration should not collide");

    registry
        .register_function_in_package(
            package_id,
            ExternalFunctionId::CollectionGet,
            ExternalFunctionDef {
                name: COLLECTION_GET_HOST_NAME.to_owned(),
                parameters: vec![
                    crate::compiler_frontend::external_packages::ExternalParameter {
                        language_type: ExternalSignatureType::Abi(ExternalAbiType::Inferred),
                        access_kind: ExternalAccessKind::Shared,
                    },
                    crate::compiler_frontend::external_packages::ExternalParameter {
                        language_type: ExternalSignatureType::Abi(ExternalAbiType::I32),
                        access_kind: ExternalAccessKind::Shared,
                    },
                ],
                returns: external_success_returns(
                    ExternalAbiType::Void,
                    ExternalReturnAlias::Fresh,
                ),
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        "__moth_collection_get".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("builtin function registration should not collide");

    registry
        .register_function_in_package(
            package_id,
            ExternalFunctionId::CollectionSet,
            ExternalFunctionDef {
                name: COLLECTION_SET_HOST_NAME.to_owned(),
                parameters: vec![
                    crate::compiler_frontend::external_packages::ExternalParameter {
                        language_type: ExternalSignatureType::Abi(ExternalAbiType::Inferred),
                        access_kind: ExternalAccessKind::Mutable,
                    },
                    crate::compiler_frontend::external_packages::ExternalParameter {
                        language_type: ExternalSignatureType::Abi(ExternalAbiType::I32),
                        access_kind: ExternalAccessKind::Shared,
                    },
                    crate::compiler_frontend::external_packages::ExternalParameter {
                        language_type: ExternalSignatureType::Abi(ExternalAbiType::Inferred),
                        access_kind: ExternalAccessKind::Shared,
                    },
                ],
                returns: external_success_returns(
                    ExternalAbiType::Void,
                    ExternalReturnAlias::Fresh,
                ),
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        "__moth_collection_set".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("builtin function registration should not collide");

    registry
        .register_function_in_package(
            package_id,
            ExternalFunctionId::CollectionPushGrowable,
            ExternalFunctionDef {
                name: COLLECTION_PUSH_GROWABLE_HOST_NAME.to_owned(),
                parameters: vec![
                    crate::compiler_frontend::external_packages::ExternalParameter {
                        language_type: ExternalSignatureType::Abi(ExternalAbiType::Inferred),
                        access_kind: ExternalAccessKind::Mutable,
                    },
                    crate::compiler_frontend::external_packages::ExternalParameter {
                        language_type: ExternalSignatureType::Abi(ExternalAbiType::Inferred),
                        access_kind: ExternalAccessKind::Shared,
                    },
                ],
                returns: external_success_returns(
                    ExternalAbiType::Void,
                    ExternalReturnAlias::Fresh,
                ),
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        "__moth_collection_push_growable".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("builtin function registration should not collide");

    registry
        .register_function_in_package(
            package_id,
            ExternalFunctionId::CollectionPushFixed,
            ExternalFunctionDef {
                name: COLLECTION_PUSH_FIXED_HOST_NAME.to_owned(),
                parameters: vec![
                    crate::compiler_frontend::external_packages::ExternalParameter {
                        language_type: ExternalSignatureType::Abi(ExternalAbiType::Inferred),
                        access_kind: ExternalAccessKind::Mutable,
                    },
                    crate::compiler_frontend::external_packages::ExternalParameter {
                        language_type: ExternalSignatureType::Abi(ExternalAbiType::Inferred),
                        access_kind: ExternalAccessKind::Shared,
                    },
                ],
                returns: external_success_returns(
                    ExternalAbiType::Void,
                    ExternalReturnAlias::Fresh,
                ),
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        "__moth_collection_push_fixed".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("builtin function registration should not collide");

    registry
        .register_function_in_package(
            package_id,
            ExternalFunctionId::CollectionRemove,
            ExternalFunctionDef {
                name: COLLECTION_REMOVE_HOST_NAME.to_owned(),
                parameters: vec![
                    crate::compiler_frontend::external_packages::ExternalParameter {
                        language_type: ExternalSignatureType::Abi(ExternalAbiType::Inferred),
                        access_kind: ExternalAccessKind::Mutable,
                    },
                    crate::compiler_frontend::external_packages::ExternalParameter {
                        language_type: ExternalSignatureType::Abi(ExternalAbiType::I32),
                        access_kind: ExternalAccessKind::Shared,
                    },
                ],
                returns: external_success_returns(
                    ExternalAbiType::Void,
                    ExternalReturnAlias::Fresh,
                ),
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        "__moth_collection_remove".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("builtin function registration should not collide");

    registry
        .register_function_in_package(
            package_id,
            ExternalFunctionId::CollectionLength,
            ExternalFunctionDef {
                name: COLLECTION_LENGTH_HOST_NAME.to_owned(),
                parameters: vec![
                    crate::compiler_frontend::external_packages::ExternalParameter {
                        language_type: ExternalSignatureType::Abi(ExternalAbiType::Inferred),
                        access_kind: ExternalAccessKind::Shared,
                    },
                ],
                returns: external_success_returns(ExternalAbiType::I32, ExternalReturnAlias::Fresh),
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        "__moth_collection_length".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("builtin function registration should not collide");
}
