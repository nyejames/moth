//! Choice declaration shell parsing.
//!
//! WHAT: defines the choice metadata types and the parser that produces them from
//! `Choice :: VariantA, VariantB, ...;` syntax.
//! WHY: the header stage stores choice shells (variants + payload types) for dependency sorting
//! and AST construction. Centralising these types here keeps the header stage free of direct AST
//! imports for the choice shell contract.
//!
//! Body-context choice expression parsing (`Choice::Variant` values) lives in
//! `ast/expressions/parse_expression_identifiers.rs` and is intentionally separate.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::DeferredFeatureReason;
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::InvalidChoiceVariantReason;
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword, reserved_trait_keyword_error,
    reserved_trait_keyword_or_dispatch_mismatch,
};
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::record_body::parse_record_body;
use crate::compiler_frontend::declaration_syntax::signature_members::{
    SignatureMemberContext, SignatureMemberSyntax,
};
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceSpan};
use crate::compiler_frontend::symbols::identifier_policy::{
    IdentifierNamingKind, ensure_not_keyword_shadow_identifier, naming_warning_for_identifier,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use rustc_hash::FxHashMap;

#[derive(Clone, Debug)]
pub struct ChoiceVariant {
    pub id: StringId,
    pub payload: ChoiceVariantPayload,
    /// Exact authored variant-name span, when this shell still has its source identity.
    ///
    /// Header/import/materialisation boundaries use `None` when no owning source exists.
    pub span: Option<SourceSpan>,
}

#[derive(Clone, Debug)]
pub enum ChoiceVariantPayload {
    /// Unit variant with no payload fields.
    Unit,
    /// Record payload: `Variant | field Type, ... |`.
    Record { fields: Vec<Declaration> },
}

#[derive(Clone, Debug)]
pub struct ChoiceVariantSyntax {
    pub id: StringId,
    pub payload: ChoiceVariantPayloadSyntax,
    pub span: Option<SourceSpan>,
}

#[derive(Clone, Debug)]
pub enum ChoiceVariantPayloadSyntax {
    /// Unit variant with no payload fields.
    Unit,
    /// Record payload shell: `Variant | field Type, ... |`.
    Record { fields: Vec<SignatureMemberSyntax> },
}

impl ChoiceVariantSyntax {
    /// Remap variant name and payload fields recursively.
    // Called by per-file frontend output remapping before module-wide dependency sorting.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.id = remap.get(self.id);
        self.payload.remap_string_ids(remap);
    }

    pub fn validate_required_source_prefixes(
        &self,
        provisional_source_file: &InternedPath,
    ) -> Result<(), CompilerError> {
        self.payload
            .validate_required_source_prefixes(provisional_source_file)
    }
}

impl ChoiceVariantPayloadSyntax {
    /// Remap declaration-shell fields in record payloads.
    // Called by per-file frontend output remapping before module-wide dependency sorting.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match self {
            ChoiceVariantPayloadSyntax::Unit => {}

            ChoiceVariantPayloadSyntax::Record { fields } => {
                for field in fields {
                    field.remap_string_ids(remap);
                }
            }
        }
    }

    pub fn validate_required_source_prefixes(
        &self,
        provisional_source_file: &InternedPath,
    ) -> Result<(), CompilerError> {
        match self {
            Self::Unit => Ok(()),
            Self::Record { fields } => {
                for field in fields {
                    field.validate_required_source_prefix(provisional_source_file)?;
                }
                Ok(())
            }
        }
    }
}

pub(crate) fn starts_rejected_choice_payload_shorthand(token: &TokenKind) -> bool {
    matches!(
        token,
        TokenKind::DatatypeInt
            | TokenKind::DatatypeFloat
            | TokenKind::DatatypeBool
            | TokenKind::DatatypeString
            | TokenKind::DatatypeChar
            | TokenKind::DatatypeNone
            | TokenKind::OpenCurly
            | TokenKind::Mutable
            | TokenKind::Symbol(_)
    )
}

/// Parse `Choice :: VariantA, VariantB, ...;` declarations.
///
/// WHAT: accepts unit variants and record-body payload variants.
///
/// - `Variant` alone => `ChoiceVariantPayload::Unit`.
/// - `Variant | field Type, ... |` => `ChoiceVariantPayload::Record`.
///
/// Rejects shorthand payloads (`Variant Type`), constructor-style declarations
/// (`Variant(...)`), and default values (`Variant = ...`).
pub(crate) fn parse_choice_shell(
    token_stream: &mut FileTokens,
    choice_path: &InternedPath,
    string_table: &mut StringTable,
    warnings: &mut Vec<CompilerDiagnostic>,
    span_builder: &mut ExtendedSpanBuilder,
) -> Result<Vec<ChoiceVariantSyntax>, HeaderParseFailure> {
    // Mutation: EOF diagnostic payloads intern delimiter symbols that are not present
    // in the source text.
    let mut variants = Vec::new();
    let mut seen_variants: FxHashMap<StringId, SourceSpan> = FxHashMap::default();
    let mut bag = DiagnosticBag::new();

    // Caller is positioned on the `::` token.
    token_stream.advance();

    loop {
        token_stream.skip_newlines();
        let current_span = token_stream
            .tokens
            .get(token_stream.index)
            .map(|token| SourceSpan::new(token_stream.file_id, token.span));
        let current_token = token_stream.current_token_kind().to_owned();

        match current_token {
            TokenKind::Must | TokenKind::TraitThis => {
                let keyword = reserved_trait_keyword_or_dispatch_mismatch(
                    token_stream.current_token_kind(),
                    current_span,
                    "Header Parsing",
                    "choice header payload parsing",
                )?;

                return Err(reserved_trait_keyword_error(keyword, current_span).into());
            }

            TokenKind::Symbol(variant_name) => {
                ensure_not_keyword_shadow_identifier(variant_name, current_span, string_table)?;

                // Make sure this is not a duplicate variant name.
                if let Some(first_span) = seen_variants.get(&variant_name) {
                    bag.push(CompilerDiagnostic::duplicate_declaration(
                        variant_name,
                        Some(*first_span),
                        current_span,
                    ));
                } else if let Some(variant_span) = current_span {
                    seen_variants.insert(variant_name, variant_span);
                }

                if let Some(warning) = naming_warning_for_identifier(
                    variant_name,
                    current_span,
                    IdentifierNamingKind::TypeLike,
                    string_table,
                ) {
                    warnings.push(warning);
                }

                // Advance past the variant name
                token_stream.advance();
                token_stream.skip_newlines();

                // The token immediately after a parsed variant decides whether this stays in
                // alpha-scope syntax or enters richer syntax.
                if let Some(keyword) = reserved_trait_keyword(token_stream.current_token_kind()) {
                    return Err(reserved_trait_keyword_error(
                        keyword,
                        current_source_span(token_stream),
                    )
                    .into());
                }

                // Determine payload form based on the next token.
                let payload = match token_stream.current_token_kind() {
                    TokenKind::TypeParameterBracket => {
                        // Record body: Variant | field Type, ... |
                        let fields = parse_record_body(
                            token_stream,
                            string_table,
                            warnings,
                            SignatureMemberContext::ChoicePayloadField,
                            choice_path,
                            span_builder,
                        )?;
                        if fields.is_empty() {
                            return Err(CompilerDiagnostic::invalid_choice_variant(
                                InvalidChoiceVariantReason::EmptyRecordBody,
                                None,
                                None,
                                vec![],
                                current_span.clone(),
                            )
                            .into());
                        }

                        // Reject direct non-generic recursive choice declarations.
                        // Generic recursion is diagnosed after generic applications are
                        // resolved, so `Tree of T` can receive the generic-specific message.
                        // Duplicate payload field names are rejected by the shared
                        // signature-member parser before this loop runs.
                        for field in &fields {
                            if contains_non_generic_choice_self_reference(
                                &field.type_annotation,
                                choice_path.name(),
                            ) {
                                return Err(CompilerDiagnostic::invalid_choice_variant(
                                    InvalidChoiceVariantReason::RecursiveDeclaration,
                                    None,
                                    None,
                                    vec![],
                                    field.span,
                                )
                                .into());
                            }
                        }

                        ChoiceVariantPayloadSyntax::Record { fields }
                    }

                    TokenKind::OpenParenthesis => {
                        return Err(CompilerDiagnostic::invalid_choice_variant(
                            InvalidChoiceVariantReason::ConstructorStyleNotSupported,
                            None,
                            None,
                            vec![],
                            current_source_span(token_stream),
                        )
                        .into());
                    }

                    TokenKind::Assign => {
                        return Err(choice_variant_default_value_diagnostic(current_source_span(
                            token_stream,
                        ))
                        .into());
                    }

                    token if starts_rejected_choice_payload_shorthand(token) => {
                        return Err(CompilerDiagnostic::invalid_choice_variant(
                            InvalidChoiceVariantReason::PayloadShorthandNotSupported,
                            None,
                            None,
                            vec![],
                            current_source_span(token_stream),
                        )
                        .into());
                    }

                    // Unit variant: comma, end, newline, or EOF follows.
                    _ => ChoiceVariantPayloadSyntax::Unit,
                };

                variants.push(ChoiceVariantSyntax {
                    id: variant_name,
                    payload,
                    span: current_span,
                });

                // Handle the separator after the variant (or after its record body).
                match token_stream.current_token_kind() {
                    TokenKind::Comma => {
                        token_stream.advance();
                        continue;
                    }
                    TokenKind::End => {
                        token_stream.advance();
                        break;
                    }
                    TokenKind::Assign => {
                        return Err(choice_variant_default_value_diagnostic(current_source_span(
                            token_stream,
                        ))
                        .into());
                    }
                    TokenKind::Newline => {
                        token_stream.skip_newlines();
                        match token_stream.current_token_kind() {
                            TokenKind::Comma => {
                                token_stream.advance();
                                continue;
                            }
                            TokenKind::End => {
                                token_stream.advance();
                                break;
                            }
                            TokenKind::Must | TokenKind::TraitThis => {
                                let keyword = reserved_trait_keyword_or_dispatch_mismatch(
                                    token_stream.current_token_kind(),
                                    current_source_span(token_stream),
                                    "Header Parsing",
                                    "choice header payload parsing",
                                )?;

                                return Err(reserved_trait_keyword_error(
                                    keyword,
                                    current_source_span(token_stream),
                                )
                                .into());
                            }
                            TokenKind::Symbol(_) => {
                                continue;
                            }
                            TokenKind::TypeParameterBracket => {
                                return Err(CompilerDiagnostic::invalid_choice_variant(
                                    InvalidChoiceVariantReason::UnexpectedSeparator,
                                    None,
                                    None,
                                    vec![],
                                    current_source_span(token_stream),
                                )
                                .into());
                            }
                            TokenKind::Assign => {
                                return Err(choice_variant_default_value_diagnostic(
                                    current_source_span(token_stream),
                                )
                                .into());
                            }
                            payload_token
                                if starts_rejected_choice_payload_shorthand(payload_token) =>
                            {
                                return Err(CompilerDiagnostic::invalid_choice_variant(
                                    InvalidChoiceVariantReason::PayloadShorthandNotSupported,
                                    None,
                                    None,
                                    vec![],
                                    current_source_span(token_stream),
                                )
                                .into());
                            }
                            _ => {
                                return Err(CompilerDiagnostic::invalid_choice_variant(
                                    InvalidChoiceVariantReason::UnexpectedSeparator,
                                    None,
                                    None,
                                    vec![],
                                    current_source_span(token_stream),
                                )
                                .into());
                            }
                        }
                    }
                    TokenKind::Eof => {
                        return Err(CompilerDiagnostic::unexpected_end_of_file(
                            Some(string_table.intern(";")),
                            current_source_span(token_stream),
                        )
                        .into());
                    }
                    _ => {
                        return Err(CompilerDiagnostic::invalid_choice_variant(
                            InvalidChoiceVariantReason::UnexpectedSeparator,
                            None,
                            None,
                            vec![],
                            current_source_span(token_stream),
                        )
                        .into());
                    }
                }
            }
            TokenKind::TypeParameterBracket => {
                return Err(CompilerDiagnostic::invalid_choice_variant(
                    InvalidChoiceVariantReason::UnexpectedSeparator,
                    None,
                    None,
                    vec![],
                    current_span,
                )
                .into());
            }
            TokenKind::End => {
                if variants.is_empty() {
                    return Err(CompilerDiagnostic::invalid_choice_variant(
                        InvalidChoiceVariantReason::MissingVariants,
                        None,
                        None,
                        vec![],
                        current_span,
                    )
                    .into());
                }

                token_stream.advance();
                break;
            }
            TokenKind::Eof => {
                return Err(CompilerDiagnostic::unexpected_end_of_file(
                    Some(string_table.intern(";")),
                    current_span,
                )
                .into());
            }
            _ => {
                return Err(
                    CompilerDiagnostic::unexpected_token(current_token, current_span).into(),
                );
            }
        }
    }

    if bag.has_errors() {
        let first = bag
            .into_diagnostics()
            .into_iter()
            .next()
            .expect("choice duplicate bag holds at least one diagnostic");
        return Err(HeaderParseFailure::Diagnostic(first));
    }

    Ok(variants)
}

fn choice_variant_default_value_diagnostic(span: Option<SourceSpan>) -> CompilerDiagnostic {
    CompilerDiagnostic::deferred_feature_reason(
        DeferredFeatureReason::ChoiceVariantDefaultValue,
        span,
    )
}

fn current_source_span(token_stream: &FileTokens) -> Option<SourceSpan> {
    token_stream
        .tokens
        .get(token_stream.index)
        .map(|token| SourceSpan::new(token_stream.file_id, token.span))
}

fn contains_non_generic_choice_self_reference(
    type_ref: &ParsedTypeRef,
    choice_name: Option<StringId>,
) -> bool {
    match type_ref {
        ParsedTypeRef::Named { name, .. } => Some(*name) == choice_name,
        ParsedTypeRef::Optional { inner, .. }
        | ParsedTypeRef::Collection { element: inner, .. } => {
            contains_non_generic_choice_self_reference(inner, choice_name)
        }
        ParsedTypeRef::Applied { arguments, .. } => arguments
            .iter()
            .any(|argument| contains_non_generic_choice_self_reference(argument, choice_name)),
        _ => false,
    }
}
