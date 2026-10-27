//! Import section emission.

use crate::backends::wasm::emit::sections::WasmEmitPlan;
use crate::backends::wasm::lir::linkage::WasmImportKind;
use crate::backends::wasm::lir::module::WasmLirModule;
use crate::compiler_frontend::compiler_messages::compiler_errors::InfrastructureFailure;
use wasm_encoder::{EntityType, ImportSection};

pub(crate) fn build_import_section(
    module: &WasmLirModule,
    plan: &WasmEmitPlan,
) -> Result<ImportSection, InfrastructureFailure> {
    let mut section = ImportSection::new();
    let mut imports = module.imports.iter().collect::<Vec<_>>();
    imports.sort_by_key(|import| import.id.0);

    // WHAT: encode imports in stable id order.
    // WHY: import indices are shared with call/export planning and must stay deterministic.
    for import in imports {
        match &import.kind {
            WasmImportKind::Function(signature) => {
                let type_index = plan
                    .type_index_by_signature
                    .get(signature)
                    .copied()
                    .ok_or_else(|| {
                        InfrastructureFailure::wasm_generation(format!(
                            "Wasm emission could not resolve type index for import {:?}",
                            import.id
                        ))
                    })?;
                section.import(
                    import.module_name.as_str(),
                    import.item_name.as_str(),
                    EntityType::Function(type_index),
                );
            }
        }
    }

    Ok(section)
}
