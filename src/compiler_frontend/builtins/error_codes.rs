//! Compiler-owned builtin error codes.
//!
//! WHAT: gives backend/generated errors stable integer codes and fallback messages.
//! WHY: the public `Error` surface stores `code U32`, so generated errors need one
//! canonical Rust-side mapping rather than scattered string codes or implicit enum values.

#[allow(dead_code)] // Some codes are reserved for planned surfaces and must keep stable values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinErrorCode {
    UnknownOrUnassigned = 0,
    Unsupported = 1,
    CollectionExpectedOrderedCollection = 100,
    CollectionIndexOutOfBounds = 101,
    CollectionFixedCapacityExceeded = 102,
    MapExpectedOrderedMap = 110,
    MapKeyNotFound = 111,
    IntParseInvalidFormat = 200,
    IntParseOutOfRange = 201,
    FloatParseInvalidFormat = 210,
    FloatParseOutOfRange = 211,
    StringParseBoolInvalidFormat = 220,
    StringParseCharInvalidFormat = 230,
    FloatCastToIntInvalidValue = 240,
    FloatCastToIntOutOfRange = 241,
    IntCastOutOfRange = 242,
    FloatCastNonFinite = 243,
    IntCastToCharInvalidCodepoint = 250,
    NumberParseInvalidFormat = 260,
    NumberParseInexactScale = 261,
    NumberCastInexact = 262,
    NumberParseCapacity = 263,
    /// Checked numeric operations use this when division or modulo receives a zero divisor.
    DivideByZero = 300,
    /// Checked integer operations use this when an operation leaves the signed i32 range.
    IntOverflow = 301,
    /// Checked exponent operations use this when an exponent is unsupported by the operation.
    InvalidExponent = 302,
    /// Checked Float operations use this when arithmetic produces a non-finite value.
    FloatNonFinite = 303,
    /// External/backend Float boundary validation uses this for non-finite incoming values.
    FloatBoundaryNonFinite = 304,
    /// Defensive Float formatting checks use this when an internal finite-Float invariant fails.
    FloatFormatInvariant = 305,
    /// A dynamic range step must be non-zero before the first iteration.
    InvalidRangeStep = 306,
    /// An in-range floating-point candidate must advance after rounding.
    RangeStepNoProgress = 307,
    /// Time ISO parsing uses this when text does not match the accepted timestamp format.
    TimeInvalidTimestampText = 310,
    /// Time rendering uses this when an instant lies outside the renderable range.
    TimeTimestampOutOfRange = 311,
}

impl BuiltinErrorCode {
    /// The single closed classification of failures inferred for checked arithmetic and ranges.
    ///
    /// Boundary validation and defensive formatting invariants are not implicit failures.
    /// Their delivery remains governed by separately declared API contracts.
    pub(crate) fn is_implicit_failure(self) -> bool {
        matches!(
            self,
            Self::DivideByZero
                | Self::IntOverflow
                | Self::InvalidExponent
                | Self::FloatNonFinite
                | Self::InvalidRangeStep
                | Self::RangeStepNoProgress
        )
    }

    /// The canonical unsigned runtime value this code carries in `Error.code`.
    pub(crate) fn as_u32(self) -> u32 {
        self as u32
    }

    pub(crate) fn default_message(self) -> &'static str {
        match self {
            BuiltinErrorCode::UnknownOrUnassigned => "Unknown error",
            BuiltinErrorCode::Unsupported => "Unsupported operation",
            BuiltinErrorCode::CollectionExpectedOrderedCollection => {
                "Collection operation expects an ordered collection"
            }
            BuiltinErrorCode::CollectionIndexOutOfBounds => "Collection index out of bounds",
            BuiltinErrorCode::CollectionFixedCapacityExceeded => {
                "Fixed collection capacity exceeded"
            }
            BuiltinErrorCode::MapExpectedOrderedMap => "Map operation expects an ordered map",
            BuiltinErrorCode::MapKeyNotFound => "Map key not found",
            BuiltinErrorCode::IntParseInvalidFormat => "Cannot parse Int from text",
            BuiltinErrorCode::IntParseOutOfRange => "Int value is out of supported range",
            BuiltinErrorCode::FloatParseInvalidFormat => "Cannot parse Float from text",
            BuiltinErrorCode::FloatParseOutOfRange => "Float value is out of supported range",
            BuiltinErrorCode::StringParseBoolInvalidFormat => "Cannot parse Bool from text",
            BuiltinErrorCode::StringParseCharInvalidFormat => "Cannot parse Char from text",
            BuiltinErrorCode::FloatCastToIntInvalidValue => "Float value cannot be cast to Int",
            BuiltinErrorCode::FloatCastToIntOutOfRange => "Float value is out of Int range",
            BuiltinErrorCode::IntCastOutOfRange => "Integer value is out of the target range",
            BuiltinErrorCode::FloatCastNonFinite => "Float conversion produced a non-finite value",
            BuiltinErrorCode::IntCastToCharInvalidCodepoint => {
                "Int value is not a valid Unicode scalar"
            }
            BuiltinErrorCode::NumberParseInvalidFormat => "Cannot parse Dec from text",
            BuiltinErrorCode::NumberParseInexactScale => {
                "Dec value is not exactly representable at the target scale"
            }
            BuiltinErrorCode::NumberCastInexact => {
                "Dec value is not exactly representable as the target type"
            }
            BuiltinErrorCode::NumberParseCapacity => {
                "the Dec value exceeds host representation capacity"
            }
            BuiltinErrorCode::IntOverflow => "Int operation overflowed",
            BuiltinErrorCode::DivideByZero => "Division by zero",
            BuiltinErrorCode::InvalidExponent => "Invalid exponent",
            BuiltinErrorCode::FloatNonFinite => "Float operation produced a non-finite value",
            BuiltinErrorCode::FloatBoundaryNonFinite => {
                "External Float boundary produced a non-finite value"
            }
            BuiltinErrorCode::FloatFormatInvariant => "Float formatting invariant failed",
            BuiltinErrorCode::InvalidRangeStep => "Loop step cannot be zero",
            BuiltinErrorCode::RangeStepNoProgress => "Floating-point range step made no progress",
            BuiltinErrorCode::TimeInvalidTimestampText => "Cannot parse Timestamp from text",
            BuiltinErrorCode::TimeTimestampOutOfRange => {
                "Timestamp instant is outside the renderable range"
            }
        }
    }
}
