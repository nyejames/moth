//! Debug-only validation for AST TypeId boundaries.
//!
//! WHAT: walks the finalized AST payloads and asserts that every carried `TypeId` exists in the
//! final module `TypeEnvironment`.
//! WHY: finalization is the last AST-owned boundary before HIR lowering. Keeping this check local
//! to finalization catches stale or orphaned semantic type IDs early during debug builds without
//! mixing recursive validation mechanics into the final assembly code.

use crate::compiler_frontend::ast::AstChoiceDefinition;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, LoopBindings, NodeKind};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, PlaceExpression, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::expressions::expression_types::CastHandling;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrStore, finalized_tir_view_for_template, walk_tir_view_expression_payloads,
};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff,
    OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantPayloadDefinition, TypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;

/// Context shared by every helper in this debug validation pass.
///
/// WHAT: bundles the final module `TypeEnvironment` with the module-scoped
///       `TemplateIrStore` and `TemplateIrStore` so template-expression
///       payload validation resolves one required finalized `TirView`.
/// WHY: debug validation is read-only and short-lived; a small context struct
///      keeps the recursive walk signatures focused.
struct DebugTypeValidationContext<'a> {
    type_environment: &'a TypeEnvironment,
    template_ir_store: &'a TemplateIrStore,
}

/// Entry point for debug TypeId validation before HIR lowering.
///
/// WHAT: recursively walks every AST node, module constant, and choice
/// definition to verify that all referenced `TypeId`s resolve to real,
/// non-generic definitions in the module `TypeEnvironment`.
/// WHY: catches frontend bugs where unresolved or stale type identities leak
/// across the AST→HIR boundary, where they would cause opaque failures in later
/// stages.
pub(super) fn debug_validate_type_ids_for_hir(
    nodes: &[AstNode],
    const_values: &ConstValueStore,
    choice_definitions: &[AstChoiceDefinition],
    type_environment: &TypeEnvironment,
    template_ir_store: &TemplateIrStore,
) {
    let context = DebugTypeValidationContext {
        type_environment,
        template_ir_store,
    };

    for node in nodes {
        debug_validate_node_type_ids(node, &context);
    }

    if let Err(error) = const_values.validate_type_ids(type_environment) {
        debug_assert!(false, "{error:?}");
    }

    // Choice definitions are collected separately from the AST node tree.
    // Validate that their variant payload fields also carry resolved TypeIds.
    for choice_definition in choice_definitions {
        let Some(nominal_id) =
            type_environment.nominal_id_for_path(&choice_definition.nominal_path)
        else {
            debug_assert!(
                false,
                "AST choice definition path is missing from TypeEnvironment"
            );
            continue;
        };

        let Some(type_id) = type_environment.type_id_for_nominal_id(nominal_id) else {
            debug_assert!(
                false,
                "AST choice definition nominal id is missing a canonical TypeId"
            );
            continue;
        };

        let Some(variants) = type_environment.variants_for(type_id) else {
            debug_assert!(
                false,
                "AST choice definition TypeId is missing variant metadata"
            );
            continue;
        };

        for variant in variants {
            if let ChoiceVariantPayloadDefinition::Record { fields } = &variant.payload {
                for field in fields {
                    debug_validate_type_id(field.type_id, type_environment, "choice field");
                }
            }
        }
    }
}

fn debug_validate_node_type_ids(node: &AstNode, context: &DebugTypeValidationContext) {
    match &node.kind {
        NodeKind::Return(values) => {
            debug_validate_expressions_type_ids(values, context);
        }

        NodeKind::ReturnError(value)
        | NodeKind::PushStartRuntimeFragment(value)
        | NodeKind::ExpressionStatement(value) => {
            debug_validate_expression_type_id(value, context);
        }

        NodeKind::If(condition, then_body, else_body) => {
            debug_validate_expression_type_id(condition, context);
            debug_validate_nodes_type_ids(then_body, context);
            if let Some(else_body) = else_body {
                debug_validate_nodes_type_ids(else_body, context);
            }
        }

        NodeKind::Assert { condition, message } => {
            debug_validate_expression_type_id(condition, context);
            debug_validate_expression_type_id(message, context);
        }

        NodeKind::Match {
            scrutinee,
            arms,
            default,
            ..
        } => {
            debug_validate_expression_type_id(scrutinee, context);
            for arm in arms {
                debug_validate_match_pattern_type_ids(&arm.pattern, context);
                if let Some(guard) = &arm.guard {
                    debug_validate_expression_type_id(guard, context);
                }
                debug_validate_nodes_type_ids(&arm.body, context);
            }
            if let Some(default) = default {
                debug_validate_nodes_type_ids(default, context);
            }
        }

        NodeKind::ScopedBlock { body } => {
            debug_validate_nodes_type_ids(body, context);
        }

        NodeKind::RangeLoop {
            bindings,
            range,
            body,
        } => {
            debug_validate_loop_bindings_type_ids(bindings, context);
            debug_validate_expression_type_id(&range.start, context);
            debug_validate_expression_type_id(&range.end, context);
            if let Some(step) = &range.step {
                debug_validate_expression_type_id(step, context);
            }
            debug_validate_nodes_type_ids(body, context);
        }

        NodeKind::CollectionLoop {
            bindings,
            iterable,
            body,
        } => {
            debug_validate_loop_bindings_type_ids(bindings, context);
            debug_validate_expression_type_id(iterable, context);
            debug_validate_nodes_type_ids(body, context);
        }

        NodeKind::WhileLoop(condition, body) => {
            debug_validate_expression_type_id(condition, context);
            debug_validate_nodes_type_ids(body, context);
        }

        NodeKind::VariableDeclaration(declaration) => {
            debug_validate_declaration_type_id(declaration, context);
        }

        NodeKind::StructDefinition(_, fields) => {
            debug_validate_declarations_type_ids(fields, context);
        }

        NodeKind::Function(_, signature, body) => {
            debug_validate_declarations_type_ids(&signature.parameters, context);
            debug_validate_type_ids(
                &signature.success_return_type_ids(),
                context.type_environment,
                "function success return",
            );
            if let Some(error_return) = signature.error_return_type_id() {
                debug_validate_type_id(
                    error_return,
                    context.type_environment,
                    "function error return",
                );
            }
            debug_validate_nodes_type_ids(body, context);
        }

        NodeKind::Assignment { target, value } => {
            debug_validate_place_expression_type_ids(target, context);
            debug_validate_expression_type_id(value, context);
        }

        NodeKind::MultiBind { targets, value } => {
            for target in targets {
                debug_validate_type_id(
                    target.type_id,
                    context.type_environment,
                    "multi-bind target",
                );
            }
            debug_validate_expression_type_id(value, context);
        }

        // Terminal nodes that carry no type identities.
        NodeKind::Break | NodeKind::Continue => {}

        NodeKind::ThenValue(produced_values) => {
            debug_validate_expressions_type_ids(&produced_values.expressions, context);
        }
    }
}

fn debug_validate_place_expression_type_ids(
    place: &PlaceExpression,
    context: &DebugTypeValidationContext,
) {
    debug_validate_type_id(place.type_id, context.type_environment, "place expression");
    match &place.kind {
        PlaceExpressionKind::Local(_) => {}
        PlaceExpressionKind::Field { base, .. } => {
            debug_validate_place_expression_type_ids(base, context)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpressionValidationContext {
    Ordinary,
    ValueCatchHandledValue,
}

fn debug_validate_expression_type_id(
    expression: &Expression,
    context: &DebugTypeValidationContext,
) {
    debug_validate_expression_type_id_with_context(
        expression,
        context,
        ExpressionValidationContext::Ordinary,
    );
}

fn debug_validate_expression_type_id_with_context(
    expression: &Expression,
    context: &DebugTypeValidationContext,
    expression_context: ExpressionValidationContext,
) {
    debug_validate_type_id(expression.type_id, context.type_environment, "expression");

    match &expression.kind {
        ExpressionKind::Runtime(rpn) => {
            for item in &rpn.items {
                match item {
                    ExpressionRpnItem::Operand(expression) => {
                        debug_validate_expression_type_id(expression, context);
                    }
                    ExpressionRpnItem::Operator { .. } => {}
                }
            }
        }

        ExpressionKind::Copy(place) => {
            debug_validate_place_expression_type_ids(place, context);
        }

        ExpressionKind::FieldAccess { base, .. } => {
            debug_validate_expression_type_id(base, context);
        }

        ExpressionKind::MethodCall { receiver, args, .. }
        | ExpressionKind::CollectionBuiltinCall { receiver, args, .. }
        | ExpressionKind::MapBuiltinCall { receiver, args, .. } => {
            debug_validate_expression_type_id(receiver, context);
            debug_validate_call_arguments_type_ids(args, context);
        }

        ExpressionKind::Function(signature) => {
            debug_validate_declarations_type_ids(&signature.parameters, context);
            debug_validate_type_ids(
                &signature.success_return_type_ids(),
                context.type_environment,
                "function expression success return",
            );
            if let Some(error_return) = signature.error_return_type_id() {
                debug_validate_type_id(
                    error_return,
                    context.type_environment,
                    "function expression error return",
                );
            }
        }

        ExpressionKind::FunctionCall {
            args,
            result_type_ids,
            ..
        }
        | ExpressionKind::HostFunctionCall {
            args,
            result_type_ids,
            ..
        } => {
            debug_validate_call_arguments_type_ids(args, context);
            debug_validate_type_ids(
                result_type_ids,
                context.type_environment,
                "expression call result",
            );
        }

        ExpressionKind::HandledFallibleFunctionCall {
            args,
            result_type_ids,
            handling,
            ..
        } => {
            debug_validate_recover_marker_context(
                *handling,
                expression_context,
                "fallible function call",
            );
            debug_validate_call_arguments_type_ids(args, context);
            debug_validate_type_ids(
                result_type_ids,
                context.type_environment,
                "fallible-handled expression call result",
            );
        }

        ExpressionKind::HandledFallibleHostFunctionCall {
            args,
            result_type_ids,
            error_type_id,
            handling,
            ..
        } => {
            debug_validate_recover_marker_context(
                *handling,
                expression_context,
                "fallible external function call",
            );
            debug_validate_call_arguments_type_ids(args, context);
            debug_validate_type_ids(
                result_type_ids,
                context.type_environment,
                "fallible-handled external expression call result",
            );
            debug_validate_type_ids(
                &[*error_type_id],
                context.type_environment,
                "external call error",
            );
        }

        ExpressionKind::Cast(cast) => {
            if matches!(&cast.handling, CastHandling::Recover)
                && expression_context != ExpressionValidationContext::ValueCatchHandledValue
            {
                debug_assert!(
                    false,
                    "recovering cast expression reached AST finalization outside ValueBlock::Catch"
                );
            }
            debug_validate_expression_type_id(&cast.source, context);
            debug_validate_type_id(cast.target_type_id, context.type_environment, "cast target");
            debug_validate_type_id(cast.source_type_id, context.type_environment, "cast source");
        }

        #[cfg(test)]
        ExpressionKind::FallibleCarrierConstruct { value, .. } => {
            debug_validate_expression_type_id(value, context);
        }

        ExpressionKind::OptionPropagation { value } => {
            debug_validate_expression_type_id(value, context);
        }

        ExpressionKind::HandledFallibleExpression {
            value, handling, ..
        } => {
            debug_validate_recover_marker_context(
                *handling,
                expression_context,
                "fallible result expression",
            );
            debug_validate_expression_type_id(value, context);
        }

        ExpressionKind::Template(template) => {
            // Finalized effective TIR remains authoritative until the template is
            // replaced by one of the owned runtime handoff variants below.
            debug_validate_template_expression_payloads(template, context);

            // The finalizer materializes the TIR-derived owned wrapper node
            // before this debug check runs, so it is an authoritative expression
            // source for conditional wrapper output.
        }

        ExpressionKind::RuntimeTemplateHandoff(handoff) => {
            debug_validate_runtime_template_handoff_type_ids(handoff, context);
        }

        ExpressionKind::RuntimeSlotApplicationHandoff(handoff) => {
            debug_validate_runtime_slot_application_handoff_type_ids(handoff, context);
        }

        ExpressionKind::Collection(items) => {
            debug_validate_expressions_type_ids(items, context);
        }

        ExpressionKind::MapLiteral(entries) => {
            for entry in entries {
                debug_validate_expression_type_id(&entry.key, context);
                debug_validate_expression_type_id(&entry.value, context);
            }
        }

        ExpressionKind::StructDefinition(fields) | ExpressionKind::StructInstance(fields) => {
            debug_validate_declarations_type_ids(fields, context);
        }

        ExpressionKind::Range(start, end) => {
            debug_validate_expression_type_id(start, context);
            debug_validate_expression_type_id(end, context);
        }

        ExpressionKind::Coerced { value, to_type } => {
            debug_validate_expression_type_id(value, context);
            debug_validate_type_id(*to_type, context.type_environment, "coercion target");
        }

        ExpressionKind::ChoiceConstruct { fields, .. } => {
            debug_validate_declarations_type_ids(fields, context);
        }

        ExpressionKind::ValueBlock { block } => match block.as_ref() {
            ValueBlock::If(value_if) => {
                debug_validate_expression_type_id(&value_if.condition, context);
                debug_validate_nodes_type_ids(&value_if.then_body, context);
                debug_validate_nodes_type_ids(&value_if.else_body, context);
            }
            ValueBlock::Match(value_match) => {
                debug_validate_expression_type_id(&value_match.scrutinee, context);
                for arm in &value_match.arms {
                    if let Some(guard) = &arm.guard {
                        debug_validate_expression_type_id(guard, context);
                    }
                    debug_validate_nodes_type_ids(&arm.body, context);
                }
                if let Some(default_body) = &value_match.default {
                    debug_validate_nodes_type_ids(default_body, context);
                }
            }
            ValueBlock::Catch(value_catch) => {
                debug_assert!(
                    matches!(value_catch.handler, FallibleHandling::Handler { .. }),
                    "ValueBlock::Catch carried non-handler fallible handling"
                );
                debug_assert!(
                    expression_is_recovering_catch_subject(&value_catch.handled_value),
                    "ValueBlock::Catch handled value was not marked for recovery"
                );
                debug_validate_expression_type_id_with_context(
                    &value_catch.handled_value,
                    context,
                    ExpressionValidationContext::ValueCatchHandledValue,
                );
                debug_validate_fallible_handling_type_ids(&value_catch.handler, context);
            }
        },

        // Leaf expressions that carry no nested type identities.
        ExpressionKind::NoValue
        | ExpressionKind::OptionNone
        | ExpressionKind::Int(_)
        | ExpressionKind::Float(_)
        | ExpressionKind::StringSlice(_)
        | ExpressionKind::Bool(_)
        | ExpressionKind::Char(_)
        | ExpressionKind::Reference(_) => {}

        #[cfg(test)]
        ExpressionKind::Path(_) => {}
    }
}

fn expression_is_recovering_catch_subject(expression: &Expression) -> bool {
    match &expression.kind {
        ExpressionKind::HandledFallibleFunctionCall { handling, .. }
        | ExpressionKind::HandledFallibleHostFunctionCall { handling, .. }
        | ExpressionKind::HandledFallibleExpression { handling, .. } => {
            matches!(handling, FallibleExpressionHandling::Recover)
        }

        ExpressionKind::Cast(cast) => matches!(&cast.handling, CastHandling::Recover),

        _ => false,
    }
}

fn debug_validate_recover_marker_context(
    handling: FallibleExpressionHandling,
    context: ExpressionValidationContext,
    owner: &str,
) {
    if matches!(handling, FallibleExpressionHandling::Recover)
        && context != ExpressionValidationContext::ValueCatchHandledValue
    {
        debug_assert!(
            false,
            "recovering {owner} reached AST finalization outside ValueBlock::Catch"
        );
    }
}

fn debug_validate_match_pattern_type_ids(
    pattern: &MatchPattern,
    context: &DebugTypeValidationContext,
) {
    match pattern {
        MatchPattern::Literal(expression) => {
            debug_validate_expression_type_id(expression, context);
        }

        MatchPattern::OptionValue { value, .. } => {
            debug_validate_expression_type_id(value, context);
        }

        MatchPattern::Relational { value, .. } => {
            debug_validate_expression_type_id(value, context);
        }

        MatchPattern::ChoiceVariant { captures, .. } => {
            for capture in captures {
                debug_validate_type_id(capture.type_id, context.type_environment, "choice capture");
            }
        }

        // Patterns that carry no nested type identities.
        MatchPattern::OptionNone { .. } | MatchPattern::OptionPresentCapture { .. } => {}
    }
}

fn debug_validate_fallible_handling_type_ids(
    handling: &FallibleHandling,
    context: &DebugTypeValidationContext,
) {
    match handling {
        FallibleHandling::Handler { body, .. } => {
            debug_validate_nodes_type_ids(body, context);
        }

        FallibleHandling::Propagate => {}
    }
}

/// Validates a template's nested expression payloads through one finalized `TirView`.
///
/// WHAT: resolves the required finalized module-store `TirView` for the
///       template and walks its effective expression payloads. Effective
///       expression overlays are authoritative for dynamic-expression splices,
///       branch selectors and loop headers. A template that reaches this
///       boundary without a Finalized module-store identity is an internal
///       compiler invariant violation.
/// WHY: debug validation consumes the same effective TIR representation that
///      later phases consume. The walk returns `()`; the required-view error
///      and any walk failure are debug-asserted so a release build never
///      silently downgrades to a raw same-store path.
fn debug_validate_template_expression_payloads(
    template: &Template,
    context: &DebugTypeValidationContext,
) {
    match finalized_tir_view_for_template(template, context.template_ir_store) {
        Ok(view) => {
            let result = walk_tir_view_expression_payloads(&view, &mut |expression| {
                debug_validate_expression_type_id(expression, context);
                Ok(())
            });
            debug_assert!(
                result.is_ok(),
                "TIR view expression-payload walk failed during debug TypeId validation: {:?}",
                result.err()
            );
        }

        Err(error) => {
            debug_assert!(
                false,
                "finalized TIR view resolution failed during debug TypeId validation: {error:?}"
            );
        }
    }
}
fn debug_validate_loop_bindings_type_ids(
    bindings: &LoopBindings,
    context: &DebugTypeValidationContext,
) {
    if let Some(item) = &bindings.item {
        debug_validate_declaration_type_id(item, context);
    }
    if let Some(index) = &bindings.index {
        debug_validate_declaration_type_id(index, context);
    }
}

// -------------------------
//  Owned runtime-template handoff traversal
// -------------------------
//
// These helpers walk the owned runtime handoff shapes that the finalizer
// materializes before this debug check runs so every dynamic expression
// payload carried by finalized HIR-bound templates is validated.

fn debug_validate_runtime_template_handoff_type_ids(
    handoff: &OwnedRuntimeTemplateHandoff,
    context: &DebugTypeValidationContext,
) {
    debug_validate_runtime_template_body_type_ids(&handoff.body, context);
}

fn debug_validate_runtime_template_body_type_ids(
    body: &OwnedRuntimeTemplateBody,
    context: &DebugTypeValidationContext,
) {
    match body {
        OwnedRuntimeTemplateBody::Render(node) => {
            debug_validate_runtime_template_node_type_ids(node, context);
        }

        OwnedRuntimeTemplateBody::RuntimeSlotApplication(slot_handoff) => {
            debug_validate_runtime_slot_application_handoff_type_ids(slot_handoff, context);
        }
    }
}

fn debug_validate_runtime_template_node_type_ids(
    node: &OwnedRuntimeTemplateNode,
    context: &DebugTypeValidationContext,
) {
    match node {
        OwnedRuntimeTemplateNode::Sequence { children, .. } => {
            for child in children {
                debug_validate_runtime_template_node_type_ids(child, context);
            }
        }

        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
            debug_validate_expression_type_id(expression, context);
        }

        OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => {
            debug_validate_runtime_template_handoff_type_ids(template, context);
        }

        OwnedRuntimeTemplateNode::ConditionalWrapper { child, wrapper, .. } => {
            debug_validate_runtime_template_node_type_ids(child, context);
            debug_validate_runtime_template_node_type_ids(wrapper, context);
        }

        OwnedRuntimeTemplateNode::BranchChain {
            branches, fallback, ..
        } => {
            for branch in branches {
                debug_validate_template_branch_selector_type_ids(&branch.selector, context);
                debug_validate_runtime_template_node_type_ids(&branch.body, context);
            }
            if let Some(fallback) = fallback {
                debug_validate_runtime_template_node_type_ids(fallback, context);
            }
        }

        OwnedRuntimeTemplateNode::Loop {
            header,
            body,
            aggregate_wrapper,
            ..
        } => {
            debug_validate_template_loop_header_type_ids(header, context);
            debug_validate_runtime_template_node_type_ids(body, context);
            if let Some(aggregate_wrapper) = aggregate_wrapper {
                debug_validate_runtime_template_node_type_ids(aggregate_wrapper, context);
            }
        }

        OwnedRuntimeTemplateNode::Text { .. }
        | OwnedRuntimeTemplateNode::AggregateOutput
        | OwnedRuntimeTemplateNode::LoopControl { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotSite { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { .. }
        | OwnedRuntimeTemplateNode::Slot { .. } => {}
    }
}

fn debug_validate_template_branch_selector_type_ids(
    selector: &TemplateBranchSelector,
    context: &DebugTypeValidationContext,
) {
    match selector {
        TemplateBranchSelector::Bool(condition) => {
            debug_validate_expression_type_id(condition, context);
        }

        TemplateBranchSelector::OptionPresentCapture { scrutinee, .. } => {
            debug_validate_expression_type_id(scrutinee, context);
        }
    }
}

fn debug_validate_template_loop_header_type_ids(
    header: &TemplateLoopHeader,
    context: &DebugTypeValidationContext,
) {
    match header {
        TemplateLoopHeader::Conditional { condition } => {
            debug_validate_expression_type_id(condition, context);
        }

        TemplateLoopHeader::Range { range, .. } => {
            debug_validate_expression_type_id(&range.start, context);
            debug_validate_expression_type_id(&range.end, context);
            if let Some(step) = &range.step {
                debug_validate_expression_type_id(step, context);
            }
        }

        TemplateLoopHeader::Collection { iterable, .. } => {
            debug_validate_expression_type_id(iterable, context);
        }
    }
}

fn debug_validate_runtime_slot_application_handoff_type_ids(
    handoff: &OwnedRuntimeSlotApplicationHandoff,
    context: &DebugTypeValidationContext,
) {
    debug_validate_runtime_template_node_type_ids(&handoff.wrapper, context);

    for source in &handoff.contribution_sources {
        debug_validate_runtime_template_node_type_ids(&source.render_root, context);
    }

    for site in &handoff.slot_sites {
        debug_validate_runtime_template_node_type_ids(&site.render_root, context);
    }
}

fn debug_validate_nodes_type_ids(nodes: &[AstNode], context: &DebugTypeValidationContext) {
    for node in nodes {
        debug_validate_node_type_ids(node, context);
    }
}

fn debug_validate_expressions_type_ids(
    expressions: &[Expression],
    context: &DebugTypeValidationContext,
) {
    for expression in expressions {
        debug_validate_expression_type_id(expression, context);
    }
}

fn debug_validate_declarations_type_ids(
    declarations: &[Declaration],
    context: &DebugTypeValidationContext,
) {
    for declaration in declarations {
        debug_validate_declaration_type_id(declaration, context);
    }
}

fn debug_validate_declaration_type_id(
    declaration: &Declaration,
    context: &DebugTypeValidationContext,
) {
    debug_validate_expression_type_id(&declaration.value, context);
}

fn debug_validate_call_arguments_type_ids(
    args: &[CallArgument],
    context: &DebugTypeValidationContext,
) {
    for argument in args {
        debug_validate_expression_type_id(&argument.value, context);
    }
}

fn debug_validate_type_ids(type_ids: &[TypeId], type_environment: &TypeEnvironment, owner: &str) {
    for type_id in type_ids {
        debug_validate_type_id(*type_id, type_environment, owner);
    }
}

fn debug_validate_type_id(type_id: TypeId, type_environment: &TypeEnvironment, owner: &str) {
    let definition = type_environment.get(type_id);

    debug_assert!(
        definition.is_some(),
        "AST {owner} carried orphan TypeId({}) not registered in the final TypeEnvironment",
        type_id.0,
    );

    // Generic parameters must be fully resolved to concrete types before HIR.
    // Carrying an unresolved generic parameter across the boundary indicates a
    // frontend type-resolution bug.
    debug_assert!(
        !matches!(definition, Some(TypeDefinition::GenericParameter(..))),
        "AST {owner} carried unresolved generic parameter TypeId({}) to the HIR boundary",
        type_id.0,
    );
}
