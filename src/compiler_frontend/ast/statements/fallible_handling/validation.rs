//! Catch-handler semantic validation helpers.
//!
//! WHAT: validates handler bindings, name conflicts, and value-required fallthrough rules.
//! WHY: parser entrypoints should stay focused on syntax while this module owns semantic checks
//! shared by call and expression fallible handling.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticBag, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::identifier_policy::{
    IdentifierNamingKind, ensure_not_keyword_shadow_identifier, naming_warning_for_identifier,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

use crate::compiler_frontend::ast::statements::value_production::analyze_branch_exits;

/// Validates that a catch-handler binding name is legal and emits naming warnings.
///
/// WHAT: checks the handler identifier does not shadow a keyword and follows naming conventions.
/// WHY: catch handlers introduce a new local binding, so they must obey the same identifier
/// policy as ordinary value declarations.
pub(super) fn validate_catch_fallible_handler_binding(
    handler_name: StringId,
    span: Option<SourceSpan>,
    _compilation_stage: &str,
    warnings: &mut DiagnosticBag,
    string_table: &StringTable,
) -> Result<(), ExpressionParseError> {
    ensure_not_keyword_shadow_identifier(handler_name, span, string_table)
        .map_err(ExpressionParseError::from)?;

    if let Some(warning) = naming_warning_for_identifier(
        handler_name,
        span,
        IdentifierNamingKind::ValueLike,
        string_table,
    ) {
        warnings.push(warning);
    }

    Ok(())
}

/// Validates that a catch-handler binding does not collide with an existing scope reference.
///
/// WHAT: rejects handlers whose error name is already visible in the current scope.
/// WHY: Moth has a strict no-shadowing rule; catch handlers are not exempt.
pub(super) fn validate_catch_fallible_handler_conflict(
    context: &ScopeContext,
    handler_name: StringId,
    span: Option<SourceSpan>,
    string_table: &StringTable,
) -> Result<(), ExpressionParseError> {
    if context.get_reference(&handler_name).is_some() {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::CatchHandlerConflicts,
            span,
        )
        .into());
    }

    if let Some(diagnostic) =
        context.check_body_binding_visibility(handler_name, span, string_table)
    {
        return Err(diagnostic.into());
    }

    Ok(())
}

/// Validates that a catch handler body cannot fall through when a value is required.
///
/// WHAT: rejects handler bodies that can simply fall through when the call expression must
/// still produce success values for the surrounding statement/expression. `None` marks a
/// contract that only resolves at completion: the requirement is unknown, never zero success
/// values, so the body is revalidated against the resolved site instead of here.
/// WHY: without fallback values, a fallthrough path would leave the handled call with no value
/// continuation to merge back into; a provisional site has no such decision to make yet.
pub(super) fn validate_catch_fallible_handler_value_requirement(
    value_required: bool,
    success_result_type_ids: Option<&[TypeId]>,
    handler_body: &[AstNode],
    expression_builder: &AstExpressionBuilder,
    const_values: &ConstValueStore,
    span: Option<SourceSpan>,
) -> Result<(), ExpressionParseError> {
    let Some(success_result_type_ids) = success_result_type_ids else {
        return Ok(());
    };

    let body_exits = analyze_branch_exits(handler_body, expression_builder, const_values);

    if value_required && !success_result_type_ids.is_empty() && body_exits.can_fall_through {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::CatchHandlerCanFallThrough,
            span,
        )
        .into());
    }

    Ok(())
}
