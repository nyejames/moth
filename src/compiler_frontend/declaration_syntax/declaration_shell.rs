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

use crate::compiler_frontend::compiler_messages::{
    CommonSyntaxMistakeReason, CompilerDiagnostic, InvalidDeclarationReason,
};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::declaration_syntax::build_config_contract::{
    BuildConfigQualifierSyntax, parse_build_config_qualifier, starts_build_config_qualifier,
};
use crate::compiler_frontend::declaration_syntax::type_syntax::{
    TypeAnnotationContext, parse_type_annotation,
};
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenKind};
use crate::compiler_frontend::utilities::token_scan::{
    TokenScanFailure, collect_declaration_initializer_tokens, collect_symbol_references,
};
use crate::compiler_frontend::value_mode::ValueMode;

pub use crate::compiler_frontend::utilities::token_scan::InitializerReference;

/// Two-lane result for declaration-shell parsing.
///
/// WHAT: keeps declaration parsing and binding-marker validation on one small
///       error boundary while preserving the original structured diagnostic, with
///       infrastructure failures aborting through the typed lane.
/// WHY: these connected helpers otherwise carry the large diagnostic value
///      through every successful parse. Plain-diagnostic callers unbox once at
///      their existing boundary.
type DeclarationShellResult<T> = Result<T, HeaderParseFailure>;
#[derive(Clone, Debug)]
pub struct DeclarationSyntax {
    pub binding_mode: BindingMode,
    pub type_annotation: ParsedTypeRef,
    /// Syntax-only `#Config of T` metadata retained for header consumers and AST resolution.
    pub(crate) config_qualifier: Option<BuildConfigQualifierSyntax>,
    pub initializer_tokens: Vec<Token>,
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

    /// Remap type names, initializer token payloads, and initializer references into a merged
    /// string table. Source-qualified spans are identity independent and are left unchanged.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.type_annotation.remap_string_ids(remap);
        if let Some(qualifier) = &mut self.config_qualifier {
            qualifier.remap_string_ids(remap);
        }
        for token in &mut self.initializer_tokens {
            token.remap_string_ids(remap);
        }
        for reference in &mut self.initializer_references {
            reference.remap_string_ids(remap);
        }
    }
}

pub fn parse_declaration_syntax(
    token_stream: &mut FileTokens,
    name: StringId,
    string_table: &mut StringTable,
    span_builder: &mut crate::compiler_frontend::source::ExtendedSpanBuilder,
) -> DeclarationShellResult<DeclarationSyntax> {
    // `#Config of T` is declaration-owned syntax, not an ordinary `#` binding followed by a
    // type named `Config`. Detect it before the generic target parser so all declaration stages
    // retain one qualifier representation.
    let target_span = current_source_span(token_stream);
    let config_qualifier = if starts_build_config_qualifier(token_stream, string_table) {
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
        // This checks for mutability marker first (in the case of mutable methods), or whether
        // the declaration has an explicit type.
        parse_binding_target_syntax(name, token_stream, string_table, span_builder)?
    };

    // A source `#Config` declaration may intentionally omit its initializer so a later
    // provider-independent resolution barrier can supply the required input. Ordinary constants
    // still require `= value`; the distinction is owned by the declaration qualifier itself.
    if config_qualifier.is_some()
        && matches!(
            token_stream.current_token_kind(),
            TokenKind::Comma | TokenKind::Eof | TokenKind::Newline
        )
    {
        return Ok(DeclarationSyntax {
            binding_mode: target.binding_mode,
            type_annotation: target.type_annotation,
            config_qualifier,
            initializer_tokens: Vec::new(),
            initializer_references: Vec::new(),
            span: target.span,
        });
    }

    // Require assignment for declarations.
    match token_stream.current_token_kind() {
        TokenKind::Assign => {
            token_stream.advance();
        }
        TokenKind::Comma | TokenKind::Eof | TokenKind::Newline => {
            return Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::missing_declaration_initializer(
                    name,
                    current_source_span(token_stream),
                ),
            ));
        }
        _ => {
            return Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::expected_token(
                    TokenKind::Assign,
                    Some(token_stream.current_token_kind().to_owned()),
                    current_source_span(token_stream),
                ),
            ));
        }
    }

    // Transitive mutation: the token scanner may intern EOF delimiters for diagnostics
    // when the initializer is unclosed at end-of-file.
    let mut initializer_tokens = collect_declaration_initializer_tokens(token_stream, string_table)
        .map_err(|failure| match failure {
            TokenScanFailure::Diagnostic(diagnostic) => HeaderParseFailure::Diagnostic(diagnostic),
            TokenScanFailure::Infrastructure(error) => HeaderParseFailure::Infrastructure(error),
        })?;
    if initializer_tokens.is_empty() {
        // The author wrote `=` but supplied no initializer expression. Point at the real
        // boundary after `=` (newline, end, EOF or comma) rather than the declaration name
        // or target type, so the diagnostic anchors where the initializer is missing.
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::invalid_declaration(
                InvalidDeclarationReason::MissingInitializerExpression,
                Some(name),
                current_source_span(token_stream),
            ),
        ));
    }

    // Retain the real boundary after an incomplete inline value-`if` tail. AST otherwise
    // appends a synthetic EOF at the declaration location, losing both multiline context
    // and the source location of an authored block close.
    if matches!(
        initializer_tokens.last().map(|token| &token.kind),
        Some(TokenKind::Then | TokenKind::Else)
    ) && matches!(
        token_stream.current_token_kind(),
        TokenKind::Newline | TokenKind::End | TokenKind::Eof | TokenKind::Comma
    ) {
        initializer_tokens.push(token_stream.current_token());
    }

    Ok(DeclarationSyntax {
        binding_mode: target.binding_mode,
        type_annotation: target.type_annotation,
        config_qualifier,
        initializer_references: collect_symbol_references(
            &initializer_tokens,
            token_stream.file_id,
        ),
        initializer_tokens,
        span: target.span,
    })
}

pub fn parse_binding_target_syntax(
    name: StringId,
    token_stream: &mut FileTokens,
    string_table: &StringTable,
    span_builder: &mut crate::compiler_frontend::source::ExtendedSpanBuilder,
) -> DeclarationShellResult<BindingTargetSyntax> {
    let target_span = current_source_span(token_stream);

    let binding_mode = if token_stream.current_token_kind() == &TokenKind::Mutable {
        require_binding_marker_adjacent(token_stream, BindingMode::MutableRuntime, span_builder)?;
        token_stream.advance();
        BindingMode::MutableRuntime
    } else if token_stream.current_token_kind() == &TokenKind::Hash {
        require_binding_marker_adjacent(
            token_stream,
            BindingMode::CompileTimeConstant,
            span_builder,
        )?;
        token_stream.advance();
        BindingMode::CompileTimeConstant
    } else if token_stream.current_token_kind() == &TokenKind::Reactive {
        require_binding_marker_adjacent(token_stream, BindingMode::ReactiveRuntime, span_builder)?;
        token_stream.advance();
        BindingMode::ReactiveRuntime
    } else {
        BindingMode::ImmutableRuntime
    };

    let type_annotation = parse_type_annotation(
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
//
// Returns an error when the marker is not adjacent to the next token, using the marker span as
// the diagnostic primary span.
pub(crate) fn require_binding_marker_adjacent(
    token_stream: &FileTokens,
    mode: BindingMode,
    span_builder: &mut crate::compiler_frontend::source::ExtendedSpanBuilder,
) -> DeclarationShellResult<()> {
    let Some(current_token) = token_stream.tokens.get(token_stream.index) else {
        return Ok(());
    };
    let Some(next_token) = token_stream.tokens.get(token_stream.index + 1) else {
        return Ok(());
    };

    let resolver = span_builder.resolver();
    let current_range = current_token.span.resolve_with(resolver);
    let next_range = next_token.span.resolve_with(resolver);
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
            CompilerDiagnostic::common_syntax_mistake(
                reason,
                Some(SourceSpan::new(token_stream.file_id, current_token.span)),
            ),
        ));
    }

    Ok(())
}

fn current_source_span(token_stream: &FileTokens) -> Option<SourceSpan> {
    token_stream
        .tokens
        .get(token_stream.index)
        .map(|token| SourceSpan::new(token_stream.file_id, token.span))
}

#[cfg(test)]
#[path = "tests/shell_remap_tests.rs"]
mod shell_remap_tests;

#[cfg(test)]
#[path = "tests/initializer_boundary_tests.rs"]
mod initializer_boundary_tests;
