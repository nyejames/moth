//! `@core/io` package registration.
//!
//! WHAT: registers the lowercase `io` namespace surface provided by the core IO package:
//!       console output functions and the polling input handle and helpers.
//! WHY: centralizes the builtin `@core/io` surface so the frontend, prelude, and JS backend
//!       share the same canonical metadata.

use crate::compiler_frontend::external_packages::CORE_IO_PACKAGE_PATH;
use crate::compiler_frontend::external_packages::{
    ExternalAbiType, ExternalAccessKind, ExternalFunctionDef, ExternalFunctionId,
    ExternalFunctionLowerings, ExternalJsLowering, ExternalPackageRegistry, ExternalReturnAlias,
    ExternalReturnSlot, ExternalSignatureType, ExternalSymbolPath, ExternalTypeDef,
    IO_INPUT_EXTERNAL_TYPE_ID, external_success_returns,
};

struct IoConsoleFunctionSpec {
    id: ExternalFunctionId,
    path: &'static str,
    js_helper: &'static str,
}

const IO_CONSOLE_FUNCTIONS: &[IoConsoleFunctionSpec] = &[
    IoConsoleFunctionSpec {
        id: ExternalFunctionId::IoPrint,
        path: "print",
        js_helper: "__moth_io_print",
    },
    IoConsoleFunctionSpec {
        id: ExternalFunctionId::IoLine,
        path: "line",
        js_helper: "__moth_io_line",
    },
    IoConsoleFunctionSpec {
        id: ExternalFunctionId::IoDebug,
        path: "debug",
        js_helper: "__moth_io_debug",
    },
    IoConsoleFunctionSpec {
        id: ExternalFunctionId::IoWarn,
        path: "warn",
        js_helper: "__moth_io_warn",
    },
    IoConsoleFunctionSpec {
        id: ExternalFunctionId::IoError,
        path: "error",
        js_helper: "__moth_io_error",
    },
];

struct IoInputFunctionSpec {
    id: ExternalFunctionId,
    path: &'static str,
    parameters: Vec<(ExternalSignatureType, ExternalAccessKind)>,
    returns: Vec<ExternalReturnSlot>,
    error_return_type: Option<ExternalSignatureType>,
}

fn input_handle_param() -> Vec<(ExternalSignatureType, ExternalAccessKind)> {
    vec![(
        ExternalSignatureType::External(IO_INPUT_EXTERNAL_TYPE_ID),
        ExternalAccessKind::Shared,
    )]
}

fn input_mutable_handle_param() -> Vec<(ExternalSignatureType, ExternalAccessKind)> {
    vec![(
        ExternalSignatureType::External(IO_INPUT_EXTERNAL_TYPE_ID),
        ExternalAccessKind::Mutable,
    )]
}

fn string_param() -> Vec<(ExternalSignatureType, ExternalAccessKind)> {
    vec![(
        ExternalSignatureType::Abi(ExternalAbiType::Utf8Str),
        ExternalAccessKind::Shared,
    )]
}

fn input_handle_and_string_param() -> Vec<(ExternalSignatureType, ExternalAccessKind)> {
    let mut parameters = input_handle_param();
    parameters.extend(string_param());
    parameters
}

fn io_input_functions() -> Vec<IoInputFunctionSpec> {
    vec![
        IoInputFunctionSpec {
            id: ExternalFunctionId::IoInputNew,
            path: "new",
            parameters: Vec::new(),
            returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::External(
                IO_INPUT_EXTERNAL_TYPE_ID,
            ))],
            error_return_type: Some(ExternalSignatureType::BuiltinError),
        },
        IoInputFunctionSpec {
            id: ExternalFunctionId::IoInputUpdate,
            path: "update",
            parameters: input_mutable_handle_param(),
            returns: external_success_returns(ExternalAbiType::Void, ExternalReturnAlias::Fresh),
            error_return_type: None,
        },
        IoInputFunctionSpec {
            id: ExternalFunctionId::IoInputClose,
            path: "close",
            parameters: input_mutable_handle_param(),
            returns: external_success_returns(ExternalAbiType::Void, ExternalReturnAlias::Fresh),
            error_return_type: None,
        },
        IoInputFunctionSpec {
            id: ExternalFunctionId::IoInputKeyDown,
            path: "key_down",
            parameters: input_handle_and_string_param(),
            returns: external_success_returns(ExternalAbiType::Bool, ExternalReturnAlias::Fresh),
            error_return_type: None,
        },
        IoInputFunctionSpec {
            id: ExternalFunctionId::IoInputKeyPressed,
            path: "key_pressed",
            parameters: input_handle_and_string_param(),
            returns: external_success_returns(ExternalAbiType::Bool, ExternalReturnAlias::Fresh),
            error_return_type: None,
        },
        IoInputFunctionSpec {
            id: ExternalFunctionId::IoInputKeyReleased,
            path: "key_released",
            parameters: input_handle_and_string_param(),
            returns: external_success_returns(ExternalAbiType::Bool, ExternalReturnAlias::Fresh),
            error_return_type: None,
        },
        IoInputFunctionSpec {
            id: ExternalFunctionId::IoInputPointerX,
            path: "pointer_x",
            parameters: input_handle_param(),
            returns: external_success_returns(ExternalAbiType::F64, ExternalReturnAlias::Fresh),
            error_return_type: None,
        },
        IoInputFunctionSpec {
            id: ExternalFunctionId::IoInputPointerY,
            path: "pointer_y",
            parameters: input_handle_param(),
            returns: external_success_returns(ExternalAbiType::F64, ExternalReturnAlias::Fresh),
            error_return_type: None,
        },
        IoInputFunctionSpec {
            id: ExternalFunctionId::IoInputPointerDown,
            path: "pointer_down",
            parameters: input_handle_and_string_param(),
            returns: external_success_returns(ExternalAbiType::Bool, ExternalReturnAlias::Fresh),
            error_return_type: None,
        },
        IoInputFunctionSpec {
            id: ExternalFunctionId::IoInputPointerPressed,
            path: "pointer_pressed",
            parameters: input_handle_and_string_param(),
            returns: external_success_returns(ExternalAbiType::Bool, ExternalReturnAlias::Fresh),
            error_return_type: None,
        },
        IoInputFunctionSpec {
            id: ExternalFunctionId::IoInputPointerReleased,
            path: "pointer_released",
            parameters: input_handle_and_string_param(),
            returns: external_success_returns(ExternalAbiType::Bool, ExternalReturnAlias::Fresh),
            error_return_type: None,
        },
        IoInputFunctionSpec {
            id: ExternalFunctionId::IoInputLastKeyPressed,
            path: "last_key_pressed",
            parameters: input_handle_param(),
            returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::Optional(
                Box::new(ExternalSignatureType::Abi(ExternalAbiType::Utf8Str)),
            ))],
            error_return_type: None,
        },
        IoInputFunctionSpec {
            id: ExternalFunctionId::IoInputLastKeyReleased,
            path: "last_key_released",
            parameters: input_handle_param(),
            returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::Optional(
                Box::new(ExternalSignatureType::Abi(ExternalAbiType::Utf8Str)),
            ))],
            error_return_type: None,
        },
        IoInputFunctionSpec {
            id: ExternalFunctionId::IoInputLastPointerPressed,
            path: "last_pointer_pressed",
            parameters: input_handle_param(),
            returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::Optional(
                Box::new(ExternalSignatureType::Abi(ExternalAbiType::Utf8Str)),
            ))],
            error_return_type: None,
        },
        IoInputFunctionSpec {
            id: ExternalFunctionId::IoInputLastPointerReleased,
            path: "last_pointer_released",
            parameters: input_handle_param(),
            returns: vec![ExternalReturnSlot::fresh(ExternalSignatureType::Optional(
                Box::new(ExternalSignatureType::Abi(ExternalAbiType::Utf8Str)),
            ))],
            error_return_type: None,
        },
    ]
}

pub fn register_core_io_package(registry: &mut ExternalPackageRegistry) {
    let package_id = registry
        .register_package(
            CORE_IO_PACKAGE_PATH,
            crate::builder_surface::PackageOrigin::Core,
        )
        .expect("builtin package registration should not collide");

    for spec in IO_CONSOLE_FUNCTIONS {
        register_io_console_function(registry, package_id, spec);
    }

    register_io_input_type(registry, package_id);

    for spec in io_input_functions() {
        register_io_input_function(registry, package_id, spec);
    }
}

fn register_io_console_function(
    registry: &mut ExternalPackageRegistry,
    package_id: crate::compiler_frontend::external_packages::ExternalPackageId,
    spec: &IoConsoleFunctionSpec,
) {
    let path = ExternalSymbolPath::from_single(spec.path);
    let function = ExternalFunctionDef {
        name: spec.path.to_owned(),
        parameters: vec![
            crate::compiler_frontend::external_packages::ExternalParameter {
                language_type: ExternalSignatureType::StringContent,
                access_kind: ExternalAccessKind::Shared,
            },
        ],
        returns: external_success_returns(ExternalAbiType::Void, ExternalReturnAlias::Fresh),
        error_return_type: None,
        lowerings: ExternalFunctionLowerings {
            js: Some(ExternalJsLowering::RuntimeFunction(
                spec.js_helper.to_owned(),
            )),
            wasm: None,
        },
    };

    registry
        .register_function_at_path(package_id, path, spec.id, function)
        .expect("builtin console function registration should not collide");
}

fn register_io_input_type(
    registry: &mut ExternalPackageRegistry,
    package_id: crate::compiler_frontend::external_packages::ExternalPackageId,
) {
    let path = ExternalSymbolPath::from_components(vec!["input".to_owned(), "Input".to_owned()]);
    let type_def = ExternalTypeDef {
        name: "Input".to_owned(),
        package_id,
        abi_type: ExternalAbiType::Handle,
    };

    registry
        .register_type_at_path(package_id, path, IO_INPUT_EXTERNAL_TYPE_ID, type_def)
        .expect("builtin io.input.Input type registration should not collide");
}

fn register_io_input_function(
    registry: &mut ExternalPackageRegistry,
    package_id: crate::compiler_frontend::external_packages::ExternalPackageId,
    spec: IoInputFunctionSpec,
) {
    let path = ExternalSymbolPath::from_components(vec!["input".to_owned(), spec.path.to_owned()]);
    let function = ExternalFunctionDef {
        name: spec.path.to_owned(),
        parameters: spec
            .parameters
            .iter()
            .map(|(language_type, access_kind)| {
                crate::compiler_frontend::external_packages::ExternalParameter {
                    language_type: language_type.clone(),
                    access_kind: *access_kind,
                }
            })
            .collect(),
        returns: spec.returns,
        error_return_type: spec.error_return_type,
        lowerings: ExternalFunctionLowerings {
            js: Some(ExternalJsLowering::RuntimeFunction(
                spec.id.name().to_owned(),
            )),
            wasm: None,
        },
    };

    registry
        .register_function_at_path(package_id, path, spec.id, function)
        .expect("builtin io.input function registration should not collide");
}
