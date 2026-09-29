//! LIR -> Wasm emission subsystem.
//!
//! This layer owns binary encoding only. It does not reinterpret frontend semantics.
//! `instructions` dispatches LIR statements; `checked_integer` and `checked_float` enforce the
//! resolved numeric domains. `binary16` handles compact representation boundaries, while power
//! and remainder helpers implement portable numerical operations with core Wasm instructions.

mod binary16;
mod checked_float;
mod checked_integer;
pub(crate) mod data;
pub(crate) mod exports;
mod float_format;
mod float_power;
mod float_remainder;
pub(crate) mod functions;
pub(crate) mod helpers;
pub(crate) mod imports;
pub(crate) mod instructions;
pub(crate) mod module;
pub(crate) mod names;
pub(crate) mod sections;
pub(crate) mod types;
pub(crate) mod validate;
pub(crate) mod vec_helpers;

#[derive(Debug, Clone, Default)]
pub(crate) struct WasmEmitDebugOutputs {
    /// Canonical section ordering/count summary.
    pub sections_text: String,
    /// Deterministic type/function/global/data index map summary.
    pub indices_text: String,
    /// Static-data placement and heap-base summary.
    pub data_layout_text: String,
    /// Validator output when in-process validation is enabled.
    pub validation_text: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct WasmEmitResult {
    /// Final `.wasm` bytes ready for host/runtime consumption.
    pub wasm_bytes: Vec<u8>,
    /// Text diagnostics controlled by backend debug flags.
    pub debug_outputs: WasmEmitDebugOutputs,
}
