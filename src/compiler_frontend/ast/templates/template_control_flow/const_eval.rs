//! Const evaluation helpers for template control flow.
//!
//! Validation uses this module to reject const-required control-flow shapes that
//! would otherwise leak runtime work into const template contexts. Template-head
//! parsing also uses it for the small amount of source-constant inlining needed
//! before validation can recognize dependency-sorted const declarations.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::LoopBindings;
use crate::compiler_frontend::ast::const_eval::constant_fold;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpn, ExpressionRpnItem,
};
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use super::types::{TemplateBranchSelector, TemplateLoopHeader};

pub(crate) fn inline_source_consts_for_const_required_if_condition(
    condition: TemplateBranchSelector,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> TemplateBranchSelector {
    match condition {
        TemplateBranchSelector::Bool(condition) => TemplateBranchSelector::Bool(
            inline_source_consts_for_const_required_expression(condition, context, string_table),
        ),

        TemplateBranchSelector::OptionPresentCapture { scrutinee, pattern } => {
            TemplateBranchSelector::OptionPresentCapture {
                scrutinee: inline_source_consts_for_const_required_expression(
                    scrutinee,
                    context,
                    string_table,
                ),
                pattern,
            }
        }
    }
}

pub(crate) fn inline_source_consts_for_const_required_expression(
    expression: Expression,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> Expression {
    inline_source_consts_for_const_required_condition(expression, context, string_table)
}

pub(crate) fn loop_body_const_evaluation_bindings(
    header: &TemplateLoopHeader,
    inherited_loop_binding_paths: &[InternedPath],
) -> Vec<InternedPath> {
    let mut loop_binding_paths = inherited_loop_binding_paths.to_vec();
    match header {
        TemplateLoopHeader::Range { bindings, .. }
        | TemplateLoopHeader::Collection { bindings, .. } => {
            collect_loop_binding_paths(bindings, &mut loop_binding_paths);
        }

        TemplateLoopHeader::Conditional { .. } => {}
    }

    loop_binding_paths
}

pub(crate) fn collect_option_capture_binding_path(
    pattern: &MatchPattern,
    output: &mut Vec<InternedPath>,
) {
    if let MatchPattern::OptionPresentCapture { binding_path, .. } = pattern {
        output.push(binding_path.clone());
    }
}

fn collect_loop_binding_paths(bindings: &LoopBindings, output: &mut Vec<InternedPath>) {
    if let Some(item) = &bindings.item {
        output.push(item.id.clone());
    }

    if let Some(index) = &bindings.index {
        output.push(index.id.clone());
    }
}

fn inline_source_consts_for_const_required_condition(
    expression: Expression,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> Expression {
    // This is intentionally shape-preserving instead of using the broader
    // ConstValueResolver: option-present template `if` needs the outer `T?`
    // coercion to survive so validation and folding still see an option scrutinee.
    let substituted = substitute_source_consts_in_expression(expression, context, string_table);

    if matches!(substituted.kind, ExpressionKind::Runtime(_)) {
        return fold_substituted_runtime_condition(substituted, context, string_table);
    }

    substituted
}

fn substitute_source_consts_in_expression(
    expression: Expression,
    context: &ScopeContext,
    string_table: &StringTable,
) -> Expression {
    match &expression.kind {
        ExpressionKind::Reference(path) => source_const_value_for_path(path, context)
            .cloned()
            .unwrap_or(expression),

        ExpressionKind::Coerced { value, to_type } => {
            let resolved =
                substitute_source_consts_in_expression((**value).clone(), context, string_table);

            if matches!(resolved.kind, ExpressionKind::Reference(_)) {
                return expression;
            }

            Expression {
                kind: ExpressionKind::Coerced {
                    value: Box::new(resolved),
                    to_type: *to_type,
                },
                ..expression
            }
        }

        ExpressionKind::Runtime(rpn) => {
            let substituted_items = rpn
                .items
                .iter()
                .map(|item| substitute_source_consts_in_rpn_item(item, context, string_table))
                .collect();

            Expression {
                kind: ExpressionKind::Runtime(ExpressionRpn {
                    items: substituted_items,
                }),
                ..expression
            }
        }

        _ => expression,
    }
}

fn substitute_source_consts_in_rpn_item(
    item: &ExpressionRpnItem,
    context: &ScopeContext,
    string_table: &StringTable,
) -> ExpressionRpnItem {
    match item {
        ExpressionRpnItem::Operand(expression) => ExpressionRpnItem::Operand(
            substitute_source_consts_in_expression(expression.clone(), context, string_table),
        ),
        operator @ ExpressionRpnItem::Operator { .. } => operator.clone(),
    }
}

fn fold_substituted_runtime_condition(
    expression: Expression,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> Expression {
    let ExpressionKind::Runtime(rpn) = &expression.kind else {
        return expression;
    };

    // Folding consumes what it is given, and the unfolded condition is the answer whenever it
    // does not reduce to one compile-time value - so the items are copied and the condition
    // itself is returned by move.
    add_ast_counter(AstCounter::ExpressionOperandClones, rpn.items.len());

    match constant_fold(rpn.items.clone(), string_table) {
        Ok(mut stack) => {
            if stack.len() == 1
                && let Some(ExpressionRpnItem::Operand(folded)) = stack.pop()
                && expression_is_compile_time_constant_from_effective_tir(&folded, context)
                    .unwrap_or(false)
            {
                return folded;
            }

            expression
        }

        Err(_) => expression,
    }
}

fn source_const_value_for_path<'a>(
    path: &InternedPath,
    context: &'a ScopeContext,
) -> Option<&'a Expression> {
    let declaration = context
        .top_level_declarations
        .get_visible_resolved_by_path(path, context.visible_declaration_ids.as_deref())?;

    let decidable = source_const_value_is_condition_decidable(&declaration.value, context);

    if decidable {
        Some(&declaration.value)
    } else {
        None
    }
}

fn source_const_value_is_condition_decidable(
    expression: &Expression,
    context: &ScopeContext,
) -> bool {
    match &expression.kind {
        ExpressionKind::OptionNone => true,
        ExpressionKind::Coerced { value, .. } => {
            expression_is_compile_time_constant_from_effective_tir(value, context).unwrap_or(false)
        }
        _ => expression_is_compile_time_constant_from_effective_tir(expression, context)
            .unwrap_or(false),
    }
}

fn expression_is_compile_time_constant_from_effective_tir(
    expression: &Expression,
    context: &ScopeContext,
) -> Result<bool, TemplateError> {
    Ok(expression
        .const_value_kind_with_template_classifier(&mut |template| {
            classify_template_from_effective_tir(template, &context.template_ir_store)
        })?
        .is_compile_time_value())
}
