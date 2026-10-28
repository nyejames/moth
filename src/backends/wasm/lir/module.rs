//! Module-level LIR container and static data metadata.

use crate::backends::wasm::lir::function::WasmLirFunction;
use crate::backends::wasm::lir::linkage::{WasmExport, WasmImport};
use crate::backends::wasm::lir::types::WasmStaticDataId;
use crate::backends::wasm::runtime::memory::WasmMemoryPlan;

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct WasmLirModule {
    /// Lowered function bodies (including export wrappers).
    pub functions: Vec<WasmLirFunction>,
    /// Imports required by lowered calls and runtime ops.
    pub imports: Vec<WasmImport>,
    /// External exports requested by builder policy.
    pub exports: Vec<WasmExport>,
    /// Interned static byte segments.
    pub static_data: Vec<WasmStaticData>,
    /// Linear-memory layout consumed by Wasm emission.
    pub memory_plan: WasmMemoryPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WasmStaticData {
    pub id: WasmStaticDataId,
    /// Debug label for dumps (not semantically relevant).
    pub debug_name: String,
    /// Raw bytes emitted for this static segment.
    pub bytes: Vec<u8>,
    pub kind: WasmStaticDataKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WasmStaticDataKind {
    /// UTF-8 encoded string payload.
    Utf8StringBytes,
}
