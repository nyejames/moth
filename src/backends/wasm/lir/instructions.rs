//! Statement and terminator instructions for Wasm LIR.
//!
//! The instruction set keeps target-local computation facts explicit for validation and binary
//! emission. Unsupported semantic families remain gated before HIR lowering.

use crate::backends::wasm::lir::types::{
    WasmAbiType, WasmImportId, WasmLirBlockId, WasmLirFunctionId, WasmLirLocalId, WasmStaticDataId,
};
use crate::backends::wasm::runtime::memory::WasmScalarStorageKind;
use crate::compiler_frontend::datatypes::numeric_operators::NumericOperator;
use crate::compiler_frontend::datatypes::numeric_scalar::BinaryFloatPrecision;

/// Integer operation domain resolved from canonical ranges, separate from scalar storage layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WasmIntegerOperationKind {
    Signed32,
    Unsigned32,
    Signed64,
    Unsigned64,
}

impl WasmIntegerOperationKind {
    pub(crate) fn carrier(self) -> WasmAbiType {
        match self {
            WasmIntegerOperationKind::Signed32 | WasmIntegerOperationKind::Unsigned32 => {
                WasmAbiType::I32
            }
            WasmIntegerOperationKind::Signed64 | WasmIntegerOperationKind::Unsigned64 => {
                WasmAbiType::I64
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WasmNumericOperationOperands {
    Unary {
        operand: WasmLirLocalId,
    },
    Binary {
        left: WasmLirLocalId,
        right: WasmLirLocalId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WasmIntegerPowerScratch {
    pub(crate) factor: WasmLirLocalId,
    pub(crate) exponent: WasmLirLocalId,
}

/// Non-aliasing intermediate locals allocated only when the operation needs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WasmIntegerScratch {
    /// A full I64 product for checked 32-bit multiplication, including power's repeated products.
    pub(crate) product: Option<WasmLirLocalId>,
    /// Power preserves factor and exponent here and accumulates directly in its destination.
    pub(crate) power: Option<WasmIntegerPowerScratch>,
}

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
    /// Convert a signed integer carrier to a finalized decimal string handle.
    /// The emitter sign-extends I32 carriers; I64 carriers pass directly.
    StringFromI64 {
        dst: WasmLirLocalId,
        value: WasmLirLocalId,
    },
    /// Convert an unsigned integer carrier to a finalized decimal string handle.
    /// The emitter zero-extends I32 carriers; I64 carriers pass directly.
    StringFromU64 {
        dst: WasmLirLocalId,
        value: WasmLirLocalId,
    },
    /// Convert an F32/F64 carrier to a finalized decimal string using its semantic precision.
    /// F16 values use their exact F32 carrier with `Binary16` precision.
    StringFromFloat {
        dst: WasmLirLocalId,
        value: WasmLirLocalId,
        precision: BinaryFloatPrecision,
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
    /// Trap-mode checked integer arithmetic in its canonical semantic result domain.
    CheckedIntegerOp {
        dst: WasmLirLocalId,
        operator: NumericOperator,
        kind: WasmIntegerOperationKind,
        operands: WasmNumericOperationOperands,
        scratch: WasmIntegerScratch,
    },
    /// Trap on non-finite input and write the validated profile-precision Float to `dst`.
    ValidateFloat {
        dst: WasmLirLocalId,
        source: WasmLirLocalId,
        precision: BinaryFloatPrecision,
    },
    /// Trap-mode checked float arithmetic in its profile-resolved semantic precision.
    CheckedFloatOp {
        dst: WasmLirLocalId,
        operator: NumericOperator,
        precision: BinaryFloatPrecision,
        operands: WasmNumericOperationOperands,
    },
    /// Compute a float range candidate in a backend-local scratch and commit only if accepted.
    FloatRangeCandidate {
        candidate_dst: WasmLirLocalId,
        in_range_dst: WasmLirLocalId,
        scratch: WasmLirLocalId,
        current: WasmLirLocalId,
        step: WasmLirLocalId,
        end: WasmLirLocalId,
        ascending: WasmLirLocalId,
        precision: BinaryFloatPrecision,
        inclusive: bool,
    },
    /// Convert a signed or unsigned integer carrier directly to an F32/F64 computation carrier.
    IntegerToFloat {
        dst: WasmLirLocalId,
        source: WasmLirLocalId,
        source_signed: bool,
    },
    /// Round an F32 computation carrier at an explicit semantic F16 boundary.
    RoundF16 {
        dst: WasmLirLocalId,
        source: WasmLirLocalId,
    },
    /// Widen an F32 carrier to F64 without changing its value.
    FloatExtend {
        dst: WasmLirLocalId,
        source: WasmLirLocalId,
    },
    /// Sign- or zero-extend a narrower canonical integer carrier for an infallible cast.
    IntegerExtend {
        dst: WasmLirLocalId,
        source: WasmLirLocalId,
        source_signed: bool,
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
    /// Load a typed scalar from linear memory at `address + offset`.
    ///
    /// The address is an already-owned i32 local; producers supply slots with the natural
    /// alignment promised by `kind`. The Wasm memory operation retains normal bounds trapping.
    #[allow(dead_code)]
    // Executed by low-level LIR clients; HIR aggregate places remain gated.
    LoadScalar {
        dst: WasmLirLocalId,
        address: WasmLirLocalId,
        offset: u32,
        kind: WasmScalarStorageKind,
    },
    /// Store a typed scalar to linear memory at `address + offset`.
    ///
    /// Producers must validate the source value against its semantic scalar range first. The
    /// narrow Wasm store truncation is only the physical 8-/16-bit storage operation, not a
    /// source-level narrowing conversion.
    /// The producer must preserve the natural alignment promised by `kind`.
    #[allow(dead_code)]
    // Executed by low-level LIR clients; HIR aggregate places remain gated.
    StoreScalar {
        address: WasmLirLocalId,
        offset: u32,
        value: WasmLirLocalId,
        kind: WasmScalarStorageKind,
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
