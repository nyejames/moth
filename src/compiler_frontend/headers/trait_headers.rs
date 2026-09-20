//! Header-stage trait declaration and conformance syntax parsing.
//!
//! WHAT: owns parsing of trait declaration shells (`TRAIT must: ... ;`) and conformance shells
//!       (`Type must TRAIT` / `Type of Generic must TRAIT`) discovered during header parsing.
//! WHY: extracting these helpers keeps `header_dispatch.rs` focused on top-level declaration
//!      dispatch/orchestration, while trait-specific syntax rules live in one focused module.
//!      This module parses syntax shells only; semantic trait resolution and evidence validation
//!      are owned by AST.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidDeclarationReason, InvalidSignatureMemberReason,
};
use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::declaration_syntax::signature_members::parse_trait_requirement_signature_syntax;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceSpan};
use crate::compiler_frontend::symbols::identifier_policy::is_uppercase_constant_name;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{TokenCursor, TokenRef, TokenTag};
use crate::compiler_frontend::traits::syntax::{
    ConformanceTargetKind, ConformanceTargetSyntax, TraitConformanceSyntax, TraitDeclarationSyntax,
    TraitIncompatibilitySyntax, TraitReferenceSyntax, TraitRequirementSyntax,
};

use crate::compiler_frontend::headers::types::{HeaderBuildContext, HeaderParseFailure};
/// Two-lane result for trait-header parsing.
///
/// WHAT: gives every trait declaration, requirement, conformance, specialized-target,
///      incompatibility and trait-name validation helper one small error boundary that keeps
///      authored-source diagnostics separate from internal compiler-state failures.
/// WHY: the signature-member boundary already carries both lanes, so trait-header
///      parsing propagates them directly across its delegation steps. The
///      header-dispatch boundary also uses two lanes, so callers stay in sync.
type TraitHeaderResult<T> = Result<T, HeaderParseFailure>;

fn cursor_span(cursor: &TokenCursor<'_>) -> Option<SourceSpan> {
    cursor.current().map(TokenRef::source_span)
}

fn skip_cursor_newlines(cursor: &mut TokenCursor<'_>) {
    while cursor
        .current()
        .is_some_and(|token| token.tag() == TokenTag::NEWLINE)
    {
        let _ = cursor.advance();
    }
}

fn checked_symbol_id(
    token: TokenRef<'_>,
    string_table: &StringTable,
) -> TraitHeaderResult<StringId> {
    let Some(name) = token.string_id() else {
        return Err(HeaderParseFailure::Infrastructure(
            CompilerError::compiler_error("canonical symbol token is missing its string payload"),
        ));
    };
    token
        .string_spelling(string_table)
        .map_err(|error| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
                "canonical symbol payload could not be read: {error:?}",
            )))
        })?
        .ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "canonical symbol token is missing its string payload",
            ))
        })?;
    Ok(name)
}

// ------------------------
//  Trait declaration parsing
// ------------------------

pub(super) fn parse_trait_declaration(
    cursor: &mut TokenCursor<'_>,
    declaration_span: SourceSpan,
    declaration_name: StringId,
    source_order: usize,
    context: &mut HeaderBuildContext<'_>,
    span_builder: &mut ExtendedSpanBuilder,
) -> TraitHeaderResult<TraitDeclarationSyntax> {
    let mut requirements = Vec::new();
    let name_span = declaration_span;
    let trait_path = context
        .path_fork
        .try_intern_child(context.source_file, declaration_name)
        .ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "path table exhausted while interning trait path",
            ))
        })?;

    skip_cursor_newlines(cursor);

    loop {
        match cursor.current().map(TokenRef::tag).unwrap_or(TokenTag::EOF) {
            TokenTag::END => {
                let _ = cursor.advance();
                break;
            }

            TokenTag::EOF => {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::unexpected_end_of_file(
                        Some(context.string_table.intern(";")),
                        cursor_span(cursor),
                    ),
                ));
            }

            TokenTag::NEWLINE => {
                skip_cursor_newlines(cursor);
            }

            _ => {
                let requirement =
                    parse_trait_requirement(cursor, trait_path, context, span_builder)?;
                requirements.push(requirement);
            }
        }
    }

    Ok(TraitDeclarationSyntax {
        name: declaration_name,
        name_span,
        source_order,
        requirements,
        span: name_span,
    })
}

fn parse_trait_requirement(
    cursor: &mut TokenCursor<'_>,
    trait_path: PathId,
    context: &mut HeaderBuildContext<'_>,
    span_builder: &mut ExtendedSpanBuilder,
) -> TraitHeaderResult<TraitRequirementSyntax> {
    let Some(first) = cursor.current() else {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::unexpected_token_in_declaration(None),
        ));
    };
    let name_span = first.source_span();
    if first.tag() != TokenTag::SYMBOL {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::unexpected_token_in_declaration(Some(name_span)),
        ));
    }
    let method_name = checked_symbol_id(first, context.string_table)?;
    let _ = cursor.advance();

    if cursor
        .current()
        .is_none_or(|token| token.tag() != TokenTag::TYPE_PARAMETER_BRACKET)
    {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::unexpected_token_in_declaration(cursor_span(cursor)),
        ));
    }

    // Requirement-member identifiers are retained declaration syntax, so they must live below
    // the same source-owned prefix as every other header path. The trait and method suffixes
    // keep members from distinct requirements distinct without inventing a parallel namespace.
    let method_path = context
        .path_fork
        .try_intern_child(trait_path, method_name)
        .ok_or_else(|| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                "path table exhausted while interning trait requirement path",
            ))
        })?;

    let mut declaration_cursor = DeclarationCursor::new(*cursor)?;
    let signature = parse_trait_requirement_signature_syntax(
        &mut declaration_cursor,
        context.warnings,
        context.string_table,
        method_path,
        context.path_fork,
        span_builder,
    )?;
    *cursor = declaration_cursor.canonical_cursor();

    // Every non-empty requirement must start with `This` or `~This`.
    if let Some(first_param) = signature.parameters.first() {
        let param_name = context
            .path_fork
            .component(first_param.id)
            .map(|id| context.string_table.resolve(id))
            .unwrap_or("");
        if param_name != "This" {
            return Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::invalid_signature_member(
                    InvalidSignatureMemberReason::TraitReceiverMustBeThis,
                    first_param.span,
                ),
            ));
        }
    } else {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::invalid_signature_member(
                InvalidSignatureMemberReason::TraitReceiverMustBeThis,
                Some(name_span),
            ),
        ));
    }

    Ok(TraitRequirementSyntax {
        name: method_name,
        name_span,
        signature,
        span: name_span,
    })
}

// ------------------------
//  Trait conformance parsing
// ------------------------

pub(super) fn parse_trait_conformance(
    cursor: &mut TokenCursor<'_>,
    target: ConformanceTargetSyntax,
    context: &mut HeaderBuildContext<'_>,
) -> TraitHeaderResult<TraitConformanceSyntax> {
    let mut traits = Vec::new();

    loop {
        let Some(current) = cursor.current() else {
            return Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::invalid_declaration(
                    InvalidDeclarationReason::TraitConformanceMissingTrait,
                    Some(target.name),
                    None,
                ),
            ));
        };
        if current.tag() != TokenTag::SYMBOL {
            if traits.is_empty() {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::invalid_declaration(
                        InvalidDeclarationReason::TraitConformanceMissingTrait,
                        Some(target.name),
                        Some(current.source_span()),
                    ),
                ));
            }
            return Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::unexpected_token_in_declaration(Some(current.source_span())),
            ));
        }
        let trait_name = checked_symbol_id(current, context.string_table)?;
        let trait_span = current.source_span();
        ensure_trait_name_is_all_caps(trait_name, Some(trait_span), context.string_table)?;

        traits.push(TraitReferenceSyntax {
            name: trait_name,
            span: trait_span,
        });
        let _ = cursor.advance();

        match cursor.current().map(TokenRef::tag).unwrap_or(TokenTag::EOF) {
            TokenTag::COMMA => {
                let comma_span = cursor_span(cursor);
                let _ = cursor.advance();
                skip_cursor_newlines(cursor);

                // A comma may continue across newlines, but it must still be followed by a trait.
                if cursor
                    .current()
                    .is_none_or(|token| matches!(token.tag(), TokenTag::END | TokenTag::EOF))
                {
                    return Err(HeaderParseFailure::Diagnostic(
                        CompilerDiagnostic::unexpected_trailing_comma(comma_span),
                    ));
                }
            }

            TokenTag::NEWLINE | TokenTag::EOF => break,

            TokenTag::END => {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::invalid_declaration(
                        InvalidDeclarationReason::TraitConformanceSemicolon,
                        Some(target.name),
                        cursor_span(cursor),
                    ),
                ));
            }

            _ => {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::unexpected_token_in_declaration(cursor_span(cursor)),
                ));
            }
        }
    }

    Ok(TraitConformanceSyntax { target, traits })
}

pub(super) fn parse_specialized_conformance_target(
    cursor: &mut TokenCursor<'_>,
    target_name: StringId,
    target_span: SourceSpan,
) -> TraitHeaderResult<ConformanceTargetSyntax> {
    let _ = cursor.advance(); // past `of`

    loop {
        match cursor.current().map(TokenRef::tag).unwrap_or(TokenTag::EOF) {
            TokenTag::MUST => {
                return Ok(ConformanceTargetSyntax {
                    name: target_name,
                    kind: ConformanceTargetKind::SpecializedGenericInstance,
                    span: target_span,
                });
            }

            TokenTag::NEWLINE | TokenTag::END | TokenTag::EOF => {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::unexpected_token_in_declaration(cursor_span(cursor)),
                ));
            }

            _ => {
                let _ = cursor.advance();
            }
        }
    }
}

pub(super) fn parse_trait_incompatibility(
    cursor: &mut TokenCursor<'_>,
    subject: TraitReferenceSyntax,
    source_order: usize,
    context: &mut HeaderBuildContext<'_>,
) -> TraitHeaderResult<TraitIncompatibilitySyntax> {
    let mut incompatible_traits = Vec::new();

    loop {
        let Some(current) = cursor.current() else {
            return Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::invalid_declaration(
                    InvalidDeclarationReason::TraitIncompatibilityMissingTrait,
                    Some(subject.name),
                    None,
                ),
            ));
        };
        if current.tag() != TokenTag::SYMBOL {
            if incompatible_traits.is_empty() {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::invalid_declaration(
                        InvalidDeclarationReason::TraitIncompatibilityMissingTrait,
                        Some(subject.name),
                        Some(current.source_span()),
                    ),
                ));
            }
            return Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::unexpected_token_in_declaration(Some(current.source_span())),
            ));
        }
        let trait_name = checked_symbol_id(current, context.string_table)?;
        let trait_span = current.source_span();
        ensure_trait_name_is_all_caps(trait_name, Some(trait_span), context.string_table)?;

        incompatible_traits.push(TraitReferenceSyntax {
            name: trait_name,
            span: trait_span,
        });
        let _ = cursor.advance();

        match cursor.current().map(TokenRef::tag).unwrap_or(TokenTag::EOF) {
            TokenTag::COMMA => {
                let comma_span = cursor_span(cursor);
                let _ = cursor.advance();
                skip_cursor_newlines(cursor);

                if cursor
                    .current()
                    .is_none_or(|token| matches!(token.tag(), TokenTag::END | TokenTag::EOF))
                {
                    return Err(HeaderParseFailure::Diagnostic(
                        CompilerDiagnostic::unexpected_trailing_comma(comma_span),
                    ));
                }
            }

            TokenTag::NEWLINE | TokenTag::EOF => break,

            TokenTag::END => {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::invalid_declaration(
                        InvalidDeclarationReason::TraitIncompatibilitySemicolon,
                        Some(subject.name),
                        cursor_span(cursor),
                    ),
                ));
            }

            _ => {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::unexpected_token_in_declaration(cursor_span(cursor)),
                ));
            }
        }
    }

    Ok(TraitIncompatibilitySyntax {
        subject,
        source_order,
        incompatible_traits,
    })
}

pub(super) fn conformance_header_path(
    target_path: PathId,
    span: SourceSpan,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<PathId, CompilerError> {
    let name = string_table.intern(&format!("__trait_conformance_{:?}", span.local()));
    path_fork
        .try_intern_child(target_path, name)
        .ok_or_else(|| {
            CompilerError::compiler_error("path table exhausted while interning conformance path")
        })
}

pub(super) fn incompatibility_header_path(
    subject_path: PathId,
    span: SourceSpan,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<PathId, CompilerError> {
    let name = string_table.intern(&format!("__trait_incompatibility_{:?}", span.local()));
    path_fork
        .try_intern_child(subject_path, name)
        .ok_or_else(|| {
            CompilerError::compiler_error(
                "path table exhausted while interning incompatibility path",
            )
        })
}

pub(super) fn ensure_trait_name_is_all_caps(
    trait_name: StringId,
    span: Option<SourceSpan>,
    string_table: &StringTable,
) -> TraitHeaderResult<()> {
    if is_uppercase_constant_name(string_table.resolve(trait_name)) {
        return Ok(());
    }

    Err(HeaderParseFailure::Diagnostic(
        CompilerDiagnostic::invalid_declaration(
            InvalidDeclarationReason::InvalidTraitName,
            Some(trait_name),
            span,
        ),
    ))
}
