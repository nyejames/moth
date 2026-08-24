//! HIR compile-time constants.
//!
//! WHAT: data carried from AST into HIR for module constants.
//! WHY: constants are backend/tooling metadata, not ordinary runtime statements.

use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::ids::HirConstId;

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
