//! Shared helper for registering parsed JS module symbols in `ExternalPackageRegistry`.
//!
//! WHAT: converts `ParsedJsModule` into registry entries, used by both the JS external import
//!       provider and built-in package registration.
//! WHY: avoids duplicating the conversion logic between project-local `.js` imports and
//!      builder-owned packages such as `@web/canvas`.

use crate::builder_surface::external_import_providers::provider::RequiredRuntimeImport;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalAccessKind, ExternalFunctionLowerings, ExternalFunctionSpec,
    ExternalJsLowering, ExternalPackageId, ExternalPackageRegistry, ExternalParameter,
    ExternalReturnSlot, ExternalSignatureType, ExternalTypeSpec,
};
use crate::projects::html_project::external_js::parser::parsed_js_module::{
    ParsedJsFunction, ParsedJsModule, ParsedSignature,
};
use std::collections::HashMap;

/// Result of registering a parsed JS module in the external package registry.
///
/// WHAT: carries the assigned IDs produced from parsed JS annotations.
/// WHY: the caller (provider or built-in registration) still owns package path, origin,
///      and runtime asset metadata.
pub struct RegisteredJsModule {
    pub exported_types: Vec<crate::compiler_frontend::external_packages::ExternalTypeId>,
    pub exported_free_functions:
        Vec<crate::compiler_frontend::external_packages::ExternalFunctionId>,
}

/// Registers parsed JS module symbols into the external package registry.
///
/// WHAT: converts parsed opaque types and free functions into registry entries with
///       `ExternalModuleExport` JS lowerings.
/// WHY: shared between the JS external import provider and built-in package registration,
///      while keeping external packages on one free-function-only surface.
pub fn register_parsed_js_module(
    package_id: ExternalPackageId,
    parsed: &ParsedJsModule,
    registry: &mut ExternalPackageRegistry,
) -> Result<RegisteredJsModule, CompilerError> {
    let mut type_id_by_opaque_name: HashMap<
        String,
        crate::compiler_frontend::external_packages::ExternalTypeId,
    > = HashMap::new();
    let mut exported_types = Vec::new();

    for opaque in &parsed.opaque_types {
        let type_id = registry.register_external_type(
            package_id,
            ExternalTypeSpec {
                name: opaque.name.clone(),
                abi_type: ExternalAbiType::Handle,
            },
        )?;
        type_id_by_opaque_name.insert(opaque.name.clone(), type_id);
        exported_types.push(type_id);
    }

    let mut exported_free_functions = Vec::new();

    if let Some(receiver_method) = parsed.receiver_methods.first() {
        return Err(CompilerError::compiler_error(format!(
            "JS package registration reached receiver-style signature '{}'. External packages must expose free functions and opaque types only.",
            receiver_method.moth_name
        )));
    }

    for function in &parsed.free_functions {
        let spec = convert_parsed_function_to_spec(function, &type_id_by_opaque_name)?;
        let function_id = registry.register_external_function(package_id, spec)?;
        exported_free_functions.push(function_id);
    }

    Ok(RegisteredJsModule {
        exported_types,
        exported_free_functions,
    })
}

/// Converts parser-recorded runtime imports into provider/build metadata.
///
/// WHAT: preserves the actual registered runtime modules imported by the JS source.
/// WHY: runtime module emission must follow authored JS imports, not inferred fallibility.
pub fn required_runtime_imports_from_parsed(parsed: &ParsedJsModule) -> Vec<RequiredRuntimeImport> {
    parsed
        .runtime_imports
        .iter()
        .map(|runtime_import| RequiredRuntimeImport {
            module_name: runtime_import.module_name.clone(),
            imported_names: runtime_import.imported_names.clone(),
        })
        .collect()
}

// ------------------------------------------
//  Function conversion
// ------------------------------------------

fn convert_parsed_function_to_spec(
    function: &ParsedJsFunction,
    type_id_by_opaque_name: &HashMap<
        String,
        crate::compiler_frontend::external_packages::ExternalTypeId,
    >,
) -> Result<ExternalFunctionSpec, CompilerError> {
    assert_parsed_signature_receiver_invariants(&function.signature);

    let mut parameters = Vec::new();

    for parameter in &function.signature.parameters {
        let language_type =
            parsed_type_to_signature_type(&parameter.type_name, type_id_by_opaque_name)?;
        let access_kind = if parameter.is_mutable {
            ExternalAccessKind::Mutable
        } else {
            ExternalAccessKind::Shared
        };

        parameters.push(ExternalParameter {
            language_type,
            access_kind,
        });
    }

    let mut returns: Vec<ExternalReturnSlot> = Vec::new();
    let mut error_return_type: Option<ExternalSignatureType> = None;

    for return_type in &function.signature.returns {
        let signature_type =
            parsed_type_to_signature_type(&return_type.type_name, type_id_by_opaque_name)?;
        returns.push(ExternalReturnSlot::fresh(signature_type));
    }

    if function.signature.has_error_return {
        error_return_type = Some(ExternalSignatureType::BuiltinError);
    }

    Ok(ExternalFunctionSpec {
        name: function.moth_name.clone(),
        parameters,
        returns,
        error_return_type,
        lowerings: ExternalFunctionLowerings {
            js: Some(ExternalJsLowering::ExternalModuleExport {
                export_name: function.js_name.clone(),
            }),
            wasm: None,
        },
    })
}

/// Debug-only invariant check to catch malformed parsed signatures that should have
/// been rejected by the parser diagnostics.
///
/// WHY: the parser is the user-facing owner of `this` validation, but registration
///      should still fail loudly in development if an invariant is broken.
fn assert_parsed_signature_receiver_invariants(signature: &ParsedSignature) {
    #[cfg(debug_assertions)]
    {
        let mut receiver_count = 0;

        for (index, parameter) in signature.parameters.iter().enumerate() {
            if !parameter.is_receiver {
                continue;
            }

            receiver_count += 1;
            debug_assert_eq!(
                index, 0,
                "malformed parsed signature has receiver parameter at index {index}"
            );
        }

        debug_assert!(
            receiver_count <= 1,
            "malformed parsed signature has {receiver_count} receiver parameters"
        );
    }

    #[cfg(not(debug_assertions))]
    {
        let _ = signature;
    }
}

fn parsed_type_to_signature_type(
    type_name: &str,
    type_id_by_opaque_name: &HashMap<
        String,
        crate::compiler_frontend::external_packages::ExternalTypeId,
    >,
) -> Result<ExternalSignatureType, CompilerError> {
    match type_name {
        "Int" => Ok(ExternalSignatureType::Abi(ExternalAbiType::I32)),
        "Float" => Ok(ExternalSignatureType::Abi(ExternalAbiType::F64)),
        "Bool" => Ok(ExternalSignatureType::Abi(ExternalAbiType::Bool)),
        "String" => Ok(ExternalSignatureType::Abi(ExternalAbiType::Utf8Str)),
        "Char" => Ok(ExternalSignatureType::Abi(ExternalAbiType::Char)),
        _ => {
            if let Some(type_id) = type_id_by_opaque_name.get(type_name) {
                Ok(ExternalSignatureType::External(*type_id))
            } else {
                Err(CompilerError::compiler_error(format!(
                    "JS provider reached registry conversion with unknown parsed type '{}'.",
                    type_name
                )))
            }
        }
    }
}
