//! HIR fixture support for frontend unit tests.
//!
//! WHAT: wraps synthetic AST-to-HIR lowering used by HIR and borrow-checker tests.
//! WHY: these helpers sit at the HIR boundary and must not depend on borrow validation.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{Style, TemplateType};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrBuilder, TemplateIrStore, TemplateIrSummary, TemplateTirPhase, TemplateTirReference,
    TemplateViewContext,
};
use crate::compiler_frontend::hir::functions::HirFunctionOriginLookup;
use crate::compiler_frontend::hir::hir_builder::lower_module;
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) fn build_ast(nodes: Vec<AstNode>, entry_path: InternedPath) -> Ast {
    crate::compiler_frontend::hir::hir_builder::build_ast(nodes, entry_path)
}

pub(crate) fn entry_and_start(string_table: &mut StringTable) -> (InternedPath, InternedPath) {
    let entry_path = InternedPath::from_single_str("main.moth", string_table);
    let start_name = entry_path.join_str(IMPLICIT_START_FUNC_NAME, string_table);
    (entry_path, start_name)
}

pub(crate) fn lower_hir(ast: Ast, string_table: &mut StringTable) -> HirModule {
    let lowering = lower_module(
        ast,
        string_table,
        PathStringFormatConfig::default(),
        HirFunctionOriginLookup::default(),
    )
    .expect("HIR lowering should succeed");
    lowering.hir_module
}

/// Builds a malformed raw-template expression for HIR boundary invariant tests.
///
/// AST finalization should replace this shape with an owned runtime handoff. The
/// returned shared store keeps the deliberately unnormalized template's TIR identity valid.
pub(crate) fn raw_template_expression_for_hir_invariant(
    kind: TemplateType,
    location: SourceLocation,
    value_mode: ValueMode,
) -> (Expression, Rc<RefCell<TemplateIrStore>>) {
    let store_handle = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let template_id = {
        let mut store = store_handle.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let root = builder.push_sequence_node(vec![], location.clone());
        builder.finish_template(
            root,
            Style::default(),
            kind.clone(),
            TemplateIrSummary::empty(),
            location.clone(),
        )
    };
    let template = Template {
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Parsed,
            context,
        },
        location,
    };

    (Expression::template(template, value_mode), store_handle)
}
