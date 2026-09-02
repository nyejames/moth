//! AST const fact payloads.
//!
//! WHAT: defines the shape of const facts recorded for declarations during
//!       AST finalization and consumed by later stages such as HIR const facts.
//! WHY: one typed fact shape lets later stages share the same resolution
//!      result without each stage inventing its own representation.

use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use rustc_hash::FxHashMap;

use super::store::ConstValueId;

/// Collection of all const facts discovered in one AST module.
///
/// WHAT: maps declaration path to the resolved const fact for that declaration.
/// WHY: later stages look up facts by path without re-walking the AST.
#[derive(Clone, Debug, Default)]
pub struct AstConstFacts {
    pub declarations: FxHashMap<InternedPath, AstConstDeclarationFact>,
}

/// A single resolved const fact for one declaration.
///
/// WHAT: records the scope, source, value classification, and fully resolved
///       value for a compile-time declaration.
/// WHY: authored module constants reuse the module-owned [`ConstValueStore`] identity while
///      body-local and inferred facts retain their resolver-owned expression only as advisory
///      metadata.
#[derive(Clone, Debug)]
pub struct AstConstDeclarationFact {
    pub declaration_path: InternedPath,
    pub scope: ConstBindingScope,
    pub source: ConstBindingSource,
    pub value_kind: ConstFactValueKind,
    pub value: AstConstFactValue,
    pub location: SourceLocation,
}

/// Value retained by one advisory const fact.
#[derive(Clone, Debug)]
pub enum AstConstFactValue {
    /// The authored module constant's folded value is owned by the module store.
    ///
    /// The store identity remains on the fact so HIR can join store-backed rows. Config
    /// validation no longer reads it; it consumes owned folded declarations instead.
    #[allow(dead_code)]
    Stored(ConstValueId),

    /// A body-local or inferred declaration keeps its resolver result as advisory metadata.
    Expression(Box<Expression>),
}

/// Where a const binding is visible in the source program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstBindingScope {
    /// Explicit `#=` constant at module top level.
    ExplicitTopLevel,

    /// Inferred immutable declaration at module top level (start body).
    PrivateTopLevel,

    /// Inferred immutable declaration inside a function or block body.
    BodyLocal,
}

/// How the compiler determined that a declaration is const.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstBindingSource {
    /// User wrote `#=`.
    ExplicitHash,

    /// User wrote `=` and the compiler inferred const-ness from the initializer.
    InferredImmutable,
}

/// Classification of a resolved const fact's value shape.
///
/// WHAT: mirrors the AST `ConstValueKind` classification but is owned by the
///       const facts module so fact consumers do not depend on expression internals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstFactValueKind {
    Literal,
    Composite,
    RenderableTemplate,
    TemplateWrapper,
    SlotInsertTemplate,
    NonConst,
}

impl ConstFactValueKind {
    /// Derive the fact value kind from an expression const classification.
    ///
    /// WHAT: callers provide the already-computed `ConstValueKind` so production
    ///       const fact collection can classify templates through fresh TIR
    ///       instead of the legacy no-store template path.
    pub fn from_const_value_kind(kind: ConstValueKind) -> Self {
        match kind {
            ConstValueKind::Literal => Self::Literal,
            ConstValueKind::Composite => Self::Composite,
            ConstValueKind::RenderableTemplate => Self::RenderableTemplate,
            ConstValueKind::TemplateWrapper => Self::TemplateWrapper,
            ConstValueKind::SlotInsertTemplate => Self::SlotInsertTemplate,
            ConstValueKind::NonConst => Self::NonConst,
        }
    }
}
