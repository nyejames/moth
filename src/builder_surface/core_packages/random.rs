//! `@core/random` package registration.
//!
//! WHAT: registers a minimal random-number surface for builders that opt into it.
//! WHY: this proves optional core external packages can grow without making the compiler
//! assume every builder supports them.

use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::external_packages::{
    ExternalAccessKind, ExternalReturnAlias, ExternalSignatureType,
};
use crate::compiler_frontend::external_packages::{
    ExternalFunctionLowerings, ExternalFunctionSpec, ExternalJsLowering, ExternalParameter,
    external_success_returns,
};

pub fn register_core_random_package(registry: &mut ExternalPackageRegistry) {
    let package_id = registry
        .register_package("@core/random", crate::builder_surface::PackageOrigin::Core)
        .expect("builtin package registration should not collide");

    registry
        .register_external_function(
            package_id,
            ExternalFunctionSpec {
                name: "random_float".to_owned(),
                parameters: Vec::new(),
                returns: external_success_returns(
                    ExternalSignatureType::NativeFloat,
                    ExternalReturnAlias::Fresh,
                ),
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        "__moth_random_float".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("builtin random_float registration should not collide");

    let int_param = ExternalParameter {
        language_type: ExternalSignatureType::NativeInt,
        access_kind: ExternalAccessKind::Shared,
    };

    registry
        .register_external_function(
            package_id,
            ExternalFunctionSpec {
                name: "random_int".to_owned(),
                parameters: vec![int_param.clone(), int_param],
                returns: external_success_returns(
                    ExternalSignatureType::NativeInt,
                    ExternalReturnAlias::Fresh,
                ),
                error_return_type: None,
                lowerings: ExternalFunctionLowerings {
                    js: Some(ExternalJsLowering::RuntimeFunction(
                        "__moth_random_int".to_owned(),
                    )),
                    wasm: None,
                },
            },
        )
        .expect("builtin random_int registration should not collide");
}
