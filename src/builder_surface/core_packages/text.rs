//! `@core/text` package registration.
//!
//! WHAT: registers the minimal text helper surface for builders that opt into it.
//! WHY: text helpers are external package metadata so frontend visibility, type checking,
//! and backend lowering all share one canonical API shape.

use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalAccessKind, ExternalReturnAlias, ExternalSignatureType,
};
use crate::compiler_frontend::external_packages::{
    ExternalFunctionLowerings, ExternalFunctionSpec, ExternalJsLowering, ExternalParameter,
    external_success_returns,
};

pub fn register_core_text_package(registry: &mut ExternalPackageRegistry) {
    let package_id = registry
        .register_package("@core/text", crate::builder_surface::PackageOrigin::Core)
        .expect("builtin package registration should not collide");

    let text_param = ExternalParameter {
        language_type: ExternalSignatureType::Abi(ExternalAbiType::Utf8Str),
        access_kind: ExternalAccessKind::Shared,
    };

    let text_functions: &[(
        &'static str,
        &'static str,
        Vec<ExternalParameter>,
        ExternalAbiType,
    )] = &[
        (
            "length",
            "__moth_text_length",
            vec![text_param.clone()],
            ExternalAbiType::I32,
        ),
        (
            "is_empty",
            "__moth_text_is_empty",
            vec![text_param.clone()],
            ExternalAbiType::Bool,
        ),
        (
            "contains",
            "__moth_text_contains",
            vec![text_param.clone(), text_param.clone()],
            ExternalAbiType::Bool,
        ),
        (
            "starts_with",
            "__moth_text_starts_with",
            vec![text_param.clone(), text_param.clone()],
            ExternalAbiType::Bool,
        ),
        (
            "ends_with",
            "__moth_text_ends_with",
            vec![text_param.clone(), text_param],
            ExternalAbiType::Bool,
        ),
    ];

    for (name, js_name, parameters, return_type) in text_functions {
        registry
            .register_external_function(
                package_id,
                ExternalFunctionSpec {
                    name: (*name).to_owned(),
                    parameters: parameters.clone(),
                    returns: external_success_returns(
                        return_type.clone(),
                        ExternalReturnAlias::Fresh,
                    ),
                    error_return_type: None,
                    lowerings: ExternalFunctionLowerings {
                        js: Some(ExternalJsLowering::RuntimeFunction((*js_name).to_owned())),
                        wasm: None,
                    },
                },
            )
            .expect("builtin text function registration should not collide");
    }
}
