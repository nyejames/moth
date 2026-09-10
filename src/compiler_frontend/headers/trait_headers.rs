//! Header-stage trait declaration and conformance syntax parsing.
//!
//! WHAT: owns parsing of trait declaration shells (`TRAIT must: ... ;`) and conformance shells
//!       (`Type must TRAIT` / `Type of Generic must TRAIT`) discovered during header parsing.
//! WHY: extracting these helpers keeps `header_dispatch.rs` focused on top-level declaration
//!      dispatch/orchestration, while trait-specific syntax rules live in one focused module.
//!      This module parses syntax shells only; semantic trait resolution and evidence validation
//!      are owned by AST.

use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidDeclarationReason, InvalidSignatureMemberReason,
};
use crate::compiler_frontend::declaration_syntax::signature_members::parse_trait_requirement_signature_syntax;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceSpan};
use crate::compiler_frontend::symbols::identifier_policy::is_uppercase_constant_name;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, Token, TokenKind};
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

// ------------------------
//  Trait declaration parsing
// ------------------------

pub(super) fn parse_trait_declaration(
    token_stream: &mut FileTokens,
    declaration_token: &Token,
    declaration_name: StringId,
    source_order: usize,
    context: &mut HeaderBuildContext<'_>,
    span_builder: &mut ExtendedSpanBuilder,
) -> TraitHeaderResult<TraitDeclarationSyntax> {
    let mut requirements = Vec::new();
    let trait_path = context.source_file.append(declaration_name);
    let name_span = SourceSpan::new(token_stream.file_id, declaration_token.span);

    token_stream.skip_newlines();

    loop {
        match token_stream.current_token_kind() {
            TokenKind::End => {
                token_stream.advance();
                break;
            }

            TokenKind::Eof => {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::unexpected_end_of_file(
                        Some(context.string_table.intern(";")),
                        Some(token_stream.current_span()),
                    ),
                ));
            }

            TokenKind::Newline => {
                token_stream.skip_newlines();
            }

            _ => {
                let requirement =
                    parse_trait_requirement(token_stream, &trait_path, context, span_builder)?;
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
    token_stream: &mut FileTokens,
    trait_path: &InternedPath,
    context: &mut HeaderBuildContext<'_>,
    span_builder: &mut ExtendedSpanBuilder,
) -> TraitHeaderResult<TraitRequirementSyntax> {
    let name_span = token_stream.current_span();

    let TokenKind::Symbol(method_name) = token_stream.current_token_kind() else {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::unexpected_token_in_declaration(Some(token_stream.current_span())),
        ));
    };
    let method_name = *method_name;
    token_stream.advance();

    if token_stream.current_token_kind() != &TokenKind::TypeParameterBracket {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::unexpected_token_in_declaration(Some(token_stream.current_span())),
        ));
    }

    // Requirement-member identifiers are retained declaration syntax, so they must live below
    // the same source-owned prefix as every other header path. The trait and method suffixes
    // keep members from distinct requirements distinct without inventing a parallel namespace.
    let method_path = trait_path.append(method_name);

    let signature = parse_trait_requirement_signature_syntax(
        token_stream,
        context.warnings,
        context.string_table,
        &method_path,
        span_builder,
    )?;

    // Every non-empty requirement must start with `This` or `~This`.
    if let Some(first_param) = signature.parameters.first() {
        let param_name = first_param
            .id
            .name()
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
    token_stream: &mut FileTokens,
    target: ConformanceTargetSyntax,
    context: &mut HeaderBuildContext<'_>,
) -> TraitHeaderResult<TraitConformanceSyntax> {
    let mut traits = Vec::new();

    loop {
        match token_stream.current_token_kind() {
            TokenKind::Symbol(trait_name) => {
                let trait_span = token_stream.current_span();
                ensure_trait_name_is_all_caps(*trait_name, Some(trait_span), context.string_table)?;

                traits.push(TraitReferenceSyntax {
                    name: *trait_name,
                    span: trait_span,
                });
                token_stream.advance();
            }

            _ => {
                if traits.is_empty() {
                    return Err(HeaderParseFailure::Diagnostic(
                        CompilerDiagnostic::invalid_declaration(
                            InvalidDeclarationReason::TraitConformanceMissingTrait,
                            Some(target.name),
                            Some(token_stream.current_span()),
                        ),
                    ));
                }

                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::unexpected_token_in_declaration(Some(
                        token_stream.current_span(),
                    )),
                ));
            }
        }

        match token_stream.current_token_kind() {
            TokenKind::Comma => {
                let comma_span = Some(token_stream.current_span());
                token_stream.advance();
                token_stream.skip_newlines();

                // A comma may continue across newlines, but it must still be followed by a trait.
                if matches!(
                    token_stream.current_token_kind(),
                    TokenKind::End | TokenKind::Eof
                ) {
                    return Err(HeaderParseFailure::Diagnostic(
                        CompilerDiagnostic::unexpected_trailing_comma(comma_span),
                    ));
                }
            }

            TokenKind::Newline | TokenKind::Eof => {
                break;
            }

            TokenKind::End => {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::invalid_declaration(
                        InvalidDeclarationReason::TraitConformanceSemicolon,
                        Some(target.name),
                        Some(token_stream.current_span()),
                    ),
                ));
            }

            _ => {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::unexpected_token_in_declaration(Some(
                        token_stream.current_span(),
                    )),
                ));
            }
        }
    }

    Ok(TraitConformanceSyntax { target, traits })
}

pub(super) fn parse_specialized_conformance_target(
    token_stream: &mut FileTokens,
    target_name: StringId,
    target_token: &Token,
) -> TraitHeaderResult<ConformanceTargetSyntax> {
    token_stream.advance(); // past `of`

    loop {
        match token_stream.current_token_kind() {
            TokenKind::Must => {
                return Ok(ConformanceTargetSyntax {
                    name: target_name,
                    kind: ConformanceTargetKind::SpecializedGenericInstance,
                    span: SourceSpan::new(token_stream.file_id, target_token.span),
                });
            }

            TokenKind::Newline | TokenKind::End | TokenKind::Eof => {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::unexpected_token_in_declaration(Some(
                        token_stream.current_span(),
                    )),
                ));
            }

            _ => token_stream.advance(),
        }
    }
}

pub(super) fn parse_trait_incompatibility(
    token_stream: &mut FileTokens,
    subject: TraitReferenceSyntax,
    source_order: usize,
    context: &mut HeaderBuildContext<'_>,
) -> TraitHeaderResult<TraitIncompatibilitySyntax> {
    let mut incompatible_traits = Vec::new();

    loop {
        match token_stream.current_token_kind() {
            TokenKind::Symbol(trait_name) => {
                let trait_span = token_stream.current_span();
                ensure_trait_name_is_all_caps(*trait_name, Some(trait_span), context.string_table)?;

                incompatible_traits.push(TraitReferenceSyntax {
                    name: *trait_name,
                    span: trait_span,
                });
                token_stream.advance();
            }

            _ => {
                if incompatible_traits.is_empty() {
                    return Err(HeaderParseFailure::Diagnostic(
                        CompilerDiagnostic::invalid_declaration(
                            InvalidDeclarationReason::TraitIncompatibilityMissingTrait,
                            Some(subject.name),
                            Some(token_stream.current_span()),
                        ),
                    ));
                }

                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::unexpected_token_in_declaration(Some(
                        token_stream.current_span(),
                    )),
                ));
            }
        }

        match token_stream.current_token_kind() {
            TokenKind::Comma => {
                let comma_span = Some(token_stream.current_span());
                token_stream.advance();
                token_stream.skip_newlines();

                if matches!(
                    token_stream.current_token_kind(),
                    TokenKind::End | TokenKind::Eof
                ) {
                    return Err(HeaderParseFailure::Diagnostic(
                        CompilerDiagnostic::unexpected_trailing_comma(comma_span),
                    ));
                }
            }

            TokenKind::Newline | TokenKind::Eof => {
                break;
            }

            TokenKind::End => {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::invalid_declaration(
                        InvalidDeclarationReason::TraitIncompatibilitySemicolon,
                        Some(subject.name),
                        Some(token_stream.current_span()),
                    ),
                ));
            }

            _ => {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::unexpected_token_in_declaration(Some(
                        token_stream.current_span(),
                    )),
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
    target_path: &InternedPath,
    span: SourceSpan,
    string_table: &mut StringTable,
) -> InternedPath {
    target_path.join_str(
        &format!("__trait_conformance_{:?}", span.local()),
        string_table,
    )
}

pub(super) fn incompatibility_header_path(
    subject_path: &InternedPath,
    span: SourceSpan,
    string_table: &mut StringTable,
) -> InternedPath {
    subject_path.join_str(
        &format!("__trait_incompatibility_{:?}", span.local()),
        string_table,
    )
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
