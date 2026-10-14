//! Source-constant substitution and binding visibility for template control flow.
//!
//! Substitution shares immutable declaration roots. The expression folder isolates changed
//! operation rows and returns the exact original root whenever opportunistic folding refuses.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::LoopBindings;
use crate::compiler_frontend::ast::const_eval::{BindingFold, fold_expression_with_bindings};
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionView,
};
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use super::types::{TemplateBranchSelector, TemplateLoopHeader};

pub(crate) fn inline_source_consts_for_const_required_if_condition(
    condition: TemplateBranchSelector,
    context: &ScopeContext,
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    string_table: &mut StringTable,
) -> Result<TemplateBranchSelector, TemplateError> {
    Ok(match condition {
        TemplateBranchSelector::Bool(condition) => {
            TemplateBranchSelector::Bool(inline_source_consts_for_const_required_expression(
                condition,
                context,
                expressions,
                constants,
                string_table,
            )?)
        }
        TemplateBranchSelector::OptionPresentCapture { scrutinee, pattern } => {
            TemplateBranchSelector::OptionPresentCapture {
                scrutinee: inline_source_consts_for_const_required_expression(
                    scrutinee,
                    context,
                    expressions,
                    constants,
                    string_table,
                )?,
                pattern,
            }
        }
    })
}

pub(crate) fn inline_source_consts_for_const_required_expression(
    expression: ExprId,
    context: &ScopeContext,
    expressions: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    string_table: &mut StringTable,
) -> Result<ExprId, TemplateError> {
    // The visibility-filtered declaration table is the source authority. Collect only paths
    // actually reached by this expression, without reconstructing declaration values.
    let mut substitutions = Vec::new();
    collect_source_substitutions(
        expressions.view(expression)?,
        context,
        expressions,
        constants,
        &mut substitutions,
    )?;
    if substitutions.is_empty() {
        return Ok(expression);
    }
    Ok(fold_expression_with_bindings(
        expression,
        expressions,
        constants,
        string_table,
        BindingFold::speculative(Some(context)),
        |path| {
            substitutions
                .iter()
                .find(|(binding, _)| *binding == path)
                .map(|(_, value)| *value)
        },
    )?)
}

fn collect_source_substitutions(
    expression: ExpressionView<'_>,
    context: &ScopeContext,
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
    substitutions: &mut Vec<(PathId, ExprId)>,
) -> Result<(), TemplateError> {
    if let crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView::Reference(path) = expression.operation()
        && let Some(declaration) = context.shared.top_level_declarations.get_visible_resolved_by_path(&path, context.visible_declaration_ids.as_deref())
        && let Some(initializer) = declaration.value.initializer()
    {
        let initializer_view = expression.bind(initializer)?;
        if expression_is_compile_time_constant_from_effective_tir(initializer_view, context, expressions, constants)? {
            substitutions.push((path, initializer));
        }
    }
    let mut failure = None;
    expression.visit_children(|child| {
        if failure.is_none() {
            failure =
                collect_source_substitutions(child, context, expressions, constants, substitutions)
                    .err();
        }
    });
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(())
}

pub(crate) fn loop_body_const_evaluation_bindings(
    header: &TemplateLoopHeader,
    inherited_loop_binding_paths: &[PathId],
) -> Vec<PathId> {
    let mut paths = inherited_loop_binding_paths.to_vec();
    match header {
        TemplateLoopHeader::Range { bindings, .. }
        | TemplateLoopHeader::Collection { bindings, .. } => {
            collect_loop_binding_paths(bindings, &mut paths)
        }
        TemplateLoopHeader::Conditional { .. } => {}
    }
    paths
}

pub(crate) fn collect_option_capture_binding_path(
    pattern: &MatchPattern,
    output: &mut Vec<PathId>,
) {
    if let MatchPattern::OptionPresentCapture { binding_path, .. } = pattern {
        output.push(*binding_path);
    }
}

fn collect_loop_binding_paths(bindings: &LoopBindings, output: &mut Vec<PathId>) {
    if let Some(item) = &bindings.item {
        output.push(item.id);
    }
    if let Some(index) = &bindings.index {
        output.push(index.id);
    }
}

fn expression_is_compile_time_constant_from_effective_tir(
    expression: ExpressionView<'_>,
    context: &ScopeContext,
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
) -> Result<bool, TemplateError> {
    Ok(expression
        .const_value_kind_with_template_classifier(constants, &mut |template| {
            classify_template_from_effective_tir(
                template,
                &context.template_ir_store,
                expressions,
                constants,
            )
        })?
        .is_compile_time_value())
}
