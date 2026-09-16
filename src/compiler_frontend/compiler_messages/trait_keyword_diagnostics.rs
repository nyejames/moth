//! Reserved trait syntax helpers for the frontend.
//!
//! WHAT: centralized diagnostics for `must` and `This` in contexts where trait syntax is not valid
//!       or where the token is used outside a valid trait declaration.
//! WHY: multiple parser stages need to reject the same reserved keywords with typed diagnostics
//! while keeping parser-dispatch mismatches on the internal compiler-error path.
//!
//! NOTE: full trait declaration and conformance parsing lives in `headers/trait_headers.rs`.
//! This module is retained only for the diagnostic helpers used by parser paths that must reject
//! trait-only keywords outside valid trait syntax.

use crate::compiler_frontend::compiler_messages::compiler_errors::{
    CompilerError, CompilerErrorMetadataKey, ErrorType,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTraitKeywordUsageReason,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::tokenizer::tokens::{TokenKind, TokenTag};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReservedTraitKeyword {
    Must,
    This,
}

impl ReservedTraitKeyword {
    fn invalid_usage_reason(self) -> InvalidTraitKeywordUsageReason {
        match self {
            ReservedTraitKeyword::Must => InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax,
            ReservedTraitKeyword::This => InvalidTraitKeywordUsageReason::ThisOutsideTraitSyntax,
        }
    }
}

pub(crate) fn reserved_trait_keyword(token_kind: &TokenKind) -> Option<ReservedTraitKeyword> {
    reserved_trait_keyword_for_tag(token_kind.token_tag())
}

/// Tag-based reserved trait keyword lookup for source-view header callers.
///
/// WHAT: maps `must`/`This` stable tags without cloning a legacy token.
/// WHY: the outer file walk classifies through `TokenRef`/`TokenTag` while later
///      declaration parsers still use the `TokenKind` entry point.
pub(crate) fn reserved_trait_keyword_for_tag(tag: TokenTag) -> Option<ReservedTraitKeyword> {
    if tag == TokenTag::MUST {
        Some(ReservedTraitKeyword::Must)
    } else if tag == TokenTag::TRAIT_THIS {
        Some(ReservedTraitKeyword::This)
    } else {
        None
    }
}

/// Resolves a reserved trait keyword in contexts that already dispatched on reserved tokens.
///
/// WHAT: converts `must` / `This` token kinds into their reserved-keyword enum variant.
/// WHY: parser dispatch drift should return a structured internal compiler diagnostic instead of
/// relying on nearby `expect(...)` assumptions.
pub(crate) fn reserved_trait_keyword_or_dispatch_mismatch(
    token_kind: &TokenKind,
    span: Option<SourceSpan>,
    compilation_stage: &'static str,
    parser_context: &'static str,
) -> Result<ReservedTraitKeyword, CompilerError> {
    reserved_trait_keyword(token_kind).ok_or_else(|| {
        reserved_trait_dispatch_mismatch_error(token_kind, span, compilation_stage, parser_context)
    })
}
/// Tag-based reserved-keyword resolution for callsites holding a canonical tag.
///
/// WHAT: converts an already-classified `must`/`This` tag into its reserved
///       variant without cloning `TokenKind`.
/// WHY: source-view header walks dispatch on `TokenTag`; drift still returns
///      the structured internal compiler diagnostic with the same message.
pub(crate) fn reserved_trait_keyword_or_dispatch_mismatch_for_tag(
    tag: TokenTag,
    span: Option<SourceSpan>,
    compilation_stage: &'static str,
    parser_context: &'static str,
) -> Result<ReservedTraitKeyword, CompilerError> {
    reserved_trait_keyword_for_tag(tag).ok_or_else(|| {
        reserved_trait_dispatch_mismatch_error_for_tag(tag, span, compilation_stage, parser_context)
    })
}

/// Tag-based dispatch-mismatch error with the legacy message shape.
pub(crate) fn reserved_trait_dispatch_mismatch_error_for_tag(
    tag: TokenTag,
    span: Option<SourceSpan>,
    compilation_stage: &'static str,
    parser_context: &'static str,
) -> CompilerError {
    let mut metadata = HashMap::new();
    metadata.insert(
        CompilerErrorMetadataKey::CompilationStage,
        compilation_stage.to_owned(),
    );
    metadata.insert(
        CompilerErrorMetadataKey::PrimarySuggestion,
        String::from("This indicates parser dispatch drift. Please report this compiler bug."),
    );

    let mut error = CompilerError::new(
        format!(
            "Reserved trait token dispatch mismatch in {parser_context}: {}",
            tag.descriptor().text()
        ),
        span,
        ErrorType::Compiler,
    );
    error.metadata = metadata;
    error
}


pub(crate) fn reserved_trait_keyword_error(
    keyword: ReservedTraitKeyword,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_trait_keyword_usage(keyword.invalid_usage_reason(), span)
}

pub(crate) fn reserved_trait_dispatch_mismatch_error(
    token_kind: &TokenKind,
    span: Option<SourceSpan>,
    compilation_stage: &'static str,
    parser_context: &'static str,
) -> CompilerError {
    let mut metadata = HashMap::new();
    metadata.insert(
        CompilerErrorMetadataKey::CompilationStage,
        compilation_stage.to_owned(),
    );
    metadata.insert(
        CompilerErrorMetadataKey::PrimarySuggestion,
        String::from("This indicates parser dispatch drift. Please report this compiler bug."),
    );

    let mut error = CompilerError::new(
        format!("Reserved trait token dispatch mismatch in {parser_context}: {token_kind:?}"),
        span,
        ErrorType::Compiler,
    );
    error.metadata = metadata;
    error
}
