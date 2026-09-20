//! Generic parameter-list parsing shared by top-level declaration headers.
//!
//! WHAT: parses `type T, U` after a declaration name into `GenericParameterList`.
//! WHY: functions, structs, and choices share exactly one generic-parameter syntax and
//! should not grow parallel validation paths as generics expand.

use super::DeclarationCursor;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticToken, InvalidDeclarationReason, InvalidGenericParameterReason,
};
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameter, GenericParameterList, GenericParameterScope, GenericTraitBound,
    TypeParameterId,
};
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::identifier_policy::is_uppercase_constant_name;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use rustc_hash::FxHashSet;

/// Typed failure result for generic-parameter parsing.
///
/// WHAT: keeps list parsing, trait-bound parsing and trait-name validation on
///       one error boundary that preserves source diagnostics and infrastructure failures.
/// WHY: these connected helpers otherwise carry the large diagnostic value
///      through every successful header parse while losing the outer failure lane.
type GenericParameterParseResult<T> = Result<T, HeaderParseFailure>;

fn current_diagnostic_token(
    token_stream: &DeclarationCursor<'_>,
    string_table: &mut StringTable,
) -> GenericParameterParseResult<Option<DiagnosticToken>> {
    token_stream
        .current_diagnostic_token(string_table)
        .map_err(|error| {
            HeaderParseFailure::Infrastructure(CompilerDiagnostic::token_view_invariant_error(
                error,
                "generic-parameter diagnostic projection",
            ))
        })
}

/// Parse a generic parameter list after the current `type` keyword.
///
/// The parser stops with the token stream positioned on the declaration delimiter
/// (`|`, `=`, `::`, or `as`) so the owning header parser can continue normally.
pub(crate) fn parse_generic_parameter_list_after_type_keyword(
    token_stream: &mut DeclarationCursor<'_>,
    forbidden_names: &FxHashSet<StringId>,
    string_table: &mut StringTable,
) -> GenericParameterParseResult<GenericParameterList> {
    if token_stream.current_tag() != TokenTag::TYPE {
        return Err(CompilerError::new(
            "Generic parameter parser was called when the current token was not `type`.",
            None,
            ErrorType::Compiler,
        )
        .into());
    }

    let type_keyword_span = current_source_span(token_stream);
    token_stream.advance();

    let mut parameters = Vec::new();
    let mut expecting_parameter = true;

    loop {
        match token_stream.current_tag() {
            TokenTag::SYMBOL if expecting_parameter => {
                let Some(name) = token_stream
                    .current_string_id_in(string_table)
                    .map_err(HeaderParseFailure::Infrastructure)?
                else {
                    return Err(CompilerError::compiler_error(
                        "symbol token is missing its string payload",
                    )
                    .into());
                };
                let span = current_source_span(token_stream);
                parameters.push(GenericParameter {
                    id: TypeParameterId(parameters.len() as u32),
                    name,
                    span,
                    trait_bounds: Vec::new(),
                });
                token_stream.advance();
                expecting_parameter = false;
            }

            TokenTag::SYMBOL => {
                let span = current_source_span(token_stream);
                let Some(found) = current_diagnostic_token(token_stream, string_table)? else {
                    return Err(with_token_span(
                        span,
                        CompilerDiagnostic::unexpected_end_of_file(None, span),
                    )
                    .into());
                };
                return Err(with_token_span(
                    span,
                    CompilerDiagnostic::unexpected_token_from_tag(found, span),
                )
                .into());
            }

            TokenTag::COMMA => {
                if expecting_parameter {
                    let span = current_source_span(token_stream);
                    let Some(found) = current_diagnostic_token(token_stream, string_table)? else {
                        return Err(with_token_span(
                            span,
                            CompilerDiagnostic::unexpected_end_of_file(None, span),
                        )
                        .into());
                    };
                    return Err(with_token_span(
                        span,
                        CompilerDiagnostic::unexpected_token_from_tag(found, span),
                    )
                    .into());
                }

                token_stream.advance();
                expecting_parameter = true;
            }

            TokenTag::IS if !expecting_parameter => {
                token_stream.advance();
                parse_trait_bounds_for_current_parameter(
                    token_stream,
                    &mut parameters,
                    string_table,
                )?;
            }

            TokenTag::TYPE_PARAMETER_BRACKET
            | TokenTag::ASSIGN
            | TokenTag::DOUBLE_COLON
            | TokenTag::AS => {
                if parameters.is_empty() {
                    return Err(CompilerDiagnostic::invalid_generic_parameter(
                        InvalidGenericParameterReason::EmptyParameterList,
                        type_keyword_span,
                    )
                    .into());
                }

                if expecting_parameter {
                    let span = current_source_span(token_stream);
                    let Some(found) = current_diagnostic_token(token_stream, string_table)? else {
                        return Err(with_token_span(
                            span,
                            CompilerDiagnostic::unexpected_end_of_file(None, span),
                        )
                        .into());
                    };
                    return Err(with_token_span(
                        span,
                        CompilerDiagnostic::unexpected_token_from_tag(found, span),
                    )
                    .into());
                }

                let parameter_list = GenericParameterList { parameters };
                GenericParameterScope::from_parameter_list(
                    &parameter_list,
                    None,
                    forbidden_names,
                    string_table,
                    "Header Parsing",
                )
                .map_err(|diagnostic| {
                    with_parameter_span(token_stream, &parameter_list, diagnostic)
                })?;
                return Ok(parameter_list);
            }

            TokenTag::MUST => {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::invalid_generic_parameter(
                        InvalidGenericParameterReason::BoundsMustUseIs,
                        current_source_span(token_stream),
                    ),
                )
                .into());
            }

            TokenTag::COLON => {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::invalid_generic_parameter(
                        InvalidGenericParameterReason::BoundsMustUseIs,
                        current_source_span(token_stream),
                    ),
                )
                .into());
            }

            TokenTag::NEWLINE => {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::invalid_generic_parameter(
                        InvalidGenericParameterReason::ListMustStayWithHeader,
                        current_source_span(token_stream),
                    ),
                )
                .into());
            }

            TokenTag::EOF | TokenTag::END => {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::unexpected_end_of_file(
                        None,
                        current_source_span(token_stream),
                    ),
                )
                .into());
            }

            _other => {
                let span = current_source_span(token_stream);
                let found =
                    current_diagnostic_token(token_stream, string_table)?.ok_or_else(|| {
                        CompilerError::compiler_error(
                            "generic-parameter diagnostic requested without a current token",
                        )
                    })?;
                return Err(with_token_span(
                    span,
                    CompilerDiagnostic::invalid_generic_parameter(
                        InvalidGenericParameterReason::InvalidToken { found },
                        span,
                    ),
                )
                .into());
            }
        }
    }
}

fn parse_trait_bounds_for_current_parameter(
    token_stream: &mut DeclarationCursor<'_>,
    parameters: &mut [GenericParameter],
    string_table: &mut StringTable,
) -> GenericParameterParseResult<()> {
    let Some(parameter) = parameters.last_mut() else {
        let span = current_source_span(token_stream);
        let Some(found) = current_diagnostic_token(token_stream, string_table)? else {
            return Err(with_token_span(
                span,
                CompilerDiagnostic::unexpected_end_of_file(None, span),
            )
            .into());
        };
        return Err(with_token_span(
            span,
            CompilerDiagnostic::unexpected_token_from_tag(found, span),
        )
        .into());
    };

    let mut expecting_trait_name = true;

    loop {
        match token_stream.current_tag() {
            TokenTag::SYMBOL if expecting_trait_name => {
                let Some(trait_name) = token_stream
                    .current_string_id_in(string_table)
                    .map_err(HeaderParseFailure::Infrastructure)?
                else {
                    return Err(CompilerError::compiler_error(
                        "symbol token is missing its string payload",
                    )
                    .into());
                };
                let span = current_source_span(token_stream);
                if let Err(diagnostic) =
                    ensure_trait_bound_name_is_all_caps(trait_name, span, string_table)
                {
                    return Err(with_token_span(span, diagnostic).into());
                }

                parameter
                    .trait_bounds
                    .push(GenericTraitBound { trait_name, span });
                token_stream.advance();
                expecting_trait_name = false;
            }

            TokenTag::AND if !expecting_trait_name => {
                token_stream.advance();
                expecting_trait_name = true;
            }

            TokenTag::COMMA
            | TokenTag::TYPE_PARAMETER_BRACKET
            | TokenTag::ASSIGN
            | TokenTag::DOUBLE_COLON
            | TokenTag::AS
                if !expecting_trait_name =>
            {
                return Ok(());
            }

            TokenTag::MUST => {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::invalid_generic_parameter(
                        InvalidGenericParameterReason::BoundsMustUseIs,
                        current_source_span(token_stream),
                    ),
                )
                .into());
            }

            TokenTag::EOF | TokenTag::END => {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::unexpected_end_of_file(
                        None,
                        current_source_span(token_stream),
                    ),
                )
                .into());
            }
            _other => {
                let span = current_source_span(token_stream);
                let found =
                    current_diagnostic_token(token_stream, string_table)?.ok_or_else(|| {
                        CompilerError::compiler_error(
                            "generic-parameter diagnostic requested without a current token",
                        )
                    })?;
                return Err(with_token_span(
                    span,
                    CompilerDiagnostic::invalid_generic_parameter(
                        InvalidGenericParameterReason::InvalidToken { found },
                        span,
                    ),
                )
                .into());
            }
        }
    }
}
fn ensure_trait_bound_name_is_all_caps(
    trait_name: StringId,
    span: Option<SourceSpan>,
    string_table: &StringTable,
) -> Result<(), CompilerDiagnostic> {
    if is_uppercase_constant_name(string_table.resolve(trait_name)) {
        return Ok(());
    }

    Err(CompilerDiagnostic::invalid_declaration(
        InvalidDeclarationReason::InvalidTraitName,
        Some(trait_name),
        span,
    ))
}

fn with_current_token_span(
    token_stream: &DeclarationCursor<'_>,
    diagnostic: CompilerDiagnostic,
) -> CompilerDiagnostic {
    with_token_span(current_source_span(token_stream), diagnostic)
}

fn with_token_span(
    span: Option<SourceSpan>,
    mut diagnostic: CompilerDiagnostic,
) -> CompilerDiagnostic {
    if diagnostic.primary_span.is_none() {
        diagnostic.primary_span = span;
    }
    diagnostic
}

fn with_parameter_span(
    _token_stream: &DeclarationCursor<'_>,
    _parameter_list: &GenericParameterList,
    diagnostic: CompilerDiagnostic,
) -> CompilerDiagnostic {
    diagnostic
}

fn current_source_span(token_stream: &DeclarationCursor<'_>) -> Option<SourceSpan> {
    token_stream.current_span()
}
