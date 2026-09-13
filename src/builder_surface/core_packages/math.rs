//! `@core/math` package registration.

use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalAccessKind, ExternalReturnAlias, ExternalSignatureType,
};
use crate::compiler_frontend::external_packages::{
    ExternalConstantDef, ExternalConstantValue, ExternalFunctionLowerings, ExternalFunctionSpec,
    ExternalJsLowering, ExternalParameter, external_success_returns,
};

struct MathFunctionSpec {
    name: &'static str,
    js_lowering: &'static str,
    parameter_count: usize,
}

pub fn register_core_math_package(registry: &mut ExternalPackageRegistry) {
    let package_id = registry
        .register_package("@core/math", crate::builder_surface::PackageOrigin::Core)
        .expect("builtin package registration should not collide");

    let math_f64_param = || ExternalParameter {
        language_type: ExternalSignatureType::Abi(ExternalAbiType::F64),
        access_kind: ExternalAccessKind::Shared,
    };

    let math_functions = [
        MathFunctionSpec {
            name: "sin",
            js_lowering: "Math.sin(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "cos",
            js_lowering: "Math.cos(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "tan",
            js_lowering: "Math.tan(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "atan2",
            js_lowering: "Math.atan2(#0, #1)",
            parameter_count: 2, // y, x
        },
        MathFunctionSpec {
            name: "log",
            js_lowering: "Math.log(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "log2",
            js_lowering: "Math.log2(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "log10",
            js_lowering: "Math.log10(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "exp",
            js_lowering: "Math.exp(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "pow",
            js_lowering: "Math.pow(#0, #1)",
            parameter_count: 2, // base, exponent
        },
        MathFunctionSpec {
            name: "sqrt",
            js_lowering: "Math.sqrt(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "abs",
            js_lowering: "Math.abs(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "floor",
            js_lowering: "Math.floor(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "ceil",
            js_lowering: "Math.ceil(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "round",
            js_lowering: "Math.round(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "trunc",
            js_lowering: "Math.trunc(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "min",
            js_lowering: "Math.min(#0, #1)",
            parameter_count: 2, // a, b
        },
        MathFunctionSpec {
            name: "max",
            js_lowering: "Math.max(#0, #1)",
            parameter_count: 2, // a, b
        },
        MathFunctionSpec {
            name: "clamp",
            js_lowering: "Math.min(Math.max(#0, #1), #2)",
            parameter_count: 3, // x, min, max
        },
        MathFunctionSpec {
            name: "asin",
            js_lowering: "Math.asin(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "acos",
            js_lowering: "Math.acos(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "atan",
            js_lowering: "Math.atan(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "cbrt",
            js_lowering: "Math.cbrt(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "hypot",
            js_lowering: "Math.hypot(#0, #1)",
            parameter_count: 2, // x, y
        },
        MathFunctionSpec {
            name: "expm1",
            js_lowering: "Math.expm1(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "log1p",
            js_lowering: "Math.log1p(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "sinh",
            js_lowering: "Math.sinh(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "cosh",
            js_lowering: "Math.cosh(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "tanh",
            js_lowering: "Math.tanh(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "asinh",
            js_lowering: "Math.asinh(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "acosh",
            js_lowering: "Math.acosh(#0)",
            parameter_count: 1, // x
        },
        MathFunctionSpec {
            name: "atanh",
            js_lowering: "Math.atanh(#0)",
            parameter_count: 1, // x
        },
    ];

    for function in math_functions {
        let parameters: Vec<ExternalParameter> = (0..function.parameter_count)
            .map(|_| math_f64_param())
            .collect();

        registry
            .register_external_function(
                package_id,
                ExternalFunctionSpec {
                    name: function.name.to_owned(),
                    parameters,
                    returns: external_success_returns(
                        ExternalAbiType::F64,
                        ExternalReturnAlias::Fresh,
                    ),
                    error_return_type: None,
                    lowerings: ExternalFunctionLowerings {
                        js: Some(ExternalJsLowering::InlineExpression(
                            function.js_lowering.to_owned(),
                        )),
                        wasm: None,
                    },
                },
            )
            .expect("builtin math function registration should not collide");
    }

    let math_constants = [
        ("PI", ExternalConstantValue::Float(std::f64::consts::PI)),
        ("TAU", ExternalConstantValue::Float(std::f64::consts::TAU)),
        ("E", ExternalConstantValue::Float(std::f64::consts::E)),
        (
            "SQRT_2",
            ExternalConstantValue::Float(std::f64::consts::SQRT_2),
        ),
        ("LN_2", ExternalConstantValue::Float(std::f64::consts::LN_2)),
        (
            "LN_10",
            ExternalConstantValue::Float(std::f64::consts::LN_10),
        ),
    ];

    for (name, value) in math_constants {
        registry
            .register_external_constant(
                package_id,
                ExternalConstantDef {
                    name: name.to_owned(),
                    data_type: ExternalAbiType::F64,
                    value,
                },
            )
            .expect("builtin math constant registration should not collide");
    }
}
