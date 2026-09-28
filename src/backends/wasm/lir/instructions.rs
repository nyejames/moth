//! Statement and terminator instructions for Wasm LIR.
//!
//! The instruction set is intentionally narrow and tuned for lowering validation plus direct
//! binary emission. Variants marked with dead-code allowances are already supported by the emitter
//! or reserved by the memory model, but production HIR lowering does not construct them yet.

use crate::backends::wasm::lir::types::{
    WasmImportId, WasmLirBlockId, WasmLirFunctionId, WasmLirLocalId, WasmStaticDataId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WasmScalarComparisonType {
    SignedInteger(u8),
    UnsignedInteger(u8),
    Float(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WasmScalarComparisonOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

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
    /// Profile-Int string bridge for template interpolation.
    /// The emitter sign-extends an I32 carrier; I64 carriers pass directly.
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
    /// Compare semantic scalar values after applying their declared widths and signedness.
    ScalarCompare {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
        op: WasmScalarComparisonOp,
        lhs_type: WasmScalarComparisonType,
        rhs_type: WasmScalarComparisonType,
    },
    /// Compare finalized String handles by UTF-8 content through the runtime helper.
    StringEq {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    /// Compare finalized String handles by UTF-8 content and negate the result.
    StringNe {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    #[allow(dead_code)]
    // Numeric plan Phase 5: checked NumericOp lowering reuses these primitives.
    IntAdd {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    #[allow(dead_code)]
    // Numeric plan Phase 5: checked NumericOp lowering reuses these primitives.
    IntSub {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    #[allow(dead_code)]
    // Numeric plan Phase 5: checked NumericOp lowering reuses these primitives.
    IntMod {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    #[allow(dead_code)]
    // Numeric plan Phase 5: checked NumericOp lowering reuses these primitives.
    IntMul {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    /// Truncating integer division (for `//` operator); dst, lhs, rhs all I64.
    #[allow(dead_code)]
    // Numeric plan Phase 5: checked NumericOp lowering reuses these primitives.
    IntFloorDiv {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    /// Convert the profile-selected signed Int carrier to the profile-selected Float carrier.
    IntToFloat {
        dst: WasmLirLocalId,
        source: WasmLirLocalId,
    },
    /// Regular division with integer operands. lhs/rhs are I64; dst is F64.
    /// WHY: Moth `Int / Int` always yields Float; conversion is emitted here.
    #[allow(dead_code)]
    // Numeric plan Phase 5: checked NumericOp lowering reuses these primitives.
    IntToFloatDiv {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    #[allow(dead_code)]
    // Numeric plan Phase 5: checked NumericOp lowering reuses these primitives.
    FloatAdd {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    #[allow(dead_code)]
    // Numeric plan Phase 5: checked NumericOp lowering reuses these primitives.
    FloatSub {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    #[allow(dead_code)]
    // Numeric plan Phase 5: checked NumericOp lowering reuses these primitives.
    FloatMul {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    #[allow(dead_code)]
    // Numeric plan Phase 5: checked NumericOp lowering reuses these primitives.
    FloatDiv {
        dst: WasmLirLocalId,
        lhs: WasmLirLocalId,
        rhs: WasmLirLocalId,
    },
    /// Euclidean float modulus; emitted as `a − b·floor(a/b)` using the WASM stack.
    #[allow(dead_code)]
    // Numeric plan Phase 5: checked NumericOp lowering reuses these primitives.
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
