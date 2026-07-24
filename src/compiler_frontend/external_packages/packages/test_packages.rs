//! Test-only synthetic package registration.

use super::super::abi::{
    ExternalAbiType, ExternalAccessKind, ExternalReturnAlias, ExternalSignatureType,
};
use super::super::definitions::{
    ExternalConstantDef, ExternalConstantValue, ExternalFunctionDef, ExternalFunctionLowerings,
    ExternalJsLowering, ExternalReturnSlot, ExternalTypeDef, external_success_returns,
};
use super::super::ids::{ExternalConstantId, ExternalFunctionId, ExternalTypeId};
use super::super::registry::ExternalPackageRegistry;
use super::super::symbol_path::ExternalSymbolPath;

/// Registers test packages `@test/pkg-a` and `@test/pkg-b` with a duplicate
/// symbol name for integration-test coverage of package-scoped resolution.
pub(crate) fn register_test_packages_for_integration(registry: &mut ExternalPackageRegistry) {
    let pkg_a_id = registry
        .register_package(
            "@test/pkg-a",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test package registration should not collide");
    registry
        .register_type_in_package(
            pkg_a_id,
            ExternalTypeId(1005),
            ExternalTypeDef {
                name: "PkgError".to_owned(),
                package_id: pkg_a_id,
                abi_type: ExternalAbiType::Handle,
            },
        )
        .expect("test external type registration should not collide");
    registry
        .register_function_in_package(
            pkg_a_id,
            ExternalFunctionId::Synthetic(1000),
            ExternalFunctionDef {
                name: "open".to_owned(),
                parameters: vec![super::super::abi::ExternalParameter {
                    language_type: ExternalSignatureType::Abi(ExternalAbiType::Inferred),
                    access_kind: ExternalAccessKind::Shared,
                }],
                returns: external_success_returns(
                    ExternalAbiType::Void,
                    ExternalReturnAlias::Fresh,
                ),
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        "__moth_test_pkg_a_open".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("test function registration should not collide");

    registry
        .register_function_in_package(
            pkg_a_id,
            ExternalFunctionId::Synthetic(1003),
            ExternalFunctionDef {
                name: "fallible_text_ok".to_owned(),
                parameters: vec![super::super::abi::ExternalParameter {
                    language_type: ExternalSignatureType::Abi(ExternalAbiType::Utf8Str),
                    access_kind: ExternalAccessKind::Shared,
                }],
                returns: vec![ExternalReturnSlot::fresh(ExternalAbiType::Utf8Str)],
                error_return_type: Some(ExternalSignatureType::BuiltinError),
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::InlineExpression(
                        "({ tag: \"ok\", value: #0 })".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("test fallible function registration should not collide");

    registry
        .register_function_in_package(
            pkg_a_id,
            ExternalFunctionId::Synthetic(1004),
            ExternalFunctionDef {
                name: "fallible_text_err".to_owned(),
                parameters: Vec::new(),
                returns: vec![ExternalReturnSlot::fresh(ExternalAbiType::Utf8Str)],
                error_return_type: Some(ExternalSignatureType::BuiltinError),
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::InlineExpression(
                        "({ tag: \"err\", value: { message: \"external failed\", code: 91 } })"
                            .to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("test fallible function registration should not collide");

    registry
        .register_function_in_package(
            pkg_a_id,
            ExternalFunctionId::Synthetic(1006),
            ExternalFunctionDef {
                name: "fallible_custom_error_ok".to_owned(),
                parameters: Vec::new(),
                returns: vec![ExternalReturnSlot::fresh(ExternalAbiType::Utf8Str)],
                error_return_type: Some(ExternalSignatureType::External(ExternalTypeId(1005))),
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::InlineExpression(
                        "({ tag: \"ok\", value: \"custom-ok\" })".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("test custom-error fallible function registration should not collide");

    let pkg_b_id = registry
        .register_package(
            "@test/pkg-b",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test package registration should not collide");
    registry
        .register_function_in_package(
            pkg_b_id,
            ExternalFunctionId::Synthetic(1001),
            ExternalFunctionDef {
                name: "open".to_owned(),
                parameters: vec![super::super::abi::ExternalParameter {
                    language_type: ExternalSignatureType::Abi(ExternalAbiType::Inferred),
                    access_kind: ExternalAccessKind::Shared,
                }],
                returns: external_success_returns(
                    ExternalAbiType::Void,
                    ExternalReturnAlias::Fresh,
                ),
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        "__moth_test_pkg_b_open".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("test function registration should not collide");

    registry
        .register_constant_in_package(
            pkg_b_id,
            ExternalConstantId(1002),
            ExternalConstantDef {
                name: "TEST_NON_SCALAR_CONST".to_owned(),
                data_type: ExternalAbiType::Utf8Str,
                value: ExternalConstantValue::StringSlice("test"),
            },
        )
        .expect("test constant registration should not collide");

    let nested_id = registry
        .register_package(
            "@test/nested",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test nested package registration should not collide");

    registry
        .register_type_at_path(
            nested_id,
            ExternalSymbolPath::from_components(vec!["tools".to_owned(), "Opaque".to_owned()]),
            ExternalTypeId(1010),
            ExternalTypeDef {
                name: "Opaque".to_owned(),
                package_id: nested_id,
                abi_type: ExternalAbiType::Handle,
            },
        )
        .expect("test nested type registration should not collide");

    registry
        .register_constant_at_path(
            nested_id,
            ExternalSymbolPath::from_components(vec!["tools".to_owned(), "PI".to_owned()]),
            ExternalConstantId(1011),
            ExternalConstantDef {
                name: "PI".to_owned(),
                data_type: ExternalAbiType::F64,
                value: ExternalConstantValue::Float(3.15),
            },
        )
        .expect("test nested constant registration should not collide");

    registry
        .register_function_at_path(
            nested_id,
            ExternalSymbolPath::from_components(vec!["tools".to_owned(), "greet".to_owned()]),
            ExternalFunctionId::Synthetic(1012),
            ExternalFunctionDef {
                name: "greet".to_owned(),
                parameters: Vec::new(),
                returns: vec![ExternalReturnSlot::fresh(ExternalAbiType::Utf8Str)],
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::InlineExpression(
                        "\"hello nested\"".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("test nested function registration should not collide");

    let string_content_id = registry
        .register_package(
            "@test/string_content",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test string-content package registration should not collide");

    registry
        .register_function_in_package(
            string_content_id,
            ExternalFunctionId::Synthetic(1100),
            ExternalFunctionDef {
                name: "echo".to_owned(),
                parameters: vec![super::super::abi::ExternalParameter {
                    language_type: ExternalSignatureType::StringContent,
                    access_kind: ExternalAccessKind::Shared,
                }],
                returns: vec![ExternalReturnSlot::fresh(ExternalAbiType::Utf8Str)],
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::InlineExpression("#0".to_owned())),
                    wasm: None,
                },
            },
        )
        .expect("test string-content function registration should not collide");

    let optional_id = registry
        .register_package(
            "@test/optional",
            crate::builder_surface::PackageOrigin::Builder,
        )
        .expect("test optional package registration should not collide");

    registry
        .register_function_at_path(
            optional_id,
            ExternalSymbolPath::from_single("present_text"),
            ExternalFunctionId::Synthetic(1200),
            ExternalFunctionDef {
                name: "present_text".to_owned(),
                parameters: Vec::new(),
                returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::Optional(
                    Box::new(ExternalSignatureType::Abi(ExternalAbiType::Utf8Str)),
                ))],
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::InlineExpression(
                        "({ tag: \"some\", value: \"present\" })".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("test optional present function registration should not collide");

    registry
        .register_function_at_path(
            optional_id,
            ExternalSymbolPath::from_single("absent_text"),
            ExternalFunctionId::Synthetic(1201),
            ExternalFunctionDef {
                name: "absent_text".to_owned(),
                parameters: Vec::new(),
                returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::Optional(
                    Box::new(ExternalSignatureType::Abi(ExternalAbiType::Utf8Str)),
                ))],
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::InlineExpression(
                        "({ tag: \"none\" })".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("test optional absent function registration should not collide");
}
