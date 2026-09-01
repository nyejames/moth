//! Body-local folded const-record insertion.
//!
//! WHAT: walks finalized AST function bodies and inserts each body-local const record into the
//! module [`ConstValueStore`] under its qualified declaration path.
//! WHY: HIR field projection must consume the same folded-value authority as module constants
//! instead of retaining AST declarations and reconstructing field lookup.

use super::store::{ConstTemplateValue, ConstValueStore, ConstValueStoreError};
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::expression_kind::ExpressionKind;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, PlaceExpression, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::expressions::expression_types::FallibleHandling;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::interned_path::InternedPath;

/// Fold every body-local const record in `nodes` into `store`.
///
/// Module-constant paths already present in the store are skipped. Body-local rows are path
/// bindings only; they do not become module-constant pool entries.
pub(crate) fn insert_body_local_const_records(
    store: &mut ConstValueStore,
    nodes: &[AstNode],
    type_environment: &TypeEnvironment,
    template_builder: &mut impl FnMut(
        Option<&InternedPath>,
        &Template,
    ) -> Result<ConstTemplateValue, ConstValueStoreError>,
) -> Result<(), ConstValueStoreError> {
    for node in nodes {
        insert_from_node(store, node, type_environment, template_builder)?;
    }

    Ok(())
}

fn insert_from_node(
    store: &mut ConstValueStore,
    node: &AstNode,
    type_environment: &TypeEnvironment,
    template_builder: &mut impl FnMut(
        Option<&InternedPath>,
        &Template,
    ) -> Result<ConstTemplateValue, ConstValueStoreError>,
) -> Result<(), ConstValueStoreError> {
    match &node.kind {
        NodeKind::VariableDeclaration(declaration) => {
            insert_from_expression(
                store,
                &declaration.value,
                type_environment,
                template_builder,
            )?;
            insert_declaration(store, declaration, type_environment, template_builder)
        }

        NodeKind::Function(_, _, body) | NodeKind::LexicalScope { body } => {
            insert_body_local_const_records(store, body, type_environment, template_builder)
        }

        NodeKind::If(condition, then_body, else_body, _) => {
            insert_from_expression(store, condition, type_environment, template_builder)?;
            insert_body_local_const_records(store, then_body, type_environment, template_builder)?;
            if let Some(else_body) = else_body {
                insert_body_local_const_records(
                    store,
                    else_body,
                    type_environment,
                    template_builder,
                )?;
            }
            Ok(())
        }

        NodeKind::Match {
            scrutinee,
            arms,
            default,
            ..
        } => {
            insert_from_expression(store, scrutinee, type_environment, template_builder)?;
            for arm in arms {
                insert_from_match_pattern(store, &arm.pattern, type_environment, template_builder)?;
                if let Some(guard) = &arm.guard {
                    insert_from_expression(store, guard, type_environment, template_builder)?;
                }
                insert_body_local_const_records(
                    store,
                    &arm.body,
                    type_environment,
                    template_builder,
                )?;
            }
            if let Some(default_body) = default {
                insert_body_local_const_records(
                    store,
                    default_body,
                    type_environment,
                    template_builder,
                )?;
            }
            Ok(())
        }

        NodeKind::RangeLoop { range, body, .. } => {
            insert_from_expression(store, &range.start, type_environment, template_builder)?;
            insert_from_expression(store, &range.end, type_environment, template_builder)?;
            if let Some(step) = &range.step {
                insert_from_expression(store, step, type_environment, template_builder)?;
            }
            insert_body_local_const_records(store, body, type_environment, template_builder)
        }

        NodeKind::CollectionLoop { iterable, body, .. } => {
            insert_from_expression(store, iterable, type_environment, template_builder)?;
            insert_body_local_const_records(store, body, type_environment, template_builder)
        }

        NodeKind::WhileLoop(condition, body) => {
            insert_from_expression(store, condition, type_environment, template_builder)?;
            insert_body_local_const_records(store, body, type_environment, template_builder)
        }

        NodeKind::Assert { condition, message } => {
            insert_from_expression(store, condition, type_environment, template_builder)?;
            insert_from_expression(store, message, type_environment, template_builder)
        }

        NodeKind::Return(expressions) => {
            insert_from_expressions(store, expressions, type_environment, template_builder)
        }

        NodeKind::ThenValue(produced_values) => insert_from_expressions(
            store,
            &produced_values.expressions,
            type_environment,
            template_builder,
        ),

        NodeKind::ReturnError(expression) | NodeKind::PushStartRuntimeFragment(expression) => {
            insert_from_expression(store, expression, type_environment, template_builder)
        }

        NodeKind::Assignment { value, .. } => {
            insert_from_expression(store, value, type_environment, template_builder)
        }

        NodeKind::MultiBind { value, .. } | NodeKind::ExpressionStatement(value) => {
            insert_from_expression(store, value, type_environment, template_builder)
        }

        NodeKind::StructDefinition(_, fields) => {
            for field in fields {
                insert_from_expression(store, &field.value, type_environment, template_builder)?;
            }
            Ok(())
        }

        NodeKind::Break | NodeKind::Continue => Ok(()),
    }
}

fn insert_declaration(
    store: &mut ConstValueStore,
    declaration: &Declaration,
    type_environment: &TypeEnvironment,
    template_builder: &mut impl FnMut(
        Option<&InternedPath>,
        &Template,
    ) -> Result<ConstTemplateValue, ConstValueStoreError>,
) -> Result<(), ConstValueStoreError> {
    if !declaration.value.is_const_record_value() {
        return Ok(());
    }

    if store.value_for_path(&declaration.id).is_some() {
        return Ok(());
    }

    store.insert_body_local_binding(declaration, type_environment, template_builder)
}

fn insert_from_expressions(
    store: &mut ConstValueStore,
    expressions: &[Expression],
    type_environment: &TypeEnvironment,
    template_builder: &mut impl FnMut(
        Option<&InternedPath>,
        &Template,
    ) -> Result<ConstTemplateValue, ConstValueStoreError>,
) -> Result<(), ConstValueStoreError> {
    for expression in expressions {
        insert_from_expression(store, expression, type_environment, template_builder)?;
    }
    Ok(())
}

fn insert_from_call_arguments(
    store: &mut ConstValueStore,
    arguments: &[CallArgument],
    type_environment: &TypeEnvironment,
    template_builder: &mut impl FnMut(
        Option<&InternedPath>,
        &Template,
    ) -> Result<ConstTemplateValue, ConstValueStoreError>,
) -> Result<(), ConstValueStoreError> {
    for argument in arguments {
        insert_from_expression(store, &argument.value, type_environment, template_builder)?;
    }
    Ok(())
}

fn insert_from_match_pattern(
    store: &mut ConstValueStore,
    pattern: &MatchPattern,
    type_environment: &TypeEnvironment,
    template_builder: &mut impl FnMut(
        Option<&InternedPath>,
        &Template,
    ) -> Result<ConstTemplateValue, ConstValueStoreError>,
) -> Result<(), ConstValueStoreError> {
    match pattern {
        MatchPattern::Literal(expression) => {
            insert_from_expression(store, expression, type_environment, template_builder)
        }
        MatchPattern::OptionValue { value, .. } | MatchPattern::Relational { value, .. } => {
            insert_from_expression(store, value, type_environment, template_builder)
        }
        MatchPattern::OptionNone { .. }
        | MatchPattern::ChoiceVariant { .. }
        | MatchPattern::OptionPresentCapture { .. } => Ok(()),
    }
}

fn insert_from_place(place: &PlaceExpression) {
    match &place.kind {
        PlaceExpressionKind::Local(_) => {}
        PlaceExpressionKind::Field { base, .. } => insert_from_place(base),
    }
}

fn insert_from_fallible_handling(
    store: &mut ConstValueStore,
    handling: &FallibleHandling,
    type_environment: &TypeEnvironment,
    template_builder: &mut impl FnMut(
        Option<&InternedPath>,
        &Template,
    ) -> Result<ConstTemplateValue, ConstValueStoreError>,
) -> Result<(), ConstValueStoreError> {
    let FallibleHandling::Handler { body, .. } = handling else {
        return Ok(());
    };
    insert_body_local_const_records(store, body, type_environment, template_builder)
}

fn insert_from_expression(
    store: &mut ConstValueStore,
    expression: &Expression,
    type_environment: &TypeEnvironment,
    template_builder: &mut impl FnMut(
        Option<&InternedPath>,
        &Template,
    ) -> Result<ConstTemplateValue, ConstValueStoreError>,
) -> Result<(), ConstValueStoreError> {
    match &expression.kind {
        ExpressionKind::Runtime(rpn) => {
            for item in &rpn.items {
                if let ExpressionRpnItem::Operand(operand) = item {
                    insert_from_expression(store, operand, type_environment, template_builder)?;
                }
            }
            Ok(())
        }

        ExpressionKind::Copy(place) => {
            insert_from_place(place);
            Ok(())
        }

        ExpressionKind::FieldAccess { base, .. } => {
            insert_from_expression(store, base, type_environment, template_builder)
        }

        ExpressionKind::MethodCall { receiver, args, .. }
        | ExpressionKind::CollectionBuiltinCall { receiver, args, .. }
        | ExpressionKind::MapBuiltinCall { receiver, args, .. } => {
            insert_from_expression(store, receiver, type_environment, template_builder)?;
            insert_from_call_arguments(store, args, type_environment, template_builder)
        }

        ExpressionKind::FunctionCall { args, .. }
        | ExpressionKind::HostFunctionCall { args, .. }
        | ExpressionKind::HandledFallibleFunctionCall { args, .. }
        | ExpressionKind::HandledFallibleHostFunctionCall { args, .. } => {
            insert_from_call_arguments(store, args, type_environment, template_builder)
        }

        ExpressionKind::HandledFallibleExpression { value, .. } => {
            insert_from_expression(store, value, type_environment, template_builder)
        }

        ExpressionKind::Cast(cast) => {
            insert_from_expression(store, &cast.source, type_environment, template_builder)
        }

        #[cfg(test)]
        ExpressionKind::FallibleCarrierConstruct { value, .. } => {
            insert_from_expression(store, value, type_environment, template_builder)
        }

        ExpressionKind::OptionPropagation { value } | ExpressionKind::Coerced { value, .. } => {
            insert_from_expression(store, value, type_environment, template_builder)
        }

        ExpressionKind::Collection(items) => {
            insert_from_expressions(store, items, type_environment, template_builder)
        }

        ExpressionKind::MapLiteral(entries) => {
            for entry in entries {
                insert_from_expression(store, &entry.key, type_environment, template_builder)?;
                insert_from_expression(store, &entry.value, type_environment, template_builder)?;
            }
            Ok(())
        }

        ExpressionKind::StructDefinition(fields)
        | ExpressionKind::StructInstance(fields)
        | ExpressionKind::AnonymousConstRecord { fields }
        | ExpressionKind::ChoiceConstruct { fields, .. } => {
            for field in fields {
                insert_from_expression(store, &field.value, type_environment, template_builder)?;
            }
            Ok(())
        }

        ExpressionKind::Range(start, end) => {
            insert_from_expression(store, start, type_environment, template_builder)?;
            insert_from_expression(store, end, type_environment, template_builder)
        }

        ExpressionKind::ValueBlock { block } => match block.as_ref() {
            ValueBlock::If(value_if) => {
                insert_from_expression(
                    store,
                    &value_if.condition,
                    type_environment,
                    template_builder,
                )?;
                insert_body_local_const_records(
                    store,
                    &value_if.then_body,
                    type_environment,
                    template_builder,
                )?;
                insert_body_local_const_records(
                    store,
                    &value_if.else_body,
                    type_environment,
                    template_builder,
                )
            }
            ValueBlock::LexicalScope(value_lexical_scope) => insert_body_local_const_records(
                store,
                &value_lexical_scope.body,
                type_environment,
                template_builder,
            ),
            ValueBlock::Match(value_match) => {
                insert_from_expression(
                    store,
                    &value_match.scrutinee,
                    type_environment,
                    template_builder,
                )?;
                for arm in &value_match.arms {
                    insert_from_match_pattern(
                        store,
                        &arm.pattern,
                        type_environment,
                        template_builder,
                    )?;
                    if let Some(guard) = &arm.guard {
                        insert_from_expression(store, guard, type_environment, template_builder)?;
                    }
                    insert_body_local_const_records(
                        store,
                        &arm.body,
                        type_environment,
                        template_builder,
                    )?;
                }
                if let Some(default_body) = &value_match.default {
                    insert_body_local_const_records(
                        store,
                        default_body,
                        type_environment,
                        template_builder,
                    )?;
                }
                Ok(())
            }
            ValueBlock::Catch(value_catch) => {
                insert_from_expression(
                    store,
                    &value_catch.handled_value,
                    type_environment,
                    template_builder,
                )?;
                insert_from_fallible_handling(
                    store,
                    &value_catch.handler,
                    type_environment,
                    template_builder,
                )
            }
        },

        ExpressionKind::NoValue
        | ExpressionKind::OptionNone
        | ExpressionKind::Int(_)
        | ExpressionKind::Float(_)
        | ExpressionKind::StringSlice(_)
        | ExpressionKind::StructuralString { .. }
        | ExpressionKind::Bool(_)
        | ExpressionKind::Char(_)
        | ExpressionKind::Reference(_)
        | ExpressionKind::Function(_)
        | ExpressionKind::Template(_)
        | ExpressionKind::RuntimeTemplateHandoff(_)
        | ExpressionKind::RuntimeSlotApplicationHandoff(_) => Ok(()),
    }
}
