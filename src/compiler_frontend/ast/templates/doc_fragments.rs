//! Top-level doc-template collection and stripping.
//!
//! WHAT: extracts `$doc` comment template output from authoritative TIR into
//! `AstDocFragment` metadata and strips those declarations from executable
//! function bodies.
//! WHY: documentation extraction is a separate concern from runtime fragment
//! synthesis and should remain independently auditable.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::store::{ConstStringPiece, ConstStringValue};
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{CommentDirectiveKind, TemplateType};
use crate::compiler_frontend::ast::templates::template_folding::{
    TemplateEmission, TemplateFoldResult, TirFoldContext,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrStore, TemplatePreparationMode, TemplatePreparationOutcome, TemplateTirPhase,
    TirView, fold_prepared_template, prepare_tir_view,
};
use crate::compiler_frontend::ast::templates::top_level_templates::{
    AstDocFragment, AstDocFragmentKind,
};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::cell::RefCell;
use std::rc::Rc;

// -------------------------
//  Fragment Extraction
// -------------------------

pub(in crate::compiler_frontend::ast::templates) fn collect_and_strip_comment_templates(
    ast_nodes: &mut [AstNode],
    string_table: &mut StringTable,
    template_const_loop_iteration_limit: usize,
    template_ir_store: Rc<RefCell<TemplateIrStore>>,
) -> Result<Vec<AstDocFragment>, TemplateError> {
    let mut fragments = Vec::new();
    let mut context = DocFragmentCollectionContext {
        string_table,
        template_const_loop_iteration_limit,
        template_ir_store,
    };

    for node in ast_nodes.iter_mut() {
        let NodeKind::Function(_, _, body) = &mut node.kind else {
            continue;
        };

        let mut retained = Vec::with_capacity(body.len());

        for statement in std::mem::take(body) {
            if let Some((comment_template, comment_kind)) =
                as_top_level_template_comment_declaration(&statement, &context.template_ir_store)
            {
                collect_doc_fragments(
                    comment_template,
                    comment_kind,
                    &mut fragments,
                    &mut context,
                )?;
                continue;
            }

            retained.push(statement);
        }

        *body = retained;
    }

    // Sort fragments deterministically by source location.
    fragments.sort_by_key(|fragment| {
        (
            fragment.location.scope.to_string(context.string_table),
            fragment.location.start_pos.line_number,
            fragment.location.start_pos.char_column,
        )
    });

    Ok(fragments)
}

/// Shared state for doc-template extraction.
///
/// WHAT: carries fold services through top-level comment extraction.
/// WHY: every fold should see the same module-store authority without
/// growing helper signatures.
struct DocFragmentCollectionContext<'strings> {
    string_table: &'strings mut StringTable,
    template_const_loop_iteration_limit: usize,
    template_ir_store: Rc<RefCell<TemplateIrStore>>,
}

// -------------------------
//  Internal Helpers
// -------------------------

/// Matches a top-level `PushStartRuntimeFragment` node containing a comment
/// template.
///
/// WHAT: reads the authoritative TIR kind from the shared module store.
/// WHY: comment extraction runs after AST emission, when every template value
///      belongs to that store.
fn as_top_level_template_comment_declaration<'a>(
    node: &'a AstNode,
    store: &Rc<RefCell<TemplateIrStore>>,
) -> Option<(&'a Template, CommentDirectiveKind)> {
    let NodeKind::PushStartRuntimeFragment(expression) = &node.kind else {
        return None;
    };

    let ExpressionKind::Template(template) = &expression.kind else {
        return None;
    };

    let comment_kind = comment_kind_at_doc_fragment_boundary(template, store)?;
    Some((template.as_ref(), comment_kind))
}

/// Extracts one top-level `$doc` fragment.
fn collect_doc_fragments(
    template: &Template,
    comment_kind: CommentDirectiveKind,
    fragments: &mut Vec<AstDocFragment>,
    context: &mut DocFragmentCollectionContext<'_>,
) -> Result<(), TemplateError> {
    if comment_kind == CommentDirectiveKind::Doc {
        let mut fold_context = TirFoldContext {
            string_table: context.string_table,
            template_const_loop_iteration_limit: context.template_const_loop_iteration_limit,
            bindings: Vec::new(),
        };
        let reference = template.tir_reference;
        let store = context.template_ir_store.borrow();
        let view = TirView::with_minimum_phase(
            &store,
            reference.root,
            reference.phase,
            TemplateTirPhase::Composed,
            reference.context,
        )?;
        let preparation = prepare_tir_view(&view, TemplatePreparationMode::ConstRequired)?;
        let prepared = match preparation {
            preparation if matches!(preparation.outcome, TemplatePreparationOutcome::Foldable) => {
                preparation
            }
            _ => {
                return Err(CompilerDiagnostic::invalid_template_structure(
                    InvalidTemplateStructureReason::NonFoldableConstTemplate,
                    template.location.to_owned(),
                )
                .into());
            }
        };
        // Documentation is rendered text metadata. Site-root anchors keep their authored `@/`
        // spelling. Resource pieces still need a builder URL context, so they stay a concrete-text
        // diagnostic rather than an internal Phase 4 error.
        let TemplateFoldResult { emission, .. } =
            fold_prepared_template(&prepared, view, &mut fold_context)?;
        let rendered = match emission {
            TemplateEmission::Output(ConstStringValue::Text(value)) => value,
            TemplateEmission::Output(ConstStringValue::Pieces(pieces)) => {
                flatten_documentation_string(
                    pieces,
                    fold_context.string_table,
                    template.location.to_owned(),
                )?
            }
            TemplateEmission::NoOutput => fold_context.string_table.intern(""),
            TemplateEmission::Break(_) | TemplateEmission::Continue(_) => {
                return Err(CompilerDiagnostic::invalid_template_structure(
                    InvalidTemplateStructureReason::NonFoldableConstTemplate,
                    template.location.to_owned(),
                )
                .into());
            }
        };

        fragments.push(AstDocFragment {
            kind: AstDocFragmentKind::Doc,
            value: rendered,
            location: template.location.to_owned(),
        });
    }

    Ok(())
}

/// Reads a doc-fragment template's kind from the shared module TIR store.
fn comment_kind_at_doc_fragment_boundary(
    template: &Template,
    store: &Rc<RefCell<TemplateIrStore>>,
) -> Option<CommentDirectiveKind> {
    store
        .borrow()
        .get_template(template.tir_reference.root)
        .and_then(|template_ir| match template_ir.kind {
            TemplateType::Comment(kind) => Some(kind),
            _ => None,
        })
}

fn flatten_documentation_string(
    pieces: Vec<ConstStringPiece>,
    string_table: &mut StringTable,
    location: crate::compiler_frontend::compiler_messages::source_location::SourceLocation,
) -> Result<crate::compiler_frontend::symbols::string_interning::StringId, TemplateError> {
    let mut text = String::new();
    for piece in pieces {
        match piece {
            ConstStringPiece::Text(value) => text.push_str(string_table.resolve(value)),
            ConstStringPiece::SiteRoot => text.push_str("@/"),
            ConstStringPiece::Resource(_) => {
                let operation = string_table.intern("documentation fragment");
                return Err(CompilerDiagnostic::compile_time_evaluation_error(
                    CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText,
                    Some(operation),
                    location,
                )
                .into());
            }
        }
    }
    Ok(string_table.get_or_intern(text))
}
