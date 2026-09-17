//! Header declaration dispatch.
//!
//! WHAT: classifies one top-level declaration after its leading symbol has been seen and builds the
//! concrete `HeaderKind` payload.
//! WHY: declaration-kind parsing is separate from per-file token walking and from dependency sorting.

use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidConfigReason, InvalidDeclarationReason,
};
use crate::compiler_frontend::datatypes::generic_parameters::GenericParameterList;
use crate::compiler_frontend::symbols::string_interning::StringId;

use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::declaration_syntax::build_config_contract::starts_build_config_qualifier_at_source;
use crate::compiler_frontend::declaration_syntax::choice::parse_choice_shell as parse_choice_header_payload;
use crate::compiler_frontend::declaration_syntax::declaration_shell::{
    DeclarationSyntax, parse_declaration_syntax,
};
use crate::compiler_frontend::declaration_syntax::generic_parameters::parse_generic_parameter_list_after_type_keyword;
use crate::compiler_frontend::declaration_syntax::signature_members::parse_function_signature_syntax;

use crate::compiler_frontend::declaration_syntax::r#struct::parse_struct_shell;
use crate::compiler_frontend::declaration_syntax::type_syntax::{
    ParsedNamedTypeReference, TypeAnnotationContext, collect_capacity_references_in_parsed_ref,
    for_each_named_type_in_parsed_ref, parse_type_annotation_cursor,
};

use super::trait_headers::{
    conformance_header_path, ensure_trait_name_is_all_caps, incompatibility_header_path,
    parse_specialized_conformance_target, parse_trait_conformance, parse_trait_declaration,
    parse_trait_incompatibility,
};
use crate::compiler_frontend::headers::HeaderParseFailure;
use crate::compiler_frontend::headers::ordering_hints::{
    collect_constant_type_hints, collect_named_type_ordering_hint,
};
use crate::compiler_frontend::headers::types::{
    Header, HeaderBuildContext, HeaderExportMode, HeaderKind, LocalDeclarationOrderingHint,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::identifier_policy::{
    IdentifierNamingKind, ensure_not_keyword_shadow_identifier, naming_warning_for_identifier,
};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{
    FileTokens, Token, TokenCursor, TokenIndex, TokenRange, TokenRef, TokenTag,
};
use crate::compiler_frontend::traits::syntax::{
    ConformanceTargetKind, ConformanceTargetSyntax, TraitReferenceSyntax,
};
use crate::compiler_frontend::utilities::token_scan::InitializerReference;
use rustc_hash::FxHashSet;
use std::collections::HashSet;

/// Two-lane result for header dispatch.
///
/// Typed failure result for header dispatch.
///
/// WHAT: gives declaration dispatch and its local helpers one error boundary that keeps
///       authored-source diagnostics separate from internal compiler-state failures.
/// WHY: delegated declaration parsers carry both lanes, so dispatch can propagate them directly
///      across each delegation step without converting either lane.
type HeaderDispatchResult<T> = Result<T, HeaderParseFailure>;
fn compatibility_cursor_span(token_stream: &FileTokens) -> Option<SourceSpan> {
    token_stream
        .tokens
        .get(token_stream.index)
        .map(|token| SourceSpan::new(token_stream.file_id, token.span))
}

fn source_token_at_index<'a>(
    token_stream: &'a FileTokens,
    index: usize,
    owner: &'static str,
) -> HeaderDispatchResult<TokenRef<'a>> {
    let canonical = token_stream.source_tokens().map_err(|_| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Header dispatch is missing its source token owner.",
            None,
        ))
    })?;
    if canonical.source() != token_stream.file_id {
        return Err(internal_header_dispatch_error(
            "Header dispatch source token owner does not match its file identity.",
            None,
        )
        .into());
    }
    let position = TokenIndex::try_from_index(index).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            owner,
            compatibility_cursor_span(token_stream),
        ))
    })?;
    canonical.token(position).map_err(|_| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            owner,
            compatibility_cursor_span(token_stream),
        ))
    })
}

fn source_tag_at_cursor(token_stream: &FileTokens) -> HeaderDispatchResult<TokenTag> {
    Ok(source_token_at_index(
        token_stream,
        token_stream.index,
        "Header dispatch token index exceeded its source owner.",
    )?
    .tag())
}

fn source_next_tag(token_stream: &FileTokens) -> HeaderDispatchResult<Option<TokenTag>> {
    let next_index = token_stream.index.checked_add(1).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Header dispatch follower index overflowed its source range.",
            compatibility_cursor_span(token_stream),
        ))
    })?;
    let canonical = token_stream.source_tokens().map_err(|_| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Header dispatch is missing its source token owner.",
            compatibility_cursor_span(token_stream),
        ))
    })?;
    if canonical.source() != token_stream.file_id {
        return Err(internal_header_dispatch_error(
            "Header dispatch source token owner does not match its file identity.",
            compatibility_cursor_span(token_stream),
        )
        .into());
    }
    if next_index >= canonical.len() {
        return Ok(None);
    }
    Ok(Some(
        source_token_at_index(
            token_stream,
            next_index,
            "Header dispatch follower index exceeded its source owner.",
        )?
        .tag(),
    ))
}

fn starts_build_config_qualifier_at_cursor(
    token_stream: &FileTokens,
    string_table: &StringTable,
) -> HeaderDispatchResult<bool> {
    let canonical = token_stream.source_tokens().map_err(|_| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Header dispatch is missing its source token owner.",
            compatibility_cursor_span(token_stream),
        ))
    })?;
    let index = TokenIndex::try_from_index(token_stream.index).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Header dispatch token index exceeded its checked domain.",
            compatibility_cursor_span(token_stream),
        ))
    })?;
    Ok(starts_build_config_qualifier_at_source(
        canonical,
        index,
        string_table,
    ))
}

// WHAT: classifies one top-level declaration by its leading token and builds the concrete header
// payload (kind + body source range + dependency set) that later AST passes consume.
//
// WHY: every declaration kind (function, struct, choice/union, constant) has a different leading
// token pattern. This function dispatches on that token and delegates to kind-specific helpers
// where they exist, or captures body source ranges directly for simpler cases.
//
// Dispatch summary:
//   `|`  (TypeParameterBracket)  → function signature + body range capture
//   `=`  (Assign)                → struct shell
//   `::` (DoubleColon)           → choice/union variants
//   `#`  (Hash)                  → compile-time constant binding
//   `must:` / `must TRAIT`       → trait declaration/conformance
pub(super) fn create_header(
    full_name: PathId,
    token_stream: &mut FileTokens,
    declaration_token: &Token,
    export_mode: HeaderExportMode,
    context: &mut HeaderBuildContext<'_>,
    span_builder: &mut ExtendedSpanBuilder,
) -> HeaderDispatchResult<Header> {
    let name_span = SourceSpan::new(token_stream.file_id, declaration_token.span);
    let declaration_order = token_stream.index;
    let Some(declaration_name) = context.path_fork.component(full_name) else {
        return Err(internal_header_dispatch_error(
            "Header declaration path is missing its declaration name.",
            Some(name_span),
        )
        .into());
    };
    // Conservative local declaration-ordering hints; binding and Stage 3 resolve them.
    let mut kind: HeaderKind = HeaderKind::StartFunction;
    let mut capacity_references: Vec<InitializerReference> = Vec::new();
    let mut local_ordering_hints: HashSet<LocalDeclarationOrderingHint> = HashSet::new();
    let generic_parameters = parse_optional_generic_parameters(token_stream, context)?;
    let current_tag = source_tag_at_cursor(token_stream)?;
    let next_tag = source_next_tag(token_stream)?;
    let mut body_range = empty_token_range(token_stream)?;

    if current_tag == TokenTag::OF {
        if !generic_parameters.is_empty() {
            return Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::invalid_declaration(
                    InvalidDeclarationReason::GenericTraitsUnsupported,
                    Some(declaration_name),
                    Some(name_span),
                ),
            ));
        }

        let target = parse_specialized_conformance_target(
            token_stream,
            declaration_name,
            declaration_token,
        )?;
        token_stream.advance(); // past must

        let conformance = parse_trait_conformance(token_stream, target, context)?;
        kind = HeaderKind::TraitConformance { conformance };

        let conformance_id = conformance_header_path(
            full_name,
            name_span,
            context.path_fork,
            context.string_table,
        )?;
        return Ok(Header {
            kind,
            file_role: context.file_role,
            export_mode,
            local_ordering_hints,
            name_span: Some(name_span),
            synthetic_content_payload: None,
            tokens: body_range,
            declaration_path: conformance_id,
            token_sequence: None,
            capacity_references,
        });
    }

    // Check for trait syntax after generic parameters and before token dispatch.
    // WHAT: `must:` begins a trait declaration; `must not` begins an incompatibility
    //      declaration; `must TRAIT` begins a conformance declaration.
    // WHY: trait declarations, conformances, and incompatibility metadata are top-level
    //      declarations that participate in header parsing; they replace the old
    //      reserved-trait rejection path.
    if current_tag == TokenTag::MUST {
        if !generic_parameters.is_empty() {
            return Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::invalid_declaration(
                    InvalidDeclarationReason::GenericTraitsUnsupported,
                    Some(declaration_name),
                    Some(name_span),
                ),
            ));
        }

        let peek = next_tag;

        if peek == Some(TokenTag::NOT) {
            ensure_trait_name_is_all_caps(declaration_name, Some(name_span), context.string_table)?;

            // Trait incompatibility declaration: `Name must not TRAIT, TRAIT`
            token_stream.advance(); // past must
            token_stream.advance(); // past not
            let subject = TraitReferenceSyntax {
                name: declaration_name,
                span: name_span,
            };
            let incompatibility =
                parse_trait_incompatibility(token_stream, subject, declaration_order, context)?;
            kind = HeaderKind::TraitIncompatibility { incompatibility };
        } else if peek == Some(TokenTag::COLON) {
            ensure_trait_name_is_all_caps(declaration_name, Some(name_span), context.string_table)?;

            // Trait declaration: `Name must: requirements ;`
            token_stream.advance(); // past must
            token_stream.advance(); // past :

            let declaration = parse_trait_declaration(
                token_stream,
                declaration_token,
                declaration_name,
                declaration_order,
                context,
                span_builder,
            )?;

            // Collect local declaration-ordering hints from requirement signatures.
            for requirement in &declaration.requirements {
                for param in &requirement.signature.parameters {
                    collect_type_ordering_hints(
                        &param.type_annotation,
                        &generic_parameters,
                        full_name,
                        context,
                        &mut local_ordering_hints,
                        &mut capacity_references,
                    )?;
                }
                for ret in &requirement.signature.returns {
                    collect_type_ordering_hints(
                        &ret.value.type_annotation,
                        &generic_parameters,
                        full_name,
                        context,
                        &mut local_ordering_hints,
                        &mut capacity_references,
                    )?;
                }
            }

            kind = HeaderKind::Trait { declaration };
        } else {
            // Conformance declaration: `Name must TRAIT, TRAIT`
            token_stream.advance(); // past must

            let conformance = parse_trait_conformance(
                token_stream,
                ConformanceTargetSyntax {
                    name: declaration_name,
                    kind: ConformanceTargetKind::Named,
                    span: name_span,
                },
                context,
            )?;

            kind = HeaderKind::TraitConformance { conformance };
        }

        let header_id = match &kind {
            HeaderKind::TraitConformance { .. } => conformance_header_path(
                full_name,
                name_span,
                context.path_fork,
                context.string_table,
            )?,
            HeaderKind::TraitIncompatibility { .. } => incompatibility_header_path(
                full_name,
                name_span,
                context.path_fork,
                context.string_table,
            )?,
            _ => full_name,
        };
        return Ok(Header {
            kind,
            file_role: context.file_role,
            export_mode,
            local_ordering_hints,
            name_span: Some(name_span),
            synthetic_content_payload: None,
            tokens: body_range,
            declaration_path: header_id,
            token_sequence: None,
            capacity_references,
        });
    }

    match current_tag {
        // Function declaration: `name |params| -> return_type : body ;`
        TokenTag::TYPE_PARAMETER_BRACKET => {
            ensure_not_keyword_shadow_identifier(
                declaration_name,
                Some(name_span),
                context.string_table,
            )?;
            emit_header_naming_warning(
                context.warnings,
                declaration_name,
                Some(name_span),
                IdentifierNamingKind::ValueLike,
                context.string_table,
            );
            let mut declaration_cursor =
                DeclarationCursor::new(token_stream.canonical_cursor_from_current()?)?;
            let signature = parse_function_signature_syntax(
                &mut declaration_cursor,
                context.warnings,
                context.string_table,
                full_name,
                context.path_fork,
                span_builder,
            )?;
            let next_index = token_stream
                .compatibility_index_for_cursor(declaration_cursor.canonical_cursor())?;
            token_stream.index = next_index;

            // Local declaration-ordering hints: parameter + return type references only.
            for param in &signature.parameters {
                collect_type_ordering_hints(
                    &param.type_annotation,
                    &generic_parameters,
                    full_name,
                    context,
                    &mut local_ordering_hints,
                    &mut capacity_references,
                )?;
            }

            for ret in &signature.returns {
                collect_type_ordering_hints(
                    &ret.value.type_annotation,
                    &generic_parameters,
                    full_name,
                    context,
                    &mut local_ordering_hints,
                    &mut capacity_references,
                )?;
            }

            body_range = capture_function_body_range(token_stream, context.string_table)?;

            kind = HeaderKind::Function {
                generic_parameters,
                signature,
            };
        }

        // `This` keyword outside trait declarations is invalid.
        TokenTag::TRAIT_THIS => {
            return Err(CompilerDiagnostic::invalid_this_usage(
                crate::compiler_frontend::compiler_messages::InvalidThisUsageReason::OutsideTraitDeclaration,
                Some(token_stream.current_span()),
            )
            .into());
        }

        // `=` only creates a declaration header for struct shells. Runtime top-level
        // `name = value` stays in the entry start body outside `config.moth`.
        TokenTag::ASSIGN => {
            if next_tag == Some(TokenTag::TYPE_PARAMETER_BRACKET) {
                ensure_not_keyword_shadow_identifier(
                    declaration_name,
                    Some(name_span),
                    context.string_table,
                )?;
                emit_header_naming_warning(
                    context.warnings,
                    declaration_name,
                    Some(name_span),
                    IdentifierNamingKind::TypeLike,
                    context.string_table,
                );
                token_stream.advance();
                let mut declaration_cursor =
                    DeclarationCursor::new(token_stream.canonical_cursor_from_current()?)?;
                let fields = parse_struct_shell(
                    &mut declaration_cursor,
                    context.string_table,
                    context.warnings,
                    full_name,
                    context.path_fork,
                    span_builder,
                )?;
                let next_index = token_stream
                    .compatibility_index_for_cursor(declaration_cursor.canonical_cursor())?;
                token_stream.index = next_index;

                // Collect strict type edges from field types only (no default-expression edges).
                for field in &fields {
                    collect_type_ordering_hints(
                        &field.type_annotation,
                        &generic_parameters,
                        full_name,
                        context,
                        &mut local_ordering_hints,
                        &mut capacity_references,
                    )?;
                }

                kind = HeaderKind::Struct {
                    generic_parameters,
                    fields,
                };
            }
        }

        // `#` (Hash): compile-time constant declaration `name #= value` or `name #Type = value`.
        TokenTag::HASH => {
            if !generic_parameters.is_empty()
                && starts_build_config_qualifier_at_cursor(token_stream, context.string_table)?
            {
                return Err(CompilerDiagnostic::invalid_config_reason(
                    Some(declaration_name),
                    InvalidConfigReason::ConfigQualifierInvalidPlacement,
                    Some(token_stream.current_span()),
                )
                .into());
            }
            ensure_not_keyword_shadow_identifier(
                declaration_name,
                Some(name_span),
                context.string_table,
            )?;
            emit_header_naming_warning(
                context.warnings,
                declaration_name,
                Some(name_span),
                IdentifierNamingKind::TopLevelConstant,
                context.string_table,
            );

            let constant_header = create_constant_header_payload(
                &full_name,
                token_stream,
                context,
                &mut local_ordering_hints,
                &mut capacity_references,
                span_builder,
            )?;

            kind = HeaderKind::Constant {
                declaration: constant_header,
            };
        }

        // `::` (DoubleColon): choice/union declaration `name :: VariantA | VariantB | ...`
        TokenTag::DOUBLE_COLON => {
            ensure_not_keyword_shadow_identifier(
                declaration_name,
                Some(name_span),
                context.string_table,
            )?;
            emit_header_naming_warning(
                context.warnings,
                declaration_name,
                Some(name_span),
                IdentifierNamingKind::TypeLike,
                context.string_table,
            );

            let mut declaration_cursor =
                DeclarationCursor::new(token_stream.canonical_cursor_from_current()?)?;
            let choice_header = parse_choice_header_payload(
                &mut declaration_cursor,
                full_name,
                context.path_fork,
                context.string_table,
                context.warnings,
                span_builder,
            )?;
            let next_index = token_stream
                .compatibility_index_for_cursor(declaration_cursor.canonical_cursor())?;
            token_stream.index = next_index;

            // Collect strict type edges from payload field types.
            for variant in &choice_header {
                if let crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayloadSyntax::Record {
                    fields,
                } = &variant.payload
                {
                    for field in fields {
                        collect_type_ordering_hints(
                            &field.type_annotation,
                            &generic_parameters,
                            full_name,
                            context,
                            &mut local_ordering_hints,
                            &mut capacity_references,
                        )?;
                    }
                }
            }

            kind = HeaderKind::Choice {
                generic_parameters,
                variants: choice_header,
            };
        }

        // `as`: type alias declaration `Name as Type`
        TokenTag::AS => {
            if !generic_parameters.is_empty() {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::invalid_declaration(
                        InvalidDeclarationReason::ParameterizedGenericTypeAlias,
                        Some(declaration_name),
                        Some(name_span),
                    ),
                ));
            }

            ensure_not_keyword_shadow_identifier(
                declaration_name,
                Some(name_span),
                context.string_table,
            )?;
            emit_header_naming_warning(
                context.warnings,
                declaration_name,
                Some(name_span),
                IdentifierNamingKind::TypeLike,
                context.string_table,
            );

            token_stream.advance();
            let mut declaration_cursor =
                DeclarationCursor::new(token_stream.canonical_cursor_from_current()?)?;
            let target = parse_type_annotation_cursor(
                &mut declaration_cursor,
                TypeAnnotationContext::TypeAliasTarget,
                context.string_table,
            )?;
            let next_index = token_stream
                .compatibility_index_for_cursor(declaration_cursor.canonical_cursor())?;
            token_stream.index = next_index;

            let mut selection_error = None;

            for_each_named_type_in_parsed_ref(&target, &mut |type_reference| {
                if selection_error.is_none() {
                    selection_error = collect_named_type_ordering_hint(
                        type_reference,
                        context.file_dependency_clauses,
                        context.dependency_selections,
                        context.source_file,
                        context.string_table,
                        context.path_fork,
                        &mut local_ordering_hints,
                    )
                    .err();
                }
            });
            if let Some(error) = selection_error {
                return Err(error.into());
            }
            collect_capacity_references_in_parsed_ref(&target, &mut capacity_references);

            kind = HeaderKind::TypeAlias { target };
        }

        _ => {}
    }

    let header_id = full_name;
    Ok(Header {
        kind,
        file_role: context.file_role,
        export_mode,
        local_ordering_hints,
        name_span: Some(name_span),
        synthetic_content_payload: None,
        tokens: body_range,
        declaration_path: header_id,
        token_sequence: None,
        capacity_references,
    })
}

fn emit_header_naming_warning(
    warnings: &mut Vec<CompilerDiagnostic>,
    identifier: StringId,
    span: Option<SourceSpan>,
    naming_kind: IdentifierNamingKind,
    string_table: &crate::compiler_frontend::symbols::string_interning::StringTable,
) {
    if let Some(warning) =
        naming_warning_for_identifier(identifier, span, naming_kind, string_table)
    {
        warnings.push(warning);
    }
}
fn parse_optional_generic_parameters(
    token_stream: &mut FileTokens,
    context: &mut HeaderBuildContext<'_>,
) -> HeaderDispatchResult<GenericParameterList> {
    if source_tag_at_cursor(token_stream)? != TokenTag::TYPE {
        return Ok(GenericParameterList::default());
    }

    // Dependency visibility is file-wide, so the complete retained clause set is not available
    // until the header parser has finished walking this file. File-level preparation validates
    // dependency-name collisions after all clauses and declaration shells are retained.
    let forbidden_names = FxHashSet::default();
    let mut declaration_cursor =
        DeclarationCursor::new(token_stream.canonical_cursor_from_current()?)?;
    let result = parse_generic_parameter_list_after_type_keyword(
        &mut declaration_cursor,
        &forbidden_names,
        context.string_table,
    )?;
    let next_index =
        token_stream.compatibility_index_for_cursor(declaration_cursor.canonical_cursor())?;
    token_stream.index = next_index;
    Ok(result)
}

fn collect_type_ordering_hints(
    type_ref: &crate::compiler_frontend::datatypes::parsed::ParsedTypeRef,
    generic_parameters: &GenericParameterList,
    current_header_path: PathId,
    context: &mut HeaderBuildContext<'_>,
    local_ordering_hints: &mut HashSet<LocalDeclarationOrderingHint>,
    capacity_references: &mut Vec<InitializerReference>,
) -> Result<(), CompilerError> {
    let mut selection_error = None;
    for_each_named_type_in_parsed_ref(type_ref, &mut |type_reference| {
        if selection_error.is_some() {
            return;
        }

        match type_reference {
            // Generic parameters are bare local names. A qualified path must remain intact even
            // when its terminal component happens to share a generic parameter's spelling.
            ParsedNamedTypeReference::Bare(type_name) => {
                if generic_parameters.contains_name(type_name)
                    || context.path_fork.component(current_header_path) == Some(type_name)
                {
                    return;
                }
            }
            ParsedNamedTypeReference::Qualified(_) => {}
        }

        selection_error = collect_named_type_ordering_hint(
            type_reference,
            context.file_dependency_clauses,
            context.dependency_selections,
            context.source_file,
            context.string_table,
            context.path_fork,
            local_ordering_hints,
        )
        .err();
    });
    if let Some(error) = selection_error {
        return Err(error);
    }
    collect_capacity_references_in_parsed_ref(type_ref, capacity_references);
    Ok(())
}

fn empty_token_range(token_stream: &FileTokens) -> HeaderDispatchResult<TokenRange> {
    let index = TokenIndex::try_from_index(token_stream.index).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Header token range exceeded the source token index space.",
            Some(token_stream.current_span()),
        ))
    })?;
    TokenRange::new(token_stream.file_id, index, index).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Empty header token range was invalid.",
            Some(token_stream.current_span()),
        ))
    })
}

/// Capture a function body directly from a canonical cursor.
///
/// WHAT: consumes the cursor that the declaration parser left on the first body token and returns
/// the source range from that token through its matching `;`, tracking scope depth so nested
/// scopes (inner `if`/`loop`/etc.) close correctly.
/// WHY: the canonical header path must retain ranges, not clone a compatibility token vector or
/// rescan source text, and the cursor is the sole source of parser position and EOF/span facts.
/// Only the structural range is captured here: AST owns body syntax diagnostics, declaration
/// ordering hints come from the signature, and Stage 0 records graph-active file references from
/// its own source-owned start-body ranges.
pub(super) fn capture_function_body_range_from_cursor(
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    string_table: &mut StringTable,
) -> HeaderDispatchResult<TokenRange> {
    let source = cursor.source_tokens();
    if source.source() != file_id {
        return Err(HeaderParseFailure::Infrastructure(
            internal_header_dispatch_error(
                "Function body cursor source owner does not match its header identity.",
                None,
            ),
        ));
    }

    let body_start = cursor.position();
    let mut scopes_opened: usize = 1;
    let mut scopes_closed: usize = 0;
    let mut body_end = None;

    // `parse_function_signature_syntax` stops on the first body token, so inspect the current
    // canonical token before moving the cursor. EOF is handled from its exact TokenRef span.
    while scopes_opened > scopes_closed {
        let current = cursor.advance().ok_or_else(|| {
            HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
                "Function body capture reached the source end before its closing delimiter.",
                None,
            ))
        })?;

        match current.tag() {
            TokenTag::END => {
                scopes_closed = scopes_closed.checked_add(1).ok_or_else(|| {
                    HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
                        "Function body closing-delimiter depth overflowed.",
                        Some(current.source_span()),
                    ))
                })?;
                if scopes_opened == scopes_closed {
                    body_end = Some(current.index());
                }
            }
            // Colons used in templates parse into a different token (`StartTemplateBody`), so
            // every ordinary colon still contributes one balanced source scope.
            TokenTag::COLON => {
                scopes_opened = scopes_opened.checked_add(1).ok_or_else(|| {
                    HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
                        "Function body opening-delimiter depth overflowed.",
                        Some(current.source_span()),
                    ))
                })?;
            }
            TokenTag::DOUBLE_COLON => {}
            TokenTag::EOF => {
                return Err(CompilerDiagnostic::unexpected_end_of_file(
                    Some(string_table.intern(";")),
                    Some(current.source_span()),
                )
                .into());
            }
            _ => {}
        }
    }

    let body_end = body_end.ok_or_else(|| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Function body capture did not identify its closing delimiter.",
            None,
        ))
    })?;
    TokenRange::try_new_for(source, body_start, body_end).map_err(|error| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
            "function body token range was invalid: {error:?}"
        )))
    })
}

/// Compatibility wrapper for deferred declaration/AST parser handoffs.
///
/// The wrapper borrows the canonical cursor and synchronises the old `FileTokens` index only
/// after the cursor-based capture completes. It does not create or retain another source owner.
fn capture_function_body_range(
    token_stream: &mut FileTokens,
    string_table: &mut StringTable,
) -> HeaderDispatchResult<TokenRange> {
    let mut cursor = token_stream
        .canonical_cursor_from_current()
        .map_err(HeaderParseFailure::Infrastructure)?;
    let range =
        capture_function_body_range_from_cursor(&mut cursor, token_stream.file_id, string_table)?;
    token_stream.index = token_stream
        .compatibility_index_for_cursor(cursor)
        .map_err(HeaderParseFailure::Infrastructure)?;
    Ok(range)
}

fn create_constant_header_payload(
    full_name: &PathId,
    token_stream: &mut FileTokens,
    context: &mut HeaderBuildContext<'_>,
    local_ordering_hints: &mut HashSet<LocalDeclarationOrderingHint>,
    capacity_references: &mut Vec<InitializerReference>,
    span_builder: &mut ExtendedSpanBuilder,
) -> HeaderDispatchResult<DeclarationSyntax> {
    let Some(declaration_name) = context.path_fork.component(*full_name) else {
        return Err(internal_header_dispatch_error(
            "Constant header path is missing its declaration name.",
            Some(token_stream.current_span()),
        )
        .into());
    };
    let mut declaration_cursor =
        DeclarationCursor::new(token_stream.canonical_cursor_from_current()?)?;
    let declaration_syntax = parse_declaration_syntax(
        &mut declaration_cursor,
        declaration_name,
        context.string_table,
        span_builder,
    )?;
    let next_index =
        token_stream.compatibility_index_for_cursor(declaration_cursor.canonical_cursor())?;
    token_stream.index = next_index;
    // A comma terminates anonymous-record fields, but it cannot terminate a top-level source
    // contract. Keep the shared declaration parser permissive for record fields and reject this
    // malformed source declaration at the header boundary.
    if declaration_syntax.initializer_range.is_none()
        && source_tag_at_cursor(token_stream)? == TokenTag::COMMA
        && let Some(qualifier) = &declaration_syntax.config_qualifier
    {
        return Err(CompilerDiagnostic::invalid_config_reason(
            Some(declaration_name),
            InvalidConfigReason::ConfigQualifierInvalidPlacement,
            qualifier.qualifier_span,
        )
        .into());
    }

    // Local declaration-ordering hints: declared type annotation only.
    // WHY: constant initializer references are now first-class ordering hints generated by
    // headers/constant_dependencies.rs; this function only collects type-surface hints.
    collect_constant_type_hints(
        &declaration_syntax,
        context,
        local_ordering_hints,
        capacity_references,
    )?;

    Ok(declaration_syntax)
}

fn internal_header_dispatch_error(
    message: &'static str,
    span: Option<SourceSpan>,
) -> CompilerError {
    CompilerError::new(message, span, ErrorType::Compiler)
}
