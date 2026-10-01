//! Data section emission.

use crate::backends::error_types::BackendErrorType;
use crate::backends::wasm::emit::sections::WasmEmitPlan;
use crate::backends::wasm::lir::module::WasmLirModule;
use crate::compiler_frontend::compiler_messages::compiler_errors::{CompilerError, ErrorType};
use wasm_encoder::{ConstExpr, DataSection};

pub(crate) fn build_data_section(
    module: &WasmLirModule,
    plan: &mut WasmEmitPlan,
) -> Result<DataSection, CompilerError> {
    let mut section = DataSection::new();

    // WHAT: emit data segments in stable id order.
    // WHY: deterministic segment ordering keeps literal-pointer behavior reproducible.
    let mut static_data = module.static_data.iter().collect::<Vec<_>>();
    static_data.sort_by_key(|segment| segment.id.0);
    for segment in static_data {
        let offset = plan.data_offsets.get(&segment.id).copied().ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Wasm emission missing static data offset for segment {:?}",
                segment.id
            ))
            .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
        })?;

        section.active(
            0,
            &ConstExpr::i32_const(offset as i32),
            segment.bytes.to_owned(),
        );
    }
    if let Some(offset) = plan.float_format_tables_offset {
        let bytes = plan.float_format_tables.take().ok_or_else(|| {
            CompilerError::compiler_error("Wasm emission lost its planned Ryu table bytes")
                .with_error_type(ErrorType::Backend(BackendErrorType::WasmGeneration))
        })?;
        section.active(0, &ConstExpr::i32_const(offset as i32), bytes);
    }

    Ok(section)
}
