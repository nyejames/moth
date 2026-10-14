//! Head expression insertion helpers.
//!
//! WHAT:
//! - Normalizes values inserted from template heads into parser TIR.
//! - Handles template-valued expressions, non-template expressions, and compile-time
//!   path resolution.
//!
//! WHY:
//! - Head parsing needs one place for const-context checks so the orchestration
//!   loop remains readable and consistent.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::const_values::store::{ConstStringValue, ConstValuePayload};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExpressionOperationView, ExpressionView, OwnedExpr,
};
use crate::compiler_frontend::ast::file_value_resolution::resolve_file_value;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{TemplateSegmentOrigin, TemplateType};
use crate::compiler_frontend::ast::templates::template_renderability::is_template_renderable_type;
use crate::compiler_frontend::ast::templates::tir::TemplateConstructionContext;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;

pub(super) struct TemplateHeadExpressionContext<'a> {
    pub(super) context: &'a ScopeContext,
    pub(super) type_environment: &'a TypeEnvironment,
    pub(super) expression_builder: &'a AstExpressionBuilder,
    pub(super) const_values: &'a ConstValueStore,
    pub(super) construction_context: &'a mut TemplateConstructionContext,
    pub(super) path_fork: &'a PathInternerFork,
    /// Value mode the head intake parsed this expression with.
    ///
    /// WHY: a retained numeric intake resumes the ordinary typed expression intake with the
    ///      same access classification the head used, so the metadata must travel with it.
    pub(super) value_mode: ValueMode,
}

/// Typed result shared by head-expression insertion helpers.
type HeadExpressionResult<T> = Result<T, TemplateError>;

fn is_unresolved_constant_placeholder_reference(
    expression: ExpressionView<'_>,
    context: &ScopeContext,
    path_fork: &PathInternerFork,
) -> bool {
    let ExpressionOperationView::Reference(path) = expression.operation() else {
        return false;
    };

    path_fork
        .component(path)
        .and_then(|name| context.get_reference(&name))
        .is_some_and(|reference| {
            // A pending numeric binding is a runtime body-local value with no declaration,
            // never a constant placeholder, so it cannot satisfy this predicate.
            reference
                .ready_declaration()
                .is_some_and(|declaration| context.is_unresolved_constant_placeholder(declaration))
        })
}

pub(crate) fn validate_template_head_value_type(
    expression: ExpressionView<'_>,
    span: Option<SourceSpan>,
    type_environment: &TypeEnvironment,
) -> HeadExpressionResult<()> {
    let type_id = expression.require_single(ValueReceiverContext::TemplateInterpolation)?;
    if expression.error_type().is_some() {
        return Err(with_source_span(
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::FallibleValueInTemplateHead,
                span,
            ),
            span,
        )
        .into());
    }

    // Template head values must be simple scalar types that can render as
    // text. Templates and paths are handled by separate code paths, so this
    // function only sees non-template, non-path expression values.
    //
    // Use the shared renderability classifier so the policy lives in one
    // AST-template-owned place and uses semantic TypeId identity.
    if is_template_renderable_type(type_id, type_environment) {
        return Ok(());
    }

    Err(with_source_span(
        CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::UnsupportedTypeInTemplateHead { type_id },
            span,
        ),
        span,
    )
    .into())
}

fn with_source_span(
    mut diagnostic: CompilerDiagnostic,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    if diagnostic.primary_span.is_none() {
        diagnostic.primary_span = span;
    }
    diagnostic
}

/// Handles a template-typed value found in the template head.
/// Wrapper templates preserve slot semantics while preparation owns constness.
pub(super) fn handle_template_value_in_template_head(
    value: &Template,
    context: &ScopeContext,
    construction_context: &mut TemplateConstructionContext,
    span: Option<SourceSpan>,
) -> HeadExpressionResult<()> {
    let template_kind = {
        let store = context.template_ir_store.borrow();
        store
            .get_template(value.tir_reference.root)
            .map(|template_ir| template_ir.kind.clone())
            .ok_or_else(|| {
                TemplateError::from(
                    crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                        "Template head value was missing from the module TIR store.",
                    ),
                )
            })?
    };

    if context.kind.is_constant_context() && matches!(&template_kind, TemplateType::StringFunction)
    {
        return Err(with_source_span(
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::RuntimeTemplateInConst,
                span,
            ),
            span,
        )
        .into());
    }

    if matches!(&template_kind, TemplateType::Comment(_)) {
        return Ok(());
    }

    if matches!(&template_kind, TemplateType::SlotDefinition(_)) {
        return Err(with_source_span(
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::SlotInHead,
                span,
            ),
            span,
        )
        .into());
    }

    let child_reference = &value.tir_reference;
    // `$insert("name")` helpers are slot contributions, not ordinary child
    // template output. Recording them as `InsertContribution` nodes lets
    // TIR-native slot routing bucket them by the helper's target slot key
    // rather than treating them as loose fill content.
    if matches!(&template_kind, TemplateType::SlotInsert(_)) {
        construction_context.record_insert_contribution(child_reference.root, span);
    } else {
        construction_context.record_child_template(
            child_reference,
            TemplateSegmentOrigin::Head,
            span,
        );
    }

    Ok(())
}

/// Pushes one parsed head expression, retaining a numeric deferral in TIR.
///
/// WHAT: a completed expression takes the ordinary head path; an owner-marked numeric deferral
///       records a payload-free TIR position plus one retained intake under the owning body
///       domain, so body-root completion installs the completed value there.
/// WHY: the head is a formatting consumer, so it must not select a pending value's type. The
///      deferral is the producer's actual retained result, and only the position (routing) and
///      the intake metadata stay behind until completion.
pub(super) fn push_parsed_template_head_expression(
    result: Result<OwnedExpr, ExpressionParseError>,
    target: TemplateHeadExpressionContext<'_>,
    span: Option<SourceSpan>,
    string_table: &StringTable,
) -> HeadExpressionResult<()> {
    match result {
        Ok(expression) => push_template_head_expression(expression, target, span, string_table),
        Err(error) => retain_template_head_numeric_deferral(error, target, span),
    }
}

/// Retains one numeric deferral raised by a template head expression position.
///
/// WHAT: consumes the retained fragment from the expression producer and records it against a
///       payload-free head position.
/// WHY: the head states no numeric expectation and offers no cast channel, so it never fabricates
///      a typed payload: the fragment waits in the store's intake table until body completion
///      selects the binding's single static type.
fn retain_template_head_numeric_deferral(
    error: ExpressionParseError,
    target: TemplateHeadExpressionContext<'_>,
    span: Option<SourceSpan>,
) -> HeadExpressionResult<()> {
    let deferral = error.take_numeric_deferral("a template head expression")?;

    target.construction_context.record_head_numeric_intake(
        target.context.numeric_domain_id(),
        deferral.items,
        deferral.expected,
        target.value_mode,
        // The authored trailing handling travels with the fragment, so completion applies the
        // exact `!` the author wrote instead of re-reading tokens.
        deferral.suffix,
        deferral.span.or(span),
    )?;

    Ok(())
}

/// Pushes a non-template expression into the head content after validation.
pub(super) fn push_template_head_expression(
    expression: OwnedExpr,
    target: TemplateHeadExpressionContext<'_>,
    span: Option<SourceSpan>,
    string_table: &StringTable,
) -> HeadExpressionResult<()> {
    let expression = target.expression_builder.view_owned(expression)?;
    if let ExpressionOperationView::Template(template_value) = expression.operation() {
        return handle_template_value_in_template_head(
            template_value,
            target.context,
            target.construction_context,
            span,
        );
    }

    let defer_inferred_type_validation =
        is_unresolved_constant_placeholder_reference(expression, target.context, target.path_fork);

    if !defer_inferred_type_validation {
        validate_template_head_value_type(expression, span, target.type_environment)?;
    }

    let expression_is_compile_time_constant = expression
        .const_value_kind_with_template_classifier(target.const_values, &mut |template| {
            classify_template_from_effective_tir(
                template,
                &target.context.template_ir_store,
                target.expression_builder,
                target.const_values,
            )
        })?
        .is_compile_time_value();

    if target.context.kind.is_constant_context()
        && !expression_is_compile_time_constant
        && !is_unresolved_constant_placeholder_reference(
            expression,
            target.context,
            target.path_fork,
        )
    {
        return Err(with_source_span(
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::RuntimeValueInConstTemplateHead,
                span,
            ),
            span,
        )
        .into());
    }

    // Record head segments into parser TIR in source order before any body
    // nodes are appended.
    // Text nodes carry no provenance, so a constant reference that already folded to a
    // `StringSlice` must stay a dynamic expression when it bears provenance. The dynamic
    // fold path merges the selected payload's provenance into the template output, while
    // ordinary provenance-free literals keep the text fast path.
    let plain_text = match expression.operation() {
        ExpressionOperationView::Constant(value)
            if expression
                .occurrence_facts()
                .is_none_or(|facts| facts.provenance.is_empty()) =>
        {
            match target.const_values.payload(value).ok_or_else(|| {
                CompilerError::compiler_error(
                    "template expression references an absent constant value",
                )
            })? {
                ConstValuePayload::String(ConstStringValue::Text(text)) => Some(*text),
                _ => None,
            }
        }
        _ => None,
    };
    if let Some(text) = plain_text {
        let byte_len = string_table.resolve(text).len();
        target
            .construction_context
            .record_head_text(text, byte_len, span);
    } else {
        target
            .construction_context
            .record_head_dynamic_expression(expression.id(), span);
    }

    Ok(())
}

/// Resolves a compile-time file value in template-head context and records its expression.
///
/// Stage 0 owns physical resolution for every authored path occurrence. Reusing the ordinary
/// file-value resolver keeps resource and site-root values structural while preserving the shared
/// extensionless and source-kind diagnostics.
#[allow(
    clippy::too_many_arguments,
    reason = "the path syntax and construction context are extra inputs beside the parser borrows"
)]
pub(super) fn push_template_head_path_expression(
    path_syntax: PathSyntaxId,
    token_stream: &AstCursor,
    context: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    construction_context: &mut TemplateConstructionContext,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> HeadExpressionResult<()> {
    let value_mode = ValueMode::ImmutableOwned;
    let source_span = path_source_span_for(token_stream);
    let expression = resolve_file_value(
        path_syntax,
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        &value_mode,
        string_table,
        path_fork,
    )
    .map_err(|error| with_source_span_error(source_span, TemplateError::from(error)))?;

    push_template_head_expression(
        expression,
        TemplateHeadExpressionContext {
            context,
            type_environment: type_interner.environment(),
            expression_builder,
            const_values,
            construction_context,
            path_fork: &*path_fork,
            value_mode,
        },
        source_span,
        string_table,
    )
}

/// Read-only path-expression source span on the canonical cursor view.
///
/// WHAT: reports the current token span for head path diagnostics and insertion.
/// WHY: the path span is a pure token-local read.
fn path_source_span_for(token_stream: &AstCursor) -> Option<SourceSpan> {
    Some(token_stream.current_span())
}

fn with_source_span_error(source_span: Option<SourceSpan>, error: TemplateError) -> TemplateError {
    error.map_diagnostic(|mut diagnostic| {
        if diagnostic.primary_span.is_none() {
            diagnostic.primary_span = source_span;
        }
        diagnostic
    })
}
