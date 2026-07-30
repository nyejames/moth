//! HIR expressions.
//!
//! WHAT: typed value-producing nodes used by statements, terminators, and pattern matching.
//! WHY: HIR keeps normal value construction as expression trees while control flow stays explicit.
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
use crate::compiler_frontend::hir::ids::{ChoiceId, FieldId, HirValueId, RegionId, StructId};
use crate::compiler_frontend::hir::operators::{HirBinOp, HirUnaryOp};
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};

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
#[derive(Debug, Clone)]
pub struct HirVariantField {
    pub name: Option<StringId>,
    pub value: HirExpression,
}

/// One key/value pair inside a `MapLiteral`.
///
/// WHAT: holds the lowered HIR key and value expressions for a single map entry.
/// WHY: map literals need an explicit entry shape so lowering, validation, and backend
///      emission can traverse children consistently.
#[derive(Debug, Clone)]
pub struct HirMapEntry {
    pub key: HirExpression,
    pub value: HirExpression,
}

impl HirExpression {
    /// Remap the interned names retained directly by executable expression payloads.
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match &mut self.kind {
            HirExpressionKind::Load(place) | HirExpressionKind::Copy(place) => {
                place.remap_string_ids(remap);
            }
            HirExpressionKind::BinOp { left, right, .. } => {
                left.remap_string_ids(remap);
                right.remap_string_ids(remap);
            }
            HirExpressionKind::UnaryOp { operand, .. }
            | HirExpressionKind::TupleGet { tuple: operand, .. }
            | HirExpressionKind::FallibleUnwrapSuccess { result: operand }
            | HirExpressionKind::FallibleUnwrapError { result: operand }
            | HirExpressionKind::Cast {
                source: operand, ..
            }
            | HirExpressionKind::VariantPayloadGet {
                source: operand, ..
            } => operand.remap_string_ids(remap),
            HirExpressionKind::StructConstruct { fields, .. } => {
                for (_, value) in fields {
                    value.remap_string_ids(remap);
                }
            }
            HirExpressionKind::Collection(elements)
            | HirExpressionKind::TupleConstruct { elements } => {
                for element in elements {
                    element.remap_string_ids(remap);
                }
            }
            HirExpressionKind::Range { start, end } => {
                start.remap_string_ids(remap);
                end.remap_string_ids(remap);
            }
            HirExpressionKind::VariantConstruct { fields, .. } => {
                for field in fields {
                    if let Some(name) = &mut field.name {
                        *name = remap.get(*name);
                    }
                    field.value.remap_string_ids(remap);
                }
            }
            HirExpressionKind::MapLiteral(entries) => {
                for entry in entries {
                    entry.key.remap_string_ids(remap);
                    entry.value.remap_string_ids(remap);
                }
            }
            HirExpressionKind::Int(_)
            | HirExpressionKind::Float(_)
            | HirExpressionKind::Bool(_)
            | HirExpressionKind::Char(_)
            | HirExpressionKind::StringLiteral(_) => {}
        }
    }
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
    pub id: HirValueId,
    pub kind: HirExpressionKind,
    pub ty: TypeId,
    pub value_kind: ValueKind,
    pub region: RegionId,
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
    Int(i32),
    Float(f64),
    Bool(bool),
    Char(char),
    StringLiteral(String),

    // -------------------------
    //  Memory & Data Flow
    // -------------------------
    Load(HirPlace),
    Copy(HirPlace),

    // -------------------------
    //  Operations
    // -------------------------
    BinOp {
        left: Box<HirExpression>,
        op: HirBinOp,
        right: Box<HirExpression>,
    },

    UnaryOp {
        op: HirUnaryOp,
        operand: Box<HirExpression>,
    },

    // -------------------------
    //  Object Construction
    // -------------------------
    StructConstruct {
        struct_id: StructId,
        fields: Vec<(FieldId, HirExpression)>,
    },

    Collection(Vec<HirExpression>),

    Range {
        start: Box<HirExpression>,
        end: Box<HirExpression>,
    },

    // -------------------------
    //  Tuple Operations
    // -------------------------
    /// Construct a tuple value (for multi-return)
    /// Example: return (42, "hello")
    /// EMPTY TUPLE IS THE UNIT TYPE ()
    /// EMPTY TUPLE == the builtin none `TypeId`.
    TupleConstruct {
        elements: Vec<HirExpression>,
    },

    /// Project a tuple slot by flat index.
    TupleGet {
        tuple: Box<HirExpression>,
        index: usize,
    },

    // -------------------------
    //  Fallible Carrier Handling
    // -------------------------
    /// Extracts the success payload from an internal fallible carrier.
    FallibleUnwrapSuccess {
        result: Box<HirExpression>,
    },

    /// Extracts the error payload from an internal fallible carrier.
    FallibleUnwrapError {
        result: Box<HirExpression>,
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
        source: Box<HirExpression>,
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
        fields: Vec<HirVariantField>,
    },

    /// Extract a payload field from a variant-shaped value.
    ///
    /// WHY: match-arm capture bindings are materialized as local assignments from the
    /// scrutinee. Using a dedicated HIR expression keeps backend lowering uniform and
    /// preserves the carrier type for field-name resolution.
    VariantPayloadGet {
        carrier: HirVariantCarrier,
        source: Box<HirExpression>,
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
    MapLiteral(Vec<HirMapEntry>),
}

// Option none/some are represented through VariantConstruct with HirVariantCarrier::Option.

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FallibleCarrierVariant {
    Success,
    Error,
}
