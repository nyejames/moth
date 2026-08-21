//! Final AST type-boundary validation before HIR lowering.
//!
//! WHAT: validates that HIR-bound AST values carry TypeIds registered in the module
//! `TypeEnvironment`.
//! WHY: AST owns name/type resolution; HIR should receive canonical semantic type identity,
//! not diagnostic-only `DataType` reconstructions.
//!
//! Template expression payloads are validated through one required finalized
//! `TirView` authority; there is no raw same-store fallback after normalization.

use super::finalizer::AstFinalizer;
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, Declaration, LoopBindings, MultiBindTarget, NodeKind,
};
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, PlaceExpression, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::runtime_handoff;
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrStore, finalized_tir_view_for_template, walk_tir_view_expression_payloads,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

/// Context shared by every helper in this type-boundary validation pass.
///
/// WHAT: bundles the final module `TypeEnvironment` with the module-scoped
///       `TemplateIrStore` and `TemplateIrStore` so template-expression
///       payload validation resolves one required finalized `TirView`.
/// WHY: the pass is read-only and short-lived; a small context struct keeps the
///      recursive walk signatures focused and stage-local.
struct TypeValidationContext<'a> {
    type_environment: &'a TypeEnvironment,
    template_ir_store: &'a TemplateIrStore,
}

impl AstFinalizer<'_, '_> {
    pub(crate) fn validate_no_unresolved_executable_types(
        &self,
        ast: &[AstNode],
        module_constants: &[Declaration],
        _string_table: &StringTable,
    ) -> Result<(), CompilerError> {
        let template_ir_store = self.context.template_ir_store.borrow();
        let context = TypeValidationContext {
            type_environment: &self.environment.type_environment,
            template_ir_store: &template_ir_store,
        };

        for node in ast {
            validate_node(node, &context)?;
        }

        for constant in module_constants {
            validate_declaration(constant, &context)?;
        }

        Ok(())
    }
}

// --------------------------
//  Node validation
// --------------------------

/// Recursively validates all type-carrying positions inside an AST node.
///
/// WHAT: Walks every recursive sub-position in the node (expressions, nested
/// statement bodies, pattern captures, call arguments) and asserts that each
/// `TypeId` exists in the module `TypeEnvironment`.
///
/// WHY: AST owns semantic type resolution; HIR must receive only canonical
/// `TypeId`s. Any missing entry indicates a compiler bug.
fn validate_node(node: &AstNode, context: &TypeValidationContext) -> Result<(), CompilerError> {
    match &node.kind {
        // Control flow with nested statement bodies.
        NodeKind::If(condition, then_body, else_body) => {
            validate_expression(condition, context)?;
            validate_nodes(then_body, context)?;
            if let Some(else_body) = else_body {
                validate_nodes(else_body, context)?;
            }
            Ok(())
        }

        NodeKind::Match {
            scrutinee,
            arms,
            default,
            exhaustiveness: _,
        } => {
            validate_expression(scrutinee, context)?;

            for arm in arms {
                match &arm.pattern {
                    MatchPattern::Literal(value)
                    | MatchPattern::OptionValue { value, .. }
                    | MatchPattern::Relational { value, .. } => {
                        validate_expression(value, context)?;
                    }
                    MatchPattern::ChoiceVariant { captures, .. } => {
                        for capture in captures {
                            validate_type_id(capture.type_id, &capture.location, context)?;
                        }
                    }
                    MatchPattern::OptionNone { .. } | MatchPattern::OptionPresentCapture { .. } => {
                    }
                }

                if let Some(guard) = &arm.guard {
                    validate_expression(guard, context)?;
                }

                validate_nodes(&arm.body, context)?;
            }

            if let Some(default_body) = default {
                validate_nodes(default_body, context)?;
            }

            Ok(())
        }

        NodeKind::ScopedBlock { body } => validate_nodes(body, context),

        NodeKind::RangeLoop {
            bindings,
            range,
            body,
        } => {
            validate_loop_bindings(bindings, context)?;
            validate_expression(&range.start, context)?;
            validate_expression(&range.end, context)?;
            if let Some(step) = &range.step {
                validate_expression(step, context)?;
            }
            validate_nodes(body, context)
        }

        NodeKind::CollectionLoop {
            bindings,
            iterable,
            body,
        } => {
            validate_loop_bindings(bindings, context)?;
            validate_expression(iterable, context)?;
            validate_nodes(body, context)
        }

        NodeKind::WhileLoop(condition, body) => {
            validate_expression(condition, context)?;
            validate_nodes(body, context)
        }

        // Terminal expressions that carry a single value.
        NodeKind::Return(values) => validate_expressions(values, context),

        NodeKind::ReturnError(value)
        | NodeKind::PushStartRuntimeFragment(value)
        | NodeKind::ExpressionStatement(value) => validate_expression(value, context),

        // Declarations and assignments.
        NodeKind::VariableDeclaration(declaration) => validate_declaration(declaration, context),

        NodeKind::Assignment { target, value } => {
            validate_place_expression(target, context)?;
            validate_expression(value, context)
        }

        NodeKind::MultiBind { targets, value } => {
            for target in targets {
                validate_multi_bind_target(target, context)?;
            }
            validate_expression(value, context)
        }

        // Type and function definitions.
        NodeKind::StructDefinition(_, fields) => validate_declarations(fields, context),

        NodeKind::Function(_, signature, body) => {
            validate_signature(signature, &node.location, context)?;
            validate_nodes(body, context)
        }

        NodeKind::Assert { condition, message } => {
            validate_expression(condition, context)?;
            validate_expression(message, context)
        }

        // Terminal nodes that contain no type-carrying positions.
        NodeKind::Break | NodeKind::Continue => Ok(()),

        // Value-producing terminator inside an active value block.
        NodeKind::ThenValue(produced_values) => {
            validate_expressions(&produced_values.expressions, context)
        }
    }
}

// --------------------------
//  Expression validation
// --------------------------

/// Recursively validates all type-carrying positions inside an expression.
///
/// WHAT: Validates the expression's own `type_id`, then recursively checks
/// nested expressions, call arguments, templates, and sub-nodes.
///
/// WHY: Expressions are the leaves and branches of the AST value tree;
/// unresolved types here would propagate into HIR as invalid semantic identity.
fn validate_place_expression(
    place: &PlaceExpression,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    validate_type_id(place.type_id, &place.location, context)?;
    match &place.kind {
        PlaceExpressionKind::Local(_) => Ok(()),
        PlaceExpressionKind::Field { base, .. } => validate_place_expression(base, context),
    }
}

fn validate_expression(
    expression: &Expression,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    validate_type_id(expression.type_id, &expression.location, context)?;

    match &expression.kind {
        // Recursive expression containers.
        ExpressionKind::Runtime(rpn) => {
            for item in &rpn.items {
                match item {
                    ExpressionRpnItem::Operand(expression) => {
                        validate_expression(expression, context)?;
                    }
                    ExpressionRpnItem::Operator { .. } => {}
                }
            }
            Ok(())
        }

        ExpressionKind::Copy(place) => validate_place_expression(place, context),

        ExpressionKind::FieldAccess { base, .. } => validate_expression(base, context),

        ExpressionKind::MethodCall { receiver, args, .. }
        | ExpressionKind::CollectionBuiltinCall { receiver, args, .. }
        | ExpressionKind::MapBuiltinCall { receiver, args, .. } => {
            validate_expression(receiver, context)?;
            validate_call_arguments(args, context)
        }

        // Function expressions carry signature metadata only; bodies are statement-level nodes.
        ExpressionKind::Function(signature) => {
            validate_signature(signature, &expression.location, context)
        }

        // Calls.
        ExpressionKind::FunctionCall { args, .. }
        | ExpressionKind::HostFunctionCall { args, .. } => validate_call_arguments(args, context),

        ExpressionKind::HandledFallibleFunctionCall { args, .. } => {
            validate_call_arguments(args, context)
        }

        ExpressionKind::HandledFallibleHostFunctionCall {
            args,
            error_type_id,
            ..
        } => {
            validate_call_arguments(args, context)?;
            validate_type_ids(&[*error_type_id], &expression.location, context)
        }

        // Wrapped and coerced values.
        #[cfg(test)]
        ExpressionKind::FallibleCarrierConstruct { value, .. } => {
            validate_expression(value, context)
        }

        ExpressionKind::OptionPropagation { value } | ExpressionKind::Coerced { value, .. } => {
            validate_expression(value, context)
        }

        ExpressionKind::Cast(cast) => {
            validate_expression(&cast.source, context)?;
            validate_type_id(cast.target_type_id, &cast.location, context)?;
            validate_type_id(cast.source_type_id, &cast.source.location, context)
        }

        ExpressionKind::HandledFallibleExpression { value, .. } => {
            validate_expression(value, context)
        }

        // Template and collection literals.
        ExpressionKind::Template(template) => {
            // A finalized module-store TIR view is required after normalization.
            validate_template_expression_payloads(template, context)?;
            Ok(())
        }

        ExpressionKind::RuntimeTemplateHandoff(handoff) => {
            validate_owned_runtime_template_handoff(handoff, context)
        }

        ExpressionKind::RuntimeSlotApplicationHandoff(handoff) => {
            validate_owned_runtime_slot_application_handoff(handoff, context)
        }

        ExpressionKind::Collection(items) => validate_expressions(items, context),

        // Struct and choice constructors.
        ExpressionKind::StructDefinition(fields)
        | ExpressionKind::StructInstance(fields)
        | ExpressionKind::ChoiceConstruct { fields, .. } => validate_declarations(fields, context),

        // Range expressions.
        ExpressionKind::Range(start, end) => {
            validate_expression(start, context)?;
            validate_expression(end, context)
        }

        ExpressionKind::ValueBlock { block } => match block.as_ref() {
            ValueBlock::If(value_if) => {
                validate_expression(&value_if.condition, context)?;
                validate_nodes(&value_if.then_body, context)?;
                validate_nodes(&value_if.else_body, context)
            }
            ValueBlock::Match(value_match) => {
                validate_expression(&value_match.scrutinee, context)?;
                for arm in &value_match.arms {
                    if let Some(guard) = &arm.guard {
                        validate_expression(guard, context)?;
                    }
                    validate_nodes(&arm.body, context)?;
                }
                if let Some(default_body) = &value_match.default {
                    validate_nodes(default_body, context)?;
                }
                Ok(())
            }
            ValueBlock::Catch(value_catch) => {
                validate_expression(&value_catch.handled_value, context)?;
                validate_fallible_handling(&value_catch.handler, context)
            }
        },

        ExpressionKind::MapLiteral(entries) => {
            for entry in entries {
                validate_expression(&entry.key, context)?;
                validate_expression(&entry.value, context)?;
            }
            Ok(())
        }

        // Terminal literals and references — types were resolved at construction.
        ExpressionKind::NoValue
        | ExpressionKind::OptionNone
        | ExpressionKind::Int(_)
        | ExpressionKind::Float(_)
        | ExpressionKind::StringSlice(_)
        | ExpressionKind::Bool(_)
        | ExpressionKind::Char(_)
        | ExpressionKind::Reference(_) => Ok(()),

        #[cfg(test)]
        ExpressionKind::Path(_) => Ok(()),
    }
}

fn validate_owned_runtime_template_handoff(
    handoff: &OwnedRuntimeTemplateHandoff,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    runtime_handoff::walk_owned_runtime_template_handoff(handoff, &mut |node| {
        if let OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } = node {
            validate_expression(expression, context)?;
        }

        Ok(())
    })
}

fn validate_owned_runtime_slot_application_handoff(
    handoff: &OwnedRuntimeSlotApplicationHandoff,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    runtime_handoff::walk_owned_runtime_slot_application_handoff(handoff, &mut |node| {
        if let OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } = node {
            validate_expression(expression, context)?;
        }

        Ok(())
    })
}

// --------------------------
//  Template expression payload validation
// --------------------------

/// Validates a template's nested expression payloads through one finalized `TirView`.
///
/// WHAT: resolves the required finalized module-store `TirView` for the
///       template and walks its effective expression payloads. Effective
///       expression overlays are authoritative for dynamic-expression splices,
///       branch selectors and loop headers. A template that reaches this
///       boundary without a Finalized module-store identity is an internal
///       compiler invariant violation.
/// WHY: type-boundary validation must validate the same effective TIR
///      representation that later phases consume; there is no raw same-store
///      downgrade after normalization.
fn validate_template_expression_payloads(
    template: &Template,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    let view = finalized_tir_view_for_template(template, context.template_ir_store)?;

    walk_tir_view_expression_payloads(&view, &mut |expression| {
        validate_expression(expression, context)
    })
}

// --------------------------
//  Helpers
// --------------------------

fn validate_fallible_handling(
    handling: &FallibleHandling,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    match handling {
        FallibleHandling::Propagate => Ok(()),
        FallibleHandling::Handler { body, .. } => validate_nodes(body, context),
    }
}

fn validate_loop_bindings(
    bindings: &LoopBindings,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    if let Some(item) = &bindings.item {
        validate_declaration(item, context)?;
    }

    if let Some(index) = &bindings.index {
        validate_declaration(index, context)?;
    }

    Ok(())
}

fn validate_call_arguments(
    arguments: &[CallArgument],
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    for argument in arguments {
        validate_expression(&argument.value, context)?;
    }
    Ok(())
}

fn validate_signature(
    signature: &FunctionSignature,
    location: &SourceLocation,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    validate_declarations(&signature.parameters, context)?;
    validate_type_ids(&signature.success_return_type_ids(), location, context)?;
    if let Some(error_return_type_id) = signature.error_return_type_id() {
        validate_type_id(error_return_type_id, location, context)?;
    }
    Ok(())
}

fn validate_declarations(
    declarations: &[Declaration],
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    for declaration in declarations {
        validate_declaration(declaration, context)?;
    }
    Ok(())
}

fn validate_declaration(
    declaration: &Declaration,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    validate_expression(&declaration.value, context)
}

fn validate_multi_bind_target(
    target: &MultiBindTarget,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    validate_type_id(target.type_id, &target.location, context)
}

fn validate_nodes(nodes: &[AstNode], context: &TypeValidationContext) -> Result<(), CompilerError> {
    for node in nodes {
        validate_node(node, context)?;
    }
    Ok(())
}

fn validate_expressions(
    expressions: &[Expression],
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    for expression in expressions {
        validate_expression(expression, context)?;
    }
    Ok(())
}

fn validate_type_ids(
    type_ids: &[TypeId],
    location: &SourceLocation,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    for type_id in type_ids {
        validate_type_id(*type_id, location, context)?;
    }
    Ok(())
}

/// Asserts that a single `TypeId` is registered in the `TypeEnvironment`.
///
/// WHY: A missing `TypeId` at this stage means AST type resolution failed to
/// record a canonical type for a value position. This is an internal compiler
/// invariant, not a user-facing diagnostic.
fn validate_type_id(
    type_id: TypeId,
    location: &SourceLocation,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    if context.type_environment.get(type_id).is_some() {
        return Ok(());
    }

    Err(CompilerError::new(
        format!(
            "Resolved TypeId({}) reached executable AST without a matching TypeEnvironment entry.",
            type_id.0
        ),
        location.to_owned(),
        ErrorType::Compiler,
    ))
}

#[cfg(test)]
#[path = "tests/validate_types_tests.rs"]
mod validate_types_tests;
