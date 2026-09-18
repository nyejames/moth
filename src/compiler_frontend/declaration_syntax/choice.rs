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

use super::DeclarationCursor;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::compiler_messages::DeferredFeatureReason;
use crate::compiler_frontend::compiler_messages::DiagnosticBag;
use crate::compiler_frontend::compiler_messages::InvalidChoiceVariantReason;
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_or_dispatch_mismatch_for_tag,
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
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
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

    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        self.payload.remap_path_ids(remap);
    }

    pub fn validate_required_source_prefixes(
        &self,
        provisional_source_file: PathId,
        path_fork: &PathInternerFork,
    ) -> Result<(), CompilerError> {
        self.payload
            .validate_required_source_prefixes(provisional_source_file, path_fork)
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

    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        if let Self::Record { fields } = self {
            for field in fields {
                field.remap_path_ids(remap);
            }
        }
    }

    pub fn validate_required_source_prefixes(
        &self,
        provisional_source_file: PathId,
        path_fork: &PathInternerFork,
    ) -> Result<(), CompilerError> {
        match self {
            Self::Unit => Ok(()),
            Self::Record { fields } => {
                for field in fields {
                    field.validate_required_source_prefix(provisional_source_file, path_fork)?;
                }
                Ok(())
            }
        }
    }
}

pub(crate) fn starts_rejected_choice_payload_shorthand(tag: TokenTag) -> bool {
    matches!(
        tag,
        TokenTag::DATATYPE_INT
            | TokenTag::DATATYPE_FLOAT
            | TokenTag::DATATYPE_BOOL
            | TokenTag::DATATYPE_STRING
            | TokenTag::DATATYPE_CHAR
            | TokenTag::DATATYPE_NONE
            | TokenTag::OPEN_CURLY
            | TokenTag::MUTABLE
            | TokenTag::SYMBOL
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
    token_stream: &mut DeclarationCursor<'_>,
    choice_path: PathId,
    path_fork: &mut PathInternerFork,
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
        let current_span = current_source_span(token_stream);

        match token_stream.current_tag() {
            TokenTag::MUST | TokenTag::TRAIT_THIS => {
                let keyword = reserved_trait_keyword_or_dispatch_mismatch_for_tag(
                    token_stream.current_tag(),
                    current_span,
                    "Header Parsing",
                    "choice header payload parsing",
                )?;

                return Err(reserved_trait_keyword_error(keyword, current_span).into());
            }

            TokenTag::SYMBOL => {
                let Some(variant_name) = token_stream.current_string_id() else {
                    return Err(CompilerError::compiler_error(
                        "symbol token is missing its string payload",
                    )
                    .into());
                };
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

                // Determine payload form based on the next token.
                let payload = match token_stream.current_tag() {
                    TokenTag::TYPE_PARAMETER_BRACKET => {
                        // Record body: Variant | field Type, ... |
                        let fields = parse_record_body(
                            token_stream,
                            string_table,
                            warnings,
                            SignatureMemberContext::ChoicePayloadField,
                            choice_path,
                            path_fork,
                            span_builder,
                        )?;
                        if fields.is_empty() {
                            return Err(CompilerDiagnostic::invalid_choice_variant(
                                InvalidChoiceVariantReason::EmptyRecordBody,
                                None,
                                None,
                                vec![],
                                current_span,
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
                                path_fork.component(choice_path),
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

                    TokenTag::OPEN_PARENTHESIS => {
                        return Err(CompilerDiagnostic::invalid_choice_variant(
                            InvalidChoiceVariantReason::ConstructorStyleNotSupported,
                            None,
                            None,
                            vec![],
                            current_source_span(token_stream),
                        )
                        .into());
                    }

                    TokenTag::ASSIGN => {
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
                match token_stream.current_tag() {
                    TokenTag::COMMA => {
                        token_stream.advance();
                        continue;
                    }
                    TokenTag::END => {
                        token_stream.advance();
                        break;
                    }
                    TokenTag::ASSIGN => {
                        return Err(choice_variant_default_value_diagnostic(current_source_span(
                            token_stream,
                        ))
                        .into());
                    }
                    TokenTag::NEWLINE => {
                        token_stream.skip_newlines();
                        match token_stream.current_tag() {
                            TokenTag::COMMA => {
                                token_stream.advance();
                                continue;
                            }
                            TokenTag::END => {
                                token_stream.advance();
                                break;
                            }
                            TokenTag::MUST | TokenTag::TRAIT_THIS => {
                                let keyword = reserved_trait_keyword_or_dispatch_mismatch_for_tag(
                                    token_stream.current_tag(),
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
                            TokenTag::SYMBOL => {
                                continue;
                            }
                            TokenTag::TYPE_PARAMETER_BRACKET => {
                                return Err(CompilerDiagnostic::invalid_choice_variant(
                                    InvalidChoiceVariantReason::UnexpectedSeparator,
                                    None,
                                    None,
                                    vec![],
                                    current_source_span(token_stream),
                                )
                                .into());
                            }
                            TokenTag::ASSIGN => {
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
                    TokenTag::EOF => {
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
            TokenTag::TYPE_PARAMETER_BRACKET => {
                return Err(CompilerDiagnostic::invalid_choice_variant(
                    InvalidChoiceVariantReason::UnexpectedSeparator,
                    None,
                    None,
                    vec![],
                    current_span,
                )
                .into());
            }
            TokenTag::END => {
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
            TokenTag::EOF => {
                return Err(CompilerDiagnostic::unexpected_end_of_file(
                    Some(string_table.intern(";")),
                    current_span,
                )
                .into());
            }
            _ => {
                let span = current_source_span(token_stream);
                let Some(found) = token_stream.canonical_cursor().current() else {
                    return Err(CompilerDiagnostic::unexpected_end_of_file(None, span).into());
                };
                return Err(CompilerDiagnostic::unexpected_token_from_ref(found, span).into());
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

fn current_source_span(token_stream: &DeclarationCursor<'_>) -> Option<SourceSpan> {
    token_stream.current_span()
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
