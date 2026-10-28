//! ABI and Wasm value-type conversion helpers.

use crate::backends::wasm::lir::types::WasmAbiType;
use crate::compiler_frontend::compiler_messages::compiler_errors::InfrastructureFailure;
use wasm_encoder::ValType;

pub(crate) fn abi_to_val_type(abi: WasmAbiType) -> Result<ValType, InfrastructureFailure> {
    // WHAT: map backend ABI types onto concrete core-Wasm value types.
    // WHY: the current emitter uses an `i32` handle ABI for linear-memory objects to keep interop
    // simple.
    match abi {
        WasmAbiType::I32 | WasmAbiType::Handle => Ok(ValType::I32),
        WasmAbiType::I64 => Ok(ValType::I64),
        WasmAbiType::F32 => Ok(ValType::F32),
        WasmAbiType::F64 => Ok(ValType::F64),
        WasmAbiType::Void => Err(InfrastructureFailure::wasm_generation(
            "Wasm emission cannot lower `Void` to a concrete Wasm value type",
        )),
    }
}
