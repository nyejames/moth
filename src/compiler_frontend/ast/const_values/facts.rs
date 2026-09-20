//! AST const fact metadata.
//!
//! WHAT: defines the advisory metadata recorded for const declarations during
//!       AST finalization and consumed by later stages such as HIR const facts.
//! WHY: one typed fact shape lets later stages share the same classification
//!      result without each stage retaining its own resolved value copy.

use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;

use rustc_hash::FxHashMap;

/// Collection of all const facts discovered in one AST module.
///
/// WHAT: maps declaration path to the resolved const fact for that declaration.
/// WHY: later stages look up facts by path without re-walking the AST.
#[derive(Clone, Debug, Default)]
pub struct AstConstFacts {
    pub declarations: FxHashMap<PathId, AstConstDeclarationFact>,
}

/// A single advisory const fact for one declaration.
///
/// WHAT: records the scope, source, value classification, and exact authored
///       span for a compile-time declaration without retaining the resolved value.
/// WHY: resolved values stay owned by the const value store for explicit module
///      constants and by the lexical const environment for inferred declarations;
///      facts are metadata for later advisory consumers only.
#[derive(Clone, Debug)]
pub struct AstConstDeclarationFact {
    pub declaration_path: PathId,
    pub scope: ConstBindingScope,
    pub source: ConstBindingSource,
    pub value_kind: ConstFactValueKind,
    /// Span carried by the resolved expression; explicit facts and spanless
    /// expressions use `None`.
    pub span: Option<SourceSpan>,
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
