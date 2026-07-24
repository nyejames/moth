//! Catch-handler semantic validation helpers.
//!
//! WHAT: validates handler bindings, name conflicts, and value-required fallthrough rules.
//! WHY: parser entrypoints should stay focused on syntax while this module owns semantic checks
//! shared by call and expression fallible handling.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::identifier_policy::{
    IdentifierNamingKind, ensure_not_keyword_shadow_identifier, naming_warning_for_identifier,
};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

use crate::compiler_frontend::ast::statements::value_production::{
    BranchFlow, analyze_branch_flow,
};

/// Validates that a catch-handler binding name is legal and emits naming warnings.
///
/// WHAT: checks the handler identifier does not shadow a keyword and follows naming conventions.
/// WHY: catch handlers introduce a new local binding, so they must obey the same identifier
/// policy as ordinary value declarations.
pub(super) fn validate_catch_fallible_handler_binding(
    handler_name: StringId,
    location: SourceLocation,
    _compilation_stage: &str,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &StringTable,
) -> Result<(), ExpressionParseError> {
    ensure_not_keyword_shadow_identifier(handler_name, location.to_owned(), string_table)
        .map_err(ExpressionParseError::from)?;

    if let Some(warning) = naming_warning_for_identifier(
        handler_name,
        location,
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
    location: SourceLocation,
) -> Result<(), ExpressionParseError> {
    if context.get_reference(&handler_name).is_some() {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::CatchHandlerConflicts,
            location,
        )
        .into());
    }

    Ok(())
}

/// Validates that a catch handler body cannot fall through when a value is required.
///
/// WHAT: rejects handler bodies that can simply fall through when the call expression must
/// still produce success values for the surrounding statement/expression.
/// WHY: without fallback values, a fallthrough path would leave the handled call with no value
/// continuation to merge back into.
pub(super) fn validate_catch_fallible_handler_value_requirement(
    value_required: bool,
    success_result_type_ids: &[TypeId],
    handler_body: &[AstNode],
    location: SourceLocation,
) -> Result<(), ExpressionParseError> {
    let body_flow = analyze_branch_flow(handler_body);

    if value_required
        && !success_result_type_ids.is_empty()
        && !matches!(
            body_flow,
            BranchFlow::ProducesValue | BranchFlow::Terminates
        )
    {
        return Err(CompilerDiagnostic::invalid_fallible_handling(
            InvalidFallibleHandlingReason::CatchHandlerCanFallThrough,
            location,
        )
        .into());
    }

    Ok(())
}
