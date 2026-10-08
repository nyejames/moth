//! HIR expressions.
//!
//! WHAT: typed dense expression rows used by statements, terminators, and pattern matching.
//! WHY: one module-owned row store holds fixed child IDs and typed ranges for variable edges while
//!      control flow stays explicit in blocks and terminators.
//!
//! ## Cast contract
//!
//! AST resolves all cast targets, evidence, fallibility, and optional wrapping flags before HIR.
//! HIR only carries compiler-owned builtin runtime casts as `HirExpressionKind::Cast` or
//! `HirStatementKind::CastOp`. User-defined cast evidence lowers to a direct user-function call
//! during HIR lowering, and `ResolvedCastEvidence::GenericBound` is validation-only and must not
//! reach HIR.

use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::number::NumberValue;
use crate::compiler_frontend::hir::expression_store::{
    HirMapEntryRange, HirStringPieceRange, HirStructFieldRange, HirValueRange, HirVariantFieldRange,
};
use crate::compiler_frontend::hir::ids::{ChoiceId, HirValueId, RegionId, StructId};
use crate::compiler_frontend::hir::operators::{HirBinOp, HirUnaryOp};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;
use moth_lexical::numeric::fixed_scalar::FixedScalarValue;

/// Shared carrier tag for variant construction in HIR.
///
/// WHY: Choices, Options, and Results all construct variant-shaped values. A shared carrier
/// keeps backend lowering uniform while preserving distinct semantic type identity.
#[derive(Debug, Clone)]
pub enum HirVariantCarrier {
    Choice {
        choice_id: ChoiceId,
    },
    Option,
    #[cfg(test)]
    Fallible,
}

/// Variant index used for `some` inside `HirVariantCarrier::Option`.
pub const OPTION_SOME_VARIANT_INDEX: usize = 1;

/// One field inside a `VariantConstruct`.
///
/// WHY: payload field names are part of the runtime carrier shape for JS and future backends.
#[derive(Debug, Clone, Copy)]
pub struct HirVariantField {
    pub name: Option<StringId>,
    pub value: HirValueId,
}

/// One key/value pair inside a `MapLiteral`.
///
/// WHAT: holds the lowered HIR key and value expressions for a single map entry.
/// WHY: map literals need an explicit entry shape so lowering, validation, and backend
///      emission can traverse children consistently.
#[derive(Debug, Clone, Copy)]
pub struct HirMapEntry {
    pub key: HirValueId,
    pub value: HirValueId,
}

/// Compiler-owned map operation kinds used in HIR.
///
/// WHAT: identifies the specific map builtin being requested at the HIR level.
/// WHY: separates frontend `MapBuiltinOp` from the HIR statement representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirMapOp {
    /// Retrieve a value by key.
    Get,
    /// Check whether a key exists.
    Contains,
    /// Insert or update a key/value pair.
    Set,
    /// Remove a key and its value.
    Remove,
    /// Remove all entries.
    Clear,
    /// Count the number of entries.
    Length,
}

impl HirMapOp {
    #[cfg(any(test, feature = "show_hir"))]
    pub(crate) fn source_name(self) -> &'static str {
        match self {
            HirMapOp::Get => "get",
            HirMapOp::Contains => "contains",
            HirMapOp::Set => "set",
            HirMapOp::Remove => "remove",
            HirMapOp::Clear => "clear",
            HirMapOp::Length => "length",
        }
    }

    /// Whether the operation mutates the receiver map.
    pub(crate) fn requires_mutable_receiver(self) -> bool {
        matches!(self, HirMapOp::Set | HirMapOp::Remove | HirMapOp::Clear)
    }
}

#[derive(Debug, Clone)]
pub struct HirExpression {
    pub kind: HirExpressionKind,
    pub ty: TypeId,
    pub value_kind: ValueKind,
    pub region: RegionId,
    /// Exact authored syntax span; compiler-generated HIR expressions are span-free.
    pub span: Option<SourceSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// Refers to a memory location.
    Place,

    /// Produces a value.
    RValue,

    /// Compile-time constant.
    Const,
}

#[derive(Debug, Clone)]
pub enum HirExpressionKind {
    // -------------------------
    //  Literals
    // -------------------------
    /// One unsigned 64-bit integer literal.
    Uint(u64),
    Int(i64),
    Float(f64),
    /// One materialised fixed-width scalar or `Byte` value with exact-bit identity.
    ///
    /// WHY: HTML-JS lowers the value through `JsNumericCarrier::fixed_literal`, while HTML-Wasm
    ///      rejects reachable fixed values until Phase 5.
    FixedScalar(FixedScalarValue),
    /// One exact arbitrary-precision decimal literal with its canonical scale.
    Number(NumberValue),
    Bool(bool),
    Char(char),
    StringLiteral(String),
    StructuralString {
        pieces: HirStringPieceRange,
    },

    // -------------------------
    //  Memory & Data Flow
    // -------------------------
    Load(HirPlace),
    Copy(HirPlace),

    // -------------------------
    //  Operations
    // -------------------------
    BinOp {
        left: HirValueId,
        op: HirBinOp,
        right: HirValueId,
    },

    UnaryOp {
        op: HirUnaryOp,
        operand: HirValueId,
    },

    // -------------------------
    //  Object Construction
    // -------------------------
    StructConstruct {
        struct_id: StructId,
        fields: HirStructFieldRange,
    },

    Collection(HirValueRange),

    Range {
        start: HirValueId,
        end: HirValueId,
    },

    // -------------------------
    //  Tuple Operations
    // -------------------------
    /// Construct a tuple value (for multi-return)
    /// Example: return (42, "hello")
    /// EMPTY TUPLE IS THE UNIT TYPE ()
    /// EMPTY TUPLE == the builtin none `TypeId`.
    TupleConstruct {
        elements: HirValueRange,
    },

    /// Project a tuple slot by flat index.
    TupleGet {
        tuple: HirValueId,
        index: usize,
    },

    // -------------------------
    //  Fallible Carrier Handling
    // -------------------------
    /// Extracts the success payload from an internal fallible carrier.
    FallibleUnwrapSuccess {
        result: HirValueId,
    },

    /// Extracts the error payload from an internal fallible carrier.
    FallibleUnwrapError {
        result: HirValueId,
    },

    // -------------------------
    //  Type Conversion
    // -------------------------
    /// Builtin cast applied to an already-evaluated source value.
    ///
    /// WHAT: carries the stable builtin cast policy so the backend can dispatch to
    ///      the correct runtime helper without re-deriving source/target pairs.
    /// WHY: AST already resolved the target, evidence, fallibility, and optional wrap flag;
    ///      HIR only materializes the resulting builtin runtime cast. Infallible casts stay as
    ///      pure expressions, while fallible casts lower through `HirStatementKind::CastOp` so
    ///      success/error control flow remains explicit.
    Cast {
        source: HirValueId,
        policy: BuiltinCastPolicyId,
    },

    // -------------------------
    //  Variant Operations
    // -------------------------
    /// Construct a variant value through a shared carrier.
    ///
    /// WHY: unifies choice/option/result construction in HIR while keeping type kinds distinct.
    VariantConstruct {
        carrier: HirVariantCarrier,
        variant_index: usize,
        fields: HirVariantFieldRange,
    },

    /// Extract a payload field from a variant-shaped value.
    ///
    /// WHY: match-arm capture bindings are materialized as local assignments from the
    /// scrutinee. Using a dedicated HIR expression keeps backend lowering uniform and
    /// preserves the carrier type for field-name resolution.
    VariantPayloadGet {
        carrier: HirVariantCarrier,
        source: HirValueId,
        variant_index: usize,
        field_index: usize,
    },

    // -------------------------
    //  Map Operations
    // -------------------------
    /// Construct an insertion-ordered hashmap value from explicit key/value entries.
    ///
    /// WHAT: each entry is lowered independently so prelude order and side effects are
    ///       preserved before the literal value is produced.
    /// WHY: map literals are first-class compiler-owned values, not external calls.
    MapLiteral(HirMapEntryRange),
}

// Option none/some are represented through VariantConstruct with HirVariantCarrier::Option.
