//! HIR advisory const fact metadata.
//!
//! WHAT: projects AST const facts into a smaller HIR-safe summary that carries
//!       declaration path, scope, source, value kind, and exact source span.
//! WHY: borrow checking and backend lowering may use these facts for optimization
//!      in the future, but they are strictly advisory and must not affect semantic
//!      lowering decisions today.
//!
//! HIR facts deliberately omit the full AST `Expression` payload. They are metadata
//! for future optimization passes, not semantic inputs to HIR lowering or borrow
//! validation.

use crate::compiler_frontend::ast::const_values::facts::{
    AstConstFacts, ConstBindingScope, ConstBindingSource, ConstFactValueKind,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap};
use rustc_hash::FxHashMap;
use crate::compiler_frontend::symbols::string_interning::StringIdRemap;

/// Collection of HIR advisory const facts for one module.
#[derive(Clone, Debug, Default)]
pub struct HirConstFacts {
    pub declarations: FxHashMap<PathId, HirConstDeclarationFact>,
}

/// A single projected const fact in HIR.
///
/// WHAT: records the scope, source, value classification, and exact source span of a
///       compile-time declaration without storing the full AST expression.
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

    /// NOTE: currently advisory; only read in tests until optimization passes consume it.
    #[allow(dead_code)]
    pub value_kind: ConstFactValueKind,

    /// Exact authored declaration span; synthetic declarations are span-free.
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
            let span = match &fact.value {
                crate::compiler_frontend::ast::const_values::facts::AstConstFactValue::Stored(_) => {
                    None
                }
                crate::compiler_frontend::ast::const_values::facts::AstConstFactValue::Expression(
                    expression,
                ) => expression.span,
            };
            let hir_fact = HirConstDeclarationFact {
                declaration_path: fact.declaration_path,
                scope: fact.scope,
                source: fact.source,
                value_kind: fact.value_kind,
                span,
            };
            declarations.insert(*path, hir_fact);
        }

        Self { declarations }
    }
}
