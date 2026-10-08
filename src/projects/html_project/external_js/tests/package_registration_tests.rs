//! Registration invariants for parsed JavaScript scalar signatures.

use super::*;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::projects::html_project::external_js::parser::parse_js_module;
use crate::projects::html_project::external_js::runtime_module_registry::RuntimeModuleRegistry;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

#[test]
fn js_numeric_annotations_remain_native_moth_signatures() {
    let opaque_types = HashMap::new();
    for (name, expected) in [
        ("Int", ExternalSignatureType::NativeInt),
        ("Uint", ExternalSignatureType::NativeUint),
        ("Float", ExternalSignatureType::NativeFloat),
    ] {
        assert_eq!(
            parsed_type_to_signature_type(name, &opaque_types).expect("native annotation resolves"),
            expected,
        );
    }
}

#[test]
fn parsed_fixed_scalar_functions_register_canonical_parameter_and_result_types() {
    for (scalar, native) in [
        (FixedScalar::U32, ExternalSignatureType::NativeUint),
        (FixedScalar::F32, ExternalSignatureType::NativeFloat),
    ] {
        for error_slot in ["", ", Error!"] {
            let source = format!(
                "/** @moth.sig identity |value {}| -> {}{error_slot} */\nexport function identity(value) {{ return value; }}",
                scalar.name(),
                scalar.name()
            );
            let parsed = parse_js_module(&source, &RuntimeModuleRegistry::v1());
            assert!(parsed.diagnostics.is_empty());
            let mut registry = ExternalPackageRegistry::new();
            let package_id = registry
                .register_package(
                    "@test/fixed",
                    crate::builder_surface::PackageOrigin::ProjectLocal,
                )
                .expect("test package registers");
            let registered = register_parsed_js_module(package_id, &parsed, &mut registry)
                .expect("fixed scalar module registers");
            let function = registry
                .get_function_by_id(registered.exported_free_functions[0])
                .expect("registered function resolves");
            let fixed = ExternalSignatureType::Abi(ExternalAbiType::Fixed(scalar));
            assert_eq!(function.parameters[0].language_type, fixed);
            assert_eq!(function.returns[0].value_type, fixed);
            assert_eq!(function.error_return_type.is_some(), !error_slot.is_empty());
            assert!(function.lowerings.wasm.is_none());
            let mut environment = TypeEnvironment::new();
            assert_eq!(
                fixed.to_parameter_type_id(&mut environment),
                Some(builtin_type_ids::fixed_scalar(scalar))
            );
            assert_ne!(
                fixed.to_parameter_type_id(&mut environment),
                native.to_parameter_type_id(&mut environment)
            );
        }
    }
}
