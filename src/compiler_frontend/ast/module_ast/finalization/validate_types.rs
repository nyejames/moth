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
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ArgumentListView, AstExpressionBuilder, CoercionOperation, ExprId, ExpressionOperationView,
    ExpressionView, HandlingRecord, PlaceId, PlaceProjectionView, PlaceView,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::failure_facts::FailureDisposition;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::statements::match_patterns::PatternValueKind;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::runtime_handoff;
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrStore, finalized_tir_view_for_template, walk_tir_view_expression_payloads,
};
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastFallibility;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::source::SourceSpan;

/// Context shared by every check in this type-boundary validation pass.
///
/// WHAT: bundles the final module `TypeEnvironment` with the module-scoped
///       `TemplateIrStore` so template-expression payload validation resolves one
///       required finalized `TirView`. `recovery_allowed` is the only position-specific field.
/// WHY: the pass is read-only and short-lived. Child steps copy the context, so a catch
///      block can allow recovery for its subject while its handler disallows it.
#[derive(Clone, Copy)]
pub(super) struct TypeValidationContext<'a> {
    pub(super) expression_builder: &'a AstExpressionBuilder,
    pub(super) const_values: &'a ConstValueStore,
    recovery_allowed: bool,
    pub(super) type_environment: &'a TypeEnvironment,
    pub(super) template_ir_store: &'a TemplateIrStore,
}

impl AstFinalizer<'_, '_> {
    pub(crate) fn validate_no_unresolved_executable_types(
        &self,
        ast: &[AstNode],
        const_values: &ConstValueStore,
    ) -> Result<(), CompilerError> {
        let template_ir_store = self.context.template_ir_store.borrow();
        let context = TypeValidationContext {
            expression_builder: &self.context.expression_builder,
            const_values,
            recovery_allowed: false,
            type_environment: &self.environment.type_environment,
            template_ir_store: &template_ir_store,
        };

        validate_roots(
            context,
            |children| {
                push_nodes(ast, &context, children);
                Ok(())
            },
            &mut count_expression_census,
        )?;

        const_values.validate_type_ids(context.type_environment)?;

        Ok(())
    }
}

// --------------------------
//  Iterative walk
// --------------------------

/// One pending check on the explicit validation stack.
struct Step<'a> {
    work: Work<'a>,
    context: TypeValidationContext<'a>,
}

/// The unit of work a `Step` performs.
enum Work<'a> {
    /// A statement borrowed from the authored AST.
    Node(&'a AstNode),
    /// A stored expression row. Validated once per recovery context.
    Expression(ExprId),
    /// A check that failed. Queued beneath the children its expansion produced first, so those
    /// children report their errors before this one, as the recursive order did.
    Failed(Box<CompilerError>),
}

/// Bits in the per-row visited flags. A row is validated once for each recovery context.
const VISITED_PLAIN: u8 = 1;
const VISITED_RECOVERING: u8 = 2;

/// Drives the type-boundary walk over every root with an explicit stack.
///
/// WHAT: each `Work` item is expanded in turn. An expansion runs its own checks in source order
///       and queues child steps. No Rust recursion grows with the AST or with the expression DAG.
/// WHY: `ExprId` rows form a shared DAG. Recursing into every child on every parent validated
///      shared rows once per path, so work grew with path count. The visited key keeps work
///      proportional to unique rows and edges. The recovery flag is part of the key because a
///      catch subject is validated with recovery allowed, while the same row reached elsewhere is
///      not.
///
/// Errors keep the recursive order. A failing check queues `Work::Failed` beneath the children its
/// expansion already queued, so earlier siblings and their subtrees still report first.
///
/// The dedupe cannot change the first error. A row's checks depend only on the row, its recovery
/// flag and the per-pass stores. A duplicate can only pop after its first occurrence has fully
/// expanded, because a duplicate inside that row's own subtree would make the row its own child.
fn validate_roots<'a>(
    context: TypeValidationContext<'a>,
    expand_roots: impl FnOnce(&mut Vec<Step<'a>>) -> Result<(), CompilerError>,
    count_expression: &mut impl FnMut(),
) -> Result<(), CompilerError> {
    let mut visited = vec![0u8; context.expression_builder.len()];
    let mut stack: Vec<Step<'a>> = Vec::new();
    let mut children: Vec<Step<'a>> = Vec::new();

    if let Err(error) = expand_roots(&mut children) {
        stack.push(Step {
            work: Work::Failed(Box::new(error)),
            context,
        });
    }
    stack.extend(children.drain(..).rev());

    while let Some(Step { work, context }) = stack.pop() {
        let expanded = match work {
            Work::Failed(error) => return Err(*error),
            Work::Node(node) => expand_node(node, &context, &mut children),
            Work::Expression(id) => {
                match enter_expression(id, &context, &mut visited, count_expression) {
                    Ok(Some(expression)) => expand_expression(expression, &context, &mut children),
                    Ok(None) => Ok(()),
                    Err(error) => Err(error),
                }
            }
        };

        if let Err(error) = expanded {
            stack.push(Step {
                work: Work::Failed(Box::new(error)),
                context,
            });
        }
        stack.extend(children.drain(..).rev());
    }

    Ok(())
}

/// Resolves an expression row and records the visit.
///
/// Returns `None` when this row was already validated under the same recovery context.
fn enter_expression<'a>(
    id: ExprId,
    context: &TypeValidationContext<'a>,
    visited: &mut [u8],
    count_expression: &mut impl FnMut(),
) -> Result<Option<ExpressionView<'a>>, CompilerError> {
    let builder: &'a AstExpressionBuilder = context.expression_builder;
    let expression = builder.view(id)?;
    let flag = if context.recovery_allowed {
        VISITED_RECOVERING
    } else {
        VISITED_PLAIN
    };
    let state = &mut visited[id.index()];
    if *state & flag != 0 {
        return Ok(None);
    }
    *state |= flag;
    count_expression();
    Ok(Some(expression))
}

/// Counts one unique expression expansion in the frontend census when benchmark counters are on.
///
/// WHAT: the census records unique (row, recovery context) expansions rather than raw reads.
/// WHY: a shared row is now validated once per context, so counting raw reads would report
///      duplicate walk work that the visited key removes.
fn count_expression_census() {
    #[cfg(feature = "benchmark_counters")]
    increment_frontend_counter(FrontendCounter::CensusAstValidationExpressionExpansions);
}

fn push_expression<'a>(id: ExprId, context: &TypeValidationContext<'a>, out: &mut Vec<Step<'a>>) {
    out.push(Step {
        work: Work::Expression(id),
        context: *context,
    });
}

fn push_expressions<'a>(
    ids: &[ExprId],
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) {
    for id in ids {
        push_expression(*id, context, out);
    }
}

fn push_nodes<'a>(
    nodes: &'a [AstNode],
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) {
    out.extend(nodes.iter().map(|node| Step {
        work: Work::Node(node),
        context: *context,
    }));
}

// --------------------------
//  Node expansion
// --------------------------

/// Runs the checks for one statement and queues the expressions and bodies it contains.
///
/// Children are queued in source order, and checks run as the walk reaches them.
fn expand_node<'a>(
    node: &'a AstNode,
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) -> Result<(), CompilerError> {
    match &node.kind {
        // Control flow with nested statement bodies.
        NodeKind::If(condition, then_body, else_body, _) => {
            push_expression(*condition, context, out);
            push_nodes(then_body, context, out);
            if let Some(else_body) = else_body {
                push_nodes(else_body, context, out);
            }
        }

        NodeKind::Match {
            scrutinee,
            arms,
            default,
            exhaustiveness: _,
        } => {
            push_expression(*scrutinee, context, out);

            for arm in arms {
                expand_match_pattern(&arm.pattern, context, out)?;
                if let Some(guard) = &arm.guard {
                    push_expression(*guard, context, out);
                }

                push_nodes(&arm.body, context, out);
            }

            if let Some(default_body) = default {
                push_nodes(default_body, context, out);
            }
        }

        NodeKind::LexicalScope { body } => push_nodes(body, context, out),

        NodeKind::RangeLoop {
            bindings,
            range,
            body,
        } => {
            expand_loop_bindings(bindings, context, out)?;
            push_expression(range.start, context, out);
            push_expression(range.end, context, out);
            if let Some(step) = &range.step {
                push_expression(*step, context, out);
            }
            push_nodes(body, context, out);
        }

        NodeKind::CollectionLoop {
            bindings,
            iterable,
            body,
        } => {
            expand_loop_bindings(bindings, context, out)?;
            push_expression(*iterable, context, out);
            push_nodes(body, context, out);
        }

        NodeKind::WhileLoop(condition, body) => {
            push_expression(*condition, context, out);
            push_nodes(body, context, out);
        }

        // Terminal expressions that carry a single value.
        NodeKind::Return(values) => push_expressions(values, context, out),

        NodeKind::ReturnError(value)
        | NodeKind::PushStartRuntimeFragment(value)
        | NodeKind::ExpressionStatement(value) => push_expression(*value, context, out),

        // Declarations and assignments.
        NodeKind::VariableDeclaration(declaration) => {
            expand_declaration(declaration, context, out)?
        }

        NodeKind::Assignment { target, value } => {
            expand_place_expression(*target, context, out)?;
            push_expression(*value, context, out);
        }

        NodeKind::MultiBind { targets, value } => {
            for target in targets {
                validate_multi_bind_target(target, context)?;
            }
            push_expression(*value, context, out);
        }

        // Type and function definitions.
        NodeKind::StructDefinition(_, fields) => expand_declarations(fields, context, out)?,

        NodeKind::Function(_, signature, body) => {
            validate_signature(signature, node.span, context)?;
            push_nodes(body, context, out);
        }

        NodeKind::Assert { condition, message } => {
            push_expression(*condition, context, out);
            push_expression(*message, context, out);
        }

        // Terminal nodes that contain no type-carrying positions.
        NodeKind::Break | NodeKind::Continue => {}

        // Value-producing terminator inside an active value block.
        NodeKind::ThenValue(produced_values) => {
            push_expressions(&produced_values.expressions, context, out);
        }
    }

    Ok(())
}

// --------------------------
//  Expression expansion
// --------------------------

/// Runs the checks for one expression row and queues its children.
///
/// WHAT: validates the row's own TypeIds and handling records, then queues child rows and
///       nested bodies in source order.
/// WHY: `MethodCall` checks its receiver before its own type and handling. Queuing the receiver
///      before the call's checks keeps that error order.
fn expand_expression<'a>(
    expression: ExpressionView<'a>,
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) -> Result<(), CompilerError> {
    let span = expression.span();
    validate_type_ids(expression.success_types(), span, context)?;
    if let Some(type_id) = expression.error_type() {
        validate_type_id(type_id, span, context)?;
    }

    use ExpressionOperationView as Operation;
    match expression.operation() {
        Operation::Constant(id) => {
            context.const_values.value(id).ok_or_else(|| {
                CompilerError::new("Constant ID outside owner", span, ErrorType::Compiler)
            })?;
        }
        Operation::Reference(_) => {}
        Operation::Copy(place) => expand_place(place, context, out)?,
        Operation::FieldAccess { base, .. } => push_expression(base.id(), context, out),
        Operation::NumericUnary { operand, facts } => {
            validate_type_ids(&[facts.result_type, facts.left_type], facts.span, context)?;
            push_expression(operand.id(), context, out);
        }
        Operation::NumericBinary { left, right, facts }
        | Operation::Compare { left, right, facts }
        | Operation::Range {
            start: left,
            end: right,
            facts,
        } => {
            validate_type_ids(&[facts.result_type, facts.left_type], facts.span, context)?;
            if let Some(type_id) = facts.right_type {
                validate_type_id(type_id, facts.span, context)?;
            }
            push_expression(left.id(), context, out);
            push_expression(right.id(), context, out);
        }
        Operation::LogicalNot { operand } => push_expression(operand.id(), context, out),
        Operation::LogicalBinary { left, right, .. } => {
            push_expression(left.id(), context, out);
            push_expression(right.id(), context, out);
        }
        Operation::Call(call) => {
            if let Some(type_id) = call.metadata().error_type {
                validate_type_id(type_id, span, context)?;
            }
            if let Some(handling) = call.handling() {
                validate_handling(handling, span, context)?;
            }
            push_call_arguments(call.arguments(), context, out);
        }
        Operation::MethodCall { receiver, call } => {
            push_expression(receiver.id(), context, out);
            if let Some(type_id) = call.metadata().error_type {
                validate_type_id(type_id, span, context)?;
            }
            if let Some(handling) = call.handling() {
                validate_handling(handling, span, context)?;
            }
            push_call_arguments(call.arguments(), context, out);
        }
        Operation::CollectionBuiltin {
            receiver,
            arguments,
            ..
        }
        | Operation::MapBuiltin {
            receiver,
            arguments,
            ..
        } => {
            push_expression(receiver.id(), context, out);
            push_call_arguments(arguments, context, out);
        }
        Operation::Cast { source, facts } => {
            validate_type_ids(
                &[facts.source_type_id, facts.target_type_id],
                facts.span,
                context,
            )?;
            if facts.handling == CastHandling::Recover && !context.recovery_allowed {
                return Err(CompilerError::new(
                    "Recovering cast reached finalization outside ValueBlock::Catch",
                    span,
                    ErrorType::Compiler,
                ));
            }
            push_expression(source.id(), context, out);
        }
        Operation::HandleFallible { value, handling } => {
            validate_handling(handling, span, context)?;
            push_expression(value.id(), context, out);
        }
        Operation::PropagateOption { value } => push_expression(value.id(), context, out),
        Operation::Coerce {
            value,
            operation: CoercionOperation::ToType(type_id),
        } => {
            validate_type_id(type_id, span, context)?;
            push_expression(value.id(), context, out);
        }
        Operation::Collection(items) => {
            for item in items.iter() {
                push_expression(item.id(), context, out);
            }
        }
        Operation::Map(entries) => {
            for entry in entries.iter() {
                push_expression(entry.key().id(), context, out);
                push_expression(entry.value().id(), context, out);
            }
        }
        Operation::Struct { fields } | Operation::Choice { fields, .. } => {
            for field in fields.iter() {
                push_expression(field.value().id(), context, out);
            }
        }
        Operation::Template(template) => expand_template(template, context, out)?,
        Operation::RuntimeTemplate(handoff) => {
            expand_owned_runtime_template_handoff(handoff, context, out)?
        }
        Operation::RuntimeSlotApplication(handoff) => {
            expand_owned_runtime_slot_application_handoff(handoff, context, out)?
        }
        Operation::ValueBlock(block) => expand_value_block(block, span, context, out)?,
    }

    Ok(())
}

fn expand_value_block<'a>(
    block: &'a ValueBlock,
    span: Option<SourceSpan>,
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) -> Result<(), CompilerError> {
    match block {
        ValueBlock::If(value) => {
            push_expression(value.condition, context, out);
            push_nodes(&value.then_body, context, out);
            push_nodes(&value.else_body, context, out);
        }
        ValueBlock::LexicalScope(value) => {
            validate_type_ids(&value.result_type_ids, span, context)?;
            push_nodes(&value.body, context, out);
        }
        ValueBlock::Match(value) => {
            push_expression(value.scrutinee, context, out);
            for arm in &value.arms {
                expand_match_pattern(&arm.pattern, context, out)?;
                if let Some(guard) = &arm.guard {
                    push_expression(*guard, context, out);
                }
                push_nodes(&arm.body, context, out);
            }
            if let Some(body) = &value.default {
                push_nodes(body, context, out);
            }
        }
        ValueBlock::Catch(value) => {
            let subject = context.expression_builder.view(value.handled_value)?;
            if !matches!(value.handler, FallibleHandling::Handler { .. }) {
                return Err(CompilerError::new(
                    "ValueBlock::Catch carried non-handler handling",
                    span,
                    ErrorType::Compiler,
                ));
            }
            validate_catch_subject(subject, context)?;

            // The subject is the only position where a recovering expression is legal.
            let protected = TypeValidationContext {
                recovery_allowed: true,
                ..*context
            };
            push_expression(value.handled_value, &protected, out);

            let handler = TypeValidationContext {
                recovery_allowed: false,
                ..*context
            };
            if let FallibleHandling::Handler { body, .. } = &value.handler {
                push_nodes(body, &handler, out);
            }
        }
    }

    Ok(())
}

fn expand_place_expression<'a>(
    place: PlaceId,
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) -> Result<(), CompilerError> {
    expand_place(context.expression_builder.place(place)?, context, out)
}

fn expand_place<'a>(
    place: PlaceView<'_>,
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) -> Result<(), CompilerError> {
    let metadata = place.metadata();
    validate_type_id(metadata.root_type_id, metadata.span, context)?;
    validate_type_id(metadata.type_id, metadata.span, context)?;
    for projection in place.projections() {
        match projection {
            PlaceProjectionView::Field { result_type, .. } => {
                validate_type_id(result_type, metadata.span, context)?
            }
            PlaceProjectionView::Index { index, result_type } => {
                validate_type_id(result_type, metadata.span, context)?;
                push_expression(index.id(), context, out);
            }
        }
    }
    Ok(())
}

fn push_call_arguments<'a>(
    arguments: ArgumentListView<'_>,
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) {
    for argument in arguments.iter() {
        push_expression(argument.value().id(), context, out);
    }
}

fn validate_handling(
    handling: &HandlingRecord,
    span: Option<SourceSpan>,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    if let Some(type_id) = handling.error_type {
        validate_type_id(type_id, span, context)?;
    }
    if handling.handling == FallibleExpressionHandling::Recover && !context.recovery_allowed {
        return Err(CompilerError::new(
            "Recovering expression reached finalization outside ValueBlock::Catch",
            span,
            ErrorType::Compiler,
        ));
    }
    Ok(())
}

// --------------------------
//  Template and runtime payloads
// --------------------------

/// Queues every expression payload of a template's finalized `TirView`.
///
/// WHAT: resolves the required finalized module-store `TirView` for the template and queues its
///       effective expression payloads. Effective expression overlays are authoritative for
///       dynamic-expression splices, branch selectors and loop headers. A template that reaches
///       this boundary without a Finalized module-store identity is an internal compiler
///       invariant violation.
/// WHY: type-boundary validation must validate the same effective TIR representation that later
///      phases consume. There is no raw same-store downgrade after normalization. The TIR
///      walker owns its own recursion, so this callback only records payload rows.
fn expand_template<'a>(
    template: &Template,
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) -> Result<(), CompilerError> {
    let view = finalized_tir_view_for_template(template, context.template_ir_store)?;

    walk_tir_view_expression_payloads(&view, context.expression_builder, &mut |expression| {
        push_expression(expression.id(), context, out);
        Ok(())
    })
}

fn expand_owned_runtime_template_handoff<'a>(
    handoff: &OwnedRuntimeTemplateHandoff,
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) -> Result<(), CompilerError> {
    runtime_handoff::walk_owned_runtime_template_handoff(handoff, &mut |node| {
        expand_owned_runtime_template_node(node, context, out)
    })
}

fn expand_owned_runtime_slot_application_handoff<'a>(
    handoff: &OwnedRuntimeSlotApplicationHandoff,
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) -> Result<(), CompilerError> {
    runtime_handoff::walk_owned_runtime_slot_application_handoff(handoff, &mut |node| {
        expand_owned_runtime_template_node(node, context, out)
    })
}

/// Queues the expression-bearing payloads of one owned runtime handoff node.
///
/// WHAT: queues dynamic expressions, branch selectors and loop headers while the canonical
///       runtime-handoff walker owns recursion through child/body/slot structures.
/// WHY: these control-flow payloads are not child nodes, so a dynamic-expression-only callback
///      would let orphan TypeIds cross the AST/HIR boundary unnoticed.
fn expand_owned_runtime_template_node<'a>(
    node: &OwnedRuntimeTemplateNode,
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) -> Result<(), CompilerError> {
    match node {
        OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
            push_expression(*expression, context, out);
            Ok(())
        }

        OwnedRuntimeTemplateNode::Conditional { selector, .. } => {
            expand_template_branch_selector(selector, context, out)
        }

        OwnedRuntimeTemplateNode::Loop { header, .. } => {
            expand_template_loop_header(header, context, out)
        }

        OwnedRuntimeTemplateNode::Sequence { .. }
        | OwnedRuntimeTemplateNode::Text { .. }
        | OwnedRuntimeTemplateNode::ChildTemplate { .. }
        | OwnedRuntimeTemplateNode::ConditionalWrapper { .. }
        | OwnedRuntimeTemplateNode::AggregateOutput
        | OwnedRuntimeTemplateNode::RuntimeSlotSite { .. }
        | OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { .. }
        | OwnedRuntimeTemplateNode::Slot { .. } => Ok(()),
    }
}

fn expand_template_branch_selector<'a>(
    selector: &TemplateBranchSelector,
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) -> Result<(), CompilerError> {
    match selector {
        TemplateBranchSelector::Bool(condition) => {
            push_expression(*condition, context, out);
            Ok(())
        }
        TemplateBranchSelector::OptionPresentCapture { scrutinee, pattern } => {
            push_expression(*scrutinee, context, out);
            expand_match_pattern(pattern, context, out)
        }
    }
}

fn expand_template_loop_header<'a>(
    header: &TemplateLoopHeader,
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) -> Result<(), CompilerError> {
    match header {
        TemplateLoopHeader::Conditional { condition } => {
            push_expression(*condition, context, out);
        }
        TemplateLoopHeader::Range { bindings, range } => {
            expand_loop_bindings(bindings, context, out)?;
            push_expression(range.start, context, out);
            push_expression(range.end, context, out);
            if let Some(step) = &range.step {
                push_expression(*step, context, out);
            }
        }
        TemplateLoopHeader::Collection { bindings, iterable } => {
            expand_loop_bindings(bindings, context, out)?;
            push_expression(*iterable, context, out);
        }
    }
    Ok(())
}

fn expand_match_pattern<'a>(
    pattern: &MatchPattern,
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) -> Result<(), CompilerError> {
    match pattern {
        MatchPattern::Literal(value)
        | MatchPattern::OptionValue { value, .. }
        | MatchPattern::Relational { value, .. } => {
            validate_type_id(value.type_id, value.span, context)?;
            match value.kind {
                PatternValueKind::Runtime(id) => push_expression(id, context, out),
                PatternValueKind::Constant(id) => {
                    context.const_values.value(id).map(|_| ()).ok_or_else(|| {
                        CompilerError::new(
                            "Pattern constant ID outside owner",
                            value.span,
                            ErrorType::Compiler,
                        )
                    })?;
                }
            }
        }
        MatchPattern::ChoiceVariant { captures, .. } => {
            for capture in captures {
                validate_type_id(capture.type_id, capture.span, context)?;
            }
        }
        MatchPattern::OptionNone { .. } => {}
        MatchPattern::OptionPresentCapture {
            inner_type_id,
            binding_span,
            ..
        } => validate_type_id(*inner_type_id, *binding_span, context)?,
    }
    Ok(())
}

// --------------------------
//  Declarations and signatures
// --------------------------

fn expand_loop_bindings<'a>(
    bindings: &LoopBindings,
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) -> Result<(), CompilerError> {
    if let Some(item) = &bindings.item {
        expand_declaration(item, context, out)?;
    }

    if let Some(index) = &bindings.index {
        expand_declaration(index, context, out)?;
    }

    Ok(())
}

fn expand_declarations<'a>(
    declarations: &[Declaration],
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) -> Result<(), CompilerError> {
    for declaration in declarations {
        expand_declaration(declaration, context, out)?;
    }
    Ok(())
}

fn expand_declaration<'a>(
    declaration: &Declaration,
    context: &TypeValidationContext<'a>,
    out: &mut Vec<Step<'a>>,
) -> Result<(), CompilerError> {
    validate_type_id(declaration.type_id, declaration.binding_span, context)?;
    if let Some(initializer) = declaration.value.initializer() {
        push_expression(initializer, context, out);
    }
    Ok(())
}

fn validate_multi_bind_target(
    target: &MultiBindTarget,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    validate_type_id(target.type_id, target.span, context)
}

fn validate_signature(
    signature: &FunctionSignature,
    span: Option<SourceSpan>,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    for parameter in &signature.parameters {
        validate_type_id(parameter.type_id, parameter.binding_span, context)?;
        if let Some(id) = parameter.default {
            context.const_values.value(id).ok_or_else(|| {
                CompilerError::new(
                    "Default constant ID outside owner",
                    parameter.binding_span,
                    ErrorType::Compiler,
                )
            })?;
        }
    }
    validate_type_ids(&signature.success_return_type_ids(), span, context)?;
    if let Some(error_return_type_id) = signature.error_return_type_id() {
        validate_type_id(error_return_type_id, span, context)?;
    }
    Ok(())
}

// --------------------------
//  TypeId checks
// --------------------------

fn validate_type_ids(
    type_ids: &[TypeId],
    span: Option<SourceSpan>,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    for type_id in type_ids {
        validate_type_id(*type_id, span, context)?;
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
    span: Option<SourceSpan>,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    if context
        .type_environment
        .get(type_id)
        .is_some_and(|definition| !matches!(definition, TypeDefinition::GenericParameter(..)))
    {
        return Ok(());
    }

    Err(CompilerError::new(
        format!(
            "Resolved TypeId({}) reached executable AST without a matching TypeEnvironment entry.",
            type_id.0
        ),
        span,
        ErrorType::Compiler,
    ))
}

#[cfg(test)]
#[path = "tests/validate_types_tests.rs"]
mod validate_types_tests;

/// Catch eligibility belongs to AST failure facts, not to the subset of shapes HIR recovery
/// can deliver. A handled call, handled expression or cast marked `Recover` is also a valid
/// subject because its own handling record carries the recovery.
fn validate_catch_subject(
    expression: ExpressionView<'_>,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    if catch_subject_recovers(expression, context)? {
        return Ok(());
    }
    Err(CompilerError::new(
        "ValueBlock::Catch subject lacks compatible completed recovery facts",
        expression.span(),
        ErrorType::Compiler,
    ))
}

fn catch_subject_recovers(
    expression: ExpressionView<'_>,
    context: &TypeValidationContext,
) -> Result<bool, CompilerError> {
    let operation = expression.operation();
    if let Some(facts) = expression.failure_facts()
        && let FailureDisposition::HandledByCatch { error_type_id } = facts.disposition
    {
        validate_type_id(error_type_id, expression.span(), context)?;
        let builtin_error = context.type_environment.type_id_for_canonical_identity(
            &crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity::Builtin(
                crate::compiler_frontend::canonical_type_identity::CanonicalBuiltinType::Error,
            ),
        );
        let implicit_compatible = facts.summary.first_implicit.is_none()
            || Some(error_type_id) == builtin_error
            || (facts.summary.first_numeric.is_none()
                && facts
                    .deferred_custom_catch
                    .is_some_and(|check| check.error_type_id == error_type_id));
        let typed_compatible = facts.summary.conflicting_typed.is_none()
            && facts
                .summary
                .first_typed
                .is_none_or(|producer| producer.error_type_id == error_type_id);
        let eligible = facts.checked_numeric_operation
            || facts.authored_cast
            || facts.safety_call_count >= 2
            || facts.summary.first_implicit.is_some()
            || facts.summary.first_typed.is_some();
        if !implicit_compatible
            || !typed_compatible
            || !eligible
            || facts.postfix_exit_span.is_some()
        {
            return Ok(false);
        }
        // An infallible conversion can still recover checked work in its operand.
        match &operation {
            ExpressionOperationView::Cast { facts, .. }
                if facts.fallibility == BuiltinCastFallibility::Infallible =>
            {
                return Ok(true);
            }
            ExpressionOperationView::Call(call) if call.handling().is_some() => {}
            ExpressionOperationView::HandleFallible { .. }
            | ExpressionOperationView::Cast { .. } => {}
            _ => return Ok(true),
        }
    }

    Ok(match operation {
        ExpressionOperationView::Call(call) => call.handling().is_some_and(|handling| {
            matches!(handling.handling, FallibleExpressionHandling::Recover)
        }),
        ExpressionOperationView::HandleFallible { handling, .. } => {
            matches!(handling.handling, FallibleExpressionHandling::Recover)
        }
        ExpressionOperationView::Cast { facts, .. } => {
            matches!(facts.handling, CastHandling::Recover)
        }
        _ => false,
    })
}

#[cfg(debug_assertions)]
pub(super) fn validate_debug_roots(
    nodes: &[AstNode],
    constants: &ConstValueStore,
    types: &TypeEnvironment,
    tir: &TemplateIrStore,
    builder: &AstExpressionBuilder,
) -> Result<(), CompilerError> {
    let context = TypeValidationContext {
        type_environment: types,
        template_ir_store: tir,
        expression_builder: builder,
        const_values: constants,
        recovery_allowed: false,
    };
    validate_roots(
        context,
        |children| {
            push_nodes(nodes, &context, children);
            Ok(())
        },
        &mut || {},
    )?;
    constants.validate_type_ids(types)
}
