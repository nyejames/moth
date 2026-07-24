//! Statement and terminator instructions for Wasm LIR.
//!
//! The instruction set is intentionally narrow and tuned for lowering validation plus direct
//! binary emission. Variants marked with dead-code allowances are already supported by the emitter
//! or reserved by the memory model, but production HIR lowering does not construct them yet.

use crate::backends::wasm::lir::types::{
    WasmImportId, WasmLirBlockId, WasmLirFunctionId, WasmLirLocalId, WasmStaticDataId,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WasmLirStmt {
    /// Materialize immediate scalar constants.
    ConstI32 {
        dst: WasmLirLocalId,
        value: i32,
    },
    ConstI64 {
        dst: WasmLirLocalId,
        value: i64,
    },
    #[allow(dead_code)] // Wasm roadmap: f32 literals are emitter-tested before HIR maps to them.
    ConstF32 {
        dst: WasmLirLocalId,
        value: f32,
    },
    ConstF64 {
        dst: WasmLirLocalId,
        value: f64,
    },
    #[allow(dead_code)] // Wasm roadmap: static-data pointer materialization.
    ConstStaticPtr {
        dst: WasmLirLocalId,
        data: WasmStaticDataId,
    },
    #[allow(dead_code)] // Wasm roadmap: static-data byte-length materialization.
    ConstLength {
        dst: WasmLirLocalId,
        value: u32,
    },
    /// Explicit copy/move separation keeps ownership-optimization intent visible.
    Copy {
        dst: WasmLirLocalId,
        src: WasmLirLocalId,
    },
    Move {
        dst: WasmLirLocalId,
        src: WasmLirLocalId,
    },
    Call {
        /// Optional destination local for non-void calls.
        dst: Option<WasmLirLocalId>,
        callee: WasmCalleeRef,
        args: Vec<WasmLirLocalId>,
    },
    /// Runtime-template/string-building primitives.
    StringNewBuffer {
        dst: WasmLirLocalId,
    },
    StringPushLiteral {
        buffer: WasmLirLocalId,
        data: WasmStaticDataId,
    },
    StringPushHandle {
        buffer: WasmLirLocalId,
        handle: WasmLirLocalId,
    },
    /// Scalar-to-string bridge for template interpolation paths.
    /// NOTE: currently used for i64 chunks emitted by frontend string coercion.
    StringFromI64 {
        dst: WasmLirLocalId,
        value: WasmLirLocalId,
    },
    StringFinish {
        dst: WasmLirLocalId,
        buffer: WasmLirLocalId,
    },
    VecNew {
        dst: WasmLirLocalId,
    },
    VecPushHandle {
        vec: WasmLirLocalId,
        handle: WasmLirLocalId,
    },
    DropIfOwned {
        value: WasmLirLocalId,
    },
    /// Reserved for future ownership tuning.
    #[allow(dead_code)] // Memory model roadmap: explicit handle-retain operations.
    RetainHandle {
        value: WasmLirLocalId,
    },
    IntEq {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    IntNe {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    IntAdd {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    IntSub {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    IntMod {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    IntMul {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    /// Truncating integer division (for `//` operator); dst, lhs, rhs all I64.
    IntFloorDiv {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    /// Regular division with integer operands. lhs/rhs are I64; dst is F64.
    /// WHY: Moth `Int / Int` always yields Float; conversion is emitted here.
    IntToFloatDiv {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    FloatAdd {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    FloatSub {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    FloatMul {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    FloatDiv {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    /// Euclidean float modulus; emitted as `a − b·floor(a/b)` using the WASM stack.
    FloatMod {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    BoolAnd {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    BoolOr {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    OrderedLt {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    OrderedLe {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    OrderedGt {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    OrderedGe {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WasmCalleeRef {
    /// Direct call to another lowered function.
    Function(WasmLirFunctionId),
    /// Call through imported host function.
    Import(WasmImportId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WasmLirTerminator {
    /// Unconditional branch.
    Jump(WasmLirBlockId),
    /// Two-way conditional branch.
    Branch {
        condition: WasmLirLocalId,
        then_block: WasmLirBlockId,
        else_block: WasmLirBlockId,
    },
    /// Function return.
    Return { value: Option<WasmLirLocalId> },
    /// Fallback hard stop for unsupported/unreachable paths.
    Trap,
}
