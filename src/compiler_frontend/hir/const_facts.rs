//! HIR advisory const fact metadata.
//!
//! WHAT: projects AST const facts into a smaller HIR-safe summary that carries
//!       declaration path, scope, source, and exact source span.
//! WHY: borrow checking and backend lowering may use these facts for optimization, but
//!      they are strictly advisory and must not affect semantic lowering decisions.
//!
//! HIR facts carry metadata only. The resolved constant stays owned by the const value
//! store, and optimization passes may read these facts. They are not semantic inputs
//! to HIR lowering or borrow validation.

use crate::compiler_frontend::ast::const_values::facts::{
    AstConstFacts, ConstBindingScope, ConstBindingSource,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap};
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;
use rustc_hash::FxHashMap;

/// Collection of HIR advisory const facts for one module.
#[derive(Clone, Debug, Default)]
pub struct HirConstFacts {
    pub declarations: FxHashMap<PathId, HirConstDeclarationFact>,
}

/// A single projected const fact in HIR.
///
/// WHAT: records the scope, source, and exact source span of a compile-time declaration
///       without storing the resolved constant value.
/// WHY: keeps HIR lightweight while preserving the metadata needed by later
///      optimization passes.
#[derive(Clone, Debug)]
pub struct HirConstDeclarationFact {
    pub declaration_path: PathId,

    /// NOTE: currently advisory; only read in tests until optimization passes consume it.
    #[allow(dead_code)]
    pub scope: ConstBindingScope,

    /// NOTE: currently advisory; only read in tests until optimization passes consume it.
    #[allow(dead_code)]
    pub source: ConstBindingSource,

    /// Span carried by the resolved expression; explicit facts and spanless
    /// expressions use `None`.
    pub span: Option<SourceSpan>,
}

impl HirConstFacts {
    /// Remap declaration paths after the module-local path fork merges.
    pub(crate) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        if remap.is_identity() {
            return;
        }
        let declarations = std::mem::take(&mut self.declarations);
        self.declarations = declarations
            .into_iter()
            .map(|(path, mut fact)| {
                let path = remap.get(path);
                fact.declaration_path = remap.get(fact.declaration_path);
                (path, fact)
            })
            .collect();
    }

    /// Path identities are independent of string-table merges.
    pub fn remap_string_ids(&mut self, _remap: &StringIdRemap) {}
}

impl From<&AstConstFacts> for HirConstFacts {
    fn from(ast_facts: &AstConstFacts) -> Self {
        let mut declarations = FxHashMap::default();

        for (path, fact) in &ast_facts.declarations {
            let hir_fact = HirConstDeclarationFact {
                declaration_path: fact.declaration_path,
                scope: fact.scope,
                source: fact.source,
                span: fact.span,
            };
            declarations.insert(*path, hir_fact);
        }

        Self { declarations }
    }
}
