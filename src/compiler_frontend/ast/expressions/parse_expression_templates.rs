//! Template expression parsing helpers.
//!
//! WHAT: parses template expressions and handles compile-time folding where applicable.
//! WHY: template expressions have distinct constant/runtime behavior and should not be buried in general token dispatch.

use super::error::ExpressionParseError;
use super::expression_store::{AstExpressionBuilder, OwnedExpr};
use crate::ast_log;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{TemplateConstValueKind, TemplateType};
use crate::compiler_frontend::ast::templates::template_folding::TemplateEmission;
use crate::compiler_frontend::ast::templates::tir::{
    TemplatePreparationMode, TemplatePreparationOutcome, TemplateTirPhase, TirView,
    fold_prepared_template, prepare_tir_view_with_source_scope,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::InvalidTemplateSlotReason;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::value_mode::ValueMode;

// WHAT: parses a template literal and optionally folds it to a string slice expression.
// WHY: templates appear in expression positions but have their own grammar (slots, definitions,
//      comments). This function decides whether the template stays a runtime value or can be
//      folded at compile time.
#[allow(
    clippy::too_many_arguments,
    reason = "template expression parsing keeps the token stream, scope, type interner, expression builder, const store, closing-parenthesis policy, value mode, string table, and path fork as separate borrows"
)]
pub(super) fn parse_template_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    consume_closing_parenthesis: bool,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Option<OwnedExpr>, ExpressionParseError> {
    let template_span = Some(token_stream.current_span());
    let template_context = context.new_template_parsing_context();
    let template = if context.kind.is_constant_context() {
        Template::new_const_required_with_type_interner(
            token_stream,
            context.scope,
            &template_context,
            type_interner,
            expression_builder,
            const_values,
            vec![],
            string_table,
            path_fork,
        )
        // Const-required preparation proves a distinct semantic mode. This
        // expression path intentionally prepares the resulting template again
        // in `Value` mode below because runtime dependence is valid here.
        .map(|construction| construction.template)
    } else {
        Template::new_with_type_interner(
            token_stream,
            context.scope,
            &template_context,
            type_interner,
            expression_builder,
            const_values,
            vec![],
            string_table,
            path_fork,
        )
    }
    .map_err(ExpressionParseError::from)?;

    let template_kind = {
        let store = template_context.template_ir_store.borrow();
        store
            .get_template(template.tir_reference.root)
            .map(|template_ir| template_ir.kind.clone())
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "Parsed template kind was missing from its module-local TIR store.",
                )
            })?
    };

    match template_kind {
        TemplateType::StringFunction => {
            // In a constant context, return as Template rather than erroring immediately.
            // WHY: constant dependency ordering resolves referenced constants before parsing;
            // if the template is still runtime-only here, constant-header validation emits the
            // permanent "not compile-time resolvable" error.
            maybe_consume_closing_parenthesis(token_stream, consume_closing_parenthesis);
            Ok(Some(expression_builder.template(template, *value_mode)?))
        }

        TemplateType::String => {
            maybe_consume_closing_parenthesis(token_stream, consume_closing_parenthesis);

            let store = template_context.template_ir_store.borrow();
            let reference = template.tir_reference;
            let view = TirView::with_minimum_phase(
                &store,
                reference.root,
                reference.phase,
                TemplateTirPhase::Composed,
                reference.context,
            )?;
            let preparation = prepare_tir_view_with_source_scope(
                &view,
                expression_builder,
                const_values,
                TemplatePreparationMode::Value,
                &template_context,
            )?;
            if !matches!(preparation.outcome, TemplatePreparationOutcome::Foldable) {
                return Ok(Some(expression_builder.template(template, *value_mode)?));
            }
            if matches!(
                preparation.facts.final_value_kind,
                TemplateConstValueKind::WrapperTemplate
            ) {
                return Ok(Some(expression_builder.template(template, *value_mode)?));
            }

            ast_log!("Template is foldable now. Folding...");

            let fold_result = {
                let mut fold_context = template_context.new_tir_fold_context(
                    string_table,
                    expression_builder,
                    const_values,
                );
                fold_prepared_template(&preparation, view, &mut fold_context)?
            };
            if fold_result.refusal.is_some() {
                // The value still exists at runtime: structural text anchored to a resource or
                // site root has no characters until the owning output boundary supplies them.
                return Ok(Some(expression_builder.template(template, *value_mode)?));
            }
            let string_value = match fold_result.emission {
                TemplateEmission::Output(value) => value,
                TemplateEmission::NoOutput => {
                    let empty = string_table.intern("");
                    ConstStringValue::Text(empty)
                }
            };
            let type_id = builtin_type_ids::STRING;
            let value = const_values.insert_resolved(
                type_id,
                ConstValuePayload::String(string_value),
                template_span,
            )?;
            let folded_expression =
                expression_builder.constant(value, type_id, template_span, *value_mode)?;
            let folded_expression = expression_builder
                .with_synthetic_interface_provenance(folded_expression, fold_result.provenance)?;
            Ok(Some(folded_expression))
        }

        // Comment templates do not produce an expression.
        TemplateType::Comment(_) => Ok(None),

        TemplateType::SlotInsert(_) => {
            maybe_consume_closing_parenthesis(token_stream, consume_closing_parenthesis);
            Ok(Some(expression_builder.template(template, *value_mode)?))
        }

        TemplateType::SlotDefinition(_) => {
            let diagnostic = crate::compiler_frontend::compiler_messages::diagnostics::families::templates::invalid_template_slot(InvalidTemplateSlotReason::SlotDefinitionOutsideTemplateBody,
            None,
            Some(token_stream.current_span()),);

            Err(diagnostic.into())
        }
    }
}

// Consume a trailing `)` when requested and one is present.
fn maybe_consume_closing_parenthesis(token_stream: &mut AstCursor, should_consume: bool) {
    if should_consume && token_stream.current_tag() == TokenTag::CLOSE_PARENTHESIS {
        token_stream.advance();
    }
}
