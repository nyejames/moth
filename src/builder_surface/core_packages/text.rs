//! `@core/text` package registration.
//!
//! WHAT: registers the minimal text helper surface for builders that opt into it.
//! WHY: text helpers are external package metadata so frontend visibility, type checking,
//! and backend lowering all share one canonical API shape.

use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::external_packages::external_returns;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalAccessKind, ExternalReturnAlias, ExternalSignatureType,
};
use crate::compiler_frontend::external_packages::{
    ExternalConstEvalOp, ExternalFunctionLowerings, ExternalFunctionSpec, ExternalJsLowering,
    ExternalNumericContract, ExternalParameter, TextConstEvalOp, external_success_returns,
    trusted_numeric_contract,
};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

pub fn register_core_text_package(registry: &mut ExternalPackageRegistry) {
    let package_id = registry
        .register_package("@core/text", crate::builder_surface::PackageOrigin::Core)
        .expect("builtin package registration should not collide");

    let text_param = ExternalParameter {
        language_type: ExternalSignatureType::Abi(ExternalAbiType::Utf8Str),
        access_kind: ExternalAccessKind::Shared,
    };

    struct TextFunctionSpec {
        name: &'static str,
        js_name: &'static str,
        parameters: Vec<ExternalParameter>,
        return_type: ExternalSignatureType,
        const_eval: TextConstEvalOp,
        /// Closed numeric evidence for this callable. Every registered text helper is total and
        /// ordinary, so each row states the same contract explicitly.
        numeric_contract: ExternalNumericContract,
    }

    let text_functions = [
        TextFunctionSpec {
            name: "length",
            js_name: "__moth_text_length",
            parameters: vec![text_param.clone()],
            return_type: ExternalSignatureType::Abi(ExternalAbiType::Fixed(FixedScalar::I32)),
            const_eval: TextConstEvalOp::Length,
            // The helper's I32 carrier guard is unreachable for host-representable strings and is
            // not a Moth trap or error channel.
            numeric_contract: trusted_numeric_contract(false, false, false),
        },
        TextFunctionSpec {
            name: "is_empty",
            js_name: "__moth_text_is_empty",
            parameters: vec![text_param.clone()],
            return_type: ExternalSignatureType::Abi(ExternalAbiType::Bool),
            const_eval: TextConstEvalOp::IsEmpty,
            numeric_contract: trusted_numeric_contract(false, false, false),
        },
        TextFunctionSpec {
            name: "contains",
            js_name: "__moth_text_contains",
            parameters: vec![text_param.clone(), text_param.clone()],
            return_type: ExternalSignatureType::Abi(ExternalAbiType::Bool),
            const_eval: TextConstEvalOp::Contains,
            numeric_contract: trusted_numeric_contract(false, false, false),
        },
        TextFunctionSpec {
            name: "starts_with",
            js_name: "__moth_text_starts_with",
            parameters: vec![text_param.clone(), text_param.clone()],
            return_type: ExternalSignatureType::Abi(ExternalAbiType::Bool),
            const_eval: TextConstEvalOp::StartsWith,
            numeric_contract: trusted_numeric_contract(false, false, false),
        },
        TextFunctionSpec {
            name: "ends_with",
            js_name: "__moth_text_ends_with",
            parameters: vec![text_param.clone(), text_param],
            return_type: ExternalSignatureType::Abi(ExternalAbiType::Bool),
            const_eval: TextConstEvalOp::EndsWith,
            numeric_contract: trusted_numeric_contract(false, false, false),
        },
    ];

    for function in text_functions {
        registry
            .register_external_function(
                package_id,
                ExternalFunctionSpec {
                    name: function.name.to_owned(),
                    parameters: function.parameters,
                    return_contract: external_returns(
                        external_success_returns(function.return_type, ExternalReturnAlias::Fresh),
                        None,
                    ),
                    const_eval: Some(ExternalConstEvalOp::Text(function.const_eval)),
                    numeric_contract: function.numeric_contract,
                    lowerings: ExternalFunctionLowerings {
                        js: Some(ExternalJsLowering::RuntimeFunction(
                            function.js_name.to_owned(),
                        )),
                        wasm: None,
                    },
                },
            )
            .expect("builtin text function registration should not collide");
    }
}
