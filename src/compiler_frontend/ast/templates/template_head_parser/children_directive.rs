//! `$children(..)` directive parsing and normalization.
//!
//! WHAT:
//! - Parses `$children(template_or_string)` arguments.
//! - Validates compile-time restrictions and accepted argument types.
//! - Normalizes string arguments into wrapper templates.
//!
//! WHY:
//! - `$children(..)` has directive-specific compile-time behavior that should stay
//!   isolated from generic style-handler logic.

use super::directive_args::parse_required_parenthesized_expression;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{Style, TemplateType};
use crate::compiler_frontend::ast::templates::template_build_state::TemplateBuildState;
use crate::compiler_frontend::ast::templates::tir::{
    TemplateConstructionContext, TemplateTirPhase, TemplateWrapperReference,
    wrapper_reference_for_template,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateDirectiveReason,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};

/// Typed result for the connected `$children` directive family.
type ChildrenDirectiveResult<T> = Result<T, TemplateError>;

/// Parses the `$children(template_or_string)` directive which specifies a
/// wrapper template to apply around all direct child templates in the body.
pub(super) fn parse_children_style_directive(
    directive_name: StringId,
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    build_state: &mut TemplateBuildState,
    string_table: &mut StringTable,
) -> ChildrenDirectiveResult<()> {
    let directive_argument = parse_required_parenthesized_expression(
        directive_name,
        token_stream,
        context,
        type_interner,
        string_table,
    )
    .map_err(|error| error.map_diagnostic(|diagnostic| {
        // Convert the generic EmptyArguments reason into the children-specific
        // InvalidChildrenArgument so rendered guidance mentions wrapper templates
        // and strings rather than generic empty parens.
        if matches!(
            diagnostic.payload,
            crate::compiler_frontend::compiler_messages::DiagnosticPayload::InvalidTemplateDirective {
                reason: InvalidTemplateDirectiveReason::EmptyArguments,
                ..
            }
        ) {
            Box::new(CompilerDiagnostic::invalid_template_directive(
                Some(directive_name),
                InvalidTemplateDirectiveReason::InvalidChildrenArgument,
                diagnostic.primary_location.clone(),
            ))
        } else {
            diagnostic
        }
    }))?;
    let argument_location = directive_argument.location.clone();

    // The wrapper must be fully known at compile time; runtime expressions
    // cannot determine how children are composed.
    let argument_is_compile_time_constant = directive_argument
        .const_value_kind_with_template_classifier(&mut |template| {
            classify_template_from_effective_tir(template, &context.template_ir_store)
        })?
        .is_compile_time_value();

    if !argument_is_compile_time_constant {
        return Err(CompilerDiagnostic::invalid_template_directive(
            Some(string_table.intern("children")),
            InvalidTemplateDirectiveReason::InvalidChildrenArgument,
            argument_location,
        )
        .into());
    }

    // Normalize the argument at the directive boundary. Accepted wrappers
    // already have durable TIR authority, so later template construction
    // carries only the module-local wrapper reference.
    let wrapper_reference = match directive_argument.kind {
        ExpressionKind::Template(child_template) => {
            let current_store = context.template_ir_store.borrow();
            let child_template_root = child_template.tir_reference.root;
            let child_kind = current_store
                .get_template(child_template_root)
                .map(|template_ir| template_ir.kind.clone())
                .ok_or_else(|| {
                    TemplateError::from(
                        crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                            "`$children` referenced a template missing from the module TIR store.",
                        ),
                    )
                })?;
            if matches!(
                child_kind,
                TemplateType::StringFunction
                    | TemplateType::SlotDefinition(_)
                    | TemplateType::SlotInsert(_)
                    | TemplateType::Comment(_)
            ) {
                return Err(CompilerDiagnostic::invalid_template_directive(
                    Some(string_table.intern("children")),
                    InvalidTemplateDirectiveReason::InvalidChildrenArgument,
                    argument_location,
                )
                .into());
            }

            // The wrapper-reference helper reports internal authority failures
            // as `CompilerError`; carry them through `TemplateError` so the
            // diagnostic lane stays the infrastructure lane rather than a
            // fabricated user-facing directive diagnostic.
            wrapper_reference_for_template(&child_template, &current_store)
                .map_err(TemplateError::from)?
        }

        ExpressionKind::StringSlice(value) => normalize_string_child_wrapper_reference(
            value,
            argument_location,
            context,
            string_table,
        )?,

        _ => {
            return Err(CompilerDiagnostic::invalid_template_directive(
                Some(string_table.intern("children")),
                InvalidTemplateDirectiveReason::InvalidChildrenArgument,
                argument_location,
            )
            .into());
        }
    };

    build_state.child_wrappers.push(wrapper_reference);
    Ok(())
}

/// Builds a TIR wrapper reference around a literal string id.
///
/// The resulting wrapper records its literal body directly in the module-scoped
/// parser TIR store and returns the durable module-local reference.
fn normalize_string_child_wrapper_reference(
    value: StringId,
    argument_location: SourceLocation,
    context: &ScopeContext,
    string_table: &StringTable,
) -> Result<TemplateWrapperReference, TemplateError> {
    let mut construction_context = TemplateConstructionContext::new(
        context.template_ir_store.clone(),
        argument_location.clone(),
    );
    construction_context.record_text(
        value,
        string_table.resolve(value).len(),
        argument_location.clone(),
    );
    let reference = construction_context.finish(
        Style::default(),
        TemplateType::String,
        TemplateTirPhase::Parsed,
        argument_location,
    );

    Ok(TemplateWrapperReference::new(
        reference.root,
        reference.phase,
        reference.context,
    ))
}
