//! Generic parameter-list parsing shared by top-level declaration headers.
//!
//! WHAT: parses `type T, U` after a declaration name into `GenericParameterList`.
//! WHY: functions, structs, and choices share exactly one generic-parameter syntax and
//! should not grow parallel validation paths as generics expand.

use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidDeclarationReason, InvalidGenericParameterReason,
};
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameter, GenericParameterList, GenericParameterScope, GenericTraitBound,
    TypeParameterId,
};
use crate::compiler_frontend::source::{LocalSpan, SourceSpan};
use crate::compiler_frontend::symbols::identifier_policy::is_uppercase_constant_name;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};
use rustc_hash::FxHashSet;

/// Boxed diagnostic result for generic-parameter parsing.
///
/// WHAT: keeps list parsing, trait-bound parsing and trait-name validation on
///       one small error boundary while preserving structured diagnostics.
/// WHY: these connected helpers otherwise carry the large diagnostic value
///      through every successful header parse. The header owner unboxes once.
type GenericParameterParseResult<T> = Result<T, Box<CompilerDiagnostic>>;

/// Parse a generic parameter list after the current `type` keyword.
///
/// The parser stops with the token stream positioned on the declaration delimiter
/// (`|`, `=`, `::`, or `as`) so the owning header parser can continue normally.
pub(crate) fn parse_generic_parameter_list_after_type_keyword(
    token_stream: &mut FileTokens,
    forbidden_names: &FxHashSet<StringId>,
    string_table: &StringTable,
) -> GenericParameterParseResult<GenericParameterList> {
    if token_stream.current_token_kind() != &TokenKind::Type {
        return Err(Box::new(
            CompilerError::new(
                "Generic parameter parser was called when the current token was not `type`.",
                token_stream.current_location(),
                ErrorType::Compiler,
            )
            .into(),
        ));
    }

    let type_keyword_location = token_stream.current_location();
    let type_keyword_span = token_stream.current_token().span;
    token_stream.advance();

    let mut parameters = Vec::new();
    let mut expecting_parameter = true;

    loop {
        match token_stream.current_token_kind().to_owned() {
            TokenKind::Symbol(name) if expecting_parameter => {
                let span = token_stream.tokens[token_stream.index].span;
                parameters.push(GenericParameter {
                    id: TypeParameterId(parameters.len() as u32),
                    name,
                    location: token_stream.current_location(),
                    span,
                    trait_bounds: Vec::new(),
                });
                token_stream.advance();
                expecting_parameter = false;
            }

            TokenKind::Symbol(_) => {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::unexpected_token(
                        token_stream.current_token_kind().to_owned(),
                        token_stream.current_location(),
                    ),
                )
                .into());
            }

            TokenKind::Comma => {
                if expecting_parameter {
                    return Err(with_current_token_span(
                        token_stream,
                        CompilerDiagnostic::unexpected_token(
                            token_stream.current_token_kind().to_owned(),
                            token_stream.current_location(),
                        ),
                    )
                    .into());
                }

                token_stream.advance();
                expecting_parameter = true;
            }

            TokenKind::Is if !expecting_parameter => {
                token_stream.advance();
                parse_trait_bounds_for_current_parameter(
                    token_stream,
                    &mut parameters,
                    string_table,
                )?;
            }

            TokenKind::TypeParameterBracket
            | TokenKind::Assign
            | TokenKind::DoubleColon
            | TokenKind::As => {
                if parameters.is_empty() {
                    return Err(with_token_span(
                        token_stream,
                        type_keyword_span,
                        CompilerDiagnostic::invalid_generic_parameter(
                            InvalidGenericParameterReason::EmptyParameterList,
                            type_keyword_location,
                        ),
                    )
                    .into());
                }

                if expecting_parameter {
                    return Err(with_current_token_span(
                        token_stream,
                        CompilerDiagnostic::unexpected_token(
                            token_stream.current_token_kind().to_owned(),
                            token_stream.current_location(),
                        ),
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

            TokenKind::Must => {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::invalid_generic_parameter(
                        InvalidGenericParameterReason::BoundsMustUseIs,
                        token_stream.current_location(),
                    ),
                )
                .into());
            }

            TokenKind::Colon => {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::invalid_generic_parameter(
                        InvalidGenericParameterReason::BoundsMustUseIs,
                        token_stream.current_location(),
                    ),
                )
                .into());
            }

            TokenKind::Newline => {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::invalid_generic_parameter(
                        InvalidGenericParameterReason::ListMustStayWithHeader,
                        token_stream.current_location(),
                    ),
                )
                .into());
            }

            TokenKind::Eof | TokenKind::End => {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::unexpected_end_of_file(
                        None,
                        token_stream.current_location(),
                    ),
                )
                .into());
            }

            other => {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::invalid_generic_parameter(
                        InvalidGenericParameterReason::InvalidToken { found: other },
                        token_stream.current_location(),
                    ),
                )
                .into());
            }
        }
    }
}

fn parse_trait_bounds_for_current_parameter(
    token_stream: &mut FileTokens,
    parameters: &mut [GenericParameter],
    string_table: &StringTable,
) -> GenericParameterParseResult<()> {
    let Some(parameter) = parameters.last_mut() else {
        return Err(with_current_token_span(
            token_stream,
            CompilerDiagnostic::unexpected_token(
                token_stream.current_token_kind().to_owned(),
                token_stream.current_location(),
            ),
        )
        .into());
    };

    let mut expecting_trait_name = true;

    loop {
        match token_stream.current_token_kind().to_owned() {
            TokenKind::Symbol(trait_name) if expecting_trait_name => {
                let span = token_stream.tokens[token_stream.index].span;
                if let Err(diagnostic) = ensure_trait_bound_name_is_all_caps(
                    trait_name,
                    token_stream.current_location(),
                    string_table,
                ) {
                    return Err(with_token_span(token_stream, span, *diagnostic).into());
                }

                parameter.trait_bounds.push(GenericTraitBound {
                    trait_name,
                    location: token_stream.current_location(),
                    span,
                });
                token_stream.advance();
                expecting_trait_name = false;
            }

            TokenKind::And if !expecting_trait_name => {
                token_stream.advance();
                expecting_trait_name = true;
            }

            TokenKind::Comma
            | TokenKind::TypeParameterBracket
            | TokenKind::Assign
            | TokenKind::DoubleColon
            | TokenKind::As
                if !expecting_trait_name =>
            {
                return Ok(());
            }

            TokenKind::Must => {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::invalid_generic_parameter(
                        InvalidGenericParameterReason::BoundsMustUseIs,
                        token_stream.current_location(),
                    ),
                )
                .into());
            }

            TokenKind::Eof | TokenKind::End => {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::unexpected_end_of_file(
                        None,
                        token_stream.current_location(),
                    ),
                )
                .into());
            }

            other => {
                return Err(with_current_token_span(
                    token_stream,
                    CompilerDiagnostic::invalid_generic_parameter(
                        InvalidGenericParameterReason::InvalidToken { found: other },
                        token_stream.current_location(),
                    ),
                )
                .into());
            }
        }
    }
}

fn ensure_trait_bound_name_is_all_caps(
    trait_name: StringId,
    location: SourceLocation,
    string_table: &StringTable,
) -> GenericParameterParseResult<()> {
    if is_uppercase_constant_name(string_table.resolve(trait_name)) {
        return Ok(());
    }

    Err(CompilerDiagnostic::invalid_declaration(
        InvalidDeclarationReason::InvalidTraitName,
        Some(trait_name),
        location,
    )
    .into())
}

/// Attach the authored token range while retaining the legacy location bridge.
fn with_current_token_span(
    token_stream: &FileTokens,
    diagnostic: CompilerDiagnostic,
) -> CompilerDiagnostic {
    with_token_span(token_stream, token_stream.current_token().span, diagnostic)
}

fn with_token_span(
    token_stream: &FileTokens,
    span: LocalSpan,
    mut diagnostic: CompilerDiagnostic,
) -> CompilerDiagnostic {
    if diagnostic.primary_span.is_none()
        && let Some(source) = token_stream.file_id
    {
        diagnostic.primary_span = Some(SourceSpan::new(source, span));
    }
    diagnostic
}

fn with_parameter_span(
    token_stream: &FileTokens,
    parameter_list: &GenericParameterList,
    diagnostic: Box<CompilerDiagnostic>,
) -> Box<CompilerDiagnostic> {
    let Some(parameter) = parameter_list
        .parameters
        .iter()
        .find(|parameter| parameter.location == diagnostic.primary_location)
    else {
        return diagnostic;
    };

    Box::new(with_token_span(token_stream, parameter.span, *diagnostic))
}
