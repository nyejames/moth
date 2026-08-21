//! Assert statement parsing.
//!
//! WHAT: parses the reserved `assert` statement through the shared call-argument contract.
//! WHY: assertion placement, unrecoverable failure, and message control-flow policy are special;
//!      parentheses, separators, named routing, defaults, access and type validation are not.

use std::collections::HashSet;

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::expressions::call_arguments::{
    NamedArgumentSyntax, parse_call_arguments_typed_with_expectations,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext, ExpectedAccessMode,
    ExpectedParameterType, ParameterExpectation, resolve_call_arguments,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, PlaceExpression, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::expressions::expression_types::CastHandling;
use crate::compiler_frontend::ast::statements::condition_validation::ensure_boolean_condition;
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
    walk_owned_runtime_slot_application_handoff, walk_owned_runtime_template_handoff,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrStore, TemplateTirReference, walk_expression_payloads_with_nested_tir_views,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};

pub(crate) fn parse_assert_statement(
    token_stream: &mut FileTokens,
    ast: &mut Vec<AstNode>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<(), ExpressionParseError> {
    let assert_location = token_stream.current_location();
    let assert_name = string_table.intern("assert");
    let condition_name = string_table.intern("condition");
    let message_name = string_table.intern("message");

    token_stream.advance(); // past `assert`

    let bool_type_id = type_interner.environment().builtins().bool;
    let string_type_id = type_interner.environment().builtins().string;
    let default_message = Expression::option_none_with_type_id(
        string_type_id,
        DataType::StringSlice,
        type_interner.environment_mut_for_derived_types(),
        assert_location.clone(),
    );
    let message_type_id = default_message.type_id;

    let expectations = [
        ParameterExpectation {
            name: Some(condition_name),
            expected_type: ExpectedParameterType::Known(bool_type_id),
            access_mode: ExpectedAccessMode::Shared,
            requires_reactive_source: false,
            default_value: None,
        },
        ParameterExpectation {
            name: Some(message_name),
            expected_type: ExpectedParameterType::Known(message_type_id),
            access_mode: ExpectedAccessMode::Shared,
            requires_reactive_source: false,
            default_value: Some(default_message),
        },
    ];

    let raw_arguments = parse_call_arguments_typed_with_expectations(
        token_stream,
        context,
        type_interner,
        string_table,
        &expectations,
        NamedArgumentSyntax::Supported {
            callee_name: Some(assert_name),
        },
    )?;

    let resolved_arguments = {
        let type_check_context = type_interner.type_check_context();
        resolve_call_arguments(
            CallDiagnosticContext::assertion("assert"),
            &raw_arguments,
            &expectations,
            assert_location.clone(),
            CallArgumentResolutionContext {
                string_table,
                type_environment: type_check_context.type_environment,
                compatibility_cache: type_check_context.compatibility_cache,
            },
        )?
    };

    let condition = resolved_arguments
        .first()
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Assertion call resolution did not produce a condition argument",
            )
        })?
        .value
        .clone();
    let message = resolved_arguments
        .get(1)
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "Assertion call resolution did not produce a message argument",
            )
        })?
        .value
        .clone();

    ensure_boolean_condition(&condition, &condition.location, type_interner.environment())
        .map_err(ExpressionParseError::Diagnostic)?;
    reject_assert_message_effect(&message, &context.template_ir_store.borrow())?;

    // Reject `assert(...)!` — assert is not a fallible expression.
    if token_stream.current_token_kind() == &TokenKind::Bang {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::BangOnNonFallible,
            token_stream.current_location(),
        )
        .into());
    }

    // Reject `assert(...) catch ...` — assert is not a fallible expression.
    if token_stream.current_token_kind() == &TokenKind::Catch {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::CatchOnNonFallible,
            token_stream.current_location(),
        )
        .into());
    }

    ast.push(AstNode {
        kind: NodeKind::Assert { condition, message },
        location: assert_location,
        scope: context.scope.clone(),
    });

    Ok(())
}

/// Rejects only semantic message effects that can escape the assertion's failure-edge value.
///
/// WHAT: walks resolved expression payloads, including nested call arguments and value blocks,
///       instead of looking back at tokens after parsing.
/// WHY: message evaluation must be an ordinary infallible value; propagating an error, propagating
///      an option, or returning through the enclosing function would bypass assertion failure.
fn reject_assert_message_effect(
    message: &Expression,
    template_ir_store: &TemplateIrStore,
) -> Result<(), ExpressionParseError> {
    if let Some(diagnostic) = assert_message_escape_diagnostic(message, template_ir_store)? {
        return Err(diagnostic.into());
    }

    Ok(())
}

/// Finds the user-facing diagnostic for an assertion message whose evaluation can escape.
///
/// WHAT: traverses ordinary AST expression payloads, nested value-block bodies, finalized TIR
///       expression sites, and owned runtime-template handoffs through their existing owners.
/// WHY: an assertion message is evaluated only on the failure edge, so propagation or terminal
///      control flow inside any nested payload would bypass the assertion's unrecoverable stop.
pub(crate) fn assert_message_escape_diagnostic(
    message: &Expression,
    template_ir_store: &TemplateIrStore,
) -> Result<Option<CompilerDiagnostic>, CompilerError> {
    let mut visited_templates = HashSet::new();
    let location =
        expression_assert_escape_location(message, template_ir_store, &mut visited_templates)?;

    Ok(location.map(|location| {
        CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::AssertionMessageCannotEscape,
            location,
        )
    }))
}

fn expression_assert_escape_location(
    expression: &Expression,
    template_ir_store: &TemplateIrStore,
    visited_templates: &mut HashSet<TemplateTirReference>,
) -> Result<Option<SourceLocation>, CompilerError> {
    match &expression.kind {
        ExpressionKind::HandledFallibleFunctionCall { args, handling, .. }
        | ExpressionKind::HandledFallibleHostFunctionCall { args, handling, .. } => {
            if matches!(handling, FallibleExpressionHandling::Propagate) {
                return Ok(Some(expression.location.clone()));
            }
            call_arguments_assert_escape_location(args, template_ir_store, visited_templates)
        }
        ExpressionKind::HandledFallibleExpression { value, handling } => {
            if matches!(handling, FallibleExpressionHandling::Propagate) {
                return Ok(Some(expression.location.clone()));
            }
            expression_assert_escape_location(value, template_ir_store, visited_templates)
        }
        ExpressionKind::OptionPropagation { .. } => Ok(Some(expression.location.clone())),
        ExpressionKind::Cast(cast) => {
            if matches!(cast.handling, CastHandling::Propagate) {
                return Ok(Some(expression.location.clone()));
            }
            expression_assert_escape_location(&cast.source, template_ir_store, visited_templates)
        }
        ExpressionKind::Runtime(rpn) => {
            for item in &rpn.items {
                if let ExpressionRpnItem::Operand(operand) = item
                    && let Some(location) = expression_assert_escape_location(
                        operand,
                        template_ir_store,
                        visited_templates,
                    )?
                {
                    return Ok(Some(location));
                }
            }
            Ok(None)
        }
        ExpressionKind::FunctionCall { args, .. }
        | ExpressionKind::HostFunctionCall { args, .. } => {
            call_arguments_assert_escape_location(args, template_ir_store, visited_templates)
        }
        ExpressionKind::MethodCall { receiver, args, .. }
        | ExpressionKind::CollectionBuiltinCall { receiver, args, .. }
        | ExpressionKind::MapBuiltinCall { receiver, args, .. } => {
            if let Some(location) =
                expression_assert_escape_location(receiver, template_ir_store, visited_templates)?
            {
                return Ok(Some(location));
            }
            call_arguments_assert_escape_location(args, template_ir_store, visited_templates)
        }
        ExpressionKind::FieldAccess { base, .. } => {
            expression_assert_escape_location(base, template_ir_store, visited_templates)
        }
        ExpressionKind::Copy(place) => place_expression_assert_escape_location(place),
        ExpressionKind::Collection(items) => {
            for item in items {
                if let Some(location) =
                    expression_assert_escape_location(item, template_ir_store, visited_templates)?
                {
                    return Ok(Some(location));
                }
            }
            Ok(None)
        }
        ExpressionKind::MapLiteral(entries) => {
            for entry in entries {
                if let Some(location) = expression_assert_escape_location(
                    &entry.key,
                    template_ir_store,
                    visited_templates,
                )? {
                    return Ok(Some(location));
                }
                if let Some(location) = expression_assert_escape_location(
                    &entry.value,
                    template_ir_store,
                    visited_templates,
                )? {
                    return Ok(Some(location));
                }
            }
            Ok(None)
        }
        ExpressionKind::StructInstance(fields) | ExpressionKind::ChoiceConstruct { fields, .. } => {
            for field in fields {
                if let Some(location) = expression_assert_escape_location(
                    &field.value,
                    template_ir_store,
                    visited_templates,
                )? {
                    return Ok(Some(location));
                }
            }
            Ok(None)
        }
        ExpressionKind::Range(start, end) => {
            if let Some(location) =
                expression_assert_escape_location(start, template_ir_store, visited_templates)?
            {
                return Ok(Some(location));
            }
            expression_assert_escape_location(end, template_ir_store, visited_templates)
        }
        ExpressionKind::Coerced { value, .. } => {
            expression_assert_escape_location(value, template_ir_store, visited_templates)
        }
        ExpressionKind::ValueBlock { block } => {
            value_block_assert_escape_location(block, template_ir_store, visited_templates)
        }
        ExpressionKind::Template(template) => {
            let reference = template.tir_reference;
            if !visited_templates.insert(reference) {
                return Ok(None);
            }

            let mut location = None;
            walk_expression_payloads_with_nested_tir_views(
                expression,
                template_ir_store,
                &mut |nested_expression| {
                    if location.is_none() {
                        location = expression_assert_escape_location(
                            nested_expression,
                            template_ir_store,
                            visited_templates,
                        )?;
                    }
                    Ok(())
                },
            )?;
            Ok(location)
        }
        ExpressionKind::RuntimeTemplateHandoff(handoff) => {
            runtime_template_handoff_assert_escape_location(
                handoff,
                template_ir_store,
                visited_templates,
            )
        }
        ExpressionKind::RuntimeSlotApplicationHandoff(handoff) => {
            runtime_slot_application_handoff_assert_escape_location(
                handoff,
                template_ir_store,
                visited_templates,
            )
        }
        ExpressionKind::NoValue
        | ExpressionKind::OptionNone
        | ExpressionKind::Int(_)
        | ExpressionKind::Float(_)
        | ExpressionKind::StringSlice(_)
        | ExpressionKind::Bool(_)
        | ExpressionKind::Char(_)
        | ExpressionKind::Reference(_)
        | ExpressionKind::Function(_)
        | ExpressionKind::StructDefinition(_) => Ok(None),
        #[cfg(test)]
        ExpressionKind::Path(_) => Ok(None),
        #[cfg(test)]
        ExpressionKind::FallibleCarrierConstruct { value, .. } => {
            expression_assert_escape_location(value, template_ir_store, visited_templates)
        }
    }
}

fn call_arguments_assert_escape_location(
    arguments: &[crate::compiler_frontend::ast::expressions::call_argument::CallArgument],
    template_ir_store: &TemplateIrStore,
    visited_templates: &mut HashSet<TemplateTirReference>,
) -> Result<Option<SourceLocation>, CompilerError> {
    for argument in arguments {
        if let Some(location) = expression_assert_escape_location(
            &argument.value,
            template_ir_store,
            visited_templates,
        )? {
            return Ok(Some(location));
        }
    }
    Ok(None)
}

fn place_expression_assert_escape_location(
    place: &PlaceExpression,
) -> Result<Option<SourceLocation>, CompilerError> {
    match &place.kind {
        PlaceExpressionKind::Local(_) => Ok(None),
        PlaceExpressionKind::Field { base, .. } => place_expression_assert_escape_location(base),
    }
}

fn value_block_assert_escape_location(
    block: &ValueBlock,
    template_ir_store: &TemplateIrStore,
    visited_templates: &mut HashSet<TemplateTirReference>,
) -> Result<Option<SourceLocation>, CompilerError> {
    match block {
        ValueBlock::If(value_if) => {
            if let Some(location) = expression_assert_escape_location(
                &value_if.condition,
                template_ir_store,
                visited_templates,
            )? {
                return Ok(Some(location));
            }
            if let Some(location) = nodes_assert_escape_location(
                &value_if.then_body,
                template_ir_store,
                visited_templates,
            )? {
                return Ok(Some(location));
            }
            nodes_assert_escape_location(&value_if.else_body, template_ir_store, visited_templates)
        }
        ValueBlock::Match(value_match) => {
            if let Some(location) = expression_assert_escape_location(
                &value_match.scrutinee,
                template_ir_store,
                visited_templates,
            )? {
                return Ok(Some(location));
            }
            for arm in &value_match.arms {
                if let Some(location) =
                    match_arm_assert_escape_location(arm, template_ir_store, visited_templates)?
                {
                    return Ok(Some(location));
                }
            }
            if let Some(default) = value_match.default.as_deref() {
                return nodes_assert_escape_location(default, template_ir_store, visited_templates);
            }
            Ok(None)
        }
        ValueBlock::Catch(value_catch) => {
            if let Some(location) = expression_assert_escape_location(
                &value_catch.handled_value,
                template_ir_store,
                visited_templates,
            )? {
                return Ok(Some(location));
            }
            match &value_catch.handler {
                FallibleHandling::Propagate => Ok(Some(value_catch.handled_value.location.clone())),
                FallibleHandling::Handler { body, .. } => {
                    nodes_assert_escape_location(body, template_ir_store, visited_templates)
                }
            }
        }
    }
}

fn match_arm_assert_escape_location(
    arm: &MatchArm,
    template_ir_store: &TemplateIrStore,
    visited_templates: &mut HashSet<TemplateTirReference>,
) -> Result<Option<SourceLocation>, CompilerError> {
    if let Some(guard) = &arm.guard
        && let Some(location) =
            expression_assert_escape_location(guard, template_ir_store, visited_templates)?
    {
        return Ok(Some(location));
    }
    nodes_assert_escape_location(&arm.body, template_ir_store, visited_templates)
}

fn declarations_assert_escape_location(
    declarations: &[Declaration],
    template_ir_store: &TemplateIrStore,
    visited_templates: &mut HashSet<TemplateTirReference>,
) -> Result<Option<SourceLocation>, CompilerError> {
    for declaration in declarations {
        if let Some(location) = expression_assert_escape_location(
            &declaration.value,
            template_ir_store,
            visited_templates,
        )? {
            return Ok(Some(location));
        }
    }
    Ok(None)
}

fn nodes_assert_escape_location(
    nodes: &[AstNode],
    template_ir_store: &TemplateIrStore,
    visited_templates: &mut HashSet<TemplateTirReference>,
) -> Result<Option<SourceLocation>, CompilerError> {
    for node in nodes {
        let location = match &node.kind {
            NodeKind::Return(_)
            | NodeKind::ReturnError(_)
            | NodeKind::Break
            | NodeKind::Continue => Some(node.location.clone()),
            NodeKind::If(condition, then_body, else_body) => {
                expression_assert_escape_location(condition, template_ir_store, visited_templates)?
                    .or(nodes_assert_escape_location(
                        then_body,
                        template_ir_store,
                        visited_templates,
                    )?)
                    .or(match else_body.as_deref() {
                        Some(body) => nodes_assert_escape_location(
                            body,
                            template_ir_store,
                            visited_templates,
                        )?,
                        None => None,
                    })
            }
            NodeKind::Match {
                scrutinee,
                arms,
                default,
                ..
            } => {
                let mut location = expression_assert_escape_location(
                    scrutinee,
                    template_ir_store,
                    visited_templates,
                )?;
                if location.is_none() {
                    for arm in arms {
                        location = match_arm_assert_escape_location(
                            arm,
                            template_ir_store,
                            visited_templates,
                        )?;
                        if location.is_some() {
                            break;
                        }
                    }
                }
                if location.is_none()
                    && let Some(default) = default.as_deref()
                {
                    location = nodes_assert_escape_location(
                        default,
                        template_ir_store,
                        visited_templates,
                    )?;
                }
                location
            }
            NodeKind::ScopedBlock { body } => {
                nodes_assert_escape_location(body, template_ir_store, visited_templates)?
            }
            NodeKind::RangeLoop { range, body, .. } => {
                let mut location = expression_assert_escape_location(
                    &range.start,
                    template_ir_store,
                    visited_templates,
                )?;
                if location.is_none() {
                    location = expression_assert_escape_location(
                        &range.end,
                        template_ir_store,
                        visited_templates,
                    )?;
                }
                if location.is_none()
                    && let Some(step) = &range.step
                {
                    location = expression_assert_escape_location(
                        step,
                        template_ir_store,
                        visited_templates,
                    )?;
                }
                if location.is_none() {
                    location =
                        nodes_assert_escape_location(body, template_ir_store, visited_templates)?;
                }
                location
            }
            NodeKind::CollectionLoop { iterable, body, .. } => {
                expression_assert_escape_location(iterable, template_ir_store, visited_templates)?
                    .or(nodes_assert_escape_location(
                        body,
                        template_ir_store,
                        visited_templates,
                    )?)
            }
            NodeKind::WhileLoop(condition, body) => {
                expression_assert_escape_location(condition, template_ir_store, visited_templates)?
                    .or(nodes_assert_escape_location(
                        body,
                        template_ir_store,
                        visited_templates,
                    )?)
            }
            NodeKind::VariableDeclaration(Declaration { value, .. })
            | NodeKind::ExpressionStatement(value)
            | NodeKind::PushStartRuntimeFragment(value) => {
                expression_assert_escape_location(value, template_ir_store, visited_templates)?
            }
            NodeKind::Assignment { value, .. } | NodeKind::MultiBind { value, .. } => {
                expression_assert_escape_location(value, template_ir_store, visited_templates)?
            }
            NodeKind::Assert { condition, message } => {
                expression_assert_escape_location(condition, template_ir_store, visited_templates)?
                    .or(expression_assert_escape_location(
                        message,
                        template_ir_store,
                        visited_templates,
                    )?)
            }
            NodeKind::Function(_, _, body) => {
                nodes_assert_escape_location(body, template_ir_store, visited_templates)?
            }
            NodeKind::StructDefinition(_, fields) => {
                declarations_assert_escape_location(fields, template_ir_store, visited_templates)?
            }
            NodeKind::ThenValue(values) => {
                let mut location = None;
                for expression in &values.expressions {
                    location = expression_assert_escape_location(
                        expression,
                        template_ir_store,
                        visited_templates,
                    )?;
                    if location.is_some() {
                        break;
                    }
                }
                location
            }
        };
        if let Some(location) = location {
            return Ok(Some(location));
        }
    }

    Ok(None)
}

fn runtime_template_handoff_assert_escape_location(
    handoff: &OwnedRuntimeTemplateHandoff,
    template_ir_store: &TemplateIrStore,
    visited_templates: &mut HashSet<TemplateTirReference>,
) -> Result<Option<SourceLocation>, CompilerError> {
    let mut location = None;
    walk_owned_runtime_template_handoff(handoff, &mut |node| {
        if location.is_some() {
            return Ok(());
        }
        location = match node {
            OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
                expression_assert_escape_location(expression, template_ir_store, visited_templates)?
            }
            OwnedRuntimeTemplateNode::BranchChain { branches, .. } => {
                let mut selector_location = None;
                for branch in branches {
                    selector_location = template_branch_selector_assert_escape_location(
                        &branch.selector,
                        template_ir_store,
                        visited_templates,
                    )?;
                    if selector_location.is_some() {
                        break;
                    }
                }
                selector_location
            }
            OwnedRuntimeTemplateNode::Loop { header, .. } => {
                template_loop_header_assert_escape_location(
                    header,
                    template_ir_store,
                    visited_templates,
                )?
            }
            _ => None,
        };
        Ok(())
    })?;
    Ok(location)
}

fn runtime_slot_application_handoff_assert_escape_location(
    handoff: &OwnedRuntimeSlotApplicationHandoff,
    template_ir_store: &TemplateIrStore,
    visited_templates: &mut HashSet<TemplateTirReference>,
) -> Result<Option<SourceLocation>, CompilerError> {
    let mut location = None;
    walk_owned_runtime_slot_application_handoff(handoff, &mut |node| {
        if location.is_some() {
            return Ok(());
        }
        location = match node {
            OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
                expression_assert_escape_location(expression, template_ir_store, visited_templates)?
            }
            OwnedRuntimeTemplateNode::BranchChain { branches, .. } => {
                let mut selector_location = None;
                for branch in branches {
                    selector_location = template_branch_selector_assert_escape_location(
                        &branch.selector,
                        template_ir_store,
                        visited_templates,
                    )?;
                    if selector_location.is_some() {
                        break;
                    }
                }
                selector_location
            }
            OwnedRuntimeTemplateNode::Loop { header, .. } => {
                template_loop_header_assert_escape_location(
                    header,
                    template_ir_store,
                    visited_templates,
                )?
            }
            _ => None,
        };
        Ok(())
    })?;
    Ok(location)
}

fn template_branch_selector_assert_escape_location(
    selector: &TemplateBranchSelector,
    template_ir_store: &TemplateIrStore,
    visited_templates: &mut HashSet<TemplateTirReference>,
) -> Result<Option<SourceLocation>, CompilerError> {
    match selector {
        TemplateBranchSelector::Bool(condition) => {
            expression_assert_escape_location(condition, template_ir_store, visited_templates)
        }
        TemplateBranchSelector::OptionPresentCapture {
            scrutinee, pattern, ..
        } => {
            if let Some(location) =
                expression_assert_escape_location(scrutinee, template_ir_store, visited_templates)?
            {
                return Ok(Some(location));
            }
            match pattern.as_ref() {
                MatchPattern::Literal(expression)
                | MatchPattern::OptionValue {
                    value: expression, ..
                }
                | MatchPattern::Relational {
                    value: expression, ..
                } => expression_assert_escape_location(
                    expression,
                    template_ir_store,
                    visited_templates,
                ),
                MatchPattern::OptionNone { .. }
                | MatchPattern::OptionPresentCapture { .. }
                | MatchPattern::ChoiceVariant { .. } => Ok(None),
            }
        }
    }
}

fn template_loop_header_assert_escape_location(
    header: &TemplateLoopHeader,
    template_ir_store: &TemplateIrStore,
    visited_templates: &mut HashSet<TemplateTirReference>,
) -> Result<Option<SourceLocation>, CompilerError> {
    match header {
        TemplateLoopHeader::Conditional { condition } => {
            expression_assert_escape_location(condition, template_ir_store, visited_templates)
        }
        TemplateLoopHeader::Range { range, .. } => {
            if let Some(location) = expression_assert_escape_location(
                &range.start,
                template_ir_store,
                visited_templates,
            )? {
                return Ok(Some(location));
            }
            if let Some(location) =
                expression_assert_escape_location(&range.end, template_ir_store, visited_templates)?
            {
                return Ok(Some(location));
            }
            match &range.step {
                Some(step) => {
                    expression_assert_escape_location(step, template_ir_store, visited_templates)
                }
                None => Ok(None),
            }
        }
        TemplateLoopHeader::Collection { iterable, .. } => {
            expression_assert_escape_location(iterable, template_ir_store, visited_templates)
        }
    }
}

#[cfg(test)]
#[path = "tests/assertion_message_tests.rs"]
mod assertion_message_tests;
