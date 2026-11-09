//! Expression-qualified reference primaries.
//!
//! WHAT: consumes one prepared `PATH SYMBOL` occurrence and lowers the declaration binding proved
//!       for it through the ordinary expression, constructor and callable machinery.
//! WHY: header binding already resolved the selected target once. AST must not look the selected
//!      name up by text, reopen the path as a file value, or invent a new value operation; it
//!      consumes the authored tokens and the `(source, path row)` target fact.

use super::error::ExpressionParseError;
use super::expression_rpn::ExpressionRpnItem;
use super::expression_store::AstExpressionBuilder;
use super::parse_expression_identifiers::{
    lower_external_constant_expression, lower_external_function_expression,
    lower_resolved_declaration_expression,
};
use super::parse_expression_input::RouteArgumentContext;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::families::{
    choices_control_flow, names_declarations,
};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerStage, NameNamespace, TypeUsedAsValueReason,
};
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::binding_environment::{
    ResolvedQualifiedReference, ResolvedQualifiedTarget,
};
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Why a value-position path token is or is not an expression-qualified occurrence.
///
/// A recognised occurrence always carries a bound target by the time AST runs: binding resolves
/// every occurrence or rejects the module, so there is no unresolved third state to report here.
pub(crate) enum QualifiedPathStatus<'a> {
    /// No authored occurrence was recognised at this path row.
    FileValue,
    /// The occurrence is bound to exactly one selected declaration.
    Resolved(&'a ResolvedQualifiedReference),
}

/// Classify one value-position path row against the prepared qualified-reference facts.
pub(crate) fn qualified_path_status<'a>(
    context: &'a ScopeContext,
    path_syntax: PathSyntaxId,
) -> QualifiedPathStatus<'a> {
    let Some(lookups) = context.shared.lookups.as_ref() else {
        // Constant headers and generated bodies parse outside the completed lookup package. They
        // consume the same binding-owned target map through value services.
        return match context
            .shared
            .file_value_resolution
            .as_ref()
            .and_then(|services| {
                services
                    .qualified_reference_targets
                    .get(&(context.shared.declaring_file_id, path_syntax))
            }) {
            Some(resolved) => QualifiedPathStatus::Resolved(resolved),
            None => QualifiedPathStatus::FileValue,
        };
    };
    match lookups
        .binding_environment
        .qualified_reference_target(context.shared.declaring_file_id, path_syntax)
    {
        Some(resolved) => QualifiedPathStatus::Resolved(resolved),
        None => QualifiedPathStatus::FileValue,
    }
}

/// Consume and lower one qualified reference primary.
///
/// WHAT: verifies the recorded selected symbol follows the path row, consumes both tokens and
///       applies the ordinary declaration, constructor, call and suffix handling.
/// WHY: qualified references are one expression primary, not a file value followed by an
///      application. Returning `false` keeps an ordinary file-value path on its existing lane.
#[allow(
    clippy::too_many_arguments,
    reason = "qualified primaries keep the token stream, scope, interner, builders, value store, expression output, catch policy, and path fork as separate parser borrows"
)]
pub(crate) fn try_parse_qualified_reference_primary(
    path_syntax: PathSyntaxId,
    token_stream: &mut AstCursor<'_>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    expected_result_evidence_allowed: bool,
    route_argument_context: Option<RouteArgumentContext>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<bool, ExpressionParseError> {
    let resolved = match qualified_path_status(context, path_syntax) {
        QualifiedPathStatus::FileValue => return Ok(false),
        QualifiedPathStatus::Resolved(resolved) => resolved,
    };

    // The selected symbol is the immediately following token and must be the exact authored
    // symbol the preparation pass recorded. A mismatch means the occurrence fact and the token
    // stream disagree, which is a compiler invariant rather than user syntax.
    let selected_token = token_stream
        .token_ref_at_offset(1)
        .ok_or_else(|| missing_selected_symbol_failure(path_syntax))?;
    if selected_token.tag() != TokenTag::SYMBOL
        || selected_token.source_span() != resolved.selected_span
    {
        return Err(missing_selected_symbol_failure(path_syntax));
    }
    let selected_name = token_stream
        .token_string_id_at_in(token_stream.position().saturating_add(1), string_table)?
        .ok_or_else(|| missing_selected_symbol_failure(path_syntax))?;

    // Consume the authored path. The selected symbol remains current so shared identifier
    // lowering sees the same cursor position an ordinary name reference expects.
    token_stream.advance();

    match &resolved.target {
        // A source declaration and an imported declaration both live in the consumer's
        // top-level table; only the path that names the row differs.
        ResolvedQualifiedTarget::Source {
            declaration_path: path,
        }
        | ResolvedQualifiedTarget::Imported {
            local_path: path, ..
        } => {
            let declaration = context
                .shared
                .top_level_declarations
                .get_by_path(path)
                .ok_or_else(|| missing_target_declaration_failure(*path))?;
            lower_resolved_declaration_expression(
                selected_name,
                declaration,
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                expression,
                allow_boundary_catch,
                expected_result_evidence_allowed,
                route_argument_context,
                string_table,
                path_fork,
            )?;
        }
        ResolvedQualifiedTarget::External { symbol_id } => {
            lower_external_qualified_target(
                *symbol_id,
                selected_name,
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                expression,
                allow_boundary_catch,
                string_table,
                path_fork,
            )?;
        }
    }

    Ok(true)
}

/// Lower one external package symbol selected by a qualified reference.
///
/// WHAT: dispatches the registered symbol category to the same constant, call or type-as-value
///       diagnostic an authored external name produces.
/// WHY: a qualified external selection must not promote a type into a value or bypass the
///      registered call contract.
#[allow(
    clippy::too_many_arguments,
    reason = "external qualified lowering keeps the token stream, scope, builders, value store, expression output, catch policy, and path fork as separate parser borrows"
)]
fn lower_external_qualified_target(
    symbol_id: ExternalSymbolId,
    selected_name: StringId,
    token_stream: &mut AstCursor<'_>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    expression: &mut Vec<ExpressionRpnItem>,
    allow_boundary_catch: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<(), ExpressionParseError> {
    match symbol_id {
        ExternalSymbolId::Constant(constant_id) => {
            let definition = context
                .external_package_registry
                .get_constant_by_id(constant_id)
                .ok_or_else(missing_external_symbol_failure)?;
            lower_external_constant_expression(
                definition,
                selected_name,
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                expression,
                allow_boundary_catch,
                string_table,
                path_fork,
            )
        }
        ExternalSymbolId::Function(function_id) => {
            let definition = context
                .external_package_registry
                .get_function_by_id(function_id)
                .ok_or_else(missing_external_symbol_failure)?;
            lower_external_function_expression(
                function_id,
                definition,
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                expression,
                allow_boundary_catch,
                string_table,
                path_fork,
            )
        }
        ExternalSymbolId::Type(_) => {
            let span = Some(token_stream.current_span());
            if token_stream.peek_next_tag() == Some(TokenTag::OPEN_PARENTHESIS) {
                return Err(choices_control_flow::compile_time_evaluation_error(
                    CompileTimeEvaluationErrorReason::ExternalTypeConstructionNotSupported,
                    Some(selected_name),
                    span,
                )
                .into());
            }
            Err(names_declarations::type_used_as_value_row(
                selected_name,
                NameNamespace::Value,
                NameNamespace::Type,
                TypeUsedAsValueReason::BareTypeName,
                span,
            )
            .into())
        }
    }
}

fn missing_selected_symbol_failure(path_syntax: PathSyntaxId) -> ExpressionParseError {
    InfrastructureFailure::internal(
        CompilerStage::AstConstruction,
        format!(
            "qualified-reference path row {path_syntax:?} has no matching authored selected symbol token"
        ),
    )
    .into()
}

fn missing_target_declaration_failure(
    path: crate::compiler_frontend::symbols::path_interner::PathId,
) -> ExpressionParseError {
    InfrastructureFailure::internal(
        CompilerStage::AstConstruction,
        format!(
            "qualified-reference target declaration {:?} was not present in the AST declaration table",
            path
        ),
    )
    .into()
}

fn missing_external_symbol_failure() -> ExpressionParseError {
    InfrastructureFailure::internal(
        CompilerStage::AstConstruction,
        "qualified-reference external target was absent from the active package registry",
    )
    .into()
}
