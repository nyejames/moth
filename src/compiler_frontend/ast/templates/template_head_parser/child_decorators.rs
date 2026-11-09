//! `$before ..` and `$after ..` directive parsing and wrapper normalization.
//!
//! WHAT:
//! - Parses the single bare operand, validates compile-time values and rejects
//!   default or positional slot occurrences inside that value.
//! - Normalizes each operand into a durable wrapper reference and
//!   composes both edges into one direct-child wrapper per owning template.
//!
//! WHY:
//! - The two decorators share one wrapper shape: prefix content, the implicit
//!   direct-child slot, then suffix content. Keeping their syntax and
//!   validation here leaves generic style-handler parsing and the TIR wrapper
//!   machinery unchanged.

use super::head_parser::parse_head_expression;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::ast::field_access::{
    constant_reference_expression_from_declaration, materialize_constant_template_expression,
    project_explicit_const_record_field,
};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{SlotKey, Style, Template, TemplateType};
use crate::compiler_frontend::ast::templates::template_build_state::TemplateBuildState;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateConstructionContext, TemplateIrId, TemplateIrNodeId, TemplateIrNodeKind,
    TemplateIrStore, TemplateTirPhase, TemplateWrapperReference, compose_child_decorator_wrapper,
    wrapper_reference_for_template,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates;
use crate::compiler_frontend::compiler_messages::{
    CompilerStage, InvalidTemplateDirectiveReason, InvalidTemplateSlotReason,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashSet;
use std::cell::RefCell;
use std::rc::Rc;

/// One optional direct-child decorator edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChildDecoratorEdge {
    Before,
    After,
}

/// Typed result for the connected child-decorator directive family.
type ChildDecoratorResult<T> = Result<T, TemplateError>;

/// Parses `$before <compile-time value>` or `$after <compile-time value>`.
///
/// The caller leaves the cursor on the directive token. A successful return
/// leaves it after the operand expression so the head parser continues with
/// the separator or body boundary, like a slot declaration.
#[allow(
    clippy::too_many_arguments,
    reason = "the directive name and edge are extra inputs beside the usual parser borrows"
)]
pub(super) fn parse_child_decorator_directive(
    directive_name: StringId,
    edge: ChildDecoratorEdge,
    token_stream: &mut AstCursor<'_>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    build_state: &mut TemplateBuildState,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> ChildDecoratorResult<()> {
    // The directive owns its token: the head parser only advances after a
    // directive that did not consume trailing tokens itself.
    token_stream.advance();

    // The assignment spelling was removed with no compatibility mode: `$before` and `$after` take
    // one bare operand on the directive's line.
    if token_stream.current_tag() == TokenTag::ASSIGN {
        return Err(invalid_child_decorator_value(
            directive_name,
            Some(token_stream.current_span()),
        ));
    }

    if child_decorator_value_ends_at(token_stream.current_tag()) {
        return Err(invalid_child_decorator_value(
            directive_name,
            Some(token_stream.current_span()),
        ));
    }

    let assignment_span = Some(token_stream.current_span());
    let expression = match parse_head_expression(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        &ValueMode::ImmutableOwned,
        false,
        string_table,
        path_fork,
    ) {
        Ok(expression) => expression.id(),
        // A decorator value must be known at this boundary, so a retained
        // numeric fragment is an invalid value rather than pending body work.
        Err(ExpressionParseError::NumericDeferral(deferral)) => {
            return Err(invalid_child_decorator_value(
                directive_name,
                Some(deferral.span.unwrap_or_else(|| token_stream.current_span())),
            ));
        }
        Err(error) => return Err(error.into()),
    };

    let expression_view = expression_builder.view(expression)?;
    let value_span = expression_view.span().or(assignment_span);
    // A stored constant reaches this position as an ordinary runtime `Reference`, a folded
    // `Constant`, or an explicit const-record field projection. Resolve each through its
    // canonical constant owner before constness classification so a wrapper's template and
    // slot identity survives instead of being rejected as `NonConst`.
    let expression = resolve_stored_constant_value(
        expression,
        expression_builder,
        context,
        const_values,
        string_table,
        path_fork,
        value_span,
    )?;
    let expression_view = expression_builder.view(expression)?;
    let value_kind = expression_view.const_value_kind_with_template_classifier(
        const_values,
        &mut |template| {
            classify_template_from_effective_tir(
                template,
                &context.template_ir_store,
                expression_builder,
                const_values,
                Some(context.external_package_registry.as_ref()),
            )
        },
    )?;

    let wrapper_reference = match decorator_value(expression_view, value_kind, const_values) {
        Some(DecoratorValue::Template(template)) => {
            validate_named_only_slots(
                &context.template_ir_store.borrow(),
                template.tir_reference.root,
            )?;
            let store = context.template_ir_store.borrow();
            wrapper_reference_for_template(template, &store)?
        }

        Some(DecoratorValue::String { plain_text }) => {
            string_decorator_wrapper(expression, plain_text, value_span, context, string_table)?
        }

        None => {
            return Err(invalid_child_decorator_value(directive_name, value_span));
        }
    };

    match edge {
        ChildDecoratorEdge::Before => build_state.child_wrapper_before = Some(wrapper_reference),
        ChildDecoratorEdge::After => build_state.child_wrapper_after = Some(wrapper_reference),
    }

    Ok(())
}

/// Composes the authored decorator edges into one direct-child wrapper.
///
/// The composite is a `Sequence` of the original prefix and suffix template
/// references around one compiler-created default-slot placeholder. Building
/// it once per owning template keeps later direct-child and runtime
/// applications on the established wrapper-set path.
pub(crate) fn finish_child_decorators(
    build_state: &mut TemplateBuildState,
    store_handle: &Rc<RefCell<TemplateIrStore>>,
) -> ChildDecoratorResult<()> {
    let before = build_state.child_wrapper_before.take();
    let after = build_state.child_wrapper_after.take();
    if before.is_none() && after.is_none() {
        return Ok(());
    }

    let reference = compose_child_decorator_wrapper(&mut store_handle.borrow_mut(), before, after)?;
    build_state.child_wrappers.push(reference);

    Ok(())
}

/// Resolves one stored compile-time value into the shape decorator classification needs.
///
/// WHAT: replaces a runtime-context `Reference` with the constant resolution a constant context
///       receives, projects an explicit const-record field through the canonical field owner, and
///       rebuilds a folded template-wrapper constant into a `Template` expression.
/// WHY: decorator values are compile-time composition patterns even when the owning template is
///      runtime-capable. Each step delegates to its existing owner, so this path neither evaluates
///      a value a second time nor accepts runtime data absent from the constant store.
fn resolve_stored_constant_value(
    expression: ExprId,
    expression_builder: &mut AstExpressionBuilder,
    context: &ScopeContext,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
    span: Option<SourceSpan>,
) -> ChildDecoratorResult<ExprId> {
    let expression = resolve_stored_constant_reference(
        expression,
        expression_builder,
        context,
        const_values,
        string_table,
        path_fork,
        span,
    )?;
    let expression = project_stored_constant_field(
        expression,
        expression_builder,
        context,
        const_values,
        path_fork,
    )?;
    let materialized = materialize_constant_template_expression(
        expression_builder.own(expression),
        context,
        expression_builder,
        const_values,
        string_table,
        span,
    )?;
    Ok(materialized.id())
}

/// Projects an explicit const-record field into its folded constant value.
///
/// WHAT: delegates to the postfix field owner so a decorator value such as `settings.prefix`
///       reaches classification with the field's stored constant identity.
/// WHY: a runtime-capable receiver keeps the field access as an expression, while a constant
///      receiver inlines the same field as a folded constant. Both forms must resolve through the
///      canonical projection, which applies visibility, authored `#` provenance and const-record
///      checks, so a runtime record keeps its field access and is rejected by the value check.
fn project_stored_constant_field(
    expression: ExprId,
    expression_builder: &mut AstExpressionBuilder,
    context: &ScopeContext,
    const_values: &ConstValueStore,
    path_fork: &PathInternerFork,
) -> ChildDecoratorResult<ExprId> {
    let projected = project_explicit_const_record_field(
        expression_builder.own(expression),
        context,
        path_fork,
        expression_builder,
        const_values,
    )?;
    Ok(projected.map_or(expression, |projected| projected.id()))
}

/// Resolves a direct reference to an explicit compile-time constant.
///
/// WHAT: replaces a runtime-context `Reference` with the same constant resolution a
///       constant context receives: an authored initializer keeps its expression and TIR
///       identity, while an imported constant template rebuilds through the canonical
///       stored public projection.
/// WHY: decorator values are compile-time composition patterns even when the owning template
///      is runtime-capable. A declaration without authored compile-time provenance keeps its
///      `Reference` so the ordinary value check rejects it instead of turning a runtime
///      template into a wrapper.
fn resolve_stored_constant_reference(
    expression: ExprId,
    expression_builder: &mut AstExpressionBuilder,
    context: &ScopeContext,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
    span: Option<SourceSpan>,
) -> ChildDecoratorResult<ExprId> {
    let view = expression_builder.view(expression)?;
    let ExpressionOperationView::Reference(path) = view.operation() else {
        return Ok(expression);
    };

    // Module and imported constants resolve by their canonical path. Body-local constants live
    // in lexical frames, so they fall back to the simple name and must still name this exact
    // declaration: a same-named binding resolved later would be a different value than the
    // expression already carries.
    let declaration = context
        .shared
        .top_level_declarations
        .get_visible_resolved_by_path(&path, context.visible_declaration_ids.as_deref());
    let lexical_reference = if declaration.is_none() {
        let Some(name) = path_fork.component(path) else {
            return Ok(expression);
        };
        context.get_value_reference(&name, span).ok().flatten()
    } else {
        None
    };
    let declaration = match declaration {
        Some(declaration) => declaration,
        None => {
            let Some(reference) = lexical_reference.as_ref() else {
                return Ok(expression);
            };
            let Some(declaration) = reference.ready_declaration() else {
                return Ok(expression);
            };
            if declaration.id != path {
                return Ok(expression);
            }
            declaration
        }
    };

    // Only authored compile-time constants may become wrapper values; a runtime template or
    // variable keeps its Reference and is rejected by the ordinary value check.
    if !context.is_explicit_compile_time_constant(declaration) {
        return Ok(expression);
    }

    Ok(constant_reference_expression_from_declaration(
        declaration,
        context,
        expression_builder,
        const_values,
        string_table,
        span,
    )?
    .id())
}

/// One accepted child-decorator value after compile-time validation.
enum DecoratorValue<'a> {
    /// A constant template value; its wrapper identity is preserved.
    Template(&'a Template),
    /// A compile-time `String`. `plain_text` is present only when the value can
    /// keep the plain-text fast path; structural pieces stay a dynamic expression.
    String { plain_text: Option<StringId> },
}

/// Selects the accepted value shape without evaluating anything a second time.
fn decorator_value<'a>(
    expression: ExpressionView<'a>,
    value_kind: ConstValueKind,
    const_values: &ConstValueStore,
) -> Option<DecoratorValue<'a>> {
    match expression.operation() {
        ExpressionOperationView::Template(template)
            if matches!(
                value_kind,
                ConstValueKind::RenderableTemplate | ConstValueKind::TemplateWrapper
            ) =>
        {
            Some(DecoratorValue::Template(template))
        }

        ExpressionOperationView::Coerce { value, .. } => {
            decorator_value(value, value_kind, const_values)
        }

        ExpressionOperationView::Constant(_) if value_kind.is_compile_time_value() => {
            let plain_text = match constant_string_payload(expression, const_values)? {
                ConstantStringPayload::Text {
                    text,
                    carries_provenance: false,
                } if expression
                    .occurrence_facts()
                    .is_none_or(|facts| facts.provenance.is_empty()) =>
                {
                    Some(text)
                }
                ConstantStringPayload::Text { .. } | ConstantStringPayload::Pieces => None,
            };
            Some(DecoratorValue::String { plain_text })
        }

        _ => None,
    }
}

/// The compile-time `String` payload behind an accepted constant expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConstantStringPayload {
    Text {
        text: StringId,
        /// Whether the spelling carries synthetic interface provenance that the
        /// plain-text fast path would drop.
        carries_provenance: bool,
    },
    Pieces,
}

/// Resolves the constant `String` behind an accepted constant expression.
///
/// Coercion payloads are transparent: the decorator value is the string the
/// expression produced, not the coercion step. Non-String payloads are
/// rejected by returning `None`. The payload, not the value-kind label, owns
/// this decision: a folded template-derived String is classified
/// `RenderableTemplate` but is still an ordinary compile-time `String`.
fn constant_string_payload(
    expression: ExpressionView<'_>,
    const_values: &ConstValueStore,
) -> Option<ConstantStringPayload> {
    match expression.operation() {
        ExpressionOperationView::Constant(value) => {
            let mut resolved = value;
            let mut carries_provenance = false;
            loop {
                carries_provenance |= const_values
                    .metadata(resolved)
                    .is_some_and(|metadata| !metadata.synthetic_interface_provenance.is_empty());
                match const_values.payload(resolved)? {
                    ConstValuePayload::String(ConstStringValue::Text(text)) => {
                        return Some(ConstantStringPayload::Text {
                            text: *text,
                            carries_provenance,
                        });
                    }
                    ConstValuePayload::String(ConstStringValue::Pieces(_)) => {
                        return Some(ConstantStringPayload::Pieces);
                    }
                    ConstValuePayload::Coerced(inner) => resolved = *inner,
                    _ => return None,
                }
            }
        }
        ExpressionOperationView::Coerce { value, .. } => {
            constant_string_payload(value, const_values)
        }
        _ => None,
    }
}

/// Builds a wrapper template around one compile-time `String` decorator value.
fn string_decorator_wrapper(
    expression: ExprId,
    plain_text: Option<StringId>,
    span: Option<SourceSpan>,
    context: &ScopeContext,
    string_table: &StringTable,
) -> ChildDecoratorResult<TemplateWrapperReference> {
    let mut construction_context =
        TemplateConstructionContext::new(context.template_ir_store.clone(), span);
    match plain_text {
        Some(text) => {
            construction_context.record_text(text, string_table.resolve(text).len(), span);
        }
        None => {
            // Structural pieces and provenance-bearing strings must stay a
            // dynamic expression so folding keeps resources and site roots.
            construction_context.record_head_dynamic_expression(expression, span);
        }
    }

    // The wrapper root is complete once the literal or dynamic expression is
    // recorded: it owns no child contributions or slot routing awaiting
    // composition. Wrapper sets consume wrapper references as preparation and
    // projection roots, which require `Composed`-or-later identity; `Parsed`
    // would understate the finished root and block const-template projection.
    let reference = construction_context.finish(
        Style::default(),
        TemplateType::String,
        TemplateTirPhase::Composed,
    )?;

    Ok(TemplateWrapperReference::new(
        reference.root,
        reference.phase,
        reference.context,
    ))
}

/// Rejects default and positional slot occurrences anywhere in a decorator value.
///
/// Named occurrences are the value's only addressable interface: the composite
/// wrapper owns the single implicit default insertion point for the direct
/// child. Nested template values are part of the same authored wrapper, so the
/// walk follows child-template references as well as control-flow bodies.
fn validate_named_only_slots(
    store: &TemplateIrStore,
    template_id: TemplateIrId,
) -> ChildDecoratorResult<()> {
    let mut visited_templates = FxHashSet::default();
    let mut visited_nodes = FxHashSet::default();
    visited_templates.insert(template_id);
    validate_named_only_slots_in_template(
        store,
        template_id,
        &mut visited_templates,
        &mut visited_nodes,
    )
}

fn validate_named_only_slots_in_template(
    store: &TemplateIrStore,
    template_id: TemplateIrId,
    visited_templates: &mut FxHashSet<TemplateIrId>,
    visited_nodes: &mut FxHashSet<TemplateIrNodeId>,
) -> ChildDecoratorResult<()> {
    let root = store
        .get_template(template_id)
        .map(|template| template.root)
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "child decorator validation referenced a missing template.",
            )
        })?;
    validate_named_only_slots_in_node(store, root, visited_templates, visited_nodes)
}

fn validate_named_only_slots_in_node(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
    visited_templates: &mut FxHashSet<TemplateIrId>,
    visited_nodes: &mut FxHashSet<TemplateIrNodeId>,
) -> ChildDecoratorResult<()> {
    if !visited_nodes.insert(node_id) {
        return Ok(());
    }
    let node = store.get_node(node_id).ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "child decorator validation referenced a missing node.",
        )
    })?;

    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => {
            for child_id in children {
                validate_named_only_slots_in_node(
                    store,
                    *child_id,
                    visited_templates,
                    visited_nodes,
                )?;
            }
        }

        TemplateIrNodeKind::Slot { placeholder } => match &placeholder.key {
            SlotKey::Default => {
                return Err(templates::invalid_template_slot(
                    InvalidTemplateSlotReason::DefaultSlotInChildDecorator,
                    None,
                    None,
                    node.span,
                )
                .into());
            }
            SlotKey::Positional(_) => {
                return Err(templates::invalid_template_slot(
                    InvalidTemplateSlotReason::PositionalSlotInChildDecorator,
                    None,
                    None,
                    node.span,
                )
                .into());
            }
            SlotKey::Named(_) => {}
        },

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            if visited_templates.insert(reference.root) {
                validate_named_only_slots_in_template(
                    store,
                    reference.root,
                    visited_templates,
                    visited_nodes,
                )?;
            }
        }

        TemplateIrNodeKind::InsertContribution { template } => {
            if visited_templates.insert(*template) {
                validate_named_only_slots_in_template(
                    store,
                    *template,
                    visited_templates,
                    visited_nodes,
                )?;
            }
        }

        TemplateIrNodeKind::Conditional { body, .. } => {
            validate_named_only_slots_in_node(store, *body, visited_templates, visited_nodes)?;
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            validate_named_only_slots_in_node(store, *body, visited_templates, visited_nodes)?;
            if let Some(aggregate_wrapper_id) = aggregate_wrapper {
                validate_named_only_slots_in_node(
                    store,
                    *aggregate_wrapper_id,
                    visited_templates,
                    visited_nodes,
                )?;
            }
        }

        TemplateIrNodeKind::Text { .. }
        | TemplateIrNodeKind::DynamicExpression { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. }
        | TemplateIrNodeKind::RetainedNumericIntake(_) => {}
    }

    Ok(())
}

/// Builds the directive-owned diagnostic for an unusable decorator value.
fn invalid_child_decorator_value(
    directive_name: StringId,
    span: Option<SourceSpan>,
) -> TemplateError {
    templates::invalid_template_directive(
        Some(directive_name),
        InvalidTemplateDirectiveReason::InvalidChildDecoratorValue,
        span,
    )
    .into()
}

/// Tokens that cannot begin an assigned decorator value.
fn child_decorator_value_ends_at(tag: TokenTag) -> bool {
    matches!(
        tag,
        TokenTag::TEMPLATE_CLOSE
            | TokenTag::START_TEMPLATE_BODY
            | TokenTag::COMMA
            | TokenTag::EOF
            | TokenTag::ELSE
            | TokenTag::END
            | TokenTag::CLOSE_CURLY
    )
}
