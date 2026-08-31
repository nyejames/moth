//! Top-level const fragment collection and doc fragment extraction.
//!
//! WHAT: collects folded top-level const string fragments and extracts doc fragments from
//! comment templates.
//! WHY: builders consume ordered const fragments (with runtime insertion indices) and doc
//! metadata; all runtime template handling moves into the entry start() function body via
//! PushStartRuntimeFragment nodes.

use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::const_values::store::ConstStringValue;
use crate::compiler_frontend::ast::templates::doc_fragments;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::headers::parse_file_headers::TopLevelConstFragment;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;

// -------------------------
//  Fragment Types
// -------------------------

/// A top-level const template that has been folded to a string at compile time.
///
/// WHAT: carries the folded string value, including any unresolved structural pieces, and its
/// insertion index relative to runtime fragments.
/// WHY: builders merge const fragments with the runtime fragment list using the insertion index
/// to reconstruct source-order interleaving, while resolving structural pieces at their boundary.
#[derive(Clone, Debug)]
pub struct AstConstTopLevelFragment {
    /// Number of runtime fragments preceding this const fragment in source order.
    pub runtime_insertion_index: usize,
    pub value: ConstStringValue,
    pub _location: SourceLocation,
}

/// Folded value for a top-level const template.
///
/// WHAT: carries the already-folded string value produced from the template's
///       shared TIR authority, including unresolved structural pieces.
/// WHY: top-level fragment collection is keyed by source file and consumes the
///      folded value after AST emission has already validated and folded the
///      template.
#[derive(Clone, Debug)]
pub(crate) struct FoldedConstTemplateResult {
    value: ConstStringValue,
}

impl FoldedConstTemplateResult {
    pub(crate) fn new(value: ConstStringValue) -> Self {
        Self { value }
    }

    /// Returns a copy of the folded string value.
    pub(crate) fn value(&self) -> ConstStringValue {
        self.value.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AstDocFragmentKind {
    Doc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AstDocFragment {
    pub kind: AstDocFragmentKind,
    pub value: StringId,
    pub location: SourceLocation,
}

// -------------------------
//  Fragment Collection
// -------------------------

/// Collects const top-level fragments from folded template result records.
///
/// WHAT: maps each header-parsed const fragment to its folded string value using the
/// const template result map produced during AST emission.
/// WHY: const fragments are folded during emit; this function gathers the results into the
/// ordered `AstConstTopLevelFragment` list. HIR never consumes fragments: module compilation
/// converts each one into an owned structural string on the module metadata, which builders read
/// to render entry fragments.
pub(crate) fn collect_const_top_level_fragments(
    top_level_const_fragments: &[TopLevelConstFragment],
    const_templates_by_path: &FxHashMap<InternedPath, FoldedConstTemplateResult>,
) -> Result<Vec<AstConstTopLevelFragment>, CompilerError> {
    let mut result = Vec::with_capacity(top_level_const_fragments.len());

    for fragment in top_level_const_fragments {
        let value = const_templates_by_path
            .get(&fragment.header_path)
            .map(FoldedConstTemplateResult::value)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Top-level const fragment has no corresponding folded template value. This is a compiler bug.",
                )
            })?;

        result.push(AstConstTopLevelFragment {
            runtime_insertion_index: fragment.runtime_insertion_index,
            value,
            _location: fragment.location.clone(),
        });
    }

    Ok(result)
}

/// Extracts documentation fragments from comment templates and removes the comments from the AST.
pub(crate) fn collect_and_strip_comment_templates(
    ast_nodes: &mut [AstNode],
    string_table: &mut StringTable,
    template_const_loop_iteration_limit: usize,
    template_ir_store: Rc<RefCell<TemplateIrStore>>,
) -> Result<Vec<AstDocFragment>, TemplateError> {
    doc_fragments::collect_and_strip_comment_templates(
        ast_nodes,
        string_table,
        template_const_loop_iteration_limit,
        template_ir_store,
    )
}

#[cfg(test)]
#[path = "tests/template_tests.rs"]
mod template_tests;
