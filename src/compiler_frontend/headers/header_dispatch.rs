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
    SourceTokens, TokenCursor, TokenIndex, TokenRange, TokenRef, TokenTag,
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

/// Source position and visibility metadata for one declaration header.
pub(super) struct HeaderDeclarationMetadata {
    pub(super) declaration_span: SourceSpan,
    pub(super) declaration_order: usize,
    pub(super) export_mode: HeaderExportMode,
}

fn source_token_at_index<'a>(
    canonical: &'a SourceTokens,
    file_id: SourceId,
    index: usize,
    owner: &'static str,
) -> HeaderDispatchResult<TokenRef<'a>> {
    if canonical.source() != file_id {
        return Err(internal_header_dispatch_error(
            "Header dispatch source token owner does not match its file identity.",
            None,
        )
        .into());
    }
    let position = TokenIndex::try_from_index(index).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(owner, None))
    })?;
    canonical.token(position).map_err(|_| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(owner, None))
    })
}

fn source_tag_at_cursor(
    cursor: &TokenCursor<'_>,
    file_id: SourceId,
) -> HeaderDispatchResult<TokenTag> {
    let current = cursor.current().ok_or_else(|| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Header dispatch cursor exceeded its source owner.",
            None,
        ))
    })?;
    if current.source() != file_id {
        return Err(internal_header_dispatch_error(
            "Header dispatch source token owner does not match its file identity.",
            Some(current.source_span()),
        )
        .into());
    }
    Ok(current.tag())
}

fn source_next_tag(
    cursor: &TokenCursor<'_>,
    file_id: SourceId,
) -> HeaderDispatchResult<Option<TokenTag>> {
    let canonical = cursor.source_tokens();
    if canonical.source() != file_id {
        return Err(internal_header_dispatch_error(
            "Header dispatch source token owner does not match its file identity.",
            cursor.current().map(|token| token.source_span()),
        )
        .into());
    }
    let next_index = cursor.position().index().checked_add(1).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Header dispatch follower index overflowed its source range.",
            cursor.current().map(|token| token.source_span()),
        ))
    })?;
    if next_index >= canonical.len() {
        return Ok(None);
    }
    Ok(Some(
        source_token_at_index(
            canonical,
            file_id,
            next_index,
            "Header dispatch follower index exceeded its source owner.",
        )?
        .tag(),
    ))
}

fn starts_build_config_qualifier_at_cursor(
    cursor: &TokenCursor<'_>,
    file_id: SourceId,
    string_table: &StringTable,
) -> HeaderDispatchResult<bool> {
    let canonical = cursor.source_tokens();
    let index = TokenIndex::try_from_index(cursor.position().index()).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Header dispatch token index exceeded its checked domain.",
            cursor.current().map(|token| token.source_span()),
        ))
    })?;
    if canonical.source() != file_id {
        return Err(internal_header_dispatch_error(
            "Header dispatch source token owner does not match its file identity.",
            cursor.current().map(|token| token.source_span()),
        )
        .into());
    }
    Ok(starts_build_config_qualifier_at_source(
        canonical,
        index,
        string_table,
    ))
}

fn set_cursor_position(
    cursor: &mut TokenCursor<'_>,
    position: TokenIndex,
    owner: &'static str,
) -> HeaderDispatchResult<()> {
    cursor.set_position(position).map_err(|error| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
            "{owner}: {error:?}",
        )))
    })
}

fn cursor_span(cursor: &TokenCursor<'_>, fallback: SourceSpan) -> SourceSpan {
    cursor
        .current()
        .map_or(fallback, |token| token.source_span())
}
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
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    metadata: HeaderDeclarationMetadata,
    context: &mut HeaderBuildContext<'_>,
    span_builder: &mut ExtendedSpanBuilder,
) -> HeaderDispatchResult<Header> {
    let HeaderDeclarationMetadata {
        declaration_span,
        declaration_order,
        export_mode,
    } = metadata;
    let name_span = declaration_span;
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
    let generic_parameters = parse_optional_generic_parameters(cursor, file_id, context)?;
    let current_tag = source_tag_at_cursor(cursor, file_id)?;
    let next_tag = source_next_tag(cursor, file_id)?;
    let mut body_range = empty_token_range(cursor, file_id)?;

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

        let mut trait_cursor = *cursor;
        let target =
            parse_specialized_conformance_target(&mut trait_cursor, declaration_name, name_span)?;
        let _ = trait_cursor.advance(); // past must

        let conformance = parse_trait_conformance(&mut trait_cursor, target, context)?;
        let cursor_position = trait_cursor.position();
        set_cursor_position(
            cursor,
            cursor_position,
            "specialized conformance cursor handoff exceeded its source owner",
        )?;
        let conformance_id = conformance_header_path(
            full_name,
            name_span,
            context.path_fork,
            context.string_table,
        )?;
        return Ok(Header {
            kind: HeaderKind::TraitConformance { conformance },
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
            let mut trait_cursor = *cursor;
            let _ = trait_cursor.advance(); // past must
            let _ = trait_cursor.advance(); // past not
            let subject = TraitReferenceSyntax {
                name: declaration_name,
                span: name_span,
            };
            let incompatibility = parse_trait_incompatibility(
                &mut trait_cursor,
                subject,
                declaration_order,
                context,
            )?;
            let cursor_position = trait_cursor.position();
            set_cursor_position(
                cursor,
                cursor_position,
                "trait incompatibility cursor handoff exceeded its source owner",
            )?;
            kind = HeaderKind::TraitIncompatibility { incompatibility };
        } else if peek == Some(TokenTag::COLON) {
            ensure_trait_name_is_all_caps(declaration_name, Some(name_span), context.string_table)?;

            // Trait declaration: `Name must: requirements ;`
            let mut trait_cursor = *cursor;
            let _ = trait_cursor.advance(); // past must
            let _ = trait_cursor.advance(); // past :

            let declaration = parse_trait_declaration(
                &mut trait_cursor,
                name_span,
                declaration_name,
                declaration_order,
                context,
                span_builder,
            )?;
            let cursor_position = trait_cursor.position();
            set_cursor_position(
                cursor,
                cursor_position,
                "trait declaration cursor handoff exceeded its source owner",
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
            let mut trait_cursor = *cursor;
            let _ = trait_cursor.advance(); // past must

            let conformance = parse_trait_conformance(
                &mut trait_cursor,
                ConformanceTargetSyntax {
                    name: declaration_name,
                    kind: ConformanceTargetKind::Named,
                    span: name_span,
                },
                context,
            )?;
            let cursor_position = trait_cursor.position();
            set_cursor_position(
                cursor,
                cursor_position,
                "trait conformance cursor handoff exceeded its source owner",
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
            let (signature, cursor_position) = {
                let mut declaration_cursor = DeclarationCursor::new(*cursor)?;
                let signature = parse_function_signature_syntax(
                    &mut declaration_cursor,
                    context.warnings,
                    context.string_table,
                    full_name,
                    context.path_fork,
                    span_builder,
                )?;
                let cursor_position = declaration_cursor.canonical_cursor().position();
                (signature, cursor_position)
            };
            set_cursor_position(
                cursor,
                cursor_position,
                "function signature cursor handoff exceeded its source owner",
            )?;

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

            body_range =
                capture_function_body_range_from_cursor(cursor, file_id, context.string_table)?;

            kind = HeaderKind::Function {
                generic_parameters,
                signature,
            };
        }

        // `This` keyword outside trait declarations is invalid.
        TokenTag::TRAIT_THIS => {
            return Err(CompilerDiagnostic::invalid_this_usage(
                crate::compiler_frontend::compiler_messages::InvalidThisUsageReason::OutsideTraitDeclaration,
                Some(cursor_span(cursor, declaration_span)),
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
                cursor.advance();
                let (fields, cursor_position) = {
                    let mut declaration_cursor = DeclarationCursor::new(*cursor)?;
                    let fields = parse_struct_shell(
                        &mut declaration_cursor,
                        context.string_table,
                        context.warnings,
                        full_name,
                        context.path_fork,
                        span_builder,
                    )?;
                    let cursor_position = declaration_cursor.canonical_cursor().position();
                    (fields, cursor_position)
                };
                set_cursor_position(
                    cursor,
                    cursor_position,
                    "struct declaration cursor handoff exceeded its source owner",
                )?;
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
                && starts_build_config_qualifier_at_cursor(cursor, file_id, context.string_table)?
            {
                return Err(CompilerDiagnostic::invalid_config_reason(
                    Some(declaration_name),
                    InvalidConfigReason::ConfigQualifierInvalidPlacement,
                    Some(cursor_span(cursor, declaration_span)),
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
                cursor,
                file_id,
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

            let (choice_header, cursor_position) = {
                let mut declaration_cursor = DeclarationCursor::new(*cursor)?;
                let choice_header = parse_choice_header_payload(
                    &mut declaration_cursor,
                    full_name,
                    context.path_fork,
                    context.string_table,
                    context.warnings,
                    span_builder,
                )?;
                let cursor_position = declaration_cursor.canonical_cursor().position();
                (choice_header, cursor_position)
            };
            set_cursor_position(
                cursor,
                cursor_position,
                "choice declaration cursor handoff exceeded its source owner",
            )?;

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

            cursor.advance();
            let (target, cursor_position) = {
                let mut declaration_cursor = DeclarationCursor::new(*cursor)?;
                let target = parse_type_annotation_cursor(
                    &mut declaration_cursor,
                    TypeAnnotationContext::TypeAliasTarget,
                    context.string_table,
                )?;
                let cursor_position = declaration_cursor.canonical_cursor().position();
                (target, cursor_position)
            };
            set_cursor_position(
                cursor,
                cursor_position,
                "type alias cursor handoff exceeded its source owner",
            )?;

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
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    context: &mut HeaderBuildContext<'_>,
) -> HeaderDispatchResult<GenericParameterList> {
    if source_tag_at_cursor(cursor, file_id)? != TokenTag::TYPE {
        return Ok(GenericParameterList::default());
    }

    // Dependency visibility is file-wide, so the complete retained clause set is not available
    // until the header parser has finished walking this file. File-level preparation validates
    // dependency-name collisions after all clauses and declaration shells are retained.
    let forbidden_names = FxHashSet::default();
    let (result, cursor_position) = {
        let mut declaration_cursor = DeclarationCursor::new(*cursor)?;
        let result = parse_generic_parameter_list_after_type_keyword(
            &mut declaration_cursor,
            &forbidden_names,
            context.string_table,
        )?;
        let cursor_position = declaration_cursor.canonical_cursor().position();
        (result, cursor_position)
    };
    set_cursor_position(
        cursor,
        cursor_position,
        "generic parameter cursor handoff exceeded its source owner",
    )?;
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

fn empty_token_range(
    cursor: &TokenCursor<'_>,
    file_id: SourceId,
) -> HeaderDispatchResult<TokenRange> {
    let index = TokenIndex::try_from_index(cursor.position().index()).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Header token range exceeded the source token index space.",
            cursor.current().map(|token| token.source_span()),
        ))
    })?;
    TokenRange::new(file_id, index, index).ok_or_else(|| {
        HeaderParseFailure::Infrastructure(internal_header_dispatch_error(
            "Empty header token range was invalid.",
            cursor.current().map(|token| token.source_span()),
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
fn create_constant_header_payload(
    full_name: &PathId,
    cursor: &mut TokenCursor<'_>,
    file_id: SourceId,
    context: &mut HeaderBuildContext<'_>,
    local_ordering_hints: &mut HashSet<LocalDeclarationOrderingHint>,
    capacity_references: &mut Vec<InitializerReference>,
    span_builder: &mut ExtendedSpanBuilder,
) -> HeaderDispatchResult<DeclarationSyntax> {
    let Some(declaration_name) = context.path_fork.component(*full_name) else {
        return Err(internal_header_dispatch_error(
            "Constant header path is missing its declaration name.",
            cursor.current().map(|token| token.source_span()),
        )
        .into());
    };
    let (declaration_syntax, cursor_position) = {
        let mut declaration_cursor = DeclarationCursor::new(*cursor)?;
        let declaration_syntax = parse_declaration_syntax(
            &mut declaration_cursor,
            declaration_name,
            context.string_table,
            span_builder,
        )?;
        let cursor_position = declaration_cursor.canonical_cursor().position();
        (declaration_syntax, cursor_position)
    };
    set_cursor_position(
        cursor,
        cursor_position,
        "constant declaration cursor handoff exceeded its source owner",
    )?;
    // A comma terminates anonymous-record fields, but it cannot terminate a top-level source
    // contract. Keep the shared declaration parser permissive for record fields and reject this
    // malformed source declaration at the header boundary.
    if declaration_syntax.initializer_range.is_none()
        && source_tag_at_cursor(cursor, file_id)? == TokenTag::COMMA
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
