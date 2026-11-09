//! Declaration shell parsing for constants and variables.
//!
//! WHAT: parses the structural components of a declaration (mutability marker, type annotation,
//! initializer token slice, and initializer reference hints) into `DeclarationSyntax` and
//! `BindingTargetSyntax` shells.
//! WHY: header parsing stores these shells so that dependency sorting can see initializer
//! references, while AST resolves the full expression semantics later.
//! Each shell retains the exact local span of its binding target anchor. The enclosing file
//! owns that span's source identity and extended table; AST forwards the anchor to initializer EOF.
//! MUST NOT: perform type checking, constant folding, or semantic validation.

use super::DeclarationCursor;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, DiagnosticToken, InvalidDeclarationReason,
};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::declaration_syntax::build_config_contract::BuildConfigContractSyntax;
use crate::compiler_frontend::declaration_syntax::type_syntax::{
    TypeAnnotationContext, parse_type_annotation_cursor, token_can_start_type_specification,
};
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::syntax_errors::expression_position::separator_equality_mistake;
use crate::compiler_frontend::tokenizer::tokens::{TokenRange, TokenTag};
use crate::compiler_frontend::utilities::token_scan::{
    TokenScanFailure, collect_declaration_initializer_range,
};
use crate::compiler_frontend::value_mode::ValueMode;

pub use crate::compiler_frontend::utilities::token_scan::InitializerReference;

/// Two-lane result for declaration-shell parsing.
///
/// WHAT: keeps declaration parsing and binding-marker validation on one small
///       error boundary while preserving the original structured diagnostic, with
///       infrastructure failures aborting through the typed lane.
/// WHY: these connected helpers otherwise carry the large diagnostic value
///      through every successful parse. Plain-diagnostic callers extract the
///      inline diagnostic lane at their existing boundary.
type DeclarationShellResult<T> = Result<T, HeaderParseFailure>;

#[derive(Clone, Debug)]
pub struct DeclarationSyntax {
    pub binding_mode: BindingMode,
    pub type_annotation: ParsedTypeRef,
    /// Contract syntax retained for an attached `$config` source modifier.
    pub(crate) config_contract: Option<BuildConfigContractSyntax>,
    /// Canonical source-owned initializer range. `None` means the declaration has no initializer.
    pub initializer_range: Option<TokenRange>,
    pub initializer_references: Vec<InitializerReference>,
    /// Exact source-qualified span of the declaration binding anchor.
    pub span: Option<SourceSpan>,
}

/// Whether a token can begin the tokens after a binding name.
///
/// WHAT: the authored starts `parse_binding_target_syntax` accepts: an inferred `=` or any token
///       that can begin an explicit type annotation (binding markers included).
/// WHY: statement dispatch must not speculate a declaration for ordinary expression statements.
///      Keeping the start set beside the binding grammar means the dispatch gate and the parser
///      cannot drift apart.
pub(crate) fn binding_target_can_start(tag: TokenTag) -> bool {
    matches!(tag, TokenTag::ASSIGN) || token_can_start_type_specification(tag)
}

#[derive(Clone, Debug)]
pub struct BindingTargetSyntax {
    pub name: StringId,
    pub binding_mode: BindingMode,
    pub type_annotation: ParsedTypeRef,
    /// Exact source-qualified span of the binding target anchor.
    pub span: Option<SourceSpan>,
}

impl BindingTargetSyntax {
    /// Whether the authored target began with an explicit marker or type annotation.
    ///
    /// WHAT: distinguishes the bare `name = value` shape from every other current binding-target
    ///       spelling.
    /// WHY: statement dispatch only needs to recognise declaration shapes after an existing name;
    ///      reading the canonical target keeps that decision beside the binding grammar.
    pub(crate) fn declares_explicitly(&self) -> bool {
        self.binding_mode != BindingMode::ImmutableRuntime
            || !matches!(self.type_annotation, ParsedTypeRef::Inferred)
    }
}

impl DeclarationSyntax {
    pub fn value_mode(&self) -> ValueMode {
        self.binding_mode.value_mode()
    }

    pub fn semantic_type(&self) -> ParsedTypeRef {
        self.type_annotation.clone()
    }

    /// Remap type names and initializer references into a merged string table.
    ///
    /// Source-qualified ranges are identity independent and therefore remain unchanged.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.type_annotation.remap_string_ids(remap);
        if let Some(contract) = &mut self.config_contract {
            contract.remap_string_ids(remap);
        }
        for reference in &mut self.initializer_references {
            reference.remap_string_ids(remap);
        }
    }
}

pub fn parse_declaration_syntax(
    token_stream: &mut DeclarationCursor<'_>,
    name: StringId,
    string_table: &mut StringTable,
    span_builder: &mut crate::compiler_frontend::source::ExtendedSpanBuilder,
    config_input_span: Option<SourceSpan>,
) -> DeclarationShellResult<DeclarationSyntax> {
    let target = parse_binding_target_syntax(name, token_stream, string_table, span_builder)?;
    // The marker lets its target omit an initializer. Only an explicitly typed compile-time
    // binding becomes a contract; any other marked shape is a wrong-target attachment that the
    // header walk reports after this shell is parsed.
    let config_contract_target = config_input_span.filter(|_| {
        target.binding_mode == BindingMode::CompileTimeConstant
            && !matches!(&target.type_annotation, ParsedTypeRef::Inferred)
    });

    let (initializer_range, initializer_references) = match token_stream.current_tag() {
        TokenTag::ASSIGN => {
            if let Some(diagnostic) = separator_equality_mistake(
                token_stream.current_token_ref(),
                token_stream.token_ref_at_offset(1),
                token_stream.token_ref_at_offset(2),
            ) {
                return Err(diagnostic.into());
            }
            token_stream.advance();
            let result = {
                let canonical_cursor = token_stream.canonical_cursor_mut();
                let result = collect_declaration_initializer_range(canonical_cursor, string_table);
                token_stream.refresh();
                result
            };
            let (range, references) = result.map_err(|failure| match failure {
                TokenScanFailure::Diagnostic(diagnostic) => {
                    HeaderParseFailure::Diagnostic(diagnostic)
                }
                TokenScanFailure::Infrastructure(error) => {
                    HeaderParseFailure::Infrastructure(error)
                }
            })?;
            if range.is_empty() {
                return Err(HeaderParseFailure::Diagnostic(
                    names_declarations::invalid_declaration(
                        InvalidDeclarationReason::MissingInitializerExpression,
                        Some(name),
                        current_source_span(token_stream),
                    ),
                ));
            }
            (Some(range), references)
        }
        // A `$config`-marked binding may omit its initializer: contract resolution supplies either
        // the external input or the declared absence.
        TokenTag::COMMA | TokenTag::EOF | TokenTag::NEWLINE if config_input_span.is_some() => {
            (None, Vec::new())
        }
        TokenTag::COMMA | TokenTag::EOF | TokenTag::NEWLINE => {
            return Err(HeaderParseFailure::Diagnostic(
                names_declarations::missing_declaration_initializer(
                    name,
                    current_source_span(token_stream),
                ),
            ));
        }
        _ => {
            let span = current_source_span(token_stream);
            let found = token_stream
                .current_diagnostic_token(string_table)
                .map_err(|error| {
                    HeaderParseFailure::Infrastructure(
                        InfrastructureFailure::malformed_token_projection(
                            "declaration initializer diagnostic projection",
                            error,
                        ),
                    )
                })?
                .or_else(|| Some(DiagnosticToken::from_static_tag(TokenTag::EOF)));
            return Err(HeaderParseFailure::Diagnostic(
                token_lexical::expected_token_from_projections(
                    crate::compiler_frontend::compiler_messages::DiagnosticToken::from_static_tag(
                        TokenTag::ASSIGN,
                    ),
                    found,
                    span,
                ),
            ));
        }
    };

    let config_contract = match config_contract_target {
        Some(marker_span) => Some(BuildConfigContractSyntax {
            // Absence of an initializer on an optional contract means absence; an authored `none`
            // initializer states the same fact explicitly.
            default_none: match initializer_range {
                None => matches!(&target.type_annotation, ParsedTypeRef::Optional { .. }),
                Some(range) => initializer_is_none_literal(token_stream, range)?,
            },
            type_annotation: target.type_annotation.clone(),
            marker_span,
        }),
        None => None,
    };

    Ok(DeclarationSyntax {
        binding_mode: target.binding_mode,
        type_annotation: target.type_annotation,
        config_contract,
        initializer_range,
        initializer_references,
        span: target.span,
    })
}

/// Whether one already-validated initializer range is exactly the `none` literal.
///
/// WHY: `$config` records absence as a contract fact, while the ordinary expression grammar owns
/// the `none` value itself. Reading the single token keeps that distinction without re-parsing the
/// initializer.
fn initializer_is_none_literal(
    token_stream: &DeclarationCursor<'_>,
    range: TokenRange,
) -> DeclarationShellResult<bool> {
    if range.len() != 1 {
        return Ok(false);
    }
    let source = token_stream.canonical_cursor().source_tokens();
    let mut cursor = source.cursor(range).map_err(|error| {
        HeaderParseFailure::Infrastructure(InfrastructureFailure::internal(
            crate::compiler_frontend::compiler_messages::CompilerStage::HeaderParsing,
            format!("config default range is outside its canonical source owner: {error:?}"),
        ))
    })?;
    Ok(cursor
        .advance()
        .is_some_and(|token| token.tag() == TokenTag::NONE_LITERAL))
}

pub fn parse_binding_target_syntax(
    name: StringId,
    token_stream: &mut DeclarationCursor<'_>,
    string_table: &mut StringTable,
    span_builder: &mut crate::compiler_frontend::source::ExtendedSpanBuilder,
) -> DeclarationShellResult<BindingTargetSyntax> {
    let target_span = current_source_span(token_stream);

    let binding_mode = if token_stream.current_tag() == TokenTag::MUTABLE {
        require_binding_marker_adjacent(token_stream, BindingMode::MutableRuntime, span_builder)?;
        token_stream.advance();
        BindingMode::MutableRuntime
    } else if token_stream.current_tag() == TokenTag::HASH {
        require_binding_marker_adjacent(
            token_stream,
            BindingMode::CompileTimeConstant,
            span_builder,
        )?;
        token_stream.advance();
        BindingMode::CompileTimeConstant
    } else {
        BindingMode::ImmutableRuntime
    };

    let type_annotation = parse_type_annotation_cursor(
        token_stream,
        TypeAnnotationContext::DeclarationTarget,
        string_table,
    )?;

    Ok(BindingTargetSyntax {
        name,
        binding_mode,
        type_annotation,
        span: target_span,
    })
}

// WHAT: checks that a binding-mode marker (`#` or `~`) is immediately adjacent to the token
// that follows it (`=` for inferred, or the first token of the explicit type annotation).
//
// WHY: the language requires `name #= value` and `name ~= value`, rejecting `name # = value`
// and `name ~ = value`. Exact local byte ranges make the check independent of line/column
// bookkeeping and UTF-8 character width.
pub(crate) fn require_binding_marker_adjacent(
    token_stream: &DeclarationCursor<'_>,
    mode: BindingMode,
    span_builder: &mut crate::compiler_frontend::source::ExtendedSpanBuilder,
) -> DeclarationShellResult<()> {
    let cursor = token_stream.canonical_cursor();
    let Some(current_token) = cursor.current() else {
        return Ok(());
    };
    let Some(next_token) = cursor.peek_next() else {
        return Ok(());
    };

    let resolver = span_builder.resolver();
    let current_range = current_token.span().resolve_with(resolver);
    let next_range = next_token.span().resolve_with(resolver);
    let adjacent = current_range.end() == next_range.start();

    if !adjacent {
        let reason = match mode {
            BindingMode::MutableRuntime => CommonSyntaxMistakeReason::InvalidMutableBindingSpacing,
            BindingMode::CompileTimeConstant => {
                CommonSyntaxMistakeReason::InvalidCompileTimeBindingSpacing
            }
            BindingMode::ImmutableRuntime => return Ok(()),
        };
        return Err(HeaderParseFailure::Diagnostic(
            token_lexical::common_syntax_mistake(reason, Some(current_token.source_span())),
        ));
    }

    Ok(())
}

fn current_source_span(token_stream: &DeclarationCursor<'_>) -> Option<SourceSpan> {
    token_stream.current_span()
}

#[cfg(test)]
#[path = "tests/shell_remap_tests.rs"]
mod shell_remap_tests;

#[cfg(test)]
#[path = "tests/initializer_boundary_tests.rs"]
mod initializer_boundary_tests;
