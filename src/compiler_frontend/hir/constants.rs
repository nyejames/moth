//! HIR compile-time constants.
//!
//! WHAT: data carried from AST into HIR for module constants.
//! WHY: constants are backend/tooling metadata, not ordinary runtime statements.

use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::ids::HirConstId;
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;

#[derive(Debug, Clone)]
pub struct HirConstField {
    pub name: String,
    pub value: HirConstValue,
}

#[derive(Debug, Clone)]
pub enum HirConstValue {
    /// Scalar payloads are preserved for data-model completeness even though
    /// current validation matches them with `_`. Tests and future backends may
    /// read these values.
    #[allow(dead_code)]
    Int(i32),
    #[allow(dead_code)]
    Float(f64),
    #[allow(dead_code)]
    Bool(bool),
    #[allow(dead_code)]
    Char(char),
    String(String),

    /// One folded `String` carrying structural pieces.
    ///
    /// WHAT: the module-local counterpart of `HirExpressionKind::StructuralString`, holding the
    /// same [`ConstStringPiece`] vocabulary. Text runs keep their interned handles and resource
    /// or site-root anchors stay structural.
    /// WHY: a resource-bearing constant is ordinary compile-time data, so its piece order must
    /// survive the HIR constant pool until physical variant planning resolves URL contexts.
    /// Plain text stays on the non-allocating `String` fast path.
    StructuralString {
        pieces: Vec<ConstStringPiece>,
    },
    Collection(Vec<HirConstValue>),
    Record(Vec<HirConstField>),
    Range(Box<HirConstValue>, Box<HirConstValue>),
    OptionSome(Box<HirConstValue>),
    OptionNone,
    Choice {
        /// Stored for completeness so the const-value payload carries the full
        /// choice shape. Currently not read outside of test assertions.
        #[allow(dead_code)]
        tag: usize,
        fields: Vec<HirConstField>,
    },
}

#[derive(Debug, Clone)]
pub struct HirModuleConst {
    pub id: HirConstId,
    pub name: String,
    pub ty: TypeId,
    pub value: HirConstValue,
}

impl HirConstValue {
    /// Remap interned string handles after a module-local string table merge.
    ///
    /// WHAT: re-binds the `Text` handles inside structural string pieces, mirroring the
    /// expression-side remap on `HirExpressionKind::StructuralString`.
    /// WHY: remapped modules must resolve structural piece text against the table that issued
    /// the new handles; owned `String` payloads carry text directly and `Resource` handles are
    /// module-local dense IDs with no interned text.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match self {
            Self::StructuralString { pieces } => {
                for piece in pieces {
                    if let ConstStringPiece::Text(string_id) = piece {
                        *string_id = remap.get(*string_id);
                    }
                }
            }
            Self::Record(fields) | Self::Choice { fields, .. } => {
                for field in fields {
                    field.value.remap_string_ids(remap);
                }
            }
            Self::Collection(values) => {
                for value in values {
                    value.remap_string_ids(remap);
                }
            }
            Self::Range(start, end) => {
                start.remap_string_ids(remap);
                end.remap_string_ids(remap);
            }
            Self::OptionSome(inner) => inner.remap_string_ids(remap),
            Self::Int(_)
            | Self::Float(_)
            | Self::Bool(_)
            | Self::Char(_)
            | Self::String(_)
            | Self::OptionNone => {}
        }
    }
}
