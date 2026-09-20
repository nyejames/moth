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
use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, CompilerDiagnostic, DiagnosticToken, InvalidDeclarationReason,
};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::declaration_syntax::build_config_contract::{
    BuildConfigQualifierSyntax, parse_build_config_qualifier,
    starts_build_config_qualifier_at_cursor,
};
use crate::compiler_frontend::declaration_syntax::type_syntax::{
    TypeAnnotationContext, parse_type_annotation_cursor,
};
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
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
    /// Syntax-only `#Config of T` metadata retained for header consumers and AST resolution.
    pub(crate) config_qualifier: Option<BuildConfigQualifierSyntax>,
    /// Canonical source-owned initializer range. `None` means the declaration has no initializer.
    pub initializer_range: Option<TokenRange>,
    pub initializer_references: Vec<InitializerReference>,
    /// Exact source-qualified span of the declaration binding anchor.
    pub span: Option<SourceSpan>,
}

#[derive(Clone, Debug)]
pub struct BindingTargetSyntax {
    pub name: StringId,
    pub binding_mode: BindingMode,
    pub type_annotation: ParsedTypeRef,
    /// Exact source-qualified span of the binding target anchor.
    pub span: Option<SourceSpan>,
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
        if let Some(qualifier) = &mut self.config_qualifier {
            qualifier.remap_string_ids(remap);
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
) -> DeclarationShellResult<DeclarationSyntax> {
    let target_span = current_source_span(token_stream);
    let config_qualifier = if starts_build_config_qualifier_at_cursor(token_stream, string_table)
        .map_err(HeaderParseFailure::Infrastructure)?
    {
        Some(parse_build_config_qualifier(
            token_stream,
            string_table,
            Some(span_builder),
        )?)
    } else {
        None
    };

    let target = if let Some(qualifier) = &config_qualifier {
        BindingTargetSyntax {
            name,
            binding_mode: BindingMode::CompileTimeConstant,
            type_annotation: qualifier.type_annotation.clone(),
            span: qualifier.qualifier_span.or(target_span),
        }
    } else {
        parse_binding_target_syntax(name, token_stream, string_table, span_builder)?
    };

    // A source `#Config` declaration may omit its initializer so a later provider-independent
    // resolution barrier can supply the required input.
    if config_qualifier.is_some()
        && matches!(
            token_stream.current_tag(),
            TokenTag::COMMA | TokenTag::EOF | TokenTag::NEWLINE
        )
    {
        return Ok(DeclarationSyntax {
            binding_mode: target.binding_mode,
            type_annotation: target.type_annotation,
            config_qualifier,
            initializer_range: None,
            initializer_references: Vec::new(),
            span: target.span,
        });
    }

    match token_stream.current_tag() {
        TokenTag::ASSIGN => {
            token_stream.advance();
        }
        TokenTag::COMMA | TokenTag::EOF | TokenTag::NEWLINE => {
            return Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::missing_declaration_initializer(
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
                        CompilerDiagnostic::token_view_invariant_error(
                            error,
                            "declaration initializer diagnostic projection",
                        ),
                    )
                })?
                .or_else(|| Some(DiagnosticToken::from_static_tag(TokenTag::EOF)));
            return Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::expected_token_from_tags(TokenTag::ASSIGN, found, span),
            ));
        }
    }

    let (initializer_range, initializer_references) = {
        let canonical_cursor = token_stream.canonical_cursor_mut();
        let result = collect_declaration_initializer_range(canonical_cursor, string_table);
        token_stream.refresh();
        result.map_err(|failure| match failure {
            TokenScanFailure::Diagnostic(diagnostic) => HeaderParseFailure::Diagnostic(diagnostic),
            TokenScanFailure::Infrastructure(error) => HeaderParseFailure::Infrastructure(error),
        })?
    };

    if initializer_range.is_empty() {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::invalid_declaration(
                InvalidDeclarationReason::MissingInitializerExpression,
                Some(name),
                current_source_span(token_stream),
            ),
        ));
    }

    Ok(DeclarationSyntax {
        binding_mode: target.binding_mode,
        type_annotation: target.type_annotation,
        config_qualifier,
        initializer_range: Some(initializer_range),
        initializer_references,
        span: target.span,
    })
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
    } else if token_stream.current_tag() == TokenTag::REACTIVE {
        require_binding_marker_adjacent(token_stream, BindingMode::ReactiveRuntime, span_builder)?;
        token_stream.advance();
        BindingMode::ReactiveRuntime
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
            BindingMode::ReactiveRuntime => {
                CommonSyntaxMistakeReason::InvalidReactiveBindingSpacing
            }
            BindingMode::ImmutableRuntime => return Ok(()),
        };
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::common_syntax_mistake(reason, Some(current_token.source_span())),
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
