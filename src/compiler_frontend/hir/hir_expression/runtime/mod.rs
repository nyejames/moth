//! Runtime operation lowering from owner-bound semantic expression views.
//!
//! Arithmetic consumes resolved facts directly. Logical operations emit lazy CFG,
//! and merge values use temporary locals without reconstructing an AST tree.

mod operation_lowering;
pub(super) mod short_circuit_cfg;
mod temp_assignment;
