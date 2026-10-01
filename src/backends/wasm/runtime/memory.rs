//! Runtime memory planning structures.

use crate::backends::wasm::lir::types::WasmAbiType;
use crate::compiler_frontend::datatypes::fixed_scalar::FixedScalar;
use crate::compiler_frontend::datatypes::numeric_profile::{
    FloatPrecision, IntWidth, NumericProfile,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;

/// Physical Wasm scalar-memory representation, separate from semantic identity.
///
/// F16 stores compact binary16 bits while its computation and ABI carrier stays F32.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WasmScalarStorageKind {
    F16,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

impl WasmScalarStorageKind {
    /// Natural scalar byte size.
    pub(crate) const fn size(self) -> u32 {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 | Self::F16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 => 8,
        }
    }

    /// Natural alignment is the scalar byte size for this layout.
    pub(crate) const fn alignment(self) -> u32 {
        self.size()
    }

    /// Scalar collection stride is its scalar byte size.
    #[allow(dead_code)] // Scalar slot layout is exercised below the gated HIR aggregate path.
    pub(crate) const fn stride(self) -> u32 {
        self.size()
    }

    /// Wasm computation carrier for loads and stores of this physical kind.
    pub(crate) const fn carrier(self) -> WasmAbiType {
        match self {
            Self::I8 | Self::U8 | Self::I16 | Self::U16 | Self::I32 | Self::U32 => WasmAbiType::I32,
            Self::I64 | Self::U64 => WasmAbiType::I64,
            Self::F16 | Self::F32 => WasmAbiType::F32,
            Self::F64 => WasmAbiType::F64,
        }
    }

    /// Derive supported physical storage from fixed scalar identity.
    ///
    /// `Byte` remains a distinct semantic type but has the same physical octet representation as
    /// U8. F16 uses the portable binary16 conversion helpers at the scalar memory boundary.
    #[allow(dead_code)] // Wasm scalar places will consume this when their owning lowering lands.
    pub(crate) const fn from_fixed_scalar(scalar: FixedScalar) -> Option<Self> {
        match scalar {
            FixedScalar::I8 => Some(Self::I8),
            FixedScalar::U8 => Some(Self::U8),
            FixedScalar::I16 => Some(Self::I16),
            FixedScalar::U16 => Some(Self::U16),
            FixedScalar::I32 => Some(Self::I32),
            FixedScalar::U32 => Some(Self::U32),
            FixedScalar::I64 => Some(Self::I64),
            FixedScalar::U64 => Some(Self::U64),
            FixedScalar::F16 => Some(Self::F16),
            FixedScalar::F32 => Some(Self::F32),
            FixedScalar::F64 => Some(Self::F64),
            FixedScalar::Byte => Some(Self::U8),
        }
    }

    /// Resolve profile-dependent numeric identity to physical scalar storage.
    #[allow(dead_code)] // The production HIR scalar-place path is not in this change.
    pub(crate) const fn for_numeric_scalar(
        scalar: NumericScalar,
        profile: NumericProfile,
    ) -> Option<Self> {
        match scalar {
            NumericScalar::Int => Some(match profile.int_width {
                IntWidth::Bits32 => Self::I32,
                IntWidth::Bits64 => Self::I64,
            }),
            NumericScalar::Float => Some(match profile.float_precision {
                FloatPrecision::Bits32 => Self::F32,
                FloatPrecision::Bits64 => Self::F64,
            }),
            NumericScalar::Fixed(scalar) => Self::from_fixed_scalar(scalar),
            NumericScalar::Number(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeapBaseStrategy {
    /// Heap starts after aligned static-data end.
    StaticDataEndAligned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WasmMemoryPlan {
    /// Initial memory page count (64KiB pages).
    pub initial_pages: u32,
    /// Optional max page cap.
    pub max_pages: Option<u32>,
    /// Base address for static segment placement.
    pub static_data_base: u32,
    /// Strategy used to compute runtime heap base.
    pub heap_base_strategy: HeapBaseStrategy,
}

impl Default for WasmMemoryPlan {
    fn default() -> Self {
        Self {
            initial_pages: 1,
            max_pages: None,
            static_data_base: 0,
            heap_base_strategy: HeapBaseStrategy::StaticDataEndAligned,
        }
    }
}
