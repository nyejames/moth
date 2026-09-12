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
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSource,
};
use crate::compiler_frontend::ast::file_value_resolution::resolve_file_value;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{
    ReactiveSubscription, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_renderability::is_template_renderable_type;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::ast::templates::tir::TemplateConstructionContext;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::value_mode::ValueMode;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::FileTokens;

pub(super) struct TemplateHeadExpressionContext<'a> {
    pub(super) context: &'a ScopeContext,
    pub(super) type_environment: &'a TypeEnvironment,
    pub(super) construction_context: &'a mut TemplateConstructionContext,
    pub(super) path_fork: &'a PathInternerFork,
}

/// Typed result shared by head-expression insertion helpers.
type HeadExpressionResult<T> = Result<T, TemplateError>;

fn is_unresolved_constant_placeholder_reference(
    expression: &Expression,
    context: &ScopeContext,
    path_fork: &PathInternerFork,
) -> bool {
    let ExpressionKind::Reference(path) = &expression.kind else {
        return false;
    };

    path_fork
        .component(*path)
        .and_then(|name| context.get_reference(&name))
        .is_some_and(|declaration| {
            declaration
                .as_declaration()
                .is_unresolved_constant_placeholder()
        })
}

fn validate_template_head_value_type(
    expression: &Expression,
    span: Option<SourceSpan>,
    type_environment: &TypeEnvironment,
) -> HeadExpressionResult<()> {
    if type_environment.is_fallible_carrier(expression.type_id) {
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
    if is_template_renderable_type(expression.type_id, type_environment) {
        return Ok(());
    }

    Err(with_source_span(
        CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::UnsupportedTypeInTemplateHead {
                type_id: expression.type_id,
            },
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

/// Pushes a non-template expression into the head content after validation.
pub(super) fn push_template_head_expression(
    expression: Expression,
    target: TemplateHeadExpressionContext<'_>,
    span: Option<SourceSpan>,
    string_table: &StringTable,
) -> HeadExpressionResult<()> {
    if let ExpressionKind::Template(template_value) = &expression.kind {
        return handle_template_value_in_template_head(
            template_value,
            target.context,
            target.construction_context,
            span,
        );
    }

    let defer_inferred_type_validation =
        is_unresolved_constant_placeholder_reference(&expression, target.context, target.path_fork);

    if !defer_inferred_type_validation {
        validate_template_head_value_type(&expression, span, target.type_environment)?;
    }

    let expression_needs_constness =
        target.context.kind.is_constant_context() || !expression.kind.is_foldable();
    let expression_is_compile_time_constant = if expression_needs_constness {
        expression
            .const_value_kind_with_template_classifier(&mut |template| {
                classify_template_from_effective_tir(template, &target.context.template_ir_store)
            })?
            .is_compile_time_value()
    } else {
        false
    };

    if target.context.kind.is_constant_context()
        && !expression_is_compile_time_constant
        && !is_unresolved_constant_placeholder_reference(
            &expression,
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

    // Ordinary `[source]` insertion is a snapshot read. Keep reactive source identity only for
    // the explicit `$(source)` subscription path.
    let mut snapshot_expression = expression;
    snapshot_expression.clear_reactive_source();

    // Record head segments into parser TIR in source order before any body
    // nodes are appended.
    match &snapshot_expression.kind {
        ExpressionKind::StringSlice(text) => {
            let byte_len = string_table.resolve(*text).len();
            target
                .construction_context
                .record_head_text(*text, byte_len, span);
        }

        _ => {
            target.construction_context.record_head_dynamic_expression(
                snapshot_expression.clone(),
                None,
                span,
            );
        }
    }

    Ok(())
}

/// Pushes an explicit `$(source)` subscription into parser TIR.
///
/// WHAT: reuses the ordinary reference expression for current rendering while attaching V1
/// subscription metadata to the segment.
/// WHY: the language type stays `String`/underlying scalar rendering; the reactive dependency is
/// a template fact for later HIR/backend phases, not a value type or borrow.
pub(super) fn push_template_head_reactive_subscription(
    expression: Expression,
    source: ReactiveSource,
    target: TemplateHeadExpressionContext<'_>,
    span: Option<SourceSpan>,
    string_table: &StringTable,
) -> HeadExpressionResult<()> {
    if target.context.kind.is_constant_context() {
        return Err(with_source_span(
            CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::ReactiveSubscriptionInConstTemplate,
                span,
            ),
            span,
        )
        .into());
    }

    validate_template_head_value_type(&expression, span, target.type_environment)?;

    let subscription = ReactiveSubscription {
        source,
        type_id: expression.type_id,
        span,
    };
    // Reactive literal text in the head is recorded as a Text node carrying the
    // subscription in the store side-table, not as a dynamic-expression anchor.
    // The dependency remains available to reactive metadata and HIR invalidation.
    match &expression.kind {
        ExpressionKind::StringSlice(text) => {
            let byte_len = string_table.resolve(*text).len();
            target.construction_context.record_reactive_head_text(
                *text,
                byte_len,
                Some(subscription.clone()),
                span,
            );
        }
        _ => {
            target.construction_context.record_head_dynamic_expression(
                expression.clone(),
                Some(subscription.clone()),
                span,
            );
        }
    }

    Ok(())
}
/// Resolves a compile-time file value in template-head context and records its expression.
///
/// Stage 0 owns physical resolution for every authored path occurrence. Reusing the ordinary
/// file-value resolver keeps resource and site-root values structural while preserving the shared
/// extensionless and source-kind diagnostics.
pub(super) fn push_template_head_path_expression(
    path_syntax: PathSyntaxId,
    token_stream: &FileTokens,
    context: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
    construction_context: &mut TemplateConstructionContext,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> HeadExpressionResult<()> {
    let value_mode = ValueMode::ImmutableOwned;
    let source_span = Some(token_stream.current_span());
    let expression = resolve_file_value(
        path_syntax,
        token_stream,
        context,
        type_interner,
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
            construction_context,
            path_fork: &*path_fork,
        },
        source_span,
        string_table,
    )
}

fn with_source_span_error(source_span: Option<SourceSpan>, error: TemplateError) -> TemplateError {
    error.map_diagnostic(|mut diagnostic| {
        if diagnostic.primary_span.is_none() {
            diagnostic.primary_span = source_span;
        }
        diagnostic
    })
}
