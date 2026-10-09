//! External-call binary-float validation selection tests.
//!
//! WHAT: pins `external_call_requires_float_validation`, the single parse-time fact that decides
//!       whether HIR must guard a raw external success value.
//! WHY: the fact must follow the resolved exact binary-float slot and the callee's lowering
//!      metadata, so only raw boundaries and non-exempt lowering shapes request the guard.

use super::external_call_requires_float_validation;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalFunctionDef, ExternalFunctionLowerings, ExternalJsLowering,
    ExternalReturnSlot, ExternalSignatureType, ExternalWasmLowering,
};
use moth_lexical::numeric::fixed_scalar::FixedScalar;

fn fixture(
    returns: Vec<ExternalReturnSlot>,
    js: Option<ExternalJsLowering>,
    wasm: Option<ExternalWasmLowering>,
) -> ExternalFunctionDef {
    ExternalFunctionDef {
        name: "fixture".to_owned(),
        parameters: vec![],
        returns,
        error_return_type: None,
        lowerings: ExternalFunctionLowerings { js, wasm },
    }
}

fn fixed_return(scalar: FixedScalar) -> Vec<ExternalReturnSlot> {
    vec![ExternalReturnSlot::fresh(ExternalAbiType::Fixed(scalar))]
}

fn fixed_type(scalar: FixedScalar) -> TypeId {
    builtin_type_ids::fixed_scalar(scalar)
}

fn export_lowering() -> Option<ExternalJsLowering> {
    Some(ExternalJsLowering::ExternalModuleExport {
        export_name: "rawExport".to_owned(),
    })
}

#[test]
fn native_float_slot_requires_validation_even_for_export_glue() {
    let environment = TypeEnvironment::new();
    let external = fixture(
        vec![ExternalReturnSlot::fresh(
            ExternalSignatureType::NativeFloat,
        )],
        export_lowering(),
        None,
    );

    assert!(
        external_call_requires_float_validation(
            &external,
            &[environment.builtins().float],
            &environment,
        ),
        "the fixed-float glue exemption must not cover a profile-selected native Float slot"
    );
}

#[test]
fn raw_fixed_f64_lowering_requires_validation() {
    let environment = TypeEnvironment::new();
    let external = fixture(
        fixed_return(FixedScalar::F64),
        Some(ExternalJsLowering::RuntimeFunction("helper".to_owned())),
        None,
    );

    assert!(
        external_call_requires_float_validation(
            &external,
            &[fixed_type(FixedScalar::F64)],
            &environment,
        ),
        "a raw runtime-helper lowering owns no finite check, so HIR must validate"
    );
}

#[test]
fn fixed_f32_export_glue_without_raw_wasm_is_exempt() {
    let environment = TypeEnvironment::new();
    let external = fixture(fixed_return(FixedScalar::F32), export_lowering(), None);

    assert!(
        !external_call_requires_float_validation(
            &external,
            &[fixed_type(FixedScalar::F32)],
            &environment,
        ),
        "the generated fixed F32 export wrapper rounds and rejects non-finite values itself"
    );
}

#[test]
fn fixed_f64_export_glue_with_wasm_lowering_is_not_exempt() {
    let environment = TypeEnvironment::new();
    let external = fixture(
        fixed_return(FixedScalar::F64),
        export_lowering(),
        Some(ExternalWasmLowering::HostFunction("host_import")),
    );

    assert!(
        external_call_requires_float_validation(
            &external,
            &[fixed_type(FixedScalar::F64)],
            &environment,
        ),
        "a raw Wasm lowering could bypass the JavaScript wrapper, so the exemption drops"
    );
}

#[test]
fn fixed_f16_export_glue_is_not_exempt() {
    let environment = TypeEnvironment::new();
    let external = fixture(fixed_return(FixedScalar::F16), export_lowering(), None);

    assert!(
        external_call_requires_float_validation(
            &external,
            &[fixed_type(FixedScalar::F16)],
            &environment,
        ),
        "only the delivered fixed F32/F64 glue is known to validate its own result"
    );
}

#[test]
fn non_float_arity_and_empty_results_never_require_validation() {
    let environment = TypeEnvironment::new();
    let integer = fixture(fixed_return(FixedScalar::I32), None, None);
    assert!(!external_call_requires_float_validation(
        &integer,
        &[fixed_type(FixedScalar::I32)],
        &environment,
    ));

    let multi_float = fixture(
        vec![
            ExternalReturnSlot::fresh(ExternalAbiType::Fixed(FixedScalar::F64)),
            ExternalReturnSlot::fresh(ExternalAbiType::Fixed(FixedScalar::F64)),
        ],
        None,
        None,
    );
    assert!(!external_call_requires_float_validation(
        &multi_float,
        &[fixed_type(FixedScalar::F64), fixed_type(FixedScalar::F64)],
        &environment,
    ));

    let single_float = fixture(fixed_return(FixedScalar::F64), None, None);
    assert!(
        !external_call_requires_float_validation(&single_float, &[], &environment),
        "an empty resolved success list is never a single float boundary"
    );
}
